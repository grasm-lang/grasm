//! grasm_rt — the native runtime library linked into `grasm build --native`
//! executables. Everything here is `extern "C"` and callable from Cranelift-
//! generated code.
//!
//! Safety contract for every `unsafe extern "C"` entry point below: the only
//! callers are Cranelift-emitted instructions from `grasm_native`, which pass
//! pointers this runtime itself produced (and keeps alive via refcounts) plus
//! interned UTF-8 name/data blobs the compiler embedded. Callers outside that
//! contract are not supported.
#![allow(clippy::missing_safety_doc)]

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
    Map(Vec<(*mut RtVal, *mut RtVal)>),
    Struct {
        name: String,
        fields: Vec<(String, *mut RtVal)>,
    },
    Variant {
        type_name: String,
        name: String,
        fields: Vec<*mut RtVal>,
    },
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
        match &boxed.payload {
            RtPayload::List(items) => {
                for &it in items {
                    dec(it);
                }
            }
            RtPayload::Map(entries) => {
                for &(k, v) in entries {
                    dec(k);
                    dec(v);
                }
            }
            RtPayload::Struct { fields, .. } => {
                for (_, fv) in fields {
                    dec(*fv);
                }
            }
            RtPayload::Variant { fields, .. } => {
                for &fv in fields {
                    dec(fv);
                }
            }
            _ => {}
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
        RtPayload::Struct { name, fields } => {
            let parts: Vec<String> = fields
                .iter()
                .map(|(k, fv)| format!("{k}: {}", display(*fv)))
                .collect();
            format!("{name}{{{}}}", parts.join(", "))
        }
        RtPayload::Variant { name, fields, .. } => {
            if fields.is_empty() {
                name.clone()
            } else {
                let parts: Vec<String> = fields.iter().map(|&f| display(f)).collect();
                format!("{name}({})", parts.join(", "))
            }
        }
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
        RtPayload::Map(entries) => {
            let parts: Vec<String> = entries
                .iter()
                .map(|&(k, v)| format!("{}: {}", display(k), display(v)))
                .collect();
            format!("{{{}}}", parts.join(", "))
        }
    }
}

// ---------------------------------------------------------------------------
// N3: structs, variants, fallibles, structural equality
// ---------------------------------------------------------------------------

unsafe fn take_str(ptr: *const u8, len: u64) -> String {
    let bytes = std::slice::from_raw_parts(ptr, len as usize);
    String::from_utf8_lossy(bytes).into_owned()
}

/// # Safety: compiler-generated calls with interned name data.
#[no_mangle]
pub unsafe extern "C" fn grasm_struct_new(name: *const u8, len: u64) -> *mut RtVal {
    alloc(RtPayload::Struct {
        name: take_str(name, len),
        fields: Vec::new(),
    })
}

/// # Safety: as above; `val` ownership transfers into the struct.
#[no_mangle]
pub unsafe extern "C" fn grasm_struct_push_field(
    s: *mut RtVal,
    name: *const u8,
    len: u64,
    val: *mut RtVal,
) {
    let RtPayload::Struct { fields, .. } = &mut (*s).payload else {
        grasm_panic_msg_str("push_field on non-struct");
    };
    fields.push((take_str(name, len), val));
}

/// # Safety: as above; returns +1.
#[no_mangle]
pub unsafe extern "C" fn grasm_struct_get(
    s: *mut RtVal,
    name: *const u8,
    len: u64,
) -> *mut RtVal {
    let RtPayload::Struct { fields, .. } = &(*s).payload else {
        grasm_panic_msg_str("field access on non-struct");
    };
    let key = take_str(name, len);
    for (k, v) in fields {
        if *k == key {
            inc(*v);
            return *v;
        }
    }
    grasm_panic_msg_str("no such field");
}

/// CoW field update: returns the (possibly new) struct pointer.
/// # Safety: as above; `val` ownership transfers.
#[no_mangle]
pub unsafe extern "C" fn grasm_struct_set_field(
    s: *mut RtVal,
    name: *const u8,
    len: u64,
    val: *mut RtVal,
) -> *mut RtVal {
    let target = if (*s).rc.get() == 1 {
        s
    } else {
        let RtPayload::Struct { name: sn, fields } = &(*s).payload else {
            grasm_panic_msg_str("set_field on non-struct");
        };
        let cloned: Vec<(String, *mut RtVal)> = fields
            .iter()
            .map(|(k, v)| {
                inc(*v);
                (k.clone(), *v)
            })
            .collect();
        let nv = alloc(RtPayload::Struct {
            name: sn.clone(),
            fields: cloned,
        });
        dec(s);
        nv
    };
    let RtPayload::Struct { fields, .. } = &mut (*target).payload else {
        unreachable!()
    };
    let key = take_str(name, len);
    for (k, v) in fields.iter_mut() {
        if *k == key {
            dec(*v);
            *v = val;
            return target;
        }
    }
    grasm_panic_msg_str("no such field");
}

/// # Safety: interned names.
#[no_mangle]
pub unsafe extern "C" fn grasm_variant_new(
    tp: *const u8,
    tl: u64,
    vp: *const u8,
    vl: u64,
) -> *mut RtVal {
    alloc(RtPayload::Variant {
        type_name: take_str(tp, tl),
        name: take_str(vp, vl),
        fields: Vec::new(),
    })
}

/// # Safety: `val` ownership transfers.
#[no_mangle]
pub unsafe extern "C" fn grasm_variant_push(v: *mut RtVal, val: *mut RtVal) {
    let RtPayload::Variant { fields, .. } = &mut (*v).payload else {
        grasm_panic_msg_str("variant_push on non-variant");
    };
    fields.push(val);
}

/// # Safety: live pointers.
#[no_mangle]
pub unsafe extern "C" fn grasm_variant_is(v: *mut RtVal, p: *const u8, l: u64) -> i8 {
    match &(*v).payload {
        RtPayload::Variant { name, .. } => i8::from(*name == take_str(p, l)),
        _ => 0,
    }
}

/// # Safety: live pointers; returns +1.
#[no_mangle]
pub unsafe extern "C" fn grasm_variant_field(v: *mut RtVal, i: i64) -> *mut RtVal {
    let RtPayload::Variant { fields, .. } = &(*v).payload else {
        grasm_panic_msg_str("variant_field on non-variant");
    };
    let f = fields[i as usize];
    inc(f);
    f
}

// Fallibles: Variant with type_name "!".

/// # Safety: `val` ownership transfers.
#[no_mangle]
pub unsafe extern "C" fn grasm_ok_new(val: *mut RtVal) -> *mut RtVal {
    alloc(RtPayload::Variant {
        type_name: "!".into(),
        name: "Ok".into(),
        fields: vec![val],
    })
}

/// # Safety: `val` ownership transfers.
#[no_mangle]
pub unsafe extern "C" fn grasm_err_new(val: *mut RtVal) -> *mut RtVal {
    alloc(RtPayload::Variant {
        type_name: "!".into(),
        name: "Err".into(),
        fields: vec![val],
    })
}

/// # Safety: live pointer.
#[no_mangle]
pub unsafe extern "C" fn grasm_is_ok(v: *mut RtVal) -> i8 {
    match &(*v).payload {
        RtPayload::Variant { name, .. } => i8::from(name == "Ok"),
        _ => 0,
    }
}

/// # Safety: live pointer; prints the interpreter-identical line, exits 1.
#[no_mangle]
pub unsafe extern "C" fn grasm_report_main_err(v: *mut RtVal) {
    let RtPayload::Variant { fields, .. } = &(*v).payload else {
        rt_panic("bad main error value");
    };
    let mut out = std::io::stdout().lock();
    let _ = writeln!(out, "error: unhandled error from main: {}", display(fields[0]));
    let _ = out.flush();
    std::process::exit(1);
}

/// Structural equality across all payloads.
/// # Safety: live pointers.
#[no_mangle]
pub unsafe extern "C" fn grasm_val_eq(a: *mut RtVal, b: *mut RtVal) -> i8 {
    i8::from(val_eq(a, b))
}

unsafe fn val_eq(a: *mut RtVal, b: *mut RtVal) -> bool {
    match (&(*a).payload, &(*b).payload) {
        (RtPayload::I64(x), RtPayload::I64(y)) => x == y,
        (RtPayload::F64(x), RtPayload::F64(y)) => x == y,
        (RtPayload::Bool(x), RtPayload::Bool(y)) => x == y,
        (RtPayload::Str(x), RtPayload::Str(y)) => x == y,
        (RtPayload::List(x), RtPayload::List(y)) => {
            x.len() == y.len() && x.iter().zip(y).all(|(&i, &j)| val_eq(i, j))
        }
        (RtPayload::Map(x), RtPayload::Map(y)) => {
            x.len() == y.len()
                && x.iter()
                    .zip(y)
                    .all(|(&(k1, v1), &(k2, v2))| val_eq(k1, k2) && val_eq(v1, v2))
        }
        (
            RtPayload::Struct { name: n1, fields: f1 },
            RtPayload::Struct { name: n2, fields: f2 },
        ) => {
            n1 == n2
                && f1.len() == f2.len()
                && f1
                    .iter()
                    .zip(f2)
                    .all(|((k1, v1), (k2, v2))| k1 == k2 && val_eq(*v1, *v2))
        }
        (
            RtPayload::Variant { type_name: t1, name: n1, fields: f1 },
            RtPayload::Variant { type_name: t2, name: n2, fields: f2 },
        ) => {
            t1 == t2
                && n1 == n2
                && f1.len() == f2.len()
                && f1.iter().zip(f2).all(|(&i, &j)| val_eq(i, j))
        }
        _ => false,
    }
}

/// list.get(i) → Option variant (None on OOB).
/// # Safety: live pointers.
#[no_mangle]
pub unsafe extern "C" fn grasm_list_get_opt(v: *mut RtVal, i: i64) -> *mut RtVal {
    match &(*v).payload {
        RtPayload::List(items) => {
            if i < 0 || i as usize >= items.len() {
                alloc(RtPayload::Variant {
                    type_name: "Option".into(),
                    name: "None".into(),
                    fields: vec![],
                })
            } else {
                let e = items[i as usize];
                inc(e);
                alloc(RtPayload::Variant {
                    type_name: "Option".into(),
                    name: "Some".into(),
                    fields: vec![e],
                })
            }
        }
        _ => grasm_panic_msg_str("get on non-list"),
    }
}

/// Display any boxed value as a string value (for interpolation).
/// # Safety: live pointer.
#[no_mangle]
pub unsafe extern "C" fn grasm_str_from_val(v: *mut RtVal) -> *mut RtVal {
    alloc(RtPayload::Str(display(v)))
}

// ---------------------------------------------------------------------------
// N4: closures, maps, builtin method surface
// ---------------------------------------------------------------------------

/// Closure object: code pointer + captured environment (list of boxed values).
pub struct RtClosure {
    pub rc: Cell<i64>,
    pub code: *const u8,
    pub env: Vec<*mut RtVal>,
}

/// # Safety: `code` is a compiler-emitted function pointer.
#[no_mangle]
pub unsafe extern "C" fn grasm_closure_new(code: *const u8, cap: i64) -> *mut RtClosure {
    Box::into_raw(Box::new(RtClosure {
        rc: Cell::new(1),
        code,
        env: Vec::with_capacity(cap.max(0) as usize),
    }))
}

/// # Safety: `val` ownership transfers into the closure environment.
#[no_mangle]
pub unsafe extern "C" fn grasm_closure_push(c: *mut RtClosure, val: *mut RtVal) {
    let env = &mut (*c).env;
    env.push(val);
}

/// # Safety: live closure; returns the raw code pointer.
#[no_mangle]
pub unsafe extern "C" fn grasm_closure_code(c: *mut RtClosure) -> *const u8 {
    (*c).code
}

/// # Safety: live closure; returns captured slot i WITHOUT an rc bump
/// (callee treats it as borrowed for the call's duration).
#[no_mangle]
pub unsafe extern "C" fn grasm_closure_env(c: *mut RtClosure, i: i64) -> *mut RtVal {
    let env = &(*c).env;
    env[i as usize]
}

#[no_mangle]
pub unsafe extern "C" fn grasm_closure_inc(c: *mut RtClosure) {
    if !c.is_null() {
        let rc = &(*c).rc;
        rc.set(rc.get() + 1);
    }
}

#[no_mangle]
pub unsafe extern "C" fn grasm_closure_dec(c: *mut RtClosure) {
    if c.is_null() {
        return;
    }
    let rc = &(*c).rc;
    rc.set(rc.get() - 1);
    if rc.get() <= 0 {
        let boxed = Box::from_raw(c);
        for &e in &boxed.env {
            dec(e);
        }
    }
}

// --- maps (sorted, deterministic iteration like the interpreter) ---

#[no_mangle]
pub extern "C" fn grasm_map_new() -> *mut RtVal {
    alloc(RtPayload::Map(Vec::new()))
}

unsafe fn map_key_ord(a: &RtVal, b: &RtVal) -> std::cmp::Ordering {
    use std::cmp::Ordering;
    // Interpreter's MapKey order: Bool < Int < Str.
    fn rank(p: &RtPayload) -> u8 {
        match p {
            RtPayload::Bool(_) => 0,
            RtPayload::I64(_) => 1,
            RtPayload::Str(_) => 2,
            _ => 3,
        }
    }
    let (ra, rb) = (rank(&a.payload), rank(&b.payload));
    if ra != rb {
        return ra.cmp(&rb);
    }
    match (&a.payload, &b.payload) {
        (RtPayload::Bool(x), RtPayload::Bool(y)) => x.cmp(y),
        (RtPayload::I64(x), RtPayload::I64(y)) => x.cmp(y),
        (RtPayload::Str(x), RtPayload::Str(y)) => x.cmp(y),
        _ => Ordering::Equal,
    }
}

unsafe fn cow_map(m: *mut RtVal) -> *mut RtVal {
    if (*m).rc.get() == 1 {
        return m;
    }
    let RtPayload::Map(entries) = &(*m).payload else {
        grasm_panic_msg_str("map op on non-map");
    };
    let cloned: Vec<(*mut RtVal, *mut RtVal)> = entries
        .iter()
        .map(|&(k, v)| {
            inc(k);
            inc(v);
            (k, v)
        })
        .collect();
    dec(m);
    alloc(RtPayload::Map(cloned))
}

/// # Safety: key/val ownership transfers; returns the (possibly new) map.
#[no_mangle]
pub unsafe extern "C" fn grasm_map_set(
    m: *mut RtVal,
    k: *mut RtVal,
    v: *mut RtVal,
) -> *mut RtVal {
    let target = cow_map(m);
    let RtPayload::Map(entries) = &mut (*target).payload else {
        unreachable!()
    };
    for (ek, ev) in entries.iter_mut() {
        if map_key_ord(&**ek, &*k) == std::cmp::Ordering::Equal {
            dec(*ev);
            *ev = v;
            dec(k);
            return target;
        }
    }
    let pos = entries
        .iter()
        .position(|(ek, _)| map_key_ord(&**ek, &*k) == std::cmp::Ordering::Greater)
        .unwrap_or(entries.len());
    entries.insert(pos, (k, v));
    target
}

/// # Safety: live pointers; returns Option variant (+1).
#[no_mangle]
pub unsafe extern "C" fn grasm_map_get(m: *mut RtVal, k: *mut RtVal) -> *mut RtVal {
    let RtPayload::Map(entries) = &(*m).payload else {
        grasm_panic_msg_str("get on non-map");
    };
    for (ek, ev) in entries {
        if map_key_ord(&**ek, &*k) == std::cmp::Ordering::Equal {
            inc(*ev);
            return alloc(RtPayload::Variant {
                type_name: "Option".into(),
                name: "Some".into(),
                fields: vec![*ev],
            });
        }
    }
    alloc(RtPayload::Variant {
        type_name: "Option".into(),
        name: "None".into(),
        fields: vec![],
    })
}

/// # Safety: live pointers.
#[no_mangle]
pub unsafe extern "C" fn grasm_map_has(m: *mut RtVal, k: *mut RtVal) -> i8 {
    let RtPayload::Map(entries) = &(*m).payload else {
        grasm_panic_msg_str("has on non-map");
    };
    i8::from(
        entries
            .iter()
            .any(|(ek, _)| map_key_ord(&**ek, &*k) == std::cmp::Ordering::Equal),
    )
}

/// # Safety: live pointers; returns (map, removed?) via out-param style: the
/// new map pointer is returned, `found` receives 1/0.
#[no_mangle]
pub unsafe extern "C" fn grasm_map_delete(
    m: *mut RtVal,
    k: *mut RtVal,
    found: *mut i8,
) -> *mut RtVal {
    let target = cow_map(m);
    let RtPayload::Map(entries) = &mut (*target).payload else {
        unreachable!()
    };
    let pos = entries
        .iter()
        .position(|(ek, _)| map_key_ord(&**ek, &*k) == std::cmp::Ordering::Equal);
    match pos {
        Some(i) => {
            let (ek, ev) = entries.remove(i);
            dec(ek);
            dec(ev);
            *found = 1;
        }
        None => *found = 0,
    }
    target
}

/// # Safety: live pointer.
#[no_mangle]
pub unsafe extern "C" fn grasm_map_len(m: *mut RtVal) -> i64 {
    match &(*m).payload {
        RtPayload::Map(entries) => entries.len() as i64,
        _ => grasm_panic_msg_str("len on non-map"),
    }
}

/// # Safety: live pointer; returns a fresh list (+1).
#[no_mangle]
pub unsafe extern "C" fn grasm_map_keys(m: *mut RtVal) -> *mut RtVal {
    let RtPayload::Map(entries) = &(*m).payload else {
        grasm_panic_msg_str("keys on non-map");
    };
    let items: Vec<*mut RtVal> = entries
        .iter()
        .map(|&(k, _)| {
            inc(k);
            k
        })
        .collect();
    alloc(RtPayload::List(items))
}

/// # Safety: live pointer; returns a fresh list (+1).
#[no_mangle]
pub unsafe extern "C" fn grasm_map_values(m: *mut RtVal) -> *mut RtVal {
    let RtPayload::Map(entries) = &(*m).payload else {
        grasm_panic_msg_str("values on non-map");
    };
    let items: Vec<*mut RtVal> = entries
        .iter()
        .map(|&(_, v)| {
            inc(v);
            v
        })
        .collect();
    alloc(RtPayload::List(items))
}

// --- N4: full builtin method surface (str / list / numeric) ---

unsafe fn as_str(v: *mut RtVal) -> &'static str {
    match &(*v).payload {
        RtPayload::Str(s) => {
            // Lifetime is tied to the caller's borrow; safe for the call span.
            std::mem::transmute::<&str, &'static str>(s.as_str())
        }
        _ => grasm_panic_msg_str("expected a string"),
    }
}

macro_rules! str_pred {
    ($name:ident, $m:ident) => {
        /// # Safety: live string pointers.
        #[no_mangle]
        pub unsafe extern "C" fn $name(a: *mut RtVal, b: *mut RtVal) -> i8 {
            i8::from(as_str(a).$m(as_str(b)))
        }
    };
}
str_pred!(grasm_str_contains, contains);
str_pred!(grasm_str_starts_with, starts_with);
str_pred!(grasm_str_ends_with, ends_with);

/// # Safety: live pointers; returns a fresh list of strings.
#[no_mangle]
pub unsafe extern "C" fn grasm_str_split(s: *mut RtVal, sep: *mut RtVal) -> *mut RtVal {
    let text = as_str(s);
    let sp = as_str(sep);
    let parts: Vec<*mut RtVal> = if sp.is_empty() {
        text.chars()
            .map(|c| alloc(RtPayload::Str(c.to_string())))
            .collect()
    } else {
        text.split(sp)
            .map(|p| alloc(RtPayload::Str(p.to_string())))
            .collect()
    };
    alloc(RtPayload::List(parts))
}

/// # Safety: live pointer.
#[no_mangle]
pub unsafe extern "C" fn grasm_str_chars(s: *mut RtVal) -> *mut RtVal {
    let items: Vec<*mut RtVal> = as_str(s)
        .chars()
        .map(|c| alloc(RtPayload::Str(c.to_string())))
        .collect();
    alloc(RtPayload::List(items))
}

/// # Safety: live pointer.
#[no_mangle]
pub unsafe extern "C" fn grasm_str_trim(s: *mut RtVal) -> *mut RtVal {
    alloc(RtPayload::Str(as_str(s).trim().to_string()))
}

/// # Safety: live pointer.
#[no_mangle]
pub unsafe extern "C" fn grasm_str_upper(s: *mut RtVal) -> *mut RtVal {
    alloc(RtPayload::Str(as_str(s).to_uppercase()))
}

/// # Safety: live pointer.
#[no_mangle]
pub unsafe extern "C" fn grasm_str_lower(s: *mut RtVal) -> *mut RtVal {
    alloc(RtPayload::Str(as_str(s).to_lowercase()))
}

/// # Safety: live pointer.
#[no_mangle]
pub unsafe extern "C" fn grasm_str_repeat(s: *mut RtVal, n: i64) -> *mut RtVal {
    if n < 0 {
        rt_panic("repeat count must be a non-negative int");
    }
    alloc(RtPayload::Str(as_str(s).repeat(n as usize)))
}

/// # Safety: live pointers.
#[no_mangle]
pub unsafe extern "C" fn grasm_str_replace(
    s: *mut RtVal,
    from: *mut RtVal,
    to: *mut RtVal,
) -> *mut RtVal {
    alloc(RtPayload::Str(as_str(s).replace(as_str(from), as_str(to))))
}

/// # Safety: live pointer.
#[no_mangle]
pub unsafe extern "C" fn grasm_str_substr(s: *mut RtVal, start: i64, len: i64) -> *mut RtVal {
    let chars: Vec<char> = as_str(s).chars().collect();
    let n = chars.len() as i64;
    let start = start.clamp(0, n) as usize;
    let end = (start as i64 + len.max(0)).clamp(0, n) as usize;
    alloc(RtPayload::Str(chars[start..end].iter().collect::<String>()))
}

/// # Safety: live pointers; returns Option[i64] (char index).
#[no_mangle]
pub unsafe extern "C" fn grasm_str_index_of(s: *mut RtVal, needle: *mut RtVal) -> *mut RtVal {
    let text = as_str(s);
    match text.find(as_str(needle)) {
        Some(byte_idx) => {
            let char_idx = text[..byte_idx].chars().count() as i64;
            some_of(alloc(RtPayload::I64(char_idx)))
        }
        None => none_val(),
    }
}

fn none_val() -> *mut RtVal {
    alloc(RtPayload::Variant {
        type_name: "Option".into(),
        name: "None".into(),
        fields: vec![],
    })
}

fn some_of(v: *mut RtVal) -> *mut RtVal {
    alloc(RtPayload::Variant {
        type_name: "Option".into(),
        name: "Some".into(),
        fields: vec![v],
    })
}

/// # Safety: live pointer.
#[no_mangle]
pub unsafe extern "C" fn grasm_str_parse_int(s: *mut RtVal) -> *mut RtVal {
    match as_str(s).trim().parse::<i64>() {
        Ok(v) => some_of(alloc(RtPayload::I64(v))),
        Err(_) => none_val(),
    }
}

/// # Safety: live pointer.
#[no_mangle]
pub unsafe extern "C" fn grasm_str_parse_float(s: *mut RtVal) -> *mut RtVal {
    match as_str(s).trim().parse::<f64>() {
        Ok(v) => some_of(alloc(RtPayload::F64(v))),
        Err(_) => none_val(),
    }
}

unsafe fn list_items(v: *mut RtVal) -> &'static Vec<*mut RtVal> {
    match &(*v).payload {
        RtPayload::List(items) => {
            std::mem::transmute::<&Vec<*mut RtVal>, &'static Vec<*mut RtVal>>(items)
        }
        _ => grasm_panic_msg_str("expected a list"),
    }
}

/// # Safety: live pointers.
#[no_mangle]
pub unsafe extern "C" fn grasm_list_join(v: *mut RtVal, sep: *mut RtVal) -> *mut RtVal {
    let parts: Vec<String> = list_items(v).iter().map(|&i| display(i)).collect();
    alloc(RtPayload::Str(parts.join(as_str(sep))))
}

/// # Safety: live pointer.
#[no_mangle]
pub unsafe extern "C" fn grasm_list_rev(v: *mut RtVal) -> *mut RtVal {
    let mut items: Vec<*mut RtVal> = list_items(v)
        .iter()
        .map(|&i| {
            inc(i);
            i
        })
        .collect();
    items.reverse();
    alloc(RtPayload::List(items))
}

unsafe fn cmp_vals(a: *mut RtVal, b: *mut RtVal) -> std::cmp::Ordering {
    use std::cmp::Ordering;
    match (&(*a).payload, &(*b).payload) {
        (RtPayload::I64(x), RtPayload::I64(y)) => x.cmp(y),
        (RtPayload::F64(x), RtPayload::F64(y)) => x.partial_cmp(y).unwrap_or(Ordering::Equal),
        (RtPayload::Str(x), RtPayload::Str(y)) => x.cmp(y),
        _ => Ordering::Equal,
    }
}

/// In-place sort honoring CoW; returns the (possibly new) list.
/// # Safety: live pointer.
#[no_mangle]
pub unsafe extern "C" fn grasm_list_sort(v: *mut RtVal) -> *mut RtVal {
    let target = cow_list(v);
    let RtPayload::List(items) = &mut (*target).payload else {
        unreachable!()
    };
    items.sort_by(|&a, &b| cmp_vals(a, b));
    target
}

/// # Safety: live pointer.
#[no_mangle]
pub unsafe extern "C" fn grasm_list_slice(v: *mut RtVal, a: i64, b: i64) -> *mut RtVal {
    let items = list_items(v);
    let len = items.len() as i64;
    let s = a.clamp(0, len) as usize;
    let e = b.clamp(0, len) as usize;
    let out: Vec<*mut RtVal> = if s <= e {
        items[s..e]
            .iter()
            .map(|&i| {
                inc(i);
                i
            })
            .collect()
    } else {
        Vec::new()
    };
    alloc(RtPayload::List(out))
}

/// # Safety: live pointers.
#[no_mangle]
pub unsafe extern "C" fn grasm_list_contains(v: *mut RtVal, needle: *mut RtVal) -> i8 {
    i8::from(list_items(v).iter().any(|&i| val_eq(i, needle)))
}

/// Pop honoring CoW: returns the new list; `out` receives the popped Option.
/// # Safety: live pointers.
#[no_mangle]
pub unsafe extern "C" fn grasm_list_pop(v: *mut RtVal, out: *mut *mut RtVal) -> *mut RtVal {
    let target = cow_list(v);
    let RtPayload::List(items) = &mut (*target).payload else {
        unreachable!()
    };
    *out = match items.pop() {
        Some(e) => some_of(e),
        None => none_val(),
    };
    target
}

/// # Safety: live pointer.
#[no_mangle]
pub unsafe extern "C" fn grasm_list_first(v: *mut RtVal) -> *mut RtVal {
    match list_items(v).first() {
        Some(&e) => {
            inc(e);
            some_of(e)
        }
        None => none_val(),
    }
}

/// # Safety: live pointer.
#[no_mangle]
pub unsafe extern "C" fn grasm_list_last(v: *mut RtVal) -> *mut RtVal {
    match list_items(v).last() {
        Some(&e) => {
            inc(e);
            some_of(e)
        }
        None => none_val(),
    }
}

/// # Safety: live pointer.
#[no_mangle]
pub unsafe extern "C" fn grasm_list_sum_i64(v: *mut RtVal) -> i64 {
    let mut total = 0i64;
    for &i in list_items(v) {
        if let RtPayload::I64(x) = &(*i).payload {
            total += x;
        }
    }
    total
}

/// # Safety: live pointer.
#[no_mangle]
pub unsafe extern "C" fn grasm_list_sum_f64(v: *mut RtVal) -> f64 {
    let mut total = 0f64;
    for &i in list_items(v) {
        match &(*i).payload {
            RtPayload::F64(x) => total += x,
            RtPayload::I64(x) => total += *x as f64,
            _ => {}
        }
    }
    total
}

#[no_mangle]
pub extern "C" fn grasm_i64_abs(v: i64) -> i64 {
    v.abs()
}
#[no_mangle]
pub extern "C" fn grasm_i64_min(a: i64, b: i64) -> i64 {
    a.min(b)
}
#[no_mangle]
pub extern "C" fn grasm_i64_max(a: i64, b: i64) -> i64 {
    a.max(b)
}
#[no_mangle]
pub extern "C" fn grasm_f64_abs(v: f64) -> f64 {
    v.abs()
}
#[no_mangle]
pub extern "C" fn grasm_f64_sqrt(v: f64) -> f64 {
    v.sqrt()
}
#[no_mangle]
pub extern "C" fn grasm_f64_floor(v: f64) -> f64 {
    v.floor()
}
#[no_mangle]
pub extern "C" fn grasm_f64_ceil(v: f64) -> f64 {
    v.ceil()
}
#[no_mangle]
pub extern "C" fn grasm_f64_round(v: f64) -> f64 {
    v.round()
}
#[no_mangle]
pub extern "C" fn grasm_f64_min(a: f64, b: f64) -> f64 {
    a.min(b)
}
#[no_mangle]
pub extern "C" fn grasm_f64_max(a: f64, b: f64) -> f64 {
    a.max(b)
}

// ---------------------------------------------------------------------------
// N5: effect handler stack + fs/args/env builtins
// ---------------------------------------------------------------------------

use std::cell::RefCell;

/// One installed effect handler: its ops as (op name, code pointer).
type HandlerFrame = (String, Vec<(String, *const u8)>);

thread_local! {
    /// Innermost-last stack of installed handlers.
    static HANDLERS: RefCell<Vec<HandlerFrame>> = const { RefCell::new(Vec::new()) };
    static PROGRAM_ARGS: RefCell<Vec<String>> = const { RefCell::new(Vec::new()) };
}

#[no_mangle]
pub unsafe extern "C" fn grasm_handler_push(name: *const u8, len: u64) {
    let n = take_str(name, len);
    HANDLERS.with(|h| h.borrow_mut().push((n, Vec::new())));
}

#[no_mangle]
pub unsafe extern "C" fn grasm_handler_op(name: *const u8, len: u64, code: *const u8) {
    let n = take_str(name, len);
    HANDLERS.with(|h| {
        if let Some((_, ops)) = h.borrow_mut().last_mut() {
            ops.push((n, code));
        }
    });
}

#[no_mangle]
pub extern "C" fn grasm_handler_pop() {
    HANDLERS.with(|h| {
        h.borrow_mut().pop();
    });
}

/// Resolves an effect operation to its handler code pointer (innermost wins).
#[no_mangle]
pub unsafe extern "C" fn grasm_handler_lookup(
    ep: *const u8,
    el: u64,
    op: *const u8,
    ol: u64,
) -> *const u8 {
    let effect = take_str(ep, el);
    let opname = take_str(op, ol);
    let found = HANDLERS.with(|h| {
        h.borrow()
            .iter()
            .rev()
            .find(|(e, _)| *e == effect)
            .and_then(|(_, ops)| {
                ops.iter()
                    .find(|(o, _)| *o == opname)
                    .map(|(_, c)| *c)
            })
    });
    match found {
        Some(c) => c,
        None => rt_panic(&format!(
            "unhandled effect `{effect}.{opname}` — no handler installed"
        )),
    }
}

// --- program args / stdin ---

#[no_mangle]
pub unsafe extern "C" fn grasm_set_args(argc: i64, argv: *const *const u8) {
    let mut out = Vec::new();
    for i in 0..argc.max(0) {
        let p = *argv.offset(i as isize);
        let mut len = 0usize;
        while *p.add(len) != 0 {
            len += 1;
        }
        out.push(take_str(p, len as u64));
    }
    PROGRAM_ARGS.with(|a| *a.borrow_mut() = out);
}

#[no_mangle]
pub extern "C" fn grasm_args() -> *mut RtVal {
    let items: Vec<*mut RtVal> = PROGRAM_ARGS.with(|a| {
        a.borrow()
            .iter()
            .map(|s| alloc(RtPayload::Str(s.clone())))
            .collect()
    });
    alloc(RtPayload::List(items))
}

#[no_mangle]
pub extern "C" fn grasm_input() -> *mut RtVal {
    let mut line = String::new();
    let _ = std::io::BufRead::read_line(&mut std::io::stdin().lock(), &mut line);
    while line.ends_with('\n') || line.ends_with('\r') {
        line.pop();
    }
    alloc(RtPayload::Str(line))
}

// --- filesystem (fallible: returns Ok/Err variants) ---

fn io_err(variant: &str, payload: String) -> *mut RtVal {
    let inner = alloc(RtPayload::Str(payload));
    alloc(RtPayload::Variant {
        type_name: "IoError".into(),
        name: variant.into(),
        fields: vec![inner],
    })
}

fn err_of(v: *mut RtVal) -> *mut RtVal {
    alloc(RtPayload::Variant {
        type_name: "!".into(),
        name: "Err".into(),
        fields: vec![v],
    })
}

fn ok_of(v: *mut RtVal) -> *mut RtVal {
    alloc(RtPayload::Variant {
        type_name: "!".into(),
        name: "Ok".into(),
        fields: vec![v],
    })
}

#[no_mangle]
pub unsafe extern "C" fn grasm_read_file(path: *mut RtVal) -> *mut RtVal {
    let p = as_str(path).to_string();
    match std::fs::read_to_string(&p) {
        Ok(s) => ok_of(alloc(RtPayload::Str(s))),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => err_of(io_err("NotFound", p)),
        Err(e) => err_of(io_err("IoFailed", e.to_string())),
    }
}

#[no_mangle]
pub unsafe extern "C" fn grasm_write_file(path: *mut RtVal, content: *mut RtVal) -> *mut RtVal {
    let p = as_str(path).to_string();
    let c = as_str(content).to_string();
    match std::fs::write(&p, c) {
        Ok(()) => ok_of(alloc(RtPayload::I64(0))),
        Err(e) => err_of(io_err("IoFailed", e.to_string())),
    }
}

#[no_mangle]
pub unsafe extern "C" fn grasm_file_exists(path: *mut RtVal) -> i8 {
    i8::from(std::path::Path::new(as_str(path)).exists())
}

/// `xs[i] = v` honoring copy-on-write; `val` ownership transfers.
/// Returns the (possibly new) list.
#[no_mangle]
pub unsafe extern "C" fn grasm_list_set(v: *mut RtVal, i: i64, val: *mut RtVal) -> *mut RtVal {
    let target = cow_list(v);
    let RtPayload::List(items) = &mut (*target).payload else {
        unreachable!()
    };
    if i < 0 || i as usize >= items.len() {
        rt_panic(&format!("index {i} out of bounds (len {})", items.len()));
    }
    dec(items[i as usize]);
    items[i as usize] = val;
    target
}
