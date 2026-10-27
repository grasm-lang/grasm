//! TIR: a checked program with every decision written down (`docs/tir.md`).
//!
//! The emitter walks the AST beside the checker's [`TirFacts`]. It resolves
//! nothing itself: what a name means, what a call reaches and every type come
//! from the checker. What it does work out is syntax — which binding of a name
//! is in scope — and what follows from a type without inference: which field
//! of a struct a name is, which variant of a sum a constructor builds.
//!
//! Anything the checker did not record is printed as `?` and reported as a
//! hole, so a gate can demand that a whole corpus has none.

use std::collections::HashMap;

use grasm_sema::check::Analysis;
use grasm_sema::loader::Program;
use grasm_sema::tirfacts::{CallRes, Key, MethodRes, NameRes, TirFacts};
use grasm_sema::types::{substitute_generics, IntKind, Type, TypeDefKind, TypeDefs};
use grasm_syntax::ast::*;
use grasm_syntax::span::Span;

/// The printed TIR of a program, and what the checker left unrecorded.
pub struct Emitted {
    pub text: String,
    pub holes: Vec<String>,
}

/// A node: its head (keyword and inline atoms) and its children, each printed
/// on a line of its own.
struct Node {
    head: String,
    kids: Vec<Node>,
}

impl Node {
    fn leaf(head: impl Into<String>) -> Node {
        Node {
            head: head.into(),
            kids: Vec::new(),
        }
    }
    fn new(head: impl Into<String>, kids: Vec<Node>) -> Node {
        Node {
            head: head.into(),
            kids,
        }
    }
    fn write(&self, depth: usize, out: &mut String) {
        for _ in 0..depth {
            out.push_str("  ");
        }
        out.push('(');
        out.push_str(&self.head);
        for k in &self.kids {
            out.push('\n');
            k.write(depth + 1, out);
        }
        out.push(')');
    }
}

/// A string as a JSON string literal: the one escaping both frontends can
/// write without a library.
pub fn quote(s: &str) -> String {
    let mut out = String::with_capacity(s.len() + 2);
    out.push('"');
    for c in s.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c if (c as u32) < 0x20 || c as u32 == 0x7f => {
                out.push_str(&format!("\\u{:04x}", c as u32));
            }
            c => out.push(c),
        }
    }
    out.push('"');
    out
}

fn span_atom(s: Span) -> String {
    format!("@{}:{}", s.start, s.end)
}

fn key(s: Span) -> Key {
    (s.start, s.end)
}

/// Emits the TIR of a checked program. `analysis` must come from
/// `check_program_tir` and hold no errors.
pub fn emit(program: &Program, analysis: &Analysis) -> Result<Emitted, String> {
    let Some(facts) = analysis.tir.as_ref() else {
        return Err("internal: the program was not checked for TIR".to_string());
    };
    if analysis
        .diagnostics
        .iter()
        .any(|d| d.severity == grasm_syntax::diag::Severity::Error)
    {
        return Err("the program has errors, and a program with errors has no TIR".to_string());
    }
    let mut e = Emitter {
        prog: program,
        an: analysis,
        facts,
        defs: &analysis.defs,
        holes: Vec::new(),
        scopes: Vec::new(),
        next_var: 0,
        aliases: HashMap::new(),
        pattern_rebind: None,
    };
    let mut out = String::from("(tir 1)\n");
    for n in e.program_nodes() {
        n.write(0, &mut out);
        out.push('\n');
    }
    Ok(Emitted {
        text: out,
        holes: e.holes,
    })
}

struct Emitter<'a> {
    prog: &'a Program,
    an: &'a Analysis,
    facts: &'a TirFacts,
    defs: &'a TypeDefs,
    holes: Vec<String>,
    /// Lexical scopes of the function being emitted: name → var id.
    scopes: Vec<Vec<(String, u32)>>,
    next_var: u32,
    /// The current file's import aliases → module names.
    aliases: HashMap<String, String>,
    /// Inside the second and later alternatives of an or-pattern, the names
    /// the first alternative bound: the same name is the same variable.
    pattern_rebind: Option<Vec<(String, u32)>>,
}

impl<'a> Emitter<'a> {
    fn hole(&mut self, span: Span, what: &str) -> String {
        let f = &self.prog.files[self.prog.file_of(span)];
        let local = (span.start.saturating_sub(f.base)) as usize;
        let before = &f.src[..local.min(f.src.len())];
        let line = before.matches('\n').count() + 1;
        let col = before.len() - before.rfind('\n').map(|i| i + 1).unwrap_or(0) + 1;
        let end = ((span.end.saturating_sub(f.base)) as usize).min(f.src.len());
        let text: String = f.src[local.min(end)..end].chars().take(60).collect();
        self.holes.push(format!(
            "{}:{line}:{col}: {what} [{}]",
            f.path,
            text.replace('\n', "\\n")
        ));
        "?".to_string()
    }

    // ----- types -------------------------------------------------------------

    fn ty(&self, t: &Type) -> String {
        match t {
            Type::I64 => "i64".into(),
            Type::Int(k) => int_name(*k).into(),
            Type::F64 => "f64".into(),
            Type::Bool => "bool".into(),
            Type::Str => "str".into(),
            Type::Bytes => "bytes".into(),
            Type::StrBuf => "strbuilder".into(),
            Type::ByteBuf => "bytebuilder".into(),
            Type::Addr => "addr".into(),
            Type::Unit => "unit".into(),
            Type::Range => "range".into(),
            Type::Tuple(ts) => self.ty_list("tuple", ts),
            Type::List(t) => format!("(list {})", self.ty(t)),
            Type::Ref(t) => format!("(ref {})", self.ty(t)),
            Type::Map(k, v) => format!("(map {} {})", self.ty(k), self.ty(v)),
            Type::Named { id, args } => {
                let name = quote(&self.defs.get(*id).name);
                if args.is_empty() {
                    format!("(named {name})")
                } else {
                    let a: Vec<String> = args.iter().map(|t| self.ty(t)).collect();
                    format!("(named {name} {})", a.join(" "))
                }
            }
            Type::Fn { params, ret, row } => {
                let p: Vec<String> = params.iter().map(|t| self.ty(t)).collect();
                let r = match row {
                    None => "(row *)".to_string(),
                    Some(set) => {
                        let mut s = String::from("(row");
                        for e in set {
                            s.push(' ');
                            s.push_str(e);
                        }
                        s.push(')');
                        s
                    }
                };
                format!("(fn ({}) {} {r})", p.join(" "), self.ty(ret))
            }
            Type::Fallible(t, e) => format!("(fallible {} {})", self.ty(t), self.ty(e)),
            Type::Union(ts) => self.ty_list("union", ts),
            Type::Generic(i) => format!("(generic {i})"),
            // A variable nothing constrained: no value of it ever exists
            // (the result of `fail`, the error half of a `main` that never
            // fails), so it is the empty type.
            Type::Var(_) => "never".into(),
            Type::Error => "?".into(),
        }
    }

    fn ty_list(&self, head: &str, ts: &[Type]) -> String {
        let mut s = format!("({head}");
        for t in ts {
            s.push(' ');
            s.push_str(&self.ty(t));
        }
        s.push(')');
        s
    }

    /// The type of an expression, as the checker settled it.
    fn expr_ty(&mut self, span: Span) -> String {
        match self.facts.types.get(&key(span)) {
            Some(t) if !holds_var(t) => self.ty(t),
            _ => self.hole(span, "expression type"),
        }
    }

    fn expr_type_of(&self, span: Span) -> Option<&'a Type> {
        self.facts.types.get(&key(span)).filter(|t| !holds_var(t))
    }

    // ----- program -------------------------------------------------------------

    fn program_nodes(&mut self) -> Vec<Node> {
        let mut out = Vec::new();
        for f in &self.prog.files {
            let mut imports: Vec<(&String, &String)> = f.imports.iter().collect();
            imports.sort();
            let kids = imports
                .into_iter()
                .map(|(a, t)| Node::leaf(format!("import {} {}", quote(a), quote(t))))
                .collect();
            out.push(Node::new(
                format!(
                    "file {} {} {} {}",
                    quote(&f.name),
                    quote(&f.alias),
                    quote(&f.path),
                    f.base
                ),
                kids,
            ));
        }
        for d in &self.defs.defs {
            out.push(self.typedef_node(d));
        }
        for (name, eff) in &self.facts.effects {
            let kids = eff
                .ops
                .iter()
                .map(|(op, params, ret)| {
                    let p: Vec<String> = params.iter().map(|t| self.ty(t)).collect();
                    Node::leaf(format!(
                        "op {} ({}) {}",
                        quote(op),
                        p.join(" "),
                        self.ty(ret)
                    ))
                })
                .collect();
            out.push(Node::new(format!("effect {}", quote(name)), kids));
        }
        let files: Vec<(String, &Module, HashMap<String, String>)> = self
            .prog
            .files
            .iter()
            .map(|f| (f.name.clone(), &f.module, f.imports.clone()))
            .collect();
        for (module, m, imports) in &files {
            self.aliases = imports.clone();
            for d in &m.decls {
                if let Some(n) = self.decl_node(module, d) {
                    out.push(n);
                }
                // [T-EXPORT]: C reaches this function by its own name too.
                if let Decl::Fn(f) = d {
                    if f.is_export {
                        let q = format!("{module}::{}", f.name.name);
                        out.push(Node::leaf(format!("export {}", quote(&q))));
                    }
                }
            }
        }
        if self.facts.fns.contains_key("main::main") {
            out.push(Node::leaf(format!("main {}", quote("main::main"))));
        }
        out
    }

    fn typedef_node(&self, d: &grasm_sema::types::TypeDef) -> Node {
        let gens: Vec<String> = d.generics.iter().map(|g| quote(g)).collect();
        let head = format!(
            "type {} (generics{}{})",
            quote(&d.name),
            if gens.is_empty() { "" } else { " " },
            gens.join(" ")
        );
        let kids = match &d.kind {
            TypeDefKind::Struct { fields } => vec![Node::new(
                "struct",
                fields
                    .iter()
                    .map(|(n, t)| Node::leaf(format!("field {} {}", quote(n), self.ty(t))))
                    .collect(),
            )],
            TypeDefKind::Sum { variants } => vec![Node::new(
                "sum",
                variants
                    .iter()
                    .map(|v| {
                        Node::new(
                            format!("variant {}", quote(&v.name)),
                            v.fields
                                .iter()
                                .map(|(n, t)| {
                                    Node::leaf(format!("field {} {}", quote(n), self.ty(t)))
                                })
                                .collect(),
                        )
                    })
                    .collect(),
            )],
        };
        Node::new(head, kids)
    }

    fn decl_node(&mut self, module: &str, d: &Decl) -> Option<Node> {
        match d {
            Decl::Fn(f) => Some(self.fn_node(module, f)),
            // [T-TLS]: a cell, by its qualified name and layout type.
            Decl::ThreadLocal(t) => {
                let q = format!("{module}::{}", t.name.name);
                let ty = match self.an.c_facts.cells.get(&q) {
                    Some(ty) => ty.name().to_string(),
                    None => self.hole(t.span, "a cell's type"),
                };
                Some(Node::leaf(format!("threadlocal {} {ty}", quote(&q))))
            }
            // [T-RUNTIME]: a process cell, likewise.
            Decl::ProcessLocal(t) => {
                let q = format!("{module}::{}", t.name.name);
                let ty = match self.an.c_facts.process_cells.get(&q) {
                    Some(ty) => ty.name().to_string(),
                    None => self.hole(t.span, "a process cell's type"),
                };
                Some(Node::leaf(format!("processlocal {} {ty}", quote(&q))))
            }
            Decl::Const(c) => {
                let q = format!("{module}::{}", c.name.name);
                let t = match self.facts.consts.get(&q) {
                    Some(t) => self.ty(t),
                    None => self.hole(c.span, "constant type"),
                };
                self.begin_fn();
                let init = self.expr(&c.init);
                Some(Node::new(format!("const {} {t}", quote(&q)), vec![init]))
            }
            Decl::Extern(x) => {
                let q = format!("{module}::{}", x.name.name);
                let sig = match self.facts.fns.get(&q) {
                    Some(sig) => {
                        let p: Vec<String> = sig.params.iter().map(|t| self.ty(t)).collect();
                        format!("({}) {}", p.join(" "), self.ty(&sig.ret))
                    }
                    None => self.hole(x.span, "extern signature"),
                };
                Some(Node::leaf(format!("extern {} {sig}", quote(&q))))
            }
            Decl::Impl(i) => {
                let kids = i
                    .methods
                    .iter()
                    .map(|(m, f)| {
                        Node::leaf(format!(
                            "method {} {}",
                            quote(&m.name),
                            quote(&format!("{module}::{f}"))
                        ))
                    })
                    .collect();
                Some(Node::new(
                    format!("impl {} {}", quote(&i.iface.dotted()), quote(&i.type_name)),
                    kids,
                ))
            }
            Decl::Interface(i) => {
                let kids = i
                    .methods
                    .iter()
                    .map(|m| Node::leaf(format!("method {}", quote(&m.name.name))))
                    .collect();
                Some(Node::new(
                    format!("interface {}", quote(&i.name.name)),
                    kids,
                ))
            }
            Decl::Test(t) => {
                self.begin_fn();
                self.push_scope();
                let params: Vec<Node> = t.params.iter().map(|p| self.param_node(p)).collect();
                let body = self.block(&t.body);
                self.pop_scope();
                Some(Node::new(
                    format!("test {}", quote(&t.name)),
                    vec![Node::new("params", params), body],
                ))
            }
            Decl::Bench(b) => {
                self.begin_fn();
                self.push_scope();
                let body = self.block(&b.body);
                self.pop_scope();
                Some(Node::new(format!("bench {}", quote(&b.name)), vec![body]))
            }
            Decl::Type(_) | Decl::Effect(_) | Decl::Import(_) => None,
        }
    }

    fn fn_node(&mut self, module: &str, f: &FnDecl) -> Node {
        let q = format!("{module}::{}", f.name.name);
        let sig = self.facts.fns.get(&q).cloned();
        self.begin_fn();
        self.push_scope();
        let params: Vec<Node> = f.params.iter().map(|p| self.param_node(p)).collect();
        let body = self.block(&f.body);
        self.pop_scope();
        let gens: Vec<String> = f.generics.iter().map(|g| quote(&g.name)).collect();
        let (ret, err, row, bounds) = match &sig {
            Some(s) => (
                self.ty(&s.ret),
                s.err
                    .as_ref()
                    .map(|e| self.ty(e))
                    .unwrap_or_else(|| "none".into()),
                match &s.row {
                    Some(r) => format!(
                        "(row{}{})",
                        if r.is_empty() { "" } else { " " },
                        r.join(" ")
                    ),
                    None => "(row *)".to_string(),
                },
                s.bounds
                    .iter()
                    .map(|b| {
                        let q: Vec<String> = b.iter().map(|x| quote(x)).collect();
                        format!("({})", q.join(" "))
                    })
                    .collect::<Vec<_>>()
                    .join(" "),
            ),
            None => {
                let h = self.hole(f.span, "function signature");
                (h.clone(), h.clone(), h.clone(), h)
            }
        };
        Node::new(
            format!(
                "fn {} (generics{}{}) (bounds{}{}) {ret} {err} {row}",
                quote(&q),
                if gens.is_empty() { "" } else { " " },
                gens.join(" "),
                if bounds.is_empty() { "" } else { " " },
                bounds
            ),
            vec![Node::new("params", params), body],
        )
    }

    fn param_node(&mut self, p: &Param) -> Node {
        let mode = match p.mode {
            ParamMode::Let => "let",
            ParamMode::Inout => "inout",
            ParamMode::Own => "own",
        };
        let id = self.declare(&p.name, p.mode == ParamMode::Inout);
        let t = self.bind_ty(&p.name);
        Node::leaf(format!("param {mode} {id} {} {t}", quote(&p.name.name)))
    }

    // ----- scopes and variables -----------------------------------------------

    fn begin_fn(&mut self) {
        self.scopes.clear();
        self.next_var = 0;
        self.pattern_rebind = None;
    }

    fn push_scope(&mut self) {
        self.scopes.push(Vec::new());
    }

    fn pop_scope(&mut self) {
        self.scopes.pop();
    }

    /// A new variable for a binding site; returns its id.
    fn declare(&mut self, name: &Ident, _mutable: bool) -> u32 {
        let id = self.next_var;
        self.next_var += 1;
        if self.scopes.is_empty() {
            self.push_scope();
        }
        self.scopes
            .last_mut()
            .unwrap()
            .push((name.name.clone(), id));
        id
    }

    fn lookup(&self, name: &str) -> Option<u32> {
        for sc in self.scopes.iter().rev() {
            if let Some((_, id)) = sc.iter().rev().find(|(n, _)| n == name) {
                return Some(*id);
            }
        }
        None
    }

    fn bind_ty(&mut self, name: &Ident) -> String {
        match self.facts.binds.get(&key(name.span)) {
            Some(t) if !holds_var(t) => self.ty(t),
            _ => self.hole(name.span, &format!("type of binding `{}`", name.name)),
        }
    }

    /// A `var` node for a fresh binding site: id, name, mutability, type.
    fn var_atom(&mut self, name: &Ident, mutable: bool) -> String {
        let id = self.declare(name, mutable);
        let t = self.bind_ty(name);
        format!(
            "{id} {} {} {t}",
            quote(&name.name),
            if mutable { "var" } else { "let" }
        )
    }

    // ----- statements -----------------------------------------------------------

    fn block(&mut self, b: &Block) -> Node {
        self.push_scope();
        let kids: Vec<Node> = b.stmts.iter().map(|s| self.stmt(s)).collect();
        self.pop_scope();
        Node::new(format!("block {}", span_atom(b.span)), kids)
    }

    fn stmt(&mut self, s: &Stmt) -> Node {
        match s {
            Stmt::Let {
                mutable,
                name,
                init,
                ..
            } => {
                let init = self.expr(init);
                let v = self.var_atom(name, *mutable);
                Node::new(format!("let {v}"), vec![init])
            }
            Stmt::LetTuple {
                mutable,
                names,
                init,
                ..
            } => {
                let init = self.expr(init);
                let vars: Vec<Node> = names
                    .iter()
                    .map(|n| {
                        let v = self.var_atom(n, *mutable);
                        Node::leaf(format!("var {v}"))
                    })
                    .collect();
                Node::new("lettuple", vec![Node::new("vars", vars), init])
            }
            Stmt::Assign {
                target, op, value, ..
            } => {
                let op = match op {
                    AssignOp::Assign => "set",
                    AssignOp::Add => "add",
                    AssignOp::Sub => "sub",
                    AssignOp::Mul => "mul",
                    AssignOp::Div => "div",
                };
                let t = self.expr_ty(target.span());
                let place = self.place(target);
                let value = self.expr(value);
                Node::new(format!("assign {op} {t}"), vec![place, value])
            }
            Stmt::Expr(e) => self.expr(e),
            Stmt::Return(e, sp) => match e {
                Some(e) => Node::new(format!("return {}", span_atom(*sp)), vec![self.expr(e)]),
                None => Node::leaf(format!("return {}", span_atom(*sp))),
            },
            Stmt::Break(sp) => Node::leaf(format!("break {}", span_atom(*sp))),
            Stmt::Continue(sp) => Node::leaf(format!("continue {}", span_atom(*sp))),
            Stmt::While { cond, body, span } => {
                let c = self.expr(cond);
                let b = self.block(body);
                Node::new(format!("while {}", span_atom(*span)), vec![c, b])
            }
            Stmt::For {
                var,
                iter,
                body,
                span,
            } => {
                let kind: String = match self.expr_type_of(iter.span()) {
                    Some(Type::Range) => "range".into(),
                    Some(Type::List(_)) => "list".into(),
                    Some(Type::Str) => "str".into(),
                    Some(Type::Bytes) => "bytes".into(),
                    _ => self.hole(iter.span(), "iterable kind"),
                };
                let it = self.expr(iter);
                self.push_scope();
                let v = self.var_atom(var, false);
                let b = self.block(body);
                self.pop_scope();
                Node::new(
                    format!("for {kind} {}", span_atom(*span)),
                    vec![Node::leaf(format!("var {v}")), it, b],
                )
            }
            Stmt::Parallel {
                body,
                span,
                concurrent,
            } => {
                let proved = self.facts.parallel_ok.contains(&span.start);
                let b = self.block(body);
                Node::new(
                    format!(
                        "parallel {} {} {}",
                        if *concurrent {
                            "concurrent"
                        } else {
                            "sequential"
                        },
                        if proved { "proved" } else { "unproved" },
                        span_atom(*span)
                    ),
                    vec![b],
                )
            }
            Stmt::Spawn {
                body,
                span,
                concurrent,
            } => {
                let b = self.block(body);
                Node::new(
                    format!(
                        "spawn {} {}",
                        if *concurrent {
                            "concurrent"
                        } else {
                            "sequential"
                        },
                        span_atom(*span)
                    ),
                    vec![b],
                )
            }
            Stmt::With {
                effect,
                handler,
                body,
                span,
            } => {
                let h = self.expr(handler);
                let b = self.block(body);
                Node::new(
                    format!("with {} {}", quote(&effect.dotted()), span_atom(*span)),
                    vec![h, b],
                )
            }
        }
    }

    /// An assignment target: a local, then a path of fields and indexes.
    fn place(&mut self, target: &Expr) -> Node {
        match target {
            Expr::Ident(id) => match self.lookup(&id.name) {
                Some(v) => Node::leaf(format!("local {v} {}", span_atom(id.span))),
                None => {
                    let h = self.hole(
                        id.span,
                        &format!("assignment to unknown local `{}`", id.name),
                    );
                    Node::leaf(format!("local {h}"))
                }
            },
            Expr::Field { recv, name, span } => {
                // Writing `.value` through a `ref` to anything but a struct or
                // a tuple writes the cell, as reading it reads the cell.
                if let Some(Type::Ref(inner)) = self.expr_type_of(recv.span()).cloned() {
                    let struct_or_tuple = match &*inner {
                        Type::Tuple(_) => true,
                        Type::Named { id, .. } => {
                            matches!(self.defs.get(*id).kind, TypeDefKind::Struct { .. })
                        }
                        _ => false,
                    };
                    if name.name == "value" && !struct_or_tuple {
                        let r = self.place(recv);
                        return Node::new(format!("deref {}", span_atom(*span)), vec![r]);
                    }
                }
                let idx = self.field_index(recv.span(), &name.name, *span);
                let r = self.place(recv);
                Node::new(format!("field {idx} {}", span_atom(*span)), vec![r])
            }
            Expr::Index { recv, index, span } => {
                let r = self.place(recv);
                let i = self.expr(index);
                Node::new(format!("index {}", span_atom(*span)), vec![r, i])
            }
            other => {
                let h = self.hole(other.span(), "assignment target");
                Node::leaf(format!("place {h}"))
            }
        }
    }

    // ----- expressions ----------------------------------------------------------

    fn expr(&mut self, e: &Expr) -> Node {
        let n = self.expr_inner(e);
        let k = key(e.span());
        if self.an.ok_wrapped.contains(&k) {
            let t = match self.facts.wraps.get(&k) {
                Some(t) if !holds_var(t) => self.ty(t),
                _ => self.hole(e.span(), "type an ok half is wrapped into"),
            };
            return Node::new(format!("wrapok {t} {}", span_atom(e.span())), vec![n]);
        }
        n
    }

    fn head(&mut self, kw: &str, span: Span) -> String {
        let t = self.expr_ty(span);
        format!("{kw} {t} {}", span_atom(span))
    }

    /// An integer literal the checker made a double, negated or not.
    fn is_promoted_literal(&self, e: &Expr) -> bool {
        match e {
            Expr::Int(_, sp) => self.an.float_literals.contains(&sp.start),
            Expr::Unary {
                op: UnaryOp::Neg,
                expr,
                ..
            } => self.is_promoted_literal(expr),
            _ => false,
        }
    }

    fn expr_inner(&mut self, e: &Expr) -> Node {
        let sp = e.span();
        match e {
            Expr::Int(v, _) => {
                if self.an.float_literals.contains(&sp.start) {
                    let bits = (*v as f64).to_bits();
                    return Node::leaf(format!("float f64 {} 0x{bits:016x}", span_atom(sp)));
                }
                let h = self.head("int", sp);
                // A `u64` literal is held as its bits in an `i64`; it is
                // printed as the number that was written.
                if h.starts_with("int u64 ") {
                    return Node::leaf(format!("{h} {}", *v as u64));
                }
                Node::leaf(format!("{h} {v}"))
            }
            Expr::Float(f, _) => Node::leaf(format!(
                "float f64 {} 0x{:016x}",
                span_atom(sp),
                f.to_bits()
            )),
            Expr::Bool(b, _) => {
                let h = self.head("bool", sp);
                Node::leaf(format!("{h} {b}"))
            }
            Expr::Str(pieces, _) => {
                let h = self.head("str", sp);
                let kids = pieces
                    .iter()
                    .map(|p| match p {
                        StrPiece::Text(t) => Node::leaf(format!("text {}", quote(t))),
                        StrPiece::Expr(x) => Node::new("value", vec![self.expr(x)]),
                    })
                    .collect();
                Node::new(h, kids)
            }
            Expr::Ident(id) => self.ident(id),
            Expr::List(xs, _) => {
                let h = self.head("list", sp);
                let kids = xs.iter().map(|x| self.expr(x)).collect();
                Node::new(h, kids)
            }
            Expr::Map(kvs, _) => {
                let h = self.head("maplit", sp);
                let kids = kvs
                    .iter()
                    .map(|(k, v)| {
                        let k = self.expr(k);
                        let v = self.expr(v);
                        Node::new("entry", vec![k, v])
                    })
                    .collect();
                Node::new(h, kids)
            }
            Expr::Tuple(xs, _) => {
                let h = self.head("tuple", sp);
                let kids = xs.iter().map(|x| self.expr(x)).collect();
                Node::new(h, kids)
            }
            Expr::StructLit {
                base, fields, span, ..
            } => {
                let h = self.head("struct", *span);
                let mut kids = Vec::new();
                for (name, x) in fields {
                    let idx = self.field_index(*span, &name.name, name.span);
                    kids.push(Node::new(format!("field {idx}"), vec![self.expr(x)]));
                }
                if let Some(b) = base {
                    kids.push(Node::new("base", vec![self.expr(b)]));
                }
                Node::new(h, kids)
            }
            Expr::Call { callee, args, span } => self.call(callee, args, *span),
            Expr::MethodCall {
                recv,
                name,
                args,
                span,
            } => self.method_call(recv, name, args, *span),
            Expr::Field { recv, name, span } => self.field(recv, name, *span),
            Expr::Index { recv, index, span } => {
                let h = self.head("index", *span);
                let r = self.expr(recv);
                let i = self.expr(index);
                Node::new(h, vec![r, i])
            }
            Expr::Unary { op, expr, span } => {
                // `-128` written against `i8` is typed as a whole; the literal
                // inside it has the same type.
                if let (UnaryOp::Neg, Expr::Int(v, isp)) = (op, &**expr) {
                    if self.expr_type_of(*isp).is_none() {
                        let t = self.expr_ty(*span);
                        let inner = if self.an.float_literals.contains(&isp.start) {
                            let bits = (*v as f64).to_bits();
                            Node::leaf(format!("float f64 {} 0x{bits:016x}", span_atom(*isp)))
                        } else {
                            Node::leaf(format!("int {t} {} {v}", span_atom(*isp)))
                        };
                        return Node::new(
                            format!("unary {t} {} neg {t}", span_atom(*span)),
                            vec![inner],
                        );
                    }
                }
                let h = self.head("unary", *span);
                let ot = self.expr_ty(expr.span());
                let op = match op {
                    UnaryOp::Neg => "neg",
                    UnaryOp::Not => "not",
                };
                let x = self.expr(expr);
                Node::new(format!("{h} {op} {ot}"), vec![x])
            }
            Expr::Binary { op, lhs, rhs, span } => {
                let h = self.head("binary", *span);
                // An integer literal promoted to a double by [E-NUM-PROMOTE]
                // is written as a `float` node, but its recorded type is
                // still the literal's: `3 + f` came out as `add i64` over two
                // doubles. The operator's meaning is the double one.
                let ot = if self.is_promoted_literal(lhs) {
                    "f64".to_string()
                } else {
                    self.expr_ty(lhs.span())
                };
                let op = match op {
                    BinaryOp::Add => "add",
                    BinaryOp::Sub => "sub",
                    BinaryOp::Mul => "mul",
                    BinaryOp::Div => "div",
                    BinaryOp::Rem => "rem",
                    BinaryOp::Pow => "pow",
                    BinaryOp::Eq => "eq",
                    BinaryOp::NotEq => "ne",
                    BinaryOp::Lt => "lt",
                    BinaryOp::LtEq => "le",
                    BinaryOp::Gt => "gt",
                    BinaryOp::GtEq => "ge",
                    BinaryOp::And => "and",
                    BinaryOp::Or => "or",
                    BinaryOp::Coalesce => "coalesce",
                };
                let l = self.expr(lhs);
                let r = self.expr(rhs);
                Node::new(format!("{h} {op} {ot}"), vec![l, r])
            }
            Expr::Range {
                start,
                end,
                inclusive,
                span,
            } => {
                let h = self.head("range", *span);
                let s = self.expr(start);
                let e = self.expr(end);
                Node::new(
                    format!("{h} {}", if *inclusive { "inclusive" } else { "exclusive" }),
                    vec![s, e],
                )
            }
            Expr::If {
                cond,
                then_block,
                else_branch,
                span,
            } => {
                let h = self.head("if", *span);
                let c = self.expr(cond);
                let t = self.block(then_block);
                let mut kids = vec![c, t];
                if let Some(eb) = else_branch {
                    kids.push(match &**eb {
                        ElseBranch::Else(b) => Node::new("else", vec![self.block(b)]),
                        ElseBranch::ElseIf(x) => Node::new("elseif", vec![self.expr(x)]),
                    });
                }
                Node::new(h, kids)
            }
            Expr::Match {
                scrutinee,
                arms,
                span,
            } => {
                let h = self.head("match", *span);
                let scrut_ty = self.expr_type_of(scrutinee.span()).cloned();
                let s = self.expr(scrutinee);
                let mut kids = vec![s];
                for arm in arms {
                    self.push_scope();
                    let p = self.pattern(&arm.pattern, scrut_ty.as_ref());
                    let mut ak = vec![p];
                    if let Some(g) = &arm.guard {
                        ak.push(Node::new("guard", vec![self.expr(g)]));
                    }
                    ak.push(self.expr(&arm.body));
                    self.pop_scope();
                    kids.push(Node::new(format!("arm {}", span_atom(arm.span)), ak));
                }
                Node::new(h, kids)
            }
            Expr::Block(b) => {
                let h = self.head("blockexpr", b.span);
                Node::new(h, vec![self.block(b)])
            }
            Expr::Region(b) => {
                let h = self.head("region", b.span);
                Node::new(h, vec![self.block(b)])
            }
            Expr::Unsafe(b) => {
                let h = self.head("unsafe", b.span);
                Node::new(h, vec![self.block(b)])
            }
            Expr::Try { expr, span } => {
                let h = self.head("try", *span);
                Node::new(h, vec![self.expr(expr)])
            }
            Expr::Closure { params, body, span } => {
                let h = self.head("closure", *span);
                let fallible = self.an.fallible_closures.contains(&span.start);
                self.push_scope();
                let ps: Vec<Node> = params
                    .iter()
                    .map(|p| {
                        let v = self.var_atom(p, false);
                        Node::leaf(format!("var {v}"))
                    })
                    .collect();
                let b = self.expr(body);
                self.pop_scope();
                Node::new(
                    format!("{h} {}", if fallible { "fallible" } else { "total" }),
                    vec![Node::new("params", ps), b],
                )
            }
        }
    }

    fn ident(&mut self, id: &Ident) -> Node {
        let sp = id.span;
        match self.facts.names.get(&key(sp)).cloned() {
            Some(NameRes::Local) => {
                let h = self.head("local", sp);
                match self.lookup(&id.name) {
                    Some(v) => Node::leaf(format!("{h} {v}")),
                    None => {
                        let x = self.hole(sp, &format!("local `{}` not in scope", id.name));
                        Node::leaf(format!("{h} {x}"))
                    }
                }
            }
            Some(NameRes::Variant) => {
                let h = self.head("variant", sp);
                let v = self.variant_ref(sp, &id.name);
                Node::leaf(format!("{h} {v}"))
            }
            Some(NameRes::Const(q)) => {
                let h = self.head("const", sp);
                Node::leaf(format!("{h} {}", quote(&q)))
            }
            Some(NameRes::Fn(q, targs)) => {
                let h = self.head("fnref", sp);
                let ta = self.targs(&targs, sp);
                Node::leaf(format!("{h} {} {ta}", quote(&q)))
            }
            None => {
                let x = self.hole(sp, &format!("what `{}` names", id.name));
                Node::leaf(format!("name {x} {}", quote(&id.name)))
            }
        }
    }

    fn targs(&mut self, ts: &[Type], sp: Span) -> String {
        let mut s = String::from("(targs");
        for t in ts {
            s.push(' ');
            if holds_var(t) {
                let h = self.hole(sp, "type argument");
                s.push_str(&h);
            } else {
                s.push_str(&self.ty(t));
            }
        }
        s.push(')');
        s
    }

    /// `"Type" index` for a variant named `name` of the sum type the
    /// expression at `sp` has.
    fn variant_ref(&mut self, sp: Span, name: &str) -> String {
        let short = name.rsplit('.').next().unwrap_or(name);
        let ty = self.expr_type_of(sp).cloned();
        self.variant_of(ty.as_ref(), short, sp)
    }

    fn variant_of(&mut self, ty: Option<&Type>, short: &str, sp: Span) -> String {
        if let Some(Type::Named { id, .. }) = ty {
            let def = self.defs.get(*id);
            if let TypeDefKind::Sum { variants } = &def.kind {
                if let Some(i) = variants.iter().position(|v| v.name == short) {
                    return format!("{} {i}", quote(&def.name));
                }
            }
        }
        self.hole(sp, &format!("variant `{short}`"))
    }

    fn call(&mut self, callee: &Expr, args: &[Expr], span: Span) -> Node {
        let res = self.facts.calls.get(&key(span)).cloned();
        match res {
            Some(CallRes::Builtin(name)) => {
                // `fail`, `panic` and `exit` do not return, and the checker
                // gives them a type that fits anywhere: in TIR that is `never`.
                let h = if matches!(name.as_str(), "fail" | "panic" | "exit")
                    && self.expr_type_of(span).is_none()
                {
                    format!("builtin never {}", span_atom(span))
                } else {
                    self.head("builtin", span)
                };
                let kids = self.mem_args(args, span);
                Node::new(format!("{h} {}", quote(&name)), kids)
            }
            Some(CallRes::Fn(q, targs)) => self.fn_call(&q, &targs, None, args, span),
            Some(CallRes::Variant) => {
                let h = self.head("variant", span);
                let name = match callee {
                    Expr::Ident(id) => id.name.clone(),
                    _ => String::new(),
                };
                let v = self.variant_ref(span, &name);
                let kids = args.iter().map(|a| self.expr(a)).collect();
                Node::new(format!("{h} {v}"), kids)
            }
            Some(CallRes::Value) => {
                let h = self.head("callvalue", span);
                let mut kids = vec![self.expr(callee)];
                kids.extend(args.iter().map(|a| self.expr(a)));
                Node::new(h, kids)
            }
            None => {
                let x = self.hole(span, "what a call reaches");
                Node::leaf(format!("call {x}"))
            }
        }
    }

    /// A built-in's arguments. Two raw-memory arguments are names, not values
    /// ([L-LIB-MEM], [L-LIB-ATOMIC]): `mem_size_of(u16)` takes a type, printed
    /// `(type u16)`, and an atomic's ordering is printed `(order acquire)`.
    fn mem_args(&mut self, args: &[Expr], span: Span) -> Vec<Node> {
        use grasm_sema::mem::MemOp;
        match self.an.mem_ops.get(&key(span)) {
            Some(MemOp::Size(t) | MemOp::Align(t)) => {
                vec![Node::leaf(format!("type {}", t.name()))]
            }
            Some(MemOp::Atomic(_, _, o)) => {
                let (last, values) = args.split_last().expect("an atomic has its ordering");
                let mut kids: Vec<Node> = values.iter().map(|a| self.expr(a)).collect();
                let o = o.name().to_ascii_lowercase();
                kids.push(Node::leaf(format!("order {o} {}", span_atom(last.span()))));
                kids
            }
            // [T-TLS]: the cell is a name, and is not evaluated.
            Some(MemOp::Tls(_)) => match self.an.c_facts.ops.get(&key(span)) {
                Some(grasm_sema::mem::COp::Tls { cell }) => {
                    vec![Node::leaf(format!("tlsname {}", quote(cell)))]
                }
                _ => vec![Node::leaf(self.hole(span, "the cell an address names"))],
            },
            // [T-FN-ADDR]: the function is a name, and is not evaluated.
            Some(MemOp::FnAddr) => match self.an.c_facts.ops.get(&key(span)) {
                Some(grasm_sema::mem::COp::FnAddr { target, .. }) => {
                    vec![Node::leaf(format!("fnname {}", quote(target)))]
                }
                _ => vec![Node::leaf(self.hole(span, "the function an address names"))],
            },
            // [L-LIB-CALL-C]: the signature is a name, and is not evaluated.
            Some(MemOp::CallC) => {
                let mut kids = match self.an.c_facts.ops.get(&key(span)) {
                    Some(grasm_sema::mem::COp::CallC(sig)) => {
                        let ps: Vec<&str> = sig.params.iter().map(|p| p.name()).collect();
                        let r = sig.ret.map_or("unit", |r| r.name());
                        vec![Node::leaf(format!("sig ({}) {r}", ps.join(" ")))]
                    }
                    _ => vec![Node::leaf(self.hole(span, "a call's C signature"))],
                };
                kids.extend(args.iter().skip(1).map(|a| self.expr(a)));
                kids
            }
            // [T-RUNTIME]: a process cell and a literal's bytes are names,
            // and are not evaluated: the literal is never a `str` value.
            Some(MemOp::Runtime(grasm_sema::mem::RtOp::Process(_))) => {
                match self.an.c_facts.ops.get(&key(span)) {
                    Some(grasm_sema::mem::COp::Process { cell }) => {
                        vec![Node::leaf(format!("processname {}", quote(cell)))]
                    }
                    _ => vec![Node::leaf(self.hole(span, "the process cell an address names"))],
                }
            }
            Some(MemOp::Runtime(
                grasm_sema::mem::RtOp::Static | grasm_sema::mem::RtOp::StaticLen,
            )) => match self.an.c_facts.ops.get(&key(span)) {
                Some(grasm_sema::mem::COp::Static { text }) => {
                    vec![Node::leaf(format!("mstatic {}", quote(text)))]
                }
                _ => vec![Node::leaf(self.hole(span, "the literal a static names"))],
            },
            _ => args.iter().map(|a| self.expr(a)).collect(),
        }
    }

    /// A call of a declared function or `extern`, with an optional receiver
    /// that becomes the first argument (UFCS and `impl` methods).
    fn fn_call(
        &mut self,
        q: &str,
        targs: &[Type],
        recv: Option<&Expr>,
        args: &[Expr],
        span: Span,
    ) -> Node {
        let is_extern = self.facts.fns.get(q).is_some_and(|s| s.is_extern);
        let kw = if is_extern { "callextern" } else { "call" };
        let h = self.head(kw, span);
        let ta = self.targs(targs, span);
        let mut kids = Vec::new();
        if let Some(r) = recv {
            kids.push(self.expr(r));
        }
        kids.extend(args.iter().map(|a| self.expr(a)));
        Node::new(format!("{h} {} {ta}", quote(q)), kids)
    }

    fn method_call(&mut self, recv: &Expr, name: &Ident, args: &[Expr], span: Span) -> Node {
        let res = self.facts.methods.get(&key(span)).cloned();
        let call = self.facts.calls.get(&key(span)).cloned();
        match res {
            Some(MethodRes::Module) => match call {
                Some(CallRes::Fn(q, targs)) => self.fn_call(&q, &targs, None, args, span),
                _ => {
                    let x = self.hole(span, "module function");
                    Node::leaf(format!("call {x}"))
                }
            },
            Some(MethodRes::Interface) | Some(MethodRes::Ufcs) => match call {
                Some(CallRes::Fn(q, targs)) => self.fn_call(&q, &targs, Some(recv), args, span),
                _ => {
                    let x = self.hole(span, "method's function");
                    Node::leaf(format!("call {x}"))
                }
            },
            Some(MethodRes::Bound(iface)) => {
                let h = self.head("boundcall", span);
                let mut kids = vec![self.expr(recv)];
                kids.extend(args.iter().map(|a| self.expr(a)));
                Node::new(format!("{h} {} {}", quote(&iface), quote(&name.name)), kids)
            }
            Some(MethodRes::EffectOp(eff, op)) => {
                let h = self.head("effectop", span);
                let kids = args.iter().map(|a| self.expr(a)).collect();
                Node::new(format!("{h} {} {}", quote(&eff), quote(&op)), kids)
            }
            Some(MethodRes::Variant) => {
                let h = self.head("variant", span);
                let v = self.variant_ref(span, &name.name);
                let kids = args.iter().map(|a| self.expr(a)).collect();
                Node::new(format!("{h} {v}"), kids)
            }
            Some(MethodRes::Builtin) => {
                let h = self.head("method", span);
                let mut kids = vec![self.expr(recv)];
                kids.extend(self.mem_args(args, span));
                Node::new(format!("{h} {}", quote(&name.name)), kids)
            }
            Some(MethodRes::FieldFn) => {
                let h = self.head("callvalue", span);
                // A function held in a tuple component: `pair.0(4)`.
                if let Some(Type::Tuple(parts)) = self.expr_type_of(recv.span()).cloned() {
                    if let Some(ft) = name.name.parse::<usize>().ok().and_then(|i| parts.get(i)) {
                        let ft = self.ty(ft);
                        let r = self.expr(recv);
                        let f = Node::new(
                            format!("tuplefield {ft} {} {}", span_atom(name.span), name.name),
                            vec![r],
                        );
                        let mut kids = vec![f];
                        kids.extend(args.iter().map(|a| self.expr(a)));
                        return Node::new(h, kids);
                    }
                }
                let idx = self.field_index(recv.span(), &name.name, name.span);
                let ft = self.field_type(recv.span(), &name.name, name.span);
                let r = self.expr(recv);
                let f = Node::new(
                    format!("field {ft} {} {idx}", span_atom(name.span)),
                    vec![r],
                );
                let mut kids = vec![f];
                kids.extend(args.iter().map(|a| self.expr(a)));
                Node::new(h, kids)
            }
            None => {
                let x = self.hole(span, &format!("how `.{}` dispatched", name.name));
                Node::leaf(format!("method {x}"))
            }
        }
    }

    fn field(&mut self, recv: &Expr, name: &Ident, span: Span) -> Node {
        // What the checker resolved through the dot: `Color.Red`, a module's
        // constant, a module's function as a value.
        match self.facts.names.get(&key(span)).cloned() {
            Some(NameRes::Variant) => {
                let h = self.head("variant", span);
                let v = self.variant_ref(span, &name.name);
                return Node::leaf(format!("{h} {v}"));
            }
            Some(NameRes::Const(q)) => {
                let h = self.head("const", span);
                return Node::leaf(format!("{h} {}", quote(&q)));
            }
            Some(NameRes::Fn(q, targs)) => {
                let h = self.head("fnref", span);
                let ta = self.targs(&targs, span);
                return Node::leaf(format!("{h} {} {ta}", quote(&q)));
            }
            _ => {}
        }
        // A constant reached through a module alias: `utils.LIMIT`.
        if let Expr::Ident(rid) = recv {
            if self.lookup(&rid.name).is_none() && !self.facts.names.contains_key(&key(rid.span)) {
                if let Some(target) = self.aliases.get(&rid.name).cloned() {
                    let h = self.head("const", span);
                    return Node::leaf(format!(
                        "{h} {}",
                        quote(&format!("{target}::{}", name.name))
                    ));
                }
            }
        }
        let recv_ty = self.expr_type_of(recv.span()).cloned();
        let through = match &recv_ty {
            Some(Type::Ref(inner)) => Some((**inner).clone()),
            other => other.clone(),
        };
        // `.value` through a `ref` whose referent is neither a struct nor a
        // tuple reads the cell (the checker's `field_type_of`).
        if let Some(Type::Ref(inner)) = &recv_ty {
            let struct_or_tuple = match &**inner {
                Type::Tuple(_) => true,
                Type::Named { id, .. } => {
                    matches!(self.defs.get(*id).kind, TypeDefKind::Struct { .. })
                }
                _ => false,
            };
            if name.name == "value" && !struct_or_tuple {
                let h = self.head("deref", span);
                let r = self.expr(recv);
                return Node::new(h, vec![r]);
            }
        }
        if let Some(Type::Tuple(_)) = through {
            let h = self.head("tuplefield", span);
            let r = self.expr(recv);
            return Node::new(format!("{h} {}", name.name), vec![r]);
        }
        let h = self.head("field", span);
        let idx = self.field_index(recv.span(), &name.name, span);
        let r = self.expr(recv);
        Node::new(format!("{h} {idx}"), vec![r])
    }

    /// The position of field `name` in the struct type of the expression at
    /// `recv` (or of the struct literal at `recv`), through a `ref`.
    fn field_index(&mut self, recv: Span, name: &str, at: Span) -> String {
        let t = self.expr_type_of(recv).cloned();
        let t = match t {
            Some(Type::Ref(inner)) => Some(*inner),
            other => other,
        };
        if let Some(Type::Named { id, .. }) = &t {
            if let TypeDefKind::Struct { fields } = &self.defs.get(*id).kind {
                if let Some(i) = fields.iter().position(|(n, _)| n == name) {
                    return i.to_string();
                }
            }
        }
        self.hole(at, &format!("field `{name}`"))
    }

    fn field_type(&mut self, recv: Span, name: &str, at: Span) -> String {
        let t = self.expr_type_of(recv).cloned();
        let t = match t {
            Some(Type::Ref(inner)) => Some(*inner),
            other => other,
        };
        if let Some(Type::Named { id, args }) = &t {
            if let TypeDefKind::Struct { fields } = &self.defs.get(*id).kind {
                if let Some((_, ft)) = fields.iter().find(|(n, _)| n == name) {
                    return self.ty(&substitute_generics(ft, args));
                }
            }
        }
        self.hole(at, &format!("type of field `{name}`"))
    }

    // ----- patterns ---------------------------------------------------------------

    fn pattern(&mut self, p: &Pattern, ty: Option<&Type>) -> Node {
        // A pattern matches through a `ref` (the checker's `check_pattern`).
        if let Some(Type::Ref(inner)) = ty {
            let inner = (**inner).clone();
            return self.pattern(p, Some(&inner));
        }
        match p {
            Pattern::Wildcard(sp) => Node::leaf(format!("pwild {}", span_atom(*sp))),
            Pattern::Int(v, sp) => Node::leaf(format!("pint {} {v}", span_atom(*sp))),
            Pattern::Bool(b, sp) => Node::leaf(format!("pbool {} {b}", span_atom(*sp))),
            Pattern::Str(s, sp) => Node::leaf(format!("pstr {} {}", span_atom(*sp), quote(s))),
            Pattern::Bind(id) => {
                // A bare name the checker did not bind is a nullary variant of
                // the matched type (`None`), as the checker reads it.
                if !self.facts.binds.contains_key(&key(id.span)) {
                    if let Some(Type::Named { id: def_id, .. }) = ty {
                        if let TypeDefKind::Sum { variants } = &self.defs.get(*def_id).kind {
                            if let Some(i) = variants.iter().position(|v| v.name == id.name) {
                                let name = quote(&self.defs.get(*def_id).name);
                                return Node::leaf(format!(
                                    "pvariant {} {name} {i}",
                                    span_atom(id.span)
                                ));
                            }
                        }
                    }
                }
                if let Some(bound) = &self.pattern_rebind {
                    if let Some((_, v)) = bound.iter().find(|(n, _)| *n == id.name) {
                        return Node::leaf(format!("pbind {} {v}", span_atom(id.span)));
                    }
                }
                let v = self.var_atom(id, false);
                Node::leaf(format!("pbind {} {v}", span_atom(id.span)))
            }
            Pattern::Variant { path, fields, span } => {
                let short = path.segments.last().map(|s| s.name.as_str()).unwrap_or("");
                // `Ok(..)` and `Err(..)` match the two halves of a `T ! E`,
                // which is not a named sum type.
                if let (Some(Type::Fallible(ok, err)), "Ok" | "Err") = (ty, short) {
                    let (kw, inner) = if short == "Ok" {
                        ("pok", ok)
                    } else {
                        ("perr", err)
                    };
                    let inner = (**inner).clone();
                    let kids = fields
                        .iter()
                        .map(|f| self.pattern(f, Some(&inner)))
                        .collect();
                    return Node::new(format!("{kw} {}", span_atom(*span)), kids);
                }
                let v = self.variant_of(ty, short, *span);
                let field_tys: Vec<Option<Type>> = match ty {
                    Some(Type::Named { id, args }) => match &self.defs.get(*id).kind {
                        TypeDefKind::Sum { variants } => variants
                            .iter()
                            .find(|x| x.name == short)
                            .map(|x| {
                                x.fields
                                    .iter()
                                    .map(|(_, ft)| Some(substitute_generics(ft, args)))
                                    .collect()
                            })
                            .unwrap_or_default(),
                        _ => Vec::new(),
                    },
                    _ => Vec::new(),
                };
                let kids = fields
                    .iter()
                    .enumerate()
                    .map(|(i, f)| {
                        let ft = field_tys.get(i).cloned().flatten();
                        self.pattern(f, ft.as_ref())
                    })
                    .collect();
                Node::new(format!("pvariant {} {v}", span_atom(*span)), kids)
            }
            Pattern::Tuple(ps, sp) => {
                let tys: Vec<Option<Type>> = match ty {
                    Some(Type::Tuple(ts)) => ts.iter().cloned().map(Some).collect(),
                    _ => Vec::new(),
                };
                let kids = ps
                    .iter()
                    .enumerate()
                    .map(|(i, x)| {
                        let t = tys.get(i).cloned().flatten();
                        self.pattern(x, t.as_ref())
                    })
                    .collect();
                Node::new(format!("ptuple {}", span_atom(*sp)), kids)
            }
            Pattern::Or(ps, sp) => {
                let mut kids = Vec::new();
                let before = self.scopes.last().map(|s| s.len()).unwrap_or(0);
                for (i, x) in ps.iter().enumerate() {
                    if i == 0 {
                        kids.push(self.pattern(x, ty));
                        let bound: Vec<(String, u32)> = self
                            .scopes
                            .last()
                            .map(|s| s[before..].to_vec())
                            .unwrap_or_default();
                        let saved = self.pattern_rebind.replace(bound);
                        // Later alternatives bind the same names to the same
                        // variables; restore once all of them are written.
                        let rest: Vec<Node> = ps[1..].iter().map(|y| self.pattern(y, ty)).collect();
                        self.pattern_rebind = saved;
                        kids.extend(rest);
                        break;
                    }
                }
                Node::new(format!("por {}", span_atom(*sp)), kids)
            }
        }
    }
}

fn int_name(k: IntKind) -> &'static str {
    k.name()
}

fn holds_var(t: &Type) -> bool {
    match t {
        Type::Error => true,
        Type::Tuple(ts) | Type::Union(ts) => ts.iter().any(holds_var),
        Type::List(t) | Type::Ref(t) => holds_var(t),
        Type::Map(k, v) | Type::Fallible(k, v) => holds_var(k) || holds_var(v),
        Type::Named { args, .. } => args.iter().any(holds_var),
        Type::Fn { params, ret, .. } => params.iter().any(holds_var) || holds_var(ret),
        _ => false,
    }
}
