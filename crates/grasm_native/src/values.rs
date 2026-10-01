//! Which struct types can be compiled as values (docs/values.md).
//!
//! A struct type is value-eligible when nothing generic ever has to be told
//! about one of its values: every use, anywhere in the program, is building
//! one, reading a field, passing or returning one, or keeping a list of them
//! and pushing to, measuring, indexing or iterating it. Everything else keeps
//! the type boxed, exactly as before. The test is conservative on purpose --
//! an expression whose position this file does not recognise disqualifies its
//! type -- so a mistake here costs speed, never correctness.

use std::collections::{BTreeSet, HashMap, HashSet};

use grasm_syntax::ast::{
    Block, Decl, ElseBranch, Expr, Module, ParamMode, Stmt, StrPiece, TypeBody, TypeExpr,
};

/// The struct types of `modules` that can be values, given the checker's
/// resolved type of every expression (by span).
pub fn eligible(
    modules: &[&Module],
    types: &HashMap<(u32, u32), String>,
) -> BTreeSet<String> {
    let mut candidates: BTreeSet<String> = BTreeSet::new();
    for m in modules {
        for d in &m.decls {
            if let Decl::Type(t) = d {
                // A linear value's moves are checked, not counted; giving it
                // value codegen means getting `own` exactly right, later.
                if t.generics.is_empty() && !t.is_linear && matches!(t.body, TypeBody::Struct(_)) {
                    candidates.insert(t.name.name.clone());
                }
            }
        }
    }
    let mut out = Walk {
        candidates: candidates.clone(),
        types,
        bad: HashSet::new(),
        fns: HashMap::new(),
        in_closure: 0,
    };
    // Declarations: where a candidate may appear in a type, and where not.
    for m in modules {
        for d in &m.decls {
            match d {
                Decl::Type(t) => match &t.body {
                    TypeBody::Struct(fields) => {
                        for f in fields {
                            // A list of them is a pointer like any list; the
                            // value itself held in another struct would need
                            // that struct to hold it inline, which is a later
                            // stage.
                            out.type_uses(&f.ty, false);
                        }
                    }
                    TypeBody::Sum(vs) => {
                        for v in vs {
                            for f in &v.fields {
                                out.type_uses(&f.ty, false);
                            }
                        }
                    }
                    TypeBody::Alias(te) => out.type_uses(te, false),
                },
                Decl::Fn(f) => {
                    if f.generics.is_empty() {
                        out.fns
                            .insert(f.name.name.clone(), f.params.iter().map(|p| p.mode).collect());
                    }
                    for p in &f.params {
                        out.type_uses(&p.ty, p.mode == ParamMode::Let);
                    }
                    // A fallible function's result travels inside an `Ok`
                    // variant, which is a box.
                    if let Some(r) = &f.ret {
                        out.type_uses(r, f.err.is_none());
                    }
                    if let Some(e) = &f.err {
                        out.type_uses(e, false);
                    }
                }
                Decl::Impl(i) => {
                    // A type with an `impl` is reached through an interface,
                    // which is generic by construction.
                    out.bad.insert(i.type_name.clone());
                }
                _ => {}
            }
        }
    }
    for m in modules {
        for d in &m.decls {
            if let Decl::Fn(f) = d {
                if f.generics.is_empty() {
                    out.block(&f.body, true);
                } else {
                    out.in_closure += 1;
                    out.block(&f.body, false);
                    out.in_closure -= 1;
                }
            }
        }
    }
    candidates.retain(|c| !out.bad.contains(c));
    candidates
}

struct Walk<'a> {
    candidates: BTreeSet<String>,
    types: &'a HashMap<(u32, u32), String>,
    bad: HashSet<String>,
    /// Non-generic user functions, whose arguments may be values, with the
    /// mode of each parameter.
    fns: HashMap<String, Vec<ParamMode>>,
    /// Inside a closure, a task or a handler: anything that reaches a value
    /// there captures it, which is a generic operation.
    in_closure: u32,
}

/// How a type text names a candidate: as the struct itself, as a list of it,
/// or inside something else.
enum Shape {
    Value(String),
    ListOf(String),
    Other,
}

impl Walk<'_> {
    fn shape(&self, text: &str) -> Shape {
        if self.candidates.contains(text) {
            return Shape::Value(text.to_string());
        }
        if let Some(inner) = text.strip_prefix('[').and_then(|t| t.strip_suffix(']')) {
            if self.candidates.contains(inner) {
                return Shape::ListOf(inner.to_string());
            }
        }
        Shape::Other
    }

    /// Every candidate named anywhere in `text` as a whole word.
    fn mentioned(&self, text: &str) -> Vec<String> {
        text.split(|c: char| !(c.is_alphanumeric() || c == '_'))
            .filter(|w| self.candidates.contains(*w))
            .map(str::to_string)
            .collect()
    }

    /// A type written in a declaration. `ok`: the position may hold a value
    /// or a list of them.
    fn type_uses(&mut self, te: &TypeExpr, ok: bool) {
        let text = type_text(te);
        match (self.shape(&text), ok) {
            (Shape::Value(_) | Shape::ListOf(_), true) => {}
            // A list of values in a struct field is a pointer, like any list.
            (Shape::ListOf(_), false) if matches!(te, TypeExpr::List(..)) => {}
            _ => {
                for c in self.mentioned(&text) {
                    self.bad.insert(c);
                }
            }
        }
    }

    /// An expression, `ok` when its parent's use of it is one a value allows.
    fn expr(&mut self, e: &Expr, ok: bool) {
        if let Some(t) = self.types.get(&(e.span().start, e.span().end)) {
            let t = t.clone();
            match self.shape(&t) {
                Shape::Value(c) | Shape::ListOf(c) => {
                    if !ok || self.in_closure > 0 {
                        self.bad.insert(c);
                    }
                }
                Shape::Other => {
                    for c in self.mentioned(&t) {
                        self.bad.insert(c);
                    }
                }
            }
        }
        match e {
            Expr::Int(..) | Expr::Float(..) | Expr::Bool(..) | Expr::Ident(_) => {}
            Expr::Str(pieces, _) => {
                for p in pieces {
                    if let StrPiece::Expr(x) = p {
                        self.expr(x, false);
                    }
                }
            }
            Expr::List(items, _) => {
                for x in items {
                    self.expr(x, true);
                }
            }
            Expr::Map(entries, _) => {
                for (k, v) in entries {
                    self.expr(k, false);
                    self.expr(v, false);
                }
            }
            Expr::Tuple(items, _) => {
                for x in items {
                    self.expr(x, false);
                }
            }
            Expr::StructLit { base, fields, .. } => {
                if let Some(b) = base {
                    self.expr(b, false);
                }
                for (_, v) in fields {
                    // The field's declared type decided, at the declaration,
                    // whether a value may go there.
                    self.expr(v, true);
                }
            }
            Expr::Call { callee, args, .. } => {
                let modes = match &**callee {
                    Expr::Ident(id) => self.fns.get(&id.name).cloned(),
                    _ => None,
                };
                if !matches!(&**callee, Expr::Ident(_)) {
                    self.expr(callee, false);
                }
                for (i, a) in args.iter().enumerate() {
                    match modes.as_ref().map(|m| m.get(i).copied()) {
                        // An `inout` argument is a place the callee writes
                        // back to; one rooted in a value is a later stage.
                        Some(Some(ParamMode::Inout)) => {
                            self.disqualify_chain(a);
                            self.expr(a, false);
                        }
                        Some(_) => self.expr(a, true),
                        None => self.expr(a, false),
                    }
                }
            }
            Expr::MethodCall {
                recv, name, args, ..
            } => {
                let on_list = self
                    .types
                    .get(&(recv.span().start, recv.span().end))
                    .is_some_and(|t| matches!(self.shape(t), Shape::ListOf(_)));
                let allowed = on_list && matches!(name.name.as_str(), "push" | "len");
                // A method on a field of a value (`bag.items.push(7)`) may
                // change that field in place, which is a write into the
                // value's slot -- a later stage.
                if matches!(&**recv, Expr::Field { .. }) {
                    self.disqualify_chain(recv);
                }
                self.expr(recv, allowed);
                for a in args {
                    self.expr(a, allowed && name.name == "push");
                }
            }
            Expr::Field { recv, .. } => self.expr(recv, true),
            Expr::Index { recv, index, .. } => {
                self.expr(recv, true);
                self.expr(index, false);
            }
            Expr::Unary { expr, .. } => self.expr(expr, false),
            Expr::Binary { lhs, rhs, .. } => {
                self.expr(lhs, false);
                self.expr(rhs, false);
            }
            Expr::Range { start, end, .. } => {
                self.expr(start, false);
                self.expr(end, false);
            }
            Expr::If {
                cond,
                then_block,
                else_branch,
                ..
            } => {
                self.expr(cond, false);
                self.block(then_block, ok);
                match else_branch.as_deref() {
                    Some(ElseBranch::Else(b)) => self.block(b, ok),
                    Some(ElseBranch::ElseIf(x)) => self.expr(x, ok),
                    None => {}
                }
            }
            Expr::Match {
                scrutinee, arms, ..
            } => {
                self.expr(scrutinee, false);
                for a in arms {
                    if let Some(g) = &a.guard {
                        self.expr(g, false);
                    }
                    self.expr(&a.body, ok);
                }
            }
            Expr::Block(b) => self.block(b, ok),
            // A region frees what it allocated at its closing brace, and a
            // value list there would need region bookkeeping it has not got.
            Expr::Region(b) | Expr::Unsafe(b) => {
                self.in_closure += 1;
                self.block(b, false);
                self.in_closure -= 1;
            }
            Expr::Try { expr, .. } => self.expr(expr, false),
            Expr::Closure { body, .. } => {
                self.in_closure += 1;
                self.expr(body, false);
                self.in_closure -= 1;
            }
        }
    }

    /// A block; `ok` is how its value -- the last expression -- is used.
    fn block(&mut self, b: &Block, ok: bool) {
        let n = b.stmts.len();
        for (i, s) in b.stmts.iter().enumerate() {
            let last = i + 1 == n;
            match s {
                Stmt::Let { init, .. } => self.expr(init, true),
                Stmt::LetTuple { init, .. } => self.expr(init, false),
                Stmt::Assign { target, value, .. } => {
                    // A whole value assigned to a name is a copy, and a write
                    // to one of its own fields (`p.y = 4`) is a store into
                    // its slot. A write deeper than that, or through an
                    // index, would copy-on-write a box *inside* a value,
                    // which is a later stage.
                    match target {
                        Expr::Ident(_) => {
                            self.expr(target, true);
                            self.expr(value, true);
                        }
                        Expr::Field { recv, .. } if matches!(&**recv, Expr::Ident(_)) => {
                            self.expr(recv, true);
                            self.expr(value, false);
                        }
                        // `xs[i] = v` into a list of values held in a name.
                        Expr::Index { recv, index, .. } if matches!(&**recv, Expr::Ident(_)) => {
                            self.expr(recv, true);
                            self.expr(index, false);
                            self.expr(value, true);
                        }
                        _ => {
                            self.disqualify_chain(target);
                            self.expr(value, false);
                        }
                    }
                }
                Stmt::Expr(e) => self.expr(e, if last { ok } else { true }),
                Stmt::Return(Some(e), _) => self.expr(e, true),
                Stmt::Return(None, _) | Stmt::Break(_) | Stmt::Continue(_) => {}
                Stmt::While { cond, body, .. } => {
                    self.expr(cond, false);
                    self.block(body, true);
                }
                Stmt::For { iter, body, .. } => {
                    self.expr(iter, true);
                    self.block(body, true);
                }
                Stmt::Parallel { body, .. } | Stmt::Spawn { body, .. } => {
                    self.in_closure += 1;
                    self.block(body, true);
                    self.in_closure -= 1;
                }
                Stmt::With { handler, body, .. } => {
                    self.expr(handler, false);
                    self.in_closure += 1;
                    self.block(body, true);
                    self.in_closure -= 1;
                }
            }
        }
    }
}

impl Walk<'_> {
    /// Every candidate along an assignment target's path, root included.
    fn disqualify_chain(&mut self, e: &Expr) {
        if let Some(t) = self.types.get(&(e.span().start, e.span().end)) {
            for c in self.mentioned(&t.clone()) {
                self.bad.insert(c);
            }
        }
        match e {
            Expr::Field { recv, .. } => self.disqualify_chain(recv),
            Expr::Index { recv, index, .. } => {
                self.disqualify_chain(recv);
                self.expr(index, false);
            }
            _ => {}
        }
    }
}

/// A type as the checker would render it: `Node`, `[Node]`, `Map[str, i64]`.
fn type_text(te: &TypeExpr) -> String {
    match te {
        TypeExpr::List(inner, _) => format!("[{}]", type_text(inner)),
        TypeExpr::Name { path, args, .. } => {
            let base = path.dotted();
            if args.is_empty() {
                base
            } else {
                let a: Vec<String> = args.iter().map(type_text).collect();
                format!("{base}[{}]", a.join(", "))
            }
        }
        // Anything else -- a function type, a tuple, a reference, a union --
        // is rendered loosely: it only has to name the candidates it holds,
        // and every such mention disqualifies.
        other => format!("{other:?}"),
    }
}
