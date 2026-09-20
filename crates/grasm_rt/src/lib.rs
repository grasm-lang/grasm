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

/// The record-and-replay journal.
///
/// Re-exported rather than defined here: the interpreter and the browser build
/// share the journal and cannot link this runtime, so the format lives in its
/// own crate. Every `replay::` path in this file is unchanged.
pub use grasm_journal as replay;
use std::sync::atomic::AtomicI64;
use std::sync::atomic::Ordering::{Acquire, Relaxed, Release};

// ---------------------------------------------------------------------------
// Output (print / interpolation are lowered to piecewise `out` calls)
// ---------------------------------------------------------------------------

/// Which stream the `grasm_out_*` family is currently writing to.
///
/// A thread-local flag rather than a second copy of every output function.
/// `print` and `eprint` differ in one bit and in nothing else — same
/// interpolation, same formatting, same flushing — so duplicating six
/// functions to change a destination would be six places for them to drift
/// apart ([L-LIB-EPRINT]).
mod sink {
    use std::cell::Cell;
    thread_local! {
        pub static TO_STDERR: Cell<bool> = const { Cell::new(false) };
    }
    pub fn on() -> bool {
        TO_STDERR.with(|f| f.get())
    }
}

/// Directs the next `grasm_out_*` calls at stderr (`1`) or stdout (`0`).
///
/// The compiler brackets one `eprint` with a pair of these. Nothing else may
/// run between them, so the flag cannot be observed in the wrong state.
#[no_mangle]
pub extern "C" fn grasm_out_to_stderr(on: i8) {
    sink::TO_STDERR.with(|f| f.set(on != 0));
}

/// Writes to whichever stream is currently selected.
fn emit(f: impl Fn(&mut dyn Write)) {
    if sink::on() {
        let mut out = std::io::stderr().lock();
        f(&mut out);
    } else {
        let mut out = std::io::stdout().lock();
        f(&mut out);
    }
}

/// # Safety
/// `ptr` must point to `len` bytes of valid UTF-8 (guaranteed: the compiler
/// only passes string-literal data it embedded itself).
#[no_mangle]
pub unsafe extern "C" fn grasm_out_text(ptr: *const u8, len: u64) {
    let bytes = std::slice::from_raw_parts(ptr, len as usize);
    emit(|out| {
        let _ = out.write_all(bytes);
    });
}

#[no_mangle]
pub extern "C" fn grasm_out_i64(v: i64) {
    emit(|out| {
        let _ = write!(out, "{v}");
    });
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
    emit(|out| {
        let _ = write!(out, "{}", float_text(v));
    });
}

#[no_mangle]
pub extern "C" fn grasm_out_bool(v: i8) {
    emit(|out| {
        let _ = write!(out, "{}", v != 0);
    });
}

#[no_mangle]
pub extern "C" fn grasm_out_nl() {
    emit(|out| {
        let _ = out.write_all(b"\n");
        let _ = out.flush();
    });
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
    /// The number the compiled backend passes. The inverse of [`from_i64`], and
    /// beside it so the two cannot drift.
    pub fn to_i64(self) -> i64 {
        match self {
            ScalarKind::I64 => 0,
            ScalarKind::F64 => 1,
            ScalarKind::Ref => 3,
            ScalarKind::Bool => 2,
        }
    }

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

/// The hash a `map` uses, and why it is not the one Rust picks by default.
///
/// `std`'s `HashMap` hashes with SipHash-1-3, which is a keyed MAC chosen so
/// that an attacker who can choose the keys cannot make the table degenerate.
/// It costs about half a byte per cycle. Zig's standard map uses Wyhash, which
/// is an order of magnitude faster, and the difference is most of a benchmark:
/// counting four hundred thousand short words takes Rust 25 ms, Zig 8.5, and
/// the programs are otherwise the same program.
///
/// This is the same trade, taken the same way Zig takes it, and it keeps the
/// part that matters. The multiply-fold below is wyhash's mixing step — a full
/// 64x64 to 128 multiply whose halves are folded together, so every input bit
/// reaches every output bit — and the table is still keyed: the seed is drawn
/// once per process from the operating system, through the same entropy `std`
/// uses. An attacker who does not know the seed cannot compute colliding keys
/// ahead of time, which is the attack that actually happens.
///
/// What is given up is resistance to an attacker who can *measure* the table's
/// behaviour and search for collisions adaptively against the live seed.
/// SipHash is proved against that; this is not. A program that holds a map
/// keyed by input from a hostile network and cares about that attack should
/// not rely on the default — which is a thing worth writing down rather than
/// leaving implied.
///
/// Nothing here can make a program answer wrongly. A hash chooses a bucket;
/// `Eq` decides what is in it, and `Eq` is untouched. A bad hash is slow, not
/// incorrect.
mod maphash {
    use std::hash::{BuildHasher, Hasher};

    const S0: u64 = 0xa076_1d64_78bd_642f;
    const S1: u64 = 0xe703_7ed1_a0b4_28db;
    const S2: u64 = 0x8ebc_6af0_9c88_c6e3;

    /// Multiply and fold: the whole 128-bit product, halves xored together.
    #[inline(always)]
    fn mix(a: u64, b: u64) -> u64 {
        let p = (a as u128).wrapping_mul(b as u128);
        (p as u64) ^ ((p >> 64) as u64)
    }

    /// One seed for the process, from the same place `std` gets its own.
    fn seed() -> u64 {
        static SEED: std::sync::OnceLock<u64> = std::sync::OnceLock::new();
        *SEED.get_or_init(|| {
            std::collections::hash_map::RandomState::new()
                .build_hasher()
                .finish()
        })
    }

    #[derive(Clone, Copy)]
    pub struct MapHash {
        seed: u64,
    }

    impl Default for MapHash {
        fn default() -> Self {
            MapHash { seed: seed() }
        }
    }

    impl BuildHasher for MapHash {
        type Hasher = MapHasher;
        #[inline]
        fn build_hasher(&self) -> MapHasher {
            MapHasher { h: self.seed }
        }
    }

    pub struct MapHasher {
        h: u64,
    }

    impl Hasher for MapHasher {
        #[inline]
        fn write(&mut self, bytes: &[u8]) {
            // The length goes in first, so that "ab" and "a" + "b" fed as two
            // writes cannot land on the same value.
            let mut h = self.h ^ (bytes.len() as u64).wrapping_mul(S0);
            let (words, rest) = bytes.as_chunks::<8>();
            for c in words {
                h = mix(h ^ u64::from_le_bytes(*c), S1);
            }
            if !rest.is_empty() {
                let mut last = [0u8; 8];
                last[..rest.len()].copy_from_slice(rest);
                h = mix(h ^ u64::from_le_bytes(last), S2);
            }
            self.h = h;
        }

        #[inline]
        fn write_u64(&mut self, n: u64) {
            self.h = mix(self.h ^ n, S1);
        }
        #[inline]
        fn write_u8(&mut self, n: u8) {
            self.write_u64(u64::from(n));
        }
        #[inline]
        fn write_u32(&mut self, n: u32) {
            self.write_u64(u64::from(n));
        }
        #[inline]
        fn write_usize(&mut self, n: usize) {
            self.write_u64(n as u64);
        }
        #[inline]
        fn write_i64(&mut self, n: i64) {
            self.write_u64(n as u64);
        }
        #[inline]
        fn write_isize(&mut self, n: isize) {
            self.write_u64(n as u64);
        }
        #[inline]
        fn finish(&self) -> u64 {
            mix(self.h, S2)
        }
    }
}

/// What a `map` is made of.
pub type MapTable = std::collections::HashMap<MapKey, *mut RtVal, maphash::MapHash>;

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
    /// Short keys are held inline, so `m.set("a", v)` allocates nothing at all.
    /// `MapKey::of` used to call `to_string()` on every key on every `set`,
    /// including one already in the table — four hundred thousand allocations
    /// for eight distinct one-byte keys.
    Str(StrData),
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
        // Short text needs no buffer at all: the inline form *is* the key.
        // Long text still borrows the scratch buffer, so a probe with a long
        // key does not allocate either.
        RtPayload::Str(s) if s.len() <= 15 => f(&MapKey::Str(StrData::of(s))),
        RtPayload::Str(s) => PROBE.with(|cell| {
            let mut buf = cell.borrow_mut();
            buf.clear();
            buf.push_str(s);
            let key = MapKey::Str(StrData::Heap(std::mem::take(&mut buf)));
            let out = f(&key);
            let MapKey::Str(StrData::Heap(returned)) = key else {
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
            RtPayload::Str(s) => MapKey::Str(StrData::of(s)),
            _ => rt_panic(&format!("invalid map key `{}`", display(v))),
        }
    }

    fn boxed(&self) -> *mut RtVal {
        match self {
            MapKey::Bool(b) => interned_bool(*b),
            MapKey::I64(n) => grasm_box_i64(*n),
            MapKey::Str(s) => alloc(RtPayload::Str(RtStr::of(s.as_str()))),
        }
    }
}

pub enum ScalarData {
    Words(Vec<i64>),
    Bytes(Vec<u8>),
}

impl ScalarData {
    /// Room for `n` more elements, taken now rather than in a dozen doublings.
    fn reserve(&mut self, n: usize) {
        match self {
            ScalarData::Words(v) => v.reserve(n),
            ScalarData::Bytes(v) => v.reserve(n),
        }
    }

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
    /// Sorted in place.
    //
    // Unstable, and a plain `sort_unstable` where the words *are* the order.
    // Every comparison here is a total one: two words that compare equal are
    // the same word — `total_cmp` distinguishes -0.0 from 0.0 and orders NaNs
    // by their bits — so no program can tell which of two equal elements came
    // first, and stability costs a scratch buffer of half the list plus a
    // closure call per comparison. For integers there is not even a
    // comparison to make: the machine's own order is the answer.
    fn sort(&mut self, kind: ScalarKind) {
        if let ScalarData::Words(v) = self {
            match kind {
                ScalarKind::I64 | ScalarKind::Ref => v.sort_unstable(),
                ScalarKind::F64 => v.sort_unstable_by(|&a, &b| {
                    f64::from_bits(a as u64).total_cmp(&f64::from_bits(b as u64))
                }),
                // Every bool compares equal to every other, so there is
                // nothing to do — and they are bytes, not words, anyway.
                ScalarKind::Bool => {}
            }
        }
    }

    fn from_words(kind: ScalarKind, words: Vec<i64>) -> ScalarData {
        match kind {
            ScalarKind::Bool => {
                ScalarData::Bytes(words.into_iter().map(|b| u8::from(b != 0)).collect())
            }
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
/// How many bytes a string keeps in its own box before it needs the heap.
pub const INLINE_STR_CAP: usize = 15;

#[derive(Debug)]
pub struct RtStr {
    s: StrData,
    /// `u32::MAX` until asked for.
    ///
    /// Atomic, and the index below is a `OnceLock`, because a value crosses
    /// threads here: `spawn { scan(text) }` hands the same allocation to
    /// several workers, and two of them asking a string its length at the
    /// same moment is an ordinary thing to write. A `Cell` would have been a
    /// data race that happened to print the right answer.
    /// The character index, built only for text that needs one.
    ///
    /// Boxed, and behind one `OnceLock` rather than two fields, because this
    /// is where `RtStr`'s size came from: a `OnceLock<Vec<u32>>` is 32 bytes
    /// and an `AtomicU32` another 8 with padding, so 36 of `RtStr`'s 64 were
    /// an index most strings never build. `RtStr` is the largest `RtPayload`
    /// variant, so those bytes were charged to *every* boxed value in the
    /// runtime — every integer, every bool.
    ///
    /// ASCII text needs no index at all: there the byte offset is the index
    /// and the character count is the byte count, which `char_len` answers
    /// without touching this.
    index: std::sync::OnceLock<Box<CharIndex>>,
}

/// A struct's field values: inline when there are few, on the heap when not.
///
/// Four is the largest run that keeps `RtVal` at 64 bytes, which is the budget
/// every other value in the runtime is already fitted to. Most structs have
/// four fields or fewer, and for those a value is now *one* allocation instead
/// of two — building a tree of two-field nodes was allocating the box, this
/// vector, the child list and its storage.
pub enum StructFields {
    Inline { vals: [*mut RtVal; 4], len: u8 },
    Heap(Vec<*mut RtVal>),
}

impl StructFields {
    fn len(&self) -> usize {
        match self {
            StructFields::Inline { len, .. } => *len as usize,
            StructFields::Heap(v) => v.len(),
        }
    }

    fn as_slice(&self) -> &[*mut RtVal] {
        match self {
            StructFields::Inline { vals, len } => &vals[..*len as usize],
            StructFields::Heap(v) => v,
        }
    }

    fn as_mut_slice(&mut self) -> &mut [*mut RtVal] {
        match self {
            StructFields::Inline { vals, len } => &mut vals[..*len as usize],
            StructFields::Heap(v) => v,
        }
    }

    fn iter(&self) -> std::slice::Iter<'_, *mut RtVal> {
        self.as_slice().iter()
    }

    fn get(&self, i: usize) -> Option<&*mut RtVal> {
        self.as_slice().get(i)
    }

    fn get_mut(&mut self, i: usize) -> Option<&mut *mut RtVal> {
        self.as_mut_slice().get_mut(i)
    }

    /// The fields of a value the backend just built, taken from the stack
    /// slot it stored them into. Four or fewer land in the box itself.
    ///
    /// # Safety: `argv` points at `n` values whose ownership transfers here.
    unsafe fn from_ptrs(argv: *const *mut RtVal, n: usize) -> StructFields {
        if n <= 4 {
            let mut vals = [std::ptr::null_mut(); 4];
            std::ptr::copy_nonoverlapping(argv, vals.as_mut_ptr(), n);
            StructFields::Inline { vals, len: n as u8 }
        } else {
            StructFields::Heap(std::slice::from_raw_parts(argv, n).to_vec())
        }
    }

    fn from_values(values: Vec<*mut RtVal>) -> StructFields {
        if values.len() <= 4 {
            let mut vals = [std::ptr::null_mut(); 4];
            vals[..values.len()].copy_from_slice(&values);
            StructFields::Inline {
                vals,
                len: values.len() as u8,
            }
        } else {
            StructFields::Heap(values)
        }
    }
}

/// A struct type: its name, and its field names in declaration order.
///
/// One of these per type, built once from the descriptor the compiler emitted
/// for it and living as long as the program. Immutable: the field names are
/// known when the object file is written, so nothing here is discovered at run
/// time and nothing has to guard against a second value of the type filling it
/// in differently.
pub struct StructTy {
    pub name: &'static str,
    /// Declaration order. A field is read by index because of this: the
    /// backend sorts every literal into this order before storing anything.
    pub fields: Vec<&'static str>,
}

/// The compiler's description of a struct type, one per type, sitting in the
/// object file's data section.
///
/// The names of a type's fields are known when that object file is written, so
/// they are written there — rather than rediscovered on the first construction
/// by hashing the type name and pushing each field name in turn. Building a
/// value now reads a pointer out of `cache` and is done.
///
/// Laid out by `struct_desc` in the backend, in this order and 8-byte aligned:
///
/// ```text
///  +0  cache      the StructTy built from this descriptor, or 0
///  +8  name_ptr   -> the type's name          (relocation)
/// +16  name_len
/// +24  nfields
/// +32  field 0: ptr (relocation), len
/// +48  field 1: ...
/// ```
#[repr(C)]
pub struct StructDesc {
    cache: std::sync::atomic::AtomicUsize,
    name_ptr: *const u8,
    name_len: u64,
    nfields: u64,
    // `nfields` (ptr, len) pairs follow.
}

/// The `StructTy` a descriptor stands for, built on first use.
///
/// The cache is process-wide rather than per-thread, so a `StructTy` can be
/// read by any thread `spawn` starts — which is why it holds no `RefCell`.
/// Two threads reaching an unbuilt descriptor at once both build one and one
/// wins the exchange; the loser's is leaked, and nothing can tell, because
/// every comparison of two struct values goes by type *name*, never by the
/// address of the type.
///
/// # Safety: `desc` is a descriptor the backend emitted.
unsafe fn desc_ty(desc: *const StructDesc) -> &'static StructTy {
    use std::sync::atomic::Ordering;
    let cached = (*desc).cache.load(Ordering::Acquire);
    if cached != 0 {
        return &*(cached as *const StructTy);
    }
    let n = (*desc).nfields as usize;
    let pairs = (desc as *const u8).add(32) as *const u64;
    let mut fields = Vec::with_capacity(n);
    for i in 0..n {
        let p = *pairs.add(i * 2) as *const u8;
        let l = *pairs.add(i * 2 + 1);
        fields.push(borrowed_str(p, l));
    }
    // Leaked on purpose: one per struct type, and it has to outlive every
    // value that points at it.
    let fresh: &'static StructTy = Box::leak(Box::new(StructTy {
        name: borrowed_str((*desc).name_ptr, (*desc).name_len),
        fields,
    }));
    let addr = fresh as *const StructTy as usize;
    match (*desc)
        .cache
        .compare_exchange(0, addr, Ordering::Release, Ordering::Acquire)
    {
        Ok(_) => fresh,
        Err(won) => &*(won as *const StructTy),
    }
}

/// Where a string's bytes live.
///
/// Short text is held **inline**, in the `RtVal` that already had to be
/// allocated, rather than in a second heap buffer of its own. Building a string
/// otherwise costs two allocations, and measurement said that was nearly the
/// whole cost: converting two million integers to text took 105 ms, of which
/// the digits were 4 ms and the rest was allocating.
///
/// Fifteen bytes because that makes `StrData` exactly the size of the `String`
/// it replaces — 24 bytes — so `RtStr` stays 40 and every boxed value in the
/// runtime stays 64. A larger inline buffer would make every integer and bool
/// pay for it. Fifteen covers what short text actually is: a map key, a word,
/// a number, a field name.
/// Compared, ordered and hashed **by content**, so the inline and heap forms
/// of the same text are the same key. Deriving these instead would make
/// `Inline("a")` and `Heap("a")` different, which is a wrong answer in a map.
impl PartialEq for StrData {
    fn eq(&self, other: &StrData) -> bool {
        // Two inline strings are equal exactly when their fifteen bytes are,
        // because `of` starts from zeroes and writes only the text: whatever
        // follows it is zero in both. That makes the comparison two loads the
        // compiler can inline, rather than a call to `memcmp` — which for a
        // seven-byte map key is nearly all call.
        if let (StrData::Inline { bytes: a, len: la }, StrData::Inline { bytes: b, len: lb }) =
            (self, other)
        {
            return la == lb && a == b;
        }
        self.as_str() == other.as_str()
    }
}
impl Eq for StrData {}
impl std::hash::Hash for StrData {
    fn hash<H: std::hash::Hasher>(&self, state: &mut H) {
        self.as_str().hash(state);
    }
}
impl PartialOrd for StrData {
    fn partial_cmp(&self, other: &StrData) -> Option<std::cmp::Ordering> {
        Some(self.cmp(other))
    }
}
impl Ord for StrData {
    fn cmp(&self, other: &StrData) -> std::cmp::Ordering {
        self.as_str().cmp(other.as_str())
    }
}

/// Public only because `RtPayload` is, and a map key holds one. Nothing
/// outside this crate has any use for it: the interpreter has its own value
/// representation and never links the runtime.
#[derive(Debug, Clone)]
pub enum StrData {
    Inline {
        bytes: [u8; INLINE_STR_CAP],
        len: u8,
    },
    Heap(String),
}

impl StrData {
    fn as_str(&self) -> &str {
        match self {
            // Valid UTF-8 by construction: every constructor either copies
            // from an existing `str` or writes ASCII digits.
            StrData::Inline { bytes, len } => unsafe {
                std::str::from_utf8_unchecked(&bytes[..*len as usize])
            },
            StrData::Heap(s) => s,
        }
    }

    /// Inline when the text fits, heap otherwise.
    fn of(text: &str) -> StrData {
        if text.len() <= INLINE_STR_CAP {
            let mut bytes = [0u8; 15];
            bytes[..text.len()].copy_from_slice(text.as_bytes());
            StrData::Inline {
                bytes,
                len: text.len() as u8,
            }
        } else {
            StrData::Heap(text.to_string())
        }
    }
}

/// Character offsets for text that is not ASCII.
#[derive(Debug)]
struct CharIndex {
    count: u32,
    offsets: Vec<u32>,
}

impl RtStr {
    pub fn new(s: String) -> RtStr {
        // A short string is copied inline and the `String` released: the
        // allocation has already happened here, but every later read avoids a
        // pointer hop. The constructors that matter for speed —
        // `concat` and `of_i64` — never build a `String` at all.
        let data = if s.len() <= 15 {
            StrData::of(&s)
        } else {
            StrData::Heap(s)
        };
        RtStr {
            s: data,
            index: std::sync::OnceLock::new(),
        }
    }

    /// Borrowed text, with no allocation at all when it is short.
    ///
    /// Prefer this over `new(x.to_string())` wherever the `String` exists only
    /// to be handed over: that spelling allocates a heap buffer and then frees
    /// it again one line later.
    pub fn of(text: &str) -> RtStr {
        RtStr {
            s: StrData::of(text),
            index: std::sync::OnceLock::new(),
        }
    }

    /// An integer's decimal text, written straight into the inline buffer.
    ///
    /// Every `i64` whose magnitude is under 10^14 lands inline, which is every
    /// integer a program is likely to interpolate. The rest fall back to
    /// `to_string`. Interpolation is the most common way programs build text,
    /// so this path skips the allocation that spelling would need.
    pub fn of_i64(v: i64) -> RtStr {
        let mut buf = [0u8; 20];
        let mut n = v.unsigned_abs();
        let mut i = buf.len();
        loop {
            i -= 1;
            buf[i] = b'0' + (n % 10) as u8;
            n /= 10;
            if n == 0 {
                break;
            }
        }
        if v < 0 {
            i -= 1;
            buf[i] = b'-';
        }
        // ASCII digits, so this is valid UTF-8.
        RtStr::of(unsafe { std::str::from_utf8_unchecked(&buf[i..]) })
    }

    /// Two pieces joined, with no allocation at all when the result is short.
    /// Appends to this string **in place**.
    ///
    /// Building text by `acc = acc + piece` copies the whole accumulator every
    /// step, so a string built in a loop costs time proportional to the square
    /// of its length. C does not, because C makes you write the buffer
    /// yourself; the point of this is that the obvious Grasm loop should not
    /// be quadratic either.
    ///
    /// Only reached when the caller has established that this value is the
    /// *only* holder of the string, which is what makes mutating it invisible.
    ///
    /// The character index is thrown away: it described the old text.
    fn push_str(&mut self, t: &str) {
        self.index = std::sync::OnceLock::new();
        let spill = match &mut self.s {
            StrData::Inline { bytes, len } => {
                let at = *len as usize;
                if at + t.len() <= 15 {
                    bytes[at..at + t.len()].copy_from_slice(t.as_bytes());
                    *len = (at + t.len()) as u8;
                    return;
                }
                // Doubling rather than exact: this is a loop, and the next
                // call is going to ask for more.
                let mut out = String::with_capacity(((at + t.len()) * 2).max(32));
                out.push_str(std::str::from_utf8(&bytes[..at]).unwrap_or(""));
                out.push_str(t);
                out
            }
            StrData::Heap(h) => {
                h.push_str(t);
                return;
            }
        };
        self.s = StrData::Heap(spill);
    }

    pub fn concat(a: &str, b: &str) -> RtStr {
        let total = a.len() + b.len();
        let data = if total <= 15 {
            let mut bytes = [0u8; 15];
            bytes[..a.len()].copy_from_slice(a.as_bytes());
            bytes[a.len()..total].copy_from_slice(b.as_bytes());
            StrData::Inline {
                bytes,
                len: total as u8,
            }
        } else {
            let mut out = String::with_capacity(total);
            out.push_str(a);
            out.push_str(b);
            StrData::Heap(out)
        };
        RtStr {
            s: data,
            index: std::sync::OnceLock::new(),
        }
    }

    /// The character index, built on first use. Only reached for non-ASCII.
    fn index(&self) -> &CharIndex {
        self.index.get_or_init(|| {
            let offsets: Vec<u32> = self
                .s
                .as_str()
                .char_indices()
                .map(|(o, _)| o as u32)
                .collect();
            Box::new(CharIndex {
                count: offsets.len() as u32,
                offsets,
            })
        })
    }

    /// The byte offset of character `i`, in constant time after the first
    /// call. Shared with `substr`, which walked from the start otherwise.
    pub fn byte_at(&self, i: usize) -> Option<usize> {
        if self.is_flat() {
            return (i <= self.s.as_str().len()).then_some(i);
        }
        let offs = &self.index().offsets;
        match i.cmp(&offs.len()) {
            std::cmp::Ordering::Less => Some(offs[i] as usize),
            std::cmp::Ordering::Equal => Some(self.s.as_str().len()),
            std::cmp::Ordering::Greater => None,
        }
    }

    /// The character at `i`, in constant time after the first call.
    pub fn char_at(&self, i: usize) -> Option<char> {
        if self.is_flat() {
            return self.s.as_str().as_bytes().get(i).map(|b| *b as char);
        }
        let offs = &self.index().offsets;
        let start = *offs.get(i)? as usize;
        self.s.as_str()[start..].chars().next()
    }

    pub fn char_len(&self) -> usize {
        // ASCII is the overwhelmingly common case and needs nothing built:
        // one byte per character, so the count *is* the length. `is_ascii` is
        // a vectorised scan in `std`, far cheaper than counting characters and
        // cheaper than the cache lookup it replaces.
        if self.s.as_str().is_ascii() {
            return self.s.as_str().len();
        }
        self.index().count as usize
    }

    /// Every character one byte, so an index is an index.
    pub fn is_flat(&self) -> bool {
        self.s.as_str().is_ascii()
    }
}

impl PartialEq for RtStr {
    fn eq(&self, other: &RtStr) -> bool {
        self.s.as_str() == other.s.as_str()
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
        self.s.as_str().cmp(other.s.as_str())
    }
}

impl std::ops::Deref for RtStr {
    type Target = str;
    fn deref(&self) -> &str {
        self.s.as_str()
    }
}

impl std::fmt::Display for RtStr {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.s.as_str())
    }
}

/// The elements of a list or a tuple, held **in the box** when there are four
/// or fewer.
///
/// A list was a `Vec`, so building `[a, b, c]` cost the box the value needs
/// anyway plus a second allocation for three pointers. Small lists are most
/// lists: the children of a tree node, the parts of a split, a pair returned
/// from a function, every tuple a pattern takes apart.
///
/// Four for the same reason `StructFields` uses four — it is the largest run
/// that leaves `RtVal` at 64 bytes, which the assertion below holds the file
/// to. Beyond four the elements move to the heap once and stay there.
pub enum ListData {
    Inline { vals: [*mut RtVal; 4], len: u8 },
    Heap(Vec<*mut RtVal>),
}

impl std::ops::Deref for ListData {
    type Target = [*mut RtVal];
    fn deref(&self) -> &[*mut RtVal] {
        match self {
            ListData::Inline { vals, len } => &vals[..*len as usize],
            ListData::Heap(v) => v,
        }
    }
}

impl std::ops::DerefMut for ListData {
    fn deref_mut(&mut self) -> &mut [*mut RtVal] {
        match self {
            ListData::Inline { vals, len } => &mut vals[..*len as usize],
            ListData::Heap(v) => v,
        }
    }
}

impl Default for ListData {
    fn default() -> ListData {
        ListData::new()
    }
}

impl<'a> IntoIterator for &'a ListData {
    type Item = &'a *mut RtVal;
    type IntoIter = std::slice::Iter<'a, *mut RtVal>;
    fn into_iter(self) -> Self::IntoIter {
        self.iter()
    }
}

impl From<Vec<*mut RtVal>> for ListData {
    /// Kept on the heap as it arrives. A `Vec` that already exists has already
    /// paid for its allocation, and copying it into the box to save an
    /// allocation that happened would be work for nothing.
    fn from(v: Vec<*mut RtVal>) -> ListData {
        ListData::Heap(v)
    }
}

impl ListData {
    /// Room for `n` elements. Below the inline threshold this is already true
    /// and does nothing; above it, the heap vector is made at its final size.
    fn reserve(&mut self, n: usize) {
        if n <= 4 {
            return;
        }
        if let ListData::Inline { vals, len } = self {
            let mut heap: Vec<*mut RtVal> = Vec::with_capacity(n);
            heap.extend_from_slice(&vals[..*len as usize]);
            *self = ListData::Heap(heap);
        } else if let ListData::Heap(v) = self {
            v.reserve(n.saturating_sub(v.len()));
        }
    }

    fn new() -> ListData {
        ListData::Inline {
            vals: [std::ptr::null_mut(); 4],
            len: 0,
        }
    }

    /// Spills to the heap on the fifth element, once.
    fn push(&mut self, v: *mut RtVal) {
        match self {
            ListData::Inline { vals, len } if (*len as usize) < 4 => {
                vals[*len as usize] = v;
                *len += 1;
            }
            ListData::Inline { vals, len } => {
                let mut heap: Vec<*mut RtVal> = Vec::with_capacity(8);
                heap.extend_from_slice(&vals[..*len as usize]);
                heap.push(v);
                *self = ListData::Heap(heap);
            }
            ListData::Heap(h) => h.push(v),
        }
    }

    /// Two elements, in the box. Every tuple this runtime builds internally
    /// is a pair, and `vec![a, b]` allocated for it.
    fn pair(a: *mut RtVal, b: *mut RtVal) -> ListData {
        ListData::Inline {
            vals: [a, b, std::ptr::null_mut(), std::ptr::null_mut()],
            len: 2,
        }
    }

    /// Three or four elements, in the box, for the same reason as `pair`.
    fn small(vals: &[*mut RtVal]) -> ListData {
        let mut out = [std::ptr::null_mut(); 4];
        out[..vals.len()].copy_from_slice(vals);
        ListData::Inline {
            vals: out,
            len: vals.len() as u8,
        }
    }

    fn pop(&mut self) -> Option<*mut RtVal> {
        match self {
            ListData::Inline { vals, len } => {
                if *len == 0 {
                    return None;
                }
                *len -= 1;
                Some(vals[*len as usize])
            }
            ListData::Heap(h) => h.pop(),
        }
    }

    fn remove(&mut self, i: usize) -> *mut RtVal {
        match self {
            ListData::Inline { vals, len } => {
                let n = *len as usize;
                assert!(i < n, "index out of range");
                let out = vals[i];
                vals.copy_within(i + 1..n, i);
                *len -= 1;
                out
            }
            ListData::Heap(h) => h.remove(i),
        }
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
    /// A handler operation's captured environment, on the heap.
    ///
    /// The lifted operation reads its captures as machine words at `data + i*8`
    /// — the same layout a stack slot gave it — so nothing about the operation
    /// changes when the environment outlives the frame that built it. `boxed`
    /// says which of those words are references, and is what lets releasing the
    /// environment release them.
    Env {
        words: Vec<i64>,
        boxed: Vec<bool>,
    },
    /// An effect handler as a value: its effect, and each operation's code and
    /// environment.
    ///
    /// Held in the order the literal wrote its fields, because that is the
    /// order the interpreter prints a handler in and the two must agree.
    Handler {
        effect: String,
        ops: Vec<(String, *const u8, *mut RtVal)>,
    },
    /// `a..b` or `a..=b`, held rather than walked.
    ///
    /// Three scalars, not the elements: `let r = 0..2_000_000` is two words
    /// and a flag, and the interpreter's `Value::Range` is the same three
    /// fields — so a range that crosses between engines is the same value on
    /// both, and prints as `0..2000000` rather than as two million integers.
    Range {
        start: i64,
        end: i64,
        inclusive: bool,
    },
    /// A contiguous run of bytes ([L-LIB-BYTES]). `Vec<u8>`, not a list of
    /// boxed integers: the promise the type makes is that every element is a
    /// byte, and the representation should carry that promise.
    ///
    /// `Arc` rather than `Rc`, for the same reason the count above is atomic:
    /// this was the one payload in the runtime that could not cross a thread,
    /// and a `bytes` value is exactly the kind of thing a parallel worker
    /// would be handed. The count is only touched when a `bytes` value is
    /// copied, which is rare enough that nothing measurable rides on it.
    Bytes(std::sync::Arc<Vec<u8>>),
    List(ListData),
    /// Same shape as a list, rendered with parentheses.
    ///
    /// Tuples are built as lists and sealed, because every operation on them
    /// — refcounting, equality, reading an element out in a pattern — is a
    /// list operation. Only `display` differs, and it differs in a way a
    /// program's output is judged on, so it cannot be left to look the same.
    Tuple(ListData),
    /// Scalar elements stored inline — no per-element heap cell, no refcount
    /// traffic, no pointer chase. This is why `c[i] = c[i] + x` in a hot loop
    /// is a load and a store instead of an allocation.
    ListScalar {
        kind: ScalarKind,
        data: ScalarData,
    },
    /// Hash table for lookup; iteration sorts, which is where the ordering
    /// guarantee is paid for rather than on every get and set.
    Map(MapTable),
    /// A struct value carries its field *pointers* and nothing else. The names
    /// are identical for every value of a type, so they live once in a
    /// `StructTy` the value points at rather than being repeated in each one.
    ///
    /// They were `String`s first — a heap allocation per name on every value.
    /// Making them `&'static str` removed the allocations and left the width:
    /// three words per field, where a field is one. Building a tree of
    /// two-field nodes is mostly this vector.
    Struct {
        ty: &'static StructTy,
        fields: StructFields,
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
    /// Atomic, so that a value may be held by more than one thread.
    ///
    /// The obvious objection is cost, and the obvious objection was measured:
    /// against the same programs with a plain `Cell<i64>`, `tree` — the most
    /// reference-count-heavy thing here — is 4.3% slower, and `wordfreq`,
    /// `sort` and `sieve` do not move at all. About half a percent on the
    /// geometric mean, on a machine whose uncontended atomics are a single
    /// instruction.
    ///
    /// That was worth knowing on its own. "Parallelism needs atomic
    /// refcounting and atomic refcounting is expensive" is the sentence that
    /// kept `par_map` restricted to machine words, and it was an assumption
    /// nobody here had priced.
    pub rc: AtomicI64,
    pub payload: RtPayload,
}

/// Sixty-four bytes, and every inline-storage decision in this file is sized
/// against that: fifteen bytes of text in `StrData`, four fields in
/// `StructFields`. Those numbers were chosen by measuring the size of this
/// struct and stopping where it stopped growing, and until now that reasoning
/// lived only in prose — a fifth inline field or a sixteenth inline byte would
/// have widened every boxed integer in the program and nothing would have said
/// so. Now the build says so.
const _: () = assert!(std::mem::size_of::<RtVal>() == 64);

/// Boxes that have been freed and are waiting to be used again.
///
/// Every `RtVal` is the same 64 bytes, so a freed one is exactly what the next
/// allocation wants and handing it back is a pointer pop. Measurement said
/// this was worth doing: a string concatenation is an allocation, a memcpy and
/// a free, and the memcpy is not what it costs — `s + t` in a loop took 45 ns
/// an iteration for six bytes of text, which is the allocator's round trip and
/// almost nothing else.
///
/// The chain runs through the `rc` field of a freed box, whose payload has
/// already been dropped and whose memory is otherwise uninitialised.
struct Pool {
    head: Cell<*mut RtVal>,
    n: Cell<usize>,
}

/// Capped so a program that frees a million values at once does not hold the
/// memory for all of them. Past the cap a free goes back to the allocator.
const POOL_CAP: usize = 65536;

/// How many children a release keeps in its stack frame before spilling.
///
/// Four, which is what `StructFields::Inline` and `ListData::Inline` already
/// hold: the shapes that recur are a small struct's fields and a short list's
/// elements. Wider was measurably worse — this frame is built on *every*
/// release, including the ones that push nothing.
const PENDING_INLINE: usize = 4;

impl Drop for Pool {
    /// The thread is ending, so its cached boxes go back to the allocator
    /// rather than leaking. They may have been allocated on another thread —
    /// a value built by `spawn` and released by the parent lands here — which
    /// the global allocator allows.
    fn drop(&mut self) {
        let mut cur = self.head.get();
        while !cur.is_null() {
            let next = unsafe { (*cur).rc.load(Relaxed) as usize as *mut RtVal };
            drop(unsafe { Box::from_raw(cur as *mut std::mem::MaybeUninit<RtVal>) });
            cur = next;
        }
    }
}

/// The per-thread state the allocation path touches.
///
/// macOS resolves a thread-local through `_tlv_get_addr`, which is a call,
/// and a profile of `benchmarks/grasm/tree.gr` — the program whose whole shape
/// is allocate, walk, free — put that call at the top of the list with about a
/// third of the samples. `alloc` reached three separate thread-locals and the
/// release path two, so every value cost three resolutions to make and two to
/// release. They are one structure here, resolved once.
struct Local {
    /// Boxes waiting to be used again.
    pool: Pool,
    /// Allocations this thread has made and not yet freed.
    ///
    /// A leak is invisible to the differential harness: it compares output,
    /// and a program that leaks produces exactly the right output. This is
    /// what makes the difference observable, and it is the prerequisite for
    /// Tier 1 — a `region` works by *not* decrementing inside the block and
    /// sweeping at the brace, so a miscount there is a double-free or a leak
    /// and only one of those announces itself.
    live: Cell<i64>,
    /// One list of allocations per open region, innermost last.
    regions: std::cell::RefCell<Vec<Vec<*mut RtVal>>>,
    /// How many of those are open. The hot path asks this rather than
    /// borrowing the list, because borrowing a `RefCell` to ask whether it is
    /// empty is a flag write on every allocation.
    region_depth: Cell<usize>,
    /// The address this thread's stack may not go past.
    stack_floor: Cell<usize>,
    /// Buffers for building interpolated strings, kept for their capacity.
    //
    // Boxed on purpose, whatever clippy thinks: `grasm_str_build` hands the
    // generated code the address of one of these as its handle, so the
    // `String` has to stay where it is while the vector behind it grows.
    #[allow(clippy::vec_box)]
    builders: std::cell::RefCell<Vec<Box<String>>>,
    /// Every allocation this thread has made, whether or not it is still
    /// live. Reported by `GRASM_ALLOC_STATS`, which is how a question like
    /// "how many boxes does one iteration of this loop cost" gets an answer
    /// rather than an estimate off a profile.
    allocated: Cell<u64>,
}

thread_local! {
    static LOCAL: Local = const {
        Local {
            pool: Pool {
                head: Cell::new(std::ptr::null_mut()),
                n: Cell::new(0),
            },
            live: Cell::new(0),
            regions: std::cell::RefCell::new(Vec::new()),
            region_depth: Cell::new(0),
            stack_floor: Cell::new(0),
            allocated: Cell::new(0),
            builders: std::cell::RefCell::new(Vec::new()),
        }
    };
}

/// What a freed box's refcount is set to under the `poison` feature.
///
/// Chosen so it cannot be mistaken for a count, for `IMMORTAL`, for `ARENA`,
/// or for a pool link — the free list stores the next box's address there, and
/// no address is negative.
#[cfg(feature = "poison")]
const POISON: i64 = i64::MIN + 0x5EAD;

/// Refuses to go on if `v` has already been released.
///
/// A premature free is not a leak, so `leakcheck` cannot see one: it counts
/// what is still live at exit, and a value freed too early is *less* live, not
/// more. One went undetected through all 497 programs. Under `poison` a freed
/// box is never handed out again and its count is set to something no live
/// value can hold, so the next thing to touch it says so instead of reading
/// whatever moved in.
#[cfg(feature = "poison")]
#[inline]
unsafe fn assert_alive(v: *mut RtVal, what: &str) {
    if !v.is_null() && (*v).rc.load(Relaxed) == POISON {
        rt_panic(&format!("use after free: {what} on a value already released"));
    }
}

#[cfg(not(feature = "poison"))]
#[inline(always)]
unsafe fn assert_alive(_v: *mut RtVal, _what: &str) {}

/// Sixty-four bytes to put an `RtVal` in, recycled if one is going spare.
///
/// Takes the thread's state rather than resolving it: on macOS that
/// resolution is a call, and the allocation path already has it.
fn take_box_in(local: &Local) -> *mut RtVal {
    let pool = &local.pool;
    {
        let head = pool.head.get();
        if head.is_null() {
            let fresh: Box<std::mem::MaybeUninit<RtVal>> =
                Box::new(std::mem::MaybeUninit::uninit());
            Box::into_raw(fresh) as *mut RtVal
        } else {
            // Safety: a box on the chain has a dropped payload and holds its
            // successor in `rc`.
            let next = unsafe { (*head).rc.load(Relaxed) as usize as *mut RtVal };
            pool.head.set(next);
            pool.n.set(pool.n.get() - 1);
            head
        }
    }
}

/// Drops what a box holds and keeps the box.
///
/// # Safety: `p` was allocated here, its refcount has reached zero, and
/// nothing points at it any more.
/// # Safety: `p` was allocated here, its refcount has reached zero, and
/// nothing points at it any more.
unsafe fn give_box_in(local: &Local, p: *mut RtVal) {
    std::ptr::drop_in_place(&mut (*p).payload);
    #[cfg(feature = "poison")]
    {
        // Kept out of circulation on purpose: a box never handed out again is
        // one whose later use is still there to be caught.
        //
        // The payload is overwritten as well, with `Unit` — a real variant,
        // not a pattern, because an invalid discriminant would be undefined
        // rather than detectable. Anything that later reads this box as the
        // list or the string it used to be finds a unit instead and says so,
        // which is how a stale *read* is caught; the count below is how a
        // stale `inc` or `dec` is.
        let _ = local;
        std::ptr::write(&mut (*p).payload, RtPayload::Unit);
        (*p).rc.store(POISON, Relaxed);
        return;
    }
    #[allow(unreachable_code)]
    {
        let pool = &local.pool;
        if pool.n.get() >= POOL_CAP {
            drop(Box::from_raw(p as *mut std::mem::MaybeUninit<RtVal>));
            return;
        }
        (*p).rc.store(pool.head.get() as usize as i64, Relaxed);
        pool.head.set(p);
        pool.n.set(pool.n.get() + 1);
    }
}

fn alloc(payload: RtPayload) -> *mut RtVal {
    LOCAL.with(|local| alloc_in(local, payload))
}

/// Allocation with the thread's state already in hand.
///
/// A caller that is going to reach `LOCAL` anyway should reach it once and
/// call this: on macOS every `thread_local!` access is a call through
/// `_tlv_get_addr`, which a profile of `wordfreq` put at fifteen percent.
fn alloc_in(local: &Local, payload: RtPayload) -> *mut RtVal {
    local.live.set(local.live.get() + 1);
    local.allocated.set(local.allocated.get() + 1);
    // Inside a region the value is arena-owned: no reference counting, and
    // the whole set is freed at the closing brace ([T-REGION]).
    let in_region = local.region_depth.get() > 0;
    let p = take_box_in(local);
    // Safety: `take_box_in` returns storage for exactly one `RtVal`,
    // uninitialised.
    unsafe {
        std::ptr::write(
            p,
            RtVal {
                rc: AtomicI64::new(if in_region { ARENA } else { 1 }),
                payload,
            },
        );
    }
    if in_region {
        if let Some(top) = local.regions.borrow_mut().last_mut() {
            top.push(p);
        }
    }
    p
}

/// What worker threads allocated and did not free themselves.
///
/// `LIVE` is per-thread so that allocating stays a load and a store rather than
/// an atomic. That is right until a value crosses a thread — a `par_map` worker
/// builds a string and the main thread frees it — at which point one counter
/// goes up and the other goes down. A worker folds its balance in here when it
/// finishes, once, and the exit check adds the two.
static ESCAPED: AtomicI64 = AtomicI64::new(0);

/// What worker threads allocated, for the same report.
static ESCAPED_ALLOCS: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

/// Called by a worker as it ends: its balance becomes everyone's.
fn hand_back_live() {
    ESCAPED_ALLOCS.fetch_add(LOCAL.with(|l| l.allocated.replace(0)), Relaxed);
    let n = LOCAL.with(|l| l.live.replace(0));
    if n != 0 {
        ESCAPED.fetch_add(n, Relaxed);
    }
}

/// Prints a reference as the slot it names.
///
/// Not the value it points at: the value may already be gone, and reading it
/// to render a message would be the exact use-after-free the tier prevents.
#[no_mangle]
pub extern "C" fn grasm_out_ref(r: i64) {
    let (i, gen) = unpack_ref(r);
    emit(|out| {
        let _ = write!(out, "ref#{i}.{gen}");
    });
}

/// NUL-terminated copies handed to C and not yet freed.
///
/// Global rather than per-thread: a copy is made and freed around one call, so
/// there is no hot path to protect, and a `parallel` body calling an `extern
/// fn` would otherwise balance on the wrong thread.
static C_COPIES: AtomicI64 = AtomicI64::new(0);

/// Allocations still live on this thread.
///
/// Zero at the end of `main` means every allocation was released. It is not
/// zero *during* a program, and it counts only this thread — a worker's
/// allocations are its own.
#[no_mangle]
pub extern "C" fn grasm_live_allocations() -> i64 {
    LOCAL.with(|l| l.live.get())
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
    if std::env::var_os("GRASM_ALLOC_STATS").is_some() {
        let made = LOCAL.with(|l| l.allocated.get()) + ESCAPED_ALLOCS.load(Relaxed);
        eprintln!("allocations: {made}");
    }
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
    // Allocations a worker thread made and handed back live in `ESCAPED`: the
    // counter is per-thread so that the hot path stays a plain load and store,
    // and a thread folds its balance in when it ends. A value built on a
    // worker and freed here would otherwise take this count negative and
    // report a leak of minus four.
    let live = LOCAL.with(|l| l.live.get()) + ESCAPED.load(Relaxed);
    let copies = C_COPIES.load(Relaxed);
    if live == 0 && copies == 0 {
        return;
    }
    let _ = std::io::stdout().flush();
    let mut err = std::io::stderr().lock();
    if live != 0 {
        let _ = writeln!(err, "leak: {live} allocation(s) still live at exit");
    }
    if copies != 0 {
        let _ = writeln!(
            err,
            "leak: {copies} string copy/copies handed to C and never freed"
        );
    }
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

/// Opens a region: allocations until the matching pop belong to it.
#[no_mangle]
pub extern "C" fn grasm_region_push() {
    LOCAL.with(|local| {
        local.regions.borrow_mut().push(Vec::new());
        local.region_depth.set(local.region_depth.get() + 1);
    });
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
    let popped = LOCAL.with(|local| {
        let out = local.regions.borrow_mut().pop();
        local
            .region_depth
            .set(local.region_depth.get().saturating_sub(1));
        out
    });
    let Some(vals) = popped else {
        return;
    };
    for p in vals {
        // Release what this value points at that the region does not own.
        let mut outside: Vec<*mut RtVal> = Vec::new();
        match &(*p).payload {
            RtPayload::List(items) | RtPayload::Tuple(items) => {
                outside.extend(
                    items
                        .iter()
                        .copied()
                        .filter(|c| (**c).rc.load(Relaxed) != ARENA),
                );
            }
            RtPayload::Map(entries) => {
                outside.extend(
                    entries
                        .values()
                        .copied()
                        .filter(|c| (**c).rc.load(Relaxed) != ARENA),
                );
            }
            RtPayload::Struct { fields, .. } => {
                outside.extend(
                    fields
                        .iter()
                        .copied()
                        .filter(|c| (**c).rc.load(Relaxed) != ARENA),
                );
            }
            RtPayload::Variant { fields, .. } => {
                outside.extend(
                    fields
                        .iter()
                        .copied()
                        .filter(|c| (**c).rc.load(Relaxed) != ARENA),
                );
            }
            RtPayload::Handler { ops, .. } => {
                outside.extend(
                    ops.iter()
                        .map(|(_, _, env)| *env)
                        .filter(|e| !e.is_null() && (**e).rc.load(Relaxed) != ARENA),
                );
            }
            RtPayload::Env { words, boxed } => {
                for (w, is_boxed) in words.iter().zip(boxed.iter()) {
                    if *is_boxed && *w != 0 {
                        let c = *w as *mut RtVal;
                        if (*c).rc.load(Relaxed) != ARENA {
                            outside.push(c);
                        }
                    }
                }
            }
            _ => {}
        }
        for c in outside {
            dec(c);
        }
        LOCAL.with(|local| {
            local.live.set(local.live.get() - 1);
            give_box_in(local, p);
        });
    }
}

/// Small integers worth interning. Covers loop counters, indices, flags, and
/// the small constants that dominate real programs.
const SMALL_MIN: i64 = -128;
const SMALL_MAX: i64 = 1024;

/// Where the interned boxes live: one block, for the whole process.
///
/// This was a thread-local vector of pointers, which cost two memory touches
/// and — on macOS, where a thread-local is reached through a call to
/// `_tlv_get_addr` — a call, every time a small integer was boxed. That is one
/// of the hottest things the language does: a loop counter, an index, a length
/// and a count are all small integers. `_tlv_get_addr` was fifteen percent of
/// `wordfreq`'s profile, and most of it arrived here.
///
/// Nothing about the table needed to be per-thread. Every box in it is
/// `IMMORTAL`, so `inc` and `dec` skip it, `cow_*` copies rather than writes
/// it, and no thread can ever change what another reads. Sharing one block
/// across the process is sound for exactly that reason, and it reduces the
/// lookup to an add and a multiply: the boxes are contiguous, so the address
/// of the box for `n` can be computed rather than read out of a table.
fn interned_base() -> usize {
    static BASE: std::sync::OnceLock<usize> = std::sync::OnceLock::new();
    *BASE.get_or_init(|| {
        let mut table: Vec<RtVal> = Vec::with_capacity((SMALL_MAX - SMALL_MIN + 3) as usize);
        for v in [false, true] {
            table.push(RtVal {
                rc: AtomicI64::new(IMMORTAL),
                payload: RtPayload::Bool(v),
            });
        }
        for n in SMALL_MIN..=SMALL_MAX {
            table.push(RtVal {
                rc: AtomicI64::new(IMMORTAL),
                payload: RtPayload::I64(n),
            });
        }
        // Leaked on purpose, and not through `alloc`: these are never freed,
        // so counting them would report a leak at exit.
        Box::leak(table.into_boxed_slice()).as_mut_ptr() as usize
    })
}

fn interned_at(index: usize) -> *mut RtVal {
    (interned_base() + index * std::mem::size_of::<RtVal>()) as *mut RtVal
}

fn interned_bool(v: bool) -> *mut RtVal {
    interned_at(usize::from(v))
}

/// The shared box for `n`, or null when `n` is outside the interned range.
fn interned_int(n: i64) -> *mut RtVal {
    if !(SMALL_MIN..=SMALL_MAX).contains(&n) {
        return std::ptr::null_mut();
    }
    interned_at((n - SMALL_MIN) as usize + 2)
}

/// # Safety
/// `v` must be a live pointer produced by this runtime.
unsafe fn inc(v: *mut RtVal) {
    assert_alive(v, "inc");
    if !v.is_null() {
        let rc = &(*v).rc;
        // Relaxed: taking a reference to a value only requires that the count
        // end up right, not that anything else be ordered by it. This is the
        // same reasoning `Arc::clone` uses.
        if is_counted(rc.load(Relaxed)) {
            rc.fetch_add(1, Relaxed);
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
    // The thread's state is resolved once for the whole traversal rather than
    // once per value freed.
    //
    // On macOS a `thread_local!` access is a call into `_tlv_get_addr`, and a
    // profile of the tree benchmark put that call at the top by six times —
    // about a fifth of the program. Dropping a tree of ten thousand nodes made
    // ten thousand of those calls, one per node, for a pointer that cannot
    // change: the traversal never leaves the thread it started on.
    //
    // But most releases free nothing at all. A borrow taken for the length of
    // a call, an element a loop bound and dropped, a value somebody else still
    // holds — each of those is one atomic decrement and nothing more, and it
    // needs no pool and no thread state. Reaching for `LOCAL` before finding
    // that out cost a call through `_tlv_get_addr` on every one of them, which
    // was twenty-eight percent of a loop over a list of strings.
    if v.is_null() {
        return;
    }
    assert_alive(v, "dec");
    let rc = &(*v).rc;
    if !is_counted(rc.load(Relaxed)) {
        return;
    }
    // Release on the way down, Acquire before freeing: the `Arc` discipline,
    // for the same reason it has it.
    if rc.fetch_sub(1, Release) - 1 > 0 {
        return;
    }
    std::sync::atomic::fence(Acquire);
    // It has reached zero, so the box is going back to the pool and whatever
    // it held has to be followed. *Now* the thread's state is worth fetching.
    dec_reached_zero(v)
}

/// The rest of a release, once the count has reached zero.
///
/// Its own function, and marked cold, so that the common release — decrement
/// and return — can be a leaf: reaching `LOCAL` needs the link register saved,
/// and with this inline `grasm_val_dec` built a forty-eight byte frame and
/// saved four registers before finding out it had nothing to do.
///
/// # Safety: `v`'s count has reached zero and the acquire fence has been taken.
#[cold]
#[inline(never)]
unsafe fn dec_reached_zero(v: *mut RtVal) {
    LOCAL.with(|local| dec_walk(local, v, true))
}

/// The walk itself, with the thread's state already in hand.
///
/// # Safety: as `dec`.
/// The walk, told whether the first value has already reached zero.
///
/// `dec` does that decrement itself, so that the common release — the one that
/// frees nothing — never reaches the thread's state at all.
///
/// # Safety: as `dec`. When `first_is_zero`, `v`'s count has already reached
/// zero and the acquire fence has already been taken.
unsafe fn dec_walk(local: &Local, v: *mut RtVal, first_is_zero: bool) {
    // The children still to release. Almost always a handful — a struct has
    // two or three fields, a list a few elements — so they go in the frame,
    // and only a genuinely large value reaches the heap.
    //
    // It was a `Vec` for every call. `Vec::new()` does not allocate, but the
    // first push does, so releasing *any* value that had children cost a
    // `malloc` and a `free`: in a loop that makes and drops a two-field struct
    // they were three quarters of the profile, and the whole make-and-drop
    // cost forty-nine nanoseconds.
    // Uninitialised, because most releases push nothing at all: a value whose
    // refcount has not reached zero, or one with no children, would otherwise
    // pay to zero the whole array. Only the slots below `n` are ever read, and
    // each was written before `n` passed it.
    let mut inline: [std::mem::MaybeUninit<*mut RtVal>; PENDING_INLINE] =
        [const { std::mem::MaybeUninit::uninit() }; PENDING_INLINE];
    let mut n: usize = 0;
    let mut spill: Vec<*mut RtVal> = Vec::new();
    let mut cur = v;
    let mut known_zero = first_is_zero;
    loop {
        if !cur.is_null() {
            let rc = &(*cur).rc;
            // Release on the way down, Acquire before freeing: everything
            // every other holder wrote through this value has to be visible to
            // the thread that takes it apart. Again the `Arc` discipline, and
            // for the same reason.
            let hit_zero = if known_zero {
                true
            } else if is_counted(rc.load(Relaxed)) {
                let left = rc.fetch_sub(1, Release) - 1;
                if left <= 0 {
                    std::sync::atomic::fence(Acquire);
                    true
                } else {
                    false
                }
            } else {
                false
            };
            known_zero = false;
            if hit_zero {
                // The children are read out *before* the payload is
                // dropped; the box itself goes back to the pool.
                match &(*cur).payload {
                    RtPayload::List(items) | RtPayload::Tuple(items) => {
                        for c in items.iter() {
                            push_pending(&mut inline, &mut n, &mut spill, *c);
                        }
                    }
                    RtPayload::Map(entries) => {
                        for c in entries.values() {
                            push_pending(&mut inline, &mut n, &mut spill, *c);
                        }
                    }
                    RtPayload::Struct { fields, .. } => {
                        for c in fields.iter() {
                            push_pending(&mut inline, &mut n, &mut spill, *c);
                        }
                    }
                    RtPayload::Fn { env, .. } => {
                        if !env.is_null() {
                            push_pending(&mut inline, &mut n, &mut spill, *env);
                        }
                    }
                    RtPayload::Variant { fields, .. } => {
                        for c in fields.iter() {
                            push_pending(&mut inline, &mut n, &mut spill, *c);
                        }
                    }
                    // A handler owns one environment per operation, and an
                    // environment owns whichever of its words are
                    // references. Without these two arms a handler that
                    // escaped its frame would leak everything it captured —
                    // which is the whole reason it is allowed to escape.
                    RtPayload::Handler { ops, .. } => {
                        for env in ops.iter().map(|(_, _, e)| *e).filter(|e| !e.is_null()) {
                            push_pending(&mut inline, &mut n, &mut spill, env);
                        }
                    }
                    RtPayload::Env { words, boxed } => {
                        for (w, is_boxed) in words.iter().zip(boxed.iter()) {
                            if *is_boxed && *w != 0 {
                                push_pending(&mut inline, &mut n, &mut spill, *w as *mut RtVal);
                            }
                        }
                    }
                    _ => {}
                }
                local.live.set(local.live.get() - 1);
                give_box_in(local, cur);
            }
        }
        // The frame first, then whatever spilled: the order releases are done
        // in is not observable, only that each happens once.
        match spill.pop().or_else(|| {
            (n > 0).then(|| {
                n -= 1;
                // Safety: written by `push_pending` before `n` passed it.
                unsafe { inline[n].assume_init() }
            })
        }) {
            Some(next) => cur = next,
            None => return,
        }
    }
}

/// One more child to release, in the frame while there is room.
#[inline(always)]
fn push_pending(
    inline: &mut [std::mem::MaybeUninit<*mut RtVal>; PENDING_INLINE],
    n: &mut usize,
    spill: &mut Vec<*mut RtVal>,
    p: *mut RtVal,
) {
    // Once a value has been big enough to spill, everything goes to the heap
    // buffer: a deep walk — freeing a whole tree in one call — would otherwise
    // pay to check and refill the frame on every step for no benefit.
    if *n < PENDING_INLINE && spill.is_empty() {
        inline[*n].write(p);
        *n += 1;
    } else {
        if *n > 0 {
            for slot in inline.iter().take(*n) {
                // Safety: written before `n` passed it.
                spill.push(unsafe { slot.assume_init() });
            }
            *n = 0;
        }
        spill.push(p);
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

/// A string literal's value, made **once** and shared forever.
///
/// `"hello".len()` in a loop allocated a fresh box every time round: two
/// million of them cost 24 ns each against nothing at all for the same text
/// bound to a `let`. A literal cannot change, so there is no reason for a
/// second one to exist.
///
/// `IMMORTAL`, so `inc` and `dec` skip it and nothing ever frees it — the
/// caller does not have to know it is shared. Built with `Box::into_raw`
/// rather than through `alloc`, for the same reason the small-integer table is:
/// `alloc` counts a live value, and a value that is never freed would be
/// reported as a leak at exit.
///
/// The *caching* is the backend's, one slot per distinct text, because a load
/// and a branch beat a hash lookup — this call happens only the first time each
/// literal is reached.
///
/// # Safety: compiler-generated calls with interned name data.
#[no_mangle]
pub unsafe extern "C" fn grasm_str_lit_immortal(ptr: *const u8, len: u64) -> *mut RtVal {
    let bytes = std::slice::from_raw_parts(ptr, len as usize);
    Box::into_raw(Box::new(RtVal {
        rc: AtomicI64::new(IMMORTAL),
        payload: RtPayload::Str(RtStr::of(&String::from_utf8_lossy(bytes))),
    }))
}

#[no_mangle]
pub unsafe extern "C" fn grasm_str_lit(ptr: *const u8, len: u64) -> *mut RtVal {
    let bytes = std::slice::from_raw_parts(ptr, len as usize);
    alloc(RtPayload::Str(RtStr::of(&String::from_utf8_lossy(bytes))))
}

/// A place to build a string in, taken from this thread and given back.
///
/// Every piece of `"a{b}c"` used to become a value of its own: the number was
/// formatted into a fresh box and then copied into the accumulator, which is
/// two allocations and a copy for something the program asked to write once.
/// `wordfreq` allocates exactly two values per iteration for
/// `words[i % 8] + "{i % 97}"`, and one of them is that intermediate.
///
/// Buffers are stacked rather than single, because a piece can itself
/// interpolate — `"{ "{inner}" }"` is legal — and they are kept for their
/// capacity, so a loop that builds a million strings allocates the buffer
/// once.
///
/// # Safety: the handle is one this thread was given by `grasm_str_build` and
/// has not yet ended.
#[no_mangle]
pub extern "C" fn grasm_str_build() -> i64 {
    LOCAL.with(|local| {
        let mut held = local.builders.borrow_mut();
        // The box is recycled, not made: `Box::new` here would be a fresh
        // allocation for every string built, which is the allocation this was
        // written to remove.
        let mut buf = held.pop().unwrap_or_else(|| Box::new(String::new()));
        buf.clear();
        Box::into_raw(buf) as i64
    })
}

/// # Safety: `at` is a live builder handle; `ptr`/`len` are a UTF-8 slice.
#[no_mangle]
pub unsafe extern "C" fn grasm_str_build_text(at: i64, ptr: i64, len: i64) {
    let buf = &mut *(at as *mut String);
    let bytes = std::slice::from_raw_parts(ptr as *const u8, len as usize);
    buf.push_str(std::str::from_utf8_unchecked(bytes));
}

/// # Safety: `at` is a live builder handle.
#[no_mangle]
pub unsafe extern "C" fn grasm_str_build_i64(at: i64, v: i64) {
    let buf = &mut *(at as *mut String);
    // Digits written by hand rather than through `core::fmt`: the formatting
    // machinery is a trait object, a width calculation and a padding pass for
    // something that is a divide and a table lookup.
    let mut digits = [0u8; 20];
    let mut at_end = digits.len();
    let negative = v < 0;
    // Taken as unsigned so the most negative integer, which has no positive
    // counterpart, does not wrap when its sign is removed.
    let mut n = v.unsigned_abs();
    loop {
        at_end -= 1;
        digits[at_end] = b'0' + (n % 10) as u8;
        n /= 10;
        if n == 0 {
            break;
        }
    }
    if negative {
        at_end -= 1;
        digits[at_end] = b'-';
    }
    buf.push_str(std::str::from_utf8_unchecked(&digits[at_end..]));
}

/// # Safety: `at` is a live builder handle.
#[no_mangle]
pub unsafe extern "C" fn grasm_str_build_f64(at: i64, bits: i64) {
    let buf = &mut *(at as *mut String);
    buf.push_str(&float_text(f64::from_bits(bits as u64)));
}

/// # Safety: `at` is a live builder handle.
#[no_mangle]
pub unsafe extern "C" fn grasm_str_build_bool(at: i64, v: i64) {
    let buf = &mut *(at as *mut String);
    buf.push_str(if v != 0 { "true" } else { "false" });
}

/// # Safety: `at` is a live builder handle; `v` is a live string value.
#[no_mangle]
pub unsafe extern "C" fn grasm_str_build_str(at: i64, v: *mut RtVal) {
    let RtPayload::Str(s) = &(*v).payload else {
        grasm_panic_msg_str("interpolating a non-string as a string");
    };
    let text: &str = s;
    let (p, n) = (text.as_ptr(), text.len());
    let buf = &mut *(at as *mut String);
    buf.push_str(std::str::from_utf8_unchecked(std::slice::from_raw_parts(
        p, n,
    )));
}

/// The finished string, and the buffer back to the thread.
///
/// # Safety: `at` is a live builder handle, not used again after this.
#[no_mangle]
pub unsafe extern "C" fn grasm_str_build_end(at: i64) -> *mut RtVal {
    let mut buf = Box::from_raw(at as *mut String);
    // One reach for the thread's state, not two: the value is allocated and
    // the buffer given back under the same `LOCAL.with`, because on macOS each
    // of those is a call through `_tlv_get_addr`.
    LOCAL.with(|local| {
        let long = buf.len() > INLINE_STR_CAP;
        let out = if long {
            // A long result is *moved*: copying it would be a second pass over
            // text that was just written, which is what made `acc = acc +
            // "{i}"` — a loop that grows one string — twice as slow as the
            // concatenation it replaced. The buffer is given up in that case
            // and the next build makes a fresh one.
            alloc_in(local, RtPayload::Str(RtStr::new(std::mem::take(&mut *buf))))
        } else {
            // A short result is copied into the value's own inline storage and
            // the buffer comes back for the next one.
            let v = alloc_in(local, RtPayload::Str(RtStr::of(&buf)));
            buf.clear();
            v
        };
        let mut held = local.builders.borrow_mut();
        // A handful is enough to cover any nesting a program actually has;
        // past that the buffer is dropped rather than hoarded.
        if held.len() < 8 {
            held.push(buf);
        }
        out
    })
}

#[no_mangle]
pub unsafe extern "C" fn grasm_str_concat(a: *mut RtVal, b: *mut RtVal) -> *mut RtVal {
    let (RtPayload::Str(sa), RtPayload::Str(sb)) = (&(*a).payload, &(*b).payload) else {
        grasm_panic_msg_str("concat on non-strings");
    };
    // No intermediate `String` when the result is short: `RtStr::concat` writes
    // both pieces straight into the inline buffer. Concatenation is on the hot
    // path of every program that builds text.
    alloc(RtPayload::Str(RtStr::concat(sa, sb)))
}

/// `a + b` where `a` will not be used again: the result may reuse `a`'s box.
///
/// Ownership of `a` transfers here; `b` is borrowed. When `a` is the only
/// holder of its string — which the caller arranges by handing over the
/// reference it was keeping, rather than a copy of it — the text is appended
/// in place and the same box comes back. Building a string in a loop is then
/// linear rather than quadratic.
///
/// A count of exactly 1 is also what rules out `s = s + s`: two references to
/// one box cannot both be the only one, so `a` and `b` are different values
/// whenever the fast path is taken.
///
/// # Safety: both are live strings; `a`'s reference transfers to this call.
#[no_mangle]
pub unsafe extern "C" fn grasm_str_append(a: *mut RtVal, b: *mut RtVal) -> *mut RtVal {
    let (RtPayload::Str(_), RtPayload::Str(sb)) = (&(*a).payload, &(*b).payload) else {
        grasm_panic_msg_str("concat on non-strings");
    };
    // Taken as a raw pointer and length: `a` and `b` are provably different
    // boxes on this path, but the borrow checker cannot see that through raw
    // pointers, and reborrowing after the mutable borrow starts would be the
    // thing it is right to complain about.
    let bt: &str = sb;
    let (bp, bl) = (bt.as_ptr(), bt.len());
    if (*a).rc.load(Relaxed) == 1 {
        let RtPayload::Str(sa) = &mut (*a).payload else {
            unreachable!()
        };
        let t = std::str::from_utf8_unchecked(std::slice::from_raw_parts(bp, bl));
        sa.push_str(t);
        return a;
    }
    let RtPayload::Str(sa) = &(*a).payload else {
        unreachable!()
    };
    let out = alloc(RtPayload::Str(RtStr::concat(sa, bt)));
    dec(a);
    out
}

/// A tuple of two, in one call and one allocation.
///
/// A tuple literal used to be built as a list and then sealed: `list_new`, a
/// push per element, and `tuple_seal` — four calls into the runtime for a
/// pair, which measured more than making a two-field struct with the same
/// contents. Both arguments' references transfer.
///
/// # Safety: live runtime pointers, whose ownership passes here.
#[no_mangle]
pub unsafe extern "C" fn grasm_tuple2(a: *mut RtVal, b: *mut RtVal) -> *mut RtVal {
    alloc(RtPayload::Tuple(ListData::pair(a, b)))
}

/// The same for three, which is where tuple literals stop being common.
///
/// # Safety: as `grasm_tuple2`.
#[no_mangle]
pub unsafe extern "C" fn grasm_tuple3(a: *mut RtVal, b: *mut RtVal, c: *mut RtVal) -> *mut RtVal {
    alloc(RtPayload::Tuple(ListData::small(&[a, b, c])))
}

#[no_mangle]
pub unsafe extern "C" fn grasm_str_from_i64(v: i64) -> *mut RtVal {
    alloc(RtPayload::Str(RtStr::of_i64(v)))
}

#[no_mangle]
pub unsafe extern "C" fn grasm_str_from_f64(v: f64) -> *mut RtVal {
    alloc(RtPayload::Str(RtStr::new(float_text(v))))
}

#[no_mangle]
pub unsafe extern "C" fn grasm_str_from_bool(v: i8) -> *mut RtVal {
    alloc(RtPayload::Str(RtStr::of(if v != 0 {
        "true"
    } else {
        "false"
    })))
}

#[no_mangle]
pub unsafe extern "C" fn grasm_str_len(v: *mut RtVal) -> i64 {
    match &(*v).payload {
        RtPayload::Str(s) => s.char_len() as i64,
        _ => grasm_panic_msg_str("len on non-string"),
    }
}

/// How many *bytes* the text occupies, as distinct from how many characters.
///
/// `len` counts code points, which is the right answer for a high-level
/// language and the wrong one the moment a program writes a string to a socket
/// or a file and has to say how long it is. `"🙂".len()` is 1 and
/// `"🙂".byte_len()` is 4, and a `Content-Length` needs the second.
#[no_mangle]
pub unsafe extern "C" fn grasm_str_byte_len(v: *mut RtVal) -> i64 {
    match &(*v).payload {
        RtPayload::Str(s) => s.len() as i64,
        _ => grasm_panic_msg_str("byte_len on non-string"),
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

/// A list this long is split without asking any further questions.
///
/// Spawning costs tens of microseconds, so over-splitting a cheap list wastes
/// almost nothing, while under-splitting an expensive one costs the whole
/// speedup. The user asked for parallelism by writing `par_map`.
const PAR_MIN_ELEMS: usize = 256;

/// And a list shorter than that is split when the work in it is worth a
/// thread, which is measured rather than guessed at.
///
/// A count is the wrong question. The thing that decides whether threading
/// pays is the work *per element*, and a count-based threshold cannot see it:
/// an eight-clip audio mixdown, where one element is forty milliseconds of
/// filtering, ran on one core while a thousand-element `x * x` ran on eight.
/// That was backwards, and it was found by writing the mixdown.
///
/// So the first element is computed on the calling thread and timed — work
/// that had to happen anyway — and the rest is split only if what remains is
/// worth more than the spawns. Being wrong costs one element's time; the
/// answer never changes, because the function is effect-free and each worker
/// writes its own indices.
const PAR_MIN_WORK: std::time::Duration = std::time::Duration::from_micros(500);

/// How many workers to give `n` elements, one of which took `one` to compute.
fn workers_for(n: usize, one: std::time::Duration) -> usize {
    if IN_PARALLEL.with(|f| f.get()) || n < 2 {
        return 1;
    }
    let worth_it = n >= PAR_MIN_ELEMS || one.saturating_mul(n as u32) >= PAR_MIN_WORK;
    if !worth_it {
        return 1;
    }
    std::thread::available_parallelism()
        .map(|p| p.get())
        .unwrap_or(1)
        .min(n)
}

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
    let out = par_words(words, code, &env_owned);
    let kind = ScalarKind::from_i64(out_kind);
    alloc(RtPayload::ListScalar {
        kind,
        data: ScalarData::from_words(kind, out),
    })
}

/// Runs `code` over every word, on as many cores as there is work for.
///
/// Everything crossing a thread here is an `i64`, which is what makes this one
/// function serve both kinds of `par_map`: a scalar list's elements *are*
/// words, and a boxed list's elements are pointers, which are words too.
///
/// # Safety: `code` is a compiler-emitted function with this ABI, and `env`
/// holds `env.len()` words it may read.
unsafe fn par_words(
    words: &[i64],
    code: extern "C" fn(*const i64, i64) -> i64,
    env_owned: &[i64],
) -> Vec<i64> {
    let n = words.len();
    let mut out: Vec<i64> = vec![0; n];
    if n == 0 {
        return out;
    }

    // The first element, on this thread, timed: what it costs is what decides
    // whether the rest is worth spreading.
    let began = std::time::Instant::now();
    out[0] = code(env_owned.as_ptr(), words[0]);
    let workers = workers_for(n, began.elapsed());

    if workers <= 1 {
        for (o, &x) in out.iter_mut().zip(words.iter()).skip(1) {
            *o = code(env_owned.as_ptr(), x);
        }
        return out;
    }
    // Everything but the element already computed, handed out in pieces from
    // a shared queue rather than one big slice each. Elements are not equally
    // expensive — a mandelbrot row in the middle of the set costs far more
    // than one at the edge — and splitting statically means every worker waits
    // for whichever got the expensive half. Dynamic hand-out keeps them all
    // busy until the work is actually gone.
    {
        let input: &[i64] = &words[1..];
        let rest = &mut out[1..];
        let chunk = input.len().div_ceil(workers * 16).max(1);
        let pieces: std::sync::Mutex<Vec<(usize, &mut [i64])>> =
            std::sync::Mutex::new(rest.chunks_mut(chunk).enumerate().collect());
        std::thread::scope(|scope| {
            let queue = &pieces;
            let env_ref = env_owned;
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
                // A worker that cannot be created is not an error: the queue is
                // hand-out, so whatever it would have taken is still there for
                // someone else. The calling thread drains the remainder below,
                // which is why this result can be dropped and why the answer is
                // complete even if every spawn fails.
                let _ = std::thread::Builder::new()
                    .stack_size(WORKER_STACK)
                    .spawn_scoped(scope, move || {
                        drain();
                        // This thread is done, so what it allocated and handed
                        // back stops being its business and starts being
                        // everyone's.
                        hand_back_live();
                    });
            }
            drain();
        });
    }
    out
}

/// `par_map` where the elements or the results are boxed values.
///
/// The same machine as the scalar version — the elements are pointers, and a
/// pointer is a word. What is new is that the workers now touch reference
/// counts: the element and every captured value are shared between them, so
/// this is only correct because `RtVal::rc` is atomic.
///
/// The lifted body *borrows* what it is handed and owns what it returns: it
/// increments the element and each boxed capture on entry, so the releases it
/// makes on the way out balance, and this function keeps its own reference to
/// the input list for the whole call.
///
/// Both representations are told by the compiler rather than read off the
/// value, because they follow the *static* type: `Map.values()` builds a boxed
/// list even for integer elements, and a body compiled for `i64` wants the
/// integer, not the box holding it. `out_kind` is -1 when the results are
/// boxed.
///
/// # Safety: `list` is a live list; `code` and `env` are as above.
#[no_mangle]
pub unsafe extern "C" fn grasm_par_map_mixed(
    list: *mut RtVal,
    code: extern "C" fn(*const i64, i64) -> i64,
    env: *const i64,
    env_len: i64,
    in_boxed: i64,
    out_kind: i64,
) -> *mut RtVal {
    let owned: Vec<i64> = if in_boxed != 0 {
        let RtPayload::List(items) = &(*list).payload else {
            grasm_panic_msg_str("par_map on a non-list");
        };
        items.iter().map(|&p| p as i64).collect()
    } else {
        scalar_words(list, "par_map")
    };
    let env_owned: Vec<i64> = if env_len > 0 {
        std::slice::from_raw_parts(env, env_len as usize).to_vec()
    } else {
        Vec::new()
    };
    let out = par_words(&owned, code, &env_owned);
    if out_kind < 0 {
        let mut built = ListData::new();
        for w in out {
            built.push(w as *mut RtVal);
        }
        return alloc(RtPayload::List(built));
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
/// Opens a scalar list for appending: the buffer, how much is in it, and how
/// much room it has.
///
/// `map` and `filter` fill a list the compiler made a moment earlier, one
/// element per iteration, and each of those elements used to cross the runtime
/// boundary — a call, a copy-on-write check and a match on the payload, for a
/// store. With the buffer and the capacity in hand the loop writes straight
/// into it and calls back only when the room runs out.
///
/// The list must be one the caller owns: this hands out a raw pointer into its
/// storage, and anything that reallocates it while the pointer is live is a
/// use-after-free. The compiler only does this for a list it created itself
/// and that nothing else can name.
///
/// # Safety: `list` is a live list this runtime allocated; the out-params are
/// writable.
#[no_mangle]
pub unsafe extern "C" fn grasm_list_append_begin(
    list: *mut RtVal,
    kind: i64,
    out_data: *mut i64,
    out_len: *mut i64,
    out_cap: *mut i64,
) -> *mut RtVal {
    let want = ScalarKind::from_i64(kind);
    let target = cow_list(list);
    if let RtPayload::List(items) = &(*target).payload {
        let mut data = ScalarData::new(want);
        for &it in items {
            data.push(scalar_unbox_val(want, it));
            dec(it);
        }
        (*target).payload = RtPayload::ListScalar { kind: want, data };
    }
    match &mut (*target).payload {
        RtPayload::ListScalar {
            data: ScalarData::Words(v),
            ..
        } => {
            *out_data = v.as_mut_ptr() as i64;
            *out_len = v.len() as i64;
            *out_cap = v.capacity() as i64;
        }
        RtPayload::ListScalar {
            data: ScalarData::Bytes(v),
            ..
        } => {
            *out_data = v.as_mut_ptr() as i64;
            *out_len = v.len() as i64;
            *out_cap = v.capacity() as i64;
        }
        _ => {
            *out_data = 0;
            *out_len = 0;
            *out_cap = 0;
        }
    }
    target
}

/// Makes room for at least one more element and republishes the buffer.
///
/// # Safety: as `grasm_list_append_begin`; `len` is how many elements the
/// caller has actually written.
#[no_mangle]
pub unsafe extern "C" fn grasm_list_append_grow(
    list: *mut RtVal,
    len: i64,
    out_data: *mut i64,
    out_cap: *mut i64,
) {
    match &mut (*list).payload {
        RtPayload::ListScalar {
            data: ScalarData::Words(v),
            ..
        } => {
            v.set_len(len as usize);
            v.reserve(1);
            *out_data = v.as_mut_ptr() as i64;
            *out_cap = v.capacity() as i64;
        }
        RtPayload::ListScalar {
            data: ScalarData::Bytes(v),
            ..
        } => {
            v.set_len(len as usize);
            v.reserve(1);
            *out_data = v.as_mut_ptr() as i64;
            *out_cap = v.capacity() as i64;
        }
        _ => {
            *out_data = 0;
            *out_cap = 0;
        }
    }
}

/// Closes an append: the list's length becomes what the caller wrote.
///
/// # Safety: `len` elements have actually been written through the pointer
/// `grasm_list_append_begin` handed out, and none of them is uninitialised.
#[no_mangle]
pub unsafe extern "C" fn grasm_list_append_end(list: *mut RtVal, len: i64) {
    match &mut (*list).payload {
        RtPayload::ListScalar {
            data: ScalarData::Words(v),
            ..
        } => v.set_len(len as usize),
        RtPayload::ListScalar {
            data: ScalarData::Bytes(v),
            ..
        } => v.set_len(len as usize),
        _ => {}
    }
}

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
    // The first element decides the rest, the same way `par_map` does it: what
    // one costs is the only thing that says whether threading the others pays.
    let mut workers = 1;
    if n > 0 {
        let began = std::time::Instant::now();
        keep[0] = u8::from(code(env_owned.as_ptr(), words[0]) != 0);
        workers = workers_for(n, began.elapsed());
    }

    if workers <= 1 {
        for (k, &x) in keep.iter_mut().zip(words.iter()).skip(1) {
            *k = u8::from(code(env_owned.as_ptr(), x) != 0);
        }
    } else {
        let input: &[i64] = &words[1..];
        let rest = &mut keep[1..];
        let chunk = input.len().div_ceil(workers * 16).max(1);
        let pieces: std::sync::Mutex<Vec<(usize, &mut [u8])>> =
            std::sync::Mutex::new(rest.chunks_mut(chunk).enumerate().collect());
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

/// `par_filter` over a list of boxed elements.
///
/// The predicate half is the same machine as `par_map`'s: pointers are words,
/// and the workers may touch reference counts because the count is atomic. The
/// compaction is sequential and in input order, which is what makes the result
/// independent of how many cores ran it.
///
/// A survivor is a *new* reference: it stays in the input list as well, and
/// the caller releases that list when the call returns.
///
/// # Safety: `list` is a live boxed list; `code` and `env` are as in `par_map`.
#[no_mangle]
pub unsafe extern "C" fn grasm_par_filter_boxed(
    list: *mut RtVal,
    code: extern "C" fn(*const i64, i64) -> i64,
    env: *const i64,
    env_len: i64,
) -> *mut RtVal {
    let RtPayload::List(items) = &(*list).payload else {
        grasm_panic_msg_str("par_filter on a non-list");
    };
    let words: Vec<i64> = items.iter().map(|&p| p as i64).collect();
    let env_owned: Vec<i64> = if env_len > 0 {
        std::slice::from_raw_parts(env, env_len as usize).to_vec()
    } else {
        Vec::new()
    };
    let verdicts = par_words(&words, code, &env_owned);
    let mut out = ListData::new();
    for (&w, &k) in words.iter().zip(verdicts.iter()) {
        if k != 0 {
            let p = w as *mut RtVal;
            inc(p);
            out.push(p);
        }
    }
    alloc(RtPayload::List(out))
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
    Boxed(&'a [*mut RtVal]),
    Scalar(ScalarKind, &'a ScalarData),
}

/// # Safety: live runtime pointer.
unsafe fn items_of<'a>(v: *mut RtVal) -> Items<'a> {
    match &(*v).payload {
        RtPayload::List(items) => Items::Boxed(std::mem::transmute::<
            &[*mut RtVal],
            &'a [*mut RtVal],
        >(items)),
        RtPayload::ListScalar { kind, data } => Items::Scalar(
            *kind,
            std::mem::transmute::<&ScalarData, &'a ScalarData>(data),
        ),
        _ => grasm_panic_msg_str("expected a list"),
    }
}

/// A new empty scalar list of the given kind (0 = i64, 1 = f64, 2 = bool).
/// An empty scalar list with room for `n` elements already taken.
///
/// `xs.map(f)` produces exactly as many elements as it was given, and a range
/// knows its own length, so the list it builds never has to be grown. Without
/// this a two-million element sieve grew its flag list twenty-one times,
/// copying every byte written so far each time — a third of the program's
/// running time was in the push path rather than in the sieve.
///
/// A negative or absurd count reserves nothing rather than failing: the
/// capacity is a hint, and the pushes that follow are correct either way.
#[no_mangle]
pub extern "C" fn grasm_list_new_scalar_cap(kind: i64, n: i64) -> *mut RtVal {
    let k = ScalarKind::from_i64(kind);
    if n <= 0 {
        return empty_list_of(Some(k));
    }
    let mut data = ScalarData::new(k);
    if n <= (1 << 34) {
        data.reserve(n as usize);
    }
    alloc(RtPayload::ListScalar { kind: k, data })
}

/// The same for a list of boxed values.
#[no_mangle]
pub extern "C" fn grasm_list_new_cap(n: i64) -> *mut RtVal {
    if n <= 0 {
        return empty_list_of(None);
    }
    let mut data = ListData::new();
    if n <= (1 << 34) {
        data.reserve(n as usize);
    }
    alloc(RtPayload::List(data))
}

#[no_mangle]
pub extern "C" fn grasm_list_new_scalar(kind: i64) -> *mut RtVal {
    empty_list_of(Some(ScalarKind::from_i64(kind)))
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

/// The empty list, shared by everything that asks for one.
///
/// Two thirds of the nodes in a tree are leaves, and a leaf's list of children
/// is empty — so a third of everything `tree` allocates is a box holding
/// nothing, made and freed a moment later. There is no reason for a second
/// one to exist: a list is only ever changed through `cow_list`, which copies
/// anything it does not uniquely own, and this is owned by nobody.
///
/// `IMMORTAL`, so `inc` and `dec` skip it and the first push copies it. Built
/// with `Box::into_raw` rather than through `alloc`, for the same reason the
/// string literals are: `alloc` counts a live value and this one is never
/// freed, so it would be reported as a leak at exit.
fn empty_list_of(kind: Option<ScalarKind>) -> *mut RtVal {
    // One per shape: a list of boxed values, and one per scalar width, because
    // the first push writes into whichever storage is already there.
    static BOXED: std::sync::OnceLock<usize> = std::sync::OnceLock::new();
    static SCALARS: std::sync::OnceLock<[usize; 5]> = std::sync::OnceLock::new();
    match kind {
        None => *BOXED.get_or_init(|| {
            Box::into_raw(Box::new(RtVal {
                rc: AtomicI64::new(IMMORTAL),
                payload: RtPayload::List(ListData::new()),
            })) as usize
        }) as *mut RtVal,
        Some(k) => {
            let table = SCALARS.get_or_init(|| {
                let mut out = [0usize; 5];
                for (i, slot) in out.iter_mut().enumerate() {
                    let of = ScalarKind::from_i64(i as i64);
                    *slot = Box::into_raw(Box::new(RtVal {
                        rc: AtomicI64::new(IMMORTAL),
                        payload: RtPayload::ListScalar {
                            kind: of,
                            data: ScalarData::new(of),
                        },
                    })) as usize;
                }
                out
            });
            let at = k.to_i64().clamp(0, 4) as usize;
            table[at] as *mut RtVal
        }
    }
}

#[no_mangle]
pub extern "C" fn grasm_list_new() -> *mut RtVal {
    empty_list_of(None)
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
    if (*list).rc.load(Relaxed) == 1 {
        return list;
    }
    let fresh = match &(*list).payload {
        RtPayload::List(items) => {
            // Built by pushing rather than collected into a `Vec` and then
            // wrapped: collecting allocates, and wrapping keeps the result on
            // the heap even when it would have fitted in the box. A
            // copy-on-write clone of a three-element list was losing its
            // inline storage and allocating again on the next push.
            let mut cloned = ListData::new();
            for &it in items.iter() {
                inc(it);
                cloned.push(it);
            }
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
    let text = display(v);
    emit(|out| {
        let _ = write!(out, "{text}");
    });
}

unsafe fn display(v: *mut RtVal) -> String {
    rt_stack_check();
    match &(*v).payload {
        RtPayload::Unit => "()".to_string(),
        // The bounds, not the elements. `0..2000000` is what the range *is*,
        // and printing two million integers instead would be a different value.
        // `LogHandler{info: <closure>}` — what the interpreter prints, which
        // holds a struct whose fields are the operations. A handler is not a
        // useful thing to print, and the two engines still have to agree on it.
        RtPayload::Handler { effect, ops } => {
            let fields: Vec<String> = ops
                .iter()
                .map(|(name, _, _)| format!("{name}: <closure>"))
                .collect();
            format!("{effect}Handler{{{}}}", fields.join(", "))
        }
        RtPayload::Env { .. } => "<environment>".to_string(),
        RtPayload::Range {
            start,
            end,
            inclusive,
        } => {
            if *inclusive {
                format!("{start}..={end}")
            } else {
                format!("{start}..{end}")
            }
        }
        // Hex, two digits per byte — the same form the interpreter prints, so
        // the two engines agree on what a `bytes` looks like ([L-LIB-BYTES]).
        RtPayload::Bytes(b) => {
            let mut out = String::with_capacity(2 + b.len() * 2);
            out.push_str("0x");
            for byte in b.iter() {
                out.push_str(&format!("{byte:02x}"));
            }
            out
        }
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
        RtPayload::Struct { ty, fields } => {
            let name = ty.name;
            let names = &ty.fields;
            let parts: Vec<String> = fields
                .iter()
                .enumerate()
                .map(|(i, fv)| {
                    let k = names.get(i).copied().unwrap_or("?");
                    format!("{k}: {}", display(*fv))
                })
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

/// Build a struct value: one call, from the type's descriptor and the field
/// values the caller stored into a stack slot.
///
/// This was three calls for a two-field value — make the box, then name and
/// push each field — and each push validated a name as UTF-8, took a `RefCell`
/// and compared lengths to decide whether the type still needed naming. None
/// of that is work the program asked for: the names are the same every time
/// and the compiler already knows them.
///
/// # Safety: `desc` is a descriptor the backend emitted; `argv` points at
/// `desc.nfields` values whose ownership transfers into the struct.
#[no_mangle]
pub unsafe extern "C" fn grasm_struct_make(
    desc: *const StructDesc,
    argv: *const *mut RtVal,
) -> *mut RtVal {
    let ty = desc_ty(desc);
    let n = (*desc).nfields as usize;
    alloc(RtPayload::Struct {
        ty,
        fields: StructFields::from_ptrs(argv, n),
    })
}

/// A field pointer, **borrowed** — no reference count is touched.
///
/// Only sound while the struct holding it is alive, which the caller
/// establishes rather than this function: the backend emits this only for a
/// field path rooted at a live local, where every intermediate is reachable
/// from that local and so cannot be freed during the read.
///
/// It exists for chains. `o.i.v` was three calls — fetch `i` and increment it,
/// read `v` out of it, release `i` — and the increment and release exist only
/// to protect a value the root already protects.
///
/// # Safety: `i` is a field index the compiler resolved for this struct type,
/// and the caller keeps the struct alive for as long as it uses the result.
#[no_mangle]
pub unsafe extern "C" fn grasm_struct_borrow_at(s: *mut RtVal, i: i64) -> *mut RtVal {
    let RtPayload::Struct { fields, .. } = &(*s).payload else {
        grasm_panic_msg_str("field access on non-struct");
    };
    match fields.get(i as usize) {
        Some(v) => *v,
        None => grasm_panic_msg_str("no such field"),
    }
}

/// A **scalar** field's bits, touching no reference count.
///
/// `p.a` for an `i64` field was four runtime calls: read the field and
/// increment it, release the receiver, unbox, release the field. Three million
/// of them cost 12 ns each, against 1.5 ns for reading a local — and a struct
/// field is not supposed to be eight times a local.
///
/// The increment and the decrement cancel exactly. They exist so a boxed field
/// handed out can outlive the read, and a scalar's *bits* cannot outlive
/// anything: they are copied out on the spot. So this returns the bits and the
/// count is never touched.
///
/// Bits rather than a typed value because the backend already knows the type —
/// it chose this call — and one entry point is one entry point.
///
/// # Safety: `i` is a field index the compiler resolved for this struct type,
/// and the field's static type is a scalar.
#[no_mangle]
pub unsafe extern "C" fn grasm_struct_scalar_at(s: *mut RtVal, i: i64) -> i64 {
    let RtPayload::Struct { fields, .. } = &(*s).payload else {
        grasm_panic_msg_str("field access on non-struct");
    };
    match fields.get(i as usize) {
        Some(v) => match &(**v).payload {
            RtPayload::I64(x) => *x,
            RtPayload::Bool(b) => i64::from(*b),
            RtPayload::F64(f) => f.to_bits() as i64,
            RtPayload::Ref(r) => *r,
            RtPayload::Unit => 0,
            // The compiler picked this call from the field's static type, so a
            // non-scalar here is a backend defect rather than a program's.
            _ => grasm_panic_msg_str("scalar field access on a boxed field"),
        },
        None => grasm_panic_msg_str("no such field"),
    }
}

/// A field by its position in the struct's declaration.
///
/// Fields are always stored in declaration order — the backend sorts a literal
/// into that order before pushing, so `P{ b: 2, a: 1 }` and `P{ a: 1, b: 2 }`
/// are laid out identically, which is also why they print identically. So the
/// index is fixed at compile time and a read is a load rather than a walk
/// comparing names.
///
/// Reading the *last* of six fields took twice as long as reading the first,
/// which is what a linear scan looks like from the outside.
///
/// # Safety: `i` is a field index the compiler resolved for this struct type.
#[no_mangle]
pub unsafe extern "C" fn grasm_struct_get_at(s: *mut RtVal, i: i64) -> *mut RtVal {
    let RtPayload::Struct { fields, .. } = &(*s).payload else {
        grasm_panic_msg_str("field access on non-struct");
    };
    match fields.get(i as usize) {
        Some(v) => {
            inc(*v);
            *v
        }
        // Not reachable from compiled code, and cheaper to say so than to
        // return something wrong if it ever is.
        None => grasm_panic_msg_str("no such field"),
    }
}

/// # Safety: as above; returns +1.
#[no_mangle]
pub unsafe extern "C" fn grasm_struct_get(s: *mut RtVal, name: *const u8, len: u64) -> *mut RtVal {
    let RtPayload::Struct { ty, fields } = &(*s).payload else {
        grasm_panic_msg_str("field access on non-struct");
    };
    // Borrowed, not copied: the name is in the binary's read-only data and
    // the stored names are `&'static str`. This used to allocate a heap
    // `String` on *every field read* — three million of them for a loop that
    // reads one field three million times.
    let key = borrowed_str(name, len);
    let idx = ty.fields.iter().position(|n| *n == key);
    if let Some(i) = idx {
        if let Some(v) = fields.get(i) {
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
pub unsafe extern "C" fn grasm_struct_take(s: *mut RtVal, name: *const u8, len: u64) -> *mut RtVal {
    if (*s).rc.load(Relaxed) != 1 {
        return std::ptr::null_mut();
    }
    let RtPayload::Struct { ty, fields } = &mut (*s).payload else {
        grasm_panic_msg_str("field access on non-struct");
    };
    // Borrowed, not copied: the name is in the binary's read-only data and
    // the stored names are `&'static str`. This used to allocate a heap
    // `String` on *every field read* — three million of them for a loop that
    // reads one field three million times.
    let key = borrowed_str(name, len);
    let idx = ty.fields.iter().position(|n| *n == key);
    let hole = interned_bool(false);
    if let Some(i) = idx {
        if let Some(v) = fields.get_mut(i) {
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
    if (*l).rc.load(Relaxed) != 1 {
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

/// The same, by declared position rather than by name ([the reason is the same
/// as `grasm_struct_get_at`'s]).
///
/// A write was the slowest thing left on a struct — three million of them took
/// 157 ms, against 66 ms for six million reads — because it walked the fields
/// comparing names while a read no longer did.
///
/// # Safety: `i` is a field index the compiler resolved for this struct type;
/// `val` ownership transfers.
#[no_mangle]
pub unsafe extern "C" fn grasm_struct_set_field_at(
    s: *mut RtVal,
    i: i64,
    val: *mut RtVal,
) -> *mut RtVal {
    let target = if (*s).rc.load(Relaxed) == 1 {
        s
    } else {
        let RtPayload::Struct { ty: sty, fields } = &(*s).payload else {
            grasm_panic_msg_str("set_field on non-struct");
        };
        let cloned: Vec<*mut RtVal> = fields
            .iter()
            .map(|v| {
                inc(*v);
                *v
            })
            .collect();
        let nv = alloc(RtPayload::Struct {
            ty: sty,
            fields: StructFields::from_values(cloned),
        });
        dec(s);
        nv
    };
    let RtPayload::Struct { fields, .. } = &mut (*target).payload else {
        unreachable!()
    };
    match fields.get_mut(i as usize) {
        Some(slot) => {
            // The value is already in this slot, so the store is a no-op —
            // but the caller took a reference to read it and is handing that
            // reference over, so *something* has to drop it. Decrementing the
            // slot would free what is being stored; doing nothing leaks the
            // caller's, which is what `h.a = h.a` did for a boxed field
            // (F79). Dropping the incoming one is what balances it.
            if *slot == val {
                dec(val);
            } else {
                dec(*slot);
                *slot = val;
            }
        }
        None => grasm_panic_msg_str("no such field"),
    }
    target
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
    let target = if (*s).rc.load(Relaxed) == 1 {
        s
    } else {
        let RtPayload::Struct { ty: sty, fields } = &(*s).payload else {
            grasm_panic_msg_str("set_field on non-struct");
        };
        // Copying a struct copies its field *pointers*; the names live once in
        // the type and cost nothing to carry.
        let cloned: Vec<*mut RtVal> = fields
            .iter()
            .map(|v| {
                inc(*v);
                *v
            })
            .collect();
        let nv = alloc(RtPayload::Struct {
            ty: sty,
            fields: StructFields::from_values(cloned),
        });
        dec(s);
        nv
    };
    let RtPayload::Struct { ty, fields } = &mut (*target).payload else {
        unreachable!()
    };
    let key = borrowed_str(name, len);
    let idx = ty.fields.iter().position(|n| *n == key);
    if let Some(i) = idx {
        if let Some(v) = fields.get_mut(i) {
            // As in `grasm_struct_set_field_at`: the caller's reference is
            // the surplus one when the value is already here (F79).
            if *v == val {
                dec(val);
            } else {
                dec(*v);
                *v = val;
            }
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
    let _ = writeln!(
        out,
        "error: unhandled error from main: {}",
        display(fields[0])
    );
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
unsafe fn val_eq_step(
    a: *mut RtVal,
    b: *mut RtVal,
    work: &mut Vec<(*mut RtVal, *mut RtVal)>,
) -> bool {
    match (&(*a).payload, &(*b).payload) {
        // By the bounds. `0..3 == 0..3` is true and `0..3 == 0..=3` is false,
        // which is what the interpreter answers — and comparing the elements
        // instead would have made `0..0` equal to `5..5`.
        (
            RtPayload::Range {
                start: s1,
                end: e1,
                inclusive: i1,
            },
            RtPayload::Range {
                start: s2,
                end: e2,
                inclusive: i2,
            },
        ) => s1 == s2 && e1 == e2 && i1 == i2,
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
        (RtPayload::Struct { ty: t1, fields: f1 }, RtPayload::Struct { ty: t2, fields: f2 }) => {
            // The type name decides; the field names follow from it, so two
            // values of one type have the same names in the same order by
            // construction and only the values need comparing.
            if t1.name != t2.name || f1.len() != f2.len() {
                return false;
            }
            for (v1, v2) in f1.iter().zip(f2.iter()) {
                work.push((*v1, *v2));
            }
            true
        }
        (
            RtPayload::Variant {
                type_name: t1,
                name: n1,
                fields: f1,
            },
            RtPayload::Variant {
                type_name: t2,
                name: n2,
                fields: f2,
            },
        ) => {
            if t1 != t2 || n1 != n2 || f1.len() != f2.len() {
                return false;
            }
            for (&i, &j) in f1.iter().zip(f2) {
                work.push((i, j));
            }
            true
        }
        (RtPayload::List(x), RtPayload::List(y)) | (RtPayload::Tuple(x), RtPayload::Tuple(y)) => {
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
        // Two runs of bytes are equal when they hold the same bytes. Missing
        // this arm made `b == b` false for every `bytes` value, in both
        // engines at once ([E-EQ-STRUCTURAL]) -- so the two agreed, and the
        // agreement gates could not see it.
        (RtPayload::Bytes(x), RtPayload::Bytes(y)) => x == y,
        // Two function values are the same function when they run the same
        // code over the same captures. Falling through to "not equal" made
        // `dbl == dbl` false and `assert_eq(dbl, dbl)` fail — the one type
        // that was not equal to itself, which also broke deduplicating a list
        // of them ([E-EQ-STRUCTURAL]).
        (
            RtPayload::Fn {
                code: c1, env: e1, ..
            },
            RtPayload::Fn {
                code: c2, env: e2, ..
            },
        ) => c1 == c2 && (e1 == e2 || (!e1.is_null() && !e2.is_null() && val_eq(*e1, *e2))),
        (RtPayload::Tuple(x), RtPayload::Tuple(y)) | (RtPayload::List(x), RtPayload::List(y)) => {
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
                        (ScalarKind::F64, RtPayload::F64(x)) => f64::from_bits(bits as u64) == *x,
                        (ScalarKind::Bool, RtPayload::Bool(x)) => (bits != 0) == *x,
                        _ => false,
                    })
        }
        (RtPayload::Map(x), RtPayload::Map(y)) => {
            x.len() == y.len()
                && x.iter()
                    .all(|(k, &v)| y.get(k).is_some_and(|&w| val_eq(v, w)))
        }
        (RtPayload::Struct { ty: t1, fields: f1 }, RtPayload::Struct { ty: t2, fields: f2 }) => {
            t1.name == t2.name
                && f1.len() == f2.len()
                && f1.iter().zip(f2.iter()).all(|(v1, v2)| val_eq(*v1, *v2))
        }
        (
            RtPayload::Variant {
                type_name: t1,
                name: n1,
                fields: f1,
            },
            RtPayload::Variant {
                type_name: t2,
                name: n2,
                fields: f2,
            },
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
    alloc(RtPayload::Map(MapTable::default()))
}

/// A map's keys in the order iteration must present them.
fn sorted_keys(entries: &MapTable) -> Vec<MapKey> {
    let mut keys: Vec<MapKey> = entries.keys().cloned().collect();
    keys.sort();
    keys
}

unsafe fn cow_map(m: *mut RtVal) -> *mut RtVal {
    if (*m).rc.load(Relaxed) == 1 {
        return m;
    }
    let RtPayload::Map(entries) = &(*m).payload else {
        grasm_panic_msg_str("map op on non-map");
    };
    let cloned: MapTable = entries
        .iter()
        .map(|(k, &v)| {
            inc(v);
            (k.clone(), v)
        })
        .collect();
    dec(m);
    alloc(RtPayload::Map(cloned))
}

/// `m.set(k, (m.get(k) ?? d) + n)` — read, add and write back, hashing **once**.
///
/// Written out, that idiom hashes the key twice: once for the lookup and once
/// for the store. It is how counting is spelled in every language, and every
/// language answers it with one lookup — `m[k]++` in Go, `*m.entry(k)
/// .or_insert(0) += 1` in Rust, `Counter` in Python. The C program this
/// benchmark compares against hashes once and increments the slot in place,
/// so doing it twice was Grasm paying a cost the comparison did not.
///
/// The hasher is unchanged and still SipHash: the saving is one hash, not a
/// cheaper one. Map keys are user data and flood resistance is deliberate.
///
/// # Safety: key ownership transfers; returns the (possibly new) map.
#[no_mangle]
pub unsafe extern "C" fn grasm_map_add_i64(
    m: *mut RtVal,
    k: *mut RtVal,
    delta: i64,
    default: i64,
) -> *mut RtVal {
    let target = cow_map(m);
    let RtPayload::Map(entries) = &mut (*target).payload else {
        unreachable!()
    };
    let slot = with_key(k, |kk| entries.get_mut(kk).map(std::ptr::from_mut));
    match slot {
        Some(slot) => {
            let cur = *slot;
            let old = match &(*cur).payload {
                RtPayload::I64(n) => *n,
                _ => rt_panic("map_add on a map whose values are not integers"),
            };
            // The counter is written where it already is when this map is the
            // only thing holding it. A counting loop otherwise allocated a box
            // for every increment and freed the one before it -- four hundred
            // thousand of each in `wordfreq` -- to store a number one larger.
            //
            // A refcount of exactly 1 is what makes it safe: nobody else can
            // be reading this box, and it rules out the interned small
            // integers on its own, since those are immortal and never 1.
            // Written through the variant rather than by assigning a new
            // payload: assigning drops the old one, and dropping an
            // `RtPayload` is a match over every variant it could have been.
            //
            // A count that is still small has an interned box waiting for it,
            // which costs no allocation either — so that case is asked about
            // first and never pays for the refcount load.
            let new = old + delta;
            let shared = interned_int(new);
            if !shared.is_null() {
                dec(std::mem::replace(&mut *slot, shared));
            } else if (*cur).rc.load(Relaxed) == 1 {
                if let RtPayload::I64(n) = &mut (*cur).payload {
                    *n = new;
                }
            } else {
                dec(std::mem::replace(&mut *slot, grasm_box_i64(new)));
            }
        }
        None => {
            entries.insert(MapKey::of(k), grasm_box_i64(default + delta));
        }
    }
    dec(k);
    target
}

/// # Safety: key/value ownership transfers; returns the (possibly new) map.
#[no_mangle]
pub unsafe extern "C" fn grasm_map_set(m: *mut RtVal, k: *mut RtVal, v: *mut RtVal) -> *mut RtVal {
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

/// The value for `k`, or **null** when absent — no `Option` built.
///
/// `grasm_map_get` builds a `Some`: an `RtVal` *and* a `Vec` for its one field,
/// both freed again by the very next operation. Measured over two million
/// lookups the wrapper cost ~85 ns each, three times the lookup it wrapped, and
/// `m.get(k) ?? d` is among the most common things anyone writes — counting,
/// defaulting, reading a table with a fallback.
///
/// Null rather than an `Option` so the caller can branch on it and evaluate its
/// default **only when the key is missing**. `??` is lazy, and a fused form that
/// evaluated the default eagerly would change what `m.get(k) ?? panic("...")`
/// means.
///
/// # Safety: called from compiled Grasm with boxed values.
#[no_mangle]
pub unsafe extern "C" fn grasm_map_get_ptr(m: *mut RtVal, k: *mut RtVal) -> *mut RtVal {
    let RtPayload::Map(entries) = &(*m).payload else {
        grasm_panic_msg_str("get on non-map");
    };
    match with_key(k, |kk| entries.get(kk).copied()) {
        Some(ev) => {
            inc(ev);
            ev
        }
        None => std::ptr::null_mut(),
    }
}

/// The value for `k` as an `Option`. Still needed: `m.get(k)` used on its own,
/// bound to a name, or matched on takes this path.
///
/// # Safety: called from compiled Grasm with boxed values.
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
    alloc(RtPayload::List(items.into()))
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
            alloc(RtPayload::Tuple(ListData::pair(k.boxed(), v)))
        })
        .collect();
    alloc(RtPayload::List(items.into()))
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
    alloc(RtPayload::List(items.into()))
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
            .map(|c| alloc(RtPayload::Str(RtStr::of(c.encode_utf8(&mut [0u8; 4])))))
            .collect()
    } else {
        text.split(sp)
            .map(|p| alloc(RtPayload::Str(RtStr::of(p))))
            .collect()
    };
    alloc(RtPayload::List(parts.into()))
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
    if (*p).rc.load(Relaxed) == 1 {
        return p;
    }
    // Someone else is holding this value, so mutating it in place would be
    // visible to them as well — and value semantics say it must not be
    // ([E-VALUE-SEMANTICS]). Clone into the slot; the reference still names
    // the same slot, so every holder of the *reference* sees what follows.
    let fresh = match &(*p).payload {
        RtPayload::Struct { ty, fields } => {
            let cloned: Vec<*mut RtVal> = fields
                .iter()
                .map(|v| {
                    inc(*v);
                    *v
                })
                .collect();
            alloc(RtPayload::Struct {
                ty,
                fields: StructFields::from_values(cloned),
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
            alloc(RtPayload::List(cloned.into()))
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
    LOCAL.with(|local| {
        let floor = local.stack_floor.get();
        if floor == 0 {
            local.stack_floor.set(sp.saturating_sub(STACK_BUDGET));
            return;
        }
        if sp < floor {
            rt_panic("stack overflow: recursion too deep");
        }
    });
}

/// This thread's stack floor, for a caller that compares against it itself.
///
/// A function that can recurse used to call `grasm_stack_check` on entry, and
/// that function reached `LOCAL` — which on macOS is a second call, through
/// `_tlv_get_addr`. Two calls, on every recursive call in the language.
/// Measured on `fib(30)`, which makes about 2.7 million of them: the pair cost
/// 5.0 ms of a 7.5 ms program, against 0.67 ms for the same test written
/// inline against a floor already in a register.
///
/// So the floor is fetched once, where the recursion is entered, and passed
/// down as a hidden first argument; the check itself becomes a compare and a
/// branch that the compiler emits in place. This is the call that fetches it.
///
/// # Safety: `sp` is an address in the current thread's stack.
#[no_mangle]
pub extern "C" fn grasm_stack_floor(sp: i64) -> i64 {
    LOCAL.with(|local| {
        let floor = local.stack_floor.get();
        if floor == 0 {
            let set = (sp as usize).saturating_sub(STACK_BUDGET);
            local.stack_floor.set(set);
            return set as i64;
        }
        floor as i64
    })
}

/// What a compiled function calls when its own check fails.
#[no_mangle]
pub extern "C" fn grasm_panic_stack() -> ! {
    rt_panic("stack overflow: recursion too deep")
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
    LOCAL.with(|local| {
        let floor = local.stack_floor.get();
        if floor == 0 {
            local.stack_floor.set(sp.saturating_sub(STACK_BUDGET));
            return;
        }
        if sp < floor {
            rt_panic("stack overflow: recursion too deep");
        }
    });
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
        .map(|w| alloc(RtPayload::Str(RtStr::of(w))))
        .collect();
    alloc(RtPayload::List(items.into()))
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
        Some(c) => alloc(RtPayload::Str(RtStr::of(c.encode_utf8(&mut [0u8; 4])))),
        None => rt_panic(&format!("{n} is not a Unicode code point")),
    }
}

#[no_mangle]
pub unsafe extern "C" fn grasm_str_chars(s: *mut RtVal) -> *mut RtVal {
    let items: Vec<*mut RtVal> = as_str(s)
        .chars()
        .map(|c| alloc(RtPayload::Str(RtStr::of(c.encode_utf8(&mut [0u8; 4])))))
        .collect();
    alloc(RtPayload::List(items.into()))
}

/// # Safety: live pointer.
#[no_mangle]
pub unsafe extern "C" fn grasm_str_trim(s: *mut RtVal) -> *mut RtVal {
    alloc(RtPayload::Str(RtStr::of(as_str(s).trim())))
}

/// # Safety: live pointer.
#[no_mangle]
pub unsafe extern "C" fn grasm_str_trim_start(s: *mut RtVal) -> *mut RtVal {
    alloc(RtPayload::Str(RtStr::of(as_str(s).trim_start())))
}

/// # Safety: live pointer.
#[no_mangle]
pub unsafe extern "C" fn grasm_str_trim_end(s: *mut RtVal) -> *mut RtVal {
    alloc(RtPayload::Str(RtStr::of(as_str(s).trim_end())))
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
    alloc(RtPayload::Str(RtStr::new(
        as_str(s).replace(as_str(from), as_str(to)),
    )))
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
    alloc(RtPayload::Str(RtStr::of(&gs[a..b])))
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

/// An integer's text in a given base, digits `0-9a-z`.
///
/// Base ten is not the only base a systems program needs: every protocol,
/// hash, colour and permission mask is read and written in hex or binary. A
/// base outside 2..=36 panics rather than answering, for the same reason
/// `code_at` panics out of range — there is no right answer to return and a
/// wrong one would be silently wrong.
///
/// `i64::MIN` is handled through the unsigned magnitude, because negating it
/// overflows.
///
/// # Safety: live pointer.
#[no_mangle]
pub unsafe extern "C" fn grasm_i64_to_base(v: i64, base: i64) -> *mut RtVal {
    alloc(RtPayload::Str(RtStr::of(&i64_in_base(v, base))))
}

fn i64_in_base(v: i64, base: i64) -> String {
    if !(2..=36).contains(&base) {
        rt_panic(&format!("to_base takes a base in 2..36, not {base}"));
    }
    const DIGITS: &[u8; 36] = b"0123456789abcdefghijklmnopqrstuvwxyz";
    let base = base as u64;
    let mut n = v.unsigned_abs();
    // Longest possible: 64 binary digits plus a sign.
    let mut buf = [0u8; 65];
    let mut i = buf.len();
    loop {
        i -= 1;
        buf[i] = DIGITS[(n % base) as usize];
        n /= base;
        if n == 0 {
            break;
        }
    }
    if v < 0 {
        i -= 1;
        buf[i] = b'-';
    }
    // ASCII by construction.
    unsafe { std::str::from_utf8_unchecked(&buf[i..]) }.to_string()
}

/// Reads an integer written in a given base.
///
/// `parse_int` deliberately refuses `0xFF` ([L-LIB-PARSE]), so without this a
/// program could write hex and not read it back. No prefix is accepted or
/// required: the base is the argument, not something to be guessed from the
/// text.
///
/// # Safety: live pointer.
#[no_mangle]
pub unsafe extern "C" fn grasm_str_parse_int_base(s: *mut RtVal, base: i64) -> *mut RtVal {
    if !(2..=36).contains(&base) {
        rt_panic(&format!("parse_int_base takes a base in 2..36, not {base}"));
    }
    match i64::from_str_radix(as_str(s).trim(), base as u32) {
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
            alloc(RtPayload::List(out.into()))
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
pub unsafe extern "C" fn grasm_list_remove(
    v: *mut RtVal,
    i: i64,
    out: *mut *mut RtVal,
) -> *mut RtVal {
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
        .map(|i| {
            alloc(RtPayload::Tuple(ListData::pair(
                grasm_box_i64(i),
                grasm_list_get(v, i),
            )))
        })
        .collect();
    alloc(RtPayload::List(items.into()))
}

/// `[(a, b)]` up to the shorter of the two — pairing past the end of one
/// would have to invent an element.
///
/// # Safety: live pointers.
#[no_mangle]
pub unsafe extern "C" fn grasm_list_zip(a: *mut RtVal, b: *mut RtVal) -> *mut RtVal {
    let n = grasm_list_len(a).min(grasm_list_len(b));
    let items: Vec<*mut RtVal> = (0..n)
        .map(|i| {
            alloc(RtPayload::Tuple(ListData::pair(
                grasm_list_get(a, i),
                grasm_list_get(b, i),
            )))
        })
        .collect();
    alloc(RtPayload::List(items.into()))
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
            let src = items.to_vec();
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
            alloc(RtPayload::List(out.into()))
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
            alloc(RtPayload::ListScalar {
                kind: ka,
                data: out,
            })
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
            alloc(RtPayload::List(out.into()))
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
            (ScalarKind::Bool, RtPayload::Bool(x)) => data.iter().any(|b| (b != 0) == *x),
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
    unsafe { journaled("args", || grasm_args_now()) }
}

unsafe fn grasm_args_now() -> *mut RtVal {
    let items: Vec<*mut RtVal> = PROGRAM_ARGS.with(|a| {
        a.borrow()
            .iter()
            .map(|s| alloc(RtPayload::Str(RtStr::of(s))))
            .collect()
    });
    alloc(RtPayload::List(items.into()))
}

#[no_mangle]
pub extern "C" fn grasm_input() -> *mut RtVal {
    unsafe { journaled("input", || grasm_input_now()) }
}

unsafe fn grasm_input_now() -> *mut RtVal {
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
    /// A datagram socket. It sits in the same table as the stream kinds so
    /// that `net_wait`, `net_port` and `net_close` work on it unchanged — a
    /// program multiplexing a listener and a UDP socket in one loop is the
    /// ordinary case for anything that speaks two protocols.
    Datagram(std::net::UdpSocket),
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

/// `poll(2)`, declared here rather than taken from a crate.
///
/// The runtime has no dependencies — it is the thing every compiled Grasm
/// program links, and a dependency there is a dependency in every binary the
/// language produces. Three constants and one function signature is a smaller
/// price than that.
///
/// `nfds_t` is not the same width everywhere: an `unsigned long` on Linux, an
/// `unsigned int` on macOS and the BSDs. Getting it wrong passes garbage in the
/// high half of the register on one platform and works on the other, which is
/// the kind of defect that only appears on somebody else's machine.
#[cfg(unix)]
mod sys_poll {
    #[cfg(target_os = "linux")]
    pub type NfdsT = u64;
    #[cfg(not(target_os = "linux"))]
    pub type NfdsT = u32;

    /// There is something to read — or, for a listener, something to accept.
    pub const POLLIN: i16 = 0x0001;
    /// The peer hung up. Reported in `revents` whether or not it was asked for.
    pub const POLLHUP: i16 = 0x0010;
    /// The descriptor is not open. Also unsolicited.
    pub const POLLNVAL: i16 = 0x0020;

    #[repr(C)]
    pub struct PollFd {
        pub fd: i32,
        pub events: i16,
        pub revents: i16,
    }

    extern "C" {
        pub fn poll(fds: *mut PollFd, nfds: NfdsT, timeout: i32) -> i32;
    }
}

/// Which of these descriptors can be read from without waiting.
///
/// A closed peer counts as ready: the read that follows returns zero bytes,
/// which is how a caller learns the connection ended. Reporting it as *not*
/// ready would leave a hung-up connection in the wait set forever.
#[cfg(unix)]
fn wait_readable(fds: &[i32], timeout_ms: i32) -> Result<Vec<bool>, String> {
    use sys_poll::*;
    let mut set: Vec<PollFd> = fds
        .iter()
        .map(|&fd| PollFd {
            fd,
            events: POLLIN,
            revents: 0,
        })
        .collect();
    loop {
        let n = unsafe { poll(set.as_mut_ptr(), set.len() as NfdsT, timeout_ms) };
        if n >= 0 {
            return Ok(set
                .iter()
                .map(|p| p.revents & (POLLIN | POLLHUP | POLLNVAL) != 0)
                .collect());
        }
        let e = std::io::Error::last_os_error();
        // A signal arriving is not a failure — it is a delivery that happened to
        // land mid-wait, and the caller asked to wait. Retrying here is what
        // stops an ordinary `SIGWINCH` from looking like a network error.
        if e.kind() != std::io::ErrorKind::Interrupted {
            return Err(e.to_string());
        }
    }
}

#[cfg(not(unix))]
fn wait_readable(_fds: &[i32], _timeout_ms: i32) -> Result<Vec<bool>, String> {
    Err("net_wait needs poll(2), which this platform does not provide".to_string())
}

/// The local port a handle is bound to.
///
/// `tcp_listen(0)` asks the system to choose a free port, which is the only way
/// to write a program that runs on a machine where something else already holds
/// the port it wanted. Without a way to read the choice back, that spelling
/// could not be used for anything — a program could bind a port it was unable
/// to name, dial, or print.
///
/// # Safety: called from compiled Grasm with a boxed value.
#[no_mangle]
pub unsafe extern "C" fn grasm_net_port(handle: *mut RtVal) -> *mut RtVal {
    journaled("net_port", || grasm_net_port_now(handle))
}

unsafe fn grasm_net_port_now(handle: *mut RtVal) -> *mut RtVal {
    let h = match &(*handle).payload {
        RtPayload::I64(n) => *n,
        _ => return err_of(io_err("IoFailed", "handle must be an integer".to_string())),
    };
    let port = socket_with(h, |s| {
        let addr = match s {
            Socket::Listener(l) => l.local_addr(),
            Socket::Stream(st) => st.local_addr(),
            Socket::Datagram(d) => d.local_addr(),
        };
        addr.map(|a| i64::from(a.port())).map_err(|e| e.to_string())
    });
    match port {
        Ok(p) => ok_of(alloc(RtPayload::I64(p))),
        Err(e) => err_of(io_err("IoFailed", e)),
    }
}

/// Waits until one of these handles has something to read, or the timeout runs
/// out, and answers with the handles that are ready.
///
/// This is the primitive that lets one thread serve many connections. Without
/// it every `tcp_accept` and every `tcp_read` blocks, so a server can hold one
/// conversation at a time, and holding two needs a thread per connection —
/// which Grasm cannot spawn, because `spawn` does not carry `net`.
///
/// # Safety: called from compiled Grasm with boxed values.
#[no_mangle]
pub unsafe extern "C" fn grasm_net_wait(handles: *mut RtVal, timeout: *mut RtVal) -> *mut RtVal {
    journaled("net_wait", || grasm_net_wait_now(handles, timeout))
}

unsafe fn grasm_net_wait_now(handles: *mut RtVal, timeout: *mut RtVal) -> *mut RtVal {
    use std::os::fd::AsRawFd;

    let timeout_ms = match &(*timeout).payload {
        RtPayload::I64(n) => *n,
        _ => return err_of(io_err("IoFailed", "timeout must be an integer".to_string())),
    };
    // Anything past `i32` is "wait forever" spelled at length, and `-1` is how
    // a caller asks for that directly.
    let timeout_ms = if timeout_ms < 0 {
        -1
    } else {
        timeout_ms.min(i64::from(i32::MAX)) as i32
    };

    let wanted = scalar_words(handles, "net_wait");
    if wanted.is_empty() {
        return ok_of(alloc(RtPayload::ListScalar {
            kind: ScalarKind::I64,
            data: ScalarData::new(ScalarKind::I64),
        }));
    }

    let fds = SOCKETS.with(|table| {
        let table = table.borrow();
        wanted
            .iter()
            .map(|&h| match table.get(h as usize).and_then(Option::as_ref) {
                Some(Socket::Listener(l)) => Ok(l.as_raw_fd()),
                Some(Socket::Stream(st)) => Ok(st.as_raw_fd()),
                Some(Socket::Datagram(d)) => Ok(d.as_raw_fd()),
                None => Err(format!("socket {h} is not open")),
            })
            .collect::<Result<Vec<i32>, String>>()
    });
    let fds = match fds {
        Ok(fds) => fds,
        Err(e) => return err_of(io_err("IoFailed", e)),
    };

    let ready = match wait_readable(&fds, timeout_ms) {
        Ok(r) => r,
        Err(e) => return err_of(io_err("IoFailed", e)),
    };

    // The answer keeps the order it was asked in, so a program that walks the
    // result serves its connections in a fixed order rather than whichever one
    // the kernel happened to list first ([L-DETERMINISM]).
    let mut out = ScalarData::new(ScalarKind::I64);
    for (i, &h) in wanted.iter().enumerate() {
        if ready[i] {
            out.push(h);
        }
    }
    ok_of(alloc(RtPayload::ListScalar {
        kind: ScalarKind::I64,
        data: out,
    }))
}

/// Listens on a TCP port, answering with a handle.
///
/// # Safety: called from compiled Grasm with a boxed value.
#[no_mangle]
pub unsafe extern "C" fn grasm_tcp_listen(port: *mut RtVal) -> *mut RtVal {
    journaled("tcp_listen", || grasm_tcp_listen_now(port))
}

unsafe fn grasm_tcp_listen_now(port: *mut RtVal) -> *mut RtVal {
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
    journaled("tcp_connect", || grasm_tcp_connect_now(host, port))
}

unsafe fn grasm_tcp_connect_now(host: *mut RtVal, port: *mut RtVal) -> *mut RtVal {
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
    journaled("tcp_accept", || grasm_tcp_accept_now(handle))
}

unsafe fn grasm_tcp_accept_now(handle: *mut RtVal) -> *mut RtVal {
    let h = match &(*handle).payload {
        RtPayload::I64(n) => *n,
        _ => return err_of(io_err("IoFailed", "handle must be an integer".to_string())),
    };
    let accepted = socket_with(h, |s| match s {
        Socket::Listener(l) => l
            .accept()
            .map(|(stream, _)| stream)
            .map_err(|e| e.to_string()),
        Socket::Stream(_) => Err(format!("socket {h} is a connection, not a listener")),
        Socket::Datagram(_) => Err(format!("socket {h} is a datagram socket, not a listener")),
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
    journaled("tcp_read", || grasm_tcp_read_now(handle))
}

unsafe fn grasm_tcp_read_now(handle: *mut RtVal) -> *mut RtVal {
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
        Socket::Datagram(_) => Err(format!(
            "socket {h} is a datagram socket — use udp_recv, which keeps message boundaries"
        )),
    });
    match read {
        Ok(text) => ok_of(alloc(RtPayload::Str(RtStr::new(text)))),
        Err(e) => err_of(io_err("IoFailed", e)),
    }
}

/// `n` bytes from the operating system's entropy source.
///
/// Grasm had `now_millis()` and nothing else, so a program could not pick a
/// nonce, make a UUID, shuffle a list, or issue a session token that was not
/// guessable by anyone who knew roughly when it was issued (ProjectGold F34).
///
/// Charged `io`, because it reaches outside the program and two calls do not
/// agree — the same reason the clock is charged. A pure function that answered
/// differently each time would break every promise the effect row makes.
///
/// `/dev/urandom` rather than a userspace generator: it is seeded by the
/// kernel, it does not need reseeding, and a language with no audited crypto
/// has no business shipping its own. A short read is a failure rather than a
/// short answer, because a caller asking for 32 bytes of key material and
/// getting 12 must not proceed.
///
/// # Safety: called from compiled Grasm with boxed values.
#[no_mangle]
pub unsafe extern "C" fn grasm_random_bytes(count: *mut RtVal) -> *mut RtVal {
    journaled("random_bytes", || grasm_random_bytes_now(count))
}

unsafe fn grasm_random_bytes_now(count: *mut RtVal) -> *mut RtVal {
    use std::io::Read;
    let n = match &(*count).payload {
        RtPayload::I64(v) => *v,
        _ => return err_of(io_err("IoFailed", "count must be an integer".to_string())),
    };
    if n < 0 {
        return err_of(io_err(
            "IoFailed",
            format!("random_bytes needs a count of 0 or more, got {n}"),
        ));
    }
    // A cap, because the argument is often a length somebody computed and a
    // mistake there should be a refusal rather than a gigabyte.
    if n > 1_048_576 {
        return err_of(io_err(
            "IoFailed",
            format!("random_bytes is capped at 1048576 bytes, asked for {n}"),
        ));
    }
    // Entropy is the one thing a replay must reproduce exactly and can never
    // obtain again: two runs disagree by construction.
    let mut buf = vec![0u8; n as usize];
    if n == 0 {
        return ok_of(alloc(RtPayload::Bytes(std::sync::Arc::new(buf))));
    }
    match std::fs::File::open("/dev/urandom") {
        Err(e) => err_of(io_err("IoFailed", format!("cannot open /dev/urandom: {e}"))),
        Ok(mut f) => match f.read_exact(&mut buf) {
            Ok(()) => ok_of(alloc(RtPayload::Bytes(std::sync::Arc::new(buf)))),
            Err(e) => err_of(io_err("IoFailed", format!("short read: {e}"))),
        },
    }
}

/// What has arrived, as **bytes**.
///
/// `tcp_read` answers text, and does it lossily, because a `str` is text and a
/// socket carries bytes ([L-LIB-NET]). That is right for a protocol made of
/// text and wrong for one that is not: `multipart/form-data` carries a JPEG,
/// and replacing every invalid sequence with U+FFFD corrupts the file in a way
/// nothing downstream can detect. `bytes` already existed; this is the socket
/// learning to speak it.
///
/// # Safety: called from compiled Grasm with boxed values.
#[no_mangle]
pub unsafe extern "C" fn grasm_tcp_read_bytes(handle: *mut RtVal) -> *mut RtVal {
    journaled("tcp_read_bytes", || grasm_tcp_read_bytes_now(handle))
}

unsafe fn grasm_tcp_read_bytes_now(handle: *mut RtVal) -> *mut RtVal {
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
                    Ok(buf)
                }
                Err(e) => Err(e.to_string()),
            }
        }
        Socket::Listener(_) => Err(format!("socket {h} is a listener, not a connection")),
        Socket::Datagram(_) => Err(format!(
            "socket {h} is a datagram socket — use udp_recv, which keeps message boundaries"
        )),
    });
    match read {
        Ok(b) => ok_of(alloc(RtPayload::Bytes(std::sync::Arc::new(b)))),
        Err(e) => err_of(io_err("IoFailed", e)),
    }
}

/// Writes **bytes** to a connection: `tcp_write`'s other half.
///
/// # Safety: called from compiled Grasm with boxed values.
#[no_mangle]
pub unsafe extern "C" fn grasm_tcp_write_bytes(handle: *mut RtVal, data: *mut RtVal) -> *mut RtVal {
    journaled("tcp_write_bytes", || {
        grasm_tcp_write_bytes_now(handle, data)
    })
}

unsafe fn grasm_tcp_write_bytes_now(handle: *mut RtVal, data: *mut RtVal) -> *mut RtVal {
    use std::io::Write;
    let h = match &(*handle).payload {
        RtPayload::I64(n) => *n,
        _ => return err_of(io_err("IoFailed", "handle must be an integer".to_string())),
    };
    let RtPayload::Bytes(payload) = &(*data).payload else {
        return err_of(io_err(
            "IoFailed",
            "tcp_write_bytes needs bytes".to_string(),
        ));
    };
    let payload = payload.clone();
    let wrote = socket_with(h, |s| match s {
        Socket::Stream(stream) => stream
            .write_all(&payload)
            .and_then(|()| stream.flush())
            .map_err(|e| e.to_string()),
        Socket::Listener(_) => Err(format!("socket {h} is a listener, not a connection")),
        Socket::Datagram(_) => Err(format!(
            "socket {h} is a datagram socket — use udp_send, which needs a destination"
        )),
    });
    match wrote {
        // `Unit`, not `I64(0)`. The two are indistinguishable to a compiled
        // build — the static type is `()` and the backend prints it from the
        // type rather than the value — but they are not indistinguishable once
        // the value is written to a journal and handed to another engine, which
        // printed `0` where every direct run prints `()` (F37).
        Ok(()) => ok_of(alloc(RtPayload::Unit)),
        Err(e) => err_of(io_err("IoFailed", e)),
    }
}

/// Writes text to a connection.
///
/// # Safety: called from compiled Grasm with boxed values.
#[no_mangle]
pub unsafe extern "C" fn grasm_tcp_write(handle: *mut RtVal, data: *mut RtVal) -> *mut RtVal {
    journaled("tcp_write", || grasm_tcp_write_now(handle, data))
}

unsafe fn grasm_tcp_write_now(handle: *mut RtVal, data: *mut RtVal) -> *mut RtVal {
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
        Socket::Datagram(_) => Err(format!(
            "socket {h} is a datagram socket — use udp_send, which needs a destination"
        )),
    });
    match wrote {
        // `Unit`, not `I64(0)`. The two are indistinguishable to a compiled
        // build — the static type is `()` and the backend prints it from the
        // type rather than the value — but they are not indistinguishable once
        // the value is written to a journal and handed to another engine, which
        // printed `0` where every direct run prints `()` (F37).
        Ok(()) => ok_of(alloc(RtPayload::Unit)),
        Err(e) => err_of(io_err("IoFailed", e)),
    }
}

/// Runs a chain of programs, each one's output feeding the next.
///
/// A pipeline cannot be built out of `proc_run`: that waits for a program to
/// finish and hands back its output, and a pipeline needs one child's stdout to
/// *be* another's stdin. Copying the bytes through Grasm instead would need
/// concurrent reads and writes on pipe handles — a much larger surface for a
/// smaller result, and the kernel already does this.
///
/// `input` feeds the first stage's stdin, which is the other thing
/// `proc_run` could not do: a filter with nothing to filter is not a filter.
///
/// # Safety: called from compiled Grasm with boxed values.
#[no_mangle]
pub unsafe extern "C" fn grasm_proc_pipeline(stages: *mut RtVal, input: *mut RtVal) -> *mut RtVal {
    journaled("proc_pipeline", || grasm_proc_pipeline_now(stages, input))
}

unsafe fn grasm_proc_pipeline_now(stages: *mut RtVal, input: *mut RtVal) -> *mut RtVal {
    use std::io::Write;
    use std::process::{Command, Stdio};

    let RtPayload::List(items) = &(*stages).payload else {
        return err_of(io_err(
            "IoFailed",
            "proc_pipeline takes a list of stages".to_string(),
        ));
    };
    if items.is_empty() {
        return err_of(io_err(
            "IoFailed",
            "a pipeline needs at least one stage".to_string(),
        ));
    }
    let mut plan: Vec<(String, Vec<String>)> = Vec::with_capacity(items.len());
    for &it in items.iter() {
        let RtPayload::Tuple(fields) = &(*it).payload else {
            return err_of(io_err(
                "IoFailed",
                "each stage is a (program, arguments) pair".to_string(),
            ));
        };
        let program = as_str(fields[0]).to_string();
        let argv: Vec<String> = match &(*fields[1]).payload {
            RtPayload::List(a) => a.iter().map(|&p| as_str(p).to_string()).collect(),
            _ => Vec::new(),
        };
        plan.push((program, argv));
    }
    let stdin_text = as_str(input).to_string();

    let mut children: Vec<std::process::Child> = Vec::with_capacity(plan.len());
    let mut prev: Option<std::process::ChildStdout> = None;
    for (i, (program, argv)) in plan.iter().enumerate() {
        let is_last = i + 1 == plan.len();
        let mut cmd = Command::new(program);
        cmd.args(argv);
        match prev.take() {
            Some(out) => {
                cmd.stdin(Stdio::from(out));
            }
            None => {
                cmd.stdin(Stdio::piped());
            }
        }
        cmd.stdout(Stdio::piped());
        // Only the last stage's stderr is answered with. An earlier stage's
        // goes to this program's own stderr rather than being discarded: a
        // diagnostic nobody sees is worse than one in the wrong place.
        if is_last {
            cmd.stderr(Stdio::piped());
        } else {
            cmd.stderr(Stdio::inherit());
        }
        let mut child = match cmd.spawn() {
            Ok(c) => c,
            Err(e) => {
                // Everything already started is killed and reaped, so a
                // pipeline that fails halfway leaves nothing running.
                for mut c in children {
                    let _ = c.kill();
                    let _ = c.wait();
                }
                return err_of(io_err("IoFailed", format!("{program}: {e}")));
            }
        };
        if i == 0 && !stdin_text.is_empty() {
            // Written on a thread: a large input and a chatty child deadlock
            // if the parent writes everything before it reads anything.

            if let Some(mut sink) = child.stdin.take() {
                let text = stdin_text.clone();
                std::thread::spawn(move || {
                    let _ = sink.write_all(text.as_bytes());
                });
            }
        } else if i == 0 {
            // Closed, so a filter reading to end-of-input does not wait
            // forever for input that is never coming.
            drop(child.stdin.take());
        }
        if !is_last {
            prev = child.stdout.take();
        }
        children.push(child);
    }

    let last = children.pop().expect("at least one stage");
    let out = match last.wait_with_output() {
        Ok(o) => o,
        Err(e) => return err_of(io_err("IoFailed", e.to_string())),
    };
    // The earlier stages are reaped so they do not become zombies. Their exit
    // statuses are deliberately not reported: a pipeline's status is its last
    // stage's, which is what every shell means by it.
    for mut c in children {
        let _ = c.wait();
    }
    let code = out.status.code().map_or(-1, i64::from);
    let fields = vec![
        alloc(RtPayload::I64(code)),
        alloc(RtPayload::Str(RtStr::of(&String::from_utf8_lossy(
            &out.stdout,
        )))),
        alloc(RtPayload::Str(RtStr::of(&String::from_utf8_lossy(
            &out.stderr,
        )))),
    ];
    ok_of(alloc(RtPayload::Tuple(fields.into())))
}

// --- the C boundary --------------------------------------------------------
//
// A `str` or `bytes` argument to an `extern fn` crosses as a pointer
// ([T-EXTERN-POINTER]). The buffer belongs to Grasm and is alive for the
// duration of the call, so nothing about ownership has to be decided at the
// boundary — which is exactly why a *returned* pointer is still refused.

/// A NUL-terminated copy of a string, for C.
///
/// Grasm strings carry their length and are not terminated; C decides where a
/// string ends by looking for the NUL. Handing over the raw bytes would have
/// the callee read past the end of the buffer, so this copies.
///
/// The caller frees it with `grasm_str_c_free` after the call.
///
/// # Safety: called from compiled Grasm with a boxed value.
#[no_mangle]
pub unsafe extern "C" fn grasm_str_c_new(v: *mut RtVal) -> *mut std::ffi::c_char {
    match std::ffi::CString::new(as_str(v).as_bytes()) {
        Ok(c) => {
            // Counted, because nothing else counts it. `LIVE` tracks the
            // runtime's own allocations, and this one is `CString`'s -- so a
            // copy handed to C and never freed was invisible to
            // `GRASM_LEAK_CHECK`, which is the one leak an `extern fn` is
            // most likely to have. Deleting the free outright still reported
            // a clean run (F57).
            C_COPIES.fetch_add(1, Relaxed);
            c.into_raw()
        }
        Err(_) => rt_panic(
            "a `str` passed to C contains a NUL byte, which is what C uses to mark the end of a string",
        ),
    }
}

/// # Safety: `p` came from `grasm_str_c_new` and is freed once.
#[no_mangle]
pub unsafe extern "C" fn grasm_str_c_free(p: *mut std::ffi::c_char) {
    if !p.is_null() {
        C_COPIES.fetch_sub(1, Relaxed);
        drop(std::ffi::CString::from_raw(p));
    }
}

/// A pointer to a `bytes` buffer.
///
/// No copy and no terminator: the buffer is already contiguous, it is not text,
/// and appending a NUL would change its length. How long it is stays the
/// program's to pass, because C has no way to ask.
///
/// # Safety: called from compiled Grasm with a boxed value that outlives the
/// call the pointer is passed to.
#[no_mangle]
pub unsafe extern "C" fn grasm_bytes_ptr(v: *mut RtVal) -> *mut u8 {
    match &(*v).payload {
        RtPayload::Bytes(b) => b.as_ptr() as *mut u8,
        _ => rt_panic("bytes_ptr on a value that is not bytes"),
    }
}

// ---------------------------------------------------------------------------
// Processes — the `proc` capability
// ---------------------------------------------------------------------------
//
// Running another program is not `io` and it is not `net`. It is strictly
// larger than both: a program that can run `curl` has the network whether or
// not it declared `net`, and a program that can run `rm` has the filesystem.
// Folding it into `io` would have made every capability claim in the language
// weaker than it reads, so it gets its own.

/// Runs a program to completion, answering with `(exit code, stdout, stderr)`.
///
/// Output is captured rather than inherited, because a builtin that wrote to
/// the parent's stdout would make the three engines disagree on ordering — the
/// child's writes and the parent's go through different buffers. A caller that
/// wants the output shown prints it.
///
/// # Safety: called from compiled Grasm with boxed values.
#[no_mangle]
pub unsafe extern "C" fn grasm_proc_run(cmd: *mut RtVal, args: *mut RtVal) -> *mut RtVal {
    journaled("proc_run", || grasm_proc_run_now(cmd, args))
}

unsafe fn grasm_proc_run_now(cmd: *mut RtVal, args: *mut RtVal) -> *mut RtVal {
    let program = as_str(cmd).to_string();
    let argv: Vec<String> = match &(*args).payload {
        RtPayload::List(items) => items.iter().map(|&p| as_str(p).to_string()).collect(),
        _ => {
            return err_of(io_err(
                "IoFailed",
                "run takes a list of arguments".to_string(),
            ))
        }
    };
    match std::process::Command::new(&program).args(&argv).output() {
        Ok(out) => {
            // `None` means a signal killed it. `-1` is not a status any
            // process can exit with, so it cannot be confused for one.
            let code = out.status.code().map_or(-1, i64::from);
            let fields = vec![
                alloc(RtPayload::I64(code)),
                alloc(RtPayload::Str(RtStr::of(&String::from_utf8_lossy(
                    &out.stdout,
                )))),
                alloc(RtPayload::Str(RtStr::of(&String::from_utf8_lossy(
                    &out.stderr,
                )))),
            ];
            ok_of(alloc(RtPayload::Tuple(fields.into())))
        }
        Err(e) => err_of(io_err("IoFailed", format!("{program}: {e}"))),
    }
}

// --- datagrams -------------------------------------------------------------
//
// UDP is not TCP with the reliability removed; it is a different shape. A
// stream has no message boundaries and both ends are fixed at connect time, so
// `tcp_write(h, text)` is complete. A datagram is a *whole message* sent to an
// address named at send time, and the address a reply came from is part of what
// was received. So the operations cannot share signatures with the stream ones,
// and pretending otherwise would lose exactly the information that makes UDP
// worth having.
//
// Payloads are `bytes`, not `str`. A DNS query is a packed binary structure
// with bit-level flags, and routing it through a lossy UTF-8 conversion — which
// is what the stream operations do, correctly, for text — would corrupt it.

/// Binds a UDP socket. Port `0` asks the system to choose; `net_port` reads the
/// choice back.
///
/// # Safety: called from compiled Grasm with a boxed value.
#[no_mangle]
pub unsafe extern "C" fn grasm_udp_bind(port: *mut RtVal) -> *mut RtVal {
    journaled("udp_bind", || grasm_udp_bind_now(port))
}

unsafe fn grasm_udp_bind_now(port: *mut RtVal) -> *mut RtVal {
    let port = match &(*port).payload {
        RtPayload::I64(n) => *n,
        _ => return err_of(io_err("IoFailed", "port must be an integer".to_string())),
    };
    if !(0..=65535).contains(&port) {
        return err_of(io_err("IoFailed", format!("port {port} is out of range")));
    }
    match std::net::UdpSocket::bind(("0.0.0.0", port as u16)) {
        Ok(d) => ok_of(alloc(RtPayload::I64(socket_put(Socket::Datagram(d))))),
        Err(e) => err_of(io_err("IoFailed", e.to_string())),
    }
}

/// Sends one datagram to `host:port`, answering with the number of bytes sent.
///
/// # Safety: called from compiled Grasm with boxed values.
#[no_mangle]
pub unsafe extern "C" fn grasm_udp_send(
    handle: *mut RtVal,
    host: *mut RtVal,
    port: *mut RtVal,
    data: *mut RtVal,
) -> *mut RtVal {
    journaled("udp_send", || grasm_udp_send_now(handle, host, port, data))
}

unsafe fn grasm_udp_send_now(
    handle: *mut RtVal,
    host: *mut RtVal,
    port: *mut RtVal,
    data: *mut RtVal,
) -> *mut RtVal {
    let h = match &(*handle).payload {
        RtPayload::I64(n) => *n,
        _ => return err_of(io_err("IoFailed", "handle must be an integer".to_string())),
    };
    let port = match &(*port).payload {
        RtPayload::I64(n) => *n,
        _ => return err_of(io_err("IoFailed", "port must be an integer".to_string())),
    };
    if !(1..=65535).contains(&port) {
        return err_of(io_err("IoFailed", format!("port {port} is out of range")));
    }
    let host = as_str(host).to_string();
    let payload = match &(*data).payload {
        RtPayload::Bytes(b) => b.clone(),
        _ => return err_of(io_err("IoFailed", "udp_send takes bytes".to_string())),
    };
    let sent = socket_with(h, |s| match s {
        Socket::Datagram(d) => d
            .send_to(&payload, (host.as_str(), port as u16))
            .map(|n| n as i64)
            .map_err(|e| e.to_string()),
        _ => Err(format!("socket {h} is not a datagram socket")),
    });
    match sent {
        Ok(n) => ok_of(alloc(RtPayload::I64(n))),
        Err(e) => err_of(io_err("IoFailed", e)),
    }
}

/// Receives **one** datagram, whole.
///
/// A datagram is a message, not a position in a stream: a read that returned
/// half of one, or two of them joined, would have destroyed the only thing UDP
/// guarantees. Anything past 65535 bytes cannot arrive over IPv4 in one piece,
/// so a buffer that size cannot truncate a datagram that was deliverable.
///
/// This blocks until a datagram arrives. `net_wait` is how a program asks first
/// ([L-LIB-NET]).
///
/// # Safety: called from compiled Grasm with a boxed value.
#[no_mangle]
pub unsafe extern "C" fn grasm_udp_recv(handle: *mut RtVal) -> *mut RtVal {
    journaled("udp_recv", || grasm_udp_recv_now(handle))
}

unsafe fn grasm_udp_recv_now(handle: *mut RtVal) -> *mut RtVal {
    let h = match &(*handle).payload {
        RtPayload::I64(n) => *n,
        _ => return err_of(io_err("IoFailed", "handle must be an integer".to_string())),
    };
    let got = socket_with(h, |s| match s {
        Socket::Datagram(d) => {
            let mut buf = vec![0u8; 65536];
            match d.recv_from(&mut buf) {
                Ok((n, _)) => {
                    buf.truncate(n);
                    Ok(buf)
                }
                Err(e) => Err(e.to_string()),
            }
        }
        _ => Err(format!("socket {h} is not a datagram socket")),
    });
    match got {
        Ok(b) => ok_of(alloc(RtPayload::Bytes(std::sync::Arc::new(b)))),
        Err(e) => err_of(io_err("IoFailed", e)),
    }
}

/// One datagram, and the address it came from.
///
/// `udp_recv` already asked the operating system for both and threw the second
/// away. A datagram socket has no fixed peer — that is what distinguishes it
/// from a stream — so a receiver that cannot learn the sender's address can
/// only answer peers it was configured with in advance, which rules out every
/// protocol where a client's address is not known until it calls.
///
/// # Safety: called from compiled Grasm with a boxed value.
#[no_mangle]
pub unsafe extern "C" fn grasm_udp_recv_from(handle: *mut RtVal) -> *mut RtVal {
    journaled("udp_recv_from", || grasm_udp_recv_from_now(handle))
}

unsafe fn grasm_udp_recv_from_now(handle: *mut RtVal) -> *mut RtVal {
    let h = match &(*handle).payload {
        RtPayload::I64(n) => *n,
        _ => return err_of(io_err("IoFailed", "handle must be an integer".to_string())),
    };
    let got = socket_with(h, |s| match s {
        Socket::Datagram(d) => {
            let mut buf = vec![0u8; 65536];
            match d.recv_from(&mut buf) {
                Ok((n, from)) => {
                    buf.truncate(n);
                    Ok((buf, from.ip().to_string(), i64::from(from.port())))
                }
                Err(e) => Err(e.to_string()),
            }
        }
        _ => Err(format!("socket {h} is not a datagram socket")),
    });
    match got {
        Ok((b, host, port)) => {
            let fields = vec![
                alloc(RtPayload::Bytes(std::sync::Arc::new(b))),
                alloc(RtPayload::Str(RtStr::of(&host))),
                alloc(RtPayload::I64(port)),
            ];
            ok_of(alloc(RtPayload::Tuple(fields.into())))
        }
        Err(e) => err_of(io_err("IoFailed", e)),
    }
}

/// Closes a listener or a connection. Closing one twice is not an error, for
/// the same reason releasing a `ref` twice is not: cleanup order is not
/// something a caller should have to track.
///
/// # Safety: called from compiled Grasm with a boxed value.
#[no_mangle]
pub unsafe extern "C" fn grasm_net_close(handle: *mut RtVal) {
    // Answers nothing, but is still a journal record: a replay never opened
    // the socket, so closing it must not reach the socket table either, and
    // the record keeps the call sequence lined up with the recording.
    if replay::gate("net_close").is_some() {
        return;
    }
    if replay::recording() {
        replay::put("net_close", &replay::Wire::Unit);
    }
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
    journaled("read_file", || grasm_read_file_now(path))
}

unsafe fn grasm_read_file_now(path: *mut RtVal) -> *mut RtVal {
    let p = as_str(path).to_string();
    match std::fs::read_to_string(&p) {
        Ok(s) => ok_of(alloc(RtPayload::Str(RtStr::new(s)))),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => err_of(io_err("NotFound", p)),
        Err(e) => err_of(io_err("IoFailed", e.to_string())),
    }
}

#[no_mangle]
pub unsafe extern "C" fn grasm_write_file(path: *mut RtVal, content: *mut RtVal) -> *mut RtVal {
    journaled("write_file", || grasm_write_file_now(path, content))
}

unsafe fn grasm_write_file_now(path: *mut RtVal, content: *mut RtVal) -> *mut RtVal {
    let p = as_str(path).to_string();
    let c = as_str(content).to_string();
    match std::fs::write(&p, c) {
        // `Unit`, not `I64(0)`. The two are indistinguishable to a compiled
        // build — the static type is `()` and the backend prints it from the
        // type rather than the value — but they are not indistinguishable once
        // the value is written to a journal and handed to another engine, which
        // printed `0` where every direct run prints `()` (F37).
        Ok(()) => ok_of(alloc(RtPayload::Unit)),
        Err(e) => err_of(io_err("IoFailed", e.to_string())),
    }
}

#[no_mangle]
pub unsafe extern "C" fn grasm_file_exists(path: *mut RtVal) -> i8 {
    if let Some(replay::Wire::Bool(b)) = replay::gate("file_exists") {
        return i8::from(b);
    }
    let there = std::path::Path::new(as_str(path)).exists();
    if replay::recording() {
        replay::put("file_exists", &replay::Wire::Bool(there));
    }
    i8::from(there)
}

// ---------------------------------------------------------------------------
// bytes ([L-LIB-BYTES])

unsafe fn as_bytes(v: *mut RtVal) -> std::sync::Arc<Vec<u8>> {
    match &(*v).payload {
        RtPayload::Bytes(b) => b.clone(),
        _ => std::sync::Arc::new(Vec::new()),
    }
}

/// # Safety: `list` is a boxed list of integers.
#[no_mangle]
pub unsafe extern "C" fn grasm_bytes_of(list: *mut RtVal) -> *mut RtVal {
    // Both list representations. A `[i64]` reaching here from compiled code is
    // a *scalar* list — elements stored inline, eight bytes each, no boxes —
    // while the same list built through a boxed path is `List`. Handling only
    // one of them is how a runtime function works under one engine and panics
    // under the other.
    let values: Vec<i64> = match &(*list).payload {
        RtPayload::ListScalar { data, .. } => (0..data.len()).map(|i| data.get(i)).collect(),
        RtPayload::List(items) => items
            .iter()
            .map(|it| match (**it).payload {
                RtPayload::I64(n) => n,
                _ => {
                    let m = "bytes_of expects a list of integers";
                    rt_panic(m);
                }
            })
            .collect(),
        _ => rt_panic("bytes_of expects a list"),
    };
    let mut out = Vec::with_capacity(values.len());
    for n in values {
        if !(0..=255).contains(&n) {
            rt_panic(&format!(
                "{n} is not a byte — bytes_of takes values in 0..255"
            ));
        }
        out.push(n as u8);
    }
    alloc(RtPayload::Bytes(std::sync::Arc::new(out)))
}

/// # Safety: boxed values.
#[no_mangle]
pub unsafe extern "C" fn grasm_bytes_len(v: *mut RtVal) -> i64 {
    as_bytes(v).len() as i64
}

/// # Safety: boxed values.
#[no_mangle]
pub unsafe extern "C" fn grasm_bytes_at(v: *mut RtVal, i: i64) -> i64 {
    let b = as_bytes(v);
    if i < 0 || i as usize >= b.len() {
        rt_panic(&format!("index {i} out of bounds (len {})", b.len()));
    }
    b[i as usize] as i64
}

/// # Safety: boxed values.
#[no_mangle]
pub unsafe extern "C" fn grasm_bytes_slice(v: *mut RtVal, from: i64, end: i64) -> *mut RtVal {
    let b = as_bytes(v);
    if from < 0 || end < from || end as usize > b.len() {
        rt_panic(&format!(
            "slice {from}..{end} out of bounds (len {})",
            b.len()
        ));
    }
    alloc(RtPayload::Bytes(std::sync::Arc::new(
        b[from as usize..end as usize].to_vec(),
    )))
}

/// # Safety: boxed values.
#[no_mangle]
pub unsafe extern "C" fn grasm_bytes_concat(a: *mut RtVal, b: *mut RtVal) -> *mut RtVal {
    let mut out = as_bytes(a).as_ref().clone();
    out.extend_from_slice(&as_bytes(b));
    alloc(RtPayload::Bytes(std::sync::Arc::new(out)))
}

/// # Safety: boxed values.
#[no_mangle]
pub unsafe extern "C" fn grasm_bytes_to_list(v: *mut RtVal) -> *mut RtVal {
    let items: Vec<*mut RtVal> = as_bytes(v)
        .iter()
        .map(|x| alloc(RtPayload::I64(*x as i64)))
        .collect();
    alloc(RtPayload::List(items.into()))
}

/// Lossy, as `tcp_read` is: a buffer carries bytes, a `str` carries text.
///
/// # Safety: boxed values.
#[no_mangle]
pub unsafe extern "C" fn grasm_bytes_to_str(v: *mut RtVal) -> *mut RtVal {
    let text = String::from_utf8_lossy(&as_bytes(v)).into_owned();
    alloc(RtPayload::Str(RtStr::new(text)))
}

/// # Safety: boxed values.
#[no_mangle]
pub unsafe extern "C" fn grasm_str_to_bytes(v: *mut RtVal) -> *mut RtVal {
    let bytes = as_str(v).to_string().into_bytes();
    alloc(RtPayload::Bytes(std::sync::Arc::new(bytes)))
}

/// # Safety: boxed values.
#[no_mangle]
pub unsafe extern "C" fn grasm_read_bytes(path: *mut RtVal) -> *mut RtVal {
    journaled("read_bytes", || grasm_read_bytes_now(path))
}

unsafe fn grasm_read_bytes_now(path: *mut RtVal) -> *mut RtVal {
    let p = as_str(path).to_string();
    match std::fs::read(&p) {
        Ok(b) => ok_of(alloc(RtPayload::Bytes(std::sync::Arc::new(b)))),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => err_of(io_err("NotFound", p)),
        Err(e) => err_of(io_err("IoFailed", e.to_string())),
    }
}

/// A range of a file, read without loading the rest.
///
/// Every other file call in this runtime is whole-file, which is the right
/// shape for configuration and the wrong one for data: fetching one record
/// from a four-megabyte table read four megabytes — 40,000x amplification,
/// measured — and a table larger than memory could not be read at all.
///
/// A read starting past the end answers empty rather than failing, so a caller
/// walking to the end stops rather than raising. A read overlapping the end
/// answers what is there, short, like any other read of a file that ended.
///
/// # Safety: boxed values.
#[no_mangle]
pub unsafe extern "C" fn grasm_read_bytes_at(
    path: *mut RtVal,
    offset: *mut RtVal,
    count: *mut RtVal,
) -> *mut RtVal {
    journaled("read_bytes_at", || {
        grasm_read_bytes_at_now(path, offset, count)
    })
}

unsafe fn grasm_read_bytes_at_now(
    path: *mut RtVal,
    offset: *mut RtVal,
    count: *mut RtVal,
) -> *mut RtVal {
    use std::io::{Read, Seek, SeekFrom};
    let p = as_str(path).to_string();
    let (RtPayload::I64(off), RtPayload::I64(want)) = (&(*offset).payload, &(*count).payload)
    else {
        return err_of(io_err(
            "IoFailed",
            "read_bytes_at takes an integer offset and count".to_string(),
        ));
    };
    let (off, want) = (*off, *want);
    if off < 0 || want < 0 {
        return err_of(io_err(
            "IoFailed",
            format!("read_bytes_at: offset {off} and count {want} must not be negative"),
        ));
    }
    match std::fs::File::open(&p) {
        Ok(mut f) => {
            if let Err(e) = f.seek(SeekFrom::Start(off as u64)) {
                return err_of(io_err("IoFailed", e.to_string()));
            }
            let mut buf = vec![0u8; want as usize];
            let mut filled = 0usize;
            while filled < buf.len() {
                match f.read(&mut buf[filled..]) {
                    Ok(0) => break,
                    Ok(n) => filled += n,
                    Err(e) => return err_of(io_err("IoFailed", e.to_string())),
                }
            }
            buf.truncate(filled);
            ok_of(alloc(RtPayload::Bytes(std::sync::Arc::new(buf))))
        }
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => err_of(io_err("NotFound", p)),
        Err(e) => err_of(io_err("IoFailed", e.to_string())),
    }
}

/// How many bytes a file holds, without reading any of them.
///
/// # Safety: boxed values.
#[no_mangle]
pub unsafe extern "C" fn grasm_file_size(path: *mut RtVal) -> *mut RtVal {
    journaled("file_size", || grasm_file_size_now(path))
}

unsafe fn grasm_file_size_now(path: *mut RtVal) -> *mut RtVal {
    let p = as_str(path).to_string();
    match std::fs::metadata(&p) {
        Ok(m) => ok_of(alloc(RtPayload::I64(m.len() as i64))),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => err_of(io_err("NotFound", p)),
        Err(e) => err_of(io_err("IoFailed", e.to_string())),
    }
}

/// Add to the end of a file, creating it if it is not there.
///
/// A write-ahead log rewritten in full on every record is quadratic, which is
/// the difference between a log and a liability.
///
/// # Safety: boxed values.
#[no_mangle]
pub unsafe extern "C" fn grasm_append_bytes(path: *mut RtVal, data: *mut RtVal) -> *mut RtVal {
    journaled("append_bytes", || grasm_append_bytes_now(path, data))
}

unsafe fn grasm_append_bytes_now(path: *mut RtVal, data: *mut RtVal) -> *mut RtVal {
    use std::io::Write;
    let p = as_str(path).to_string();
    let done = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(&p)
        .and_then(|mut f| f.write_all(as_bytes(data).as_slice()));
    match done {
        Ok(()) => ok_of(alloc(RtPayload::Unit)),
        Err(e) => err_of(io_err("IoFailed", e.to_string())),
    }
}

/// Ask the operating system to put a file on the disk before answering.
///
/// Without this, "the write returned" and "the data survives a power cut" are
/// different claims and a program cannot tell which one it has.
///
/// # Safety: boxed values.
#[no_mangle]
pub unsafe extern "C" fn grasm_sync_file(path: *mut RtVal) -> *mut RtVal {
    journaled("sync_file", || grasm_sync_file_now(path))
}

unsafe fn grasm_sync_file_now(path: *mut RtVal) -> *mut RtVal {
    let p = as_str(path).to_string();
    let done = std::fs::OpenOptions::new()
        .append(true)
        .open(&p)
        .and_then(|f| f.sync_all());
    match done {
        Ok(()) => ok_of(alloc(RtPayload::Unit)),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => err_of(io_err("NotFound", p)),
        Err(e) => err_of(io_err("IoFailed", e.to_string())),
    }
}

/// Move a file to a new name, atomically where the platform allows it.
///
/// This is the primitive a crash-safe commit is built from: write the new
/// state beside the old, sync it, and rename over. A copy-then-remove leaves
/// both files when the power goes out between the two, which is why
/// `fs.move_file` has always said it is not atomic.
///
/// # Safety: boxed values.
#[no_mangle]
pub unsafe extern "C" fn grasm_rename_file(from: *mut RtVal, to: *mut RtVal) -> *mut RtVal {
    journaled("rename_file", || grasm_rename_file_now(from, to))
}

unsafe fn grasm_rename_file_now(from: *mut RtVal, to: *mut RtVal) -> *mut RtVal {
    let a = as_str(from).to_string();
    let b = as_str(to).to_string();
    match std::fs::rename(&a, &b) {
        Ok(()) => ok_of(alloc(RtPayload::Unit)),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => err_of(io_err("NotFound", a)),
        Err(e) => err_of(io_err("IoFailed", e.to_string())),
    }
}

/// # Safety: boxed values.
#[no_mangle]
pub unsafe extern "C" fn grasm_write_bytes(path: *mut RtVal, data: *mut RtVal) -> *mut RtVal {
    journaled("write_bytes", || grasm_write_bytes_now(path, data))
}

unsafe fn grasm_write_bytes_now(path: *mut RtVal, data: *mut RtVal) -> *mut RtVal {
    let p = as_str(path).to_string();
    match std::fs::write(&p, as_bytes(data).as_slice()) {
        // `Unit`, not `I64(0)`. The two are indistinguishable to a compiled
        // build — the static type is `()` and the backend prints it from the
        // type rather than the value — but they are not indistinguishable once
        // the value is written to a journal and handed to another engine, which
        // printed `0` where every direct run prints `()` (F37).
        Ok(()) => ok_of(alloc(RtPayload::Unit)),
        Err(e) => err_of(io_err("IoFailed", e.to_string())),
    }
}

/// Creates the whole path, and succeeds if it already exists ([L-LIB-FS]).
///
/// # Safety: called from compiled Grasm with a boxed value.
#[no_mangle]
pub unsafe extern "C" fn grasm_make_dir(path: *mut RtVal) -> *mut RtVal {
    journaled("make_dir", || grasm_make_dir_now(path))
}

unsafe fn grasm_make_dir_now(path: *mut RtVal) -> *mut RtVal {
    match std::fs::create_dir_all(as_str(path)) {
        // `Unit`, not `I64(0)`. The two are indistinguishable to a compiled
        // build — the static type is `()` and the backend prints it from the
        // type rather than the value — but they are not indistinguishable once
        // the value is written to a journal and handed to another engine, which
        // printed `0` where every direct run prints `()` (F37).
        Ok(()) => ok_of(alloc(RtPayload::Unit)),
        Err(e) => err_of(io_err("IoFailed", e.to_string())),
    }
}

/// # Safety: called from compiled Grasm with a boxed value.
#[no_mangle]
pub unsafe extern "C" fn grasm_remove_file(path: *mut RtVal) -> *mut RtVal {
    journaled("remove_file", || grasm_remove_file_now(path))
}

unsafe fn grasm_remove_file_now(path: *mut RtVal) -> *mut RtVal {
    let p = as_str(path).to_string();
    match std::fs::remove_file(&p) {
        // `Unit`, not `I64(0)`. The two are indistinguishable to a compiled
        // build — the static type is `()` and the backend prints it from the
        // type rather than the value — but they are not indistinguishable once
        // the value is written to a journal and handed to another engine, which
        // printed `0` where every direct run prints `()` (F37).
        Ok(()) => ok_of(alloc(RtPayload::Unit)),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => err_of(io_err("NotFound", p)),
        Err(e) => err_of(io_err("IoFailed", e.to_string())),
    }
}

/// A directory's entries, by name, **sorted**.
///
/// The filesystem's own order differs between machines and between runs, and
/// principle 6 makes determinism a feature. Both engines sort, and identically
/// ([L-LIB-FS]) — an unsorted listing would make any program that prints one
/// untestable and would break the three-engine agreement on the first run.
///
/// # Safety: called from compiled Grasm with a boxed value.
#[no_mangle]
pub unsafe extern "C" fn grasm_list_dir(path: *mut RtVal) -> *mut RtVal {
    journaled("list_dir", || grasm_list_dir_now(path))
}

unsafe fn grasm_list_dir_now(path: *mut RtVal) -> *mut RtVal {
    let p = as_str(path).to_string();
    match std::fs::read_dir(&p) {
        Ok(entries) => {
            let mut names: Vec<String> = entries
                .filter_map(|e| e.ok())
                .map(|e| e.file_name().to_string_lossy().into_owned())
                .collect();
            names.sort();
            let items: Vec<*mut RtVal> = names
                .into_iter()
                .map(|n| alloc(RtPayload::Str(RtStr::new(n))))
                .collect();
            ok_of(alloc(RtPayload::List(items.into())))
        }
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => err_of(io_err("NotFound", p)),
        Err(e) => err_of(io_err("IoFailed", e.to_string())),
    }
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
            // As in `grasm_struct_set_field_at`: the caller's reference is
            // the surplus one when the element is already here (F79).
            if items[i as usize] == val {
                dec(val);
            } else {
                dec(items[i as usize]);
                items[i as usize] = val;
            }
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
    // The clock is the clearest case for this: a program that behaves
    // differently at midnight cannot be debugged without one that says what
    // time it was.
    if let Some(replay::Wire::Int(v)) = replay::gate("now_millis") {
        return v;
    }
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0);
    if replay::recording() {
        replay::put("now_millis", &replay::Wire::Int(now));
    }
    now
}

/// Milliseconds since a fixed moment this process cannot name.
///
/// `now_millis` is the *wall* clock and it moves backwards: a machine
/// correcting itself, a leap second, a virtual machine resuming. Measuring a
/// duration with it gives a negative answer twice a year, and every timeout
/// built on it fires early or late by however far the correction went. This
/// one has no relation to any calendar and only ever increases, which is the
/// only thing a duration can be built on.
#[no_mangle]
pub extern "C" fn grasm_monotonic_millis() -> i64 {
    use std::sync::OnceLock;
    static START: OnceLock<std::time::Instant> = OnceLock::new();
    if let Some(replay::Wire::Int(v)) = replay::gate("monotonic_millis") {
        return v;
    }
    let start = START.get_or_init(std::time::Instant::now);
    let now = start.elapsed().as_millis() as i64;
    if replay::recording() {
        replay::put("monotonic_millis", &replay::Wire::Int(now));
    }
    now
}

/// Wait.
///
/// A replay does not: the recording holds what the program *did*, and
/// re-living its pauses would make a replay take as long as the original run
/// and tell nobody anything.
#[no_mangle]
pub unsafe extern "C" fn grasm_sleep_millis(ms: *mut RtVal) -> *mut RtVal {
    let n = match &(*ms).payload {
        RtPayload::I64(v) => *v,
        _ => 0,
    };
    if replay::gate("sleep_millis").is_some() {
        return alloc(RtPayload::Unit);
    }
    if n > 0 {
        std::thread::sleep(std::time::Duration::from_millis(n as u64));
    }
    if replay::recording() {
        replay::put("sleep_millis", &replay::Wire::Unit);
    }
    alloc(RtPayload::Unit)
}

/// Returns `Option[str]` for an environment variable.
/// # Safety: live runtime pointer.
#[no_mangle]
pub unsafe extern "C" fn grasm_env(name: *mut RtVal) -> *mut RtVal {
    journaled("env", || grasm_env_now(name))
}

unsafe fn grasm_env_now(name: *mut RtVal) -> *mut RtVal {
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
    alloc(RtPayload::List(items.into()))
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

// ---------------------------------------------------------------------------
// The journal, in the runtime's own values
//
// One conversion each way, and every hook is then two lines. The alternative —
// a bespoke encoder at each of the twenty-odd effectful builtins — is where
// the bugs would be: `list_dir` recording its list in a different shape from
// the one it returns is not something a test of `read_file` would ever catch.

/// Names outlive the values that carry them, and a journal holds few of them.
fn interned(text: &str) -> &'static str {
    use std::collections::HashMap;
    use std::sync::{Mutex, OnceLock};
    static NAMES: OnceLock<Mutex<HashMap<String, &'static str>>> = OnceLock::new();
    let mut map = NAMES
        .get_or_init(|| Mutex::new(HashMap::new()))
        .lock()
        .unwrap_or_else(|e| e.into_inner());
    if let Some(s) = map.get(text) {
        return s;
    }
    let leaked: &'static str = Box::leak(text.to_string().into_boxed_str());
    map.insert(text.to_string(), leaked);
    leaked
}

/// # Safety: a live value.
unsafe fn wire_of(v: *mut RtVal) -> replay::Wire {
    if v.is_null() {
        return replay::Wire::Unit;
    }
    let items = |d: &ListData| d.iter().map(|&p| wire_of(p)).collect::<Vec<_>>();
    match &(*v).payload {
        RtPayload::Unit => replay::Wire::Unit,
        RtPayload::I64(n) => replay::Wire::Int(*n),
        RtPayload::F64(n) => replay::Wire::Float(*n),
        RtPayload::Bool(b) => replay::Wire::Bool(*b),
        RtPayload::Str(s) => replay::Wire::Str((**s).to_string()),
        RtPayload::Bytes(b) => replay::Wire::Bytes(b.as_ref().clone()),
        RtPayload::List(d) => replay::Wire::List(items(d)),
        RtPayload::Tuple(d) => replay::Wire::Tuple(items(d)),
        RtPayload::ListScalar { .. } => {
            let n = grasm_list_len(v);
            replay::Wire::List((0..n).map(|i| wire_of(grasm_list_get(v, i))).collect())
        }
        RtPayload::Variant {
            type_name,
            name,
            fields,
        } => replay::Wire::Variant {
            ty: (*type_name).to_string(),
            name: (*name).to_string(),
            fields: fields.iter().map(|&p| wire_of(p)).collect(),
        },
        // A builtin answers with none of these. Reaching here means a hook was
        // put on the wrong call, and guessing an encoding would hide that.
        RtPayload::Range { .. } => replay::Wire::of_unsupported("range"),
        RtPayload::Handler { .. } => replay::Wire::of_unsupported("handler"),
        RtPayload::Env { .. } => replay::Wire::of_unsupported("environment"),
        RtPayload::Ref(_) => replay::Wire::of_unsupported("reference"),
        RtPayload::Map(_) => replay::Wire::of_unsupported("map"),
        RtPayload::Struct { .. } => replay::Wire::of_unsupported("struct"),
        RtPayload::Fn { .. } => replay::Wire::of_unsupported("function"),
    }
}

fn val_of(w: &replay::Wire) -> *mut RtVal {
    let many = |items: &Vec<replay::Wire>| items.iter().map(val_of).collect::<Vec<_>>();
    match w {
        replay::Wire::Unit => alloc(RtPayload::Unit),
        replay::Wire::Int(n) => alloc(RtPayload::I64(*n)),
        replay::Wire::Float(n) => alloc(RtPayload::F64(*n)),
        replay::Wire::Bool(b) => alloc(RtPayload::Bool(*b)),
        replay::Wire::Str(s) => alloc(RtPayload::Str(RtStr::of(s))),
        replay::Wire::Bytes(b) => alloc(RtPayload::Bytes(std::sync::Arc::new(b.clone()))),
        replay::Wire::List(v) => alloc(RtPayload::List(many(v).into())),
        replay::Wire::Tuple(v) => alloc(RtPayload::Tuple(many(v).into())),
        replay::Wire::Variant { ty, name, fields } => alloc(RtPayload::Variant {
            type_name: interned(ty),
            name: interned(name),
            fields: many(fields),
        }),
    }
}

/// Runs `body` unless the journal already knows what it answered.
///
/// On replay the effect does not happen at all — which is the whole reason the
/// wrapper is shaped this way. Re-running a `write_file` to reproduce a bug
/// would damage a disk, and re-running a `tcp_connect` would need the other end
/// of the connection to still be there, which is exactly what it will not be.
///
/// # Safety: `body` returns a live value.
unsafe fn journaled(name: &str, body: impl FnOnce() -> *mut RtVal) -> *mut RtVal {
    if !replay::active() {
        return body();
    }
    if let Some(w) = replay::gate(name) {
        return val_of(&w);
    }
    let out = body();
    replay::put(name, &wire_of(out));
    out
}

#[cfg(test)]
mod small_string_tests {
    use crate::RtStr;

    /// The inline and heap paths must be indistinguishable to every reader.
    #[test]
    fn inline_and_heap_agree() {
        let cases = [
            "",
            "a",
            "abc",
            "0123456789abcde",  // 15 — the last inline size
            "0123456789abcdef", // 16 — first heap
            "a much longer string than fifteen bytes",
            "héllo",
            "🙂",
            "αβγ",
            "naïve café", // non-ASCII, both sides of 15
        ];
        for c in cases {
            let s = RtStr::new(c.to_string());
            assert_eq!(&*s, c, "round trip for {c:?}");
            assert_eq!(s.char_len(), c.chars().count(), "char_len for {c:?}");
            assert_eq!(s.is_flat(), c.is_ascii(), "is_flat for {c:?}");
            for i in 0..c.chars().count() {
                assert_eq!(s.char_at(i), c.chars().nth(i), "char {i} of {c:?}");
            }
        }
    }

    /// The integer path writes digits itself, so it has to agree with the
    /// standard formatting it replaces — including at the boundaries.
    #[test]
    fn of_i64_agrees_with_to_string() {
        let mut cases: Vec<i64> = (-3000..3000).collect();
        cases.extend([
            i64::MIN,
            i64::MIN + 1,
            i64::MAX,
            i64::MAX - 1,
            -99_999_999_999_999,
            99_999_999_999_999, // last inline magnitudes
            -100_000_000_000_000,
            100_000_000_000_000, // first heap magnitudes
            10,
            -10,
            100,
            -100,
            1_000_000_000,
        ]);
        for v in cases {
            assert_eq!(&*RtStr::of_i64(v), v.to_string().as_str(), "of_i64({v})");
        }
    }

    /// Concatenation must agree with the obvious implementation, especially
    /// across the boundary where the result stops fitting inline.
    #[test]
    fn concat_agrees_across_the_inline_boundary() {
        let parts = [
            "",
            "a",
            "abcdefg",
            "0123456789abcde",
            "0123456789abcdef",
            "é",
            "🙂",
        ];
        for a in parts {
            for b in parts {
                let joined = RtStr::concat(a, b);
                let want = format!("{a}{b}");
                assert_eq!(&*joined, want.as_str(), "{a:?} + {b:?}");
                assert_eq!(joined.char_len(), want.chars().count(), "{a:?} + {b:?}");
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Ranges held as values
//
// `for i in 0..n` never builds one of these: the backend compiles that shape
// into a counted loop, which is why it was fast before a range was a value and
// stays fast now. These exist for the other case — a range bound to a name,
// passed to a function, printed, compared — which was a build-time refusal.

/// `a..b`, or `a..=b` when `inclusive` is non-zero.
#[no_mangle]
pub extern "C" fn grasm_range_new(start: i64, end: i64, inclusive: i8) -> *mut RtVal {
    alloc(RtPayload::Range {
        start,
        end,
        inclusive: inclusive != 0,
    })
}

/// The elements, as a scalar list.
///
/// The combinators are answered by building this first, exactly as the
/// interpreter does — so there is one definition of what a range contains and
/// not two. An empty or backwards range gives an empty list rather than an
/// error, which is what iterating one does.
///
/// # Safety: live runtime pointer.
#[no_mangle]
pub unsafe extern "C" fn grasm_range_to_list(r: *mut RtVal) -> *mut RtVal {
    let RtPayload::Range {
        start,
        end,
        inclusive,
    } = &(*r).payload
    else {
        panic!("range_to_list: not a range");
    };
    let (start, end, inclusive) = (*start, *end, *inclusive);
    let last = if inclusive { end } else { end - 1 };
    // Filled directly rather than pushed one at a time.
    //
    // This used to call `grasm_list_push_scalar` per element, so a two-million
    // element range crossed the runtime boundary two million times and paid a
    // copy-on-write check and a payload match on each — better than a third of
    // the sieve benchmark's running time was here rather than in the sieve.
    // The length is known before the first element, so the vector is made once
    // at its final size and written straight into.
    let mut words: Vec<i64> = Vec::new();
    if start <= last {
        // Widened so a range spanning the whole of `i64` does not overflow the
        // count itself, and clamped: a range longer than memory is a bad
        // program, and pushing until the allocator gives up is a worse answer
        // than a big allocation that fails honestly.
        let count = (last as i128 - start as i128 + 1) as u128;
        words.reserve(count.min(u32::MAX as u128 * 4) as usize);
        let mut i = start;
        loop {
            words.push(i);
            // `i64::MAX` as the last element would wrap the counter back to
            // `i64::MIN` and iterate forever, so the end is recognised as the
            // end.
            if i == last {
                break;
            }
            i += 1;
        }
    }
    alloc(RtPayload::ListScalar {
        kind: ScalarKind::I64,
        data: ScalarData::Words(words),
    })
}

/// How many elements it has, without building them.
///
/// # Safety: live runtime pointer.
#[no_mangle]
pub unsafe extern "C" fn grasm_range_len(r: *mut RtVal) -> i64 {
    let RtPayload::Range {
        start,
        end,
        inclusive,
    } = &(*r).payload
    else {
        panic!("range_len: not a range");
    };
    let span = if *inclusive {
        (*end as i128) - (*start as i128) + 1
    } else {
        (*end as i128) - (*start as i128)
    };
    span.max(0).min(i64::MAX as i128) as i64
}

/// The three fields, for a compiled `for` over a range held as a value.
///
/// # Safety: live runtime pointer.
#[no_mangle]
pub unsafe extern "C" fn grasm_range_start(r: *mut RtVal) -> i64 {
    match &(*r).payload {
        RtPayload::Range { start, .. } => *start,
        _ => panic!("range_start: not a range"),
    }
}

/// # Safety: live runtime pointer.
#[no_mangle]
pub unsafe extern "C" fn grasm_range_end(r: *mut RtVal) -> i64 {
    match &(*r).payload {
        RtPayload::Range { end, .. } => *end,
        _ => panic!("range_end: not a range"),
    }
}

/// # Safety: live runtime pointer.
#[no_mangle]
pub unsafe extern "C" fn grasm_range_inclusive(r: *mut RtVal) -> i8 {
    match &(*r).payload {
        RtPayload::Range { inclusive, .. } => i8::from(*inclusive),
        _ => panic!("range_inclusive: not a range"),
    }
}

#[cfg(test)]
mod scalar_kind_tests {
    use super::ScalarKind;

    /// The two directions agree. They are read by different sides of the ABI —
    /// the backend writes a number, the runtime reads one — so a mapping that
    /// drifted would put integers in a list of bools rather than fail to build.
    #[test]
    fn the_number_and_the_kind_round_trip() {
        for k in [
            ScalarKind::I64,
            ScalarKind::F64,
            ScalarKind::Bool,
            ScalarKind::Ref,
        ] {
            assert_eq!(ScalarKind::from_i64(k.to_i64()), k, "{k:?} did not survive");
        }
    }
}

// ---------------------------------------------------------------------------
// Handlers as values
//
// A handler installed directly by `with E handled_by ELiteral{..}` never builds
// one of these: its operations are lifted at compile time and its captures live
// in the frame that ran the `with`, which cannot return while the handler is
// installed. That path is unchanged and costs nothing.
//
// These are for a handler that outlives the expression that wrote it — returned
// from a function, held in a struct or a list, passed around. The captures go
// on the heap and are reference-counted like every other value, so the question
// "is this environment still alive?" has the same answer here as everywhere.

/// An environment of `n` machine words, all zero and none of them references.
#[no_mangle]
pub extern "C" fn grasm_env_new(n: i64) -> *mut RtVal {
    let n = n.max(0) as usize;
    alloc(RtPayload::Env {
        words: vec![0; n],
        boxed: vec![false; n],
    })
}

/// Stores one capture. `is_boxed` says whether the word is a reference, and so
/// whether releasing the environment must release it.
///
/// The caller has already retained a boxed word; this takes that reference.
///
/// # Safety: live runtime pointer, `i` within the length given to `env_new`.
#[no_mangle]
pub unsafe extern "C" fn grasm_env_set(env: *mut RtVal, i: i64, bits: i64, is_boxed: i8) {
    let RtPayload::Env { words, boxed } = &mut (*env).payload else {
        rt_panic("env_set: not an environment");
    };
    let i = i.max(0) as usize;
    if i < words.len() {
        words[i] = bits;
        boxed[i] = is_boxed != 0;
    }
}

/// The address the lifted operation reads its captures from.
///
/// Stable for the life of the environment: the words are allocated once by
/// `env_new` and never grow, so nothing can move them.
///
/// # Safety: live runtime pointer.
#[no_mangle]
pub unsafe extern "C" fn grasm_env_data(env: *mut RtVal) -> *const u8 {
    match &(*env).payload {
        RtPayload::Env { words, .. } => words.as_ptr() as *const u8,
        _ => rt_panic("env_data: not an environment"),
    }
}

/// An empty handler for `effect`.
///
/// # Safety: interned name data.
#[no_mangle]
pub unsafe extern "C" fn grasm_handler_new(name: *const u8, len: u64) -> *mut RtVal {
    alloc(RtPayload::Handler {
        effect: take_str(name, len),
        ops: Vec::new(),
    })
}

/// Adds one operation. The handler takes the environment's reference.
///
/// # Safety: live runtime pointers and interned name data.
#[no_mangle]
pub unsafe extern "C" fn grasm_handler_add(
    h: *mut RtVal,
    name: *const u8,
    len: u64,
    code: *const u8,
    env: *mut RtVal,
) {
    let n = take_str(name, len);
    let RtPayload::Handler { ops, .. } = &mut (*h).payload else {
        rt_panic("handler_add: not a handler");
    };
    ops.push((n, code, env));
}

/// Installs a handler value, so operations performed inside the block resolve
/// to it. Paired with `grasm_handler_pop` exactly as the literal path is.
///
/// The environment addresses handed to the handler stack stay good because the
/// handler itself is a live value for the whole block — the reference the
/// `with` holds is what keeps them alive.
///
/// # Safety: live runtime pointer.
#[no_mangle]
pub unsafe extern "C" fn grasm_handler_install(h: *mut RtVal) {
    let (effect, ops) = match &(*h).payload {
        RtPayload::Handler { effect, ops } => (effect.clone(), ops.clone()),
        _ => rt_panic("handler_install: not a handler"),
    };
    HANDLERS.with(|st| st.borrow_mut().push((effect, Vec::new())));
    for (name, code, env) in ops {
        let data = if env.is_null() {
            std::ptr::null()
        } else {
            grasm_env_data(env)
        };
        HANDLERS.with(|st| {
            if let Some((_, list)) = st.borrow_mut().last_mut() {
                list.push((name.clone(), code, data));
            }
        });
    }
}

#[cfg(test)]
mod maphash_tests {
    use super::maphash::MapHash;
    use std::collections::HashSet;
    use std::hash::{BuildHasher, Hash};

    fn hash_of<T: Hash>(b: &MapHash, v: &T) -> u64 {
        b.hash_one(v)
    }

    /// How many distinct values `n` keys land on, out of `slots`.
    ///
    /// The expected count for a hash that behaves like a random function is
    /// `slots * (1 - e^(-n/slots))`; a hash that clusters comes in far under
    /// it. This is the property a table actually depends on — not secrecy —
    /// and it is the one a mistake in the mixing would break.
    fn spread(pick: impl Fn(u64) -> u64, keys: &[String], b: &MapHash) -> usize {
        keys.iter()
            .map(|k| pick(hash_of(b, k)))
            .collect::<HashSet<_>>()
            .len()
    }

    fn expected(n: f64, slots: f64) -> usize {
        (slots * (1.0 - (-n / slots).exp())) as usize
    }

    #[test]
    fn short_keys_spread_across_buckets() {
        let b = MapHash::default();
        // The shape that matters: short keys sharing a long prefix, which is
        // what a counting loop over words produces.
        let keys: Vec<String> = (0..100_000).map(|i| format!("alpha{i}")).collect();
        let slots = 1u64 << 16;
        // The low bits choose the bucket.
        let low = spread(|h| h & (slots - 1), &keys, &b);
        // The top seven choose the control byte, which is what a probe
        // compares first; clustering there costs a memcmp per collision.
        let top = spread(|h| h >> 57, &keys, &b);
        let want = expected(100_000.0, slots as f64);
        assert!(
            low > want * 99 / 100,
            "low bits cluster: {low} distinct, expected about {want}"
        );
        assert!(top > 120, "top bits cluster: {top} of 128");
    }

    #[test]
    fn near_identical_keys_do_not_collide() {
        let b = MapHash::default();
        // One bit apart, one byte apart, and a length apart.
        let keys: Vec<String> = ["a", "b", "aa", "ab", "ba", "", "aaaaaaaa", "aaaaaaab"]
            .iter()
            .map(|s| (*s).to_string())
            .collect();
        let hashes: HashSet<u64> = keys.iter().map(|k| hash_of(&b, k)).collect();
        assert_eq!(hashes.len(), keys.len(), "distinct keys shared a hash");
    }

    #[test]
    fn integers_spread_too() {
        let b = MapHash::default();
        // Consecutive integers and multiples of a power of two are the two
        // ways an integer-keyed table is usually filled.
        let slots = 1u64 << 12;
        for step in [1i64, 8, 64, 1024] {
            let mut seen = HashSet::new();
            for i in 0..10_000i64 {
                seen.insert(b.hash_one(i * step) & (slots - 1));
            }
            let want = expected(10_000.0, slots as f64);
            assert!(
                seen.len() > want * 98 / 100,
                "step {step} clusters: {} distinct, expected about {want}",
                seen.len()
            );
        }
    }

    #[test]
    fn the_seed_actually_varies_the_hash() {
        // Two tables in one process share a seed, so this checks the seed is
        // used at all rather than that it differs between them.
        let b = MapHash::default();
        let plain = b.hash_one("alpha");
        assert_ne!(plain, 0);
        // The same key in the same process hashes the same, which is what a
        // table needs between a `set` and the `get` that follows it.
        assert_eq!(plain, b.hash_one("alpha"));
    }
}

#[cfg(all(test, feature = "poison"))]
mod poison_tests {
    use super::*;

    /// The detector has to actually detect. Without this the feature could be
    /// silently inert and a gate built on it would report success forever.
    ///
    /// The poisoning itself is what is checked rather than the panic: a
    /// `rt_panic` ends the process rather than unwinding, which is right for a
    /// running program and untestable from here.
    #[test]
    fn a_released_value_is_marked() {
        unsafe {
            let v = alloc(RtPayload::I64(7));
            assert_ne!((*v).rc.load(Relaxed), POISON, "fresh value already marked");
            // One owner, so this releases it.
            dec(v);
            assert_eq!(
                (*v).rc.load(Relaxed),
                POISON,
                "a released box was not marked, so a later use would go unseen"
            );
        }
    }

    #[test]
    fn a_live_value_is_not_marked() {
        unsafe {
            let v = alloc(RtPayload::I64(9));
            inc(v);
            dec(v);
            assert_ne!((*v).rc.load(Relaxed), POISON, "still held, but marked freed");
            assert!(matches!((*v).payload, RtPayload::I64(9)));
            dec(v);
        }
    }
}
