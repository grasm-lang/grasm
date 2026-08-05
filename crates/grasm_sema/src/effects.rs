//! Effect-row inference: a syntactic call-graph fixpoint (module-aware).
//!
//! Every function gets an inferred effect row (set of effect names; `io` is
//! the built-in effect of `print`/fs builtins). `with E handled_by …` blocks
//! discharge effect `E` inside them. Declared rows (`/ {io, Clock}`) are
//! upper bounds; `main` and tests must end up with nothing but `io`.

use std::collections::{BTreeSet, HashMap, HashSet};

use grasm_syntax::ast::*;
use grasm_syntax::diag::Diagnostic;
use grasm_syntax::span::Span;

use crate::check::ModuleView;

pub const IO: &str = "io";
pub const ASYNC: &str = "async";

const IO_BUILTINS: &[&str] = &[
    "print",
    "read_file",
    "write_file",
    "file_exists",
    "input",
    "args",
    "now_millis",
    "env",
];

struct Ctx<'a> {
    /// qualified name (`mod::fn`) → decl
    fns: HashMap<String, &'a FnDecl>,
    effect_names: HashSet<String>,
    rows: HashMap<String, BTreeSet<String>>,
    /// module → (alias → target module)
    imports: HashMap<String, HashMap<String, String>>,
}

impl Ctx<'_> {
    fn resolve_call(&self, current: &str, name: &str) -> Option<String> {
        let qual = format!("{current}::{name}");
        if self.fns.contains_key(&qual) {
            Some(qual)
        } else {
            None
        }
    }

    fn resolve_qualified(&self, current: &str, alias: &str, name: &str) -> Option<String> {
        let target = self.imports.get(current)?.get(alias)?;
        let qual = format!("{target}::{name}");
        if self.fns.contains_key(&qual) {
            Some(qual)
        } else {
            None
        }
    }
}

/// Single-module compatibility wrapper.
pub fn check_effects(module: &Module) -> Vec<Diagnostic> {
    let files = vec![ModuleView {
        name: "main".to_string(),
        module,
        imports: HashMap::new(),
    }];
    check_effects_multi(&files)
}

/// The union of every effect performed by a program's functions, ignoring
/// declared rows. Used by the package manager to check a dependency against
/// its manifest's capability ceiling.
/// Every effect name a program declares, plus the built-in rows.
///
/// A package manifest names capabilities as plain strings, so this is what
/// makes them checkable: a ceiling that mentions an effect nobody declared is
/// a typo or a fiction, and either way the manifest is claiming something the
/// code cannot mean.
pub fn effect_vocabulary(program: &crate::loader::Program) -> BTreeSet<String> {
    let mut names: BTreeSet<String> = [IO.to_string(), ASYNC.to_string()].into();
    for file in &program.files {
        for decl in &file.module.decls {
            if let Decl::Effect(e) = decl {
                names.insert(e.name.name.clone());
            }
        }
    }
    names
}

pub fn effects_of_program(program: &crate::loader::Program) -> BTreeSet<String> {
    let views: Vec<ModuleView> = program
        .files
        .iter()
        .map(|f| ModuleView {
            name: f.name.clone(),
            module: &f.module,
            imports: f.imports.clone(),
        })
        .collect();
    let mut ctx = Ctx {
        fns: HashMap::new(),
        effect_names: HashSet::new(),
        rows: HashMap::new(),
        imports: HashMap::new(),
    };
    for f in &views {
        ctx.imports.insert(f.name.clone(), f.imports.clone());
        for d in &f.module.decls {
            match d {
                Decl::Fn(fd) => {
                    ctx.fns.insert(format!("{}::{}", f.name, fd.name.name), fd);
                }
                Decl::Effect(e) => {
                    ctx.effect_names.insert(e.name.name.clone());
                }
                _ => {}
            }
        }
    }
    ctx.rows = ctx.fns.keys().map(|k| (k.clone(), BTreeSet::new())).collect();
    loop {
        let mut changed = false;
        let keys: Vec<String> = ctx.fns.keys().cloned().collect();
        for key in keys {
            let f = ctx.fns[&key];
            let current = key
                .rsplit_once("::")
                .map(|(m, _)| m)
                .unwrap_or("main")
                .to_string();
            let mut row = BTreeSet::new();
            let mut active = Vec::new();
            walk_block(&f.body, &current, &mut active, &ctx, &mut row);
            let entry = ctx.rows.get_mut(&key).unwrap();
            if &row != entry {
                *entry = row;
                changed = true;
            }
        }
        if !changed {
            break;
        }
    }
    let mut all = BTreeSet::new();
    for row in ctx.rows.values() {
        all.extend(row.iter().cloned());
    }
    // Tests and benchmarks may perform effects too.
    for f in &views {
        for d in &f.module.decls {
            let body = match d {
                Decl::Test(t) => &t.body,
                Decl::Bench(b) => &b.body,
                _ => continue,
            };
            let mut row = BTreeSet::new();
            let mut active = Vec::new();
            walk_block(body, &f.name, &mut active, &ctx, &mut row);
            all.extend(row);
        }
    }
    all
}

pub fn check_effects_multi(files: &[ModuleView]) -> Vec<Diagnostic> {
    let mut ctx = Ctx {
        fns: HashMap::new(),
        effect_names: HashSet::new(),
        rows: HashMap::new(),
        imports: HashMap::new(),
    };
    for f in files {
        ctx.imports.insert(f.name.clone(), f.imports.clone());
        for d in &f.module.decls {
            match d {
                Decl::Fn(fd) => {
                    ctx.fns
                        .insert(format!("{}::{}", f.name, fd.name.name), fd);
                }
                Decl::Effect(e) => {
                    ctx.effect_names.insert(e.name.name.clone());
                }
                _ => {}
            }
        }
    }
    ctx.rows = ctx
        .fns
        .keys()
        .map(|k| (k.clone(), BTreeSet::new()))
        .collect();

    // Fixpoint.
    loop {
        let mut changed = false;
        let keys: Vec<String> = ctx.fns.keys().cloned().collect();
        for key in keys {
            let f = ctx.fns[&key];
            let current = key.rsplit_once("::").map(|(m, _)| m).unwrap_or("main").to_string();
            let mut row = BTreeSet::new();
            let mut active = Vec::new();
            walk_block(&f.body, &current, &mut active, &ctx, &mut row);
            let entry = ctx.rows.get_mut(&key).unwrap();
            if &row != entry {
                *entry = row;
                changed = true;
            }
        }
        if !changed {
            break;
        }
    }

    let mut diags = Vec::new();

    // Declared rows are upper bounds.
    for (key, f) in &ctx.fns {
        if let Some(declared) = &f.effects {
            let declared_set: BTreeSet<String> = declared.iter().map(|p| p.dotted()).collect();
            for d in &declared_set {
                let base = d.split('.').next().unwrap_or(d);
                if base != IO && base != ASYNC && !ctx.effect_names.contains(base) {
                    diags.push(Diagnostic::error(
                        f.name.span,
                        format!("function `{}` declares unknown effect `{d}`", f.name.name),
                    ));
                }
            }
            let inferred = &ctx.rows[key];
            let missing: Vec<&String> = inferred
                .iter()
                .filter(|e| {
                    !declared_set.contains(*e)
                        && !declared_set
                            .iter()
                            .any(|d| d.split('.').next() == Some(e.as_str()))
                })
                .collect();
            if !missing.is_empty() {
                let list = missing
                    .iter()
                    .map(|m| format!("`{m}`"))
                    .collect::<Vec<_>>()
                    .join(", ");
                diags.push(
                    Diagnostic::error(
                        f.name.span,
                        format!(
                            "function `{}` performs effect(s) {list} not in its declared row",
                            f.name.name
                        ),
                    )
                    .with_help("add them to the `/ { ... }` clause or handle them inside"),
                );
            }
        }
    }

    // main (entry module only): only `io` may remain unhandled.
    if let Some(main) = ctx.fns.get("main::main") {
        let leftover: Vec<&String> = ctx.rows["main::main"]
            .iter()
            .filter(|e| *e != IO && *e != ASYNC)
            .collect();
        if !leftover.is_empty() {
            let list = leftover
                .iter()
                .map(|m| format!("`{m}`"))
                .collect::<Vec<_>>()
                .join(", ");
            diags.push(
                Diagnostic::error(main.name.span, format!("`main` has unhandled effect(s) {list}"))
                    .with_help("wrap the calls in `with <Effect> handled_by <handler> { ... }`"),
            );
        }
    }

    // Tests and benchmarks: same rule as main.
    for f in files {
        for d in &f.module.decls {
            if let Decl::Test(_) | Decl::Bench(_) = d {
                let (kind, name, body, span) = match d {
                    Decl::Test(t) => ("test", &t.name, &t.body, t.span),
                    Decl::Bench(b) => ("bench", &b.name, &b.body, b.span),
                    _ => unreachable!(),
                };
                let mut row = BTreeSet::new();
                let mut active = Vec::new();
                walk_block(body, &f.name, &mut active, &ctx, &mut row);
                let leftover: Vec<&String> =
                    row.iter().filter(|e| *e != IO && *e != ASYNC).collect();
                if !leftover.is_empty() {
                    let list = leftover
                        .iter()
                        .map(|m| format!("`{m}`"))
                        .collect::<Vec<_>>()
                        .join(", ");
                    diags.push(
                        Diagnostic::error(
                            span,
                            format!("{kind} \"{name}\" has unhandled effect(s) {list}"),
                        )
                        .with_help("install a test handler with `with <Effect> handled_by ...`"),
                    );
                }
            }
        }
    }

    for f in files {
        for d in &f.module.decls {
            let body = match d {
                Decl::Fn(fd) => &fd.body,
                Decl::Test(t) => &t.body,
                Decl::Bench(b) => &b.body,
                _ => continue,
            };
            check_par_map_block(body, &f.name, &ctx, &mut diags);
        }
    }

    diags
}

/// `par_map` runs its function on several cores at once, so an effect inside
/// it would interleave unpredictably between runs. Requiring the row to be
/// empty is exactly what makes the parallelism deterministic — and because
/// effects are inferred, this is a compile error rather than a race nobody can
/// reproduce ([T-PAR-MAP-PURE]).
fn check_par_map_block(b: &Block, current: &str, ctx: &Ctx, diags: &mut Vec<Diagnostic>) {
    let mut calls = Vec::new();
    collect_par_maps_block(b, &mut calls);
    for (func, span) in calls {
        let mut row = BTreeSet::new();
        let mut active = Vec::new();
        walk_expr(func, current, &mut active, ctx, &mut row);
        if row.is_empty() {
            continue;
        }
        let list = row
            .iter()
            .map(|e| format!("`{e}`"))
            .collect::<Vec<_>>()
            .join(", ");
        diags.push(
            Diagnostic::error(
                span,
                format!("`par_map` needs an effect-free function, but this one performs {list}"),
            )
            .with_help(
                "elements are processed on several cores, so an effect would \
interleave differently between runs — use `map` for effectful work",
            ),
        );
    }
}

fn collect_par_maps_block<'e>(b: &'e Block, out: &mut Vec<(&'e Expr, Span)>) {
    for s in &b.stmts {
        match s {
            Stmt::Let { init, .. } => collect_par_maps_expr(init, out),
            Stmt::Assign { target, value, .. } => {
                collect_par_maps_expr(target, out);
                collect_par_maps_expr(value, out);
            }
            Stmt::Return(Some(e), _) | Stmt::Expr(e) => collect_par_maps_expr(e, out),
            Stmt::While { cond, body, .. } => {
                collect_par_maps_expr(cond, out);
                collect_par_maps_block(body, out);
            }
            Stmt::For { iter, body, .. } => {
                collect_par_maps_expr(iter, out);
                collect_par_maps_block(body, out);
            }
            Stmt::Parallel { body, .. } | Stmt::Spawn { body, .. } => {
                collect_par_maps_block(body, out)
            }
            Stmt::With { handler, body, .. } => {
                collect_par_maps_expr(handler, out);
                collect_par_maps_block(body, out);
            }
            _ => {}
        }
    }
}

fn collect_par_maps_expr<'e>(e: &'e Expr, out: &mut Vec<(&'e Expr, Span)>) {
    if let Expr::MethodCall {
        name, args, span, ..
    } = e
    {
        if name.name == "par_map" {
            if let Some(f) = args.first() {
                out.push((f, *span));
            }
        }
    }
    match e {
        Expr::MethodCall { recv, args, .. } => {
            collect_par_maps_expr(recv, out);
            for a in args {
                collect_par_maps_expr(a, out);
            }
        }
        Expr::Call { callee, args, .. } => {
            collect_par_maps_expr(callee, out);
            for a in args {
                collect_par_maps_expr(a, out);
            }
        }
        Expr::Binary { lhs, rhs, .. } => {
            collect_par_maps_expr(lhs, out);
            collect_par_maps_expr(rhs, out);
        }
        Expr::Unary { expr, .. } | Expr::Try { expr, .. } => collect_par_maps_expr(expr, out),
        Expr::Field { recv, .. } => collect_par_maps_expr(recv, out),
        Expr::Index { recv, index, .. } => {
            collect_par_maps_expr(recv, out);
            collect_par_maps_expr(index, out);
        }
        Expr::List(items, _) | Expr::Tuple(items, _) => {
            for i in items {
                collect_par_maps_expr(i, out);
            }
        }
        Expr::Str(pieces, _) => {
            for p in pieces {
                if let StrPiece::Expr(inner) = p {
                    collect_par_maps_expr(inner, out);
                }
            }
        }
        Expr::Closure { body, .. } => collect_par_maps_expr(body, out),
        Expr::Block(b) => collect_par_maps_block(b, out),
        Expr::If {
            cond,
            then_block,
            else_branch,
            ..
        } => {
            collect_par_maps_expr(cond, out);
            collect_par_maps_block(then_block, out);
            match else_branch.as_deref() {
                Some(grasm_syntax::ast::ElseBranch::Else(b)) => collect_par_maps_block(b, out),
                Some(grasm_syntax::ast::ElseBranch::ElseIf(inner)) => {
                    collect_par_maps_expr(inner, out)
                }
                None => {}
            }
        }
        Expr::Match {
            scrutinee, arms, ..
        } => {
            collect_par_maps_expr(scrutinee, out);
            for arm in arms {
                if let Some(g) = &arm.guard {
                    collect_par_maps_expr(g, out);
                }
                collect_par_maps_expr(&arm.body, out);
            }
        }
        _ => {}
    }
}

fn contribute(qual: &str, active: &[String], ctx: &Ctx, out: &mut BTreeSet<String>) {
    if let Some(row) = ctx.rows.get(qual) {
        for e in row {
            if !active.contains(e) {
                out.insert(e.clone());
            }
        }
    }
}

fn walk_block(
    b: &Block,
    current: &str,
    active: &mut Vec<String>,
    ctx: &Ctx,
    out: &mut BTreeSet<String>,
) {
    for s in &b.stmts {
        walk_stmt(s, current, active, ctx, out);
    }
}

fn walk_stmt(
    s: &Stmt,
    current: &str,
    active: &mut Vec<String>,
    ctx: &Ctx,
    out: &mut BTreeSet<String>,
) {
    match s {
        Stmt::Let { init, .. } => walk_expr(init, current, active, ctx, out),
        Stmt::Assign { target, value, .. } => {
            walk_expr(target, current, active, ctx, out);
            walk_expr(value, current, active, ctx, out);
        }
        Stmt::Expr(e) => walk_expr(e, current, active, ctx, out),
        Stmt::Return(v, _) => {
            if let Some(e) = v {
                walk_expr(e, current, active, ctx, out);
            }
        }
        Stmt::Break(_) | Stmt::Continue(_) => {}
        Stmt::While { cond, body, .. } => {
            walk_expr(cond, current, active, ctx, out);
            walk_block(body, current, active, ctx, out);
        }
        Stmt::For { iter, body, .. } => {
            walk_expr(iter, current, active, ctx, out);
            walk_block(body, current, active, ctx, out);
        }
        Stmt::Parallel { body, .. } => {
            // Structured concurrency is the `async` effect.
            if !active.iter().any(|a| a == ASYNC) {
                out.insert(ASYNC.to_string());
            }
            walk_block(body, current, active, ctx, out);
        }
        Stmt::Spawn { body, .. } => {
            if !active.iter().any(|a| a == ASYNC) {
                out.insert(ASYNC.to_string());
            }
            walk_block(body, current, active, ctx, out);
        }
        Stmt::With {
            effect,
            handler,
            body,
            ..
        } => {
            walk_expr(handler, current, active, ctx, out);
            active.push(effect.dotted());
            walk_block(body, current, active, ctx, out);
            active.pop();
        }
    }
}

fn walk_expr(
    e: &Expr,
    current: &str,
    active: &mut Vec<String>,
    ctx: &Ctx,
    out: &mut BTreeSet<String>,
) {
    match e {
        Expr::Int(..) | Expr::Float(..) | Expr::Bool(..) | Expr::Ident(_) => {}
        Expr::Str(pieces, _) => {
            for p in pieces {
                if let StrPiece::Expr(inner) = p {
                    walk_expr(inner, current, active, ctx, out);
                }
            }
        }
        Expr::List(items, _) | Expr::Tuple(items, _) => {
            for i in items {
                walk_expr(i, current, active, ctx, out);
            }
        }
        Expr::StructLit { fields, .. } => {
            for (_, v) in fields {
                walk_expr(v, current, active, ctx, out);
            }
        }
        Expr::Call { callee, args, .. } => {
            if let Expr::Ident(id) = &**callee {
                if IO_BUILTINS.contains(&id.name.as_str()) && !active.iter().any(|a| a == IO) {
                    out.insert(IO.to_string());
                }
                if let Some(qual) = ctx.resolve_call(current, &id.name) {
                    contribute(&qual, active, ctx, out);
                }
            } else {
                walk_expr(callee, current, active, ctx, out);
            }
            for a in args {
                walk_expr(a, current, active, ctx, out);
            }
        }
        Expr::MethodCall {
            recv, name, args, ..
        } => {
            if let Expr::Ident(tid) = &**recv {
                if ctx.effect_names.contains(&tid.name) {
                    let ename = tid.name.clone();
                    if !active.contains(&ename) {
                        out.insert(ename);
                    }
                } else if let Some(qual) = ctx.resolve_qualified(current, &tid.name, &name.name) {
                    // Cross-module call `utils.helper(...)`.
                    contribute(&qual, active, ctx, out);
                } else {
                    walk_expr(recv, current, active, ctx, out);
                }
            } else {
                walk_expr(recv, current, active, ctx, out);
            }
            // UFCS / callback-taking builtin: same-module named fn contributes.
            if let Some(qual) = ctx.resolve_call(current, &name.name) {
                contribute(&qual, active, ctx, out);
            }
            for a in args {
                walk_expr(a, current, active, ctx, out);
            }
        }
        Expr::Field { recv, .. } => walk_expr(recv, current, active, ctx, out),
        Expr::Index { recv, index, .. } => {
            walk_expr(recv, current, active, ctx, out);
            walk_expr(index, current, active, ctx, out);
        }
        Expr::Unary { expr, .. } | Expr::Try { expr, .. } => {
            walk_expr(expr, current, active, ctx, out)
        }
        Expr::Binary { lhs, rhs, .. } => {
            walk_expr(lhs, current, active, ctx, out);
            walk_expr(rhs, current, active, ctx, out);
        }
        Expr::Range { start, end, .. } => {
            walk_expr(start, current, active, ctx, out);
            walk_expr(end, current, active, ctx, out);
        }
        Expr::If {
            cond,
            then_block,
            else_branch,
            ..
        } => {
            walk_expr(cond, current, active, ctx, out);
            walk_block(then_block, current, active, ctx, out);
            if let Some(eb) = else_branch {
                match &**eb {
                    ElseBranch::Else(b) => walk_block(b, current, active, ctx, out),
                    ElseBranch::ElseIf(e) => walk_expr(e, current, active, ctx, out),
                }
            }
        }
        Expr::Match { scrutinee, arms, .. } => {
            walk_expr(scrutinee, current, active, ctx, out);
            for arm in arms {
                if let Some(g) = &arm.guard {
                    walk_expr(g, current, active, ctx, out);
                }
                walk_expr(&arm.body, current, active, ctx, out);
            }
        }
        Expr::Block(b) => walk_block(b, current, active, ctx, out),
        Expr::Closure { body, .. } => walk_expr(body, current, active, ctx, out),
    }
}
