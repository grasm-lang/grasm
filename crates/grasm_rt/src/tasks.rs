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

use std::collections::VecDeque;
use std::sync::Mutex;

use crate::{dec, inc, none_val, rt_panic, some_of, RtPayload, RtVal, LOCAL, STACK_BUDGET};

struct Chan {
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
}

// Safety: the pointers are runtime values, whose counts are atomic, and the
// table is only reached under its mutex.
unsafe impl Send for Chans {}

static CHANS: Mutex<Chans> = Mutex::new(Chans {
    chans: Vec::new(),
    progress: 0,
});

const DEADLOCK: &str =
    "deadlock: every task is waiting on a channel, and nothing left can send or receive";

fn chans() -> std::sync::MutexGuard<'static, Chans> {
    CHANS.lock().unwrap_or_else(|p| p.into_inner())
}

/// A channel operation that cannot go ahead: inside a task, give way and try
/// again when resumed; outside every task there is nobody to give way to.
fn wait() {
    if grasm_fiber::in_fiber() {
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
    loop {
        {
            let mut guard = chans();
            let at = index(ch, &guard);
            let Chans { chans, progress } = &mut *guard;
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
                    if c.cap > 0 {
                        return;
                    }
                    ticket = Some(c.sent - 1);
                    continue;
                }
            }
        }
        wait();
    }
}

/// `chan_recv(ch)`: `Some(v)`, or `None` once closed and drained.
#[no_mangle]
pub extern "C" fn grasm_chan_recv(ch: i64) -> *mut RtVal {
    loop {
        {
            let mut guard = chans();
            let at = index(ch, &guard);
            let Chans { chans, progress } = &mut *guard;
            let c = &mut chans[at];
            if let Some(v) = c.buf.pop_front() {
                c.taken += 1;
                *progress += 1;
                return some_of(v);
            }
            if c.closed {
                return none_val();
            }
        }
        wait();
    }
}

/// `chan_close(ch)`.
#[no_mangle]
pub extern "C" fn grasm_chan_close(ch: i64) {
    let mut guard = chans();
    let at = index(ch, &guard);
    let Chans { chans, progress } = &mut *guard;
    let c = &mut chans[at];
    if c.closed {
        drop(guard);
        rt_panic("closing a channel that is already closed");
    }
    c.closed = true;
    *progress += 1;
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
    let body: extern "C" fn(*mut RtVal) = std::mem::transmute(code as usize);
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
    let mut live: Vec<(grasm_fiber::Fiber, usize)> = tasks
        .into_iter()
        .map(|f| {
            // Safety: see `run_task`; the value lives until the task ends.
            let fiber = grasm_fiber::Fiber::new(TASK_STACK, move || unsafe { run_task(f) });
            let floor = fiber.stack_bottom() + TASK_MARGIN;
            (fiber, floor)
        })
        .collect();
    loop {
        let before = chans().progress;
        let mut finished_any = false;
        let mut i = 0;
        while i < live.len() {
            // The stack check reads one floor per thread, and the stack it
            // guards is now this task's.
            let outer = LOCAL.with(|l| l.stack_floor.replace(live[i].1));
            let done = live[i].0.resume();
            LOCAL.with(|l| l.stack_floor.set(outer));
            if done {
                live.remove(i);
                finished_any = true;
            } else {
                i += 1;
            }
        }
        if live.is_empty() {
            return;
        }
        if !finished_any && chans().progress == before {
            rt_panic(DEADLOCK);
        }
    }
}
