//! The Grasm typechecker: name resolution, bidirectional inference,
//! exhaustiveness, fallibility (`T ! E`), mutability of assignment targets.

use std::collections::HashMap;

use grasm_syntax::ast::*;
use grasm_syntax::diag::Diagnostic;
use grasm_syntax::span::Span;

use crate::types::*;

pub struct Analysis {
    pub defs: TypeDefs,
    pub diagnostics: Vec<Diagnostic>,
    /// `module::NAME` -> rendered type, for every exported constant.
    ///
    /// A constant's type is inferred, so it is not in the source for a tool to
    /// read — and without it `pub let WIDTH = 42` becoming `= "x"` looked like
    /// no change at all to `grasm api`.
    pub const_types: std::collections::BTreeMap<String, String>,
    /// Start offsets of the integer literals that sit in an `f64` position.
    ///
    /// [T-INT-TO-FLOAT] says such a literal "is converted at compile time",
    /// and only the checker knows which ones those are. Without this the
    /// interpreter kept them as `i64` at run time, so `xs.push(3)` into a
    /// `[f64]` stored an integer: the list then sorted wrong, compared
    /// unequal to the same list written `3.0`, and printed without its point.
    /// Spans are globally unique across a program — the loader gives each file
    /// a disjoint base — so the start offset identifies the literal.
    pub float_literals: std::collections::HashSet<u32>,
    /// Start offset of each empty `[]` or `{}` literal → the type inference
    /// settled on for it, written the way a person would.
    ///
    /// The compiled backend reads a literal's type off the literal, so an
    /// empty one had nothing to read and was refused by name — `xs.fold([], f)`
    /// and a `None -> []` match arm both ran under `grasm run` and could not
    /// be built. The checker resolves them by unifying with the whole body, so
    /// it is the one pass that knows, and this is how it says so.
    pub empty_types: std::collections::HashMap<u32, String>,
    /// Start offsets of the `sum()` calls whose result type is `f64`.
    ///
    /// Same reason as `float_literals`: the interpreter reads the type off the
    /// elements, and an empty `[f64]` has none to read.
    pub float_sums: std::collections::HashSet<u32>,
}

pub fn check(module: &Module) -> Analysis {
    let files = vec![ModuleView {
        name: "main".to_string(),
        module,
        imports: std::collections::HashMap::new(),
    }];
    check_modules(&files)
}

/// A module as the checker sees it (decoupled from the loader).
pub struct ModuleView<'a> {
    pub name: String,
    pub module: &'a Module,
    /// alias → full target module name
    pub imports: std::collections::HashMap<String, String>,
}

pub fn check_program(program: &crate::loader::Program) -> Analysis {
    let files: Vec<ModuleView> = program
        .files
        .iter()
        .map(|f| ModuleView {
            name: f.name.clone(),
            module: &f.module,
            imports: f.imports.clone(),
        })
        .collect();
    check_modules(&files)
}

fn check_modules(files: &[ModuleView]) -> Analysis {
    let mut c = Checker::new();
    // The effect fixpoint runs first: a function's row is part of its type,
    // and nothing can be typed until the rows are known ([T-FN-TYPE-ROW]).
    let (rows, closure_rows) = crate::effects::rows_and_closures(files);
    c.fn_rows = rows;
    c.closure_rows = closure_rows;
    for f in files {
        c.module_imports
            .insert(f.name.clone(), f.imports.clone());
    }
    for f in files {
        c.current_module = f.name.clone();
        c.collect_types(f.module);
    }
    // Before the bodies are resolved: `effect E` declares the type `EHandler`,
    // and a struct field may name one. Registering handlers afterwards meant
    // `type Holder = { h: LogHandler }` was "unknown type `LogHandler`" while
    // the same name worked as a parameter, a return type, a `let` annotation
    // and a list element — one namespace with a hole in it ([T-EFFECT-DECL],
    // [T-MODULE-NS]).
    for f in files {
        c.current_module = f.name.clone();
        c.collect_effects(f.module);
    }
    let modules: Vec<(String, &Module)> =
        files.iter().map(|f| (f.name.clone(), f.module)).collect();
    c.resolve_type_bodies(&modules);
    for f in files {
        c.current_module = f.name.clone();
        c.collect_fns(f.module);
    }
    // After the functions: a constant's value may call one, or name one as a
    // value. Collecting constants first meant `let DOUBLE = two() * 2` said
    // "unknown name `two`" and suggested declaring it with `let` — advice that
    // names the thing that was already declared.
    for f in files {
        c.current_module = f.name.clone();
        c.collect_externs(f.module);
    }
    for f in files {
        c.current_module = f.name.clone();
        c.collect_consts(f.module);
    }
    for f in files {
        c.current_module = f.name.clone();
        c.check_module(f.module);
    }
    // Resolved now: unification is finished, so a literal's element type has
    // whatever answer the whole body gave it.
    let empty_types: std::collections::HashMap<u32, String> = c
        .empty_lits
        .clone()
        .into_iter()
        .filter_map(|(off, t)| {
            let shown = c.show(&t);
            // A type inference never pinned down is not one the backend could
            // use either.
            (!shown.contains('_')).then_some((off, shown))
        })
        .collect();
    let mut diagnostics = c.diags;
    diagnostics.extend(crate::effects::check_effects_multi(files));
    Analysis {
        const_types: c
            .consts
            .iter()
            .filter(|(_, (_, is_pub))| *is_pub)
            .map(|(k, (t, _))| (k.clone(), TypeDisplay { ty: t, defs: &c.defs }.to_string()))
            .collect(),
        float_literals: c.float_literals,
        empty_types,
        float_sums: c.float_sums,
        defs: c.defs,
        diagnostics,
    }
}

#[derive(Clone)]
struct Binding {
    ty: Type,
    mutable: bool,
    /// Ownership was transferred with `own` — further use is an error.
    moved: bool,
    /// Loop nesting depth at declaration (for move-in-loop detection).
    decl_loop_depth: u32,
}

#[derive(Clone)]
struct FnInfo {
    is_pub: bool,
    generics: usize,
    params: Vec<Type>,
    modes: Vec<ParamMode>,
    ret: Type,
    err: Option<Type>,
}

#[derive(Clone)]
struct EffectOp {
    name: String,
    params: Vec<Type>,
    ret: Type,
}

struct Checker {
    defs: TypeDefs,
    aliases: HashMap<String, Type>,
    fns: HashMap<String, FnInfo>,
    /// Every function's inferred effect row, from the effects fixpoint run
    /// before checking. A function's row is part of its type, so this has to
    /// be known before anything is typed.
    fn_rows: HashMap<String, std::collections::BTreeSet<String>>,
    /// Each closure literal's row, keyed by span. See `rows_and_closures`.
    closure_rows: HashMap<u32, std::collections::BTreeSet<String>>,
    /// How many `unsafe` blocks enclose the expression being checked.
    unsafe_depth: u32,
    /// How many `region` blocks enclose it.
    /// Set while checking the expression a function's body ends with, so a
    /// mismatch there is reported as what it is — the function's return value
    /// — rather than as an anonymous type mismatch.
    tail_fn: Option<String>,
    /// Effects handled by an enclosing `with`, and the scope depth each
    /// block began at. Used to stop a value carrying a discharged effect out
    /// of the block that discharged it.
    with_floors: Vec<(String, usize)>,
    region_depth: u32,
    /// The scope depth at the start of each open region: a binding below its
    /// region's floor was declared outside and may not be written to.
    region_floors: Vec<usize>,
    /// Qualified names of declared C functions, so a use of one can be
    /// required to sit inside `unsafe` (Tier 3).
    extern_names: std::collections::HashSet<String>,
    /// Integer literals promoted to `f64`, by span start. See
    /// `Analysis::float_literals`.
    float_literals: std::collections::HashSet<u32>,
    /// See `Analysis::empty_types`. Collected as type variables and resolved
    /// once checking is over, since that is when they have answers.
    empty_lits: Vec<(u32, Type)>,
    /// `sum()` calls whose result is `f64`, by span start.
    float_sums: std::collections::HashSet<u32>,
    /// effect name → (ops, handler struct TypeDefId).
    effects: HashMap<String, (Vec<EffectOp>, TypeDefId)>,
    /// variant name → (typedef, variant index); ambiguous names keep all.
    variants: HashMap<String, Vec<(TypeDefId, usize)>>,
    uni: Unifier,
    diags: Vec<Diagnostic>,
    current_module: String,
    /// module → (alias → full target module name)
    module_imports: HashMap<String, HashMap<String, String>>,
    scopes: Vec<HashMap<String, Binding>>,
    /// Scope depth at which each enclosing closure body begins. A binding
    /// found below the innermost of these is captured, not local.
    closure_floors: Vec<usize>,
    /// Type parameters of the function being checked, so an annotation inside
    /// its body can name them. Without this `fn f[T](xs: [T]) { var o: [T] }`
    /// reported "unknown type `T`" and suggested declaring it with `type`,
    /// which is advice that cannot work.
    current_generics: Vec<String>,
    /// Closure bodies not yet checked, one list per open block.
    ///
    /// `let f = |p| p.n` says nothing about what `p` is; only a later
    /// `ps.sort_by(f)` does. A closure parameter cannot be annotated, so
    /// checking the body where it is written is checking it against nothing —
    /// which reported `_1 has no field n`, naming an inference variable the
    /// program never mentioned. The body waits until the end of its block, by
    /// which point the uses have had their say.
    deferred_closures: Vec<Vec<DeferredClosure>>,
    /// Types whose bodies are not resolved yet, by module.
    pending_type_bodies: Vec<(String, Vec<(usize, TypeDefId)>)>,
    /// Module-level constants: `module::NAME` to (type, is_pub).
    consts: HashMap<String, (Type, bool)>,
    /// (ret, err) of the function being checked.
    fn_ret: Type,
    fn_err: Option<Type>,
    /// True for main/tests: any error type may propagate (runtime reports).
    err_open: bool,
    /// Nesting depth of `parallel` scopes — `spawn` requires one.
    parallel_depth: u32,
    /// Scope depth where the innermost enclosing `parallel` block began.
    parallel_floor: Option<usize>,
    /// While checking a task body: (parallel floor, depth where this task body
    /// began). A name resolved between those two was declared *inside* the
    /// parallel block, and is gone by the time the task runs.
    task_capture: Option<(usize, usize)>,
    loop_depth: u32,
}

/// A closure whose body is checked once its parameter types are settled.
struct DeferredClosure {
    params: Vec<Ident>,
    body: Expr,
    param_tys: Vec<Type>,
    ret: Type,
}

impl Checker {
    fn new() -> Self {
        let defs = TypeDefs::new();
        let mut variants = HashMap::new();
        // Prelude Option + IoError variants.
        variants.insert("Some".to_string(), vec![(defs.option_id, 0)]);
        variants.insert("None".to_string(), vec![(defs.option_id, 1)]);
        variants.insert("NotFound".to_string(), vec![(defs.io_error_id, 0)]);
        variants.insert("IoFailed".to_string(), vec![(defs.io_error_id, 1)]);
        Checker {
            defs,
            aliases: HashMap::new(),
            fns: HashMap::new(),
            fn_rows: HashMap::new(),
            closure_rows: HashMap::new(),
            unsafe_depth: 0,
            tail_fn: None,
            with_floors: Vec::new(),
            region_depth: 0,
            region_floors: Vec::new(),
            extern_names: std::collections::HashSet::new(),
            float_literals: std::collections::HashSet::new(),
            empty_lits: Vec::new(),
            float_sums: std::collections::HashSet::new(),
            effects: HashMap::new(),
            variants,
            uni: Unifier::new(),
            diags: Vec::new(),
            current_module: "main".to_string(),
            module_imports: HashMap::new(),
            scopes: Vec::new(),
            closure_floors: Vec::new(),
            current_generics: Vec::new(),
            deferred_closures: Vec::new(),
            pending_type_bodies: Vec::new(),
            consts: HashMap::new(),
            fn_ret: Type::Unit,
            fn_err: None,
            err_open: false,
            parallel_depth: 0,
            parallel_floor: None,
            task_capture: None,
            loop_depth: 0,
        }
    }

    fn error(&mut self, span: Span, msg: impl Into<String>) -> Type {
        self.diags.push(Diagnostic::error(span, msg));
        Type::Error
    }

    fn error_help(&mut self, span: Span, msg: impl Into<String>, help: impl Into<String>) -> Type {
        self.diags
            .push(Diagnostic::error(span, msg).with_help(help));
        Type::Error
    }

    fn show(&self, ty: &Type) -> String {
        let resolved = self.uni.resolve(ty);
        TypeDisplay {
            ty: &resolved,
            defs: &self.defs,
        }
        .to_string()
    }

    // ------------------------------------------------------------------
    // Declaration collection
    // ------------------------------------------------------------------

    fn collect_types(&mut self, module: &Module) {
        // Pass 1: register names so recursive references resolve.
        let mut ids: Vec<(usize, TypeDefId)> = Vec::new();
        for (di, decl) in module.decls.iter().enumerate() {
            if let Decl::Type(t) = decl {
                if let Some(&existing) = self.defs.by_name.get(&t.name.name) {
                    if existing == self.defs.option_id || existing == self.defs.io_error_id {
                        self.error_help(
                            t.name.span,
                            format!("type name `{}` is reserved by the prelude", t.name.name),
                            "pick a different name (e.g. `AppError`)",
                        );
                    } else {
                        self.error(t.name.span, format!("duplicate type name `{}`", t.name.name));
                    }
                    continue;
                }
                if self.aliases.contains_key(&t.name.name) {
                    self.error(t.name.span, format!("duplicate type name `{}`", t.name.name));
                    continue;
                }
                match &t.body {
                    TypeBody::Alias(_) => {} // resolved in pass 2
                    _ => {
                        let id = self.defs.insert(TypeDef {
                            name: t.name.name.clone(),
                            generics: t.generics.iter().map(|g| g.name.clone()).collect(),
                            kind: TypeDefKind::Struct { fields: vec![] },
                        });
                        ids.push((di, id));
                    }
                }
            }
        }
        // Pass 2: aliases (non-recursive), then bodies.
        //
        // To a fixpoint, because an alias may name a later one. Resolving in
        // declaration order made `type Name = Text` above `type Text = str`
        // an "unknown type `Text`" that swapping the two lines fixed, while
        // an alias to a later *struct* worked — so [G-DECL-ORDER]'s "forward
        // references are permitted" held everywhere but here.
        let aliases: Vec<(&Ident, &TypeExpr, Vec<String>)> = module
            .decls
            .iter()
            .filter_map(|d| match d {
                Decl::Type(t) => match &t.body {
                    TypeBody::Alias(te) => Some((
                        &t.name,
                        te,
                        t.generics.iter().map(|g| g.name.clone()).collect(),
                    )),
                    _ => None,
                },
                _ => None,
            })
            .collect();
        for (name, _, generics) in &aliases {
            if !generics.is_empty() {
                self.error(
                    name.span,
                    "generic type aliases are not supported yet".to_string(),
                );
            }
        }
        let mut todo: Vec<usize> = (0..aliases.len()).collect();
        loop {
            let before = todo.len();
            let mut still: Vec<usize> = Vec::new();
            for &i in &todo {
                let (name, te, generics) = &aliases[i];
                let mark = self.diags.len();
                let ty = self.resolve_type(te, generics);
                if self.diags.len() > mark {
                    // Not resolvable yet; it may name an alias not seen.
                    self.diags.truncate(mark);
                    still.push(i);
                } else {
                    self.aliases.insert(name.name.clone(), ty);
                }
            }
            todo = still;
            if todo.is_empty() || todo.len() == before {
                break;
            }
        }
        // Whatever is left really is unknown; report it properly.
        for &i in &todo {
            let (name, te, generics) = &aliases[i];
            let ty = self.resolve_type(te, generics);
            self.aliases.insert(name.name.clone(), ty);
        }
        self.pending_type_bodies.push((self.current_module.clone(), ids));
    }

    /// Fills in the *bodies* of the types registered by `collect_types`.
    ///
    /// A separate pass because a field's type may live in any module, and
    /// resolving bodies inside `collect_types` meant a file processed before
    /// the one declaring the type reported `unknown type` — with a help that
    /// said to declare it, naming the thing that was already declared. The
    /// same ordering bug effects had until C15.
    fn resolve_type_bodies(&mut self, modules: &[(String, &Module)]) {
        let pending = std::mem::take(&mut self.pending_type_bodies);
        for (module_name, ids) in pending {
            let Some((_, module)) = modules.iter().find(|(n, _)| *n == module_name) else {
                continue;
            };
            self.current_module = module_name.clone();
            for (di, id) in ids {
                let Decl::Type(t) = &module.decls[di] else {
                    continue;
                };
                // `pub` on a type means nothing: types share one namespace
                // across the whole program ([T-MODULE-NS]), so a type declared
                // without it is already reachable everywhere. Accepting the
                // word anyway is the `pub let` mistake again — syntax that
                // parses, checks, and does nothing, which reads as a
                // permission being granted.
                if t.is_pub {
                    self.error_help(
                        t.name.span,
                        format!("`pub` means nothing on type `{}`", t.name.name),
                        "types are one namespace across the program, so every type is \
already reachable from every module — drop the `pub`",
                    );
                }
                let generics: Vec<String> = t.generics.iter().map(|g| g.name.clone()).collect();
                let kind = match &t.body {
                    TypeBody::Struct(fields) => {
                        // (visibility is checked for every type body below)
                        // A zero-field struct has no literal: `{}` after a
                        // name is a *block*, which is what keeps
                        // `handled_by h { … }` unambiguous. So `type E = { }`
                        // declared a type no value of which could be written,
                        // and `E{}` failed with "expected end of line after
                        // statement" — a parse error about the wrong thing,
                        // for a declaration the grammar had offered
                        // ([G-STRUCT-NONEMPTY]).
                        if fields.is_empty() {
                            self.error_help(
                                t.name.span,
                                format!("struct `{}` has no fields", t.name.name),
                                "a struct needs at least one field, because `Name{}` is a block and there would be no way to write a value; for a value that carries nothing use a sum variant, as in `type Marker = | Done`",
                            );
                        }
                        let mut fs = Vec::new();
                        for f in fields {
                            let ty = self.resolve_type(&f.ty, &generics);
                            if fs.iter().any(|(n, _)| n == &f.name.name) {
                                self.error(f.name.span, format!("duplicate field `{}`", f.name.name));
                                continue;
                            }
                            fs.push((f.name.name.clone(), ty));
                        }
                        TypeDefKind::Struct { fields: fs }
                    }
                    TypeBody::Sum(variants) => {
                        let mut vs = Vec::new();
                        for (vi, v) in variants.iter().enumerate() {
                            let mut fs = Vec::new();
                            for f in &v.fields {
                                let ty = self.resolve_type(&f.ty, &generics);
                                fs.push((f.name.name.clone(), ty));
                            }
                            self.variants
                                .entry(v.name.name.clone())
                                .or_default()
                                .push((id, vi));
                            vs.push(Variant {
                                name: v.name.name.clone(),
                                fields: fs,
                            });
                        }
                        TypeDefKind::Sum { variants: vs }
                    }
                    TypeBody::Alias(_) => continue,
                };
                self.defs.defs[id].kind = kind;
            }
        }
    }

    /// Effects, in a pass of their own.
    ///
    /// An operation's signature may name a type from any module, and every
    /// module's types are registered by the time this runs. Doing it inside
    /// `collect_types` meant a file processed before the one declaring the
    /// type reported `unknown type` — and the help said to declare it with
    /// `type`, which then said `duplicate type name`. Two rules, neither of
    /// which could be satisfied.
    fn collect_effects(&mut self, module: &Module) {
        for decl in &module.decls {
            if let Decl::Effect(e) = decl {
                if self.effects.contains_key(&e.name.name) {
                    self.error(e.name.span, format!("duplicate effect `{}`", e.name.name));
                    continue;
                }
                let mut ops = Vec::new();
                let mut handler_fields = Vec::new();
                for op in &e.ops {
                    // An effect operation cannot take `inout`. The handler is
                    // called through a function value, so the write lands in a
                    // copy the caller never sees — it typechecked, ran, and
                    // silently discarded the write, with every engine
                    // agreeing on the wrong answer. Refusing it at the
                    // declaration is what [T-CONST-PUB] and [T-TYPE-NO-PUB]
                    // do for the same disease: syntax that parses and does
                    // nothing is worse than syntax that is rejected.
                    for prm in &op.params {
                        if prm.mode == ParamMode::Inout {
                            self.error_help(
                                prm.name.span,
                                format!(
                                    "effect operation `{}` cannot take an `inout` parameter",
                                    op.name.name
                                ),
                                "a handler runs through a function value, so the write would \
land in a copy the caller never sees — return the new value from the operation instead",
                            );
                        }
                    }
                    let params: Vec<Type> =
                        op.params.iter().map(|p| self.resolve_type(&p.ty, &[])).collect();
                    let mut ret = op
                        .ret
                        .as_ref()
                        .map(|t| self.resolve_type(t, &[]))
                        .unwrap_or(Type::Unit);
                    // `fn get(k: str) -> f64 ! Diag` — an operation that can
                    // fail. The grammar has always allowed this and the
                    // checker used to drop the error type on the floor, so an
                    // error raised in a handler crossed every function on the
                    // way out while `check` reported clean. Making it part of
                    // the operation's type is what puts it back under
                    // [T-FALLIBLE-UNHANDLED]: the caller must `try` it, and
                    // `try` needs a declared error type of its own.
                    if let Some(et) = &op.err {
                        let e = self.resolve_type(et, &[]);
                        ret = Type::Fallible(Box::new(ret), Box::new(e));
                    }
                    handler_fields.push((
                        op.name.name.clone(),
                        Type::Fn {
                            params: params.clone(),
                            ret: Box::new(ret.clone()),
                            row: None,
                        },
                    ));
                    ops.push(EffectOp {
                        name: op.name.name.clone(),
                        params,
                        ret,
                    });
                }
                let handler_name = format!("{}Handler", e.name.name);
                let handler_id = self.defs.insert(TypeDef {
                    name: handler_name,
                    generics: vec![],
                    kind: TypeDefKind::Struct {
                        fields: handler_fields,
                    },
                });
                self.effects.insert(e.name.name.clone(), (ops, handler_id));
            }
        }
    }

    /// Module-level constants.
    ///
    /// The initializer is checked here, so a constant may name a type or a
    /// function from any module but not another constant declared later —
    /// which keeps "what does this name mean" answerable by reading forwards.
    fn collect_consts(&mut self, module: &Module) {
        for decl in &module.decls {
            let Decl::Const(c) = decl else { continue };
            let key = format!("{}::{}", self.current_module, c.name.name);
            if self.consts.contains_key(&key) {
                self.error(c.name.span, format!("duplicate constant `{}`", c.name.name));
                continue;
            }
            let ty = match &c.ty {
                Some(te) => {
                    let want = self.resolve_type(te, &[]);
                    self.check_expr(&c.init, &want);
                    want
                }
                None => self.infer_expr(&c.init),
            };
            self.consts.insert(key, (self.uni.resolve(&ty), c.is_pub));
        }
    }

    /// C functions, registered as ordinary functions with an `io` effect.
    ///
    /// Their effects are unknowable, so the conservative row is the one that
    /// says "this touches the outside world" — which is what keeps a package's
    /// capability ceiling meaningful across an FFI call.
    fn collect_externs(&mut self, module: &Module) {
        for decl in &module.decls {
            let Decl::Extern(e) = decl else { continue };
            let key = format!("{}::{}", self.current_module, e.name.name);
            if self.fns.contains_key(&key) {
                self.error(e.name.span, format!("duplicate function `{}`", e.name.name));
                continue;
            }
            self.extern_names.insert(key.clone());
            let params: Vec<Type> = e
                .params
                .iter()
                .map(|p| self.resolve_type(&p.ty, &[]))
                .collect();
            let ret = e
                .ret
                .as_ref()
                .map(|t| self.resolve_type(t, &[]))
                .unwrap_or(Type::Unit);
            for (p, t) in e.params.iter().zip(&params) {
                if !Self::is_c_abi_type(t) {
                    let shown = self.show(t);
                    self.error_help(
                        p.ty.span(),
                        format!("`{shown}` has no C representation"),
                        "an `extern fn` takes and returns `i64` or `f64`",
                    );
                }
            }
            if ret != Type::Unit && !Self::is_c_abi_type(&ret) {
                let shown = self.show(&ret);
                self.error_help(
                    e.span,
                    format!("`{shown}` has no C representation"),
                    "an `extern fn` takes and returns `i64` or `f64`",
                );
            }
            self.fns.insert(
                key,
                FnInfo {
                    is_pub: e.is_pub,
                    generics: 0,
                    params,
                    modes: e.params.iter().map(|p| p.mode).collect(),
                    ret,
                    err: None,
                },
            );
        }
    }

    /// Types that cross the C boundary unchanged.
    ///
    /// `bool` is deliberately excluded: C's is one byte and Grasm's is a
    /// register, and a silent width mismatch at an FFI boundary is the kind of
    /// bug that shows up as garbage three calls later.
    fn is_c_abi_type(t: &Type) -> bool {
        matches!(t, Type::I64 | Type::F64)
    }

    fn collect_fns(&mut self, module: &Module) {
        for decl in &module.decls {
            if let Decl::Fn(f) = decl {
                let key = format!("{}::{}", self.current_module, f.name.name);
                if self.fns.contains_key(&key) {
                    self.error(
                        f.name.span,
                        format!("duplicate function name `{}`", f.name.name),
                    );
                    continue;
                }
                let generics: Vec<String> = f.generics.iter().map(|g| g.name.clone()).collect();
                let params = f
                    .params
                    .iter()
                    .map(|p| self.resolve_type(&p.ty, &generics))
                    .collect();
                let ret = f
                    .ret
                    .as_ref()
                    .map(|t| self.resolve_type(t, &generics))
                    .unwrap_or(Type::Unit);
                let err = f.err.as_ref().map(|t| self.resolve_type(t, &generics));
                // `type Res = i64 ! E` used as a return type: the `! E` is
                // inside the alias, not written on the declaration, so the
                // error half has to be taken out of the resolved type or the
                // function looks infallible and `fail` inside it is an error.
                let (ret, err) = match (self.uni.shallow_resolve(&ret), &err) {
                    (Type::Fallible(ok, e), None) => (*ok, Some(*e)),
                    _ => (ret, err),
                };
                self.fns.insert(
                    key,
                    FnInfo {
                        is_pub: f.is_pub,
                        generics: generics.len(),
                        params,
                        modes: f.params.iter().map(|p| p.mode).collect(),
                        ret,
                        err,
                    },
                );
            }
        }
    }

    /// TypeExpr → Type. `generics` are the in-scope generic parameter names.
    fn resolve_type(&mut self, te: &TypeExpr, generics: &[String]) -> Type {
        match te {
            TypeExpr::Ref(inner, _) => {
                Type::Ref(Box::new(self.resolve_type(inner, generics)))
            }
            TypeExpr::Fn { params, ret, effects, .. } => {
                let ps = params.iter().map(|p| self.resolve_type(p, generics)).collect();
                // The row is part of the type, not decoration on the syntax.
                // A bare `fn(i64) -> i64` is the *empty* row, which is what
                // makes it reject a function that prints ([T-FN-TYPE-ROW]).
                Type::Fn {
                    params: ps,
                    ret: Box::new(self.resolve_type(ret, generics)),
                    row: Some(effects.iter().map(|e| e.dotted()).collect()),
                }
            }
            TypeExpr::Name { path, args, span } => {
                if path.segments.len() > 1 {
                    // Not "yet" — never. Types share one global namespace
                    // ([T-MODULE-NS]), so the qualifier says nothing the bare
                    // name does not ([G-TYPE-NOT-QUALIFIED]).
                    let bare = path
                        .segments
                        .last()
                        .map(|s| s.name.clone())
                        .unwrap_or_default();
                    return self.error_help(
                        *span,
                        "a type is named without a module qualifier".to_string(),
                        format!("types share one namespace across the program — write `{bare}`"),
                    );
                }
                let name = &path.segments[0].name;
                if let Some(i) = generics.iter().position(|g| g == name) {
                    if !args.is_empty() {
                        return self.error(*span, "generic parameters cannot take arguments");
                    }
                    return Type::Generic(i as u32);
                }
                let prim = match name.as_str() {
                    "i8" | "i16" | "i32" | "i64" | "u8" | "u16" | "u32" | "u64" | "usize"
                    | "int" => Some(Type::I64),
                    "f32" | "f64" | "float" => Some(Type::F64),
                    "bool" => Some(Type::Bool),
                    "str" => Some(Type::Str),
                    _ => None,
                };
                if let Some(p) = prim {
                    if !args.is_empty() {
                        return self.error(*span, format!("`{name}` cannot take type arguments"));
                    }
                    return p;
                }
                if name == "Map" {
                    let resolved_args: Vec<Type> = args
                        .iter()
                        .map(|a| self.resolve_type(a, generics))
                        .collect();
                    if resolved_args.len() != 2 {
                        return self.error(*span, "`Map` takes exactly 2 type arguments: Map[K, V]");
                    }
                    self.require_map_key(&resolved_args[0], *span);
                    return Type::Map(
                        Box::new(resolved_args[0].clone()),
                        Box::new(resolved_args[1].clone()),
                    );
                }
                if let Some(alias) = self.aliases.get(name) {
                    return alias.clone();
                }
                if let Some(&id) = self.defs.by_name.get(name) {
                    let want = self.defs.get(id).generics.len();
                    let resolved_args: Vec<Type> = args
                        .iter()
                        .map(|a| self.resolve_type(a, generics))
                        .collect();
                    if resolved_args.len() != want {
                        return self.error(
                            *span,
                            format!(
                                "type `{name}` expects {want} type argument(s), got {}",
                                resolved_args.len()
                            ),
                        );
                    }
                    return Type::Named {
                        id,
                        args: resolved_args,
                    };
                }
                self.error_help(
                    *span,
                    format!("unknown type `{name}`"),
                    "declare it with `type` or check the spelling",
                )
            }
            TypeExpr::List(inner, _) => Type::List(Box::new(self.resolve_type(inner, generics))),
            TypeExpr::Fallible(ok, err, _) => Type::Fallible(
                Box::new(self.resolve_type(ok, generics)),
                Box::new(self.resolve_type(err, generics)),
            ),
            TypeExpr::Tuple(items, _) => {
                if items.is_empty() {
                    Type::Unit
                } else {
                    Type::Tuple(
                        items
                            .iter()
                            .map(|t| self.resolve_type(t, generics))
                            .collect(),
                    )
                }
            }
            TypeExpr::Union(members, _) => Type::Union(
                members
                    .iter()
                    .map(|t| self.resolve_type(t, generics))
                    .collect(),
            ),
        }
    }

    // ------------------------------------------------------------------
    // Function bodies
    // ------------------------------------------------------------------

    fn check_module(&mut self, module: &Module) {
        for decl in &module.decls {
            match decl {
                Decl::Fn(f) => self.check_fn(f),
                Decl::Test(t) => {
                    self.fn_ret = Type::Unit;
                    self.fn_err = Some(self.uni.fresh()); // tests may propagate any error
                    self.err_open = true;
                    let mut scope = HashMap::new();
                    // A parameter list makes this a property test: the runner
                    // supplies the values, so each type must be generatable.
                    for p in &t.params {
                        let ty = self.resolve_type(&p.ty, &[]);
                        if p.mode != ParamMode::Let {
                            self.error(
                                p.span,
                                "a property test parameter is supplied by the runner, so it \
                                 cannot be `inout` or `own`",
                            );
                        }
                        if !self.generatable(&ty) {
                            let shown = self.show(&ty);
                            self.error_help(
                                p.span,
                                format!("cannot generate values of type `{shown}`"),
                                "property tests generate `i64`, `f64`, `bool`, `str`, lists of \
                                 those, and `Option` of those",
                            );
                        }
                        scope.insert(
                            p.name.name.clone(),
                            Binding {
                                ty,
                                mutable: false,
                                moved: false,
                                decl_loop_depth: 0,
                            },
                        );
                    }
                    self.scopes.push(scope);
                    self.check_block(&t.body);
                    self.scopes.pop();
                }
                Decl::Bench(b) => {
                    self.fn_ret = Type::Unit;
                    self.fn_err = Some(self.uni.fresh());
                    self.err_open = true;
                    self.scopes.push(HashMap::new());
                    self.check_block(&b.body);
                    self.scopes.pop();
                }
                _ => {}
            }
        }
    }

    fn check_fn(&mut self, f: &FnDecl) {
        let info = match self.lookup_fn(&f.name.name) {
            Some(i) => i,
            None => return,
        };
        // Generic params stay opaque (`Generic(i)`) while checking the body,
        // and are in scope for annotations written inside it.
        self.current_generics = f.generics.iter().map(|g| g.name.clone()).collect();
        self.fn_ret = info.ret.clone();
        self.err_open = f.name.name == "main" && info.err.is_none();
        self.fn_err = if f.name.name == "main" && info.err.is_none() {
            Some(self.uni.fresh()) // main may propagate; runtime reports
        } else {
            info.err.clone()
        };
        let mut scope = HashMap::new();
        for (p, ty) in f.params.iter().zip(&info.params) {
            scope.insert(
                p.name.name.clone(),
                Binding {
                    ty: ty.clone(),
                    mutable: p.mode == ParamMode::Inout,
                    moved: false,
                    decl_loop_depth: 0,
                },
            );
        }
        self.scopes.push(scope);
        let ret = info.ret.clone();
        // The tail expression is the return value, so it is checked against
        // the declared return type rather than inferred and compared after.
        // A fallible function's body may also produce the whole `T ! E`, and
        // that is decided below, so it is not pushed down here.
        // A fallible function's body may produce either the ok half or the
        // whole `T ! E`, so the tail is only pushed the ok half when it
        // cannot be the whole thing — a literal, or a container of them.
        // Skipping the push entirely meant `fn a() -> f64 ! E { 3 }` was
        // refused while `{ return 3 }` and `let x: f64 ! E = Ok(3)` were
        // accepted: [T-INT-TO-FLOAT-CARRIED] survived, one `! E` away.
        let tail_is_literal = matches!(
            f.body.stmts.last(),
            Some(Stmt::Expr(
                Expr::Int(..) | Expr::Float(..) | Expr::List(..) | Expr::Tuple(..) | Expr::Map(..)
            ))
        );
        // `Ok(3)` / `Err(e)` in tail position build the *whole* `T ! E`, so
        // that is what reaches them ([T-OK-ERR]).
        let tail_is_wrapper = matches!(
            f.body.stmts.last(),
            Some(Stmt::Expr(Expr::Call { callee, .. }))
                if matches!(&**callee, Expr::Ident(id) if id.name == "Ok" || id.name == "Err")
        );
        let want_tail = if ret == Type::Unit {
            None
        } else if let (true, Some(e)) = (tail_is_wrapper, self.fn_err.clone()) {
            Some(Type::Fallible(Box::new(ret.clone()), Box::new(e)))
        } else if self.fn_err.is_none() || tail_is_literal {
            Some(ret.clone())
        } else {
            None
        };
        // Only when the body actually ends in an expression: an explicit
        // `return` is not "the last expression of the body", and saying so
        // would attach the wrong advice to a plain type mismatch.
        if want_tail.is_some() && matches!(f.body.stmts.last(), Some(Stmt::Expr(_))) {
            self.tail_fn = Some(f.name.name.clone());
        }
        let body_ty = self.check_block_with(&f.body, want_tail.as_ref());
        self.tail_fn = None;
        let ends_with_return = matches!(f.body.stmts.last(), Some(Stmt::Return(..)));
        // A fallible function's body may produce either the ok half — the
        // ordinary case, wrapped in `Ok` on the way out — or the whole
        // `T ! E`, which is what `Ok(v)` and `Err(e)` build ([T-OK-ERR]).
        // Only the first was accepted, so the constructors worked everywhere
        // except the position a function's own result is written in.
        let whole = self.fn_err.clone().map(|e| {
            Type::Fallible(Box::new(ret.clone()), Box::new(e))
        });
        let fits_whole = whole
            .as_ref()
            .is_some_and(|w| self.uni.unify(&body_ty, w));
        // The row check has to run here too: a function body's value is
        // compared with `unify`, not `check_expr`, so returning a named
        // function was the one coercion nothing looked at.
        if !ends_with_return {
            self.check_row_fits(&body_ty, &ret, f.body.span);
        }
        if ret != Type::Unit && !ends_with_return && !fits_whole && !self.uni.unify(&body_ty, &ret) {
            let span = f
                .body
                .stmts
                .last()
                .map(stmt_span)
                .unwrap_or(f.body.span);
            let want = self.show(&ret);
            let got = self.show(&body_ty);
            self.error_help(
                span,
                format!("function `{}` must return `{want}` but its body produces `{got}`", f.name.name),
                "the last expression of the body is the return value",
            );
        }
        self.scopes.pop();
        self.current_generics.clear();
    }

    // ------------------------------------------------------------------
    // Statements & blocks
    // ------------------------------------------------------------------

    /// Returns the block's value type (last expression statement, else unit).
    fn check_block(&mut self, b: &Block) -> Type {
        self.check_block_with(b, None)
    }

    /// Checks a block, pushing an expected type into its tail expression.
    ///
    /// The tail is the block's value, so a type known at the outside is known
    /// there too. Without this an integer literal in tail position had no
    /// expected type to adopt: `fn f() -> f64 { 3 }` was rejected while
    /// `fn f() -> f64 { return 3 }` — the same value, one keyword apart —
    /// was accepted and printed `3.0` ([T-INT-TO-FLOAT]).
    fn check_block_with(&mut self, b: &Block, want: Option<&Type>) -> Type {
        self.scopes.push(HashMap::new());
        self.deferred_closures.push(Vec::new());
        let mut ty = Type::Unit;
        let n = b.stmts.len();
        for (i, stmt) in b.stmts.iter().enumerate() {
            match (i + 1 == n, want, stmt) {
                (true, Some(w), Stmt::Expr(e)) if *w != Type::Unit => {
                    let w = w.clone();
                    self.check_expr(e, &w);
                    ty = w;
                }
                _ => {
                    ty = self.check_stmt(stmt);
                    // A statement that is not the block's value throws that
                    // value away, and throwing away a failure is using a
                    // fallible value as if it were `T`. `write_file` to an
                    // unwritable path printed "wrote the report", exit 0,
                    // nothing on stderr and no file — and every user function
                    // that can fail did the same ([T-FALLIBLE-UNHANDLED]).
                    if i + 1 < n
                        && matches!(stmt, Stmt::Expr(_))
                        && matches!(self.uni.shallow_resolve(&ty), Type::Fallible(..))
                    {
                        let shown = self.show(&ty);
                        self.error_help(
                            stmt_span(stmt),
                            format!(
                                "this expression can fail (`{shown}`) and its failure is discarded"
                            ),
                            "use `try` to propagate it, `match` to handle both cases, or `??` \
with a fallback to say what should happen instead",
                        );
                    }
                }
            }
            if i + 1 < n {
                ty = Type::Unit;
            }
        }
        // Every use in this block has now been seen, so the deferred bodies
        // have types to be checked against. Checking one can defer another.
        while let Some(next) = self.deferred_closures.last_mut().and_then(|v| v.pop()) {
            self.check_deferred_closure(next);
        }
        self.deferred_closures.pop();
        self.scopes.pop();
        ty
    }

    fn check_deferred_closure(&mut self, d: DeferredClosure) {
        self.scopes.push(HashMap::new());
        self.closure_floors.push(self.scopes.len() - 1);
        for (p, t) in d.params.iter().zip(&d.param_tys) {
            self.bind(p, t.clone(), false);
        }
        let produced = self.infer_expr(&d.body);
        self.closure_floors.pop();
        self.scopes.pop();
        if !self.uni.unify(&produced, &d.ret) {
            let got = self.show(&produced);
            let want = self.show(&d.ret);
            self.error(
                d.body.span(),
                format!("this closure produces `{got}` but `{want}` is expected here"),
            );
        }
    }

    fn check_stmt(&mut self, s: &Stmt) -> Type {
        match s {
            // `let (a, b) = pair`. The initializer is unified with a tuple of
            // fresh variables, so each name gets its component's type.
            Stmt::LetTuple {
                mutable,
                names,
                init,
                span,
            } => {
                let parts: Vec<Type> = names.iter().map(|_| self.uni.fresh()).collect();
                let want = Type::Tuple(parts.clone());
                self.check_expr(init, &want);
                let got = self.uni.resolve(&want);
                if let Type::Tuple(actual) = &got {
                    if actual.len() != names.len() {
                        self.error(
                            *span,
                            format!(
                                "this tuple has {} component(s) but {} name(s) were given",
                                actual.len(),
                                names.len()
                            ),
                        );
                    }
                }
                for (n, t) in names.iter().zip(&parts) {
                    let t = self.uni.resolve(t);
                    self.bind(n, t, *mutable);
                }
                Type::Unit
            }
            Stmt::Let {
                mutable,
                name,
                ty,
                init,
                ..
            } => {
                let bound_ty = if let Some(te) = ty {
                    let gs = self.current_generics.clone();
                    let want = self.resolve_type(te, &gs);
                    self.check_expr(init, &want);
                    want
                } else {
                    let t = self.infer_expr(init);
                    let resolved = self.uni.shallow_resolve(&t);
                    if let Type::Fallible(..) = resolved {
                        let shown = self.show(&resolved);
                        self.error_help(
                            init.span(),
                            "this expression can fail — its error must be handled",
                            format!("use `try` to propagate it, `match` to handle both cases, or annotate the binding `: {shown}` to hold it as a value"),
                        );
                    }
                    t
                };
                self.bind(name, bound_ty, *mutable);
                Type::Unit
            }
            Stmt::Assign {
                target, op, value, ..
            } => {
                // Re-initializing a moved variable revives it.
                if *op == AssignOp::Assign {
                    if let Expr::Ident(id) = target {
                        let name = id.name.clone();
                        if let Some(b) = self.lookup_mut(&name) {
                            b.moved = false;
                        }
                    }
                }
                self.check_assign_target(target);
                let target_ty = self.infer_expr(target);
                self.reject_handler_escape(target, &target_ty);
                if *op == AssignOp::Assign {
                    self.check_expr(value, &target_ty);
                } else {
                    // Compound: numeric (or str for +=).
                    self.check_expr(value, &target_ty);
                    let t = self.uni.shallow_resolve(&target_ty);
                    let ok = matches!(t, Type::I64 | Type::F64 | Type::Error)
                        || (t == Type::Str && *op == AssignOp::Add);
                    if !ok {
                        let shown = self.show(&target_ty);
                        self.error(
                            target.span(),
                            format!("compound assignment needs a numeric target, found `{shown}`"),
                        );
                    }
                }
                Type::Unit
            }
            Stmt::Expr(e) => self.infer_expr(e),
            Stmt::Return(value, span) => {
                // A `return` through a region's brace carries a value out of a
                // scope that promises nothing leaves it — and skips the sweep
                // that would free the region's allocations.
                if self.region_depth > 0 {
                    self.error_help(
                        *span,
                        "cannot `return` from inside a region".to_string(),
                        "a region promises that nothing allocated inside outlives the \
closing brace — yield a number from the region and return that",
                    );
                }
                // Returning out of a `with` carries the value past the
                // handler exactly as assigning to an outer binding does —
                // and the escape rule covered assignment and container-push
                // but not this. `fn mk() -> fn() -> i64 / {Cfg} { with Cfg
                // handled_by … { return || Cfg.value() } }` checked clean and
                // panicked at the call ([E-HANDLER-MISSING]).
                if let Some(v) = value.as_ref() {
                    let rt = self.fn_ret.clone();
                    if !self.with_floors.is_empty() {
                        let row = self.row_carried_by(&rt);
                        let escaping: Vec<String> = self
                            .with_floors
                            .iter()
                            .map(|(e, _)| e.clone())
                            .filter(|e| row.contains(e))
                            .collect();
                        if let Some(first) = escaping.first() {
                            self.error_help(
                                v.span(),
                                format!(
                                    "cannot return a function that performs `{first}` from inside a `with` block"
                                ),
                                "the handler is installed only for this block, so calling the value afterwards has nowhere to send the effect — call it here, or return what it computed",
                            );
                        }
                    }
                }
                let ret = self.fn_ret.clone();
                match value {
                    Some(e) => {
                        self.check_expr(e, &ret);
                    }
                    None => {
                        if self.uni.shallow_resolve(&ret) != Type::Unit {
                            let want = self.show(&ret);
                            self.error(*span, format!("this function must return `{want}`"));
                        }
                    }
                }
                // Divergent: control leaves before any value is produced, so
                // this fits wherever it is written — the same rule `fail` has
                // ([T-FAIL]). Without it `0 -> return false` typed the arm as
                // `()` and the error landed on the *other* arm.
                self.uni.fresh()
            }
            Stmt::Break(span) | Stmt::Continue(span) => {
                if self.loop_depth == 0 {
                    let what = if matches!(s, Stmt::Break(_)) {
                        "break"
                    } else {
                        "continue"
                    };
                    self.error(*span, format!("`{what}` outside of a loop"));
                }
                self.uni.fresh() // divergent, as `return` above
            }
            Stmt::While { cond, body, .. } => {
                self.check_expr(cond, &Type::Bool);
                self.loop_depth += 1;
                self.check_block(body);
                self.loop_depth -= 1;
                Type::Unit
            }
            Stmt::Parallel { body, .. } => {
                self.parallel_depth += 1;
                let saved_floor = self.parallel_floor;
                // Bindings from here down live only for the collection pass;
                // tasks run after this block's scope is gone.
                self.parallel_floor = Some(self.scopes.len());
                self.check_block(body);
                self.parallel_floor = saved_floor;
                self.parallel_depth -= 1;
                Type::Unit
            }
            Stmt::Spawn { body, span } => {
                if self.parallel_depth == 0 {
                    self.error_help(
                        *span,
                        "`spawn` outside a `parallel` block",
                        "wrap it in `parallel { ... }` so the task is joined before the scope exits",
                    );
                }
                // A task body is checked in its own scope; `break`/`continue`
                // cannot cross the task boundary.
                let saved_loop = self.loop_depth;
                let saved_capture = self.task_capture;
                self.loop_depth = 0;
                if let Some(floor) = self.parallel_floor {
                    self.task_capture = Some((floor, self.scopes.len()));
                }
                self.check_block(body);
                self.task_capture = saved_capture;
                self.loop_depth = saved_loop;
                Type::Unit
            }
            Stmt::With {
                effect,
                handler,
                body,
                span,
            } => {
                let ename = effect.dotted();
                match self.effects.get(&ename).cloned() {
                    Some((_, handler_id)) => {
                        let want = Type::Named {
                            id: handler_id,
                            args: vec![],
                        };
                        self.check_expr(handler, &want);
                    }
                    None => {
                        self.error_help(
                            *span,
                            format!("unknown effect `{ename}`"),
                            "declare it with `effect Name { fn op(...) -> T }`",
                        );
                        self.infer_expr(handler);
                    }
                }
                // A closure written inside the block has this effect
                // discharged *here*, because here is where the handler is.
                // If the value then leaves the block, the discharge leaves
                // with it and the call happens somewhere the handler is not
                // installed — which is how a well-typed program reached
                // [E-HANDLER-MISSING]'s panic.
                self.with_floors.push((ename, self.scopes.len()));
                self.check_block(body);
                self.with_floors.pop();
                Type::Unit
            }
            Stmt::For {
                var, iter, body, ..
            } => {
                let iter_ty = self.infer_expr(iter);
                let elem = match self.uni.shallow_resolve(&iter_ty) {
                    Type::Range => Type::I64,
                    Type::List(inner) => *inner,
                    Type::Str => Type::Str,
                    Type::Error => Type::Error,
                    other => {
                        let shown = TypeDisplay {
                            ty: &other,
                            defs: &self.defs,
                        }
                        .to_string();
                        self.error_help(
                            iter.span(),
                            format!("cannot iterate over `{shown}`"),
                            "iterate over a list (`[T]`), a range (`0..n`), or a string — a map iterates through `keys()`, `values()` or `entries()`",
                        )
                    }
                };
                self.scopes.push(HashMap::new());
                self.bind(var, elem, false);
                self.loop_depth += 1;
                self.check_block(body);
                self.loop_depth -= 1;
                self.scopes.pop();
                Type::Unit
            }
        }
    }

    fn bind(&mut self, name: &Ident, ty: Type, mutable: bool) {
        let decl_loop_depth = self.loop_depth;
        self.scopes.last_mut().unwrap().insert(
            name.name.clone(),
            Binding {
                ty,
                mutable,
                moved: false,
                decl_loop_depth,
            },
        );
    }

    fn lookup_mut(&mut self, name: &str) -> Option<&mut Binding> {
        for scope in self.scopes.iter_mut().rev() {
            if scope.contains_key(name) {
                return scope.get_mut(name);
            }
        }
        None
    }

    /// Marks `name` as moved (`own` transfer). Errors for loop-crossing moves.
    /// Which bindings are currently moved, so a branch can be checked from
    /// the state before it rather than from what a sibling branch did.
    ///
    /// Branches are mutually exclusive: only one of them runs. Checking them
    /// in sequence let the first one's move leak into the second, so
    /// `if c { eat(s) } else { eat(s) }` — where `s` is moved exactly once
    /// however it goes — was "use of moved value" ([T-USE-AFTER-MOVE]).
    fn moved_snapshot(&self) -> Vec<Vec<(String, bool)>> {
        self.scopes
            .iter()
            .map(|sc| sc.iter().map(|(k, b)| (k.clone(), b.moved)).collect())
            .collect()
    }

    fn restore_moved(&mut self, snap: &[Vec<(String, bool)>]) {
        for (sc, saved) in self.scopes.iter_mut().zip(snap) {
            for (name, moved) in saved {
                if let Some(b) = sc.get_mut(name) {
                    b.moved = *moved;
                }
            }
        }
    }

    /// After the branches: a value is gone if any path could have taken it.
    fn merge_moved(&mut self, branches: &[Vec<Vec<(String, bool)>>]) {
        for snap in branches {
            for (sc, saved) in self.scopes.iter_mut().zip(snap) {
                for (name, moved) in saved {
                    if *moved {
                        if let Some(b) = sc.get_mut(name) {
                            b.moved = true;
                        }
                    }
                }
            }
        }
    }

    fn mark_moved(&mut self, id: &Ident) {
        let loop_depth = self.loop_depth;
        // A closure body runs once per call, so moving something captured
        // from outside it is the same mistake as moving inside a loop: the
        // second call finds the value gone. It was refused in the loop and
        // accepted in the closure, which are the same program written two
        // ways ([T-MOVE-LOOP]).
        let captured = self.closure_floors.last().is_some_and(|floor| {
            self.scopes
                .iter()
                .rposition(|sc| sc.contains_key(&id.name))
                .is_some_and(|at| at < *floor)
        });
        if captured {
            self.error_help(
                id.span,
                format!(
                    "cannot move `{}` here — it was captured, and would be gone the next time the closure is called",
                    id.name
                ),
                "move a copy instead, or take the value as a parameter of the closure",
            );
            return;
        }
        let mut err: Option<Diagnostic> = None;
        if let Some(b) = self.lookup_mut(&id.name) {
            if b.decl_loop_depth < loop_depth {
                err = Some(
                    Diagnostic::error(
                        id.span,
                        format!(
                            "cannot move `{}` here — it would be gone in the next loop iteration",
                            id.name
                        ),
                    )
                    .with_help("move a copy instead, or restructure so the move happens outside the loop"),
                );
            } else {
                b.moved = true;
            }
        }
        if let Some(d) = err {
            self.diags.push(d);
        }
    }

    fn is_copy_type(&self, ty: &Type) -> bool {
        matches!(
            self.uni.resolve(ty),
            // A reference is one word — a slot number and a generation — and
            // [T-REF-GENERATIONAL] says it copies. It moved instead, so
            // `own r: ref N` consumed the caller's, which is neither what the
            // clause says nor useful: the point of a reference is that several
            // places hold it.
            Type::I64 | Type::F64 | Type::Bool | Type::Unit | Type::Range | Type::Ref(_)
        )
    }

    /// Whether a value could have been built before the region began.
    ///
    /// A literal is built here. A name is judged by where it was declared —
    /// the same floor the escape rule uses. Anything else is treated as
    /// having come from outside, which is the safe answer.
    fn from_outside_region(&self, e: &Expr) -> bool {
        let Some(&floor) = self.region_floors.last() else {
            return false;
        };
        let mut root = e;
        loop {
            match root {
                Expr::Closure { .. }
                | Expr::StructLit { .. }
                | Expr::List(..)
                | Expr::Tuple(..)
                | Expr::Map(..) => return false,
                Expr::Field { recv, .. } | Expr::Index { recv, .. } => root = recv,
                Expr::Ident(id) => {
                    return self
                        .scopes
                        .iter()
                        .rposition(|sc| sc.contains_key(&id.name))
                        .is_some_and(|at| at < floor);
                }
                _ => return true,
            }
        }
    }

    /// Whether a type can reach a `ref` anywhere inside it.
    ///
    /// A region's rule was checked against the *syntax written between the
    /// braces*, so hiding the reference one field deep defeated it: passing a
    /// `Holder { s: ref Sink }` into the region names no `ref` at all, and the
    /// callee reads the field and writes through it ([T-REGION-REF]).
    fn carries(&self, t: &Type, want_fn: bool, seen: &mut Vec<usize>) -> bool {
        match self.uni.resolve(t) {
            Type::Ref(inner) => !want_fn || self.carries(&inner, want_fn, seen),
            Type::Fn { params, ret, .. } => {
                want_fn
                    || params.iter().any(|p| self.carries(p, want_fn, seen))
                    || self.carries(&ret, want_fn, seen)
            }
            Type::List(e) => self.carries(&e, want_fn, seen),
            Type::Map(k, v) => {
                self.carries(&k, want_fn, seen) || self.carries(&v, want_fn, seen)
            }
            Type::Tuple(items) => items.iter().any(|i| self.carries(i, want_fn, seen)),
            Type::Fallible(a, b) => {
                self.carries(&a, want_fn, seen) || self.carries(&b, want_fn, seen)
            }
            Type::Named { id, args } => {
                if args.iter().any(|a| self.carries(a, want_fn, seen)) {
                    return true;
                }
                // A recursive type is reached twice; the second visit answers
                // for the first rather than recursing forever.
                if seen.contains(&id) {
                    return false;
                }
                seen.push(id);
                let inner: Vec<Type> = match &self.defs.get(id).kind {
                    TypeDefKind::Struct { fields } => {
                        fields.iter().map(|(_, t)| t.clone()).collect()
                    }
                    TypeDefKind::Sum { variants } => variants
                        .iter()
                        .flat_map(|v| v.fields.iter().map(|(_, t)| t.clone()))
                        .collect(),
                };
                let found = inner.iter().any(|t| self.carries(t, want_fn, seen));
                seen.pop();
                found
            }
            _ => false,
        }
    }

    fn carries_ref(&self, t: &Type) -> bool {
        self.carries(t, false, &mut Vec::new())
    }

    fn carries_fn(&self, t: &Type) -> bool {
        self.carries(t, true, &mut Vec::new())
    }

    /// Function lookup in the current module's namespace.
    fn lookup_fn(&self, name: &str) -> Option<FnInfo> {
        self.fns
            .get(&format!("{}::{}", self.current_module, name))
            .cloned()
    }

    /// A named function's inferred effect row, if it has one.
    fn row_of_fn(&self, name: &str) -> Option<std::collections::BTreeSet<String>> {
        let key = format!("{}::{}", self.current_module, name);
        // A C function has no body to infer from, and what it does is
        // unknowable: `[T-EXTERN]` says calling one performs `io` and
        // `[T-UNSAFE-FFI]` puts it in Tier 3. Without a row of its own it
        // became a row-less value the moment it was stored — pushing `sqrt`
        // into a `[fn(f64) -> f64 / {io}]` inside an `unsafe` block was
        // accepted, and calling it back out ran libc from a function
        // declaring nothing.
        if self.extern_names.contains(&key) {
            return Some(
                ["io", "unsafe"]
                    .iter()
                    .map(|s| s.to_string())
                    .collect(),
            );
        }
        self.fn_rows.get(&key).cloned()
    }

    /// A function value's row must fit the row of the type it is going into.
    ///
    /// This is a *subset* rule, not equality: a function that performs nothing
    /// fits `fn(..) / {io}`, and one that prints does not fit `fn(..)`. It runs
    /// wherever a value meets a written type — a parameter, a return, a `let`
    /// annotation, a struct field, a list element, a map value — because the
    /// row is part of the type. It used to run only for a parameter written
    /// inline as `fn(...)`, and every other route dropped the row: through a
    /// return type, through a `type Stage = fn(..)` alias, through a struct
    /// field, through a `[fn(..)]`. Each of those was a way to hand a printing
    /// function to a `/ {}` row, to `par_map`, and to a package declaring
    /// `capabilities = []`.
    fn check_row_fits(&mut self, got: &Type, want: &Type, span: Span) {
        let (g, w) = (self.uni.resolve(got), self.uni.resolve(want));
        // Structural, because a function value reaches its destination inside
        // things: `[fn(..)]`, `Map[str, fn(..)]`, `(fn(..), str)`,
        // `Option[fn(..)]`, a generic argument. Checking only the outermost
        // type left every one of those as a way through.
        match (&g, &w) {
            (Type::List(a), Type::List(b)) => return self.check_row_fits(a, b, span),
            (Type::Map(ak, av), Type::Map(bk, bv)) => {
                self.check_row_fits(ak, bk, span);
                return self.check_row_fits(av, bv, span);
            }
            (Type::Tuple(a), Type::Tuple(b)) if a.len() == b.len() => {
                for (x, y) in a.iter().zip(b) {
                    self.check_row_fits(x, y, span);
                }
                return;
            }
            (Type::Fallible(ao, ae), Type::Fallible(bo, be)) => {
                self.check_row_fits(ao, bo, span);
                return self.check_row_fits(ae, be, span);
            }
            (Type::Named { id: i1, args: a }, Type::Named { id: i2, args: b })
                if i1 == i2 && a.len() == b.len() =>
            {
                for (x, y) in a.iter().zip(b) {
                    self.check_row_fits(x, y, span);
                }
                return;
            }
            (Type::Fn { ret: ar, .. }, Type::Fn { ret: br, .. }) => {
                // A function's result may itself be a function.
                self.check_row_fits(ar, br, span);
            }
            _ => {}
        }
        let (Type::Fn { row: Some(have), .. }, Type::Fn { row: Some(allowed), .. }) = (&g, &w)
        else {
            return;
        };
        let over: Vec<&String> = have.difference(allowed).collect();
        if over.is_empty() {
            return;
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
        self.error_help(
            span,
            format!("this expects a function that {shown}, but this one performs {list}"),
            "widen the written row — `fn(..) -> .. / {io}` — or use a function that does not \
perform it",
        );
    }

    /// Tier 3: a declared C function may only be reached inside `unsafe`.
    ///
    /// Calling one and merely naming one are the same permission — holding the
    /// function *is* the ability to call it — so both go through here.
    /// Reporting at the use rather than at the declaration is deliberate: the
    /// declaration is fine, and the use is where the reader has to be told
    /// what they are taking on.
    fn require_unsafe(&mut self, name: &str, span: Span) {
        if self.unsafe_depth > 0 {
            return;
        }
        let key = format!("{}::{}", self.current_module, name);
        if self.extern_names.contains(&key) {
            self.error_help(
                span,
                format!("`{name}` is a C function, so it can only be used inside `unsafe`"),
                "wrap the use in `unsafe { ... }`; the package must then declare the \
`unsafe` capability, which is what makes it visible to everyone who depends on you",
            );
        }
    }

    /// Rejects a task body's reference to a binding that will not exist when
    /// the task runs.
    ///
    /// Tasks are collected during the `parallel` body and executed after it,
    /// so anything the body declared — including a loop variable — is already
    /// gone. The interpreter used to panic with "unknown name" at run time;
    /// this makes it a compile error instead ([T-TASK-CAPTURE]).
    fn reject_dead_capture(&mut self, id: &Ident) {
        let Some((floor, task_base)) = self.task_capture else {
            return;
        };
        let Some(at) = self
            .scopes
            .iter()
            .rposition(|scope| scope.contains_key(&id.name))
        else {
            return;
        };
        if at >= floor && at < task_base {
            self.error_help(
                id.span,
                format!(
                    "task body uses `{}`, which is declared inside the `parallel` block",
                    id.name
                ),
                "tasks run after the block finishes, so that binding is gone. \
Declaring it before `parallel` compiles but reads the value it has *then*, \
which for a loop counter is its final one — give each task the value it \
needs before the loop advances, or have the task compute it from something \
that does not change",
            );
        }
    }

    /// Rejects writing to a binding that was declared outside the region.
    ///
    /// This is the escape rule. A region promises that nothing allocated
    /// inside it is reachable afterwards, and writing into an outer binding —
    /// by assignment or by a mutating method — is precisely how something
    /// would get out. `outer.push(made_here)` is the case that a rule about
    /// assignment alone would miss.
    fn reject_region_escape(&mut self, id: &Ident, verb: &str) {
        let Some(&floor) = self.region_floors.last() else {
            return;
        };
        let declared_at = self
            .scopes
            .iter()
            .rposition(|sc| sc.contains_key(&id.name));
        if declared_at.is_some_and(|d| d < floor) {
            self.error_help(
                id.span,
                format!("cannot {verb} `{}` from inside a region", id.name),
                "a region promises that nothing allocated inside it outlives the closing \
brace, and writing to something declared outside is how a value escapes — move the \
declaration into the region, or compute a number and yield that",
            );
        }
    }

    fn lookup(&self, name: &str) -> Option<Binding> {
        for scope in self.scopes.iter().rev() {
            if let Some(b) = scope.get(name) {
                return Some(b.clone());
            }
        }
        None
    }

    /// Assignment target must be a place rooted in a mutable binding.
    /// Whether a place path passes through a `ref` at any depth.
    ///
    /// Writing through a reference does not modify what holds it, so nothing
    /// on the way needs to be mutable. Only the *root* was tested, so a node
    /// in a list or in a struct field — the ordinary Tier 2 arena — demanded
    /// `var` on the container, named the container in the error, and could
    /// not be expressed at all for a `[ref N]` parameter ([T-REF-WRITE]).
    fn path_crosses_ref(&mut self, target: &Expr) -> bool {
        let mark = self.diags.len();
        let mut cur = target;
        let mut found = false;
        loop {
            let (Expr::Field { recv, .. } | Expr::Index { recv, .. }) = cur else {
                break;
            };
            let t = self.infer_expr(recv);
            if matches!(self.uni.resolve(&t), Type::Ref(_)) {
                found = true;
                break;
            }
            cur = recv;
        }
        // Re-typing a prefix must not report the same problem twice.
        self.diags.truncate(mark);
        found
    }

    /// Refuses carrying a handled effect out of the block that handles it.
    ///
    /// The value is a function whose row still names an effect an enclosing
    /// `with` discharges, and the place it is going was declared before that
    /// `with` began — so it outlives the handler. Calling it afterwards
    /// reaches the runtime's missing-handler panic, which
    /// `[E-HANDLER-MISSING]` says well-typed programs never do.
    fn reject_handler_escape(&mut self, target: &Expr, value_ty: &Type) {
        if self.with_floors.is_empty() {
            return;
        }
        let mut root = target;
        while let Expr::Field { recv, .. } | Expr::Index { recv, .. } = root {
            root = recv;
        }
        let Expr::Ident(id) = root else { return };
        let Some(declared_at) = self
            .scopes
            .iter()
            .rposition(|sc| sc.contains_key(&id.name))
        else {
            return;
        };
        let row = self.row_carried_by(value_ty);
        let floors = self.with_floors.clone();
        for (effect, floor) in floors {
            if declared_at < floor && row.contains(&effect) {
                self.error_help(
                    target.span(),
                    format!(
                        "cannot store a function that performs `{effect}` in `{}` — it was declared outside the `with` block",
                        id.name
                    ),
                    "the handler is installed only for this block, so calling the value \
afterwards has nowhere to send the effect — call it here, or return what it computed",
                );
                return;
            }
        }
    }

    /// Every effect named by a function type anywhere inside `t`.
    fn row_carried_by(&self, t: &Type) -> std::collections::BTreeSet<String> {
        let mut out = std::collections::BTreeSet::new();
        let mut stack = vec![self.uni.resolve(t)];
        let mut seen = 0;
        while let Some(cur) = stack.pop() {
            seen += 1;
            // A recursive type would otherwise walk forever; the row is
            // written on the function type itself, so depth buys nothing.
            if seen > 64 {
                break;
            }
            match cur {
                Type::Fn { params, ret, row } => {
                    if let Some(r) = row {
                        out.extend(r.iter().cloned());
                    }
                    stack.extend(params.iter().map(|p| self.uni.resolve(p)));
                    stack.push(self.uni.resolve(&ret));
                }
                Type::List(e) => stack.push(self.uni.resolve(&e)),
                Type::Ref(e) => stack.push(self.uni.resolve(&e)),
                Type::Map(k, v) => {
                    stack.push(self.uni.resolve(&k));
                    stack.push(self.uni.resolve(&v));
                }
                Type::Tuple(items) => {
                    stack.extend(items.iter().map(|i| self.uni.resolve(i)))
                }
                Type::Fallible(a, b) => {
                    stack.push(self.uni.resolve(&a));
                    stack.push(self.uni.resolve(&b));
                }
                _ => {}
            }
        }
        out
    }

    fn check_assign_target(&mut self, target: &Expr) {
        let mut root = target;
        while let Expr::Field { recv, .. } | Expr::Index { recv, .. } = root {
            root = recv;
        }
        // Writing *through* a `ref` does not mutate the reference — it mutates
        // what the reference points at, which is shared state by construction.
        // Requiring `var` for that asked for the wrong thing and, since the
        // write then panicked at run time, its suggested fix turned a compile
        // error into a crash.
        if let Expr::Ident(id) = root {
            self.reject_region_escape(id, "assign to");
        }
        let through_ref = self.path_crosses_ref(target);
        match root {
            Expr::Ident(id) => match self.lookup(&id.name) {
                Some(b) if !b.mutable && !through_ref => {
                    self.error_help(
                        id.span,
                        format!("cannot assign to `{}` — it is immutable", id.name),
                        format!("declare it with `var {}` instead of `let`", id.name),
                    );
                }
                Some(_) => {}
                None if self
                    .consts
                    .contains_key(&format!("{}::{}", self.current_module, id.name)) =>
                {
                    self.error_help(
                        id.span,
                        format!("cannot assign to `{}` — it is a constant", id.name),
                        "constants have one value for the whole program; use a `var` if it needs to change",
                    );
                }
                None => {
                    self.error(id.span, format!("unknown variable `{}`", id.name));
                }
            },
            other => {
                self.error(other.span(), "invalid assignment target");
            }
        }
        if let Expr::Ident(id) = root {
            self.reject_capture_write(id, "assign to", "");
        }
    }

    // ------------------------------------------------------------------
    // Expressions
    // ------------------------------------------------------------------

    fn check_expr(&mut self, e: &Expr, expected: &Type) {
        let tail_fn = self.tail_fn.take();
        let want = self.uni.shallow_resolve(expected);
        // Int literal adapts to f64. Recorded, not just allowed: the engines
        // have no types of their own and would otherwise keep the value an
        // integer for the whole run ([T-INT-TO-FLOAT]).
        if let (Expr::Int(..), Type::F64) = (e, &want) {
            self.float_literals.insert(e.span().start);
            return;
        }
        if let (Expr::Unary { op: UnaryOp::Neg, expr, .. }, Type::F64) = (e, &want) {
            if matches!(**expr, Expr::Int(..)) {
                self.float_literals.insert(expr.span().start);
                return;
            }
        }
        // [T-INT-TO-FLOAT-CARRIED]: the expected type reaches the literal
        // through the shape that carries it. Without this the rule held only
        // where the literal was the whole expression, so `let x: f64 = 3`
        // worked and `let xs: [f64] = [1, 2]` did not — and neither did a
        // tuple, `Some`, `Ok`, an `if`, or a `match`, each of which reported
        // a type error about a program the clause says is well typed.
        match (e, &want) {
            (Expr::List(items, span), Type::List(elem)) => {
                let elem = (**elem).clone();
                for it in items {
                    self.check_expr(it, &elem);
                }
                // Recorded here too. This path returns before `infer_expr`
                // ever sees the literal, so an empty one pushed a type into
                // was known to the checker and not handed to the backend:
                // `log = []` after `var log: [str] = []`, `{"a": []}`,
                // `([], "x")` and `if c { [] } else { … }` all checked and ran
                // and could not be built ([T-EMPTY-LIT]).
                if items.is_empty() {
                    self.empty_lits.push((span.start, elem));
                }
                return;
            }
            (Expr::Tuple(items, _), Type::Tuple(ts)) if items.len() == ts.len() => {
                let ts = ts.clone();
                for (it, t) in items.iter().zip(&ts) {
                    self.check_expr(it, t);
                }
                return;
            }
            (Expr::Map(entries, span), Type::Map(k, v)) => {
                let (k, v) = ((**k).clone(), (**v).clone());
                for (ke, ve) in entries {
                    self.check_expr(ke, &k);
                    self.check_expr(ve, &v);
                }
                if entries.is_empty() {
                    self.empty_lits
                        .push((span.start, Type::Map(Box::new(k), Box::new(v))));
                }
                return;
            }
            // `Some(3)` against `Option[f64]`, `Ok(3)` and `Err(3)` against
            // `f64 ! E`. A user-declared variant already carried its
            // declared field type down; the three built-in ones did not.
            (Expr::Call { callee, args, .. }, _) if args.len() == 1 => {
                if let Expr::Ident(id) = &**callee {
                    let inner = match (id.name.as_str(), &want) {
                        ("Some", Type::Named { id: nid, args: targs })
                            if self.defs.get(*nid).name == "Option" && targs.len() == 1 =>
                        {
                            Some(targs[0].clone())
                        }
                        ("Ok", Type::Fallible(ok, _)) => Some((**ok).clone()),
                        ("Err", Type::Fallible(_, err)) => Some((**err).clone()),
                        _ => None,
                    };
                    if let Some(inner) = inner {
                        self.check_expr(&args[0], &inner);
                        return;
                    }
                }
            }
            _ => {}
        }
        // The same, through the shapes whose value is another expression.
        match e {
            Expr::Block(b) => {
                let t = self.check_block_with(b, Some(&want));
                self.unify_or_report(e.span(), &t, &want);
                return;
            }
            Expr::Match {
                scrutinee, arms, ..
            } => {
                let t = self.infer_match_with(scrutinee, arms, e.span(), Some(&want));
                self.unify_or_report(e.span(), &t, &want);
                return;
            }
            Expr::If {
                cond,
                then_block,
                else_branch: Some(eb),
                ..
            } => {
                self.check_expr(cond, &Type::Bool);
                self.check_block_with(then_block, Some(&want));
                match &**eb {
                    ElseBranch::Else(b) => {
                        self.check_block_with(b, Some(&want));
                    }
                    ElseBranch::ElseIf(inner) => self.check_expr(inner, &want),
                }
                return;
            }
            _ => {}
        }
        // Closures check against an expected fn type for param inference.
        if let (Expr::Closure { .. }, Type::Fn { row, .. }) = (e, &want) {
            // The closure's own row must fit the row written on the type it
            // is going into ([T-FN-TYPE-ROW]). Only the *shape* was compared,
            // so an effectful closure went into a `fn(i64) -> i64` — a
            // written empty row — on every route but a direct call argument:
            // a struct field, a reassignment, a `push`, a `set`, an
            // index-assign, `Some(...)`, a tuple. A named function was
            // checked everywhere; only closures leaked.
            if let (Some(want_row), Some(got_row)) =
                (row.clone(), self.closure_rows.get(&e.span().start).cloned())
            {
                let extra: Vec<&String> = got_row.difference(&want_row).collect();
                if let Some(first) = extra.first() {
                    let list = extra
                        .iter()
                        .map(|x| format!("`{x}`"))
                        .collect::<Vec<_>>()
                        .join(", ");
                    let _ = first;
                    self.error_help(
                        e.span(),
                        format!("this closure performs {list}, which the expected type does not allow"),
                        "widen the type's effect row, or move the effect out of the closure",
                    );
                }
            }
            let got = self.infer_closure(e, Some(&want));
            self.unify_or_report(e.span(), &got, &want);
            return;
        }
        let got = self.infer_expr(e);
        self.check_row_fits(&got, &want, e.span());
        if let Some(name) = tail_fn {
            if !self.uni.unify(&got, &want) {
                let w = self.show(&want);
                let g = self.show(&got);
                self.error_help(
                    e.span(),
                    format!("function `{name}` must return `{w}` but its body produces `{g}`"),
                    "the last expression of the body is the return value",
                );
            }
            return;
        }
        self.unify_or_report(e.span(), &got, &want);
    }

    fn unify_or_report(&mut self, span: Span, got: &Type, want: &Type) {
        if !self.uni.unify(got, want) {
            let got_r = self.uni.shallow_resolve(got);
            let g = self.show(got);
            let w = self.show(want);
            if matches!(got_r, Type::Fallible(..)) && !matches!(self.uni.shallow_resolve(want), Type::Fallible(..)) {
                self.error_help(
                    span,
                    format!("this expression can fail (`{g}`) but `{w}` is required"),
                    "use `try` to propagate the error or `match` to handle it",
                );
            } else if let (
                Type::Fn { ret: got_ret, .. },
                Type::Fn { ret: want_ret, .. },
            ) = (&got_r, &self.uni.shallow_resolve(want))
            {
                // A fallible function passed where a plain one is wanted. The
                // types alone do not say what to do about it, and the answer
                // — handle the failure at each element — is not obvious.
                if matches!(self.uni.shallow_resolve(got_ret), Type::Fallible(..))
                    && !matches!(self.uni.shallow_resolve(want_ret), Type::Fallible(..))
                {
                    self.error_help(
                        span,
                        format!("type mismatch: expected `{w}`, found `{g}`"),
                        "this function can fail, and there is nowhere for the error to go once per element — wrap it in a closure that handles the failure, as in `|x| f(x) ?? fallback`",
                    );
                } else {
                    self.error(span, format!("type mismatch: expected `{w}`, found `{g}`"));
                }
            } else if matches!(
                (&got_r, self.uni.shallow_resolve(want)),
                (Type::I64, Type::F64) | (Type::F64, Type::I64)
            ) {
                self.error_help(
                    span,
                    format!("type mismatch: expected `{w}`, found `{g}`"),
                    "the two number types do not mix on their own — convert with `.as_f64()`, or use a literal, which does adopt the other side",
                );
            } else {
                self.error(span, format!("type mismatch: expected `{w}`, found `{g}`"));
            }
        }
    }

    /// Types one expression, and enforces the one rule that is about the
    /// *type* a region may not touch.
    ///
    /// A `ref` reaches storage the region cannot see. It needs no `inout`, so
    /// [T-REGION-INOUT] never fires, and it can be copied to a name declared
    /// inside, so the declared-outside rule never fires either. Two
    /// independent evaluations carried a value out of a region through one.
    /// Since a region's whole guarantee is that nothing inside outlives the
    /// brace, and a reference is the one thing that can carry something out
    /// without naming anything outside, a region and a reference do not mix
    /// ([T-REGION-REF]).
    fn infer_expr(&mut self, e: &Expr) -> Type {
        let t = self.infer_expr_inner(e);
        // A closure written *here* can only capture what a region already
        // allows, and so can one bound to a name declared here. One that
        // arrived from *outside* carries its captures, and running it inside
        // the region is how what the region allocated gets out — whether it
        // arrives by name, in a struct field, a list, a tuple, a map or an
        // `Option` ([T-REGION-FOREIGN]).
        if self.region_depth > 0 && self.from_outside_region(e) && self.carries_fn(&t) {
            self.error_help(
                e.span(),
                "a function value cannot be used inside a region",
                "a function written outside the region carries its captures, so what the region allocates could be kept — write the closure inside the region, or move the call out",
            );
            return Type::Error;
        }
        if self.region_depth > 0 && self.carries_ref(&t) {
            self.error_help(
                e.span(),
                "a `ref` cannot be used inside a region",
                "a reference reaches storage outside the region, so what the region \
allocates could outlive it — move the work out of the region, or yield a number",
            );
            return Type::Error;
        }
        t
    }

    fn infer_expr_inner(&mut self, e: &Expr) -> Type {
        match e {
            Expr::Int(..) => Type::I64,
            Expr::Float(..) => Type::F64,
            Expr::Bool(..) => Type::Bool,
            Expr::Str(pieces, _) => {
                for p in pieces {
                    if let StrPiece::Expr(inner) = p {
                        // Any type can be interpolated; just check it's valid.
                        let t = self.infer_expr(inner);
                        let resolved = self.uni.shallow_resolve(&t);
                        if let Type::Fallible(..) = resolved {
                            self.error_help(
                                inner.span(),
                                "cannot interpolate a fallible expression",
                                "handle the error first with `try` or `match`",
                            );
                        }
                    }
                }
                Type::Str
            }
            Expr::Ident(id) => self.infer_ident(id),
            Expr::List(items, span) => {
                let elem = self.uni.fresh();
                for item in items {
                    self.check_expr(item, &elem);
                }
                if items.is_empty() {
                    self.empty_lits.push((span.start, elem.clone()));
                }
                Type::List(Box::new(elem))
            }
            Expr::Map(entries, span) => {
                let k = self.uni.fresh();
                let v = self.uni.fresh();
                for (ke, ve) in entries {
                    self.check_expr(ke, &k);
                    self.check_expr(ve, &v);
                }
                if entries.is_empty() {
                    self.empty_lits
                        .push((span.start, Type::Map(Box::new(k.clone()), Box::new(v.clone()))));
                }
                Type::Map(Box::new(k), Box::new(v))
            }
            Expr::Tuple(items, _) => {
                if items.is_empty() {
                    Type::Unit
                } else {
                    Type::Tuple(items.iter().map(|i| self.infer_expr(i)).collect())
                }
            }
            Expr::StructLit {
                name,
                base,
                fields,
                span,
            } => self.infer_struct_lit(name, base.as_deref(), fields, *span),
            Expr::Call { callee, args, span } => self.infer_call(callee, args, *span),
            Expr::MethodCall {
                recv,
                name,
                args,
                span,
            } => self.infer_method_call(recv, name, args, *span),
            Expr::Field { recv, name, span } => self.infer_field(recv, name, *span),
            Expr::Index { recv, index, span } => {
                let recv_ty = self.infer_expr(recv);
                // What may be indexed is decided before the index is checked
                // against `i64`. Otherwise `m["a"]` on a `Map[str, i64]`
                // reported `expected i64, found str` *first* — naming the key
                // type as the mistake — and only then said maps are not
                // indexable at all.
                if let Type::Map(k, v) = self.uni.shallow_resolve(&recv_ty) {
                    let kt = self.show(&k);
                    let vt = self.show(&v);
                    return self.error_help(
                        *span,
                        format!("cannot index into `Map[{kt}, {vt}]`"),
                        "a map is read with `m.get(k)`, which answers `Option` because the key may be absent",
                    );
                }
                self.check_expr(index, &Type::I64);
                match self.uni.shallow_resolve(&recv_ty) {
                    Type::List(inner) => *inner,
                    Type::Str => self.error_help(
                        *span,
                        "strings cannot be indexed directly",
                        "use `.chars()` to get a list of characters",
                    ),
                    Type::Error => Type::Error,
                    other => {
                        let shown = TypeDisplay {
                            ty: &other,
                            defs: &self.defs,
                        }
                        .to_string();
                        self.error(*span, format!("cannot index into `{shown}`"))
                    }
                }
            }
            Expr::Unary { op, expr, span } => match op {
                UnaryOp::Neg => {
                    let t = self.infer_expr(expr);
                    let r = self.uni.shallow_resolve(&t);
                    if matches!(r, Type::I64 | Type::F64 | Type::Var(_) | Type::Error) {
                        if let Type::Var(_) = r {
                            self.uni.unify(&t, &Type::I64);
                            return Type::I64;
                        }
                        r
                    } else {
                        let shown = self.show(&t);
                        self.error(*span, format!("cannot negate `{shown}`"))
                    }
                }
                UnaryOp::Not => {
                    self.check_expr(expr, &Type::Bool);
                    Type::Bool
                }
            },
            Expr::Binary { op, lhs, rhs, span } => self.infer_binary(*op, lhs, rhs, *span),
            Expr::Range { start, end, .. } => {
                self.check_expr(start, &Type::I64);
                self.check_expr(end, &Type::I64);
                Type::Range
            }
            Expr::If {
                cond,
                then_block,
                else_branch,
                ..
            } => {
                self.check_expr(cond, &Type::Bool);
                let before = self.moved_snapshot();
                let then_ty = self.check_block(then_block);
                let after_then = self.moved_snapshot();
                match else_branch {
                    Some(eb) => {
                        self.restore_moved(&before);
                        let else_ty = match &**eb {
                            ElseBranch::Else(b) => self.check_block(b),
                            ElseBranch::ElseIf(e) => self.infer_expr(e),
                        };
                        let after_else = self.moved_snapshot();
                        self.merge_moved(&[after_then, after_else]);
                        if self.uni.unify(&then_ty, &else_ty) {
                            then_ty
                        } else {
                            // Branches disagree — it's a statement-style if.
                            Type::Unit
                        }
                    }
                    None => Type::Unit,
                }
            }
            Expr::Match {
                scrutinee, arms, ..
            } => self.infer_match(scrutinee, arms, e.span()),
            Expr::Block(b) => self.check_block(b),
            // Tier 3. The block itself is ordinary; what it does is permit
            // what the safe tiers refuse — today, calling a declared C
            // function. The depth is a counter rather than a flag so a nested
            // `unsafe` does not turn the outer one off on the way out.
            Expr::Unsafe(b) => {
                self.unsafe_depth += 1;
                let t = self.check_block(b);
                self.unsafe_depth -= 1;
                t
            }
            // Tier 1. The rule is what a region *is*: nothing allocated inside
            // may be reachable after the brace. That is what would make bulk
            // deallocation sound, and it is a guarantee on its own — the peak
            // memory of the block is the block.
            //
            // Three restrictions give it. No writing to a binding declared
            // outside (assignment or a mutating method), because that is how a
            // value gets out. No `return` through the brace. And the region
            // yields a scalar or nothing, since a scalar is not an allocation.
            Expr::Region(b) => {
                self.region_depth += 1;
                let floor = self.scopes.len();
                self.region_floors.push(floor);
                let t = self.check_block(b);
                self.region_floors.pop();
                self.region_depth -= 1;
                let r = self.uni.resolve(&t);
                if !matches!(
                    r,
                    Type::Unit | Type::I64 | Type::F64 | Type::Bool | Type::Var(_) | Type::Error
                ) {
                    let shown = self.show(&t);
                    self.error_help(
                        b.span,
                        format!("a region cannot produce a `{shown}`"),
                        "everything allocated inside a region dies at the closing brace, so \
only a number or a `bool` can come out — compute what you need and yield that",
                    );
                }
                t
            }
            Expr::Try { expr, span } => self.infer_try(expr, *span),
            Expr::Closure { .. } => self.infer_closure(e, None),
        }
    }

    fn infer_ident(&mut self, id: &Ident) -> Type {
        if let Some(b) = self.lookup(&id.name) {
            self.reject_dead_capture(id);
            if b.moved {
                return self.error_help(
                    id.span,
                    format!("use of moved value `{}`", id.name),
                    "ownership was transferred with `own` — the value is gone here",
                );
            }
            return b.ty;
        }
        // Nullary variant used as a value (e.g. `None`).
        if let Some(cands) = self.variants.get(&id.name).cloned() {
            if cands.len() == 1 {
                let (def_id, vi) = cands[0];
                return self.instantiate_variant(def_id, vi, &[], id.span);
            } else if cands.len() > 1 {
                return self.error_help(
                    id.span,
                    format!("`{}` is ambiguous — it names a variant of multiple types", id.name),
                    "qualify it as `TypeName.Variant`",
                );
            }
        }
        // A module-level constant. Looked up before functions, because a
        // constant and a function cannot share a name and the constant is the
        // more specific reading of a bare identifier.
        if let Some((ty, _)) = self
            .consts
            .get(&format!("{}::{}", self.current_module, id.name))
            .cloned()
        {
            return ty;
        }
        // Function referenced as a value. A fallible function's `! E` is
        // part of what it answers with, so it has to travel with the value —
        // dropping it let `xs.filter(risky)` typecheck against
        // `fn(T) -> bool`, then filter on `Ok(true)`, which is not `true`, and
        // quietly return the empty list.
        if let Some(info) = self.lookup_fn(&id.name) {
            self.require_unsafe(&id.name, id.span);
            let ret_of = |ret: &Type, err: &Option<Type>| match err {
                Some(e) => Type::Fallible(Box::new(ret.clone()), Box::new(e.clone())),
                None => ret.clone(),
            };
            // The function's *own* row travels with it. Without this a
            // named function became a row-less value the moment it stopped
            // being called directly, and could then be stored anywhere.
            let own_row = self.row_of_fn(&id.name);
            if info.generics > 0 {
                let fresh: Vec<Type> = (0..info.generics).map(|_| self.uni.fresh()).collect();
                let ret = ret_of(&info.ret, &info.err);
                return Type::Fn {
                    params: info
                        .params
                        .iter()
                        .map(|p| substitute_generics(p, &fresh))
                        .collect(),
                    ret: Box::new(substitute_generics(&ret, &fresh)),
                    row: own_row,
                };
            }
            return Type::Fn {
                params: info.params.clone(),
                ret: Box::new(ret_of(&info.ret, &info.err)),
                row: own_row,
            };
        }
        self.error_help(
            id.span,
            format!("unknown name `{}`", id.name),
            "check the spelling, or declare it with `let` first",
        )
    }

    fn instantiate_variant(
        &mut self,
        def_id: TypeDefId,
        vi: usize,
        arg_exprs: &[Expr],
        span: Span,
    ) -> Type {
        let def = self.defs.get(def_id);
        let generics = def.generics.len();
        let TypeDefKind::Sum { variants } = &def.kind else {
            return self.error(span, "internal: variant of non-sum type");
        };
        let variant = variants[vi].clone();
        let vname = variant.name.clone();
        let fresh: Vec<Type> = (0..generics).map(|_| self.uni.fresh()).collect();
        if arg_exprs.len() != variant.fields.len() {
            return self.error(
                span,
                format!(
                    "variant `{vname}` takes {} argument(s), got {}",
                    variant.fields.len(),
                    arg_exprs.len()
                ),
            );
        }
        for (arg, (_, fty)) in arg_exprs.iter().zip(&variant.fields) {
            let want = substitute_generics(fty, &fresh);
            self.check_expr(arg, &want);
        }
        Type::Named {
            id: def_id,
            args: fresh,
        }
    }

    fn infer_struct_lit(
        &mut self,
        name: &Path,
        base: Option<&Expr>,
        fields: &[(Ident, Expr)],
        span: Span,
    ) -> Type {
        let tname = &name.segments[0].name;
        let Some(&def_id) = self.defs.by_name.get(tname) else {
            return self.error(name.span, format!("unknown type `{tname}`"));
        };
        let def = self.defs.get(def_id);
        let generics = def.generics.len();
        let TypeDefKind::Struct {
            fields: def_fields,
        } = def.kind.clone()
        else {
            return self.error_help(
                name.span,
                format!("`{tname}` is not a struct"),
                "sum type values are built with variant constructors",
            );
        };
        let fresh: Vec<Type> = (0..generics).map(|_| self.uni.fresh()).collect();
        let mut seen: Vec<&str> = Vec::new();
        for (fname, fexpr) in fields {
            match def_fields.iter().find(|(n, _)| n == &fname.name) {
                Some((_, fty)) => {
                    if seen.contains(&fname.name.as_str()) {
                        self.error(fname.span, format!("field `{}` set twice", fname.name));
                    }
                    seen.push(&fname.name);
                    let want = substitute_generics(fty, &fresh);
                    self.check_expr(fexpr, &want);
                }
                None => {
                    self.error(
                        fname.span,
                        format!("`{tname}` has no field `{}`", fname.name),
                    );
                }
            }
        }
        let result = Type::Named {
            id: def_id,
            args: fresh.clone(),
        };
        // `Point{ ..p, x: 1 }` — `p` supplies every field not written, so it
        // must be the same struct, and nothing is missing.
        if let Some(b) = base {
            self.check_expr(b, &result);
            return result;
        }
        let missing: Vec<&String> = def_fields
            .iter()
            .map(|(n, _)| n)
            .filter(|n| !seen.contains(&n.as_str()))
            .collect();
        if !missing.is_empty() {
            let list = missing
                .iter()
                .map(|s| format!("`{s}`"))
                .collect::<Vec<_>>()
                .join(", ");
            self.error(span, format!("missing field(s) {list} in `{tname}` literal"));
        }
        Type::Named {
            id: def_id,
            args: fresh,
        }
    }

    fn infer_call(&mut self, callee: &Expr, args: &[Expr], span: Span) -> Type {
        if let Expr::Ident(id) = callee {
            // Local binding takes priority (closure in a variable).
            if self.lookup(&id.name).is_none() {
                // Builtin free functions.
                match id.name.as_str() {
                    "print" => {
                        if args.len() != 1 {
                            return self.error(span, "print takes exactly 1 argument");
                        }
                        let t = self.infer_expr(&args[0]);
                        if let Type::Fallible(..) = self.uni.shallow_resolve(&t) {
                            self.error_help(
                                args[0].span(),
                                "cannot print a fallible value",
                                "handle the error first with `try` or `match`",
                            );
                        }
                        return Type::Unit;
                    }
                    "assert" => {
                        if args.len() == 1 {
                            self.check_expr(&args[0], &Type::Bool);
                        } else {
                            return self.error(span, "assert takes exactly 1 argument");
                        }
                        return Type::Unit;
                    }
                    "assert_eq" => {
                        if args.len() != 2 {
                            return self.error(span, "assert_eq takes exactly 2 arguments");
                        }
                        let t = self.infer_expr(&args[0]);
                        self.check_expr(&args[1], &t);
                        return Type::Unit;
                    }
                    "panic" => {
                        if args.len() != 1 {
                            return self.error(span, "panic takes exactly 1 argument (message)");
                        }
                        self.check_expr(&args[0], &Type::Str);
                        return self.uni.fresh(); // bottom: fits anywhere
                    }
                    "read_file" => {
                        if args.len() != 1 {
                            return self.error(span, "read_file takes exactly 1 argument (path)");
                        }
                        self.check_expr(&args[0], &Type::Str);
                        let io_err = Type::Named {
                            id: self.defs.io_error_id,
                            args: vec![],
                        };
                        return Type::Fallible(Box::new(Type::Str), Box::new(io_err));
                    }
                    "write_file" => {
                        if args.len() != 2 {
                            return self.error(span, "write_file takes 2 arguments (path, content)");
                        }
                        self.check_expr(&args[0], &Type::Str);
                        self.check_expr(&args[1], &Type::Str);
                        let io_err = Type::Named {
                            id: self.defs.io_error_id,
                            args: vec![],
                        };
                        return Type::Fallible(Box::new(Type::Unit), Box::new(io_err));
                    }
                    "file_exists" => {
                        if args.len() != 1 {
                            return self.error(span, "file_exists takes exactly 1 argument (path)");
                        }
                        self.check_expr(&args[0], &Type::Str);
                        return Type::Bool;
                    }
                    "args" => {
                        if !args.is_empty() {
                            return self.error(span, "args takes no arguments");
                        }
                        return Type::List(Box::new(Type::Str));
                    }
                    "map" => {
                        if !args.is_empty() {
                            return self.error(span, "map() takes no arguments — add entries with .set(k, v)");
                        }
                        let k = self.uni.fresh();
                        let v = self.uni.fresh();
                        return Type::Map(Box::new(k), Box::new(v));
                    }
                    // The two constants the math methods are useless
                    // without. Guarded on `lookup_fn`, so a program with its
                    // own `fn e()` keeps it — unlike the older builtins here,
                    // which shadow a user function of the same name. `e` is a
                    // common enough identifier that taking it outright would
                    // break working programs.
                    "pi" | "e" if self.lookup_fn(&id.name).is_none() => {
                        if !args.is_empty() {
                            return self.error(
                                span,
                                format!("{} takes no arguments", id.name),
                            );
                        }
                        return Type::F64;
                    }
                    // Tier 2. `ref(v)` puts a value in the shared heap and
                    // hands back a generational reference to it; `release(r)`
                    // gives the slot up, after which every reference to it is
                    // stale and reading through one panics rather than reading
                    // whatever landed there next.
                    "ref" if self.lookup_fn("ref").is_none() => {
                        if args.len() != 1 {
                            return self.error(span, "ref takes exactly 1 argument (the value)");
                        }
                        let inner = self.infer_expr(&args[0]);
                        return Type::Ref(Box::new(inner));
                    }
                    "release" if self.lookup_fn("release").is_none() => {
                        if args.len() != 1 {
                            return self.error(span, "release takes exactly 1 argument (a `ref`)");
                        }
                        let t = self.infer_expr(&args[0]);
                        if !matches!(self.uni.resolve(&t), Type::Ref(_) | Type::Error) {
                            let shown = self.show(&t);
                            return self.error(
                                args[0].span(),
                                format!("release expects a `ref`, not `{shown}`"),
                            );
                        }
                        return Type::Unit;
                    }
                    "alive" if self.lookup_fn("alive").is_none() => {
                        if args.len() != 1 {
                            return self.error(span, "alive takes exactly 1 argument (a `ref`)");
                        }
                        let t = self.infer_expr(&args[0]);
                        if !matches!(self.uni.resolve(&t), Type::Ref(_) | Type::Error) {
                            let shown = self.show(&t);
                            return self.error(
                                args[0].span(),
                                format!("alive expects a `ref`, not `{shown}`"),
                            );
                        }
                        return Type::Bool;
                    }
                    "now_millis" => {
                        if !args.is_empty() {
                            return self.error(span, "now_millis takes no arguments");
                        }
                        return Type::I64;
                    }
                    "env" => {
                        if args.len() != 1 {
                            return self.error(span, "env takes exactly 1 argument (the variable name)");
                        }
                        self.check_expr(&args[0], &Type::Str);
                        return self.defs.option_some(Type::Str);
                    }
                    "exit" => {
                        if args.len() != 1 {
                            return self.error(span, "exit takes exactly 1 argument (the status code)");
                        }
                        self.check_expr(&args[0], &Type::I64);
                        return self.uni.fresh(); // diverges
                    }
                    "input" => {
                        if !args.is_empty() {
                            return self.error(span, "input takes no arguments");
                        }
                        return Type::Str;
                    }
                    // `Ok(v)` / `Err(e)` build a fallible value directly.
                    // They were matchable and not constructible, so a plain
                    // value could not be put into a `Map[str, T ! E]` without
                    // a trampoline function whose whole body was `v`.
                    "Ok" | "Err" => {
                        if args.len() != 1 {
                            return self.error(
                                span,
                                format!("`{}` takes exactly 1 argument", id.name),
                            );
                        }
                        let inner = self.infer_expr(&args[0]);
                        let other = self.uni.fresh();
                        return if id.name == "Ok" {
                            Type::Fallible(Box::new(inner), Box::new(other))
                        } else {
                            Type::Fallible(Box::new(other), Box::new(inner))
                        };
                    }
                    "fail" => {
                        if args.len() != 1 {
                            return self.error(span, "fail takes exactly 1 argument (the error value)");
                        }
                        // `fail` leaves the region carrying a value, the same
                        // as `return` — and its payload is usually built right
                        // there, which is precisely what must not escape.
                        // `return` was refused and this was not.
                        if self.region_depth > 0 {
                            self.error_help(
                                span,
                                "cannot `fail` from inside a region".to_string(),
                                "a region promises that nothing allocated inside outlives the \
closing brace, and an error value carries one out — yield a number from the region and \
fail outside it",
                            );
                        }
                        let e_ty = self.infer_expr(&args[0]);
                        match self.fn_err.clone() {
                            Some(declared) => {
                                if !self.err_fits(&e_ty, &declared) {
                                    let e = self.show(&e_ty);
                                    let d = self.show(&declared);
                                    self.error_help(
                                        span,
                                        format!("`fail` raises `{e}` but this function declares `{d}`"),
                                        "add the error type to the function's `!` clause",
                                    );
                                }
                            }
                            None => {
                                self.error_help(
                                    span,
                                    "`fail` used in a function that declares no error type",
                                    "add `! ErrorType` to the function signature",
                                );
                            }
                        }
                        return self.uni.fresh(); // bottom: fits anywhere
                    }
                    _ => {}
                }
                // Variant constructor.
                if let Some(cands) = self.variants.get(&id.name).cloned() {
                    if cands.len() == 1 {
                        let (def_id, vi) = cands[0];
                        return self.instantiate_variant(def_id, vi, args, span);
                    } else if cands.len() > 1 {
                        return self.error_help(
                            id.span,
                            format!("`{}` is ambiguous — variant of multiple types", id.name),
                            "qualify it as `TypeName.Variant(...)`",
                        );
                    }
                }
                // Named function.
                if let Some(info) = self.lookup_fn(&id.name) {
                    self.require_unsafe(&id.name, id.span);
                    return self.check_fn_call(&id.name, &info, args, span);
                }
            }
        }
        // Qualified variant: `Shape.Circle(...)` parses as MethodCall on type
        // name and lands in infer_method_call, not here. General case:
        //
        // A function *value* bound outside the region is a body written
        // outside it, and a closure carries its captures — the same way out
        // an effect handler is ([T-REGION-FOREIGN]). A closure written inside
        // the region is not: it can only capture what a region already
        // allows.
        if let Expr::Ident(id) = callee {
            self.reject_region_escape(id, "call");
        }
        let callee_ty = self.infer_expr(callee);
        match self.uni.shallow_resolve(&callee_ty) {
            Type::Fn { params, ret, .. } => {
                if args.len() != params.len() {
                    return self.error(
                        span,
                        format!("expected {} argument(s), got {}", params.len(), args.len()),
                    );
                }
                for (a, p) in args.iter().zip(&params) {
                    self.check_expr(a, p);
                }
                *ret
            }
            Type::Error => Type::Error,
            other => {
                let shown = TypeDisplay {
                    ty: &other,
                    defs: &self.defs,
                }
                .to_string();
                self.error(span, format!("`{shown}` is not callable"))
            }
        }
    }

    /// What a call may do when it is made from inside a region.
    ///
    /// The region rules were each written against the syntax between the
    /// braces, and a one-hop wrapper walked through all of them: a helper
    /// that performs the effect, or that is handed a closure bound outside,
    /// writes what the region allocated somewhere the region cannot see. What
    /// matters is not where the text is but what the call can reach
    /// ([T-REGION-FOREIGN]).
    fn check_region_call(&mut self, name: &str, args: &[Expr], span: Span) {
        if self.region_depth == 0 {
            return;
        }
        // A user-declared effect means a handler, and a handler is written
        // outside and may keep what it is given. `io`, `async` and `unsafe`
        // are built in: they have no handler and retain nothing.
        if let Some(row) = self.row_of_fn(name) {
            let user: Vec<&String> = row
                .iter()
                .filter(|e| !matches!(e.as_str(), "io" | "async" | "unsafe"))
                .collect();
            if let Some(first) = user.first() {
                self.error_help(
                    span,
                    format!("cannot call `{name}` inside a region — it performs `{first}`"),
                    "the effect's handler is written outside the region and may keep what it is given, which is how a value allocated inside gets out — perform the effect after the closing brace",
                );
            }
        }
        // A function value reaching the callee is a body written elsewhere,
        // and it carries its captures. One written *here* carries only what a
        // region already allows.
        for a in args {
            if matches!(a, Expr::Closure { .. }) {
                continue;
            }
            let t = self.infer_expr(a);
            if self.carries_fn(&t) {
                self.error_help(
                    a.span(),
                    "cannot pass a function into a call inside a region",
                    "a function value written outside the region carries its captures, so what the region allocates could be kept — write the closure inside the region, or move the call out",
                );
            }
        }
    }

    fn check_fn_call(&mut self, name: &str, info: &FnInfo, args: &[Expr], span: Span) -> Type {
        self.check_region_call(name, args, span);
        let fresh: Vec<Type> = (0..info.generics).map(|_| self.uni.fresh()).collect();
        if args.len() != info.params.len() {
            return self.error(
                span,
                format!(
                    "`{name}` expects {} argument(s), got {}",
                    info.params.len(),
                    args.len()
                ),
            );
        }
        for (a, p) in args.iter().zip(&info.params) {
            let want = substitute_generics(p, &fresh);
            self.check_expr(a, &want);
        }
        // Now that the arguments have fixed the type parameters, a `Map[K, _]`
        // parameter has a real key type — which is where the key rule lands
        // for a generic ([T-MAP-KEY]). Checking it in the loop above would ask
        // before the answer existed.
        if !fresh.is_empty() {
            for (a, p) in args.iter().zip(&info.params) {
                let want = substitute_generics(p, &fresh);
                self.require_map_keys_within(&want, a.span());
            }
            // The *result* too. `fn fresh[K](k: K) -> Map[K, i64]` puts the
            // key type nowhere in its parameters, so checking only those let
            // `fresh(2.5)` through the checker and into a runtime
            // `invalid map key` — a well-typed program panicking, which is the
            // thing [T-MAP-KEY] exists to stop.
            let ret = substitute_generics(&info.ret, &fresh);
            self.require_map_keys_within(&ret, span);
            if let Some(e) = &info.err {
                let e = substitute_generics(e, &fresh);
                self.require_map_keys_within(&e, span);
            }
        }
        // Ownership Tier-0: `own` moves, `inout` needs an exclusive mutable place.
        let mut inout_roots: Vec<(String, Span)> = Vec::new();
        let mut own_roots: Vec<String> = Vec::new();
        for ((a, p), mode) in args.iter().zip(&info.params).zip(&info.modes) {
            match mode {
                ParamMode::Own => {
                    if let Expr::Ident(id) = a {
                        if !self.is_copy_type(p) && self.lookup(&id.name).is_some() {
                            self.mark_moved(id);
                            own_roots.push(id.name.clone());
                        }
                    }
                }
                ParamMode::Inout => {
                    let root = place_root(a);
                    match root {
                        Some(id) => {
                            // `inout` is a write, so the region rule applies:
                            // passing an outer binding lets the callee store
                            // something region-allocated into it, which is the
                            // direct write one call away. The direct form was
                            // already refused and this was not.
                            self.reject_region_escape(id, "pass `inout`");
                            // `inout` works by snapshotting the place at the
                            // call and writing it back at the return. A `ref`
                            // reaches the same value without going through
                            // that snapshot, so a write made through one
                            // during the call is overwritten by the writeback
                            // and vanishes with no diagnostic. The two ways of
                            // mutating are individually sound and cannot be
                            // combined ([T-INOUT-REF]).
                            // The whole path, not only its root: hiding the
                            // reference one field deep — `f(h.r.xs, h)` where
                            // `h: Holder { r: ref B }` — walked straight past
                            // a rule about the root's type, and
                            // [T-REF-GENERATIONAL] names a struct or a list
                            // as the ordinary place a reference lives.
                            let root_is_ref = self
                                .lookup(&id.name)
                                .is_some_and(|b| matches!(b.ty, Type::Ref(_)))
                                || self.path_crosses_ref(a);
                            if root_is_ref {
                                self.error_help(
                                    id.span,
                                    format!(
                                        "cannot pass `{}` as `inout` — it is reached through a `ref`",
                                        id.name
                                    ),
                                    "write through the reference instead, or hold the value in a `var` and drop the reference",
                                );
                            }
                            match self.lookup(&id.name) {
                                Some(b) if !b.mutable && !root_is_ref => {
                                    self.error_help(
                                        id.span,
                                        format!(
                                            "cannot pass `{}` as `inout` — it is immutable",
                                            id.name
                                        ),
                                        format!("declare it with `var {}`", id.name),
                                    );
                                }
                                Some(_) => {}
                                None => {} // unknown-name error already reported
                            }
                            // Passing a captured variable as `inout` writes to
                            // the closure's copy, which nobody can read. It is
                            // the same mistake as assigning to it, and used to
                            // be the quiet one: the call ran, the write landed
                            // somewhere invisible, and the caller saw nothing.
                            self.reject_capture_write(id, "pass", " as `inout`");
                            if let Some((first, _)) =
                                inout_roots.iter().find(|(n, _)| n == &id.name)
                            {
                                let first = first.clone();
                                self.error_help(
                                    id.span,
                                    format!(
                                        "`{first}` is passed as `inout` twice in one call"
                                    ),
                                    "two exclusive references to the same value cannot coexist",
                                );
                            }
                            inout_roots.push((id.name.clone(), id.span));
                        }
                        None => {
                            self.error_help(
                                a.span(),
                                "`inout` argument must be a variable, field, or index expression",
                                "temporaries cannot be mutated in place",
                            );
                        }
                    }
                }
                ParamMode::Let => {}
            }
        }
        for (n, s) in &inout_roots {
            if own_roots.contains(n) {
                self.diags.push(
                    Diagnostic::error(
                        *s,
                        format!("`{n}` is both moved (`own`) and mutated (`inout`) in one call"),
                    )
                    .with_help("a value cannot be transferred and borrowed at the same time"),
                );
            }
        }
        let ret = substitute_generics(&info.ret, &fresh);
        match &info.err {
            Some(err) => Type::Fallible(
                Box::new(ret),
                Box::new(substitute_generics(err, &fresh)),
            ),
            None => ret,
        }
    }

    fn infer_field(&mut self, recv: &Expr, name: &Ident, span: Span) -> Type {
        // `Type.Variant` as a qualified nullary variant?
        if let Expr::Ident(tid) = recv {
            if self.lookup(&tid.name).is_none() {
                if let Some(&def_id) = self.defs.by_name.get(&tid.name) {
                    if let TypeDefKind::Sum { variants } = &self.defs.get(def_id).kind {
                        if let Some(vi) = variants.iter().position(|v| v.name == name.name) {
                            return self.instantiate_variant(def_id, vi, &[], span);
                        }
                    }
                    return self.error(
                        span,
                        format!("type `{}` has no variant `{}`", tid.name, name.name),
                    );
                }
            }
        }
        // `lib.WIDTH` — an exported constant. `pub let` parsed, checked and
        // did nothing: no other module could read one, `doc` omitted it and
        // `api` omitted it, so it was syntax with no meaning.
        if let Expr::Ident(tid) = recv {
            if self.lookup(&tid.name).is_none() {
                let target = self
                    .module_imports
                    .get(&self.current_module)
                    .and_then(|m| m.get(&tid.name))
                    .cloned();
                if let Some(target_mod) = target {
                    let qual = format!("{target_mod}::{}", name.name);
                    if let Some((ty, is_pub)) = self.consts.get(&qual).cloned() {
                        if is_pub {
                            return ty;
                        }
                        return self.error_help(
                            name.span,
                            format!("constant `{}` in module `{target_mod}` is private", name.name),
                            "mark it `pub let` to allow cross-module use",
                        );
                    }
                }
            }
        }
        // `utils.helper` as a *value* — the same function a call would reach,
        // not called. Without this, every `map`/`sort_by`/`fold` over an
        // imported function had to be wrapped in a closure, and the error said
        // the module did not exist.
        if let Expr::Ident(tid) = recv {
            if self.lookup(&tid.name).is_none() {
                let target = self
                    .module_imports
                    .get(&self.current_module)
                    .and_then(|m| m.get(&tid.name))
                    .cloned();
                if let Some(target_mod) = target {
                    let qual = format!("{target_mod}::{}", name.name);
                    match self.fns.get(&qual).cloned() {
                        Some(info) if info.is_pub => {
                            let ret = match &info.err {
                                Some(e) => Type::Fallible(
                                    Box::new(info.ret.clone()),
                                    Box::new(e.clone()),
                                ),
                                None => info.ret.clone(),
                            };
                            // A generic's type parameters are instantiated
                            // here, as they are for a bare name. Leaving them
                            // as `Generic(i)` meant `xs |> lib.uniq` and
                            // `xs.map(lib.uniq)` reported `expected [T0]`,
                            // while `lib.uniq(xs)` on the next line was fine.
                            let fresh: Vec<Type> =
                                (0..info.generics).map(|_| self.uni.fresh()).collect();
                            return Type::Fn {
                                params: info
                                    .params
                                    .iter()
                                    .map(|p| substitute_generics(p, &fresh))
                                    .collect(),
                                ret: Box::new(substitute_generics(&ret, &fresh)),
                                row: None,
                            };
                        }
                        Some(_) => {
                            return self.error_help(
                                name.span,
                                format!(
                                    "function `{}` in module `{target_mod}` is private",
                                    name.name
                                ),
                                "mark it `pub fn` to allow cross-module use",
                            );
                        }
                        None => {}
                    }
                }
            }
        }
        let recv_ty = self.infer_expr(recv);
        // Tier 2: a `ref T` reads as the `T` it points at. The check happens
        // at run time, on the generation, so a reference to a released slot
        // panics rather than reading whatever occupies it now.
        let recv_ty = match self.uni.resolve(&recv_ty) {
            Type::Ref(inner) => *inner,
            _ => recv_ty,
        };
        match self.uni.shallow_resolve(&recv_ty) {
            // `pair.0` — a tuple's components are named by position.
            Type::Tuple(parts) => match name.name.parse::<usize>() {
                Ok(i) if i < parts.len() => parts[i].clone(),
                Ok(i) => self.error(
                    span,
                    format!(
                        "this tuple has {} component(s), so there is no `.{i}`",
                        parts.len()
                    ),
                ),
                Err(_) => self.error_help(
                    span,
                    format!("a tuple has no field `{}`", name.name),
                    "tuple components are named by position, as in `pair.0`",
                ),
            },
            Type::Named { id, args } => {
                let def = self.defs.get(id);
                match &def.kind {
                    TypeDefKind::Struct { fields } => {
                        match fields.iter().find(|(n, _)| n == &name.name) {
                            Some((_, fty)) => substitute_generics(fty, &args),
                            None => {
                                let tname = def.name.clone();
                                self.error(
                                    name.span,
                                    format!("`{tname}` has no field `{}`", name.name),
                                )
                            }
                        }
                    }
                    TypeDefKind::Sum { .. } => {
                        let tname = def.name.clone();
                        self.error_help(
                            name.span,
                            format!("`{tname}` is a sum type — fields are accessed via `match`"),
                            "match on the value to get at the variant's fields",
                        )
                    }
                }
            }
            Type::Error => Type::Error,
            other => {
                let shown = TypeDisplay {
                    ty: &other,
                    defs: &self.defs,
                }
                .to_string();
                self.error(
                    name.span,
                    format!("`{shown}` has no field `{}`", name.name),
                )
            }
        }
    }

    fn infer_method_call(
        &mut self,
        recv: &Expr,
        name: &Ident,
        args: &[Expr],
        span: Span,
    ) -> Type {
        // Cross-module qualified call `utils.helper(...)`.
        if let Expr::Ident(tid) = recv {
            if self.lookup(&tid.name).is_none() {
                let target = self
                    .module_imports
                    .get(&self.current_module)
                    .and_then(|m| m.get(&tid.name))
                    .cloned();
                if let Some(target_mod) = target {
                    let qual = format!("{target_mod}::{}", name.name);
                    match self.fns.get(&qual).cloned() {
                        Some(info) if info.is_pub => {
                            return self.check_fn_call(&name.name, &info, args, span);
                        }
                        Some(_) => {
                            return self.error_help(
                                name.span,
                                format!(
                                    "function `{}` in module `{target_mod}` is private",
                                    name.name
                                ),
                                "mark it `pub fn` to allow cross-module calls",
                            );
                        }
                        None => {
                            // Fall through: `shapes.Circle(...)` — a variant
                            // constructor referenced through the module alias
                            // (type namespaces are global).
                            if let Some(cands) = self.variants.get(&name.name).cloned() {
                                if cands.len() == 1 {
                                    let (def_id, vi) = cands[0];
                                    return self.instantiate_variant(def_id, vi, args, span);
                                }
                            }
                            return self.error(
                                name.span,
                                format!(
                                    "module `{target_mod}` has no function `{}`",
                                    name.name
                                ),
                            );
                        }
                    }
                }
            }
        }
        // Effect operation call `Clock.now(...)`.
        if let Expr::Ident(tid) = recv {
            if self.lookup(&tid.name).is_none() {
                if let Some((ops, _)) = self.effects.get(&tid.name).cloned() {
                    // The handler is written outside the region and runs
                    // inside it, so whatever it captures — a `ref`, most
                    // usefully — is a way out that the region cannot see
                    // ([T-REGION-FOREIGN]).
                    if self.region_depth > 0 {
                        self.error_help(
                            span,
                            format!(
                                "cannot perform effect `{}.{}` inside a region",
                                tid.name, name.name
                            ),
                            "the handler is written outside the region and may keep what it \
is given, which is how a value allocated inside gets out — perform the effect after the \
closing brace",
                        );
                    }
                    match ops.iter().find(|o| o.name == name.name) {
                        Some(op) => {
                            if args.len() != op.params.len() {
                                return self.error(
                                    span,
                                    format!(
                                        "effect operation `{}.{}` takes {} argument(s), got {}",
                                        tid.name,
                                        name.name,
                                        op.params.len(),
                                        args.len()
                                    ),
                                );
                            }
                            let params = op.params.clone();
                            let ret = op.ret.clone();
                            for (a, p) in args.iter().zip(&params) {
                                self.check_expr(a, p);
                            }
                            return ret;
                        }
                        None => {
                            return self.error(
                                name.span,
                                format!("effect `{}` has no operation `{}`", tid.name, name.name),
                            );
                        }
                    }
                }
            }
        }
        // Qualified variant constructor `Shape.Circle(...)`.
        if let Expr::Ident(tid) = recv {
            if self.lookup(&tid.name).is_none() {
                if let Some(&def_id) = self.defs.by_name.get(&tid.name) {
                    if let TypeDefKind::Sum { variants } = &self.defs.get(def_id).kind {
                        if let Some(vi) = variants.iter().position(|v| v.name == name.name) {
                            return self.instantiate_variant(def_id, vi, args, span);
                        }
                    }
                    return self.error(
                        span,
                        format!("type `{}` has no variant `{}`", tid.name, name.name),
                    );
                }
            }
        }
        let recv_ty = self.infer_expr(recv);
        let recv_res = self.uni.resolve(&recv_ty);
        if let Some(result) = self.builtin_method(&recv_res, recv, name, args, span) {
            return result;
        }
        // UFCS: `x.f(a)` → `f(x, a)`.
        if let Some(info) = self.lookup_fn(&name.name) {
            // Including the Tier 3 gate. [L-UFCS] says the two spellings are
            // the same call, and only one of them was checked: `n.close()`
            // reached libc with no `unsafe`, no `io` in the row, and no
            // capability — through a rule that exists to make exactly that
            // impossible.
            self.require_unsafe(&name.name, name.span);
            let mut full_args: Vec<Expr> = Vec::with_capacity(args.len() + 1);
            full_args.push(recv.clone());
            full_args.extend(args.iter().cloned());
            return self.check_fn_call(&name.name, &info, &full_args, span);
        }
        // A field holding a function, called through the field. Storing one
        // and building the value both worked, and `n.run(5)` did not: the only
        // spelling that ran was `let f = n.run` and then `f(5)`, which nothing
        // pointed to. The help offered instead — declare `fn run(Node, ...)` —
        // is a different function that would not call the field at all.
        if self.struct_field_type(&recv_res, &name.name).is_some() {
            let ft = self.infer_field(recv, name, span);
            let resolved = self.uni.shallow_resolve(&ft);
            if let Type::Fn { params, ret, .. } = resolved {
                if args.len() != params.len() {
                    return self.error(
                        span,
                        format!(
                            "`{}` expects {} argument(s), got {}",
                            name.name,
                            params.len(),
                            args.len()
                        ),
                    );
                }
                for (a, p) in args.iter().zip(&params) {
                    self.check_expr(a, p);
                }
                return *ret;
            }
        }
        let shown = self.show(&recv_ty);
        self.error_help(
            name.span,
            format!("no method `{}` on `{shown}`", name.name),
            format!("a free function `fn {}({shown}, ...)` would also work here (UFCS)", name.name),
        )
    }

    /// The declared type of `field` on a struct type, if `t` is one with it.
    fn struct_field_type(&mut self, t: &Type, field: &str) -> Option<Type> {
        match self.uni.shallow_resolve(t) {
            Type::Named { id, .. } => {
                let TypeDefKind::Struct { fields } = &self.defs.get(id).kind else {
                    return None;
                };
                fields.iter().find(|(n, _)| n == field).map(|(_, t)| t.clone())
            }
            // A tuple's components are fields spelled with numbers. Reading
            // one as a value already worked; calling one did not, so a
            // function in a tuple could only be called by binding it to a
            // name first — the same defect [T-FIELD-CALL] records for struct
            // fields, and the help offered was `fn 0(...)`, which is not a
            // name anyone can write.
            Type::Tuple(items) => field.parse::<usize>().ok().and_then(|i| items.get(i).cloned()),
            // Reading a field through a reference works, so calling one does
            // too ([T-REF-READ]).
            Type::Ref(inner) => self.struct_field_type(&inner, field),
            _ => None,
        }
    }

    /// Built-in methods on list/str/i64/f64. Returns None → not a builtin.
    fn builtin_method(
        &mut self,
        recv_ty: &Type,
        recv: &Expr,
        name: &Ident,
        args: &[Expr],
        span: Span,
    ) -> Option<Type> {
        let check_arity = |c: &mut Self, want: usize| -> bool {
            if args.len() != want {
                c.error(
                    span,
                    format!("`{}` takes {want} argument(s), got {}", name.name, args.len()),
                );
                return false;
            }
            true
        };
        let t = match recv_ty {
            Type::List(elem) => {
                let elem = (**elem).clone();
                match name.name.as_str() {
                    "len" => {
                        check_arity(self, 0);
                        Type::I64
                    }
                    "push" => {
                        self.require_mutable(recv, name);
                        if check_arity(self, 1) {
                            self.check_expr(&args[0], &elem);
                        }
                        Type::Unit
                    }
                    "pop" => {
                        self.require_mutable(recv, name);
                        check_arity(self, 0);
                        self.defs.option_some(elem)
                    }
                    "get" => {
                        if check_arity(self, 1) {
                            self.check_expr(&args[0], &Type::I64);
                        }
                        self.defs.option_some(elem)
                    }
                    "contains" => {
                        if check_arity(self, 1) {
                            self.check_expr(&args[0], &elem);
                        }
                        Type::Bool
                    }
                    "map" => {
                        let out = self.uni.fresh();
                        if check_arity(self, 1) {
                            let f = Type::Fn {
                                params: vec![elem.clone()],
                                ret: Box::new(out.clone()),
                                row: None,
                            };
                            self.check_expr(&args[0], &f);
                        }
                        Type::List(Box::new(out))
                    }
                    // The accumulator's type comes from the initial value, and
                    // the function is checked against it — so `fold` is the
                    // one builtin whose result is not a function of the
                    // element type alone.
                    "fold" => {
                        let acc = self.uni.fresh();
                        if check_arity(self, 2) {
                            self.check_expr(&args[0], &acc);
                            let f = Type::Fn {
                                params: vec![acc.clone(), elem.clone()],
                                ret: Box::new(acc.clone()),
                                row: None,
                            };
                            self.check_expr(&args[1], &f);
                        }
                        acc
                    }
                    "par_map" => {
                        // Same shape as `map`; the difference is that the
                        // function must be effect-free (checked in effects.rs)
                        // so the elements can be processed on several cores
                        // without any observable interleaving.
                        let out = self.uni.fresh();
                        if check_arity(self, 1) {
                            let f = Type::Fn {
                                params: vec![elem.clone()],
                                ret: Box::new(out.clone()),
                                row: None,
                            };
                            self.check_expr(&args[0], &f);
                        }
                        Type::List(Box::new(out))
                    }
                    // `par_filter` is `filter` with the predicate required to
                    // be effect-free (checked in effects.rs), which is what
                    // lets the elements be tested on several cores at once.
                    "filter" | "par_filter" => {
                        if check_arity(self, 1) {
                            let f = Type::Fn {
                                params: vec![elem.clone()],
                                ret: Box::new(Type::Bool),
                                row: None,
                            };
                            self.check_expr(&args[0], &f);
                        }
                        Type::List(Box::new(elem))
                    }
                    "any" | "all" => {
                        if check_arity(self, 1) {
                            let f = Type::Fn {
                                params: vec![elem.clone()],
                                ret: Box::new(Type::Bool),
                                row: None,
                            };
                            self.check_expr(&args[0], &f);
                        }
                        Type::Bool
                    }
                    "find" => {
                        if check_arity(self, 1) {
                            let f = Type::Fn {
                                params: vec![elem.clone()],
                                ret: Box::new(Type::Bool),
                                row: None,
                            };
                            self.check_expr(&args[0], &f);
                        }
                        self.defs.option_some(elem)
                    }
                    "min" | "max" => {
                        check_arity(self, 0);
                        // The same predicate `sort` uses. Spelling the list
                        // out separately here left tuples out, so
                        // `xs.enumerate().min()` — the obvious "index of the
                        // smallest" — was an error on a list `sort` will
                        // happily order, against [L-LIB-TUPLE].
                        if !self.is_ordered(&elem) {
                            let shown = self.show(&elem);
                            self.error(span, format!("cannot take min/max of `{shown}` values"));
                        }
                        self.defs.option_some(elem)
                    }
                    "is_empty" => {
                        check_arity(self, 0);
                        Type::Bool
                    }
                    "sum" => {
                        check_arity(self, 0);
                        let r = self.uni.shallow_resolve(&elem);
                        if !matches!(r, Type::I64 | Type::F64 | Type::Var(_) | Type::Error) {
                            let shown = self.show(&elem);
                            self.error(span, format!("cannot sum a list of `{shown}`"));
                        }
                        if let Type::Var(_) = r {
                            self.uni.unify(&elem, &Type::I64);
                            return Some(Type::I64);
                        }
                        // A float sum is a float even with nothing to add.
                        // The interpreter decides int-versus-float from the
                        // elements, and an empty `[f64]` has none — so it
                        // answered `0`, printed without a point, and then
                        // panicked `no method to_fixed on 0` when the caller
                        // treated the result as the `f64` the checker had
                        // promised. Recorded the same way promoted literals
                        // are, because the type is again something only this
                        // pass knows.
                        // Keyed on the method name's own span, which the
                        // interpreter also has where it needs it.
                        if r == Type::F64 {
                            self.float_sums.insert(name.span.start);
                        }
                        r
                    }
                    "join" => {
                        if check_arity(self, 1) {
                            self.check_expr(&args[0], &Type::Str);
                        }
                        if !self.uni.unify(&elem, &Type::Str) {
                            let shown = self.show(&elem);
                            self.error(span, format!("`join` needs `[str]`, found list of `{shown}`"));
                        }
                        Type::Str
                    }
                    "rev" => {
                        check_arity(self, 0);
                        Type::List(Box::new(elem))
                    }
                    "slice" => {
                        if check_arity(self, 2) {
                            self.check_expr(&args[0], &Type::I64);
                            self.check_expr(&args[1], &Type::I64);
                        }
                        Type::List(Box::new(elem))
                    }
                    "first" | "last" => {
                        check_arity(self, 0);
                        self.defs.option_some(elem)
                    }
                    "sort" => {
                        self.require_mutable(recv, name);
                        check_arity(self, 0);
                        // Sorting elements with no order was a silent no-op:
                        // the list came back unchanged and nothing said why.
                        // Comparing two of those same values with `<` is a
                        // compile error, so this is now the same error rather
                        // than the quiet one.
                        let e = self.uni.shallow_resolve(&elem);
                        if !self.is_ordered(&e) {
                            let shown = self.show(&elem);
                            self.error_help(
                                span,
                                format!("`{shown}` has no order, so `sort` cannot arrange it"),
                                "only `i64`, `f64` and `str` are ordered; use `sort_by` with a key you extract from each element",
                            );
                        }
                        Type::Unit
                    }
                    // Sorting by an extracted key. The keys are what must be
                    // ordered, not the elements, so this is how a list of
                    // structs gets sorted at all.
                    // Removing by index. A queue drained with
                    // `xs = xs.slice(1, xs.len())` is O(n) a step and reads
                    // like a workaround, because it is one.
                    "remove" => {
                        self.require_mutable(recv, name);
                        if check_arity(self, 1) {
                            self.check_expr(&args[0], &Type::I64);
                        }
                        elem.clone()
                    }
                    "enumerate" => {
                        check_arity(self, 0);
                        Type::List(Box::new(Type::Tuple(vec![Type::I64, elem.clone()])))
                    }
                    "zip" => {
                        let other = self.uni.fresh();
                        if check_arity(self, 1) {
                            self.check_expr(&args[0], &Type::List(Box::new(other.clone())));
                        }
                        Type::List(Box::new(Type::Tuple(vec![elem.clone(), other])))
                    }
                    "sort_by" => {
                        self.require_mutable(recv, name);
                        let key = self.uni.fresh();
                        if check_arity(self, 1) {
                            let f = Type::Fn {
                                params: vec![elem.clone()],
                                ret: Box::new(key.clone()),
                                row: None,
                            };
                            self.check_expr(&args[0], &f);
                            // No default here: the key function may be a
                            // closure whose body has not been checked yet, so
                            // an unresolved key is "not known", not "an int".
                            let k = self.uni.shallow_resolve(&key);
                            if !self.is_ordered(&k) {
                                let shown = self.show(&key);
                                self.error_help(
                                    span,
                                    format!("`{shown}` has no order, so `sort_by` cannot use it as a key"),
                                    "return an `i64`, `f64` or `str` from the key function",
                                );
                            }
                        }
                        Type::Unit
                    }
                    _ => return None,
                }
            }
            Type::Str => match name.name.as_str() {
                "cmp" => return self.check_cmp(recv, args, span),
                "len" => {
                    check_arity(self, 0);
                    Type::I64
                }
                "contains" | "starts_with" | "ends_with" => {
                    if check_arity(self, 1) {
                        self.check_expr(&args[0], &Type::Str);
                    }
                    Type::Bool
                }
                "split" => {
                    if check_arity(self, 1) {
                        self.check_expr(&args[0], &Type::Str);
                    }
                    Type::List(Box::new(Type::Str))
                }
                "chars" | "lines" => {
                    check_arity(self, 0);
                    Type::List(Box::new(Type::Str))
                }
                "pad_start" | "pad_end" => {
                    if check_arity(self, 2) {
                        self.check_expr(&args[0], &Type::I64);
                        self.check_expr(&args[1], &Type::Str);
                    }
                    Type::Str
                }
                "is_empty" => {
                    check_arity(self, 0);
                    Type::Bool
                }
                "trim" | "trim_start" | "trim_end" | "to_upper" | "to_lower" => {
                    check_arity(self, 0);
                    Type::Str
                }
                // Code points, both ways. Without these a string can be cut
                // up and compared but never *computed* on — no hash, no
                // cipher, no hand-written number parser.
                "code_at" => {
                    if check_arity(self, 1) {
                        self.check_expr(&args[0], &Type::I64);
                    }
                    Type::I64
                }
                "codes" => {
                    check_arity(self, 0);
                    Type::List(Box::new(Type::I64))
                }
                // Splitting on runs of whitespace. `split(" ")` cannot do it:
                // aligned columns give `["INFO", "", "", "", "server"]`, so
                // every call site grew a `.filter(|s| not s.is_empty())`.
                "words" => {
                    check_arity(self, 0);
                    Type::List(Box::new(Type::Str))
                }
                "repeat" => {
                    if check_arity(self, 1) {
                        self.check_expr(&args[0], &Type::I64);
                    }
                    Type::Str
                }
                "replace" => {
                    if check_arity(self, 2) {
                        self.check_expr(&args[0], &Type::Str);
                        self.check_expr(&args[1], &Type::Str);
                    }
                    Type::Str
                }
                "substr" => {
                    if check_arity(self, 2) {
                        self.check_expr(&args[0], &Type::I64);
                        self.check_expr(&args[1], &Type::I64);
                    }
                    Type::Str
                }
                "index_of" => {
                    if check_arity(self, 1) {
                        self.check_expr(&args[0], &Type::Str);
                    }
                    self.defs.option_some(Type::I64)
                }
                "parse_int" => {
                    check_arity(self, 0);
                    self.defs.option_some(Type::I64)
                }
                "parse_float" => {
                    check_arity(self, 0);
                    self.defs.option_some(Type::F64)
                }
                _ => return None,
            },
            Type::Tuple(_) if name.name == "cmp" => return self.check_cmp(recv, args, span),
            Type::I64 => match name.name.as_str() {
                "cmp" => return self.check_cmp(recv, args, span),
                "as_f64" => {
                    check_arity(self, 0);
                    Type::F64
                }
                // The other half of `code_at`: a code point back to text.
                "to_char" => {
                    check_arity(self, 0);
                    Type::Str
                }
                "abs" => {
                    check_arity(self, 0);
                    Type::I64
                }
                "min" | "max" => {
                    if check_arity(self, 1) {
                        self.check_expr(&args[0], &Type::I64);
                    }
                    Type::I64
                }
                "to_str" => {
                    check_arity(self, 0);
                    Type::Str
                }
                _ => return None,
            },
            Type::F64 => match name.name.as_str() {
                "cmp" => return self.check_cmp(recv, args, span),
                // Defensive parsing needs these: `parse_float` accepts `nan`
                // and overflows to an infinity ([L-LIB-PARSE]), so a program
                // reading untrusted numbers has to be able to ask.
                "is_nan" | "is_finite" => {
                    check_arity(self, 0);
                    Type::Bool
                }
                // Fixed-point text. Without it a report that has a mean can
                // only print `289.22222222222223`, and every program grows a
                // hand-rolled formatter built out of `round` and `to_str`.
                "to_fixed" => {
                    if check_arity(self, 1) {
                        self.check_expr(&args[0], &Type::I64);
                    }
                    Type::Str
                }
                "as_i64" => {
                    check_arity(self, 0);
                    Type::I64
                }
                "abs" | "sqrt" | "floor" | "ceil" | "round" => {
                    check_arity(self, 0);
                    Type::F64
                }
                // The math library: pure `f64 -> f64`. Reaching them through
                // `extern fn` instead cost a capability ([T-EXTERN-VALUE]),
                // which put a trigonometry helper over the ceiling of any
                // package declaring `capabilities = []`.
                "sin" | "cos" | "tan" | "asin" | "acos" | "atan" | "sinh" | "cosh" | "tanh" | "exp" | "ln" | "log2" | "log10" | "cbrt" | "to_radians" | "to_degrees" => {
                    check_arity(self, 0);
                    Type::F64
                }
                "atan2" | "hypot" | "log" => {
                    if check_arity(self, 1) {
                        self.check_expr(&args[0], &Type::F64);
                    }
                    Type::F64
                }
                "min" | "max" => {
                    if check_arity(self, 1) {
                        self.check_expr(&args[0], &Type::F64);
                    }
                    Type::F64
                }
                "to_str" => {
                    check_arity(self, 0);
                    Type::Str
                }
                _ => return None,
            },
            Type::Map(k, v) => {
                let k = (**k).clone();
                let v = (**v).clone();
                match name.name.as_str() {
                    "set" => {
                        self.require_mutable(recv, name);
                        if check_arity(self, 2) {
                            self.check_expr(&args[0], &k);
                            self.check_expr(&args[1], &v);
                            self.require_map_key(&k, span);
                        }
                        Type::Unit
                    }
                    "get" => {
                        if check_arity(self, 1) {
                            self.check_expr(&args[0], &k);
                            self.require_map_key(&k, span);
                        }
                        self.defs.option_some(v)
                    }
                    "has" => {
                        if check_arity(self, 1) {
                            self.check_expr(&args[0], &k);
                        }
                        Type::Bool
                    }
                    "delete" => {
                        self.require_mutable(recv, name);
                        if check_arity(self, 1) {
                            self.check_expr(&args[0], &k);
                        }
                        Type::Bool
                    }
                    "len" => {
                        check_arity(self, 0);
                        Type::I64
                    }
                    "is_empty" => {
                        check_arity(self, 0);
                        Type::Bool
                    }
                    "keys" => {
                        check_arity(self, 0);
                        Type::List(Box::new(k))
                    }
                    "values" => {
                        check_arity(self, 0);
                        Type::List(Box::new(v))
                    }
                    "entries" => {
                        check_arity(self, 0);
                        Type::List(Box::new(Type::Tuple(vec![k, v])))
                    }
                    _ => return None,
                }
            }
            Type::Named { id, args: targs } if *id == self.defs.option_id => {
                let inner = targs[0].clone();
                match name.name.as_str() {
                    "is_some" | "is_none" => {
                        check_arity(self, 0);
                        Type::Bool
                    }
                    "unwrap_or" => {
                        if check_arity(self, 1) {
                            self.check_expr(&args[0], &inner);
                        }
                        inner
                    }
                    // The escape hatch for "this is certainly Some". It takes
                    // a message rather than being a bare `unwrap`, because the
                    // message is what makes the panic worth reading.
                    "expect" => {
                        if check_arity(self, 1) {
                            self.check_expr(&args[0], &Type::Str);
                        }
                        inner
                    }
                    _ => return None,
                }
            }
            _ => return None,
        };
        Some(t)
    }

    /// Applies the map-key rule to every map inside a type.
    ///
    /// Declaring `Map[K, V]` with a type parameter is allowed; each call site
    /// is where `K` is finally something, and so where the rule lands.
    fn require_map_keys_within(&mut self, t: &Type, span: Span) {
        match self.uni.resolve(t) {
            Type::Map(k, v) => {
                self.require_map_key(&k, span);
                self.require_map_keys_within(&v, span);
            }
            Type::List(e) => self.require_map_keys_within(&e, span),
            Type::Tuple(parts) => {
                for p in parts {
                    self.require_map_keys_within(&p, span);
                }
            }
            Type::Fallible(ok, _) => self.require_map_keys_within(&ok, span),
            _ => {}
        }
    }

    /// Map keys must be orderable primitives.
    fn require_map_key(&mut self, k: &Type, span: Span) {
        let r = self.uni.resolve(k);
        // A type *parameter* is allowed here and checked at each call site
        // instead ([T-MAP-KEY]). Without this a generic helper over maps —
        // `fn bump[K](inout m: Map[K, i64], k: K)`, the most repeated shape
        // there is — could not be declared at all, and there are no bounds to
        // declare it with.
        if !matches!(
            r,
            Type::I64 | Type::Str | Type::Bool | Type::Var(_) | Type::Generic(_) | Type::Error
        ) {
            let shown = self.show(k);
            self.error_help(
                span,
                format!("map keys must be `i64`, `str`, or `bool`, not `{shown}`"),
                "use a primitive key, or serialize the value to a string key",
            );
        }
    }

    /// Mutating methods need a mutable receiver root.
    /// Rejects a write to a binding the enclosing closure captured.
    ///
    /// A closure captures values, not bindings ([E-CLOSURE-CAPTURE]), so a
    /// write to a captured name changes a copy that is discarded when the call
    /// returns. It compiles, it runs, and it does nothing — which is how an
    /// effect handler that appears to accumulate state silently accumulates
    /// nothing. There is no case where this is what someone meant.
    fn reject_capture_write(&mut self, id: &Ident, verb: &str, tail: &str) {
        let Some(floor) = self.closure_floors.last().copied() else {
            return;
        };
        // A captured `ref` is exempt, and it is the whole point of Tier 2.
        // The rule exists because a closure captures *values*, so a write
        // lands in a copy nobody can see — but a reference copies as a slot
        // id, so a write through one is visible to everybody holding it.
        // Refusing it left an effect handler unable to accumulate anything at
        // all: the obvious `|e| log.push(e)` was rejected, and so was the
        // reference that answers it, which is what made `inout` on an effect
        // operation look like the remaining option ([E-EFFECT-NO-INOUT]).
        if let Some(b) = self.lookup(&id.name) {
            if matches!(self.uni.resolve(&b.ty), Type::Ref(_)) {
                return;
            }
        }
        let declared_at = self
            .scopes
            .iter()
            .rposition(|sc| sc.contains_key(&id.name));
        if let Some(at) = declared_at {
            if at < floor {
                self.error_help(
                    id.span,
                    format!(
                        "cannot {verb} `{}`{tail} inside a closure — it was captured from outside",
                        id.name
                    ),
                    // `ref` is first because it is the one that always
                    // works. When the closure is an effect operation —
                    // which is the shape people hit this in — `inout` is
                    // not available at all ([E-EFFECT-NO-INOUT]), and
                    // following that suggestion led to the dead end
                    // [T-REF-CAPTURE] exists to keep people out of.
                    "a closure captures values, not bindings, so this write would change a copy \
nobody can see — hold it behind a `ref`, which copies as a slot rather than as a value and so \
is shared; or return the new value; or, where the closure is not an effect operation, thread \
the state through an `inout` parameter",
                );
            }
        }
    }

    fn require_mutable(&mut self, recv: &Expr, name: &Ident) {
        let mut root = recv;
        while let Expr::Field { recv, .. } | Expr::Index { recv, .. } = root {
            root = recv;
        }
        // As in `check_assign_target`: mutating through a `ref` mutates what
        // it points at, not the reference, so the binding's own mutability is
        // not the question.
        let through_ref = self.path_crosses_ref(recv);
        if let Expr::Ident(id) = root {
            self.reject_region_escape(id, "mutate");
            if let Some(b) = self.lookup(&id.name) {
                if !b.mutable && !through_ref {
                    self.error_help(
                        id.span,
                        format!("cannot mutate `{}` — it is immutable", id.name),
                        format!("declare it with `var {}`", id.name),
                    );
                }
            }
            self.reject_capture_write(id, "mutate", "");
            // Pushing into an outer container is the same departure as
            // assigning to an outer binding.
            let t = self.infer_expr(recv);
            self.reject_handler_escape(recv, &t);
        } else {
            // Not rooted at a binding at all: `[3, 1, 2].remove(0)`,
            // `mk().pop()`. There is nothing for the mutation to land in, so
            // it was accepted and then panicked at run time — and one of the
            // messages printed a compiler-internal `Span { start: .., end: .. }`
            // into a user's terminal. `[T-VAR-MUTABLE]` says a mutating method
            // needs a `var`; a temporary is not even a place.
            self.error_help(
                recv.span(),
                format!("cannot use `{}` on a temporary", name.name),
                "a mutating method changes a value in place, and there is nowhere for this one to land — bind it with `var` first",
            );
        }
    }

    /// The left operand's type, except that an integer *literal* on the left
    /// adopts a float on the right — so `3 * f` and `f * 3` agree, and so do
    /// `3 < f` and `f > 3`. Comparisons used to skip this, which made
    /// `5.0 == 5` legal and `5 == 5.0` an error.
    ///
    /// A non-literal `i64` still does not convert ([T-INT-TO-FLOAT]); that is
    /// the rule, not an oversight, and the mismatch says how to cross it.
    /// `a.cmp(b)` — the same total order sorting uses, as a number.
    ///
    /// [L-LIB-SORT-NAN] specifies an order that `sort` uses internally and
    /// nothing could call, so a program needing it reimplemented it with `<`
    /// and `>` — and got NaN wrong, which is the exact hazard the clause
    /// warns about.
    fn check_cmp(&mut self, recv: &Expr, args: &[Expr], span: Span) -> Option<Type> {
        let t = self.infer_expr(recv);
        if args.len() != 1 {
            self.error(span, "`cmp` takes exactly 1 argument".to_string());
            return Some(Type::I64);
        }
        self.check_expr(&args[0], &t);
        Some(Type::I64)
    }

    /// Whether values of this type have an order.
    ///
    /// `i64`, `f64` and `str` do. A tuple of them does too, compared
    /// component by component — which is how a sort gets more than one key.
    /// Nothing else does: a struct has no order of its own, and inventing one
    /// would make `sort` quietly pick an arbitrary answer.
    fn is_ordered(&mut self, t: &Type) -> bool {
        match self.uni.shallow_resolve(t) {
            Type::I64 | Type::F64 | Type::Str | Type::Var(_) | Type::Error => true,
            Type::Tuple(parts) => !parts.is_empty() && parts.iter().all(|p| self.is_ordered(p)),
            _ => false,
        }
    }

    fn float_aware_lhs(&mut self, lhs: &Expr, rhs: &Expr) -> Type {
        let lhs_ty = self.infer_expr(lhs);
        if matches!(lhs, Expr::Int(..)) && self.uni.shallow_resolve(&lhs_ty) == Type::I64 {
            let probe = self.infer_expr(rhs);
            if self.uni.shallow_resolve(&probe) == Type::F64 {
                self.float_literals.insert(lhs.span().start);
                return Type::F64;
            }
        }
        lhs_ty
    }

    fn infer_binary(&mut self, op: BinaryOp, lhs: &Expr, rhs: &Expr, span: Span) -> Type {
        use BinaryOp::*;
        match op {
            And | Or => {
                self.check_expr(lhs, &Type::Bool);
                self.check_expr(rhs, &Type::Bool);
                Type::Bool
            }
            Eq | NotEq => {
                let t = self.float_aware_lhs(lhs, rhs);
                self.check_expr(rhs, &t);
                Type::Bool
            }
            Lt | LtEq | Gt | GtEq => {
                let t = self.float_aware_lhs(lhs, rhs);
                self.check_expr(rhs, &t);
                let r = self.uni.shallow_resolve(&t);
                if let Type::Var(_) = r {
                    self.uni.unify(&t, &Type::I64);
                } else if !self.is_ordered(&r) {
                    let shown = self.show(&t);
                    self.error(span, format!("cannot compare `{shown}` values with `<`/`>`"));
                }
                Type::Bool
            }
            Coalesce => {
                let lhs_ty = self.infer_expr(lhs);
                match self.uni.shallow_resolve(&lhs_ty) {
                    Type::Named { id, args } if id == self.defs.option_id => {
                        let inner = args[0].clone();
                        self.check_expr(rhs, &inner);
                        inner
                    }
                    // `T ! E` takes the same fallback. Absence and failure are
                    // different things, but "give me the value or this
                    // default" is one thing, and writing it as a four-line
                    // `match` at every call site was the most repetitive
                    // shape in the first real program anyone wrote.
                    Type::Fallible(ok, _) => {
                        let inner = (*ok).clone();
                        self.check_expr(rhs, &inner);
                        inner
                    }
                    Type::Error => Type::Error,
                    _ => {
                        let shown = self.show(&lhs_ty);
                        self.error_help(
                            lhs.span(),
                            format!("`??` needs an `Option` or a fallible value on the left, found `{shown}`"),
                            "`a ?? b` unwraps `a` or falls back to `b`",
                        )
                    }
                }
            }
            Add | Sub | Mul | Div | Rem | Pow => {
                let lhs_ty = self.float_aware_lhs(lhs, rhs);
                self.check_expr(rhs, &lhs_ty);
                let r = self.uni.shallow_resolve(&lhs_ty);
                match r {
                    Type::I64 | Type::F64 => r,
                    Type::Str if op == Add => Type::Str,
                    // Lists concatenate. `+` already joins strings, and a
                    // list is the other thing programs build by appending.
                    Type::List(_) if op == Add => r,
                    Type::Var(_) => {
                        // Un-annotated (e.g. from an empty list) — default int.
                        self.uni.unify(&lhs_ty, &Type::I64);
                        Type::I64
                    }
                    Type::Error => Type::Error,
                    _ => {
                        let shown = self.show(&lhs_ty);
                        let opname = match op {
                            Add => "+",
                            Sub => "-",
                            Mul => "*",
                            Div => "/",
                            Rem => "%",
                            Pow => "^",
                            _ => unreachable!(),
                        };
                        self.error(span, format!("cannot apply `{opname}` to `{shown}`"))
                    }
                }
            }
        }
    }

    fn infer_try(&mut self, inner: &Expr, span: Span) -> Type {
        // `try` is a conditional `return` of an `Err`, and the error value it
        // carries out is usually built right where it failed. `return` and
        // `fail` were both refused inside a region and this, which is the
        // same departure spelled differently, was not ([T-REGION]).
        if self.region_depth > 0 {
            self.error_help(
                span,
                "cannot `try` from inside a region",
                "`try` leaves the region carrying an error value, exactly as `return` and `fail` do — take the fallible call out of the region, or handle it there with `match` and yield a number",
            );
        }
        let t = self.infer_expr(inner);
        match self.uni.shallow_resolve(&t) {
            Type::Fallible(ok, err) => {
                match self.fn_err.clone() {
                    Some(fn_err) => {
                        if !self.err_fits(&err, &fn_err) {
                            let e = self.show(&err);
                            let fe = self.show(&fn_err);
                            self.error_help(
                                span,
                                format!("`try` propagates `{e}` but this function only declares `{fe}`"),
                                "add the error type to the function's `!` clause",
                            );
                        }
                    }
                    None => {
                        self.error_help(
                            span,
                            "`try` used in a function that declares no error type",
                            "add `! ErrorType` to the function signature, or handle with `match`",
                        );
                    }
                }
                *ok
            }
            Type::Error => Type::Error,
            other => {
                let shown = TypeDisplay {
                    ty: &other,
                    defs: &self.defs,
                }
                .to_string();
                self.error_help(
                    span,
                    format!("`try` applied to a value that cannot fail (`{shown}`)"),
                    "remove the `try`",
                )
            }
        }
    }

    /// Is error type `e` acceptable where `declared` is the fn's error clause?
    fn err_fits(&mut self, e: &Type, declared: &Type) -> bool {
        if self.err_open {
            return true;
        }
        let e = self.uni.resolve(e);
        let d = self.uni.resolve(declared);
        if let Type::Var(_) = d {
            // Open error slot (main/tests): bind it and accept.
            let _ = self.uni.unify(&e, &d);
            return true;
        }
        let members = |t: &Type| -> Vec<Type> {
            match t {
                Type::Union(ms) => ms.clone(),
                other => vec![other.clone()],
            }
        };
        let dm = members(&d);
        members(&e)
            .iter()
            .all(|em| dm.iter().any(|dmem| dmem == em) || matches!(em, Type::Error))
    }

    // ------------------------------------------------------------------
    // Match & patterns
    // ------------------------------------------------------------------

    fn infer_match(&mut self, scrutinee: &Expr, arms: &[Arm], span: Span) -> Type {
        self.infer_match_with(scrutinee, arms, span, None)
    }

    fn infer_match_with(
        &mut self,
        scrutinee: &Expr,
        arms: &[Arm],
        span: Span,
        want: Option<&Type>,
    ) -> Type {
        let scrut_ty = self.infer_expr(scrutinee);
        // Matching through a reference matches what it points at. Without
        // this a `ref` to a sum type could never be opened — `r.v` said "sum
        // types are accessed via match" and the match said "pattern cannot
        // match a `ref Expr`", a closed loop — which excluded exactly the
        // recursive shapes an AST or a graph node is made of.
        let scrut_ty = match self.uni.resolve(&scrut_ty) {
            Type::Ref(inner) => *inner,
            _ => scrut_ty,
        };
        // Seeded from the outside when the caller knows what it wants, so
        // every arm adopts it rather than the first arm deciding for all of
        // them: `let x: f64 = match n { 1 -> 2  _ -> 3.5 }` reported that the
        // second arm produced `f64` where `i64` was expected, having settled
        // the question on the first.
        let result = match want {
            Some(w) => w.clone(),
            None => self.uni.fresh(),
        };
        let mut covered: Vec<String> = Vec::new();
        let mut has_irrefutable = false;
        let before = self.moved_snapshot();
        let mut arm_states: Vec<Vec<Vec<(String, bool)>>> = Vec::new();
        for arm in arms {
            // Each arm starts from the state before the match: at most one of
            // them runs, so a move in an earlier arm has not happened here.
            self.restore_moved(&before);
            self.scopes.push(HashMap::new());
            let tag = self.check_pattern(&arm.pattern, &scrut_ty);
            if let Some(guard) = &arm.guard {
                self.check_expr(guard, &Type::Bool);
            } else {
                match tag {
                    None => has_irrefutable = true,
                    // An or-pattern reports every alternative it covers, so
                    // `Circle | Tri` counts as both rather than as one tag
                    // spelled with a bar in it.
                    Some(t) => covered.extend(t.split('\u{1}').map(str::to_string)),
                }
            }
            self.check_expr(&arm.body, &result);
            self.scopes.pop();
            arm_states.push(self.moved_snapshot());
        }
        self.restore_moved(&before);
        self.merge_moved(&arm_states);
        // Exhaustiveness.
        if !has_irrefutable {
            let missing = self.missing_for(&scrut_ty, &covered);
            let missing: Vec<String> = missing.into_iter().filter(|m| !covered.contains(m)).collect();
            if !missing.is_empty() {
                let list = missing
                    .iter()
                    .map(|m| format!("`{m}`"))
                    .collect::<Vec<_>>()
                    .join(", ");
                self.error_help(
                    span,
                    format!("match is not exhaustive — missing {list}"),
                    "add the missing arm(s) or a final `_ ->` catch-all",
                );
            }
        }
        result
    }

    /// Which constructors of `ty` no arm covers, given the tags collected.
    ///
    /// Split out of the match checker so the payload-coverage helpers can ask
    /// the same question about a *nested* type: `Wrap(A)` and `Wrap(B)` cover
    /// `Wrap` exactly when `A` and `B` cover `Inner`, which is the same
    /// question one level down.
    fn missing_for(&mut self, ty: &Type, covered: &[String]) -> Vec<String> {
        let missing: Vec<String> = match self.uni.resolve(ty) {
            Type::Named { id, args } => match &self.defs.get(id).kind.clone() {
                TypeDefKind::Sum { variants } => variants
                    .iter()
                    .filter(|v| !self.variant_is_covered(v, &args, covered))
                    .map(|v| v.name.clone())
                    .collect(),
                TypeDefKind::Struct { .. } => vec!["_".into()],
            },
            Type::Bool => ["true", "false"]
                .iter()
                .map(|s| s.to_string())
                .filter(|n| !covered.contains(n))
                .collect(),
            Type::Fallible(ok, err) => ["Ok", "Err"]
                .iter()
                .map(|s| s.to_string())
                .filter(|n| {
                    let inner = if n == "Ok" { &ok } else { &err };
                    !self.tag_is_covered(n, inner, covered)
                })
                .collect(),
            // All-bool tuples: check the full cartesian product of
            // combinations (up to 2^8) against the covered combo tags.
            Type::Tuple(members)
                if !members.is_empty()
                    && members.len() <= 8
                    && members
                        .iter()
                        .all(|m| self.uni.resolve(m) == Type::Bool) =>
            {
                let n = members.len();
                let mut missing = Vec::new();
                for bits in 0..(1u32 << n) {
                    let combo: Vec<&str> = (0..n)
                        .map(|i| if bits & (1 << i) != 0 { "true" } else { "false" })
                        .collect();
                    let is_covered = covered.iter().any(|tag| {
                        tag.strip_prefix('(')
                            .and_then(|t| t.strip_suffix(')'))
                            .map(|inner| {
                                let parts: Vec<&str> =
                                    inner.split(", ").collect();
                                parts.len() == n
                                    && parts.iter().zip(&combo).all(
                                        |(p, want)| *p == "_" || p == want,
                                    )
                            })
                            .unwrap_or(false)
                    });
                    if !is_covered {
                        missing.push(format!("({})", combo.join(", ")));
                    }
                }
                missing
            }
            Type::Error => vec![],
            _ => vec!["_".into()], // ints, strings, … need a catch-all
        };
        missing.into_iter().filter(|m| !covered.contains(m)).collect()
    }

    /// Is every value of this variant matched by some arm?
    ///
    /// Either an arm matched it with an irrefutable payload — the tag is the
    /// bare name — or the arms that matched it with *refutable* payloads
    /// between them cover the payload type. A single-field variant is the case
    /// worth handling: `Wrap(A)` and `Wrap(B)` together do cover `Wrap`, and
    /// refusing that would be a conservative check people would have to work
    /// around with a `_` that hides the next added variant.
    fn variant_is_covered(
        &mut self,
        v: &crate::types::Variant,
        targs: &[Type],
        covered: &[String],
    ) -> bool {
        if covered.contains(&v.name) {
            return true;
        }
        let [(_, fty)] = v.fields.as_slice() else {
            return false;
        };
        let want = substitute_generics(fty, targs);
        self.tag_is_covered(&v.name, &want, covered)
    }

    /// The same question for a one-payload constructor named `tag`.
    fn tag_is_covered(&mut self, tag: &str, payload: &Type, covered: &[String]) -> bool {
        if covered.contains(&tag.to_string()) {
            return true;
        }
        let prefix = format!("{tag}{PAYLOAD_SEP}");
        let subs: Vec<String> = covered
            .iter()
            .filter_map(|c| c.strip_prefix(&prefix))
            .map(str::to_string)
            .collect();
        // `?` is the multi-field case, which is not tracked precisely.
        if subs.is_empty() || subs.iter().any(|s| s == "?") {
            return false;
        }
        self.missing_for(payload, &subs).is_empty()
    }

    /// Checks a pattern against the expected type, binding names.
    /// Returns the covered constructor tag, or None if irrefutable.
    fn check_pattern(&mut self, pat: &Pattern, expected: &Type) -> Option<String> {
        match pat {
            // `A | B`. Every alternative is checked against the same type.
            // None may bind, so there is no question of which bindings the arm
            // body can see.
            Pattern::Or(alts, _) => {
                let mut tags: Vec<String> = Vec::new();
                let mut irrefutable = false;
                for a in alts {
                    // Whether an alternative binds is only knowable after
                    // checking it: a bare name is a binding unless it resolves
                    // to a nullary variant, and `A | B` over two of those is
                    // exactly the case people want. So watch the scope rather
                    // than guess from the shape.
                    let before = self.scopes.last().map(|sc| sc.len()).unwrap_or(0);
                    match self.check_pattern(a, expected) {
                        Some(t) => tags.push(t),
                        None => irrefutable = true,
                    }
                    let after = self.scopes.last().map(|sc| sc.len()).unwrap_or(0);
                    if after > before {
                        self.error_help(
                            a.span(),
                            "an alternative in `a | b` cannot bind a name",
                            "give each alternative its own arm, or match the shape without binding",
                        );
                    }
                }
                if irrefutable {
                    None
                } else {
                    // The arm covers every alternative, so report them all.
                    Some(tags.join("\u{1}"))
                }
            }
            Pattern::Wildcard(_) => None,
            Pattern::Int(_, span) => {
                self.unify_or_report(*span, &Type::I64, expected);
                Some("<int>".into())
            }
            Pattern::Bool(v, span) => {
                self.unify_or_report(*span, &Type::Bool, expected);
                Some(if *v { "true".into() } else { "false".into() })
            }
            Pattern::Str(_, span) => {
                self.unify_or_report(*span, &Type::Str, expected);
                Some("<str>".into())
            }
            Pattern::Bind(id) => {
                // A nullary variant of the expected sum type?
                if let Type::Named { id: def_id, args } = self.uni.resolve(expected) {
                    if let TypeDefKind::Sum { variants } = &self.defs.get(def_id).kind {
                        if let Some(vi) = variants.iter().position(|v| v.name == id.name) {
                            let v = &variants[vi];
                            if !v.fields.is_empty() {
                                let n = v.fields.len();
                                let vname = v.name.clone();
                                self.error(
                                    id.span,
                                    format!("variant `{vname}` has {n} field(s) — bind them: `{vname}(..)`"),
                                );
                            }
                            let _ = args;
                            return Some(id.name.clone());
                        }
                    }
                }
                self.bind(id, expected.clone(), false);
                None
            }
            Pattern::Variant { path, fields, span } => {
                let vname = &path.segments.last().unwrap().name;
                // Ok/Err on fallible scrutinee.
                if let Type::Fallible(ok, err) = self.uni.resolve(expected) {
                    match vname.as_str() {
                        "Ok" => {
                            let sub = if fields.len() == 1 {
                                self.check_pattern(&fields[0], &ok)
                            } else {
                                self.error(*span, "`Ok` pattern takes exactly 1 binding");
                                None
                            };
                            return Some(tag_with_payload("Ok", &[sub]));
                        }
                        "Err" => {
                            let sub = if fields.len() == 1 {
                                self.check_pattern(&fields[0], &err)
                            } else {
                                self.error(*span, "`Err` pattern takes exactly 1 binding");
                                None
                            };
                            return Some(tag_with_payload("Err", &[sub]));
                        }
                        _ => {
                            self.error_help(
                                *span,
                                "matching a fallible value — use `Ok(..)` and `Err(..)` arms",
                                "e.g. `Ok(v) -> ...` / `Err(e) -> ...`",
                            );
                            return Some(vname.clone());
                        }
                    }
                }
                // Sum type variant.
                let expected_res = self.uni.resolve(expected);
                let (def_id, targs) = match &expected_res {
                    Type::Named { id, args } => (*id, args.clone()),
                    Type::Error => return Some(vname.clone()),
                    other => {
                        let shown = TypeDisplay {
                            ty: other,
                            defs: &self.defs,
                        }
                        .to_string();
                        self.error(
                            *span,
                            format!("pattern `{vname}(..)` cannot match a `{shown}`"),
                        );
                        return Some(vname.clone());
                    }
                };
                // Qualified `Type.Variant` must name the right type.
                if path.segments.len() == 2 {
                    let tname = &path.segments[0].name;
                    if tname != &self.defs.get(def_id).name {
                        self.error(
                            path.span,
                            format!(
                                "pattern names `{tname}` but the value is `{}`",
                                self.defs.get(def_id).name
                            ),
                        );
                    }
                }
                let TypeDefKind::Sum { variants } = self.defs.get(def_id).kind.clone() else {
                    self.error(*span, "struct values are matched with bindings, not variants");
                    return Some(vname.clone());
                };
                match variants.iter().find(|v| &v.name == vname) {
                    Some(v) => {
                        if fields.len() != v.fields.len() {
                            self.error(
                                *span,
                                format!(
                                    "variant `{vname}` has {} field(s), pattern binds {}",
                                    v.fields.len(),
                                    fields.len()
                                ),
                            );
                        }
                        let mut subs = Vec::new();
                        for (fpat, (_, fty)) in fields.iter().zip(&v.fields) {
                            let want = substitute_generics(fty, &targs);
                            subs.push(self.check_pattern(fpat, &want));
                        }
                        // The tag records whether the *payload* was refutable.
                        // Reporting a bare `Wrap` here made `Wrap(A)` cover
                        // `Wrap(_)`, so `match o { Wrap(A) -> .. Plain -> .. }`
                        // typechecked and then panicked on `Wrap(B)` — the
                        // runtime surprise [T-MATCH-EXHAUSTIVE] exists to
                        // prevent, and the one the tutorial advertises against.
                        return Some(tag_with_payload(vname, &subs));
                    }
                    None => {
                        let tname = self.defs.get(def_id).name.clone();
                        self.error(*span, format!("`{tname}` has no variant `{vname}`"));
                    }
                }
                Some(vname.clone())
            }
            Pattern::Tuple(items, span) => {
                let mut sub_tags: Vec<Option<String>> = Vec::new();
                match self.uni.resolve(expected) {
                    Type::Tuple(member_tys) => {
                        if items.len() != member_tys.len() {
                            self.error(
                                *span,
                                format!(
                                    "tuple pattern has {} element(s) but the value has {}",
                                    items.len(),
                                    member_tys.len()
                                ),
                            );
                        }
                        for (p, t) in items.iter().zip(&member_tys) {
                            sub_tags.push(self.check_pattern(p, t));
                        }
                    }
                    Type::Error => {}
                    other => {
                        let shown = TypeDisplay {
                            ty: &other,
                            defs: &self.defs,
                        }
                        .to_string();
                        self.error(*span, format!("tuple pattern cannot match `{shown}`"));
                    }
                }
                // Irrefutable only if every sub-pattern is; otherwise it covers
                // one combination and the match needs a catch-all.
                if sub_tags.iter().all(|t| t.is_none()) {
                    None
                } else {
                    let combo: Vec<String> = sub_tags
                        .into_iter()
                        .map(|t| t.unwrap_or_else(|| "_".into()))
                        .collect();
                    Some(format!("({})", combo.join(", ")))
                }
            }
        }
    }

    fn infer_closure(&mut self, e: &Expr, expected: Option<&Type>) -> Type {
        let Expr::Closure { params, body, span } = e else {
            unreachable!()
        };
        let param_tys: Vec<Type> = match expected {
            Some(Type::Fn {
                params: want_params, ..
            }) if want_params.len() == params.len() => want_params.clone(),
            _ => params.iter().map(|_| self.uni.fresh()).collect(),
        };
        if let Some(Type::Fn {
            params: want_params, ..
        }) = expected
        {
            if want_params.len() != params.len() {
                self.error(
                    *span,
                    format!(
                        "closure takes {} parameter(s) but {} are expected",
                        params.len(),
                        want_params.len()
                    ),
                );
            }
        }
        // Nothing here says what the parameters are, so the body waits for
        // whatever does. Zero-parameter closures have nothing to wait for.
        if expected.is_none() && !params.is_empty() {
            let ret = self.uni.fresh();
            if let Some(slot) = self.deferred_closures.last_mut() {
                slot.push(DeferredClosure {
                    params: params.clone(),
                    body: (**body).clone(),
                    param_tys: param_tys.clone(),
                    ret: ret.clone(),
                });
                return Type::Fn {
                    params: param_tys,
                    ret: Box::new(ret),
                    row: None,
                };
            }
        }
        self.scopes.push(HashMap::new());
        self.closure_floors.push(self.scopes.len() - 1);
        for (p, t) in params.iter().zip(&param_tys) {
            self.bind(p, t.clone(), false);
        }
        // A closure filling a fallible slot — a handler for an operation that
        // declares `! E` — is checked the way a fallible function body is:
        // producing a plain `T` is producing `Ok(T)`, and `fail` inside it
        // raises `E`. Without this the operation's error type would be
        // declarable and no handler could satisfy it.
        let want_ret = match expected {
            Some(Type::Fn { ret, .. }) => Some(self.uni.resolve(ret)),
            _ => None,
        };
        let saved_err = self.fn_err.clone();
        let ret = if let Some(Type::Fallible(ok, err)) = &want_ret {
            self.fn_err = Some((**err).clone());
            let produced = self.infer_expr(body);
            // Either the body already yields the fallible type (it ends in a
            // `fail`, or in something that propagates), or it yields the ok
            // half and is wrapped.
            if self.uni.unify(&produced, &want_ret.clone().unwrap()) {
                want_ret.clone().unwrap()
            } else {
                self.check_expr(body, ok);
                want_ret.clone().unwrap()
            }
        } else {
            self.infer_expr(body)
        };
        self.fn_err = saved_err;
        self.closure_floors.pop();
        self.scopes.pop();
        Type::Fn {
            params: param_tys,
            ret: Box::new(ret),
            row: None,
        }
    }
}

/// Root identifier of a place expression (`x`, `x.f`, `x[i].g`), if any.
/// Types a property test's generator can produce values of.
impl Checker {
    fn generatable(&self, ty: &Type) -> bool {
        match ty {
            Type::I64 | Type::F64 | Type::Bool | Type::Str => true,
            Type::List(e) => self.generatable(e),
            Type::Tuple(items) => items.iter().all(|t| self.generatable(t)),
            Type::Named { id, args } if *id == self.defs.option_id => {
                args.iter().all(|t| self.generatable(t))
            }
            _ => false,
        }
    }
}

fn place_root(e: &Expr) -> Option<&Ident> {
    match e {
        Expr::Ident(id) => Some(id),
        Expr::Field { recv, .. } | Expr::Index { recv, .. } => place_root(recv),
        _ => None,
    }
}

/// Separates a constructor from a refutable payload tag.
const PAYLOAD_SEP: char = '\u{2}';

/// A constructor tag that records whether its payload patterns were refutable.
///
/// `Wrap` means "this arm matches every `Wrap`". `Wrap` plus a payload tag
/// means "this arm matches only the `Wrap`s whose payload is that" — not the
/// same thing, and treating it as the same is how `Wrap(A)` came to cover
/// `Wrap(_)`.
fn tag_with_payload(name: &str, subs: &[Option<String>]) -> String {
    if subs.iter().all(Option::is_none) {
        return name.to_string();
    }
    if let [Some(one)] = subs {
        // One field: the sub-tag is kept, so several arms over the same
        // variant can be shown together to cover it.
        return format!("{name}{PAYLOAD_SEP}{one}");
    }
    format!("{name}{PAYLOAD_SEP}?")
}


fn stmt_span(s: &Stmt) -> Span {
    match s {
        Stmt::Let { span, .. }
        | Stmt::LetTuple { span, .. }
        | Stmt::Assign { span, .. }
        | Stmt::Return(_, span)
        | Stmt::Break(span)
        | Stmt::Continue(span)
        | Stmt::While { span, .. }
        | Stmt::For { span, .. }
        | Stmt::With { span, .. }
        | Stmt::Parallel { span, .. }
        | Stmt::Spawn { span, .. } => *span,
        Stmt::Expr(e) => e.span(),
    }
}
