//! grasm_rt — the native runtime library linked into `grasm build --native`
//! executables. Everything here is `extern "C"` and callable from Cranelift-
//! generated code.

use std::io::Write;

// ---------------------------------------------------------------------------
// Output (print / interpolation are lowered to piecewise `out` calls)
// ---------------------------------------------------------------------------

/// # Safety
/// `ptr` must point to `len` bytes of valid UTF-8 (guaranteed: the compiler
/// only passes string-literal data it embedded itself).
#[no_mangle]
pub unsafe extern "C" fn grasm_out_text(ptr: *const u8, len: u64) {
    let bytes = std::slice::from_raw_parts(ptr, len as usize);
    let mut out = std::io::stdout().lock();
    let _ = out.write_all(bytes);
}

#[no_mangle]
pub extern "C" fn grasm_out_i64(v: i64) {
    let mut out = std::io::stdout().lock();
    let _ = write!(out, "{v}");
}

#[no_mangle]
pub extern "C" fn grasm_out_f64(v: f64) {
    let mut out = std::io::stdout().lock();
    if v.fract() == 0.0 && v.is_finite() && v.abs() < 1e15 {
        let _ = write!(out, "{v:.1}");
    } else {
        let _ = write!(out, "{v}");
    }
}

#[no_mangle]
pub extern "C" fn grasm_out_bool(v: i8) {
    let mut out = std::io::stdout().lock();
    let _ = write!(out, "{}", v != 0);
}

#[no_mangle]
pub extern "C" fn grasm_out_nl() {
    let mut out = std::io::stdout().lock();
    let _ = out.write_all(b"\n");
    let _ = out.flush();
}

// ---------------------------------------------------------------------------
// Panics (clean message + exit 101, matching the interpreter)
// ---------------------------------------------------------------------------

fn rt_panic(msg: &str) -> ! {
    let mut out = std::io::stdout().lock();
    let _ = writeln!(out, "panic: {msg}");
    let _ = out.flush();
    std::process::exit(101);
}

#[no_mangle]
pub extern "C" fn grasm_panic_div_zero() {
    rt_panic("division by zero");
}

#[no_mangle]
pub extern "C" fn grasm_panic_rem_zero() {
    rt_panic("remainder by zero");
}

#[no_mangle]
pub extern "C" fn grasm_panic_assert() {
    rt_panic("assertion failed");
}

#[no_mangle]
pub extern "C" fn grasm_panic_neg_pow() {
    rt_panic("negative integer exponent (use floats)");
}

/// # Safety
/// `ptr`/`len` as in `grasm_out_text`.
#[no_mangle]
pub unsafe extern "C" fn grasm_panic_msg(ptr: *const u8, len: u64) {
    let bytes = std::slice::from_raw_parts(ptr, len as usize);
    rt_panic(&String::from_utf8_lossy(bytes));
}

// ---------------------------------------------------------------------------
// Integer power (matches interpreter semantics: wrapping, negative exp panics)
// ---------------------------------------------------------------------------

#[no_mangle]
pub extern "C" fn grasm_pow_i64(base: i64, exp: i64) -> i64 {
    if exp < 0 {
        grasm_panic_neg_pow();
    }
    base.wrapping_pow(exp.min(u32::MAX as i64) as u32)
}

#[no_mangle]
pub extern "C" fn grasm_pow_f64(base: f64, exp: f64) -> f64 {
    base.powf(exp)
}

// ---------------------------------------------------------------------------
// Boxed values (N2): refcounted str/list with copy-on-write semantics.
// Codegen passes these as opaque pointers (`i64`-sized).
// ---------------------------------------------------------------------------

use std::cell::Cell;

pub enum RtPayload {
    I64(i64),
    F64(f64),
    Bool(bool),
    Str(String),
    List(Vec<*mut RtVal>),
}

pub struct RtVal {
    pub rc: Cell<i64>,
    pub payload: RtPayload,
}

fn alloc(payload: RtPayload) -> *mut RtVal {
    Box::into_raw(Box::new(RtVal {
        rc: Cell::new(1),
        payload,
    }))
}

/// # Safety
/// `v` must be a live pointer produced by this runtime.
unsafe fn inc(v: *mut RtVal) {
    if !v.is_null() {
        let rc = &(*v).rc;
        rc.set(rc.get() + 1);
    }
}

/// # Safety
/// `v` must be a live pointer produced by this runtime.
unsafe fn dec(v: *mut RtVal) {
    if v.is_null() {
        return;
    }
    let rc = &(*v).rc;
    rc.set(rc.get() - 1);
    if rc.get() <= 0 {
        let boxed = Box::from_raw(v);
        if let RtPayload::List(items) = &boxed.payload {
            for &it in items {
                dec(it);
            }
        }
    }
}

#[no_mangle]
pub unsafe extern "C" fn grasm_val_inc(v: *mut RtVal) {
    inc(v);
}

#[no_mangle]
pub unsafe extern "C" fn grasm_val_dec(v: *mut RtVal) {
    dec(v);
}

#[no_mangle]
pub unsafe extern "C" fn grasm_str_lit(ptr: *const u8, len: u64) -> *mut RtVal {
    let bytes = std::slice::from_raw_parts(ptr, len as usize);
    alloc(RtPayload::Str(String::from_utf8_lossy(bytes).into_owned()))
}

#[no_mangle]
pub unsafe extern "C" fn grasm_str_concat(a: *mut RtVal, b: *mut RtVal) -> *mut RtVal {
    let (RtPayload::Str(sa), RtPayload::Str(sb)) = (&(*a).payload, &(*b).payload) else {
        grasm_panic_msg_str("concat on non-strings");
    };
    alloc(RtPayload::Str(format!("{sa}{sb}")))
}

#[no_mangle]
pub unsafe extern "C" fn grasm_str_from_i64(v: i64) -> *mut RtVal {
    alloc(RtPayload::Str(v.to_string()))
}

#[no_mangle]
pub unsafe extern "C" fn grasm_str_from_f64(v: f64) -> *mut RtVal {
    let s = if v.fract() == 0.0 && v.is_finite() && v.abs() < 1e15 {
        format!("{v:.1}")
    } else {
        format!("{v}")
    };
    alloc(RtPayload::Str(s))
}

#[no_mangle]
pub unsafe extern "C" fn grasm_str_from_bool(v: i8) -> *mut RtVal {
    alloc(RtPayload::Str((v != 0).to_string()))
}

#[no_mangle]
pub unsafe extern "C" fn grasm_str_len(v: *mut RtVal) -> i64 {
    match &(*v).payload {
        RtPayload::Str(s) => s.chars().count() as i64,
        _ => grasm_panic_msg_str("len on non-string"),
    }
}

#[no_mangle]
pub unsafe extern "C" fn grasm_str_eq(a: *mut RtVal, b: *mut RtVal) -> i8 {
    match (&(*a).payload, &(*b).payload) {
        (RtPayload::Str(x), RtPayload::Str(y)) => i8::from(x == y),
        _ => 0,
    }
}

#[no_mangle]
pub unsafe extern "C" fn grasm_str_cmp(a: *mut RtVal, b: *mut RtVal) -> i64 {
    match (&(*a).payload, &(*b).payload) {
        (RtPayload::Str(x), RtPayload::Str(y)) => match x.cmp(y) {
            std::cmp::Ordering::Less => -1,
            std::cmp::Ordering::Equal => 0,
            std::cmp::Ordering::Greater => 1,
        },
        _ => 0,
    }
}

fn grasm_panic_msg_str(msg: &str) -> ! {
    rt_panic(msg);
}

// --- lists ---

#[no_mangle]
pub extern "C" fn grasm_list_new() -> *mut RtVal {
    alloc(RtPayload::List(Vec::new()))
}

/// Push honoring copy-on-write: returns the (possibly new) list pointer.
/// # Safety
/// live runtime pointers.
#[no_mangle]
pub unsafe extern "C" fn grasm_list_push(list: *mut RtVal, elem: *mut RtVal) -> *mut RtVal {
    let target = cow_list(list);
    let RtPayload::List(items) = &mut (*target).payload else {
        unreachable!()
    };
    items.push(elem); // elem ownership transfers (caller inc'd for us)
    target
}

unsafe fn cow_list(list: *mut RtVal) -> *mut RtVal {
    if (*list).rc.get() == 1 {
        return list;
    }
    let RtPayload::List(items) = &(*list).payload else {
        grasm_panic_msg_str("list op on non-list");
    };
    let cloned: Vec<*mut RtVal> = items
        .iter()
        .map(|&it| {
            inc(it);
            it
        })
        .collect();
    dec(list);
    alloc(RtPayload::List(cloned))
}

#[no_mangle]
pub unsafe extern "C" fn grasm_list_len(v: *mut RtVal) -> i64 {
    match &(*v).payload {
        RtPayload::List(items) => items.len() as i64,
        _ => grasm_panic_msg_str("len on non-list"),
    }
}

/// Returns element (+1 rc for the caller); panics on OOB like the interpreter.
/// # Safety
/// live runtime pointers.
#[no_mangle]
pub unsafe extern "C" fn grasm_list_get(v: *mut RtVal, i: i64) -> *mut RtVal {
    match &(*v).payload {
        RtPayload::List(items) => {
            if i < 0 || i as usize >= items.len() {
                rt_panic(&format!("index {i} out of bounds (len {})", items.len()));
            }
            let e = items[i as usize];
            inc(e);
            e
        }
        _ => grasm_panic_msg_str("index on non-list"),
    }
}

// Boxed scalars: list elements are uniformly boxed in N2.

#[no_mangle]
pub extern "C" fn grasm_box_i64(v: i64) -> *mut RtVal {
    alloc(RtPayload::I64(v))
}

#[no_mangle]
pub extern "C" fn grasm_box_f64(v: f64) -> *mut RtVal {
    alloc(RtPayload::F64(v))
}

#[no_mangle]
pub extern "C" fn grasm_box_bool(v: i8) -> *mut RtVal {
    alloc(RtPayload::Bool(v != 0))
}

/// # Safety
/// live runtime pointer of the right payload kind (checker-verified types).
#[no_mangle]
pub unsafe extern "C" fn grasm_unbox_i64(v: *mut RtVal) -> i64 {
    match &(*v).payload {
        RtPayload::I64(x) => *x,
        _ => grasm_panic_msg_str("unbox: not an i64"),
    }
}

/// # Safety
/// as above.
#[no_mangle]
pub unsafe extern "C" fn grasm_unbox_f64(v: *mut RtVal) -> f64 {
    match &(*v).payload {
        RtPayload::F64(x) => *x,
        _ => grasm_panic_msg_str("unbox: not an f64"),
    }
}

/// # Safety
/// as above.
#[no_mangle]
pub unsafe extern "C" fn grasm_unbox_bool(v: *mut RtVal) -> i8 {
    match &(*v).payload {
        RtPayload::Bool(x) => i8::from(*x),
        _ => grasm_panic_msg_str("unbox: not a bool"),
    }
}

// --- printing boxed values ---

/// # Safety
/// live runtime pointer.
#[no_mangle]
pub unsafe extern "C" fn grasm_out_val(v: *mut RtVal) {
    let mut out = std::io::stdout().lock();
    let _ = write!(out, "{}", display(v));
}

unsafe fn display(v: *mut RtVal) -> String {
    match &(*v).payload {
        RtPayload::I64(x) => x.to_string(),
        RtPayload::F64(x) => {
            if x.fract() == 0.0 && x.is_finite() && x.abs() < 1e15 {
                format!("{x:.1}")
            } else {
                format!("{x}")
            }
        }
        RtPayload::Bool(x) => x.to_string(),
        RtPayload::Str(s) => s.clone(),
        RtPayload::List(items) => {
            let parts: Vec<String> = items.iter().map(|&i| display(i)).collect();
            format!("[{}]", parts.join(", "))
        }
    }
}
