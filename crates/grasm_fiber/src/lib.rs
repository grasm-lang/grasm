//! Stackful fibers: a stack of their own and a switch between them.
//!
//! The mechanism under concurrent tasks (docs/concurrency.md). A fiber is a
//! function running on a stack this crate allocated; [`Fiber::resume`] runs it
//! until it calls [`suspend`] or returns, and hands control back to whoever
//! resumed it. Everything stays on one OS thread, which is what lets the
//! interpreter's `Rc` values live in a fiber without ever crossing threads.
//!
//! The switch saves the registers the platform's calling convention says a
//! callee must preserve, and the stack pointer, and loads another fiber's.
//! Nothing else needs saving: a switch is an ordinary call as far as the code
//! on either side can tell, so everything caller-saved is already dead.
//!
//! aarch64 and x86_64 only; on other targets -- wasm32, which cannot switch
//! stacks at all -- [`supported`] is false and nothing here may be called.

#![no_std]
extern crate alloc;
#[macro_use]
extern crate grasm_sys;

use alloc::boxed::Box;
use alloc::vec::Vec;

use core::cell::Cell;

/// Whether this target can run fibers.
pub const fn supported() -> bool {
    cfg!(all(unix, any(target_arch = "aarch64", target_arch = "x86_64")))
}

/// Where a suspended fiber (or the thread that resumed one) left off: its
/// stack pointer, with its callee-saved registers stored on that stack.
#[repr(C)]
struct Context {
    sp: usize,
}

thread_local! {
    /// The fiber running on this thread, so `suspend` knows whose context
    /// to save and where to return. Null outside any fiber.
    static CURRENT: Cell<*mut Inner> = const { Cell::new(core::ptr::null_mut()) };
}

struct Inner {
    /// This fiber's saved context while it is suspended.
    own: Context,
    /// The resumer's saved context while this fiber runs.
    back: Context,
    /// Taken by the entry trampoline on the first resume.
    body: Option<Box<dyn FnOnce()>>,
    finished: bool,
    /// Was: a panic that escaped the body, carried back to the resumer.
    /// than unwound off the end of a stack it does not belong to.

    stack: Stack,
}

/// A function and the stack it runs on.
pub struct Fiber {
    inner: Box<Inner>,
}

impl Fiber {
    /// A fiber that will run `body` on a fresh stack of at least `stack_size`
    /// bytes, plus a guard page below it that faults rather than letting a
    /// deep recursion write into whatever lies beneath.
    ///
    /// # Panics
    /// When the target does not support fibers, or the stack cannot be mapped.
    pub fn new(stack_size: usize, body: impl FnOnce() + 'static) -> Fiber {
        // Safety: `'static` needs no promise.
        unsafe { Fiber::new_scoped(stack_size, body) }
    }

    /// [`Fiber::new`] for a body that borrows something shorter-lived.
    ///
    /// # Safety
    /// The caller runs the fiber to its end, or drops it, before anything the
    /// body borrows goes away. An interpreter's tasks borrow the program they
    /// run, and a `concurrent` block does not end until its tasks have -- or
    /// until one panics, in which case the rest are dropped unrun.
    pub unsafe fn new_scoped<'a>(stack_size: usize, body: impl FnOnce() + 'a) -> Fiber {
        let body: Box<dyn FnOnce() + 'a> = Box::new(body);
        // Safety: the caller's promise above is what makes the lifetime moot.
        let body: Box<dyn FnOnce() + 'static> = core::mem::transmute(body);
        assert!(supported(), "fibers are not supported on this target");
        let stack = Stack::new(stack_size);
        let mut inner = Box::new(Inner {
            own: Context { sp: 0 },
            back: Context { sp: 0 },
            body: Some(body),
            finished: false,
            stack,
        });
        let top = inner.stack.top();
        let arg = &mut *inner as *mut Inner as usize;
        // Safety: `top` is the aligned top of a stack we own and nothing else
        // uses; `prepare` writes only below it.
        inner.own.sp = unsafe { arch::prepare(top, entry as *const () as usize, arg) };
        Fiber { inner }
    }

    /// Runs the fiber until it suspends or finishes. Returns whether it has
    /// finished. A panic in the body exits the process where it happens,
    /// since nothing unwinds ([E-PANIC-EXIT]).
    ///
    /// # Panics
    /// If the fiber has already finished.
    pub fn resume(&mut self) -> bool {
        assert!(!self.inner.finished, "resuming a fiber that has finished");
        let me = &mut *self.inner as *mut Inner;
        let outer = CURRENT.with(|c| c.replace(me));
        // Safety: `back` is ours to fill; `own` holds a context saved by
        // `prepare` or by `suspend` on this fiber's own stack.
        unsafe { arch::switch(&mut (*me).back, &(*me).own) };
        CURRENT.with(|c| c.set(outer));
        self.inner.finished
    }

    /// The lowest address of this fiber's usable stack. A runtime that
    /// checks its own depth sets its floor from this while the fiber runs.
    pub fn stack_bottom(&self) -> usize {
        self.inner.stack.bottom()
    }

    pub fn is_finished(&self) -> bool {
        self.inner.finished
    }
}

/// Gives control back to whoever resumed the running fiber. Returns when the
/// fiber is resumed again.
///
/// # Panics
/// When called outside a fiber.
pub fn suspend() {
    let me = CURRENT.with(|c| c.get());
    assert!(!me.is_null(), "`suspend` outside a fiber");
    // Safety: `me` is the fiber running on this thread, set by `resume`.
    unsafe { arch::switch(&mut (*me).own, &(*me).back) };
}

/// Whether the caller is running inside a fiber.
pub fn in_fiber() -> bool {
    CURRENT.with(|c| !c.get().is_null())
}

/// Where every fiber starts: runs its body, records how it ended, and switches
/// back for the last time. It never returns -- there is nothing on the fiber's
/// stack to return to.
extern "C" fn entry(arg: usize) -> ! {
    let me = arg as *mut Inner;
    // Safety: `arg` is the fiber's own `Inner`, which outlives its stack.
    unsafe {
        let body = (*me).body.take().expect("a fiber starts once");
        // Called directly, not through `catch_unwind`. That existed so a
        // panic could not unwind across the context switch below -- the
        // unwinder would walk into a stack that is not a normal call chain,
        // which is undefined behaviour. In a `no_std` build nothing unwinds:
        // `#[panic_handler]` prints and exits, so the hazard cannot arise and
        // the fiber path now matches [E-PANIC-EXIT] rather than diverging
        // from it.
        body();
        (*me).finished = true;
        arch::switch(&mut (*me).own, &(*me).back);
    }
    unreachable!("a finished fiber was resumed");
}

// ---------------------------------------------------------------- stacks

struct Stack {
    base: *mut u8,
    len: usize,
}

#[cfg(unix)]
const PAGE: usize = 16 * 1024;

#[cfg(unix)]
mod sys {
    extern "C" {
        pub fn mmap(addr: *mut u8, len: usize, prot: i32, flags: i32, fd: i32, off: i64) -> *mut u8;
        pub fn munmap(addr: *mut u8, len: usize) -> i32;
        pub fn mprotect(addr: *mut u8, len: usize, prot: i32) -> i32;
    }
    pub const PROT_NONE: i32 = 0;
    pub const PROT_READ: i32 = 1;
    pub const PROT_WRITE: i32 = 2;
    pub const MAP_PRIVATE: i32 = 0x0002;
    #[cfg(target_os = "macos")]
    pub const MAP_ANON: i32 = 0x1000;
    #[cfg(not(target_os = "macos"))]
    pub const MAP_ANON: i32 = 0x20;
}

/// Stacks a finished fiber gave back, for the next fiber of the same size.
/// A program that starts a hundred thousand short tasks maps one stack, not a
/// hundred thousand: a task that ends without waiting leaves its stack here for
/// the next one. Bounded, so a burst of waiting tasks does not pin its peak.
const POOL_MAX: usize = 64;

/// A pooled stack keeps the pages its fiber touched, so only stacks small
/// enough that holding them is cheap are pooled. The interpreter's, sized for
/// its deep recursion budget, are unmapped as they finish.
const POOL_STACK_MAX: usize = 8 * 1024 * 1024 + 64 * 1024;

thread_local! {
    static POOL: core::cell::RefCell<Vec<Stack>> = const { core::cell::RefCell::new(Vec::new()) };
}

impl Stack {
    #[cfg(unix)]
    fn new(size: usize) -> Stack {
        let usable = size.div_ceil(PAGE).max(1) * PAGE;
        let len = usable + PAGE;
        let reused = POOL
            .try_with(|p| {
                let mut p = p.borrow_mut();
                let at = p.iter().rposition(|s| s.len == len)?;
                Some(p.swap_remove(at))
            })
            .ok()
            .flatten();
        if let Some(stack) = reused {
            return stack;
        }
        // Safety: an anonymous private mapping; the result is checked.
        unsafe {
            let base = sys::mmap(
                core::ptr::null_mut(),
                len,
                sys::PROT_READ | sys::PROT_WRITE,
                sys::MAP_PRIVATE | sys::MAP_ANON,
                -1,
                0,
            );
            assert!(
                !base.is_null() && base as isize != -1,
                "could not map a fiber stack of {len} bytes"
            );
            // The lowest page faults on touch: a stack grows down into it.
            let _ = sys::mprotect(base, PAGE, sys::PROT_NONE);
            Stack { base, len }
        }
    }

    #[cfg(not(unix))]
    fn new(_size: usize) -> Stack {
        unreachable!("fibers are not supported on this target")
    }

    /// The lowest address the fiber may use; the guard page lies below it.
    fn bottom(&self) -> usize {
        #[cfg(unix)]
        return self.base as usize + PAGE;
        #[cfg(not(unix))]
        return self.base as usize;
    }

    /// The top of the stack, 16-byte aligned, which every ABI here requires.
    fn top(&self) -> usize {
        (self.base as usize + self.len) & !15
    }
}

impl Drop for Stack {
    fn drop(&mut self) {
        if self.base.is_null() {
            return;
        }
        // Kept for the next fiber if there is room. The pool itself is being
        // torn down when the thread ends, and then the stack is unmapped.
        let (base, len) = (self.base, self.len);
        let kept = POOL
            .try_with(|p| match p.try_borrow_mut() {
                Ok(mut p) if p.len() < POOL_MAX && len <= POOL_STACK_MAX => {
                    p.push(Stack { base, len });
                    true
                }
                _ => false,
            })
            .unwrap_or(false);
        if kept {
            self.base = core::ptr::null_mut();
            return;
        }
        #[cfg(unix)]
        // Safety: exactly the mapping `new` made.
        unsafe {
            sys::munmap(self.base, self.len);
        }
    }
}

// ---------------------------------------------------------------- switch

#[cfg(target_arch = "aarch64")]
mod arch {
    /// Saves x19-x30 and d8-d15 on the current stack, stores its pointer in
    /// `from`, loads `to`'s, restores its registers and returns into it.
    #[unsafe(naked)]
    pub unsafe extern "C" fn switch(from: *mut super::Context, to: *const super::Context) {
        core::arch::naked_asm!(
            "sub sp, sp, #160",
            "stp x19, x20, [sp, #0]",
            "stp x21, x22, [sp, #16]",
            "stp x23, x24, [sp, #32]",
            "stp x25, x26, [sp, #48]",
            "stp x27, x28, [sp, #64]",
            "stp x29, x30, [sp, #80]",
            "stp d8, d9, [sp, #96]",
            "stp d10, d11, [sp, #112]",
            "stp d12, d13, [sp, #128]",
            "stp d14, d15, [sp, #144]",
            "mov x2, sp",
            "str x2, [x0]",
            "ldr x2, [x1]",
            "mov sp, x2",
            "ldp x19, x20, [sp, #0]",
            "ldp x21, x22, [sp, #16]",
            "ldp x23, x24, [sp, #32]",
            "ldp x25, x26, [sp, #48]",
            "ldp x27, x28, [sp, #64]",
            "ldp x29, x30, [sp, #80]",
            "ldp d8, d9, [sp, #96]",
            "ldp d10, d11, [sp, #112]",
            "ldp d12, d13, [sp, #128]",
            "ldp d14, d15, [sp, #144]",
            "add sp, sp, #160",
            "ret",
        );
    }

    /// The first thing a new fiber runs: calls `entry(arg)`, which never
    /// returns. `switch` restores x19 = entry and x20 = arg into it.
    #[unsafe(naked)]
    unsafe extern "C" fn trampoline() {
        core::arch::naked_asm!("mov x0, x20", "blr x19", "brk #1");
    }

    /// Lays out a saved context on a fresh stack so that the first `switch`
    /// into it "returns" into `trampoline` with the entry and its argument in
    /// hand. Returns the stack pointer to store.
    pub unsafe fn prepare(top: usize, entry: usize, arg: usize) -> usize {
        let sp = top - 160;
        let slots = sp as *mut usize;
        for i in 0..20 {
            *slots.add(i) = 0;
        }
        *slots.add(0) = entry; // x19
        *slots.add(1) = arg; // x20
        *slots.add(10) = 0; // x29: no caller frame
        *slots.add(11) = trampoline as *const () as usize; // x30: where `ret` goes
        sp
    }
}

#[cfg(target_arch = "x86_64")]
mod arch {
    /// Saves rbp, rbx, r12-r15 on the current stack, stores its pointer in
    /// `from`, loads `to`'s, restores its registers and returns into it.
    #[unsafe(naked)]
    pub unsafe extern "C" fn switch(from: *mut super::Context, to: *const super::Context) {
        core::arch::naked_asm!(
            "push rbp",
            "push rbx",
            "push r12",
            "push r13",
            "push r14",
            "push r15",
            "mov [rdi], rsp",
            "mov rsp, [rsi]",
            "pop r15",
            "pop r14",
            "pop r13",
            "pop r12",
            "pop rbx",
            "pop rbp",
            "ret",
        );
    }

    /// The first thing a new fiber runs: calls `entry(arg)` with r12 = entry
    /// and r13 = arg, restored by `switch`. Reached by `ret`, so the stack is
    /// 16-aligned here and the call leaves the callee its expected alignment.
    #[unsafe(naked)]
    unsafe extern "C" fn trampoline() {
        core::arch::naked_asm!("mov rdi, r13", "call r12", "ud2");
    }

    pub unsafe fn prepare(top: usize, entry: usize, arg: usize) -> usize {
        // Six saved registers and the return address. `ret` leaves the stack
        // pointer just above that address, and it has to be 16-aligned there
        // so that the trampoline's `call` gives `entry` the alignment every
        // function expects on entry: so the frame sits at `top - 72`, and
        // `ret` leaves `top - 16`.
        let sp = top - 72;
        let slots = sp as *mut usize;
        *slots.add(0) = 0; // r15
        *slots.add(1) = 0; // r14
        *slots.add(2) = arg; // r13
        *slots.add(3) = entry; // r12
        *slots.add(4) = 0; // rbx
        *slots.add(5) = 0; // rbp
        *slots.add(6) = trampoline as *const () as usize; // return address
        sp
    }
}

#[cfg(not(any(target_arch = "aarch64", target_arch = "x86_64")))]
mod arch {
    pub unsafe fn switch(_from: *mut super::Context, _to: *const super::Context) {
        unreachable!("fibers are not supported on this target")
    }
    pub unsafe fn prepare(_top: usize, _entry: usize, _arg: usize) -> usize {
        unreachable!("fibers are not supported on this target")
    }
}

#[cfg(all(test, unix, any(target_arch = "aarch64", target_arch = "x86_64")))]
mod tests {
    // `std`'s prelude is gone; tests need `alloc`'s names explicitly.
    #[allow(unused_imports)]
    use alloc::{format, string::String, string::ToString, vec, vec::Vec};
    use super::*;
    use alloc::rc::Rc;

    #[test]
    fn a_fiber_runs_to_its_end() {
        let log = Rc::new(core::cell::RefCell::new(Vec::new()));
        let l = log.clone();
        let mut f = Fiber::new(64 * 1024, move || l.borrow_mut().push(1));
        assert!(f.resume());
        assert_eq!(*log.borrow(), vec![1]);
    }

    #[test]
    fn suspend_and_resume_interleave() {
        let log = Rc::new(core::cell::RefCell::new(Vec::new()));
        let mk = |tag: i64| {
            let l = log.clone();
            Fiber::new(64 * 1024, move || {
                for i in 0..3 {
                    l.borrow_mut().push(tag * 10 + i);
                    suspend();
                }
            })
        };
        let mut a = mk(1);
        let mut b = mk(2);
        let mut done = (false, false);
        while !(done.0 && done.1) {
            if !done.0 {
                done.0 = a.resume();
            }
            if !done.1 {
                done.1 = b.resume();
            }
        }
        assert_eq!(*log.borrow(), vec![10, 20, 11, 21, 12, 22]);
    }

    /// Callee-saved registers survive a switch: a float and integer
    /// computation kept live across `suspend` in both fibers must come out
    /// right, which it would not if either set leaked between them.
    #[test]
    fn registers_survive_switches() {
        let out = Rc::new(core::cell::RefCell::new(Vec::new()));
        let mut fs: Vec<Fiber> = (0..64)
            .map(|k| {
                let o = out.clone();
                Fiber::new(64 * 1024, move || {
                    let mut acc_i: i64 = k;
                    let mut acc_f: f64 = k as f64;
                    for step in 0..50 {
                        acc_i = acc_i.wrapping_mul(31).wrapping_add(step);
                        acc_f = acc_f * 1.5 + step as f64;
                        suspend();
                    }
                    o.borrow_mut().push((k, acc_i, acc_f));
                })
            })
            .collect();
        let mut live = fs.len();
        while live > 0 {
            live = 0;
            for f in fs.iter_mut() {
                if !f.is_finished() && !f.resume() {
                    live += 1;
                }
            }
        }
        let got = out.borrow();
        assert_eq!(got.len(), 64);
        for &(k, i, f) in got.iter() {
            let (mut ei, mut ef) = (k, k as f64);
            for step in 0..50 {
                ei = ei.wrapping_mul(31).wrapping_add(step);
                ef = ef * 1.5 + step as f64;
            }
            assert_eq!((i, f), (ei, ef), "fiber {k}");
        }
    }

    // `a_panic_comes_back_to_the_resumer` was here, and is gone with the
    // mechanism it tested. It asserted that a panic inside a fiber body
    // arrived at `resume` as a caught payload, which is what `catch_unwind`
    // and `resume_unwind` were for.
    //
    // A `no_std` crate cannot unwind -- `#[panic_handler]` returns `!` -- so
    // a panic in a fiber body now prints and exits the process where it
    // happens ([E-PANIC-EXIT]). There is nothing left to come back.
    //
    // What that leaves uncovered, stated rather than quietly dropped: no test
    // here checks what a panicking fiber body does. Checking it would mean
    // running a program that exits 101 and reading its output, which belongs
    // in the gate scripts rather than a unit test.

    #[test]
    fn deep_recursion_fits_a_big_stack() {
        fn depth(n: u64) -> u64 {
            if n == 0 { 0 } else { 1 + depth(core::hint::black_box(n - 1)) }
        }
        let r = Rc::new(Cell::new(0));
        let r2 = r.clone();
        let mut f = Fiber::new(8 * 1024 * 1024, move || r2.set(depth(100_000)));
        assert!(f.resume());
        assert_eq!(r.get(), 100_000);
    }

    #[test]
    fn many_fibers() {
        let n = 10_000;
        let count = Rc::new(Cell::new(0));
        let mut fs: Vec<Fiber> = (0..n)
            .map(|_| {
                let c = count.clone();
                Fiber::new(16 * 1024, move || {
                    suspend();
                    c.set(c.get() + 1);
                })
            })
            .collect();
        for f in fs.iter_mut() {
            assert!(!f.resume());
        }
        for f in fs.iter_mut() {
            assert!(f.resume());
        }
        assert_eq!(count.get(), n);
    }
}
