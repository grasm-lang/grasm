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
    /// Constants already built, by qualified name.
    ///
    /// The initializer is effect-free, so building it once and handing out
    /// copies is exactly what the name means — and copies are cheap, because
    /// a value is shared until something writes to it ([E-COW]). Rebuilding
    /// per mention made every *read* of an n-element constant O(n): a loop of
    /// k reads over a 4 000-entry table took 632 ms where the same loop over
    /// a local copy took 1 ms, and [E-CONST] recommends constants for exactly
    /// that dispatch-table shape.
    const_cache: HashMap<String, Value>,
    /// Integer literals the checker resolved to `f64`, by span start.
    float_literals: std::collections::HashSet<u32>,
    /// See `Analysis::ok_wrapped`.
    ok_wrapped: std::collections::HashSet<(u32, u32)>,
    /// Closure literals whose own type is fallible, by start offset.
    ///
    /// `fail(e)` raises the error branch of the enclosing fallible function,
    /// and a closure is one only when its type says so. Which closures those
    /// are is a fact only the checker has.
    fallible_closures: std::collections::HashSet<u32>,
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

/// How much of this thread's stack the interpreter will spend before it calls
/// the recursion too deep.
///
/// The call counter alone is not enough: how many Rust frames one Grasm call
/// costs depends on the path it takes, and recursion through a built-in
/// higher-order method — `[n].map(down)` — costs several times what a plain
/// call does. At 100 000 Grasm frames that overflowed the real stack first,
/// and the reference implementation died with `fatal runtime error: stack
/// overflow` and SIGABRT — the abort [E-RECURSION-DEPTH] says cannot happen.
// A debug build's frames are several times larger for the same recursion, so
// the same *depth* needs several times the bytes. Held to one number, the
// 10,000 frames [E-RECURSION-DEPTH] guarantees stopped short under
// `cargo test` — a red suite that said nothing about the language.
#[cfg(debug_assertions)]
const STACK_BUDGET: usize = 1792 * 1024 * 1024;
#[cfg(not(debug_assertions))]
const STACK_BUDGET: usize = 448 * 1024 * 1024;

thread_local! {
    /// The lowest stack address this thread may use, set on the first check.
    static STACK_FLOOR: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
}

/// Whether the stack has run down past this thread's floor.
fn stack_exhausted() -> bool {
    let here = 0u8;
    let sp = std::ptr::addr_of!(here) as usize;
    let floor = STACK_FLOOR.with(std::cell::Cell::get);
    if floor == 0 {
        STACK_FLOOR.with(|c| c.set(sp.saturating_sub(STACK_BUDGET)));
        return false;
    }
    sp < floor
}

/// One step of an assignment path.
#[derive(Clone)]
enum Seg {
    Field(String),
    Index(usize),
}

/// Walks `segs` into `root` and stores `v` at the end, copy-on-write.
/// The deepest `ref` reachable along a path, and how many segments it took.
///
/// `None` when the path holds no reference, or when it cannot be followed —
/// the ordinary walk reports that, with its own message.
fn deepest_ref(root: &Value, segs: &[Seg]) -> Option<(usize, u32, u32)> {
    // With no path to walk, `r = other` rebinds the *variable*; it does not
    // write through the reference, so a released one is not touched and the
    // liveness question does not arise. Asking it anyway made
    // `release(r)  r = ref(…)` panic in the interpreter and succeed compiled.
    if segs.is_empty() {
        return None;
    }
    let mut cur = root;
    let mut found: Option<(usize, u32, u32)> = None;
    for (i, seg) in segs.iter().enumerate() {
        if let Value::Ref(slot, gen) = cur {
            found = Some((i, *slot, *gen));
            // Anything past a reference is inside the slab, and the caller
            // takes it from there.
            return found;
        }
        cur = match (cur, seg) {
            (Value::Struct { fields, .. }, Seg::Field(f)) => {
                &fields.iter().find(|(k, _)| k == f)?.1
            }
            (Value::Tuple(items), Seg::Field(f)) => items.get(f.parse::<usize>().ok()?)?,
            (Value::List(items), Seg::Index(i)) => items.get(*i)?,
            _ => return None,
        };
    }
    if let Value::Ref(slot, gen) = cur {
        found = Some((segs.len(), *slot, *gen));
    }
    found
}

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

/// Applies a mutating method to a value in place.
///
/// `None` means "not a shape this handles" — the caller falls back to the
/// read-modify-write path rather than reporting an error, because the set of
/// mutators is checked elsewhere.
fn mutate_in_place(
    slot: &mut Value,
    method: &str,
    args: &mut Vec<Value>,
) -> Result<Option<Value>, Flow> {
    let r = match (slot, method) {
        (Value::List(items), "push") => {
            Rc::make_mut(items).push(args.remove(0));
            Value::Unit
        }
        (Value::List(items), "pop") => match Rc::make_mut(items).pop() {
            Some(v) => Value::some(v),
            None => Value::none(),
        },
        (Value::List(items), "sort") => {
            Rc::make_mut(items).sort_by(compare_values);
            Value::Unit
        }
        // In place like the rest. Going through the read-modify-write path
        // instead evaluated the receiver's index twice, so
        // `kk[q.remove(0)].remove(0)` returned an element from one list and
        // removed it from another ([E-ORDER-LTR]).
        (Value::List(items), "remove") => {
            let Value::Int(i) = args.remove(0) else {
                return Err(Flow::Panic("remove expects an integer".into()));
            };
            let items = Rc::make_mut(items);
            let n = items.len() as i64;
            // Out of range panics, the way indexing does ([L-LIB-REMOVE]).
            // Treating a negative index as counting from the end removed the
            // *last* element for `remove(-1)` and said nothing, while the
            // compiled backend panicked — two engines, two answers.
            if i < 0 || i >= n {
                return Err(Flow::Panic(format!(
                    "remove {i} out of bounds (len {n})"
                )));
            }
            items.remove(i as usize)
        }
        (Value::Map(entries), "set") => {
            let v = args.pop().unwrap();
            let k = args.pop().unwrap();
            let Some(key) = crate::value::MapKey::from_value(&k) else {
                return Err(Flow::Panic(format!("invalid map key `{k}`")));
            };
            Rc::make_mut(entries).insert(key, v);
            Value::Unit
        }
        (Value::Map(entries), "delete") => {
            let k = args.pop().unwrap();
            let Some(key) = crate::value::MapKey::from_value(&k) else {
                return Err(Flow::Panic(format!("invalid map key `{k}`")));
            };
            Value::Bool(Rc::make_mut(entries).remove(&key).is_some())
        }
        _ => return Ok(None),
    };
    Ok(Some(r))
}

/// The binding a place expression is rooted at, if it is a place at all.
fn place_root_name(e: &Expr) -> Option<String> {
    let mut cur = e;
    loop {
        match cur {
            Expr::Field { recv, .. } | Expr::Index { recv, .. } => cur = recv,
            Expr::Ident(id) => return Some(id.name.clone()),
            _ => return None,
        }
    }
}

/// Every identifier a closure body could mention, over-approximated.
///
/// A closure captured the *whole* visible environment by value, which put a
/// second owner on every list in scope — so the next `xs.push(v)` in the same
/// loop copied the list instead of mutating it, and a loop that created a
/// closure was quadratic while the identical loop without one was linear
/// ([E-COW]: the copy "is never observable", and its cost was).
///
/// Over-approximating is safe: a name captured and never used costs one
/// refcount, and missing one would change what the closure sees.
fn mentioned_names(e: &Expr, out: &mut std::collections::HashSet<String>) {
    match e {
        Expr::Ident(id) => {
            out.insert(id.name.clone());
        }
        Expr::Unary { expr, .. } | Expr::Try { expr, .. } => mentioned_names(expr, out),
        Expr::Binary { lhs, rhs, .. } => {
            mentioned_names(lhs, out);
            mentioned_names(rhs, out);
        }
        Expr::Field { recv, .. } => mentioned_names(recv, out),
        Expr::Index { recv, index, .. } => {
            mentioned_names(recv, out);
            mentioned_names(index, out);
        }
        Expr::Call { callee, args, .. } => {
            mentioned_names(callee, out);
            for a in args {
                mentioned_names(a, out);
            }
        }
        Expr::MethodCall { recv, args, .. } => {
            mentioned_names(recv, out);
            for a in args {
                mentioned_names(a, out);
            }
        }
        Expr::List(items, _) | Expr::Tuple(items, _) => {
            for i in items {
                mentioned_names(i, out);
            }
        }
        Expr::Map(entries, _) => {
            for (k, v) in entries {
                mentioned_names(k, out);
                mentioned_names(v, out);
            }
        }
        Expr::Str(pieces, _) => {
            for p in pieces {
                if let StrPiece::Expr(inner) = p {
                    mentioned_names(inner, out);
                }
            }
        }
        Expr::Closure { body, .. } => mentioned_names(body, out),
        Expr::StructLit { fields, base, .. } => {
            if let Some(b) = base {
                mentioned_names(b, out);
            }
            for (_, v) in fields {
                mentioned_names(v, out);
            }
        }
        Expr::Block(b) | Expr::Unsafe(b) | Expr::Region(b) => mentioned_names_block(b, out),
        Expr::If { cond, then_block, else_branch, .. } => {
            mentioned_names(cond, out);
            mentioned_names_block(then_block, out);
            match else_branch.as_deref() {
                Some(grasm_syntax::ast::ElseBranch::Else(b)) => mentioned_names_block(b, out),
                Some(grasm_syntax::ast::ElseBranch::ElseIf(inner)) => {
                    mentioned_names(inner, out)
                }
                None => {}
            }
        }
        Expr::Match { scrutinee, arms, .. } => {
            mentioned_names(scrutinee, out);
            for a in arms {
                mentioned_names(&a.body, out);
                if let Some(g) = &a.guard {
                    mentioned_names(g, out);
                }
            }
        }
        Expr::Range { start, end, .. } => {
            mentioned_names(start, out);
            mentioned_names(end, out);
        }
        _ => {}
    }
}

fn mentioned_names_block(b: &Block, out: &mut std::collections::HashSet<String>) {
    for st in &b.stmts {
        match st {
            Stmt::Let { init, .. } | Stmt::LetTuple { init, .. } => mentioned_names(init, out),
            Stmt::Assign { target, value, .. } => {
                mentioned_names(target, out);
                mentioned_names(value, out);
            }
            Stmt::Return(Some(e), _) | Stmt::Expr(e) => mentioned_names(e, out),
            Stmt::While { cond, body, .. } => {
                mentioned_names(cond, out);
                mentioned_names_block(body, out);
            }
            Stmt::For { iter, body, .. } => {
                mentioned_names(iter, out);
                mentioned_names_block(body, out);
            }
            Stmt::With { handler, body, .. } => {
                mentioned_names(handler, out);
                mentioned_names_block(body, out);
            }
            Stmt::Parallel { body, .. } | Stmt::Spawn { body, .. } => {
                mentioned_names_block(body, out)
            }
            _ => {}
        }
    }
}

/// Whether an expression may mention a name.
///
/// Deliberately blunt, and conservative in the one direction that matters:
/// anything it does not recognise answers "yes". This decides whether it is
/// safe to move a value out of its place for the duration of a call, and a
/// false "yes" only costs the clone that used to happen unconditionally.
fn mentions_ident(e: &Expr, name: &str) -> bool {
    match e {
        Expr::Ident(id) => id.name == name,
        Expr::Int(..) | Expr::Float(..) | Expr::Bool(..) => false,
        Expr::Str(pieces, _) => pieces.iter().any(|p| match p {
            StrPiece::Expr(inner) => mentions_ident(inner, name),
            _ => false,
        }),
        Expr::Unary { expr, .. } | Expr::Try { expr, .. } => mentions_ident(expr, name),
        Expr::Field { recv, .. } => mentions_ident(recv, name),
        Expr::Binary { lhs, rhs, .. } => {
            mentions_ident(lhs, name) || mentions_ident(rhs, name)
        }
        Expr::Index { recv, index, .. } => {
            mentions_ident(recv, name) || mentions_ident(index, name)
        }
        Expr::List(items, _) | Expr::Tuple(items, _) => {
            items.iter().any(|i| mentions_ident(i, name))
        }
        Expr::Call { callee, args, .. } => {
            mentions_ident(callee, name) || args.iter().any(|a| mentions_ident(a, name))
        }
        Expr::MethodCall { recv, args, .. } => {
            mentions_ident(recv, name) || args.iter().any(|a| mentions_ident(a, name))
        }
        _ => true,
    }
}

/// One step of a place path, with any index already evaluated.
///
/// Indices are values, not expressions, because walking to the slot needs a
/// mutable borrow of the environment and evaluating an index needs the
/// environment too. Doing all the evaluation first is what lets the walk be a
/// single borrow.
enum PathSeg {
    Field(String),
    Index(i64),
}

/// The largest string this implementation will build.
///
/// A `repeat` past it is a panic with a sentence, not a Rust allocator
/// message or `capacity overflow` with a backtrace.
const MAX_ALLOC: usize = 1 << 33;

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
        me.ok_wrapped = a.ok_wrapped;
        me.float_sums = a.float_sums;
        me.fallible_closures = a.fallible_closures;
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

    /// The branch tails the checker accepted as the bare ok half, from
    /// `Analysis`.
    pub fn set_ok_wrapped(&mut self, spans: std::collections::HashSet<(u32, u32)>) {
        self.ok_wrapped = spans;
    }

    /// The closure literals the checker typed as fallible, from `Analysis`.
    pub fn set_fallible_closures(&mut self, spans: std::collections::HashSet<u32>) {
        self.fallible_closures = spans;
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
            const_cache: HashMap::new(),
            float_literals: std::collections::HashSet::new(),
            ok_wrapped: std::collections::HashSet::new(),
            fallible_closures: std::collections::HashSet::new(),
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
        if self.depth >= MAX_DEPTH || stack_exhausted() {
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
                fallible,
            } => {
                if self.depth >= MAX_DEPTH || stack_exhausted() {
                    // The same sentence every engine says. [E-RECURSION-DEPTH]
                    // guarantees the message does not differ and that no
                    // engine names its own bound; a second overflow site with
                    // its own wording made `[1].map(|x| f(x))` recursion
                    // report differently under `grasm run` than compiled.
                    return Err(Flow::Panic(
                        "stack overflow: recursion too deep".into(),
                    ));
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
                // A fallible closure's plain value is its ok half, wrapped
                // on the way out — the same rule a named fallible function
                // follows ([T-OK-ERR-RETURN]). Unwrapped, `[1, 2].map(f)` for
                // `let f: fn(i64) -> i64 ! E = |x| x * 2` produced a
                // `[i64 ! E]` holding bare integers: printed as `[2, 4]`, and
                // a `match` on one panicked `no match arm matched`.
                let wrap = |v: Value| match v {
                    already @ (Value::Ok(_) | Value::Err(_)) if fallible => already,
                    v if fallible => Value::Ok(Rc::new(v)),
                    v => v,
                };
                match r {
                    Ok(v) => Ok(wrap(v)),
                    Err(Flow::Return(v)) => Ok(wrap(v)),
                    // A *fallible* closure is a function boundary like any
                    // other, so a `fail` inside one is that closure's own
                    // failure. Letting it through unwound past the `??` and
                    // the `match` written to handle it, all the way out of
                    // `main` ([T-FALLIBLE]: "an ordinary value, not an
                    // exception"). A closure whose type is *not* fallible has
                    // no error branch of its own, and there the failure
                    // belongs to the enclosing function —
                    // `xs.par_map(|x| if c { fail(e) } else { x })` produces
                    // the failure, not a list holding one ([E-PAR-FAIL]).
                    Err(Flow::Err(e)) if fallible => Ok(Value::Err(Rc::new(e))),
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
                // The target's own subexpressions come first: `xs[f()] = g()`
                // runs `f` before `g` ([E-ORDER-LTR]). Evaluating the value
                // and only then walking the place ran them the other way
                // round, consistently in every engine.
                //
                // Held here rather than on `self`: evaluating the value can
                // run another assignment — a call that assigns, a compound
                // `+=` in a loop — and a field would hand this statement's
                // path to that one.
                let resolved = self.place_segs(target)?;
                let new = self.eval_expr(value)?;
                let final_v = if *op == AssignOp::Assign {
                    new
                } else {
                    // Read through the path already resolved, not by
                    // evaluating the target again: `xs[step(i)] += 5` must
                    // call `step` once. Re-evaluating read one element and
                    // wrote another, and the two engines disagreed about how
                    // many times it happened ([E-ORDER-LTR]).
                    let old = match &resolved {
                        Some(segs) => self.read_rooted(target, segs)?,
                        None => self.eval_expr(target)?,
                    };
                    let bop = match op {
                        AssignOp::Add => BinaryOp::Add,
                        AssignOp::Sub => BinaryOp::Sub,
                        AssignOp::Mul => BinaryOp::Mul,
                        AssignOp::Div => BinaryOp::Div,
                        AssignOp::Assign => unreachable!(),
                    };
                    self.apply_binary(bop, old, new)?
                };
                match resolved {
                    Some(segs) => {
                        let mut root = target;
                        while let Expr::Field { recv, .. } | Expr::Index { recv, .. } = root {
                            root = recv;
                        }
                        match root {
                            Expr::Ident(id) => {
                                let id = id.clone();
                                self.assign_rooted(&id, segs, final_v)?
                            }
                            _ => self.assign_place(target, final_v)?,
                        }
                    }
                    None => self.assign_place(target, final_v)?,
                }
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
                // A range is walked, not built. Collecting it first cost ~40
                // bytes per *potential* iteration whether or not the loop ran
                // that far — `for i in 0..200000000 { break }` took 5.8 GB
                // where the compiled backend took 1.4 MB — and at the top of
                // `i64` it did not run at all: the allocation overflowed and
                // the process died with a raw `capacity overflow` from inside
                // the host language, with no `panic:` line, against
                // [E-PANIC-EXIT] and [E-LOOP-RANGE-MAX].
                if let Value::Range(a, b, incl) = iterable {
                    let mut i = a;
                    loop {
                        if incl {
                            if i > b {
                                break;
                            }
                        } else if i >= b {
                            break;
                        }
                        self.scopes.push(HashMap::new());
                        self.scopes
                            .last_mut()
                            .unwrap()
                            .insert(var.name.clone(), Value::Int(i));
                        let r = self.eval_block(body);
                        self.scopes.pop();
                        match r {
                            Ok(_) => {}
                            Err(Flow::Break) => break,
                            Err(Flow::Continue) => {}
                            Err(other) => return Err(other),
                        }
                        // The last value of an inclusive range is the end, so
                        // stepping past it would wrap ([E-LOOP-RANGE-MAX]).
                        if incl && i == b {
                            break;
                        }
                        i += 1;
                    }
                    return Ok(Value::Unit);
                }
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

    /// The same, using each argument's already-resolved path where there is
    /// one — so the writeback lands exactly where the snapshot came from and
    /// the index expression runs once ([E-ORDER-LTR]).
    fn apply_writebacks_at(
        &mut self,
        arg_exprs: &[&Expr],
        paths: &[Option<Vec<Seg>>],
    ) -> Result<(), Flow> {
        let wbs = std::mem::take(&mut self.pending_writebacks);
        for (i, v) in wbs {
            if let Some(e) = arg_exprs.get(i) {
                let is_place = matches!(
                    e,
                    Expr::Ident(_) | Expr::Field { .. } | Expr::Index { .. }
                );
                if !is_place {
                    continue;
                }
                match paths.get(i).and_then(|p| p.clone()) {
                    Some(segs) => {
                        let mut root = *e;
                        while let Expr::Field { recv, .. } | Expr::Index { recv, .. } = root {
                            root = recv;
                        }
                        match root {
                            Expr::Ident(id) => {
                                let id = id.clone();
                                self.assign_rooted(&id, segs, v)?
                            }
                            _ => self.assign_place(e, v)?,
                        }
                    }
                    None => self.assign_place(e, v)?,
                }
            }
        }
        Ok(())
    }

    // ------------------------------------------------------------------
    // Places (assignment targets)
    // ------------------------------------------------------------------

    /// Reads the value at a path already resolved to its root binding.
    fn read_rooted(&mut self, target: &Expr, segs: &[Seg]) -> Result<Value, Flow> {
        let mut root = target;
        while let Expr::Field { recv, .. } | Expr::Index { recv, .. } = root {
            root = recv;
        }
        let Expr::Ident(id) = root else {
            return self.eval_expr(target);
        };
        let Some(mut cur) = self.scopes.iter().rev().find_map(|sc| sc.get(&id.name)).cloned()
        else {
            return self.eval_expr(target);
        };
        for seg in segs {
            cur = match (&cur, seg) {
                (Value::Ref(i, g), _) => {
                    let idx = *i as usize;
                    let live = matches!(self.slab.get(idx), Some((Some(_), gen)) if *gen == *g);
                    if !live {
                        return Err(Flow::Panic(
                            "stale reference: the value it pointed to was released".to_string(),
                        ));
                    }
                    let inner = self.slab[idx].0.clone().expect("checked live");
                    match (&inner, seg) {
                        (Value::Struct { fields, .. }, Seg::Field(f)) => fields
                            .iter()
                            .find(|(k, _)| k == f)
                            .map(|(_, v)| v.clone())
                            .ok_or_else(|| Flow::Panic(format!("no field `{f}`")))?,
                        (Value::List(items), Seg::Index(i)) => items
                            .get(*i)
                            .cloned()
                            .ok_or_else(|| Flow::Panic(format!("index {i} out of bounds")))?,
                        _ => return self.eval_expr(target),
                    }
                }
                (Value::Struct { fields, .. }, Seg::Field(f)) => fields
                    .iter()
                    .find(|(k, _)| k == f)
                    .map(|(_, v)| v.clone())
                    .ok_or_else(|| Flow::Panic(format!("no field `{f}`")))?,
                (Value::Tuple(items), Seg::Field(f)) => {
                    let i: usize = f.parse().map_err(|_| Flow::Panic(format!("no field `{f}`")))?;
                    items
                        .get(i)
                        .cloned()
                        .ok_or_else(|| Flow::Panic(format!("no component `{i}`")))?
                }
                (Value::List(items), Seg::Index(i)) => items
                    .get(*i)
                    .cloned()
                    .ok_or_else(|| Flow::Panic(format!("index {i} out of bounds (len {})", items.len())))?,
                _ => return self.eval_expr(target),
            };
        }
        Ok(cur)
    }

    /// The target's path, with its index expressions already evaluated.
    ///
    /// `None` when the target is not a place rooted at a name; the ordinary
    /// walk reports that, with its own message.
    fn place_segs(&mut self, target: &Expr) -> Result<Option<Vec<Seg>>, Flow> {
        // The path is collected first and *then* evaluated, so the indices
        // run root-to-leaf — left to right as written. Evaluating while
        // walking inward ran `d[f()][g()]` as `g` then `f` ([E-ORDER-LTR]).
        enum Step<'e> {
            Field(String),
            Index(&'e Expr),
        }
        let mut steps: Vec<Step> = Vec::new();
        let mut cur = target;
        loop {
            match cur {
                Expr::Field { recv, name, .. } => {
                    steps.push(Step::Field(name.name.clone()));
                    cur = recv;
                }
                Expr::Index { recv, index, .. } => {
                    steps.push(Step::Index(index));
                    cur = recv;
                }
                Expr::Ident(_) => break,
                _ => return Ok(None),
            }
        }
        steps.reverse();
        let mut segs: Vec<Seg> = Vec::with_capacity(steps.len());
        for step in steps {
            match step {
                Step::Field(f) => segs.push(Seg::Field(f)),
                Step::Index(e) => {
                    let idx = match self.eval_expr(e)? {
                        Value::Int(i) => i,
                        other => {
                            return Err(Flow::Panic(format!(
                                "index must be an int, got `{other}`"
                            )))
                        }
                    };
                    if idx < 0 {
                        return Err(Flow::Panic(format!("negative index {idx}")));
                    }
                    segs.push(Seg::Index(idx as usize));
                }
            }
        }
        Ok(Some(segs))
    }

    /// How many references one write may follow.
    ///
    /// A bound rather than a cycle check: a path is written by hand and no
    /// real one is this long, while `ref(ref(...))` pointing back at itself
    /// would otherwise loop forever inside the interpreter.
    const REF_HOPS: u32 = 64;

    /// Writes `v` at `segs`, starting from the value in slab slot `idx`.
    ///
    /// A path may cross more than one reference: `b.one.v`, where `b` holds a
    /// reference to a node whose `one` field is a reference to another. The
    /// write used to descend into one slab value and panic the moment it met
    /// the next — `cannot access field `v` of `ref#0.0`` — so a graph node
    /// holding a reference to a graph node was write-once, which is every
    /// graph. Reading the identical path worked, which is what
    /// [T-REF-WRITE]'s "anywhere on the path, not only at the root" means.
    fn assign_in_slab(
        &mut self,
        idx: usize,
        gen: u32,
        segs: &[Seg],
        v: Value,
        hops: u32,
    ) -> Result<(), Flow> {
        if hops > Self::REF_HOPS {
            return Err(Flow::Panic(format!(
                "a write followed more than {} references",
                Self::REF_HOPS
            )));
        }
        let live = matches!(self.slab.get(idx), Some((Some(_), g)) if *g == gen);
        if !live {
            return Err(Flow::Panic(
                "stale reference: the value it pointed to was released".to_string(),
            ));
        }
        let inner = self.slab[idx].0.take().expect("checked live");
        // The next reference on what is left of the path, if any. Put this
        // slot back before descending: the next one may be this one again.
        if let Some((consumed, next, next_gen)) = deepest_ref(&inner, segs) {
            let rest: Vec<Seg> = segs[consumed..].to_vec();
            self.slab[idx].0 = Some(inner);
            return self.assign_in_slab(next as usize, next_gen, &rest, v, hops + 1);
        }
        let mut inner = inner;
        let r = assign_into(&mut inner, segs, v);
        self.slab[idx].0 = Some(inner);
        r
    }

    /// Writes `v` at a path already resolved to its root binding.
    fn assign_rooted(&mut self, id: &Ident, segs: Vec<Seg>, v: Value) -> Result<(), Flow> {
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
        // A `ref` anywhere on the path, not only at the root.
        // Writing *through* a reference does not modify what
        // holds it, so the way there is read-only and the write
        // lands in the slab. Only the root was handled, so
        // `rs[0].v = 5` and `h.r.v = 9` — a node in a list and a
        // node in a field, which is the ordinary Tier 2 arena —
        // typechecked and panicked `cannot access field 'v' of
        // 'ref#0.0'` ([T-REF-WRITE], [T-REF-GENERATIONAL]).
        let via = self
            .scopes
            .iter()
            .rev()
            .find_map(|sc| sc.get(&id.name))
            .cloned()
            .and_then(|rv| deepest_ref(&rv, &segs));
        if let Some((consumed, i, g)) = via {
            return self.assign_in_slab(i as usize, g, &segs[consumed..], v, 0);
        }
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
            return self.assign_in_slab(i as usize, g, &segs, v, 0);
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
    fn assign_place(&mut self, target: &Expr, v: Value) -> Result<(), Flow> {
        // Collect the access path down to the root identifier.
        //
        // Already evaluated by `place_segs` when an assignment statement got
        // here, so an index's side effects happen once and in source order.
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
                    return self.assign_rooted(id, segs, v);
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
        let v = self.eval_expr_inner(e)?;
        // A branch that gave the bare ok half where the whole `T ! E` was
        // wanted is wrapped here ([T-OK-ERR-RETURN]: "the branches giving only
        // the ok half are the ones wrapped"). Only the checker knows which
        // those are, so it hands the set over the way it hands over the float
        // literals. A fallible *function* re-wraps on the way out, which
        // masked this everywhere except the positions that do not — a `let`
        // annotation, a struct field, a list element — where the raw value was
        // stored: `let a: i64 ! E = if true { 1 } else { Ok(2) }` printed `1`,
        // compared unequal to `Ok(1)`, and matched no arm of a `match` over
        // it, while a native build answered correctly.
        if self.ok_wrapped.contains(&(e.span().start, e.span().end))
            && !matches!(v, Value::Ok(_) | Value::Err(_))
        {
            return Ok(Value::Ok(std::rc::Rc::new(v)));
        }
        Ok(v)
    }

    fn eval_expr_inner(&mut self, e: &Expr) -> EResult {
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
            Expr::Block(b) | Expr::Unsafe(b) | Expr::Region(b) => self.eval_block(b),
            Expr::Try { expr, .. } => {
                let v = self.eval_expr(expr)?;
                match v {
                    Value::Ok(inner) => Ok((*inner).clone()),
                    Value::Err(e) => Err(Flow::Err((*e).clone())),
                    other => Ok(other),
                }
            }
            Expr::Closure { params, body, span } => {
                // Capture, by value, the names the body could mention — not
                // the whole visible environment. Capturing everything put a
                // second owner on every list in scope, so the next
                // `xs.push(v)` copied instead of mutating and a loop that
                // created a closure was quadratic.
                let mut wanted = std::collections::HashSet::new();
                mentioned_names(body, &mut wanted);
                let mut env: Vec<(String, Value)> = Vec::new();
                for scope in &self.scopes {
                    for (k, v) in scope {
                        if !wanted.contains(k.as_str()) {
                            continue;
                        }
                        env.retain(|(ek, _)| ek != k);
                        env.push((k.clone(), v.clone()));
                    }
                }
                let cb = match &**body {
                    Expr::Block(b) | Expr::Unsafe(b) | Expr::Region(b) => ClosureBody::Block(b.clone()),
                    other => ClosureBody::Expr(other.clone()),
                };
                Ok(Value::Closure {
                    params: Rc::new(params.clone()),
                    body: Rc::new(cb),
                    env: Rc::new(env),
                    module: Rc::new(self.current_module.clone()),
                    fallible: self.fallible_closures.contains(&span.start),
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
            if let Some(v) = self.const_cache.get(&qual) {
                return Ok(v.clone());
            }
            let v = self.eval_expr(&c.init)?;
            self.const_cache.insert(qual, v.clone());
            return Ok(v);
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
                                //
                                // A generation is 32 bits, and wrapping it
                                // would hand a slot's *identity* back: after
                                // 2^32 release/allocate cycles a reference
                                // released at the start of the program was
                                // reported alive, read the current occupant's
                                // value, and compared equal to a live one —
                                // the use-after-free [T-REF-STALE] exists to
                                // rule out. So a slot whose generation cannot
                                // advance is retired instead of reused: it
                                // never has a new occupant, so nothing can
                                // ever match a reference to it again.
                                match self.slab[idx].1.checked_add(1) {
                                    Some(next) => {
                                        self.slab[idx].1 = next;
                                        self.slab_free.push(i);
                                    }
                                    None => {}
                                }
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
                        // The low eight bits, as the operating system reports it.
                        return Err(Flow::Exit((code & 0xff) as i32));
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
                    let modes: Vec<ParamMode> = f.params.iter().map(|p| p.mode).collect();
                    let (vs, paths) = self.eval_call_args_paths(&modes, args, None)?;
                    let r = self.call_decl(f, &module, vs)?;
                    let arg_refs: Vec<&Expr> = args.iter().collect();
                    self.apply_writebacks_at(&arg_refs, &paths)?;
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
                        if let Some(v) = self.const_cache.get(&qual) {
                            return Ok(v.clone());
                        }
                        // Evaluated in the module that declared it, so its own
                        // names mean what they meant there.
                        let saved = std::mem::replace(&mut self.current_module, target_mod);
                        let v = self.eval_expr(&c.init);
                        self.current_module = saved;
                        if let Ok(v) = &v {
                            self.const_cache.insert(qual, v.clone());
                        }
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
                        let modes: Vec<ParamMode> = f.params.iter().map(|p| p.mode).collect();
                        let (vs, paths) = self.eval_call_args_paths(&modes, args, None)?;
                        let r = self.call_decl(f, &target_mod, vs)?;
                        let arg_refs: Vec<&Expr> = args.iter().collect();
                        self.apply_writebacks_at(&arg_refs, &paths)?;
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
        if name.name == "sort_by" {
            // Resolve the receiver's place once. Reading it with `eval_expr`
            // and writing it back with `assign_place` walked the path twice,
            // so `xs[next()].sort_by(f)` ran `next()` twice ([E-ORDER-LTR]).
            let segs = self.place_segs(recv)?;
            let current = match &segs {
                Some(s) => self.read_rooted(recv, s)?,
                None => self.eval_expr(recv)?,
            };
            let Value::List(items) = current else {
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
            let sorted = Value::List(Rc::new(sorted));
            match segs {
                Some(s) => {
                    let mut root = recv;
                    while let Expr::Field { recv, .. } | Expr::Index { recv, .. } = root {
                        root = recv;
                    }
                    match root {
                        Expr::Ident(id) => {
                            let id = id.clone();
                            self.assign_rooted(&id, s, sorted)?;
                        }
                        _ => self.assign_place(recv, sorted)?,
                    }
                }
                None => self.assign_place(recv, sorted)?,
            }
            return Ok(Value::Unit);
        }
        if matches!(
            name.name.as_str(),
            "push" | "pop" | "sort" | "set" | "delete" | "remove"
        ) {
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
            let modes: Vec<ParamMode> = f.params.iter().map(|p| p.mode).collect();
            let (vs, paths) = self.eval_call_args_paths(&modes, args, Some(recv_v))?;
            let r = self.call_decl(f, &module, vs)?;
            let mut arg_refs: Vec<&Expr> = Vec::with_capacity(args.len() + 1);
            arg_refs.push(recv);
            arg_refs.extend(args.iter());
            self.apply_writebacks_at(&arg_refs, &paths)?;
            return Ok(r);
        }
        // A field holding a function, called through the field. The checker
        // accepts this, so refusing here would be a well-typed program failing
        // at run time.
        // A tuple's components are fields spelled with numbers, and a
        // reference is read through.
        let callee = match &self.deref_value(recv_v.clone())? {
            Value::Struct { fields, .. } => fields
                .iter()
                .find(|(n, _)| *n == name.name)
                .map(|(_, f)| f.clone()),
            Value::Tuple(items) => name
                .name
                .parse::<usize>()
                .ok()
                .and_then(|i| items.get(i).cloned()),
            _ => None,
        };
        if let Some(callee) = callee {
            let mut vs = Vec::with_capacity(args.len());
            for a in args {
                vs.push(self.eval_expr(a)?);
            }
            return self.call_value(callee, vs, &name.name);
        }
        Err(Flow::Panic(format!(
            "no method `{}` on `{recv_v}`",
            name.name
        )))
    }

    /// The same, also returning each `inout` argument's resolved path.
    ///
    /// The writeback must land where the snapshot came from. Re-walking the
    /// place ran its index expression a second time, so
    /// `push_one(g[idx.remove(0)])` read one element and wrote another —
    /// leaving a `[[i64]]` with a `()` in it, which then failed `len`
    /// ([E-ORDER-LTR]).
    fn eval_call_args_paths(
        &mut self,
        modes: &[ParamMode],
        args: &[Expr],
        recv: Option<Value>,
    ) -> Result<(Vec<Value>, Vec<Option<Vec<Seg>>>), Flow> {
        let shift = usize::from(recv.is_some());
        let mut vs = Vec::with_capacity(args.len() + shift);
        let mut paths: Vec<Option<Vec<Seg>>> = vec![None; args.len() + shift];
        if let Some(r) = recv {
            vs.push(r);
        }
        for (i, a) in args.iter().enumerate() {
            let takeable = modes.get(i + shift) == Some(&ParamMode::Inout)
                && place_root_name(a).is_some_and(|root| {
                    !args
                        .iter()
                        .enumerate()
                        .any(|(j, other)| j != i && mentions_ident(other, &root))
                });
            // An `inout` place is resolved once, whether or not the value
            // is moved out of it, and the writeback uses that.
            let v = if modes.get(i + shift) == Some(&ParamMode::Inout) {
                let segs = self.place_segs(a)?;
                paths[i + shift] = segs.clone();
                match (takeable, segs) {
                    (true, Some(segs)) => match self.take_rooted(a, &segs) {
                        Some(v) => v,
                        None => self.eval_expr(a)?,
                    },
                    _ => self.eval_expr(a)?,
                }
            } else {
                self.eval_expr(a)?
            };
            vs.push(v);
        }
        Ok((vs, paths))
    }

    /// Moves the value out of an already-resolved place.
    fn take_rooted(&mut self, target: &Expr, segs: &[Seg]) -> Option<Value> {
        let mut root = target;
        while let Expr::Field { recv, .. } | Expr::Index { recv, .. } = root {
            root = recv;
        }
        let Expr::Ident(id) = root else { return None };
        let path: Vec<PathSeg> = segs
            .iter()
            .map(|s| match s {
                Seg::Field(f) => PathSeg::Field(f.clone()),
                Seg::Index(i) => PathSeg::Index(*i as i64),
            })
            .collect();
        let name = id.name.clone();
        self.slot_mut(&name, &path)
            .map(|slot| std::mem::replace(slot, Value::Unit))
    }

    /// Resolves a place expression to a root binding and a path of steps.
    ///
    /// `None` when the expression is not a place rooted at a binding, or when
    /// an index is not an integer — a map key is not a step this walk takes.
    fn place_path(&mut self, e: &Expr) -> Result<Option<(String, Vec<PathSeg>)>, Flow> {
        // Collected first, evaluated after, so the indices run root-to-leaf —
        // left to right as written. Evaluating while walking inward ran
        // `gg[outer][inner].push(x)` as `inner` then `outer` ([E-ORDER-LTR]).
        enum Step<'e> {
            Field(String),
            Index(&'e Expr),
        }
        let mut steps: Vec<Step> = Vec::new();
        let mut cur = e;
        let root;
        loop {
            match cur {
                Expr::Field { recv, name, .. } => {
                    steps.push(Step::Field(name.name.clone()));
                    cur = recv;
                }
                Expr::Index { recv, index, .. } => {
                    steps.push(Step::Index(index));
                    cur = recv;
                }
                Expr::Ident(id) => {
                    root = id.name.clone();
                    break;
                }
                _ => return Ok(None),
            }
        }
        steps.reverse();
        let mut segs: Vec<PathSeg> = Vec::with_capacity(steps.len());
        for step in steps {
            match step {
                Step::Field(f) => segs.push(PathSeg::Field(f)),
                Step::Index(ix) => {
                    let Value::Int(i) = self.eval_expr(ix)? else {
                        return Ok(None);
                    };
                    segs.push(PathSeg::Index(i));
                }
            }
        }
        Ok(Some((root, segs)))
    }

    /// The slot a path names, ready to be written through.
    ///
    /// `Rc::make_mut` at each step is what keeps value semantics: a container
    /// shared with anything else is copied here, and one that is not is
    /// mutated where it lies.
    fn slot_mut(&mut self, root: &str, path: &[PathSeg]) -> Option<&mut Value> {
        // A `ref` root lives in the slab, not in the binding. Reading the
        // slot id first — it is two integers, and copying it ends the borrow
        // on the scopes — is what lets the walk continue into the shared
        // heap, so mutating through a reference is as cheap as mutating a
        // local. Without it every `node.edges.push(e)` copied the list.
        let via_ref = match self.scopes.iter().rev().find_map(|s| s.get(root)) {
            Some(Value::Ref(i, g)) if !path.is_empty() => Some((*i, *g)),
            _ => None,
        };
        let mut cur = match via_ref {
            Some((i, g)) => {
                let idx = i as usize;
                match self.slab.get_mut(idx) {
                    Some((Some(v), gen)) if *gen == g => v,
                    // A stale reference is a panic, and this is not the place
                    // that reports one — the slow path does, with its message.
                    _ => return None,
                }
            }
            None => self
                .scopes
                .iter_mut()
                .rev()
                .find_map(|s| s.get_mut(root))?,
        };
        for seg in path {
            cur = match (cur, seg) {
                (Value::Struct { fields, .. }, PathSeg::Field(f)) => Rc::make_mut(fields)
                    .iter_mut()
                    .find(|(n, _)| n == f)
                    .map(|(_, v)| v)?,
                (Value::Tuple(items), PathSeg::Field(f)) => {
                    let i: usize = f.parse().ok()?;
                    Rc::make_mut(items).get_mut(i)?
                }
                (Value::List(items), PathSeg::Index(i)) => {
                    let items = Rc::make_mut(items);
                    let n = items.len() as i64;
                    // A negative index counts from the end, as everywhere else.
                    let i = if *i < 0 { n + *i } else { *i };
                    if i < 0 || i >= n {
                        return None;
                    }
                    &mut items[i as usize]
                }
                _ => return None,
            };
        }
        Some(cur)
    }

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
            // Same mutator table as the nested case below. It used to be a
            // second, shorter copy that knew only `push`/`pop`/`sort`, so
            // `xs.remove(i)` on a plain variable fell through to a
            // read-modify-write that evaluated the receiver twice.
            if let Some(slot) = slot {
                if let Some(r) = mutate_in_place(slot, &name.name, &mut arg_vals)? {
                    return Ok(Some(r));
                }
            }
            return Ok(None);
        }

        // Nested place (`bag.items.push(v)` / `obj.cache.set(k, v)`).
        //
        // Walking to the slot and mutating it through `Rc::make_mut` is what
        // makes this O(1) amortized like the bare-variable case above. It
        // used to read the whole collection, clone it, and assign the clone
        // back — so `s.xs.push(i)` in a loop was O(n) per push, and 40 000
        // pushes took 3.5 seconds against 0.01 for the same loop on a plain
        // variable. Value semantics are unaffected: `make_mut` copies exactly
        // when the value is shared, which is the rule the fast path and the
        // compiled backend already follow ([E-COW]).
        if let Some(path) = self.place_path(recv)? {
            let mut arg_vals = Vec::with_capacity(args.len());
            for a in args {
                arg_vals.push(self.eval_expr(a)?);
            }
            if let Some(slot) = self.slot_mut(&path.0, &path.1) {
                if let Some(r) = mutate_in_place(slot, &name.name, &mut arg_vals)? {
                    return Ok(Some(r));
                }
            }
        }
        // Anything the walk could not reach — a place rooted somewhere other
        // than a binding, or a shape with no in-place mutator: read, modify,
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
                // Folded from the first element, not from a `+0.0` seed:
                // `-0.0 + -0.0` is `-0.0` under IEEE-754, and seeding with
                // `+0.0` turned it into `+0.0`. `-0.0` is an observable
                // rendering ([L-DISPLAY]), so the two engines printed
                // different answers for `[-0.0].sum()` ([E-ARITH-FLOAT]).
                let mut float_sum: Option<f64> = None;
                let mut is_float = false;
                for item in items.iter() {
                    match item {
                        Value::Int(i) => int_sum += i,
                        Value::Float(f) => {
                            is_float = true;
                            float_sum = Some(match float_sum {
                                Some(acc) => acc + f,
                                None => *f,
                            });
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
                    match float_sum {
                        Some(f) if int_sum == 0 => Value::Float(f),
                        Some(f) => Value::Float(f + int_sum as f64),
                        None => Value::Float(int_sum as f64),
                    }
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
            (Value::Str(s), "len") => Value::Int(s.char_len() as i64),
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
                let len = s.char_len();
                // A width past what a string can hold is a panic with a
                // sentence, not the allocator's own `memory allocation of
                // 9223372036854775805 bytes failed` and SIGABRT — the same
                // class as `repeat` ([E-PANIC-EXIT]).
                if width > MAX_ALLOC {
                    return Err(Flow::Panic(format!(
                        "pad width {width} is larger than a string can hold"
                    )));
                }
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
                    // A count that cannot fit in memory is a panic with a
                    // message, not a Rust `capacity overflow` and a
                    // 19-frame backtrace ([E-PANIC-EXIT]).
                    Value::Int(k) if k >= 0 => {
                        match (k as usize).checked_mul(s.len()) {
                            Some(bytes) if bytes <= MAX_ALLOC => Value::str(s.repeat(k as usize)),
                            _ => {
                                return Err(Flow::Panic(format!(
                                    "repeat would need more memory than a string can hold ({k} copies)"
                                )))
                            }
                        }
                    }
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
                        // Through the same character index `code_at` uses, so
                        // a `substr(i, 1)` scan is linear rather than
                        // quadratic. Collecting the characters on every call
                        // cost O(n) whatever the slice.
                        let n = s.char_len() as i64;
                        let start = start.clamp(0, n) as usize;
                        // Saturating: `substr(2, i64::MAX)` overflowed to a
                        // negative end and panicked inside Rust with a slice
                        // message and a developer's absolute path, where
                        // [L-LIB-STR] says `substr` clamps.
                        let end = (start as i64).saturating_add(len.max(0)).clamp(0, n) as usize;
                        let (a, b) = (
                            s.byte_at(start).unwrap_or(0),
                            s.byte_at(end).unwrap_or_else(|| s.as_str().len()),
                        );
                        Value::str(s.as_str()[a..b].to_string())
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
                let n = s.char_len() as i64;
                if i < 0 || i >= n {
                    return Err(Flow::Panic(format!(
                        "code_at {i} out of bounds (len {n})"
                    )));
                }
                // Constant time whatever the text: ASCII indexes directly,
                // and anything else builds a character index once. Walking
                // made `code_at` cost O(i), so a loop reading a string left
                // to right was quadratic — and that loop is what
                // [L-LIB-CODES] recommends the method for.
                Value::Int(s.char_at(i as usize).expect("in bounds") as i64)
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
            (Value::Int(i), "abs") => Value::Int(i.wrapping_abs()),
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
