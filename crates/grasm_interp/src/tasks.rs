//! Concurrent tasks and channels in the interpreter (docs/concurrency.md).
//!
//! Each task of a `concurrent` block is its own interpreter, running on a
//! fiber of this thread. They share two things, both behind `Rc<RefCell<..>>`
//! and never borrowed across a switch: the channel table, and one output
//! buffer the scheduler drains into the program's output after every resume.
//! So no `&mut` interpreter is ever live on two stacks at once, nothing a
//! suspended task holds is touched by another, and the `Rc` values everything
//! here is made of never leave the thread.
//!
//! Scheduling is cooperative and fixed: tasks start in the order their `go`s
//! ran and each runs until it finishes or has to wait on a channel, which is
//! the only place a task gives way; then the next one in line runs. A task
//! that waits simply tries again the next time its turn comes round. If a
//! whole round goes by with every live task waiting and no channel having
//! changed, nothing ever will: that is a deadlock, and it is reported.

use std::cell::RefCell;
use std::collections::{HashMap, VecDeque};
use std::io::Write;
use std::rc::Rc;

use grasm_syntax::ast::Block;

use crate::interp::{EResult, Flow, Interp, STACK_BUDGET, STACK_FLOOR};
use crate::value::Value;

/// A `go` body waiting for its block to start it, with the scopes it sees.
pub(crate) type Task = (Block, Vec<HashMap<String, Value>>);

/// Every channel the program has made, by id. A channel value is its id.
#[derive(Default)]
pub struct Chans {
    chans: Vec<Chan>,
    /// Bumped whenever any channel changes, so the scheduler can tell a round
    /// in which someone got somewhere from one in which nobody could.
    pub(crate) progress: u64,
    /// `grasm test --schedules`: when set, each round visits the tasks in an
    /// order drawn from this state rather than first in first out. The same
    /// seed draws the same orders, which is what makes a failure replayable.
    pub(crate) schedule: Option<u64>,
    /// How many `concurrent` blocks have run tasks, so the test runner can
    /// tell a test that has a schedule to vary from one that has none.
    pub(crate) blocks_run: u64,
    /// Which block's task slot is running, so a channel operation that has
    /// to wait knows whom to enqueue. Set by the scheduler around each
    /// resume, and put back when a nested block returns.
    pub(crate) running: Waiter,
    /// Whether each slot of each open block is currently in some channel's
    /// wait queue.
    ///
    /// A blocked operation retries in a loop, so a task re-joins a queue
    /// every time it gives way, and a queue accumulates entries for slots
    /// that have since been woken or finished. Without this, `wake_one`
    /// spends its wake on a stale entry and the task that was actually
    /// waiting is never resumed -- which reports a deadlock on a program
    /// that has none.
    ///
    /// Per block, as the compiled runtimes keep it: a task of one
    /// `concurrent` block can run another, and while the inner one
    /// schedules, a task of the outer one can still be woken -- by a channel
    /// operation the outer block's tasks made before it. One table of slots
    /// for both lost that wake: the inner block cleared it, and the outer
    /// block then reported a deadlock in a program that has none.
    pub(crate) waiting: Vec<(u64, Vec<bool>)>,
    /// Waiters a channel operation has woken and their block's scheduler has
    /// not yet taken. Draining this rather than resuming every live task is
    /// what makes a round O(runnable) instead of O(live) ([E-SCHEDULE]).
    pub(crate) ready: VecDeque<Waiter>,
    /// Ids handed to `concurrent` blocks, so a nested block's slots are not
    /// mistaken for its parent's.
    pub(crate) next_block: u64,
    /// Tier 2's slab, while it is in transit between interpreters. A `ref`
    /// (an actor's state is one) names a slot in *the* slab, so there can be
    /// only one; each task's interpreter holds it while it runs and puts it
    /// here when it gives way, and the scheduler does the same around each
    /// resume. Only one interpreter runs at a time, so it is never needed in
    /// two places at once.
    pub(crate) slab_hold: Option<Slab>,
}

pub(crate) type Slab = (Vec<(Option<Value>, u32)>, Vec<u32>);

/// Which `concurrent` block a waiting task belongs to, and which of its
/// slots it is.
pub(crate) type Waiter = (u64, usize);

/// SplitMix64: a fixed, portable generator, so a seed names the same orders
/// on every machine.
fn next(state: &mut u64) -> u64 {
    *state = state.wrapping_add(0x9e37_79b9_7f4a_7c15);
    let mut z = *state;
    z = (z ^ (z >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
    z = (z ^ (z >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
    z ^ (z >> 31)
}

impl Chans {
    /// Makes room for a block's slots, so marking one is an index.
    fn open_block(&mut self, block: u64, slots: usize) {
        self.waiting.push((block, vec![false; slots]));
    }

    /// Forgets a block, which a `concurrent` block does when it returns.
    fn close_block(&mut self, block: u64) {
        self.waiting.retain(|(b, _)| *b != block);
        self.ready.retain(|(b, _)| *b != block);
    }

    fn mark_waiting(&mut self, who: Waiter) {
        if let Some((_, flags)) = self.waiting.iter_mut().find(|(b, _)| *b == who.0) {
            if let Some(f) = flags.get_mut(who.1) {
                *f = true;
            }
        }
    }

    /// True if that waiter was really waiting, and clears it. False for an
    /// entry a retry left behind.
    fn take_waiting(&mut self, who: Waiter) -> bool {
        if let Some((_, flags)) = self.waiting.iter_mut().find(|(b, _)| *b == who.0) {
            if let Some(f) = flags.get_mut(who.1) {
                if *f {
                    *f = false;
                    return true;
                }
            }
        }
        false
    }

    fn is_waiting(&self, who: Waiter) -> bool {
        self.waiting
            .iter()
            .find(|(b, _)| *b == who.0)
            .and_then(|(_, flags)| flags.get(who.1).copied())
            .unwrap_or(false)
    }

    /// Wakes the longest-waiting task on `id`, if any, by moving it to the
    /// ready list. Exactly one: a value, or a space for one, serves one
    /// waiter, and waking the rest only to have them wait again is the
    /// O(live) round this replaces ([E-SCHEDULE]).
    ///
    /// A `select` sits in several queues, so a slot can be woken twice. The
    /// scheduler takes each slot once per round, which makes the second
    /// wake-up a no-op rather than a double resume.
    fn wake_one(&mut self, id: usize, side: Side) {
        let Some(c) = self.chans.get_mut(id) else {
            return;
        };
        let queue = match side {
            Side::Room => &mut c.want_room,
            Side::Value => &mut c.want_value,
        };
        let mut taken = None;
        while let Some(who) = queue.pop_front() {
            // `self.waiting` and not `is_waiting`: the queue still borrows
            // `self.chans`.
            if self
                .waiting
                .iter()
                .find(|(b, _)| *b == who.0)
                .and_then(|(_, f)| f.get(who.1).copied())
                .unwrap_or(false)
            {
                taken = Some(who);
                break;
            }
            // Stale: that slot was woken by another channel, or has
            // finished. Keep looking rather than spending the wake on it.
        }
        if let Some(who) = taken {
            self.take_waiting(who);
            self.ready.push_back(who);
        }
    }

    /// The order a round visits what is runnable: as given, unless
    /// `--schedules` has a seed to draw a permutation from.
    fn round_order(&mut self, n: usize) -> Vec<usize> {
        let mut order: Vec<usize> = (0..n).collect();
        if let Some(state) = self.schedule.as_mut() {
            for i in (1..n).rev() {
                let j = (next(state) % (i as u64 + 1)) as usize;
                order.swap(i, j);
            }
        }
        order
    }
}

struct Chan {
    /// Slots waiting for room to send, and slots waiting for a value to
    /// receive, longest-waiting first in each ([E-SCHEDULE]).
    ///
    /// Two queues and not one. With a single queue a send wakes whichever
    /// slot waited longest, which may be another *sender* -- it finds the
    /// buffer just as full, waits again, and the receiver that could have
    /// gone on is never reached. That reports a deadlock on a program that
    /// has none, which is what one queue did here.
    want_room: VecDeque<Waiter>,
    want_value: VecDeque<Waiter>,
    buf: VecDeque<Value>,
    /// How many values it holds before a send waits; 0 is a rendezvous.
    cap: usize,
    closed: bool,
    /// Values sent and values received so far, for a rendezvous: a send
    /// returns once its own value has been taken.
    sent: u64,
    taken: u64,
}

/// Which queue of a channel a wake-up is for: a sender wants room, a
/// receiver wants a value.
#[derive(Clone, Copy)]
pub(crate) enum Side {
    Room,
    Value,
}

/// What a channel operation that cannot go ahead does next.
pub(crate) enum Wait {
    /// Inside a task: give way, and try again when resumed.
    Yield,
    /// Outside every task -- a `concurrent` block's own statements, or
    /// straight-line code -- there is nobody to give way to.
    Deadlock,
}

impl Chans {
    pub(crate) fn make(&mut self, cap: i64) -> Value {
        self.chans.push(Chan {
            want_room: VecDeque::new(),
            want_value: VecDeque::new(),
            buf: VecDeque::new(),
            cap: cap.max(0) as usize,
            closed: false,
            sent: 0,
            taken: 0,
        });
        Value::Int(self.chans.len() as i64 - 1)
    }
}

/// The output every task writes to, drained in order by the scheduler.
pub struct TaskOut(pub Rc<RefCell<Vec<u8>>>);

impl Write for TaskOut {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        self.0.borrow_mut().extend_from_slice(buf);
        Ok(buf.len())
    }
    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

fn chan_id(v: &Value) -> Result<usize, Flow> {
    match v {
        Value::Int(i) if *i >= 0 => Ok(*i as usize),
        _ => Err(Flow::Panic("not a channel".into())),
    }
}

/// Room below a task's floor for the frames the depth check does not cover.
const TASK_MARGIN: usize = 1024 * 1024;

/// A task's stack: the budget the interpreter's own thread has, so recursion
/// stops at the same depth inside a task as outside one. Reserved, not
/// touched -- a page costs memory only once a task reaches it.
const TASK_STACK: usize = STACK_BUDGET + TASK_MARGIN;

const DEADLOCK: &str =
    "deadlock: every task is waiting on a channel, and nothing left can send or receive";

/// A task that has not had its first turn holds only its body; its fiber, and
/// the stack under it, are made when it does. A task that ends without
/// waiting then gives its stack back before the next one starts.
enum Run<'b> {
    Pending(Option<Box<dyn FnOnce() + 'b>>),
    Running(grasm_fiber::Fiber, usize),
}

impl<'a, W: Write> Interp<'a, W> {
    /// Puts this interpreter's slab where the next one to run will take it.
    pub(crate) fn park_slab(&mut self) {
        let slab = (
            std::mem::take(&mut self.slab),
            std::mem::take(&mut self.slab_free),
        );
        self.chans.borrow_mut().slab_hold = Some(slab);
    }

    /// Takes the slab the last interpreter to run left.
    pub(crate) fn take_slab(&mut self) {
        if let Some((slab, free)) = self.chans.borrow_mut().slab_hold.take() {
            self.slab = slab;
            self.slab_free = free;
        }
    }

    /// Gives way until one of `on` moves.
    ///
    /// The slot joins each channel's wait queue before suspending, so a send
    /// or receive can wake exactly it rather than the scheduler resuming
    /// every live task to find out who can go on ([E-SCHEDULE]).
    fn wait(&mut self, on: &[usize], side: Side) -> Result<(), Flow> {
        match if self.in_task { Wait::Yield } else { Wait::Deadlock } {
            Wait::Yield => {
                {
                    let mut guard = self.chans.borrow_mut();
                    let me = guard.running;
                    guard.mark_waiting(me);
                    for &id in on {
                        if let Some(c) = guard.chans.get_mut(id) {
                            match side {
                                Side::Room => c.want_room.push_back(me),
                                Side::Value => c.want_value.push_back(me),
                            }
                        }
                    }
                }
                self.park_slab();
                grasm_fiber::suspend();
                self.take_slab();
                Ok(())
            }
            Wait::Deadlock => Err(Flow::Panic(format!(
                "{DEADLOCK} -- a channel operation outside every `go` cannot wait for one; move it into a task"
            ))),
        }
    }

    /// `chan_send(ch, v)`.
    pub(crate) fn chan_send(&mut self, ch: &Value, v: Value) -> EResult {
        let id = chan_id(ch)?;
        let mut v = Some(v);
        let mut ticket: Option<u64> = None;
        loop {
            {
                let mut guard = self.chans.borrow_mut();
                let Chans {
                    chans, progress, ..
                } = &mut *guard;
                let Some(c) = chans.get_mut(id) else {
                    return Err(Flow::Panic("not a channel".into()));
                };
                if let Some(t) = ticket {
                    // A rendezvous send is done once its value is taken.
                    if c.taken > t {
                        return Ok(Value::Unit);
                    }
                } else {
                    if c.closed {
                        return Err(Flow::Panic("send on a closed channel".into()));
                    }
                    if c.cap == 0 || c.buf.len() < c.cap {
                        c.buf.push_back(v.take().expect("sent once"));
                        c.sent += 1;
                        *progress += 1;
                        let full = c.cap > 0;
                        let next_ticket = c.sent - 1;
                        drop(guard);
                        // A value arrived, so one receiver can go on. Woken
                        // outside the borrow, because `wake_one` needs the
                        // whole `Chans` and this held one channel of it.
                        self.chans.borrow_mut().wake_one(id, Side::Value);
                        if full {
                            return Ok(Value::Unit);
                        }
                        ticket = Some(next_ticket);
                        continue;
                    }
                }
            }
            self.wait(&[id], Side::Room)?;
        }
    }

    /// `chan_recv(ch)`: `Some(v)`, or `None` once closed and drained.
    pub(crate) fn chan_recv(&mut self, ch: &Value) -> EResult {
        let id = chan_id(ch)?;
        loop {
            {
                let mut guard = self.chans.borrow_mut();
                let Chans {
                    chans, progress, ..
                } = &mut *guard;
                let Some(c) = chans.get_mut(id) else {
                    return Err(Flow::Panic("not a channel".into()));
                };
                if let Some(v) = c.buf.pop_front() {
                    c.taken += 1;
                    *progress += 1;
                    drop(guard);
                    // A space opened, and a rendezvous sender is waiting for
                    // its own value to be taken.
                    self.chans.borrow_mut().wake_one(id, Side::Room);
                    return Ok(crate::interp::some(v));
                }
                if c.closed {
                    return Ok(crate::interp::none());
                }
            }
            self.wait(&[id], Side::Value)?;
        }
    }

    /// `chan_select(chs)`: the index of the first channel in `chs` that a
    /// `chan_recv` would not wait on -- one holding a value, or closed --
    /// waiting until there is one. Nothing runs between this and the
    /// `chan_recv` that follows it, so the answer is still true then.
    pub(crate) fn chan_select(&mut self, chs: &Value) -> EResult {
        let ids: Vec<usize> = match chs {
            Value::List(items) => items.iter().map(chan_id).collect::<Result<_, _>>()?,
            _ => return Err(Flow::Panic("not a list of channels".into())),
        };
        if ids.is_empty() {
            return Err(Flow::Panic(
                "`chan_select` of no channels would wait forever".into(),
            ));
        }
        loop {
            {
                let guard = self.chans.borrow();
                for (i, id) in ids.iter().enumerate() {
                    let Some(c) = guard.chans.get(*id) else {
                        return Err(Flow::Panic("not a channel".into()));
                    };
                    if !c.buf.is_empty() || c.closed {
                        return Ok(Value::Int(i as i64));
                    }
                }
            }
            // A select waits on all of them and is woken by whichever moves
            // first, so it joins every queue.
            self.wait(&ids, Side::Value)?;
        }
    }

    /// `chan_close(ch)`.
    pub(crate) fn chan_close(&mut self, ch: &Value) -> EResult {
        let id = chan_id(ch)?;
        let mut guard = self.chans.borrow_mut();
        let Chans {
            chans, progress, ..
        } = &mut *guard;
        let Some(c) = chans.get_mut(id) else {
            return Err(Flow::Panic("not a channel".into()));
        };
        if c.closed {
            return Err(Flow::Panic(
                "closing a channel that is already closed".into(),
            ));
        }
        c.closed = true;
        *progress += 1;
        // Closing is the one case that wakes *all* of them: every waiter's
        // answer changes, and none will wait on this channel again.
        let mut woken: VecDeque<Waiter> = core::mem::take(&mut c.want_room);
        woken.extend(core::mem::take(&mut c.want_value));
        for who in woken {
            if guard.take_waiting(who) {
                guard.ready.push_back(who);
            }
        }
        Ok(Value::Unit)
    }

    /// Runs a `concurrent` block's tasks to completion, in the fixed order
    /// described at the top of this file.
    pub(crate) fn run_tasks(&mut self, tasks: Vec<Task>) -> EResult {
        if tasks.is_empty() {
            return Ok(Value::Unit);
        }
        if !grasm_fiber::supported() {
            return Err(Flow::Panic(
                "`concurrent` needs a platform that can switch stacks, which this one cannot"
                    .into(),
            ));
        }
        self.chans.borrow_mut().blocks_run += 1;
        let shared = Rc::new(RefCell::new(Vec::new()));
        type Ending = Rc<RefCell<Option<Result<(), Flow>>>>;
        let mut live: Vec<(Run<'_>, Ending, *mut TaskOut)> = Vec::new();
        for (body, scopes) in tasks {
            let out = Box::into_raw(Box::new(TaskOut(shared.clone())));
            // Safety: the box lives until after this task's interpreter is
            // dropped, below; nothing else holds the pointer.
            let out_ref: &'a mut TaskOut = unsafe { &mut *out };
            let mut task = self.fork_for_task(out_ref);
            task.scopes = scopes;
            let ending: Ending = Rc::new(RefCell::new(None));
            let end2 = ending.clone();
            // What the task runs, once it has a fiber to run on.
            let body: Box<dyn FnOnce() + '_> = Box::new(move || {
                task.take_slab();
                let r = task.eval_block(&body).map(|_| ());
                task.park_slab();
                let r = match r {
                    Err(Flow::Return(_)) => Ok(()),
                    other => other,
                };
                *end2.borrow_mut() = Some(r);
                drop(task);
            });
            live.push((Run::Pending(Some(body)), ending, out));
        }
        // Every task starts runnable, in the order its `go` ran. After that a
        // task is runnable only when a channel operation wakes it, so a round
        // costs O(runnable) and not O(live) -- which is what lets the task
        // count grow ([E-SCHEDULE]).
        let mut slots: Vec<Option<_>> = live.drain(..).map(Some).collect();
        let mut runnable: VecDeque<usize> = (0..slots.len()).collect();
        let mut alive = slots.len();
        let (block, outer_running) = {
            let mut guard = self.chans.borrow_mut();
            guard.next_block += 1;
            let block = guard.next_block;
            guard.open_block(block, slots.len());
            (block, guard.running)
        };

        let result = loop {
            let mut failed: Option<Flow> = None;
            // One round is whatever is runnable at its start. Anything woken
            // during it is taken by the next, which keeps the order a task
            // sees independent of where in the round it happened to be.
            let mut this_round: Vec<usize> = runnable.drain(..).collect();
            // `--schedules` reorders the round, as it always has. The queue
            // decides who is *in* a round; the seed decides their order
            // within it.
            let order = self.chans.borrow_mut().round_order(this_round.len());
            this_round = order.into_iter().map(|i| this_round[i]).collect();

            for i in this_round {
                // `None` means it finished earlier, or a `select` woke it
                // twice and this is the second: either way, nothing to run.
                let Some(mut entry) = slots[i].take() else {
                    continue;
                };
                {
                    let mut guard = self.chans.borrow_mut();
                    guard.running = (block, i);
                    // About to run, so not waiting -- whatever queues still
                    // hold its slot are stale from here on.
                    guard.take_waiting((block, i));
                }
                // The depth check measures the stack it runs on, and that is
                // now this task's -- not the thread's, whose floor lies far
                // below it and would call every task's first call too deep.
                if let Run::Pending(body) = &mut entry.0 {
                    let body = body.take().expect("started once");
                    // Safety: every fiber made here is run to its end or
                    // dropped before this function returns, and everything it
                    // borrows -- the program, through the task -- outlives it.
                    let fiber = unsafe { grasm_fiber::Fiber::new_scoped(TASK_STACK, body) };
                    let floor = fiber.stack_bottom() + TASK_MARGIN;
                    entry.0 = Run::Running(fiber, floor);
                }
                let Run::Running(fiber, floor) = &mut entry.0 else {
                    unreachable!("started above")
                };
                let outer = STACK_FLOOR.with(|f| f.replace(*floor));
                self.park_slab();
                let ended = fiber.resume();
                self.take_slab();
                STACK_FLOOR.with(|f| f.set(outer));
                // Output in the order it was written, as every engine prints.
                let bytes = std::mem::take(&mut *shared.borrow_mut());
                if !bytes.is_empty() {
                    let _ = self.out.write_all(&bytes);
                }
                if ended {
                    alive -= 1;
                    let ended_with = entry.1.borrow_mut().take();
                    // Safety: the task's interpreter was dropped at the end of
                    // its fiber, so nothing refers to this box any more.
                    unsafe { drop(Box::from_raw(entry.2)) };
                    if let Some(Err(e)) = ended_with {
                        failed = Some(e);
                        break;
                    }
                } else {
                    // It gave way. If it joined a channel's queue, that queue
                    // will wake it; if it gave way without blocking, it is
                    // still runnable.
                    if !self.chans.borrow().is_waiting((block, i)) {
                        runnable.push_back(i);
                    }
                    slots[i] = Some(entry);
                }
            }

            if let Some(e) = failed {
                break Err(e);
            }
            if alive == 0 {
                break Ok(Value::Unit);
            }
            // Take the wake-ups meant for this block, and leave any for the
            // block around it: a task of that one may have been woken before
            // this block started, and its own scheduler takes it when this
            // block's task gives way.
            {
                let mut guard = self.chans.borrow_mut();
                let all: VecDeque<Waiter> = std::mem::take(&mut guard.ready);
                for who in all {
                    if who.0 == block {
                        runnable.push_back(who.1);
                    } else {
                        guard.ready.push_back(who);
                    }
                }
            }
            if runnable.is_empty() {
                // Every live task is in a wait queue and no channel can move:
                // nothing will ever wake them ([E-DEADLOCK]).
                break Err(Flow::Panic(DEADLOCK.to_string()));
            }
        };
        {
            let mut guard = self.chans.borrow_mut();
            guard.running = outer_running;
            guard.close_block(block);
        }
        let mut live: Vec<_> = slots.into_iter().flatten().collect();

        // A task still suspended here is abandoned with the block: its fiber
        // and stack go, and so does its output box.
        for (run, _, out) in live.drain(..) {
            drop(run);
            // Safety: the fiber that could reach it is gone.
            unsafe { drop(Box::from_raw(out)) };
        }
        result
    }
}
