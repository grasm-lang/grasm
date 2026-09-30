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
    /// Tier 2's slab, while it is in transit between interpreters. A `ref`
    /// (an actor's state is one) names a slot in *the* slab, so there can be
    /// only one; each task's interpreter holds it while it runs and puts it
    /// here when it gives way, and the scheduler does the same around each
    /// resume. Only one interpreter runs at a time, so it is never needed in
    /// two places at once.
    pub(crate) slab_hold: Option<Slab>,
}

pub(crate) type Slab = (Vec<(Option<Value>, u32)>, Vec<u32>);

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
    /// The order this round visits `n` tasks in.
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
    buf: VecDeque<Value>,
    /// How many values it holds before a send waits; 0 is a rendezvous.
    cap: usize,
    closed: bool,
    /// Values sent and values received so far, for a rendezvous: a send
    /// returns once its own value has been taken.
    sent: u64,
    taken: u64,
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
        let slab = (std::mem::take(&mut self.slab), std::mem::take(&mut self.slab_free));
        self.chans.borrow_mut().slab_hold = Some(slab);
    }

    /// Takes the slab the last interpreter to run left.
    pub(crate) fn take_slab(&mut self) {
        if let Some((slab, free)) = self.chans.borrow_mut().slab_hold.take() {
            self.slab = slab;
            self.slab_free = free;
        }
    }

    fn wait(&mut self) -> Result<(), Flow> {
        match if self.in_task { Wait::Yield } else { Wait::Deadlock } {
            Wait::Yield => {
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
                let Chans { chans, progress, .. } = &mut *guard;
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
                        if c.cap > 0 {
                            return Ok(Value::Unit);
                        }
                        ticket = Some(c.sent - 1);
                        continue;
                    }
                }
            }
            self.wait()?;
        }
    }

    /// `chan_recv(ch)`: `Some(v)`, or `None` once closed and drained.
    pub(crate) fn chan_recv(&mut self, ch: &Value) -> EResult {
        let id = chan_id(ch)?;
        loop {
            {
                let mut guard = self.chans.borrow_mut();
                let Chans { chans, progress, .. } = &mut *guard;
                let Some(c) = chans.get_mut(id) else {
                    return Err(Flow::Panic("not a channel".into()));
                };
                if let Some(v) = c.buf.pop_front() {
                    c.taken += 1;
                    *progress += 1;
                    return Ok(crate::interp::some(v));
                }
                if c.closed {
                    return Ok(crate::interp::none());
                }
            }
            self.wait()?;
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
            return Err(Flow::Panic("`chan_select` of no channels would wait forever".into()));
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
            self.wait()?;
        }
    }

    /// `chan_close(ch)`.
    pub(crate) fn chan_close(&mut self, ch: &Value) -> EResult {
        let id = chan_id(ch)?;
        let mut guard = self.chans.borrow_mut();
        let Chans { chans, progress, .. } = &mut *guard;
        let Some(c) = chans.get_mut(id) else {
            return Err(Flow::Panic("not a channel".into()));
        };
        if c.closed {
            return Err(Flow::Panic("closing a channel that is already closed".into()));
        }
        c.closed = true;
        *progress += 1;
        Ok(Value::Unit)
    }

    /// Runs a `concurrent` block's tasks to completion, in the fixed order
    /// described at the top of this file.
    pub(crate) fn run_tasks(
        &mut self,
        tasks: Vec<Task>,
    ) -> EResult {
        if tasks.is_empty() {
            return Ok(Value::Unit);
        }
        if !grasm_fiber::supported() {
            return Err(Flow::Panic(
                "`concurrent` needs a platform that can switch stacks, which this one cannot".into(),
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
        let result = loop {
            let before = self.chans.borrow().progress;
            let mut finished_any = false;
            let mut failed: Option<Flow> = None;
            let order = self.chans.borrow_mut().round_order(live.len());
            // Each round takes the tasks out and puts back the ones still
            // waiting, in the order they ran. Removing finished ones from the
            // middle of the list cost a shift of everything after each, which
            // is quadratic in the number of tasks.
            let mut slots: Vec<Option<_>> = live.drain(..).map(Some).collect();
            for i in order {
                let mut entry = slots[i].take().expect("each index once");
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
                    finished_any = true;
                    let ended_with = entry.1.borrow_mut().take();
                    // Safety: the task's interpreter was dropped at the end of
                    // its fiber, so nothing refers to this box any more.
                    unsafe { drop(Box::from_raw(entry.2)) };
                    if let Some(Err(e)) = ended_with {
                        failed = Some(e);
                        break;
                    }
                } else {
                    live.push(entry);
                }
            }
            // Tasks a failure cut the round short before: abandoned below.
            live.extend(slots.into_iter().flatten());
            if let Some(e) = failed {
                break Err(e);
            }
            if live.is_empty() {
                break Ok(Value::Unit);
            }
            if !finished_any && self.chans.borrow().progress == before {
                break Err(Flow::Panic(DEADLOCK.to_string()));
            }
        };
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
