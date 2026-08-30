//! `grasm lint` — the rules a line cannot answer.
//!
//! `apps/grlint` is a linter for Grasm written in Grasm, and it is deliberately
//! line-based. It ends with a list called `unsupported()`: five rules it will
//! not attempt, each with the same reason —
//!
//! > Every one needs the syntax tree, and the compiler has one. A linter that
//! > guessed at them with a regular expression would be wrong on exactly the
//! > programs that matter — the ones where the shape is unusual.
//!
//! This is the compiler's half of that split. It runs after parsing, over the
//! tree, so it answers what a line cannot: whether a name is ever read, whether
//! one hides another, how long a function really is. Nothing here re-derives
//! what the checker already knows — a non-exhaustive match and an undeclared
//! effect are refused by the checker itself, and a linter that repeated them
//! would be a second implementation of a rule that already has one.
//!
//! Scope is tracked the way the language nests it, and a name is "used" only
//! when it is *read*. `var n = 0` followed by `n = 1` and nothing else is an
//! unused binding: the write is not a use, and treating it as one is what makes
//! most unused-variable warnings useless.

use grasm_syntax::ast::*;
use grasm_syntax::span::Span;

pub struct Finding {
    pub line: u32,
    pub col: u32,
    pub rule: &'static str,
    pub message: String,
}

/// Statements a function may hold before it is called long.
///
/// Counted over the whole tree rather than by line, because a line is not a
/// statement — which is the reason the line-based linter could not have this
/// rule at all.
const LONG_FN: usize = 60;

struct Binding {
    name: String,
    span: Span,
    read: bool,
    /// `let`, `var`, `parameter`, or `import` — the word the message uses.
    what: &'static str,
    /// Bindings a writer marks as deliberately unused are left alone.
    exempt: bool,
}

pub struct Linter<'a> {
    src: &'a str,
    scopes: Vec<Vec<Binding>>,
    findings: Vec<Finding>,
    only: Option<String>,
}

/// Line and column (both 1-based) of a byte offset.
fn line_col(src: &str, at: u32) -> (u32, u32) {
    let at = (at as usize).min(src.len());
    let before = &src[..at];
    let line = before.matches('\n').count() as u32 + 1;
    let col = before.len() as u32 - before.rfind('\n').map_or(0, |i| i as u32 + 1) + 1;
    (line, col)
}

impl<'a> Linter<'a> {
    pub fn new(src: &'a str, only: Option<String>) -> Self {
        Linter { src, scopes: Vec::new(), findings: Vec::new(), only }
    }

    fn report(&mut self, span: Span, rule: &'static str, message: String) {
        if self.only.as_deref().is_some_and(|r| r != rule) {
            return;
        }
        let (line, col) = line_col(self.src, span.start);
        self.findings.push(Finding { line, col, rule, message });
    }

    fn declare(&mut self, name: &Ident, what: &'static str) {
        // A name a writer opened with `_` is unused on purpose. Without this
        // the rule fires on every deliberately-ignored binding and the only
        // way to silence it is to delete code that was written to be read.
        let exempt = name.name.starts_with('_');
        if !exempt && what != "import" {
            // A name that hides one already in scope is reported where it is
            // written, naming where the hidden one was — the pair is the
            // finding, and one half of it is useless.
            if let Some(prev) = self.lookup_span(&name.name) {
                let (l, _) = line_col(self.src, prev.start);
                self.report(
                    name.span,
                    "shadowing",
                    format!("`{}` hides the one bound on line {l}", name.name),
                );
            }
        }
        if let Some(scope) = self.scopes.last_mut() {
            scope.push(Binding {
                name: name.name.clone(),
                span: name.span,
                read: false,
                what,
                exempt,
            });
        }
    }

    fn lookup_span(&self, name: &str) -> Option<Span> {
        for scope in self.scopes.iter().rev() {
            if let Some(b) = scope.iter().rev().find(|b| b.name == name) {
                return Some(b.span);
            }
        }
        None
    }

    /// Mark the innermost binding of `name` as read.
    fn read(&mut self, name: &str) {
        for scope in self.scopes.iter_mut().rev() {
            if let Some(b) = scope.iter_mut().rev().find(|b| b.name == name) {
                b.read = true;
                return;
            }
        }
    }

    fn push_scope(&mut self) {
        self.scopes.push(Vec::new());
    }

    /// Close a scope, reporting every binding in it that was never read.
    fn pop_scope(&mut self) {
        let scope = self.scopes.pop().unwrap_or_default();
        for b in scope {
            if b.read || b.exempt {
                continue;
            }
            let rule = match b.what {
                "parameter" => "unused-parameter",
                "import" => "unused-import",
                _ => "unused-binding",
            };
            self.report(
                b.span,
                rule,
                format!("{} `{}` is never read", b.what, b.name),
            );
        }
    }

    // -- walking ------------------------------------------------------------

    pub fn module(&mut self, m: &Module) {
        self.push_scope();
        for d in &m.decls {
            if let Decl::Import(i) = d {
                // An import is named by the last segment of its path.
                if let Some(last) = i.path.segments.last() {
                    self.declare(last, "import");
                }
            }
        }
        for d in &m.decls {
            self.decl(d);
        }
        self.pop_scope();
    }

    fn decl(&mut self, d: &Decl) {
        match d {
            Decl::Fn(f) => self.function(f),
            Decl::Test(t) => {
                self.push_scope();
                for p in &t.params {
                    self.declare(&p.name, "parameter");
                }
                self.block(&t.body);
                self.pop_scope();
            }
            Decl::Bench(b) => {
                self.push_scope();
                self.block(&b.body);
                self.pop_scope();
            }
            Decl::Const(c) => self.expr(&c.init),
            Decl::Import(_) | Decl::Type(_) | Decl::Effect(_) | Decl::Extern(_) => {}
        }
    }

    fn function(&mut self, f: &FnDecl) {
        let n = count_stmts(&f.body);
        if n > LONG_FN {
            self.report(
                f.span,
                "long-function",
                format!("`{}` is {n} statements; the limit is {LONG_FN}", f.name.name),
            );
        }
        self.push_scope();
        for p in &f.params {
            // An `inout` or `own` parameter exists to be written or consumed,
            // so "never read" is not a defect for one.
            match p.mode {
                ParamMode::Let => self.declare(&p.name, "parameter"),
                _ => self.declare_used(&p.name),
            }
        }
        self.block(&f.body);
        self.pop_scope();
    }

    /// Bind a name that cannot be an unused-binding finding, but can still be
    /// the one a later binding shadows.
    fn declare_used(&mut self, name: &Ident) {
        if let Some(scope) = self.scopes.last_mut() {
            scope.push(Binding {
                name: name.name.clone(),
                span: name.span,
                read: true,
                what: "parameter",
                exempt: true,
            });
        }
    }

    fn block(&mut self, b: &Block) {
        self.push_scope();
        for s in &b.stmts {
            self.stmt(s);
        }
        self.pop_scope();
    }

    fn stmt(&mut self, s: &Stmt) {
        match s {
            Stmt::Let { mutable, name, init, .. } => {
                self.expr(init);
                self.declare(name, if *mutable { "var" } else { "let" });
            }
            Stmt::LetTuple { mutable, names, init, .. } => {
                self.expr(init);
                for n in names {
                    self.declare(n, if *mutable { "var" } else { "let" });
                }
            }
            Stmt::Assign { target, op, value, .. } => {
                self.expr(value);
                // `x = v` writes; `x += v` reads first. Either way a *field* or
                // *index* target reads the thing it reaches into.
                match (target, op) {
                    (Expr::Ident(_), AssignOp::Assign) => {}
                    _ => self.expr(target),
                }
            }
            Stmt::Expr(e) => self.expr(e),
            Stmt::Return(e, _) => {
                if let Some(e) = e {
                    self.expr(e);
                }
            }
            Stmt::Break(_) | Stmt::Continue(_) => {}
            Stmt::While { cond, body, .. } => {
                self.expr(cond);
                self.block(body);
            }
            Stmt::For { var, iter, body, .. } => {
                self.expr(iter);
                self.push_scope();
                self.declare(var, "let");
                self.block(body);
                self.pop_scope();
            }
            Stmt::Parallel { body, .. } | Stmt::Spawn { body, .. } => self.block(body),
            Stmt::With { handler, body, .. } => {
                self.expr(handler);
                self.block(body);
            }
        }
    }

    fn expr(&mut self, e: &Expr) {
        match e {
            Expr::Ident(id) => self.read(&id.name),
            Expr::Int(..) | Expr::Float(..) | Expr::Bool(..) => {}
            Expr::Str(pieces, _) => {
                for p in pieces {
                    if let StrPiece::Expr(e) = p {
                        self.expr(e);
                    }
                }
            }
            Expr::List(xs, _) | Expr::Tuple(xs, _) => {
                for x in xs {
                    self.expr(x);
                }
            }
            Expr::Map(kvs, _) => {
                for (k, v) in kvs {
                    self.expr(k);
                    self.expr(v);
                }
            }
            Expr::StructLit { name, base, fields, .. } => {
                // A constructor names its type, and a type can come from an
                // import — so this counts as a use of the first segment.
                if let Some(first) = name.segments.first() {
                    self.read(&first.name);
                }
                if let Some(b) = base {
                    self.expr(b);
                }
                for (_, v) in fields {
                    self.expr(v);
                }
            }
            Expr::Call { callee, args, .. } => {
                self.expr(callee);
                for a in args {
                    self.expr(a);
                }
            }
            Expr::MethodCall { recv, args, .. } => {
                self.expr(recv);
                for a in args {
                    self.expr(a);
                }
            }
            Expr::Field { recv, .. } => self.expr(recv),
            Expr::Index { recv, index, .. } => {
                self.expr(recv);
                self.expr(index);
            }
            Expr::Unary { expr, .. } | Expr::Try { expr, .. } => self.expr(expr),
            Expr::Binary { lhs, rhs, .. } => {
                self.expr(lhs);
                self.expr(rhs);
            }
            Expr::Range { start, end, .. } => {
                self.expr(start);
                self.expr(end);
            }
            Expr::If { cond, then_block, else_branch, .. } => {
                self.expr(cond);
                self.block(then_block);
                match else_branch.as_deref() {
                    Some(ElseBranch::Else(b)) => self.block(b),
                    Some(ElseBranch::ElseIf(e)) => self.expr(e),
                    None => {}
                }
            }
            Expr::Match { scrutinee, arms, .. } => {
                self.expr(scrutinee);
                for a in arms {
                    self.push_scope();
                    self.pattern(&a.pattern);
                    if let Some(g) = &a.guard {
                        self.expr(g);
                    }
                    self.expr(&a.body);
                    self.pop_scope();
                }
            }
            Expr::Block(b) | Expr::Region(b) | Expr::Unsafe(b) => self.block(b),
            Expr::Closure { params, body, .. } => {
                self.push_scope();
                for p in params {
                    self.declare(p, "parameter");
                }
                self.expr(body);
                self.pop_scope();
            }
        }
    }

    /// A pattern binds names. Whether one of them is a binding or a nullary
    /// variant is settled during typecheck, not here — so a `Bind` whose name
    /// is capitalised is left alone rather than reported as an unused binding
    /// that is really a constructor.
    fn pattern(&mut self, p: &Pattern) {
        match p {
            Pattern::Bind(id) => {
                if id.name.starts_with(|c: char| c.is_ascii_uppercase()) {
                    self.read(&id.name);
                } else {
                    self.declare(id, "let");
                }
            }
            Pattern::Variant { path, fields, .. } => {
                if let Some(first) = path.segments.first() {
                    self.read(&first.name);
                }
                for f in fields {
                    self.pattern(f);
                }
            }
            Pattern::Tuple(ps, _) | Pattern::Or(ps, _) => {
                for q in ps {
                    self.pattern(q);
                }
            }
            Pattern::Wildcard(_) | Pattern::Int(..) | Pattern::Bool(..) | Pattern::Str(..) => {}
        }
    }

    pub fn finish(mut self) -> Vec<Finding> {
        self.findings.sort_by_key(|f| (f.line, f.col));
        std::mem::take(&mut self.findings)
    }
}

fn count_stmts(b: &Block) -> usize {
    b.stmts.iter().map(count_stmt).sum()
}

fn count_stmt(s: &Stmt) -> usize {
    1 + match s {
        Stmt::While { body, .. }
        | Stmt::For { body, .. }
        | Stmt::Parallel { body, .. }
        | Stmt::Spawn { body, .. }
        | Stmt::With { body, .. } => count_stmts(body),
        Stmt::Let { init, .. } | Stmt::LetTuple { init, .. } => count_expr(init),
        Stmt::Assign { value, .. } => count_expr(value),
        Stmt::Expr(e) => count_expr(e),
        Stmt::Return(Some(e), _) => count_expr(e),
        _ => 0,
    }
}

/// Statements hiding inside an expression — a block, a match arm, an `if`.
fn count_expr(e: &Expr) -> usize {
    match e {
        Expr::Block(b) | Expr::Region(b) | Expr::Unsafe(b) => count_stmts(b),
        Expr::If { then_block, else_branch, .. } => {
            count_stmts(then_block)
                + match else_branch.as_deref() {
                    Some(ElseBranch::Else(b)) => count_stmts(b),
                    Some(ElseBranch::ElseIf(e)) => count_expr(e),
                    None => 0,
                }
        }
        Expr::Match { arms, .. } => arms.iter().map(|a| count_expr(&a.body)).sum(),
        Expr::Closure { body, .. } => count_expr(body),
        _ => 0,
    }
}

/// Every rule this reports, with the reason it needs the tree.
pub const RULES: &[(&str, &str)] = &[
    ("unused-binding", "a `let` or `var` that is never read — needs scope"),
    ("unused-parameter", "a parameter that is never read — needs scope"),
    ("unused-import", "an import nothing in the file names"),
    ("shadowing", "a name that hides one already in scope — needs scope"),
    ("long-function", "more statements than the limit — a line is not a statement"),
];
