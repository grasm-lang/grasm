//! The operating system, for every crate that needs it.
//!
//! This exists because `grasm_rt` is not the only crate a compiled program
//! links. `grasm_journal` and `grasm_fiber` link too, and `grasm_rt` depends
//! on both, so neither can call back into it -- but all three need the same
//! `fs`, `io`, `process` and `env`. Putting that layer underneath all of them
//! is what lets every one of them be `no_std`, which is what the 135 KB of
//! `std`'s backtrace symboliser costs if even one of them is not
//! (docs/binary-size.md).
#![no_std]
#![allow(clippy::missing_safety_doc)]
extern crate alloc;

/// The handful of libc calls `wasm32-unknown-unknown` does not have.
///
/// That target has no filesystem, no environment and no process to exit, and
/// `std` reports the same by failing every one of these. So they fail here
/// too, rather than each caller growing a `cfg`: `grasm_journal` asks for a
/// file when `GRASM_RECORD` is set, that ask returns an error, and the
/// journal is simply off in the browser -- which is what it already was.
#[cfg(target_arch = "wasm32")]
pub mod absent {
    pub unsafe fn sys_open(_path: *const core::ffi::c_char, _flags: i32, _mode: u32) -> i32 {
        -1
    }
    pub unsafe fn close(_fd: i32) -> i32 {
        0
    }
    pub unsafe fn sys_read(_fd: i32, _buf: *mut u8, _n: usize) -> isize {
        -1
    }
    pub unsafe fn sys_write(_fd: i32, _buf: *const u8, _n: usize) -> isize {
        -1
    }
    pub unsafe fn lseek(_fd: i32, _at: i64, _whence: i32) -> i64 {
        -1
    }
    pub unsafe fn fsync(_fd: i32) -> i32 {
        -1
    }
    pub unsafe fn sys_rename(_from: *const u8, _to: *const u8) -> i32 {
        -1
    }
    pub unsafe fn unlink(_path: *const u8) -> i32 {
        -1
    }
    pub unsafe fn mkdir(_path: *const u8, _mode: u32) -> i32 {
        -1
    }
    pub unsafe fn access(_path: *const u8, _mode: i32) -> i32 {
        -1
    }
    pub unsafe fn opendir(_path: *const u8) -> *mut u8 {
        core::ptr::null_mut()
    }
    pub unsafe fn closedir(_dir: *mut u8) -> i32 {
        0
    }
    pub unsafe fn getenv(_name: *const u8) -> *const u8 {
        core::ptr::null()
    }
    /// A wasm module cannot end a process, so it traps instead, which is the
    /// nearest thing the platform has.
    pub unsafe fn libc_exit(code: i32) -> ! {
        panic!("exit({code}) in a wasm module, which cannot end a process")
    }
}

/// The bytes of a NUL-terminated C string, not counting the NUL.
///
/// Declaring `strlen` is flagged as shadowing the one `std` itself links, and
/// the loop is four lines.
///
/// # Safety
/// `at` must be non-null and point at a NUL within the allocation it belongs
/// to.
pub unsafe fn c_bytes(at: *const u8) -> &'static [u8] {
    let mut len = 0;
    while *at.add(len) != 0 {
        len += 1;
    }
    core::slice::from_raw_parts(at, len)
}

/// `thread_local!` without `std`.
///
/// `std`'s macro is the only thing in this runtime that cannot be renamed to
/// `core` or `alloc`, because thread-local storage needs the operating
/// system. POSIX already provides it, and this runtime already calls POSIX
/// directly for `poll`, `kqueue` and `mmap`, so this follows that pattern
/// rather than taking a dependency.
///
/// macOS and Linux are the targets (`crates/grasm_rt` names no other), and
/// both have pthreads.
pub mod tls {
    #[cfg(not(target_arch = "wasm32"))]
    use alloc::boxed::Box;
    #[cfg(not(target_arch = "wasm32"))]
    use core::sync::atomic::{AtomicU64, Ordering};

    /// The value is gone because its thread is ending. On `wasm32` it never
    /// is, because the thread that would end is the only one there is.
    #[derive(Debug)]
    pub struct Gone;

    /// `wasm32-unknown-unknown` has no pthreads and no second thread
    /// (scripts/wasmtest.sh: "no threads, `par_map` runs on one core"), so a
    /// thread-local there is just a value, and asking the system for one
    /// would be asking for `pthread_key_create` -- which is exactly how the
    /// wasm build broke when this crate first appeared.
    #[cfg(target_arch = "wasm32")]
    pub struct Local<T: 'static> {
        slot: core::cell::UnsafeCell<Option<T>>,
        init: fn() -> T,
    }

    // Safety: `wasm32-unknown-unknown` is single-threaded, so nothing can
    // reach this concurrently. This is the assumption the whole branch rests
    // on, and it is the target's, not a hope about how callers behave.
    #[cfg(target_arch = "wasm32")]
    unsafe impl<T: 'static> Sync for Local<T> {}

    #[cfg(target_arch = "wasm32")]
    impl<T: 'static> Local<T> {
        pub const fn new(init: fn() -> T) -> Local<T> {
            Local {
                slot: core::cell::UnsafeCell::new(None),
                init,
            }
        }

        pub fn with<R>(&self, f: impl FnOnce(&T) -> R) -> R {
            match self.try_with(f) {
                Ok(r) => r,
                Err(_) => unreachable!("a wasm32 thread-local is never gone"),
            }
        }

        pub fn try_with<R>(&self, f: impl FnOnce(&T) -> R) -> Result<R, Gone> {
            let at = self.slot.get();
            // Safety: made before it is borrowed, and the initialiser runs
            // to completion before anything holds a reference -- so a
            // re-entrant `try_with` finds a value and never writes over one
            // that is borrowed.
            unsafe {
                if (*at).is_none() {
                    let made = (self.init)();
                    *at = Some(made);
                }
                Ok(f((*at).as_ref().unwrap()))
            }
        }
    }

    #[cfg(target_os = "macos")]
    pub type Key = u64;
    #[cfg(all(not(target_os = "macos"), not(target_arch = "wasm32")))]
    pub type Key = u32;

    #[cfg(not(target_arch = "wasm32"))]
    extern "C" {
        fn pthread_key_create(k: *mut Key, dtor: Option<unsafe extern "C" fn(*mut u8)>) -> i32;
        fn pthread_key_delete(k: Key) -> i32;
        fn pthread_getspecific(k: Key) -> *mut u8;
        fn pthread_setspecific(k: Key, v: *const u8) -> i32;
    }

    /// One value per thread, made on first use and dropped when the thread
    /// ends -- which is what `pthread_key_create`'s destructor is for, and
    /// what keeps this equivalent to the macro it replaces.
    #[cfg(not(target_arch = "wasm32"))]
    pub struct Local<T: 'static> {
        /// The pthread key plus one, so that zero can mean "not yet made".
        key: AtomicU64,
        init: fn() -> T,
    }

    // Safety: the `T` never lives in this struct. It lives in thread-local
    // storage, where exactly one thread can reach it, so `T` need not be
    // `Sync` for the handle to be shared -- the same reasoning `std`'s own
    // `LocalKey` uses.
    #[cfg(not(target_arch = "wasm32"))]
    unsafe impl<T: 'static> Sync for Local<T> {}

    #[cfg(not(target_arch = "wasm32"))]
    impl<T: 'static> Local<T> {
        pub const fn new(init: fn() -> T) -> Local<T> {
            Local { key: AtomicU64::new(0), init }
        }

        /// The key, made once. Two threads racing both make one; the loser
        /// deletes its own and takes the winner's, so no key leaks and every
        /// thread agrees which one holds the value.
        fn key(&self) -> Key {
            let seen = self.key.load(Ordering::Acquire);
            if seen != 0 {
                return (seen - 1) as Key;
            }
            let mut fresh: Key = 0;
            unsafe {
                pthread_key_create(&mut fresh, Some(drop_in_place::<T>));
            }
            match self.key.compare_exchange(
                0,
                fresh as u64 + 1,
                Ordering::AcqRel,
                Ordering::Acquire,
            ) {
                Ok(_) => fresh,
                Err(won) => {
                    unsafe { pthread_key_delete(fresh) };
                    (won - 1) as Key
                }
            }
        }

        pub fn with<R>(&self, f: impl FnOnce(&T) -> R) -> R {
            match self.try_with(f) {
                Ok(r) => r,
                Err(_) => panic!("a thread-local was used after its thread began ending"),
            }
        }

        /// `Err` once this thread's value has been dropped, which happens
        /// while the thread is ending. A caller that runs then -- returning a
        /// stack to a pool, say -- has to know the pool is gone rather than
        /// quietly building a new one that nothing will ever free.
        pub fn try_with<R>(&self, f: impl FnOnce(&T) -> R) -> Result<R, Gone> {
            let k = self.key();
            let mut at = unsafe { pthread_getspecific(k) } as *mut Slot<T>;
            if at.is_null() {
                at = Box::into_raw(Box::new(Slot {
                    alive: true,
                    value: core::mem::ManuallyDrop::new((self.init)()),
                }));
                unsafe { pthread_setspecific(k, at as *const u8) };
            }
            // Safety: just made or previously made by this thread, and only
            // this thread can reach it.
            let slot = unsafe { &*at };
            if !slot.alive {
                return Err(Gone);
            }
            Ok(f(&slot.value))
        }
    }

    /// The value plus whether it is still there. The flag has to live beside
    /// the value rather than in the descriptor, because the destructor below
    /// is handed the value and not the key, so it has nowhere else to record
    /// that it ran.
    #[cfg(not(target_arch = "wasm32"))]
    struct Slot<T> {
        alive: bool,
        value: core::mem::ManuallyDrop<T>,
    }

    /// Run by pthreads when a thread that used this key ends.
    ///
    /// The `Slot` is deliberately **not** freed. POSIX re-runs this
    /// destructor while the key's value stays non-null -- up to
    /// `PTHREAD_DESTRUCTOR_ITERATIONS`, four in practice -- and the
    /// destructor is not given the key, so it cannot clear it. Freeing the
    /// box would therefore hand the next call a dangling pointer. Keeping it
    /// and flipping `alive` makes the repeat calls harmless and lets
    /// `try_with` report the value as gone. The cost is one small box per
    /// thread that touched this key, which is bounded by the number of
    /// threads; the value's own allocations are released here as usual.
    #[cfg(not(target_arch = "wasm32"))]
    unsafe extern "C" fn drop_in_place<T>(at: *mut u8) {
        if at.is_null() {
            return;
        }
        let slot = &mut *(at as *mut Slot<T>);
        if slot.alive {
            slot.alive = false;
            core::mem::ManuallyDrop::drop(&mut slot.value);
        }
    }
}

/// `std::io`, as much of it as this runtime uses.
///
/// The shapes are deliberately the ones `std` has -- `stdout().lock()`, a
/// `Write` trait, `Error::last_os_error()`, `ErrorKind` -- so that converting
/// the runtime is a change of import and not a rewrite of every call site.
///
/// **Buffering is the part that matters.** `std`'s stdout is buffered and
/// flushed when the process ends. Raw `write(2)` is neither, and Grasm's four
/// engines have to agree byte for byte, so dropping the buffer would change
/// how output interleaves with stderr and show up as a difftest failure
/// rather than a compile error. So: buffered here too, and `sys::exit` flushes
/// before it exits, which is what `std` was doing for us.
pub mod io {
    #[cfg(not(target_arch = "wasm32"))]
    use crate::c_bytes;
    use crate::sync::{Guard, Mutex};
    use alloc::string::String;
    use alloc::vec::Vec;
    use core::fmt;

    #[cfg(target_os = "macos")]
    extern "C" {
        #[link_name = "__error"]
        fn errno_at() -> *mut i32;
    }
    #[cfg(all(not(target_os = "macos"), not(target_arch = "wasm32")))]
    extern "C" {
        #[link_name = "__errno_location"]
        fn errno_at() -> *mut i32;
    }

    #[cfg(not(target_arch = "wasm32"))]
    extern "C" {
        #[link_name = "write"]
        fn sys_write(fd: i32, buf: *const core::ffi::c_void, n: usize) -> isize;
        #[link_name = "read"]
        fn sys_read(fd: i32, buf: *mut core::ffi::c_void, n: usize) -> isize;
        fn strerror(code: i32) -> *const u8;
    }

    pub const EINTR: i32 = 4;
    #[cfg(target_os = "macos")]
    pub const EAGAIN: i32 = 35;
    #[cfg(not(target_os = "macos"))]
    pub const EAGAIN: i32 = 11;
    pub const ENOENT: i32 = 2;
    #[cfg(target_os = "macos")]
    pub const EILSEQ: i32 = 92;
    #[cfg(not(target_os = "macos"))]
    pub const EILSEQ: i32 = 84;

    #[cfg(not(target_arch = "wasm32"))]
    pub fn errno() -> i32 {
        unsafe { *errno_at() }
    }

    /// There is no errno on `wasm32-unknown-unknown`, and nothing here can
    /// fail in a way that would set one: the calls that would are all absent
    /// on that target.
    #[cfg(target_arch = "wasm32")]
    pub fn errno() -> i32 {
        0
    }

    #[derive(Clone, Copy, PartialEq, Eq, Debug)]
    pub enum ErrorKind {
        NotFound,
        Interrupted,
        WouldBlock,
        Other,
    }

    #[derive(Clone, Copy, Debug)]
    pub struct Error(i32);

    pub type Result<T> = core::result::Result<T, Error>;

    impl Error {
        pub fn last_os_error() -> Error {
            Error(errno())
        }

        pub fn from_raw_os_error(code: i32) -> Error {
            Error(code)
        }

        pub fn raw_os_error(&self) -> Option<i32> {
            Some(self.0)
        }

        pub fn kind(&self) -> ErrorKind {
            match self.0 {
                ENOENT => ErrorKind::NotFound,
                EINTR => ErrorKind::Interrupted,
                EAGAIN => ErrorKind::WouldBlock,
                _ => ErrorKind::Other,
            }
        }
    }

    /// The text `std` prints for an errno, which keeps error messages
    /// identical across this conversion. `wasm32-unknown-unknown` has no
    /// `strerror`, and no call here can set an errno on that target anyway.
    #[cfg(not(target_arch = "wasm32"))]
    fn errno_text(code: i32) -> Option<&'static str> {
        let at = unsafe { strerror(code) };
        if at.is_null() {
            return None;
        }
        core::str::from_utf8(unsafe { c_bytes(at) }).ok()
    }

    #[cfg(target_arch = "wasm32")]
    fn errno_text(_code: i32) -> Option<&'static str> {
        None
    }

    impl fmt::Display for Error {
        fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
            match errno_text(self.0) {
                Some(text) => f.write_str(text),
                None => write!(f, "os error {}", self.0),
            }
        }
    }

    /// `wasm32-unknown-unknown` has no file descriptors. `std`'s stderr on
    /// that target accepts and discards, which is what `eprintln!` did here
    /// before this crate existed, so this does the same rather than
    /// inventing a failure. A Grasm program's own output does not come this
    /// way: the wasm engine hands it to the host through `grasm_run`.
    #[cfg(target_arch = "wasm32")]
    fn write_fully(_fd: i32, _bytes: &[u8]) -> Result<()> {
        Ok(())
    }

    #[cfg(not(target_arch = "wasm32"))]
    fn write_fully(fd: i32, bytes: &[u8]) -> Result<()> {
        let mut done = 0;
        while done < bytes.len() {
            let n = unsafe { sys_write(fd, bytes[done..].as_ptr() as *const core::ffi::c_void, bytes.len() - done) };
            if n > 0 {
                done += n as usize;
            } else if n < 0 && errno() == EINTR {
                continue;
            } else {
                return Err(Error::last_os_error());
            }
        }
        Ok(())
    }

    pub trait Write {
        fn write_all(&mut self, bytes: &[u8]) -> Result<()>;
        fn flush(&mut self) -> Result<()>;

        /// What `write!` and `writeln!` call.
        fn write_fmt(&mut self, args: fmt::Arguments<'_>) -> Result<()> {
            // `core::fmt` hands out `&str` pieces and reports only that
            // something failed, so the errno is carried out of the adapter by
            // hand rather than lost.
            struct Adapt<'a, W: Write + ?Sized>(&'a mut W, Result<()>);
            impl<W: Write + ?Sized> fmt::Write for Adapt<'_, W> {
                fn write_str(&mut self, piece: &str) -> fmt::Result {
                    match self.0.write_all(piece.as_bytes()) {
                        Ok(()) => Ok(()),
                        Err(e) => {
                            self.1 = Err(e);
                            Err(fmt::Error)
                        }
                    }
                }
            }
            let mut adapt = Adapt(self, Ok(()));
            match fmt::write(&mut adapt, args) {
                Ok(()) => Ok(()),
                Err(_) => adapt.1,
            }
        }
    }

    /// 8 KiB, which is what `std` uses for a `BufWriter`.
    const BUF: usize = 8192;

    static OUT: Mutex<Vec<u8>> = Mutex::new(Vec::new());
    static ERR: Mutex<Vec<u8>> = Mutex::new(Vec::new());

    pub struct Stream {
        fd: i32,
        held: Guard<'static, Vec<u8>>,
    }

    impl Write for Stream {
        fn write_all(&mut self, bytes: &[u8]) -> Result<()> {
            // A write larger than the buffer goes straight out rather than
            // growing the buffer to hold it.
            if self.held.len() + bytes.len() > BUF {
                let pending = core::mem::take(&mut *self.held);
                if !pending.is_empty() {
                    write_fully(self.fd, &pending)?;
                }
                if bytes.len() >= BUF {
                    return write_fully(self.fd, bytes);
                }
            }
            self.held.extend_from_slice(bytes);
            Ok(())
        }

        fn flush(&mut self) -> Result<()> {
            if self.held.is_empty() {
                return Ok(());
            }
            let pending = core::mem::take(&mut *self.held);
            write_fully(self.fd, &pending)
        }
    }

    /// Stderr is unbuffered in `std`, and is here too: a message written just
    /// before a crash has to be out before the crash.
    impl Drop for Stream {
        fn drop(&mut self) {
            if self.fd == 2 {
                let _ = self.flush();
            }
        }
    }

    pub struct Handle {
        fd: i32,
    }

    impl Handle {
        pub fn lock(&self) -> Stream {
            Stream {
                fd: self.fd,
                held: if self.fd == 1 { OUT.lock() } else { ERR.lock() },
            }
        }

        pub fn flush(&self) -> Result<()> {
            self.lock().flush()
        }
    }

    pub fn stdout() -> Handle {
        Handle { fd: 1 }
    }

    pub fn stderr() -> Handle {
        Handle { fd: 2 }
    }

    /// Called by `sys::exit`, because nothing else will: `std` flushed stdout
    /// when the process ended and raw file descriptors do not.
    pub fn flush_all() {
        let _ = stdout().flush();
        let _ = stderr().flush();
    }

    /// One line from stdin, newline included, as `BufRead::read_line` gives
    /// it. Unbuffered -- the runtime reads lines rarely and a buffer here
    /// would have to be shared with anything else reading fd 0.
    /// No stdin on `wasm32-unknown-unknown`: end of input, immediately,
    /// which is what reading fd 0 there amounts to.
    #[cfg(target_arch = "wasm32")]
    pub fn read_line(_into: &mut String) -> Result<usize> {
        Ok(0)
    }

    #[cfg(not(target_arch = "wasm32"))]
    pub fn read_line(into: &mut String) -> Result<usize> {
        let mut got = 0;
        let mut byte = [0u8; 1];
        // Bytes first, then one UTF-8 check. Pushing `byte as char` would
        // read each byte as a codepoint, which turns every multi-byte
        // character into mojibake -- and would only show up as a difftest
        // failure on non-ASCII input.
        let mut held: Vec<u8> = Vec::new();
        loop {
            let n = unsafe { sys_read(0, byte.as_mut_ptr() as *mut core::ffi::c_void, 1) };
            if n == 0 {
                break;
            }
            if n < 0 {
                if errno() == EINTR {
                    continue;
                }
                return Err(Error::last_os_error());
            }
            got += 1;
            held.push(byte[0]);
            if byte[0] == b'\n' {
                break;
            }
        }
        match core::str::from_utf8(&held) {
            Ok(text) => {
                into.push_str(text);
                Ok(got)
            }
            // Invalid UTF-8, which `std` reports as `InvalidData`.
            Err(_) => Err(Error::from_raw_os_error(EILSEQ)),
        }
    }
}

/// Subprocesses: `fork`, `execvp`, `waitpid`, and pipes.
///
/// Two hazards decide the shape of this module, and both are the kind that
/// works in testing and fails in production.
///
/// **Nothing is allocated between `fork` and `exec`.** This runtime has
/// threads -- `par_map` and the network workers -- and `fork` gives the child
/// one thread while leaving every lock as it was. If another thread held
/// `malloc`'s lock at the moment of the fork, the child deadlocks the first
/// time it allocates. So every C string and pointer array is built before the
/// fork, and the child calls only `dup2`, `close`, `execvp` and `_exit`, all
/// of which are async-signal-safe.
///
/// **Two pipes are drained together, not one after the other.** Reading
/// stdout to the end before touching stderr deadlocks as soon as the child
/// writes more to stderr than its pipe buffer holds -- 64 KiB on Linux. The
/// child blocks writing, the parent blocks reading, and neither moves. So
/// both descriptors go through one `poll` loop.
pub mod process {
    use crate::io::{errno, Error, Result, EINTR};
    use alloc::string::String;
    use alloc::vec::Vec;

    const POLLIN: i16 = 0x0001;

    #[repr(C)]
    struct PollFd {
        fd: i32,
        wanted: i16,
        got: i16,
    }

    extern "C" {
        fn pipe(into: *mut i32) -> i32;
        fn fork() -> i32;
        fn dup2(from: i32, to: i32) -> i32;
        fn close(fd: i32) -> i32;
        fn execvp(file: *const u8, argv: *const *const u8) -> i32;
        fn _exit(code: i32) -> !;
        fn waitpid(pid: i32, status: *mut i32, flags: i32) -> i32;
        #[link_name = "read"]
        fn sys_read(fd: i32, buf: *mut core::ffi::c_void, n: usize) -> isize;
        #[link_name = "write"]
        fn sys_write(fd: i32, buf: *const core::ffi::c_void, n: usize) -> isize;
        fn poll(fds: *mut PollFd, n: u32, timeout: i32) -> i32;
        // Variadic too, for the same reason as `open` above.
        fn fcntl(fd: i32, cmd: i32, ...) -> i32;
        fn kill(pid: i32, signal: i32) -> i32;
    }

    const F_SETFD: i32 = 2;
    const FD_CLOEXEC: i32 = 1;
    pub const SIGKILL: i32 = 9;

    pub fn stop(pid: i32) {
        unsafe { kill(pid, SIGKILL) };
    }

    /// The argument vector, owned, so that the child between `fork` and
    /// `exec` only reads pointers that already exist.
    struct Argv {
        _owned: Vec<Vec<u8>>,
        pointers: Vec<*const u8>,
    }

    fn argv_of(program: &str, args: &[String]) -> Argv {
        let mut owned = Vec::with_capacity(args.len() + 1);
        let mut one = Vec::with_capacity(program.len() + 1);
        one.extend_from_slice(program.as_bytes());
        one.push(0);
        owned.push(one);
        for arg in args {
            let mut next = Vec::with_capacity(arg.len() + 1);
            next.extend_from_slice(arg.as_bytes());
            next.push(0);
            owned.push(next);
        }
        let mut pointers: Vec<*const u8> =
            owned.iter().map(|a| a.as_ptr()).collect();
        // `execvp` reads until it sees a null, so the array must end with one.
        pointers.push(core::ptr::null());
        Argv { _owned: owned, pointers }
    }

    pub struct Pipe {
        pub reader: i32,
        pub writer: i32,
    }

    /// Both ends are marked close-on-exec, which is what makes a pipeline
    /// terminate. A child inherits every open descriptor, so without this
    /// the second stage would hold the first stage's writer open and the
    /// first stage's reader would never see end of file -- a pipeline that
    /// hangs instead of finishing. `dup2` clears the flag on the descriptor
    /// it makes, so the child still gets its own 0, 1 and 2.
    pub fn new_pipe() -> Result<Pipe> {
        let mut ends = [0i32; 2];
        if unsafe { pipe(ends.as_mut_ptr()) } < 0 {
            return Err(Error::last_os_error());
        }
        for fd in ends {
            unsafe { fcntl(fd, F_SETFD, FD_CLOEXEC) };
        }
        Ok(Pipe { reader: ends[0], writer: ends[1] })
    }

    pub fn shut(fd: i32) {
        if fd >= 0 {
            unsafe { close(fd) };
        }
    }

    /// Forks and execs. `stdin`, `stdout` and `stderr` replace the child's
    /// own when given. Every other descriptor is already close-on-exec, so
    /// the child keeps nothing else.
    pub fn start(
        program: &str,
        args: &[String],
        stdin: Option<i32>,
        stdout: Option<i32>,
        stderr: Option<i32>,
    ) -> Result<i32> {
        // Before the fork, while allocating is still safe.
        let argv = argv_of(program, args);
        let pid = unsafe { fork() };
        if pid < 0 {
            return Err(Error::last_os_error());
        }
        if pid == 0 {
            unsafe {
                if let Some(fd) = stdin {
                    dup2(fd, 0);
                }
                if let Some(fd) = stdout {
                    dup2(fd, 1);
                }
                if let Some(fd) = stderr {
                    dup2(fd, 2);
                }
                execvp(argv.pointers[0], argv.pointers.as_ptr());
                // Only reached if `execvp` failed. 127 is what a shell
                // reports for a command it could not run.
                _exit(127)
            }
        }
        Ok(pid)
    }

    /// The exit code, or -1 for a signal -- which is what the caller already
    /// treats as "no status a process can exit with".
    pub fn wait_for(pid: i32) -> Result<i64> {
        let mut status = 0i32;
        loop {
            let r = unsafe { waitpid(pid, &mut status, 0) };
            if r >= 0 {
                break;
            }
            if errno() != EINTR {
                return Err(Error::last_os_error());
            }
        }
        // The low seven bits hold the signal, the next eight the exit code,
        // which is what `WIFEXITED` and `WEXITSTATUS` read.
        if status & 0x7f == 0 {
            Ok(((status >> 8) & 0xff) as i64)
        } else {
            Ok(-1)
        }
    }

    pub fn write_and_close(fd: i32, bytes: &[u8]) {
        let mut done = 0;
        while done < bytes.len() {
            let n = unsafe {
                sys_write(fd, bytes[done..].as_ptr() as *const core::ffi::c_void, bytes.len() - done)
            };
            if n > 0 {
                done += n as usize;
            } else if n < 0 && errno() == EINTR {
                continue;
            } else {
                // A child that exited without reading leaves nowhere to put
                // the rest, which is not this function's problem to report.
                break;
            }
        }
        shut(fd);
    }

    /// Both descriptors in one `poll` loop, so neither pipe can fill while
    /// the other is being read. Each is closed as it reaches end of file.
    pub fn drain_both(mut out: i32, mut err: i32) -> (Vec<u8>, Vec<u8>) {
        let mut got_out = Vec::new();
        let mut got_err = Vec::new();
        let mut chunk = [0u8; 8192];
        while out >= 0 || err >= 0 {
            let mut watching: Vec<PollFd> = Vec::with_capacity(2);
            if out >= 0 {
                watching.push(PollFd { fd: out, wanted: POLLIN, got: 0 });
            }
            if err >= 0 {
                watching.push(PollFd { fd: err, wanted: POLLIN, got: 0 });
            }
            let r = unsafe {
                poll(watching.as_mut_ptr(), watching.len() as u32, -1)
            };
            if r < 0 {
                if errno() == EINTR {
                    continue;
                }
                break;
            }
            for watched in &watching {
                if watched.got == 0 {
                    continue;
                }
                let n = unsafe {
                    sys_read(watched.fd, chunk.as_mut_ptr() as *mut core::ffi::c_void, chunk.len())
                };
                let into = if watched.fd == out {
                    &mut got_out
                } else {
                    &mut got_err
                };
                if n > 0 {
                    into.extend_from_slice(&chunk[..n as usize]);
                } else if n == 0 || errno() != EINTR {
                    // End of file, or an error that will not clear. Either
                    // way this descriptor is finished.
                    shut(watched.fd);
                    if watched.fd == out {
                        out = -1;
                    } else {
                        err = -1;
                    }
                }
            }
        }
        (got_out, got_err)
    }

    /// `Command::new(p).args(a).output()`.
    pub fn run(program: &str, args: &[String]) -> Result<(i64, Vec<u8>, Vec<u8>)> {
        let out = new_pipe()?;
        let err = match new_pipe() {
            Ok(p) => p,
            Err(e) => {
                shut(out.reader);
                shut(out.writer);
                return Err(e);
            }
        };
        let pid = start(
            program,
            args,
            None,
            Some(out.writer),
            Some(err.writer),
        );
        // The parent holds no writer: while it does, the reader never sees
        // end of file even after the child exits.
        shut(out.writer);
        shut(err.writer);
        let pid = match pid {
            Ok(pid) => pid,
            Err(e) => {
                shut(out.reader);
                shut(err.reader);
                return Err(e);
            }
        };
        let (got_out, got_err) = drain_both(out.reader, err.reader);
        let code = wait_for(pid)?;
        Ok((code, got_out, got_err))
    }
}

/// `std::thread`, over pthreads.
///
/// The piece that matters is `scope`. `par_map` spawns closures holding `&mut`
/// into slices on the caller's stack, so **every thread must be joined before
/// `scope` returns** or those borrows outlive the frame they point into. The
/// joining lives in a `Drop`, not at the end of the function body, so it also
/// happens on a path out that is not the normal one.
///
/// A `no_std` crate cannot unwind -- `#[panic_handler]` returns `!` -- so
/// there is no such path today. The guard does not depend on that staying
/// true.
pub mod thread {
    use alloc::boxed::Box;
    use alloc::string::String;
    use alloc::vec::Vec;
    use core::cell::UnsafeCell;
    use core::marker::PhantomData;
    use core::num::NonZeroUsize;
    use core::time::Duration;

    #[cfg(target_os = "macos")]
    pub type Handle = *mut u8;
    #[cfg(not(target_os = "macos"))]
    pub type Handle = usize;

    #[repr(C, align(16))]
    struct RawAttr([u8; 128]);

    #[repr(C)]
    struct TimeSpec {
        secs: i64,
        nanos: i64,
    }

    extern "C" {
        fn pthread_create(
            out: *mut Handle,
            attr: *const u8,
            body: extern "C" fn(*mut u8) -> *mut u8,
            arg: *mut u8,
        ) -> i32;
        fn pthread_join(handle: Handle, out: *mut *mut u8) -> i32;
        fn pthread_detach(handle: Handle) -> i32;
        fn pthread_attr_init(attr: *mut u8) -> i32;
        fn pthread_attr_destroy(attr: *mut u8) -> i32;
        fn pthread_attr_setstacksize(attr: *mut u8, size: usize) -> i32;
        fn nanosleep(wait: *const TimeSpec, left: *mut TimeSpec) -> i32;
        fn sysconf(name: i32) -> i64;
    }

    #[cfg(target_os = "macos")]
    extern "C" {
        fn pthread_setname_np(name: *const u8) -> i32;
    }
    #[cfg(not(target_os = "macos"))]
    extern "C" {
        fn pthread_self() -> Handle;
        fn pthread_setname_np(who: Handle, name: *const u8) -> i32;
    }

    #[cfg(target_os = "macos")]
    const SC_NPROCESSORS_ONLN: i32 = 58;
    #[cfg(not(target_os = "macos"))]
    const SC_NPROCESSORS_ONLN: i32 = 84;

    /// What `pthread_create` is given: the closure, plus the name to set from
    /// inside the new thread. The name is set there because macOS's
    /// `pthread_setname_np` only names the calling thread.
    struct Start {
        body: Box<dyn FnOnce() + Send>,
        name: Option<String>,
    }

    extern "C" fn run(arg: *mut u8) -> *mut u8 {
        // Safety: `arg` is the `Box<Start>` leaked by `spawn_raw`, handed to
        // exactly one thread, and taken back here exactly once.
        let start = unsafe { Box::from_raw(arg as *mut Start) };
        if let Some(name) = &start.name {
            let mut owned = Vec::with_capacity(name.len() + 1);
            owned.extend_from_slice(name.as_bytes());
            owned.push(0);
            #[cfg(target_os = "macos")]
            unsafe {
                pthread_setname_np(owned.as_ptr())
            };
            #[cfg(not(target_os = "macos"))]
            unsafe {
                pthread_setname_np(pthread_self(), owned.as_ptr())
            };
        }
        (start.body)();
        core::ptr::null_mut()
    }

    fn spawn_raw(
        body: Box<dyn FnOnce() + Send>,
        name: Option<String>,
        stack: Option<usize>,
    ) -> Option<Handle> {
        let start = Box::into_raw(Box::new(Start { body, name })) as *mut u8;
        let mut handle: Handle = unsafe { core::mem::zeroed() };
        let made = match stack {
            Some(size) => unsafe {
                let mut attr = RawAttr([0; 128]);
                let at = &mut attr as *mut RawAttr as *mut u8;
                pthread_attr_init(at);
                pthread_attr_setstacksize(at, size);
                let r = pthread_create(&mut handle, at, run, start);
                pthread_attr_destroy(at);
                r
            },
            None => unsafe {
                pthread_create(&mut handle, core::ptr::null(), run, start)
            },
        };
        if made != 0 {
            // The thread was never made, so nothing will take the box back.
            drop(unsafe { Box::from_raw(start as *mut Start) });
            return None;
        }
        Some(handle)
    }

    pub struct JoinHandle {
        handle: Handle,
        joined: bool,
    }

    /// Why a thread could not be made or joined, as pthreads reported it.
    /// `Result<_, ()>` would say only that something went wrong, and the
    /// caller has a message to write.
    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    pub struct ThreadError(pub i32);

    impl JoinHandle {
        pub fn join(mut self) -> Result<(), ThreadError> {
            self.joined = true;
            let mut out = core::ptr::null_mut();
            let r = unsafe { pthread_join(self.handle, &mut out) };
            if r == 0 {
                Ok(())
            } else {
                Err(ThreadError(r))
            }
        }
    }

    /// An unjoined thread is detached rather than leaked, so the system can
    /// reclaim its stack when it ends.
    impl Drop for JoinHandle {
        fn drop(&mut self) {
            if !self.joined {
                unsafe { pthread_detach(self.handle) };
            }
        }
    }

    pub fn spawn<F: FnOnce() + Send + 'static>(body: F) -> Option<JoinHandle> {
        spawn_raw(Box::new(body), None, None).map(|handle| JoinHandle {
            handle,
            joined: false,
        })
    }

    pub fn sleep(how_long: Duration) {
        let wait = TimeSpec {
            secs: how_long.as_secs() as i64,
            nanos: how_long.subsec_nanos() as i64,
        };
        // A signal can cut the sleep short; `left` says how much remains, so
        // the loop finishes the wait rather than returning early.
        let mut left = TimeSpec { secs: 0, nanos: 0 };
        let mut at = &wait as *const TimeSpec;
        while unsafe { nanosleep(at, &mut left) } != 0 {
            if crate::io::errno() != crate::io::EINTR {
                return;
            }
            at = &left as *const TimeSpec;
        }
    }

    pub fn available_parallelism() -> Option<NonZeroUsize> {
        let n = unsafe { sysconf(SC_NPROCESSORS_ONLN) };
        if n > 0 {
            NonZeroUsize::new(n as usize)
        } else {
            None
        }
    }

    /// `PhantomData<&'scope mut &'scope ()>` makes `'scope` invariant, which
    /// is what stops a borrow taken inside the closure from being passed out
    /// of it. A covariant marker would let the lifetime shrink and the
    /// guarantee with it.
    pub struct Scope<'scope> {
        running: UnsafeCell<Vec<Handle>>,
        _scope: PhantomData<&'scope mut &'scope ()>,
    }

    impl<'scope> Scope<'scope> {
        pub fn spawn<F: FnOnce() + Send + 'scope>(&'scope self, body: F) {
            self.spawn_named(body, None, None);
        }

        fn spawn_named<F: FnOnce() + Send + 'scope>(
            &'scope self,
            body: F,
            name: Option<String>,
            stack: Option<usize>,
        ) -> bool {
            // Safety: the closure borrows for `'scope`, and `scope` joins
            // every thread before `'scope` ends, so the borrow cannot outlive
            // what it points at. Erasing the lifetime is what `pthread_create`
            // needs and what that join pays for.
            let erased: Box<dyn FnOnce() + Send> =
                unsafe { core::mem::transmute(Box::new(body) as Box<dyn FnOnce() + Send + 'scope>) };
            match spawn_raw(erased, name, stack) {
                Some(handle) => {
                    // Safety: `Scope` is not `Sync`, so only the thread that
                    // made it reaches this.
                    unsafe { (*self.running.get()).push(handle) };
                    true
                }
                None => false,
            }
        }
    }

    /// Joins in `Drop` so it happens on every path out of `scope`, not only
    /// the one that reaches the end of the body.
    struct JoinAtEnd<'a, 'scope>(&'a Scope<'scope>);

    impl Drop for JoinAtEnd<'_, '_> {
        fn drop(&mut self) {
            let running = unsafe { &mut *self.0.running.get() };
            for handle in running.drain(..) {
                let mut out = core::ptr::null_mut();
                unsafe { pthread_join(handle, &mut out) };
            }
        }
    }

    pub fn scope<'env, R>(body: impl FnOnce(&'env Scope<'env>) -> R) -> R {
        let held = Scope {
            running: UnsafeCell::new(Vec::new()),
            _scope: PhantomData,
        };
        // Safety: `held` outlives `join`, which outlives the call below, so
        // the reference handed to the body is valid for as long as the body
        // can use it -- and every thread it spawned is joined when `join` is
        // dropped, before this function returns.
        let borrowed: &'env Scope<'env> = unsafe { core::mem::transmute(&held) };
        let _join = JoinAtEnd(&held);
        body(borrowed)
    }

    pub struct Builder {
        name: Option<String>,
        stack: Option<usize>,
    }

    impl Builder {
        pub fn new() -> Builder {
            Builder { name: None, stack: None }
        }

        pub fn name(mut self, name: String) -> Builder {
            self.name = Some(name);
            self
        }

        pub fn stack_size(mut self, size: usize) -> Builder {
            self.stack = Some(size);
            self
        }

        pub fn spawn_scoped<'scope, F: FnOnce() + Send + 'scope>(
            self,
            scope: &'scope Scope<'scope>,
            body: F,
        ) -> Result<(), ThreadError> {
            if scope.spawn_named(body, self.name, self.stack) {
                Ok(())
            } else {
                // `pthread_create`'s own code, which `spawn_raw` does not
                // keep; errno carries the same reason.
                Err(ThreadError(crate::io::errno()))
            }
        }
    }

    impl Default for Builder {
        fn default() -> Builder {
            Builder::new()
        }
    }
}

/// `std::net`, narrowed to the TCP and UDP this runtime offers.
///
/// Addresses are resolved with `getaddrinfo`, because the host comes from
/// Grasm code and may be a name rather than a literal address -- which is
/// also what `std` does underneath.
pub mod net {
    use crate::io::{errno, Error, Result, EINTR};
    use alloc::string::String;
    use alloc::vec::Vec;

    pub const AF_INET: i32 = 2;
    pub const SOCK_STREAM: i32 = 1;
    pub const SOCK_DGRAM: i32 = 2;

    #[cfg(target_os = "macos")]
    pub const SOL_SOCKET: i32 = 0xffff;
    #[cfg(not(target_os = "macos"))]
    pub const SOL_SOCKET: i32 = 1;
    #[cfg(target_os = "macos")]
    pub const SO_REUSEADDR: i32 = 0x0004;
    #[cfg(not(target_os = "macos"))]
    pub const SO_REUSEADDR: i32 = 2;
    pub const IPPROTO_TCP: i32 = 6;
    #[cfg(target_os = "macos")]
    pub const TCP_NODELAY: i32 = 0x01;
    #[cfg(not(target_os = "macos"))]
    pub const TCP_NODELAY: i32 = 1;

    #[cfg(target_os = "macos")]
    const O_NONBLOCK: i32 = 0x0004;
    #[cfg(not(target_os = "macos"))]
    const O_NONBLOCK: i32 = 0o4000;
    const F_GETFL: i32 = 3;
    const F_SETFL: i32 = 4;

    // `addrinfo` is the trap in this module: macOS puts `canonname` before
    // `addr` and Linux puts it after. Reading the wrong one gives a pointer
    // to a string where a socket address is expected.
    #[cfg(target_os = "macos")]
    #[repr(C)]
    struct AddrInfo {
        flags: i32,
        family: i32,
        socktype: i32,
        protocol: i32,
        addrlen: u32,
        canonname: *const u8,
        addr: *const u8,
        next: *const AddrInfo,
    }
    #[cfg(not(target_os = "macos"))]
    #[repr(C)]
    struct AddrInfo {
        flags: i32,
        family: i32,
        socktype: i32,
        protocol: i32,
        addrlen: u32,
        addr: *const u8,
        canonname: *const u8,
        next: *const AddrInfo,
    }

    extern "C" {
        fn socket(domain: i32, kind: i32, protocol: i32) -> i32;
        fn bind(fd: i32, addr: *const u8, len: u32) -> i32;
        fn listen(fd: i32, backlog: i32) -> i32;
        fn accept(fd: i32, addr: *mut u8, len: *mut u32) -> i32;
        fn connect(fd: i32, addr: *const u8, len: u32) -> i32;
        fn send(fd: i32, buf: *const u8, n: usize, flags: i32) -> isize;
        fn recv(fd: i32, buf: *mut u8, n: usize, flags: i32) -> isize;
        fn sendto(fd: i32, buf: *const u8, n: usize, flags: i32, addr: *const u8, alen: u32) -> isize;
        fn recvfrom(fd: i32, buf: *mut u8, n: usize, flags: i32, addr: *mut u8, alen: *mut u32) -> isize;
        fn setsockopt(fd: i32, level: i32, name: i32, value: *const u8, len: u32) -> i32;
        fn shutdown(fd: i32, how: i32) -> i32;
        fn close(fd: i32) -> i32;
        // Variadic too, for the same reason as `open` above.
        fn fcntl(fd: i32, cmd: i32, ...) -> i32;
        fn getaddrinfo(host: *const u8, service: *const u8, hint: *const AddrInfo, out: *mut *const AddrInfo) -> i32;
        fn freeaddrinfo(at: *const AddrInfo);
        fn getsockname(fd: i32, addr: *mut u8, len: *mut u32) -> i32;
        fn getpeername(fd: i32, addr: *mut u8, len: *mut u32) -> i32;
        fn inet_ntop(family: i32, addr: *const u8, out: *mut u8, len: u32) -> *const u8;
    }

    // 30 on macOS, 10 on Linux. One of the few `AF_` values that differs,
    // and getting it wrong does not fail: an IPv6 address would not match,
    // so `parts` would read it as IPv4 -- four bytes from offset 4 instead of
    // sixteen from offset 8, and the wrong family handed to `inet_ntop`.
    // Wrong text for a peer's address, reported as success.
    #[cfg(target_os = "macos")]
    pub const AF_INET6: i32 = 30;
    #[cfg(not(target_os = "macos"))]
    pub const AF_INET6: i32 = 10;

    impl Addr {
        /// The port, and the address as text.
        ///
        /// `sin_port` sits at offset 2 in network byte order for IPv4 and
        /// IPv6 alike, and on both platforms -- macOS splits the family field
        /// into a length and a family byte, which happens to leave the port
        /// where Linux has it. The family itself does move, so that is read
        /// per platform.
        pub fn parts(&self) -> (String, u16) {
            if self.bytes.len() < 4 {
                return (String::new(), 0);
            }
            let port = u16::from_be_bytes([self.bytes[2], self.bytes[3]]);
            #[cfg(target_os = "macos")]
            let family = self.bytes[1] as i32;
            #[cfg(not(target_os = "macos"))]
            let family = u16::from_ne_bytes([self.bytes[0], self.bytes[1]]) as i32;
            // The address itself starts after the flow info in IPv6 and
            // straight after the port in IPv4.
            let (at, want) = match family {
                f if f == AF_INET6 => (8, 46),
                _ => (4, 16),
            };
            if self.bytes.len() < at {
                return (String::new(), port);
            }
            let mut text = [0u8; 46];
            let shown = unsafe {
                inet_ntop(
                    family,
                    self.bytes[at..].as_ptr(),
                    text.as_mut_ptr(),
                    want as u32,
                )
            };
            if shown.is_null() {
                return (String::new(), port);
            }
            let len = text.iter().position(|&b| b == 0).unwrap_or(0);
            (
                core::str::from_utf8(&text[..len])
                    .unwrap_or("")
                    .into(),
                port,
            )
        }
    }

    impl Addr {
        pub fn port(&self) -> u16 {
            self.parts().1
        }

        pub fn ip_text(&self) -> String {
            self.parts().0
        }
    }

    fn name_of(fd: i32, peer: bool) -> Result<Addr> {
        // Large enough for `sockaddr_in6`, which is the biggest this runtime
        // can produce.
        let mut bytes = [0u8; 128];
        let mut len = bytes.len() as u32;
        let r = unsafe {
            if peer {
                getpeername(fd, bytes.as_mut_ptr(), &mut len)
            } else {
                getsockname(fd, bytes.as_mut_ptr(), &mut len)
            }
        };
        if r < 0 {
            return Err(Error::last_os_error());
        }
        Ok(Addr {
            bytes: bytes[..len as usize].to_vec(),
            family: 0,
        })
    }

    fn c_str(text: &str) -> Vec<u8> {
        let mut owned = Vec::with_capacity(text.len() + 1);
        owned.extend_from_slice(text.as_bytes());
        owned.push(0);
        owned
    }

    /// The resolved bytes of one socket address, copied out before
    /// `freeaddrinfo` takes the list back.
    pub struct Addr {
        bytes: Vec<u8>,
        family: i32,
    }

    pub fn resolve(host: &str, port: u16) -> Result<Addr> {
        let (h, p) = (c_str(host), c_str(&alloc::format!("{port}")));
        let hint = AddrInfo {
            flags: 0,
            family: 0,
            socktype: SOCK_STREAM,
            protocol: 0,
            addrlen: 0,
            addr: core::ptr::null(),
            canonname: core::ptr::null(),
            next: core::ptr::null(),
        };
        let mut found: *const AddrInfo = core::ptr::null();
        let r = unsafe { getaddrinfo(h.as_ptr(), p.as_ptr(), &hint, &mut found) };
        if r != 0 || found.is_null() {
            // `getaddrinfo` reports through its own return value, not errno.
            return Err(Error::from_raw_os_error(crate::io::ENOENT));
        }
        let addr = unsafe {
            let len = (*found).addrlen as usize;
            Addr {
                bytes: core::slice::from_raw_parts((*found).addr, len).to_vec(),
                family: (*found).family,
            }
        };
        unsafe { freeaddrinfo(found) };
        Ok(addr)
    }

    fn check(r: i32) -> Result<()> {
        if r < 0 {
            Err(Error::last_os_error())
        } else {
            Ok(())
        }
    }

    fn set_opt(fd: i32, level: i32, name: i32, on: bool) -> Result<()> {
        let value: i32 = on as i32;
        check(unsafe {
            setsockopt(
                fd,
                level,
                name,
                &value as *const i32 as *const u8,
                core::mem::size_of::<i32>() as u32,
            )
        })
    }

    pub struct Socket {
        fd: i32,
    }

    impl Socket {
        pub fn as_raw_fd(&self) -> i32 {
            self.fd
        }

        /// Takes the descriptor out without closing it, for the caller that
        /// stores raw descriptors of its own.
        pub fn into_raw_fd(self) -> i32 {
            let fd = self.fd;
            core::mem::forget(self);
            fd
        }

        pub unsafe fn from_raw_fd(fd: i32) -> Socket {
            Socket { fd }
        }

        pub fn set_nonblocking(&self, on: bool) -> Result<()> {
            let flags = unsafe { fcntl(self.fd, F_GETFL, 0) };
            if flags < 0 {
                return Err(Error::last_os_error());
            }
            let want = if on {
                flags | O_NONBLOCK
            } else {
                flags & !O_NONBLOCK
            };
            check(unsafe { fcntl(self.fd, F_SETFL, want) })
        }

        pub fn set_nodelay(&self, on: bool) -> Result<()> {
            set_opt(self.fd, IPPROTO_TCP, TCP_NODELAY, on)
        }

        pub fn read(&self, into: &mut [u8]) -> Result<usize> {
            loop {
                let n = unsafe { recv(self.fd, into.as_mut_ptr(), into.len(), 0) };
                if n >= 0 {
                    return Ok(n as usize);
                }
                if errno() != EINTR {
                    return Err(Error::last_os_error());
                }
            }
        }

        pub fn write_all(&self, bytes: &[u8]) -> Result<()> {
            let mut done = 0;
            while done < bytes.len() {
                let n = unsafe {
                    send(self.fd, bytes[done..].as_ptr(), bytes.len() - done, 0)
                };
                if n > 0 {
                    done += n as usize;
                } else if n < 0 && errno() == EINTR {
                    continue;
                } else {
                    return Err(Error::last_os_error());
                }
            }
            Ok(())
        }

        pub fn local_addr(&self) -> Result<Addr> {
            name_of(self.fd, false)
        }

        pub fn peer_addr(&self) -> Result<Addr> {
            name_of(self.fd, true)
        }

        pub fn shutdown_both(&self) -> Result<()> {
            check(unsafe { shutdown(self.fd, 2) })
        }

        pub fn accept(&self) -> Result<Socket> {
            loop {
                let fd = unsafe {
                    accept(self.fd, core::ptr::null_mut(), core::ptr::null_mut())
                };
                if fd >= 0 {
                    return Ok(Socket { fd });
                }
                if errno() != EINTR {
                    return Err(Error::last_os_error());
                }
            }
        }

        pub fn send_to(&self, bytes: &[u8], to: &Addr) -> Result<usize> {
            let n = unsafe {
                sendto(
                    self.fd,
                    bytes.as_ptr(),
                    bytes.len(),
                    0,
                    to.bytes.as_ptr(),
                    to.bytes.len() as u32,
                )
            };
            if n < 0 {
                Err(Error::last_os_error())
            } else {
                Ok(n as usize)
            }
        }

        /// The sender comes back with the bytes, because replying to a
        /// datagram needs it and asking again would ask about a different
        /// one.
        pub fn recv_from(&self, into: &mut [u8]) -> Result<(usize, Addr)> {
            let mut from = [0u8; 128];
            let mut flen = from.len() as u32;
            loop {
                let n = unsafe {
                    recvfrom(
                        self.fd,
                        into.as_mut_ptr(),
                        into.len(),
                        0,
                        from.as_mut_ptr(),
                        &mut flen,
                    )
                };
                if n >= 0 {
                    return Ok((
                        n as usize,
                        Addr {
                            bytes: from[..flen as usize].to_vec(),
                            family: 0,
                        },
                    ));
                }
                if errno() != EINTR {
                    return Err(Error::last_os_error());
                }
            }
        }
    }

    impl Drop for Socket {
        fn drop(&mut self) {
            unsafe { close(self.fd) };
        }
    }

    fn open_socket(kind: i32, family: i32) -> Result<Socket> {
        let fd = unsafe { socket(family, kind, 0) };
        if fd < 0 {
            Err(Error::last_os_error())
        } else {
            Ok(Socket { fd })
        }
    }

    /// `SO_REUSEADDR` before `bind`, as `std` sets it: without it a listener
    /// cannot be restarted until the kernel releases the port.
    pub fn listen_on(host: &str, port: u16) -> Result<Socket> {
        let addr = resolve(host, port)?;
        let sock = open_socket(SOCK_STREAM, addr.family)?;
        set_opt(sock.fd, SOL_SOCKET, SO_REUSEADDR, true)?;
        check(unsafe { bind(sock.fd, addr.bytes.as_ptr(), addr.bytes.len() as u32) })?;
        check(unsafe { listen(sock.fd, 1024) })?;
        Ok(sock)
    }

    pub fn connect_to(host: &str, port: u16) -> Result<Socket> {
        let addr = resolve(host, port)?;
        let sock = open_socket(SOCK_STREAM, addr.family)?;
        loop {
            let r = unsafe {
                connect(sock.fd, addr.bytes.as_ptr(), addr.bytes.len() as u32)
            };
            if r >= 0 {
                return Ok(sock);
            }
            if errno() != EINTR {
                return Err(Error::last_os_error());
            }
        }
    }

    pub fn udp_on(host: &str, port: u16) -> Result<Socket> {
        let addr = resolve(host, port)?;
        let sock = open_socket(SOCK_DGRAM, addr.family)?;
        check(unsafe { bind(sock.fd, addr.bytes.as_ptr(), addr.bytes.len() as u32) })?;
        Ok(sock)
    }
}

/// `std::fs`, narrowed to what this runtime calls.
///
/// Only the ten entry points the runtime uses, and `read_dir` gives names
/// rather than an iterator of entries because names are all the one caller
/// wants. Narrowing on purpose: every line here is unsafe code in the crate
/// every compiled program links.
pub mod fs {
    use crate::io::{errno, Error, Result, EINTR};
    use alloc::string::String;
    use alloc::vec::Vec;

    pub const O_RDONLY: i32 = 0;
    pub const O_WRONLY: i32 = 1;
    pub const O_RDWR: i32 = 2;
    #[cfg(target_os = "macos")]
    pub const O_CREAT: i32 = 0x0200;
    #[cfg(not(target_os = "macos"))]
    pub const O_CREAT: i32 = 0o100;
    #[cfg(target_os = "macos")]
    pub const O_TRUNC: i32 = 0x0400;
    #[cfg(not(target_os = "macos"))]
    pub const O_TRUNC: i32 = 0o1000;
    #[cfg(target_os = "macos")]
    pub const O_APPEND: i32 = 0x0008;
    #[cfg(not(target_os = "macos"))]
    pub const O_APPEND: i32 = 0o2000;

    pub const SEEK_SET: i32 = 0;
    pub const SEEK_CUR: i32 = 1;
    pub const SEEK_END: i32 = 2;

    const EEXIST: i32 = 17;
    const F_OK: i32 = 0;

    #[cfg(target_arch = "wasm32")]
    use crate::absent::*;

    #[cfg(not(target_arch = "wasm32"))]
    extern "C" {
        // Variadic, and that is not a detail: `int open(const char *, int,
        // ...)` takes its mode as a variadic argument, which arm64 Apple's
        // ABI passes **on the stack** where a fixed parameter goes in a
        // register. Declared with a fixed third parameter, the mode went to
        // x2 and `open` read whatever was on the stack -- so a created file
        // got a garbage mode, and the second `append_bytes` to it failed with
        // "Permission denied" while the first succeeded.
        #[link_name = "open"]
        fn sys_open(path: *const core::ffi::c_char, flags: i32, ...) -> i32;
        fn close(fd: i32) -> i32;
        // Named apart from this module's own `read`, `write` and `rename`,
        // which would otherwise be duplicate definitions.
        #[link_name = "read"]
        fn sys_read(fd: i32, buf: *mut core::ffi::c_void, n: usize) -> isize;
        #[link_name = "write"]
        fn sys_write(fd: i32, buf: *const core::ffi::c_void, n: usize) -> isize;
        fn lseek(fd: i32, at: i64, whence: i32) -> i64;
        fn fsync(fd: i32) -> i32;
        #[link_name = "rename"]
        fn sys_rename(from: *const u8, to: *const u8) -> i32;
        fn unlink(path: *const u8) -> i32;
        fn mkdir(path: *const u8, mode: u32) -> i32;
        fn access(path: *const u8, mode: i32) -> i32;
        fn opendir(path: *const u8) -> *mut u8;
        fn closedir(dir: *mut u8) -> i32;
    }

    // The 64-bit-inode `readdir`, which is spelled differently on the two
    // macOS architectures and caught three apps failing to link when this
    // was written per-OS instead of per-architecture:
    //
    //   - arm64: plain `readdir` already returns a 64-bit inode, and no
    //     `$INODE64` alias exists -- asking for one does not link.
    //   - x86_64: plain `readdir` is the old 32-bit-inode version kept for
    //     compatibility, so the suffixed symbol is the one that matches
    //     `Dirent` below.
    #[cfg(all(target_os = "macos", target_arch = "aarch64"))]
    extern "C" {
        fn readdir(dir: *mut u8) -> *const Dirent;
    }
    #[cfg(all(target_os = "macos", not(target_arch = "aarch64")))]
    extern "C" {
        #[link_name = "readdir$INODE64"]
        fn readdir(dir: *mut u8) -> *const Dirent;
    }
    #[cfg(all(not(target_os = "macos"), not(target_arch = "wasm32")))]
    extern "C" {
        #[link_name = "readdir64"]
        fn readdir(dir: *mut u8) -> *const Dirent;
    }

    // Laid out per platform so the compiler works out where `name` starts.
    // Hardcoding that offset is how this kind of code goes wrong.
    #[cfg(all(target_os = "macos", not(target_arch = "wasm32")))]
    #[repr(C)]
    struct Dirent {
        ino: u64,
        seekoff: u64,
        reclen: u16,
        namlen: u16,
        kind: u8,
        name: [u8; 1024],
    }
    #[cfg(all(not(target_os = "macos"), not(target_arch = "wasm32")))]
    #[repr(C)]
    struct Dirent {
        ino: u64,
        off: i64,
        reclen: u16,
        kind: u8,
        name: [u8; 256],
    }

    /// `open` and friends want a NUL-terminated path and Rust strings are not.
    fn c_path(path: &str) -> Vec<u8> {
        let mut owned = Vec::with_capacity(path.len() + 1);
        owned.extend_from_slice(path.as_bytes());
        owned.push(0);
        owned
    }

    fn check(r: i32) -> Result<()> {
        if r < 0 {
            Err(Error::last_os_error())
        } else {
            Ok(())
        }
    }

    pub struct File {
        fd: i32,
    }

    impl File {
        pub fn open(path: &str) -> Result<File> {
            File::with_flags(path, O_RDONLY, 0)
        }

        pub fn create(path: &str) -> Result<File> {
            File::with_flags(path, O_WRONLY | O_CREAT | O_TRUNC, 0o666)
        }

        pub fn with_flags(path: &str, flags: i32, mode: u32) -> Result<File> {
            let at = c_path(path);
            loop {
                let fd = unsafe { sys_open(at.as_ptr() as *const core::ffi::c_char, flags, mode) };
                if fd >= 0 {
                    return Ok(File { fd });
                }
                if errno() != EINTR {
                    return Err(Error::last_os_error());
                }
            }
        }

        pub fn as_raw_fd(&self) -> i32 {
            self.fd
        }

        pub fn read(&mut self, into: &mut [u8]) -> Result<usize> {
            loop {
                let n = unsafe { sys_read(self.fd, into.as_mut_ptr() as *mut core::ffi::c_void, into.len()) };
                if n >= 0 {
                    return Ok(n as usize);
                }
                if errno() != EINTR {
                    return Err(Error::last_os_error());
                }
            }
        }

        /// Reads until end of file, which is what `read_to_end` means: a
        /// single `read` is allowed to return less than asked for, and
        /// stopping at the first short read is the classic bug here.
        pub fn read_to_end(&mut self, into: &mut Vec<u8>) -> Result<usize> {
            let mut got = 0;
            let mut chunk = [0u8; 8192];
            loop {
                let n = self.read(&mut chunk)?;
                if n == 0 {
                    return Ok(got);
                }
                into.extend_from_slice(&chunk[..n]);
                got += n;
            }
        }

        /// Fills the whole buffer or fails, which is what distinguishes this
        /// from `read`: a short read is not the end of the data.
        pub fn read_exact(&mut self, into: &mut [u8]) -> Result<()> {
            let mut done = 0;
            while done < into.len() {
                let n = self.read(&mut into[done..])?;
                if n == 0 {
                    // Fewer bytes than asked for and none left to come.
                    return Err(Error::from_raw_os_error(0));
                }
                done += n;
            }
            Ok(())
        }

        pub fn write_all(&mut self, bytes: &[u8]) -> Result<()> {
            let mut done = 0;
            while done < bytes.len() {
                let n = unsafe {
                    sys_write(self.fd, bytes[done..].as_ptr() as *const core::ffi::c_void, bytes.len() - done)
                };
                if n > 0 {
                    done += n as usize;
                } else if n < 0 && errno() == EINTR {
                    continue;
                } else {
                    return Err(Error::last_os_error());
                }
            }
            Ok(())
        }

        /// `sync_all`: the bytes are on the device, not merely accepted.
        pub fn sync(&mut self) -> Result<()> {
            check(unsafe { fsync(self.fd) })
        }

        pub fn seek(&mut self, whence: i32, at: i64) -> Result<u64> {
            let r = unsafe { lseek(self.fd, at, whence) };
            if r < 0 {
                Err(Error::last_os_error())
            } else {
                Ok(r as u64)
            }
        }

        pub fn is_empty(&mut self) -> Result<bool> {
            Ok(self.len()? == 0)
        }

        pub fn len(&mut self) -> Result<u64> {
            let here = self.seek(SEEK_CUR, 0)?;
            let end = self.seek(SEEK_END, 0)?;
            self.seek(SEEK_SET, here as i64)?;
            Ok(end)
        }
    }

    impl Drop for File {
        fn drop(&mut self) {
            unsafe { close(self.fd) };
        }
    }

    #[derive(Default)]
    pub struct OpenOptions {
        read: bool,
        write: bool,
        append: bool,
        create: bool,
        truncate: bool,
    }

    impl OpenOptions {
        pub fn new() -> OpenOptions {
            OpenOptions::default()
        }
        pub fn read(&mut self, on: bool) -> &mut OpenOptions {
            self.read = on;
            self
        }
        pub fn write(&mut self, on: bool) -> &mut OpenOptions {
            self.write = on;
            self
        }
        pub fn append(&mut self, on: bool) -> &mut OpenOptions {
            self.append = on;
            self
        }
        pub fn create(&mut self, on: bool) -> &mut OpenOptions {
            self.create = on;
            self
        }
        pub fn truncate(&mut self, on: bool) -> &mut OpenOptions {
            self.truncate = on;
            self
        }

        pub fn open(&self, path: &str) -> Result<File> {
            let mut flags = match (self.read, self.write || self.append) {
                (true, true) => O_RDWR,
                (false, true) => O_WRONLY,
                _ => O_RDONLY,
            };
            if self.append {
                flags |= O_APPEND;
            }
            if self.create {
                flags |= O_CREAT;
            }
            if self.truncate {
                flags |= O_TRUNC;
            }
            File::with_flags(path, flags, 0o666)
        }
    }

    pub fn read(path: &str) -> Result<Vec<u8>> {
        let mut file = File::open(path)?;
        let mut held = Vec::new();
        file.read_to_end(&mut held)?;
        Ok(held)
    }

    pub fn read_to_string(path: &str) -> Result<String> {
        let bytes = read(path)?;
        match String::from_utf8(bytes) {
            Ok(text) => Ok(text),
            Err(_) => Err(Error::from_raw_os_error(crate::io::EILSEQ)),
        }
    }

    pub fn write(path: &str, bytes: &[u8]) -> Result<()> {
        File::create(path)?.write_all(bytes)
    }

    pub fn rename(from: &str, to: &str) -> Result<()> {
        let (a, b) = (c_path(from), c_path(to));
        check(unsafe { sys_rename(a.as_ptr(), b.as_ptr()) })
    }

    pub fn remove_file(path: &str) -> Result<()> {
        let at = c_path(path);
        check(unsafe { unlink(at.as_ptr()) })
    }

    /// Each component in turn, treating "it is already there" as success --
    /// which is what makes this `_all` and not `mkdir`.
    pub fn create_dir_all(path: &str) -> Result<()> {
        let mut so_far = String::new();
        if path.starts_with('/') {
            so_far.push('/');
        }
        for part in path.split('/').filter(|p| !p.is_empty()) {
            if !so_far.is_empty() && !so_far.ends_with('/') {
                so_far.push('/');
            }
            so_far.push_str(part);
            let at = c_path(&so_far);
            let r = unsafe { mkdir(at.as_ptr(), 0o777) };
            if r < 0 && errno() != EEXIST {
                return Err(Error::last_os_error());
            }
        }
        Ok(())
    }

    /// `Path::exists`, which is one `access` call and does not need the
    /// file opened.
    pub fn exists(path: &str) -> bool {
        let at = c_path(path);
        unsafe { access(at.as_ptr(), F_OK) == 0 }
    }

    pub struct Metadata {
        len: u64,
    }

    impl Metadata {
        pub fn len(&self) -> u64 {
            self.len
        }

        pub fn is_empty(&self) -> bool {
            self.len == 0
        }
    }

    /// Size by `lseek` on an opened file rather than `stat`. `struct stat`'s
    /// layout differs between macOS and Linux in ways that are easy to get
    /// subtly wrong, and size is all the one caller reads. The difference
    /// this leaves: a file that cannot be opened reports its own error here
    /// where `stat` would have succeeded.
    pub fn metadata(path: &str) -> Result<Metadata> {
        let mut file = File::open(path)?;
        Ok(Metadata { len: file.len()? })
    }

    /// No directories on `wasm32-unknown-unknown`, as `std` also reports.
    #[cfg(target_arch = "wasm32")]
    pub fn read_dir(_path: &str) -> Result<Vec<String>> {
        Err(Error::from_raw_os_error(crate::io::ENOENT))
    }

    #[cfg(not(target_arch = "wasm32"))]
    /// Names only, which is what the one caller wants. The directory handle
    /// is closed on every path out, including the error ones.
    pub fn read_dir(path: &str) -> Result<Vec<String>> {
        let at = c_path(path);
        let dir = unsafe { opendir(at.as_ptr()) };
        if dir.is_null() {
            return Err(Error::last_os_error());
        }
        let mut names = Vec::new();
        loop {
            let entry = unsafe { readdir(dir) };
            if entry.is_null() {
                break;
            }
            let name_at = unsafe { (*entry).name.as_ptr() };
            let mut len = 0;
            while unsafe { *name_at.add(len) } != 0 {
                len += 1;
            }
            let bytes = unsafe { core::slice::from_raw_parts(name_at, len) };
            // `.` and `..` are entries that `std`'s `read_dir` does not give.
            if bytes == b"." || bytes == b".." {
                continue;
            }
            names.push(String::from_utf8_lossy(bytes).into_owned());
        }
        unsafe { closedir(dir) };
        Ok(names)
    }
}

/// The operating system, called directly.
///
/// `core` has no `Instant`, no `exit` and no environment, because none of
/// them can be had without asking the kernel. It does have
/// `core::time::Duration`, so that stays.
pub mod sys {
    use crate::c_bytes;
    use core::time::Duration;

    pub const CLOCK_REALTIME: i32 = 0;
    #[cfg(target_os = "macos")]
    pub const CLOCK_MONOTONIC: i32 = 6;
    #[cfg(not(target_os = "macos"))]
    pub const CLOCK_MONOTONIC: i32 = 1;

    #[repr(C)]
    #[derive(Clone, Copy, Default)]
    pub struct TimeSpec {
        pub secs: i64,
        pub nanos: i64,
    }

    #[cfg(target_arch = "wasm32")]
    use crate::absent::{getenv, libc_exit};

    #[cfg(not(target_arch = "wasm32"))]
    extern "C" {
        fn getenv(name: *const u8) -> *const u8;
        #[link_name = "exit"]
        fn libc_exit(code: i32) -> !;
        pub fn abort() -> !;
    }

    extern "C" {
        fn clock_gettime(clock: i32, at: *mut TimeSpec) -> i32;
    }

    /// `std` flushed stdout when the process ended; raw file descriptors do
    /// not, so anything still in the buffer would be lost. Every exit in this
    /// runtime goes through here for that reason.
    pub fn exit(code: i32) -> ! {
        crate::io::flush_all();
        unsafe { libc_exit(code) }
    }

    fn now(clock: i32) -> Duration {
        let mut at = TimeSpec::default();
        unsafe { clock_gettime(clock, &mut at) };
        Duration::new(at.secs as u64, at.nanos as u32)
    }

    /// Monotonic, so it cannot go backwards when the wall clock is set --
    /// which is the whole reason to prefer it for measuring.
    #[derive(Clone, Copy)]
    pub struct Instant(Duration);

    impl Instant {
        pub fn now() -> Instant {
            Instant(now(CLOCK_MONOTONIC))
        }

        pub fn elapsed(&self) -> Duration {
            now(CLOCK_MONOTONIC).saturating_sub(self.0)
        }

        pub fn duration_since(&self, earlier: Instant) -> Duration {
            self.0.saturating_sub(earlier.0)
        }
    }

    /// Wall clock, for the one thing that wants it: seconds since the epoch.
    pub fn unix_time() -> Duration {
        now(CLOCK_REALTIME)
    }

    /// `getenv` gives a pointer into the process environment, valid until
    /// something calls `setenv`. The runtime only reads, so copying the bytes
    /// out keeps that from mattering.
    pub fn env_var(name: &str) -> Option<alloc::string::String> {
        // `getenv` wants a C string and `name` is a Rust one, so a copy with
        // a terminator is unavoidable.
        let mut owned = alloc::vec::Vec::with_capacity(name.len() + 1);
        owned.extend_from_slice(name.as_bytes());
        owned.push(0);
        let at = unsafe { getenv(owned.as_ptr()) };
        if at.is_null() {
            return None;
        }
        let bytes = unsafe { c_bytes(at) };
        core::str::from_utf8(bytes).ok().map(alloc::borrow::ToOwned::to_owned)
    }
}

/// `Mutex`, `RwLock` and `OnceLock` without `std`.
///
/// POSIX again, for the same reason `tls` is: these need the operating
/// system, and this runtime already calls it directly. No poisoning -- a
/// panic here calls `exit(101)` rather than unwinding ([E-PANIC-EXIT]), so a
/// lock can never be left poisoned and `PoisonError` has nothing to report.
pub mod sync {
    use core::cell::UnsafeCell;
    use core::mem::MaybeUninit;
    #[cfg(not(target_arch = "wasm32"))]
    use core::ops::{Deref, DerefMut};
    use core::sync::atomic::{AtomicU8, Ordering};

    /// `wasm32-unknown-unknown` has no pthreads and one thread, so a lock
    /// there holds no lock. It still counts, for one reason: without the
    /// count, two live guards would be two `&mut` to the same value, which is
    /// undefined behaviour. A pthread mutex would deadlock on that; this
    /// panics and says so, which is the same bug reported instead of
    /// miscompiled.
    #[cfg(target_arch = "wasm32")]
    mod single {
        use core::cell::{Cell, UnsafeCell};
        use core::ops::{Deref, DerefMut};

        pub struct Mutex<T> {
            held: Cell<bool>,
            value: UnsafeCell<T>,
        }

    unsafe impl<T: Send> Send for Mutex<T> {}
    unsafe impl<T: Send> Sync for Mutex<T> {}

        impl<T> Mutex<T> {
            pub const fn new(value: T) -> Mutex<T> {
                Mutex {
                    held: Cell::new(false),
                    value: UnsafeCell::new(value),
                }
            }

            pub fn lock(&self) -> Guard<'_, T> {
                assert!(
                    !self.held.get(),
                    "a lock was taken twice on the one thread wasm32 has, which a pthread mutex would deadlock on"
                );
                self.held.set(true);
                Guard { held: &self.held, value: self.value.get() }
            }
        }

        pub struct Guard<'a, T> {
            held: &'a Cell<bool>,
            value: *mut T,
        }

    impl<T> Deref for Guard<'_, T> {
            type Target = T;
            fn deref(&self) -> &T {
                unsafe { &*self.value }
            }
        }

    impl<T> DerefMut for Guard<'_, T> {
            fn deref_mut(&mut self) -> &mut T {
                unsafe { &mut *self.value }
            }
        }

    impl<T> Drop for Guard<'_, T> {
            fn drop(&mut self) {
                self.held.set(false);
            }
        }

        /// Readers may overlap, so this counts them rather than forbidding a
        /// second one.
        pub struct RwLock<T> {
            readers: Cell<usize>,
            writing: Cell<bool>,
            value: UnsafeCell<T>,
        }

    unsafe impl<T: Send> Send for RwLock<T> {}
    unsafe impl<T: Send + Sync> Sync for RwLock<T> {}

        impl<T> RwLock<T> {
            pub const fn new(value: T) -> RwLock<T> {
                RwLock {
                    readers: Cell::new(0),
                    writing: Cell::new(false),
                    value: UnsafeCell::new(value),
                }
            }

            pub fn read(&self) -> ReadGuard<'_, T> {
                assert!(!self.writing.get(), "a read while writing, on one thread");
                self.readers.set(self.readers.get() + 1);
                ReadGuard { readers: &self.readers, value: self.value.get() }
            }

            pub fn write(&self) -> Guard<'_, T> {
                assert!(
                    !self.writing.get() && self.readers.get() == 0,
                    "a write while the lock is held, on one thread"
                );
                self.writing.set(true);
                Guard { held: &self.writing, value: self.value.get() }
            }
        }

        pub struct ReadGuard<'a, T> {
            readers: &'a Cell<usize>,
            value: *mut T,
        }

    impl<T> Deref for ReadGuard<'_, T> {
            type Target = T;
            fn deref(&self) -> &T {
                unsafe { &*self.value }
            }
        }

    impl<T> Drop for ReadGuard<'_, T> {
            fn drop(&mut self) {
                self.readers.set(self.readers.get() - 1);
            }
        }
    }

    #[cfg(target_arch = "wasm32")]
    pub use single::{Guard, Mutex, ReadGuard, RwLock};

    // Opaque, and sized generously: macOS wants 64 bytes for a mutex and 200
    // for an rwlock, glibc 40 and 56. Over-sizing costs bytes in a static and
    // under-sizing corrupts the heap, so these are the larger of the two
    // plus room.
    #[repr(C, align(16))]
    #[cfg(not(target_arch = "wasm32"))]
    struct RawLock([u8; 256]);

    #[cfg(not(target_arch = "wasm32"))]
    extern "C" {
        fn pthread_mutex_init(m: *mut u8, attr: *const u8) -> i32;
        fn pthread_mutex_lock(m: *mut u8) -> i32;
        fn pthread_mutex_unlock(m: *mut u8) -> i32;
        fn pthread_rwlock_init(l: *mut u8, attr: *const u8) -> i32;
        fn pthread_rwlock_rdlock(l: *mut u8) -> i32;
        fn pthread_rwlock_wrlock(l: *mut u8) -> i32;
        fn pthread_rwlock_unlock(l: *mut u8) -> i32;
    }

    /// Not started, being started, started. A lock is made once, by whoever
    /// gets there first; everyone else spins over the handful of
    /// instructions that takes.
    const UNSET: u8 = 0;
    const SETTING: u8 = 1;
    const READY: u8 = 2;

    #[cfg(not(target_arch = "wasm32"))]
    pub struct Mutex<T> {
        raw: UnsafeCell<MaybeUninit<RawLock>>,
        state: AtomicU8,
        value: UnsafeCell<T>,
    }

    #[cfg(not(target_arch = "wasm32"))]
    unsafe impl<T: Send> Send for Mutex<T> {}
    #[cfg(not(target_arch = "wasm32"))]
    unsafe impl<T: Send> Sync for Mutex<T> {}

    #[cfg(not(target_arch = "wasm32"))]
    #[cfg(not(target_arch = "wasm32"))]
    impl<T> Mutex<T> {
        pub const fn new(value: T) -> Mutex<T> {
            Mutex {
                raw: UnsafeCell::new(MaybeUninit::uninit()),
                state: AtomicU8::new(UNSET),
                value: UnsafeCell::new(value),
            }
        }

        fn raw(&self) -> *mut u8 {
            let at = self.raw.get() as *mut u8;
            loop {
                match self.state.compare_exchange(
                    UNSET,
                    SETTING,
                    Ordering::AcqRel,
                    Ordering::Acquire,
                ) {
                    Ok(_) => {
                        unsafe { pthread_mutex_init(at, core::ptr::null()) };
                        self.state.store(READY, Ordering::Release);
                        return at;
                    }
                    Err(READY) => return at,
                    Err(_) => core::hint::spin_loop(),
                }
            }
        }

        pub fn lock(&self) -> Guard<'_, T> {
            let at = self.raw();
            unsafe { pthread_mutex_lock(at) };
            Guard { at, value: self.value.get(), _held: core::marker::PhantomData }
        }
    }

    #[cfg(not(target_arch = "wasm32"))]
    pub struct Guard<'a, T> {
        at: *mut u8,
        value: *mut T,
        _held: core::marker::PhantomData<&'a T>,
    }

    #[cfg(not(target_arch = "wasm32"))]
    impl<T> Deref for Guard<'_, T> {
        type Target = T;
        fn deref(&self) -> &T {
            unsafe { &*self.value }
        }
    }

    #[cfg(not(target_arch = "wasm32"))]
    impl<T> DerefMut for Guard<'_, T> {
        fn deref_mut(&mut self) -> &mut T {
            unsafe { &mut *self.value }
        }
    }

    #[cfg(not(target_arch = "wasm32"))]
    impl<T> Drop for Guard<'_, T> {
        fn drop(&mut self) {
            unsafe { pthread_mutex_unlock(self.at) };
        }
    }

    #[cfg(not(target_arch = "wasm32"))]
    pub struct RwLock<T> {
        raw: UnsafeCell<MaybeUninit<RawLock>>,
        state: AtomicU8,
        value: UnsafeCell<T>,
    }

    #[cfg(not(target_arch = "wasm32"))]
    unsafe impl<T: Send> Send for RwLock<T> {}
    #[cfg(not(target_arch = "wasm32"))]
    unsafe impl<T: Send + Sync> Sync for RwLock<T> {}

    #[cfg(not(target_arch = "wasm32"))]
    #[cfg(not(target_arch = "wasm32"))]
    impl<T> RwLock<T> {
        pub const fn new(value: T) -> RwLock<T> {
            RwLock {
                raw: UnsafeCell::new(MaybeUninit::uninit()),
                state: AtomicU8::new(UNSET),
                value: UnsafeCell::new(value),
            }
        }

        fn raw(&self) -> *mut u8 {
            let at = self.raw.get() as *mut u8;
            loop {
                match self.state.compare_exchange(
                    UNSET,
                    SETTING,
                    Ordering::AcqRel,
                    Ordering::Acquire,
                ) {
                    Ok(_) => {
                        unsafe { pthread_rwlock_init(at, core::ptr::null()) };
                        self.state.store(READY, Ordering::Release);
                        return at;
                    }
                    Err(READY) => return at,
                    Err(_) => core::hint::spin_loop(),
                }
            }
        }

        pub fn read(&self) -> ReadGuard<'_, T> {
            let at = self.raw();
            unsafe { pthread_rwlock_rdlock(at) };
            ReadGuard { at, value: self.value.get(), _held: core::marker::PhantomData }
        }

        pub fn write(&self) -> Guard<'_, T> {
            let at = self.raw();
            unsafe { pthread_rwlock_wrlock(at) };
            // A write guard unlocks an rwlock, not a mutex, so it cannot be
            // the `Guard` above: that one calls `pthread_mutex_unlock`.
            Guard { at, value: self.value.get(), _held: core::marker::PhantomData }
        }
    }

    #[cfg(not(target_arch = "wasm32"))]
    pub struct ReadGuard<'a, T> {
        at: *mut u8,
        value: *mut T,
        _held: core::marker::PhantomData<&'a T>,
    }

    #[cfg(not(target_arch = "wasm32"))]
    impl<T> Deref for ReadGuard<'_, T> {
        type Target = T;
        fn deref(&self) -> &T {
            unsafe { &*self.value }
        }
    }

    #[cfg(not(target_arch = "wasm32"))]
    impl<T> Drop for ReadGuard<'_, T> {
        fn drop(&mut self) {
            unsafe { pthread_rwlock_unlock(self.at) };
        }
    }

    /// Written once, read for ever after. `Mutex` could carry an `Option`
    /// instead, but every read would then take a lock, and these hold things
    /// the runtime reads on hot paths.
    pub struct OnceLock<T> {
        state: AtomicU8,
        value: UnsafeCell<MaybeUninit<T>>,
    }

    unsafe impl<T: Send> Send for OnceLock<T> {}
    unsafe impl<T: Send + Sync> Sync for OnceLock<T> {}

    impl<T> OnceLock<T> {
        pub const fn new() -> OnceLock<T> {
            OnceLock {
                state: AtomicU8::new(UNSET),
                value: UnsafeCell::new(MaybeUninit::uninit()),
            }
        }

        pub fn get(&self) -> Option<&T> {
            if self.state.load(Ordering::Acquire) == READY {
                // Safety: READY is stored with Release after the write
                // below, so the value is there and never changes again.
                Some(unsafe { (*self.value.get()).assume_init_ref() })
            } else {
                None
            }
        }

        pub fn get_or_init(&self, make: impl FnOnce() -> T) -> &T {
            loop {
                match self.state.compare_exchange(
                    UNSET,
                    SETTING,
                    Ordering::AcqRel,
                    Ordering::Acquire,
                ) {
                    Ok(_) => {
                        let at = self.value.get();
                        unsafe { (*at).write(make()) };
                        self.state.store(READY, Ordering::Release);
                        return unsafe { (*at).assume_init_ref() };
                    }
                    Err(READY) => {
                        return unsafe { (*self.value.get()).assume_init_ref() }
                    }
                    // Another thread is running its `make`. It may take a
                    // while, but a `OnceLock` initialiser here is short and
                    // this only happens on the first touch.
                    Err(_) => core::hint::spin_loop(),
                }
            }
        }

        pub fn set(&self, value: T) -> Result<(), T> {
            match self.state.compare_exchange(
                UNSET,
                SETTING,
                Ordering::AcqRel,
                Ordering::Acquire,
            ) {
                Ok(_) => {
                    unsafe { (*self.value.get()).write(value) };
                    self.state.store(READY, Ordering::Release);
                    Ok(())
                }
                Err(_) => Err(value),
            }
        }
    }

    impl<T> Default for OnceLock<T> {
        fn default() -> OnceLock<T> {
            OnceLock::new()
        }
    }
}

/// The shape `std`'s macro accepts, over `tls::Local`. The `const { … }` form
/// is taken and the braces dropped: the initialiser is run on first use here
/// rather than folded at compile time, which costs one branch per thread.
#[macro_export]
macro_rules! thread_local {
    ($($(#[$m:meta])* $v:vis static $N:ident: $T:ty = const { $e:expr };)+) => {
        $($(#[$m])* $v static $N: $crate::tls::Local<$T> = $crate::tls::Local::new(|| $e);)+
    };
    ($($(#[$m:meta])* $v:vis static $N:ident: $T:ty = $e:expr;)+) => {
        $($(#[$m])* $v static $N: $crate::tls::Local<$T> = $crate::tls::Local::new(|| $e);)+
    };
}

/// Floating-point maths, from libm.
///
/// `core` has no `sqrt`, `powf` or `floor`: those are not language operations
/// but calls into libm, and only `std` declares them. We link libm anyway, so
/// this declares them directly -- and as a trait on `f64`, so that every
/// existing `x.sqrt()` reads the same as it did under `std`.
///
/// `round` is libm's, which rounds halves **away from zero**, the same rule
/// `std`'s `f64::round` uses and the one [L-LIB-ROUND] requires. A different
/// rounding here would move every money column in the corpus.
pub mod math {
    extern "C" {
        fn sqrt(x: f64) -> f64;
        fn cbrt(x: f64) -> f64;
        fn pow(x: f64, y: f64) -> f64;
        fn hypot(x: f64, y: f64) -> f64;
        fn floor(x: f64) -> f64;
        fn ceil(x: f64) -> f64;
        fn round(x: f64) -> f64;
        fn exp(x: f64) -> f64;
        fn log(x: f64) -> f64;
        fn log2(x: f64) -> f64;
        fn log10(x: f64) -> f64;
        fn sin(x: f64) -> f64;
        fn cos(x: f64) -> f64;
        fn tan(x: f64) -> f64;
        fn asin(x: f64) -> f64;
        fn acos(x: f64) -> f64;
        fn atan(x: f64) -> f64;
        fn atan2(y: f64, x: f64) -> f64;
        fn sinh(x: f64) -> f64;
        fn cosh(x: f64) -> f64;
        fn tanh(x: f64) -> f64;
    }

    /// The methods `std` puts on `f64`, with the same names and the same
    /// meanings, so no call site changes.
    pub trait Float {
        fn sqrt(self) -> f64;
        fn cbrt(self) -> f64;
        fn powf(self, exp: f64) -> f64;
        fn hypot(self, other: f64) -> f64;
        fn floor(self) -> f64;
        fn ceil(self) -> f64;
        fn round(self) -> f64;
        fn exp(self) -> f64;
        fn ln(self) -> f64;
        fn log(self, base: f64) -> f64;
        fn log2(self) -> f64;
        fn log10(self) -> f64;
        fn sin(self) -> f64;
        fn cos(self) -> f64;
        fn tan(self) -> f64;
        fn asin(self) -> f64;
        fn acos(self) -> f64;
        fn atan(self) -> f64;
        fn atan2(self, x: f64) -> f64;
        fn sinh(self) -> f64;
        fn cosh(self) -> f64;
        fn tanh(self) -> f64;
    }

    impl Float for f64 {
        fn sqrt(self) -> f64 {
            unsafe { sqrt(self) }
        }
        fn cbrt(self) -> f64 {
            unsafe { cbrt(self) }
        }
        fn powf(self, exp: f64) -> f64 {
            unsafe { pow(self, exp) }
        }
        fn hypot(self, other: f64) -> f64 {
            unsafe { hypot(self, other) }
        }
        fn floor(self) -> f64 {
            unsafe { floor(self) }
        }
        fn ceil(self) -> f64 {
            unsafe { ceil(self) }
        }
        fn round(self) -> f64 {
            unsafe { round(self) }
        }
        fn exp(self) -> f64 {
            unsafe { exp(self) }
        }
        fn ln(self) -> f64 {
            unsafe { log(self) }
        }
        /// `std`'s `log(base)` is a ratio of natural logarithms, and is
        /// computed the same way here so the two agree bit for bit.
        fn log(self, base: f64) -> f64 {
            unsafe { log(self) / log(base) }
        }
        fn log2(self) -> f64 {
            unsafe { log2(self) }
        }
        fn log10(self) -> f64 {
            unsafe { log10(self) }
        }
        fn sin(self) -> f64 {
            unsafe { sin(self) }
        }
        fn cos(self) -> f64 {
            unsafe { cos(self) }
        }
        fn tan(self) -> f64 {
            unsafe { tan(self) }
        }
        fn asin(self) -> f64 {
            unsafe { asin(self) }
        }
        fn acos(self) -> f64 {
            unsafe { acos(self) }
        }
        fn atan(self) -> f64 {
            unsafe { atan(self) }
        }
        fn atan2(self, x: f64) -> f64 {
            unsafe { atan2(self, x) }
        }
        fn sinh(self) -> f64 {
            unsafe { sinh(self) }
        }
        fn cosh(self) -> f64 {
            unsafe { cosh(self) }
        }
        fn tanh(self) -> f64 {
            unsafe { tanh(self) }
        }
    }
}

// `std` is taken back for the tests, which need somewhere to put files and a
// thread to prove a thread-local is one. Nothing below is linked into a
// compiled program.
#[cfg(test)]
extern crate std;

#[cfg(test)]
mod tests {
    #[allow(unused_imports)]
    use alloc::{format, string::String, string::ToString, vec, vec::Vec};

    fn scratch(name: &str) -> String {
        let at = format!(
            "{}/grasm_sys_test_{}_{}",
            std::env::temp_dir().display(),
            std::process::id(),
            name
        );
        let _ = crate::fs::create_dir_all(&at);
        at
    }

    /// The defect this exists for: `open`'s mode is a **variadic** argument,
    /// and declaring it as a fixed parameter sent it to a register on arm64
    /// where `open` reads the stack. The file was then created with a garbage
    /// mode, so the *first* append worked and the *second* failed with
    /// "Permission denied". One append would not have caught it.
    #[test]
    fn appending_twice_leaves_both_writes() {
        let dir = scratch("append");
        let path = format!("{dir}/log");
        let _ = crate::fs::remove_file(&path);
        crate::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(&path)
            .expect("first open")
            .write_all(b"one")
            .expect("first write");
        crate::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(&path)
            .expect("second open -- a garbage mode fails here, not above")
            .write_all(b"two")
            .expect("second write");
        assert_eq!(crate::fs::read(&path).unwrap(), b"onetwo");
        assert_eq!(crate::fs::metadata(&path).unwrap().len(), 6);
    }

    #[test]
    fn a_file_round_trips() {
        let dir = scratch("roundtrip");
        let path = format!("{dir}/f");
        crate::fs::write(&path, b"hello").unwrap();
        assert_eq!(crate::fs::read_to_string(&path).unwrap(), "hello");
        assert!(crate::fs::exists(&path));
        assert_eq!(crate::fs::metadata(&path).unwrap().len(), 5);
        assert!(!crate::fs::metadata(&path).unwrap().is_empty());

        let moved = format!("{dir}/g");
        crate::fs::rename(&path, &moved).unwrap();
        assert!(!crate::fs::exists(&path));
        assert!(crate::fs::exists(&moved));
        crate::fs::remove_file(&moved).unwrap();
        assert!(!crate::fs::exists(&moved));
    }

    /// A `read` is allowed to return less than asked for, so the loops in
    /// `read_to_end` and `read_exact` are the point. 40 KiB is several
    /// 8 KiB chunks.
    #[test]
    fn a_long_read_does_not_stop_at_the_first_chunk() {
        let dir = scratch("long");
        let path = format!("{dir}/big");
        let body: Vec<u8> = (0..40_960u32).map(|i| (i % 251) as u8).collect();
        crate::fs::write(&path, &body).unwrap();
        assert_eq!(crate::fs::read(&path).unwrap(), body);

        let mut f = crate::fs::File::open(&path).unwrap();
        let mut exact = vec![0u8; 9000];
        f.read_exact(&mut exact).unwrap();
        assert_eq!(exact, body[..9000]);
    }

    #[test]
    fn seeking_reads_from_where_it_was_asked() {
        let dir = scratch("seek");
        let path = format!("{dir}/s");
        crate::fs::write(&path, b"0123456789").unwrap();
        let mut f = crate::fs::File::open(&path).unwrap();
        f.seek(crate::fs::SEEK_SET, 4).unwrap();
        let mut four = [0u8; 4];
        f.read_exact(&mut four).unwrap();
        assert_eq!(&four, b"4567");
        // `len` has to leave the position where it found it.
        let before = f.seek(crate::fs::SEEK_CUR, 0).unwrap();
        assert_eq!(f.len().unwrap(), 10);
        assert_eq!(f.seek(crate::fs::SEEK_CUR, 0).unwrap(), before);
    }

    #[test]
    fn a_directory_lists_its_names_and_not_dot_or_dotdot() {
        let dir = scratch("listing");
        crate::fs::write(&format!("{dir}/a"), b"").unwrap();
        crate::fs::write(&format!("{dir}/b"), b"").unwrap();
        let mut names = crate::fs::read_dir(&dir).unwrap();
        names.sort();
        assert_eq!(names, vec!["a".to_string(), "b".to_string()]);
    }

    #[test]
    fn create_dir_all_makes_every_component() {
        let dir = scratch("nested");
        let deep = format!("{dir}/x/y/z");
        crate::fs::create_dir_all(&deep).unwrap();
        assert!(crate::fs::exists(&deep));
        // Twice is not an error, which is what makes it `_all`.
        crate::fs::create_dir_all(&deep).unwrap();
    }

    #[test]
    fn a_missing_file_says_so() {
        let e = crate::fs::read("/nonexistent/grasm_sys/probe").unwrap_err();
        assert_eq!(e.kind(), crate::io::ErrorKind::NotFound);
        // `strerror`'s text, which is what `std` prints for the same errno.
        assert!(e.to_string().to_lowercase().contains("no such file"));
    }

    /// The reasoning this checks: `sin_port` sits at offset 2 in network byte
    /// order for IPv4 and IPv6 and on both platforms, while the family field
    /// does move. `AF_INET6` was once 30 on both, which made an IPv6 address
    /// parse as IPv4 and come back as wrong text with no error.
    #[test]
    fn a_resolved_address_reads_back() {
        let v4 = crate::net::resolve("127.0.0.1", 8080).unwrap();
        assert_eq!(v4.parts(), ("127.0.0.1".to_string(), 8080));

        if let Ok(v6) = crate::net::resolve("::1", 9090) {
            let (text, port) = v6.parts();
            assert_eq!(port, 9090);
            assert_eq!(text, "::1", "an IPv6 address read as IPv4 gives this away");
        }
    }

    #[test]
    fn a_listener_reports_the_port_it_was_given() {
        let sock = crate::net::listen_on("127.0.0.1", 0).unwrap();
        let port = sock.local_addr().unwrap().port();
        assert_ne!(port, 0, "port 0 means the kernel chose one and we read it");
    }

    #[test]
    fn the_environment_is_readable_and_absence_is_none() {
        std::env::set_var("GRASM_SYS_TEST_VAR", "a value");
        assert_eq!(
            crate::sys::env_var("GRASM_SYS_TEST_VAR").as_deref(),
            Some("a value")
        );
        assert!(crate::sys::env_var("GRASM_SYS_TEST_ABSENT_VAR").is_none());
    }

    #[test]
    fn the_monotonic_clock_does_not_go_backwards() {
        let start = crate::sys::Instant::now();
        let mut last = core::time::Duration::ZERO;
        for _ in 0..1000 {
            let now = start.elapsed();
            assert!(now >= last);
            last = now;
        }
    }

    #[test]
    fn a_thread_local_is_one_per_thread() {
        thread_local! {
            static COUNT: core::cell::Cell<i64> = const { core::cell::Cell::new(0) };
        }
        COUNT.with(|c| c.set(7));
        let seen_elsewhere = std::thread::spawn(|| COUNT.with(|c| c.get()))
            .join()
            .unwrap();
        assert_eq!(seen_elsewhere, 0, "a second thread must not see the first's 7");
        assert_eq!(COUNT.with(|c| c.get()), 7);
    }

    #[test]
    fn a_mutex_serialises_and_a_oncelock_runs_once() {
        static TOTAL: crate::sync::Mutex<i64> = crate::sync::Mutex::new(0);
        crate::thread::scope(|scope| {
            for _ in 0..8 {
                scope.spawn(|| {
                    for _ in 0..1000 {
                        *TOTAL.lock() += 1;
                    }
                });
            }
        });
        assert_eq!(*TOTAL.lock(), 8000, "a lost update means the lock is not one");

        static MADE: crate::sync::OnceLock<i64> = crate::sync::OnceLock::new();
        static TIMES: core::sync::atomic::AtomicI64 = core::sync::atomic::AtomicI64::new(0);
        crate::thread::scope(|scope| {
            for _ in 0..8 {
                scope.spawn(|| {
                    MADE.get_or_init(|| {
                        TIMES.fetch_add(1, core::sync::atomic::Ordering::SeqCst);
                        42
                    });
                });
            }
        });
        assert_eq!(*MADE.get().unwrap(), 42);
        assert_eq!(TIMES.load(core::sync::atomic::Ordering::SeqCst), 1);
    }

    /// `scope` has to join every thread before it returns, because the
    /// closures hold `&mut` into the caller's frame.
    #[test]
    fn scope_joins_before_it_returns() {
        let mut out = [0i64; 64];
        crate::thread::scope(|scope| {
            for (i, slot) in out.iter_mut().enumerate() {
                scope.spawn(move || {
                    *slot = i as i64 * 2;
                });
            }
        });
        for (i, got) in out.iter().enumerate() {
            assert_eq!(*got, i as i64 * 2, "slot {i} was not written before scope returned");
        }
    }

    /// libm's `round`, which is halves **away from zero** -- `std`'s rule and
    /// [L-LIB-ROUND]'s. Rust's formatting rounds halves to even, so getting
    /// this from the wrong place would move every money column in the corpus.
    #[test]
    fn rounding_takes_halves_away_from_zero() {
        #[allow(unused_imports)]
        use crate::math::Float;
        assert_eq!(2.5f64.round(), 3.0);
        assert_eq!((-2.5f64).round(), -3.0);
        assert_eq!(0.5f64.round(), 1.0);
        assert_eq!(1.5f64.round(), 2.0);
    }

    #[test]
    fn the_float_maths_agrees_with_known_values() {
        #[allow(unused_imports)]
        use crate::math::Float;
        assert_eq!(4.0f64.sqrt(), 2.0);
        assert_eq!(2.0f64.powf(10.0), 1024.0);
        assert_eq!(27.0f64.cbrt(), 3.0);
        assert_eq!(1.0f64.ln(), 0.0);
        assert_eq!(1000.0f64.log10(), 3.0);
        assert_eq!(8.0f64.log2(), 3.0);
        assert_eq!(0.0f64.sin(), 0.0);
        assert_eq!(1.0f64.cos(), 1.0f64.cos());
        assert_eq!((-1.5f64).floor(), -2.0);
        assert_eq!((-1.5f64).ceil(), -1.0);
        assert_eq!(3.0f64.hypot(4.0), 5.0);
        // `log(base)` is a ratio of natural logs, as `std` computes it.
        assert_eq!(8.0f64.log(2.0), 3.0);
    }

    /// Both of a child's pipes are drained together, so neither can fill and
    /// stall the other. More than a pipe buffer on stdout is the case that
    /// deadlocks if they are read one after the other.
    #[test]
    fn a_child_that_writes_more_than_a_pipe_buffer_does_not_stall() {
        // Both streams are written, and stdout is far more than a pipe
        // buffer (64 KiB on Linux), so reading one to the end before the
        // other would stall here rather than fail.
        let (code, out, err) = crate::process::run(
            "sh",
            &[
                "-c".to_string(),
                "i=0; while [ $i -lt 2000 ]; do echo aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa; echo bbbb >&2; i=$((i+1)); done".to_string(),
            ],
        )
        .unwrap();
        assert_eq!(code, 0);
        // Every line of both streams arrived, and stdout is well past a pipe
        // buffer -- which is the property, rather than an exact byte count.
        assert_eq!(out.iter().filter(|&&b| b == b'\n').count(), 2000);
        assert_eq!(err.iter().filter(|&&b| b == b'\n').count(), 2000);
        assert!(out.len() > 100_000, "stdout was only {} bytes", out.len());
    }

    #[test]
    fn a_child_reports_its_status_and_its_stderr() {
        let (code, out, err) = crate::process::run(
            "sh",
            &["-c".to_string(), "echo mine; echo theirs >&2; exit 3".to_string()],
        )
        .unwrap();
        assert_eq!(code, 3);
        assert_eq!(out, b"mine\n");
        assert_eq!(err, b"theirs\n");
    }

    /// 127 is what a shell reports for a command it could not run, and what
    /// `_exit` is given when `execvp` fails.
    #[test]
    fn a_command_that_does_not_exist_gives_127() {
        let (code, _out, _err) =
            crate::process::run("grasm_sys_no_such_program", &[]).unwrap();
        assert_eq!(code, 127);
    }
}
