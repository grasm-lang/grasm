//! Concurrent tasks and channels for compiled programs (docs/concurrency.md,
//! stage 4). The same design as the interpreter's `crate::tasks` there, and
//! the same messages, so the two engines cannot disagree about a schedule.
//!
//! Tasks are fibers on the thread that ran the `concurrent` block, one running
//! at a time, switching only when a channel operation has to wait. A task is a
//! compiled closure taking no arguments; the block collects them in a list the
//! compiled code owns, so a block left early releases them with its scope.
//!
//! Channels live in one table, behind a mutex rather than per thread: a
//! channel is a plain integer, and nothing stops a `par_map` worker making one
//! and handing it back. The table is a root, like the `ref` slab, and is
//! released before the leak check counts.

// `std`'s prelude is gone with `#![no_std]`; `alloc` owns the growable types.
use alloc::format;
use alloc::vec::Vec;

use alloc::collections::BTreeSet;
use alloc::collections::VecDeque;
use crate::sync::Mutex;

use crate::{dec, inc, none_val, rt_panic, some_of, RtPayload, RtVal, LOCAL, STACK_BUDGET};

/// Which `concurrent` block a waiting task belongs to, and which of its
/// slots.
///
/// The block's id and not just the slot, because `CHANS` is one table for the
/// whole process: a `par_map` worker may run a `concurrent` block of its own,
/// and a channel can reach both. A wake has to find the scheduler that owns
/// the task, which may not be the one doing the waking.
type Waiter = (u64, usize);

/// One task of a `concurrent` block: its body, and the fiber it runs on with
/// that fiber's stack floor once it has started.
type Slot = (*mut RtVal, Option<(grasm_fiber::Fiber, usize)>);

struct Chan {
    /// Tasks waiting for room to send, and tasks waiting for a value,
    /// longest-waiting first in each ([E-SCHEDULE]).
    ///
    /// Two queues and not one: with a single queue a send wakes whichever
    /// task waited longest, which may be another sender -- it finds the
    /// buffer just as full and waits again, and the receiver that could have
    /// gone on is never reached.
    want_room: VecDeque<Waiter>,
    want_value: VecDeque<Waiter>,
    buf: VecDeque<*mut RtVal>,
    /// How many values it holds before a send waits; 0 is a rendezvous.
    cap: usize,
    closed: bool,
    /// Values sent and received so far. A rendezvous send is done once the
    /// value it sent has been taken.
    sent: u64,
    taken: u64,
}

struct Chans {
    chans: Vec<Chan>,
    /// Bumped whenever any channel changes, so the scheduler can tell a round
    /// in which someone got somewhere from one in which nobody could.
    progress: u64,
    /// Tasks a channel operation has woken, for whichever scheduler owns
    /// each. A scheduler takes the entries that are its own and leaves the
    /// rest, so a wake crossing threads is delivered rather than dropped.
    ready: VecDeque<Waiter>,
    /// Which waiters are really in a queue. A blocked operation retries in a
    /// loop, so a task re-joins a queue every time it gives way and the
    /// queues fill with entries for tasks since woken or finished; a wake
    /// spent on one of those is a wake nobody gets.
    waiting: BTreeSet<Waiter>,
    /// Ids handed to `concurrent` blocks, so two on different threads cannot
    /// collide.
    next_block: u64,
}

// Safety: the pointers are runtime values, whose counts are atomic, and the
// table is only reached under its mutex.
unsafe impl Send for Chans {}

static CHANS: Mutex<Chans> = Mutex::new(Chans {
    chans: Vec::new(),
    progress: 0,
    ready: VecDeque::new(),
    waiting: BTreeSet::new(),
    next_block: 0,
});

thread_local! {
    /// The `concurrent` block running on this thread, and the slot of the
    /// task in it, so a channel operation that has to wait knows whom to
    /// enqueue. A fiber never moves between threads, so a thread-local is
    /// the right place for it.
    static HERE: core::cell::Cell<(u64, usize)> = const { core::cell::Cell::new((0, 0)) };
}

/// Which queue of a channel a wake-up is for: a sender wants room, a
/// receiver wants a value.
#[derive(Clone, Copy)]
enum Side {
    Room,
    Value,
}

impl Chans {
    /// Wakes the longest-waiting task on one side of `id`, skipping entries
    /// left behind by a retry. Exactly one: a value, or room for one, serves
    /// one task, and waking the rest only to have them wait again is the
    /// O(live) round this replaces ([E-SCHEDULE]).
    fn wake_one(&mut self, id: usize, side: Side) {
        let Some(c) = self.chans.get_mut(id) else {
            return;
        };
        let queue = match side {
            Side::Room => &mut c.want_room,
            Side::Value => &mut c.want_value,
        };
        while let Some(who) = queue.pop_front() {
            if self.waiting.remove(&who) {
                self.ready.push_back(who);
                return;
            }
        }
    }

    /// Closing changes every waiter's answer, so it wakes both sides.
    fn wake_all(&mut self, id: usize) {
        let Some(c) = self.chans.get_mut(id) else {
            return;
        };
        let mut woken: VecDeque<Waiter> = core::mem::take(&mut c.want_room);
        woken.extend(core::mem::take(&mut c.want_value));
        for who in woken {
            if self.waiting.remove(&who) {
                self.ready.push_back(who);
            }
        }
    }
}

const DEADLOCK: &str =
    "deadlock: every task is waiting on a channel, and nothing left can send or receive";

fn chans() -> crate::sync::Guard<'static, Chans> {
    CHANS.lock()
}

/// A channel operation that cannot go ahead: inside a task, give way and try
/// again when resumed; outside every task there is nobody to give way to.
/// Gives way until one of `on` moves, joining each channel's queue first so
/// that a send or receive can wake exactly this task ([E-SCHEDULE]).
fn wait(on: &[usize], side: Side) {
    if grasm_fiber::in_fiber() {
        {
            let me = HERE.with(|h| h.get());
            let mut guard = chans();
            guard.waiting.insert(me);
            for &id in on {
                if let Some(c) = guard.chans.get_mut(id) {
                    match side {
                        Side::Room => c.want_room.push_back(me),
                        Side::Value => c.want_value.push_back(me),
                    }
                }
            }
        }
        grasm_fiber::suspend();
    } else {
        rt_panic(&format!(
            "{DEADLOCK} -- a channel operation outside every `go` cannot wait for one; move it into a task"
        ));
    }
}

fn index(ch: i64, chans: &Chans) -> usize {
    if ch < 0 || ch as usize >= chans.chans.len() {
        rt_panic("not a channel");
    }
    ch as usize
}

/// `channel(n)`.
#[no_mangle]
pub extern "C" fn grasm_chan_new(cap: i64) -> i64 {
    let mut t = chans();
    t.chans.push(Chan {
        want_room: VecDeque::new(),
        want_value: VecDeque::new(),
        buf: VecDeque::new(),
        cap: cap.max(0) as usize,
        closed: false,
        sent: 0,
        taken: 0,
    });
    t.chans.len() as i64 - 1
}

/// `chan_send(ch, v)`. Takes the reference to `v`.
///
/// # Safety: `v` is a live runtime value the caller owns a count on.
#[no_mangle]
pub unsafe extern "C" fn grasm_chan_send(ch: i64, v: *mut RtVal) {
    let mut v = Some(v);
    let mut ticket: Option<u64> = None;
    // The index does not change, and both the wait and the wake need it.
    let at = index(ch, &chans());
    loop {
        {
            let mut guard = chans();
            let Chans { chans, progress, .. } = &mut *guard;
            let c = &mut chans[at];
            if let Some(t) = ticket {
                if c.taken > t {
                    return;
                }
            } else {
                if c.closed {
                    drop(guard);
                    rt_panic("send on a closed channel");
                }
                if c.cap == 0 || c.buf.len() < c.cap {
                    c.buf.push_back(v.take().expect("sent once"));
                    c.sent += 1;
                    *progress += 1;
                    let full = c.cap > 0;
                    let next_ticket = c.sent - 1;
                    // A value arrived, so one receiver can go on.
                    guard.wake_one(at, Side::Value);
                    drop(guard);
                    if full {
                        return;
                    }
                    ticket = Some(next_ticket);
                    continue;
                }
            }
        }
        wait(&[at], Side::Room);
    }
}

/// `chan_recv(ch)`: `Some(v)`, or `None` once closed and drained.
#[no_mangle]
pub extern "C" fn grasm_chan_recv(ch: i64) -> *mut RtVal {
    let at = index(ch, &chans());
    loop {
        {
            let mut guard = chans();
            let Chans { chans, progress, .. } = &mut *guard;
            let c = &mut chans[at];
            if let Some(v) = c.buf.pop_front() {
                c.taken += 1;
                *progress += 1;
                // Room opened, and a rendezvous sender is waiting for its
                // own value to be taken.
                guard.wake_one(at, Side::Room);
                return some_of(v);
            }
            if c.closed {
                return none_val();
            }
        }
        wait(&[at], Side::Value);
    }
}

/// `chan_select(chs)`: the index of the first channel a `chan_recv` would not
/// wait on, waiting until there is one. Borrows the list.
///
/// # Safety: `chs` is a live list of channels.
#[no_mangle]
pub unsafe extern "C" fn grasm_chan_select(chs: *mut RtVal) -> i64 {
    let ids = crate::scalar_words(chs, "chan_select");
    if ids.is_empty() {
        rt_panic("`chan_select` of no channels would wait forever");
    }
    let ats: Vec<usize> = {
        let guard = chans();
        ids.iter().map(|&id| index(id, &guard)).collect()
    };
    loop {
        {
            let guard = chans();
            for (i, &at) in ats.iter().enumerate() {
                let c = &guard.chans[at];
                if !c.buf.is_empty() || c.closed {
                    return i as i64;
                }
            }
        }
        // A select waits on all of them and is woken by whichever moves
        // first, so it joins every queue.
        wait(&ats, Side::Value);
    }
}

/// `chan_close(ch)`.
#[no_mangle]
pub extern "C" fn grasm_chan_close(ch: i64) {
    let mut guard = chans();
    let at = index(ch, &guard);
    let Chans { chans, progress, .. } = &mut *guard;
    let c = &mut chans[at];
    if c.closed {
        drop(guard);
        rt_panic("closing a channel that is already closed");
    }
    c.closed = true;
    *progress += 1;
    // Closing changes every waiter's answer, and none will wait on this
    // channel again.
    guard.wake_all(at);
}

/// Releases every value still sitting in a channel. For the leak check: what
/// a channel holds is reachable for as long as the program runs.
pub(crate) fn release_channels() {
    let taken: Vec<*mut RtVal> = {
        let mut t = chans();
        t.chans.iter_mut().flat_map(|c| c.buf.drain(..)).collect()
    };
    for v in taken {
        // Safety: each buffered value holds the count its sender handed over.
        unsafe { dec(v) };
    }
}

/// Room below a task's floor for the runtime's own frames, which the check
/// does not cover.
const TASK_MARGIN: usize = 512 * 1024;

/// A task's stack: the same budget as the thread `main` runs on, so a deep
/// recursion panics at the same depth inside a task as outside one. The pages
/// are reserved, not touched; what a task does not use costs no memory.
const TASK_STACK: usize = STACK_BUDGET + TASK_MARGIN;

unsafe fn run_task(f: *mut RtVal) {
    let code = crate::grasm_fn_code(f);
    let env = crate::grasm_fn_env(f);
    // Safety: a task is a closure the compiler lifted with no parameters and
    // no result, which takes its environment and consumes that reference.
    let body: extern "C" fn(*mut RtVal) = core::mem::transmute(code as usize);
    body(env);
    dec(f);
}

/// Runs a `concurrent` block's tasks to their end, in the fixed order
/// described in docs/concurrency.md. `tasks` is the list of function values
/// its `go`s made, in the order they ran; it stays the caller's, and is
/// released with the block's scope however the block ends.
///
/// # Safety: `tasks` is a live list of function values of no arguments.
#[no_mangle]
pub unsafe extern "C" fn grasm_conc_run(tasks: *mut RtVal) {
    let tasks: Vec<*mut RtVal> = match &(*tasks).payload {
        RtPayload::List(items) => items
            .iter()
            .map(|&f| {
                // The task's own count, released when it ends.
                inc(f);
                f
            })
            .collect(),
        _ => rt_panic("`concurrent` without its task list"),
    };
    if tasks.is_empty() {
        return;
    }
    // Every task starts runnable, in the order its `go` ran. After that a
    // task is runnable only when a channel operation wakes it, so a round
    // costs what is runnable and not what is alive ([E-SCHEDULE]). A task's
    // fiber is made when it first runs, so a task that ends without waiting
    // hands its stack to the next rather than every task holding one from
    // the start.
    let mut slots: Vec<Option<Slot>> = tasks.into_iter().map(|f| Some((f, None))).collect();
    let mut runnable: VecDeque<usize> = (0..slots.len()).collect();
    let mut alive = slots.len();
    let block = {
        let mut guard = chans();
        guard.next_block += 1;
        guard.next_block
    };
    let outer_here = HERE.with(|h| h.get());

    loop {
        // One round is whatever is runnable at its start; anything woken
        // during it is taken by the next.
        let this_round: Vec<usize> = runnable.drain(..).collect();
        for i in this_round {
            // `None` means it finished earlier, or a `select` woke it twice
            // and this is the second: either way, nothing to run.
            let Some((f, mut fiber)) = slots[i].take() else {
                continue;
            };
            HERE.with(|h| h.set((block, i)));
            let (fiber_ref, floor) = fiber.get_or_insert_with(|| {
                // Safety: see `run_task`; the value lives until the task ends.
                let fb = grasm_fiber::Fiber::new(TASK_STACK, move || unsafe { run_task(f) });
                let floor = fb.stack_bottom() + TASK_MARGIN;
                (fb, floor)
            });
            // The stack check reads one floor per thread, and the stack it
            // guards is now this task's.
            let outer = LOCAL.with(|l| l.stack_floor.replace(*floor));
            let done = fiber_ref.resume();
            LOCAL.with(|l| l.stack_floor.set(outer));
            if done {
                alive -= 1;
            } else {
                // It gave way. If it joined a channel's queue, that queue
                // will wake it; if it gave way without blocking, it is still
                // runnable.
                if !chans().waiting.contains(&(block, i)) {
                    runnable.push_back(i);
                }
                slots[i] = Some((f, fiber));
            }
        }
        if alive == 0 {
            break;
        }
        // Take the wake-ups meant for this block and leave any belonging to
        // another, which a `concurrent` block on a second thread would own.
        {
            let mut guard = chans();
            let mut mine: VecDeque<usize> = VecDeque::new();
            let all: VecDeque<Waiter> = core::mem::take(&mut guard.ready);
            for who in all {
                if who.0 == block {
                    mine.push_back(who.1);
                } else {
                    guard.ready.push_back(who);
                }
            }
            runnable.extend(mine);
        }
        if runnable.is_empty() {
            // Every live task is in a wait queue and no channel can move, so
            // nothing will wake them ([E-DEADLOCK]).
            rt_panic(DEADLOCK);
        }
    }
    HERE.with(|h| h.set(outer_here));
}
