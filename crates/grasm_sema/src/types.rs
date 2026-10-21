//! Semantic type representation and the unification engine.

use std::collections::HashMap;
use std::fmt;

pub type TypeDefId = usize;
pub type VarId = u32;

/// A fixed-width integer type other than `i64` ([T-INT-FIXED]).
///
/// Each is its own type: a `u8` is not an `i64`, and the two do not mix
/// without a conversion. Every engine holds the value in 64 bits -- the true
/// value, sign- or zero-extended -- and wraps the result of each operation to
/// the type's width, the way `i64` wraps to 64 ([E-ARITH-INT]). That is what
/// makes `255 + 1` a `u8` `0` rather than the `i64` `256` these names used to
/// stand for.
///
/// `u64` is the one whose 64 bits are not its value read as an `i64`: from
/// 2^63 up, the same bits an `i64` reads as negative. So it is the one width
/// the engines cannot treat as an `i64` with a wrap after each operation; each
/// carries it as a kind of value of its own ([T-U64]), and the range helpers
/// below answer in `i128`, where every width's range fits.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum IntKind {
    I8,
    I16,
    I32,
    U8,
    U16,
    U32,
    U64,
}

impl IntKind {
    pub const ALL: [IntKind; 7] = [
        IntKind::I8,
        IntKind::I16,
        IntKind::I32,
        IntKind::U8,
        IntKind::U16,
        IntKind::U32,
        IntKind::U64,
    ];

    pub fn name(self) -> &'static str {
        match self {
            IntKind::I8 => "i8",
            IntKind::I16 => "i16",
            IntKind::I32 => "i32",
            IntKind::U8 => "u8",
            IntKind::U16 => "u16",
            IntKind::U32 => "u32",
            IntKind::U64 => "u64",
        }
    }

    pub fn from_name(name: &str) -> Option<IntKind> {
        IntKind::ALL.into_iter().find(|k| k.name() == name)
    }

    pub fn bits(self) -> u32 {
        match self {
            IntKind::I8 | IntKind::U8 => 8,
            IntKind::I16 | IntKind::U16 => 16,
            IntKind::I32 | IntKind::U32 => 32,
            IntKind::U64 => 64,
        }
    }

    pub fn signed(self) -> bool {
        matches!(self, IntKind::I8 | IntKind::I16 | IntKind::I32)
    }

    pub fn min(self) -> i128 {
        if self.signed() {
            -(1i128 << (self.bits() - 1))
        } else {
            0
        }
    }

    pub fn max(self) -> i128 {
        if self.signed() {
            (1i128 << (self.bits() - 1)) - 1
        } else {
            (1i128 << self.bits()) - 1
        }
    }

    /// `v` wrapped to this width: its low `bits` bits, sign-extended for a
    /// signed type. Two's complement, as `i64` arithmetic already is. A `u64`
    /// keeps all 64 bits as they are.
    pub fn wrap(self, v: i64) -> i64 {
        let shift = 64 - self.bits();
        if self.signed() {
            (v << shift) >> shift
        } else {
            ((v as u64) << shift >> shift) as i64
        }
    }

    /// The number a value of this width, held in 64 bits, stands for: the
    /// bits read as an `i64` for every width but `u64`, whose top half is
    /// 2^63 and up rather than negative.
    pub fn value(self, bits: i64) -> i128 {
        if self == IntKind::U64 {
            i128::from(bits as u64)
        } else {
            i128::from(bits)
        }
    }

    pub fn fits(self, v: i128) -> bool {
        v >= self.min() && v <= self.max()
    }

    /// A one-byte tag for the engines, which keep a map from an operation's
    /// span to the width it wraps to.
    pub fn tag(self) -> u8 {
        self as u8
    }

    pub fn from_tag(t: u8) -> Option<IntKind> {
        IntKind::ALL.get(t as usize).copied()
    }
}

#[derive(Debug, Clone, PartialEq)]
pub enum Type {
    I64,
    /// `i8`, `i16`, `i32`, `u8`, `u16`, `u32` ([T-INT-FIXED]) and `u64`
    /// ([T-U64]).
    Int(IntKind),
    F64,
    Bool,
    Str,
    /// A contiguous run of bytes — `bytes`.
    ///
    /// Distinct from `[i64]` on purpose. A list of integers cannot promise its
    /// elements are in 0..255, cannot be stored one byte per element, and
    /// cannot be handed to anything that wants a buffer. The whole reason this
    /// type exists is the promise: a `bytes` is bytes.
    ///
    /// It is also the type `str` deliberately is not. `str` is text and
    /// re-encodes anything above 127 as UTF-8, which is correct for text and
    /// fatal for a BMP header or a machine instruction ([L-LIB-BYTES]).
    Bytes,
    /// `StrBuilder` and `ByteBuilder` ([L-LIB-BUILDER]): text and bytes that
    /// grow at the end in amortized constant time, as values like any other.
    StrBuf,
    ByteBuf,
    /// `addr` ([T-ADDR]): an address in raw memory, one machine word. Safe
    /// code may hold one; every operation on it is `unsafe` and performs
    /// `mem` ([T-MEM-UNSAFE]).
    Addr,
    Unit,
    /// `0..n` — iterable of i64.
    Range,
    Tuple(Vec<Type>),
    List(Box<Type>),
    /// `ref T` — Tier 2. A generational reference: a slot in the shared heap
    /// plus the generation it held when the reference was taken. Copyable, so
    /// it can be stored in a struct or a list, which is the point — that is
    /// the graph shape Tier 0's second-class references cannot express.
    Ref(Box<Type>),
    /// `Map[K, V]` — keys restricted to i64/str/bool.
    Map(Box<Type>, Box<Type>),
    /// Struct or sum type instance, e.g. `Option[i64]`.
    Named {
        id: TypeDefId,
        args: Vec<Type>,
    },
    /// Closure / function value.
    /// A function value's type, **including its effect row**.
    ///
    /// The row used to live only in the syntax a reader wrote, never in the
    /// type — so `fn(i64) -> i64` and `fn(i64) -> i64 / {io}` were the same
    /// type here, and the only thing comparing them was one hand-written check
    /// on direct call arguments. Every other way of turning a named function
    /// into a value — a return type, a type alias, a struct field, a list
    /// element, a map value — dropped the row silently, which let a function
    /// that prints reach a `/ {}` row, a `par_map`, and a package declaring
    /// `capabilities = []` ([T-FN-TYPE-ROW]).
    Fn {
        params: Vec<Type>,
        ret: Box<Type>,
        /// The written effect row. `None` is *unconstrained* — the shape the
        /// built-in higher-order methods want, since `xs.map(|x| print(x))`
        /// is fine. `Some(set)` is a row someone wrote down, and `Some({})` is
        /// the empty row a bare `fn(i64) -> i64` means ([T-FN-TYPE-ROW]).
        row: Option<std::collections::BTreeSet<String>>,
    },
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
    /// The prelude's `Chan[T]`, which `chan_new()` makes (docs/concurrency.md).
    pub chan_id: TypeDefId,
}

/// The prelude's `IoError`, once, so that every engine reads the same list.
///
/// The native backend used to have no declaration for it at all — `IoError` is
/// not in anyone's source, so nothing put it in the backend's table — and
/// matching its variants was a build-time refusal. Copying the list into the
/// backend would have fixed that program and left the next divergence to
/// whoever adds a third variant.
///
/// Every field is `str`; if that stops being true this becomes a list of pairs.
pub const IO_ERROR_VARIANTS: &[(&str, &str)] = &[("NotFound", "path"), ("IoFailed", "msg")];

impl TypeDefs {
    pub fn new() -> Self {
        let mut td = TypeDefs {
            defs: Vec::new(),
            by_name: HashMap::new(),
            option_id: 0,
            io_error_id: 0,
            chan_id: 0,
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
                variants: IO_ERROR_VARIANTS
                    .iter()
                    .map(|(name, field)| Variant {
                        name: (*name).into(),
                        fields: vec![((*field).into(), Type::Str)],
                    })
                    .collect(),
            },
        });
        // Prelude: an opaque `Chan[T]`. It has no fields a program can read;
        // `chan_send`, `chan_recv` and `chan_close` are the only way in.
        td.chan_id = td.insert(TypeDef {
            name: "Chan".into(),
            generics: vec!["T".into()],
            kind: TypeDefKind::Struct { fields: vec![] },
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
            Type::Map(k, v) => Type::Map(Box::new(self.resolve(&k)), Box::new(self.resolve(&v))),
            Type::Named { id, args } => Type::Named {
                id,
                args: args.iter().map(|a| self.resolve(a)).collect(),
            },
            Type::Ref(inner) => Type::Ref(Box::new(self.resolve(&inner))),
            Type::Fn { params, ret, row } => Type::Fn {
                params: params.iter().map(|p| self.resolve(p)).collect(),
                ret: Box::new(self.resolve(&ret)),
                row,
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
            Type::Fn { params, ret, .. } => {
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
            (Type::Int(a), Type::Int(b)) => a == b,
            (Type::I64, Type::I64)
            | (Type::F64, Type::F64)
            | (Type::Bool, Type::Bool)
            | (Type::Str, Type::Str)
            | (Type::Bytes, Type::Bytes)
            | (Type::StrBuf, Type::StrBuf)
            | (Type::ByteBuf, Type::ByteBuf)
            | (Type::Addr, Type::Addr)
            | (Type::Unit, Type::Unit)
            | (Type::Range, Type::Range) => true,
            (Type::Generic(x), Type::Generic(y)) => x == y,
            (Type::Tuple(xs), Type::Tuple(ys)) => {
                xs.len() == ys.len() && xs.iter().zip(ys).all(|(x, y)| self.unify(x, y))
            }
            (Type::List(x), Type::List(y)) => self.unify(x, y),
            (Type::Map(k1, v1), Type::Map(k2, v2)) => self.unify(k1, k2) && self.unify(v1, v2),
            (Type::Named { id: i1, args: a1 }, Type::Named { id: i2, args: a2 }) => {
                i1 == i2 && a1.len() == a2.len() && a1.iter().zip(a2).all(|(x, y)| self.unify(x, y))
            }
            (Type::Ref(a), Type::Ref(b)) => self.unify(a, b),
            (
                Type::Fn {
                    params: p1,
                    ret: r1,
                    ..
                },
                Type::Fn {
                    params: p2,
                    ret: r2,
                    ..
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
        Type::Generic(i) => args.get(*i as usize).cloned().unwrap_or(Type::Error),
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
        Type::Ref(inner) => Type::Ref(Box::new(substitute_generics(inner, args))),
        Type::Fn { params, ret, row } => Type::Fn {
            params: params
                .iter()
                .map(|t| substitute_generics(t, args))
                .collect(),
            ret: Box::new(substitute_generics(ret, args)),
            row: row.clone(),
        },
        Type::Fallible(ok, err) => Type::Fallible(
            Box::new(substitute_generics(ok, args)),
            Box::new(substitute_generics(err, args)),
        ),
        Type::Union(ms) => Type::Union(ms.iter().map(|t| substitute_generics(t, args)).collect()),
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
        Type::Ref(inner) => {
            write!(f, "ref ")?;
            write_type(f, inner, defs)
        }
        Type::I64 => write!(f, "i64"),
        Type::Int(k) => write!(f, "{}", k.name()),
        Type::F64 => write!(f, "f64"),
        Type::Bool => write!(f, "bool"),
        Type::Str => write!(f, "str"),
        Type::Bytes => write!(f, "bytes"),
        Type::StrBuf => write!(f, "StrBuilder"),
        Type::ByteBuf => write!(f, "ByteBuilder"),
        Type::Addr => write!(f, "addr"),
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
        Type::Fn { params, ret, .. } => {
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
        // An unsolved inference variable prints as `_`, the way one is
        // written. Numbering them leaked the checker's own counter into
        // messages — `_3 ! Bad` names a type nobody can write, and the number
        // changes when unrelated code above it does.
        Type::Var(_) => write!(f, "_"),
        Type::Generic(i) => write!(f, "T{i}"),
        Type::Error => write!(f, "<error>"),
    }
}
