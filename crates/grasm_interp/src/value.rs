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
    Str(Rc<String>),
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
            Value::Str(s) => Some(MapKey::Str(s.as_ref().clone())),
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

impl Value {
    pub fn str(s: impl Into<String>) -> Value {
        Value::Str(Rc::new(s.into()))
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
