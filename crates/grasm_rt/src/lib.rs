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

/// Element type of an unboxed scalar list. Fixed at creation.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum ScalarKind {
    I64,
    F64,
    Bool,
}

impl ScalarKind {
    fn from_i64(k: i64) -> ScalarKind {
        match k {
            0 => ScalarKind::I64,
            1 => ScalarKind::F64,
            _ => ScalarKind::Bool,
        }
    }
}

pub enum RtPayload {
    I64(i64),
    F64(f64),
    Bool(bool),
    Str(String),
    List(Vec<*mut RtVal>),
    /// Scalar elements stored inline: i64, f64 (as raw bits), bool (0/1).
    /// One machine word per element — no per-element heap cell, no refcount
    /// traffic, no pointer chase. This is why `c[i] = c[i] + x` in a hot loop
    /// is a load and a store instead of an allocation.
    ListScalar { kind: ScalarKind, data: Vec<i64> },
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
    /// A function value: code address plus the environment it captured.
    ///
    /// One representation covers both a named function and a closure. `env` is
    /// a list of captured values (null when there are none), and `name` is the
    /// qualified function name for a named function — closures have none, and
    /// print as `<closure>`.
    Fn {
        code: i64,
        env: *mut RtVal,
        name: Option<String>,
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

/// Refcount marking a value that is shared forever and never freed.
///
/// Boxing a `bool` or a small `i64` used to allocate, so a loop writing into a
/// list allocated once per element. These values are immutable and tiny, so a
/// single instance can be shared by everyone; the sentinel keeps `inc`/`dec`
/// from ever freeing one.
const IMMORTAL: i64 = i64::MAX;

/// Small integers worth interning. Covers loop counters, indices, flags, and
/// the small constants that dominate real programs.
const SMALL_MIN: i64 = -128;
const SMALL_MAX: i64 = 1024;

thread_local! {
    static INTERNED: Vec<*mut RtVal> = {
        let mut table = Vec::with_capacity((SMALL_MAX - SMALL_MIN + 3) as usize);
        for v in [false, true] {
            table.push(Box::into_raw(Box::new(RtVal {
                rc: Cell::new(IMMORTAL),
                payload: RtPayload::Bool(v),
            })));
        }
        for n in SMALL_MIN..=SMALL_MAX {
            table.push(Box::into_raw(Box::new(RtVal {
                rc: Cell::new(IMMORTAL),
                payload: RtPayload::I64(n),
            })));
        }
        table
    };
}

fn interned_bool(v: bool) -> *mut RtVal {
    INTERNED.with(|t| t[usize::from(v)])
}

/// The shared box for `n`, or null when `n` is outside the interned range.
fn interned_int(n: i64) -> *mut RtVal {
    if !(SMALL_MIN..=SMALL_MAX).contains(&n) {
        return std::ptr::null_mut();
    }
    INTERNED.with(|t| t[(n - SMALL_MIN) as usize + 2])
}

/// # Safety
/// `v` must be a live pointer produced by this runtime.
unsafe fn inc(v: *mut RtVal) {
    if !v.is_null() {
        let rc = &(*v).rc;
        if rc.get() != IMMORTAL {
            rc.set(rc.get() + 1);
        }
    }
}

/// # Safety
/// `v` must be a live pointer produced by this runtime.
unsafe fn dec(v: *mut RtVal) {
    if v.is_null() {
        return;
    }
    let rc = &(*v).rc;
    if rc.get() == IMMORTAL {
        return;
    }
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
            RtPayload::Fn { env, .. } => {
                if !env.is_null() {
                    dec(*env);
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

// --- data parallelism ---

/// Below this length, threading costs more than it saves.
const PAR_MIN_ELEMS: usize = 4096;

thread_local! {
    /// Set while a parallel region is running. A `par_map` reached from inside
    /// another one runs sequentially rather than spawning threads per level.
    static IN_PARALLEL: Cell<bool> = const { Cell::new(false) };
}

/// Applies `code` to every element of a scalar list, in parallel.
///
/// Deterministic by construction: each worker writes a disjoint index range of
/// a preallocated output, so the result is identical for any thread count —
/// including one. The checker has already proved the function is effect-free,
/// so there is no observable interleaving to worry about either.
///
/// `env` is the closure's captured scalars, copied into each worker. Nothing
/// is shared mutably, so no refcount ever crosses a thread.
///
/// # Safety
/// `code` is a compiler-emitted function pointer; `env` points to `env_len`
/// readable i64s; `list` is a live scalar list.
#[no_mangle]
pub unsafe extern "C" fn grasm_par_map_scalar(
    list: *mut RtVal,
    code: extern "C" fn(*const i64, i64) -> i64,
    env: *const i64,
    env_len: i64,
    out_kind: i64,
) -> *mut RtVal {
    let RtPayload::ListScalar { data, .. } = &(*list).payload else {
        grasm_panic_msg_str("par_map on a non-scalar list");
    };
    let env_owned: Vec<i64> = if env_len > 0 {
        std::slice::from_raw_parts(env, env_len as usize).to_vec()
    } else {
        Vec::new()
    };
    let n = data.len();
    let mut out: Vec<i64> = vec![0; n];

    let workers = if IN_PARALLEL.with(|f| f.get()) || n < PAR_MIN_ELEMS {
        1
    } else {
        std::thread::available_parallelism()
            .map(|p| p.get())
            .unwrap_or(1)
            .min(n)
    };

    if workers <= 1 {
        for (o, &x) in out.iter_mut().zip(data.iter()) {
            *o = code(env_owned.as_ptr(), x);
        }
    } else {
        let chunk = n.div_ceil(workers);
        let input: &[i64] = data;
        std::thread::scope(|scope| {
            for (ci, slot) in out.chunks_mut(chunk).enumerate() {
                let src = &input[ci * chunk..ci * chunk + slot.len()];
                let env_ref = &env_owned;
                scope.spawn(move || {
                    IN_PARALLEL.with(|f| f.set(true));
                    for (o, &x) in slot.iter_mut().zip(src.iter()) {
                        *o = code(env_ref.as_ptr(), x);
                    }
                });
            }
        });
    }

    alloc(RtPayload::ListScalar {
        kind: ScalarKind::from_i64(out_kind),
        data: out,
    })
}

// --- unboxed scalar lists ---

/// Boxes one inline element, for the paths that hand a value out (Option
/// results, defensive generic access). Interning keeps the common cases free.
fn scalar_box(kind: ScalarKind, bits: i64) -> *mut RtVal {
    match kind {
        ScalarKind::I64 => grasm_box_i64(bits),
        ScalarKind::F64 => alloc(RtPayload::F64(f64::from_bits(bits as u64))),
        ScalarKind::Bool => interned_bool(bits != 0),
    }
}

/// The inline bits of a boxed value, for defensive cross-representation ops.
/// # Safety: live runtime pointer.
unsafe fn scalar_unbox_val(kind: ScalarKind, v: *mut RtVal) -> i64 {
    match (kind, &(*v).payload) {
        (ScalarKind::I64, RtPayload::I64(x)) => *x,
        (ScalarKind::F64, RtPayload::F64(x)) => x.to_bits() as i64,
        (ScalarKind::F64, RtPayload::I64(x)) => (*x as f64).to_bits() as i64,
        (ScalarKind::Bool, RtPayload::Bool(x)) => i64::from(*x),
        _ => grasm_panic_msg_str("scalar list element of the wrong type"),
    }
}

/// Element ordering, matching `cmp_vals` exactly — including that bools have
/// no ordering there, so `[bool].sort()` is a no-op and `min`/`max` return
/// the first element. The interpreter behaves the same way; changing either
/// alone would fail the differential gate.
fn scalar_cmp(kind: ScalarKind, a: i64, b: i64) -> std::cmp::Ordering {
    use std::cmp::Ordering;
    match kind {
        ScalarKind::I64 => a.cmp(&b),
        ScalarKind::F64 => f64::from_bits(a as u64)
            .partial_cmp(&f64::from_bits(b as u64))
            .unwrap_or(Ordering::Equal),
        ScalarKind::Bool => Ordering::Equal,
    }
}

/// Element equality: floats compare as floats (0.0 == -0.0, NaN != NaN),
/// never as bits.
fn scalar_eq(kind: ScalarKind, a: i64, b: i64) -> bool {
    match kind {
        ScalarKind::F64 => f64::from_bits(a as u64) == f64::from_bits(b as u64),
        _ => a == b,
    }
}

/// Renders one element the way `display` renders its boxed form.
fn scalar_display(kind: ScalarKind, bits: i64) -> String {
    match kind {
        ScalarKind::I64 => bits.to_string(),
        ScalarKind::F64 => {
            let x = f64::from_bits(bits as u64);
            if x.fract() == 0.0 && x.is_finite() && x.abs() < 1e15 {
                format!("{x:.1}")
            } else {
                format!("{x}")
            }
        }
        ScalarKind::Bool => (bits != 0).to_string(),
    }
}

/// A list's elements, whichever representation it has.
enum Items<'a> {
    Boxed(&'a Vec<*mut RtVal>),
    Scalar(ScalarKind, &'a Vec<i64>),
}

/// # Safety: live runtime pointer.
unsafe fn items_of<'a>(v: *mut RtVal) -> Items<'a> {
    match &(*v).payload {
        RtPayload::List(items) => {
            Items::Boxed(std::mem::transmute::<&Vec<*mut RtVal>, &'a Vec<*mut RtVal>>(items))
        }
        RtPayload::ListScalar { kind, data } => {
            Items::Scalar(*kind, std::mem::transmute::<&Vec<i64>, &'a Vec<i64>>(data))
        }
        _ => grasm_panic_msg_str("expected a list"),
    }
}

/// A new empty scalar list of the given kind (0 = i64, 1 = f64, 2 = bool).
#[no_mangle]
pub extern "C" fn grasm_list_new_scalar(kind: i64) -> *mut RtVal {
    alloc(RtPayload::ListScalar {
        kind: ScalarKind::from_i64(kind),
        data: Vec::new(),
    })
}

/// Appends inline bits. On a boxed list (one a runtime builtin produced),
/// falls back to boxing — `kind` says how to interpret the bits.
/// # Safety: live runtime pointer.
#[no_mangle]
pub unsafe extern "C" fn grasm_list_push_scalar(
    list: *mut RtVal,
    bits: i64,
    kind: i64,
) -> *mut RtVal {
    let target = cow_list(list);
    match &mut (*target).payload {
        RtPayload::ListScalar { data, .. } => data.push(bits),
        RtPayload::List(items) => items.push(scalar_box(ScalarKind::from_i64(kind), bits)),
        _ => grasm_panic_msg_str("push on non-list"),
    }
    target
}

/// Reads inline bits; on a boxed list, unboxes. Panics on OOB like the
/// interpreter.
/// # Safety: live runtime pointer.
#[no_mangle]
pub unsafe extern "C" fn grasm_list_get_scalar(v: *mut RtVal, i: i64, kind: i64) -> i64 {
    match &(*v).payload {
        RtPayload::ListScalar { data, .. } => {
            if i < 0 || i as usize >= data.len() {
                rt_panic(&format!("index {i} out of bounds (len {})", data.len()));
            }
            data[i as usize]
        }
        RtPayload::List(items) => {
            if i < 0 || i as usize >= items.len() {
                rt_panic(&format!("index {i} out of bounds (len {})", items.len()));
            }
            scalar_unbox_val(ScalarKind::from_i64(kind), items[i as usize])
        }
        _ => grasm_panic_msg_str("index on non-list"),
    }
}

/// `xs[i] = bits` honoring copy-on-write; returns the (possibly new) list.
/// # Safety: live runtime pointer.
#[no_mangle]
pub unsafe extern "C" fn grasm_list_set_scalar(
    v: *mut RtVal,
    i: i64,
    bits: i64,
    kind: i64,
) -> *mut RtVal {
    let target = cow_list(v);
    match &mut (*target).payload {
        RtPayload::ListScalar { data, .. } => {
            if i < 0 || i as usize >= data.len() {
                rt_panic(&format!("index {i} out of bounds (len {})", data.len()));
            }
            data[i as usize] = bits;
        }
        RtPayload::List(items) => {
            if i < 0 || i as usize >= items.len() {
                rt_panic(&format!("index {i} out of bounds (len {})", items.len()));
            }
            dec(items[i as usize]);
            items[i as usize] = scalar_box(ScalarKind::from_i64(kind), bits);
        }
        _ => grasm_panic_msg_str("index on non-list"),
    }
    target
}

// --- function values ---

/// # Safety: `env` is an owned list pointer or null; `name` is a live slice.
#[no_mangle]
pub unsafe extern "C" fn grasm_fn_new(
    code: i64,
    env: *mut RtVal,
    name_ptr: *const u8,
    name_len: i64,
) -> *mut RtVal {
    let name = if name_len > 0 {
        let bytes = std::slice::from_raw_parts(name_ptr, name_len as usize);
        Some(String::from_utf8_lossy(bytes).into_owned())
    } else {
        None
    };
    alloc(RtPayload::Fn { code, env, name })
}

/// # Safety: live function value.
#[no_mangle]
pub unsafe extern "C" fn grasm_fn_code(v: *mut RtVal) -> i64 {
    match &(*v).payload {
        RtPayload::Fn { code, .. } => *code,
        _ => grasm_panic_msg_str("call of a non-function value"),
    }
}

/// Borrows the captured environment (+1 so the caller may hold it).
///
/// # Safety: live function value.
#[no_mangle]
pub unsafe extern "C" fn grasm_fn_env(v: *mut RtVal) -> *mut RtVal {
    match &(*v).payload {
        RtPayload::Fn { env, .. } => {
            if !env.is_null() {
                inc(*env);
            }
            *env
        }
        _ => grasm_panic_msg_str("environment of a non-function value"),
    }
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
    match &mut (*target).payload {
        RtPayload::List(items) => items.push(elem), // ownership transfers
        RtPayload::ListScalar { kind, data } => {
            data.push(scalar_unbox_val(*kind, elem));
            dec(elem);
        }
        _ => unreachable!(),
    }
    target
}

unsafe fn cow_list(list: *mut RtVal) -> *mut RtVal {
    if (*list).rc.get() == 1 {
        return list;
    }
    let fresh = match &(*list).payload {
        RtPayload::List(items) => {
            let cloned: Vec<*mut RtVal> = items
                .iter()
                .map(|&it| {
                    inc(it);
                    it
                })
                .collect();
            alloc(RtPayload::List(cloned))
        }
        // A scalar list clones as one memcpy: no per-element refcounts exist.
        RtPayload::ListScalar { kind, data } => alloc(RtPayload::ListScalar {
            kind: *kind,
            data: data.clone(),
        }),
        _ => grasm_panic_msg_str("list op on non-list"),
    };
    dec(list);
    fresh
}

#[no_mangle]
pub unsafe extern "C" fn grasm_list_len(v: *mut RtVal) -> i64 {
    match &(*v).payload {
        RtPayload::List(items) => items.len() as i64,
        RtPayload::ListScalar { data, .. } => data.len() as i64,
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
        RtPayload::ListScalar { kind, data } => {
            if i < 0 || i as usize >= data.len() {
                rt_panic(&format!("index {i} out of bounds (len {})", data.len()));
            }
            scalar_box(*kind, data[i as usize])
        }
        _ => grasm_panic_msg_str("index on non-list"),
    }
}

// Boxed scalars: list elements are uniformly boxed in N2.

#[no_mangle]
pub extern "C" fn grasm_box_i64(v: i64) -> *mut RtVal {
    let shared = interned_int(v);
    if shared.is_null() {
        alloc(RtPayload::I64(v))
    } else {
        shared
    }
}

#[no_mangle]
pub extern "C" fn grasm_box_f64(v: f64) -> *mut RtVal {
    alloc(RtPayload::F64(v))
}

#[no_mangle]
pub extern "C" fn grasm_box_bool(v: i8) -> *mut RtVal {
    interned_bool(v != 0)
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
        // Matches the interpreter: a named function shows its qualified name,
        // a closure has none to show.
        RtPayload::Fn { name, .. } => match name {
            Some(n) => format!("<fn {n}>"),
            None => "<closure>".to_string(),
        },
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
        RtPayload::ListScalar { kind, data } => {
            let parts: Vec<String> = data.iter().map(|&b| scalar_display(*kind, b)).collect();
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
        (
            RtPayload::ListScalar { kind: k1, data: d1 },
            RtPayload::ListScalar { kind: k2, data: d2 },
        ) => {
            k1 == k2
                && d1.len() == d2.len()
                && d1.iter().zip(d2).all(|(&x, &y)| scalar_eq(*k1, x, y))
        }
        // The same logical list can arrive in either representation (a
        // literal is scalar; `keys()` is boxed), so equality must cross.
        (RtPayload::ListScalar { kind, data }, RtPayload::List(items))
        | (RtPayload::List(items), RtPayload::ListScalar { kind, data }) => {
            data.len() == items.len()
                && data
                    .iter()
                    .zip(items)
                    .all(|(&bits, &boxed)| match (*kind, &(*boxed).payload) {
                        (ScalarKind::I64, RtPayload::I64(x)) => bits == *x,
                        (ScalarKind::F64, RtPayload::F64(x)) => {
                            f64::from_bits(bits as u64) == *x
                        }
                        (ScalarKind::Bool, RtPayload::Bool(x)) => (bits != 0) == *x,
                        _ => false,
                    })
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
        RtPayload::ListScalar { kind, data } => {
            if i < 0 || i as usize >= data.len() {
                none_val()
            } else {
                some_of(scalar_box(*kind, data[i as usize]))
            }
        }
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
// Maps and the builtin method surface
// ---------------------------------------------------------------------------

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
/// Position of `k` in a map's sorted entries.
///
/// Entries are kept in key order so iteration is deterministic ([E-MAP-ORDER]),
/// which means lookups can binary search. A linear scan here made every map
/// operation O(n) and whole programs O(n^2).
///
/// # Safety: live pointers into a map's entry vector.
unsafe fn map_find(
    entries: &[(*mut RtVal, *mut RtVal)],
    k: *mut RtVal,
) -> Result<usize, usize> {
    entries.binary_search_by(|(ek, _)| map_key_ord(&**ek, &*k))
}

/// # Safety: live pointers.
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
    match map_find(entries, k) {
        Ok(i) => {
            dec(entries[i].1);
            entries[i].1 = v;
            dec(k);
        }
        Err(pos) => entries.insert(pos, (k, v)),
    }
    target
}

/// # Safety: live pointers; returns Option variant (+1).
#[no_mangle]
pub unsafe extern "C" fn grasm_map_get(m: *mut RtVal, k: *mut RtVal) -> *mut RtVal {
    let RtPayload::Map(entries) = &(*m).payload else {
        grasm_panic_msg_str("get on non-map");
    };
    if let Ok(i) = map_find(entries, k) {
        let ev = entries[i].1;
        inc(ev);
        return alloc(RtPayload::Variant {
            type_name: "Option".into(),
            name: "Some".into(),
            fields: vec![ev],
        });
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
    i8::from(map_find(entries, k).is_ok())
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
    match map_find(entries, k).ok() {
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

/// # Safety: live pointers.
#[no_mangle]
pub unsafe extern "C" fn grasm_list_join(v: *mut RtVal, sep: *mut RtVal) -> *mut RtVal {
    let parts: Vec<String> = match items_of(v) {
        Items::Boxed(items) => items.iter().map(|&i| display(i)).collect(),
        Items::Scalar(kind, data) => data.iter().map(|&b| scalar_display(kind, b)).collect(),
    };
    alloc(RtPayload::Str(parts.join(as_str(sep))))
}

/// # Safety: live pointer.
#[no_mangle]
pub unsafe extern "C" fn grasm_list_rev(v: *mut RtVal) -> *mut RtVal {
    match items_of(v) {
        Items::Boxed(items) => {
            let mut out: Vec<*mut RtVal> = items
                .iter()
                .map(|&i| {
                    inc(i);
                    i
                })
                .collect();
            out.reverse();
            alloc(RtPayload::List(out))
        }
        Items::Scalar(kind, data) => {
            let mut out = data.clone();
            out.reverse();
            alloc(RtPayload::ListScalar { kind, data: out })
        }
    }
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
    match &mut (*target).payload {
        RtPayload::List(items) => items.sort_by(|&a, &b| cmp_vals(a, b)),
        RtPayload::ListScalar { kind, data } => {
            let k = *kind;
            data.sort_by(|&a, &b| scalar_cmp(k, a, b));
        }
        _ => unreachable!(),
    }
    target
}

/// # Safety: live pointer.
#[no_mangle]
pub unsafe extern "C" fn grasm_list_slice(v: *mut RtVal, a: i64, b: i64) -> *mut RtVal {
    match items_of(v) {
        Items::Boxed(items) => {
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
        Items::Scalar(kind, data) => {
            let len = data.len() as i64;
            let s = a.clamp(0, len) as usize;
            let e = b.clamp(0, len) as usize;
            let out = if s <= e { data[s..e].to_vec() } else { Vec::new() };
            alloc(RtPayload::ListScalar { kind, data: out })
        }
    }
}

/// # Safety: live pointers.
#[no_mangle]
pub unsafe extern "C" fn grasm_list_contains(v: *mut RtVal, needle: *mut RtVal) -> i8 {
    let hit = match items_of(v) {
        Items::Boxed(items) => items.iter().any(|&i| val_eq(i, needle)),
        Items::Scalar(kind, data) => match (kind, &(*needle).payload) {
            (ScalarKind::I64, RtPayload::I64(x)) => data.contains(x),
            (ScalarKind::F64, RtPayload::F64(x)) => {
                data.iter().any(|&b| f64::from_bits(b as u64) == *x)
            }
            (ScalarKind::Bool, RtPayload::Bool(x)) => data.contains(&i64::from(*x)),
            _ => false,
        },
    };
    i8::from(hit)
}

/// Pop honoring CoW: returns the new list; `out` receives the popped Option.
/// # Safety: live pointers.
#[no_mangle]
pub unsafe extern "C" fn grasm_list_pop(v: *mut RtVal, out: *mut *mut RtVal) -> *mut RtVal {
    let target = cow_list(v);
    match &mut (*target).payload {
        RtPayload::List(items) => {
            *out = match items.pop() {
                Some(e) => some_of(e),
                None => none_val(),
            };
        }
        RtPayload::ListScalar { kind, data } => {
            *out = match data.pop() {
                Some(bits) => some_of(scalar_box(*kind, bits)),
                None => none_val(),
            };
        }
        _ => unreachable!(),
    }
    target
}

/// # Safety: live pointer.
#[no_mangle]
pub unsafe extern "C" fn grasm_list_first(v: *mut RtVal) -> *mut RtVal {
    match items_of(v) {
        Items::Boxed(items) => match items.first() {
            Some(&e) => {
                inc(e);
                some_of(e)
            }
            None => none_val(),
        },
        Items::Scalar(kind, data) => match data.first() {
            Some(&b) => some_of(scalar_box(kind, b)),
            None => none_val(),
        },
    }
}

/// # Safety: live pointer.
#[no_mangle]
pub unsafe extern "C" fn grasm_list_last(v: *mut RtVal) -> *mut RtVal {
    match items_of(v) {
        Items::Boxed(items) => match items.last() {
            Some(&e) => {
                inc(e);
                some_of(e)
            }
            None => none_val(),
        },
        Items::Scalar(kind, data) => match data.last() {
            Some(&b) => some_of(scalar_box(kind, b)),
            None => none_val(),
        },
    }
}

/// # Safety: live pointer.
#[no_mangle]
pub unsafe extern "C" fn grasm_list_sum_i64(v: *mut RtVal) -> i64 {
    match items_of(v) {
        Items::Boxed(items) => {
            let mut total = 0i64;
            for &i in items {
                if let RtPayload::I64(x) = &(*i).payload {
                    total += x;
                }
            }
            total
        }
        Items::Scalar(ScalarKind::I64, data) => data.iter().sum(),
        Items::Scalar(..) => 0,
    }
}

/// # Safety: live pointer.
#[no_mangle]
pub unsafe extern "C" fn grasm_list_sum_f64(v: *mut RtVal) -> f64 {
    match items_of(v) {
        Items::Boxed(items) => {
            let mut total = 0f64;
            for &i in items {
                match &(*i).payload {
                    RtPayload::F64(x) => total += x,
                    RtPayload::I64(x) => total += *x as f64,
                    _ => {}
                }
            }
            total
        }
        Items::Scalar(ScalarKind::F64, data) => {
            data.iter().map(|&b| f64::from_bits(b as u64)).sum()
        }
        Items::Scalar(ScalarKind::I64, data) => data.iter().map(|&x| x as f64).sum(),
        Items::Scalar(..) => 0.0,
    }
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
    match &mut (*target).payload {
        RtPayload::List(items) => {
            if i < 0 || i as usize >= items.len() {
                rt_panic(&format!("index {i} out of bounds (len {})", items.len()));
            }
            dec(items[i as usize]);
            items[i as usize] = val;
        }
        RtPayload::ListScalar { kind, data } => {
            if i < 0 || i as usize >= data.len() {
                rt_panic(&format!("index {i} out of bounds (len {})", data.len()));
            }
            data[i as usize] = scalar_unbox_val(*kind, val);
            dec(val);
        }
        _ => unreachable!(),
    }
    target
}

// --- C6: time, environment, process, and the extended collection surface ---

#[no_mangle]
pub extern "C" fn grasm_now_millis() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}

/// Returns `Option[str]` for an environment variable.
/// # Safety: live runtime pointer.
#[no_mangle]
pub unsafe extern "C" fn grasm_env(name: *mut RtVal) -> *mut RtVal {
    match std::env::var(as_str(name)) {
        Ok(v) => some_of(alloc(RtPayload::Str(v))),
        Err(_) => none_val(),
    }
}

#[no_mangle]
pub extern "C" fn grasm_exit(code: i64) {
    use std::io::Write as _;
    let _ = std::io::stdout().flush();
    std::process::exit(code.clamp(0, 255) as i32);
}

/// # Safety: live runtime pointer.
#[no_mangle]
pub unsafe extern "C" fn grasm_str_lines(s: *mut RtVal) -> *mut RtVal {
    let items: Vec<*mut RtVal> = as_str(s)
        .lines()
        .map(|l| alloc(RtPayload::Str(l.to_string())))
        .collect();
    alloc(RtPayload::List(items))
}

/// # Safety: live runtime pointers.
#[no_mangle]
pub unsafe extern "C" fn grasm_str_pad(
    s: *mut RtVal,
    width: i64,
    fill: *mut RtVal,
    at_start: i8,
) -> *mut RtVal {
    let text = as_str(s);
    let fill = as_str(fill);
    if fill.is_empty() {
        grasm_panic_msg_str("pad fill must not be empty");
    }
    let len = text.chars().count();
    // A multi-character fill repeats and is cut to the exact width, matching
    // the interpreter (spec [E-PAD-CYCLE]).
    let pad: String = fill
        .chars()
        .cycle()
        .take((width.max(0) as usize).saturating_sub(len))
        .collect();
    let out = if at_start != 0 {
        format!("{pad}{text}")
    } else {
        format!("{text}{pad}")
    };
    alloc(RtPayload::Str(out))
}

/// # Safety: live runtime pointer.
#[no_mangle]
pub unsafe extern "C" fn grasm_str_is_empty(s: *mut RtVal) -> i8 {
    i8::from(as_str(s).is_empty())
}

/// # Safety: live runtime pointer.
#[no_mangle]
pub unsafe extern "C" fn grasm_list_is_empty(v: *mut RtVal) -> i8 {
    let empty = match items_of(v) {
        Items::Boxed(items) => items.is_empty(),
        Items::Scalar(_, data) => data.is_empty(),
    };
    i8::from(empty)
}

/// # Safety: live runtime pointer.
#[no_mangle]
pub unsafe extern "C" fn grasm_map_is_empty(v: *mut RtVal) -> i8 {
    match &(*v).payload {
        RtPayload::Map(entries) => i8::from(entries.is_empty()),
        _ => grasm_panic_msg_str("is_empty on non-map"),
    }
}

/// Minimum (`want_max` = 0) or maximum element as an `Option`.
/// # Safety: live runtime pointer.
#[no_mangle]
pub unsafe extern "C" fn grasm_list_min_max(v: *mut RtVal, want_max: i8) -> *mut RtVal {
    let want_max = want_max != 0;
    match items_of(v) {
        Items::Boxed(items) => {
            let mut best: Option<*mut RtVal> = None;
            for &item in items {
                best = Some(match best {
                    None => item,
                    Some(cur) => {
                        let ord = cmp_vals(item, cur);
                        let take = if want_max {
                            ord == std::cmp::Ordering::Greater
                        } else {
                            ord == std::cmp::Ordering::Less
                        };
                        if take {
                            item
                        } else {
                            cur
                        }
                    }
                });
            }
            match best {
                Some(b) => {
                    inc(b);
                    some_of(b)
                }
                None => none_val(),
            }
        }
        Items::Scalar(kind, data) => {
            let mut best: Option<i64> = None;
            for &bits in data {
                best = Some(match best {
                    None => bits,
                    Some(cur) => {
                        let ord = scalar_cmp(kind, bits, cur);
                        let take = if want_max {
                            ord == std::cmp::Ordering::Greater
                        } else {
                            ord == std::cmp::Ordering::Less
                        };
                        if take {
                            bits
                        } else {
                            cur
                        }
                    }
                });
            }
            match best {
                Some(b) => some_of(scalar_box(kind, b)),
                None => none_val(),
            }
        }
    }
}
