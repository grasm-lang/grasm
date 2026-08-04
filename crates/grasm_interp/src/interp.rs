//! Tree-walking interpreter — the definitional semantics of Grasm-Core.

use std::collections::HashMap;
use std::io::Write;
use std::rc::Rc;

use grasm_syntax::ast::*;

use crate::value::{ClosureBody, Value};

/// Non-local control flow during evaluation.
#[derive(Debug)]
pub enum Flow {
    Return(Value),
    Break,
    Continue,
    /// Propagating error (`try` on an `Err`).
    Err(Value),
    /// Program bug: assert/panic/index-out-of-bounds/…
    Panic(String),
}

type EResult = Result<Value, Flow>;

pub struct Interp<'a, W: Write> {
    fns: HashMap<String, &'a FnDecl>,
    /// sum type name → variant names (arity implied by decl).
    sums: HashMap<String, Vec<(String, usize)>>,
    /// variant name → (owning type, arity); only unambiguous entries.
    variants: HashMap<String, Option<(String, usize)>>,
    scopes: Vec<HashMap<String, Value>>,
    /// Declared effect names (for op-call dispatch).
    effect_names: std::collections::HashSet<String>,
    /// Innermost-last handler stack: (effect name, op name → closure).
    handlers: Vec<(String, HashMap<String, Value>)>,
    pub out: &'a mut W,
    /// Recursion guard.
    depth: u32,
    /// `inout` copy-out values from the last `call_decl` (param index → value).
    pending_writebacks: Vec<(usize, Value)>,
}

const MAX_DEPTH: u32 = 2_000;

impl<'a, W: Write> Interp<'a, W> {
    pub fn new(module: &'a Module, out: &'a mut W) -> Self {
        let mut fns = HashMap::new();
        let mut sums: HashMap<String, Vec<(String, usize)>> = HashMap::new();
        let mut variants: HashMap<String, Option<(String, usize)>> = HashMap::new();
        // Prelude Option.
        sums.insert(
            "Option".into(),
            vec![("Some".into(), 1), ("None".into(), 0)],
        );
        variants.insert("Some".into(), Some(("Option".into(), 1)));
        variants.insert("None".into(), Some(("Option".into(), 0)));
        for decl in &module.decls {
            match decl {
                Decl::Fn(f) => {
                    fns.insert(f.name.name.clone(), f);
                }
                Decl::Type(t) => {
                    if let TypeBody::Sum(vs) = &t.body {
                        let list: Vec<(String, usize)> = vs
                            .iter()
                            .map(|v| (v.name.name.clone(), v.fields.len()))
                            .collect();
                        for v in vs {
                            variants
                                .entry(v.name.name.clone())
                                .and_modify(|e| *e = None) // ambiguous
                                .or_insert(Some((t.name.name.clone(), v.fields.len())));
                        }
                        sums.insert(t.name.name.clone(), list);
                    }
                }
                _ => {}
            }
        }
        let mut effect_names = std::collections::HashSet::new();
        for decl in &module.decls {
            if let Decl::Effect(e) = decl {
                effect_names.insert(e.name.name.clone());
            }
        }
        Interp {
            fns,
            sums,
            variants,
            scopes: Vec::new(),
            effect_names,
            handlers: Vec::new(),
            out,
            depth: 0,
            pending_writebacks: Vec::new(),
        }
    }

    // ------------------------------------------------------------------
    // Entry points
    // ------------------------------------------------------------------

    /// Runs `main`. Returns process exit code.
    pub fn run_main(&mut self) -> i32 {
        let Some(main) = self.fns.get("main").copied() else {
            let _ = writeln!(self.out, "error: no `main` function");
            return 1;
        };
        match self.call_decl(main, vec![]) {
            Ok(_) => 0,
            Err(Flow::Err(e)) => {
                let _ = writeln!(self.out, "error: unhandled error from main: {e}");
                1
            }
            Err(Flow::Panic(msg)) => {
                let _ = writeln!(self.out, "panic: {msg}");
                101
            }
            Err(_) => 1,
        }
    }

    /// Runs every `test` declaration. Returns (passed, failed).
    pub fn run_tests(&mut self, module: &'a Module) -> (u32, u32) {
        let mut passed = 0;
        let mut failed = 0;
        for decl in &module.decls {
            if let Decl::Test(t) = decl {
                self.scopes.push(HashMap::new());
                let result = self.eval_block(&t.body);
                self.scopes.pop();
                match result {
                    Ok(_) | Err(Flow::Return(_)) => {
                        let _ = writeln!(self.out, "test \"{}\" ... ok", t.name);
                        passed += 1;
                    }
                    Err(Flow::Panic(msg)) => {
                        let _ = writeln!(self.out, "test \"{}\" ... FAILED: {msg}", t.name);
                        failed += 1;
                    }
                    Err(Flow::Err(e)) => {
                        let _ = writeln!(self.out, "test \"{}\" ... FAILED: error: {e}", t.name);
                        failed += 1;
                    }
                    Err(_) => {
                        let _ = writeln!(self.out, "test \"{}\" ... FAILED", t.name);
                        failed += 1;
                    }
                }
            }
        }
        (passed, failed)
    }

    // ------------------------------------------------------------------
    // Calls
    // ------------------------------------------------------------------

    fn call_decl(&mut self, f: &'a FnDecl, args: Vec<Value>) -> EResult {
        if self.depth >= MAX_DEPTH {
            return Err(Flow::Panic(format!(
                "stack overflow: recursion deeper than {MAX_DEPTH} calls"
            )));
        }
        self.depth += 1;
        let mut scope = HashMap::new();
        for (p, v) in f.params.iter().zip(args) {
            scope.insert(p.name.name.clone(), v);
        }
        // Fresh frame: callee can't see caller locals.
        let saved = std::mem::take(&mut self.scopes);
        self.scopes.push(scope);
        let result = self.eval_block(&f.body);
        // Copy-out for `inout` params (applied by the caller to its places).
        self.pending_writebacks.clear();
        if result.is_ok() || matches!(result, Err(Flow::Return(_))) {
            for (i, p) in f.params.iter().enumerate() {
                if p.mode == ParamMode::Inout {
                    if let Some(v) = self.scopes[0].get(&p.name.name) {
                        self.pending_writebacks.push((i, v.clone()));
                    }
                }
            }
        }
        self.scopes = saved;
        self.depth -= 1;

        let fallible = f.err.is_some();
        match result {
            Ok(v) => Ok(if fallible { Value::Ok(Rc::new(v)) } else { v }),
            Err(Flow::Return(v)) => Ok(if fallible { Value::Ok(Rc::new(v)) } else { v }),
            Err(Flow::Err(e)) => {
                if fallible {
                    Ok(Value::Err(Rc::new(e)))
                } else {
                    // main-style propagation
                    Err(Flow::Err(e))
                }
            }
            Err(other) => Err(other),
        }
    }

    fn call_value(&mut self, callee: Value, args: Vec<Value>, span_hint: &str) -> EResult {
        match callee {
            Value::Closure { params, body, env } => {
                if self.depth >= MAX_DEPTH {
                    return Err(Flow::Panic("stack overflow in closure call".into()));
                }
                self.depth += 1;
                let mut scope: HashMap<String, Value> =
                    env.iter().cloned().collect();
                for (p, v) in params.iter().zip(args) {
                    scope.insert(p.name.clone(), v);
                }
                let saved = std::mem::take(&mut self.scopes);
                self.scopes.push(scope);
                let r = match &*body {
                    ClosureBody::Expr(e) => self.eval_expr(e),
                    ClosureBody::Block(b) => self.eval_block(b),
                };
                self.scopes = saved;
                self.depth -= 1;
                match r {
                    Err(Flow::Return(v)) => Ok(v),
                    other => other,
                }
            }
            Value::FnRef(name) => {
                let Some(f) = self.fns.get(name.as_str()).copied() else {
                    return Err(Flow::Panic(format!("unknown function `{name}`")));
                };
                self.call_decl(f, args)
            }
            other => Err(Flow::Panic(format!(
                "`{other}` is not callable ({span_hint})"
            ))),
        }
    }

    // ------------------------------------------------------------------
    // Blocks & statements
    // ------------------------------------------------------------------

    fn eval_block(&mut self, b: &Block) -> EResult {
        self.scopes.push(HashMap::new());
        let mut last = Value::Unit;
        for (i, stmt) in b.stmts.iter().enumerate() {
            match self.eval_stmt(stmt) {
                Ok(v) => {
                    last = if i + 1 == b.stmts.len() { v } else { Value::Unit };
                }
                Err(flow) => {
                    self.scopes.pop();
                    return Err(flow);
                }
            }
        }
        self.scopes.pop();
        Ok(last)
    }

    fn eval_stmt(&mut self, s: &Stmt) -> EResult {
        match s {
            Stmt::Let { name, init, .. } => {
                let v = self.eval_expr(init)?;
                self.scopes
                    .last_mut()
                    .unwrap()
                    .insert(name.name.clone(), v);
                Ok(Value::Unit)
            }
            Stmt::Assign {
                target, op, value, ..
            } => {
                let new = self.eval_expr(value)?;
                let final_v = if *op == AssignOp::Assign {
                    new
                } else {
                    let old = self.eval_expr(target)?;
                    let bop = match op {
                        AssignOp::Add => BinaryOp::Add,
                        AssignOp::Sub => BinaryOp::Sub,
                        AssignOp::Mul => BinaryOp::Mul,
                        AssignOp::Div => BinaryOp::Div,
                        AssignOp::Assign => unreachable!(),
                    };
                    self.apply_binary(bop, old, new)?
                };
                self.assign_place(target, final_v)?;
                Ok(Value::Unit)
            }
            Stmt::Expr(e) => self.eval_expr(e),
            Stmt::Return(v, _) => {
                let val = match v {
                    Some(e) => self.eval_expr(e)?,
                    None => Value::Unit,
                };
                Err(Flow::Return(val))
            }
            Stmt::Break(_) => Err(Flow::Break),
            Stmt::Continue(_) => Err(Flow::Continue),
            Stmt::While { cond, body, .. } => {
                loop {
                    match self.eval_expr(cond)? {
                        Value::Bool(true) => {}
                        _ => break,
                    }
                    match self.eval_block(body) {
                        Ok(_) => {}
                        Err(Flow::Break) => break,
                        Err(Flow::Continue) => continue,
                        Err(other) => return Err(other),
                    }
                }
                Ok(Value::Unit)
            }
            Stmt::With {
                effect,
                handler,
                body,
                ..
            } => {
                let h = self.eval_expr(handler)?;
                let Value::Struct { fields, .. } = h else {
                    return Err(Flow::Panic(format!(
                        "handler for effect `{}` must be a handler struct",
                        effect.dotted()
                    )));
                };
                let ops: HashMap<String, Value> =
                    fields.iter().cloned().collect();
                self.handlers.push((effect.dotted(), ops));
                let r = self.eval_block(body);
                self.handlers.pop();
                r?;
                Ok(Value::Unit)
            }
            Stmt::For {
                var, iter, body, ..
            } => {
                let iterable = self.eval_expr(iter)?;
                let items: Vec<Value> = match iterable {
                    Value::Range(a, b, incl) => {
                        let end = if incl { b + 1 } else { b };
                        (a..end).map(Value::Int).collect()
                    }
                    Value::List(items) => items.iter().cloned().collect(),
                    Value::Str(s) => s
                        .chars()
                        .map(|c| Value::str(c.to_string()))
                        .collect(),
                    other => {
                        return Err(Flow::Panic(format!("cannot iterate over `{other}`")))
                    }
                };
                for item in items {
                    self.scopes.push(HashMap::new());
                    self.scopes
                        .last_mut()
                        .unwrap()
                        .insert(var.name.clone(), item);
                    let r = self.eval_block(body);
                    self.scopes.pop();
                    match r {
                        Ok(_) => {}
                        Err(Flow::Break) => break,
                        Err(Flow::Continue) => continue,
                        Err(other) => return Err(other),
                    }
                }
                Ok(Value::Unit)
            }
        }
    }

    /// Applies `inout` copy-out values to the caller's argument places.
    fn apply_writebacks(&mut self, arg_exprs: &[&Expr]) -> Result<(), Flow> {
        let wbs = std::mem::take(&mut self.pending_writebacks);
        for (i, v) in wbs {
            if let Some(e) = arg_exprs.get(i) {
                let is_place = matches!(
                    e,
                    Expr::Ident(_) | Expr::Field { .. } | Expr::Index { .. }
                );
                if is_place {
                    self.assign_place(e, v)?;
                }
            }
        }
        Ok(())
    }

    // ------------------------------------------------------------------
    // Places (assignment targets)
    // ------------------------------------------------------------------

    fn assign_place(&mut self, target: &Expr, v: Value) -> Result<(), Flow> {
        // Collect the access path down to the root identifier.
        enum Seg {
            Field(String),
            Index(usize),
        }
        let mut segs: Vec<Seg> = Vec::new();
        let mut cur = target;
        loop {
            match cur {
                Expr::Field { recv, name, .. } => {
                    segs.push(Seg::Field(name.name.clone()));
                    cur = recv;
                }
                Expr::Index { recv, index, .. } => {
                    let idx = match self.eval_expr(index)? {
                        Value::Int(i) => i,
                        other => {
                            return Err(Flow::Panic(format!("index must be an int, got `{other}`")))
                        }
                    };
                    if idx < 0 {
                        return Err(Flow::Panic(format!("negative index {idx}")));
                    }
                    segs.push(Seg::Index(idx as usize));
                    cur = recv;
                }
                Expr::Ident(id) => {
                    segs.reverse();
                    // Find the binding.
                    let slot = self
                        .scopes
                        .iter_mut()
                        .rev()
                        .find_map(|s| s.get_mut(&id.name));
                    let Some(mut slot) = slot else {
                        return Err(Flow::Panic(format!("unknown variable `{}`", id.name)));
                    };
                    // Descend with copy-on-write.
                    for seg in &segs {
                        match seg {
                            Seg::Field(fname) => match slot {
                                Value::Struct { fields, .. } => {
                                    let fields = Rc::make_mut(fields);
                                    match fields.iter_mut().find(|(k, _)| k == fname) {
                                        Some((_, fv)) => slot = fv,
                                        None => {
                                            return Err(Flow::Panic(format!(
                                                "no field `{fname}`"
                                            )))
                                        }
                                    }
                                }
                                other => {
                                    return Err(Flow::Panic(format!(
                                        "cannot access field `{fname}` of `{other}`"
                                    )))
                                }
                            },
                            Seg::Index(i) => match slot {
                                Value::List(items) => {
                                    let items = Rc::make_mut(items);
                                    let len = items.len();
                                    match items.get_mut(*i) {
                                        Some(iv) => slot = iv,
                                        None => {
                                            return Err(Flow::Panic(format!(
                                                "index {i} out of bounds (len {len})"
                                            )))
                                        }
                                    }
                                }
                                other => {
                                    return Err(Flow::Panic(format!(
                                        "cannot index into `{other}`"
                                    )))
                                }
                            },
                        }
                    }
                    *slot = v;
                    return Ok(());
                }
                other => {
                    return Err(Flow::Panic(format!(
                        "invalid assignment target at {:?}",
                        other.span()
                    )))
                }
            }
        }
    }

    // ------------------------------------------------------------------
    // Expressions
    // ------------------------------------------------------------------

    fn eval_expr(&mut self, e: &Expr) -> EResult {
        match e {
            Expr::Int(v, _) => Ok(Value::Int(*v)),
            Expr::Float(v, _) => Ok(Value::Float(*v)),
            Expr::Bool(v, _) => Ok(Value::Bool(*v)),
            Expr::Str(pieces, _) => {
                let mut s = String::new();
                for p in pieces {
                    match p {
                        StrPiece::Text(t) => s.push_str(t),
                        StrPiece::Expr(inner) => {
                            let v = self.eval_expr(inner)?;
                            s.push_str(&v.to_string());
                        }
                    }
                }
                Ok(Value::str(s))
            }
            Expr::Ident(id) => self.eval_ident(id),
            Expr::List(items, _) => {
                let mut vs = Vec::with_capacity(items.len());
                for i in items {
                    vs.push(self.eval_expr(i)?);
                }
                Ok(Value::List(Rc::new(vs)))
            }
            Expr::Tuple(items, _) => {
                if items.is_empty() {
                    return Ok(Value::Unit);
                }
                let mut vs = Vec::with_capacity(items.len());
                for i in items {
                    vs.push(self.eval_expr(i)?);
                }
                Ok(Value::Tuple(Rc::new(vs)))
            }
            Expr::StructLit { name, fields, .. } => {
                let mut vs = Vec::with_capacity(fields.len());
                for (fname, fexpr) in fields {
                    let v = self.eval_expr(fexpr)?;
                    vs.push((fname.name.clone(), v));
                }
                Ok(Value::Struct {
                    name: Rc::new(name.segments[0].name.clone()),
                    fields: Rc::new(vs),
                })
            }
            Expr::Call { callee, args, .. } => self.eval_call(callee, args),
            Expr::MethodCall {
                recv, name, args, ..
            } => self.eval_method_call(recv, name, args),
            Expr::Field { recv, name, .. } => self.eval_field(recv, name),
            Expr::Index { recv, index, .. } => {
                let r = self.eval_expr(recv)?;
                let i = self.eval_expr(index)?;
                match (r, i) {
                    (Value::List(items), Value::Int(idx)) => {
                        if idx < 0 || idx as usize >= items.len() {
                            Err(Flow::Panic(format!(
                                "index {idx} out of bounds (len {})",
                                items.len()
                            )))
                        } else {
                            Ok(items[idx as usize].clone())
                        }
                    }
                    (r, i) => Err(Flow::Panic(format!("cannot index `{r}` with `{i}`"))),
                }
            }
            Expr::Unary { op, expr, .. } => {
                let v = self.eval_expr(expr)?;
                match (op, v) {
                    (UnaryOp::Neg, Value::Int(i)) => Ok(Value::Int(-i)),
                    (UnaryOp::Neg, Value::Float(f)) => Ok(Value::Float(-f)),
                    (UnaryOp::Not, Value::Bool(b)) => Ok(Value::Bool(!b)),
                    (_, v) => Err(Flow::Panic(format!("invalid unary operand `{v}`"))),
                }
            }
            Expr::Binary { op, lhs, rhs, .. } => {
                // Short-circuit and/or.
                match op {
                    BinaryOp::And => {
                        return match self.eval_expr(lhs)? {
                            Value::Bool(false) => Ok(Value::Bool(false)),
                            _ => self.eval_expr(rhs),
                        };
                    }
                    BinaryOp::Or => {
                        return match self.eval_expr(lhs)? {
                            Value::Bool(true) => Ok(Value::Bool(true)),
                            _ => self.eval_expr(rhs),
                        };
                    }
                    BinaryOp::Coalesce => {
                        let l = self.eval_expr(lhs)?;
                        return match l {
                            Value::Variant {
                                variant, fields, ..
                            } if variant.as_str() == "Some" => Ok(fields[0].clone()),
                            v if v.is_none() => self.eval_expr(rhs),
                            v => Ok(v),
                        };
                    }
                    _ => {}
                }
                let l = self.eval_expr(lhs)?;
                let r = self.eval_expr(rhs)?;
                self.apply_binary(*op, l, r)
            }
            Expr::Range {
                start,
                end,
                inclusive,
                ..
            } => {
                let s = self.eval_expr(start)?;
                let e2 = self.eval_expr(end)?;
                match (s, e2) {
                    (Value::Int(a), Value::Int(b)) => Ok(Value::Range(a, b, *inclusive)),
                    _ => Err(Flow::Panic("range bounds must be ints".into())),
                }
            }
            Expr::If {
                cond,
                then_block,
                else_branch,
                ..
            } => {
                let c = self.eval_expr(cond)?;
                match c {
                    Value::Bool(true) => self.eval_block(then_block),
                    Value::Bool(false) => match else_branch {
                        Some(eb) => match &**eb {
                            ElseBranch::Else(b) => self.eval_block(b),
                            ElseBranch::ElseIf(e) => self.eval_expr(e),
                        },
                        None => Ok(Value::Unit),
                    },
                    other => Err(Flow::Panic(format!("if condition must be bool, got `{other}`"))),
                }
            }
            Expr::Match {
                scrutinee, arms, ..
            } => {
                let v = self.eval_expr(scrutinee)?;
                for arm in arms {
                    let mut bindings = Vec::new();
                    if self.match_pattern(&arm.pattern, &v, &mut bindings) {
                        self.scopes.push(bindings.into_iter().collect());
                        let guard_ok = match &arm.guard {
                            Some(g) => matches!(self.eval_expr(g)?, Value::Bool(true)),
                            None => true,
                        };
                        if guard_ok {
                            let r = self.eval_expr(&arm.body);
                            self.scopes.pop();
                            return r;
                        }
                        self.scopes.pop();
                    }
                }
                Err(Flow::Panic(format!("no match arm matched `{v}`")))
            }
            Expr::Block(b) => self.eval_block(b),
            Expr::Try { expr, .. } => {
                let v = self.eval_expr(expr)?;
                match v {
                    Value::Ok(inner) => Ok((*inner).clone()),
                    Value::Err(e) => Err(Flow::Err((*e).clone())),
                    other => Ok(other),
                }
            }
            Expr::Closure { params, body, .. } => {
                // Capture the current visible environment by value.
                let mut env: Vec<(String, Value)> = Vec::new();
                for scope in &self.scopes {
                    for (k, v) in scope {
                        env.retain(|(ek, _)| ek != k);
                        env.push((k.clone(), v.clone()));
                    }
                }
                let cb = match &**body {
                    Expr::Block(b) => ClosureBody::Block(b.clone()),
                    other => ClosureBody::Expr(other.clone()),
                };
                Ok(Value::Closure {
                    params: Rc::new(params.clone()),
                    body: Rc::new(cb),
                    env: Rc::new(env),
                })
            }
        }
    }

    fn eval_ident(&mut self, id: &Ident) -> EResult {
        for scope in self.scopes.iter().rev() {
            if let Some(v) = scope.get(&id.name) {
                return Ok(v.clone());
            }
        }
        // Nullary variant?
        if let Some(Some((tname, 0))) = self.variants.get(&id.name) {
            return Ok(Value::Variant {
                type_name: Rc::new(tname.clone()),
                variant: Rc::new(id.name.clone()),
                fields: Rc::new(vec![]),
            });
        }
        if self.fns.contains_key(&id.name) {
            return Ok(Value::FnRef(Rc::new(id.name.clone())));
        }
        Err(Flow::Panic(format!("unknown name `{}`", id.name)))
    }

    fn eval_call(&mut self, callee: &Expr, args: &[Expr]) -> EResult {
        if let Expr::Ident(id) = callee {
            let is_local = self.scopes.iter().rev().any(|s| s.contains_key(&id.name));
            if !is_local {
                // Builtins.
                match id.name.as_str() {
                    "print" => {
                        let v = self.eval_expr(&args[0])?;
                        let _ = writeln!(self.out, "{v}");
                        return Ok(Value::Unit);
                    }
                    "assert" => {
                        let v = self.eval_expr(&args[0])?;
                        return match v {
                            Value::Bool(true) => Ok(Value::Unit),
                            _ => Err(Flow::Panic("assertion failed".into())),
                        };
                    }
                    "assert_eq" => {
                        let a = self.eval_expr(&args[0])?;
                        let b = self.eval_expr(&args[1])?;
                        return if a.eq_value(&b) {
                            Ok(Value::Unit)
                        } else {
                            Err(Flow::Panic(format!(
                                "assertion failed: `{a}` != `{b}`"
                            )))
                        };
                    }
                    "panic" => {
                        let v = self.eval_expr(&args[0])?;
                        return Err(Flow::Panic(v.to_string()));
                    }
                    "fail" => {
                        let v = self.eval_expr(&args[0])?;
                        return Err(Flow::Err(v));
                    }
                    _ => {}
                }
                // Variant constructor.
                if let Some(Some((tname, _arity))) = self.variants.get(&id.name).cloned() {
                    let mut vs = Vec::with_capacity(args.len());
                    for a in args {
                        vs.push(self.eval_expr(a)?);
                    }
                    return Ok(Value::Variant {
                        type_name: Rc::new(tname),
                        variant: Rc::new(id.name.clone()),
                        fields: Rc::new(vs),
                    });
                }
                // Named function.
                if let Some(f) = self.fns.get(&id.name).copied() {
                    let mut vs = Vec::with_capacity(args.len());
                    for a in args {
                        vs.push(self.eval_expr(a)?);
                    }
                    let r = self.call_decl(f, vs)?;
                    let arg_refs: Vec<&Expr> = args.iter().collect();
                    self.apply_writebacks(&arg_refs)?;
                    return Ok(r);
                }
            }
        }
        let callee_v = self.eval_expr(callee)?;
        let mut vs = Vec::with_capacity(args.len());
        for a in args {
            vs.push(self.eval_expr(a)?);
        }
        self.call_value(callee_v, vs, "call")
    }

    fn eval_field(&mut self, recv: &Expr, name: &Ident) -> EResult {
        // Qualified nullary variant `Type.Variant`.
        if let Expr::Ident(tid) = recv {
            let is_local = self.scopes.iter().rev().any(|s| s.contains_key(&tid.name));
            if !is_local {
                if let Some(vs) = self.sums.get(&tid.name) {
                    if vs.iter().any(|(vn, _)| vn == &name.name) {
                        return Ok(Value::Variant {
                            type_name: Rc::new(tid.name.clone()),
                            variant: Rc::new(name.name.clone()),
                            fields: Rc::new(vec![]),
                        });
                    }
                }
            }
        }
        let r = self.eval_expr(recv)?;
        match r {
            Value::Struct { fields, .. } => fields
                .iter()
                .find(|(k, _)| k == &name.name)
                .map(|(_, v)| Ok(v.clone()))
                .unwrap_or_else(|| Err(Flow::Panic(format!("no field `{}`", name.name)))),
            other => Err(Flow::Panic(format!(
                "`{other}` has no field `{}`",
                name.name
            ))),
        }
    }

    /// Effect op / qualified variant call / builtin / UFCS method.
    fn eval_method_call(&mut self, recv: &Expr, name: &Ident, args: &[Expr]) -> EResult {
        // Effect operation dispatch: innermost handler wins.
        if let Expr::Ident(tid) = recv {
            let is_local = self.scopes.iter().rev().any(|s| s.contains_key(&tid.name));
            if !is_local && self.effect_names.contains(&tid.name) {
                let handler_fn = self
                    .handlers
                    .iter()
                    .rev()
                    .find(|(e, _)| e == &tid.name)
                    .and_then(|(_, ops)| ops.get(&name.name).cloned());
                let Some(f) = handler_fn else {
                    return Err(Flow::Panic(format!(
                        "unhandled effect `{}.{}` — no handler installed",
                        tid.name, name.name
                    )));
                };
                let mut vs = Vec::with_capacity(args.len());
                for a in args {
                    vs.push(self.eval_expr(a)?);
                }
                return self.call_value(f, vs, "effect op");
            }
        }
        if let Expr::Ident(tid) = recv {
            let is_local = self.scopes.iter().rev().any(|s| s.contains_key(&tid.name));
            if !is_local && self.sums.contains_key(&tid.name) {
                let mut vs = Vec::with_capacity(args.len());
                for a in args {
                    vs.push(self.eval_expr(a)?);
                }
                return Ok(Value::Variant {
                    type_name: Rc::new(tid.name.clone()),
                    variant: Rc::new(name.name.clone()),
                    fields: Rc::new(vs),
                });
            }
        }
        // Mutating list methods need place semantics.
        if matches!(name.name.as_str(), "push" | "pop" | "sort") {
            if let Some(r) = self.try_mutating_method(recv, name, args)? {
                return Ok(r);
            }
        }
        let recv_v = self.eval_expr(recv)?;
        // Try builtin method on the value.
        if let Some(r) = self.builtin_method(recv_v.clone(), name, args)? {
            return Ok(r);
        }
        // UFCS: x.f(a) → f(x, a).
        if let Some(f) = self.fns.get(&name.name).copied() {
            let mut vs = Vec::with_capacity(args.len() + 1);
            vs.push(recv_v);
            for a in args {
                vs.push(self.eval_expr(a)?);
            }
            let r = self.call_decl(f, vs)?;
            let mut arg_refs: Vec<&Expr> = Vec::with_capacity(args.len() + 1);
            arg_refs.push(recv);
            arg_refs.extend(args.iter());
            self.apply_writebacks(&arg_refs)?;
            return Ok(r);
        }
        Err(Flow::Panic(format!(
            "no method `{}` on `{recv_v}`",
            name.name
        )))
    }

    /// push/pop/sort mutate the receiver in place (copy-on-write at the slot).
    fn try_mutating_method(
        &mut self,
        recv: &Expr,
        name: &Ident,
        args: &[Expr],
    ) -> Result<Option<Value>, Flow> {
        // Only applies when the receiver is a place rooted at a variable
        // holding a list.
        let is_place = {
            let mut cur = recv;
            loop {
                match cur {
                    Expr::Field { recv, .. } | Expr::Index { recv, .. } => cur = recv,
                    Expr::Ident(_) => break true,
                    _ => break false,
                }
            }
        };
        if !is_place {
            return Ok(None);
        }
        let current = self.eval_expr(recv)?;
        let Value::List(items) = current else {
            return Ok(None);
        };
        let mut items_vec: Vec<Value> = items.as_ref().clone();
        let result = match name.name.as_str() {
            "push" => {
                let v = self.eval_expr(&args[0])?;
                items_vec.push(v);
                Value::Unit
            }
            "pop" => match items_vec.pop() {
                Some(v) => Value::some(v),
                None => Value::none(),
            },
            "sort" => {
                items_vec.sort_by(|a, b| compare_values(a, b));
                Value::Unit
            }
            _ => return Ok(None),
        };
        self.assign_place(recv, Value::List(Rc::new(items_vec)))?;
        Ok(Some(result))
    }

    fn builtin_method(
        &mut self,
        recv: Value,
        name: &Ident,
        args: &[Expr],
    ) -> Result<Option<Value>, Flow> {
        let v = match (&recv, name.name.as_str()) {
            (Value::List(items), "len") => Value::Int(items.len() as i64),
            (Value::List(items), "get") => {
                let i = self.eval_expr(&args[0])?;
                match i {
                    Value::Int(idx) if idx >= 0 && (idx as usize) < items.len() => {
                        Value::some(items[idx as usize].clone())
                    }
                    _ => Value::none(),
                }
            }
            (Value::List(items), "contains") => {
                let needle = self.eval_expr(&args[0])?;
                Value::Bool(items.iter().any(|x| x.eq_value(&needle)))
            }
            (Value::List(items), "map") => {
                let f = self.eval_expr(&args[0])?;
                let mut out = Vec::with_capacity(items.len());
                for item in items.iter() {
                    out.push(self.call_value(f.clone(), vec![item.clone()], "map")?);
                }
                Value::List(Rc::new(out))
            }
            (Value::List(items), "filter") => {
                let f = self.eval_expr(&args[0])?;
                let mut out = Vec::new();
                for item in items.iter() {
                    if let Value::Bool(true) =
                        self.call_value(f.clone(), vec![item.clone()], "filter")?
                    {
                        out.push(item.clone());
                    }
                }
                Value::List(Rc::new(out))
            }
            (Value::List(items), "sum") => {
                let mut int_sum = 0i64;
                let mut float_sum = 0f64;
                let mut is_float = false;
                for item in items.iter() {
                    match item {
                        Value::Int(i) => int_sum += i,
                        Value::Float(f) => {
                            is_float = true;
                            float_sum += f;
                        }
                        other => {
                            return Err(Flow::Panic(format!("cannot sum `{other}`")))
                        }
                    }
                }
                if is_float {
                    Value::Float(float_sum + int_sum as f64)
                } else {
                    Value::Int(int_sum)
                }
            }
            (Value::List(items), "join") => {
                let sep = self.eval_expr(&args[0])?;
                let parts: Vec<String> = items.iter().map(|v| v.to_string()).collect();
                Value::str(parts.join(&sep.to_string()))
            }
            (Value::List(items), "rev") => {
                let mut v: Vec<Value> = items.as_ref().clone();
                v.reverse();
                Value::List(Rc::new(v))
            }
            (Value::Str(s), "len") => Value::Int(s.chars().count() as i64),
            (Value::Str(s), "contains") => {
                let n = self.eval_expr(&args[0])?;
                Value::Bool(s.contains(&n.to_string()))
            }
            (Value::Str(s), "starts_with") => {
                let n = self.eval_expr(&args[0])?;
                Value::Bool(s.starts_with(&n.to_string()))
            }
            (Value::Str(s), "ends_with") => {
                let n = self.eval_expr(&args[0])?;
                Value::Bool(s.ends_with(&n.to_string()))
            }
            (Value::Str(s), "split") => {
                let sep = self.eval_expr(&args[0])?.to_string();
                let parts: Vec<Value> = if sep.is_empty() {
                    s.chars().map(|c| Value::str(c.to_string())).collect()
                } else {
                    s.split(&sep).map(Value::str).collect()
                };
                Value::List(Rc::new(parts))
            }
            (Value::Str(s), "chars") => Value::List(Rc::new(
                s.chars().map(|c| Value::str(c.to_string())).collect(),
            )),
            (Value::Str(s), "trim") => Value::str(s.trim()),
            (Value::Str(s), "to_upper") => Value::str(s.to_uppercase()),
            (Value::Str(s), "to_lower") => Value::str(s.to_lowercase()),
            (Value::Str(s), "repeat") => {
                let n = self.eval_expr(&args[0])?;
                match n {
                    Value::Int(k) if k >= 0 => Value::str(s.repeat(k as usize)),
                    _ => return Err(Flow::Panic("repeat count must be a non-negative int".into())),
                }
            }
            (Value::Str(s), "parse_int") => match s.trim().parse::<i64>() {
                Ok(v) => Value::some(Value::Int(v)),
                Err(_) => Value::none(),
            },
            (Value::Str(s), "parse_float") => match s.trim().parse::<f64>() {
                Ok(v) => Value::some(Value::Float(v)),
                Err(_) => Value::none(),
            },
            (Value::Int(i), "as_f64") => Value::Float(*i as f64),
            (Value::Int(i), "abs") => Value::Int(i.abs()),
            (Value::Int(i), "to_str") => Value::str(i.to_string()),
            (Value::Int(i), "min") => {
                let o = self.eval_expr(&args[0])?;
                match o {
                    Value::Int(j) => Value::Int((*i).min(j)),
                    _ => return Err(Flow::Panic("min expects an int".into())),
                }
            }
            (Value::Int(i), "max") => {
                let o = self.eval_expr(&args[0])?;
                match o {
                    Value::Int(j) => Value::Int((*i).max(j)),
                    _ => return Err(Flow::Panic("max expects an int".into())),
                }
            }
            (Value::Float(x), "as_i64") => Value::Int(*x as i64),
            (Value::Float(x), "abs") => Value::Float(x.abs()),
            (Value::Float(x), "sqrt") => Value::Float(x.sqrt()),
            (Value::Float(x), "floor") => Value::Float(x.floor()),
            (Value::Float(x), "ceil") => Value::Float(x.ceil()),
            (Value::Float(x), "round") => Value::Float(x.round()),
            (Value::Float(x), "to_str") => Value::str(x.to_string()),
            (Value::Float(x), "min") => {
                let o = self.eval_expr(&args[0])?;
                match o {
                    Value::Float(y) => Value::Float(x.min(y)),
                    _ => return Err(Flow::Panic("min expects a float".into())),
                }
            }
            (Value::Float(x), "max") => {
                let o = self.eval_expr(&args[0])?;
                match o {
                    Value::Float(y) => Value::Float(x.max(y)),
                    _ => return Err(Flow::Panic("max expects a float".into())),
                }
            }
            (Value::Variant { variant, .. }, "is_some") => {
                Value::Bool(variant.as_str() == "Some")
            }
            (Value::Variant { variant, .. }, "is_none") => {
                Value::Bool(variant.as_str() == "None")
            }
            (
                Value::Variant {
                    variant, fields, ..
                },
                "unwrap_or",
            ) => {
                if variant.as_str() == "Some" {
                    fields[0].clone()
                } else {
                    self.eval_expr(&args[0])?
                }
            }
            _ => return Ok(None),
        };
        Ok(Some(v))
    }

    fn apply_binary(&mut self, op: BinaryOp, l: Value, r: Value) -> EResult {
        use BinaryOp::*;
        // Numeric promotion: mixed int/float works on floats.
        let (l, r) = match (l, r) {
            (Value::Int(a), Value::Float(b)) => (Value::Float(a as f64), Value::Float(b)),
            (Value::Float(a), Value::Int(b)) => (Value::Float(a), Value::Float(b as f64)),
            other => other,
        };
        match op {
            Eq => return Ok(Value::Bool(l.eq_value(&r))),
            NotEq => return Ok(Value::Bool(!l.eq_value(&r))),
            _ => {}
        }
        match (&l, &r) {
            (Value::Int(a), Value::Int(b)) => {
                let a = *a;
                let b = *b;
                Ok(match op {
                    Add => Value::Int(a.wrapping_add(b)),
                    Sub => Value::Int(a.wrapping_sub(b)),
                    Mul => Value::Int(a.wrapping_mul(b)),
                    Div => {
                        if b == 0 {
                            return Err(Flow::Panic("division by zero".into()));
                        }
                        Value::Int(a / b)
                    }
                    Rem => {
                        if b == 0 {
                            return Err(Flow::Panic("remainder by zero".into()));
                        }
                        Value::Int(a % b)
                    }
                    Pow => {
                        if b < 0 {
                            return Err(Flow::Panic(
                                "negative integer exponent (use floats)".into(),
                            ));
                        }
                        Value::Int(a.wrapping_pow(b.min(u32::MAX as i64) as u32))
                    }
                    Lt => Value::Bool(a < b),
                    LtEq => Value::Bool(a <= b),
                    Gt => Value::Bool(a > b),
                    GtEq => Value::Bool(a >= b),
                    _ => return Err(Flow::Panic("invalid int operation".into())),
                })
            }
            (Value::Float(a), Value::Float(b)) => {
                let a = *a;
                let b = *b;
                Ok(match op {
                    Add => Value::Float(a + b),
                    Sub => Value::Float(a - b),
                    Mul => Value::Float(a * b),
                    Div => Value::Float(a / b),
                    Rem => Value::Float(a % b),
                    Pow => Value::Float(a.powf(b)),
                    Lt => Value::Bool(a < b),
                    LtEq => Value::Bool(a <= b),
                    Gt => Value::Bool(a > b),
                    GtEq => Value::Bool(a >= b),
                    _ => return Err(Flow::Panic("invalid float operation".into())),
                })
            }
            (Value::Str(a), Value::Str(b)) => Ok(match op {
                Add => Value::str(format!("{a}{b}")),
                Lt => Value::Bool(a < b),
                LtEq => Value::Bool(a <= b),
                Gt => Value::Bool(a > b),
                GtEq => Value::Bool(a >= b),
                _ => return Err(Flow::Panic("invalid string operation".into())),
            }),
            _ => Err(Flow::Panic(format!(
                "invalid operands `{l}` and `{r}`"
            ))),
        }
    }

    // ------------------------------------------------------------------
    // Pattern matching
    // ------------------------------------------------------------------

    fn match_pattern(
        &self,
        pat: &Pattern,
        v: &Value,
        bindings: &mut Vec<(String, Value)>,
    ) -> bool {
        match pat {
            Pattern::Wildcard(_) => true,
            Pattern::Int(p, _) => matches!(v, Value::Int(x) if x == p),
            Pattern::Bool(p, _) => matches!(v, Value::Bool(x) if x == p),
            Pattern::Str(p, _) => matches!(v, Value::Str(x) if x.as_str() == p),
            Pattern::Bind(id) => {
                // Nullary-variant name check (mirrors the typechecker).
                if let Value::Variant {
                    type_name, variant, ..
                } = v
                {
                    if let Some(vs) = self.sums.get(type_name.as_str()) {
                        if vs.iter().any(|(vn, _)| vn == &id.name) {
                            return variant.as_str() == id.name;
                        }
                    }
                }
                bindings.push((id.name.clone(), v.clone()));
                true
            }
            Pattern::Variant { path, fields, .. } => {
                let want = &path.segments.last().unwrap().name;
                match v {
                    Value::Ok(inner) => {
                        want == "Ok"
                            && fields.len() == 1
                            && self.match_pattern(&fields[0], inner, bindings)
                    }
                    Value::Err(inner) => {
                        want == "Err"
                            && fields.len() == 1
                            && self.match_pattern(&fields[0], inner, bindings)
                    }
                    Value::Variant {
                        variant,
                        fields: vfields,
                        ..
                    } => {
                        variant.as_str() == want
                            && fields.len() == vfields.len()
                            && fields
                                .iter()
                                .zip(vfields.iter())
                                .all(|(p, fv)| self.match_pattern(p, fv, bindings))
                    }
                    _ => false,
                }
            }
            Pattern::Tuple(items, _) => match v {
                Value::Tuple(vs) => {
                    items.len() == vs.len()
                        && items
                            .iter()
                            .zip(vs.iter())
                            .all(|(p, pv)| self.match_pattern(p, pv, bindings))
                }
                _ => false,
            },
        }
    }
}

fn compare_values(a: &Value, b: &Value) -> std::cmp::Ordering {
    use std::cmp::Ordering;
    match (a, b) {
        (Value::Int(x), Value::Int(y)) => x.cmp(y),
        (Value::Float(x), Value::Float(y)) => x.partial_cmp(y).unwrap_or(Ordering::Equal),
        (Value::Str(x), Value::Str(y)) => x.cmp(y),
        _ => Ordering::Equal,
    }
}
