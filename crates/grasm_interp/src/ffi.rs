//! Calling C functions from the interpreter.
//!
//! `grasm build --native` resolves an `extern fn` at link time. The
//! interpreter has to find it at run time, and it does so against the
//! process's own symbol table — which already contains libc, because the
//! interpreter is itself a C-linked program.
//!
//! Signatures are restricted to `i64` and `f64` ([T-EXTERN]). Both are passed
//! in their own register class on every ABI Grasm targets, so a transmute to
//! the matching `extern "C" fn` type is exact. There is no variadic support
//! and no libffi: the enumeration below is the whole of what can be called,
//! and anything outside it is reported rather than guessed at.

use std::ffi::c_void;
#[cfg(not(target_arch = "wasm32"))]
use std::ffi::{c_char, CString};

#[cfg(not(target_arch = "wasm32"))]
unsafe extern "C" {
    fn dlsym(handle: *mut c_void, symbol: *const c_char) -> *mut c_void;
}

/// Search every symbol already loaded into this process.
#[cfg(all(target_os = "macos", not(target_arch = "wasm32")))]
const SELF: *mut c_void = -2isize as *mut c_void;
#[cfg(all(not(target_os = "macos"), not(target_arch = "wasm32")))]
const SELF: *mut c_void = std::ptr::null_mut();

/// One argument's kind. `bool` is not here on purpose — see the checker.
#[derive(Clone, Copy, PartialEq, Debug)]
pub enum CKind {
    I64,
    F64,
}

/// An argument, already converted to its C representation.
#[derive(Clone, Copy)]
pub enum CArg {
    I64(i64),
    F64(f64),
}

/// What a C function answered with.
pub enum CRet {
    I64(i64),
    F64(f64),
    Unit,
}

/// Looks up `name` in the running process.
#[cfg(not(target_arch = "wasm32"))]
pub fn find(name: &str) -> Option<*mut c_void> {
    let c = CString::new(name).ok()?;
    // SAFETY: `c` is a valid NUL-terminated string for the duration of the
    // call, and `dlsym` only reads it.
    let p = unsafe { dlsym(SELF, c.as_ptr()) };
    if p.is_null() {
        None
    } else {
        Some(p)
    }
}

/// A wasm module has no process symbol table to search, so no `extern fn`
/// resolves there and every lookup answers "not found" — the same answer a
/// missing symbol gets natively, and the one a caller already handles.
///
/// Declaring `dlsym` anyway left it as an undefined symbol in the module, and
/// `wasm32-unknown-unknown` stopped tolerating those: the link failed with
/// "undefined symbol: dlsym" and the whole wasm engine went unbuildable.
#[cfg(target_arch = "wasm32")]
pub fn find(_name: &str) -> Option<*mut c_void> {
    None
}

macro_rules! call {
    ($sym:expr, $ret:ty, ($($t:ty),*), ($($a:expr),*)) => {{
        // SAFETY: the transmute target is built from the declared signature,
        // which the checker restricted to `i64`/`f64`. Those occupy the same
        // register classes here as in the compiled backend, so the two engines
        // make the identical call.
        let f: extern "C" fn($($t),*) -> $ret = unsafe { std::mem::transmute($sym) };
        f($($a),*)
    }};
}

/// Calls `sym` with `args`, or `None` if the shape is outside what this
/// supports. The caller reports that rather than inventing an answer.
///
/// # Safety
///
/// `sym` must be a function pointer whose C signature is exactly the one
/// `args` and `ret` describe. `find` returns one for a name the program
/// declared; the checker restricts the declared types to `i64` and `f64`, so
/// the transmuted type below is that signature and nothing else.
pub unsafe fn call(sym: *mut c_void, args: &[CArg], ret: Option<CKind>) -> Option<CRet> {
    use CArg::*;
    Some(match (args, ret) {
        ([], None) => {
            call!(sym, (), (), ());
            CRet::Unit
        }
        ([], Some(CKind::I64)) => CRet::I64(call!(sym, i64, (), ())),
        ([], Some(CKind::F64)) => CRet::F64(call!(sym, f64, (), ())),

        ([I64(a)], None) => {
            call!(sym, (), (i64), (*a));
            CRet::Unit
        }
        ([I64(a)], Some(CKind::I64)) => CRet::I64(call!(sym, i64, (i64), (*a))),
        ([I64(a)], Some(CKind::F64)) => CRet::F64(call!(sym, f64, (i64), (*a))),
        ([F64(a)], None) => {
            call!(sym, (), (f64), (*a));
            CRet::Unit
        }
        ([F64(a)], Some(CKind::I64)) => CRet::I64(call!(sym, i64, (f64), (*a))),
        ([F64(a)], Some(CKind::F64)) => CRet::F64(call!(sym, f64, (f64), (*a))),

        ([I64(a), I64(b)], Some(CKind::I64)) => CRet::I64(call!(sym, i64, (i64, i64), (*a, *b))),
        ([I64(a), I64(b)], Some(CKind::F64)) => CRet::F64(call!(sym, f64, (i64, i64), (*a, *b))),
        ([I64(a), F64(b)], Some(CKind::I64)) => CRet::I64(call!(sym, i64, (i64, f64), (*a, *b))),
        ([I64(a), F64(b)], Some(CKind::F64)) => CRet::F64(call!(sym, f64, (i64, f64), (*a, *b))),
        ([F64(a), I64(b)], Some(CKind::I64)) => CRet::I64(call!(sym, i64, (f64, i64), (*a, *b))),
        ([F64(a), I64(b)], Some(CKind::F64)) => CRet::F64(call!(sym, f64, (f64, i64), (*a, *b))),
        ([F64(a), F64(b)], Some(CKind::I64)) => CRet::I64(call!(sym, i64, (f64, f64), (*a, *b))),
        ([F64(a), F64(b)], Some(CKind::F64)) => CRet::F64(call!(sym, f64, (f64, f64), (*a, *b))),
        ([I64(a), I64(b)], None) => {
            call!(sym, (), (i64, i64), (*a, *b));
            CRet::Unit
        }
        ([F64(a), F64(b)], None) => {
            call!(sym, (), (f64, f64), (*a, *b));
            CRet::Unit
        }

        ([I64(a), I64(b), I64(c)], Some(CKind::I64)) => {
            CRet::I64(call!(sym, i64, (i64, i64, i64), (*a, *b, *c)))
        }
        ([F64(a), F64(b), F64(c)], Some(CKind::F64)) => {
            CRet::F64(call!(sym, f64, (f64, f64, f64), (*a, *b, *c)))
        }
        _ => return None,
    })
}
