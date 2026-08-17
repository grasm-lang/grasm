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

/// A float, always distinguishable from an integer.
///
/// [L-DISPLAY] promises a point, and the point is the property: without it a
/// large `f64` prints exactly like an `i64` and a reader of the output cannot
/// tell which they have. The rule was written out four separate times, each
/// saying "a point below 1e15" — a guarantee with an invisible boundary in
/// it, which the conformance program missed because it only tested values on
/// the near side. One function now, so there is nowhere for the copies to
/// disagree.
pub fn float_text(v: f64) -> String {
    let s = format!("{v}");
    if v.is_finite() && !s.contains(['.', 'e', 'E']) {
        format!("{s}.0")
    } else {
        s
    }
}

#[no_mangle]
pub extern "C" fn grasm_out_f64(v: f64) {
    let mut out = std::io::stdout().lock();
    let _ = write!(out, "{}", float_text(v));
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
    // stdout is the program's output; a panic printed into it corrupts
    // whatever is reading ([E-PANIC-EXIT]). Flush stdout first so anything
    // already produced is not lost behind the exit.
    let _ = std::io::stdout().flush();
    let mut out = std::io::stderr().lock();
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

/// Floating remainder. Cranelift has no instruction for it, so the backend
/// calls this rather than leaving `%` on `f64` unsupported while the
/// interpreter accepted it.
#[no_mangle]
pub extern "C" fn grasm_f64_rem(a: f64, b: f64) -> f64 {
    a % b
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
    /// A Tier-2 reference: one word, like an `i64`, but rendered as the slot
    /// it names. Stored inline for the same reason an integer is — an object
    /// graph is a list of references, and a heap cell each would undo the
    /// tier — but it is not an integer, and printed as one it disclosed the
    /// packed slot and generation as a raw number.
    Ref,
}

impl ScalarKind {
    fn from_i64(k: i64) -> ScalarKind {
        match k {
            0 => ScalarKind::I64,
            1 => ScalarKind::F64,
            3 => ScalarKind::Ref,
            _ => ScalarKind::Bool,
        }
    }
}

/// Inline element storage. Bools get a byte each rather than a machine word:
/// a 2M-entry sieve is 2 MB instead of 16 MB, and that is the whole reason
/// this is not simply `Vec<i64>`.
enum ScalarIter<'a> {
    Words(std::slice::Iter<'a, i64>),
    Bytes(std::slice::Iter<'a, u8>),
}

impl Iterator for ScalarIter<'_> {
    type Item = i64;

    #[inline]
    fn next(&mut self) -> Option<i64> {
        match self {
            ScalarIter::Words(i) => i.next().copied(),
            ScalarIter::Bytes(i) => i.next().map(|&b| i64::from(b)),
        }
    }

    #[inline]
    fn size_hint(&self) -> (usize, Option<usize>) {
        match self {
            ScalarIter::Words(i) => i.size_hint(),
            ScalarIter::Bytes(i) => i.size_hint(),
        }
    }
}

/// A map key, owned.
///
/// Keys are restricted to `i64`, `str`, and `bool` ([T-MAP-KEY]), which makes
/// them hashable — so lookup is a hash probe rather than a binary search over
/// boxed values. The derived `Ord` puts the variants in exactly the order the
/// specification requires for iteration ([E-MAP-ORDER]): false before true,
/// then integers ascending, then strings lexicographically.
#[derive(PartialEq, Eq, Hash, PartialOrd, Ord, Clone)]
pub enum MapKey {
    Bool(bool),
    I64(i64),
    Str(String),
}

thread_local! {
    /// Buffer reused by every map probe, so looking a string key up does not
    /// allocate. Only its contents change; the capacity is handed back.
    static PROBE: std::cell::RefCell<String> = const { std::cell::RefCell::new(String::new()) };
}

/// Calls `f` with `v` seen as a map key, without taking ownership of it.
///
/// A lookup does not need to own its key, but `HashMap::get` wants a `&MapKey`
/// and `MapKey` owns its string — so building one used to copy the key onto
/// the heap and drop it a few instructions later, on every get, has, and
/// remove. Here the string is copied into a buffer that keeps its capacity
/// between calls, so after the first probe there is no allocation at all.
///
/// Marks a freshly built list as a tuple, so it renders with parentheses.
///
/// # Safety: `v` is a uniquely owned list the compiler just built.
#[no_mangle]
pub unsafe extern "C" fn grasm_tuple_seal(v: *mut RtVal) -> *mut RtVal {
    if let RtPayload::List(items) = &mut (*v).payload {
        let taken = std::mem::take(items);
        (*v).payload = RtPayload::Tuple(taken);
    }
    v
}

/// # Safety: live runtime pointer.
unsafe fn with_key<R>(v: *mut RtVal, f: impl FnOnce(&MapKey) -> R) -> R {
    match &(*v).payload {
        RtPayload::Bool(b) => f(&MapKey::Bool(*b)),
        RtPayload::I64(n) => f(&MapKey::I64(*n)),
        RtPayload::Str(s) => PROBE.with(|cell| {
            let mut buf = cell.borrow_mut();
            buf.clear();
            buf.push_str(s);
            let key = MapKey::Str(std::mem::take(&mut buf));
            let out = f(&key);
            let MapKey::Str(returned) = key else {
                unreachable!()
            };
            *buf = returned;
            out
        }),
        // Worded exactly as the interpreter words it: [E-PANIC-TRACE] says
        // the panic line and the exit status are identical in all three
        // engines, and this was the one message that was not.
        _ => rt_panic(&format!("invalid map key `{}`", display(v))),
    }
}

impl MapKey {
    /// # Safety: live runtime pointer.
    unsafe fn of(v: *mut RtVal) -> MapKey {
        match &(*v).payload {
            RtPayload::Bool(b) => MapKey::Bool(*b),
            RtPayload::I64(n) => MapKey::I64(*n),
            RtPayload::Str(s) => MapKey::Str(s.to_string()),
            _ => rt_panic(&format!("invalid map key `{}`", display(v))),
        }
    }

    fn boxed(&self) -> *mut RtVal {
        match self {
            MapKey::Bool(b) => interned_bool(*b),
            MapKey::I64(n) => grasm_box_i64(*n),
            MapKey::Str(s) => alloc(RtPayload::Str(RtStr::new(s.clone()))),
        }
    }
}

pub enum ScalarData {
    Words(Vec<i64>),
    Bytes(Vec<u8>),
}

impl ScalarData {
    fn new(kind: ScalarKind) -> ScalarData {
        match kind {
            ScalarKind::Bool => ScalarData::Bytes(Vec::new()),
            _ => ScalarData::Words(Vec::new()),
        }
    }

    fn len(&self) -> usize {
        match self {
            ScalarData::Words(v) => v.len(),
            ScalarData::Bytes(v) => v.len(),
        }
    }

    fn is_empty(&self) -> bool {
        self.len() == 0
    }

    fn get(&self, i: usize) -> i64 {
        match self {
            ScalarData::Words(v) => v[i],
            ScalarData::Bytes(v) => i64::from(v[i]),
        }
    }

    fn set(&mut self, i: usize, bits: i64) {
        match self {
            ScalarData::Words(v) => v[i] = bits,
            ScalarData::Bytes(v) => v[i] = u8::from(bits != 0),
        }
    }

    fn push(&mut self, bits: i64) {
        match self {
            ScalarData::Words(v) => v.push(bits),
            ScalarData::Bytes(v) => v.push(u8::from(bits != 0)),
        }
    }

    fn pop(&mut self) -> Option<i64> {
        match self {
            ScalarData::Words(v) => v.pop(),
            ScalarData::Bytes(v) => v.pop().map(i64::from),
        }
    }

    fn duplicate(&self) -> ScalarData {
        match self {
            ScalarData::Words(v) => ScalarData::Words(v.clone()),
            ScalarData::Bytes(v) => ScalarData::Bytes(v.clone()),
        }
    }

    fn slice(&self, s: usize, e: usize) -> ScalarData {
        match self {
            ScalarData::Words(v) => ScalarData::Words(v[s..e].to_vec()),
            ScalarData::Bytes(v) => ScalarData::Bytes(v[s..e].to_vec()),
        }
    }

    fn reversed(&self) -> ScalarData {
        match self {
            ScalarData::Words(v) => {
                let mut o = v.clone();
                o.reverse();
                ScalarData::Words(o)
            }
            ScalarData::Bytes(v) => {
                let mut o = v.clone();
                o.reverse();
                ScalarData::Bytes(o)
            }
        }
    }

    /// Elements as i64.
    ///
    /// Deliberately a concrete iterator rather than `Box<dyn Iterator>`: this
    /// runs once per element on every sum, contains, min/max, and join, and a
    /// trait object there costs a dynamic dispatch per element — measured at
    /// ~39ns/element on a 2M sum, about forty times what the work deserves.
    fn iter(&self) -> ScalarIter<'_> {
        match self {
            ScalarData::Words(v) => ScalarIter::Words(v.iter()),
            ScalarData::Bytes(v) => ScalarIter::Bytes(v.iter()),
        }
    }

    /// The backing words, when the storage already is words. Lets a hot loop
    /// work on a plain slice with no per-element branch at all.
    fn as_words(&self) -> Option<&[i64]> {
        match self {
            ScalarData::Words(v) => Some(v),
            ScalarData::Bytes(_) => None,
        }
    }

    /// Sorts in place. Bools have no ordering in `cmp_vals`, so sorting them
    /// is a no-op in both engines — replicated here rather than "fixed".
    fn sort(&mut self, kind: ScalarKind) {
        if let ScalarData::Words(v) = self {
            v.sort_by(|&a, &b| scalar_cmp(kind, a, b));
        }
    }

    fn from_words(kind: ScalarKind, words: Vec<i64>) -> ScalarData {
        match kind {
            ScalarKind::Bool => ScalarData::Bytes(
                words.into_iter().map(|b| u8::from(b != 0)).collect(),
            ),
            _ => ScalarData::Words(words),
        }
    }
}

/// A string, with its character count remembered.
///
/// `len` counts *characters*, which meant decoding the whole string every time
/// it was asked: `while i < s.len()` was quadratic, and 400 000 characters
/// took 6.7 seconds. These strings are immutable once built, so the count is
/// computed once and kept. It also answers whether the text is ASCII — that is
/// exactly "as many characters as bytes" — which is what makes `code_at(i)` an
/// index rather than a walk.
#[derive(Debug)]
pub struct RtStr {
    s: String,
    /// `u32::MAX` until asked for.
    ///
    /// Atomic, and the index below is a `OnceLock`, because a value crosses
    /// threads here: `spawn { scan(text) }` hands the same allocation to
    /// several workers, and two of them asking a string its length at the
    /// same moment is an ordinary thing to write. A `Cell` would have been a
    /// data race that happened to print the right answer.
    chars: std::sync::atomic::AtomicU32,
    /// Byte offset of each character, built the first time one is asked for
    /// by index and only when the text is not ASCII. ASCII needs no index,
    /// because there the offset *is* the index.
    offsets: std::sync::OnceLock<Vec<u32>>,
}

impl RtStr {
    pub fn new(s: String) -> RtStr {
        RtStr {
            s,
            chars: std::sync::atomic::AtomicU32::new(u32::MAX),
            offsets: std::sync::OnceLock::new(),
        }
    }

    /// The byte offset of character `i`, in constant time after the first
    /// call. Shared with `substr`, which walked from the start otherwise.
    pub fn byte_at(&self, i: usize) -> Option<usize> {
        if self.is_flat() {
            return (i <= self.s.len()).then_some(i);
        }
        let offs = self
            .offsets
            .get_or_init(|| self.s.char_indices().map(|(o, _)| o as u32).collect());
        match i.cmp(&offs.len()) {
            std::cmp::Ordering::Less => Some(offs[i] as usize),
            std::cmp::Ordering::Equal => Some(self.s.len()),
            std::cmp::Ordering::Greater => None,
        }
    }

    /// The character at `i`, in constant time after the first call.
    pub fn char_at(&self, i: usize) -> Option<char> {
        if self.is_flat() {
            return self.s.as_bytes().get(i).map(|b| *b as char);
        }
        let offs = self
            .offsets
            .get_or_init(|| self.s.char_indices().map(|(o, _)| o as u32).collect());
        let start = *offs.get(i)? as usize;
        self.s[start..].chars().next()
    }

    pub fn char_len(&self) -> usize {
        use std::sync::atomic::Ordering::Relaxed;
        let c = self.chars.load(Relaxed);
        if c != u32::MAX {
            return c as usize;
        }
        let n = self.s.chars().count();
        // Two threads may both compute it; they compute the same number, so
        // whichever stores last stores the same bytes.
        if n < u32::MAX as usize {
            self.chars.store(n as u32, Relaxed);
        }
        n
    }

    /// Every character one byte, so an index is an index.
    pub fn is_flat(&self) -> bool {
        self.char_len() == self.s.len()
    }
}

impl PartialEq for RtStr {
    fn eq(&self, other: &RtStr) -> bool {
        self.s == other.s
    }
}

impl Eq for RtStr {}

impl PartialOrd for RtStr {
    fn partial_cmp(&self, other: &RtStr) -> Option<std::cmp::Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for RtStr {
    fn cmp(&self, other: &RtStr) -> std::cmp::Ordering {
        self.s.cmp(&other.s)
    }
}

impl std::ops::Deref for RtStr {
    type Target = str;
    fn deref(&self) -> &str {
        &self.s
    }
}

impl std::fmt::Display for RtStr {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.s)
    }
}

pub enum RtPayload {
    I64(i64),
    /// The unit value. No payload, but it renders as `()` and a container can
    /// hold one — boxing it as `I64(0)` made `[(), ()]` print `[0, 0]` under
    /// a compiled build and `[(), ()]` under the interpreter.
    Unit,
    /// A Tier-2 reference: slot index and generation packed in one word,
    /// exactly as the compiled backend carries it. Boxed as a plain integer,
    /// a `ref` inside a list printed as `4294967296` — the packed word — which
    /// is both wrong and a disclosure of where the slab put it.
    Ref(i64),
    F64(f64),
    Bool(bool),
    Str(RtStr),
    List(Vec<*mut RtVal>),
    /// Same shape as a list, rendered with parentheses.
    ///
    /// Tuples are built as lists and sealed, because every operation on them
    /// — refcounting, equality, reading an element out in a pattern — is a
    /// list operation. Only `display` differs, and it differs in a way a
    /// program's output is judged on, so it cannot be left to look the same.
    Tuple(Vec<*mut RtVal>),
    /// Scalar elements stored inline — no per-element heap cell, no refcount
    /// traffic, no pointer chase. This is why `c[i] = c[i] + x` in a hot loop
    /// is a load and a store instead of an allocation.
    ListScalar { kind: ScalarKind, data: ScalarData },
    /// Hash table for lookup; iteration sorts, which is where the ordering
    /// guarantee is paid for rather than on every get and set.
    Map(std::collections::HashMap<MapKey, *mut RtVal>),
    Struct {
        name: String,
        fields: Vec<(String, *mut RtVal)>,
    },
    /// Names are `&'static str` because every one of them is either a literal
    /// here or a pointer into the compiled binary's read-only data. Copying
    /// them made `Some(x)` cost two heap allocations before the payload.
    Variant {
        type_name: &'static str,
        name: &'static str,
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
    LIVE.with(|n| n.set(n.get() + 1));
    // Inside a region the value is arena-owned: no reference counting, and the
    // whole set is freed at the closing brace ([T-REGION]).
    let in_region = REGIONS.with(|r| !r.borrow().is_empty());
    let p = Box::into_raw(Box::new(RtVal {
        rc: Cell::new(if in_region { ARENA } else { 1 }),
        payload,
    }));
    if in_region {
        REGIONS.with(|r| {
            if let Some(top) = r.borrow_mut().last_mut() {
                top.push(p);
            }
        });
    }
    p
}

thread_local! {
    /// Allocations this thread has made and not yet freed.
    ///
    /// A leak is invisible to the differential harness: it compares output,
    /// and a program that leaks produces exactly the right output. This is
    /// what makes the difference observable, and it is the prerequisite for
    /// Tier 1 — a `region` works by *not* decrementing inside the block and
    /// sweeping at the brace, so a miscount there is a double-free or a leak
    /// and only one of those announces itself.
    static LIVE: Cell<i64> = const { Cell::new(0) };
}

/// Prints a reference as the slot it names.
///
/// Not the value it points at: the value may already be gone, and reading it
/// to render a message would be the exact use-after-free the tier prevents.
#[no_mangle]
pub extern "C" fn grasm_out_ref(r: i64) {
    let (i, gen) = unpack_ref(r);
    let mut out = std::io::stdout().lock();
    let _ = write!(out, "ref#{i}.{gen}");
}

/// Allocations still live on this thread.
///
/// Zero at the end of `main` means every allocation was released. It is not
/// zero *during* a program, and it counts only this thread — a worker's
/// allocations are its own.
#[no_mangle]
pub extern "C" fn grasm_live_allocations() -> i64 {
    LIVE.with(Cell::get)
}

/// Reports still-live allocations, if `GRASM_LEAK_CHECK` is set.
///
/// Called at the end of `main`. Silent unless asked, because a leak is a
/// property of the implementation and not something a program's user can act
/// on — and printing it unasked would put it on the stderr the differential
/// harness compares. Under the variable it prints a line and exits 7, a code
/// nothing else here uses.
#[no_mangle]
pub extern "C" fn grasm_check_leaks() {
    if std::env::var_os("GRASM_LEAK_CHECK").is_none() {
        return;
    }
    // Everything the shared heap still holds is released first. A `ref` the
    // program never released is held on purpose — the slab is a root, like a
    // global — so counting it as a leak would report every program using Tier
    // 2 and drown the real thing. What is measured is what is unreachable and
    // still not freed.
    {
        let taken: Vec<*mut RtVal> = {
            let mut slab = SLAB.lock().expect("slab");
            slab.gens.clear();
            slab.free.clear();
            slab.slots.drain(..).flatten().collect()
        };
        for p in taken {
            unsafe { dec(p) };
        }
    }
    let live = LIVE.with(Cell::get);
    if live == 0 {
        return;
    }
    let _ = std::io::stdout().flush();
    let mut err = std::io::stderr().lock();
    let _ = writeln!(err, "leak: {live} allocation(s) still live at exit");
    let _ = err.flush();
    std::process::exit(7);
}

/// Refcount marking a value that is shared forever and never freed.
///
/// Boxing a `bool` or a small `i64` used to allocate, so a loop writing into a
/// list allocated once per element. These values are immutable and tiny, so a
/// single instance can be shared by everyone; the sentinel keeps `inc`/`dec`
/// from ever freeing one.
const IMMORTAL: i64 = i64::MAX;

/// Refcount marking a value owned by an open `region` ([T-REGION]).
///
/// Two things follow from one sentinel. `inc`/`dec` skip it, so a region pays
/// no reference counting at all — which is the performance the tier exists
/// for. And it is an O(1) test for "was this allocated in the region", which
/// the sweep needs to tell its own values from ones it merely points at.
///
/// This is sound only because the checker proves nothing allocated inside a
/// region is reachable afterwards ([T-REGION], [T-REGION-ESCAPE],
/// [T-REGION-INOUT]): every ARENA value is unreachable at the closing brace by
/// construction.
const ARENA: i64 = i64::MAX - 1;

fn is_counted(rc: i64) -> bool {
    rc != IMMORTAL && rc != ARENA
}

// A `Cell<usize>` depth counter was tried beside this, so that an allocation
// could ask "am I inside a region" with a load rather than a `RefCell` borrow.
// It measured **zero** on a benchmark that allocates 800,000 times, and was
// reverted: the borrow is not what costs.
thread_local! {
    /// One list of allocations per open region, innermost last.
    static REGIONS: std::cell::RefCell<Vec<Vec<*mut RtVal>>> =
        const { std::cell::RefCell::new(Vec::new()) };
}

/// Opens a region: allocations until the matching pop belong to it.
#[no_mangle]
pub extern "C" fn grasm_region_push() {
    REGIONS.with(|r| r.borrow_mut().push(Vec::new()));
}

/// Closes a region, freeing everything allocated inside it in one sweep.
///
/// Values are released without walking into their children, because every
/// child allocated inside the region is in this same list and is freed by its
/// own entry — walking would free it twice. A child allocated *outside* is a
/// reference the region took, so that one is decremented normally.
///
/// # Safety: every pointer in the region's list was allocated by this runtime
/// and, by [T-REGION], is unreachable from outside.
#[no_mangle]
pub unsafe extern "C" fn grasm_region_pop() {
    let Some(vals) = REGIONS.with(|r| r.borrow_mut().pop()) else {
        return;
    };
    for p in vals {
        // Release what this value points at that the region does not own.
        let mut outside: Vec<*mut RtVal> = Vec::new();
        match &(*p).payload {
            RtPayload::List(items) | RtPayload::Tuple(items) => {
                outside.extend(items.iter().copied().filter(|c| (**c).rc.get() != ARENA));
            }
            RtPayload::Map(entries) => {
                outside.extend(entries.values().copied().filter(|c| (**c).rc.get() != ARENA));
            }
            RtPayload::Struct { fields, .. } => {
                outside.extend(
                    fields.iter().map(|(_, v)| *v).filter(|c| (**c).rc.get() != ARENA),
                );
            }
            RtPayload::Variant { fields, .. } => {
                outside.extend(fields.iter().copied().filter(|c| (**c).rc.get() != ARENA));
            }
            _ => {}
        }
        for c in outside {
            dec(c);
        }
        LIVE.with(|n| n.set(n.get() - 1));
        drop(Box::from_raw(p));
    }
}

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
        if is_counted(rc.get()) {
            rc.set(rc.get() + 1);
        }
    }
}

/// # Safety
/// `v` must be a live pointer produced by this runtime.
/// Releases a value, and anything it was the last owner of.
///
/// Iterative, not recursive. A list a hundred thousand deep is an ordinary
/// thing for a parser to build, and freeing it by recursion ran out of stack
/// and killed the process with a signal — no message, no exit code anyone
/// could act on, and in direct contradiction of [E-RECURSION-DEPTH], which
/// says a program past the bound panics rather than crashing. The depth a
/// structure can reach is not the depth the code that frees it should need.
unsafe fn dec(v: *mut RtVal) {
    let mut pending: Vec<*mut RtVal> = Vec::new();
    let mut cur = v;
    loop {
        if !cur.is_null() {
            let rc = &(*cur).rc;
            if is_counted(rc.get()) {
                rc.set(rc.get() - 1);
                if rc.get() <= 0 {
                    LIVE.with(|n| n.set(n.get() - 1));
                    let boxed = Box::from_raw(cur);
                    match &boxed.payload {
                        RtPayload::List(items) | RtPayload::Tuple(items) => {
                            pending.extend(items.iter().copied());
                        }
                        RtPayload::Map(entries) => {
                            pending.extend(entries.values().copied());
                        }
                        RtPayload::Struct { fields, .. } => {
                            pending.extend(fields.iter().map(|(_, fv)| *fv));
                        }
                        RtPayload::Fn { env, .. } => {
                            if !env.is_null() {
                                pending.push(*env);
                            }
                        }
                        RtPayload::Variant { fields, .. } => {
                            pending.extend(fields.iter().copied());
                        }
                        _ => {}
                    }
                }
            }
        }
        match pending.pop() {
            Some(next) => cur = next,
            None => return,
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
    alloc(RtPayload::Str(RtStr::new(String::from_utf8_lossy(bytes).into_owned())))
}

#[no_mangle]
pub unsafe extern "C" fn grasm_str_concat(a: *mut RtVal, b: *mut RtVal) -> *mut RtVal {
    let (RtPayload::Str(sa), RtPayload::Str(sb)) = (&(*a).payload, &(*b).payload) else {
        grasm_panic_msg_str("concat on non-strings");
    };
    alloc(RtPayload::Str(RtStr::new(format!("{sa}{sb}"))))
}

#[no_mangle]
pub unsafe extern "C" fn grasm_str_from_i64(v: i64) -> *mut RtVal {
    alloc(RtPayload::Str(RtStr::new(v.to_string())))
}

#[no_mangle]
pub unsafe extern "C" fn grasm_str_from_f64(v: f64) -> *mut RtVal {
    alloc(RtPayload::Str(RtStr::new(float_text(v))))
}

#[no_mangle]
pub unsafe extern "C" fn grasm_str_from_bool(v: i8) -> *mut RtVal {
    alloc(RtPayload::Str(RtStr::new((v != 0).to_string())))
}

#[no_mangle]
pub unsafe extern "C" fn grasm_str_len(v: *mut RtVal) -> i64 {
    match &(*v).payload {
        RtPayload::Str(s) => s.char_len() as i64,
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
///
/// Deliberately low. The runtime cannot see how expensive one element is —
/// a 400-element `par_map` whose elements each take 100µs is very much worth
/// splitting, and a count-based threshold of a few thousand silently refuses
/// to. Spawning costs tens of microseconds, so over-splitting a cheap list
/// wastes almost nothing, while under-splitting an expensive one costs the
/// whole speedup. The user asked for parallelism by writing `par_map`.
const PAR_MIN_ELEMS: usize = 256;

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
/// A list of scalars as machine words, whichever representation holds it.
///
/// The backend picks a representation from the *static* type, so a `[i64]` is
/// normally a `ListScalar`. A few runtime producers build a boxed list even
/// for scalar elements — `Map.values()` is one — and every other operation
/// copes with both, so the parallel ones assuming the scalar form was a
/// miscompile rather than a refusal: `m.values().par_map(f)` built and then
/// panicked `par_map on a non-scalar list`, while `grasm run` answered.
///
/// # Safety: `list` is a live list this runtime allocated.
unsafe fn scalar_words(list: *mut RtVal, what: &str) -> Vec<i64> {
    match &(*list).payload {
        RtPayload::ListScalar { data, .. } => data.iter().collect(),
        RtPayload::List(items) => items
            .iter()
            .map(|&p| match &(*p).payload {
                RtPayload::I64(v) => *v,
                RtPayload::F64(v) => v.to_bits() as i64,
                RtPayload::Bool(b) => i64::from(*b),
                _ => rt_panic(&format!("{what} on a list of non-scalars")),
            })
            .collect(),
        _ => rt_panic(&format!("{what} on a non-list")),
    }
}

#[no_mangle]
pub unsafe extern "C" fn grasm_par_map_scalar(
    list: *mut RtVal,
    code: extern "C" fn(*const i64, i64) -> i64,
    env: *const i64,
    env_len: i64,
    out_kind: i64,
) -> *mut RtVal {
    // Workers operate on machine words. Word-backed input is used directly;
    // anything else — a byte-backed bool list, or a boxed list from a
    // producer like `Map.values()` — is materialized first.
    let is_words = matches!(
        &(*list).payload,
        RtPayload::ListScalar {
            data: ScalarData::Words(_),
            ..
        }
    );
    let widened: Option<Vec<i64>> = if is_words {
        None
    } else {
        Some(scalar_words(list, "par_map"))
    };
    let words: &[i64] = match (&(*list).payload, &widened) {
        (
            RtPayload::ListScalar {
                data: ScalarData::Words(v),
                ..
            },
            _,
        ) => v,
        (_, Some(v)) => v,
        _ => unreachable!(),
    };
    let env_owned: Vec<i64> = if env_len > 0 {
        std::slice::from_raw_parts(env, env_len as usize).to_vec()
    } else {
        Vec::new()
    };
    let n = words.len();
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
        for (o, &x) in out.iter_mut().zip(words.iter()) {
            *o = code(env_owned.as_ptr(), x);
        }
    } else {
        // Hand out many small pieces from a shared queue rather than giving
        // each worker one big slice. Elements are not equally expensive — a
        // mandelbrot row in the middle of the set costs far more than one at
        // the edge — and splitting statically means every worker waits for
        // whichever got the expensive half. Dynamic hand-out keeps them all
        // busy until the work is actually gone.
        let chunk = n.div_ceil(workers * 16).max(1);
        let input: &[i64] = words;
        let pieces: std::sync::Mutex<Vec<(usize, &mut [i64])>> =
            std::sync::Mutex::new(out.chunks_mut(chunk).enumerate().collect());
        std::thread::scope(|scope| {
            let queue = &pieces;
            let env_ref = &env_owned;
            let drain = move || {
                IN_PARALLEL.with(|f| f.set(true));
                loop {
                    let Some((ci, slot)) = queue.lock().expect("no panics inside").pop() else {
                        return;
                    };
                    let src = &input[ci * chunk..ci * chunk + slot.len()];
                    for (o, &x) in slot.iter_mut().zip(src.iter()) {
                        *o = code(env_ref.as_ptr(), x);
                    }
                }
            };
            for _ in 0..workers {
                // A worker that cannot be created is not an error: the queue
                // is hand-out, so whatever it would have taken is still there
                // for someone else. The calling thread drains the remainder
                // below, which is why this result can be dropped and why the
                // answer is complete even if every spawn fails.
                let _ = std::thread::Builder::new()
                    .stack_size(WORKER_STACK)
                    .spawn_scoped(scope, drain);
            }
            drain();
        });
    }

    let kind = ScalarKind::from_i64(out_kind);
    alloc(RtPayload::ListScalar {
        kind,
        data: ScalarData::from_words(kind, out),
    })
}

/// Prepares a scalar list for raw element access inside a loop.
///
/// Element access normally goes through a call per element, which costs about
/// 2ns where C compiles a load — on an 8M-iteration matmul that is the whole
/// difference. This makes the list uniquely owned once, converts a boxed list
/// to inline storage if needed, and hands back the raw data pointer and length
/// so the loop can load and store directly.
///
/// The caller must have proved the loop cannot resize the list or share it,
/// which is what keeps the pointer valid for the loop's whole extent.
///
/// Returns the (possibly new) list; writes the data pointer and length through
/// the out-params. A data pointer of 0 means raw access is unavailable and the
/// caller must use the ordinary element functions.
///
/// # Safety
/// `list` is a live list; the out-params are writable.
#[no_mangle]
pub unsafe extern "C" fn grasm_list_pin(
    list: *mut RtVal,
    kind: i64,
    out_data: *mut i64,
    out_len: *mut i64,
) -> *mut RtVal {
    let want = ScalarKind::from_i64(kind);
    let target = cow_list(list);

    // A list produced by a runtime builtin is boxed; converting it once here
    // is invisible and makes every later access in the loop a raw load.
    if let RtPayload::List(items) = &(*target).payload {
        let mut data = ScalarData::new(want);
        for &it in items {
            data.push(scalar_unbox_val(want, it));
            dec(it);
        }
        (*target).payload = RtPayload::ListScalar { kind: want, data };
    }

    // Bools are one byte per element, everything else one word; the caller
    // strides accordingly.
    match &mut (*target).payload {
        RtPayload::ListScalar {
            data: ScalarData::Words(v),
            ..
        } => {
            *out_data = v.as_mut_ptr() as i64;
            *out_len = v.len() as i64;
        }
        RtPayload::ListScalar {
            data: ScalarData::Bytes(v),
            ..
        } => {
            *out_data = v.as_mut_ptr() as i64;
            *out_len = v.len() as i64;
        }
        _ => {
            *out_data = 0;
            *out_len = 0;
        }
    }
    target
}

/// The out-of-bounds panic, called only from a failed inline bounds check.
#[no_mangle]
pub extern "C" fn grasm_panic_index(i: i64, len: i64) -> ! {
    rt_panic(&format!("index {i} out of bounds (len {len})"));
}

/// Applies a predicate to every element in parallel, then keeps the survivors
/// **in input order**.
///
/// The predicate runs concurrently; the compaction that follows is sequential
/// and ordered, so the output does not depend on how many cores ran it. The
/// expensive half is the part that parallelises, which is the half worth
/// parallelising.
///
/// # Safety
/// `code` is a compiler-emitted function pointer; `env` points to `env_len`
/// readable i64s; `list` is a live scalar list.
#[no_mangle]
pub unsafe extern "C" fn grasm_par_filter_scalar(
    list: *mut RtVal,
    code: extern "C" fn(*const i64, i64) -> i64,
    env: *const i64,
    env_len: i64,
) -> *mut RtVal {
    // As in `par_map`: the representation follows the static type, and a
    // boxed list of scalars is materialized rather than refused.
    let kind = match &(*list).payload {
        RtPayload::ListScalar { kind, .. } => *kind,
        RtPayload::List(items) => match items.first().map(|&p| &(*p).payload) {
            Some(RtPayload::F64(_)) => ScalarKind::F64,
            Some(RtPayload::Bool(_)) => ScalarKind::Bool,
            Some(RtPayload::Ref(_)) => ScalarKind::Ref,
            _ => ScalarKind::I64,
        },
        _ => grasm_panic_msg_str("par_filter on a non-list"),
    };
    let is_words = matches!(
        &(*list).payload,
        RtPayload::ListScalar {
            data: ScalarData::Words(_),
            ..
        }
    );
    let widened: Option<Vec<i64>> = if is_words {
        None
    } else {
        Some(scalar_words(list, "par_filter"))
    };
    let words: &[i64] = match (&(*list).payload, &widened) {
        (
            RtPayload::ListScalar {
                data: ScalarData::Words(v),
                ..
            },
            _,
        ) => v,
        (_, Some(v)) => v,
        _ => unreachable!(),
    };
    let env_owned: Vec<i64> = if env_len > 0 {
        std::slice::from_raw_parts(env, env_len as usize).to_vec()
    } else {
        Vec::new()
    };

    let n = words.len();
    let mut keep: Vec<u8> = vec![0; n];
    let workers = if IN_PARALLEL.with(|f| f.get()) || n < PAR_MIN_ELEMS {
        1
    } else {
        std::thread::available_parallelism()
            .map(|p| p.get())
            .unwrap_or(1)
            .min(n)
    };

    if workers <= 1 {
        for (k, &x) in keep.iter_mut().zip(words.iter()) {
            *k = u8::from(code(env_owned.as_ptr(), x) != 0);
        }
    } else {
        let chunk = n.div_ceil(workers * 16).max(1);
        let pieces: std::sync::Mutex<Vec<(usize, &mut [u8])>> =
            std::sync::Mutex::new(keep.chunks_mut(chunk).enumerate().collect());
        std::thread::scope(|scope| {
            let queue = &pieces;
            let env_ref = &env_owned;
            let drain = move || {
                IN_PARALLEL.with(|f| f.set(true));
                loop {
                    let Some((ci, slot)) = queue.lock().expect("no panics inside").pop() else {
                        return;
                    };
                    let src = &words[ci * chunk..ci * chunk + slot.len()];
                    for (k, &x) in slot.iter_mut().zip(src.iter()) {
                        *k = u8::from(code(env_ref.as_ptr(), x) != 0);
                    }
                }
            };
            for _ in 0..workers {
                // See the note in `par_map`: the calling thread drains the
                // remainder, so a failed spawn costs parallelism and never
                // correctness.
                let _ = std::thread::Builder::new()
                    .stack_size(WORKER_STACK)
                    .spawn_scoped(scope, drain);
            }
            drain();
        });
    }

    // Ordered compaction: this is what makes the result core-count independent.
    let mut out = ScalarData::new(kind);
    for (i, &k) in keep.iter().enumerate() {
        if k != 0 {
            out.push(words[i]);
        }
    }
    alloc(RtPayload::ListScalar { kind, data: out })
}

// --- unboxed scalar lists ---

/// Boxes one inline element, for the paths that hand a value out (Option
/// results, defensive generic access). Interning keeps the common cases free.
fn scalar_box(kind: ScalarKind, bits: i64) -> *mut RtVal {
    match kind {
        ScalarKind::I64 => grasm_box_i64(bits),
        ScalarKind::Ref => grasm_box_ref(bits),
        ScalarKind::F64 => alloc(RtPayload::F64(f64::from_bits(bits as u64))),
        ScalarKind::Bool => interned_bool(bits != 0),
    }
}

/// The inline bits of a boxed value, for defensive cross-representation ops.
/// # Safety: live runtime pointer.
unsafe fn scalar_unbox_val(kind: ScalarKind, v: *mut RtVal) -> i64 {
    match (kind, &(*v).payload) {
        (ScalarKind::I64, RtPayload::I64(x)) => *x,
        (ScalarKind::Ref, RtPayload::Ref(x)) => *x,
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
        ScalarKind::I64 | ScalarKind::Ref => a.cmp(&b),
        // A total order — see `cmp_vals`. An unboxed list of floats is the
        // one most likely to hold a NaN and be sorted.
        ScalarKind::F64 => f64::from_bits(a as u64).total_cmp(&f64::from_bits(b as u64)),
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
        ScalarKind::Ref => {
            let (i, gen) = unpack_ref(bits);
            format!("ref#{i}.{gen}")
        }
        ScalarKind::F64 => float_text(f64::from_bits(bits as u64)),
        ScalarKind::Bool => (bits != 0).to_string(),
    }
}

/// A list's elements, whichever representation it has.
enum Items<'a> {
    Boxed(&'a Vec<*mut RtVal>),
    Scalar(ScalarKind, &'a ScalarData),
}

/// # Safety: live runtime pointer.
unsafe fn items_of<'a>(v: *mut RtVal) -> Items<'a> {
    match &(*v).payload {
        RtPayload::List(items) => {
            Items::Boxed(std::mem::transmute::<&Vec<*mut RtVal>, &'a Vec<*mut RtVal>>(items))
        }
        RtPayload::ListScalar { kind, data } => {
            Items::Scalar(*kind, std::mem::transmute::<&ScalarData, &'a ScalarData>(data))
        }
        _ => grasm_panic_msg_str("expected a list"),
    }
}

/// A new empty scalar list of the given kind (0 = i64, 1 = f64, 2 = bool).
#[no_mangle]
pub extern "C" fn grasm_list_new_scalar(kind: i64) -> *mut RtVal {
    let k = ScalarKind::from_i64(kind);
    alloc(RtPayload::ListScalar {
        kind: k,
        data: ScalarData::new(k),
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
            data.get(i as usize)
        }
        // A sealed tuple is read the same way: destructuring a pattern is an
        // element read, and sealing changed only how the value prints.
        RtPayload::List(items) | RtPayload::Tuple(items) => {
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
            data.set(i as usize, bits);
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
            data: data.duplicate(),
        }),
        _ => grasm_panic_msg_str("list op on non-list"),
    };
    dec(list);
    fresh
}

#[no_mangle]
pub unsafe extern "C" fn grasm_list_len(v: *mut RtVal) -> i64 {
    match &(*v).payload {
        RtPayload::List(items) | RtPayload::Tuple(items) => items.len() as i64,
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
        RtPayload::List(items) | RtPayload::Tuple(items) => {
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
            scalar_box(*kind, data.get(i as usize))
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

/// Boxes the unit value. One allocation, like any other box: unit is rare
/// inside containers and not worth a shared singleton whose refcount every
/// container would have to respect.
#[no_mangle]
pub extern "C" fn grasm_box_unit() -> *mut RtVal {
    alloc(RtPayload::Unit)
}

/// Boxes a packed reference so it keeps its identity as one.
#[no_mangle]
pub extern "C" fn grasm_box_ref(v: i64) -> *mut RtVal {
    alloc(RtPayload::Ref(v))
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
        // A reference is one machine word, and the compiled backend keeps it
        // in an integer register. It boxes as itself so it can be told apart
        // when rendered, and unboxes back into that word.
        RtPayload::Ref(x) => *x,
        // Unit has one value and no bits. Something has to come back for
        // `let u = xs[0]` where the list holds units.
        RtPayload::Unit => 0,
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
    rt_stack_check();
    match &(*v).payload {
        RtPayload::Unit => "()".to_string(),
        // The slot it names, not what it points at — the value may already be
        // gone, and reading it to render would be the use-after-free the tier
        // exists to prevent.
        RtPayload::Ref(r) => {
            let (i, gen) = unpack_ref(*r);
            format!("ref#{i}.{gen}")
        }
        RtPayload::Tuple(items) => {
            let parts: Vec<String> = items.iter().map(|&i| display(i)).collect();
            format!("({})", parts.join(", "))
        }
        RtPayload::Struct { name, fields } => {
            let parts: Vec<String> = fields
                .iter()
                .map(|(k, fv)| format!("{k}: {}", display(*fv)))
                .collect();
            format!("{name}{{{}}}", parts.join(", "))
        }
        RtPayload::Variant { name, fields, .. } => {
            if fields.is_empty() {
                (*name).to_owned()
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
        RtPayload::F64(x) => float_text(*x),
        RtPayload::Bool(x) => x.to_string(),
        RtPayload::Str(s) => s.to_string(),
        RtPayload::List(items) => {
            let parts: Vec<String> = items.iter().map(|&i| display(i)).collect();
            format!("[{}]", parts.join(", "))
        }
        RtPayload::ListScalar { kind, data } => {
            let parts: Vec<String> = data.iter().map(|b| scalar_display(*kind, b)).collect();
            format!("[{}]", parts.join(", "))
        }
        RtPayload::Map(entries) => {
            // Printing is iteration, so it sorts ([E-MAP-ORDER]).
            let mut keys: Vec<&MapKey> = entries.keys().collect();
            keys.sort();
            let parts: Vec<String> = keys
                .into_iter()
                .map(|k| {
                    let kb = k.boxed();
                    let text = format!("{}: {}", display(kb), display(entries[k]));
                    dec(kb);
                    text
                })
                .collect();
            format!("{{{}}}", parts.join(", "))
        }
    }
}

// ---------------------------------------------------------------------------
// N3: structs, variants, fallibles, structural equality
// ---------------------------------------------------------------------------

unsafe fn take_str(ptr: *const u8, len: u64) -> String {
    borrowed_str(ptr, len).to_owned()
}

/// A name the compiler emitted into the binary's read-only data.
///
/// These live for the whole program, so a variant can hold the `&'static str`
/// rather than copying it. That copy used to happen on every construction and
/// on every `is` test — including the `Some`/`None` that every map lookup
/// builds, which is a heap allocation to say a six-letter word the binary
/// already contains.
unsafe fn borrowed_str(ptr: *const u8, len: u64) -> &'static str {
    let bytes = std::slice::from_raw_parts(ptr, len as usize);
    match std::str::from_utf8(bytes) {
        Ok(s) => s,
        Err(_) => grasm_panic_msg_str("compiler emitted a non-UTF-8 name"),
    }
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

/// Moves a field's value out, leaving a shared placeholder.
///
/// The caller becomes the sole owner, so a mutation through it happens where
/// the value lies instead of copying: `s.xs.push(v)` in a loop was O(n) per
/// push, because reading the field incremented its count and the push then
/// found it shared. The value is put back by `grasm_struct_set_field`, which
/// recognises a value returning to the field it came from.
///
/// Only when the struct itself is unshared. If it is not, taking would mutate
/// something another owner can see, so this reports failure by returning null
/// and the caller reads-and-copies as before.
///
/// # Safety: `s` is a struct this runtime allocated.
#[no_mangle]
pub unsafe extern "C" fn grasm_struct_take(
    s: *mut RtVal,
    name: *const u8,
    len: u64,
) -> *mut RtVal {
    if (*s).rc.get() != 1 {
        return std::ptr::null_mut();
    }
    let RtPayload::Struct { fields, .. } = &mut (*s).payload else {
        grasm_panic_msg_str("field access on non-struct");
    };
    let key = take_str(name, len);
    let hole = interned_bool(false);
    for (k, v) in fields.iter_mut() {
        if *k == key {
            return std::mem::replace(v, hole);
        }
    }
    grasm_panic_msg_str("no such field");
}

/// The same for a list element.
///
/// # Safety: `l` is a list this runtime allocated.
#[no_mangle]
pub unsafe extern "C" fn grasm_list_take(l: *mut RtVal, i: i64) -> *mut RtVal {
    if (*l).rc.get() != 1 {
        return std::ptr::null_mut();
    }
    let RtPayload::List(items) = &mut (*l).payload else {
        // A scalar list holds no pointers to take.
        return std::ptr::null_mut();
    };
    let n = items.len() as i64;
    let i = if i < 0 { n + i } else { i };
    if i < 0 || i >= n {
        return std::ptr::null_mut();
    }
    let hole = interned_bool(false);
    std::mem::replace(&mut items[i as usize], hole)
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
            // A value returning to the field it was taken from carries no
            // new reference, and the old one is the same object — decrementing
            // would free what is being stored.
            if *v != val {
                dec(*v);
            }
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
        type_name: borrowed_str(tp, tl),
        name: borrowed_str(vp, vl),
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
        RtPayload::Variant { name, .. } => i8::from(*name == borrowed_str(p, l)),
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
        type_name: "!",
        name: "Ok",
        fields: vec![val],
    })
}

/// # Safety: `val` ownership transfers.
#[no_mangle]
pub unsafe extern "C" fn grasm_err_new(val: *mut RtVal) -> *mut RtVal {
    alloc(RtPayload::Variant {
        type_name: "!",
        name: "Err",
        fields: vec![val],
    })
}

/// # Safety: live pointer.
#[no_mangle]
pub unsafe extern "C" fn grasm_is_ok(v: *mut RtVal) -> i8 {
    match &(*v).payload {
        RtPayload::Variant { name, .. } => i8::from(*name == "Ok"),
        _ => 0,
    }
}

/// # Safety: live pointer; prints the interpreter-identical line, exits 1.
#[no_mangle]
pub unsafe extern "C" fn grasm_report_main_err(v: *mut RtVal) {
    let RtPayload::Variant { fields, .. } = &(*v).payload else {
        rt_panic("bad main error value");
    };
    // stderr, for the same reason a panic goes there ([E-MAIN-ERR]).
    let _ = std::io::stdout().flush();
    let mut out = std::io::stderr().lock();
    let _ = writeln!(out, "error: unhandled error from main: {}", display(fields[0]));
    let _ = out.flush();
    std::process::exit(1);
}

/// Structural equality across all payloads.
/// # Safety: live pointers.
#[no_mangle]
pub unsafe extern "C" fn grasm_val_eq(a: *mut RtVal, b: *mut RtVal) -> i8 {
    // Iterative: the pairs still to compare live on the heap, not on the
    // stack. Recursing, `l == l` on a 100 000-deep list took SIGSEGV with an
    // empty stderr while the interpreter answered `true` — a crash where §11
    // promises no miscompile, and a disagreement between engines. The
    // interpreter walks deep values happily, so matching it means not being
    // bounded by the stack at all.
    let mut work = vec![(a, b)];
    while let Some((x, y)) = work.pop() {
        if !val_eq_step(x, y, &mut work) {
            return 0;
        }
    }
    1
}

/// One level of structural equality; children are pushed for the caller.
unsafe fn val_eq_step(a: *mut RtVal, b: *mut RtVal, work: &mut Vec<(*mut RtVal, *mut RtVal)>) -> bool {
    match (&(*a).payload, &(*b).payload) {
        (RtPayload::Map(x), RtPayload::Map(y)) => {
            if x.len() != y.len() {
                return false;
            }
            for (k, &v) in x.iter() {
                match y.get(k) {
                    Some(&w) => work.push((v, w)),
                    None => return false,
                }
            }
            true
        }
        (
            RtPayload::Struct { name: n1, fields: f1 },
            RtPayload::Struct { name: n2, fields: f2 },
        ) => {
            if n1 != n2 || f1.len() != f2.len() {
                return false;
            }
            for ((k1, v1), (k2, v2)) in f1.iter().zip(f2) {
                if k1 != k2 {
                    return false;
                }
                work.push((*v1, *v2));
            }
            true
        }
        (
            RtPayload::Variant { type_name: t1, name: n1, fields: f1 },
            RtPayload::Variant { type_name: t2, name: n2, fields: f2 },
        ) => {
            if t1 != t2 || n1 != n2 || f1.len() != f2.len() {
                return false;
            }
            for (&i, &j) in f1.iter().zip(f2) {
                work.push((i, j));
            }
            true
        }
        (RtPayload::List(x), RtPayload::List(y))
        | (RtPayload::Tuple(x), RtPayload::Tuple(y)) => {
            if x.len() != y.len() {
                return false;
            }
            for (&i, &j) in x.iter().zip(y) {
                work.push((i, j));
            }
            true
        }
        // Everything else has no children to queue.
        _ => val_eq(a, b),
    }
}

unsafe fn val_eq(a: *mut RtVal, b: *mut RtVal) -> bool {
    rt_stack_check();
    match (&(*a).payload, &(*b).payload) {
        (RtPayload::I64(x), RtPayload::I64(y)) => x == y,
        // Unit has exactly one value, so two of them are always equal.
        (RtPayload::Unit, RtPayload::Unit) => true,
        // Two references are the same reference when they name the same slot
        // at the same generation — identity, not the values they point at.
        (RtPayload::Ref(x), RtPayload::Ref(y)) => x == y,
        (RtPayload::F64(x), RtPayload::F64(y)) => x == y,
        (RtPayload::Bool(x), RtPayload::Bool(y)) => x == y,
        (RtPayload::Str(x), RtPayload::Str(y)) => x == y,
        // Two function values are the same function when they run the same
        // code over the same captures. Falling through to "not equal" made
        // `dbl == dbl` false and `assert_eq(dbl, dbl)` fail — the one type
        // that was not equal to itself, which also broke deduplicating a list
        // of them ([E-EQ-STRUCTURAL]).
        (
            RtPayload::Fn { code: c1, env: e1, .. },
            RtPayload::Fn { code: c2, env: e2, .. },
        ) => c1 == c2 && (e1 == e2 || (!e1.is_null() && !e2.is_null() && val_eq(*e1, *e2))),
        (RtPayload::Tuple(x), RtPayload::Tuple(y))
        | (RtPayload::List(x), RtPayload::List(y)) => {
            x.len() == y.len() && x.iter().zip(y).all(|(&i, &j)| val_eq(i, j))
        }
        (
            RtPayload::ListScalar { kind: k1, data: d1 },
            RtPayload::ListScalar { kind: k2, data: d2 },
        ) => {
            k1 == k2
                && d1.len() == d2.len()
                && d1.iter().zip(d2.iter()).all(|(x, y)| scalar_eq(*k1, x, y))
        }
        // The same logical list can arrive in either representation (a
        // literal is scalar; `keys()` is boxed), so equality must cross.
        (RtPayload::ListScalar { kind, data }, RtPayload::List(items))
        | (RtPayload::List(items), RtPayload::ListScalar { kind, data }) => {
            data.len() == items.len()
                && data
                    .iter()
                    .zip(items)
                    .all(|(bits, &boxed)| match (*kind, &(*boxed).payload) {
                        (ScalarKind::I64, RtPayload::I64(x)) => bits == *x,
                        (ScalarKind::Ref, RtPayload::Ref(x)) => bits == *x,
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
                    .all(|(k, &v)| y.get(k).is_some_and(|&w| val_eq(v, w)))
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
                some_of(scalar_box(*kind, data.get(i as usize)))
            }
        }
        RtPayload::List(items) => {
            if i < 0 || i as usize >= items.len() {
                alloc(RtPayload::Variant {
                    type_name: "Option",
                    name: "None",
                    fields: vec![],
                })
            } else {
                let e = items[i as usize];
                inc(e);
                alloc(RtPayload::Variant {
                    type_name: "Option",
                    name: "Some",
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
    alloc(RtPayload::Str(RtStr::new(display(v))))
}

// ---------------------------------------------------------------------------
// Maps and the builtin method surface
// ---------------------------------------------------------------------------

// --- maps (sorted, deterministic iteration like the interpreter) ---

#[no_mangle]
pub extern "C" fn grasm_map_new() -> *mut RtVal {
    alloc(RtPayload::Map(std::collections::HashMap::new()))
}

/// A map's keys in the order iteration must present them.
fn sorted_keys(entries: &std::collections::HashMap<MapKey, *mut RtVal>) -> Vec<MapKey> {
    let mut keys: Vec<MapKey> = entries.keys().cloned().collect();
    keys.sort();
    keys
}


unsafe fn cow_map(m: *mut RtVal) -> *mut RtVal {
    if (*m).rc.get() == 1 {
        return m;
    }
    let RtPayload::Map(entries) = &(*m).payload else {
        grasm_panic_msg_str("map op on non-map");
    };
    let cloned: std::collections::HashMap<MapKey, *mut RtVal> = entries
        .iter()
        .map(|(k, &v)| {
            inc(v);
            (k.clone(), v)
        })
        .collect();
    dec(m);
    alloc(RtPayload::Map(cloned))
}

/// # Safety: key/value ownership transfers; returns the (possibly new) map.
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
    // Overwriting an existing entry keeps the key already in the table, so
    // only a genuinely new key has to be built and owned. That is the rare
    // case: a counting loop misses once per distinct word and hits every
    // time after.
    match with_key(k, |kk| entries.get_mut(kk).map(std::ptr::from_mut)) {
        Some(slot) => {
            dec(std::mem::replace(&mut *slot, v));
        }
        None => {
            entries.insert(MapKey::of(k), v);
        }
    }
    // The key is owned by the caller's +1; the table holds an owned MapKey.
    dec(k);
    target
}

/// # Safety: live pointers; returns Option variant (+1).
#[no_mangle]
pub unsafe extern "C" fn grasm_map_get(m: *mut RtVal, k: *mut RtVal) -> *mut RtVal {
    let RtPayload::Map(entries) = &(*m).payload else {
        grasm_panic_msg_str("get on non-map");
    };
    if let Some(ev) = with_key(k, |kk| entries.get(kk).copied()) {
        inc(ev);
        return alloc(RtPayload::Variant {
            type_name: "Option",
            name: "Some",
            fields: vec![ev],
        });
    }
    alloc(RtPayload::Variant {
        type_name: "Option",
        name: "None",
        fields: vec![],
    })
}

/// # Safety: live pointers.
#[no_mangle]
pub unsafe extern "C" fn grasm_map_has(m: *mut RtVal, k: *mut RtVal) -> i8 {
    let RtPayload::Map(entries) = &(*m).payload else {
        grasm_panic_msg_str("has on non-map");
    };
    i8::from(with_key(k, |kk| entries.contains_key(kk)))
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
    match with_key(k, |kk| entries.remove(kk)) {
        Some(ev) => {
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
    // Iteration is where the ordering guarantee is paid for.
    let items: Vec<*mut RtVal> = sorted_keys(entries).iter().map(MapKey::boxed).collect();
    alloc(RtPayload::List(items))
}

/// Panics with a message that is a runtime string rather than a literal.
///
/// The native backend could only panic with a compile-time literal, so
/// `panic(msg)` on a computed message and `expect` on an Option both had
/// nowhere to put the text.
///
/// # Safety: live pointer to a `str`.
#[no_mangle]
pub unsafe extern "C" fn grasm_panic_value(v: *mut RtVal) -> ! {
    let msg = match &(*v).payload {
        RtPayload::Str(s) => s.to_string(),
        other => {
            let _ = other;
            display(v)
        }
    };
    rt_panic(&msg)
}

/// Key/value pairs in sorted key order, as a list of two-element tuples.
///
/// `keys()` and `values()` are each half of this, and reaching a pair by
/// looking one up in the other was the only way to iterate a map — which
/// means a lookup per entry, with a `?? default` for a key that certainly
/// exists.
///
/// # Safety: live pointer; returns a fresh list (+1).
#[no_mangle]
pub unsafe extern "C" fn grasm_map_entries(m: *mut RtVal) -> *mut RtVal {
    let RtPayload::Map(entries) = &(*m).payload else {
        grasm_panic_msg_str("entries on non-map");
    };
    let items: Vec<*mut RtVal> = sorted_keys(entries)
        .iter()
        .map(|k| {
            let v = entries[k];
            inc(v);
            alloc(RtPayload::Tuple(vec![k.boxed(), v]))
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
    let items: Vec<*mut RtVal> = sorted_keys(entries)
        .iter()
        .map(|k| {
            let v = entries[k];
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
            std::mem::transmute::<&str, &'static str>(&**s)
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
            .map(|c| alloc(RtPayload::Str(RtStr::new(c.to_string()))))
            .collect()
    } else {
        text.split(sp)
            .map(|p| alloc(RtPayload::Str(RtStr::new(p.to_string()))))
            .collect()
    };
    alloc(RtPayload::List(parts))
}

/// # Safety: live pointer.
/// `x` as text with exactly `places` digits after the point.
///
/// Both engines format through Rust, so they agree digit for digit — which is
/// the only property that matters here and the one a hand-rolled formatter
/// built from `round` cannot promise.
#[no_mangle]
pub extern "C" fn grasm_f64_to_fixed(x: f64, places: i64) -> *mut RtVal {
    if !(0..=17).contains(&places) {
        rt_panic(&format!("to_fixed({places}): places must be 0 to 17"));
    }
    alloc(RtPayload::Str(RtStr::new(fixed_text(x, places))))
}

// ---------------------------------------------------------------------------
// Tier 2 — generational references
// ---------------------------------------------------------------------------

/// The shared heap `ref T` points into.
///
/// A reference is a slot index and the generation that slot held when the
/// reference was taken, packed into one 64-bit word. Releasing a slot bumps
/// its generation, so every reference taken before that stops matching — and
/// reading through one is a deterministic panic rather than a read of whatever
/// occupies the slot now. That is the whole safety property: a stale reference
/// is *always* caught, never undefined behaviour, and the slot is free to be
/// reused immediately.
///
/// Slots are per process and never shrink; a released slot is reused, which is
/// what keeps a long-lived graph from growing without bound.
struct Slab {
    /// `None` once released. The generation keeps counting either way.
    slots: Vec<Option<*mut RtVal>>,
    gens: Vec<u32>,
    free: Vec<u32>,
}

/// One heap for the whole process, not one per thread.
///
/// It was `thread_local`, which is wrong in a way that produces silently
/// correct-looking programs: a `par_map` worker allocating a `ref` put it in
/// *its own* slab, and reading that reference from the main thread looked up
/// the same slot number in a different heap. References came back dead though
/// nothing was released, or alive holding another node's data — the "stale or
/// wrong value" [T-REF-STALE] says cannot happen. Sequential code never saw
/// it, because there was only ever one slab.
///
/// A mutex is the right cost here: taking a reference is rare next to reading
/// through one, and correctness across threads is the entire point of a shared
/// heap.
static SLAB: std::sync::Mutex<Slab> = std::sync::Mutex::new(Slab {
    slots: Vec::new(),
    gens: Vec::new(),
    free: Vec::new(),
});

/// The values in the slab are shared across threads by construction; the mutex
/// is what orders access to them.
unsafe impl Send for Slab {}

fn pack_ref(index: u32, gen: u32) -> i64 {
    ((index as i64) << 32) | (gen as i64 & 0xffff_ffff)
}

fn unpack_ref(r: i64) -> (u32, u32) {
    (((r >> 32) & 0xffff_ffff) as u32, (r & 0xffff_ffff) as u32)
}

/// Puts a value in the shared heap and returns a reference to it.
///
/// # Safety: `v` is a live pointer whose ownership passes to the slab.
#[no_mangle]
pub unsafe extern "C" fn grasm_ref_new(v: *mut RtVal) -> i64 {
    {
        let mut s = SLAB.lock().expect("slab");
        match s.free.pop() {
            Some(i) => {
                let idx = i as usize;
                s.slots[idx] = Some(v);
                pack_ref(i, s.gens[idx])
            }
            None => {
                let i = s.slots.len() as u32;
                s.slots.push(Some(v));
                s.gens.push(0);
                pack_ref(i, 0)
            }
        }
    }
}

/// The value a reference points at, or a panic if it is stale.
///
/// Hands out an *owned* reference — incremented before returning — because
/// every caller of a field read decrements its receiver when it is done. A
/// borrowed pointer here meant the first read through a `ref` dropped the
/// slab's own count, and the slot was left pointing at freed memory: the next
/// `release` corrupted the heap, which surfaced as SIGTRAP rather than
/// anything a reader could act on.
///
/// # Safety: the caller takes ownership of the returned reference.
#[no_mangle]
pub unsafe extern "C" fn grasm_ref_get(r: i64) -> *mut RtVal {
    let (i, gen) = unpack_ref(r);
    let found = {
        let s = SLAB.lock().expect("slab");
        let idx = i as usize;
        if idx >= s.slots.len() || s.gens[idx] != gen {
            None
        } else {
            s.slots[idx]
        }
    };
    match found {
        Some(p) => {
            inc(p);
            p
        }
        None => rt_panic("stale reference: the value it pointed to was released"),
    }
}

/// Replaces what a reference points at. Panics if it is stale.
///
/// # Safety: `v` is a live pointer whose ownership passes to the slab.
#[no_mangle]
pub unsafe extern "C" fn grasm_ref_set(r: i64, v: *mut RtVal) {
    let (i, gen) = unpack_ref(r);
    let old = {
        let mut s = SLAB.lock().expect("slab");
        let idx = i as usize;
        if idx >= s.slots.len() || s.gens[idx] != gen || s.slots[idx].is_none() {
            Err(())
        } else {
            Ok(s.slots[idx].replace(v))
        }
    };
    match old {
        Ok(prev) => {
            if let Some(p) = prev {
                dec(p);
            }
        }
        Err(()) => rt_panic("stale reference: the value it pointed to was released"),
    }
}

/// Borrows what a reference points at, having first made the slab its only
/// owner. Panics if the reference is stale.
///
/// A write through a reference has to be seen by every other holder of it
/// ([T-REF-GENERATIONAL]), which means mutating the value in the slot rather
/// than rebuilding a copy and storing it back. The mutators do that already —
/// `grasm_struct_set_field` and `cow_list` both mutate in place when the value
/// has one owner — so the only thing needed is to make that true, and to hand
/// back a pointer the caller must *not* release.
///
/// Rebuilding instead was wrong in a way only a cycle showed: `a.out[0].name`
/// and `a.out[0].out[0].name`, where the second path arrives back at `a`, took
/// a copy of `a` at the outer hop, wrote the inner name into the slab, and
/// then stored the outer copy over it. The inner write vanished.
///
/// # Safety: the returned pointer is borrowed — the slab keeps ownership.
#[no_mangle]
pub unsafe extern "C" fn grasm_ref_make_unique(r: i64) -> *mut RtVal {
    let (i, gen) = unpack_ref(r);
    let mut s = SLAB.lock().expect("slab");
    let idx = i as usize;
    if idx >= s.slots.len() || s.gens[idx] != gen {
        drop(s);
        rt_panic("stale reference: the value it pointed to was released");
    }
    let Some(p) = s.slots[idx] else {
        drop(s);
        rt_panic("stale reference: the value it pointed to was released");
    };
    if (*p).rc.get() == 1 {
        return p;
    }
    // Someone else is holding this value, so mutating it in place would be
    // visible to them as well — and value semantics say it must not be
    // ([E-VALUE-SEMANTICS]). Clone into the slot; the reference still names
    // the same slot, so every holder of the *reference* sees what follows.
    let fresh = match &(*p).payload {
        RtPayload::Struct { name, fields } => {
            let cloned: Vec<(String, *mut RtVal)> = fields
                .iter()
                .map(|(k, v)| {
                    inc(*v);
                    (k.clone(), *v)
                })
                .collect();
            alloc(RtPayload::Struct {
                name: name.clone(),
                fields: cloned,
            })
        }
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
        RtPayload::ListScalar { kind, data } => alloc(RtPayload::ListScalar {
            kind: *kind,
            data: data.duplicate(),
        }),
        // Nothing else is written *into*: a reference to a scalar is replaced
        // whole, which `grasm_ref_set` already does.
        _ => return p,
    };
    s.slots[idx] = Some(fresh);
    drop(s);
    dec(p);
    fresh
}

/// Releases a slot. Every reference to it becomes stale.
///
/// Releasing an already-released slot is a no-op, not a panic: a double
/// release is not a use-after-free and reporting it would make cleanup order
/// something a caller has to track.
#[no_mangle]
pub extern "C" fn grasm_ref_release(r: i64) {
    let (i, gen) = unpack_ref(r);
    let taken = {
        let mut s = SLAB.lock().expect("slab");
        let idx = i as usize;
        if idx >= s.slots.len() || s.gens[idx] != gen {
            return;
        }
        let v = s.slots[idx].take();
        // The generation moves whether or not anything was there, so a
        // reference taken before this can never match again.
        //
        // A generation is 32 bits — a reference is one machine word, index and
        // generation packed together — and wrapping it would hand a slot's
        // identity back. After 2^32 release/allocate cycles, reachable in
        // eleven minutes here, a reference released at the start of the
        // program read the current occupant's value and compared equal to a
        // live one. A slot whose generation cannot advance is retired instead
        // of reused: with no new occupant, nothing can match it again.
        if let Some(next) = s.gens[idx].checked_add(1) {
            s.gens[idx] = next;
            s.free.push(i);
        }
        v
    };
    if let Some(p) = taken {
        unsafe { dec(p) };
    }
}

/// Whether a reference still points at something.
#[no_mangle]
pub extern "C" fn grasm_ref_alive(r: i64) -> i8 {
    let (i, gen) = unpack_ref(r);
    let s = SLAB.lock().expect("slab");
    let idx = i as usize;
    i8::from(idx < s.slots.len() && s.gens[idx] == gen && s.slots[idx].is_some())
}

thread_local! {
    /// The lowest stack address this thread may use, or 0 before the first
    /// check on it. Per thread, because a worker's stack is somewhere else
    /// entirely and a limit from another one says nothing about it.
    static STACK_FLOOR: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
}

/// How much stack a program may use below its entry frame.
///
/// Under the 8 MiB a main thread usually gets, this leaves room for the panic
/// itself to be reported — which is the whole point of stopping early.
const STACK_BUDGET: usize = 6 * 1024 * 1024;

/// The stack a `par_map` / `par_filter` worker gets.
///
/// It has to be larger than `STACK_BUDGET`, and by enough to report a panic.
/// A worker took the platform default — 2 MiB — so the floor computed on its
/// first check sat *below the end of its own stack* and the guard could never
/// fire. Deep recursion inside a parallel closure then ran off the stack for
/// real: SIGBUS, exit 138, nothing on stderr, in the one place
/// [E-RECURSION-DEPTH] promises a panic and exit 101. The sequential fallback
/// for small inputs hid it, so it only appeared past a few hundred elements.
const WORKER_STACK: usize = 8 * 1024 * 1024;

/// The largest string this implementation will build. See the interpreter.
const MAX_ALLOC: usize = 1 << 33;

/// The same check, for the runtime's own recursive walks.
///
/// `grasm_stack_check` is emitted at the top of every compiled function that
/// can recurse, which covers what a *program* does. It does not cover what the
/// runtime does on the program's behalf: comparing or rendering a value walks
/// it recursively, so `l == l` on a 100 000-deep list took SIGSEGV with an
/// empty stderr, where the interpreter answered `true`. [E-RECURSION-DEPTH]
/// promises a panic and exit 101 — "it does not abort, crash, or corrupt the
/// process" — and §11 promises the compiled backend never miscompiles.
fn rt_stack_check() {
    let here = 0u8;
    let sp = std::ptr::addr_of!(here) as usize;
    let floor = STACK_FLOOR.with(std::cell::Cell::get);
    if floor == 0 {
        STACK_FLOOR.with(|c| c.set(sp.saturating_sub(STACK_BUDGET)));
        return;
    }
    if sp < floor {
        rt_panic("stack overflow: recursion too deep");
    }
}

/// Panics if the stack has run down past this thread's floor.
///
/// Called on entry to every function that can recurse. [E-RECURSION-DEPTH]
/// requires a panic and exit 101 past the bound — "it does not abort, crash,
/// or corrupt the process" — and without this a deep enough recursion took
/// SIGSEGV, exit 139, with nothing on stderr at all.
///
/// # Safety: `sp` is an address in the current thread's stack.
#[no_mangle]
pub extern "C" fn grasm_stack_check(sp: i64) {
    let sp = sp as usize;
    let floor = STACK_FLOOR.with(std::cell::Cell::get);
    if floor == 0 {
        STACK_FLOOR.with(|c| c.set(sp.saturating_sub(STACK_BUDGET)));
        return;
    }
    if sp < floor {
        rt_panic("stack overflow: recursion too deep");
    }
}

/// Whether `x` is NaN.
#[no_mangle]
pub extern "C" fn grasm_f64_is_nan(x: f64) -> i8 {
    i8::from(x.is_nan())
}

/// Whether `x` is neither NaN nor an infinity.
#[no_mangle]
pub extern "C" fn grasm_f64_is_finite(x: f64) -> i8 {
    i8::from(x.is_finite())
}

/// `x` as text with exactly `places` digits after the point, ties **away from
/// zero** — the same rule `round` uses ([L-LIB-ROUND]).
pub fn fixed_text(x: f64, places: i64) -> String {
    let p = places as usize;
    if !x.is_finite() {
        return format!("{x:.p$}");
    }
    // Rust's formatting rounds half to *even*, and `round` rounds half away
    // from zero — two rules in one language, in the function every report's
    // money column goes through. This keeps `round`'s rule.
    //
    // Only an exact tie is affected, and telling one apart from a value that
    // merely looks like one has to be done on the true stored value: 2.675 is
    // really 2.67499…, so it is not a tie and must round down. An exact tie is
    // a terminating decimal, so looking well past the requested precision
    // distinguishes "5000…" from "4999…" without any scaling error.
    const LOOKAHEAD: usize = 25;
    let wide = format!("{:.*}", p + LOOKAHEAD, x.abs());
    let (head, tail) = wide.split_at(wide.len() - LOOKAHEAD);
    let tie = tail.as_bytes()[0] == b'5' && tail.as_bytes()[1..].iter().all(|&b| b == b'0');
    if !tie {
        return format!("{x:.p$}");
    }
    let bumped = bump_last_digit(head.trim_end_matches('.'));
    if x.is_sign_negative() {
        format!("-{bumped}")
    } else {
        bumped
    }
}

/// A decimal string plus one unit in its last place, carrying as needed.
fn bump_last_digit(s: &str) -> String {
    let mut digits: Vec<u8> = s.bytes().collect();
    let mut i = digits.len();
    loop {
        if i == 0 {
            digits.insert(0, b'1');
            break;
        }
        i -= 1;
        if digits[i] == b'.' {
            continue;
        }
        if digits[i] == b'9' {
            digits[i] = b'0';
            continue;
        }
        digits[i] += 1;
        break;
    }
    String::from_utf8(digits).expect("digits stay ascii")
}


/// The Unicode code point of character `i`, counting characters not bytes.
///
/// # Safety: live pointer.
#[no_mangle]
pub unsafe extern "C" fn grasm_str_code_at(s: *mut RtVal, i: i64) -> i64 {
    let RtPayload::Str(gs) = &(*s).payload else {
        grasm_panic_msg_str("code_at on non-string");
    };
    let n = gs.char_len() as i64;
    if i < 0 || i >= n {
        rt_panic(&format!("code_at {i} out of bounds (len {n})"));
    }
    // Constant time whatever the text: ASCII indexes directly, anything else
    // builds a character index once.
    gs.char_at(i as usize).expect("in bounds") as i64
}

/// The whitespace-separated words of a string, with no empties.
///
/// # Safety: live pointer.
#[no_mangle]
pub unsafe extern "C" fn grasm_str_words(s: *mut RtVal) -> *mut RtVal {
    let items: Vec<*mut RtVal> = as_str(s)
        .split_whitespace()
        .map(|w| alloc(RtPayload::Str(RtStr::new(w.to_string()))))
        .collect();
    alloc(RtPayload::List(items))
}

/// Every code point, in order.
///
/// # Safety: live pointer.
#[no_mangle]
pub unsafe extern "C" fn grasm_str_codes(s: *mut RtVal) -> *mut RtVal {
    let mut data = ScalarData::new(ScalarKind::I64);
    for c in as_str(s).chars() {
        data.push(c as i64);
    }
    alloc(RtPayload::ListScalar {
        kind: ScalarKind::I64,
        data,
    })
}

/// A code point back to a one-character string.
#[no_mangle]
pub extern "C" fn grasm_i64_to_char(n: i64) -> *mut RtVal {
    match u32::try_from(n).ok().and_then(char::from_u32) {
        Some(c) => alloc(RtPayload::Str(RtStr::new(c.to_string()))),
        None => rt_panic(&format!("{n} is not a Unicode code point")),
    }
}

#[no_mangle]
pub unsafe extern "C" fn grasm_str_chars(s: *mut RtVal) -> *mut RtVal {
    let items: Vec<*mut RtVal> = as_str(s)
        .chars()
        .map(|c| alloc(RtPayload::Str(RtStr::new(c.to_string()))))
        .collect();
    alloc(RtPayload::List(items))
}

/// # Safety: live pointer.
#[no_mangle]
pub unsafe extern "C" fn grasm_str_trim(s: *mut RtVal) -> *mut RtVal {
    alloc(RtPayload::Str(RtStr::new(as_str(s).trim().to_string())))
}

/// # Safety: live pointer.
#[no_mangle]
pub unsafe extern "C" fn grasm_str_trim_start(s: *mut RtVal) -> *mut RtVal {
    alloc(RtPayload::Str(RtStr::new(as_str(s).trim_start().to_string())))
}

/// # Safety: live pointer.
#[no_mangle]
pub unsafe extern "C" fn grasm_str_trim_end(s: *mut RtVal) -> *mut RtVal {
    alloc(RtPayload::Str(RtStr::new(as_str(s).trim_end().to_string())))
}

/// # Safety: live pointer.
#[no_mangle]
pub unsafe extern "C" fn grasm_str_upper(s: *mut RtVal) -> *mut RtVal {
    alloc(RtPayload::Str(RtStr::new(as_str(s).to_uppercase())))
}

/// # Safety: live pointer.
#[no_mangle]
pub unsafe extern "C" fn grasm_str_lower(s: *mut RtVal) -> *mut RtVal {
    alloc(RtPayload::Str(RtStr::new(as_str(s).to_lowercase())))
}

/// # Safety: live pointer.
#[no_mangle]
pub unsafe extern "C" fn grasm_str_repeat(s: *mut RtVal, n: i64) -> *mut RtVal {
    if n < 0 {
        rt_panic("repeat count must be a non-negative int");
    }
    let text = as_str(s);
    match (n as usize).checked_mul(text.len()) {
        Some(bytes) if bytes <= MAX_ALLOC => {}
        _ => rt_panic(&format!(
            "repeat would need more memory than a string can hold ({n} copies)"
        )),
    }
    alloc(RtPayload::Str(RtStr::new(text.repeat(n as usize))))
}

/// # Safety: live pointers.
#[no_mangle]
pub unsafe extern "C" fn grasm_str_replace(
    s: *mut RtVal,
    from: *mut RtVal,
    to: *mut RtVal,
) -> *mut RtVal {
    alloc(RtPayload::Str(RtStr::new(as_str(s).replace(as_str(from), as_str(to)))))
}

/// # Safety: live pointer.
#[no_mangle]
pub unsafe extern "C" fn grasm_str_substr(s: *mut RtVal, start: i64, len: i64) -> *mut RtVal {
    let RtPayload::Str(gs) = &(*s).payload else {
        grasm_panic_msg_str("substr on non-string");
    };
    // Through the character index, as `code_at` is.
    let n = gs.char_len() as i64;
    let start = start.clamp(0, n) as usize;
    // Saturating: see the interpreter — `substr(2, i64::MAX)` overflowed.
    let end = (start as i64).saturating_add(len.max(0)).clamp(0, n) as usize;
    let a = gs.byte_at(start).unwrap_or(0);
    let b = gs.byte_at(end).unwrap_or(gs.len());
    alloc(RtPayload::Str(RtStr::new(gs[a..b].to_string())))
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
        type_name: "Option",
        name: "None",
        fields: vec![],
    })
}

fn some_of(v: *mut RtVal) -> *mut RtVal {
    alloc(RtPayload::Variant {
        type_name: "Option",
        name: "Some",
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
        Items::Scalar(kind, data) => data.iter().map(|b| scalar_display(kind, b)).collect(),
    };
    alloc(RtPayload::Str(RtStr::new(parts.join(as_str(sep)))))
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
        Items::Scalar(kind, data) => alloc(RtPayload::ListScalar {
            kind,
            data: data.reversed(),
        }),
    }
}

unsafe fn cmp_vals(a: *mut RtVal, b: *mut RtVal) -> std::cmp::Ordering {
    use std::cmp::Ordering;
    match (&(*a).payload, &(*b).payload) {
        (RtPayload::I64(x), RtPayload::I64(y)) => x.cmp(y),
        // A total order, so a sort stays a sort even with a NaN in the list
        // ([L-LIB-SORT-NAN]).
        (RtPayload::F64(x), RtPayload::F64(y)) => x.total_cmp(y),
        (RtPayload::Str(x), RtPayload::Str(y)) => x.cmp(y),
        // A tuple of ordered things is ordered, component by component — this
        // is what a multi-key sort is.
        (RtPayload::Tuple(x), RtPayload::Tuple(y)) => {
            for (&a, &b) in x.iter().zip(y.iter()) {
                let ord = cmp_vals(a, b);
                if ord != Ordering::Equal {
                    return ord;
                }
            }
            x.len().cmp(&y.len())
        }
        _ => Ordering::Equal,
    }
}

/// `assert_eq(a, b)`: panics naming both values when they differ.
///
/// The values are what makes the message useful — "assertion failed" alone
/// tells you a test broke and nothing about how — so they are boxed and
/// compared here rather than reduced to a bool at the call site.
///
/// # Safety: live pointers.
#[no_mangle]
pub unsafe extern "C" fn grasm_assert_eq(a: *mut RtVal, b: *mut RtVal) {
    if val_eq(a, b) {
        dec(a);
        dec(b);
        return;
    }
    rt_panic(&format!(
        "assertion failed: `{}` != `{}`",
        display(a),
        display(b)
    ));
}

/// Three-way comparison of two boxed values: -1, 0 or 1.
///
/// # Safety: live pointers.
#[no_mangle]
pub unsafe extern "C" fn grasm_val_cmp(a: *mut RtVal, b: *mut RtVal) -> i64 {
    match cmp_vals(a, b) {
        std::cmp::Ordering::Less => -1,
        std::cmp::Ordering::Equal => 0,
        std::cmp::Ordering::Greater => 1,
    }
}

/// In-place sort honoring CoW; returns the (possibly new) list.
/// # Safety: live pointer.
#[no_mangle]
pub unsafe extern "C" fn grasm_list_sort(v: *mut RtVal) -> *mut RtVal {
    let target = cow_list(v);
    match &mut (*target).payload {
        RtPayload::List(items) => items.sort_by(|&a, &b| cmp_vals(a, b)),
        RtPayload::ListScalar { kind, data } => data.sort(*kind),
        _ => unreachable!(),
    }
    target
}

/// Removes the element at `i`, writing it to `out`. Panics out of range, the
/// way indexing does.
///
/// # Safety: live pointers.
#[no_mangle]
pub unsafe extern "C" fn grasm_list_remove(v: *mut RtVal, i: i64, out: *mut *mut RtVal) -> *mut RtVal {
    let n = grasm_list_len(v);
    if i < 0 || i >= n {
        rt_panic(&format!("remove {i} out of bounds (len {n})"));
    }
    *out = grasm_list_get(v, i);
    let target = cow_list(v);
    match &mut (*target).payload {
        RtPayload::List(items) => {
            let removed = items.remove(i as usize);
            dec(removed);
        }
        RtPayload::ListScalar { kind, data } => {
            let kept: Vec<i64> = (0..data.len())
                .filter(|&j| j != i as usize)
                .map(|j| data.get(j))
                .collect();
            *data = ScalarData::from_words(*kind, kept);
        }
        _ => unreachable!(),
    }
    target
}

/// `[(index, element)]`, so a loop can name both.
///
/// # Safety: live pointer.
#[no_mangle]
pub unsafe extern "C" fn grasm_list_enumerate(v: *mut RtVal) -> *mut RtVal {
    let n = grasm_list_len(v);
    let items: Vec<*mut RtVal> = (0..n)
        .map(|i| alloc(RtPayload::Tuple(vec![grasm_box_i64(i), grasm_list_get(v, i)])))
        .collect();
    alloc(RtPayload::List(items))
}

/// `[(a, b)]` up to the shorter of the two — pairing past the end of one
/// would have to invent an element.
///
/// # Safety: live pointers.
#[no_mangle]
pub unsafe extern "C" fn grasm_list_zip(a: *mut RtVal, b: *mut RtVal) -> *mut RtVal {
    let n = grasm_list_len(a).min(grasm_list_len(b));
    let items: Vec<*mut RtVal> = (0..n)
        .map(|i| alloc(RtPayload::Tuple(vec![grasm_list_get(a, i), grasm_list_get(b, i)])))
        .collect();
    alloc(RtPayload::List(items))
}

/// Sorts `v` by a parallel list of keys, one per element, computed by the
/// caller. Stable, so elements with equal keys keep their written order.
///
/// # Safety: live pointers, and `keys` is the same length as `v`.
#[no_mangle]
pub unsafe extern "C" fn grasm_list_sort_by(v: *mut RtVal, keys: *mut RtVal) -> *mut RtVal {
    let n = grasm_list_len(v) as usize;
    if grasm_list_len(keys) as usize != n {
        rt_panic("sort_by: one key per element");
    }
    // Sort the positions, then permute — the elements themselves may be
    // unboxed, and the keys may be a different representation again.
    let mut order: Vec<usize> = (0..n).collect();
    let key_at = |i: usize| grasm_list_get(keys, i as i64);
    order.sort_by(|&a, &b| {
        let (ka, kb) = (key_at(a), key_at(b));
        let ord = cmp_vals(ka, kb);
        dec(ka);
        dec(kb);
        ord
    });
    let target = cow_list(v);
    match &mut (*target).payload {
        RtPayload::List(items) => {
            let src = items.clone();
            for (dst, &from) in order.iter().enumerate() {
                items[dst] = src[from];
            }
        }
        RtPayload::ListScalar { kind, data } => {
            let src = data.slice(0, n);
            for (dst, &from) in order.iter().enumerate() {
                data.set(dst, src.get(from));
            }
            let _ = kind;
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
            let out = if s <= e {
                data.slice(s, e)
            } else {
                ScalarData::new(kind)
            };
            alloc(RtPayload::ListScalar { kind, data: out })
        }
    }
}

/// Concatenation, `a + b`. Both operands keep their own representation, so a
/// scalar list and a boxed one can meet; the result is boxed only if it has
/// to be.
///
/// # Safety: live pointers.
#[no_mangle]
pub unsafe extern "C" fn grasm_list_concat(a: *mut RtVal, b: *mut RtVal) -> *mut RtVal {
    match (items_of(a), items_of(b)) {
        (Items::Scalar(ka, da), Items::Scalar(kb, db)) if ka == kb => {
            let mut out = da.slice(0, da.len());
            for i in 0..db.len() {
                out.push(db.get(i));
            }
            alloc(RtPayload::ListScalar { kind: ka, data: out })
        }
        (ia, ib) => {
            let mut out: Vec<*mut RtVal> = Vec::new();
            for items in [ia, ib] {
                match items {
                    Items::Boxed(v) => {
                        for &i in v {
                            inc(i);
                            out.push(i);
                        }
                    }
                    Items::Scalar(kind, data) => {
                        for i in 0..data.len() {
                            out.push(scalar_box(kind, data.get(i)));
                        }
                    }
                }
            }
            alloc(RtPayload::List(out))
        }
    }
}

/// # Safety: live pointers.
#[no_mangle]
pub unsafe extern "C" fn grasm_list_contains(v: *mut RtVal, needle: *mut RtVal) -> i8 {
    let hit = match items_of(v) {
        Items::Boxed(items) => items.iter().any(|&i| val_eq(i, needle)),
        Items::Scalar(kind, data) => match (kind, &(*needle).payload) {
            (ScalarKind::I64, RtPayload::I64(x)) => data.iter().any(|b| b == *x),
            (ScalarKind::Ref, RtPayload::Ref(x)) => data.iter().any(|b| b == *x),
            (ScalarKind::F64, RtPayload::F64(x)) => {
                data.iter().any(|b| f64::from_bits(b as u64) == *x)
            }
            (ScalarKind::Bool, RtPayload::Bool(x)) => {
                data.iter().any(|b| (b != 0) == *x)
            }
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
        Items::Scalar(kind, data) => match data.iter().next() {
            Some(b) => some_of(scalar_box(kind, b)),
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
        Items::Scalar(kind, data) => match data.iter().last() {
            Some(b) => some_of(scalar_box(kind, b)),
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
        Items::Scalar(ScalarKind::I64, data) => match data.as_words() {
            Some(w) => w.iter().sum(),
            None => data.iter().sum(),
        },
        Items::Scalar(..) => 0,
    }
}

/// # Safety: live pointer.
#[no_mangle]
pub unsafe extern "C" fn grasm_list_sum_f64(v: *mut RtVal) -> f64 {
    if grasm_list_len(v) == 0 {
        // Explicit, so the empty sum is `0.0` and not whatever a zero-length
        // accumulator happened to leave behind.
        return 0.0;
    }
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
        Items::Scalar(ScalarKind::F64, data) => match data.as_words() {
            Some(w) => w.iter().map(|&b| f64::from_bits(b as u64)).sum(),
            None => data.iter().map(|b| f64::from_bits(b as u64)).sum(),
        },
        Items::Scalar(ScalarKind::I64, data) => data.iter().map(|x| x as f64).sum(),
        Items::Scalar(..) => 0.0,
    }
}

#[no_mangle]
pub extern "C" fn grasm_i64_abs(v: i64) -> i64 {
    // `i64::MIN` has no positive counterpart and wraps to itself
    // ([L-LIB-ROUND]). Written as `abs()` that is an overflow: defined only
    // because a release build wraps, and an abort in any build with overflow
    // checks on. Saying `wrapping_abs` makes the documented answer the one
    // the code asks for.
    v.wrapping_abs()
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

// The math library. These are `f64 -> f64` and nothing else: no allocation, no
// effect, no error case. Without them the only way to get a cosine was
// `extern fn cos`, which carries `io` ([T-EXTERN-VALUE]) and so put a
// trigonometry helper over the ceiling of any package declaring
// `capabilities = []`. A pure function should not cost a capability.
#[no_mangle]
pub extern "C" fn grasm_f64_sin(v: f64) -> f64 {
    v.sin()
}
#[no_mangle]
pub extern "C" fn grasm_f64_cos(v: f64) -> f64 {
    v.cos()
}
#[no_mangle]
pub extern "C" fn grasm_f64_tan(v: f64) -> f64 {
    v.tan()
}
#[no_mangle]
pub extern "C" fn grasm_f64_asin(v: f64) -> f64 {
    v.asin()
}
#[no_mangle]
pub extern "C" fn grasm_f64_acos(v: f64) -> f64 {
    v.acos()
}
#[no_mangle]
pub extern "C" fn grasm_f64_atan(v: f64) -> f64 {
    v.atan()
}
#[no_mangle]
pub extern "C" fn grasm_f64_sinh(v: f64) -> f64 {
    v.sinh()
}
#[no_mangle]
pub extern "C" fn grasm_f64_cosh(v: f64) -> f64 {
    v.cosh()
}
#[no_mangle]
pub extern "C" fn grasm_f64_tanh(v: f64) -> f64 {
    v.tanh()
}
#[no_mangle]
pub extern "C" fn grasm_f64_exp(v: f64) -> f64 {
    v.exp()
}
#[no_mangle]
pub extern "C" fn grasm_f64_ln(v: f64) -> f64 {
    v.ln()
}
#[no_mangle]
pub extern "C" fn grasm_f64_log2(v: f64) -> f64 {
    v.log2()
}
#[no_mangle]
pub extern "C" fn grasm_f64_log10(v: f64) -> f64 {
    v.log10()
}
#[no_mangle]
pub extern "C" fn grasm_f64_cbrt(v: f64) -> f64 {
    v.cbrt()
}
#[no_mangle]
pub extern "C" fn grasm_f64_to_radians(v: f64) -> f64 {
    v.to_radians()
}
#[no_mangle]
pub extern "C" fn grasm_f64_to_degrees(v: f64) -> f64 {
    v.to_degrees()
}
#[no_mangle]
pub extern "C" fn grasm_f64_atan2(a: f64, b: f64) -> f64 {
    a.atan2(b)
}
#[no_mangle]
pub extern "C" fn grasm_f64_hypot(a: f64, b: f64) -> f64 {
    a.hypot(b)
}
#[no_mangle]
pub extern "C" fn grasm_f64_log(a: f64, b: f64) -> f64 {
    a.log(b)
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
/// An installed handler: the effect's name, and each operation's lifted code
/// pointer paired with the environment it captured.
///
/// The environment is a raw pointer into the frame that ran `with`, which is
/// sound because that frame cannot return while a handler it installed is
/// still reachable — the block has to exit first, and exiting pops the
/// handler. Without it a handler operation could not read anything from its
/// enclosing scope, which is most of what a handler is for.
type HandlerFrame = (String, Vec<(String, *const u8, *const u8)>);

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
pub unsafe extern "C" fn grasm_handler_op(
    name: *const u8,
    len: u64,
    code: *const u8,
    env: *const u8,
) {
    let n = take_str(name, len);
    HANDLERS.with(|h| {
        if let Some((_, ops)) = h.borrow_mut().last_mut() {
            ops.push((n, code, env));
        }
    });
}

/// The environment of the innermost handler for `effect::op`.
///
/// # Safety: compiler-generated calls with interned name data.
#[no_mangle]
pub unsafe extern "C" fn grasm_handler_env(
    ep: *const u8,
    el: u64,
    op: *const u8,
    ol: u64,
) -> *const u8 {
    let effect = take_str(ep, el);
    let opname = take_str(op, ol);
    HANDLERS
        .with(|h| {
            h.borrow()
                .iter()
                .rev()
                .find(|(e, _)| *e == effect)
                .and_then(|(_, ops)| {
                    ops.iter()
                        .find(|(o, _, _)| *o == opname)
                        .map(|(_, _, env)| *env)
                })
        })
        .unwrap_or(std::ptr::null())
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
                    .find(|(o, _, _)| *o == opname)
                    .map(|(_, c, _)| *c)
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
            .map(|s| alloc(RtPayload::Str(RtStr::new(s.clone()))))
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
    alloc(RtPayload::Str(RtStr::new(line)))
}

// --- filesystem (fallible: returns Ok/Err variants) ---

fn io_err(variant: &'static str, payload: String) -> *mut RtVal {
    let inner = alloc(RtPayload::Str(RtStr::new(payload)));
    alloc(RtPayload::Variant {
        type_name: "IoError",
        name: variant,
        fields: vec![inner],
    })
}

fn err_of(v: *mut RtVal) -> *mut RtVal {
    alloc(RtPayload::Variant {
        type_name: "!",
        name: "Err",
        fields: vec![v],
    })
}

fn ok_of(v: *mut RtVal) -> *mut RtVal {
    alloc(RtPayload::Variant {
        type_name: "!",
        name: "Ok",
        fields: vec![v],
    })
}

// ---------------------------------------------------------------------------
// Network — the `net` capability
// ---------------------------------------------------------------------------
//
// Sockets are handles, and everything crossing the boundary is an `i64` or a
// `str`. That is deliberate: Grasm has no pointer type and no byte buffer, and
// adding one to reach the network would have been a much larger change to the
// language than adding the network itself.
//
// The capability story's headline example is "a package that never declared
// network access can never touch the network" — which could not be shown at all
// while there was no network to gate.

thread_local! {
    /// Open listeners and connections, by handle. A handle is an index, so it
    /// is an `i64` at the boundary and the caller never sees an address.
    static SOCKETS: std::cell::RefCell<Vec<Option<Socket>>> =
        const { std::cell::RefCell::new(Vec::new()) };
}

enum Socket {
    Listener(std::net::TcpListener),
    Stream(std::net::TcpStream),
}

fn socket_put(s: Socket) -> i64 {
    SOCKETS.with(|table| {
        let mut table = table.borrow_mut();
        // A freed slot is reused, so a long-running server does not grow the
        // table once per connection.
        if let Some(i) = table.iter().position(Option::is_none) {
            table[i] = Some(s);
            return i as i64;
        }
        table.push(Some(s));
        (table.len() - 1) as i64
    })
}

/// Runs `f` against the socket a handle names.
///
/// A handle that names nothing is an ordinary failure, not a panic: it is what
/// a program gets for using a connection after closing it, and a wrong answer
/// is worse than an error.
fn socket_with<T>(h: i64, f: impl FnOnce(&mut Socket) -> Result<T, String>) -> Result<T, String> {
    SOCKETS.with(|table| {
        let mut table = table.borrow_mut();
        match table.get_mut(h as usize).and_then(Option::as_mut) {
            Some(s) => f(s),
            None => Err(format!("socket {h} is not open")),
        }
    })
}

/// Listens on a TCP port, answering with a handle.
///
/// # Safety: called from compiled Grasm with a boxed value.
#[no_mangle]
pub unsafe extern "C" fn grasm_tcp_listen(port: *mut RtVal) -> *mut RtVal {
    let port = match &(*port).payload {
        RtPayload::I64(n) => *n,
        _ => return err_of(io_err("IoFailed", "port must be an integer".to_string())),
    };
    if !(0..=65535).contains(&port) {
        return err_of(io_err("IoFailed", format!("port {port} is out of range")));
    }
    match std::net::TcpListener::bind(("127.0.0.1", port as u16)) {
        Ok(l) => ok_of(alloc(RtPayload::I64(socket_put(Socket::Listener(l))))),
        Err(e) => err_of(io_err("IoFailed", e.to_string())),
    }
}

/// Opens a connection to `host:port`, answering with a handle for it.
///
/// The other half of [L-LIB-NET]. Without it a Grasm program could be a server
/// and could not be a client — no fetching a URL, no calling an API, no talking
/// to a database — which is the shape of gap that is invisible until somebody
/// tries to write an ordinary program (ProjectGold 22).
///
/// The host is resolved by the standard library, so a name works as well as an
/// address. Resolution failure and connection failure are the same `IoFailed`
/// to a caller, because both mean "there is nothing to talk to" and the message
/// carries the difference.
///
/// # Safety: called from compiled Grasm with boxed values.
#[no_mangle]
pub unsafe extern "C" fn grasm_tcp_connect(host: *mut RtVal, port: *mut RtVal) -> *mut RtVal {
    let host = match &(*host).payload {
        RtPayload::Str(s) => s.to_string(),
        _ => return err_of(io_err("IoFailed", "host must be text".to_string())),
    };
    let port = match &(*port).payload {
        RtPayload::I64(n) => *n,
        _ => return err_of(io_err("IoFailed", "port must be an integer".to_string())),
    };
    if !(1..=65535).contains(&port) {
        return err_of(io_err("IoFailed", format!("port {port} is out of range")));
    }
    match std::net::TcpStream::connect((host.as_str(), port as u16)) {
        Ok(stream) => ok_of(alloc(RtPayload::I64(socket_put(Socket::Stream(stream))))),
        Err(e) => err_of(io_err("IoFailed", e.to_string())),
    }
}

/// Waits for a connection, answering with a handle for it.
///
/// # Safety: called from compiled Grasm with a boxed value.
#[no_mangle]
pub unsafe extern "C" fn grasm_tcp_accept(handle: *mut RtVal) -> *mut RtVal {
    let h = match &(*handle).payload {
        RtPayload::I64(n) => *n,
        _ => return err_of(io_err("IoFailed", "handle must be an integer".to_string())),
    };
    let accepted = socket_with(h, |s| match s {
        Socket::Listener(l) => l.accept().map(|(stream, _)| stream).map_err(|e| e.to_string()),
        Socket::Stream(_) => Err(format!("socket {h} is a connection, not a listener")),
    });
    match accepted {
        Ok(stream) => ok_of(alloc(RtPayload::I64(socket_put(Socket::Stream(stream))))),
        Err(e) => err_of(io_err("IoFailed", e)),
    }
}

/// Reads what has arrived on a connection, as text.
///
/// One read, not a loop to end-of-stream: an HTTP client keeps the connection
/// open after its request, so reading to the end would wait forever. A caller
/// that needs more calls again.
///
/// # Safety: called from compiled Grasm with a boxed value.
#[no_mangle]
pub unsafe extern "C" fn grasm_tcp_read(handle: *mut RtVal) -> *mut RtVal {
    use std::io::Read;
    let h = match &(*handle).payload {
        RtPayload::I64(n) => *n,
        _ => return err_of(io_err("IoFailed", "handle must be an integer".to_string())),
    };
    let read = socket_with(h, |s| match s {
        Socket::Stream(stream) => {
            let mut buf = vec![0u8; 65536];
            match stream.read(&mut buf) {
                Ok(n) => {
                    buf.truncate(n);
                    // Lossy, because a socket carries bytes and a Grasm `str`
                    // is text: a client that sends invalid UTF-8 gets its bytes
                    // replaced rather than crashing the server.
                    Ok(String::from_utf8_lossy(&buf).into_owned())
                }
                Err(e) => Err(e.to_string()),
            }
        }
        Socket::Listener(_) => Err(format!("socket {h} is a listener, not a connection")),
    });
    match read {
        Ok(text) => ok_of(alloc(RtPayload::Str(RtStr::new(text)))),
        Err(e) => err_of(io_err("IoFailed", e)),
    }
}

/// Writes text to a connection.
///
/// # Safety: called from compiled Grasm with boxed values.
#[no_mangle]
pub unsafe extern "C" fn grasm_tcp_write(handle: *mut RtVal, data: *mut RtVal) -> *mut RtVal {
    use std::io::Write;
    let h = match &(*handle).payload {
        RtPayload::I64(n) => *n,
        _ => return err_of(io_err("IoFailed", "handle must be an integer".to_string())),
    };
    let text = as_str(data).to_string();
    let wrote = socket_with(h, |s| match s {
        Socket::Stream(stream) => stream
            .write_all(text.as_bytes())
            .and_then(|()| stream.flush())
            .map_err(|e| e.to_string()),
        Socket::Listener(_) => Err(format!("socket {h} is a listener, not a connection")),
    });
    match wrote {
        Ok(()) => ok_of(alloc(RtPayload::I64(0))),
        Err(e) => err_of(io_err("IoFailed", e)),
    }
}

/// Closes a listener or a connection. Closing one twice is not an error, for
/// the same reason releasing a `ref` twice is not: cleanup order is not
/// something a caller should have to track.
///
/// # Safety: called from compiled Grasm with a boxed value.
#[no_mangle]
pub unsafe extern "C" fn grasm_tcp_close(handle: *mut RtVal) {
    if let RtPayload::I64(h) = &(*handle).payload {
        let h = *h;
        SOCKETS.with(|table| {
            let mut table = table.borrow_mut();
            if let Some(slot) = table.get_mut(h as usize) {
                *slot = None;
            }
        });
    }
}

#[no_mangle]
pub unsafe extern "C" fn grasm_read_file(path: *mut RtVal) -> *mut RtVal {
    let p = as_str(path).to_string();
    match std::fs::read_to_string(&p) {
        Ok(s) => ok_of(alloc(RtPayload::Str(RtStr::new(s)))),
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
            // As in `grasm_struct_set_field`: an element returning to the
            // slot it was taken from carries no new reference.
            if items[i as usize] != val {
                dec(items[i as usize]);
            }
            items[i as usize] = val;
        }
        RtPayload::ListScalar { kind, data } => {
            if i < 0 || i as usize >= data.len() {
                rt_panic(&format!("index {i} out of bounds (len {})", data.len()));
            }
            data.set(i as usize, scalar_unbox_val(*kind, val));
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
        Ok(v) => some_of(alloc(RtPayload::Str(RtStr::new(v)))),
        Err(_) => none_val(),
    }
}

#[no_mangle]
pub extern "C" fn grasm_exit(code: i64) {
    use std::io::Write as _;
    let _ = std::io::stdout().flush();
    // The low eight bits, which is what an operating system reports and what
    // C's `exit` does with the same number. Clamping to 0..=255 instead made
    // `exit(-1)` — the most common spelling of "fail" in a script — report
    // *success*.
    std::process::exit((code & 0xff) as i32);
}

/// # Safety: live runtime pointer.
#[no_mangle]
pub unsafe extern "C" fn grasm_str_lines(s: *mut RtVal) -> *mut RtVal {
    let items: Vec<*mut RtVal> = as_str(s)
        .lines()
        .map(|l| alloc(RtPayload::Str(RtStr::new(l.to_string()))))
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
    // A width past what a string can hold is a panic with a sentence, not the
    // allocator's own failure message and SIGABRT — as `repeat` already is.
    if width > MAX_ALLOC as i64 {
        rt_panic(&format!(
            "pad width {width} is larger than a string can hold"
        ));
    }
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
    alloc(RtPayload::Str(RtStr::new(out)))
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
            for bits in data.iter() {
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
