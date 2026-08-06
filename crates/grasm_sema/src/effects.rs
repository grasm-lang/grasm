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
            let mut fnb = FnBindings::default();
            seed_params(&f.params, &mut fnb);
            collect_fn_bindings(&f.body, &ctx, &current, &mut fnb);
            walk_block(&f.body, &current, &mut active, &ctx, &fnb, &mut row);
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
            let mut fnb = FnBindings::default();
            collect_fn_bindings(body, &ctx, &f.name, &mut fnb);
            walk_block(body, &f.name, &mut active, &ctx, &fnb, &mut row);
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
            let mut fnb = FnBindings::default();
            seed_params(&f.params, &mut fnb);
            collect_fn_bindings(&f.body, &ctx, &current, &mut fnb);
            walk_block(&f.body, &current, &mut active, &ctx, &fnb, &mut row);
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
                let mut fnb = FnBindings::default();
                collect_fn_bindings(body, &ctx, &f.name, &mut fnb);
                walk_block(body, &f.name, &mut active, &ctx, &fnb, &mut row);
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
            let (body, params): (&Block, &[Param]) = match d {
                Decl::Fn(fd) => (&fd.body, &fd.params),
                Decl::Test(t) => (&t.body, &[]),
                Decl::Bench(b) => (&b.body, &[]),
                _ => continue,
            };
            let mut fnb = FnBindings::default();
            seed_params(params, &mut fnb);
            collect_fn_bindings(body, &ctx, &f.name, &mut fnb);
            check_par_map_block(body, &f.name, &ctx, &fnb, &mut diags);
            check_fn_args_block(body, &f.name, &ctx, &fnb, &mut diags);
        }
    }

    diags
}

/// The effect row of the function handed to a parallel operation.
///
/// Walking the argument as an ordinary expression is not enough, and getting
/// this wrong is not a missed diagnostic — it is the safety claim failing
/// silently. An identifier contributes no effects *as an expression*, because
/// naming a value does not perform anything. But here the identifier **is** the
/// function that is about to run on several cores, so what counts is its row,
/// not the row of the mention.
///
/// Before this, only a closure written inline at the call site was checked.
/// `xs.par_map(announce)` compiled and ran a printing function across threads —
/// even with `/ {io}` declared on it in plain sight.
fn par_fn_row(
    func: &Expr,
    current: &str,
    ctx: &Ctx,
    fnb: &FnBindings,
    active: &mut Vec<String>,
    out: &mut BTreeSet<String>,
) {
    if let Expr::Ident(id) = func {
        if let Some(qual) = ctx.resolve_call(current, &id.name) {
            if let Some(row) = ctx.rows.get(&qual) {
                out.extend(row.iter().cloned());
            }
            return;
        }
        // Not a top-level function: a local holding one, or holding several.
        indirect_row(&id.name, current, active, ctx, fnb, out);
        return;
    }
    walk_expr(func, current, active, ctx, fnb, out);
}

/// `par_map` runs its function on several cores at once, so an effect inside
/// it would interleave unpredictably between runs. Requiring the row to be
/// empty is exactly what makes the parallelism deterministic — and because
/// effects are inferred, this is a compile error rather than a race nobody can
/// reproduce ([T-PAR-MAP-PURE]).
/// A function passed where a function *type* is expected must fit that type's
/// declared row.
///
/// This is the only place the row can be checked. A function type is written,
/// not inferred, so nothing else compares what the caller hands over against
/// what the callee promised — and without it `fn(i64) -> i64`, which says it
/// performs nothing, would accept a function that prints.
fn check_fn_args_block(
    b: &Block,
    current: &str,
    ctx: &Ctx,
    fnb: &FnBindings,
    diags: &mut Vec<Diagnostic>,
) {
    let mut calls = Vec::new();
    collect_calls_block(b, &mut calls);
    for (callee, args, _) in calls {
        let Some(qual) = ctx.resolve_call(current, callee) else {
            continue;
        };
        let decl = ctx.fns[&qual];
        for (arg, param) in args.iter().zip(&decl.params) {
            let Some(allowed) = declared_row(&param.ty) else {
                continue;
            };
            let mut row = BTreeSet::new();
            let mut active = Vec::new();
            par_fn_row(arg, current, ctx, fnb, &mut active, &mut row);
            let over: Vec<&String> = row.difference(&allowed).collect();
            if over.is_empty() {
                continue;
            }
            let list = over
                .iter()
                .map(|e| format!("`{e}`"))
                .collect::<Vec<_>>()
                .join(", ");
            let shown = if allowed.is_empty() {
                "performs nothing".to_string()
            } else {
                format!(
                    "performs only {}",
                    allowed
                        .iter()
                        .map(|e| format!("`{e}`"))
                        .collect::<Vec<_>>()
                        .join(", ")
                )
            };
            diags.push(
                Diagnostic::error(
                    arg.span(),
                    format!(
                        "`{}` expects a function that {shown}, but this one performs {list}",
                        param.name.name
                    ),
                )
                .with_help(
                    "widen the parameter's type — `fn(..) -> .. / {io}` — or pass a function that does not perform it",
                ),
            );
        }
    }
}

/// Every direct call in a block, as (callee name, args, span).
fn collect_calls_block<'e>(b: &'e Block, out: &mut Vec<(&'e str, &'e [Expr], Span)>) {
    for s in &b.stmts {
        collect_calls_stmt(s, out);
    }
}

fn collect_calls_stmt<'e>(s: &'e Stmt, out: &mut Vec<(&'e str, &'e [Expr], Span)>) {
    match s {
        Stmt::Let { init, .. } => collect_calls_expr(init, out),
        Stmt::Assign { target, value, .. } => {
            collect_calls_expr(target, out);
            collect_calls_expr(value, out);
        }
        Stmt::Return(Some(e), _) | Stmt::Expr(e) => collect_calls_expr(e, out),
        Stmt::While { cond, body, .. } => {
            collect_calls_expr(cond, out);
            collect_calls_block(body, out);
        }
        Stmt::For { iter, body, .. } => {
            collect_calls_expr(iter, out);
            collect_calls_block(body, out);
        }
        Stmt::Parallel { body, .. } | Stmt::Spawn { body, .. } => collect_calls_block(body, out),
        Stmt::With { handler, body, .. } => {
            collect_calls_expr(handler, out);
            collect_calls_block(body, out);
        }
        _ => {}
    }
}

fn collect_calls_expr<'e>(e: &'e Expr, out: &mut Vec<(&'e str, &'e [Expr], Span)>) {
    match e {
        Expr::Call { callee, args, span } => {
            if let Expr::Ident(id) = &**callee {
                out.push((id.name.as_str(), args.as_slice(), *span));
            }
            for a in args {
                collect_calls_expr(a, out);
            }
        }
        Expr::Str(pieces, _) => {
            for p in pieces {
                if let StrPiece::Expr(i) = p {
                    collect_calls_expr(i, out);
                }
            }
        }
        Expr::List(items, _) | Expr::Tuple(items, _) => {
            for i in items {
                collect_calls_expr(i, out);
            }
        }
        Expr::Binary { lhs, rhs, .. } => {
            collect_calls_expr(lhs, out);
            collect_calls_expr(rhs, out);
        }
        Expr::Unary { expr, .. } | Expr::Try { expr, .. } => collect_calls_expr(expr, out),
        Expr::MethodCall { recv, args, .. } => {
            collect_calls_expr(recv, out);
            for a in args {
                collect_calls_expr(a, out);
            }
        }
        Expr::Index { recv, index, .. } => {
            collect_calls_expr(recv, out);
            collect_calls_expr(index, out);
        }
        Expr::Field { recv, .. } => collect_calls_expr(recv, out),
        Expr::If {
            cond,
            then_block,
            else_branch,
            ..
        } => {
            collect_calls_expr(cond, out);
            collect_calls_block(then_block, out);
            match else_branch.as_deref() {
                Some(ElseBranch::Else(b)) => collect_calls_block(b, out),
                Some(ElseBranch::ElseIf(inner)) => collect_calls_expr(inner, out),
                None => {}
            }
        }
        Expr::Block(b) => collect_calls_block(b, out),
        Expr::Match { scrutinee, arms, .. } => {
            collect_calls_expr(scrutinee, out);
            for a in arms {
                collect_calls_expr(&a.body, out);
            }
        }
        Expr::Closure { body, .. } => collect_calls_expr(body, out),
        _ => {}
    }
}

fn check_par_map_block(
    b: &Block,
    current: &str,
    ctx: &Ctx,
    fnb: &FnBindings,
    diags: &mut Vec<Diagnostic>,
) {

    let mut calls = Vec::new();
    collect_par_maps_block(b, &mut calls);
    for (func, span) in calls {
        let mut row = BTreeSet::new();
        let mut active = Vec::new();
        par_fn_row(func, current, ctx, fnb, &mut active, &mut row);
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
                format!("this parallel operation needs an effect-free function, but the one given performs {list}"),
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
        if name.name == "par_map" || name.name == "par_filter" {
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


/// A function value a body can reach by name.
enum FnSource<'a> {
    /// A top-level function, by its unqualified name.
    Named(String),
    /// A closure written in the body.
    Closure(&'a Expr),
    /// A parameter of function type. Its row is written on the type, because
    /// there is no body here to infer one from — that declaration is the
    /// contract the caller is held to.
    Declared(BTreeSet<String>),
}

/// Function values bound to names in one function body.
///
/// Effect inference was written before function values existed, so a call
/// through a name that is not a top-level function contributed nothing —
/// `let f = shout; f(n)` performed no effects as far as the checker knew, even
/// inside a declared-empty row. Resolving the binding is what closes that.
#[derive(Default)]
struct FnBindings<'a> {
    by_name: HashMap<String, Vec<FnSource<'a>>>,
    /// Every function value the body mentions anywhere. Used when a callee
    /// cannot be pinned to one binding: over-approximating there is the whole
    /// point, since under-approximating is what let effects escape.
    any: Vec<FnSource<'a>>,
}

impl<'a> FnBindings<'a> {
    fn sources_for(&self, name: &str) -> &[FnSource<'a>] {
        match self.by_name.get(name) {
            Some(v) => v,
            None => &self.any,
        }
    }
}

/// Function values reachable from an initializer expression.
fn fn_sources<'a>(e: &'a Expr, ctx: &Ctx, current: &str, out: &mut Vec<FnSource<'a>>) {
    match e {
        Expr::Ident(id) => {
            if ctx.resolve_call(current, &id.name).is_some() {
                out.push(FnSource::Named(id.name.clone()));
            }
        }
        Expr::Closure { .. } => out.push(FnSource::Closure(e)),
        Expr::List(items, _) | Expr::Tuple(items, _) => {
            for i in items {
                fn_sources(i, ctx, current, out);
            }
        }
        _ => {}
    }
}

/// The effect row written on a function type, or the empty row when none is.
///
/// Empty is the meaningful default: `fn(i64) -> i64` accepts only functions
/// that perform nothing, so an effectful one passed there is an error rather
/// than a silent widening of whatever row the caller declared.
fn declared_row(te: &TypeExpr) -> Option<BTreeSet<String>> {
    match te {
        TypeExpr::Fn { effects, .. } => Some(effects.iter().map(|p| p.dotted()).collect()),
        _ => None,
    }
}

/// Parameters of function type, so calling one is attributed to its declared
/// row instead of to nothing.
fn seed_params<'a>(params: &[Param], fnb: &mut FnBindings<'a>) {
    for p in params {
        if let Some(row) = declared_row(&p.ty) {
            fnb.by_name
                .insert(p.name.name.clone(), vec![FnSource::Declared(row)]);
        }
    }
}

/// Collects, in statement order, every name a body binds to a function value.
fn collect_fn_bindings<'a>(b: &'a Block, ctx: &Ctx, current: &str, fnb: &mut FnBindings<'a>) {
    for stmt in &b.stmts {
        match stmt {
            Stmt::Let { name, init, .. } => {
                let mut found = Vec::new();
                fn_sources(init, ctx, current, &mut found);
                if !found.is_empty() {
                    fnb.any.extend(found.iter().map(FnSource::clone_of));
                    fnb.by_name.insert(name.name.clone(), found);
                }
            }
            Stmt::For { var, iter, body, .. } => {
                // `for f in fs` where `fs` holds function values.
                let mut found = Vec::new();
                match iter {
                    Expr::Ident(id) => {
                        if let Some(src) = fnb.by_name.get(&id.name) {
                            found.extend(src.iter().map(FnSource::clone_of));
                        }
                    }
                    other => fn_sources(other, ctx, current, &mut found),
                }
                if !found.is_empty() {
                    fnb.any.extend(found.iter().map(FnSource::clone_of));
                    fnb.by_name.insert(var.name.clone(), found);
                }
                collect_fn_bindings(body, ctx, current, fnb);
            }
            Stmt::While { body, .. }
            | Stmt::Parallel { body, .. }
            | Stmt::Spawn { body, .. }
            | Stmt::With { body, .. } => collect_fn_bindings(body, ctx, current, fnb),
            _ => {}
        }
    }
}

impl<'a> FnSource<'a> {
    fn clone_of(&self) -> FnSource<'a> {
        match self {
            FnSource::Named(n) => FnSource::Named(n.clone()),
            FnSource::Closure(e) => FnSource::Closure(e),
            FnSource::Declared(r) => FnSource::Declared(r.clone()),
        }
    }
}

/// The effects a call through `name` may perform, when `name` is not a
/// top-level function but a local holding a function value.
fn indirect_row(
    name: &str,
    current: &str,
    active: &mut Vec<String>,
    ctx: &Ctx,
    fnb: &FnBindings,
    out: &mut BTreeSet<String>,
) {
    for src in fnb.sources_for(name) {
        match src {
            FnSource::Named(n) => {
                if let Some(qual) = ctx.resolve_call(current, n) {
                    contribute(&qual, active, ctx, out);
                }
            }
            FnSource::Declared(row) => out.extend(row.iter().cloned()),
            FnSource::Closure(e) => {
                // A closure has no name to guard recursion with, so its
                // identity stands in for one.
                let guard = format!("closure@{:p}", *e as *const Expr);
                if active.contains(&guard) {
                    continue;
                }
                active.push(guard);
                if let Expr::Closure { body, .. } = e {
                    walk_expr(body, current, active, ctx, fnb, out);
                }
                active.pop();
            }
        }
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
    fnb: &FnBindings,
    out: &mut BTreeSet<String>,
) {
    for s in &b.stmts {
        walk_stmt(s, current, active, ctx, fnb, out);
    }
}

fn walk_stmt(
    s: &Stmt,
    current: &str,
    active: &mut Vec<String>,
    ctx: &Ctx,
    fnb: &FnBindings,
    out: &mut BTreeSet<String>,
) {
    match s {
        Stmt::Let { init, .. } | Stmt::LetTuple { init, .. } => {
            walk_expr(init, current, active, ctx, fnb, out)
        }
        Stmt::Assign { target, value, .. } => {
            walk_expr(target, current, active, ctx, fnb, out);
            walk_expr(value, current, active, ctx, fnb, out);
        }
        Stmt::Expr(e) => walk_expr(e, current, active, ctx, fnb, out),
        Stmt::Return(v, _) => {
            if let Some(e) = v {
                walk_expr(e, current, active, ctx, fnb, out);
            }
        }
        Stmt::Break(_) | Stmt::Continue(_) => {}
        Stmt::While { cond, body, .. } => {
            walk_expr(cond, current, active, ctx, fnb, out);
            walk_block(body, current, active, ctx, fnb, out);
        }
        Stmt::For { iter, body, .. } => {
            walk_expr(iter, current, active, ctx, fnb, out);
            walk_block(body, current, active, ctx, fnb, out);
        }
        Stmt::Parallel { body, .. } => {
            // Structured concurrency is the `async` effect.
            if !active.iter().any(|a| a == ASYNC) {
                out.insert(ASYNC.to_string());
            }
            walk_block(body, current, active, ctx, fnb, out);
        }
        Stmt::Spawn { body, .. } => {
            if !active.iter().any(|a| a == ASYNC) {
                out.insert(ASYNC.to_string());
            }
            walk_block(body, current, active, ctx, fnb, out);
        }
        Stmt::With {
            effect,
            handler,
            body,
            ..
        } => {
            walk_expr(handler, current, active, ctx, fnb, out);
            active.push(effect.dotted());
            walk_block(body, current, active, ctx, fnb, out);
            active.pop();
        }
    }
}

fn walk_expr(
    e: &Expr,
    current: &str,
    active: &mut Vec<String>,
    ctx: &Ctx,
    fnb: &FnBindings,
    out: &mut BTreeSet<String>,
) {
    match e {
        Expr::Int(..) | Expr::Float(..) | Expr::Bool(..) | Expr::Ident(_) => {}
        Expr::Str(pieces, _) => {
            for p in pieces {
                if let StrPiece::Expr(inner) = p {
                    walk_expr(inner, current, active, ctx, fnb, out);
                }
            }
        }
        Expr::List(items, _) | Expr::Tuple(items, _) => {
            for i in items {
                walk_expr(i, current, active, ctx, fnb, out);
            }
        }
        Expr::StructLit { fields, .. } => {
            for (_, v) in fields {
                walk_expr(v, current, active, ctx, fnb, out);
            }
        }
        Expr::Call { callee, args, .. } => {
            if let Expr::Ident(id) = &**callee {
                if IO_BUILTINS.contains(&id.name.as_str()) && !active.iter().any(|a| a == IO) {
                    out.insert(IO.to_string());
                }
                if let Some(qual) = ctx.resolve_call(current, &id.name) {
                    contribute(&qual, active, ctx, out);
                } else {
                    // A call through a name that is not a top-level function:
                    // a local holding a function value. Whatever that value
                    // may be, calling it performs what it performs.
                    indirect_row(&id.name, current, active, ctx, fnb, out);
                }
            } else {
                walk_expr(callee, current, active, ctx, fnb, out);
            }
            for a in args {
                walk_expr(a, current, active, ctx, fnb, out);
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
                    walk_expr(recv, current, active, ctx, fnb, out);
                }
            } else {
                walk_expr(recv, current, active, ctx, fnb, out);
            }
            // UFCS / callback-taking builtin: same-module named fn contributes.
            if let Some(qual) = ctx.resolve_call(current, &name.name) {
                contribute(&qual, active, ctx, out);
            }
            for a in args {
                walk_expr(a, current, active, ctx, fnb, out);
            }
        }
        Expr::Field { recv, .. } => walk_expr(recv, current, active, ctx, fnb, out),
        Expr::Index { recv, index, .. } => {
            walk_expr(recv, current, active, ctx, fnb, out);
            walk_expr(index, current, active, ctx, fnb, out);
        }
        Expr::Unary { expr, .. } | Expr::Try { expr, .. } => {
            walk_expr(expr, current, active, ctx, fnb, out)
        }
        Expr::Binary { lhs, rhs, .. } => {
            walk_expr(lhs, current, active, ctx, fnb, out);
            walk_expr(rhs, current, active, ctx, fnb, out);
        }
        Expr::Range { start, end, .. } => {
            walk_expr(start, current, active, ctx, fnb, out);
            walk_expr(end, current, active, ctx, fnb, out);
        }
        Expr::If {
            cond,
            then_block,
            else_branch,
            ..
        } => {
            walk_expr(cond, current, active, ctx, fnb, out);
            walk_block(then_block, current, active, ctx, fnb, out);
            if let Some(eb) = else_branch {
                match &**eb {
                    ElseBranch::Else(b) => walk_block(b, current, active, ctx, fnb, out),
                    ElseBranch::ElseIf(e) => walk_expr(e, current, active, ctx, fnb, out),
                }
            }
        }
        Expr::Match { scrutinee, arms, .. } => {
            walk_expr(scrutinee, current, active, ctx, fnb, out);
            for arm in arms {
                if let Some(g) = &arm.guard {
                    walk_expr(g, current, active, ctx, fnb, out);
                }
                walk_expr(&arm.body, current, active, ctx, fnb, out);
            }
        }
        Expr::Block(b) => walk_block(b, current, active, ctx, fnb, out),
        Expr::Closure { body, .. } => walk_expr(body, current, active, ctx, fnb, out),
    }
}
