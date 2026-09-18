//! The record-and-replay journal, in the interpreter's own values.
//!
//! The interpreter shares the *journal* with the compiled engines and none of
//! the implementations — it has its own of every builtin. That is what makes
//! the useful case work: a failure recorded in the native build, replayed under
//! `grasm run`, where there is an interpreter to step through.
//!
//! One conversion each way, so a hook is two lines and cannot encode a value
//! in a different shape from the one it returns.

use crate::value::{GStr, Value};
use grasm_journal::{self as replay, Wire};
use std::rc::Rc;

/// The interpreter carries `Ok`/`Err` as their own variants and the runtime
/// carries them as `Variant { "!" , .. }`. The wire uses the runtime's shape,
/// because the wire is what both must agree on and one of them had to give.
pub fn to_wire(v: &Value) -> Wire {
    let many = |items: &[Value]| items.iter().map(to_wire).collect::<Vec<_>>();
    match v {
        Value::Unit => Wire::Unit,
        Value::Int(n) => Wire::Int(*n),
        Value::Float(n) => Wire::Float(*n),
        Value::Bool(b) => Wire::Bool(*b),
        Value::Str(s) => Wire::Str(s.as_str().to_string()),
        Value::Bytes(b) => Wire::Bytes(b.as_ref().clone()),
        Value::List(items) => Wire::List(many(items)),
        Value::Tuple(items) => Wire::Tuple(many(items)),
        Value::Ok(inner) => Wire::ok(to_wire(inner)),
        Value::Err(inner) => Wire::err(to_wire(inner)),
        Value::Variant {
            type_name,
            variant,
            fields,
        } => Wire::Variant {
            ty: type_name.as_str().to_string(),
            name: variant.as_str().to_string(),
            fields: many(fields),
        },
        // No builtin answers with one of these. Recording something else in
        // its place would make the replay wrong rather than absent.
        Value::Range(..) => Wire::of_unsupported("range"),
        Value::Map(_) => Wire::of_unsupported("map"),
        Value::Ref(..) => Wire::of_unsupported("reference"),
        Value::Struct { .. } => Wire::of_unsupported("struct"),
        Value::Closure { .. } => Wire::of_unsupported("closure"),
        Value::FnRef(_) => Wire::of_unsupported("function"),
    }
}

pub fn from_wire(w: &Wire) -> Value {
    let many = |items: &Vec<Wire>| Rc::new(items.iter().map(from_wire).collect::<Vec<_>>());
    match w {
        Wire::Unit => Value::Unit,
        Wire::Int(n) => Value::Int(*n),
        Wire::Float(n) => Value::Float(*n),
        Wire::Bool(b) => Value::Bool(*b),
        Wire::Str(s) => Value::Str(Rc::new(GStr::new(s.clone()))),
        Wire::Bytes(b) => Value::Bytes(Rc::new(b.clone())),
        Wire::List(items) => Value::List(many(items)),
        Wire::Tuple(items) => Value::Tuple(many(items)),
        Wire::Variant { ty, name, fields } if ty == "!" && name == "Ok" => {
            Value::Ok(Rc::new(fields.first().map_or(Value::Unit, from_wire)))
        }
        Wire::Variant { ty, name, fields } if ty == "!" && name == "Err" => {
            Value::Err(Rc::new(fields.first().map_or(Value::Unit, from_wire)))
        }
        Wire::Variant { ty, name, fields } => Value::Variant {
            type_name: Rc::new(ty.clone()),
            variant: Rc::new(name.clone()),
            fields: many(fields),
        },
    }
}

/// Runs `body` unless the journal already knows what it answered.
///
/// On replay the effect does not happen at all: re-running a `write_file` to
/// reproduce a bug would damage a disk, and re-running a `tcp_connect` would
/// need the other end of the connection to still be there, which is exactly
/// what it will not be.
pub fn journaled(name: &str, body: impl FnOnce() -> Value) -> Value {
    if !replay::active() {
        return body();
    }
    if let Some(w) = replay::gate(name) {
        return from_wire(&w);
    }
    let out = body();
    replay::put(name, &to_wire(&out));
    out
}

/// The recorded answer for `name`, when replaying.
///
/// Called *after* a builtin's arguments are evaluated and *before* its effect,
/// because an argument expression is ordinary Grasm code whose own effects are
/// journaled in their own right. Gating earlier would skip them and put the
/// journal out of step with the program.
pub fn gated(name: &str) -> Option<Value> {
    replay::gate(name).as_ref().map(from_wire)
}

/// Notes what a builtin answered, and hands the answer straight back so a
/// `return` can be wrapped in place.
pub fn recorded(name: &str, v: Value) -> Value {
    if replay::recording() {
        replay::put(name, &to_wire(&v));
    }
    v
}
