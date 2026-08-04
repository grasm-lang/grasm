//! Semantic type representation and the unification engine.

use std::collections::HashMap;
use std::fmt;

pub type TypeDefId = usize;
pub type VarId = u32;

#[derive(Debug, Clone, PartialEq)]
pub enum Type {
    I64,
    F64,
    Bool,
    Str,
    Unit,
    /// `0..n` — iterable of i64.
    Range,
    Tuple(Vec<Type>),
    List(Box<Type>),
    /// `Map[K, V]` — keys restricted to i64/str/bool.
    Map(Box<Type>, Box<Type>),
    /// Struct or sum type instance, e.g. `Option[i64]`.
    Named { id: TypeDefId, args: Vec<Type> },
    /// Closure / function value.
    Fn { params: Vec<Type>, ret: Box<Type> },
    /// `T ! E` — fallible value.
    Fallible(Box<Type>, Box<Type>),
    /// Error union member set, e.g. `(IoError | ParseError)`.
    Union(Vec<Type>),
    /// Inference variable.
    Var(VarId),
    /// Bound generic parameter inside a generic definition body.
    Generic(u32),
    /// Poison type after a reported error; unifies with everything silently.
    Error,
}

#[derive(Debug, Clone, PartialEq)]
pub enum TypeDefKind {
    Struct { fields: Vec<(String, Type)> },
    Sum { variants: Vec<Variant> },
}

#[derive(Debug, Clone, PartialEq)]
pub struct Variant {
    pub name: String,
    /// Field names + types (types may reference `Generic(i)`).
    pub fields: Vec<(String, Type)>,
}

#[derive(Debug, Clone)]
pub struct TypeDef {
    pub name: String,
    pub generics: Vec<String>,
    pub kind: TypeDefKind,
}

/// All named type definitions, with the prelude pre-registered.
pub struct TypeDefs {
    pub defs: Vec<TypeDef>,
    pub by_name: HashMap<String, TypeDefId>,
    pub option_id: TypeDefId,
    pub io_error_id: TypeDefId,
}

impl TypeDefs {
    pub fn new() -> Self {
        let mut td = TypeDefs {
            defs: Vec::new(),
            by_name: HashMap::new(),
            option_id: 0,
            io_error_id: 0,
        };
        // Prelude: type Option[T] = | Some(v: T) | None
        td.option_id = td.insert(TypeDef {
            name: "Option".into(),
            generics: vec!["T".into()],
            kind: TypeDefKind::Sum {
                variants: vec![
                    Variant {
                        name: "Some".into(),
                        fields: vec![("v".into(), Type::Generic(0))],
                    },
                    Variant {
                        name: "None".into(),
                        fields: vec![],
                    },
                ],
            },
        });
        // Prelude: type IoError = | NotFound(path: str) | IoFailed(msg: str)
        td.io_error_id = td.insert(TypeDef {
            name: "IoError".into(),
            generics: vec![],
            kind: TypeDefKind::Sum {
                variants: vec![
                    Variant {
                        name: "NotFound".into(),
                        fields: vec![("path".into(), Type::Str)],
                    },
                    Variant {
                        name: "IoFailed".into(),
                        fields: vec![("msg".into(), Type::Str)],
                    },
                ],
            },
        });
        td
    }

    pub fn insert(&mut self, def: TypeDef) -> TypeDefId {
        let id = self.defs.len();
        self.by_name.insert(def.name.clone(), id);
        self.defs.push(def);
        id
    }

    pub fn get(&self, id: TypeDefId) -> &TypeDef {
        &self.defs[id]
    }

    pub fn option_some(&self, inner: Type) -> Type {
        Type::Named {
            id: self.option_id,
            args: vec![inner],
        }
    }
}

impl Default for TypeDefs {
    fn default() -> Self {
        Self::new()
    }
}

// ---------------------------------------------------------------------------
// Unification
// ---------------------------------------------------------------------------

pub struct Unifier {
    /// Substitution table: var id → resolved type (if any).
    subst: Vec<Option<Type>>,
}

impl Unifier {
    pub fn new() -> Self {
        Unifier { subst: Vec::new() }
    }

    pub fn fresh(&mut self) -> Type {
        let id = self.subst.len() as VarId;
        self.subst.push(None);
        Type::Var(id)
    }

    /// Follows substitutions one level.
    pub fn shallow_resolve(&self, ty: &Type) -> Type {
        let mut t = ty.clone();
        while let Type::Var(v) = t {
            match &self.subst[v as usize] {
                Some(next) => t = next.clone(),
                None => break,
            }
        }
        t
    }

    /// Fully applies the substitution.
    pub fn resolve(&self, ty: &Type) -> Type {
        let t = self.shallow_resolve(ty);
        match t {
            Type::Tuple(items) => Type::Tuple(items.iter().map(|i| self.resolve(i)).collect()),
            Type::List(inner) => Type::List(Box::new(self.resolve(&inner))),
            Type::Map(k, v) => Type::Map(
                Box::new(self.resolve(&k)),
                Box::new(self.resolve(&v)),
            ),
            Type::Named { id, args } => Type::Named {
                id,
                args: args.iter().map(|a| self.resolve(a)).collect(),
            },
            Type::Fn { params, ret } => Type::Fn {
                params: params.iter().map(|p| self.resolve(p)).collect(),
                ret: Box::new(self.resolve(&ret)),
            },
            Type::Fallible(ok, err) => {
                Type::Fallible(Box::new(self.resolve(&ok)), Box::new(self.resolve(&err)))
            }
            Type::Union(ms) => Type::Union(ms.iter().map(|m| self.resolve(m)).collect()),
            other => other,
        }
    }

    fn occurs(&self, v: VarId, ty: &Type) -> bool {
        match self.shallow_resolve(ty) {
            Type::Var(w) => w == v,
            Type::Tuple(items) => items.iter().any(|i| self.occurs(v, i)),
            Type::List(inner) => self.occurs(v, &inner),
            Type::Map(mk, mv) => self.occurs(v, &mk) || self.occurs(v, &mv),
            Type::Named { args, .. } => args.iter().any(|a| self.occurs(v, a)),
            Type::Fn { params, ret } => {
                params.iter().any(|p| self.occurs(v, p)) || self.occurs(v, &ret)
            }
            Type::Fallible(ok, err) => self.occurs(v, &ok) || self.occurs(v, &err),
            Type::Union(ms) => ms.iter().any(|m| self.occurs(v, m)),
            _ => false,
        }
    }

    /// Unify two types. Returns false on mismatch (caller reports).
    pub fn unify(&mut self, a: &Type, b: &Type) -> bool {
        let a = self.shallow_resolve(a);
        let b = self.shallow_resolve(b);
        match (&a, &b) {
            (Type::Error, _) | (_, Type::Error) => true,
            (Type::Var(v), _) => {
                if let Type::Var(w) = b {
                    if w == *v {
                        return true;
                    }
                }
                if self.occurs(*v, &b) {
                    return false;
                }
                self.subst[*v as usize] = Some(b);
                true
            }
            (_, Type::Var(_)) => self.unify(&b, &a),
            (Type::I64, Type::I64)
            | (Type::F64, Type::F64)
            | (Type::Bool, Type::Bool)
            | (Type::Str, Type::Str)
            | (Type::Unit, Type::Unit)
            | (Type::Range, Type::Range) => true,
            (Type::Generic(x), Type::Generic(y)) => x == y,
            (Type::Tuple(xs), Type::Tuple(ys)) => {
                xs.len() == ys.len() && xs.iter().zip(ys).all(|(x, y)| self.unify(x, y))
            }
            (Type::List(x), Type::List(y)) => self.unify(x, y),
            (Type::Map(k1, v1), Type::Map(k2, v2)) => {
                self.unify(k1, k2) && self.unify(v1, v2)
            }
            (Type::Named { id: i1, args: a1 }, Type::Named { id: i2, args: a2 }) => {
                i1 == i2 && a1.len() == a2.len() && a1.iter().zip(a2).all(|(x, y)| self.unify(x, y))
            }
            (
                Type::Fn {
                    params: p1,
                    ret: r1,
                },
                Type::Fn {
                    params: p2,
                    ret: r2,
                },
            ) => {
                p1.len() == p2.len()
                    && p1.iter().zip(p2).all(|(x, y)| self.unify(x, y))
                    && self.unify(r1, r2)
            }
            (Type::Fallible(o1, e1), Type::Fallible(o2, e2)) => {
                self.unify(o1, o2) && self.unify(e1, e2)
            }
            (Type::Union(x), Type::Union(y)) => {
                // Set-equality by name would be nicer; structural for now.
                x.len() == y.len() && x.iter().zip(y).all(|(a, b)| self.unify(a, b))
            }
            _ => false,
        }
    }
}

impl Default for Unifier {
    fn default() -> Self {
        Self::new()
    }
}

/// Substitute `Generic(i)` with `args[i]` — instantiating a generic definition.
pub fn substitute_generics(ty: &Type, args: &[Type]) -> Type {
    match ty {
        Type::Generic(i) => args
            .get(*i as usize)
            .cloned()
            .unwrap_or(Type::Error),
        Type::Tuple(items) => {
            Type::Tuple(items.iter().map(|t| substitute_generics(t, args)).collect())
        }
        Type::List(inner) => Type::List(Box::new(substitute_generics(inner, args))),
        Type::Map(k, v) => Type::Map(
            Box::new(substitute_generics(k, args)),
            Box::new(substitute_generics(v, args)),
        ),
        Type::Named { id, args: targs } => Type::Named {
            id: *id,
            args: targs.iter().map(|t| substitute_generics(t, args)).collect(),
        },
        Type::Fn { params, ret } => Type::Fn {
            params: params.iter().map(|t| substitute_generics(t, args)).collect(),
            ret: Box::new(substitute_generics(ret, args)),
        },
        Type::Fallible(ok, err) => Type::Fallible(
            Box::new(substitute_generics(ok, args)),
            Box::new(substitute_generics(err, args)),
        ),
        Type::Union(ms) => {
            Type::Union(ms.iter().map(|t| substitute_generics(t, args)).collect())
        }
        other => other.clone(),
    }
}

/// Pretty-printer used by diagnostics. Needs the defs table for names.
pub struct TypeDisplay<'a> {
    pub ty: &'a Type,
    pub defs: &'a TypeDefs,
}

impl fmt::Display for TypeDisplay<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write_type(f, self.ty, self.defs)
    }
}

fn write_type(f: &mut fmt::Formatter<'_>, ty: &Type, defs: &TypeDefs) -> fmt::Result {
    match ty {
        Type::I64 => write!(f, "i64"),
        Type::F64 => write!(f, "f64"),
        Type::Bool => write!(f, "bool"),
        Type::Str => write!(f, "str"),
        Type::Unit => write!(f, "()"),
        Type::Range => write!(f, "range"),
        Type::Tuple(items) => {
            write!(f, "(")?;
            for (i, t) in items.iter().enumerate() {
                if i > 0 {
                    write!(f, ", ")?;
                }
                write_type(f, t, defs)?;
            }
            write!(f, ")")
        }
        Type::List(inner) => {
            write!(f, "[")?;
            write_type(f, inner, defs)?;
            write!(f, "]")
        }
        Type::Map(k, v) => {
            write!(f, "Map[")?;
            write_type(f, k, defs)?;
            write!(f, ", ")?;
            write_type(f, v, defs)?;
            write!(f, "]")
        }
        Type::Named { id, args } => {
            write!(f, "{}", defs.get(*id).name)?;
            if !args.is_empty() {
                write!(f, "[")?;
                for (i, a) in args.iter().enumerate() {
                    if i > 0 {
                        write!(f, ", ")?;
                    }
                    write_type(f, a, defs)?;
                }
                write!(f, "]")?;
            }
            Ok(())
        }
        Type::Fn { params, ret } => {
            write!(f, "fn(")?;
            for (i, p) in params.iter().enumerate() {
                if i > 0 {
                    write!(f, ", ")?;
                }
                write_type(f, p, defs)?;
            }
            write!(f, ") -> ")?;
            write_type(f, ret, defs)
        }
        Type::Fallible(ok, err) => {
            write_type(f, ok, defs)?;
            write!(f, " ! ")?;
            write_type(f, err, defs)
        }
        Type::Union(ms) => {
            write!(f, "(")?;
            for (i, m) in ms.iter().enumerate() {
                if i > 0 {
                    write!(f, " | ")?;
                }
                write_type(f, m, defs)?;
            }
            write!(f, ")")
        }
        Type::Var(v) => write!(f, "_{v}"),
        Type::Generic(i) => write!(f, "T{i}"),
        Type::Error => write!(f, "<error>"),
    }
}
