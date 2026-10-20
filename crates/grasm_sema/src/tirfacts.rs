//! What the checker decided, written down for TIR (`docs/tir.md`).
//!
//! The engines re-resolve names at run time and the native backend re-infers
//! types, because the checker hands them only a few rendered side tables. TIR
//! must need neither, so the checker records each decision *as it makes it*:
//! what a name refers to, what a call reaches, how a method call dispatched,
//! and the type of every binding. Nothing here is computed a second time.
//!
//! Recording is off unless a caller asks for TIR, so `grasm check` pays one
//! branch per decision and nothing else.

use std::collections::{BTreeMap, HashMap};

use grasm_syntax::ast::ParamMode;

use crate::types::Type;

/// A span, as the other side tables key it: `(start, end)` in the program's
/// global offset space.
pub type Key = (u32, u32);

/// What a bare name in an expression position refers to.
#[derive(Debug, Clone, PartialEq)]
pub enum NameRes {
    /// A binding in scope. Which one is a matter of lexical scope, which the
    /// emitter follows itself: it is syntax, not inference.
    Local,
    /// A variant of a sum type; the expression's type says which type.
    Variant,
    /// A module constant, qualified `module::NAME`.
    Const(String),
    /// A named function used as a value, qualified, with its type arguments.
    Fn(String, Vec<Type>),
}

/// What a call `f(args)` reaches.
#[derive(Debug, Clone, PartialEq)]
pub enum CallRes {
    /// A built-in free function (`print`, `map`, `read_file`, `Ok`, `fail`, ...).
    Builtin(String),
    /// A declared function (or `extern`), qualified, with type arguments.
    Fn(String, Vec<Type>),
    /// A variant constructor; the call's type says which type.
    Variant,
    /// Calling a function value.
    Value,
}

/// How a method call `recv.name(args)` dispatched.
#[derive(Debug, Clone, PartialEq)]
pub enum MethodRes {
    /// `module.fn(args)`: the receiver names an imported module and is not an
    /// argument. The function and its type arguments are in `calls`.
    Module,
    /// `Effect.op(args)`: an effect operation.
    EffectOp(String, String),
    /// `Type.Variant(args)`.
    Variant,
    /// A method an `impl` provides for a concrete receiver type; the impl's
    /// function is in `calls`.
    Interface,
    /// An interface method on a receiver of a generic type, dispatched
    /// through the named bound: which `impl` runs depends on the type the
    /// generic is instantiated with.
    Bound(String),
    /// A built-in method of the receiver's type.
    Builtin,
    /// `x.f(a)` as `f(x, a)`; the function is in `calls`.
    Ufcs,
    /// Calling a function held in a struct field.
    FieldFn,
}

/// A function's signature, as the checker settled it.
#[derive(Debug, Clone, PartialEq)]
pub struct FnSig {
    pub generics: usize,
    pub bounds: Vec<Vec<String>>,
    pub params: Vec<Type>,
    pub modes: Vec<ParamMode>,
    pub ret: Type,
    pub err: Option<Type>,
    /// The inferred or declared effect row, if the effect pass produced one.
    pub row: Option<Vec<String>>,
    pub is_pub: bool,
    pub is_extern: bool,
}

/// An effect's operations, in declaration order.
#[derive(Debug, Clone, PartialEq)]
pub struct EffectSig {
    pub ops: Vec<(String, Vec<Type>, Type)>,
}

/// Everything TIR needs from the checker. Types are resolved: no inference
/// variable survives into this table once `check` has finished.
#[derive(Debug, Clone, Default)]
pub struct TirFacts {
    /// The type of every expression the checker reached.
    pub types: HashMap<Key, Type>,
    /// The type of every binding, keyed by the span of the name that binds it.
    pub binds: HashMap<Key, Type>,
    /// The type each expression was checked against, used where inference
    /// recorded none (folded into `types` when checking finishes).
    pub checked: HashMap<Key, Type>,
    /// The `T ! E` each expression that gives only the ok half is wrapped
    /// into ([T-OK-ERR-RETURN]), keyed by that expression's span.
    pub wraps: HashMap<Key, Type>,
    pub names: HashMap<Key, NameRes>,
    pub calls: HashMap<Key, CallRes>,
    pub methods: HashMap<Key, MethodRes>,
    /// Every function and `extern`, by qualified name.
    pub fns: BTreeMap<String, FnSig>,
    /// Every module constant, by qualified name.
    pub consts: BTreeMap<String, Type>,
    /// Every effect, by name.
    pub effects: BTreeMap<String, EffectSig>,
    /// `parallel` blocks proved independent (span start), as in `Analysis`.
    pub parallel_ok: std::collections::BTreeSet<u32>,
}
