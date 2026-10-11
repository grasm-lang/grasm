//! What a `no_std` program needs exactly one of.
//!
//! `grasm_rt` holds the runtime and `grasm_sys` the operating system, but
//! neither can declare a `#[global_allocator]` or a `#[panic_handler]`:
//! `grasm_interp` links both as rlibs and `std` already supplies those, which
//! would be a duplicate. So they live here, in a crate built only as a
//! staticlib and so only ever linked into a compiled Grasm program.
//!
//! `grasm build --native` links this archive; every `#[no_mangle]` entry point
//! Cranelift calls comes through it from `grasm_rt`.
#![no_std]

extern crate alloc;
// Linked for its `#[no_mangle]` entry points, which is the whole archive's
// purpose even though nothing here names one.
extern crate grasm_rt;

use core::alloc::{GlobalAlloc, Layout};

extern "C" {
    fn malloc(n: usize) -> *mut u8;
    fn free(at: *mut u8);
    fn realloc(at: *mut u8, n: usize) -> *mut u8;
    fn write(fd: i32, buf: *const u8, n: usize) -> isize;
    fn exit(code: i32) -> !;
}

/// The system allocator, which is what `std` would have given us. The runtime
/// pools and reuses its own boxes on top of this (`POOL_MAX`), so this is the
/// floor rather than the hot path.
struct System;

unsafe impl GlobalAlloc for System {
    unsafe fn alloc(&self, want: Layout) -> *mut u8 {
        // `malloc` guarantees alignment for any fundamental type, which is 16
        // bytes here. Anything stricter has to be asked for properly rather
        // than hoped for.
        if want.align() > 16 {
            return posix_aligned(want);
        }
        malloc(want.size())
    }

    unsafe fn dealloc(&self, at: *mut u8, _want: Layout) {
        free(at)
    }

    unsafe fn realloc(&self, at: *mut u8, was: Layout, size: usize) -> *mut u8 {
        if was.align() > 16 {
            // `realloc` cannot promise the stricter alignment, so this moves
            // the bytes by hand.
            let fresh = posix_aligned(Layout::from_size_align_unchecked(size, was.align()));
            if !fresh.is_null() {
                core::ptr::copy_nonoverlapping(at, fresh, was.size().min(size));
                free(at);
            }
            return fresh;
        }
        realloc(at, size)
    }
}

unsafe fn posix_aligned(want: Layout) -> *mut u8 {
    extern "C" {
        fn posix_memalign(out: *mut *mut u8, align: usize, size: usize) -> i32;
    }
    let mut at: *mut u8 = core::ptr::null_mut();
    if posix_memalign(&mut at, want.align(), want.size()) != 0 {
        return core::ptr::null_mut();
    }
    at
}

#[global_allocator]
static ALLOCATOR: System = System;

/// Prints and exits, which is what [E-PANIC-EXIT] says a panic does. Nothing
/// unwinds, so this never returns and nothing above it gets to clean up --
/// the reason `grasm_fiber` no longer needs to catch anything.
///
/// Written with `write(2)` rather than through `grasm_sys::io`: a panic may be
/// the allocator failing, and a buffered writer would allocate to report it.
#[panic_handler]
fn panicked(info: &core::panic::PanicInfo<'_>) -> ! {
    unsafe {
        let lead = b"panic: ";
        write(2, lead.as_ptr(), lead.len());
        // The message, if it is a plain string. Formatting it would allocate.
        if let Some(text) = info.message().as_str() {
            write(2, text.as_ptr(), text.len());
        } else {
            let unknown = b"(message needs formatting)";
            write(2, unknown.as_ptr(), unknown.len());
        }
        write(2, b"\n".as_ptr(), 1);
        exit(101)
    }
}
