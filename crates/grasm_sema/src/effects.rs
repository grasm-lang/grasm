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
pub const UNSAFE: &str = "unsafe";
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
    /// `module::name` for every declared C function.
    ///
    /// What a C function does is unknowable, so calling one performs `io`.
    /// Without that an `extern` call escaped both the capability ceiling and
    /// [T-PAR-MAP-PURE] — `par_map` accepted a function calling `getpid`, and
    /// returned different answers on different elements.
    externs: HashSet<String>,
    rows: HashMap<String, BTreeSet<String>>,
    /// module → (alias → target module)
    imports: HashMap<String, HashMap<String, String>>,
    /// field name → the union of every effect row written on a function-typed
    /// field of that name, anywhere in the program.
    ///
    /// [T-FN-TYPE-ROW-CARRIED] says a function type carries its row wherever
    /// it goes. Storing one into a field was checked; calling it back *out*
    /// was not, and inference is syntactic, so it has no type to read the row
    /// off `b.f(x)`. The declaration is where the row is written, so that is
    /// where it is harvested. Two structs with a same-named function field
    /// contribute to each other, which over-approximates — the safe direction,
    /// since under-approximating is what let a `/ {}` function print.
    field_rows: HashMap<String, BTreeSet<String>>,
    /// Every row written on a function type that is *not* a struct field: a
    /// list element, a map value, a tuple component, a generic argument. There
    /// is no name to key those on, so calling through an index or any other
    /// computed callee contributes all of them.
    elem_rows: BTreeSet<String>,
    /// effect name → what any handler for it, written anywhere in the
    /// program, performs.
    ///
    /// [E-HANDLER-OWN-ROW] says a handler's own effects are counted where it
    /// is *installed*. They were counted where it was *constructed*, which is
    /// the same place only when the handler is written at the `with`. The
    /// moment one crossed a function boundary its effects disappeared:
    /// `fn run(h: LogHandler) / {}` installed a printing handler and the
    /// declared-empty row held. Since the handler type says nothing about
    /// what its operations do, installing one that was built elsewhere
    /// charges what *any* handler for that effect does — over-approximate,
    /// and the safe direction.
    handler_rows: HashMap<String, BTreeSet<String>>,
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
    let mut names: BTreeSet<String> =
        [IO.to_string(), ASYNC.to_string(), UNSAFE.to_string()].into();
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
        externs: HashSet::new(),
        rows: HashMap::new(),
        imports: HashMap::new(),
        field_rows: HashMap::new(),
        elem_rows: BTreeSet::new(),
        handler_rows: HashMap::new(),
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
                Decl::Extern(x) => {
                    ctx.externs.insert(format!("{}::{}", f.name, x.name.name));
                }
                _ => {}
            }
        }
    }
    for f in &views {
        harvest_module(f.module, &mut ctx.field_rows, &mut ctx.elem_rows);
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
    // What the package needs from *outside* itself: the entry point's row,
    // which is what running it takes from the environment, and every `pub`
    // item's row, which is what a consumer has to supply. A private function's
    // effects are not skipped — rows are transitive, so they already appear in
    // whatever calls it — but they stop counting once something in between
    // *handles* them, which is the point.
    //
    // The union over every function counted a handled effect anyway. A package
    // that declared `effect Cfg`, performed it in a private helper, handled it
    // three lines later in `main` and never let it out of its own source
    // failed to build against `capabilities = ["io"]`, demanding a capability
    // named `Cfg`. The only way through was to publish an internal name in the
    // manifest. An effect that never escapes is not a capability; one that
    // does — `pub fn grab(u) -> str / {Network}` — still is.
    let entry_module = program
        .files
        .first()
        .map(|f| f.name.as_str())
        .unwrap_or("main");
    // A dependency's modules are loaded into the same program, and its
    // ceiling is its own business. Counting its `pub` rows here charged this
    // package for effects it never names: an app whose `main` calls one pure
    // function from a library was charged for a *different* `pub` function in
    // that library it does not use, at a source location it cannot fix. What
    // the consumer actually calls still counts — rows are transitive, so it
    // is already in the row of whatever calls it.
    let foreign = |module: &str| {
        program
            .deps
            .iter()
            .any(|d| module == d || module.starts_with(&format!("{d}.")))
    };
    let mut all = BTreeSet::new();
    for (key, row) in &ctx.rows {
        let Some((module, name)) = key.rsplit_once("::") else {
            continue;
        };
        if foreign(module) {
            continue;
        }
        let escapes = ctx.fns[key].is_pub || (module == entry_module && name == "main");
        if escapes {
            all.extend(row.iter().cloned());
        }
    }
    // Tests and benchmarks count. They are not part of the package's
    // interface, and a reader reasonably expects the ceiling to be about what
    // *escapes* — but `grasm test` runs the tests of every imported module,
    // including a dependency's, so a test body really does execute in a
    // consumer's process. A `capabilities = []` package that printed from a
    // test would print in someone else's test run.
    //
    // The cost is real: such a package cannot `print` from a test at all,
    // which is the one place a careful library most wants to. That is a
    // restriction to lift by making the ceiling distinguish "runs when a
    // consumer builds" from "runs when a consumer tests", not by dropping the
    // check.
    for f in &views {
        if foreign(&f.name) {
            continue;
        }
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

/// Does this expression take a reference anywhere inside it?
fn makes_ref(e: &Expr) -> bool {
    let mut found = false;
    let mut idents = Vec::new();
    collect_calls_expr(e, &mut idents);
    for (name, _, _) in &idents {
        if *name == "ref" {
            found = true;
        }
    }
    found
}

/// The shared context both effect passes work from.
fn build_ctx<'a>(files: &'a [ModuleView<'a>]) -> Ctx<'a> {
    let mut ctx = Ctx {
        fns: HashMap::new(),
        effect_names: HashSet::new(),
        externs: HashSet::new(),
        rows: HashMap::new(),
        imports: HashMap::new(),
        field_rows: HashMap::new(),
        elem_rows: BTreeSet::new(),
        handler_rows: HashMap::new(),
    };
    for f in files {
        ctx.imports.insert(f.name.clone(), f.imports.clone());
        for d in &f.module.decls {
            match d {
                Decl::Fn(fd) => {
                    ctx.fns.insert(format!("{}::{}", f.name, fd.name.name), fd);
                }
                Decl::Effect(e) => {
                    ctx.effect_names.insert(e.name.name.clone());
                }
                Decl::Extern(x) => {
                    ctx.externs.insert(format!("{}::{}", f.name, x.name.name));
                }
                _ => {}
            }
        }
    }
    for f in files {
        harvest_module(f.module, &mut ctx.field_rows, &mut ctx.elem_rows);
    }
    ctx.rows = ctx.fns.keys().map(|k| (k.clone(), BTreeSet::new())).collect();
    ctx
}

/// Runs the row fixpoint to completion.
fn solve_rows(ctx: &mut Ctx) {
    // Two interlocking fixpoints: a function's row depends on what the
    // handlers it installs do, and a handler's operations are code whose row
    // depends on the functions it calls. The outer loop runs until neither
    // moves.
    loop {
        solve_fn_rows(ctx);
        let found = compute_handler_rows(ctx);
        if found == ctx.handler_rows {
            break;
        }
        ctx.handler_rows = found;
        for v in ctx.rows.values_mut() {
            v.clear();
        }
    }
}

/// What every handler literal in the program performs, keyed by its effect.
fn compute_handler_rows(ctx: &Ctx) -> HashMap<String, BTreeSet<String>> {
    let mut out: HashMap<String, BTreeSet<String>> = HashMap::new();
    let keys: Vec<String> = ctx.fns.keys().cloned().collect();
    for key in keys {
        let f = ctx.fns[&key];
        let current = key.rsplit_once("::").map(|(m, _)| m).unwrap_or("main").to_string();
        let mut lits: Vec<&Expr> = Vec::new();
        collect_handler_lits_block(&f.body, &mut lits);
        for lit in lits {
            let Expr::StructLit { name, .. } = lit else {
                continue;
            };
            let tname = name.segments.last().unwrap().name.clone();
            let Some(effect) = tname.strip_suffix("Handler") else {
                continue;
            };
            if !ctx.effect_names.contains(effect) {
                continue;
            }
            let mut row = BTreeSet::new();
            let mut active = Vec::new();
            let mut fnb = FnBindings::default();
            seed_params(&f.params, &mut fnb);
            collect_fn_bindings(&f.body, ctx, &current, &mut fnb);
            walk_expr(lit, &current, &mut active, ctx, &fnb, &mut row);
            out.entry(effect.to_string()).or_default().extend(row);
        }
    }
    out
}

/// Every `EHandler{ … }` literal reachable from a block.
fn collect_handler_lits_block<'e>(b: &'e Block, out: &mut Vec<&'e Expr>) {
    for st in &b.stmts {
        match st {
            Stmt::Let { init, .. } => collect_handler_lits(init, out),
            Stmt::Assign { value, .. } => collect_handler_lits(value, out),
            Stmt::Return(Some(e), _) | Stmt::Expr(e) => collect_handler_lits(e, out),
            Stmt::While { cond, body, .. } => {
                collect_handler_lits(cond, out);
                collect_handler_lits_block(body, out);
            }
            Stmt::For { iter, body, .. } => {
                collect_handler_lits(iter, out);
                collect_handler_lits_block(body, out);
            }
            Stmt::With { handler, body, .. } => {
                collect_handler_lits(handler, out);
                collect_handler_lits_block(body, out);
            }
            Stmt::Parallel { body, .. } | Stmt::Spawn { body, .. } => {
                collect_handler_lits_block(body, out)
            }
            _ => {}
        }
    }
}

fn collect_handler_lits<'e>(e: &'e Expr, out: &mut Vec<&'e Expr>) {
    match e {
        Expr::StructLit { fields, base, .. } => {
            out.push(e);
            if let Some(b) = base {
                collect_handler_lits(b, out);
            }
            for (_, v) in fields {
                collect_handler_lits(v, out);
            }
        }
        Expr::Call { callee, args, .. } => {
            collect_handler_lits(callee, out);
            for a in args {
                collect_handler_lits(a, out);
            }
        }
        Expr::MethodCall { recv, args, .. } => {
            collect_handler_lits(recv, out);
            for a in args {
                collect_handler_lits(a, out);
            }
        }
        Expr::List(items, _) | Expr::Tuple(items, _) => {
            for i in items {
                collect_handler_lits(i, out);
            }
        }
        Expr::Map(entries, _) => {
            for (k, v) in entries {
                collect_handler_lits(k, out);
                collect_handler_lits(v, out);
            }
        }
        Expr::Binary { lhs, rhs, .. } => {
            collect_handler_lits(lhs, out);
            collect_handler_lits(rhs, out);
        }
        Expr::Unary { expr, .. } | Expr::Try { expr, .. } => collect_handler_lits(expr, out),
        Expr::Field { recv, .. } => collect_handler_lits(recv, out),
        Expr::Index { recv, index, .. } => {
            collect_handler_lits(recv, out);
            collect_handler_lits(index, out);
        }
        Expr::Closure { body, .. } => collect_handler_lits(body, out),
        Expr::Block(b) | Expr::Unsafe(b) | Expr::Region(b) => collect_handler_lits_block(b, out),
        Expr::If {
            cond,
            then_block,
            else_branch,
            ..
        } => {
            collect_handler_lits(cond, out);
            collect_handler_lits_block(then_block, out);
            match else_branch.as_deref() {
                Some(ElseBranch::Else(b)) => collect_handler_lits_block(b, out),
                Some(ElseBranch::ElseIf(inner)) => collect_handler_lits(inner, out),
                None => {}
            }
        }
        Expr::Match {
            scrutinee, arms, ..
        } => {
            collect_handler_lits(scrutinee, out);
            for a in arms {
                collect_handler_lits(&a.body, out);
            }
        }
        Expr::Str(pieces, _) => {
            for p in pieces {
                if let StrPiece::Expr(i) = p {
                    collect_handler_lits(i, out);
                }
            }
        }
        _ => {}
    }
}

fn solve_fn_rows(ctx: &mut Ctx) {
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
            collect_fn_bindings(&f.body, ctx, &current, &mut fnb);
            walk_block(&f.body, &current, &mut active, ctx, &fnb, &mut row);
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
}

/// Every function's inferred effect row, keyed `module::name`.
///
/// The checker needs this *before* it types anything, because a function's row
/// is part of its type: `fn(i64) -> i64` and `fn(i64) -> i64 / {io}` are
/// different types, and without the rows there is nothing to compare
/// ([T-FN-TYPE-ROW]). It is the same fixpoint `check_effects_multi` runs, with
/// no diagnostics — those stay in one place, reported once.
pub fn rows_of_modules(files: &[ModuleView]) -> HashMap<String, BTreeSet<String>> {
    rows_and_closures(files).0
}

/// Function rows, and the row of every closure *literal* in the program.
///
/// A closure's row is part of its type just as a named function's is
/// ([T-FN-TYPE-ROW]), and the checker needs it to compare against the type a
/// closure is being stored into. Without it, `Box{ f: |n| noisy(n) }` went
/// into a `fn(i64) -> i64` — a written *empty* row — and everything
/// downstream believed the field performed nothing: a `/ {}` function
/// printed, `par_map`'s purity proof passed, a C function reached libc, and a
/// closure carrying a handled effect left its `with` block inside a list.
/// A named function was checked on every route; only closures leaked.
pub fn rows_and_closures(
    files: &[ModuleView],
) -> (
    HashMap<String, BTreeSet<String>>,
    HashMap<u32, BTreeSet<String>>,
) {
    let mut ctx = build_ctx(files);
    solve_rows(&mut ctx);
    let closures = compute_closure_rows(&ctx);
    (ctx.rows, closures)
}

/// The row of every closure literal, keyed by its span.
fn compute_closure_rows(ctx: &Ctx) -> HashMap<u32, BTreeSet<String>> {
    let mut out = HashMap::new();
    let keys: Vec<String> = ctx.fns.keys().cloned().collect();
    for key in keys {
        let f = ctx.fns[&key];
        let current = key.rsplit_once("::").map(|(m, _)| m).unwrap_or("main").to_string();
        let mut lits: Vec<&Expr> = Vec::new();
        collect_closure_lits_inner(&f.body, &mut lits);
        for lit in lits {
            let Expr::Closure { body, span, .. } = lit else {
                continue;
            };
            let mut row = BTreeSet::new();
            let mut active = Vec::new();
            let mut fnb = FnBindings::default();
            seed_params(&f.params, &mut fnb);
            collect_fn_bindings(&f.body, ctx, &current, &mut fnb);
            walk_expr(body, &current, &mut active, ctx, &fnb, &mut row);
            out.insert(span.start, row);
        }
    }
    out
}

/// Every closure literal reachable from a block.
fn collect_closure_lits_inner<'e>(b: &'e Block, out: &mut Vec<&'e Expr>) {
    for st in &b.stmts {
        match st {
            Stmt::Let { init, .. } => collect_closure_lits(init, out),
            Stmt::Assign { value, target, .. } => {
                collect_closure_lits(value, out);
                collect_closure_lits(target, out);
            }
            Stmt::Return(Some(e), _) | Stmt::Expr(e) => collect_closure_lits(e, out),
            Stmt::While { cond, body, .. } => {
                collect_closure_lits(cond, out);
                collect_closure_lits_inner(body, out);
            }
            Stmt::For { iter, body, .. } => {
                collect_closure_lits(iter, out);
                collect_closure_lits_inner(body, out);
            }
            Stmt::With { handler, body, .. } => {
                collect_closure_lits(handler, out);
                collect_closure_lits_inner(body, out);
            }
            Stmt::Parallel { body, .. } | Stmt::Spawn { body, .. } => {
                collect_closure_lits_inner(body, out)
            }
            _ => {}
        }
    }
}

fn collect_closure_lits<'e>(e: &'e Expr, out: &mut Vec<&'e Expr>) {
    match e {
        Expr::Closure { body, .. } => {
            out.push(e);
            collect_closure_lits(body, out);
        }
        Expr::StructLit { fields, base, .. } => {
            if let Some(b) = base {
                collect_closure_lits(b, out);
            }
            for (_, v) in fields {
                collect_closure_lits(v, out);
            }
        }
        Expr::Call { callee, args, .. } => {
            collect_closure_lits(callee, out);
            for a in args {
                collect_closure_lits(a, out);
            }
        }
        Expr::MethodCall { recv, args, .. } => {
            collect_closure_lits(recv, out);
            for a in args {
                collect_closure_lits(a, out);
            }
        }
        Expr::List(items, _) | Expr::Tuple(items, _) => {
            for i in items {
                collect_closure_lits(i, out);
            }
        }
        Expr::Map(entries, _) => {
            for (k, v) in entries {
                collect_closure_lits(k, out);
                collect_closure_lits(v, out);
            }
        }
        Expr::Binary { lhs, rhs, .. } => {
            collect_closure_lits(lhs, out);
            collect_closure_lits(rhs, out);
        }
        Expr::Unary { expr, .. } | Expr::Try { expr, .. } => collect_closure_lits(expr, out),
        Expr::Field { recv, .. } => collect_closure_lits(recv, out),
        Expr::Index { recv, index, .. } => {
            collect_closure_lits(recv, out);
            collect_closure_lits(index, out);
        }
        Expr::Block(b) | Expr::Unsafe(b) | Expr::Region(b) => collect_closure_lits_inner(b, out),
        Expr::If {
            cond,
            then_block,
            else_branch,
            ..
        } => {
            collect_closure_lits(cond, out);
            collect_closure_lits_inner(then_block, out);
            match else_branch.as_deref() {
                Some(ElseBranch::Else(b)) => collect_closure_lits_inner(b, out),
                Some(ElseBranch::ElseIf(inner)) => collect_closure_lits(inner, out),
                None => {}
            }
        }
        Expr::Match {
            scrutinee, arms, ..
        } => {
            collect_closure_lits(scrutinee, out);
            for a in arms {
                collect_closure_lits(&a.body, out);
            }
        }
        Expr::Str(pieces, _) => {
            for p in pieces {
                if let StrPiece::Expr(i) = p {
                    collect_closure_lits(i, out);
                }
            }
        }
        _ => {}
    }
}

pub fn check_effects_multi(files: &[ModuleView]) -> Vec<Diagnostic> {
    let mut ctx = build_ctx(files);
    solve_rows(&mut ctx);

    let mut diags = Vec::new();

    // A module-level constant is evaluated wherever its name is used, so an
    // initializer with effects would run once per use — which is not what
    // anybody writing `let WIDTH = ...` means, and makes the number of times
    // an effect happens a property of how often a name is mentioned.
    for f in files {
        let current = f.name.clone();
        for d in &f.module.decls {
            let Decl::Const(c) = d else { continue };
            // Taking a reference in a constant has the same problem an effect
            // does, for the same reason: a constant is evaluated at every
            // mention, so each mention allocates a *different* slot. `ROOT`
            // printed as two different references on one line, `release(ROOT)`
            // released a slot nothing else would ever name, `alive(ROOT)` was
            // then true — and a loop reading it allocated a slot per
            // iteration. It followed from [E-CONST] and nothing said so.
            if makes_ref(&c.init) {
                diags.push(
                    Diagnostic::error(
                        c.init.span(),
                        "a constant's value cannot take a reference".to_string(),
                    )
                    .with_help(
                        "a constant is evaluated wherever its name is used, so every mention \
would allocate a different slot — make it a function, or take the reference \
once inside `main`",
                    ),
                );
            }
            let mut row = BTreeSet::new();
            let mut active = Vec::new();
            let fnb = FnBindings::default();
            walk_expr(&c.init, &current, &mut active, &ctx, &fnb, &mut row);
            if !row.is_empty() {
                let names: Vec<String> = row.iter().cloned().collect();
                diags.push(
                    Diagnostic::error(
                        c.init.span(),
                        format!(
                            "a constant's value cannot perform effects, and this performs `{}`",
                            names.join("`, `")
                        ),
                    )
                    .with_help(
                        "a constant is evaluated wherever its name is used, so this would happen once per mention — make it a function instead",
                    ),
                );
            }
        }
    }

    // Declared rows are upper bounds.
    for (key, f) in &ctx.fns {
        if let Some(declared) = &f.effects {
            let declared_set: BTreeSet<String> = declared.iter().map(|p| p.dotted()).collect();
            for d in &declared_set {
                let base = d.split('.').next().unwrap_or(d);
                if base != IO && base != ASYNC && base != UNSAFE && !ctx.effect_names.contains(base) {
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
            .filter(|e| *e != IO && *e != ASYNC && *e != UNSAFE)
            .collect();
        if !leftover.is_empty() {
            let list = leftover
                .iter()
                .map(|m| format!("`{m}`"))
                .collect::<Vec<_>>()
                .join(", ");
            diags.push(
                Diagnostic::error(main.name.span, format!("`main` has unhandled effect(s) {list}"))
                    .with_help(
                        "wrap the calls in `with <Effect> handled_by <handler> { ... }` — and \
if the effect comes from a *handler's own body*, the `with` goes around where that handler \
is installed, not around the calls to it ([E-HANDLER-OWN-ROW])",
                    ),
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
                    row.iter().filter(|e| *e != IO && *e != ASYNC && *e != UNSAFE).collect();
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
        // `xs.par_map(sqrt)` where `sqrt` is a C function. Naming one is
        // handing over the right to call it, so it carries the same `io` a
        // call does — otherwise the row was empty and the purity proof, the
        // effect row and the capability ceiling all passed something that
        // reaches libc.
        if ctx.externs.contains(&format!("{current}::{}", id.name)) {
            out.insert(IO.to_string());
            return;
        }
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
        Expr::Map(entries, _) => {
            for (k, v) in entries {
                collect_calls_expr(k, out);
                collect_calls_expr(v, out);
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
        Expr::Block(b) | Expr::Unsafe(b) | Expr::Region(b) => collect_calls_block(b, out),
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
        Expr::Map(entries, _) => {
            for (k, v) in entries {
                collect_par_maps_expr(k, out);
                collect_par_maps_expr(v, out);
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
        Expr::Block(b) | Expr::Unsafe(b) | Expr::Region(b) => collect_par_maps_block(b, out),
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
    /// Whether this name is bound to something known to be a function value.
    ///
    /// Distinct from `sources_for`, which falls back to every function value
    /// in the body when it does not recognise the name — right for a callee
    /// that must be *something*, wrong for a bare mention, where it would
    /// charge every `i64` local for whatever the body happens to hold.
    fn knows(&self, name: &str) -> bool {
        self.by_name.contains_key(name)
    }

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
        // `let g = b.f` — the row travels with the value, so the binding
        // inherits what the field's type declares ([T-FN-TYPE-ROW-CARRIED]).
        Expr::Field { name, .. } => {
            if name.name.parse::<usize>().is_ok() {
                if !ctx.elem_rows.is_empty() {
                    out.push(FnSource::Declared(ctx.elem_rows.clone()));
                }
            } else if let Some(row) = ctx.field_rows.get(&name.name) {
                out.push(FnSource::Declared(row.clone()));
            }
        }
        // `let g = fs[0]` names no field; every nameless row is a candidate.
        Expr::Index { .. } => {
            if !ctx.elem_rows.is_empty() {
                out.push(FnSource::Declared(ctx.elem_rows.clone()));
            }
        }
        Expr::List(items, _) | Expr::Tuple(items, _) => {
            for i in items {
                fn_sources(i, ctx, current, out);
            }
        }
        Expr::Map(entries, _) => {
            for (k, v) in entries {
                fn_sources(k, ctx, current, out);
                fn_sources(v, ctx, current, out);
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

/// Records every effect row written on a function type reachable from `t`.
///
/// A row on a struct field is keyed by the field's name, because that is what
/// a call site spells (`b.f(x)`). A row anywhere else has no name to key on
/// and joins the nameless pool.
fn harvest_rows(
    t: &TypeExpr,
    field: Option<&str>,
    field_rows: &mut HashMap<String, BTreeSet<String>>,
    elem_rows: &mut BTreeSet<String>,
) {
    match t {
        TypeExpr::Fn {
            params,
            ret,
            effects,
            ..
        } => {
            let row: BTreeSet<String> =
                effects.iter().map(|p| p.segments.last().unwrap().name.clone()).collect();
            if !row.is_empty() {
                match field {
                    Some(name) => field_rows.entry(name.to_string()).or_default().extend(row),
                    None => elem_rows.extend(row),
                }
            }
            for p in params {
                harvest_rows(p, None, field_rows, elem_rows);
            }
            harvest_rows(ret, None, field_rows, elem_rows);
        }
        // Inside any other type constructor the function type has lost its
        // field name, so what it carries joins the nameless pool.
        TypeExpr::List(inner, _) | TypeExpr::Ref(inner, _) => {
            harvest_rows(inner, None, field_rows, elem_rows)
        }
        TypeExpr::Fallible(a, b, _) => {
            harvest_rows(a, None, field_rows, elem_rows);
            harvest_rows(b, None, field_rows, elem_rows);
        }
        TypeExpr::Tuple(items, _) | TypeExpr::Union(items, _) => {
            for i in items {
                harvest_rows(i, None, field_rows, elem_rows);
            }
        }
        TypeExpr::Name { args, .. } => {
            for a in args {
                harvest_rows(a, None, field_rows, elem_rows);
            }
        }
    }
}

/// Harvests the rows every function type in one module declares.
fn harvest_module(
    m: &grasm_syntax::ast::Module,
    field_rows: &mut HashMap<String, BTreeSet<String>>,
    elem_rows: &mut BTreeSet<String>,
) {
    use grasm_syntax::ast::TypeBody;
    for d in &m.decls {
        match d {
            Decl::Type(td) => match &td.body {
                TypeBody::Struct(fields) => {
                    for f in fields {
                        harvest_rows(&f.ty, Some(&f.name.name), field_rows, elem_rows);
                    }
                }
                TypeBody::Sum(variants) => {
                    for v in variants {
                        for f in &v.fields {
                            harvest_rows(&f.ty, Some(&f.name.name), field_rows, elem_rows);
                        }
                    }
                }
                TypeBody::Alias(t) => harvest_rows(t, None, field_rows, elem_rows),
            },
            Decl::Fn(fd) => {
                for p in &fd.params {
                    harvest_rows(&p.ty, None, field_rows, elem_rows);
                }
                if let Some(r) = &fd.ret {
                    harvest_rows(r, None, field_rows, elem_rows);
                }
            }
            _ => {}
        }
    }
}

/// The row a callee that is not a plain name may carry.
fn computed_callee_row(callee: &Expr, ctx: &Ctx, out: &mut BTreeSet<String>) {
    match callee {
        // A tuple component is spelled with a number, and a tuple member's
        // row has no field name to be keyed on — it is in the nameless pool
        // with list elements and map values. Reading it as a *field* name
        // found nothing, so `p.0(1)` carried no row at all while the struct,
        // list, map and `let`-bound spellings were all charged.
        Expr::Field { name, .. } if name.name.parse::<usize>().is_ok() => {
            out.extend(ctx.elem_rows.iter().cloned())
        }
        Expr::Field { name, .. } => {
            if let Some(row) = ctx.field_rows.get(&name.name) {
                out.extend(row.iter().cloned());
            }
        }
        // An index names no field, so every nameless row is a candidate.
        _ => out.extend(ctx.elem_rows.iter().cloned()),
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
            // A handler written right here has just been walked. One that
            // arrived from somewhere else — a parameter, a variable, a list
            // element — has not, and its operations run *here*.
            if !matches!(handler, Expr::StructLit { .. }) {
                if let Some(row) = ctx.handler_rows.get(&effect.dotted()) {
                    for e in row {
                        if !active.contains(e) {
                            out.insert(e.clone());
                        }
                    }
                }
            }
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
        Expr::Int(..) | Expr::Float(..) | Expr::Bool(..) => {}
        // A bare mention of a C function is a reference to it, and holding one
        // is the same permission as calling it.
        Expr::Ident(id) => {
            if ctx.externs.contains(&format!("{current}::{}", id.name))
                && !active.iter().any(|a| a == IO)
            {
                out.insert(IO.to_string());
            }
            // A named function mentioned as a *value* carries its own row.
            //
            // This is the rule already applied to a declared C function —
            // holding one is the permission to call it — and it has to hold
            // for every function, because handing one to something that will
            // call it is how the effect actually happens. Without it,
            // `[1, 2].map(shout)` inside a `/ {}` function typechecked and
            // printed: `map`'s parameter is unconstrained by design, so the
            // row had nowhere else to come from. That defeated the declared
            // row, [T-PAR-MAP-PURE] one hop away from `par_map`'s own check,
            // [E-HANDLER-MISSING]'s "well-typed programs never reach" backstop,
            // and a package's `capabilities = []` ceiling.
            if let Some(qual) = ctx.resolve_call(current, &id.name) {
                contribute(&qual, active, ctx, out);
            } else if fnb.knows(&id.name) {
                // A *local* holding a function value — most often a parameter
                // whose type wrote the row. The direct call `f(x)` was
                // charged and the mention was not, so handing the same `f` to
                // a builtin that calls it charged nothing: `xs.map(f)` inside
                // a `/ {}` function printed, and so did `fold`, `filter`,
                // `any`, `all`, `find` and `sort_by`. `par_map` was checked,
                // which is what made the hole look closed.
                indirect_row(&id.name, current, active, ctx, fnb, out);
            }
        }
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
        Expr::Map(entries, _) => {
            for (k, v) in entries {
                walk_expr(k, current, active, ctx, fnb, out);
                walk_expr(v, current, active, ctx, fnb, out);
            }
        }
        Expr::StructLit { base, fields, .. } => {
            if let Some(b) = base {
                walk_expr(b, current, active, ctx, fnb, out);
            }
            for (_, v) in fields {
                walk_expr(v, current, active, ctx, fnb, out);
            }
        }
        Expr::Call { callee, args, .. } => {
            if let Expr::Ident(id) = &**callee {
                if IO_BUILTINS.contains(&id.name.as_str()) && !active.iter().any(|a| a == IO) {
                    out.insert(IO.to_string());
                }
                // A C function's effects are unknowable, so it performs `io`.
                if ctx.externs.contains(&format!("{current}::{}", id.name))
                    && !active.iter().any(|a| a == IO)
                {
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
                // A call through something that is not a name: a struct
                // field, a list element, a map value. The row is on the type
                // it was declared with ([T-FN-TYPE-ROW-CARRIED]).
                computed_callee_row(callee, ctx, out);
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
            // Or a call of a function held in a field of that name
            // ([T-FIELD-CALL]): `b.f(x)` where `f: fn(i64) -> i64 / {io}`.
            if name.name.parse::<usize>().is_ok() {
                // A tuple component: numbered, so keyed nowhere.
                out.extend(ctx.elem_rows.iter().cloned());
            } else if let Some(row) = ctx.field_rows.get(&name.name) {
                out.extend(row.iter().cloned());
            }
            for a in args {
                walk_expr(a, current, active, ctx, fnb, out);
            }
        }
        // Reading a function out of a field or a tuple component is the same
        // permission as naming one ([T-FN-VALUE-ROW]) — `xs.map(p.0)` hands
        // it to something that will call it.
        Expr::Field { recv, name, .. } => {
            computed_callee_row(e, ctx, out);
            let _ = name;
            walk_expr(recv, current, active, ctx, fnb, out);
        }
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
        // Tier 3 is a capability, not just a syntax. A package containing an
        // `unsafe` block declares `unsafe` in its manifest or fails to build,
        // which is what makes it visible to — and refusable by — everyone
        // downstream: your JSON parser cannot quietly contain one.
        // A region performs whatever its body performs; it is a scope, not
        // a capability.
        Expr::Region(b) => walk_block(b, current, active, ctx, fnb, out),
        Expr::Unsafe(b) => {
            out.insert(UNSAFE.to_string());
            walk_block(b, current, active, ctx, fnb, out);
        }
        Expr::Closure { body, .. } => walk_expr(body, current, active, ctx, fnb, out),
    }
}

/// Where each effect is *directly* performed, one representative site each.
///
/// The capability ceiling reports which effects a package exceeds and used to
/// stop there — so a `capabilities = []` package that printed somewhere told
/// you only that it printed. Finding the `print` was then a manual search of
/// the package. This answers "where", for the effects that have a syntactic
/// site: an `io` builtin, a C function, an `unsafe` block, an effect
/// operation. An effect reaching a package only through a call it makes has
/// no site of its own and is left out; the message falls back to naming it.
pub fn effect_sites(program: &crate::loader::Program) -> HashMap<String, String> {
    let mut out: HashMap<String, String> = HashMap::new();
    for file in &program.files {
        let externs: HashSet<String> = file
            .module
            .decls
            .iter()
            .filter_map(|d| match d {
                Decl::Extern(x) => Some(x.name.name.clone()),
                _ => None,
            })
            .collect();
        let effect_names: HashSet<String> = file
            .module
            .decls
            .iter()
            .filter_map(|d| match d {
                Decl::Effect(e) => Some(e.name.name.clone()),
                _ => None,
            })
            .collect();
        // Rendered the same way every other diagnostic is: a path, a line,
        // and a column. A byte offset into a virtual span space is not a
        // place anyone can go and look at.
        let mut note = |name: &str, span: Span| {
            out.entry(name.to_string()).or_insert_with(|| {
                let off = span.start.saturating_sub(file.base) as usize;
                let upto = &file.src[..off.min(file.src.len())];
                let line = upto.matches('\n').count() + 1;
                let col = upto.rsplit('\n').next().map_or(1, |l| l.chars().count() + 1);
                format!("{}:{line}:{col}", file.path)
            });
        };
        for decl in &file.module.decls {
            let Decl::Fn(f) = decl else { continue };
            let mut sites: Vec<(String, Span)> = Vec::new();
            site_block(&f.body, &externs, &effect_names, &mut sites);
            for (n, sp) in sites {
                note(&n, sp);
            }
        }
    }
    out
}

fn site_block(
    b: &Block,
    externs: &HashSet<String>,
    effects: &HashSet<String>,
    out: &mut Vec<(String, Span)>,
) {
    for s in &b.stmts {
        match s {
            Stmt::Let { init, .. } => site_expr(init, externs, effects, out),
            Stmt::Assign { value, .. } => site_expr(value, externs, effects, out),
            Stmt::Return(Some(e), _) | Stmt::Expr(e) => site_expr(e, externs, effects, out),
            Stmt::While { body, cond, .. } => {
                site_expr(cond, externs, effects, out);
                site_block(body, externs, effects, out);
            }
            Stmt::For { body, iter, .. } => {
                site_expr(iter, externs, effects, out);
                site_block(body, externs, effects, out);
            }
            Stmt::With { body, .. } => site_block(body, externs, effects, out),
            Stmt::Parallel { body, .. } | Stmt::Spawn { body, .. } => {
                site_block(body, externs, effects, out)
            }
            _ => {}
        }
    }
}

fn site_expr(
    e: &Expr,
    externs: &HashSet<String>,
    effects: &HashSet<String>,
    out: &mut Vec<(String, Span)>,
) {
    match e {
        Expr::Call { callee, args, span } => {
            if let Expr::Ident(id) = &**callee {
                if IO_BUILTINS.contains(&id.name.as_str()) {
                    out.push((IO.to_string(), *span));
                }
                if externs.contains(&id.name) {
                    out.push((IO.to_string(), *span));
                    out.push((UNSAFE.to_string(), *span));
                }
            }
            site_expr(callee, externs, effects, out);
            for a in args {
                site_expr(a, externs, effects, out);
            }
        }
        Expr::MethodCall {
            recv, name, args, span,
        } => {
            if let Expr::Ident(tid) = &**recv {
                if effects.contains(&tid.name) {
                    out.push((tid.name.clone(), *span));
                }
            }
            let _ = name;
            site_expr(recv, externs, effects, out);
            for a in args {
                site_expr(a, externs, effects, out);
            }
        }
        Expr::Unsafe(b) => {
            out.push((UNSAFE.to_string(), b.span));
            site_block(b, externs, effects, out);
        }
        Expr::Block(b) | Expr::Region(b) => site_block(b, externs, effects, out),
        Expr::If {
            cond,
            then_block,
            else_branch,
            ..
        } => {
            site_expr(cond, externs, effects, out);
            site_block(then_block, externs, effects, out);
            if let Some(eb) = else_branch {
                match &**eb {
                    ElseBranch::Else(b) => site_block(b, externs, effects, out),
                    ElseBranch::ElseIf(inner) => site_expr(inner, externs, effects, out),
                }
            }
        }
        Expr::Match {
            scrutinee, arms, ..
        } => {
            site_expr(scrutinee, externs, effects, out);
            for a in arms {
                site_expr(&a.body, externs, effects, out);
            }
        }
        Expr::Closure { body, .. } => site_expr(body, externs, effects, out),
        Expr::Unary { expr, .. } | Expr::Try { expr, .. } => {
            site_expr(expr, externs, effects, out)
        }
        Expr::Field { recv, .. } => site_expr(recv, externs, effects, out),
        Expr::Binary { lhs, rhs, .. } => {
            site_expr(lhs, externs, effects, out);
            site_expr(rhs, externs, effects, out);
        }
        Expr::Index { recv, index, .. } => {
            site_expr(recv, externs, effects, out);
            site_expr(index, externs, effects, out);
        }
        Expr::List(items, _) | Expr::Tuple(items, _) => {
            for i in items {
                site_expr(i, externs, effects, out);
            }
        }
        Expr::Map(entries, _) => {
            for (k, v) in entries {
                site_expr(k, externs, effects, out);
                site_expr(v, externs, effects, out);
            }
        }
        Expr::Str(pieces, _) => {
            for p in pieces {
                if let StrPiece::Expr(inner) = p {
                    site_expr(inner, externs, effects, out);
                }
            }
        }
        _ => {}
    }
}
