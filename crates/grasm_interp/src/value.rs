//! Runtime values. Compound values use `Rc` + copy-on-write (`Rc::make_mut`),
//! which gives the language's value semantics: assignment is a cheap logical
//! copy, and mutation never aliases.

use std::fmt;
use std::rc::Rc;

use grasm_syntax::ast::{Block, Ident};

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
            Value::Float(v) => {
                if v.fract() == 0.0 && v.is_finite() && v.abs() < 1e15 {
                    write!(f, "{v:.1}")
                } else {
                    write!(f, "{v}")
                }
            }
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
