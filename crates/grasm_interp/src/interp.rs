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
    /// `exit(n)` — unwinds to the entry point rather than killing the host
    /// process, so embedders (the test harness, `grasm test`, tooling) stay
    /// in control of their own exit.
    Exit(i32),
}

type EResult = Result<Value, Flow>;

/// Called before every statement, so a debugger can stop, inspect, and step.
///
/// The interpreter hands itself back to the hook, which is why the hook is
/// taken out of the interpreter for the duration of the call: a debugger needs
/// to read the very state it is suspending.
pub trait DebugHook<W: Write> {
    fn before_stmt(&mut self, interp: &mut Interp<'_, W>, at: grasm_syntax::span::Span);
}

/// One entry of a call stack, captured when a panic first escapes.
#[derive(Debug, Clone)]
pub struct Frame {
    /// Qualified `module::function`.
    pub function: String,
    /// Where execution was inside this frame — the statement being run for the
    /// innermost frame, the call site for every frame above it.
    pub at: grasm_syntax::span::Span,
}

pub struct Interp<'a, W: Write> {
    /// qualified `mod::fn` → decl
    fns: HashMap<String, &'a FnDecl>,
    current_module: String,
    /// Module-level constants by `module::NAME`. Effect-free, so each use
    /// evaluates the initializer again — the same thing the native backend
    /// does, rather than two different notions of when a constant happens.
    consts: HashMap<String, &'a grasm_syntax::ast::ConstDecl>,
    /// Integer literals the checker resolved to `f64`, by span start.
    float_literals: std::collections::HashSet<u32>,
    /// `sum()` calls the checker resolved to `f64`, by span start.
    float_sums: std::collections::HashSet<u32>,
    /// Tier 2's shared heap: the value in each slot (`None` once released) and
    /// the generation that slot is on. A reference matches only while the
    /// generation it was taken at is still current, so a released slot can be
    /// reused immediately and every reference into its previous life is caught.
    slab: Vec<(Option<Value>, u32)>,
    /// Slots free for reuse.
    slab_free: Vec<u32>,
    /// `type Res = i64 ! E` — needed to tell whether a function that returns
    /// an alias is fallible, which decides whether its value is wrapped.
    aliases: HashMap<String, TypeExpr>,
    /// C functions this program declares, by `module::name`.
    externs: HashMap<String, &'a grasm_syntax::ast::ExternDecl>,
    /// `Effect.op` for every operation declaring `! E`.
    ///
    /// A handler is an ordinary closure, so calling one produced a bare value
    /// and a `fail` inside it propagated straight past the caller. The
    /// operation's declared type is what says it should have been a `T ! E`.
    fallible_ops: std::collections::HashSet<String>,
    /// module → (alias → target module)
    imports: HashMap<String, HashMap<String, String>>,
    /// sum type name → variant names (arity implied by decl).
    sums: HashMap<String, Vec<(String, usize)>>,
    /// variant name → (owning type, arity); only unambiguous entries.
    variants: HashMap<String, Option<(String, usize)>>,
    /// Span offsets of statements that executed, when coverage is on.
    /// `None` keeps the hot path free of the bookkeeping entirely.
    pub(crate) coverage: Option<std::collections::BTreeSet<u32>>,
    pub(crate) scopes: Vec<HashMap<String, Value>>,
    /// Declared effect names (for op-call dispatch).
    effect_names: std::collections::HashSet<String>,
    /// Innermost-last handler stack: (effect name, op name → closure).
    handlers: Vec<(String, HashMap<String, Value>)>,
    pub out: &'a mut W,
    /// A panic or an unhandled error from `main`, if one ended the run.
    ///
    /// These are not the program's output. Printing them to stdout put them
    /// in the middle of whatever a pipeline was reading, so the interpreter
    /// records them and the caller decides — every caller sends them to
    /// stderr ([E-PANIC-EXIT]).
    pub fault: Option<String>,
    /// Recursion guard.
    depth: u32,
    /// Active calls, innermost last: (qualified name, call site).
    call_stack: Vec<(String, grasm_syntax::span::Span)>,
    /// Where the statement currently executing begins.
    cur_span: grasm_syntax::span::Span,
    /// Frames captured the first time a panic escaped a statement. Later
    /// panics do not overwrite it: the first one is the one that explains
    /// what went wrong.
    panic_trace: Option<Vec<Frame>>,
    /// Debugger, if one is attached.
    debug: Option<Box<dyn DebugHook<W>>>,
    /// `inout` copy-out values from the last `call_decl` (param index → value).
    pending_writebacks: Vec<(usize, Value)>,
    /// Program arguments exposed via the `args()` builtin.
    pub program_args: Vec<String>,
    /// Stack of `parallel` scopes, each holding the task bodies spawned in it.
    /// Bodies are cloned: spawns are rare and this keeps the evaluator free of
    /// AST lifetimes (closures own their bodies, so borrows can't be uniform).
    task_scopes: Vec<Vec<Block>>,
}

const MAX_DEPTH: u32 = 100_000;

/// One step of an assignment path.
enum Seg {
    Field(String),
    Index(usize),
}

/// Walks `segs` into `root` and stores `v` at the end, copy-on-write.
fn assign_into(root: &mut Value, segs: &[Seg], v: Value) -> Result<(), Flow> {
    let mut slot = root;
    for seg in segs {
        match seg {
            Seg::Field(fname) => match slot {
                Value::Struct { fields, .. } => {
                    let fields = Rc::make_mut(fields);
                    match fields.iter_mut().find(|(k, _)| k == fname) {
                        Some((_, fv)) => slot = fv,
                        None => return Err(Flow::Panic(format!("no field `{fname}`"))),
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
                other => return Err(Flow::Panic(format!("cannot index into `{other}`"))),
            },
        }
    }
    *slot = v;
    Ok(())
}

impl<'a, W: Write> Interp<'a, W> {
    pub fn new(module: &'a Module, out: &'a mut W) -> Self {
        let mut me = Self::raw_new(out);
        me.register_module("main", module);
        // Asked for here rather than by each caller. Which integer literals
        // are `f64` is a fact only the checker has ([T-INT-TO-FLOAT-CARRIED]),
        // and a caller that forgets to pass it gets silently wrong arithmetic
        // — so no caller is asked to remember.
        let a = grasm_sema::check::check(module);
        me.float_literals = a.float_literals;
        me.float_sums = a.float_sums;
        me
    }

    /// files: (module name, module, alias→target imports); files[0] is entry.
    pub fn new_program(
        files: &'a [(String, &'a Module, HashMap<String, String>)],
        out: &'a mut W,
    ) -> Self {
        Self::build(files, out)
    }

    /// The integer literals the checker resolved to `f64`, from `Analysis`.
    ///
    /// Every path that runs a program checks it first, so this is always
    /// available; a caller that skips it gets the old behaviour, which is
    /// wrong but not worse than it was.
    pub fn set_float_literals(&mut self, spans: std::collections::HashSet<u32>) {
        self.float_literals = spans;
    }

    /// The `sum()` calls the checker resolved to `f64`.
    pub fn set_float_sums(&mut self, spans: std::collections::HashSet<u32>) {
        self.float_sums = spans;
    }

    fn build(
        files: &'a [(String, &'a Module, HashMap<String, String>)],
        out: &'a mut W,
    ) -> Self {
        let mut imports_map = HashMap::new();
        for (name, _, imps) in files {
            imports_map.insert(name.clone(), imps.clone());
        }
        let mut me = Self::new_empty(out);
        me.imports = imports_map;
        for (name, module, _) in files {
            me.register_module(name, module);
        }
        me
    }

    fn new_empty(out: &'a mut W) -> Self {
        Self::raw_new(out)
    }

    fn register_module(&mut self, mod_name: &str, module: &'a Module) {
        for decl in &module.decls {
            match decl {
                Decl::Fn(f) => {
                    self.fns
                        .insert(format!("{mod_name}::{}", f.name.name), f);
                }
                Decl::Const(c) => {
                    self.consts
                        .insert(format!("{mod_name}::{}", c.name.name), c);
                }
                Decl::Extern(e) => {
                    self.externs
                        .insert(format!("{mod_name}::{}", e.name.name), e);
                }
                Decl::Type(t) if matches!(t.body, TypeBody::Alias(_)) => {
                    if let TypeBody::Alias(te) = &t.body {
                        self.aliases.insert(t.name.name.clone(), te.clone());
                    }
                }
                Decl::Type(t) => {
                    if let TypeBody::Sum(vs) = &t.body {
                        let list: Vec<(String, usize)> = vs
                            .iter()
                            .map(|v| (v.name.name.clone(), v.fields.len()))
                            .collect();
                        for v in vs {
                            self.variants
                                .entry(v.name.name.clone())
                                .and_modify(|e| *e = None)
                                .or_insert(Some((t.name.name.clone(), v.fields.len())));
                        }
                        self.sums.insert(t.name.name.clone(), list);
                    }
                }
                Decl::Effect(e) => {
                    self.effect_names.insert(e.name.name.clone());
                    for op in &e.ops {
                        if op.err.is_some() {
                            self.fallible_ops
                                .insert(format!("{}.{}", e.name.name, op.name.name));
                        }
                    }
                }
                _ => {}
            }
        }
    }

    fn raw_new(out: &'a mut W) -> Self {
        let fns = HashMap::new();
        let mut sums: HashMap<String, Vec<(String, usize)>> = HashMap::new();
        let mut variants: HashMap<String, Option<(String, usize)>> = HashMap::new();
        // Prelude Option + IoError.
        sums.insert(
            "Option".into(),
            vec![("Some".into(), 1), ("None".into(), 0)],
        );
        variants.insert("Some".into(), Some(("Option".into(), 1)));
        variants.insert("None".into(), Some(("Option".into(), 0)));
        sums.insert(
            "IoError".into(),
            vec![("NotFound".into(), 1), ("IoFailed".into(), 1)],
        );
        variants.insert("NotFound".into(), Some(("IoError".into(), 1)));
        variants.insert("IoFailed".into(), Some(("IoError".into(), 1)));
        let effect_names = std::collections::HashSet::new();
        Interp {
            fns,
            current_module: "main".to_string(),
            consts: HashMap::new(),
            float_literals: std::collections::HashSet::new(),
            float_sums: std::collections::HashSet::new(),
            slab: Vec::new(),
            slab_free: Vec::new(),
            aliases: HashMap::new(),
            externs: HashMap::new(),
            fallible_ops: std::collections::HashSet::new(),
            imports: HashMap::new(),
            sums,
            variants,
            coverage: None,
            scopes: Vec::new(),
            effect_names,
            handlers: Vec::new(),
            out,
            fault: None,
            depth: 0,
            call_stack: Vec::new(),
            cur_span: grasm_syntax::span::Span::new(0, 0),
            panic_trace: None,
            debug: None,
            pending_writebacks: Vec::new(),
            program_args: Vec::new(),
            task_scopes: Vec::new(),
        }
    }

    // ------------------------------------------------------------------
    // Entry points
    // ------------------------------------------------------------------

    /// Runs `main`. Returns process exit code.
    pub fn run_main(&mut self) -> i32 {
        let Some(main) = self.fns.get("main::main").copied() else {
            self.fault = Some("error: no `main` function".to_string());
            return 1;
        };
        match self.call_decl_at(main, "main", vec![], main.span) {
            // A fallible `main` returns Err as a *value*; report it rather
            // than exiting 0 as if nothing happened.
            Ok(Value::Err(e)) => {
                self.fault = Some(format!("error: unhandled error from main: {e}"));
                1
            }
            Ok(_) => 0,
            Err(Flow::Err(e)) => {
                self.fault = Some(format!("error: unhandled error from main: {e}"));
                1
            }
            Err(Flow::Panic(msg)) => {
                self.fault = Some(format!("panic: {msg}"));
                101
            }
            Err(Flow::Exit(code)) => code,
            Err(_) => 1,
        }
    }

    /// Runs subsequent top-level bodies as though they were written in
    /// `module`, so an unqualified call inside a test resolves against the
    /// file the test was written in.
    pub fn set_current_module(&mut self, module: &str) {
        self.current_module = module.to_string();
    }


    /// Whether this function's return type is fallible, following aliases.
    fn returns_fallible(&self, f: &FnDecl) -> bool {
        let mut te = match &f.ret {
            Some(t) => t,
            None => return false,
        };
        // Aliases can chain; a short bound is enough and cannot loop forever.
        for _ in 0..8 {
            match te {
                TypeExpr::Fallible(..) => return true,
                TypeExpr::Name { path, args, .. } if args.is_empty() => {
                    match self.aliases.get(&path.segments[0].name) {
                        Some(next) => te = next,
                        None => return false,
                    }
                }
                _ => return false,
            }
        }
        false
    }

    /// Frames captured when a panic first escaped, innermost first.
    pub fn panic_trace(&self) -> Option<&[Frame]> {
        self.panic_trace.as_deref()
    }

    /// Attaches a debugger. It is consulted before every statement.
    pub fn attach_debugger(&mut self, hook: Box<dyn DebugHook<W>>) {
        self.debug = Some(hook);
    }

    /// The live call stack, outermost first: (qualified name, call site).
    pub fn debug_call_stack(&self) -> &[(String, grasm_syntax::span::Span)] {
        &self.call_stack
    }

    /// Where execution currently is.
    pub fn debug_cur_span(&self) -> grasm_syntax::span::Span {
        self.cur_span
    }

    /// Locals of the innermost frame, innermost scope last.
    pub fn debug_scopes(&self) -> &[HashMap<String, Value>] {
        &self.scopes
    }

    /// Looks a name up the way the running code would.
    pub fn debug_lookup(&self, name: &str) -> Option<&Value> {
        self.scopes.iter().rev().find_map(|s| s.get(name))
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
                    Err(Flow::Exit(code)) => {
                        let _ = writeln!(
                            self.out,
                            "test \"{}\" ... FAILED: exit({code}) inside a test",
                            t.name
                        );
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

    fn call_decl(&mut self, f: &'a FnDecl, module: &str, args: Vec<Value>) -> EResult {
        let site = self.cur_span;
        self.call_decl_at(f, module, args, site)
    }

    fn call_decl_at(
        &mut self,
        f: &'a FnDecl,
        module: &str,
        args: Vec<Value>,
        call_site: grasm_syntax::span::Span,
    ) -> EResult {
        if self.depth >= MAX_DEPTH {
            // Word for word what the compiled backend says. The two engines
            // stop at different depths on purpose — this one counts calls, the
            // other measures remaining stack — and both bounds are
            // implementation-defined ([E-RECURSION-DEPTH]), so naming this
            // one's number here advertised a limit no program should depend on
            // and made the same failure read as two different failures.
            return Err(Flow::Panic("stack overflow: recursion too deep".to_string()));
        }
        self.depth += 1;
        self.call_stack
            .push((format!("{module}::{}", f.name.name), call_site));
        let saved_span = self.cur_span;
        let mut scope = HashMap::new();
        for (p, v) in f.params.iter().zip(args) {
            scope.insert(p.name.name.clone(), v);
        }
        // Fresh frame: callee can't see caller locals.
        let saved = std::mem::take(&mut self.scopes);
        let saved_module = std::mem::replace(&mut self.current_module, module.to_string());
        self.scopes.push(scope);
        let result = self.eval_block(&f.body);
        self.current_module = saved_module;
        // Copy-out for `inout` params (applied by the caller to its places).
        self.pending_writebacks.clear();
        // A function that *fails* still returns, and the writes it made before
        // failing happened. Dropping them on the error path meant
        // `xs.push(v)` before a `fail` was silently undone — in the
        // interpreter only, so the compiled binary and the reference
        // implementation disagreed about what the program had done.
        if result.is_ok() || matches!(result, Err(Flow::Return(_)) | Err(Flow::Err(_))) {
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
        self.call_stack.pop();
        self.cur_span = saved_span;

        // The `! E` may be written on the declaration or hidden inside an
        // alias used as the return type; both make the function fallible.
        let fallible = f.err.is_some() || self.returns_fallible(f);
        // A fallible function's plain value is wrapped on the way out — but a
        // body that already produced a `T ! E`, which is what `Ok(v)` and
        // `Err(e)` build, must not be wrapped twice.
        let wrap = |v: Value| match v {
            already @ (Value::Ok(_) | Value::Err(_)) if fallible => already,
            v if fallible => Value::Ok(Rc::new(v)),
            v => v,
        };
        match result {
            Ok(v) => Ok(wrap(v)),
            Err(Flow::Return(v)) => Ok(wrap(v)),
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
            Value::Closure {
                params,
                body,
                env,
                module,
            } => {
                if self.depth >= MAX_DEPTH {
                    return Err(Flow::Panic("stack overflow in closure call".into()));
                }
                self.depth += 1;
                // Names in the body mean what they meant where it was written.
                let saved_module = std::mem::replace(&mut self.current_module, (*module).clone());
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
                self.current_module = saved_module;
                self.depth -= 1;
                match r {
                    Err(Flow::Return(v)) => Ok(v),
                    other => other,
                }
            }
            Value::FnRef(name) => {
                if let Some(e) = self.externs.get(name.as_str()).copied() {
                    return self.call_extern_values(e, args);
                }
                let Some(f) = self.fns.get(name.as_str()).copied() else {
                    return Err(Flow::Panic(format!("unknown function `{name}`")));
                };
                let module = name
                    .rsplit_once("::")
                    .map(|(m, _)| m.to_string())
                    .unwrap_or_else(|| "main".to_string());
                self.call_decl(f, &module, args)
            }
            other => Err(Flow::Panic(format!(
                "`{other}` is not callable ({span_hint})"
            ))),
        }
    }

    // ------------------------------------------------------------------
    // Blocks & statements
    // ------------------------------------------------------------------

    pub(crate) fn eval_block(&mut self, b: &Block) -> EResult {
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
        if let Some(seen) = &mut self.coverage {
            seen.insert(crate::testing::stmt_start(s));
        }
        self.cur_span = crate::testing::stmt_span(s);
        if self.debug.is_some() {
            // Take the hook so it can borrow the interpreter it is inspecting,
            // then put it back.
            let mut hook = self.debug.take().expect("checked just above");
            let at = self.cur_span;
            hook.before_stmt(self, at);
            self.debug = Some(hook);
        }
        let result = self.eval_stmt_inner(s);
        if matches!(result, Err(Flow::Panic(_))) && self.panic_trace.is_none() {
            self.panic_trace = Some(self.capture_trace());
        }
        result
    }

    /// The call stack as frames, innermost first. Each frame reports where
    /// execution was *inside* it, so the innermost frame points at the failing
    /// statement and the rest point at their call sites.
    fn capture_trace(&self) -> Vec<Frame> {
        let mut frames = Vec::new();
        let mut at = self.cur_span;
        for (name, call_site) in self.call_stack.iter().rev() {
            frames.push(Frame {
                function: name.clone(),
                at,
            });
            at = *call_site;
        }
        frames
    }

    fn eval_stmt_inner(&mut self, s: &Stmt) -> EResult {
        match s {
            Stmt::LetTuple { names, init, span, .. } => {
                let v = self.eval_expr(init)?;
                let Value::Tuple(items) = &v else {
                    return Err(Flow::Panic("destructuring a non-tuple".into()));
                };
                if items.len() != names.len() {
                    let _ = span;
                    return Err(Flow::Panic(format!(
                        "this tuple has {} component(s) but {} name(s) were given",
                        items.len(),
                        names.len()
                    )));
                }
                for (n, item) in names.iter().zip(items.iter()) {
                    self.scopes
                        .last_mut()
                        .unwrap()
                        .insert(n.name.clone(), item.clone());
                }
                Ok(Value::Unit)
            }
            Stmt::Let {
                name, ty, init, ..
            } => {
                let mut v = self.eval_expr(init)?;
                // An annotation of `f64` makes an integer literal a float, so
                // the value matches the type the checker assigned it.
                if let (Some(t), Value::Int(i)) = (ty, &v) {
                    if is_float_type(t) {
                        v = Value::Float(*i as f64);
                    }
                }
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
                #[allow(clippy::while_let_loop)]
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
            Stmt::Parallel { body, .. } => {
                // Collect spawned tasks, then run them to completion in spawn
                // order before the scope exits (structured concurrency).
                self.task_scopes.push(Vec::new());
                let collect = self.eval_block(body);
                let tasks = self.task_scopes.pop().unwrap_or_default();
                collect?;
                for task in &tasks {
                    // Each task sees the enclosing scope (shared state is how
                    // tasks communicate) but gets its own frame for locals.
                    self.eval_block(task)?;
                }
                Ok(Value::Unit)
            }
            Stmt::Spawn { body, span } => {
                match self.task_scopes.last_mut() {
                    Some(scope) => {
                        scope.push(body.clone());
                        Ok(Value::Unit)
                    }
                    None => Err(Flow::Panic(format!(
                        "`spawn` outside a `parallel` block (at offset {})",
                        span.start
                    ))),
                }
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
                    // Built from the two forms directly rather than by
                    // turning `a..=b` into `a..(b + 1)`. That addition wraps
                    // when `b` is `i64::MAX`, and the range then ran zero
                    // times — `mx..=mx` iterated over nothing, against
                    // [E-LOOP-RANGE], with no overflow reported anywhere.
                    Value::Range(a, b, true) => (a..=b).map(Value::Int).collect(),
                    Value::Range(a, b, false) => (a..b).map(Value::Int).collect(),
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
                    // A `ref` root: the value lives in the slab, not in the
                    // binding, so it comes out, is written through, and goes
                    // back. Without this a write through a reference — the
                    // whole reason Tier 2 exists — typechecked and then
                    // panicked `cannot access field of ref#0.0`, which left a
                    // `ref` graph write-once and a cycle impossible to close.
                    // Only when there is a path to walk. `at = other` with
                    // no segments rebinds the *variable*; taking the reference
                    // path there overwrote the node `at` pointed at, which
                    // silently replaced a graph node with a reference to
                    // another one. The checker draws the same line.
                    let root_ref = self
                        .scopes
                        .iter()
                        .rev()
                        .filter(|_| !segs.is_empty())
                        .find_map(|sc| sc.get(&id.name))
                        .and_then(|rv| match rv {
                            Value::Ref(i, g) => Some((*i, *g)),
                            _ => None,
                        });
                    if let Some((i, g)) = root_ref {
                        let idx = i as usize;
                        let live = matches!(self.slab.get(idx), Some((Some(_), gen)) if *gen == g);
                        if !live {
                            return Err(Flow::Panic(
                                "stale reference: the value it pointed to was released".to_string(),
                            ));
                        }
                        let mut inner = self.slab[idx].0.take().expect("checked live");
                        let r = assign_into(&mut inner, &segs, v);
                        self.slab[idx].0 = Some(inner);
                        return r;
                    }
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
            // An integer literal the checker placed in an `f64` position is
            // a float here, because [T-INT-TO-FLOAT] says the conversion
            // happens at compile time and this engine has no types of its own
            // to do it with. Keeping it an integer made `xs.push(3)` into a
            // `[f64]` store an `i64`: the list then sorted wrong, compared
            // unequal to the same list written `3.0`, and printed with no
            // point — three wrong answers from the engine the specification
            // calls normative.
            Expr::Int(v, sp) => Ok(if self.float_literals.contains(&sp.start) {
                Value::Float(*v as f64)
            } else {
                Value::Int(*v)
            }),
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
            Expr::Map(entries, _) => {
                let mut m = std::collections::BTreeMap::new();
                for (ke, ve) in entries {
                    let k = self.eval_expr(ke)?;
                    let v = self.eval_expr(ve)?;
                    let Some(key) = crate::value::MapKey::from_value(&k) else {
                        return Err(Flow::Panic(format!("invalid map key `{k}`")));
                    };
                    m.insert(key, v);
                }
                Ok(Value::Map(Rc::new(m)))
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
            Expr::StructLit {
                name, base, fields, ..
            } => {
                // `Point{ ..p, x: 1 }` — start from `p`'s fields in their
                // declared order, then overwrite the ones written here. The
                // base is evaluated first, so `Point{ ..f(), x: g() }` runs
                // `f` before `g` like every other expression ([E-ORDER-LTR]).
                let mut vs: Vec<(String, Value)> = match base {
                    Some(b) => match self.eval_expr(b)? {
                        Value::Struct { fields, .. } => (*fields).clone(),
                        other => {
                            return Err(Flow::Panic(format!(
                                "`..` in a struct literal needs a struct, got `{other}`"
                            )))
                        }
                    },
                    None => Vec::with_capacity(fields.len()),
                };
                for (fname, fexpr) in fields {
                    let v = self.eval_expr(fexpr)?;
                    match vs.iter_mut().find(|(n, _)| n == &fname.name) {
                        Some(slot) => slot.1 = v,
                        None => vs.push((fname.name.clone(), v)),
                    }
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
                            // `Some(v)` and `Ok(v)` both yield their payload;
                            // `None` and `Err(_)` both take the fallback.
                            Value::Variant {
                                variant, fields, ..
                            } if variant.as_str() == "Some" => Ok(fields[0].clone()),
                            Value::Ok(inner) => Ok((*inner).clone()),
                            Value::Err(_) => self.eval_expr(rhs),
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
                // Matching through a reference matches what it points at, and
                // a stale one panics here rather than matching anything.
                let v = self.deref_value(v)?;
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
            Expr::Block(b) | Expr::Unsafe(b) => self.eval_block(b),
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
                    Expr::Block(b) | Expr::Unsafe(b) => ClosureBody::Block(b.clone()),
                    other => ClosureBody::Expr(other.clone()),
                };
                Ok(Value::Closure {
                    params: Rc::new(params.clone()),
                    body: Rc::new(cb),
                    env: Rc::new(env),
                    module: Rc::new(self.current_module.clone()),
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
        let qual = format!("{}::{}", self.current_module, id.name);
        // A C function named without being called. The compiled backend makes
        // one a function value; the interpreter said `unknown name`, so the
        // two engines disagreed about a program that `check` accepted.
        if self.externs.contains_key(&qual) {
            return Ok(Value::FnRef(Rc::new(qual)));
        }
        // A module-level constant: its initializer is effect-free, so
        // evaluating it here is the whole of what the name means.
        if let Some(c) = self.consts.get(&qual).copied() {
            return self.eval_expr(&c.init);
        }
        if self.fns.contains_key(&qual) {
            return Ok(Value::FnRef(Rc::new(qual)));
        }
        Err(Flow::Panic(format!("unknown name `{}`", id.name)))
    }


    /// Calls a declared C function.
    fn call_extern(
        &mut self,
        e: &grasm_syntax::ast::ExternDecl,
        args: &[Expr],
    ) -> EResult {
        let mut vals = Vec::with_capacity(args.len());
        for a in args {
            vals.push(self.eval_expr(a)?);
        }
        self.call_extern_values(e, vals)
    }

    /// The same, with the arguments already evaluated.
    fn call_extern_values(
        &mut self,
        e: &grasm_syntax::ast::ExternDecl,
        args: Vec<Value>,
    ) -> EResult {
        use crate::ffi::{CArg, CKind, CRet};
        let Some(sym) = crate::ffi::find(&e.name.name) else {
            return Err(Flow::Panic(format!(
                "cannot find the C function `{}` in this process — a compiled \
build links it, `grasm run` can only reach what is already loaded",
                e.name.name
            )));
        };
        let mut cargs = Vec::with_capacity(args.len());
        for a in args {
            cargs.push(match a {
                Value::Int(i) => CArg::I64(i),
                Value::Float(f) => CArg::F64(f),
                other => {
                    return Err(Flow::Panic(format!(
                        "`{}` cannot take `{other}` across the C boundary",
                        e.name.name
                    )))
                }
            });
        }
        let ret = match &e.ret {
            None => None,
            Some(grasm_syntax::ast::TypeExpr::Name { path, .. }) => {
                match path.segments[0].name.as_str() {
                    "f64" => Some(CKind::F64),
                    _ => Some(CKind::I64),
                }
            }
            Some(_) => Some(CKind::I64),
        };
        // SAFETY: `sym` came from `find` for this declaration's own name, and
        // the checker restricted the declared parameter and result types to
        // `i64` and `f64` ([T-EXTERN]) — which is exactly what `cargs` and
        // `ret` describe here.
        match unsafe { crate::ffi::call(sym, &cargs, ret) } {
            Some(CRet::I64(v)) => Ok(Value::Int(v)),
            Some(CRet::F64(v)) => Ok(Value::Float(v)),
            Some(CRet::Unit) => Ok(Value::Unit),
            None => Err(Flow::Panic(format!(
                "`{}` has a signature this interpreter cannot call \
(at most three arguments, and no mixed shapes past two)",
                e.name.name
            ))),
        }
    }

    fn eval_call(&mut self, callee: &Expr, args: &[Expr]) -> EResult {
        if let Expr::Ident(id) = callee {
            let is_local = self.scopes.iter().rev().any(|s| s.contains_key(&id.name));
            // A declared C function, looked up in this process rather than
            // linked. The compiled backend calls the same symbol.
            if !is_local {
                let qual = format!("{}::{}", self.current_module, id.name);
                if let Some(e) = self.externs.get(&qual).copied() {
                    return self.call_extern(e, args);
                }
            }
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
                    "read_file" => {
                        let p = self.eval_expr(&args[0])?.to_string();
                        return Ok(match std::fs::read_to_string(&p) {
                            Ok(s) => Value::Ok(Rc::new(Value::str(s))),
                            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                                Value::Err(Rc::new(io_error("NotFound", p)))
                            }
                            Err(e) => Value::Err(Rc::new(io_error("IoFailed", e.to_string()))),
                        });
                    }
                    "write_file" => {
                        let p = self.eval_expr(&args[0])?.to_string();
                        let content = self.eval_expr(&args[1])?.to_string();
                        return Ok(match std::fs::write(&p, content) {
                            Ok(()) => Value::Ok(Rc::new(Value::Unit)),
                            Err(e) => Value::Err(Rc::new(io_error("IoFailed", e.to_string()))),
                        });
                    }
                    "file_exists" => {
                        let p = self.eval_expr(&args[0])?.to_string();
                        return Ok(Value::Bool(std::path::Path::new(&p).exists()));
                    }
                    "args" => {
                        let vs: Vec<Value> =
                            self.program_args.iter().map(|a| Value::str(a.clone())).collect();
                        return Ok(Value::List(Rc::new(vs)));
                    }
                    "Ok" | "Err" => {
                        let v = self.eval_expr(&args[0])?;
                        return Ok(if id.name == "Ok" {
                            Value::Ok(Rc::new(v))
                        } else {
                            Value::Err(Rc::new(v))
                        });
                    }
                    "map" => {
                        return Ok(Value::Map(Rc::new(std::collections::BTreeMap::new())));
                    }
                    // Guarded on the user's own namespace, the way the
                    // checker guards them: `e` is a common enough identifier
                    // that a builtin taking it outright would break working
                    // programs. The older builtins beside them do shadow a
                    // user function of the same name; these do not.
                    "pi" | "e"
                        if !self
                            .fns
                            .contains_key(&format!("{}::{}", self.current_module, id.name)) =>
                    {
                        return Ok(Value::Float(if id.name == "pi" {
                            std::f64::consts::PI
                        } else {
                            std::f64::consts::E
                        }));
                    }
                    "ref" if !self.fns.contains_key(&format!("{}::ref", self.current_module)) => {
                        let v = self.eval_expr(&args[0])?;
                        let r = match self.slab_free.pop() {
                            Some(i) => {
                                let idx = i as usize;
                                self.slab[idx].0 = Some(v);
                                Value::Ref(i, self.slab[idx].1)
                            }
                            None => {
                                let i = self.slab.len() as u32;
                                self.slab.push((Some(v), 0));
                                Value::Ref(i, 0)
                            }
                        };
                        return Ok(r);
                    }
                    "release"
                        if !self
                            .fns
                            .contains_key(&format!("{}::release", self.current_module)) =>
                    {
                        let v = self.eval_expr(&args[0])?;
                        if let Value::Ref(i, g) = v {
                            let idx = i as usize;
                            if idx < self.slab.len() && self.slab[idx].1 == g {
                                self.slab[idx].0 = None;
                                // The generation moves whether or not anything
                                // was there, so every reference taken before
                                // this stops matching.
                                self.slab[idx].1 = self.slab[idx].1.wrapping_add(1);
                                self.slab_free.push(i);
                            }
                        }
                        return Ok(Value::Unit);
                    }
                    "alive"
                        if !self
                            .fns
                            .contains_key(&format!("{}::alive", self.current_module)) =>
                    {
                        let v = self.eval_expr(&args[0])?;
                        return Ok(Value::Bool(match v {
                            Value::Ref(i, g) => {
                                let idx = i as usize;
                                idx < self.slab.len()
                                    && self.slab[idx].1 == g
                                    && self.slab[idx].0.is_some()
                            }
                            _ => false,
                        }));
                    }
                    "now_millis" => {
                        let ms = std::time::SystemTime::now()
                            .duration_since(std::time::UNIX_EPOCH)
                            .map(|d| d.as_millis() as i64)
                            .unwrap_or(0);
                        return Ok(Value::Int(ms));
                    }
                    "env" => {
                        let name = self.eval_expr(&args[0])?.to_string();
                        return Ok(match std::env::var(&name) {
                            Ok(v) => Value::some(Value::str(v)),
                            Err(_) => Value::none(),
                        });
                    }
                    "exit" => {
                        let code = match self.eval_expr(&args[0])? {
                            Value::Int(i) => i,
                            other => {
                                return Err(Flow::Panic(format!(
                                    "exit status must be an int, got `{other}`"
                                )))
                            }
                        };
                        let _ = self.out.flush();
                        return Err(Flow::Exit(code.clamp(0, 255) as i32));
                    }
                    "input" => {
                        let mut line = String::new();
                        let _ = std::io::BufRead::read_line(
                            &mut std::io::stdin().lock(),
                            &mut line,
                        );
                        while line.ends_with('\n') || line.ends_with('\r') {
                            line.pop();
                        }
                        return Ok(Value::str(line));
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
                // Named function (current module's namespace).
                let qual = format!("{}::{}", self.current_module, id.name);
                if let Some(f) = self.fns.get(&qual).copied() {
                    let module = self.current_module.clone();
                    let mut vs = Vec::with_capacity(args.len());
                    for a in args {
                        vs.push(self.eval_expr(a)?);
                    }
                    let r = self.call_decl(f, &module, vs)?;
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

    /// A `ref` resolves to what it points at; anything else is itself.
    fn deref_value(&mut self, v: Value) -> EResult {
        let Value::Ref(i, g) = v else { return Ok(v) };
        let idx = i as usize;
        match self.slab.get(idx) {
            Some((Some(inner), gen)) if *gen == g => Ok(inner.clone()),
            _ => Err(Flow::Panic(
                "stale reference: the value it pointed to was released".to_string(),
            )),
        }
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
        // `utils.helper` as a value, and `lib.WIDTH` — an exported constant.
        if let Expr::Ident(tid) = recv {
            let is_local = self.scopes.iter().rev().any(|s| s.contains_key(&tid.name));
            if !is_local {
                let target = self
                    .imports
                    .get(&self.current_module)
                    .and_then(|m| m.get(&tid.name))
                    .cloned();
                if let Some(target_mod) = target {
                    let qual = format!("{target_mod}::{}", name.name);
                    if let Some(c) = self.consts.get(&qual).copied() {
                        // Evaluated in the module that declared it, so its own
                        // names mean what they meant there.
                        let saved = std::mem::replace(&mut self.current_module, target_mod);
                        let v = self.eval_expr(&c.init);
                        self.current_module = saved;
                        return v;
                    }
                    if self.fns.contains_key(&qual) {
                        return Ok(Value::FnRef(Rc::new(qual)));
                    }
                }
            }
        }
        let r = self.eval_expr(recv)?;
        // Tier 2: reading through a reference checks the generation first. A
        // stale one panics here rather than reading whatever now occupies the
        // slot, which is the property that makes `ref` safe.
        let r = self.deref_value(r)?;
        match r {
            // `pair.0` — components named by position.
            Value::Tuple(parts) => match name.name.parse::<usize>() {
                Ok(i) if i < parts.len() => Ok(parts[i].clone()),
                _ => Err(Flow::Panic(format!("no component `.{}`", name.name))),
            },
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

    /// Effect op / cross-module / qualified variant / builtin / UFCS method.
    fn eval_method_call(&mut self, recv: &Expr, name: &Ident, args: &[Expr]) -> EResult {
        // Cross-module qualified call `utils.helper(...)`.
        if let Expr::Ident(tid) = recv {
            let is_local = self.scopes.iter().rev().any(|s| s.contains_key(&tid.name));
            if !is_local {
                let target = self
                    .imports
                    .get(&self.current_module)
                    .and_then(|m| m.get(&tid.name))
                    .cloned();
                if let Some(target_mod) = target {
                    let qual = format!("{target_mod}::{}", name.name);
                    if let Some(f) = self.fns.get(&qual).copied() {
                        let mut vs = Vec::with_capacity(args.len());
                        for a in args {
                            vs.push(self.eval_expr(a)?);
                        }
                        let r = self.call_decl(f, &target_mod, vs)?;
                        let arg_refs: Vec<&Expr> = args.iter().collect();
                        self.apply_writebacks(&arg_refs)?;
                        return Ok(r);
                    }
                    // Variant constructor through the module alias.
                    if let Some(Some((tname, _))) = self.variants.get(&name.name).cloned() {
                        let mut vs = Vec::with_capacity(args.len());
                        for a in args {
                            vs.push(self.eval_expr(a)?);
                        }
                        return Ok(Value::Variant {
                            type_name: Rc::new(tname),
                            variant: Rc::new(name.name.clone()),
                            fields: Rc::new(vs),
                        });
                    }
                    return Err(Flow::Panic(format!(
                        "module `{target_mod}` has no function `{}`",
                        name.name
                    )));
                }
            }
        }
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
                let fallible = self
                    .fallible_ops
                    .contains(&format!("{}.{}", tid.name, name.name));
                let out = self.call_value(f, vs, "effect op");
                if !fallible {
                    return out;
                }
                // A fallible operation answers with a `T ! E`, whichever way
                // the handler ended: a value becomes `Ok`, a `fail` becomes
                // `Err`. Without this `??` and `match` on one saw a bare
                // value, and the failure escaped the caller entirely.
                return match out {
                    Ok(v @ Value::Ok(_)) | Ok(v @ Value::Err(_)) => Ok(v),
                    Ok(v) => Ok(Value::Ok(Rc::new(v))),
                    Err(Flow::Err(e)) => Ok(Value::Err(Rc::new(e))),
                    other => other,
                };
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
        // Mutating list/map methods need place semantics.
        if name.name == "remove" {
            let Value::List(items) = self.eval_expr(recv)? else {
                return Err(Flow::Panic("remove on a non-list".into()));
            };
            let Value::Int(i) = self.eval_expr(&args[0])? else {
                return Err(Flow::Panic("remove expects an integer".into()));
            };
            let n = items.len() as i64;
            if i < 0 || i >= n {
                return Err(Flow::Panic(format!("remove {i} out of bounds (len {n})")));
            }
            let mut next = (*items).clone();
            let taken = next.remove(i as usize);
            self.assign_place(recv, Value::List(Rc::new(next)))?;
            return Ok(taken);
        }
        if name.name == "sort_by" {
            let Value::List(items) = self.eval_expr(recv)? else {
                return Err(Flow::Panic("sort_by on a non-list".into()));
            };
            let f = self.eval_expr(&args[0])?;
            let mut keyed: Vec<(Value, Value)> = Vec::with_capacity(items.len());
            for item in items.iter() {
                let k = self.call_value(f.clone(), vec![item.clone()], "sort_by")?;
                keyed.push((k, item.clone()));
            }
            // Stable, so equal keys keep their written order.
            keyed.sort_by(|a, b| compare_values(&a.0, &b.0));
            let sorted: Vec<Value> = keyed.into_iter().map(|(_, v)| v).collect();
            self.assign_place(recv, Value::List(Rc::new(sorted)))?;
            return Ok(Value::Unit);
        }
        if matches!(name.name.as_str(), "push" | "pop" | "sort" | "set" | "delete") {
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
        let ufcs_qual = format!("{}::{}", self.current_module, name.name);
        // Including a declared C function. [L-UFCS] says the two spellings are
        // the same call, and the compiled backend made it; this engine said
        // `no method` for a program that ran natively.
        if let Some(e) = self.externs.get(&ufcs_qual).copied() {
            let mut vs = Vec::with_capacity(args.len() + 1);
            vs.push(recv_v);
            for a in args {
                vs.push(self.eval_expr(a)?);
            }
            return self.call_extern_values(e, vs);
        }
        if let Some(f) = self.fns.get(&ufcs_qual).copied() {
            let module = self.current_module.clone();
            let mut vs = Vec::with_capacity(args.len() + 1);
            vs.push(recv_v);
            for a in args {
                vs.push(self.eval_expr(a)?);
            }
            let r = self.call_decl(f, &module, vs)?;
            let mut arg_refs: Vec<&Expr> = Vec::with_capacity(args.len() + 1);
            arg_refs.push(recv);
            arg_refs.extend(args.iter());
            self.apply_writebacks(&arg_refs)?;
            return Ok(r);
        }
        // A field holding a function, called through the field. The checker
        // accepts this, so refusing here would be a well-typed program failing
        // at run time.
        if let Value::Struct { fields, .. } = &recv_v {
            if let Some((_, f)) = fields.iter().find(|(n, _)| *n == name.name) {
                let callee = f.clone();
                let mut vs = Vec::with_capacity(args.len());
                for a in args {
                    vs.push(self.eval_expr(a)?);
                }
                return self.call_value(callee, vs, &name.name);
            }
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
        // Fast path: bare variable receiver — mutate the slot in place via
        // copy-on-write (no clone when the value is uniquely owned). This is
        // what makes `xs.push(v)` in a loop O(1) amortized instead of O(n).
        if let Expr::Ident(id) = recv {
            // Evaluate args first (needs &mut self before borrowing the slot).
            let mut arg_vals = Vec::with_capacity(args.len());
            for a in args {
                arg_vals.push(self.eval_expr(a)?);
            }
            let slot = self
                .scopes
                .iter_mut()
                .rev()
                .find_map(|s| s.get_mut(&id.name));
            match slot {
                Some(Value::List(items)) => {
                    let items = Rc::make_mut(items);
                    let result = match name.name.as_str() {
                        "push" => {
                            items.push(arg_vals.remove(0));
                            Value::Unit
                        }
                        "pop" => match items.pop() {
                            Some(v) => Value::some(v),
                            None => Value::none(),
                        },
                        "sort" => {
                            items.sort_by(compare_values);
                            Value::Unit
                        }
                        _ => return Ok(None),
                    };
                    return Ok(Some(result));
                }
                Some(Value::Map(entries)) => {
                    let entries = Rc::make_mut(entries);
                    let result = match name.name.as_str() {
                        "set" => {
                            let v = arg_vals.pop().unwrap();
                            let k = arg_vals.pop().unwrap();
                            let Some(key) = crate::value::MapKey::from_value(&k) else {
                                return Err(Flow::Panic(format!(
                                    "invalid map key `{k}`"
                                )));
                            };
                            entries.insert(key, v);
                            Value::Unit
                        }
                        "delete" => {
                            let k = arg_vals.pop().unwrap();
                            let Some(key) = crate::value::MapKey::from_value(&k) else {
                                return Err(Flow::Panic(format!(
                                    "invalid map key `{k}`"
                                )));
                            };
                            Value::Bool(entries.remove(&key).is_some())
                        }
                        _ => return Ok(None),
                    };
                    return Ok(Some(result));
                }
                _ => return Ok(None),
            }
        }
        // Nested place (`bag.items.push(v)` / `obj.cache.set(k, v)`): clone +
        // write back.
        let current = self.eval_expr(recv)?;
        if let Value::Map(entries) = &current {
            let mut m = entries.as_ref().clone();
            let result = match name.name.as_str() {
                "set" => {
                    let k = self.eval_expr(&args[0])?;
                    let v = self.eval_expr(&args[1])?;
                    let Some(key) = crate::value::MapKey::from_value(&k) else {
                        return Err(Flow::Panic(format!("invalid map key `{k}`")));
                    };
                    m.insert(key, v);
                    Value::Unit
                }
                "delete" => {
                    let k = self.eval_expr(&args[0])?;
                    let Some(key) = crate::value::MapKey::from_value(&k) else {
                        return Err(Flow::Panic(format!("invalid map key `{k}`")));
                    };
                    Value::Bool(m.remove(&key).is_some())
                }
                _ => return Ok(None),
            };
            self.assign_place(recv, Value::Map(Rc::new(m)))?;
            return Ok(Some(result));
        }
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
                items_vec.sort_by(compare_values);
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
            // `par_map` is a parallel map natively; here it is a plain one.
            // The reference implementation defines the *answer*, and the
            // answer is order-preserving and identical either way — which is
            // the whole point of requiring the function to be effect-free
            // ([E-PAR-MAP-ORDER]).
            (Value::List(items), "map" | "par_map") => {
                let f = self.eval_expr(&args[0])?;
                let mut out = Vec::with_capacity(items.len());
                for item in items.iter() {
                    out.push(self.call_value(f.clone(), vec![item.clone()], "map")?);
                }
                Value::List(Rc::new(out))
            }
            // `par_filter` tests elements on several cores natively; here it
            // is a plain filter. The answer is order-preserving and identical
            // either way, which is the point of requiring purity.
            (Value::List(items), "filter" | "par_filter") => {
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
                // An empty list has no element to read the type off, so the
                // checker's answer is used instead: `[f64].sum()` is `0.0`,
                // not `0`. Without it this engine returned an integer, printed
                // it without a point, and then panicked `no method to_fixed on
                // 0` the moment the caller used the `f64` it had been
                // promised — the documented deviation this closes.
                if is_float || self.float_sums.contains(&name.span.start) {
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
            (Value::List(items), "is_empty") => Value::Bool(items.is_empty()),
            (Value::List(items), "fold") => {
                let mut acc = self.eval_expr(&args[0])?;
                let f = self.eval_expr(&args[1])?;
                for item in items.iter() {
                    acc = self.call_value(f.clone(), vec![acc, item.clone()], "fold")?;
                }
                acc
            }
            (Value::List(items), "any" | "all") => {
                let want_all = name.name == "all";
                let f = self.eval_expr(&args[0])?;
                let mut result = want_all;
                for item in items.iter() {
                    let hit = matches!(
                        self.call_value(f.clone(), vec![item.clone()], "any/all")?,
                        Value::Bool(true)
                    );
                    if want_all && !hit {
                        result = false;
                        break;
                    }
                    if !want_all && hit {
                        result = true;
                        break;
                    }
                }
                Value::Bool(result)
            }
            (Value::List(items), "find") => {
                let f = self.eval_expr(&args[0])?;
                let mut found = Value::none();
                for item in items.iter() {
                    if let Value::Bool(true) =
                        self.call_value(f.clone(), vec![item.clone()], "find")?
                    {
                        found = Value::some(item.clone());
                        break;
                    }
                }
                found
            }
            (Value::List(items), "min" | "max") => {
                let want_max = name.name == "max";
                let mut best: Option<Value> = None;
                for item in items.iter() {
                    best = Some(match best {
                        None => item.clone(),
                        Some(cur) => {
                            let ord = compare_values(item, &cur);
                            let take = if want_max {
                                ord == std::cmp::Ordering::Greater
                            } else {
                                ord == std::cmp::Ordering::Less
                            };
                            if take {
                                item.clone()
                            } else {
                                cur
                            }
                        }
                    });
                }
                match best {
                    Some(v) => Value::some(v),
                    None => Value::none(),
                }
            }
            (Value::List(items), "enumerate") => Value::List(Rc::new(
                items
                    .iter()
                    .enumerate()
                    .map(|(i, v)| Value::Tuple(Rc::new(vec![Value::Int(i as i64), v.clone()])))
                    .collect(),
            )),
            (Value::List(items), "zip") => {
                let Value::List(other) = self.eval_expr(&args[0])? else {
                    return Err(Flow::Panic("zip expects a list".into()));
                };
                Value::List(Rc::new(
                    items
                        .iter()
                        .zip(other.iter())
                        .map(|(a, b)| Value::Tuple(Rc::new(vec![a.clone(), b.clone()])))
                        .collect(),
                ))
            }
            (Value::List(items), "rev") => {
                let mut v: Vec<Value> = items.as_ref().clone();
                v.reverse();
                Value::List(Rc::new(v))
            }
            (Value::List(items), "slice") => {
                let a = self.eval_expr(&args[0])?;
                let b = self.eval_expr(&args[1])?;
                match (a, b) {
                    (Value::Int(s), Value::Int(e)) => {
                        let len = items.len() as i64;
                        let s = s.clamp(0, len) as usize;
                        let e = e.clamp(0, len) as usize;
                        let v: Vec<Value> =
                            if s <= e { items[s..e].to_vec() } else { Vec::new() };
                        Value::List(Rc::new(v))
                    }
                    _ => return Err(Flow::Panic("slice bounds must be ints".into())),
                }
            }
            (Value::List(items), "first") => match items.first() {
                Some(v) => Value::some(v.clone()),
                None => Value::none(),
            },
            (Value::List(items), "last") => match items.last() {
                Some(v) => Value::some(v.clone()),
                None => Value::none(),
            },
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
            (Value::Str(s), "lines") => Value::List(Rc::new(
                s.lines().map(Value::str).collect(),
            )),
            (Value::Str(s), "is_empty") => Value::Bool(s.is_empty()),
            (Value::Str(s), "pad_start" | "pad_end") => {
                let at_start = name.name == "pad_start";
                let width = match self.eval_expr(&args[0])? {
                    Value::Int(i) => i.max(0) as usize,
                    _ => return Err(Flow::Panic("pad width must be an int".into())),
                };
                let fill = self.eval_expr(&args[1])?.to_string();
                if fill.is_empty() {
                    return Err(Flow::Panic("pad fill must not be empty".into()));
                }
                let len = s.chars().count();
                // A multi-character fill repeats and is cut to the exact width,
                // rather than silently using only its first character.
                let pad: String = fill
                    .chars()
                    .cycle()
                    .take(width.saturating_sub(len))
                    .collect();
                if at_start {
                    Value::str(format!("{pad}{s}"))
                } else {
                    Value::str(format!("{s}{pad}"))
                }
            }
            (Value::Str(s), "trim") => Value::str(s.trim()),
            (Value::Str(s), "trim_start") => Value::str(s.trim_start()),
            (Value::Str(s), "trim_end") => Value::str(s.trim_end()),
            (Value::Str(s), "to_upper") => Value::str(s.to_uppercase()),
            (Value::Str(s), "to_lower") => Value::str(s.to_lowercase()),
            (Value::Str(s), "repeat") => {
                let n = self.eval_expr(&args[0])?;
                match n {
                    Value::Int(k) if k >= 0 => Value::str(s.repeat(k as usize)),
                    _ => return Err(Flow::Panic("repeat count must be a non-negative int".into())),
                }
            }
            (Value::Str(s), "replace") => {
                let from = self.eval_expr(&args[0])?.to_string();
                let to = self.eval_expr(&args[1])?.to_string();
                Value::str(s.replace(&from, &to))
            }
            (Value::Str(s), "substr") => {
                let a = self.eval_expr(&args[0])?;
                let b = self.eval_expr(&args[1])?;
                match (a, b) {
                    (Value::Int(start), Value::Int(len)) => {
                        let chars: Vec<char> = s.chars().collect();
                        let n = chars.len() as i64;
                        let start = start.clamp(0, n) as usize;
                        let end = (start as i64 + len.max(0)).clamp(0, n) as usize;
                        Value::str(chars[start..end].iter().collect::<String>())
                    }
                    _ => return Err(Flow::Panic("substr takes (start, len) ints".into())),
                }
            }
            (Value::Str(s), "index_of") => {
                let needle = self.eval_expr(&args[0])?.to_string();
                match s.find(&needle) {
                    // Char index, not byte index, to match len()/substr().
                    Some(byte_idx) => {
                        let char_idx = s[..byte_idx].chars().count() as i64;
                        Value::some(Value::Int(char_idx))
                    }
                    None => Value::none(),
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
            (Value::Str(s), "code_at") => {
                let Value::Int(i) = self.eval_expr(&args[0])? else {
                    return Err(Flow::Panic("code_at expects an integer".into()));
                };
                let n = s.chars().count() as i64;
                if i < 0 || i >= n {
                    return Err(Flow::Panic(format!(
                        "code_at {i} out of bounds (len {n})"
                    )));
                }
                Value::Int(s.chars().nth(i as usize).expect("in bounds") as i64)
            }
            (Value::Str(s), "words") => Value::List(Rc::new(
                s.split_whitespace().map(Value::str).collect(),
            )),
            (Value::Str(s), "codes") => {
                Value::List(Rc::new(s.chars().map(|c| Value::Int(c as i64)).collect()))
            }
            (Value::Int(i), "to_char") => match u32::try_from(*i).ok().and_then(char::from_u32) {
                Some(c) => Value::str(c.to_string()),
                None => {
                    return Err(Flow::Panic(format!("{i} is not a Unicode code point")))
                }
            },
            (Value::Float(f), "is_nan") => Value::Bool(f.is_nan()),
            (Value::Float(f), "is_finite") => Value::Bool(f.is_finite()),
            (Value::Float(f), "to_fixed") => {
                let Value::Int(places) = self.eval_expr(&args[0])? else {
                    return Err(Flow::Panic("to_fixed expects an integer".into()));
                };
                if !(0..=17).contains(&places) {
                    return Err(Flow::Panic(format!(
                        "to_fixed({places}): places must be 0 to 17"
                    )));
                }
                Value::str(crate::value::fixed_text(*f, places))
            }
            (Value::Int(_) | Value::Float(_) | Value::Str(_) | Value::Tuple(_), "cmp") => {
                let other = self.eval_expr(&args[0])?;
                Value::Int(match compare_values(&recv, &other) {
                    std::cmp::Ordering::Less => -1,
                    std::cmp::Ordering::Equal => 0,
                    std::cmp::Ordering::Greater => 1,
                })
            }
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
            (Value::Float(x), "sin") => Value::Float(x.sin()),
            (Value::Float(x), "cos") => Value::Float(x.cos()),
            (Value::Float(x), "tan") => Value::Float(x.tan()),
            (Value::Float(x), "asin") => Value::Float(x.asin()),
            (Value::Float(x), "acos") => Value::Float(x.acos()),
            (Value::Float(x), "atan") => Value::Float(x.atan()),
            (Value::Float(x), "sinh") => Value::Float(x.sinh()),
            (Value::Float(x), "cosh") => Value::Float(x.cosh()),
            (Value::Float(x), "tanh") => Value::Float(x.tanh()),
            (Value::Float(x), "exp") => Value::Float(x.exp()),
            (Value::Float(x), "ln") => Value::Float(x.ln()),
            (Value::Float(x), "log2") => Value::Float(x.log2()),
            (Value::Float(x), "log10") => Value::Float(x.log10()),
            (Value::Float(x), "cbrt") => Value::Float(x.cbrt()),
            (Value::Float(x), "to_radians") => Value::Float(x.to_radians()),
            (Value::Float(x), "to_degrees") => Value::Float(x.to_degrees()),
            (Value::Float(x), "atan2") => {
                let o = self.eval_expr(&args[0])?;
                match o {
                    Value::Float(y) => Value::Float(x.atan2(y)),
                    _ => return Err(Flow::Panic("atan2 expects a float".into())),
                }
            }
            (Value::Float(x), "hypot") => {
                let o = self.eval_expr(&args[0])?;
                match o {
                    Value::Float(y) => Value::Float(x.hypot(y)),
                    _ => return Err(Flow::Panic("hypot expects a float".into())),
                }
            }
            (Value::Float(x), "log") => {
                let o = self.eval_expr(&args[0])?;
                match o {
                    Value::Float(y) => Value::Float(x.log(y)),
                    _ => return Err(Flow::Panic("log expects a float".into())),
                }
            }
            (Value::Float(x), "floor") => Value::Float(x.floor()),
            (Value::Float(x), "ceil") => Value::Float(x.ceil()),
            (Value::Float(x), "round") => Value::Float(x.round()),
            // The same text `print` produces, not Rust's. `f64::to_string`
            // drops the point on a whole value — `(1.0).to_str()` was `"1"` —
            // so the interpreter disagreed with the compiled backend and, more
            // to the point, broke [L-DISPLAY]'s reason for existing: a float
            // has a point at every magnitude so it stays distinguishable from
            // an integer. The documented idiom `xs.map(|v| v.to_str())`
            // printed a `[f64]` as integers.
            (Value::Float(x), "to_str") => Value::str(crate::value::float_text(*x)),
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
            (Value::Map(entries), "get") => {
                let k = self.eval_expr(&args[0])?;
                match crate::value::MapKey::from_value(&k) {
                    Some(key) => match entries.get(&key) {
                        Some(v) => Value::some(v.clone()),
                        None => Value::none(),
                    },
                    None => return Err(Flow::Panic(format!("invalid map key `{k}`"))),
                }
            }
            (Value::Map(entries), "has") => {
                let k = self.eval_expr(&args[0])?;
                match crate::value::MapKey::from_value(&k) {
                    Some(key) => Value::Bool(entries.contains_key(&key)),
                    None => return Err(Flow::Panic(format!("invalid map key `{k}`"))),
                }
            }
            (Value::Map(entries), "len") => Value::Int(entries.len() as i64),
            (Value::Map(entries), "is_empty") => Value::Bool(entries.is_empty()),
            (Value::Map(entries), "keys") => Value::List(Rc::new(
                entries.keys().map(|k| k.to_value()).collect(),
            )),
            (Value::Map(entries), "values") => {
                Value::List(Rc::new(entries.values().cloned().collect()))
            }
            (Value::Map(entries), "entries") => Value::List(Rc::new(
                entries
                    .iter()
                    .map(|(k, v)| Value::Tuple(Rc::new(vec![k.to_value(), v.clone()])))
                    .collect(),
            )),
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
            (
                Value::Variant {
                    variant, fields, ..
                },
                "expect",
            ) => {
                if variant.as_str() == "Some" {
                    fields[0].clone()
                } else {
                    let msg = self.eval_expr(&args[0])?;
                    return Err(Flow::Panic(format!("{msg}")));
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
                    // `i64::MIN / -1` has no i64 result, so it wraps like
                    // every other overflow ([E-ARITH-INT]) rather than
                    // panicking. Plain `/` raised a *Rust* panic there —
                    // a host-level abort that an embedder cannot catch and
                    // that says nothing a Grasm programmer can act on.
                    Div => {
                        if b == 0 {
                            return Err(Flow::Panic("division by zero".into()));
                        }
                        Value::Int(a.wrapping_div(b))
                    }
                    Rem => {
                        if b == 0 {
                            return Err(Flow::Panic("remainder by zero".into()));
                        }
                        Value::Int(a.wrapping_rem(b))
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
            // Tuples compare component by component, which is what makes a
            // multi-key sort key work with `<` too.
            (Value::Tuple(_), Value::Tuple(_))
                if matches!(op, Lt | LtEq | Gt | GtEq) =>
            {
                let ord = compare_values(&l, &r);
                Ok(Value::Bool(match op {
                    Lt => ord.is_lt(),
                    LtEq => ord.is_le(),
                    Gt => ord.is_gt(),
                    _ => ord.is_ge(),
                }))
            }
            (Value::List(a), Value::List(b)) if op == Add => {
                let mut out = Vec::with_capacity(a.len() + b.len());
                out.extend(a.iter().cloned());
                out.extend(b.iter().cloned());
                Ok(Value::List(Rc::new(out)))
            }
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
            // Any alternative matching is enough. None of them bind, so there
            // is nothing to reconcile between the branches.
            Pattern::Or(alts, _) => alts.iter().any(|a| self.match_pattern(a, v, bindings)),
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

/// True for the surface types that mean `f64`.
fn is_float_type(t: &TypeExpr) -> bool {
    matches!(t, TypeExpr::Name { path, args, .. }
        if args.is_empty()
            && matches!(
                path.segments.last().map(|s| s.name.as_str()),
                Some("f64") | Some("f32") | Some("float")
            ))
}

fn io_error(variant: &str, payload: impl Into<String>) -> Value {
    Value::Variant {
        type_name: Rc::new("IoError".into()),
        variant: Rc::new(variant.into()),
        fields: Rc::new(vec![Value::str(payload.into())]),
    }
}

fn compare_values(a: &Value, b: &Value) -> std::cmp::Ordering {
    use std::cmp::Ordering;
    match (a, b) {
        (Value::Int(x), Value::Int(y)) => x.cmp(y),
        // `total_cmp`, not `partial_cmp`. Calling every NaN comparison `Equal`
        // is not a total order, and a sort given one does not merely misplace
        // the NaN — it returns a list that is not sorted at all. One `nan`
        // cell in a CSV was enough to scramble the whole column.
        (Value::Float(x), Value::Float(y)) => x.total_cmp(y),
        (Value::Str(x), Value::Str(y)) => x.cmp(y),
        // A tuple of ordered things is ordered, component by component — this
        // is what a multi-key sort is.
        (Value::Tuple(x), Value::Tuple(y)) => {
            for (a, b) in x.iter().zip(y.iter()) {
                let ord = compare_values(a, b);
                if ord != Ordering::Equal {
                    return ord;
                }
            }
            x.len().cmp(&y.len())
        }
        _ => Ordering::Equal,
    }
}
