//! Runtime values. Compound values use `Rc` + copy-on-write (`Rc::make_mut`),
//! which gives the language's value semantics: assignment is a cheap logical
//! copy, and mutation never aliases.

use std::fmt;
use std::rc::Rc;

use grasm_syntax::ast::{Block, Ident};

/// `x` as text with exactly `places` digits after the point, ties **away from
/// zero** — the same rule `round` uses ([L-LIB-ROUND]).
///
/// The twin of `grasm_rt::fixed_text`, kept in step by the differential suite
/// the way `float_text` is.
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

pub fn float_text(v: f64) -> String {
    let s = format!("{v}");
    if v.is_finite() && !s.contains(['.', 'e', 'E']) {
        format!("{s}.0")
    } else {
        s
    }
}

#[derive(Debug, Clone)]
pub enum Value {
    Int(i64),
    Float(f64),
    Bool(bool),
    Str(Rc<GStr>),
    /// A contiguous run of bytes ([L-LIB-BYTES]).
    ///
    /// Stored as `Vec<u8>` rather than as a list of `Value::Int`, because the
    /// promise this type makes — every element is a byte — is exactly what the
    /// representation should carry. It is also eight times smaller.
    Bytes(Rc<Vec<u8>>),
    Unit,
    Range(i64, i64, bool),
    List(Rc<Vec<Value>>),
    /// Sorted map — deterministic iteration order.
    Map(Rc<std::collections::BTreeMap<MapKey, Value>>),
    Tuple(Rc<Vec<Value>>),
    /// Tier 2 — a generational reference: `(slot, generation)`. Copyable, so
    /// it can be stored anywhere; whether it still points at something is a
    /// question for the slab, asked on every read.
    Ref(u32, u32),
    /// Ordered fields, as declared.
    Struct {
        name: Rc<String>,
        fields: Rc<Vec<(String, Value)>>,
    },
    Variant {
        type_name: Rc<String>,
        variant: Rc<String>,
        fields: Rc<Vec<Value>>,
    },
    /// Success branch of a fallible call.
    Ok(Rc<Value>),
    /// Error branch of a fallible call.
    Err(Rc<Value>),
    Closure {
        params: Rc<Vec<Ident>>,
        body: Rc<ClosureBody>,
        env: Rc<Vec<(String, Value)>>,
        /// The module the closure was *written* in.
        ///
        /// A closure's body names things where it appears, not where it is
        /// called: `engine.apply(|| helper.value())` has to find `helper` in
        /// the caller's module, and resolving it in `engine` panicked with
        /// "unknown name `helper`" for a program the compiled binary ran
        /// correctly.
        module: Rc<String>,
        /// Whether this closure's own type is fallible, so a `fail` inside it
        /// is *its* error rather than the enclosing function's.
        fallible: bool,
    },
    /// Reference to a named top-level function.
    FnRef(Rc<String>),
}

#[derive(Debug, Clone)]
pub enum ClosureBody {
    Expr(grasm_syntax::ast::Expr),
    Block(Block),
}

/// Orderable map key (checker restricts keys to these).
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub enum MapKey {
    Bool(bool),
    Int(i64),
    Str(String),
}

impl MapKey {
    pub fn from_value(v: &Value) -> Option<MapKey> {
        match v {
            Value::Int(i) => Some(MapKey::Int(*i)),
            Value::Bool(b) => Some(MapKey::Bool(*b)),
            Value::Str(s) => Some(MapKey::Str(s.as_str().to_string())),
            _ => None,
        }
    }

    pub fn to_value(&self) -> Value {
        match self {
            MapKey::Int(i) => Value::Int(*i),
            MapKey::Bool(b) => Value::Bool(*b),
            MapKey::Str(s) => Value::str(s.clone()),
        }
    }
}

/// A string, with its character count remembered.
///
/// `s.len()` counts *characters*, which meant decoding the whole string every
/// time it was asked: `while i < s.len()` was quadratic, and 400 000
/// characters took 6.7 seconds where the same loop over a list took 0.21. The
/// count never changes for a given string — these are immutable and shared by
/// reference — so it is computed once and kept. It also answers whether the
/// text is ASCII, since that is exactly "as many characters as bytes", which
/// is what makes `code_at(i)` an index rather than a walk.
pub struct GStr {
    s: String,
    /// `u32::MAX` until asked for.
    chars: std::cell::Cell<u32>,
    /// Byte offset of each character, built the first time one is asked for
    /// by index and only when the text is not ASCII.
    ///
    /// Without it `code_at(i)` walked from the start, so a loop reading a
    /// string left to right was quadratic — 400 000 characters took 6.3
    /// seconds the moment one of them was two bytes wide. ASCII needs no
    /// index, because there the offset *is* the index.
    offsets: std::cell::OnceCell<Vec<u32>>,
}

impl GStr {
    pub fn new(s: String) -> GStr {
        GStr {
            s,
            chars: std::cell::Cell::new(u32::MAX),
            offsets: std::cell::OnceCell::new(),
        }
    }

    /// The byte offset of character `i`, in constant time after the first
    /// call. `None` past the end.
    ///
    /// Shared by `code_at` and `substr`: the latter walked from the start on
    /// every call, so an ordinary `substr(i, 1)` scan was quadratic even
    /// though the index was already built and sitting next to it.
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

    /// The number of characters, computed at most once.
    pub fn char_len(&self) -> usize {
        let c = self.chars.get();
        if c != u32::MAX {
            return c as usize;
        }
        let n = self.s.chars().count();
        // A string longer than 4 billion characters is recomputed each time
        // rather than remembered wrongly.
        if n < u32::MAX as usize {
            self.chars.set(n as u32);
        }
        n
    }

    /// Whether every character is one byte, so an index is an index.
    pub fn is_flat(&self) -> bool {
        self.char_len() == self.s.len()
    }

    pub fn as_str(&self) -> &str {
        &self.s
    }
}

impl std::ops::Deref for GStr {
    type Target = str;
    fn deref(&self) -> &str {
        &self.s
    }
}

impl PartialEq for GStr {
    fn eq(&self, other: &GStr) -> bool {
        self.s == other.s
    }
}

impl Eq for GStr {}

impl PartialOrd for GStr {
    fn partial_cmp(&self, other: &GStr) -> Option<std::cmp::Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for GStr {
    fn cmp(&self, other: &GStr) -> std::cmp::Ordering {
        self.s.cmp(&other.s)
    }
}

impl std::fmt::Display for GStr {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.s)
    }
}

/// Debug is the string's own, not this wrapper's.
///
/// The derived one put `GStr { s: "aaa", chars: Cell { value: 3 } }` into a
/// property test's counterexample — a Rust type in a Grasm programmer's
/// terminal.
impl std::fmt::Debug for GStr {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{:?}", self.s)
    }
}

impl Value {
    pub fn str(s: impl Into<String>) -> Value {
        Value::Str(Rc::new(GStr::new(s.into())))
    }

    pub fn none() -> Value {
        Value::Variant {
            type_name: Rc::new("Option".into()),
            variant: Rc::new("None".into()),
            fields: Rc::new(vec![]),
        }
    }

    pub fn some(v: Value) -> Value {
        Value::Variant {
            type_name: Rc::new("Option".into()),
            variant: Rc::new("Some".into()),
            fields: Rc::new(vec![v]),
        }
    }

    pub fn is_none(&self) -> bool {
        matches!(self, Value::Variant { variant, .. } if variant.as_str() == "None")
    }

    /// Structural equality (used by `==` and `assert_eq`).
    pub fn eq_value(&self, other: &Value) -> bool {
        match (self, other) {
            (Value::Int(a), Value::Int(b)) => a == b,
            // Two references are equal when they name the same live slot.
            // Missing this arm made a `ref` unequal to *itself* — and the
            // failure was contagious: any struct, list, tuple or `Option`
            // holding one was never equal to itself either, so `contains`,
            // `assert_eq` and every dedup silently misbehaved, against
            // [E-EQ-STRUCTURAL]. It also disagreed with the compiled backend,
            // which compares the packed words and got this right by accident.
            (Value::Ref(i1, g1), Value::Ref(i2, g2)) => i1 == i2 && g1 == g2,
            (Value::Float(a), Value::Float(b)) => a == b,
            (Value::Bool(a), Value::Bool(b)) => a == b,
            (Value::Str(a), Value::Str(b)) => a == b,
            (Value::Unit, Value::Unit) => true,
            (Value::Range(a1, a2, a3), Value::Range(b1, b2, b3)) => {
                a1 == b1 && a2 == b2 && a3 == b3
            }
            (Value::List(a), Value::List(b)) | (Value::Tuple(a), Value::Tuple(b)) => {
                a.len() == b.len() && a.iter().zip(b.iter()).all(|(x, y)| x.eq_value(y))
            }
            (Value::Map(a), Value::Map(b)) => {
                a.len() == b.len()
                    && a.iter()
                        .zip(b.iter())
                        .all(|((k1, v1), (k2, v2))| k1 == k2 && v1.eq_value(v2))
            }
            (
                Value::Struct {
                    name: n1,
                    fields: f1,
                },
                Value::Struct {
                    name: n2,
                    fields: f2,
                },
            ) => {
                n1 == n2
                    && f1.len() == f2.len()
                    && f1
                        .iter()
                        .zip(f2.iter())
                        .all(|((k1, v1), (k2, v2))| k1 == k2 && v1.eq_value(v2))
            }
            (
                Value::Variant {
                    type_name: t1,
                    variant: v1,
                    fields: f1,
                },
                Value::Variant {
                    type_name: t2,
                    variant: v2,
                    fields: f2,
                },
            ) => {
                t1 == t2
                    && v1 == v2
                    && f1.len() == f2.len()
                    && f1.iter().zip(f2.iter()).all(|(x, y)| x.eq_value(y))
            }
            (Value::Ok(a), Value::Ok(b)) | (Value::Err(a), Value::Err(b)) => a.eq_value(b),
            // Two function values are the same function when they name the
            // same one. Falling through made `dbl == dbl` false and
            // `assert_eq(dbl, dbl)` fail — the one type that was not equal to
            // itself ([E-EQ-STRUCTURAL]).
            (Value::FnRef(a), Value::FnRef(b)) => a == b,
            // A closure is its body and its captures: two values built from
            // the same literal with the same captures are the same closure,
            // and the same value compared with itself always is.
            (
                Value::Closure {
                    body: b1,
                    env: e1,
                    params: p1,
                    ..
                },
                Value::Closure {
                    body: b2,
                    env: e2,
                    params: p2,
                    ..
                },
            ) => {
                Rc::ptr_eq(b1, b2)
                    && p1.len() == p2.len()
                    && e1.len() == e2.len()
                    && e1
                        .iter()
                        .zip(e2.iter())
                        .all(|((k1, v1), (k2, v2))| k1 == k2 && v1.eq_value(v2))
            }
            _ => false,
        }
    }
}

/// Display form used by `print` and string interpolation.
impl fmt::Display for Value {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Value::Int(v) => write!(f, "{v}"),
            // Printing a reference prints the slot it names, not the value —
            // the value may be gone, and reading it to render a message would
            // be the exact use-after-free the tier prevents.
            Value::Ref(i, g) => write!(f, "ref#{i}.{g}"),
            Value::Float(v) => write!(f, "{}", crate::value::float_text(*v)),
            Value::Bool(v) => write!(f, "{v}"),
            Value::Str(s) => write!(f, "{s}"),
            // Hex, two digits per byte, no separator. Not the text it might
            // decode to: a `bytes` is shown as bytes, which is the whole point
            // of it being a different type from `str` ([L-LIB-BYTES]).
            Value::Bytes(b) => {
                write!(f, "0x")?;
                for byte in b.iter() {
                    write!(f, "{byte:02x}")?;
                }
                Ok(())
            }
            Value::Unit => write!(f, "()"),
            Value::Range(a, b, incl) => {
                write!(f, "{a}..{}{b}", if *incl { "=" } else { "" })
            }
            Value::List(items) => {
                write!(f, "[")?;
                for (i, v) in items.iter().enumerate() {
                    if i > 0 {
                        write!(f, ", ")?;
                    }
                    write!(f, "{v}")?;
                }
                write!(f, "]")
            }
            Value::Map(entries) => {
                write!(f, "{{")?;
                for (i, (k, v)) in entries.iter().enumerate() {
                    if i > 0 {
                        write!(f, ", ")?;
                    }
                    write!(f, "{}: {v}", k.to_value())?;
                }
                write!(f, "}}")
            }
            Value::Tuple(items) => {
                write!(f, "(")?;
                for (i, v) in items.iter().enumerate() {
                    if i > 0 {
                        write!(f, ", ")?;
                    }
                    write!(f, "{v}")?;
                }
                write!(f, ")")
            }
            Value::Struct { name, fields } => {
                write!(f, "{name}{{")?;
                for (i, (k, v)) in fields.iter().enumerate() {
                    if i > 0 {
                        write!(f, ", ")?;
                    }
                    write!(f, "{k}: {v}")?;
                }
                write!(f, "}}")
            }
            Value::Variant {
                variant, fields, ..
            } => {
                write!(f, "{variant}")?;
                if !fields.is_empty() {
                    write!(f, "(")?;
                    for (i, v) in fields.iter().enumerate() {
                        if i > 0 {
                            write!(f, ", ")?;
                        }
                        write!(f, "{v}")?;
                    }
                    write!(f, ")")?;
                }
                Ok(())
            }
            Value::Ok(v) => write!(f, "Ok({v})"),
            Value::Err(v) => write!(f, "Err({v})"),
            Value::Closure { .. } => write!(f, "<closure>"),
            Value::FnRef(name) => write!(f, "<fn {name}>"),
        }
    }
}
