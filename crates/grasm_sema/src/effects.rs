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

/// Whether a bare name is one of the language's own, rather than something a
/// program could have bound.
fn is_builtin_name(name: &str) -> bool {
    IO_BUILTINS.contains(&name)
        || matches!(
            name,
            "len" | "map" | "filter" | "fold" | "any" | "all" | "find" | "sort" | "sort_by"
                | "push" | "pop" | "remove" | "set" | "get" | "has" | "delete" | "keys"
                | "values" | "entries" | "par_map" | "par_filter" | "range" | "list" | "map_new"
                | "exit" | "panic" | "assert" | "assert_eq" | "ref" | "release" | "alive"
                | "Some" | "None" | "Ok" | "Err" | "fail" | "try" | "pi" | "e"
        )
}

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
    /// `type Cb = fn(i64) -> i64 / {io}` — an alias is not a new type, and
    /// the two are interchangeable everywhere ([T-TYPE-ALIAS]). This pass
    /// reads written syntax rather than resolved types, so a row written
    /// behind a name was not there: `f: Cb` seeded an *empty* declared row,
    /// which then shadowed every fallback, and one `type` line laundered
    /// `io` past a `/ {}` row, `par_map`'s purity proof and a package's
    /// `capabilities = []`.
    type_aliases: HashMap<String, TypeExpr>,
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
    /// What each handler literal performs, by (effect, the module it is
    /// *written in*).
    ///
    /// Installing a handler that arrived from elsewhere charges what a handler
    /// for that effect does ([E-HANDLER-OWN-ROW]) — but charging every handler
    /// in the program made a library's row a function of its *consumer's*
    /// code. A `capabilities = []` library that installs a handler its
    /// consumer passes in was charged the consumer's `io` and failed to build
    /// for a `print` written entirely in the consumer, with the help text
    /// telling the library's author to widen a manifest that had nothing wrong
    /// with it. Authority flows from whoever wrote the handler, and it is
    /// charged there.
    handler_rows: HashMap<(String, String), BTreeSet<String>>,
    /// Module names that are dependency packages, so "the same package" can be
    /// decided the way [T-CAPABILITY] counts it.
    packages: BTreeSet<String>,
    /// Every effect row reachable from a declared type's own body, by type
    /// name.
    ///
    /// A written type is read for the rows inside it, and a *name* stood for
    /// nothing: `type V = | Hold(f: fn(i64) -> i64 / {io})` as a parameter
    /// contributed no row, so a `match` payload bound from it called a
    /// printing function inside a `/ {}` body, on `par_map`'s workers, and in
    /// a package declaring `capabilities = []`.
    type_rows: HashMap<String, BTreeSet<String>>,
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

/// What each *package* in a program performs, by the prefix its modules carry
/// (the entry package answers to `""`).
///
/// A package's ceiling is otherwise computed from that package alone, and a
/// library that installs a handler it did not construct performs nothing on
/// its own: `pub fn run(h: LogHandler) { with Log handled_by h { … } }` in a
/// `capabilities = []` package printed through a handler the consumer passed
/// it, and every command reported the library as performing nothing. In a
/// whole program the handler exists, so this is what the consumer's build
/// holds each dependency to ([E-HANDLER-OWN-ROW]).
pub fn effects_by_package(
    program: &crate::loader::Program,
) -> std::collections::BTreeMap<String, BTreeSet<String>> {
    let views: Vec<ModuleView> = program
        .files
        .iter()
        .map(|f| ModuleView {
            name: f.name.clone(),
            module: &f.module,
            imports: f.imports.clone(),
        })
        .collect();
    let mut ctx = build_ctx_in(&views, &program.deps.iter().cloned().collect());
    solve_rows(&mut ctx);
    let mut out: std::collections::BTreeMap<String, BTreeSet<String>> =
        std::collections::BTreeMap::new();
    // Which package owns each module, worked out once per module rather than
    // once per row. Scanning every dependency for every row — and building a
    // `"{d}."` string to test each one — was quadratic in the size of the
    // graph: five hundred packages spent most of `grasm check` here.
    let mut owner_of: HashMap<&str, String> = HashMap::new();
    for (key, row) in &ctx.rows {
        let Some((module, name)) = key.rsplit_once("::") else {
            continue;
        };
        // Only what *escapes* the package, as `effects_of_program` counts it:
        // a `pub` row a consumer has to handle, or the entry point. A private
        // helper's effect that the package handles three lines later is not a
        // capability, and charging it demanded one named after an internal
        // effect nobody outside can see.
        let escapes = ctx.fns[key].is_pub || name == "main";
        if !escapes {
            continue;
        }
        // The package a module belongs to is the longest declared prefix.
        let owner = owner_of.entry(module).or_insert_with(|| {
            program
                .deps
                .iter()
                .filter(|d| is_prefix_module(module, d))
                .max_by_key(|d| d.len())
                .cloned()
                .unwrap_or_default()
        });
        out.entry(owner.clone())
            .or_default()
            .extend(row.iter().cloned());
    }
    out
}

/// Whether `module` is `pkg` itself or a module inside it.
///
/// Written without building the `"{pkg}."` string it compares against: that
/// allocation ran once per (row, dependency) pair.
fn is_prefix_module(module: &str, pkg: &str) -> bool {
    module == pkg
        || (module.len() > pkg.len()
            && module.as_bytes()[pkg.len()] == b'.'
            && module.starts_with(pkg))
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
        type_aliases: HashMap::new(),
        handler_rows: HashMap::new(),
        packages: program.deps.iter().cloned().collect(),
        type_rows: HashMap::new(),
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
        collect_type_aliases(f.module, &mut ctx.type_aliases);
    }
    ctx.type_rows = collect_type_rows(&views, &ctx.type_aliases);
    for f in &views {
        harvest_module(
            f.module,
            &ctx.type_aliases,
            &ctx.type_rows,
            &mut ctx.field_rows,
            &mut ctx.elem_rows,
        );
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
            seed_params(&f.params, &mut fnb, &ctx.type_aliases, &ctx.type_rows);
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
    // Doc examples count, for the same reason and more sharply. `grasm test`
    // runs the examples of every imported module, so a dependency's `///`
    // block executes in the consumer's process — and unlike a test body it is
    // *documentation*, which nobody reads as code that runs. A
    // `capabilities = []` package could put `write_file` and
    // `unsafe { getpid() }` in one and `grasm deps` reported `performs []`,
    // manifest and lockfile agreeing that it did nothing.
    for ex in crate::docs::examples(program) {
        let module = ex.owner.rsplit_once("::").map(|(m, _)| m).unwrap_or("main");
        let module = module.split(" #").next().unwrap_or(module).to_string();
        // A dependency's examples are the dependency's, exactly as its
        // functions, tests and benchmarks are — every other loop here says so
        // and this one did not, so a consumer that performs nothing at all was
        // charged for a `print` in a `///` block in someone else's package.
        // The diagnostic carried no location, so its author had no pointer to
        // a file they did not write.
        if foreign(&module) {
            continue;
        }
        let wrapped = format!("fn __doc_example() {{\n{}\n}}\n", ex.source);
        let parsed = grasm_syntax::parse(&wrapped);
        if !parsed.diagnostics.is_empty() {
            continue;
        }
        for d in &parsed.module.decls {
            let Decl::Fn(f) = d else { continue };
            let mut row = BTreeSet::new();
            let mut active = Vec::new();
            let mut fnb = FnBindings::default();
            collect_fn_bindings(&f.body, &ctx, &module, &mut fnb);
            walk_block(&f.body, &module, &mut active, &ctx, &fnb, &mut row);
            all.extend(row);
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
    build_ctx_in(files, &BTreeSet::new())
}

/// `build_ctx`, told which module names are dependency packages.
///
/// Only the capability pass needs this: it decides where a handler's effects
/// are charged, and a handler written by a consumer runs on the consumer's
/// authority.
fn build_ctx_in<'a>(files: &'a [ModuleView<'a>], packages: &BTreeSet<String>) -> Ctx<'a> {
    let mut ctx = Ctx {
        fns: HashMap::new(),
        effect_names: HashSet::new(),
        externs: HashSet::new(),
        rows: HashMap::new(),
        imports: HashMap::new(),
        field_rows: HashMap::new(),
        elem_rows: BTreeSet::new(),
        type_aliases: HashMap::new(),
        handler_rows: HashMap::new(),
        packages: packages.clone(),
        type_rows: HashMap::new(),
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
        collect_type_aliases(f.module, &mut ctx.type_aliases);
    }
    ctx.type_rows = collect_type_rows(files, &ctx.type_aliases);
    for f in files {
        harvest_module(
            f.module,
            &ctx.type_aliases,
            &ctx.type_rows,
            &mut ctx.field_rows,
            &mut ctx.elem_rows,
        );
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
impl Ctx<'_> {
    /// Which package a module belongs to: the longest declared prefix, the
    /// same way `effects_by_package` counts it. The entry package's own
    /// modules answer with the empty string.
    fn package_of(&self, module: &str) -> String {
        self.packages
            .iter()
            .filter(|d| is_prefix_module(module, d))
            .max_by_key(|d| d.len())
            .cloned()
            .unwrap_or_default()
    }
}

fn compute_handler_rows(ctx: &Ctx) -> HashMap<(String, String), BTreeSet<String>> {
    let mut out: HashMap<(String, String), BTreeSet<String>> = HashMap::new();
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
            seed_params(&f.params, &mut fnb, &ctx.type_aliases, &ctx.type_rows);
            collect_fn_bindings(&f.body, ctx, &current, &mut fnb);
            walk_expr(lit, &current, &mut active, ctx, &fnb, &mut row);
            out.entry((effect.to_string(), current.clone()))
                .or_default()
                .extend(row);
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
            seed_params(&f.params, &mut fnb, &ctx.type_aliases, &ctx.type_rows);
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
            seed_params(&f.params, &mut fnb, &ctx.type_aliases, &ctx.type_rows);
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
            seed_params(params, &mut fnb, &ctx.type_aliases, &ctx.type_rows);
            collect_fn_bindings(body, &ctx, &f.name, &mut fnb);
            check_par_map_block(body, &f.name, &ctx, &fnb, &mut diags);
            check_fn_args_block(body, &f.name, &ctx, &fnb, &mut diags);
            let mut declared_bindings = HashMap::new();
            check_declared_rows_block(
                body,
                &f.name,
                &ctx,
                &fnb,
                &mut declared_bindings,
                &mut diags,
            );
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
    // `xs.par_map(lib.shout)` — the same function a call would reach, named
    // through its module. Only the bare spelling was recognised, so one
    // `import` emptied the row: a printing function reached `par_map`, a
    // `/ {}` row, a region and a `capabilities = []` package, and every one of
    // those refusals is the same refusal one word longer.
    if let Expr::Field { recv, name, .. } = func {
        if let Expr::Ident(alias) = &**recv {
            if let Some(target) = ctx.imports.get(current).and_then(|m| m.get(&alias.name)) {
                if ctx.externs.contains(&format!("{target}::{}", name.name)) {
                    out.insert(IO.to_string());
                    return;
                }
                if let Some(qual) = ctx.resolve_qualified(current, &alias.name, &name.name) {
                    if let Some(row) = ctx.rows.get(&qual) {
                        out.extend(row.iter().cloned());
                    }
                    return;
                }
            }
        }
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
/// A value put in a binding whose written type names a row must fit it.
///
/// The argument check below covers a function *passed* to a declared
/// parameter. Nothing covered a function *stored*: `let f: fn() -> i64 =
/// id(closure_that_prints)` and `keep = [1].map(|_| || Cfg.value())` both
/// promised an empty row and kept a value that had one, and the second
/// carried a handler's discharge out of its `with` block
/// ([T-FN-TYPE-ROW-CARRIED], [E-HANDLER-MISSING]).
fn check_declared_rows_block(
    b: &Block,
    current: &str,
    ctx: &Ctx,
    fnb: &FnBindings,
    declared: &mut HashMap<String, BTreeSet<String>>,
    diags: &mut Vec<Diagnostic>,
) {
    for stmt in &b.stmts {
        match stmt {
            Stmt::Let { name, ty: Some(t), init, .. } => {
                if let Some(allowed) = declared_row_within(t, &ctx.type_aliases, &ctx.type_rows) {
                    declared.insert(name.name.clone(), allowed.clone());
                    report_row_over(init, current, ctx, fnb, &allowed, diags);
                }
            }
            Stmt::Assign { target, value, .. } => {
                if let Expr::Ident(id) = target {
                    if let Some(allowed) = declared.get(&id.name).cloned() {
                        report_row_over(value, current, ctx, fnb, &allowed, diags);
                    }
                }
            }
            Stmt::While { body, .. }
            | Stmt::For { body, .. }
            | Stmt::With { body, .. }
            | Stmt::Parallel { body, .. }
            | Stmt::Spawn { body, .. } => {
                check_declared_rows_block(body, current, ctx, fnb, declared, diags)
            }
            _ => {}
        }
    }
}

fn report_row_over(
    e: &Expr,
    current: &str,
    ctx: &Ctx,
    fnb: &FnBindings,
    allowed: &BTreeSet<String>,
    diags: &mut Vec<Diagnostic>,
) {
    let mut row = BTreeSet::new();
    carried_row(e, current, ctx, fnb, 0, &mut row);
    let over: Vec<&String> = row.difference(allowed).collect();
    if over.is_empty() {
        return;
    }
    let list = over
        .iter()
        .map(|x| format!("`{x}`"))
        .collect::<Vec<_>>()
        .join(", ");
    let shown = if allowed.is_empty() {
        "performs nothing".to_string()
    } else {
        format!(
            "performs only {}",
            allowed
                .iter()
                .map(|x| format!("`{x}`"))
                .collect::<Vec<_>>()
                .join(", ")
        )
    };
    diags.push(
        Diagnostic::error(
            e.span(),
            format!(
                "this type says the function {shown}, but the value performs {list}"
            ),
        )
        .with_help(
            "widen the written type — `fn(..) -> .. / {io}` — or hand over a function that does not perform it",
        ),
    );
}

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
        // `helper.run(f)` is the same call as `run(f)` next door, and the
        // declared row on its parameter is the same promise. Only the bare
        // spelling was collected, so a function value crossing a module
        // boundary was never held to what it was handed to.
        let qual = match callee.split_once('.') {
            Some((alias, name)) => ctx.resolve_qualified(current, alias, name),
            None => ctx.resolve_call(current, &callee),
        };
        let Some(qual) = qual else {
            continue;
        };
        let decl = ctx.fns[&qual];
        for (arg, param) in args.iter().zip(&decl.params) {
            let Some(allowed) = declared_row(&param.ty, &ctx.type_aliases) else {
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
fn collect_calls_block<'e>(b: &'e Block, out: &mut Vec<(String, &'e [Expr], Span)>) {
    for s in &b.stmts {
        collect_calls_stmt(s, out);
    }
}

fn collect_calls_stmt<'e>(s: &'e Stmt, out: &mut Vec<(String, &'e [Expr], Span)>) {
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

fn collect_calls_expr<'e>(e: &'e Expr, out: &mut Vec<(String, &'e [Expr], Span)>) {
    match e {
        Expr::Call { callee, args, span } => {
            match &**callee {
                Expr::Ident(id) => out.push((id.name.clone(), args.as_slice(), *span)),
                // `helper.run(f)` parses as a call on a field of the module
                // alias, and is the same call as the bare spelling.
                Expr::Field { recv, name, .. } => {
                    if let Expr::Ident(alias) = &**recv {
                        out.push((
                            format!("{}.{}", alias.name, name.name),
                            args.as_slice(),
                            *span,
                        ));
                    }
                }
                _ => {}
            }
            for a in args {
                collect_calls_expr(a, out);
            }
        }
        // And the method-call spelling of the same thing.
        Expr::MethodCall { recv, name, args, span } => {
            if let Expr::Ident(alias) = &**recv {
                out.push((
                    format!("{}.{}", alias.name, name.name),
                    args.as_slice(),
                    *span,
                ));
            }
            collect_calls_expr(recv, out);
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
    /// A function value this pass could not pin to any of the above.
    ///
    /// Recorded rather than dropped: a binding with *some* recognised source
    /// and one unrecognised operand looked fully accounted for, so the
    /// signature fallback never fired. `let g = o ?? nop` recorded only the
    /// pure `nop` and charged nothing, while `o.unwrap_or(nop)` and the
    /// `match` spelling of the same thing were both charged — so one operator
    /// erased a row that defeats `/ {}`, [T-PAR-MAP-PURE] and a package's
    /// `capabilities = []`.
    Unknown,
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
    /// Every name the body binds, and the offset it comes into scope at.
    ///
    /// A local shadows a function of the same name, and only this pass
    /// disagreed: `fn bump(shout: i64) / {}` was charged the row of a
    /// same-module `fn shout` it never mentions — which broke a
    /// `capabilities = []` package in every consumer over one parameter's
    /// spelling. Parameters are in scope for the whole body, so they are
    /// recorded at 0; a `let` shadows only what follows it, which is why the
    /// offset is kept rather than a bare set.
    bound_at: HashMap<String, u32>,
    /// Every function value the body mentions anywhere. Used when a callee
    /// cannot be pinned to one binding: over-approximating there is the whole
    /// point, since under-approximating is what let effects escape.
    any: Vec<FnSource<'a>>,
    /// Rows written on function types inside *this function's own signature*,
    /// at positions with no field name to key on — a list element, a tuple
    /// component, an `Option`'s payload.
    ///
    /// A call through a local that no binder here understands is charged
    /// these. `for f in fs`, a `match` payload, a tuple destructuring and a
    /// closure parameter each called a `/ {io}` element and were charged
    /// nothing, while `let f = fs[0]` one line away was charged correctly —
    /// which let one `for` loop carry a printing function past a `/ {}` row,
    /// `par_map`'s purity proof and a package's `capabilities = []`. Scoped
    /// to this signature rather than to every row the program writes: the
    /// wider pool charged `io` to a function whose own element type declares
    /// no effects, on the strength of an unrelated declaration elsewhere.
    signature_rows: BTreeSet<String>,
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

    /// Whether a mention at this offset is inside a local of that name.
    fn shadowed(&self, name: &str, at: u32) -> bool {
        self.bound_at.get(name).is_some_and(|from| at >= *from)
    }

    fn bind_name(&mut self, name: &str, at: u32) {
        let slot = self.bound_at.entry(name.to_string()).or_insert(at);
        if at < *slot {
            *slot = at;
        }
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
            } else {
                // A local holding a function value, or a parameter. Which one
                // is settled by `sources_for`; what matters here is that this
                // operand was *not* accounted for.
                out.push(FnSource::Unknown);
            }
        }
        Expr::Closure { .. } => out.push(FnSource::Closure(e)),
        // `let g = b.f` — the row travels with the value, so the binding
        // inherits what the field's type declares ([T-FN-TYPE-ROW-CARRIED]).
        Expr::Field { recv, name, .. } => {
            // `let g = lib.shout` names a function through its module, which
            // is the same value the bare name would give and carries the same
            // row.
            if let Expr::Ident(alias) = &**recv {
                if let Some(qual) = ctx.resolve_qualified(current, &alias.name, &name.name) {
                    out.push(FnSource::Named(qual));
                    return;
                }
            }
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
        // Every shape that hands a function value through: `Some(f) ?? f`
        // laundered a printing function past [T-PAR-MAP-PURE], which the
        // clause calls "a compile error, not a race".
        Expr::Binary { lhs, rhs, .. } => {
            fn_sources(lhs, ctx, current, out);
            fn_sources(rhs, ctx, current, out);
        }
        Expr::Call { callee, args, .. } => {
            // A constructor hands its arguments through: the prelude's three,
            // and a user sum type's variants, which are spelled the same way.
            // Only the prelude's were recognised, so `Hold(shout)` for
            // `type V = | Hold(f: fn(i64) -> i64 / {io})` carried a printing
            // function into a binding with no row on it, and the `match`
            // payload that read it back called it inside `par_map`.
            let carrier = match &**callee {
                Expr::Ident(id) => {
                    matches!(id.name.as_str(), "Some" | "Ok" | "Err")
                        || (ctx.resolve_call(current, &id.name).is_none()
                            && id.name.starts_with(char::is_uppercase))
                }
                // `Type.Variant(..)`, the qualified spelling.
                Expr::Field { name, .. } => name.name.starts_with(char::is_uppercase),
                _ => false,
            };
            if carrier {
                for a in args {
                    fn_sources(a, ctx, current, out);
                }
            }
        }
        Expr::If { then_block, else_branch, .. } => {
            if let Some(Stmt::Expr(t)) = then_block.stmts.last() {
                fn_sources(t, ctx, current, out);
            }
            match else_branch.as_deref() {
                Some(ElseBranch::Else(b)) => {
                    if let Some(Stmt::Expr(t)) = b.stmts.last() {
                        fn_sources(t, ctx, current, out);
                    }
                }
                Some(ElseBranch::ElseIf(inner)) => fn_sources(inner, ctx, current, out),
                None => {}
            }
        }
        Expr::Match { arms, .. } => {
            for a in arms {
                fn_sources(&a.body, ctx, current, out);
            }
        }
        Expr::Try { expr, .. } => fn_sources(expr, ctx, current, out),
        _ => {}
    }
}

/// The effect row written on a function type, or the empty row when none is.
///
/// Empty is the meaningful default: `fn(i64) -> i64` accepts only functions
/// that perform nothing, so an effectful one passed there is an error rather
/// than a silent widening of whatever row the caller declared.
fn declared_row(te: &TypeExpr, aliases: &HashMap<String, TypeExpr>) -> Option<BTreeSet<String>> {
    match resolve_alias(te, aliases, 0) {
        TypeExpr::Fn { effects, .. } => Some(effects.iter().map(|p| p.dotted()).collect()),
        _ => None,
    }
}

/// How many `type` names one written type may hide behind.
///
/// A bound rather than a cycle check: `type A = B` and `type B = A` is
/// refused elsewhere, and this pass must not be the thing that loops.
const MAX_ALIAS_DEPTH: usize = 32;

/// A written type with its leading alias names resolved.
///
/// Returns a clone rather than a borrow because an alias body lives in
/// another declaration; the types involved are small and this runs once per
/// binding.
fn resolve_alias(
    te: &TypeExpr,
    aliases: &HashMap<String, TypeExpr>,
    depth: usize,
) -> TypeExpr {
    if depth > MAX_ALIAS_DEPTH {
        return te.clone();
    }
    if let TypeExpr::Name { path, args, .. } = te {
        if args.is_empty() && path.segments.len() == 1 {
            if let Some(body) = aliases.get(&path.segments[0].name) {
                return resolve_alias(body, aliases, depth + 1);
            }
        }
    }
    te.clone()
}

/// The row written on a function type *anywhere inside* a written type.
///
/// `[fn() -> i64]` promises its elements perform nothing just as
/// `fn() -> i64` does, and a value put in one has to keep that promise —
/// otherwise a `with` block's closure was stored in an outer list, called
/// afterwards, and reached the runtime's missing-handler panic that
/// [E-HANDLER-MISSING] says a well-typed program never reaches.
fn declared_row_within(
    te: &TypeExpr,
    aliases: &HashMap<String, TypeExpr>,
    type_rows: &HashMap<String, BTreeSet<String>>,
) -> Option<BTreeSet<String>> {
    declared_row_within_at(te, aliases, type_rows, 0)
}

fn declared_row_within_at(
    te: &TypeExpr,
    aliases: &HashMap<String, TypeExpr>,
    type_rows: &HashMap<String, BTreeSet<String>>,
    depth: usize,
) -> Option<BTreeSet<String>> {
    // Bounded for the same reason as `collect_type_names`.
    if depth > MAX_ALIAS_DEPTH {
        return None;
    }
    match &resolve_alias(te, aliases, 0) {
        TypeExpr::Fn { effects, .. } => {
            Some(effects.iter().map(|p| p.dotted()).collect())
        }
        TypeExpr::List(inner, _) | TypeExpr::Ref(inner, _) => {
            declared_row_within_at(inner, aliases, type_rows, depth + 1)
        }
        TypeExpr::Fallible(a, b, _) => union_rows(
            declared_row_within_at(a, aliases, type_rows, depth + 1),
            declared_row_within_at(b, aliases, type_rows, depth + 1),
        ),
        // `Option[T]`, `Map[K, V]` and any other named generic — and a
        // declared type, whose own fields may hold a row. A name stood for
        // nothing here, so `fn pure(v: V) -> i64 / {}` where
        // `type V = | Hold(f: fn(i64) -> i64 / {io})` had an empty signature
        // pool and the payload it bound called a printing function.
        TypeExpr::Name { path, args, .. } => {
            let mut found: Option<BTreeSet<String>> = None;
            if let Some(seg) = path.segments.last() {
                if let Some(r) = type_rows.get(&seg.name) {
                    if !r.is_empty() {
                        found = Some(r.clone());
                    }
                }
            }
            for a in args {
                found = union_rows(found, declared_row_within_at(a, aliases, type_rows, depth + 1));
            }
            found
        }
        TypeExpr::Union(items, _) | TypeExpr::Tuple(items, _) => {
            let mut found: Option<BTreeSet<String>> = None;
            for i in items {
                if let Some(r) = declared_row_within_at(i, aliases, type_rows, depth + 1) {
                    found = Some(match found {
                        None => r,
                        Some(prev) => prev.union(&r).cloned().collect(),
                    });
                }
            }
            found
        }
    }
}

fn union_rows(
    a: Option<BTreeSet<String>>,
    b: Option<BTreeSet<String>>,
) -> Option<BTreeSet<String>> {
    match (a, b) {
        (Some(x), Some(y)) => Some(x.union(&y).cloned().collect()),
        (Some(x), None) => Some(x),
        (None, y) => y,
    }
}

/// The row carried by the *value* an expression produces.
///
/// Not what evaluating it performs — what calling the function it hands over
/// would. A generic that passes its argument through, a builtin that maps a
/// closure over a list, a `??` between two candidates: each is a hop the row
/// has to survive, and each was one it did not ([T-FN-TYPE-ROW-CARRIED]).
fn carried_row(
    e: &Expr,
    current: &str,
    ctx: &Ctx,
    fnb: &FnBindings,
    depth: usize,
    out: &mut BTreeSet<String>,
) {
    if depth > 16 {
        return;
    }
    match e {
        Expr::Closure { body, .. } => {
            let mut active = Vec::new();
            walk_expr(body, current, &mut active, ctx, fnb, out);
        }
        Expr::Ident(id) => {
            let mut active = Vec::new();
            par_fn_row(e, current, ctx, fnb, &mut active, out);
            let _ = id;
        }
        // `id(f)` — a generic that hands its argument back, and any call
        // whose arguments include function values.
        Expr::Call { callee, args, .. } => {
            if let Expr::Ident(id) = &**callee {
                if matches!(id.name.as_str(), "Some" | "Ok" | "Err") {
                    for a in args {
                        carried_row(a, current, ctx, fnb, depth + 1, out);
                    }
                    return;
                }
                // A function that can only return what it was given.
                if let Some(qual) = ctx.resolve_call(current, &id.name) {
                    if ctx.fns.get(&qual).is_some_and(|d| !d.generics.is_empty()) {
                        for a in args {
                            carried_row(a, current, ctx, fnb, depth + 1, out);
                        }
                    }
                }
            }
        }
        // `xs.map(|x| || ...)` — the elements are what the closure returns.
        Expr::MethodCall { name, args, .. } => {
            if matches!(
                name.name.as_str(),
                "map" | "par_map" | "filter" | "par_filter" | "fold" | "find"
            ) {
                for a in args {
                    if let Expr::Closure { body, .. } = a {
                        carried_row(body, current, ctx, fnb, depth + 1, out);
                    }
                }
            }
        }
        Expr::Binary { lhs, rhs, .. } => {
            carried_row(lhs, current, ctx, fnb, depth + 1, out);
            carried_row(rhs, current, ctx, fnb, depth + 1, out);
        }
        Expr::List(items, _) | Expr::Tuple(items, _) => {
            for i in items {
                carried_row(i, current, ctx, fnb, depth + 1, out);
            }
        }
        Expr::Map(entries, _) => {
            for (_, v) in entries {
                carried_row(v, current, ctx, fnb, depth + 1, out);
            }
        }
        Expr::If { then_block, else_branch, .. } => {
            if let Some(Stmt::Expr(t)) = then_block.stmts.last() {
                carried_row(t, current, ctx, fnb, depth + 1, out);
            }
            match else_branch.as_deref() {
                Some(ElseBranch::Else(b)) => {
                    if let Some(Stmt::Expr(t)) = b.stmts.last() {
                        carried_row(t, current, ctx, fnb, depth + 1, out);
                    }
                }
                Some(ElseBranch::ElseIf(inner)) => {
                    carried_row(inner, current, ctx, fnb, depth + 1, out)
                }
                None => {}
            }
        }
        Expr::Match { arms, .. } => {
            for a in arms {
                carried_row(&a.body, current, ctx, fnb, depth + 1, out);
            }
        }
        Expr::Block(b) | Expr::Unsafe(b) | Expr::Region(b) => {
            if let Some(Stmt::Expr(t)) = b.stmts.last() {
                carried_row(t, current, ctx, fnb, depth + 1, out);
            }
        }
        _ => {}
    }
}

/// Parameters of function type, so calling one is attributed to its declared
/// row instead of to nothing.
fn seed_params<'a>(
    params: &[Param],
    fnb: &mut FnBindings<'a>,
    aliases: &HashMap<String, TypeExpr>,
    type_rows: &HashMap<String, BTreeSet<String>>,
) {
    for p in params {
        if let Some(row) = declared_row_within(&p.ty, aliases, type_rows) {
            fnb.signature_rows.extend(row);
        }
    }
    for p in params {
        // A parameter is in scope for the whole body, so it shadows a
        // function of the same name everywhere in it.
        fnb.bind_name(&p.name.name, 0);
        if let Some(row) = declared_row(&p.ty, aliases) {
            fnb.by_name
                .insert(p.name.name.clone(), vec![FnSource::Declared(row)]);
        }
    }
}

/// Every closure parameter in an expression, so a closure's own parameter
/// shadows a function of that name inside it.
///
/// `[1, 2].par_map(|shout| shout * 2)` was charged the row of a same-module
/// `fn shout` the closure never mentions, and the purity proof failed over
/// one parameter's spelling.
fn seed_closure_params(e: &Expr, fnb: &mut FnBindings<'_>) {
    if let Expr::Closure { params, body, .. } = e {
        for p in params {
            fnb.bind_name(&p.name, body.span().start);
        }
    }
    walk_exprs_shallow(e, &mut |inner| seed_closure_params(inner, fnb));
}

/// Every direct subexpression of `e`.
fn walk_exprs_shallow(e: &Expr, f: &mut impl FnMut(&Expr)) {
    match e {
        Expr::Closure { body, .. } | Expr::Unary { expr: body, .. } | Expr::Try { expr: body, .. } => f(body),
        Expr::Binary { lhs, rhs, .. } => {
            f(lhs);
            f(rhs);
        }
        Expr::Call { callee, args, .. } => {
            f(callee);
            for a in args {
                f(a);
            }
        }
        Expr::MethodCall { recv, args, .. } => {
            f(recv);
            for a in args {
                f(a);
            }
        }
        Expr::Field { recv, .. } => f(recv),
        Expr::Index { recv, index, .. } => {
            f(recv);
            f(index);
        }
        Expr::List(items, _) | Expr::Tuple(items, _) => {
            for i in items {
                f(i);
            }
        }
        Expr::Map(entries, _) => {
            for (k, v) in entries {
                f(k);
                f(v);
            }
        }
        Expr::Str(pieces, _) => {
            for p in pieces {
                if let StrPiece::Expr(i) = p {
                    f(i);
                }
            }
        }
        _ => {}
    }
}

/// Collects, in statement order, every name a body binds to a function value.
fn collect_fn_bindings<'a>(b: &'a Block, ctx: &Ctx, current: &str, fnb: &mut FnBindings<'a>) {
    for stmt in &b.stmts {
        match stmt {
            Stmt::Let { name, ty, init, .. } => {
                // An annotation is a written type in this function, exactly as
                // a parameter's is, and the fallback pool was seeded only from
                // the signature. So `let o: Option[fn(i64) -> i64 / {io}]` in
                // a `main` that takes nothing left the pool empty, and the
                // value it held reached `par_map` uncharged.
                if let Some(te) = ty {
                    if let Some(row) =
                        declared_row_within(te, &ctx.type_aliases, &ctx.type_rows)
                    {
                        fnb.signature_rows.extend(row);
                    }
                }
                fnb.bind_name(&name.name, name.span.start);
                seed_closure_params(init, fnb);
                let mut found = Vec::new();
                fn_sources(init, ctx, current, &mut found);
                if !found.is_empty() {
                    fnb.any.extend(found.iter().map(FnSource::clone_of));
                    fnb.by_name.insert(name.name.clone(), found);
                }
            }
            Stmt::For { var, iter, body, .. } => {
                fnb.bind_name(&var.name, var.span.start);
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
            Stmt::Expr(e) | Stmt::Return(Some(e), _) => seed_closure_params(e, fnb),
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
            FnSource::Unknown => FnSource::Unknown,
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
    // Nothing recorded: the name is a local holding a function value that no
    // binder this pass understands put there. A `for` variable over a list of
    // them, a `match` payload, a tuple destructuring, a closure parameter —
    // each of those called a `/ {io}` function and was charged nothing, while
    // `let f = fs[0]` one line away was charged correctly. Falling back to
    // every row the program writes without a field name is the same
    // conservatism `let f = fs[0]` already uses, and it errs toward refusing.
    let unaccounted = fnb.sources_for(name).is_empty()
        || fnb
            .sources_for(name)
            .iter()
            .any(|s| matches!(s, FnSource::Unknown));
    if unaccounted && !fnb.signature_rows.is_empty() {
        out.extend(
            fnb.signature_rows
                .iter()
                .filter(|e| !active.contains(e))
                .cloned(),
        );
    }
    for src in fnb.sources_for(name) {
        match src {
            FnSource::Named(n) => {
                // Already qualified when the value was named through its
                // module — `let g = lib.shout` records `lib::shout`, which is
                // the key, not a name to qualify again.
                if n.contains("::") {
                    contribute(n, active, ctx, out);
                } else if let Some(qual) = ctx.resolve_call(current, n) {
                    contribute(&qual, active, ctx, out);
                }
            }
            // Minus whatever a `with` around this mention has already
            // discharged. Without that, `with Log … { let f: fn(str) -> ()
            // / {Log} = shout }` charged `Log` to the enclosing function —
            // inside the block that handles it, and where the direct call
            // `shout("z")` and `["a"].map(shout)` both compile
            // ([T-EFFECT-DISCHARGE]).
            FnSource::Declared(row) => {
                out.extend(row.iter().filter(|e| !active.contains(e)).cloned())
            }
            // Accounted for by the fallback above.
            FnSource::Unknown => {}
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
#[allow(clippy::too_many_arguments)]
fn harvest_rows_at(
    t: &TypeExpr,
    field: Option<&str>,
    aliases: &HashMap<String, TypeExpr>,
    type_rows: &HashMap<String, BTreeSet<String>>,
    field_rows: &mut HashMap<String, BTreeSet<String>>,
    elem_rows: &mut BTreeSet<String>,
    depth: usize,
) {
    // `type A = [A]` resolves one alias and finds the shape holding it again,
    // so an unbounded walk here overflowed the process stack and aborted the
    // whole toolchain with exit 134 and no diagnostic — including a
    // consumer's, when the line was in a dependency. [G-NEST-LIMIT]: exceeding
    // a cap is a diagnostic, never a crash. The declaration is refused
    // elsewhere; this pass only has to stop.
    if depth > MAX_ALIAS_DEPTH {
        return;
    }
    // A `type` name stands for what it names, here as everywhere: a field
    // whose type was written as an alias contributed no row at all, so the
    // same program was refused with the row inline and accepted behind a name.
    match &resolve_alias(t, aliases, 0) {
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
                harvest_rows_at(p, None, aliases, type_rows, field_rows, elem_rows, depth + 1);
            }
            harvest_rows_at(ret, None, aliases, type_rows, field_rows, elem_rows, depth + 1);
        }
        // Inside any other type constructor the function type has lost its
        // field name, so what it carries joins the nameless pool.
        TypeExpr::List(inner, _) | TypeExpr::Ref(inner, _) => {
            harvest_rows_at(inner, None, aliases, type_rows, field_rows, elem_rows, depth + 1)
        }
        TypeExpr::Fallible(a, b, _) => {
            harvest_rows_at(a, None, aliases, type_rows, field_rows, elem_rows, depth + 1);
            harvest_rows_at(b, None, aliases, type_rows, field_rows, elem_rows, depth + 1);
        }
        TypeExpr::Tuple(items, _) | TypeExpr::Union(items, _) => {
            for i in items {
                harvest_rows_at(i, None, aliases, type_rows, field_rows, elem_rows, depth + 1);
            }
        }
        TypeExpr::Name { args, .. } => {
            for a in args {
                harvest_rows_at(a, None, aliases, type_rows, field_rows, elem_rows, depth + 1);
            }
        }
    }
}

/// Every `type N = ...` alias in one module, by name.
///
/// Types share one namespace across the program ([T-MODULE-NS]), so one map
/// over every module is the right shape.
fn collect_type_aliases(
    m: &grasm_syntax::ast::Module,
    out: &mut HashMap<String, TypeExpr>,
) {
    use grasm_syntax::ast::TypeBody;
    for d in &m.decls {
        if let Decl::Type(td) = d {
            if let TypeBody::Alias(t) = &td.body {
                out.insert(td.name.name.clone(), t.clone());
            }
        }
    }
}

/// Every effect row reachable from each declared type's own body.
///
/// Two passes, because a type may name another: the first reads the rows
/// written *directly* in each declaration, the second propagates along the
/// names they mention until nothing changes. A fixpoint rather than recursion,
/// so a self-referential type — `type Node = { next: Option[Node] }`, the
/// ordinary shape — terminates.
fn collect_type_rows(
    files: &[ModuleView],
    aliases: &HashMap<String, TypeExpr>,
) -> HashMap<String, BTreeSet<String>> {
    use grasm_syntax::ast::TypeBody;
    let mut own: HashMap<String, BTreeSet<String>> = HashMap::new();
    let mut mentions: HashMap<String, BTreeSet<String>> = HashMap::new();
    for f in files {
        for d in &f.module.decls {
            let Decl::Type(td) = d else {
                continue;
            };
            let parts: Vec<&TypeExpr> = match &td.body {
                TypeBody::Struct(fields) => fields.iter().map(|f| &f.ty).collect(),
                TypeBody::Sum(variants) => {
                    variants.iter().flat_map(|v| v.fields.iter().map(|f| &f.ty)).collect()
                }
                TypeBody::Alias(t) => vec![t],
            };
            let e_own = own.entry(td.name.name.clone()).or_default();
            for t in &parts {
                if let Some(r) = declared_row_shallow(t, aliases, 0) {
                    e_own.extend(r);
                }
            }
            let e_names = mentions.entry(td.name.name.clone()).or_default();
            for t in &parts {
                collect_type_names(t, aliases, e_names, 0);
            }
        }
    }
    // Propagate. Bounded by the number of declarations, since each round that
    // changes nothing stops the loop and each name can only be added once.
    for _ in 0..own.len() + 1 {
        let mut changed = false;
        let names: Vec<String> = own.keys().cloned().collect();
        for n in names {
            let from: BTreeSet<String> = mentions
                .get(&n)
                .into_iter()
                .flatten()
                .filter(|m| **m != n)
                .filter_map(|m| own.get(m).cloned())
                .flatten()
                .collect();
            let e = own.entry(n).or_default();
            let before = e.len();
            e.extend(from);
            changed |= e.len() != before;
        }
        if !changed {
            break;
        }
    }
    own
}

/// Every declared type name a written type mentions, aliases followed.
fn collect_type_names(
    te: &TypeExpr,
    aliases: &HashMap<String, TypeExpr>,
    out: &mut BTreeSet<String>,
    depth: usize,
) {
    // `type A = [A]` is a name that stands for a shape holding itself. Each
    // step through it resolves one alias and finds another, so an unbounded
    // walk here overflowed the process stack and aborted the whole toolchain
    // with exit 134 and no diagnostic — including a consumer's, when the line
    // was in a dependency. [G-NEST-LIMIT]: exceeding a cap is a diagnostic,
    // never a crash. The declaration is refused elsewhere; this pass only has
    // to stop.
    if depth > MAX_ALIAS_DEPTH {
        return;
    }
    match &resolve_alias(te, aliases, 0) {
        TypeExpr::Fn { params, ret, .. } => {
            for p in params {
                collect_type_names(p, aliases, out, depth + 1);
            }
            collect_type_names(ret, aliases, out, depth + 1);
        }
        TypeExpr::List(inner, _) | TypeExpr::Ref(inner, _) => {
            collect_type_names(inner, aliases, out, depth + 1)
        }
        TypeExpr::Fallible(a, b, _) => {
            collect_type_names(a, aliases, out, depth + 1);
            collect_type_names(b, aliases, out, depth + 1);
        }
        TypeExpr::Tuple(items, _) | TypeExpr::Union(items, _) => {
            for i in items {
                collect_type_names(i, aliases, out, depth + 1);
            }
        }
        TypeExpr::Name { path, args, .. } => {
            if let Some(seg) = path.segments.last() {
                out.insert(seg.name.clone());
            }
            for a in args {
                collect_type_names(a, aliases, out, depth + 1);
            }
        }
    }
}

/// The rows written *directly* inside a type, without following a name to
/// another declaration. Used to seed the fixpoint above.
fn declared_row_shallow(
    te: &TypeExpr,
    aliases: &HashMap<String, TypeExpr>,
    depth: usize,
) -> Option<BTreeSet<String>> {
    // Bounded for the same reason as `collect_type_names`.
    if depth > MAX_ALIAS_DEPTH {
        return None;
    }
    match &resolve_alias(te, aliases, 0) {
        TypeExpr::Fn { effects, .. } => Some(effects.iter().map(|p| p.dotted()).collect()),
        TypeExpr::List(inner, _) | TypeExpr::Ref(inner, _) => {
            declared_row_shallow(inner, aliases, depth + 1)
        }
        TypeExpr::Fallible(a, b, _) => union_rows(
            declared_row_shallow(a, aliases, depth + 1),
            declared_row_shallow(b, aliases, depth + 1),
        ),
        TypeExpr::Tuple(items, _) | TypeExpr::Union(items, _) => {
            let mut found: Option<BTreeSet<String>> = None;
            for i in items {
                found = union_rows(found, declared_row_shallow(i, aliases, depth + 1));
            }
            found
        }
        TypeExpr::Name { args, .. } => {
            let mut found: Option<BTreeSet<String>> = None;
            for a in args {
                found = union_rows(found, declared_row_shallow(a, aliases, depth + 1));
            }
            found
        }
    }
}

/// Harvests the rows every function type in one module declares.
fn harvest_module(
    m: &grasm_syntax::ast::Module,
    aliases: &HashMap<String, TypeExpr>,
    type_rows: &HashMap<String, BTreeSet<String>>,
    field_rows: &mut HashMap<String, BTreeSet<String>>,
    elem_rows: &mut BTreeSet<String>,
) {
    use grasm_syntax::ast::TypeBody;
    for d in &m.decls {
        match d {
            Decl::Type(td) => match &td.body {
                TypeBody::Struct(fields) => {
                    for f in fields {
                        harvest_rows_at(&f.ty, Some(&f.name.name), aliases, type_rows, field_rows, elem_rows, 0);
                    }
                }
                TypeBody::Sum(variants) => {
                    for v in variants {
                        for f in &v.fields {
                            harvest_rows_at(&f.ty, Some(&f.name.name), aliases, type_rows, field_rows, elem_rows, 0);
                        }
                    }
                }
                TypeBody::Alias(t) => harvest_rows_at(t, None, aliases, type_rows, field_rows, elem_rows, 0),
            },
            Decl::Fn(fd) => {
                for p in &fd.params {
                    harvest_rows_at(&p.ty, None, aliases, type_rows, field_rows, elem_rows, 0);
                }
                if let Some(r) = &fd.ret {
                    harvest_rows_at(r, None, aliases, type_rows, field_rows, elem_rows, 0);
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
                // Handlers written in *this* package. One written by a
                // consumer runs on the consumer's authority — it is charged
                // where it was written, and charging it here as well made a
                // library's ceiling a statement about somebody else's code.
                let here = ctx.package_of(current);
                for ((eff, module), row) in &ctx.handler_rows {
                    if eff != &effect.dotted() || ctx.package_of(module) != here {
                        continue;
                    }
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
            if !fnb.shadowed(&id.name, id.span.start) {
                if let Some(qual) = ctx.resolve_call(current, &id.name) {
                    contribute(&qual, active, ctx, out);
                }
            }
            if fnb.knows(&id.name) {
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
                } else if fnb.knows(&id.name) || !is_builtin_name(&id.name) {
                    // A call through a name that is not a top-level function:
                    // a local holding a function value. Whatever that value
                    // may be, calling it performs what it performs.
                    //
                    // Not for a *builtin* — `print`, `map`, `len`. Those are
                    // not locals, so the unknown-name fallback handed them
                    // every function value the body binds: a handler literal
                    // containing `print` was charged the row of an unrelated
                    // `let f = shout` elsewhere in the same function, and
                    // `main` was told it had an unhandled effect inside the
                    // `with` that handles it ([T-EFFECT-DISCHARGE]).
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
            // `lib.shout` as a value is the same permission as naming it
            // unqualified: whoever receives it can call it. Charged only for
            // the bare spelling, one `import` emptied the row, and with it
            // the `/ {}` check, [T-PAR-MAP-PURE], [T-REGION-FOREIGN] and the
            // capability ceiling.
            let mut qualified = false;
            if let Expr::Ident(alias) = &**recv {
                if let Some(target) = ctx.imports.get(current).and_then(|m| m.get(&alias.name)) {
                    if ctx.externs.contains(&format!("{target}::{}", name.name)) {
                        out.insert(IO.to_string());
                        out.insert(UNSAFE.to_string());
                        qualified = true;
                    } else if let Some(qual) =
                        ctx.resolve_qualified(current, &alias.name, &name.name)
                    {
                        contribute(&qual, active, ctx, out);
                        qualified = true;
                    }
                }
            }
            if !qualified {
                computed_callee_row(e, ctx, out);
                walk_expr(recv, current, active, ctx, fnb, out);
            }
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
