//! The Grasm typechecker: name resolution, bidirectional inference,
//! exhaustiveness, fallibility (`T ! E`), mutability of assignment targets.

use std::collections::HashMap;

/// A fast hasher for the checker's own tables, which are keyed by span — a
/// pair of integers this pass produced itself.
///
/// The default hasher is cryptographic, because a `HashMap` in a program that
/// takes untrusted keys has to resist an attacker choosing collisions. None of
/// these do: the keys are byte offsets in a file the compiler is already
/// reading, and an attacker who can choose them can choose the whole program.
/// Hashing them with SipHash showed up as 5% of `grasm check` on a large file.
#[derive(Default, Clone, Copy)]
pub struct SpanHasher(u64);

impl std::hash::Hasher for SpanHasher {
    fn finish(&self) -> u64 {
        self.0
    }
    fn write(&mut self, bytes: &[u8]) {
        for b in bytes {
            self.write_u8(*b);
        }
    }
    fn write_u8(&mut self, n: u8) {
        self.write_u64(n as u64);
    }
    fn write_u32(&mut self, n: u32) {
        self.write_u64(n as u64);
    }
    fn write_u64(&mut self, n: u64) {
        // The multiply-xor-rotate of FxHash, the hasher rustc uses for exactly
        // this: a good enough spread over integer keys for a fraction of the
        // cost.
        self.0 = (self.0.rotate_left(5) ^ n).wrapping_mul(0x51_7c_c1_b7_27_22_0a_95);
    }
    fn write_usize(&mut self, n: usize) {
        self.write_u64(n as u64);
    }
}

#[derive(Default, Clone, Copy)]
pub struct SpanHash;

impl std::hash::BuildHasher for SpanHash {
    type Hasher = SpanHasher;
    fn build_hasher(&self) -> SpanHasher {
        SpanHasher(0)
    }
}

/// A map keyed by span, hashed cheaply. See `SpanHasher`.
pub type SpanMap<V> = HashMap<(u32, u32), V, SpanHash>;
/// A set of spans, hashed cheaply.
pub type SpanSet = std::collections::HashSet<(u32, u32), SpanHash>;

use grasm_syntax::ast::*;
use grasm_syntax::diag::Diagnostic;
use grasm_syntax::span::Span;

use crate::types::*;

pub struct Analysis {
    pub defs: TypeDefs,
    /// Every expression's type, keyed by `(span.start, span.end)`, written the
    /// way the source would write it.
    ///
    /// The backend needs the type of an expression wherever it has to be
    /// *inferred* rather than read — a list element, a match arm, an argument,
    /// a `??`. It had been working them out for itself, in a second
    /// implementation that drifted from this one nine times
    /// (`docs/probe-removal.md`).
    ///
    /// Rendered rather than handed over as a `Type`, because that is the
    /// interface `generic_ctor_types` already uses and it keeps the backend
    /// with **one** function that turns a written type into its own: the
    /// string is parsed and fed to `vtype_of`. Handing over a `Type` would
    /// need a second converter in the backend, which is the shape of the
    /// problem being removed.
    ///
    /// A type still holding an unresolved variable renders with `_` and is
    /// left out: an answer the checker did not settle is not an answer.
    pub resolved_types: SpanMap<String>,
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
    /// The width each fixed-width integer operation wraps to ([T-INT-FIXED]),
    /// keyed by the span of the expression -- or the `x += …` statement --
    /// that computes it, as an [`IntKind::tag`].
    ///
    /// The engines hold every integer in 64 bits and have no types of their
    /// own, so an operation on two `u8`s computes the `i64` answer; this is
    /// what tells them to wrap it to eight bits. An `i64` program has nothing
    /// in it and pays nothing for it.
    pub narrow_ints: std::collections::HashMap<(u32, u32), u8>,
    /// Start offsets of expressions that give the bare ok half where the whole
    /// `T ! E` is wanted, and so have to be wrapped in `Ok` at run time.
    ///
    /// [T-OK-ERR-RETURN]: "the branches giving only the ok half are the ones
    /// wrapped". A fallible *function* re-wraps on the way out, which masked
    /// this everywhere except the positions that do not — a `let` annotation,
    /// a struct field, a list element. There the interpreter stored the raw
    /// value: `let a: i64 ! E = if true { 1 } else { Ok(2) }` printed `1`
    /// rather than `Ok(1)`, compared unequal to `Ok(1)`, and matched no arm of
    /// a `match` over it — while a native build, which wraps by produced type,
    /// answered correctly. The normative engine was the wrong one.
    /// Keyed by the *whole* span, not its start: a binary expression and its
    /// leftmost leaf begin at the same offset, so `0 - 7` recorded as a
    /// wrapped branch also matched the `0` inside it — and the interpreter
    /// wrapped that instead, then tried to subtract `7` from `Ok(0)`.
    pub ok_wrapped: std::collections::HashSet<(u32, u32)>,
    /// For each `m.get(k)` whose receiver is a map, the map's **value type**,
    /// encoded as one byte.
    ///
    /// The backend's own type probe is structural, so for `var m = map()` it
    /// sees `map()` and learns nothing — the real type comes from how the map
    /// is used later, which only the checker knows. Without this the
    /// `m.get(k) ?? d` fusion fires on annotated maps and silently declines on
    /// inferred ones, which is the common case.
    ///
    /// One byte rather than a `Type`, so the backend needs no general
    /// `Type` -> `VType` conversion for a table this narrow. `0` = i64,
    /// `1` = f64, `2` = bool, `3` = str, `4` = bytes; anything else is absent
    /// and the fusion declines.
    pub map_get_value_kind: std::collections::HashMap<(u32, u32), u8>,
    /// The effect row of every function-valued expression, and of every call
    /// whose callee is not an expression of its own. See `value_tables`.
    pub value_rows: SpanMap<std::collections::BTreeSet<String>>,
    pub callee_rows: SpanMap<std::collections::BTreeSet<String>>,
    /// Method-call spans the checker resolved to a *struct field* holding a
    /// function, rather than to a method or a UFCS free function. The engines
    /// resolve by name and would otherwise disagree with the types.
    pub field_calls: SpanSet,
    /// Method-call spans that resolve through an `impl` ([T-INTERFACE]). The
    /// engines look the method up by the receiver's type in `impl_fns`
    /// rather than by name, since two types may each have one.
    pub iface_calls: SpanSet,
    /// `(type, method)` -> the qualified function an `impl` lowered it to,
    /// `module::method@Type`. A type is named as its `impl` wrote it:
    /// `Point`, `i64`, `f64`, `bool`, `str`.
    pub impl_fns: HashMap<(String, String), String>,
    /// Start offsets of closure literals whose own type is fallible.
    ///
    /// `fail(e)` raises the error branch of the enclosing *fallible* function,
    /// and a closure is one only when its type says so. Catching every
    /// failure at every closure boundary made
    /// `xs.par_map(|x| if c { fail(e) } else { x })` produce a list holding an
    /// `Err` instead of failing ([E-PAR-FAIL]); catching none of them let a
    /// `fail` inside `let f: fn(i64) -> str ! E = |n| …` unwind past the
    /// `??` written to handle it. Only the checker knows which is which.
    pub fallible_closures: std::collections::HashSet<u32>,
    /// Start offset of each closure literal → the parameter types inference
    /// settled on, written the way a person would.
    ///
    /// A closure whose parameters nothing local fixes learns them from the call
    /// site, and the checker is the pass that reaches both. The compiled
    /// backend probes the closure body instead, so `let f = |p| p.0 + p.1`
    /// called as `f((i, v))` ran under `grasm run` and was refused by name.
    ///
    /// Present only when every parameter resolved: half an answer would turn a
    /// refusal the backend can explain into a miscompile it cannot.
    pub closure_param_types: std::collections::HashMap<u32, Vec<String>>,
    /// Start offset of each generic sum constructor → the type it built, once
    /// inference settled it.
    ///
    /// `let a = Good(1)` followed by `take(a)` where `take` wants a
    /// `Res[i64, str]` is determined — by the later line, which the checker
    /// reaches and a backend inferring from the constructor's arguments alone
    /// does not. Present only when every parameter resolved ([T-GENERIC-TYPE]).
    pub generic_ctor_types: std::collections::HashMap<u32, String>,
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
    /// `parallel` blocks whose tasks are proved independent, by the block's
    /// span start ([E-SPAWN-ORDER] under the as-if rule). A compiled build
    /// may run their tasks on several cores: no order of running them could
    /// be told apart from the written one. See `parallel_independent`.
    pub parallel_ok: std::collections::HashSet<u32>,
    /// Every raw-memory operation ([L-LIB-MEM], [L-LIB-ATOMIC]), by the
    /// call's whole span. The engines dispatch on this rather than on a
    /// method's name: natively an `addr` is an `i64`, and a user's UFCS
    /// `fn offset(n: i64, ..)` is not pointer arithmetic. A layout's type and
    /// an atomic's ordering are written as names, not values, and live here.
    pub mem_ops: HashMap<(u32, u32), crate::mem::MemOp>,
    /// The C boundary ([T-FN-ADDR], [L-LIB-CALL-C], [T-EXPORT]): each
    /// `mem_fn_addr` and `call_c`, the C signatures, the exports.
    pub c_facts: crate::mem::CFacts,
    /// What the checker decided, for TIR (`docs/tir.md`). Present only when
    /// the program was checked with [`check_program_tir`].
    pub tir: Option<crate::tirfacts::TirFacts>,
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

/// The rows the checker worked out for function-valued expressions and for
/// call sites whose callee is not an expression of its own.
///
/// The package-level entry points in `effects` compute rows without going
/// through the checker, so without this they would be back to guessing where a
/// function value came from — and a `capabilities = []` library taking a
/// `[fn(i64) -> i64 / {io, unsafe}]` and calling one element would be
/// certified as performing nothing.
pub type ValueTables = (
    SpanMap<std::collections::BTreeSet<String>>,
    SpanMap<std::collections::BTreeSet<String>>,
);

pub fn value_tables(program: &crate::loader::Program) -> ValueTables {
    let a = check_program(program);
    (a.value_rows, a.callee_rows)
}

/// Every method a range answers, for the diagnostic that names them.
///
/// A range is the one receiver whose type cannot be written down — `fn f(r:
/// range)` is "unknown type `range`" — so the ordinary "a free function `fn
/// f(range, ...)` would also work" help proposes an impossible signature. This
/// list is what it says instead, and `crates/grasm_sema/tests/check_tests.rs`
/// checks that each name here really does type against a range.
const RANGE_METHODS: &[&str] = &["to_list", "map", "filter", "sum", "len"];

/// Whether a wrong name is close enough to a right one to suggest it.
///
/// A prefix, a suffix, or one edit apart. Deliberately not a full edit-distance
/// table: the point is to catch `count` for `len` and `collect` for `to_list`,
/// and a suggestion that is merely *nearest* among five candidates is a guess
/// dressed as help.
fn looks_like(candidate: &str, written: &str) -> bool {
    if candidate == written {
        return false;
    }
    if candidate.starts_with(written) || written.starts_with(candidate) {
        return true;
    }
    // One insertion, deletion or substitution.
    let (a, b): (Vec<char>, Vec<char>) = (candidate.chars().collect(), written.chars().collect());
    if a.len().abs_diff(b.len()) > 1 {
        return false;
    }
    let (long, short) = if a.len() >= b.len() {
        (&a, &b)
    } else {
        (&b, &a)
    };
    let mut skipped = false;
    let mut i = 0;
    for &c in long.iter() {
        if i < short.len() && short[i] == c {
            i += 1;
        } else if skipped {
            return false;
        } else {
            skipped = true;
            // A substitution consumes a character on both sides.
            if long.len() == short.len() {
                i += 1;
            }
        }
    }
    true
}

/// The language's own free functions. Not methods: [L-UFCS] resolves method
/// syntax against the receiver type's methods and then against functions a
/// *program* declares, and `print` is a method of nothing.
pub const BUILTIN_FREE_FNS: &[&str] = &[
    "chan_new",
    "chan_send",
    "chan_recv",
    "chan_select",
    "chan_close",
    "print",
    "eprint",
    "tcp_listen",
    "tcp_listen_on",
    "tcp_connect",
    "tcp_accept",
    "tcp_read",
    "tcp_read_bytes",
    "tcp_write",
    "tcp_write_bytes",
    "net_close",
    "net_wait",
    "net_workers",
    "net_port",
    "tcp_peer",
    "udp_bind",
    "udp_send",
    "udp_recv",
    "udp_recv_from",
    "proc_run",
    "proc_pipeline",
    "read_file",
    "write_file",
    "file_exists",
    "remove_file",
    "make_dir",
    "make_dir_new",
    "bytes_of",
    "bytes_filled",
    "str_builder",
    "byte_builder",
    "mem_alloc",
    "mem_free",
    "mem_addr",
    "mem_size_of",
    "mem_align_of",
    "mem_fn_addr",
    "mem_tls",
    "read_bytes",
    "read_bytes_at",
    "append_bytes",
    "file_size",
    "sync_file",
    "rename_file",
    "write_bytes",
    "list_dir",
    "input",
    "args",
    "now_millis",
    "monotonic_millis",
    "monotonic_nanos",
    "sleep_millis",
    "random_bytes",
    "env",
    "exit",
    "panic",
    "assert",
    "assert_eq",
];

/// The one-byte encoding of a map's value type, for `map_get_value_kind`.
///
/// Deliberately narrow: only the types the backend can carry through the fused
/// `m.get(k) ?? d` path without a general `Type` -> `VType` conversion. Anything
/// else answers `None` and the fusion declines, which costs an allocation and
/// never a wrong answer.
fn map_value_kind(t: &Type) -> Option<u8> {
    Some(match t {
        Type::I64 => 0,
        Type::F64 => 1,
        Type::Bool => 2,
        Type::Str => 3,
        Type::Bytes => 4,
        _ => return None,
    })
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
    check_modules_in(&files, &program.deps.iter().cloned().collect(), false)
}

/// [`check_program`], recording every decision TIR needs
/// ([`crate::tirfacts`]). Checking is otherwise the same: the same
/// diagnostics, the same tables.
pub fn check_program_tir(program: &crate::loader::Program) -> Analysis {
    let files: Vec<ModuleView> = program
        .files
        .iter()
        .map(|f| ModuleView {
            name: f.name.clone(),
            module: &f.module,
            imports: f.imports.clone(),
        })
        .collect();
    check_modules_in(&files, &program.deps.iter().cloned().collect(), true)
}

fn check_modules(files: &[ModuleView]) -> Analysis {
    check_modules_in(files, &std::collections::BTreeSet::new(), false)
}

/// `check_modules`, told which module names are dependency packages.
///
/// Only the effect pass needs it, and only to decide where a handler's own
/// effects are charged: a handler written by a *consumer* runs on the
/// consumer's authority ([E-HANDLER-OWN-ROW]). The capability pass was told
/// and the row pass was not, so one `grasm deps` run reported
/// `lib performs []  permits []` — no violation — while refusing the same
/// function's declared row for a `print` written entirely in the consumer.
fn check_modules_in(
    files: &[ModuleView],
    packages: &std::collections::BTreeSet<String>,
    record_tir: bool,
) -> Analysis {
    let mut c = Checker::new();
    c.record_tir = record_tir;
    // The effect fixpoint runs first: a function's row is part of its type,
    // and nothing can be typed until the rows are known ([T-FN-TYPE-ROW]).
    let (rows, closure_rows) = crate::effects::rows_and_closures(files);
    c.fn_rows = rows;
    c.closure_rows = closure_rows;
    for f in files {
        c.module_imports.insert(f.name.clone(), f.imports.clone());
        c.wide_ints.extend(f.module.wide_ints.iter().copied());
    }
    for f in files {
        c.current_module = f.name.clone();
        c.collect_linear(f.module);
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
    // Aliases last of the three, because one may name a type from any module
    // and one may name a handler type an `effect` declares. Resolving them
    // where they were collected gave each alias only what its own module had
    // registered by then ([T-MODULE-NS], F68).
    c.resolve_type_aliases(&modules);
    c.resolve_type_bodies(&modules);
    for f in files {
        c.current_module = f.name.clone();
        c.check_linear_fields(f.module);
    }
    for f in files {
        c.current_module = f.name.clone();
        c.collect_interfaces(f.module);
    }
    for f in files {
        c.current_module = f.name.clone();
        c.collect_fns(f.module);
    }
    for f in files {
        c.current_module = f.name.clone();
        c.collect_impls(f.module);
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
        c.collect_thread_locals(f.module);
    }
    for f in files {
        c.current_module = f.name.clone();
        c.check_module(f.module);
    }
    // Every body has been walked, so a generic's own use of its type
    // parameters as map keys is known ([T-MAP-KEY-GENERIC]).
    c.flush_key_checks();
    c.flush_par_checks();
    c.flush_par_name_checks();
    c.flush_discards();
    // Resolved now: unification is finished, so a literal's element type has
    // whatever answer the whole body gave it.
    let mut unresolved: Vec<(u32, u32)> = Vec::new();
    // One literal can be recorded twice — inferred once to learn the
    // receiver's shape, then checked again against the parameter it feeds.
    // Only the second attempt resolves, and reporting the first made
    // `[].total()` "not determined by anything" while `total([])`, the same
    // call one spelling apart, compiled ([L-UFCS], [T-EMPTY-LIT]).
    let resolved_once: std::collections::HashSet<u32> = c
        .empty_lits
        .iter()
        .filter(|(_, _, t)| !c.show(t).contains('_'))
        .map(|(off, _, _)| *off)
        .collect();
    // Only the ones inference actually settled. A type still holding a
    // variable is not an answer, and handing the backend half of one would
    // turn a refusal it can explain into a miscompile it cannot.
    let closure_param_types: std::collections::HashMap<u32, Vec<String>> = c
        .closure_params
        .iter()
        .filter_map(|(off, ts)| {
            let shown: Vec<String> = ts.iter().map(|t| c.show(t)).collect();
            if shown.iter().any(|s| s.contains('_')) {
                return None;
            }
            Some((*off, shown))
        })
        .collect();
    let empty_types: std::collections::HashMap<u32, String> = c
        .empty_lits
        .clone()
        .into_iter()
        .filter_map(|(off, len, t)| {
            let shown = c.show(&t);
            if shown.contains('_') && !resolved_once.contains(&off) {
                // Nothing in the program says what this holds.
                // `[T-EMPTY-LIT]` says such a literal is rejected, and it was
                // not: `var xs = []` with no later use ran under `grasm run`
                // and could not be built — the one divergence the clause
                // records as fixed.
                unresolved.push((off, len));
                return None;
            }
            Some((off, shown))
        })
        .collect();
    // [T-GENERIC-TYPE]: a constructor that leaves a type parameter undetermined.
    //
    // Judged here rather than at the constructor, because a later line may fix
    // it — `let r = Good(1)` followed by a use that needs `Res[i64, str]` is
    // determined, just not yet. The same reason the empty literals below wait.
    //
    // This was the language's rule with one engine of three enforcing it: the
    // checker passed the program, `grasm run` ran it, and `--native` refused it
    // because a compiled build needs one machine representation and has nothing
    // to choose it from.
    let generic_ctor_types: std::collections::HashMap<u32, String> = c
        .generic_ctors
        .iter()
        .filter_map(|(off, _, t)| {
            let shown = c.show(t);
            if shown.contains('_') {
                return None;
            }
            Some((*off, shown))
        })
        .collect();
    for (off, len, t) in c.generic_ctors.clone() {
        let shown = c.show(&t);
        if !shown.contains('_') {
            continue;
        }
        c.diags.push(
            Diagnostic::error(
                Span::new(off, off + len),
                format!("`{shown}` leaves a type parameter undetermined"),
            )
            .with_help(
                "a constructor fixes only the parameters its arguments mention; annotate the binding, as in `let r: Res[i64, str] = Good(1)`",
            ),
        );
    }
    for (off, len) in unresolved {
        c.diags.push(
            Diagnostic::error(
                Span::new(off, off + len),
                "this empty literal's element type is not determined by anything".to_string(),
            )
            .with_help(
                "annotate it, as in `let xs: [i64] = []` or `let m: Map[str, i64] = map()`, or use it somewhere that says what it holds",
            ),
        );
    }
    // Every expression the checker settled on a fixed-width integer type,
    // plus the compound assignments, which are statements.
    let mut narrow_ints: std::collections::HashMap<(u32, u32), u8> = c
        .resolved_types
        .iter()
        .filter_map(|(k, t)| match c.uni.resolve(t) {
            Type::Int(kind) => Some((*k, kind.tag())),
            _ => None,
        })
        .collect();
    narrow_ints.extend(c.narrow_assigns.iter().map(|(k, kind)| (*k, kind.tag())));
    // Rendered before `c.diags` is moved out, since `show` borrows `c`.
    let resolved_types: SpanMap<String> = c
        .resolved_types
        .clone()
        .into_iter()
        .filter_map(|(k, t)| {
            let shown = c.show(&t);
            if shown.contains('_') {
                return None;
            }
            Some((k, shown))
        })
        .collect();
    let tir = if c.record_tir {
        Some(c.finish_tir())
    } else {
        None
    };
    let mut diagnostics = c.diags;
    diagnostics.extend(crate::effects::check_effects_multi_with(
        files,
        packages,
        &c.value_rows,
        &c.callee_rows,
    ));
    // Source order. The effect pass walks a hash map, so the *same* file
    // reported its errors in a different order on every run — and `grasm run`
    // and `grasm build` could disagree about the order of the same two — which
    // makes compiler output undiffable in CI and in any `--check` gate. The
    // sort is stable, so a diagnostic and the one deliberately emitted right
    // after it at the same span keep their order.
    diagnostics.sort_by_key(|d| d.span.start);
    // Inference has finished, so the value types recorded at each `m.get(k)`
    // can be resolved now. Doing it at the call site answers "unknown" for
    // every map whose type comes from later use, which is most of them.
    let map_get_value_kind: std::collections::HashMap<(u32, u32), u8> = c
        .map_get_pending
        .iter()
        .filter_map(|(span, t)| map_value_kind(&c.uni.resolve(t)).map(|k| (*span, k)))
        .collect();
    Analysis {
        resolved_types,
        const_types: c
            .consts
            .iter()
            .filter(|(_, (_, is_pub))| *is_pub)
            .map(|(k, (t, _))| {
                (
                    k.clone(),
                    TypeDisplay {
                        ty: t,
                        defs: &c.defs,
                    }
                    .to_string(),
                )
            })
            .collect(),
        float_literals: c.float_literals,
        narrow_ints,
        value_rows: c.value_rows.clone(),
        callee_rows: c.callee_rows.clone(),
        field_calls: c.field_calls.clone(),
        iface_calls: c.iface_calls.clone(),
        impl_fns: c
            .impls
            .iter()
            .map(|(k, (f, _))| (k.clone(), f.clone()))
            .collect(),
        ok_wrapped: c.ok_wrapped,
        map_get_value_kind,
        fallible_closures: c
            .closure_types
            .iter()
            .filter(|(_, t)| matches!(c.uni.shallow_resolve(t), Type::Fallible(..)))
            .map(|(off, _)| *off)
            .collect(),
        closure_param_types,
        generic_ctor_types,
        empty_types,
        float_sums: c.float_sums,
        parallel_ok: c.parallel_ok.clone(),
        mem_ops: c.mem_ops.clone(),
        c_facts: c.c_facts.clone(),
        tir,
        defs: c.defs,
        diagnostics,
    }
}

#[derive(Clone)]
struct Binding {
    ty: Type,
    mutable: bool,
    /// What introduced the name, so a write to an immutable one can say the
    /// thing that actually fixes it. A parameter cannot be `var` — the
    /// grammar has no such parameter — so the generic help sent people to a
    /// program that does not parse.
    kind: BindKind,
    /// Ownership was transferred with `own` — further use is an error.
    moved: bool,
    /// Whether this binding owns its value, so that a linear one must be
    /// consumed before it ends ([T-LINEAR]): a local does, an `own`
    /// parameter does (unless the module declaring its type is the one
    /// ending it), a lent parameter does not.
    owned: bool,
    /// Where it was bound, for the report when a linear value is not consumed.
    at: Span,
    /// Loop nesting depth at declaration (for move-in-loop detection).
    decl_loop_depth: u32,
}

/// What introduced a binding.
#[derive(Clone, Copy, PartialEq)]
enum BindKind {
    /// `let` or `var` in a body.
    Local,
    /// A function parameter: `inout` writes through to the caller's value.
    Param,
    /// A property test's parameter, which the runner supplies
    /// ([G-TEST-PARAMS]). There is no caller to write back to.
    Generated,
}

/// A bounded call's promise, settled once inference has: the variable a
/// bounded parameter was instantiated with, its interfaces, the call, and the
/// calling function's own bounds and parameter names at the time.
/// Which bindings are moved, scope by scope: what a branch leaves behind.
type MoveSnapshot = Vec<Vec<(String, bool)>>;

type BoundObligation = (Type, Vec<String>, Span, Vec<Vec<String>>, Vec<String>);

#[derive(Clone)]
struct FnInfo {
    is_pub: bool,
    generics: usize,
    /// Each generic parameter's interfaces ([T-INTERFACE-BOUND]).
    bounds: Vec<Vec<String>>,
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
    /// Whether to write down each decision for TIR (`check_program_tir`).
    record_tir: bool,
    /// The decisions, as made: types here may still hold inference variables,
    /// which `finish_tir` resolves once checking is done.
    tir: crate::tirfacts::TirFacts,
    defs: TypeDefs,
    aliases: HashMap<String, Type>,
    fns: HashMap<String, FnInfo>,
    /// Every function's inferred effect row, from the effects fixpoint run
    /// before checking. A function's row is part of its type, so this has to
    /// be known before anything is typed.
    fn_rows: HashMap<String, std::collections::BTreeSet<String>>,
    /// See `Analysis::parallel_ok`.
    parallel_ok: std::collections::HashSet<u32>,
    /// Each closure literal's row, keyed by span. See `rows_and_closures`.
    closure_rows: HashMap<u32, std::collections::BTreeSet<String>>,
    /// See `Analysis::ok_wrapped`.
    ok_wrapped: std::collections::HashSet<(u32, u32)>,
    /// `m.get(k)` sites and their value types, resolved once inference ends.
    map_get_pending: Vec<((u32, u32), Type)>,
    /// Every closure literal's return type, by start offset, resolved once
    /// the whole program is typed.
    closure_types: HashMap<u32, Type>,
    /// Every closure literal's start offset and its parameter types.
    ///
    /// Recorded as type *variables* and resolved at the end, because a closure
    /// whose parameters nothing local fixes learns them from the call site —
    /// which the checker reaches later. See `Analysis::closure_param_types`.
    closure_params: Vec<(u32, Vec<Type>)>,
    /// Every constructor of a generic sum, and the type it built.
    ///
    /// Held as inference variables and resolved at the end: `Good(1)` for
    /// `Res[T, E]` fixes `T` at the call and leaves `E` to whatever else the
    /// program says, which may be a later line. See [T-GENERIC-TYPE].
    generic_ctors: Vec<(u32, u32, Type)>,
    /// Closure literals that a built-in higher-order method calls *now* —
    /// `xs.map(|x| …)` and its siblings.
    ///
    /// A `fail` inside a closure that is not itself fallible raises the
    /// enclosing function's error, which is what [E-PAR-FAIL] describes and
    /// is sound exactly while the closure cannot outlive the call. Stored
    /// instead — in a struct field, a list, a returned value — it is called
    /// later, from a function that may declare no error type at all, and the
    /// failure reached `main` at run time.
    hof_closures: std::collections::HashSet<u32>,
    /// The generated `EHandler` types.
    ///
    /// Their operation fields are the one place a function value is *not*
    /// checked against a row — that exemption is what lets
    /// `LogHandler{ info: |m| print(m) }` be written. Reading one back out
    /// turned the exemption into a laundry: `h.info(x)` inside a `/ {}`
    /// function printed, `par_map(h.info)` ran it on several cores, and one
    /// carrying a handled effect escaped its `with` block.
    handler_ids: std::collections::HashSet<TypeDefId>,
    /// How many `unsafe` blocks enclose the expression being checked.
    unsafe_depth: u32,
    /// See `Analysis::mem_ops`.
    mem_ops: HashMap<(u32, u32), crate::mem::MemOp>,
    /// See `Analysis::c_facts`.
    c_facts: crate::mem::CFacts,
    /// Each `export fn`'s symbol, with where it was declared: one name, one
    /// symbol ([T-EXPORT]).
    export_names: std::collections::BTreeMap<String, Span>,
    /// How many `region` blocks enclose it.
    /// Whether the block being checked has its value thrown away.
    ///
    /// A loop body, and a branch of an `if` used as a statement. Their tail
    /// is discarded like any other line, so `if c { write_file(p, s) }`
    /// swallowed a failure that a bare `write_file(p, s)` would not.
    discarding: bool,
    /// Set while checking the expression a function's body ends with, so a
    /// mismatch there is reported as what it is — the function's return value
    /// — rather than as an anonymous type mismatch.
    tail_fn: Option<String>,
    /// The function whose body is being checked, handed to `tail_fn` only
    /// when the block's actual tail expression is reached.
    tail_fn_name: Option<String>,
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
    /// `x += …` statements whose target is a fixed-width integer.
    narrow_assigns: std::collections::HashMap<(u32, u32), IntKind>,
    /// Span starts of decimal literals past `i64` (`Module::wide_ints`).
    wide_ints: std::collections::HashSet<u32>,
    /// The effect row of every expression whose type is a function, keyed by
    /// its whole span. See `record_value_row`.
    value_rows: SpanMap<std::collections::BTreeSet<String>>,
    /// The row of the function a *call* reaches, where that function is not an
    /// expression in its own right — `t.0(1)`, `b.f(1)`. Keyed by the call's
    /// whole span.
    callee_rows: SpanMap<std::collections::BTreeSet<String>>,
    field_calls: SpanSet,
    /// Interfaces by name. They share the program-wide type namespace.
    interfaces: HashMap<String, grasm_syntax::ast::InterfaceDecl>,
    /// `(type, method)` -> (qualified lowered function, interface).
    impls: HashMap<(String, String), (String, String)>,
    /// `(interface, type)` for every `impl`.
    impl_of: std::collections::HashSet<(String, String)>,
    iface_calls: SpanSet,
    /// Linear types by name, with the module that declares each ([T-LINEAR]).
    linear_types: HashMap<String, String>,
    /// Index of the current function's parameter scope, so a `return` can
    /// find every binding it leaves behind.
    fn_scope_base: usize,
    /// Bounds of the function being checked, beside `current_generics`.
    current_bounds: Vec<Vec<String>>,
    /// A bounded call's promise to check once inference has settled: the
    /// variable a bounded parameter was instantiated with, its interfaces,
    /// the call, and the calling function's own bounds at the time.
    bound_obligations: Vec<BoundObligation>,
    /// Inferred type per expression, keyed by whole span. See `infer_expr`.
    expr_types: SpanMap<Type>,
    /// Every expression's inferred type, keyed by its span.
    ///
    /// `expr_types` above is a memoisation *cache* and is deliberately partial
    /// — a closure literal's type depends on the position it fills, so it has
    /// to be re-visited and must not be cached. This is the other thing: a
    /// record of what the checker decided, written for every expression and
    /// resolved through unification at the end.
    ///
    /// It exists because the native backend was reimplementing type inference
    /// to get these answers — about 950 lines with 49 refusal sites — and
    /// nine findings came from the two implementations disagreeing. See
    /// `docs/probe-removal.md`.
    resolved_types: SpanMap<Type>,
    /// See `Analysis::empty_types`. Collected as type variables and resolved
    /// once checking is over, since that is when they have answers.
    /// (start offset, length, element type) — the length so the
    /// diagnostic underlines `map()` and not its first two characters.
    empty_lits: Vec<(u32, u32, Type)>,
    /// `sum()` calls whose result is `f64`, by span start.
    float_sums: std::collections::HashSet<u32>,
    /// effect name → (ops, handler struct TypeDefId).
    effects: HashMap<String, (Vec<EffectOp>, TypeDefId)>,
    /// variant name → (typedef, variant index); ambiguous names keep all.
    variants: HashMap<String, Vec<(TypeDefId, usize)>>,
    uni: Unifier,
    diags: Vec<Diagnostic>,
    current_module: String,
    /// Type parameters that reach a map key inside a generic function's own
    /// body, by qualified function name.
    ///
    /// [T-MAP-KEY-GENERIC] applies the key rule at each call site, and the
    /// signature is where it looked. A parameter that reaches a map *only*
    /// inside the body is mentioned in no part of the signature, so
    /// `fn tally[T](xs: [T]) -> i64` holding a `Map[T, i64]` let
    /// `tally([1.5, 2.5])` past the checker and into a runtime
    /// `invalid map key` — the one shape where a checked program still
    /// panicked for a type error.
    key_generics: HashMap<String, std::collections::BTreeSet<usize>>,
    /// Generic parameters that reach a parallel operation's elements, by the
    /// function that declares them. Instantiating one at a type that carries a
    /// `ref` is what [T-PAR-MAP-PURE] refuses, and a generic hides that from
    /// the operation itself — `fn go[T](xs: [T], f: fn(T) -> i64) {
    /// xs.par_map(f) }` called with `[ref Acc]`. The same shape
    /// [T-MAP-KEY-GENERIC] describes for map keys, and checked the same way.
    par_generics: HashMap<String, std::collections::BTreeSet<usize>>,
    /// Call sites of generics, held until every body has been walked.
    pending_par_checks: Vec<(String, Vec<Type>, Span, Option<String>)>,
    /// The generic function whose body is being checked, if any.
    key_generic_owner: Option<String>,
    /// Call sites of generic functions, checked once every body has been
    /// walked — a call may be checked before the body it constrains.
    /// (callee, its type arguments at this site, span, the generic function
    /// the call is written inside — which is how a key generic propagates
    /// from a callee to its caller).
    pending_key_checks: Vec<(String, Vec<Type>, Span, Option<String>)>,
    /// Discarded statements whose type was still an inference variable when
    /// the statement was checked, resolved once the whole program is typed.
    pending_discards: Vec<(Span, Type)>,
    /// module → (alias → full target module name)
    module_imports: HashMap<String, HashMap<String, String>>,
    scopes: Vec<HashMap<String, Binding>>,
    /// Scope depth at which each enclosing closure body begins. A binding
    /// found below the innermost of these is captured, not local.
    closure_floors: Vec<usize>,
    /// Whether the closure literal currently being checked reaches a `ref`
    /// declared outside it. A closure written *at* a parallel operation is
    /// checked there; one bound to a name first is not, and that is the shape
    /// [T-PAR-MAP-PURE] lists as still reaching past the rule.
    closure_touches_outside_ref: bool,
    /// Closure literals, by start offset, that reach a `ref` from outside
    /// themselves. Filled when the body is checked, which for a closure whose
    /// parameter types nothing fixes happens *after* the `let` that binds it —
    /// so the answer is recorded and applied at the end.
    ref_touching_closures: std::collections::HashSet<u32>,
    /// Name → the closure literal it was bound to.
    let_closure_span: HashMap<String, u32>,
    /// Parallel operations given a name, held until every body is checked.
    pending_par_name_checks: Vec<(u32, String, Span)>,
    /// Scope depth at which the innermost enclosing *parallel* function began,
    /// if any — the function handed to `par_map` or `par_filter`. A `ref`
    /// declared below it is shared by every element's call, and writing
    /// through one is the race [T-PAR-MAP-PURE] exists to prevent.
    parallel_fn_floor: Option<usize>,
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
    /// Field reads whose receiver type is not known yet, one list per open
    /// block — the same shape, and for the same reason, as the closure
    /// bodies above.
    ///
    /// `let f = |x| S{ c: x }` defers its body, so `f(2).c` one line later
    /// asked an inference variable for a field and was told `_` has no field
    /// `c` — naming a type no program can write, about a value whose type is
    /// inferred perfectly well by the time the block ends. Every other use of
    /// the same call worked, so the refusal depended only on which use came
    /// first.
    deferred_fields: Vec<Vec<DeferredField>>,
    /// Types whose bodies are not resolved yet, by module.
    pending_type_bodies: Vec<(String, Vec<(usize, TypeDefId)>)>,
    /// Alias declarations, held until every module's type names and every
    /// effect's handler type are registered.
    ///
    /// Resolving an alias in the pass that collected it meant it could only
    /// name what that module had already registered: `type Cb = fn(Thing) ->
    /// i64` was "unknown type `Thing`" whenever `Thing` came from another
    /// module, while `fn f(t: Thing)` two lines below resolved fine — one
    /// namespace with a hole in it, the same hole [T-EFFECT-DECL] had for
    /// handler types ([T-MODULE-NS], F68).
    pending_aliases: Vec<(String, Vec<usize>)>,
    /// Alias names seen so far, across every module, for duplicate detection
    /// — `self.aliases` is not filled until the resolving pass runs.
    alias_names: std::collections::HashSet<String>,
    /// Which module declared each type name.
    ///
    /// Types share one namespace across the whole program ([T-MODULE-NS]), so
    /// two *packages* that each declare a `Bad` collide in a consumer that
    /// imports both — and the consumer owns neither. "duplicate type name
    /// `Bad`" pointed at one dependency's line and never said there was
    /// another (F78).
    type_module: HashMap<String, String>,
    /// Module-level constants: `module::NAME` to (type, is_pub).
    consts: HashMap<String, (Type, bool)>,
    /// (ret, err) of the function being checked.
    fn_ret: Type,
    fn_err: Option<Type>,
    /// True for main/tests: any error type may propagate (runtime reports).
    err_open: bool,
    /// Nesting depth of `parallel` scopes — `spawn` requires one.
    parallel_depth: u32,
    /// Nesting of `concurrent` blocks, and the scope depth at which the
    /// innermost `go` body began: a binding below it belongs to another task
    /// and may not be written from this one (docs/concurrency.md).
    concurrent_depth: u32,
    go_floor: Option<usize>,
    /// The handle types of the program's actors ([E-ACTOR]). A task may use
    /// one from outside, though it reaches a `ref`: the state behind it is
    /// reached only through methods that run one at a time.
    actor_types: std::collections::HashSet<String>,
    /// Scope depth where the innermost enclosing `parallel` block began.
    parallel_floor: Option<usize>,
    /// While checking a task body: (parallel floor, depth where this task body
    /// began). A name resolved between those two was declared *inside* the
    /// parallel block, and is gone by the time the task runs.
    task_capture: Option<(usize, usize)>,
    loop_depth: u32,
}

/// A closure whose body is checked once its parameter types are settled.
/// A field read waiting for its receiver's type.
struct DeferredField {
    recv: Type,
    name: Ident,
    /// The type the surrounding expression was handed; unified with the
    /// field's real type once the receiver is known.
    out: Type,
}

struct DeferredClosure {
    params: Vec<Ident>,
    body: Expr,
    param_tys: Vec<Type>,
    ret: Type,
    /// Start offset of the closure literal, so what the body turns out to
    /// reach can be recorded against the name it was bound to.
    at: u32,
}

impl Checker {
    /// TIR: what a bare name in an expression resolved to.
    fn tir_name(&mut self, span: Span, r: crate::tirfacts::NameRes) {
        if self.record_tir {
            self.tir.names.insert((span.start, span.end), r);
        }
    }

    /// TIR: what a call reached.
    fn tir_call(&mut self, span: Span, r: crate::tirfacts::CallRes) {
        if self.record_tir {
            self.tir.calls.insert((span.start, span.end), r);
        }
    }

    /// TIR: how a method call dispatched.
    fn tir_method(&mut self, span: Span, r: crate::tirfacts::MethodRes) {
        if self.record_tir {
            self.tir.methods.insert((span.start, span.end), r);
        }
    }

    /// TIR: the type a binding was given, keyed by the name that binds it.
    fn tir_bind(&mut self, name: &Ident, ty: &Type) {
        if self.record_tir {
            self.tir
                .binds
                .insert((name.span.start, name.span.end), ty.clone());
        }
    }

    /// Resolves every type the TIR tables hold, and adds the signatures,
    /// constants and effects, once checking is done.
    fn finish_tir(&mut self) -> crate::tirfacts::TirFacts {
        use crate::tirfacts::{CallRes, EffectSig, FnSig, NameRes};
        let mut t = std::mem::take(&mut self.tir);
        let uni = &self.uni;
        for ty in t.binds.values_mut() {
            *ty = uni.resolve(ty);
        }
        for ty in t.wraps.values_mut() {
            *ty = uni.resolve(ty);
        }
        t.types = self
            .resolved_types
            .iter()
            .map(|(k, ty)| (*k, uni.resolve(ty)))
            .collect();
        for (k, ty) in std::mem::take(&mut t.checked) {
            t.types.entry(k).or_insert_with(|| uni.resolve(&ty));
        }
        for r in t.names.values_mut() {
            if let NameRes::Fn(_, args) = r {
                for a in args.iter_mut() {
                    *a = uni.resolve(a);
                }
            }
        }
        for r in t.calls.values_mut() {
            if let CallRes::Fn(_, args) = r {
                for a in args.iter_mut() {
                    *a = uni.resolve(a);
                }
            }
        }
        for (name, info) in &self.fns {
            t.fns.insert(
                name.clone(),
                FnSig {
                    generics: info.generics,
                    bounds: info.bounds.clone(),
                    params: info.params.iter().map(|p| uni.resolve(p)).collect(),
                    modes: info.modes.clone(),
                    ret: uni.resolve(&info.ret),
                    err: info.err.as_ref().map(|e| uni.resolve(e)),
                    row: self.fn_rows.get(name).map(|r| r.iter().cloned().collect()),
                    is_pub: info.is_pub,
                    is_extern: self.extern_names.contains(name),
                },
            );
        }
        for (name, (ty, _)) in &self.consts {
            t.consts.insert(name.clone(), uni.resolve(ty));
        }
        for (name, (ops, _)) in &self.effects {
            t.effects.insert(
                name.clone(),
                EffectSig {
                    ops: ops
                        .iter()
                        .map(|op| {
                            (
                                op.name.clone(),
                                op.params.iter().map(|p| uni.resolve(p)).collect(),
                                uni.resolve(&op.ret),
                            )
                        })
                        .collect(),
                },
            );
        }
        t.parallel_ok = self.parallel_ok.iter().copied().collect();
        t
    }

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
            parallel_ok: std::collections::HashSet::new(),
            closure_rows: HashMap::new(),
            ok_wrapped: std::collections::HashSet::new(),
            map_get_pending: Vec::new(),
            closure_types: HashMap::new(),
            closure_params: Vec::new(),
            generic_ctors: Vec::new(),
            hof_closures: std::collections::HashSet::new(),
            handler_ids: std::collections::HashSet::new(),
            unsafe_depth: 0,
            mem_ops: HashMap::new(),
            c_facts: Default::default(),
            export_names: Default::default(),
            discarding: false,
            tail_fn: None,
            tail_fn_name: None,
            with_floors: Vec::new(),
            region_depth: 0,
            region_floors: Vec::new(),
            extern_names: std::collections::HashSet::new(),
            float_literals: std::collections::HashSet::new(),
            narrow_assigns: std::collections::HashMap::new(),
            wide_ints: std::collections::HashSet::new(),
            value_rows: SpanMap::default(),
            callee_rows: SpanMap::default(),
            field_calls: SpanSet::default(),
            interfaces: HashMap::new(),
            impls: HashMap::new(),
            impl_of: std::collections::HashSet::new(),
            iface_calls: SpanSet::default(),
            linear_types: HashMap::new(),
            fn_scope_base: 0,
            current_bounds: Vec::new(),
            bound_obligations: Vec::new(),
            expr_types: SpanMap::default(),
            resolved_types: SpanMap::default(),
            record_tir: false,
            tir: crate::tirfacts::TirFacts::default(),
            empty_lits: Vec::new(),
            float_sums: std::collections::HashSet::new(),
            effects: HashMap::new(),
            variants,
            uni: Unifier::new(),
            diags: Vec::new(),
            current_module: "main".to_string(),
            key_generics: HashMap::new(),
            par_generics: HashMap::new(),
            pending_par_checks: Vec::new(),
            key_generic_owner: None,
            pending_key_checks: Vec::new(),
            pending_discards: Vec::new(),
            module_imports: HashMap::new(),
            scopes: Vec::new(),
            closure_floors: Vec::new(),
            closure_touches_outside_ref: false,
            ref_touching_closures: std::collections::HashSet::new(),
            let_closure_span: HashMap::new(),
            pending_par_name_checks: Vec::new(),
            parallel_fn_floor: None,
            current_generics: Vec::new(),
            deferred_closures: Vec::new(),
            deferred_fields: Vec::new(),
            pending_type_bodies: Vec::new(),
            pending_aliases: Vec::new(),
            alias_names: std::collections::HashSet::new(),
            type_module: HashMap::new(),
            consts: HashMap::new(),
            fn_ret: Type::Unit,
            fn_err: None,
            err_open: false,
            parallel_depth: 0,
            concurrent_depth: 0,
            go_floor: None,
            actor_types: Default::default(),
            parallel_floor: None,
            task_capture: None,
            loop_depth: 0,
        }
    }

    fn error(&mut self, span: Span, msg: impl Into<String>) -> Type {
        let d = Diagnostic::error(span, msg);
        self.push_diag(d);
        Type::Error
    }

    fn error_help(&mut self, span: Span, msg: impl Into<String>, help: impl Into<String>) -> Type {
        let d = Diagnostic::error(span, msg).with_help(help);
        self.push_diag(d);
        Type::Error
    }

    /// One diagnostic per (span, message).
    ///
    /// An expression can be visited more than once — inference probes it, then
    /// the checker walks it — and a rule that fires on the expression rather
    /// than on a binding fires each time. `region { print("{r.v}") }` reported
    /// "a `ref` cannot be used inside a region" twice at the same span, which
    /// reads as two problems.
    fn push_diag(&mut self, d: Diagnostic) {
        if self
            .diags
            .iter()
            .any(|prev| prev.span == d.span && prev.message == d.message)
        {
            return;
        }
        self.diags.push(d);
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

    /// A type name declared twice, said so that the reader can find both.
    ///
    /// The two declarations are often in two *packages*, neither of which the
    /// person reading owns — so the message names the module each is in and
    /// says why one namespace is the rule, rather than pointing at a line in
    /// somebody else's library and stopping.
    fn duplicate_type(&mut self, name: &Ident) {
        let here = self.current_module.clone();
        match self.type_module.get(&name.name).cloned() {
            Some(there) if there != here => {
                self.error_help(
                    name.span,
                    format!(
                        "duplicate type name `{}` — `{there}` declares one too",
                        name.name
                    ),
                    "types share one namespace across the whole program ([T-MODULE-NS]), \
so two packages cannot each declare a type of the same name — rename one, or, \
if you own neither, ask whichever is yours to prefix its types",
                );
            }
            _ => {
                self.error(name.span, format!("duplicate type name `{}`", name.name));
            }
        }
    }

    /// Registers `linear type` declarations ([T-LINEAR]). A linear type is a
    /// non-generic struct: its fields are what the declaring module reads
    /// when it ends one, and a type parameter would let a linear value hide
    /// inside something that is not.
    fn collect_linear(&mut self, module: &Module) {
        for decl in &module.decls {
            let Decl::Type(t) = decl else { continue };
            if !t.is_linear {
                continue;
            }
            if !t.generics.is_empty() || !matches!(t.body, grasm_syntax::ast::TypeBody::Struct(_)) {
                self.error(
                    t.name.span,
                    "a `linear type` is a struct with no type parameters",
                );
                continue;
            }
            self.linear_types
                .insert(t.name.name.clone(), self.current_module.clone());
        }
    }

    /// A field of a non-linear struct cannot hold a linear value: the struct
    /// could be copied, and with it the value ([T-LINEAR-CONTAIN]).
    fn check_linear_fields(&mut self, module: &Module) {
        for decl in &module.decls {
            let Decl::Type(t) = decl else { continue };
            if t.is_linear {
                continue;
            }
            let grasm_syntax::ast::TypeBody::Struct(fields) = &t.body else {
                continue;
            };
            for fd in fields {
                let ty = self.resolve_type(
                    &fd.ty,
                    &t.generics
                        .iter()
                        .map(|g| g.name.clone())
                        .collect::<Vec<_>>(),
                );
                if self.is_linear(&ty) || self.holds_linear(&ty) {
                    self.error_help(
                        fd.span,
                        format!("field `{}` of `{}` would hold a linear value", fd.name.name, t.name.name),
                        "only a `linear type` may hold one, so the whole is consumed once too ([T-LINEAR-CONTAIN])",
                    );
                }
            }
        }
    }

    fn collect_types(&mut self, module: &Module) {
        // Pass 1: register names so recursive references resolve.
        let mut ids: Vec<(usize, TypeDefId)> = Vec::new();
        // Aliases are resolved in pass 2, so nothing had registered them by
        // the time a second declaration of the same name was collected: two
        // `type Id = ...` lines in one module both checked clean and the
        // later one silently won, where [T-MODULE-NS] says a type name may
        // not be declared twice. Struct and sum bodies were caught because
        // pass 1 inserts them immediately.
        let mut alias_decls: Vec<usize> = Vec::new();
        for (di, decl) in module.decls.iter().enumerate() {
            if let Decl::Type(t) = decl {
                // A builtin type name is not in `defs`, so nothing collided
                // with it: `type Map = { x: i64 }` and `type i64 = str` were
                // accepted and then silently ignored, since `resolve_type`
                // answers for those names before it ever consults a
                // declaration. Accepted-and-ignored reads to a reviewer as a
                // definition that took effect, which is the failure
                // [T-TYPE-NO-PUB] argues against in the same words.
                if is_builtin_type_name(&t.name.name) {
                    self.error_help(
                        t.name.span,
                        format!("type name `{}` is built in", t.name.name),
                        "a declaration of it would never be reached — every use of the name \
resolves to the builtin; pick a different name",
                    );
                    continue;
                }
                if let Some(&existing) = self.defs.by_name.get(&t.name.name) {
                    if existing == self.defs.option_id || existing == self.defs.io_error_id {
                        self.error_help(
                            t.name.span,
                            format!("type name `{}` is reserved by the prelude", t.name.name),
                            "pick a different name (e.g. `AppError`)",
                        );
                    } else {
                        self.duplicate_type(&t.name);
                    }
                    continue;
                }
                if self.aliases.contains_key(&t.name.name)
                    || self.alias_names.contains(&t.name.name)
                {
                    self.duplicate_type(&t.name);
                    continue;
                }
                match &t.body {
                    TypeBody::Alias(_) => {
                        self.alias_names.insert(t.name.name.clone());
                        self.type_module
                            .insert(t.name.name.clone(), self.current_module.clone());
                        alias_decls.push(di);
                        // The `pub` check runs over the types pass 2 gives
                        // bodies, which an alias never reaches — so
                        // `pub type N = i64` was accepted and ignored, which
                        // is the exact thing [T-TYPE-NO-PUB] exists to stop.
                        if t.is_pub {
                            self.error_help(
                                t.name.span,
                                format!("`pub` means nothing on type `{}`", t.name.name),
                                "types are one namespace across the program, so every type is \
already reachable from every module — drop the `pub`",
                            );
                        }
                    }
                    _ => {
                        let id = self.defs.insert(TypeDef {
                            name: t.name.name.clone(),
                            generics: t.generics.iter().map(|g| g.name.clone()).collect(),
                            kind: TypeDefKind::Struct { fields: vec![] },
                        });
                        self.type_module
                            .insert(t.name.name.clone(), self.current_module.clone());
                        ids.push((di, id));
                    }
                }
            }
        }
        self.pending_aliases
            .push((self.current_module.clone(), alias_decls));
        self.pending_type_bodies
            .push((self.current_module.clone(), ids));
    }

    /// Fills in the *bodies* of the types registered by `collect_types`.
    ///
    /// A separate pass because a field's type may live in any module, and
    /// resolving bodies inside `collect_types` meant a file processed before
    /// the one declaring the type reported `unknown type` — with a help that
    /// said to declare it, naming the thing that was already declared. The
    /// same ordering bug effects had until C15.
    /// Resolves every alias, after every module's type names and every
    /// effect's handler type are registered.
    ///
    /// A separate pass over the whole program, not a step inside the one that
    /// collects a module, because an alias may name a type from any module —
    /// exactly as a parameter, a return type or a struct field may. Doing it
    /// per module meant an alias could only see what that module had already
    /// registered, so `type Cb = fn(Thing) -> i64` failed where
    /// `fn f(t: Thing)` on the next line succeeded (F68).
    ///
    /// The fixpoint below is still needed for a different reason: an alias may
    /// name a *later alias*, which nothing has resolved yet whatever the order
    /// of modules.
    fn resolve_type_aliases(&mut self, modules: &[(String, &Module)]) {
        let pending = std::mem::take(&mut self.pending_aliases);
        for (module_name, decls) in pending {
            let Some((_, module)) = modules.iter().find(|(n, _)| *n == module_name) else {
                continue;
            };
            self.current_module = module_name.clone();
            //
            // To a fixpoint, because an alias may name a later one. Resolving in
            // declaration order made `type Name = Text` above `type Text = str`
            // an "unknown type `Text`" that swapping the two lines fixed, while
            // an alias to a later *struct* worked — so [G-DECL-ORDER]'s "forward
            // references are permitted" held everywhere but here.
            let aliases: Vec<(&Ident, &TypeExpr, Vec<String>)> = decls
                .iter()
                .filter_map(|&di| match &module.decls[di] {
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
            let stuck: std::collections::HashSet<String> =
                todo.iter().map(|&i| aliases[i].0.name.clone()).collect();
            for &i in &todo {
                let (name, te, generics) = &aliases[i];
                // An alias that stands for a shape holding itself. `type A = [A]`
                // has no base case and no indirection to stop at — unlike a struct
                // or a sum, which [T-RECURSIVE] does allow to refer to themselves,
                // because a value of one is built out of *other* values. Reported
                // as `unknown type `A`` it named a type declared two words to the
                // left, and every pass that walked it recursively overflowed the
                // process stack: exit 134, no diagnostic, and a dependency
                // carrying the line took its consumer's toolchain down too.
                if Self::names_itself(te, &name.name, &stuck) {
                    self.error_help(
                        name.span,
                        format!("type `{}` is defined in terms of itself", name.name),
                        "a `type` alias is another spelling of the type it names, so it has nowhere to stop — a struct or a sum type may refer to itself ([T-RECURSIVE]), because a value of one holds *other* values",
                    );
                    self.aliases.insert(name.name.clone(), Type::Error);
                    continue;
                }
                let ty = self.resolve_type(te, generics);
                self.aliases.insert(name.name.clone(), ty);
            }
        }
    }

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
                                self.error(
                                    f.name.span,
                                    format!("duplicate field `{}`", f.name.name),
                                );
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
                // The builtin effects are not handleable, and declaring one
                // made it so. `effect io { ... }` gave a program
                // `with io handled_by ...`, which discharged the *builtin*
                // `io` for everything in the block — so a function declaring
                // `/ {}` printed, `par_map` ran a printing function on several
                // cores, and a `capabilities = []` package read and wrote
                // files while `grasm deps` certified it as performing nothing.
                // One word in a manifest is the whole capability story; a
                // dependency must not be able to write it away.
                //
                // Every capability, from the one list `effects` keeps. `net`
                // and `proc` were left off this one when they arrived, so
                // `effect net` and a `with net handled_by` let a function
                // declaring `/ {}` open a socket -- the hole described above,
                // one capability over.
                if crate::effects::is_capability(&e.name.name) {
                    self.error_help(
                        e.name.span,
                        format!("effect name `{}` is reserved by the language", e.name.name),
                        "`io`, `async`, `net`, `proc`, `unsafe` and `mem` are capabilities the \
runtime discharges, not effects a program declares or handles — there is no `with io handled_by` \
([T-EFFECT-BUILTIN]); rename this one",
                    );
                    continue;
                }
                if self.effects.contains_key(&e.name.name) {
                    self.error(e.name.span, format!("duplicate effect `{}`", e.name.name));
                    continue;
                }
                // One namespace means one namespace ([T-MODULE-NS]). An
                // effect and a type of the same name were both accepted and
                // both stayed usable — and an effect declares `EHandler` too,
                // so `type ClockHandler = i64` beside `effect Clock` collided
                // with a name the reader never wrote.
                //
                // `alias_names`, not `aliases`: aliases are *resolved* after
                // effects are collected (F68), so the resolved table is empty
                // here. The name set is filled in the pass that collected the
                // declaration, which is the pass this has to agree with.
                for clash in [e.name.name.clone(), format!("{}Handler", e.name.name)] {
                    if self.defs.by_name.contains_key(&clash) || self.alias_names.contains(&clash) {
                        let what = if clash == e.name.name {
                            format!("effect `{clash}` collides with a type of that name")
                        } else {
                            format!(
                                "effect `{}` declares `{clash}`, which collides with a type of that name",
                                e.name.name
                            )
                        };
                        self.error_help(
                            e.name.span,
                            what,
                            "types and effects share one namespace across the program — rename one of them",
                        );
                    }
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
                    let params: Vec<Type> = op
                        .params
                        .iter()
                        .map(|p| self.resolve_type(&p.ty, &[]))
                        .collect();
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
                self.handler_ids.insert(handler_id);
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
            // A constant is *re-evaluated* at every mention ([E-CONST]), so a
            // reference in one is a fresh slot each time: two mentions on one
            // line printed two different references, `release` gave up a slot
            // nothing else named, and a loop leaked one per iteration. The
            // syntactic check caught a literal `ref(...)` and nothing else —
            // a helper returning one walked past it, and so would a struct or
            // a list holding one ([E-CONST-REF]).
            let resolved = self.uni.resolve(&ty);
            if self.carries_ref(&resolved) {
                self.error_help(
                    c.name.span,
                    format!("constant `{}` holds a reference", c.name.name),
                    "a constant is evaluated again at every mention, so each one would be a different slot — bind it with `let` inside a function instead",
                );
            }
            self.consts.insert(key, (resolved, c.is_pub));
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
            // A C function is a link-time symbol of the package that declares
            // it, and `pub` on one exported the name past every gate that
            // makes Tier 3 visible: `libext.getpid()` in another module
            // needed no `unsafe`, charged no capability, panicked under the
            // interpreter with `module has no function` and printed `true`
            // from a native build. The wrapper — `pub fn f(x) { unsafe {
            // c_fn(x) } }` — is how a package offers one, and it is the
            // spelling that shows up in the consumer's ceiling
            // ([T-UNSAFE-FFI]).
            if e.is_pub {
                self.error_help(
                    e.name.span,
                    format!("`extern fn {}` cannot be `pub`", e.name.name),
                    "a C function is resolved by the linker for the package that declares \
it — wrap it in a `pub fn` whose body is `unsafe`, which is what makes the `unsafe` \
capability visible to everyone who depends on you",
                );
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
                if !Self::is_c_param_type(t) {
                    let shown = self.show(t);
                    self.error_help(
                        p.ty.span(),
                        format!("`{shown}` has no C representation"),
                        "an `extern fn` parameter is an integer type, `f64`, `addr`, `str` or `bytes`",
                    );
                }
            }
            // One symbol, one meaning: an `export fn` of the same name would
            // be the definition this declaration links to ([T-EXPORT]).
            if self.export_names.contains_key(&e.name.name) {
                self.error_help(
                    e.name.span,
                    format!(
                        "`{}` is both an `extern fn` and an `export fn`",
                        e.name.name
                    ),
                    "they would be one symbol; call the exported function by its Grasm name",
                );
            }
            if let Some(sig) = self.c_sig_of(&params, &ret) {
                self.c_facts.sigs.insert(key.clone(), sig);
            }
            if ret != Type::Unit && !Self::is_c_abi_type(&ret) {
                let shown = self.show(&ret);
                // A returned pointer is a different question from a passed one
                // and is deliberately still refused ([T-EXTERN-POINTER]).
                let help = if matches!(ret, Type::Str | Type::Bytes) {
                    "an `extern fn` may *take* `str` and `bytes` — the pointer is \
valid for the call and the buffer belongs to Grasm. It may not *return* one: \
nothing says who owns it, how long it lives, or how long it is"
                } else {
                    "an `extern fn` returns an integer type, `f64`, `addr` or nothing"
                };
                self.error_help(e.span, format!("`{shown}` has no C representation"), help);
            }
            self.fns.insert(
                key,
                FnInfo {
                    is_pub: e.is_pub,
                    generics: 0,
                    bounds: Vec::new(),
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
        matches!(t, Type::I64 | Type::Int(_) | Type::F64 | Type::Addr)
    }

    /// The C signature of parameters and a result that all cross
    /// ([T-EXTERN-POINTER]), or `None` if one does not.
    fn c_sig_of(&self, params: &[Type], ret: &Type) -> Option<crate::mem::CSig> {
        use crate::mem::{CTy, MemTy};
        let mut ps = Vec::with_capacity(params.len());
        for p in params {
            ps.push(match p {
                Type::Str => CTy::Str,
                Type::Bytes => CTy::Bytes,
                t => CTy::Layout(MemTy::of(t)?),
            });
        }
        let ret = match ret {
            Type::Unit => None,
            t => Some(MemTy::of(t)?),
        };
        Some(crate::mem::CSig { params: ps, ret })
    }

    /// What may cross *into* C.
    ///
    /// Wider than what may come back. A `str` or `bytes` argument is passed as
    /// a pointer, and the buffer it points at is Grasm's: it is alive for the
    /// duration of the call and nothing about ownership has to be decided. A
    /// returned pointer is the opposite — nothing says who owns it, how long it
    /// lives, or how long it is — so that stays refused ([T-EXTERN-POINTER]).
    fn is_c_param_type(t: &Type) -> bool {
        matches!(
            t,
            Type::I64 | Type::Int(_) | Type::F64 | Type::Addr | Type::Str | Type::Bytes
        )
    }

    /// The name an `impl` is keyed by for a type, if the type can have one:
    /// `i64`, `f64`, `bool`, `str`, or a declared non-generic type.
    fn impl_key(&self, t: &Type) -> Option<String> {
        match t {
            Type::I64 => Some("i64".to_string()),
            Type::Int(k) => Some(k.name().to_string()),
            Type::F64 => Some("f64".to_string()),
            Type::Bool => Some("bool".to_string()),
            Type::Str => Some("str".to_string()),
            // A shape, not a type: `[i64]` and `[str]` both answer `[]`, and
            // `Pair[i64]` answers `Pair`, so one generic `impl` serves every
            // instantiation ([T-INTERFACE-GENERIC]).
            Type::List(_) => Some("[]".to_string()),
            Type::Named { id, .. } => Some(self.defs.get(*id).name.clone()),
            _ => None,
        }
    }

    fn collect_interfaces(&mut self, module: &Module) {
        for decl in &module.decls {
            if let Decl::Interface(i) = decl {
                if self.interfaces.contains_key(&i.name.name)
                    || self.defs.by_name.contains_key(&i.name.name)
                {
                    self.error(i.name.span, format!("duplicate name `{}`", i.name.name));
                    continue;
                }
                let mut seen = std::collections::HashSet::new();
                for m in &i.methods {
                    if !seen.insert(m.name.name.clone()) {
                        self.error(
                            m.name.span,
                            format!(
                                "interface `{}` declares `{}` twice",
                                i.name.name, m.name.name
                            ),
                        );
                    }
                    let first_is_self = m.params.first().is_some_and(|p| p.name.name == "self");
                    if !first_is_self {
                        self.error(
                            m.name.span,
                            format!(
                                "`{}` needs `self` as its first parameter: an interface method is called on a value",
                                m.name.name
                            ),
                        );
                    }
                }
                self.interfaces.insert(i.name.name.clone(), i.clone());
            }
        }
    }

    /// Records each `impl` and holds it to its interface ([T-INTERFACE-IMPL]).
    fn collect_impls(&mut self, module: &Module) {
        for decl in &module.decls {
            let Decl::Impl(im) = decl else { continue };
            let iname = im
                .iface
                .segments
                .last()
                .map(|s| s.name.clone())
                .unwrap_or_default();
            // An actor's methods ([E-ACTOR]): the parser lowered them as an
            // `impl` of an interface no program can name, so there is no
            // signature to hold them to -- only the calls to resolve.
            if iname == "\u{a7}actor" {
                self.actor_types.insert(im.type_name.clone());
                for (n, lowered) in &im.methods {
                    let key = format!("{}::{lowered}", self.current_module);
                    let slot = (im.type_name.clone(), n.name.clone());
                    if self.impls.contains_key(&slot) {
                        self.error(
                            n.span,
                            format!("actor `{}` has two methods `{}`", im.type_name, n.name),
                        );
                        continue;
                    }
                    self.impls.insert(slot, (key, iname.clone()));
                }
                continue;
            }
            let Some(iface) = self.interfaces.get(&iname).cloned() else {
                self.error(im.iface.span, format!("unknown interface `{iname}`"));
                continue;
            };
            let tname = im.type_name.clone();
            // The target is a shape. A list shape carries its element type in
            // the `impl`'s own parameter, which the parser has checked; a
            // declared type must be given exactly as many parameters as it
            // was declared with ([T-INTERFACE-GENERIC]).
            let known = if tname == "[]" {
                true
            } else if matches!(tname.as_str(), "i64" | "f64" | "bool" | "str") {
                im.generics.is_empty()
            } else if let Some(id) = self.defs.by_name.get(&tname) {
                self.defs.get(*id).generics.len() == im.generics.len()
            } else {
                false
            };
            if !known {
                let why = match self.defs.by_name.get(&tname) {
                    Some(id) => {
                        let want = self.defs.get(*id).generics.len();
                        format!(
                            "`{tname}` takes {want} type parameter(s) and this `impl` introduces {}",
                            im.generics.len()
                        )
                    }
                    None => format!(
                        "an `impl` is for a declared type, a list, or for `i64`, `f64`, `bool` or `str`, not `{tname}`"
                    ),
                };
                self.error(im.ty.span(), why);
                continue;
            }
            if !self.impl_of.insert((iname.clone(), tname.clone())) {
                self.error(
                    im.span,
                    format!("`{tname}` already implements `{iname}`; a type has one `impl` of each interface"),
                );
                continue;
            }
            let provided: HashMap<String, String> = im
                .methods
                .iter()
                .map(|(n, lowered)| (n.name.clone(), lowered.clone()))
                .collect();
            for (n, _) in &im.methods {
                if !iface.methods.iter().any(|m| m.name.name == n.name) {
                    self.error(
                        n.span,
                        format!("`{}` is not a method of interface `{iname}`", n.name),
                    );
                }
            }
            for m in &iface.methods {
                let Some(lowered) = provided.get(&m.name.name) else {
                    self.error(
                        im.span,
                        format!(
                            "`impl {iname} for {tname}` does not provide `{}`",
                            m.name.name
                        ),
                    );
                    continue;
                };
                let key = format!("{}::{lowered}", self.current_module);
                let Some(info) = self.fns.get(&key).cloned() else {
                    continue;
                };
                // `Self` is the shape the `impl` is for, so the `impl`'s own
                // parameters have to be in scope to read it.
                let gnames: Vec<String> = im.generics.iter().map(|g| g.name.clone()).collect();
                // The interface's signature with `Self` read as this type.
                let want_params: Vec<Type> = m
                    .params
                    .iter()
                    .map(|p| {
                        let te = grasm_syntax::ast::replace_self(&p.ty, &im.ty);
                        self.resolve_type(&te, &gnames)
                    })
                    .collect();
                let want_ret = m
                    .ret
                    .as_ref()
                    .map(|t| {
                        let te = grasm_syntax::ast::replace_self(t, &im.ty);
                        self.resolve_type(&te, &gnames)
                    })
                    .unwrap_or(Type::Unit);
                let want_err = m.err.as_ref().map(|t| {
                    let te = grasm_syntax::ast::replace_self(t, &im.ty);
                    self.resolve_type(&te, &gnames)
                });
                // The method carries the `impl`'s parameters and no others:
                // an interface method is not itself generic.
                if info.generics != im.generics.len()
                    || info.params != want_params
                    || info.ret != want_ret
                    || info.err != want_err
                    || info.modes.iter().any(|m| *m != ParamMode::Let)
                {
                    self.error(
                        im.span,
                        format!(
                            "`{}` in `impl {iname} for {tname}` does not match the interface's signature",
                            m.name.name
                        ),
                    );
                    continue;
                }
                // The row an `impl` may perform is the interface's, and no
                // wider: a bounded call charges the caller what the
                // *interface* declares, so a method that performed more than
                // that would be reached through a row that does not mention
                // it -- and a package's capabilities are checked against
                // those rows ([T-INTERFACE-EFFECTS]).
                let allowed: std::collections::BTreeSet<&str> = m
                    .effects
                    .iter()
                    .flatten()
                    .map(|p| p.segments[0].name.as_str())
                    .collect();
                let declared = module.decls.iter().find_map(|d| match d {
                    Decl::Fn(f) if f.name.name == *lowered => f.effects.as_ref(),
                    _ => None,
                });
                let extra: Vec<String> = declared
                    .into_iter()
                    .flatten()
                    .map(|p| p.segments[0].name.clone())
                    .filter(|e| {
                        !allowed.contains(e.as_str())
                            && !allowed.iter().any(|a| e.split('.').next() == Some(*a))
                    })
                    .collect();
                if !extra.is_empty() {
                    let list = extra
                        .iter()
                        .map(|e| format!("`{e}`"))
                        .collect::<Vec<_>>()
                        .join(", ");
                    let shown = if allowed.is_empty() {
                        "performs nothing".to_string()
                    } else {
                        let names: Vec<String> = allowed.iter().map(|a| format!("`{a}`")).collect();
                        format!("performs {}", names.join(", "))
                    };
                    // At the `impl`'s own method, which is the one to change.
                    let at = im
                        .methods
                        .iter()
                        .find(|(n, _)| n.name == m.name.name)
                        .map_or(im.span, |(n, _)| n.span);
                    self.error_help(
                        at,
                        format!(
                            "`{}` for `{tname}` declares {list}, which `{iname}` does not",
                            m.name.name
                        ),
                        format!(
                            "`{iname}` says `{}` {shown}, and a caller is charged that row ([T-INTERFACE-EFFECTS])",
                            m.name.name
                        ),
                    );
                    continue;
                }
                let slot = (tname.clone(), m.name.name.clone());
                if let Some((_, other)) = self.impls.get(&slot) {
                    let other = other.clone();
                    self.error(
                        im.span,
                        format!(
                            "`{tname}` would have two methods `{}`, from `{other}` and `{iname}`",
                            m.name.name
                        ),
                    );
                    continue;
                }
                self.impls.insert(slot, (key, iname.clone()));
            }
        }
    }

    /// `x.m(args)` where `m` comes from an interface: an `impl` for `x`'s type,
    /// or a bound on `x`'s type parameter. `None` when neither applies, and
    /// the call is resolved as it always was.
    fn interface_method(
        &mut self,
        recv_res: &Type,
        recv: &Expr,
        name: &Ident,
        args: &[Expr],
        span: Span,
    ) -> Option<Type> {
        if let Type::Generic(i) = recv_res {
            let i = *i as usize;
            let gname = self
                .current_generics
                .get(i)
                .cloned()
                .unwrap_or_else(|| "T".to_string());
            let bounds = self.current_bounds.get(i).cloned().unwrap_or_default();
            let found = bounds.iter().find_map(|b| {
                self.interfaces
                    .get(b)
                    .and_then(|iface| iface.methods.iter().find(|m| m.name.name == name.name))
                    .cloned()
            });
            let Some(sig) = found else {
                // A free function taking any type still applies to a `T`
                // (UFCS), as it did before interfaces existed.
                if self.lookup_fn(&name.name).is_some() {
                    return None;
                }
                return Some(self.error_help(
                    name.span,
                    format!("`{gname}` has no method `{}`", name.name),
                    format!(
                        "inside a generic body a `{gname}` offers only its bounds' methods — bound it with an interface declaring `{}`, as in `[{gname}: Show]`",
                        name.name
                    ),
                ));
            };
            // What the method may perform is the interface's to say, not the
            // caller's to discover: the call site records the row so the
            // effect pass charges it to whoever made the call, without
            // knowing which `impl` will answer ([T-INTERFACE-EFFECTS]).
            if let Some(fx) = &sig.effects {
                if !fx.is_empty() {
                    let row: std::collections::BTreeSet<String> =
                        fx.iter().map(|p| p.segments[0].name.clone()).collect();
                    self.callee_rows.insert((span.start, span.end), row);
                }
            }
            // `Self` is this type parameter.
            let mut names = vec![String::new(); i + 1];
            names[i] = "Self".to_string();
            let params: Vec<Type> = sig
                .params
                .iter()
                .skip(1)
                .map(|p| self.resolve_type(&p.ty, &names))
                .collect();
            let ret = sig
                .ret
                .as_ref()
                .map(|t| self.resolve_type(t, &names))
                .unwrap_or(Type::Unit);
            if args.len() != params.len() {
                return Some(self.error(
                    span,
                    format!(
                        "`{}` takes {} argument(s), got {}",
                        name.name,
                        params.len(),
                        args.len()
                    ),
                ));
            }
            for (a, p) in args.iter().zip(&params) {
                self.check_expr(a, p);
            }
            self.iface_calls.insert((span.start, span.end));
            let bound = bounds
                .iter()
                .find(|b| {
                    self.interfaces
                        .get(*b)
                        .is_some_and(|iface| iface.methods.iter().any(|m| m.name.name == name.name))
                })
                .cloned()
                .unwrap_or_default();
            self.tir_method(span, crate::tirfacts::MethodRes::Bound(bound));
            return Some(ret);
        }
        let key = self.impl_key(recv_res)?;
        let (qual, _) = self.impls.get(&(key, name.name.clone()))?.clone();
        let (owner, fname) = qual
            .rsplit_once("::")
            .map(|(a, b)| (a.to_string(), b.to_string()))?;
        let info = self.fns.get(&qual).cloned()?;
        let mut full_args: Vec<Expr> = Vec::with_capacity(args.len() + 1);
        full_args.push(recv.clone());
        full_args.extend(args.iter().cloned());
        self.iface_calls.insert((span.start, span.end));
        self.tir_method(span, crate::tirfacts::MethodRes::Interface);
        Some(self.check_fn_call_in(&owner, &fname, &info, &full_args, span))
    }

    /// Settles the bounded calls made in the body just checked
    /// ([T-INTERFACE-UNSATISFIED]).
    fn flush_bound_obligations(&mut self) {
        let obligations = std::mem::take(&mut self.bound_obligations);
        for (v, ifaces, span, caller_bounds, caller_names) in obligations {
            let t = self.uni.resolve(&v);
            match &t {
                Type::Var(_) | Type::Error => {}
                Type::Generic(j) => {
                    let have = caller_bounds.get(*j as usize).cloned().unwrap_or_default();
                    for i in &ifaces {
                        if !have.contains(i) {
                            let shown = caller_names
                                .get(*j as usize)
                                .cloned()
                                .unwrap_or_else(|| self.show(&t));
                            self.error_help(
                                span,
                                format!("`{shown}` is not bounded by `{i}`"),
                                format!("add the bound where it is declared: `[{shown}: {i}]`"),
                            );
                        }
                    }
                }
                _ => {
                    let key = self.impl_key(&t);
                    for i in &ifaces {
                        let ok = key
                            .as_ref()
                            .is_some_and(|k| self.impl_of.contains(&(i.clone(), k.clone())));
                        if !ok {
                            let shown = self.show(&t);
                            self.error_help(
                                span,
                                format!("`{shown}` does not implement `{i}`"),
                                format!("write `impl {i} for {shown} {{ ... }}`"),
                            );
                        }
                    }
                }
            }
        }
    }

    /// The module declaring `t`, when `t` is a linear type.
    fn linear_module_of(&self, t: &Type) -> Option<String> {
        match self.uni.resolve(t) {
            Type::Named { id, .. } => self.linear_types.get(&self.defs.get(id).name).cloned(),
            _ => None,
        }
    }

    fn is_linear(&self, t: &Type) -> bool {
        self.linear_module_of(t).is_some()
    }

    /// Whether `t` keeps a linear value somewhere it could be copied or lost:
    /// in a list, map, tuple, `ref`, or a generic type's argument. A linear
    /// struct holding another linear value directly is fine, and so is a
    /// fallible result or a function's signature.
    fn holds_linear(&self, t: &Type) -> bool {
        fn inside(me: &Checker, t: &Type) -> bool {
            me.is_linear(t) || me.holds_linear(t)
        }
        match self.uni.resolve(t) {
            Type::List(x) | Type::Ref(x) => inside(self, &x),
            Type::Map(k, v) => inside(self, &k) || inside(self, &v),
            Type::Tuple(items) => items.iter().any(|x| inside(self, x)),
            Type::Named { args, .. } => args.iter().any(|x| inside(self, x)),
            Type::Fallible(ok, _) => self.holds_linear(&ok),
            _ => false,
        }
    }

    /// `e` in a position that takes its value -- `let`, `return`, a block's
    /// tail, the right of `=`, a struct literal's field -- consumes it when it
    /// names a linear binding ([T-LINEAR]).
    fn consume(&mut self, e: &Expr) {
        let Expr::Ident(id) = e else { return };
        let Some(b) = self.lookup(&id.name) else {
            return;
        };
        if b.moved || !self.is_linear(&b.ty) {
            return;
        }
        if !b.owned && b.kind == BindKind::Param {
            self.error_help(
                id.span,
                format!(
                    "cannot give away `{}`: it is lent to this function, not owned by it",
                    id.name
                ),
                "take the parameter as `own` to be able to consume it ([T-LINEAR])",
            );
            return;
        }
        self.mark_moved(id);
    }

    /// Every linear value still owed in scopes `from..`, each reported once.
    fn report_unconsumed(&mut self, from: usize, when: &str) {
        let mut owed: Vec<(String, Span, Type)> = Vec::new();
        for sc in self.scopes.iter().skip(from) {
            for (name, b) in sc {
                // An `own` parameter of a linear type declared in this very
                // module is how that module ends one -- `close`, `commit` --
                // so it owes nothing. Everywhere else it is owed like a local.
                let ends_own_type = b.kind == BindKind::Param
                    && self
                        .linear_module_of(&b.ty)
                        .is_some_and(|m| m == self.current_module);
                if b.owned && !b.moved && !ends_own_type && self.is_linear(&b.ty) {
                    owed.push((name.clone(), b.at, b.ty.clone()));
                }
            }
        }
        owed.sort_by_key(|(_, at, _)| at.start);
        for (name, at, ty) in owed {
            let shown = self.show(&ty);
            self.error_help(
                at,
                format!("linear value `{name}` is never consumed{when}"),
                format!(
                    "a `{shown}` must be handed on exactly once -- pass it to the function that finishes it, return it, or move it into another binding ([T-LINEAR])"
                ),
            );
            if let Some(b) = self.lookup_mut(&name) {
                b.moved = true;
            }
        }
    }

    /// Branches that reach the code after them must agree on which linear
    /// values they consumed; a branch that leaves by `return`, `break`,
    /// `continue` or a panic does not reach it and does not vote.
    /// After branches: what the code after them sees. Only the branches that
    /// reach it count -- one that leaves by `return`, `break`, `continue` or
    /// a panic has already gone, and a value it moved on its way out is still
    /// here on every path that did not take it. Counting those made
    /// `if n > 5 { return eat(t) }` followed by `eat(t)` a "use of moved
    /// value" ([T-USE-AFTER-MOVE]).
    fn merge_branches(&mut self, branches: Vec<(MoveSnapshot, bool)>, at: Span) {
        let reaching: Vec<MoveSnapshot> = branches
            .iter()
            .filter(|(_, leaves)| !leaves)
            .map(|(s, _)| s.clone())
            .collect();
        let pick = if reaching.is_empty() {
            branches.iter().map(|(s, _)| s.clone()).collect()
        } else {
            reaching
        };
        self.restore_moved(&pick[0]);
        self.merge_moved(&pick[1..]);
        self.merge_linear(&branches, at);
    }

    fn merge_linear(&mut self, branches: &[(MoveSnapshot, bool)], at: Span) {
        let Some((first, _)) = branches.first() else {
            return;
        };
        for (si, scope) in first.iter().enumerate() {
            for (name, _) in scope {
                let linear = self
                    .scopes
                    .get(si)
                    .and_then(|sc| sc.get(name))
                    .is_some_and(|b| b.owned && self.is_linear(&b.ty));
                if !linear {
                    continue;
                }
                let votes: Vec<bool> = branches
                    .iter()
                    .filter(|(_, leaves)| !leaves)
                    .filter_map(|(snap, _)| {
                        snap.get(si)
                            .and_then(|sc| sc.iter().find(|(n, _)| n == name))
                            .map(|(_, m)| *m)
                    })
                    .collect();
                let Some(&first_vote) = votes.first() else {
                    continue;
                };
                if votes.iter().any(|v| *v != first_vote) {
                    self.error_help(
                        at,
                        format!(
                            "linear value `{name}` is consumed on one path here and not on another"
                        ),
                        "every way through must consume it, or none may ([T-LINEAR])",
                    );
                }
                if let Some(b) = self.scopes.get_mut(si).and_then(|sc| sc.get_mut(name)) {
                    b.moved = first_vote;
                }
            }
        }
    }

    /// Whether a `parallel` block's tasks are independent, so running them on
    /// several cores is indistinguishable from running them in order.
    ///
    /// The shape proved: the block holds nothing but two or more
    /// `spawn { v = e }`, each `v` a different outer `var`, all of one type;
    /// each `e` built from literals, names, arithmetic and calls to functions
    /// whose effect row is empty; and no `e` names another task's `v`. Then
    /// each task reads only values no task writes and writes a place no other
    /// task touches, and performs nothing -- so the only observable result,
    /// the final values of the `v`s, is the same in any order.
    fn parallel_independent(&self, body: &Block) -> bool {
        if body.stmts.len() < 2 {
            return false;
        }
        let mut targets: Vec<String> = Vec::new();
        let mut exprs: Vec<&Expr> = Vec::new();
        let mut ty: Option<Type> = None;
        for st in &body.stmts {
            let Stmt::Spawn { body: task, .. } = st else {
                return false;
            };
            let [Stmt::Assign {
                target: Expr::Ident(v),
                op: AssignOp::Assign,
                value,
                ..
            }] = task.stmts.as_slice()
            else {
                return false;
            };
            let Some(b) = self.lookup(&v.name) else {
                return false;
            };
            let t = self.uni.resolve(&b.ty);
            match &ty {
                None => ty = Some(t),
                Some(prev) if *prev == t => {}
                Some(_) => return false,
            }
            if targets.contains(&v.name) {
                return false;
            }
            targets.push(v.name.clone());
            exprs.push(value);
        }
        exprs.iter().enumerate().all(|(i, e)| {
            self.pure_task_expr(e) && {
                let mut names = Vec::new();
                crate::check::idents_in(e, &mut names);
                targets
                    .iter()
                    .enumerate()
                    .all(|(j, t)| j == i || !names.contains(t))
            }
        })
    }

    /// Literals, names, arithmetic and calls to functions with an empty row.
    fn pure_task_expr(&self, e: &Expr) -> bool {
        match e {
            Expr::Int(..) | Expr::Float(..) | Expr::Bool(..) | Expr::Ident(_) => true,
            Expr::Unary { expr, .. } => self.pure_task_expr(expr),
            Expr::Binary { lhs, rhs, .. } => self.pure_task_expr(lhs) && self.pure_task_expr(rhs),
            Expr::Call { callee, args, .. } => {
                let Expr::Ident(f) = &**callee else {
                    return false;
                };
                if self.lookup(&f.name).is_some() {
                    return false;
                }
                let key = format!("{}::{}", self.current_module, f.name);
                self.fns.contains_key(&key)
                    && self.fn_rows.get(&key).is_some_and(|r| r.is_empty())
                    && args.iter().all(|a| self.pure_task_expr(a))
            }
            _ => false,
        }
    }

    fn collect_fns(&mut self, module: &Module) {
        for decl in &module.decls {
            if let Decl::Fn(f) = decl {
                // A call resolves to the builtin before it consults any
                // declaration, so `fn print(x: str)` was accepted and then
                // never ran — the builtin printed instead, and the reader had
                // written a function nothing calls. Same shape as a redeclared
                // builtin type or a reserved effect name: accepted-and-ignored
                // reads as a definition that took effect.
                if is_builtin_fn_name(&f.name.name) {
                    self.error_help(
                        f.name.span,
                        format!("function name `{}` is built in", f.name.name),
                        "a call would reach the builtin, never this declaration — pick a \
different name, or call it as a method if you meant to extend a type (UFCS)",
                    );
                    continue;
                }
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
                let mut bounds = Vec::new();
                for (g, bs) in f.generics.iter().zip(&f.bounds) {
                    let mut names = Vec::new();
                    for b in bs {
                        let iname = b
                            .segments
                            .last()
                            .map(|s| s.name.clone())
                            .unwrap_or_default();
                        if !self.interfaces.contains_key(&iname) {
                            self.error(
                                b.span,
                                format!("unknown interface `{iname}` bounding `{}`", g.name),
                            );
                            continue;
                        }
                        names.push(iname);
                    }
                    bounds.push(names);
                }
                bounds.resize(generics.len(), Vec::new());
                let info = FnInfo {
                    is_pub: f.is_pub,
                    generics: generics.len(),
                    bounds,
                    params,
                    modes: f.params.iter().map(|p| p.mode).collect(),
                    ret,
                    err,
                };
                if f.is_export {
                    self.check_export(f, &key, &info);
                }
                self.fns.insert(key, info);
            }
        }
    }

    /// `export fn` ([T-EXPORT]): C reaches it by its own name, so the name is
    /// one symbol in the program and the signature is one C can call.
    fn check_export(&mut self, f: &FnDecl, key: &str, info: &FnInfo) {
        let name = &f.name.name;
        if crate::mem::reserved_symbol(name) {
            self.error_help(
                f.name.span,
                format!(
                    "`{name}` cannot be exported: the toolchain gives that name to its own code"
                ),
                "`main` is the program's C entry, and names beginning `grasm_`, `g_`, `gc_`, \
`gp_`, `gpf_`, `gv_` or `gx_` are the runtime's and the compiler's",
            );
            return;
        }
        if self.export_names.contains_key(name) {
            self.error_help(
                f.name.span,
                format!("`{name}` is exported twice"),
                "an export's name is its C symbol, and a program can define a symbol once",
            );
            return;
        }
        self.export_names.insert(name.clone(), f.name.span);
        if let Some(sig) = self.c_entry_sig(key, info, f.name.span, "be exported") {
            self.c_facts.sigs.insert(key.to_string(), sig);
            self.c_facts.exports.push(key.to_string());
        }
    }

    /// TypeExpr → Type. `generics` are the in-scope generic parameter names.
    /// Whether a written type reaches `name`, or any other alias that is also
    /// stuck, so a mutually recursive pair is reported as such rather than as
    /// two unknown names.
    fn names_itself(te: &TypeExpr, name: &str, stuck: &std::collections::HashSet<String>) -> bool {
        match te {
            TypeExpr::Name { path, args, .. } => {
                let here = path
                    .segments
                    .last()
                    .is_some_and(|s| s.name == name || stuck.contains(&s.name));
                here || args.iter().any(|a| Self::names_itself(a, name, stuck))
            }
            TypeExpr::List(inner, _) | TypeExpr::Ref(inner, _) => {
                Self::names_itself(inner, name, stuck)
            }
            TypeExpr::Fallible(a, b, _) => {
                Self::names_itself(a, name, stuck) || Self::names_itself(b, name, stuck)
            }
            TypeExpr::Tuple(items, _) | TypeExpr::Union(items, _) => {
                items.iter().any(|i| Self::names_itself(i, name, stuck))
            }
            TypeExpr::Fn { params, ret, .. } => {
                params.iter().any(|p| Self::names_itself(p, name, stuck))
                    || Self::names_itself(ret, name, stuck)
            }
        }
    }

    fn resolve_type(&mut self, te: &TypeExpr, generics: &[String]) -> Type {
        match te {
            TypeExpr::Ref(inner, _) => Type::Ref(Box::new(self.resolve_type(inner, generics))),
            TypeExpr::Fn {
                params,
                ret,
                effects,
                ..
            } => {
                let ps = params
                    .iter()
                    .map(|p| self.resolve_type(p, generics))
                    .collect();
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
                    // [T-INT-FIXED], [T-U64]: each its own type, wrapping at
                    // its width.
                    "i8" | "i16" | "i32" | "u8" | "u16" | "u32" | "u64" => {
                        IntKind::from_name(name).map(Type::Int)
                    }
                    "i64" | "int" => Some(Type::I64),
                    // These used to be silent aliases, and a silent alias is a
                    // promise broken without a word: `f32` was a double, and
                    // `usize` would say "the platform's width" about a type
                    // that has one width everywhere. Refused until they are
                    // real.
                    "usize" | "f32" => {
                        let (what, instead) = match name.as_str() {
                            "f32" => ("single-precision floating point", "f64"),
                            _ => ("a platform-sized integer", "i64"),
                        };
                        let write = if name == "usize" {
                            "`u64` or `i64`"
                        } else {
                            "`f64`"
                        };
                        return self.error_help(
                            *span,
                            format!("`{name}` is not implemented yet: Grasm does not have {what}"),
                            format!("it used to be another name for `{instead}` and behaved like one; write {write} ([T-INT-FIXED])"),
                        );
                    }
                    "f64" | "float" => Some(Type::F64),
                    "bool" => Some(Type::Bool),
                    "str" => Some(Type::Str),
                    "bytes" => Some(Type::Bytes),
                    "StrBuilder" => Some(Type::StrBuf),
                    "ByteBuilder" => Some(Type::ByteBuf),
                    "addr" => Some(Type::Addr),
                    // `Type::Range` has displayed as `range` since ranges
                    // became values, so the name was already in the language's
                    // error messages -- and nowhere else. Nothing parsed it, so
                    // a range could be bound and never declared: not a
                    // parameter, not a result, not a field. A value you can
                    // hold and cannot pass is stuck inside one function body,
                    // and the native backend had accepted the spelling all
                    // along (F53).
                    "range" => Some(Type::Range),
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
                        return self
                            .error(*span, "`Map` takes exactly 2 type arguments: Map[K, V]");
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
                        self.tir_bind(&p.name, &ty);
                        scope.insert(
                            p.name.name.clone(),
                            Binding {
                                ty,
                                mutable: false,
                                kind: BindKind::Generated,
                                moved: false,
                                owned: false,
                                at: p.span,
                                decl_loop_depth: 0,
                            },
                        );
                    }
                    self.scopes.push(scope);
                    self.check_block_discarded(&t.body);
                    self.scopes.pop();
                }
                Decl::Bench(b) => {
                    self.fn_ret = Type::Unit;
                    self.fn_err = Some(self.uni.fresh());
                    self.err_open = true;
                    self.scopes.push(HashMap::new());
                    self.check_block_discarded(&b.body);
                    self.scopes.pop();
                }
                _ => {}
            }
        }
    }

    /// Whether any branch of a tail `if`/`match` is a call to a function that
    /// can fail.
    ///
    /// Such a branch produces the whole `T ! E`, not the ok half, so pushing
    /// the ok half into it is a demand it cannot meet:
    /// `if c { risky(1) } else { risky(n) }` in a `-> i64 ! E` function was
    /// refused as "can fail but `i64` is required" while the identical
    /// `match` and the same `if` bound to an annotated `let` both compiled.
    /// Syntax alone cannot tell — `risky(1)` and `plain(1)` look the same —
    /// so this asks what the callee actually returns.
    fn branch_calls_fallible(&self, e: &Expr) -> bool {
        fn tail(b: &Block) -> Option<&Expr> {
            match b.stmts.last() {
                Some(Stmt::Expr(e)) => Some(e),
                _ => None,
            }
        }
        match e {
            Expr::Call { callee, .. } => match &**callee {
                Expr::Ident(id) => self.lookup_fn(&id.name).is_some_and(|f| f.err.is_some()),
                _ => false,
            },
            Expr::If {
                then_block,
                else_branch,
                ..
            } => {
                let t = tail(then_block).is_some_and(|e| self.branch_calls_fallible(e));
                let f = match else_branch.as_deref() {
                    Some(ElseBranch::Else(b)) => {
                        tail(b).is_some_and(|e| self.branch_calls_fallible(e))
                    }
                    Some(ElseBranch::ElseIf(inner)) => self.branch_calls_fallible(inner),
                    None => false,
                };
                t || f
            }
            Expr::Match { arms, .. } => arms.iter().any(|a| self.branch_calls_fallible(&a.body)),
            _ => false,
        }
    }

    /// Whether a value of this type *carries* failures inside a container it
    /// was built from — a list, a tuple, a map, a `ref`.
    ///
    /// Deliberately structural: a failure stored in a user struct's field is
    /// part of that struct's design and is read wherever the struct is, while
    /// `[risky(x)]` written as a statement can only be throwing them away.
    fn holds_fallible(&mut self, t: &Type) -> bool {
        self.holds_fallible_at(t, 0)
    }

    fn holds_fallible_at(&mut self, t: &Type, depth: usize) -> bool {
        if depth > 8 {
            return false;
        }
        match self.uni.shallow_resolve(t) {
            Type::Fallible(..) => true,
            Type::List(e) | Type::Ref(e) => self.holds_fallible_at(&e, depth + 1),
            Type::Map(_, v) => self.holds_fallible_at(&v, depth + 1),
            Type::Tuple(parts) => parts.iter().any(|p| self.holds_fallible_at(p, depth + 1)),
            // `Some(save())`, `Report{ written: save() }`, `W(save())` — one
            // constructor was enough to launder a failure past the rule, and
            // the recursion stopped at the type-argument boundary so even a
            // list of them slipped through. A named type's arguments and its
            // own fields carry values like any other container.
            Type::Named { id, args } => {
                if args.iter().any(|a| self.holds_fallible_at(a, depth + 1)) {
                    return true;
                }
                let fields: Vec<Type> = match &self.defs.get(id).kind {
                    TypeDefKind::Struct { fields } => {
                        fields.iter().map(|(_, t)| t.clone()).collect()
                    }
                    TypeDefKind::Sum { variants } => variants
                        .iter()
                        .flat_map(|v| v.fields.iter().map(|(_, t)| t.clone()))
                        .collect(),
                };
                fields.iter().any(|f| self.holds_fallible_at(f, depth + 1))
            }
            _ => false,
        }
    }

    fn check_fn(&mut self, f: &FnDecl) {
        let info = match self.lookup_fn(&f.name.name) {
            Some(i) => i,
            None => return,
        };
        // Generic params stay opaque (`Generic(i)`) while checking the body,
        // and are in scope for annotations written inside it.
        // `main` produces no value: nothing reads one, and a return type on
        // it read as a status code that was silently discarded — the program
        // exited 0 ([E-ENTRY]).
        if f.name.name == "main" && self.current_module == "main" {
            if !matches!(info.ret, Type::Unit) {
                self.error_help(
                    f.name.span,
                    "`main` produces no value".to_string(),
                    "drop the return type — a program's status comes from `exit(n)`, and an error reaching `main` is reported and exits 1",
                );
            }
            // And it takes none. The return type was refused and the
            // parameters were not, so `fn main(x: i64)` checked clean and then
            // panicked with `unknown name \`x\`` — a compiler-internal message
            // arriving at run time, where the whole point of the check is that
            // types are settled before anything runs ([E-ENTRY]).
            if let Some(p) = f.params.first() {
                self.error_help(
                    p.name.span,
                    "`main` takes no parameters".to_string(),
                    "nothing calls `main`, so there is nothing to pass it — read the command line with `args()`",
                );
            }
        }
        self.hof_closures.clear();
        collect_hof_closures_block(&f.body, &mut self.hof_closures);
        self.current_generics = f.generics.iter().map(|g| g.name.clone()).collect();
        self.current_bounds = info.bounds.clone();
        self.key_generic_owner = if f.generics.is_empty() {
            None
        } else {
            Some(format!("{}::{}", self.current_module, f.name.name))
        };
        self.fn_ret = info.ret.clone();
        self.err_open = f.name.name == "main" && info.err.is_none();
        self.fn_err = if f.name.name == "main" && info.err.is_none() {
            Some(self.uni.fresh()) // main may propagate; runtime reports
        } else {
            info.err.clone()
        };
        let mut scope = HashMap::new();
        for (p, ty) in f.params.iter().zip(&info.params) {
            self.tir_bind(&p.name, ty);
            scope.insert(
                p.name.name.clone(),
                Binding {
                    ty: ty.clone(),
                    mutable: p.mode == ParamMode::Inout,
                    kind: BindKind::Param,
                    moved: false,
                    owned: p.mode == ParamMode::Own,
                    at: p.name.span,
                    decl_loop_depth: 0,
                },
            );
        }
        self.scopes.push(scope);
        self.fn_scope_base = self.scopes.len() - 1;
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
            )) // An `if` or a `match` none of whose branches builds the whole
               // `T ! E` produces the ok half, so the ok half is what reaches them.
               // Without this `fn f(n: i64) -> f64 ! E { if c { 4.5 } else { 6 } }`
               // was refused for branches "of different types" while the same body
               // with a plain `-> f64` compiled — [T-INT-TO-FLOAT-CARRIED] names an
               // `if`'s branches and a `match`'s arms explicitly.
        ) || matches!(
            f.body.stmts.last(),
            Some(Stmt::Expr(e)) if yields_ok_half(e) && !self.branch_calls_fallible(e)
        );
        // `Ok(3)` / `Err(e)` in tail position build the *whole* `T ! E`, so
        // that is what reaches them ([T-OK-ERR]).
        let tail_is_wrapper = matches!(
            f.body.stmts.last(),
            Some(Stmt::Expr(Expr::Call { callee, .. }))
                if matches!(&**callee, Expr::Ident(id) if id.name == "Ok" || id.name == "Err")
        // And an `if` or `match` whose branches all do: there the whole
        // `T ! E` is what every branch builds, so it is what reaches them.
        ) || matches!(
            f.body.stmts.last(),
            // Or one where *some* branch does: a mixing `if`/`match`, however
            // it is nested, merges to the whole `T ! E` and each of its own
            // branches may then give either half ([T-OK-ERR-RETURN]).
            Some(Stmt::Expr(e)) if yields_whole(e) || yields_some_whole(e)
        );
        let want_tail = if ret == Type::Unit {
            None
        } else if let (true, Some(e)) = (tail_is_wrapper, self.fn_err.clone()) {
            Some(Type::Fallible(Box::new(ret.clone()), Box::new(e)))
        } else if self.fn_err.is_none() || tail_is_literal {
            Some(ret.clone())
        } else if let (Some(e), true) = (
            self.fn_err.clone(),
            // Only a *branching* tail. A plain one giving the ok half —
            // `a / b` in a `-> i64 ! MathError` — is the ordinary case and
            // wants the half, not the whole.
            matches!(
                f.body.stmts.last(),
                Some(Stmt::Expr(Expr::If { .. } | Expr::Match { .. }))
            ),
        ) {
            // A fallible body whose tail *mixes* the halves — one branch
            // `Err(e)`, another a bare value — is expecting the whole `T ! E`,
            // and reached no expectation at all. So the merge had only the
            // branches to go on: `-> f64 ! E` with `if c { Err(Bad) } else
            // { 4 }` produced `i64 ! E` and was refused, while the identical
            // body under a `let v: f64 ! E =` annotation was accepted.
            // [T-OK-ERR-RETURN] says the rule is about the *position*, not
            // about how it was reached; each branch may still give either
            // half.
            Some(Type::Fallible(Box::new(ret.clone()), Box::new(e)))
        } else {
            None
        };
        // Only when the body actually ends in an expression: an explicit
        // `return` is not "the last expression of the body", and saying so
        // would attach the wrong advice to a plain type mismatch.
        if want_tail.is_some() && matches!(f.body.stmts.last(), Some(Stmt::Expr(_))) {
            self.tail_fn_name = Some(f.name.name.clone());
        }
        let body_ty = self.check_block_with(&f.body, want_tail.as_ref());
        self.tail_fn_name = None;
        // A function that returns nothing throws its tail away, and throwing
        // away a failure is using a fallible value as if it were `T`.
        // `fn write_report(p, b) { write_file(p, b) }` printed "wrote the
        // report", exited 0, said nothing on stderr and left no file —
        // verbatim the failure [T-FALLIBLE-UNHANDLED] exists to prevent, one
        // position away from where it was already caught. A statement that
        // *leaves* — `exit(n)`, `panic(m)`, `fail(e)` — produces no value to
        // discard.
        if ret == Type::Unit {
            if let Some(Stmt::Expr(e)) = f.body.stmts.last() {
                let leaves = matches!(e, Expr::Call { callee, .. }
                    if matches!(&**callee, Expr::Ident(id)
                        if matches!(id.name.as_str(), "exit" | "panic" | "fail")));
                let resolved = self.uni.shallow_resolve(&body_ty);
                // `main` has an open error slot, so a directly fallible tail
                // there is propagated rather than discarded ([E-MAIN-ERR]).
                // A *container* of failures is not: the slot carries one
                // failure, not a list of them, so `fn main() { xs.map(risky) }`
                // threw away every one of them — silent, exit 0. The whole
                // check was skipped for `main`, and only this half should be.
                let may_propagate = self.fn_err.is_some();
                if !leaves && !may_propagate && matches!(resolved, Type::Fallible(..)) {
                    let shown = self.show(&body_ty);
                    self.error_help(
                        e.span(),
                        format!(
                            "this expression can fail (`{shown}`) and its failure is discarded"
                        ),
                        "use `try` to propagate it, `match` to handle both cases, or `??` with a fallback to say what should happen instead",
                    );
                } else if !leaves
                    && !matches!(resolved, Type::Fallible(..))
                    && self.holds_fallible(&resolved)
                {
                    // The tail of a `()`-returning function throws its value
                    // away like any other discarded statement, and the
                    // container rule was checked only for statements: adding
                    // one line after `paths.map(|p| write_file(p, body))`
                    // caught it, and leaving it as the last line did not —
                    // "wrote the report", exit 0, nothing on stderr, no file
                    // ([T-FALLIBLE-UNHANDLED]).
                    let shown = self.show(&body_ty);
                    self.error_help(
                        e.span(),
                        format!(
                            "this expression holds failures (`{shown}`) and they are discarded"
                        ),
                        "bind it and handle the failures it holds — a value nothing reads \
is a failure nothing reports",
                    );
                }
            }
        }
        let ends_with_return = matches!(f.body.stmts.last(), Some(Stmt::Return(..)));
        // A fallible function's body may produce either the ok half — the
        // ordinary case, wrapped in `Ok` on the way out — or the whole
        // `T ! E`, which is what `Ok(v)` and `Err(e)` build ([T-OK-ERR]).
        // Only the first was accepted, so the constructors worked everywhere
        // except the position a function's own result is written in.
        let whole = self
            .fn_err
            .clone()
            .map(|e| Type::Fallible(Box::new(ret.clone()), Box::new(e)));
        let fits_whole = whole.as_ref().is_some_and(|w| self.uni.unify(&body_ty, w));
        // The row check has to run here too: a function body's value is
        // compared with `unify`, not `check_expr`, so returning a named
        // function was the one coercion nothing looked at.
        if !ends_with_return {
            self.check_row_fits(&body_ty, &ret, f.body.span);
        }
        if ret != Type::Unit && !ends_with_return && !fits_whole && !self.uni.unify(&body_ty, &ret)
        {
            let span = f.body.stmts.last().map(stmt_span).unwrap_or(f.body.span);
            let want = self.show(&ret);
            let got = self.show(&body_ty);
            self.error_help(
                span,
                format!(
                    "function `{}` must return `{want}` but its body produces `{got}`",
                    f.name.name
                ),
                "the last expression of the body is the return value",
            );
        }
        if !ends_with_return && !block_leaves(&f.body) {
            let base = self.scopes.len() - 1;
            self.report_unconsumed(base, "");
        }
        self.scopes.pop();
        self.current_generics.clear();
        self.current_bounds.clear();
        self.key_generic_owner = None;
        self.flush_bound_obligations();
    }

    // ------------------------------------------------------------------
    // Statements & blocks
    // ------------------------------------------------------------------

    /// Returns the block's value type (last expression statement, else unit).
    /// Whether an expression is an integer literal, possibly negated.
    ///
    /// These adopt the type they meet ([T-INT-TO-FLOAT]), so in a list, a map
    /// or a set of `match` arms they are checked *after* everything else —
    /// otherwise the first element fixed the type and `[1, 2.5]` was an
    /// error while `[2.5, 1]` was fine, an asymmetry [E-NUM-PROMOTE] already
    /// records as fixed for comparisons.
    fn is_int_literal(e: &Expr) -> bool {
        match e {
            Expr::Int(..) => true,
            Expr::Unary {
                op: UnaryOp::Neg,
                expr,
                ..
            } => matches!(**expr, Expr::Int(..)),
            _ => false,
        }
    }

    /// An expression made only of integer literals and the shapes
    /// [T-INT-TO-FLOAT-CARRIED] says an expected type reaches through.
    ///
    /// Such an expression decides nothing about its own type, so it has to be
    /// checked *after* whatever does. The test looked for a bare literal, so
    /// `match n { 1 -> Some(4.5)  _ -> Some(3) }` inferred the second arm as
    /// `Option[i64]` and reported a mismatch against a program the clause
    /// calls well typed — while `Some(3)` in every other carrier position was
    /// accepted, and the same two branches spelled `3` and `4.5` were too.
    fn defers_to_its_context(e: &Expr) -> bool {
        match e {
            Expr::Int(..) => true,
            Expr::Unary {
                op: UnaryOp::Neg,
                expr,
                ..
            } => Self::defers_to_its_context(expr),
            Expr::List(items, _) | Expr::Tuple(items, _) => {
                !items.is_empty() && items.iter().all(Self::defers_to_its_context)
            }
            Expr::Map(entries, _) => {
                !entries.is_empty() && entries.iter().all(|(_, v)| Self::defers_to_its_context(v))
            }
            Expr::Call { callee, args, .. } => {
                matches!(&**callee, Expr::Ident(id)
                    if matches!(id.name.as_str(), "Some" | "Ok" | "Err"))
                    && args.len() == 1
                    && Self::defers_to_its_context(&args[0])
            }
            _ => false,
        }
    }

    fn check_block(&mut self, b: &Block) -> Type {
        self.check_block_with(b, None)
    }

    /// A block whose value is thrown away — a loop body, or a branch of an
    /// `if` written as a statement. Its *tail* is discarded too, so a
    /// fallible value there is as unhandled as one on any other line.
    fn check_block_discarded(&mut self, b: &Block) -> Type {
        let saved = std::mem::replace(&mut self.discarding, true);
        let t = self.check_block_with(b, None);
        self.discarding = saved;
        t
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
        self.deferred_fields.push(Vec::new());
        let mut ty = Type::Unit;
        let n = b.stmts.len();
        for (i, stmt) in b.stmts.iter().enumerate() {
            match (i + 1 == n, want, stmt) {
                (true, Some(w), Stmt::Expr(e)) if *w != Type::Unit => {
                    let w = w.clone();
                    // Armed here and nowhere else. Set once for the whole
                    // body, the "must return" template fired on whatever
                    // `check_expr` ran first — so `fn g() -> i64 { let r: str
                    // = 1  2 }` was reported as `g` returning `str`, on a line
                    // that is not the last expression, with a help saying it
                    // was.
                    self.tail_fn = self.tail_fn_name.take();
                    self.check_expr(e, &w);
                    self.tail_fn = None;
                    ty = w;
                }
                _ => {
                    // The tail of a block whose value is thrown away is
                    // discarded too — and a function returning `()` throws its
                    // tail away. `fn write_report(p, b) { write_file(p, b) }`
                    // printed "wrote the report", exited 0, said nothing on
                    // stderr and left no file: verbatim the failure
                    // [T-FALLIBLE-UNHANDLED] exists to prevent, one position
                    // from where it was already caught.
                    let disc = i + 1 < n || self.discarding;
                    let saved = std::mem::replace(
                        &mut self.discarding,
                        disc && matches!(stmt, Stmt::Expr(_)),
                    );
                    ty = self.check_stmt(stmt);
                    self.discarding = saved;
                    // A statement that is not the block's value throws that
                    // value away, and throwing away a failure is using a
                    // fallible value as if it were `T`. `write_file` to an
                    // unwritable path printed "wrote the report", exit 0,
                    // nothing on stderr and no file — and every user function
                    // that can fail did the same ([T-FALLIBLE-UNHANDLED]).
                    // Not resolved yet, and it may become fallible: a call
                    // to a closure has the closure's body's type, and closure
                    // bodies are checked after the block that binds them.
                    // `let f = |x| risky(x)` then `f(-1)` on its own line
                    // printed the next line and exited 0 — the silence this
                    // rule exists to stop.
                    if disc
                        && matches!(stmt, Stmt::Expr(_))
                        && matches!(self.uni.shallow_resolve(&ty), Type::Var(_))
                    {
                        self.pending_discards.push((stmt_span(stmt), ty.clone()));
                    }
                    if disc && matches!(stmt, Stmt::Expr(_)) {
                        let resolved = self.uni.shallow_resolve(&ty);
                        if matches!(resolved, Type::Fallible(..)) {
                            let shown = self.show(&ty);
                            self.error_help(
                                stmt_span(stmt),
                                format!(
                                    "this expression can fail (`{shown}`) and its failure is discarded"
                                ),
                                "use `try` to propagate it, `match` to handle both cases, or `??` \
with a fallback to say what should happen instead",
                            );
                        } else if self.holds_fallible(&resolved) {
                            // One pair of brackets was enough to get the old
                            // silence back: `[risky(-1)]` and `[1, 2].map(risky)`
                            // as bare statements threw away every failure they
                            // held and said nothing ([T-FALLIBLE-UNHANDLED]).
                            // `map` keeping the failures ([T-FALLIBLE-VALUE])
                            // only matters while somebody reads them.
                            let shown = self.show(&ty);
                            self.error_help(
                                stmt_span(stmt),
                                format!(
                                    "this expression holds failures (`{shown}`) and they are discarded"
                                ),
                                "bind it and handle the failures it holds — a value nothing reads \
is a failure nothing reports",
                            );
                        }
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
        self.settle_deferred_fields();
        // A block's last expression is its value, so a linear name there is
        // handed on rather than dropped ([T-LINEAR]).
        if let Some(Stmt::Expr(e)) = b.stmts.last() {
            self.consume(e);
        }
        if !block_leaves(b) {
            let top = self.scopes.len() - 1;
            self.report_unconsumed(top, "");
        }
        self.scopes.pop();
        ty
    }

    /// Resolves the field reads this block deferred, now that its closure
    /// bodies have been checked.
    ///
    /// A receiver still unknown here is genuinely unknown — nothing later can
    /// say more about it — so the original refusal stands, with the same
    /// message it would have had. One that resolved is checked exactly as a
    /// field read written after the call site would have been.
    fn settle_deferred_fields(&mut self) {
        let Some(pending) = self.deferred_fields.pop() else {
            return;
        };
        for d in pending {
            let got = self.field_type_of(&d.recv, &d.name, d.name.span);
            self.uni.unify(&d.out, &got);
        }
    }

    fn check_deferred_closure(&mut self, d: DeferredClosure) {
        self.scopes.push(HashMap::new());
        self.closure_floors.push(self.scopes.len() - 1);
        // A `go` must be lexically inside its block, not inside something
        // that could run after the block has ended.
        let saved_conc = std::mem::replace(&mut self.concurrent_depth, 0);
        let saved_touch = std::mem::replace(&mut self.closure_touches_outside_ref, false);
        for (p, t) in d.params.iter().zip(&d.param_tys) {
            self.bind(p, t.clone(), false);
        }
        self.deferred_closures.push(Vec::new());
        self.deferred_fields.push(Vec::new());
        // A closure is a function: `return` inside one returns from *it*, and
        // `break` cannot reach a loop outside it. Checked against the
        // enclosing function's return type instead, `let g: fn(i64) -> i64 =
        // |x| { return "oops" }` was accepted inside a `fn f() -> str`, and
        // the string came back where an integer was promised — a panic under
        // one engine and printed heap addresses under the other.
        let saved_ret = std::mem::replace(&mut self.fn_ret, d.ret.clone());
        let saved_err = self.fn_err.take();
        let saved_loop = std::mem::replace(&mut self.loop_depth, 0);
        let produced = self.infer_expr(&d.body);
        self.fn_ret = saved_ret;
        self.fn_err = saved_err;
        self.loop_depth = saved_loop;
        // A closure literal *inside* this body waits for its own call site,
        // and this body's parameters are in scope for it — here and nowhere
        // else. Left on the enclosing block's list it was checked after this
        // scope was gone, so `let mk = |n| |m| n + m` reported "unknown name
        // `n`" about the parameter one line to its left.
        while let Some(next) = self.deferred_closures.last_mut().and_then(|v| v.pop()) {
            self.check_deferred_closure(next);
        }
        self.deferred_closures.pop();
        self.settle_deferred_fields();
        self.concurrent_depth = saved_conc;
        self.closure_floors.pop();
        self.scopes.pop();
        if self.closure_touches_outside_ref {
            self.ref_touching_closures.insert(d.at);
        }
        self.closure_touches_outside_ref = saved_touch;
        // Nothing ever said what the parameters are. [T-FN-TYPE]: "A closure
        // bound to a name and never called, with no function type to take
        // them from, still has nothing to infer from and is an error."
        // Accepted silently, it compiled to a function whose parameter types
        // the backend then had to guess.
        if let Some(p) = d
            .params
            .iter()
            .zip(&d.param_tys)
            .find(|(_, t)| matches!(self.uni.shallow_resolve(t), Type::Var(_)))
            .map(|(p, _)| p.clone())
        {
            self.error_help(
                p.span,
                format!("nothing says what `{}` is", p.name),
                "a closure's parameter types come from where it is used — call it, pass it somewhere with a function type, or annotate the binding as in `let f: fn(i64) -> i64 = |x| x + 1`",
            );
        }
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
                // Whether the initializer is a closure that reaches a `ref`
                // from outside itself, so that handing this name to a parallel
                // operation can be refused the way the inline spelling is.
                let saw_ref = std::mem::replace(&mut self.closure_touches_outside_ref, false);
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
                if let Expr::Closure { span, .. } = init {
                    self.let_closure_span.insert(name.name.clone(), span.start);
                } else {
                    self.let_closure_span.remove(&name.name);
                }
                self.closure_touches_outside_ref = saw_ref;
                self.consume(init);
                self.bind(name, bound_ty, *mutable);
                Type::Unit
            }
            Stmt::Assign {
                target,
                op,
                value,
                span,
            } => {
                // Re-initializing a moved variable revives it -- unless what
                // it holds is linear and still owed ([T-LINEAR]): the write
                // would drop it unconsumed.
                if *op == AssignOp::Assign {
                    if let Expr::Ident(id) = target {
                        let name = id.name.clone();
                        let owed = self
                            .lookup(&name)
                            .is_some_and(|b| b.owned && !b.moved && self.is_linear(&b.ty));
                        if owed {
                            self.error_help(
                                id.span,
                                format!("assigning to `{name}` drops a linear value that was never consumed"),
                                "consume the value it holds first -- pass it to the function that finishes it",
                            );
                        }
                        if let Some(b) = self.lookup_mut(&name) {
                            b.moved = false;
                        }
                    }
                    self.consume(value);
                }
                self.check_assign_target(target);
                {
                    let mut root = target;
                    while let Expr::Field { recv, .. } | Expr::Index { recv, .. } = root {
                        root = recv;
                    }
                    if let Expr::Ident(id) = root {
                        self.reject_task_write(id);
                    }
                }
                let target_ty = self.infer_expr(target);
                self.reject_handler_escape(target, &target_ty);
                if *op == AssignOp::Assign {
                    self.check_expr(value, &target_ty);
                } else {
                    // Compound: numeric (or str for +=).
                    self.check_expr(value, &target_ty);
                    let t = self.uni.shallow_resolve(&target_ty);
                    let ok = matches!(t, Type::I64 | Type::Int(_) | Type::F64 | Type::Error)
                        || (t == Type::Str && *op == AssignOp::Add);
                    // `x += 1` on a `u8` wraps like `x = x + 1` does; the
                    // statement is not an expression, so its width is
                    // recorded by hand for the engines.
                    if let Type::Int(k) = t {
                        self.narrow_assigns.insert((span.start, span.end), k);
                    }
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
                        // A fallible function's `return` takes either half,
                        // exactly as its tail expression does ([T-OK-ERR-RETURN]).
                        // Only the tail was taught this, so `return Ok(1)` was
                        // "this expression can fail but `i64` is required" —
                        // a message pointing at the wrong problem, in the
                        // shape early exits are written in.
                        // An `if` or `match` counts too: its branches take
                        // either half individually, so the whole type is what
                        // reaches them. `return if c { 3 } else { Ok(4) }` was
                        // "this expression can fail but `i64` is required" —
                        // the message this whole rule replaced, one shape
                        // further out.
                        let wrapped = yields_whole_expr(e)
                            || matches!(e, Expr::If { .. } | Expr::Match { .. });
                        match (wrapped, self.fn_err.clone()) {
                            (true, Some(err)) => {
                                let whole = Type::Fallible(Box::new(ret.clone()), Box::new(err));
                                self.check_expr(e, &whole);
                            }
                            _ => self.check_expr(e, &ret),
                        }
                    }
                    None => {
                        if self.uni.shallow_resolve(&ret) != Type::Unit {
                            let want = self.show(&ret);
                            self.error(*span, format!("this function must return `{want}`"));
                        }
                    }
                }
                if let Some(v) = value.as_ref() {
                    self.consume(v);
                }
                let base = match self.closure_floors.last() {
                    Some(f) if *f + 1 > self.fn_scope_base => *f + 1,
                    _ => self.fn_scope_base,
                };
                self.report_unconsumed(base, " before this `return`");
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
                self.check_block_discarded(body);
                self.loop_depth -= 1;
                Type::Unit
            }
            Stmt::Parallel {
                body,
                concurrent: true,
                ..
            } => {
                // Its own statements run in the task that reaches it; each
                // `go` inside starts another, and the block ends when they all
                // have.
                self.concurrent_depth += 1;
                self.check_block_discarded(body);
                self.concurrent_depth -= 1;
                Type::Unit
            }
            Stmt::Spawn {
                body,
                span,
                concurrent: true,
            } => {
                if self.concurrent_depth == 0 {
                    self.error_help(
                        *span,
                        "`go` outside a `concurrent` block",
                        "wrap it in `concurrent { ... }`, which does not end until every task it started has",
                    );
                }
                // A task sees its surroundings by value, as a closure does --
                // so the closure rules apply: it cannot capture a linear value,
                // move a binding from outside, or write one.
                let saved_loop = self.loop_depth;
                let saved_go = self.go_floor;
                self.loop_depth = 0;
                self.go_floor = Some(self.scopes.len());
                self.closure_floors.push(self.scopes.len());
                // A `go` must be lexically inside its block, not inside something
                // that could run after the block has ended.
                let saved_conc = std::mem::replace(&mut self.concurrent_depth, 0);
                self.check_block_discarded(body);
                self.concurrent_depth = saved_conc;
                self.closure_floors.pop();
                self.go_floor = saved_go;
                self.loop_depth = saved_loop;
                Type::Unit
            }
            Stmt::Parallel { body, span, .. } => {
                self.parallel_depth += 1;
                let saved_floor = self.parallel_floor;
                // Bindings from here down live only for the collection pass;
                // tasks run after this block's scope is gone.
                self.parallel_floor = Some(self.scopes.len());
                self.check_block_discarded(body);
                self.parallel_floor = saved_floor;
                self.parallel_depth -= 1;
                if self.parallel_independent(body) {
                    self.parallel_ok.insert(span.start);
                }
                Type::Unit
            }
            Stmt::Spawn { body, span, .. } => {
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
                self.check_block_discarded(body);
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
                self.check_block_discarded(body);
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
                self.check_block_discarded(body);
                self.loop_depth -= 1;
                self.scopes.pop();
                Type::Unit
            }
        }
    }

    fn bind(&mut self, name: &Ident, ty: Type, mutable: bool) {
        self.tir_bind(name, &ty);
        let decl_loop_depth = self.loop_depth;
        self.scopes.last_mut().unwrap().insert(
            name.name.clone(),
            Binding {
                ty,
                mutable,
                kind: BindKind::Local,
                moved: false,
                owned: true,
                at: name.span,
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
                    .with_help(
                        "move a copy instead, or restructure so the move happens outside the loop",
                    ),
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
            Type::I64
                | Type::Int(_)
                | Type::F64
                | Type::Bool
                | Type::Unit
                | Type::Range
                | Type::Ref(_)
        )
    }

    /// Whether a value could have been built before the region began.
    ///
    /// A literal is built here. A name is judged by where it was declared —
    /// the same floor the escape rule uses. Anything else is treated as
    /// having come from outside, which is the safe answer.
    fn arrived_from_outside_region(&self, e: &Expr) -> bool {
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
        self.carries_in(t, want_fn, false, seen)
    }

    /// `carries`, where `actors_ok` stops at an actor's handle: a task may
    /// share one ([E-ACTOR]), though a parallel operation may not.
    fn carries_in(&self, t: &Type, want_fn: bool, actors_ok: bool, seen: &mut Vec<usize>) -> bool {
        match self.uni.resolve(t) {
            Type::Ref(inner) => !want_fn || self.carries_in(&inner, want_fn, actors_ok, seen),
            Type::Fn { params, ret, .. } => {
                want_fn
                    || params
                        .iter()
                        .any(|p| self.carries_in(p, want_fn, actors_ok, seen))
                    || self.carries_in(&ret, want_fn, actors_ok, seen)
            }
            Type::List(e) => self.carries_in(&e, want_fn, actors_ok, seen),
            Type::Map(k, v) => {
                self.carries_in(&k, want_fn, actors_ok, seen)
                    || self.carries_in(&v, want_fn, actors_ok, seen)
            }
            Type::Tuple(items) => items
                .iter()
                .any(|i| self.carries_in(i, want_fn, actors_ok, seen)),
            Type::Fallible(a, b) => {
                self.carries_in(&a, want_fn, actors_ok, seen)
                    || self.carries_in(&b, want_fn, actors_ok, seen)
            }
            Type::Named { id, args } => {
                if actors_ok && self.actor_types.contains(&self.defs.get(id).name) {
                    return false;
                }
                if args
                    .iter()
                    .any(|a| self.carries_in(a, want_fn, actors_ok, seen))
                {
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
                let found = inner
                    .iter()
                    .any(|t| self.carries_in(t, want_fn, actors_ok, seen));
                seen.pop();
                found
            }
            _ => false,
        }
    }

    /// Whether a task handing on or reaching a `t` would share a `ref` cell
    /// with another task. An actor's handle does not count ([E-ACTOR]).
    fn task_shares_ref(&self, t: &Type) -> bool {
        self.carries_in(t, false, true, &mut Vec::new())
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
    /// The row of a function named through its module.
    fn row_of_qualified(
        &self,
        module: &str,
        name: &str,
    ) -> Option<std::collections::BTreeSet<String>> {
        let key = format!("{module}::{name}");
        if self.extern_names.contains(&key) {
            return Some(["io", "unsafe"].iter().map(|s| s.to_string()).collect());
        }
        self.fn_rows.get(&key).cloned()
    }

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
            return Some(["io", "unsafe"].iter().map(|s| s.to_string()).collect());
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
            (
                Type::Fn {
                    ret: ar,
                    params: ap,
                    ..
                },
                Type::Fn {
                    ret: br,
                    params: bp,
                    ..
                },
            ) => {
                // A function's result may itself be a function.
                self.check_row_fits(ar, br, span);
                // And so may its *parameters* — the other way round. What is
                // handed over must accept everything the expected type
                // promises it will be given, so the roles flip. Checked
                // covariantly, `fs.par_map(call)` where `call` takes a
                // `fn(i64) -> i64` and `fs` holds an `/ {io}` one passed: the
                // element's row went into a parameter declaring none, and
                // [T-PAR-MAP-PURE]'s "compile error, not a race" ran the
                // effect on several cores instead.
                for (a, b) in ap.iter().zip(bp.iter()) {
                    self.check_row_fits(b, a, span);
                }
            }
            _ => {}
        }
        let (
            Type::Fn {
                row: Some(have), ..
            },
            Type::Fn {
                row: Some(allowed), ..
            },
        ) = (&g, &w)
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

    /// The cell `mem_tls`'s argument names ([T-TLS]): written as a name, and
    /// one this module declares.
    fn tls_cell(&mut self, arg: &Expr) -> Option<(String, crate::mem::MemTy)> {
        let found = match arg {
            Expr::Ident(id) if self.lookup(&id.name).is_none() => {
                let key = format!("{}::{}", self.current_module, id.name);
                self.c_facts.cells.get(&key).map(|t| (key, *t))
            }
            _ => None,
        };
        if found.is_none() {
            self.error_help(
                arg.span(),
                "`mem_tls` takes the name of a `thread_local` cell this module declares"
                    .to_string(),
                "declare one with `thread_local NAME: T`; a cell is private to its module, so \
another module is handed its address by a function ([T-TLS])",
            );
        }
        found
    }

    /// `thread_local NAME: T` ([T-TLS]): a layout type, private to the module.
    fn collect_thread_locals(&mut self, module: &Module) {
        for decl in &module.decls {
            let Decl::ThreadLocal(t) = decl else { continue };
            let key = format!("{}::{}", self.current_module, t.name.name);
            if self.fns.contains_key(&key)
                || self.consts.contains_key(&key)
                || self.c_facts.cells.contains_key(&key)
            {
                self.error(t.name.span, format!("duplicate name `{}`", t.name.name));
                continue;
            }
            if t.is_pub {
                self.error_help(
                    t.name.span,
                    format!("thread-local cell `{}` cannot be `pub`", t.name.name),
                    "a cell is private to its module; hand out its address from a `pub fn` \
whose body is `unsafe`",
                );
            }
            let ty = self.resolve_type(&t.ty, &[]);
            if matches!(ty, Type::Error) {
                continue;
            }
            match crate::mem::MemTy::of(&ty) {
                Some(m) => {
                    self.c_facts.cells.insert(key, m);
                }
                None => {
                    let shown = self.show(&ty);
                    self.error_help(
                        t.ty.span(),
                        format!("a thread-local cell cannot hold `{shown}`"),
                        "a cell holds a layout type -- i8, i16, i32, i64, u8, u16, u32, u64, f64 \
or addr -- and never a value the runtime counts ([T-TLS])",
                    );
                }
            }
        }
    }

    /// The function `mem_fn_addr`'s argument names ([T-FN-ADDR]): written as
    /// a name, `f` or `module.f`, and never evaluated. A Grasm function must
    /// have a signature C can call; a C function already has one.
    fn fn_addr_target(&mut self, arg: &Expr) -> Option<crate::mem::COp> {
        let (key, at) = match arg {
            Expr::Ident(id) if self.lookup(&id.name).is_none() => {
                (format!("{}::{}", self.current_module, id.name), id.span)
            }
            Expr::Field { recv, name, .. } => {
                let target = match &**recv {
                    Expr::Ident(m) if self.lookup(&m.name).is_none() => self
                        .module_imports
                        .get(&self.current_module)
                        .and_then(|i| i.get(&m.name))
                        .cloned(),
                    _ => None,
                };
                match target {
                    Some(t) => (format!("{t}::{}", name.name), name.span),
                    None => (String::new(), arg.span()),
                }
            }
            _ => (String::new(), arg.span()),
        };
        let Some(info) = self.fns.get(&key).cloned() else {
            self.error_help(
                at,
                "`mem_fn_addr` takes the name of a declared function".to_string(),
                "write the function's name, `f` or `module.f`; a closure or a value held in a \
variable has no address C can call ([T-FN-ADDR])",
            );
            return None;
        };
        if self.extern_names.contains(&key) {
            return Some(crate::mem::COp::FnAddr {
                target: key,
                is_extern: true,
            });
        }
        if !key.starts_with(&format!("{}::", self.current_module)) && !info.is_pub {
            self.error(at, format!("function `{key}` is private to its module"));
            return None;
        }
        let sig = self.c_entry_sig(&key, &info, at, "have its address taken")?;
        self.c_facts.sigs.insert(key.clone(), sig);
        self.c_facts.addressed.insert(key.clone());
        Some(crate::mem::COp::FnAddr {
            target: key,
            is_extern: false,
        })
    }

    /// The C signature of a Grasm function C will reach -- exported, or
    /// through its address -- or `None`, having said why it has none
    /// ([T-FN-ADDR]).
    fn c_entry_sig(
        &mut self,
        key: &str,
        info: &FnInfo,
        at: Span,
        what: &str,
    ) -> Option<crate::mem::CSig> {
        let name = key.rsplit("::").next().unwrap_or(key).to_string();
        let mut ok = true;
        if info.generics > 0 {
            self.error_help(
                at,
                format!("generic function `{name}` cannot {what}"),
                "C calls one machine function, and a generic has one per instantiation",
            );
            return None;
        }
        if info.err.is_some() {
            self.error_help(
                at,
                format!("fallible function `{name}` cannot {what}"),
                "C has no way to receive a Grasm error; return a status code instead",
            );
            ok = false;
        }
        if info.modes.iter().any(|m| *m != ParamMode::Let) {
            self.error_help(
                at,
                format!("`{name}` cannot {what}: it has an `inout` or `own` parameter"),
                "C passes values, not Grasm places",
            );
            ok = false;
        }
        let ret = Some(&info.ret).filter(|r| !matches!(r, Type::Unit));
        for t in info.params.iter().chain(ret) {
            if crate::mem::MemTy::of(t).is_none() {
                let shown = self.show(t);
                self.error_help(
                    at,
                    format!("`{name}` cannot {what}: `{shown}` does not cross to C"),
                    "its parameters and result must be layout types: the integer types, \
`f64` and `addr` ([T-FN-ADDR])",
                );
                ok = false;
            }
        }
        let row = self.fn_rows.get(key).cloned().unwrap_or_default();
        let held: Vec<String> = row
            .into_iter()
            .filter(|e| !crate::mem::c_reachable_effect(e))
            .collect();
        if !held.is_empty() {
            self.error_help(
                at,
                format!(
                    "`{name}` cannot {what}: it performs `{}`",
                    held.join("`, `")
                ),
                "C calls it outside every `with` and outside the task scheduler, so it may \
perform only `io`, `net`, `proc`, `unsafe` and `mem` ([T-FN-ADDR])",
            );
            ok = false;
        }
        if !ok {
            return None;
        }
        self.c_sig_of(&info.params, &info.ret)
    }

    /// `a.call_c(Sig, ..)` ([L-LIB-CALL-C]): `Sig` names a function type,
    /// read where a value would go and never evaluated.
    fn check_call_c(&mut self, name: &Ident, args: &[Expr], span: Span) -> Type {
        let Some(Expr::Ident(t)) = args.first() else {
            return self.error_help(
                span,
                "`call_c` takes a signature first, written as the name of a type alias".to_string(),
                "for example `type Cmp = fn(addr, addr) -> i32` and then `a.call_c(Cmp, x, y)`",
            );
        };
        let te = TypeExpr::Name {
            path: grasm_syntax::ast::Path {
                segments: vec![t.clone()],
                span: t.span,
            },
            args: vec![],
            span: t.span,
        };
        let generics = self.current_generics.clone();
        let ty = self.resolve_type(&te, &generics);
        if matches!(ty, Type::Error) {
            return Type::Error;
        }
        let Type::Fn { params, ret, .. } = ty else {
            let shown = self.show(&ty);
            return self.error_help(
                t.span,
                format!("`call_c` takes a function type, and `{shown}` is not one"),
                "name a type alias such as `type Cmp = fn(addr, addr) -> i32`",
            );
        };
        let mut ok = true;
        for p in &params {
            if !Self::is_c_param_type(p) {
                let shown = self.show(p);
                self.error_help(
                    t.span,
                    format!("`{shown}` has no C representation"),
                    "a `call_c` parameter is what an `extern fn` may take: an integer \
type, `f64`, `addr`, `str` or `bytes`",
                );
                ok = false;
            }
        }
        if !matches!(*ret, Type::Unit) && !Self::is_c_abi_type(&ret) {
            let shown = self.show(&ret);
            self.error_help(
                t.span,
                format!("`{shown}` has no C representation"),
                "a `call_c` result is an integer type, `f64`, `addr` or nothing",
            );
            ok = false;
        }
        let values = &args[1..];
        if values.len() != params.len() {
            return self.error(
                span,
                format!(
                    "`{}` through `{}` takes {} argument(s) after the signature, got {}",
                    name.name,
                    t.name,
                    params.len(),
                    values.len()
                ),
            );
        }
        for (a, p) in values.iter().zip(&params) {
            self.check_expr(a, p);
        }
        if !ok {
            return Type::Error;
        }
        let Some(sig) = self.c_sig_of(&params, &ret) else {
            return Type::Error;
        };
        self.mem_ops
            .insert((span.start, span.end), crate::mem::MemOp::CallC);
        self.c_facts
            .ops
            .insert((span.start, span.end), crate::mem::COp::CallC(sig));
        *ret
    }

    /// Tier 3 for raw memory ([T-MEM-UNSAFE]): every operation on an address
    /// sits inside `unsafe`, as a C function does, and performs `mem`.
    fn require_unsafe_mem(&mut self, name: &str, span: Span) {
        if self.unsafe_depth > 0 {
            return;
        }
        self.error_help(
            span,
            format!("`{name}` reaches raw memory, so it can only be used inside `unsafe`"),
            "wrap the use in `unsafe { ... }`; the package must then declare the \
`unsafe` and `mem` capabilities, which is what makes it visible to everyone who depends on you",
        );
    }

    /// `mem_alloc`, `mem_free`, `mem_addr`, `mem_size_of` and `mem_align_of`
    /// ([L-LIB-MEM]). Their row, `mem`, comes from `crate::builtins`.
    fn check_mem_fn(&mut self, id: &Ident, args: &[Expr], span: Span) -> Type {
        use crate::mem::MemOp;
        self.require_unsafe_mem(&id.name, id.span);
        let want = if id.name == "mem_alloc" { 2 } else { 1 };
        if args.len() != want {
            return self.error(
                span,
                format!("`{}` takes {want} argument(s), got {}", id.name, args.len()),
            );
        }
        let (op, ret) = match id.name.as_str() {
            "mem_alloc" => {
                self.check_expr(&args[0], &Type::I64);
                self.check_expr(&args[1], &Type::I64);
                (MemOp::Alloc, Type::Addr)
            }
            "mem_free" => {
                self.check_expr(&args[0], &Type::Addr);
                (MemOp::Free, Type::Unit)
            }
            "mem_addr" => {
                self.check_expr(&args[0], &Type::I64);
                (MemOp::FromInt, Type::Addr)
            }
            "mem_fn_addr" => {
                let Some(op) = self.fn_addr_target(&args[0]) else {
                    return Type::Error;
                };
                self.c_facts.ops.insert((span.start, span.end), op);
                (MemOp::FnAddr, Type::Addr)
            }
            "mem_tls" => {
                let Some((cell, t)) = self.tls_cell(&args[0]) else {
                    return Type::Error;
                };
                self.c_facts
                    .ops
                    .insert((span.start, span.end), crate::mem::COp::Tls { cell });
                (MemOp::Tls(t), Type::Addr)
            }
            _ => {
                let Some(t) = self.mem_layout_arg(&args[0], &id.name) else {
                    return Type::Error;
                };
                let op = if id.name == "mem_size_of" {
                    MemOp::Size(t)
                } else {
                    MemOp::Align(t)
                };
                (op, Type::I64)
            }
        };
        self.mem_ops.insert((span.start, span.end), op);
        ret
    }

    /// The `T` of `mem_size_of(T)`: written where a value would go, read as a
    /// type and never evaluated, and one with a layout. A `str` or a struct
    /// has none a program may rely on, and a generic parameter has no single
    /// one.
    fn mem_layout_arg(&mut self, arg: &Expr, fname: &str) -> Option<crate::mem::MemTy> {
        let Expr::Ident(t) = arg else {
            self.error_help(
                arg.span(),
                format!("`{fname}` takes a type, written as its name"),
                "for example `mem_size_of(u32)`",
            );
            return None;
        };
        let te = TypeExpr::Name {
            path: grasm_syntax::ast::Path {
                segments: vec![t.clone()],
                span: t.span,
            },
            args: vec![],
            span: t.span,
        };
        let generics = self.current_generics.clone();
        let ty = self.resolve_type(&te, &generics);
        if matches!(ty, Type::Error) {
            return None;
        }
        let found = crate::mem::MemTy::of(&ty);
        if found.is_none() {
            let shown = self.show(&ty);
            self.error_help(
                t.span,
                format!("`{shown}` has no layout in raw memory"),
                "a layout type is one of i8, i16, i32, i64, u8, u16, u32, u64, f64 and addr: \
the only types a load or a store moves ([L-LIB-MEM])",
            );
        }
        found
    }

    /// A method on an `addr` ([L-LIB-MEM], [L-LIB-ATOMIC]).
    fn check_mem_method(
        &mut self,
        kind: crate::mem::MethodKind,
        name: &Ident,
        args: &[Expr],
        span: Span,
    ) -> Type {
        use crate::mem::{MemOp, MethodKind, Order};
        self.require_unsafe_mem(&name.name, name.span);
        // The checker knows the receiver is an address and the effect pass
        // does not, so the call site says what it performs, as a call
        // through a field does.
        let mut row: std::collections::BTreeSet<String> = [crate::effects::MEM.to_string()].into();
        if kind == MethodKind::CallC {
            // What a call through an address reaches cannot be known, so it
            // performs what an `extern` call does ([L-LIB-CALL-C]).
            row.insert(crate::effects::IO.to_string());
            row.insert(crate::effects::UNSAFE.to_string());
        }
        self.callee_rows.insert((span.start, span.end), row);
        if kind == MethodKind::CallC {
            return self.check_call_c(name, args, span);
        }
        let arity = |c: &mut Self, want: usize| -> bool {
            if args.len() == want {
                return true;
            }
            c.error(
                span,
                format!(
                    "`{}` takes {want} argument(s), got {}",
                    name.name,
                    args.len()
                ),
            );
            false
        };
        let (op, ret) =
            match kind {
                MethodKind::CallC => unreachable!("handled above"),
                MethodKind::Plain(op) => {
                    let (params, ret) = match op {
                        MemOp::Offset => (vec![Type::I64], Type::Addr),
                        MemOp::Diff => (vec![Type::Addr], Type::I64),
                        MemOp::ToInt => (vec![], Type::I64),
                        MemOp::Load(t) => (vec![], t.ty()),
                        MemOp::Store(t) => (vec![t.ty()], Type::Unit),
                        _ => unreachable!("only methods come from `mem::method`"),
                    };
                    if !arity(self, params.len()) {
                        return Type::Error;
                    }
                    for (a, p) in args.iter().zip(&params) {
                        self.check_expr(a, p);
                    }
                    (op, ret)
                }
                MethodKind::Atomic(at, t) => {
                    let values = at.values();
                    if !arity(self, values + 1) {
                        return Type::Error;
                    }
                    for a in &args[..values] {
                        self.check_expr(a, &t.ty());
                    }
                    let last = &args[values];
                    let order = match last {
                        Expr::Ident(o) => Order::from_name(&o.name),
                        _ => None,
                    };
                    let Some(order) = order else {
                        return self.error_help(
                        last.span(),
                        format!("`{}` takes a memory ordering, written as its name", name.name),
                        "one of `Relaxed`, `Acquire`, `Release`, `AcqRel` or `SeqCst`; it must \
be known when the program is compiled, so it cannot be a value ([L-LIB-ATOMIC])",
                    );
                    };
                    if !at.allows(order) {
                        let ok: Vec<&str> = Order::ALL
                            .into_iter()
                            .filter(|o| at.allows(*o))
                            .map(|o| o.name())
                            .collect();
                        return self.error_help(
                            last.span(),
                            format!("an atomic {} cannot be `{}`", at.name(), order.name()),
                            format!("a {} may be {} ([L-LIB-ATOMIC])", at.name(), ok.join(", ")),
                        );
                    }
                    let ret = if at == crate::mem::Atomic::Store {
                        Type::Unit
                    } else {
                        t.ty()
                    };
                    (MemOp::Atomic(at, t, order), ret)
                }
            };
        self.mem_ops.insert((span.start, span.end), op);
        ret
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
    /// A task of a `concurrent` block writing a binding from outside itself.
    /// Tasks share nothing mutable -- that is what makes them free of races
    /// by construction -- so a value goes from one to another over a channel.
    fn reject_task_write(&mut self, id: &Ident) {
        let Some(floor) = self.go_floor else { return };
        let at = self.scopes.iter().rposition(|sc| sc.contains_key(&id.name));
        if at.is_some_and(|i| i < floor) {
            self.error_help(
                id.span,
                format!("a task cannot write `{}`, which belongs outside it", id.name),
                "tasks share nothing mutable -- send the value on a channel and let the owner write it",
            );
        }
    }

    fn reject_region_escape(&mut self, id: &Ident, verb: &str) {
        let Some(&floor) = self.region_floors.last() else {
            return;
        };
        let declared_at = self.scopes.iter().rposition(|sc| sc.contains_key(&id.name));
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
        while let Expr::Field { recv, .. } | Expr::Index { recv, .. } = cur {
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
        let Some(declared_at) = self.scopes.iter().rposition(|sc| sc.contains_key(&id.name)) else {
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

    /// Refuses carrying a `with`-discharged row into a call that writes to a
    /// place declared outside the block.
    ///
    /// What escapes is the *store*, not the call: passing a closure to
    /// `xs.map(f)` is fine, because nothing outliving the block keeps it.
    /// An `inout` argument rooted before the `with` is exactly a place that
    /// does.
    fn reject_handler_escape_through_call(&mut self, args: &[Expr], info: &FnInfo, span: Span) {
        // Which effects this call could carry out, and where they came from.
        let mut carried: std::collections::BTreeSet<String> = std::collections::BTreeSet::new();
        for (a, p) in args.iter().zip(&info.params) {
            carried.extend(self.row_carried_by(p));
            // A closure literal's row is known from the effect pass, and a
            // type parameter erases it: `T` says nothing about what the
            // value it stands for performs.
            if let Some(row) = self.closure_rows.get(&a.span().start) {
                carried.extend(row.iter().cloned());
            }
        }
        if carried.is_empty() {
            return;
        }
        for ((a, _), mode) in args.iter().zip(&info.params).zip(&info.modes) {
            if *mode != ParamMode::Inout {
                continue;
            }
            let Some(id) = place_root(a) else { continue };
            let Some(declared_at) = self.scopes.iter().rposition(|sc| sc.contains_key(&id.name))
            else {
                continue;
            };
            let floors = self.with_floors.clone();
            for (effect, floor) in floors {
                if declared_at < floor && carried.contains(&effect) {
                    self.error_help(
                        span,
                        format!(
                            "cannot pass a function that performs `{effect}` to a call that writes to `{}` — it was declared outside the `with` block",
                            id.name
                        ),
                        "the handler is installed only for this block, so the value would be called afterwards with nowhere to send the effect — call it here, or store what it computed",
                    );
                    return;
                }
            }
        }
    }

    /// `T ! E` merged with a bare `T`, when that is what the two branches
    /// are. `None` when they disagree about something else.
    fn merge_fallible_branches(&mut self, a: &Type, b: &Type) -> Option<Type> {
        self.merge_fallible_branches_at(a, b, None, None)
    }

    /// The same, given the expressions the two types came from.
    ///
    /// The spans are what lets an integer literal beside a whole `T ! E`
    /// promote: `fn f(n) -> f64 ! E { if c { 3 } else { Ok(4.5) } }` was
    /// refused as "branches of different types", while the same `if` with
    /// *both* halves bare — `{ 4.5 } else { 6 }` — compiled, and
    /// [T-INT-TO-FLOAT-CARRIED] names an `if`'s branches as exactly the place
    /// this promotion applies. Which literals are `f64` is a fact only this
    /// pass has, so it has to be recorded here.
    fn merge_fallible_branches_at(
        &mut self,
        a: &Type,
        b: &Type,
        a_at: Option<&Expr>,
        b_at: Option<&Expr>,
    ) -> Option<Type> {
        for (whole, half, whole_at, half_at) in [(a, b, a_at, b_at), (b, a, b_at, a_at)] {
            let Type::Fallible(ok, err) = self.uni.shallow_resolve(whole) else {
                continue;
            };
            if matches!(self.uni.shallow_resolve(half), Type::Fallible(..)) {
                continue;
            }
            let ok = self.uni.resolve(&ok);
            // An `Err(e)` branch has no ok half of its own, so an unresolved
            // one adopts the other branch's type.
            if matches!(ok, Type::Var(_)) || self.uni.unify(&ok, half) {
                let _ = self.uni.unify(&ok, half);
                return Some(self.uni.resolve(whole));
            }
            // An integer literal adopts the ok half's float.
            if ok == Type::F64 && matches!(self.uni.shallow_resolve(half), Type::I64) {
                if let Some(e) = half_at.and_then(Self::int_literal_tail) {
                    self.float_literals.insert(e.span().start);
                    return Some(self.uni.resolve(whole));
                }
            }
            // And the other way: the literal is the one *inside* the whole
            // half, as in `if c { Ok(3) } else { 4.5 }`. The same two branches
            // with the halves swapped were accepted, so the rule held or not
            // depending on which side the constructor was written on.
            if ok == Type::I64 && matches!(self.uni.shallow_resolve(half), Type::F64) {
                if let Some(e) = whole_at.and_then(Self::int_literal_tail) {
                    self.float_literals.insert(e.span().start);
                    return Some(Type::Fallible(
                        Box::new(Type::F64),
                        Box::new(self.uni.resolve(&err)),
                    ));
                }
            }
        }
        None
    }

    /// The integer literal a branch is worth, if that is all it is.
    fn int_literal_tail(e: &Expr) -> Option<&Expr> {
        match e {
            Expr::Int(..) => Some(e),
            Expr::Block(b) => match b.stmts.last() {
                Some(Stmt::Expr(inner)) => Self::int_literal_tail(inner),
                _ => None,
            },
            // Through the constructor that carries it: `Ok(3)` is a literal
            // one shape out, and [T-INT-TO-FLOAT-CARRIED] names that shape.
            Expr::Call { callee, args, .. }
                if args.len() == 1
                    && matches!(&**callee, Expr::Ident(id)
                        if matches!(id.name.as_str(), "Some" | "Ok" | "Err")) =>
            {
                Self::int_literal_tail(&args[0])
            }
            _ => None,
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
                Type::Tuple(items) => stack.extend(items.iter().map(|i| self.uni.resolve(i))),
                Type::Fallible(a, b) => {
                    stack.push(self.uni.resolve(&a));
                    stack.push(self.uni.resolve(&b));
                }
                // A named generic — `Option[T]`, and any sum or struct with
                // type arguments. Without this `Some(|| Cfg.value())` carried
                // a handler's discharge out of its `with` block while every
                // other carrier was refused, and the closure answered to a
                // different handler afterwards ([E-HANDLER-MISSING]).
                Type::Named { id, args } => {
                    stack.extend(args.iter().map(|a| self.uni.resolve(a)));
                    // And what the type itself is made of: a struct or a
                    // variant can hold a function value in a field.
                    match &self.defs.get(id).kind {
                        TypeDefKind::Struct { fields } => {
                            stack.extend(fields.iter().map(|(_, t)| self.uni.resolve(t)));
                        }
                        TypeDefKind::Sum { variants } => {
                            for v in variants {
                                stack.extend(v.fields.iter().map(|(_, t)| self.uni.resolve(t)));
                            }
                        }
                    }
                }
                _ => {}
            }
        }
        out
    }

    /// What to demand of one branch of an `if` whose whole value is `want`.
    ///
    /// When `want` is a `T ! E`, a branch may give the whole thing or just the
    /// ok half ([T-OK-ERR-RETURN]) — the rule a fallible function's body
    /// already follows. Demanding the whole type of every branch left
    /// `let v: f64 ! E = if c { 3.0 } else { Ok(4.5) }` refused as a type
    /// mismatch, while the same expression *without* the annotation was
    /// refused as an unhandled failure whose help says to annotate it: two
    /// diagnostics pointing at each other and no way to write the program.
    fn branch_want(&mut self, tail: Option<&Stmt>, want: &Type) -> Type {
        let Type::Fallible(ok, _) = want else {
            return want.clone();
        };
        match tail {
            Some(Stmt::Expr(e))
                if yields_whole_expr(e)
                    || yields_some_whole(e)
                    || self.branch_calls_fallible(e) =>
            {
                want.clone()
            }
            // This branch gives the bare half, so its value has to be wrapped
            // where it lands. Only the checker knows which branches those are.
            Some(Stmt::Expr(e)) => {
                self.ok_wrapped.insert((e.span().start, e.span().end));
                if self.record_tir {
                    self.tir
                        .wraps
                        .insert((e.span().start, e.span().end), want.clone());
                }
                (**ok).clone()
            }
            _ => (**ok).clone(),
        }
    }

    /// A branch that must deliver a value has to end in an expression.
    ///
    /// `check_block_with` pushes the expected type into a tail *expression*
    /// and has nothing to push it into otherwise — so a branch ending in a
    /// `with`, `for` or `while` statement sailed past the demand entirely:
    /// `fn go(n) -> i64 { if c { 7 } else { with D handled_by h { n } } }`
    /// checked clean, the interpreter printed `()` where an integer belonged,
    /// and a native build printed `0` — while the same `with` as the whole
    /// body was refused. A branch that *leaves* (`return`, `break`,
    /// `continue`, a diverging call) delivers its value by not coming back,
    /// and is not this case.
    fn require_branch_value(&mut self, b: &Block, got: &Type, want: &Type) {
        if *want == Type::Unit {
            return;
        }
        match b.stmts.last() {
            None => {}
            Some(Stmt::Expr(_))
            | Some(Stmt::Return(..))
            | Some(Stmt::Break(_))
            | Some(Stmt::Continue(_)) => return,
            Some(_) => {}
        }
        if self.uni.shallow_resolve(got) == Type::Unit {
            let w = self.show(want);
            let span = b.stmts.last().map(stmt_span).unwrap_or(b.span);
            self.error_help(
                span,
                format!("this branch must produce `{w}` but ends in a statement"),
                "the last expression of a branch is its value — a `with`, `for` or `while` \
produces none; end the branch with the value it computes",
            );
        }
    }

    fn check_assign_target(&mut self, target: &Expr) {
        let mut root = target;
        while let Expr::Field { recv, .. } | Expr::Index { recv, .. } = root {
            // A numeric field name is a tuple component — no identifier can
            // start with a digit — and a tuple is a fixed group of values
            // with no place to write into. `t.0 = 5` typechecked and then
            // panicked with `cannot access field \`0\``, a message about
            // reading for a program that wrote; the grammar's `place` never
            // admitted it.
            if let Expr::Field { name, span, .. } = root {
                if name.name.chars().all(|c| c.is_ascii_digit()) {
                    self.error_help(
                        *span,
                        "cannot assign to a tuple component".to_string(),
                        "a tuple is a fixed group of values — rebuild it, as in \
`t = (5, t.1)`, or use a struct when the parts have names worth writing",
                    );
                    return;
                }
            }
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
                    let kind = b.kind;
                    self.error_help(
                        id.span,
                        format!("cannot assign to `{}` — it is immutable", id.name),
                        match kind {
                            // `fn f(var n: i64)` does not parse, so the
                            // generic help sent people to a syntax error and
                            // left the two things that do work unsaid.
                            BindKind::Param => format!(
                                "a parameter cannot be `var` — take it as `inout {}` to write \
through to the caller's value, or copy it into a local first, as in `var {} = {}`",
                                id.name, id.name, id.name
                            ),
                            BindKind::Generated => format!(
                                "the test runner supplies this value, so there is nothing to \
write back to — copy it into a local first, as in `var {} = {}`",
                                id.name, id.name
                            ),
                            BindKind::Local => {
                                format!("declare it with `var {}` instead of `let`", id.name)
                            }
                        },
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
        self.check_expr_inner(e, expected);
        // TIR: an expression checked against a type has that type, and some
        // shapes (a `match`, an `if`, a literal written against `f64`) are
        // checked here without ever being inferred, so nothing else records
        // it. Inference's own answer, where there is one, is kept.
        if self.record_tir {
            let k = (e.span().start, e.span().end);
            self.tir.checked.insert(k, expected.clone());
        }
    }

    fn check_expr_inner(&mut self, e: &Expr, expected: &Type) {
        let tail_fn = self.tail_fn.take();
        let want = self.uni.shallow_resolve(expected);
        // Int literal adapts to f64. Recorded, not just allowed: the engines
        // have no types of their own and would otherwise keep the value an
        // integer for the whole run ([T-INT-TO-FLOAT]).
        if let (Expr::Int(_, sp), Type::F64) = (e, &want) {
            if self.wide_ints.contains(&sp.start) {
                self.wide_literal(*sp, "f64");
                return;
            }
            self.float_literals.insert(e.span().start);
            return;
        }
        // [T-INT-FIXED]: a literal takes the fixed-width type it is written
        // against, if it fits. It is the same number in every engine, so
        // nothing needs recording beyond the type itself.
        if let (Expr::Int(v, sp), Type::Int(k)) = (e, &want) {
            let n = self.lit_value(*k, *v, *sp, false);
            if !k.fits(n) {
                self.literal_out_of_range(*sp, n, *k);
            }
            self.resolved_types.insert((sp.start, sp.end), want.clone());
            return;
        }
        if let (
            Expr::Unary {
                op: UnaryOp::Neg,
                expr,
                span: nsp,
            },
            Type::Int(k),
        ) = (e, &want)
        {
            if let Expr::Int(v, isp) = **expr {
                let n = self.lit_value(*k, v, isp, true);
                if !k.fits(n) {
                    self.literal_out_of_range(*nsp, n, *k);
                }
                self.resolved_types
                    .insert((nsp.start, nsp.end), want.clone());
                return;
            }
        }
        if let (
            Expr::Unary {
                op: UnaryOp::Neg,
                expr,
                ..
            },
            Type::F64,
        ) = (e, &want)
        {
            if let Expr::Int(_, isp) = **expr {
                if self.wide_ints.contains(&isp.start) {
                    self.wide_literal(isp, "f64");
                    return;
                }
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
                    self.empty_lits
                        .push((span.start, span.end - span.start, elem));
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
                    self.empty_lits.push((
                        span.start,
                        span.end - span.start,
                        Type::Map(Box::new(k), Box::new(v)),
                    ));
                }
                return;
            }
            // `Some(3)` against `Option[f64]`, `Ok(3)` and `Err(3)` against
            // `f64 ! E`. A user-declared variant already carried its
            // declared field type down; the three built-in ones did not.
            (Expr::Call { callee, args, .. }, _) if args.len() == 1 => {
                if let Expr::Ident(id) = &**callee {
                    let inner = match (id.name.as_str(), &want) {
                        (
                            "Some",
                            Type::Named {
                                id: nid,
                                args: targs,
                            },
                        ) if self.defs.get(*nid).name == "Option" && targs.len() == 1 => {
                            Some(targs[0].clone())
                        }
                        ("Ok", Type::Fallible(ok, _)) => Some((**ok).clone()),
                        ("Err", Type::Fallible(_, err)) => Some((**err).clone()),
                        _ => None,
                    };
                    if let Some(inner) = inner {
                        // TIR: the same decision `infer_call` records.
                        let r = if id.name == "Some" {
                            crate::tirfacts::CallRes::Variant
                        } else {
                            crate::tirfacts::CallRes::Builtin(id.name.clone())
                        };
                        self.tir_call(e.span(), r);
                        self.check_expr(&args[0], &inner);
                        return;
                    }
                }
            }
            _ => {}
        }
        // The same, through the shapes whose value is another expression.
        //
        // Not for `()`: a block or `if` in *statement* position is checked
        // against `()` too, and pushing that inward would compare every tail
        // with unit. `let a: () = …` therefore goes the ordinary way, which
        // is what makes it an error rather than a silent success — it used to
        // accept anything at all, so `let a: () = if c { 1 } else { 2.5 }`
        // left `a` holding an `i64`.
        match e {
            _ if want == Type::Unit => {}
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
                // Only one branch runs, so each starts from the moves made
                // before the `if` -- the statement form did this and this,
                // the form in value position, did not: `if ok { commit(t) }
                // else { rollback(t) }` as a function's result was a "use of
                // moved value" for a `t` moved exactly once either way.
                let before = self.moved_snapshot();
                let want_then = self.branch_want(then_block.stmts.last(), &want);
                let t = self.check_block_with(then_block, Some(&want_then));
                self.require_branch_value(then_block, &t, &want);
                let after_then = self.moved_snapshot();
                self.restore_moved(&before);
                let else_leaves = match &**eb {
                    ElseBranch::Else(b) => {
                        let want_else = self.branch_want(b.stmts.last(), &want);
                        let t = self.check_block_with(b, Some(&want_else));
                        self.require_branch_value(b, &t, &want);
                        block_leaves(b)
                    }
                    ElseBranch::ElseIf(inner) => {
                        self.check_expr(inner, &want);
                        false
                    }
                };
                let after_else = self.moved_snapshot();
                self.merge_branches(
                    vec![
                        (after_then, block_leaves(then_block)),
                        (after_else, else_leaves),
                    ],
                    e.span(),
                );
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
                        format!(
                            "this closure performs {list}, which the expected type does not allow"
                        ),
                        "widen the type's effect row, or move the effect out of the closure",
                    );
                }
            }
            let got = self.infer_closure(e, Some(&want));
            self.unify_or_report(e.span(), &got, &want);
            // The rows *inside* the type as well as the closure's own. The
            // check above compares what this closure performs; this compares
            // what it hands back, and every other expression already got it by
            // falling through to the line below.
            //
            // A handler operation is the position where that mattered: its
            // field type carries `row: None` — unconstrained, which is what
            // [E-HANDLER-OWN-ROW] requires, since a handler's operations are
            // charged to whoever wrote the handler. That exemption is about
            // the operation, not about the value it returns, and returning
            // early took both. `MkHandler{ make: || shout }` for
            // `fn make() -> fn(i64) -> i64` handed a printing function through
            // an empty-row result: a function declaring `/ {Mk}` printed, and
            // a `capabilities = []` package reached `io`, while the same
            // signature written as an ordinary `fn` was refused.
            self.check_row_fits(&got, &want, e.span());
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
            if matches!(got_r, Type::Fallible(..))
                && !matches!(self.uni.shallow_resolve(want), Type::Fallible(..))
            {
                self.error_help(
                    span,
                    format!("this expression can fail (`{g}`) but `{w}` is required"),
                    "use `try` to propagate the error or `match` to handle it",
                );
            } else if let (Type::Fn { ret: got_ret, .. }, Type::Fn { ret: want_ret, .. }) =
                (&got_r, &self.uni.shallow_resolve(want))
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
            } else if match self.uni.shallow_resolve(want) {
                // A value that cannot fail, where one that can is wanted.
                // Half the arms of a `match` inside a fallible function are
                // usually like this, and the message named two types without
                // saying that the fix is on this side and is one word long.
                Type::Fallible(ok, _) => {
                    let ok = self.uni.resolve(&ok);
                    !matches!(got_r, Type::Fallible(..)) && self.uni.unify(&got_r, &ok)
                }
                _ => false,
            } {
                self.error_help(
                    span,
                    format!("type mismatch: expected `{w}`, found `{g}`"),
                    "a value that cannot fail is written `Ok(...)` here; in a `match`, either wrap the arms that cannot fail or add `try` to the ones that can, so every arm is the same shape",
                );
            } else if let Some(help) = int_mix_help(&got_r, &self.uni.shallow_resolve(want)) {
                self.error_help(
                    span,
                    format!("type mismatch: expected `{w}`, found `{g}`"),
                    help,
                );
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
        // Memoized by span. An expression appears once in a program and its
        // type does not depend on where it is asked from — but it *is* asked
        // more than once: a method call infers its receiver to resolve the
        // method, and then `check_fn_call` checks the same receiver against
        // the parameter it fills. That doubled the cost per link of a UFCS
        // chain, so twenty `.bump(1)`s took 0.7 seconds and forty never
        // finished. The same shape as the field chain, one path over.
        //
        // Skipping the repeat also stops the walk emitting the same diagnostic
        // twice, which it did for anything reached this way.
        // Only the shapes that cannot hold a closure are memoized. A closure
        // literal's type depends on the position it fills, and its body is
        // checked against the enclosing function's error type — so a second
        // visit is not a repeat of the first, and skipping it lost the refusal
        // of a stored closure that fails inside a function declaring no error
        // type. A projection chain holds no closure, and it is the shape that
        // was costing 2^n.
        let key = {
            let sp = e.span();
            (sp.start, sp.end)
        };
        fn simple(e: &Expr) -> bool {
            matches!(
                e,
                Expr::Ident(_) | Expr::Int(..) | Expr::Float(..) | Expr::Bool(..)
            )
        }
        let cacheable = match e {
            Expr::Ident(_) | Expr::Int(..) | Expr::Float(..) | Expr::Bool(..) => true,
            // A projection off something already known.
            Expr::Field { recv, .. } | Expr::Index { recv, .. } => self
                .expr_types
                .contains_key(&(recv.span().start, recv.span().end)),
            // A call whose arguments are plain values holds no closure, so a
            // second visit really is a repeat. This is the chain that cost
            // 2^n: `b.bump(1).bump(1)…`, where each receiver is itself a call
            // and the receiver is walked once to resolve the method and again
            // to check it against the parameter it fills.
            Expr::MethodCall { recv, args, .. } => {
                args.iter().all(simple)
                    && (simple(recv)
                        || self
                            .expr_types
                            .contains_key(&(recv.span().start, recv.span().end)))
            }
            _ => false,
        };
        if cacheable {
            if let Some(t) = self.expr_types.get(&key) {
                return t.clone();
            }
        }
        let t = self.infer_expr_inner(e);
        if cacheable {
            self.expr_types.insert(key, t.clone());
        }
        // Recorded whether or not it is cacheable, and overwriting an earlier
        // visit on purpose: a span visited twice was visited the second time
        // with more known, so the later answer is the settled one.
        self.resolved_types.insert(key, t.clone());
        if !self.linear_types.is_empty() && self.holds_linear(&t) {
            let shown = self.show(&t);
            self.error_help(
                e.span(),
                format!("`{shown}` would hold a linear value"),
                "a list, map, tuple, `ref`, `Option` or other generic cannot promise to consume what it holds exactly once -- keep the linear value in its own binding ([T-LINEAR-CONTAIN])",
            );
        }
        self.record_value_row(e, &t);
        // A `ref` from outside, mentioned inside a parallel operation's
        // function. Writing through one was already refused; *reading* is no
        // safer, because a read takes a count on what it finds and the count
        // is not atomic. Several workers reaching the same slot at once made a
        // program `check` accepted die with SIGTRAP.
        if let Expr::Ident(id) = e {
            // Inside a closure literal, reaching a `ref` from outside it.
            // Remembered rather than refused: a closure may do this perfectly
            // well ([T-REF-CAPTURE]); it is handing it to a parallel operation
            // that is the race.
            let outside = self.closure_floors.last().is_some_and(|floor| {
                self.scopes
                    .iter()
                    .rposition(|sc| sc.contains_key(&id.name))
                    .is_some_and(|at| at < *floor)
            });
            if outside && self.carries_ref(&t) {
                self.closure_touches_outside_ref = true;
            }
        }
        // A task reaching a `ref` from outside it: two tasks holding the same
        // cell is shared mutable state, which is what tasks give up in
        // exchange for being free of races.
        if let (Some(floor), Expr::Ident(id)) = (self.go_floor, e) {
            let declared_at = self.scopes.iter().rposition(|sc| sc.contains_key(&id.name));
            if declared_at.is_some_and(|at| at < floor) && self.task_shares_ref(&t) {
                self.error_help(
                    id.span,
                    format!(
                        "a task cannot use `{}` -- it reaches a `ref` from outside, which \
tasks would share",
                        id.name
                    ),
                    "tasks share nothing mutable -- send the value on a channel instead",
                );
                return Type::Error;
            }
        }
        if let (Some(floor), Expr::Ident(id)) = (self.parallel_fn_floor, e) {
            let declared_at = self.scopes.iter().rposition(|sc| sc.contains_key(&id.name));
            if declared_at.is_some_and(|at| at < floor) && self.carries_ref(&t) {
                self.error_help(
                    id.span,
                    format!(
                        "cannot use `{}` inside a parallel operation — it reaches a `ref` \
from outside, which every element would share",
                        id.name
                    ),
                    "elements are processed on several cores, so two of them reaching the same \
slot would race — use `map` or `filter` for work over shared state, or build the reference \
inside the function, one per element",
                );
                return Type::Error;
            }
        }
        // A closure written *here* can only capture what a region already
        // allows, and so can one bound to a name declared here. One that
        // arrived from *outside* carries its captures, and running it inside
        // the region is how what the region allocated gets out — whether it
        // arrives by name, in a struct field, a list, a tuple, a map or an
        // `Option` ([T-REGION-FOREIGN]).
        if self.region_depth > 0 && self.arrived_from_outside_region(e) && self.carries_fn(&t) {
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

    /// Records the effect row of any expression whose type is a function.
    ///
    /// The checker already carries a function's own row in its type
    /// ([T-FN-TYPE-ROW]) and propagates it through every carrier by ordinary
    /// unification. The effect pass was deriving the same fact a second time,
    /// syntactically — by guessing which binder a function value came out of —
    /// and that guess is where nearly every defect of the last two weeks
    /// lived: a `for` instead of a `let`, a block around the initializer, a
    /// name spelled like an imported module. One answer, computed once, in
    /// the pass that has the types.
    ///
    /// Keyed by the whole span, for the reason [T-OK-ERR-RETURN]'s table is:
    /// a binary expression and its leftmost leaf start at the same offset.
    fn record_value_row(&mut self, e: &Expr, t: &Type) {
        // Shallow first. `resolve` copies the whole type, and this runs on
        // every expression — so for a value whose type is deep, such as a
        // nested tuple literal, the copy at each of n levels cost O(n) and the
        // walk cost O(n^2) on top of an already superlinear check. Only a
        // function type is wanted here, and its head is visible without
        // copying anything below it.
        if !matches!(self.uni.shallow_resolve(t), Type::Fn { .. }) {
            return;
        }
        if let Type::Fn { row: Some(r), .. } = self.uni.resolve(t) {
            let span = e.span();
            self.value_rows.insert((span.start, span.end), r);
        }
    }

    fn infer_expr_inner(&mut self, e: &Expr) -> Type {
        // `discarding` is about *this statement's own* value, and only the
        // shapes whose value is the statement's value carry it inward. It
        // used to stay set for everything under the statement, so the first
        // `if` reached anywhere below — including one inside a closure body,
        // whose value is the closure's result and is discarded by nobody —
        // read the flag as its own. `take(xs.map(|x| if c { Err(E) } else {
        // Ok(x) }).len())` was then refused for discarding a failure, and
        // only when it was not the last statement in its block, so the same
        // line compiled or not depending on what followed it
        // ([T-FALLIBLE-UNHANDLED]).
        if !matches!(e, Expr::If { .. } | Expr::Block(..) | Expr::Region { .. }) {
            self.discarding = false;
        }
        match e {
            Expr::Int(_, sp) if self.wide_ints.contains(&sp.start) => {
                self.wide_literal(*sp, "i64");
                Type::Error
            }
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
                for item in items.iter().filter(|i| !Self::is_int_literal(i)) {
                    self.check_expr(item, &elem);
                }
                for item in items.iter().filter(|i| Self::is_int_literal(i)) {
                    self.check_expr(item, &elem);
                }
                if items.is_empty() {
                    self.empty_lits
                        .push((span.start, span.end - span.start, elem.clone()));
                }
                Type::List(Box::new(elem))
            }
            Expr::Map(entries, span) => {
                let k = self.uni.fresh();
                let v = self.uni.fresh();
                for (ke, _) in entries {
                    self.check_expr(ke, &k);
                }
                for (_, ve) in entries.iter().filter(|(_, v)| !Self::is_int_literal(v)) {
                    self.check_expr(ve, &v);
                }
                for (_, ve) in entries.iter().filter(|(_, v)| Self::is_int_literal(v)) {
                    self.check_expr(ve, &v);
                }
                if entries.is_empty() {
                    self.empty_lits.push((
                        span.start,
                        span.end - span.start,
                        Type::Map(Box::new(k.clone()), Box::new(v.clone())),
                    ));
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
            } => {
                let t = self.infer_struct_lit(name, base.as_deref(), fields, *span);
                for (_, v) in fields {
                    self.consume(v);
                }
                t
            }
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
                // `-9223372036854775808` is `i64`'s least value, written as
                // it is printed: the literal is 2^63, past `i64`, and its
                // negation is not ([T-U64]).
                UnaryOp::Neg
                    if matches!(**expr, Expr::Int(v, sp)
                        if v == i64::MIN && self.wide_ints.contains(&sp.start)) =>
                {
                    Type::I64
                }
                UnaryOp::Neg => {
                    let t = self.infer_expr(expr);
                    let r = self.uni.shallow_resolve(&t);
                    if matches!(
                        r,
                        Type::I64 | Type::Int(_) | Type::F64 | Type::Var(_) | Type::Error
                    ) {
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
                // If this `if` is a statement, its branches' values go
                // nowhere too, so `if c { write_file(p, s) }` is as unhandled
                // as `write_file(p, s)` on its own line. Read before the
                // condition, which is an expression like any other and turns
                // the flag off.
                let disc = std::mem::take(&mut self.discarding);
                self.check_expr(cond, &Type::Bool);
                let e_span = e.span();
                let before = self.moved_snapshot();
                let then_ty = if disc {
                    self.check_block_discarded(then_block)
                } else {
                    self.check_block(then_block)
                };
                let after_then = self.moved_snapshot();
                match else_branch {
                    Some(eb) => {
                        self.restore_moved(&before);
                        let else_ty = match &**eb {
                            ElseBranch::Else(b) if disc => self.check_block_discarded(b),
                            ElseBranch::Else(b) => self.check_block(b),
                            ElseBranch::ElseIf(e) => {
                                self.discarding = disc;
                                let t = self.infer_expr(e);
                                self.discarding = false;
                                t
                            }
                        };
                        let after_else = self.moved_snapshot();
                        let else_leaves = matches!(&**eb, ElseBranch::Else(b) if block_leaves(b));
                        self.merge_branches(
                            vec![
                                (after_then, block_leaves(then_block)),
                                (after_else, else_leaves),
                            ],
                            e_span,
                        );
                        if self.uni.unify(&then_ty, &else_ty) {
                            then_ty
                        } else if disc {
                            // Branches disagree, and nothing wanted the value:
                            // a statement-style `if` ([E-IF-VALUE]).
                            Type::Unit
                        } else {
                            // Something *did* want the value. Calling it `()`
                            // made `let a: () = if c { 1 } else { 2.5 }` legal
                            // and left `a` holding an `i64` at run time —
                            // printing as `1`, unequal to `()`, and passable
                            // to a `fn f(u: ())`. `()` has one value.
                            // One branch built the whole `T ! E` and the
                            // other gave the ok half, which a fallible body
                            // may do either way ([T-OK-ERR-RETURN]). The
                            // whole is what the `if` is worth.
                            let then_at = match then_block.stmts.last() {
                                Some(Stmt::Expr(e)) => Some(e),
                                _ => None,
                            };
                            let else_at = match &**eb {
                                ElseBranch::Else(b) => match b.stmts.last() {
                                    Some(Stmt::Expr(e)) => Some(e),
                                    _ => None,
                                },
                                ElseBranch::ElseIf(inner) => Some(inner),
                            };
                            if let Some(t) = self
                                .merge_fallible_branches_at(&then_ty, &else_ty, then_at, else_at)
                            {
                                return t;
                            }
                            // An integer literal branch adopts the other's
                            // float, the way a `match`'s arms already do —
                            // and a two-arm `match` on a `bool` is the same
                            // program. [T-INT-TO-FLOAT-CARRIED] names "an
                            // `if`'s branches" among the shapes an expected
                            // type reaches through; with no expected type at
                            // all the branches have only each other, and one
                            // of them is a literal that adopts.
                            for (lit_ty, lit_at, other) in
                                [(&then_ty, then_at, &else_ty), (&else_ty, else_at, &then_ty)]
                            {
                                if matches!(self.uni.shallow_resolve(lit_ty), Type::I64)
                                    && matches!(self.uni.shallow_resolve(other), Type::F64)
                                {
                                    if let Some(lit) = lit_at.and_then(Self::int_literal_tail) {
                                        self.float_literals.insert(lit.span().start);
                                        return Type::F64;
                                    }
                                }
                            }
                            let t = self.show(&then_ty);
                            let e = self.show(&else_ty);
                            self.error_help(
                                e_span,
                                format!(
                                    "the branches of this `if` have different types (`{t}` and `{e}`)"
                                ),
                                "an `if` used as a value needs both branches to agree — give them a common type, or use it as a statement",
                            );
                            Type::Error
                        }
                    }
                    None => {
                        // Either the branch ran or it did not; the code after
                        // sees whichever of the two reaches it.
                        self.merge_branches(
                            vec![
                                (after_then.clone(), block_leaves(then_block)),
                                (before.clone(), false),
                            ],
                            e_span,
                        );
                        // A one-armed `if` throws its branch's value away —
                        // there is no other branch for it to agree with — so
                        // a failure in that tail is a discarded failure.
                        // Checked nowhere: the `if` itself is `()`, so the
                        // statement rule saw nothing to complain about, and
                        // `fn save(p, b) { if c { write_file(p, b) } }`
                        // printed the line after it, exited 0, wrote nothing
                        // to stderr and left no file — verbatim the failure
                        // [T-FALLIBLE-UNHANDLED] exists to prevent.
                        let where_ = match then_block.stmts.last() {
                            Some(Stmt::Expr(t)) => t.span(),
                            _ => e_span,
                        };
                        let resolved = self.uni.shallow_resolve(&then_ty);
                        if matches!(resolved, Type::Fallible(..)) {
                            let shown = self.show(&then_ty);
                            self.error_help(
                                where_,
                                format!(
                                    "this expression can fail (`{shown}`) and its failure is discarded"
                                ),
                                "use `try` to propagate it, `match` to handle both cases, or `??` \
with a fallback to say what should happen instead",
                            );
                        } else if self.holds_fallible(&resolved) {
                            let shown = self.show(&then_ty);
                            self.error_help(
                                where_,
                                format!(
                                    "this expression holds failures (`{shown}`) and they are discarded"
                                ),
                                "bind it and handle the failures it holds — a value nothing reads \
is a failure nothing reports",
                            );
                        }
                        Type::Unit
                    }
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
                    Type::Unit
                        | Type::I64
                        | Type::Int(_)
                        | Type::F64
                        | Type::Bool
                        | Type::Var(_)
                        | Type::Error
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
            self.tir_name(id.span, crate::tirfacts::NameRes::Local);
            self.reject_dead_capture(id);
            let captured = self.closure_floors.last().is_some_and(|floor| {
                self.scopes
                    .iter()
                    .rposition(|sc| sc.contains_key(&id.name))
                    .is_some_and(|at| at < *floor)
            });
            if captured && self.is_linear(&b.ty) {
                return self.error_help(
                    id.span,
                    format!("a closure cannot capture the linear value `{}`", id.name),
                    "a closure may run any number of times, so it cannot promise to consume it once ([T-LINEAR-CONTAIN]); pass it as an argument instead",
                );
            }
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
                self.tir_name(id.span, crate::tirfacts::NameRes::Variant);
                return self.instantiate_variant(def_id, vi, &[], id.span);
            } else if cands.len() > 1 {
                // A module's own variant wins, as it does for one with
                // arguments (F88). This path was missed by that fix, so a
                // *nullary* variant went on being ambiguous inside the very
                // module that declared it — `aes` could not name its own
                // `NotAuthentic` once `tls13` declared one too (F91).
                if let Some((def_id, vi)) = self.own_variant(&cands) {
                    self.tir_name(id.span, crate::tirfacts::NameRes::Variant);
                    return self.instantiate_variant(def_id, vi, &[], id.span);
                }
                return self.error_help(
                    id.span,
                    format!(
                        "`{}` is ambiguous — it names a variant of multiple types",
                        id.name
                    ),
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
            let q = format!("{}::{}", self.current_module, id.name);
            self.tir_name(id.span, crate::tirfacts::NameRes::Const(q));
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
                let q = format!("{}::{}", self.current_module, id.name);
                self.tir_name(id.span, crate::tirfacts::NameRes::Fn(q, fresh.clone()));
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
            let q = format!("{}::{}", self.current_module, id.name);
            self.tir_name(id.span, crate::tirfacts::NameRes::Fn(q, Vec::new()));
            return Type::Fn {
                params: info.params.clone(),
                ret: Box::new(ret_of(&info.ret, &info.err)),
                row: own_row,
            };
        }
        // Types are program-wide but functions are per-module, so the name
        // that is missing here is very often one that exists next door. The
        // old advice — spelling, or `let` — was the one fix that is never
        // right for that case.
        let qualified: Vec<String> = self
            .module_imports
            .get(&self.current_module)
            .into_iter()
            .flat_map(|m| m.iter())
            .filter(|(_, target)| self.fns.contains_key(&format!("{target}::{}", id.name)))
            .map(|(alias, _)| format!("`{alias}.{}`", id.name))
            .collect();
        if !qualified.is_empty() {
            return self.error_help(
                id.span,
                format!("unknown name `{}`", id.name),
                format!("did you mean {}?", qualified.join(" or ")),
            );
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
        let built = Type::Named {
            id: def_id,
            args: fresh,
        };
        // [T-GENERIC-TYPE]: a parameter no argument mentions has to be written
        // down. Recorded rather than judged here — a later use may still fix
        // it, exactly as it may for an empty literal — and resolved at the end
        // with the same machinery.
        if generics > 0 {
            self.generic_ctors
                .push((span.start, span.end - span.start, built.clone()));
        }
        built
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
        let TypeDefKind::Struct { fields: def_fields } = def.kind.clone() else {
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
            self.error(
                span,
                format!("missing field(s) {list} in `{tname}` literal"),
            );
        }
        Type::Named {
            id: def_id,
            args: fresh,
        }
    }

    /// The one candidate this module declares, when there is exactly one.
    ///
    /// Two sites ask the same question — a variant with arguments and one
    /// without — and they gave different answers for a while, because the fix
    /// for F88 taught only the first. One function so a third site cannot make
    /// it three answers.
    fn own_variant(&self, cands: &[(TypeDefId, usize)]) -> Option<(TypeDefId, usize)> {
        let mine: Vec<(TypeDefId, usize)> = cands
            .iter()
            .filter(|(def_id, _)| {
                let name = self.defs.get(*def_id).name.clone();
                self.type_module
                    .get(&name)
                    .is_some_and(|m| *m == self.current_module)
            })
            .copied()
            .collect();
        if mine.len() == 1 {
            Some(mine[0])
        } else {
            None
        }
    }

    fn infer_call(&mut self, callee: &Expr, args: &[Expr], span: Span) -> Type {
        if let Expr::Ident(id) = callee {
            // Local binding takes priority (closure in a variable).
            if self.lookup(&id.name).is_none() {
                // TIR: a built-in if one of the arms below takes it. Every other
                // way a call can resolve (a variant, a declared function, a
                // function value) records itself afterwards and replaces this,
                // so what survives is exactly the calls a built-in arm returned.
                self.tir_call(span, crate::tirfacts::CallRes::Builtin(id.name.clone()));
                // Builtin free functions.
                match id.name.as_str() {
                    // `eprint` is `print` to the other stream ([L-LIB-EPRINT]).
                    // Same arity, same refusal for a fallible value, same `io`
                    // — a tool that cannot separate its diagnostics from its
                    // output cannot be used in a pipeline, which is most of
                    // what a command-line tool is for.
                    "print" | "eprint" => {
                        if args.len() != 1 {
                            return self
                                .error(span, format!("{} takes exactly 1 argument", id.name));
                        }
                        let t = self.infer_expr(&args[0]);
                        if let Type::Fallible(..) = self.uni.shallow_resolve(&t) {
                            self.error_help(
                                args[0].span(),
                                format!("cannot {} a fallible value", id.name),
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
                    // The `net` capability. Handles are `i64` and payloads are
                    // `str`, so reaching the network needed no pointer type —
                    // which would have been a far larger change to the language
                    // than the network itself ([L-LIB-NET]).
                    "tcp_listen" | "tcp_accept" | "tcp_read" | "tcp_read_bytes" | "net_port"
                    | "udp_bind" | "udp_recv" => {
                        let (want, gives) = match id.name.as_str() {
                            "tcp_listen" => (Type::I64, Type::I64),
                            "udp_bind" => (Type::I64, Type::I64),
                            "udp_recv" => (Type::I64, Type::Bytes),
                            "tcp_read_bytes" => (Type::I64, Type::Bytes),
                            "tcp_accept" => (Type::I64, Type::I64),
                            "net_port" => (Type::I64, Type::I64),
                            _ => (Type::I64, Type::Str),
                        };
                        if args.len() != 1 {
                            return self
                                .error(span, format!("{} takes exactly 1 argument", id.name));
                        }
                        self.check_expr(&args[0], &want);
                        let io_err = Type::Named {
                            id: self.defs.io_error_id,
                            args: vec![],
                        };
                        return Type::Fallible(Box::new(gives), Box::new(io_err));
                    }
                    // A datagram and where it came from. The reply address is
                    // not decoration: `udp_send` names a destination on every
                    // call precisely because a datagram socket has no fixed
                    // peer, so a receiver that could not learn the sender's
                    // address could answer nobody it had not been told about
                    // in advance. Like `proc_run` this is a tuple — three
                    // values with obvious positions, and inventing a struct
                    // for them would put a type in the prelude that nothing
                    // else uses ([L-LIB-UDP]).
                    "udp_recv_from" => {
                        if args.len() != 1 {
                            return self.error(span, "udp_recv_from takes exactly 1 argument");
                        }
                        self.check_expr(&args[0], &Type::I64);
                        let io_err = Type::Named {
                            id: self.defs.io_error_id,
                            args: vec![],
                        };
                        let triple = Type::Tuple(vec![Type::Bytes, Type::Str, Type::I64]);
                        return Type::Fallible(Box::new(triple), Box::new(io_err));
                    }
                    // Running another program. The result is a tuple rather
                    // than a new struct type: three values with obvious
                    // positions did not need a name, and inventing one would
                    // have put a type in the prelude that nothing else uses
                    // ([L-LIB-PROC]).
                    "proc_run" => {
                        if args.len() != 2 {
                            return self
                                .error(span, "proc_run takes 2 arguments (program, arguments)");
                        }
                        self.check_expr(&args[0], &Type::Str);
                        self.check_expr(&args[1], &Type::List(Box::new(Type::Str)));
                        let io_err = Type::Named {
                            id: self.defs.io_error_id,
                            args: vec![],
                        };
                        let triple = Type::Tuple(vec![Type::I64, Type::Str, Type::Str]);
                        return Type::Fallible(Box::new(triple), Box::new(io_err));
                    }
                    // A pipeline cannot be built out of `proc_run`: that
                    // waits and hands back output, and a pipeline needs one
                    // child's stdout to *be* another's stdin ([L-LIB-PROC]).
                    "proc_pipeline" => {
                        if args.len() != 2 {
                            return self
                                .error(span, "proc_pipeline takes 2 arguments (stages, input)");
                        }
                        let stage = Type::Tuple(vec![Type::Str, Type::List(Box::new(Type::Str))]);
                        self.check_expr(&args[0], &Type::List(Box::new(stage)));
                        self.check_expr(&args[1], &Type::Str);
                        let io_err = Type::Named {
                            id: self.defs.io_error_id,
                            args: vec![],
                        };
                        let triple = Type::Tuple(vec![Type::I64, Type::Str, Type::Str]);
                        return Type::Fallible(Box::new(triple), Box::new(io_err));
                    }
                    // The primitive that lets one thread hold many
                    // conversations. Both arguments are ordinary values — a
                    // list of handles and a millisecond timeout — so readiness
                    // needed no new type either ([L-LIB-NET]).
                    "net_wait" => {
                        if args.len() != 2 {
                            return self
                                .error(span, "net_wait takes 2 arguments (handles, timeout_ms)");
                        }
                        let handles = Type::List(Box::new(Type::I64));
                        self.check_expr(&args[0], &handles);
                        self.check_expr(&args[1], &Type::I64);
                        let io_err = Type::Named {
                            id: self.defs.io_error_id,
                            args: vec![],
                        };
                        return Type::Fallible(Box::new(handles), Box::new(io_err));
                    }
                    // `f(0)`, …, `f(n - 1)`, each on a thread of its own
                    // ([L-LIB-NET-WORKERS]). Checked as a parallel operation's
                    // function, because it is one: every worker runs at once,
                    // so a `ref` or an actor handle from outside would be
                    // shared between them. The row is the one difference —
                    // `net` is allowed, and `effects.rs` refuses anything else.
                    "net_workers" => {
                        if args.len() != 2 {
                            return self.error(span, "net_workers takes 2 arguments (n, f)");
                        }
                        self.check_expr(&args[0], &Type::I64);
                        let out = self.uni.fresh();
                        let f = Type::Fn {
                            params: vec![Type::I64],
                            ret: Box::new(out.clone()),
                            row: None,
                        };
                        self.check_parallel_fn(&args[1], &f);
                        return Type::List(Box::new(out));
                    }
                    // A datagram is a whole message sent to an address named at
                    // send time, so unlike `tcp_write` it cannot take just a
                    // handle and a payload. The payload is `bytes` rather than
                    // `str` because the protocols that use UDP are binary
                    // ([L-LIB-UDP]).
                    "udp_send" => {
                        if args.len() != 4 {
                            return self.error(
                                span,
                                "udp_send takes 4 arguments (handle, host, port, bytes)",
                            );
                        }
                        self.check_expr(&args[0], &Type::I64);
                        self.check_expr(&args[1], &Type::Str);
                        self.check_expr(&args[2], &Type::I64);
                        self.check_expr(&args[3], &Type::Bytes);
                        let io_err = Type::Named {
                            id: self.defs.io_error_id,
                            args: vec![],
                        };
                        return Type::Fallible(Box::new(Type::I64), Box::new(io_err));
                    }
                    // Who is at the other end of a connection: the address and
                    // port as two values with obvious positions, a tuple for
                    // the same reason `udp_recv_from` returns one.
                    "tcp_peer" => {
                        if args.len() != 1 {
                            return self.error(span, "tcp_peer takes exactly 1 argument");
                        }
                        self.check_expr(&args[0], &Type::I64);
                        let io_err = Type::Named {
                            id: self.defs.io_error_id,
                            args: vec![],
                        };
                        let pair = Type::Tuple(vec![Type::Str, Type::I64]);
                        return Type::Fallible(Box::new(pair), Box::new(io_err));
                    }
                    // `tcp_listen` with the address named, for a server that
                    // other machines must reach; `tcp_listen` itself stays on
                    // the loopback address so that trying a program out does
                    // not open a port to the network.
                    "tcp_listen_on" => {
                        if args.len() != 2 {
                            return self
                                .error(span, "tcp_listen_on takes 2 arguments (host, port)");
                        }
                        self.check_expr(&args[0], &Type::Str);
                        self.check_expr(&args[1], &Type::I64);
                        let io_err = Type::Named {
                            id: self.defs.io_error_id,
                            args: vec![],
                        };
                        return Type::Fallible(Box::new(Type::I64), Box::new(io_err));
                    }
                    // The other half of [L-LIB-NET]: without it a program could
                    // be a server and could not be a client.
                    "tcp_connect" => {
                        if args.len() != 2 {
                            return self.error(span, "tcp_connect takes 2 arguments (host, port)");
                        }
                        self.check_expr(&args[0], &Type::Str);
                        self.check_expr(&args[1], &Type::I64);
                        let io_err = Type::Named {
                            id: self.defs.io_error_id,
                            args: vec![],
                        };
                        return Type::Fallible(Box::new(Type::I64), Box::new(io_err));
                    }
                    "tcp_write" | "tcp_write_bytes" => {
                        let what = if id.name == "tcp_write" {
                            "text"
                        } else {
                            "bytes"
                        };
                        if args.len() != 2 {
                            return self.error(
                                span,
                                format!("{} takes 2 arguments (handle, {what})", id.name),
                            );
                        }
                        self.check_expr(&args[0], &Type::I64);
                        let payload = if id.name == "tcp_write" {
                            Type::Str
                        } else {
                            Type::Bytes
                        };
                        self.check_expr(&args[1], &payload);
                        let io_err = Type::Named {
                            id: self.defs.io_error_id,
                            args: vec![],
                        };
                        return Type::Fallible(Box::new(Type::Unit), Box::new(io_err));
                    }
                    "net_close" => {
                        if args.len() != 1 {
                            return self.error(span, "net_close takes exactly 1 argument");
                        }
                        self.check_expr(&args[0], &Type::I64);
                        return Type::Unit;
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
                            return self
                                .error(span, "write_file takes 2 arguments (path, content)");
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
                    // A file could be written and truncated and never removed,
                    // and a directory could not be read at all ([L-LIB-FS]).
                    // A program that creates a temporary file could only empty
                    // it, so the next run saw a leftover.
                    // [L-LIB-BYTES]. `bytes` is the type `str` deliberately is
                    // not: `str` re-encodes anything above 127 as UTF-8, which
                    // is correct for text and fatal for a BMP header or a
                    // machine instruction.
                    "bytes_of" => {
                        if args.len() != 1 {
                            return self.error(
                                span,
                                "bytes_of takes exactly 1 argument (a list of 0..255)",
                            );
                        }
                        self.check_expr(&args[0], &Type::List(Box::new(Type::I64)));
                        return Type::Bytes;
                    }
                    "bytes_filled" => {
                        if args.len() != 2 {
                            return self.error(
                                span,
                                "bytes_filled takes exactly 2 arguments (a count, and a byte in 0..255)",
                            );
                        }
                        self.check_expr(&args[0], &Type::I64);
                        self.check_expr(&args[1], &Type::I64);
                        return Type::Bytes;
                    }
                    // Raw memory ([L-LIB-MEM]).
                    "mem_alloc" | "mem_free" | "mem_addr" | "mem_size_of" | "mem_align_of"
                    | "mem_fn_addr" | "mem_tls" => {
                        return self.check_mem_fn(id, args, span);
                    }
                    // [L-LIB-BUILDER]: an empty one of each.
                    "str_builder" | "byte_builder" => {
                        if !args.is_empty() {
                            return self.error(span, format!("{} takes no arguments", id.name));
                        }
                        return if id.name == "str_builder" {
                            Type::StrBuf
                        } else {
                            Type::ByteBuf
                        };
                    }
                    "random_bytes" => {
                        if args.len() != 1 {
                            return self
                                .error(span, "random_bytes takes exactly 1 argument (count)");
                        }
                        self.check_expr(&args[0], &Type::I64);
                        let io_err = Type::Named {
                            id: self.defs.io_error_id,
                            args: vec![],
                        };
                        return Type::Fallible(Box::new(Type::Bytes), Box::new(io_err));
                    }
                    "read_bytes" => {
                        if args.len() != 1 {
                            return self.error(span, "read_bytes takes exactly 1 argument (path)");
                        }
                        self.check_expr(&args[0], &Type::Str);
                        let io_err = Type::Named {
                            id: self.defs.io_error_id,
                            args: vec![],
                        };
                        return Type::Fallible(Box::new(Type::Bytes), Box::new(io_err));
                    }
                    // Reading part of a file, and the four calls that make a
                    // store crash-safe.
                    //
                    // Every other file call here takes a whole file, which is
                    // the right shape for configuration and the wrong one for
                    // data: fetching one record out of a four-megabyte table
                    // read four megabytes, measured at 40,000x amplification,
                    // and a table larger than memory could not be read at all.
                    "read_bytes_at" => {
                        if args.len() != 3 {
                            return self.error(
                                span,
                                "read_bytes_at takes 3 arguments (path, offset, count)",
                            );
                        }
                        self.check_expr(&args[0], &Type::Str);
                        self.check_expr(&args[1], &Type::I64);
                        self.check_expr(&args[2], &Type::I64);
                        let io_err = Type::Named {
                            id: self.defs.io_error_id,
                            args: vec![],
                        };
                        return Type::Fallible(Box::new(Type::Bytes), Box::new(io_err));
                    }
                    "file_size" => {
                        if args.len() != 1 {
                            return self.error(span, "file_size takes exactly 1 argument (path)");
                        }
                        self.check_expr(&args[0], &Type::Str);
                        let io_err = Type::Named {
                            id: self.defs.io_error_id,
                            args: vec![],
                        };
                        return Type::Fallible(Box::new(Type::I64), Box::new(io_err));
                    }
                    "append_bytes" => {
                        if args.len() != 2 {
                            return self
                                .error(span, "append_bytes takes 2 arguments (path, bytes)");
                        }
                        self.check_expr(&args[0], &Type::Str);
                        self.check_expr(&args[1], &Type::Bytes);
                        let io_err = Type::Named {
                            id: self.defs.io_error_id,
                            args: vec![],
                        };
                        return Type::Fallible(Box::new(Type::Unit), Box::new(io_err));
                    }
                    "sync_file" => {
                        if args.len() != 1 {
                            return self.error(span, "sync_file takes exactly 1 argument (path)");
                        }
                        self.check_expr(&args[0], &Type::Str);
                        let io_err = Type::Named {
                            id: self.defs.io_error_id,
                            args: vec![],
                        };
                        return Type::Fallible(Box::new(Type::Unit), Box::new(io_err));
                    }
                    "rename_file" => {
                        if args.len() != 2 {
                            return self.error(span, "rename_file takes 2 arguments (from, to)");
                        }
                        self.check_expr(&args[0], &Type::Str);
                        self.check_expr(&args[1], &Type::Str);
                        let io_err = Type::Named {
                            id: self.defs.io_error_id,
                            args: vec![],
                        };
                        return Type::Fallible(Box::new(Type::Unit), Box::new(io_err));
                    }
                    "write_bytes" => {
                        if args.len() != 2 {
                            return self.error(span, "write_bytes takes 2 arguments (path, bytes)");
                        }
                        self.check_expr(&args[0], &Type::Str);
                        self.check_expr(&args[1], &Type::Bytes);
                        let io_err = Type::Named {
                            id: self.defs.io_error_id,
                            args: vec![],
                        };
                        return Type::Fallible(Box::new(Type::Unit), Box::new(io_err));
                    }
                    "make_dir" | "make_dir_new" | "remove_file" => {
                        if args.len() != 1 {
                            return self.error(
                                span,
                                format!("{} takes exactly 1 argument (path)", id.name),
                            );
                        }
                        self.check_expr(&args[0], &Type::Str);
                        let io_err = Type::Named {
                            id: self.defs.io_error_id,
                            args: vec![],
                        };
                        return Type::Fallible(Box::new(Type::Unit), Box::new(io_err));
                    }
                    "list_dir" => {
                        if args.len() != 1 {
                            return self.error(span, "list_dir takes exactly 1 argument (path)");
                        }
                        self.check_expr(&args[0], &Type::Str);
                        let io_err = Type::Named {
                            id: self.defs.io_error_id,
                            args: vec![],
                        };
                        return Type::Fallible(
                            Box::new(Type::List(Box::new(Type::Str))),
                            Box::new(io_err),
                        );
                    }
                    // Channels between the tasks of a `concurrent` block
                    // (docs/concurrency.md).
                    "chan_new" => {
                        if args.len() > 1 {
                            return self.error(
                                span,
                                "`chan_new` takes at most 1 argument (how many values it buffers)",
                            );
                        }
                        if let Some(n) = args.first() {
                            self.check_expr(n, &Type::I64);
                        }
                        let t = self.uni.fresh();
                        return Type::Named {
                            id: self.defs.chan_id,
                            args: vec![t],
                        };
                    }
                    // The first of several channels a `chan_recv` would not
                    // wait on ([E-CHANNEL]).
                    "chan_select" => {
                        if args.len() != 1 {
                            return self
                                .error(span, "`chan_select` takes 1 argument, a list of channels");
                        }
                        let t = self.uni.fresh();
                        let want = Type::List(Box::new(Type::Named {
                            id: self.defs.chan_id,
                            args: vec![t],
                        }));
                        self.check_expr(&args[0], &want);
                        return Type::I64;
                    }
                    "chan_send" | "chan_recv" | "chan_close" => {
                        let name = id.name.as_str();
                        let want = if name == "chan_send" { 2 } else { 1 };
                        if args.len() != want {
                            return self.error(
                                span,
                                format!("{name} takes {want} argument(s), got {}", args.len()),
                            );
                        }
                        let t = self.uni.fresh();
                        let chan = Type::Named {
                            id: self.defs.chan_id,
                            args: vec![t.clone()],
                        };
                        self.check_expr(&args[0], &chan);
                        return match name {
                            "chan_send" => {
                                self.check_expr(&args[1], &t);
                                // A `ref` handed to another task is a cell
                                // both hold: shared mutable state, which is
                                // what tasks give up ([E-CONCURRENT]). An
                                // actor is the sanctioned way to share one.
                                let sent = self.uni.resolve(&t);
                                if self.task_shares_ref(&sent) {
                                    self.error_help(
                                        args[1].span(),
                                        "a channel cannot carry a `ref`: the task receiving it would share the cell with the one sending it",
                                        "send the value itself, or wrap the shared state in an `actor` ([E-ACTOR])",
                                    );
                                }
                                Type::Unit
                            }
                            "chan_recv" => Type::Named {
                                id: self.defs.option_id,
                                args: vec![t],
                            },
                            _ => Type::Unit,
                        };
                    }
                    "args" => {
                        if !args.is_empty() {
                            return self.error(span, "args takes no arguments");
                        }
                        return Type::List(Box::new(Type::Str));
                    }
                    "map" => {
                        if !args.is_empty() {
                            return self.error(
                                span,
                                "map() takes no arguments — add entries with .set(k, v)",
                            );
                        }
                        let k = self.uni.fresh();
                        let v = self.uni.fresh();
                        // An empty literal like any other ([T-EMPTY-LIT]):
                        // `let m = map()` used only through `values()` left
                        // the *key* type decided by nothing, which `grasm run`
                        // answered anyway and the backend refused. Registering
                        // it here is what holds both engines to one answer.
                        let t = Type::Map(Box::new(k), Box::new(v));
                        self.empty_lits
                            .push((span.start, span.end - span.start, t.clone()));
                        return t;
                    }
                    // The two constants the math methods are useless
                    // without. Guarded on `lookup_fn`, so a program with its
                    // own `fn e()` keeps it — unlike the older builtins here,
                    // which shadow a user function of the same name. `e` is a
                    // common enough identifier that taking it outright would
                    // break working programs.
                    "pi" | "e" if self.lookup_fn(&id.name).is_none() => {
                        if !args.is_empty() {
                            return self.error(span, format!("{} takes no arguments", id.name));
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
                    // A clock that only ever goes forwards, and one that can
                    // wait.
                    //
                    // `now_millis` is the *wall* clock: it answers what a
                    // person would call the time, and it moves backwards when
                    // the machine is corrected. Measuring a duration with it
                    // gives a negative answer twice a year, and every timeout
                    // built on it fires early or late by however far the
                    // correction went. `monotonic_millis` has no relation to
                    // any calendar and only ever increases, which is what a
                    // duration needs.
                    //
                    // `monotonic_nanos` is the same clock read more finely:
                    // a request that takes forty microseconds is zero
                    // milliseconds, and a benchmark cannot average zeros.
                    "now_millis" | "monotonic_millis" | "monotonic_nanos" => {
                        if !args.is_empty() {
                            return self.error(span, format!("{} takes no arguments", id.name));
                        }
                        return Type::I64;
                    }
                    "sleep_millis" => {
                        if args.len() != 1 {
                            return self
                                .error(span, "sleep_millis takes 1 argument (milliseconds)");
                        }
                        self.check_expr(&args[0], &Type::I64);
                        return Type::Unit;
                    }
                    "env" => {
                        if args.len() != 1 {
                            return self
                                .error(span, "env takes exactly 1 argument (the variable name)");
                        }
                        self.check_expr(&args[0], &Type::Str);
                        return self.defs.option_some(Type::Str);
                    }
                    "exit" => {
                        if args.len() != 1 {
                            return self
                                .error(span, "exit takes exactly 1 argument (the status code)");
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
                            return self
                                .error(span, format!("`{}` takes exactly 1 argument", id.name));
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
                            return self
                                .error(span, "fail takes exactly 1 argument (the error value)");
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
                                        format!(
                                            "`fail` raises `{e}` but this function declares `{d}`"
                                        ),
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
                        self.tir_call(span, crate::tirfacts::CallRes::Variant);
                        return self.instantiate_variant(def_id, vi, args, span);
                    } else if cands.len() > 1 {
                        // **A module's own variant wins.**
                        //
                        // Two packages that each declare a private error type
                        // with an `Io` variant both compile alone, and a third
                        // program that imports both used to break *their*
                        // sources — with an error naming a file its author
                        // does not own and cannot fix. Nothing about `wal`
                        // became ambiguous when `sstable` was added to the
                        // program beside it; `wal` still declares exactly one
                        // `Io` and means it.
                        //
                        // So the name resolves against the module being
                        // checked before it is called ambiguous, which is also
                        // how a reader of that file reads it. Ambiguity is
                        // reserved for the case that really is one: a module
                        // with no `Io` of its own, seeing two.
                        if let Some((def_id, vi)) = self.own_variant(&cands) {
                            self.tir_call(span, crate::tirfacts::CallRes::Variant);
                            return self.instantiate_variant(def_id, vi, args, span);
                        }
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
        self.tir_call(span, crate::tirfacts::CallRes::Value);
        let callee_ty = self.infer_expr(callee);
        // Calling something whose type a deferred closure body will supply:
        // `let mk = |n| |m| n + m` then `mk(3)(4)` through a binding. The
        // shape has to exist before the arguments can be checked against it,
        // and the call that fixed `mk`'s parameters has already happened.
        self.force_deferred(&callee_ty);
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
    ///
    /// `owner` is the module whose rows the name is looked up in. A call
    /// written `mod.name(...)` resolves in `mod`, not in the calling module:
    /// looked up under the caller it carried no row at all, and
    /// `region { lib.cfg_user() }` was accepted — the handler is written
    /// outside the region and may keep what it is given, which is exactly how
    /// a value allocated inside gets out.
    fn check_region_call_in(&mut self, owner: &str, name: &str, args: &[Expr], span: Span) {
        if self.region_depth == 0 {
            return;
        }
        // A user-declared effect means a handler, and a handler is written
        // outside and may keep what it is given. `io`, `async` and `unsafe`
        // are built in: they have no handler and retain nothing.
        if let Some(row) = self.row_of_qualified(owner, name) {
            let user: Vec<&String> = row
                .iter()
                .filter(|e| !matches!(e.as_str(), "io" | "async" | "unsafe" | "mem"))
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
        let owner = self.current_module.clone();
        self.check_fn_call_in(&owner, name, info, args, span)
    }

    /// The same, for a call written `mod.name(...)`.
    ///
    /// The end-of-analysis map-key pass is keyed by the *callee*, and a
    /// qualified call recorded the caller's module instead — so
    /// `mk.tally([1.5])` over a `Map[T, _]` in `mk` went through the checker
    /// and into a runtime `invalid map key`, while the identical body called
    /// unqualified was refused ([T-MAP-KEY-GENERIC]).
    fn check_fn_call_in(
        &mut self,
        owner: &str,
        name: &str,
        info: &FnInfo,
        args: &[Expr],
        span: Span,
    ) -> Type {
        self.check_region_call_in(owner, name, args, span);
        let fresh: Vec<Type> = (0..info.generics).map(|_| self.uni.fresh()).collect();
        self.tir_call(
            span,
            crate::tirfacts::CallRes::Fn(format!("{owner}::{name}"), fresh.clone()),
        );
        for (v, b) in fresh.iter().zip(&info.bounds) {
            if !b.is_empty() {
                self.bound_obligations.push((
                    v.clone(),
                    b.clone(),
                    span,
                    self.current_bounds.clone(),
                    self.current_generics.clone(),
                ));
            }
        }
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
            // And whatever the body does with them, which is only known once
            // every body has been walked — this call may be checked first.
            self.pending_key_checks.push((
                format!("{owner}::{name}"),
                fresh.clone(),
                span,
                self.key_generic_owner.clone(),
            ));
            self.pending_par_checks.push((
                format!("{owner}::{name}"),
                fresh.clone(),
                span,
                self.key_generic_owner.clone(),
            ));
        }
        // A handler's discharge may not leave its block through a call that
        // writes to a place declared before it. The direct spellings —
        // `outer = closure` and `outer.push(closure)` — are refused where
        // they are written; this is the same store one call deep, and a
        // generic parameter hides the function type from the row analysis
        // entirely: `keep[T](inout xs: [T], v: T)` carried
        // `|| Cfg.value()` out of its `with` and dispatched it to a different
        // handler, with both engines agreeing on the wrong answer
        // ([E-HANDLER-MISSING]).
        if !self.with_floors.is_empty() {
            self.reject_handler_escape_through_call(args, info, span);
        }
        // Ownership Tier-0: `own` moves, `inout` needs an exclusive mutable place.
        let mut inout_roots: Vec<(String, Span)> = Vec::new();
        let mut inout_places: Vec<((String, Vec<PlaceSeg>), Span)> = Vec::new();
        let mut own_roots: Vec<String> = Vec::new();
        for ((a, p), mode) in args.iter().zip(&info.params).zip(&info.modes) {
            match mode {
                ParamMode::Own => {
                    if let Expr::Ident(id) = a {
                        let lent_linear = self.lookup(&id.name).is_some_and(|b| {
                            !b.owned && b.kind == BindKind::Param && self.is_linear(&b.ty)
                        });
                        if lent_linear {
                            self.error_help(
                                id.span,
                                format!("cannot give away `{}`: it is lent to this function, not owned by it", id.name),
                                "take the parameter as `own` to be able to consume it ([T-LINEAR])",
                            );
                        }
                        if !self.is_copy_type(p) && self.lookup(&id.name).is_some() {
                            self.mark_moved(id);
                            own_roots.push(id.name.clone());
                        }
                    }
                }
                ParamMode::Inout => {
                    // A tuple component is not a place ([G-PLACE]), and
                    // `inout` is the spelling that was missed: `bump(t.0)`
                    // typechecked and then panicked with `cannot access field
                    // \`0\``, the very message that clause exists to
                    // eliminate — after the snapshot had already emptied the
                    // component.
                    if let Some((_, segs)) = place_path(a) {
                        if segs.iter().any(|s| {
                            matches!(s, PlaceSeg::Field(f)
                            if f.chars().all(|c| c.is_ascii_digit()))
                        }) {
                            self.error_help(
                                a.span(),
                                "cannot pass a tuple component as `inout`".to_string(),
                                "a tuple is a fixed group of values with no place to write \
into — bind the component to a `var` and rebuild the tuple, or use a struct when the \
parts have names worth writing",
                            );
                            continue;
                        }
                    }
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
                            if let Some(here) = place_path(a) {
                                if inout_places
                                    .iter()
                                    .any(|(prev, _)| places_overlap(prev, &here))
                                {
                                    self.error_help(
                                        id.span,
                                        format!(
                                            "`{}` is passed as `inout` twice in one call",
                                            id.name
                                        ),
                                        "two exclusive references to the same value cannot coexist",
                                    );
                                }
                                inout_places.push((here, id.span));
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
            Some(err) => Type::Fallible(Box::new(ret), Box::new(substitute_generics(err, &fresh))),
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
                            self.tir_name(span, crate::tirfacts::NameRes::Variant);
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
        // A handler's operations belong to the effect system. They are the
        // one function value with no row to check — which is what lets a
        // handler be written at all — so taking one out and calling it
        // directly launders whatever it does. Asked here, after the
        // type-qualified forms, because those receivers are not values.
        // Walked at most once. This function used to infer the receiver here
        // and again below, so a chain of field reads cost 2^n: `n.next` forty
        // times deep never finished typechecking, at 13 MB of memory and one
        // core — under a nesting cap that [G-NEST-LIMIT] says must *report*
        // rather than hang.
        let mut recv_ty0: Option<Type> = None;
        if !matches!(recv, Expr::Ident(id) if self.lookup(&id.name).is_none()) {
            let rt0 = self.infer_expr(recv);
            recv_ty0 = Some(rt0.clone());
            if let Type::Named { id, .. } = self.uni.resolve(&rt0) {
                if self.handler_ids.contains(&id)
                    && matches!(&self.defs.get(id).kind, TypeDefKind::Struct { fields }
                        if fields.iter().any(|(n, _)| n == &name.name))
                {
                    return self.error_help(
                        span,
                        format!("`{}` is an operation of a handler, not a field to read", name.name),
                        "a handler's operations are called by performing the effect inside `with ... handled_by`, which is what makes their effects count where the handler is installed",
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
                            self.tir_name(span, crate::tirfacts::NameRes::Const(qual.clone()));
                            return ty;
                        }
                        return self.error_help(
                            name.span,
                            format!(
                                "constant `{}` in module `{target_mod}` is private",
                                name.name
                            ),
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
                                Some(e) => {
                                    Type::Fallible(Box::new(info.ret.clone()), Box::new(e.clone()))
                                }
                                None => info.ret.clone(),
                            };
                            // A generic's type parameters are instantiated
                            // here, as they are for a bare name. Leaving them
                            // as `Generic(i)` meant `xs |> lib.uniq` and
                            // `xs.map(lib.uniq)` reported `expected [T0]`,
                            // while `lib.uniq(xs)` on the next line was fine.
                            let fresh: Vec<Type> =
                                (0..info.generics).map(|_| self.uni.fresh()).collect();
                            // The function's *own* row travels with it here
                            // exactly as it does for a bare name, and it did
                            // not: `lib.shout` named as a value carried an
                            // empty row, so one `import` was enough to hand a
                            // printing function to a `/ {}` row, to `par_map`,
                            // to a `capabilities = []` package, and to a call
                            // made from inside a region. [T-FN-VALUE-ROW] says
                            // naming a function contributes its row to the
                            // function doing the naming, and says nothing
                            // about which module it was declared in.
                            self.tir_name(
                                span,
                                crate::tirfacts::NameRes::Fn(qual.clone(), fresh.clone()),
                            );
                            let own_row = self.row_of_qualified(&target_mod, &name.name);
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
                        // The module is imported and the member is not
                        // there. Falling through reported "unknown name
                        // `util`" about the module — which exists — and
                        // advised declaring it with `let`. The call position
                        // already said the right thing.
                        None => {
                            return self.error(
                                name.span,
                                format!("module `{target_mod}` has no function `{}`", name.name),
                            );
                        }
                    }
                }
            }
        }
        let recv_ty = match recv_ty0 {
            Some(t) => t,
            None => self.infer_expr(recv),
        };
        // Not known yet. A closure's body waits for its call sites, so the
        // type of what it returns is not settled until the end of the block —
        // and asking here reported ``_` has no field `c``, naming a type no
        // program can write. The read waits with it.
        if matches!(self.uni.shallow_resolve(&recv_ty), Type::Var(_))
            && !self.deferred_fields.is_empty()
        {
            let out = self.uni.fresh();
            self.deferred_fields
                .last_mut()
                .expect("checked non-empty")
                .push(DeferredField {
                    recv: recv_ty,
                    name: name.clone(),
                    out: out.clone(),
                });
            return out;
        }
        self.field_type_of(&recv_ty, name, span)
    }

    /// The type of `recv.name`, with the receiver's type already inferred.
    fn field_type_of(&mut self, recv_ty: &Type, name: &Ident, span: Span) -> Type {
        // Tier 2: a `ref T` reads as the `T` it points at. The check happens
        // at run time, on the generation, so a reference to a released slot
        // panics rather than reading whatever occupies it now.
        let (recv_ty, through_ref) = match self.uni.resolve(recv_ty) {
            Type::Ref(inner) => (*inner, true),
            other => (other, false),
        };
        // `.value` is how a reference to something with no fields of its own is
        // read and written ([T-REF-VALUE]). Without it `ref(0)` typechecked,
        // produced a `ref i64`, and nothing in the language could observe it.
        //
        // Not offered for a struct or a tuple: a struct already has field names
        // and `.value` would either shadow one or change meaning the day
        // somebody added it, and a tuple has `.0` and `.1`.
        //
        // A *sum* has neither, so it is exactly the case the rule describes —
        // but a sum is a `Named` type too, and asking whether the referent is
        // named excluded it along with the structs. `ref(Red).value` was
        // refused as a field access on a sum, and since rebinding a `var` only
        // moves the local, nothing in the language could write through a `ref`
        // to a sum at all: readable through `match`, never writable, on the
        // tier that exists so a reference is a shared cell rather than a copy.
        let referent_is_struct = matches!(
            self.uni.shallow_resolve(&recv_ty),
            Type::Named { id, .. } if matches!(
                self.defs.get(id).kind, TypeDefKind::Struct { .. })
        );
        if through_ref
            && name.name == "value"
            && !referent_is_struct
            && !matches!(self.uni.shallow_resolve(&recv_ty), Type::Tuple(_))
        {
            return recv_ty;
        }
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
                self.error(name.span, format!("`{shown}` has no field `{}`", name.name))
            }
        }
    }

    fn infer_method_call(&mut self, recv: &Expr, name: &Ident, args: &[Expr], span: Span) -> Type {
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
                            // Belt and braces for [T-UNSAFE-FFI]: a C
                            // function reached through a qualifier is still a
                            // C function.
                            if self.unsafe_depth == 0 && self.extern_names.contains(&qual) {
                                self.error_help(
                                    name.span,
                                    format!(
                                        "`{}` is a C function, so it can only be used inside `unsafe`",
                                        name.name
                                    ),
                                    "wrap the use in `unsafe { ... }`; the package must then \
declare the `unsafe` capability, which is what makes it visible to everyone who depends on you",
                                );
                            }
                            self.tir_method(span, crate::tirfacts::MethodRes::Module);
                            return self.check_fn_call_in(
                                &target_mod,
                                &name.name,
                                &info,
                                args,
                                span,
                            );
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
                                    self.tir_method(span, crate::tirfacts::MethodRes::Variant);
                                    return self.instantiate_variant(def_id, vi, args, span);
                                }
                            }
                            return self.error(
                                name.span,
                                format!("module `{target_mod}` has no function `{}`", name.name),
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
                            "a handler can keep what it is given, and nothing here says \
this one does not — which is how a value allocated inside the region would get out. \
Perform the effect after the closing brace, or yield a number and do the work outside",
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
                            self.tir_method(
                                span,
                                crate::tirfacts::MethodRes::EffectOp(
                                    tid.name.clone(),
                                    name.name.clone(),
                                ),
                            );
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
                            self.tir_method(span, crate::tirfacts::MethodRes::Variant);
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
        // A mutating method writes to its receiver, and inside a region a
        // write to something declared outside is the whole problem. Asked
        // before the receiver is typed, so that is the error the reader sees:
        // typed first, `keep.push(Box{ f: || made })` reported "a function
        // value cannot be used inside a region" about `keep`, with help
        // asking for a closure written inside the region — which it was.
        if self.region_depth > 0 && crate::builtins::MUTATING_METHODS.contains(&name.name.as_str())
        {
            let mut root = recv;
            while let Expr::Field { recv, .. } | Expr::Index { recv, .. } = root {
                root = recv;
            }
            if let Expr::Ident(id) = root {
                self.reject_region_escape(id, "mutate");
            }
        }
        if self.go_floor.is_some()
            && crate::builtins::MUTATING_METHODS.contains(&name.name.as_str())
        {
            let mut root = recv;
            while let Expr::Field { recv, .. } | Expr::Index { recv, .. } = root {
                root = recv;
            }
            if let Expr::Ident(id) = root {
                self.reject_task_write(id);
            }
        }
        let recv_ty = self.infer_expr(recv);
        // A method is dispatched on the receiver's *shape*, so a call to a
        // `let`-bound closure has to have its body checked first — the call
        // itself has just fixed that closure's parameter types. Without this
        // `let g = |y| f(y).len()` was refused with "no method `len` on `_`",
        // naming the inference variable it was about to resolve.
        self.force_deferred(&recv_ty);
        let recv_res = self.uni.resolve(&recv_ty);
        // A method an `impl` provides is found before a builtin of the same
        // name ([T-INTERFACE]).
        if let Some(result) = self.interface_method(&recv_res, recv, name, args, span) {
            return result;
        }
        if let Some(result) = self.builtin_method(&recv_res, recv, name, args, span) {
            self.tir_method(span, crate::tirfacts::MethodRes::Builtin);
            return result;
        }
        // UFCS: `x.f(a)` → `f(x, a)`.
        //
        // Only when the free function's first parameter *accepts the
        // receiver*, which is what [L-UFCS] says and what was never checked.
        // Sharing a name was enough, so declaring `fn len(xs: [i64])` anywhere
        // in the module made every `n.len()` on a struct with a `len` field a
        // type error about `[i64]` — a type that appears nowhere near the
        // call. [T-FIELD-CALL] promises the opposite in as many words: a field
        // is reached when no method and no UFCS candidate takes the receiver.
        if let Some(info) = self
            .lookup_fn(&name.name)
            .filter(|info| self.ufcs_accepts(info, &recv_res))
        {
            // Including the Tier 3 gate. [L-UFCS] says the two spellings are
            // the same call, and only one of them was checked: `n.close()`
            // reached libc with no `unsafe`, no `io` in the row, and no
            // capability — through a rule that exists to make exactly that
            // impossible.
            self.require_unsafe(&name.name, name.span);
            let mut full_args: Vec<Expr> = Vec::with_capacity(args.len() + 1);
            full_args.push(recv.clone());
            full_args.extend(args.iter().cloned());
            self.tir_method(span, crate::tirfacts::MethodRes::Ufcs);
            return self.check_fn_call(&name.name, &info, &full_args, span);
        }
        // A field holding a function, called through the field. Storing one
        // and building the value both worked, and `n.run(5)` did not: the only
        // spelling that ran was `let f = n.run` and then `f(5)`, which nothing
        // pointed to. The help offered instead — declare `fn run(Node, ...)` —
        // is a different function that would not call the field at all.
        if self.struct_field_type(&recv_res, &name.name).is_some() {
            // Recorded, because the engines have no types to decide this with:
            // the interpreter looked a free function up by name and called it
            // with the struct, so a program the checker had resolved to the
            // field printed the free function's answer instead.
            self.field_calls.insert((span.start, span.end));
            self.tir_method(span, crate::tirfacts::MethodRes::FieldFn);
            let ft = self.infer_field(recv, name, span);
            let resolved = self.uni.shallow_resolve(&ft);
            if let Type::Fn { params, ret, row } = resolved {
                // A call through a field or a tuple component: the thing being
                // called has no expression of its own for the effect pass to
                // ask about, so the call site records what it calls.
                if let Some(r) = row {
                    self.callee_rows.insert((span.start, span.end), r);
                }
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
        // The receiver's type is already broken, and the error that broke it
        // has been reported. Saying "no method `len` on `<error>`" adds
        // nothing, and the help suggested writing `fn len(<error>, ...)` — a
        // signature nobody can type.
        if matches!(self.uni.resolve(&recv_ty), Type::Error) {
            return Type::Error;
        }
        // Nothing determined the receiver's type, and the reason is reported
        // where it belongs — at the literal that has no element type
        // ([T-EMPTY-LIT]). "no method `to_str` on `_`" points at the method
        // and names a type nobody can write.
        if matches!(self.uni.resolve(&recv_ty), Type::Var(_)) {
            return self.uni.fresh();
        }
        let shown = self.show(&recv_ty);
        // A *built-in* free function is not a method of every type, and
        // offering UFCS for one promised a call the compiler will not make:
        // the help said "a free function `fn print(str, ...)` would also work
        // here" about `print`, which is exactly what had just been refused.
        // `|>` is the spelling for applying a free function to a value
        // ([G-PIPE]), and it is the one that works.
        if BUILTIN_FREE_FNS.contains(&name.name.as_str()) {
            return self.error_help(
                name.span,
                format!(
                    "`{}` is a built-in function, not a method on `{shown}`",
                    name.name
                ),
                format!(
                    "write `x |> {}` to pass a value to it, or `{}(x)`",
                    name.name, name.name
                ),
            );
        }
        // A range has no name in the type grammar — `fn f(r: range)` is
        // "unknown type `range`" — so offering UFCS for it is the third
        // instance of the same mistake as the two guarded above: help that
        // proposes a signature nobody can write. What a reader wants here is
        // the method they meant, so name the ones that exist.
        //
        // The list is checked against the implementation by
        // `range_methods_are_the_ones_the_help_names` rather than remembered,
        // since a help text that names a method the compiler dropped is worse
        // than no help.
        if matches!(self.uni.resolve(&recv_ty), Type::Range) {
            let have = RANGE_METHODS;
            let near = have
                .iter()
                .find(|m| looks_like(m, &name.name))
                .map(|m| format!("did you mean `{m}`?"));
            return self.error_help(
                name.span,
                format!("no method `{}` on a range", name.name),
                near.unwrap_or_else(|| {
                    format!(
                        "a range has {} — or `.to_list()` first, for anything a list can do",
                        have.iter()
                            .map(|m| format!("`{m}`"))
                            .collect::<Vec<_>>()
                            .join(", ")
                    )
                }),
            );
        }
        self.error_help(
            name.span,
            format!("no method `{}` on `{shown}`", name.name),
            format!(
                "a free function `fn {}({shown}, ...)` would also work here (UFCS)",
                name.name
            ),
        )
    }

    /// Whether a free function could take this receiver as its first argument.
    ///
    /// Deliberately generous: an inference variable, a generic parameter or an
    /// error type could still turn out to fit, and refusing those would make
    /// this rule the thing that breaks a program. Only a *definite* mismatch —
    /// two concrete types that cannot be the same — disqualifies the function,
    /// which is exactly the case that was mistaking a field for a bad call.
    fn ufcs_accepts(&self, info: &FnInfo, recv: &Type) -> bool {
        let Some(first) = info.params.first() else {
            return false;
        };
        fn compatible(a: &Type, b: &Type) -> bool {
            use Type::*;
            match (a, b) {
                (Var(_), _) | (_, Var(_)) => true,
                (Generic(_), _) | (_, Generic(_)) => true,
                (Error, _) | (_, Error) => true,
                (List(x), List(y)) => compatible(x, y),
                (Ref(x), Ref(y)) => compatible(x, y),
                // A reference is *not* interchangeable with what it points at
                // here. Reading through one is [T-REF-READ]; being *passed* as
                // one is not, and `fn step(b: Box, …)` cannot take a
                // `ref Box`. Treating them as compatible made that function a
                // UFCS candidate for `r.step(5)`, which then failed its
                // argument check — so a field call through a reference was
                // refused with "expected `Box`, found `ref Box`" instead of
                // falling back to the field, which is what [T-FIELD-CALL]
                // promises when no candidate accepts the receiver.
                (Ref(_), _) | (_, Ref(_)) => false,
                (Map(k1, v1), Map(k2, v2)) => compatible(k1, k2) && compatible(v1, v2),
                (Tuple(x), Tuple(y)) => {
                    x.len() == y.len() && x.iter().zip(y).all(|(p, q)| compatible(p, q))
                }
                (Named { id: i1, .. }, Named { id: i2, .. }) => i1 == i2,
                (Fallible(o1, _), Fallible(o2, _)) => compatible(o1, o2),
                (Fn { .. }, Fn { .. }) => true,
                _ => a == b,
            }
        }
        compatible(&self.uni.resolve(first), &self.uni.resolve(recv))
    }

    /// The declared type of `field` on a struct type, if `t` is one with it.
    fn struct_field_type(&mut self, t: &Type, field: &str) -> Option<Type> {
        match self.uni.shallow_resolve(t) {
            Type::Named { id, .. } => {
                let TypeDefKind::Struct { fields } = &self.defs.get(id).kind else {
                    return None;
                };
                fields
                    .iter()
                    .find(|(n, _)| n == field)
                    .map(|(_, t)| t.clone())
            }
            // A tuple's components are fields spelled with numbers. Reading
            // one as a value already worked; calling one did not, so a
            // function in a tuple could only be called by binding it to a
            // name first — the same defect [T-FIELD-CALL] records for struct
            // fields, and the help offered was `fn 0(...)`, which is not a
            // name anyone can write.
            Type::Tuple(items) => field
                .parse::<usize>()
                .ok()
                .and_then(|i| items.get(i).cloned()),
            // Reading a field through a reference works, so calling one does
            // too ([T-REF-READ]).
            //
            // `.value` is how a reference to something with no fields of its
            // own is read and written ([T-REF-VALUE]). Without it `ref(0)`
            // typechecked, produced a `ref i64`, and no expression in the
            // language could observe it — a value that could be constructed
            // and never used.
            //
            // Not offered for a struct or a tuple: a struct already has field
            // names and `.value` would either shadow a real one or start
            // meaning something different the day somebody added it, and a
            // tuple has `.0` and `.1`.
            Type::Ref(inner) => {
                let inner = self.uni.resolve(&inner);
                if field == "value" && !matches!(inner, Type::Named { .. } | Type::Tuple(_)) {
                    Some(inner)
                } else {
                    self.struct_field_type(&inner, field)
                }
            }
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
                    format!(
                        "`{}` takes {want} argument(s), got {}",
                        name.name,
                        args.len()
                    ),
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
                            let f = Type::Fn {
                                params: vec![acc.clone(), elem.clone()],
                                ret: Box::new(acc.clone()),
                                row: None,
                            };
                            // The function first when the seed is an integer
                            // literal: the literal adopts what it meets, and
                            // checking it first made it decide instead.
                            // `[1.0, 2.0].fold(0, |x, y| x + y)` reported the
                            // mismatch on `x + y` and wanted `0.0` — and a
                            // bare `0` is what anyone writes for an
                            // accumulator ([T-INT-TO-FLOAT]).
                            // Folding floats with a bare `0`: the seed is a
                            // literal, so it adopts the element type rather
                            // than deciding for it. Reordering the two checks
                            // instead would break a closure that calls a
                            // method *on* the accumulator, which needs its
                            // type known first.
                            if Self::is_int_literal(&args[0])
                                && self.uni.shallow_resolve(&elem) == Type::F64
                            {
                                self.uni.unify(&acc, &Type::F64);
                            }
                            self.check_expr(&args[0], &acc);
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
                            self.reject_parallel_ref_elem(&elem, name, span);
                            let f = Type::Fn {
                                params: vec![elem.clone()],
                                ret: Box::new(out.clone()),
                                row: None,
                            };
                            self.check_parallel_fn(&args[0], &f);
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
                            if name.name == "par_filter" {
                                self.reject_parallel_ref_elem(&elem, name, span);
                                self.check_parallel_fn(&args[0], &f);
                            } else {
                                self.check_expr(&args[0], &f);
                            }
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
                        if !matches!(
                            r,
                            Type::I64 | Type::Int(_) | Type::F64 | Type::Var(_) | Type::Error
                        ) {
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
                            self.error(
                                span,
                                format!("`join` needs `[str]`, found list of `{shown}`"),
                            );
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
                // `len` counts code points, which is right for a high-level
                // language and wrong the moment a program has to say how long
                // a string is on the wire.
                "byte_len" => {
                    check_arity(self, 0);
                    Type::I64
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
                // The other direction from `bytes.to_str()`: text to its UTF-8
                // encoding ([L-LIB-BYTES]). This is where a program working in
                // bytes gets them from a literal.
                "to_bytes" => {
                    check_arity(self, 0);
                    Type::Bytes
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
                // The other direction. `parse_int` deliberately refuses `0xFF`
                // ([L-LIB-PARSE]), so without this a program could write hex
                // and not read it back.
                "parse_int_base" => {
                    if check_arity(self, 1) {
                        self.check_expr(&args[0], &Type::I64);
                    }
                    self.defs.option_some(Type::I64)
                }
                // `parse_int` answers an `i64`, which has no room for the top
                // half of a `u64` ([T-U64]).
                "parse_u64" => {
                    check_arity(self, 0);
                    self.defs.option_some(Type::Int(IntKind::U64))
                }
                "parse_u64_base" => {
                    if check_arity(self, 1) {
                        self.check_expr(&args[0], &Type::I64);
                    }
                    self.defs.option_some(Type::Int(IntKind::U64))
                }
                "parse_float" => {
                    check_arity(self, 0);
                    self.defs.option_some(Type::F64)
                }
                _ => return None,
            },
            // A range answers the list combinators by producing a list.
            //
            // Without them, every program that builds a list from a count had
            // to write `var xs: [i64] = []`, a `for`, a `push` and two braces —
            // four lines and a dozen tokens where `(0..n).map(f)` says the same
            // thing. [G11] budgets Grasm at or below Python's token count and it
            // was 1.41×, and this pattern is where the difference was.
            //
            // These are methods rather than new syntax: the kernel's form count
            // ([spec/core.md]) does not move, because a call is a call.
            Type::Range => match name.name.as_str() {
                "to_list" => {
                    check_arity(self, 0);
                    Type::List(Box::new(Type::I64))
                }
                "map" => {
                    let out = self.uni.fresh();
                    if check_arity(self, 1) {
                        let f = Type::Fn {
                            params: vec![Type::I64],
                            ret: Box::new(out.clone()),
                            row: None,
                        };
                        self.check_expr(&args[0], &f);
                    }
                    // The element type, for the backend's fused
                    // `(a..b).map(f)` loop. Recorded unresolved and decoded
                    // once inference ends, for the same reason
                    // `map_get_pending` is.
                    self.map_get_pending
                        .push(((span.start, span.end), out.clone()));
                    Type::List(Box::new(out))
                }
                "filter" => {
                    if check_arity(self, 1) {
                        let f = Type::Fn {
                            params: vec![Type::I64],
                            ret: Box::new(Type::Bool),
                            row: None,
                        };
                        self.check_expr(&args[0], &f);
                    }
                    Type::List(Box::new(Type::I64))
                }
                "sum" => {
                    check_arity(self, 0);
                    Type::I64
                }
                "len" => {
                    check_arity(self, 0);
                    Type::I64
                }
                _ => return None,
            },
            // Methods on `bytes` ([L-LIB-BYTES]). Indexing gives an `i64` in
            // 0..255 rather than a one-byte value type: the language has one
            // integer, and a byte is a number.
            Type::Bytes => match name.name.as_str() {
                "len" => {
                    check_arity(self, 0);
                    Type::I64
                }
                "at" => {
                    if check_arity(self, 1) {
                        self.check_expr(&args[0], &Type::I64);
                    }
                    Type::I64
                }
                // `(start, end)`, the same shape as `list.slice` — see
                // ProjectGold/FINDINGS.md F97 for why it is not `(start, len)`.
                "slice" => {
                    if check_arity(self, 2) {
                        self.check_expr(&args[0], &Type::I64);
                        self.check_expr(&args[1], &Type::I64);
                    }
                    Type::Bytes
                }
                "concat" => {
                    if check_arity(self, 1) {
                        self.check_expr(&args[0], &Type::Bytes);
                    }
                    Type::Bytes
                }
                // The first position at or after `from` holding `byte`: where
                // the next line or field ends, found without walking the
                // buffer a byte at a time in the program.
                "index_of" => {
                    if check_arity(self, 2) {
                        self.check_expr(&args[0], &Type::I64);
                        self.check_expr(&args[1], &Type::I64);
                    }
                    self.defs.option_some(Type::I64)
                }
                // Writing into the buffer ([L-LIB-BYTES-WRITE]). Each writes a
                // fixed number of bytes into a buffer whose length does not
                // change, which is what lets a `bytes` keep being a window on
                // a larger allocation in the compiled runtime.
                "set" => {
                    if check_arity(self, 2) {
                        self.check_expr(&args[0], &Type::I64);
                        self.check_expr(&args[1], &Type::I64);
                        self.require_mutable(recv, name);
                    }
                    Type::Unit
                }
                "fill" => {
                    if check_arity(self, 1) {
                        self.check_expr(&args[0], &Type::I64);
                        self.require_mutable(recv, name);
                    }
                    Type::Unit
                }
                // `(at, src)` rather than `(src, at)`: the destination offset
                // reads first because the receiver is the destination.
                "copy_from" => {
                    if check_arity(self, 2) {
                        self.check_expr(&args[0], &Type::I64);
                        self.check_expr(&args[1], &Type::Bytes);
                        self.require_mutable(recv, name);
                    }
                    Type::Unit
                }
                "to_list" => {
                    check_arity(self, 0);
                    Type::List(Box::new(Type::I64))
                }
                // Lossy, for the same reason `tcp_read` is ([L-LIB-NET]): a
                // buffer carries bytes and a `str` carries text, and a
                // malformed byte should not end a program that is inspecting
                // one.
                "to_str" => {
                    check_arity(self, 0);
                    Type::Str
                }
                "is_empty" => {
                    check_arity(self, 0);
                    Type::Bool
                }
                _ => return None,
            },
            // Raw memory through an address ([L-LIB-MEM], [L-LIB-ATOMIC]).
            // Any other name falls through to UFCS, as on every type.
            Type::Addr => {
                let kind = crate::mem::method(&name.name)?;
                return Some(self.check_mem_method(kind, name, args, span));
            }
            // The builders ([L-LIB-BUILDER]). Each append writes through
            // the receiver, so it needs a mutable place, as `push` does.
            t @ (Type::StrBuf | Type::ByteBuf) => {
                let bytes = matches!(t, Type::ByteBuf);
                match (name.name.as_str(), bytes) {
                    ("push_str", _) => {
                        if check_arity(self, 1) {
                            self.check_expr(&args[0], &Type::Str);
                            self.require_mutable(recv, name);
                        }
                        Type::Unit
                    }
                    ("push_byte", true) => {
                        if check_arity(self, 1) {
                            self.check_expr(&args[0], &Type::I64);
                            self.require_mutable(recv, name);
                        }
                        Type::Unit
                    }
                    ("push_bytes", true) => {
                        if check_arity(self, 1) {
                            self.check_expr(&args[0], &Type::Bytes);
                            self.require_mutable(recv, name);
                        }
                        Type::Unit
                    }
                    ("set", true) => {
                        if check_arity(self, 2) {
                            self.check_expr(&args[0], &Type::I64);
                            self.check_expr(&args[1], &Type::I64);
                            self.require_mutable(recv, name);
                        }
                        Type::Unit
                    }
                    ("at", true) => {
                        if check_arity(self, 1) {
                            self.check_expr(&args[0], &Type::I64);
                        }
                        Type::I64
                    }
                    ("len", _) => {
                        check_arity(self, 0);
                        Type::I64
                    }
                    ("is_empty", _) => {
                        check_arity(self, 0);
                        Type::Bool
                    }
                    ("to_str", _) => {
                        check_arity(self, 0);
                        Type::Str
                    }
                    ("to_bytes", true) => {
                        check_arity(self, 0);
                        Type::Bytes
                    }
                    _ => return None,
                }
            }
            Type::Tuple(_) if name.name == "cmp" => return self.check_cmp(recv, args, span),
            Type::I64 | Type::Int(_) => {
                // Every integer type has the same methods, answering in its
                // own type: `b.abs()` on a `u8` is a `u8` ([T-INT-FIXED]).
                let me = recv_ty.clone();
                let n = name.name.as_str();
                // Conversions to another integer width: `as_u8` wraps, the
                // way arithmetic does; `try_u8` answers `None` for a value
                // that does not fit ([L-LIB-INT-CONV]).
                if let Some(target) = n.strip_prefix("as_").and_then(IntKind::from_name) {
                    check_arity(self, 0);
                    return Some(Type::Int(target));
                }
                if let Some(target) = n.strip_prefix("try_").and_then(IntKind::from_name) {
                    check_arity(self, 0);
                    return Some(self.defs.option_some(Type::Int(target)));
                }
                match n {
                    "cmp" => return self.check_cmp(recv, args, span),
                    "from_bits" if me == Type::I64 => {
                        check_arity(self, 0);
                        Type::F64
                    }
                    "as_f64" => {
                        check_arity(self, 0);
                        Type::F64
                    }
                    // Widening, so it never loses anything; on an `i64` it is
                    // the value itself. A `u64` from 2^63 up has no `i64`
                    // with its value, and gives its bits ([T-U64]).
                    "as_i64" => {
                        check_arity(self, 0);
                        Type::I64
                    }
                    "try_i64" => {
                        check_arity(self, 0);
                        self.defs.option_some(Type::I64)
                    }
                    // The other half of `code_at`: a code point back to text.
                    "to_char" => {
                        check_arity(self, 0);
                        Type::Str
                    }
                    "abs" => {
                        check_arity(self, 0);
                        me
                    }
                    "min" | "max" => {
                        if check_arity(self, 1) {
                            self.check_expr(&args[0], &me);
                        }
                        me
                    }
                    // [L-LIB-INT-CONV]: `None` where the true result is out of
                    // range, or the nearest end of the range.
                    "checked_add" | "checked_sub" | "checked_mul" => {
                        if check_arity(self, 1) {
                            self.check_expr(&args[0], &me);
                        }
                        self.defs.option_some(me)
                    }
                    "saturating_add" | "saturating_sub" | "saturating_mul" => {
                        if check_arity(self, 1) {
                            self.check_expr(&args[0], &me);
                        }
                        me
                    }
                    // Bit manipulation, as methods rather than operators
                    // ([L-LIB-BITS]). The three obvious symbols are all taken or
                    // ambiguous: `^` is exponentiation, `|` begins a closure and
                    // separates sum-type variants, and `&&` is already reported as
                    // "written `and`". Words would collide with the logical
                    // operators, which are `and`/`or`/`not` — so methods, which is
                    // what `.abs()` and `.cmp()` already are.
                    "bit_and" | "bit_or" | "bit_xor" => {
                        if check_arity(self, 1) {
                            self.check_expr(&args[0], &me);
                        }
                        me
                    }
                    // A shift count is a count, whatever the width shifted.
                    "shl" | "shr" | "ushr" => {
                        if check_arity(self, 1) {
                            self.check_expr(&args[0], &Type::I64);
                        }
                        me
                    }
                    "bit_not" => {
                        check_arity(self, 0);
                        me
                    }
                    // [L-LIB-BITS-WIDTH]: the bits at the receiver's own
                    // width. A count is an `i64`, as a shift count is, and so
                    // is the answer of the three that count.
                    "rotate_left" | "rotate_right" => {
                        if check_arity(self, 1) {
                            self.check_expr(&args[0], &Type::I64);
                        }
                        me
                    }
                    "count_ones" | "leading_zeros" | "trailing_zeros" => {
                        check_arity(self, 0);
                        Type::I64
                    }
                    "swap_bytes" => {
                        check_arity(self, 0);
                        me
                    }
                    "to_str" => {
                        check_arity(self, 0);
                        Type::Str
                    }
                    // Base ten is not the only base a systems program needs. Every
                    // protocol, hash, colour and permission mask is read and
                    // written in hex or binary, and those are the same programs
                    // that needed [L-LIB-BITS].
                    "to_base" => {
                        if check_arity(self, 1) {
                            self.check_expr(&args[0], &Type::I64);
                        }
                        Type::Str
                    }
                    _ => return None,
                }
            }
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
                // Toward zero, then held to the type's range, the way
                // `as_i64` is held to `i64`'s ([L-LIB-INT-CONV]).
                n if n.strip_prefix("as_").and_then(IntKind::from_name).is_some() => {
                    check_arity(self, 0);
                    Type::Int(IntKind::from_name(&n[3..]).expect("checked above"))
                }
                // The IEEE-754 bit pattern, and back.
                //
                // `as_i64` answers what a float *means*; this answers what it
                // *is*. Without it a program cannot write a float to a file in
                // any binary format, cannot read one back, and cannot compress
                // a series of them — every technique for that works by XORing
                // consecutive bit patterns. The text form round-trips, but
                // `1e300` spells out to 303 characters where the value is
                // eight bytes.
                "to_bits" => {
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
                "sin" | "cos" | "tan" | "asin" | "acos" | "atan" | "sinh" | "cosh" | "tanh"
                | "exp" | "ln" | "log2" | "log10" | "cbrt" | "to_radians" | "to_degrees" => {
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
                        // Recorded for the backend, which cannot work this out
                        // for itself: its type probe is structural, so a map
                        // built by `var m = map()` and typed by later use looks
                        // like nothing in particular. See `map_get_value_kind`.
                        // The type is recorded, not yet decoded: for `var m =
                        // map()` the value type is still an inference variable
                        // here and only becomes `i64` once the rest of the
                        // function has been seen. Decoding now would answer
                        // "unknown" for exactly the case this table exists for.
                        self.map_get_pending
                            .push(((span.start, span.end), v.clone()));
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

    /// Checks the deferred closure body that produces `t`, if there is one.
    ///
    /// A closure with parameters waits for a call site to say what they are,
    /// so its return type is an inference variable until its body is checked.
    /// Anything that needs the *shape* of that type — not just to unify with
    /// it — has to ask for the body now.
    fn force_deferred(&mut self, t: &Type) -> bool {
        let Type::Var(v) = self.uni.shallow_resolve(t) else {
            return false;
        };
        for depth in (0..self.deferred_closures.len()).rev() {
            let hit = self.deferred_closures[depth]
                .iter()
                .position(|d| matches!(self.uni.shallow_resolve(&d.ret), Type::Var(w) if w == v));
            if let Some(i) = hit {
                let d = self.deferred_closures[depth].remove(i);
                self.check_deferred_closure(d);
                return true;
            }
        }
        false
    }

    /// Reports discarded failures whose type only became known later.
    fn flush_discards(&mut self) {
        let pending = std::mem::take(&mut self.pending_discards);
        for (span, ty) in pending {
            if matches!(self.uni.shallow_resolve(&ty), Type::Fallible(..)) {
                let shown = self.show(&ty);
                self.error_help(
                    span,
                    format!("this expression can fail (`{shown}`) and its failure is discarded"),
                    "use `try` to propagate it, `match` to handle both cases, or `??` with a fallback to say what should happen instead",
                );
            }
        }
    }

    /// Refuses a parallel operation handed the *name* of a closure that
    /// reaches a `ref` from outside itself.
    ///
    /// Deferred to the end because a closure whose parameter types nothing
    /// fixes has its body checked after the `let` that binds it — and after
    /// the call that fixed them, which may be the parallel operation itself.
    fn flush_par_name_checks(&mut self) {
        let pending = std::mem::take(&mut self.pending_par_name_checks);
        for (at, name, span) in pending {
            if self.ref_touching_closures.contains(&at) {
                self.error_help(
                    span,
                    format!(
                        "`{name}` reaches a `ref` from outside itself, so it cannot run in \
parallel"
                    ),
                    "elements are processed on several cores, so two of them reaching the same \
slot would race — use `map` or `filter`, or build the reference inside the function, one per \
element",
                );
            }
        }
    }

    /// Applies each generic's recorded `ref`-in-parallel constraint to its
    /// call sites, propagating through generics that forward to generics.
    fn flush_par_checks(&mut self) {
        let pending = std::mem::take(&mut self.pending_par_checks);
        // Same fixpoint as the key checks, and for the same reason: a generic
        // that forwards to a generic makes its own parameter a parallel
        // parameter, and that is only knowable once the callee's is.
        loop {
            let mut changed = false;
            for (name, fresh, _, owner) in &pending {
                let (Some(indices), Some(owner)) =
                    (self.par_generics.get(name).cloned(), owner.clone())
                else {
                    continue;
                };
                for i in indices {
                    let Some(t) = fresh.get(i) else { continue };
                    if let Type::Generic(g) = self.uni.resolve(t) {
                        if self
                            .par_generics
                            .entry(owner.clone())
                            .or_default()
                            .insert(g as usize)
                        {
                            changed = true;
                        }
                    }
                }
            }
            if !changed {
                break;
            }
        }
        for (name, fresh, span, _) in pending {
            let Some(indices) = self.par_generics.get(&name).cloned() else {
                continue;
            };
            for i in indices {
                let Some(t) = fresh.get(i).cloned() else {
                    continue;
                };
                if matches!(self.uni.resolve(&t), Type::Generic(_)) {
                    continue;
                }
                if self.carries_ref(&t) {
                    let shown = self.show(&t);
                    self.error_help(
                        span,
                        format!(
                            "this call reaches a parallel operation with `{shown}`, which \
reaches a `ref` — every element would share it"
                        ),
                        "elements are processed on several cores, so two of them reaching the \
same slot would race — pass a type that holds no `ref`, or use `map` inside the callee",
                    );
                }
            }
        }
    }

    /// Applies each generic's recorded key constraints to its call sites.
    fn flush_key_checks(&mut self) {
        let pending = std::mem::take(&mut self.pending_key_checks);
        // A generic that forwards to a generic makes its *own* parameter a key
        // generic, and that is only knowable once the callee's is. Run to a
        // fixpoint first: `fn outer[T](xs: [T]) { inner(xs) }` over an
        // `inner[T]` holding a `Map[T, _]` let `outer([1.5])` through the
        // checker and into a runtime `invalid map key` — a well-typed program
        // panicking, which is what [T-MAP-KEY-GENERIC] exists to stop. One
        // hop was checked; two were not, and neither were three or four.
        loop {
            let mut changed = false;
            for (name, fresh, _, owner) in &pending {
                let (Some(indices), Some(owner)) =
                    (self.key_generics.get(name).cloned(), owner.clone())
                else {
                    continue;
                };
                for i in indices {
                    let Some(t) = fresh.get(i) else { continue };
                    if let Type::Generic(g) = self.uni.resolve(t) {
                        if self
                            .key_generics
                            .entry(owner.clone())
                            .or_default()
                            .insert(g as usize)
                        {
                            changed = true;
                        }
                    }
                }
            }
            if !changed {
                break;
            }
        }
        for (name, fresh, span, _) in pending {
            let Some(indices) = self.key_generics.get(&name).cloned() else {
                continue;
            };
            for i in indices {
                if let Some(t) = fresh.get(i) {
                    let t = t.clone();
                    self.require_map_key(&t, span);
                }
            }
        }
    }

    /// Map keys must be orderable primitives, or tuples of them
    /// ([T-MAP-KEY]).
    fn require_map_key(&mut self, k: &Type, span: Span) {
        if !self.map_key_ok(k) {
            let shown = self.show(k);
            self.error_help(
                span,
                format!(
                    "map keys must be an integer type, `str`, `bool` or a tuple of those, not `{shown}`"
                ),
                "use a key of those types, or a tuple of them for a key with several parts",
            );
        }
    }

    /// Whether `k` is a key type, noting each type parameter that is one --
    /// inside a tuple as much as alone -- so its call sites can be held to it
    /// ([T-MAP-KEY-GENERIC]).
    fn map_key_ok(&mut self, k: &Type) -> bool {
        let r = self.uni.resolve(k);
        if let Type::Tuple(items) = &r {
            let mut ok = !items.is_empty();
            for t in items.clone() {
                ok &= self.map_key_ok(&t);
            }
            return ok;
        }
        // A type *parameter* is allowed here and checked at each call site
        // instead ([T-MAP-KEY]). Without this a generic helper over maps —
        // `fn bump[K](inout m: Map[K, i64], k: K)`, the most repeated shape
        // there is — could not be declared at all, and there are no bounds to
        // declare it with.
        if let Type::Generic(i) = r {
            // Written inside a generic function's own body: remember which
            // parameter it was, so the call sites can be held to it.
            if let Some(owner) = self.key_generic_owner.clone() {
                self.key_generics
                    .entry(owner)
                    .or_default()
                    .insert(i as usize);
            }
        }
        matches!(
            r,
            Type::I64
                | Type::Int(_)
                | Type::Str
                | Type::Bool
                | Type::Var(_)
                | Type::Generic(_)
                | Type::Error
        )
    }

    /// The function argument of a parallel operation, checked knowing it runs
    /// on several cores at once.
    ///
    /// An effect is refused by [T-PAR-MAP-PURE] in `effects.rs`. A write
    /// through a captured `ref` is not an effect — mutation is not in the row
    /// system — so `xs.par_map(|x| { acc.n = acc.n + 1; x })` typechecked and
    /// every element's call wrote the same slot. Purity is what makes the
    /// parallelism deterministic, and that has to include the one write the
    /// closure rules deliberately allow ([T-REF-CAPTURE]).
    fn check_parallel_fn(&mut self, arg: &Expr, want: &Type) {
        // A closure bound to a name first. The purity proof looks at the
        // function written *at* the operation, and a name is not one — the
        // shape [T-PAR-MAP-PURE] listed as still reaching past the rule.
        if let Expr::Ident(id) = arg {
            if let Some(at) = self.let_closure_span.get(&id.name).copied() {
                self.pending_par_name_checks
                    .push((at, id.name.clone(), id.span));
            }
        }
        let outer = self.parallel_fn_floor.replace(self.scopes.len());
        self.check_expr(arg, want);
        self.parallel_fn_floor = outer;
    }

    /// Refuses a parallel operation whose *elements* reach a `ref`.
    ///
    /// [T-PAR-MAP-PURE] names this hole in as many words — "a list holding the
    /// same `ref` twice, where the write goes through a parameter rather than
    /// through a capture" — and says it is "not a race today" only because the
    /// native backend refused every write through a reference. It stopped
    /// refusing, so it is one: several workers reaching the same slot at once
    /// touch a reference count that is not atomic, and a program `check`
    /// accepted printed a different wrong answer on each run, or died with
    /// SIGTRAP, or panicked inside the runtime.
    ///
    /// Reading is no safer than writing here — a read through a reference
    /// takes a count on what it finds — so this refuses the element type
    /// outright rather than the operations on it. An element that *is* a
    /// reference, or that holds one anywhere inside, is refused; a reference
    /// created inside the function is one per element and is left alone.
    fn reject_parallel_ref_elem(&mut self, elem: &Type, name: &Ident, span: Span) {
        // A type *parameter* here says nothing yet; the call sites are where
        // it becomes a type, and that is where it is checked
        // ([T-MAP-KEY-GENERIC] settles map keys the same way). Without this a
        // generic wrapper was a door straight back to the race: `go(rs, hit)`
        // for `fn go[T](xs: [T], f: fn(T) -> i64) { xs.par_map(f) }`.
        if let Type::Generic(i) = self.uni.resolve(elem) {
            if let Some(owner) = self.key_generic_owner.clone() {
                self.par_generics
                    .entry(owner)
                    .or_default()
                    .insert(i as usize);
            }
            return;
        }
        if !self.carries_ref(elem) {
            return;
        }
        let shown = self.show(elem);
        self.error_help(
            span,
            format!(
                "`{}` cannot process `{shown}` — it reaches a `ref`, which every \
element would share",
                name.name
            ),
            "elements are processed on several cores, so two of them reaching the same slot \
would race — which is what this operation exists to make impossible. Use `map` or `filter` \
for work over shared state, or build the references inside the function, one per element",
        );
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
        let declared_at = self.scopes.iter().rposition(|sc| sc.contains_key(&id.name));
        if let Some(b) = self.lookup(&id.name) {
            if matches!(self.uni.resolve(&b.ty), Type::Ref(_)) {
                // …except inside a parallel operation's function, where the
                // exemption is the race. Every element's call holds the same
                // slot, so the write interleaves differently between runs —
                // which is exactly what [T-PAR-MAP-PURE] promises cannot
                // happen. A `ref` created *inside* the function is one per
                // element and is left alone.
                if let (Some(floor), Some(at)) = (self.parallel_fn_floor, declared_at) {
                    if at < floor {
                        self.error_help(
                            id.span,
                            format!(
                                "cannot {verb} `{}`{tail} inside a parallel operation — \
it is a `ref` from outside, so every element writes the same slot",
                                id.name
                            ),
                            "elements are processed on several cores, so this write would \
interleave differently between runs — use `map` for work that shares state, or return a \
value per element and combine the results afterwards",
                        );
                    }
                }
                return;
            }
        }
        // A task's own floor: `reject_task_write` says so, and this advice --
        // hold it behind a `ref` -- is what a task may not do.
        if self.go_floor == Some(floor) {
            return;
        }
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
            // A mutating method on a tuple component is the same write
            // `check_assign_target` refuses, spelled differently — and the
            // two engines disagreed about it: `t.0.push(5)` mutated the
            // tuple under `grasm run` and was refused by the backend.
            if let Expr::Field { name: f, span, .. } = root {
                if f.name.chars().all(|c| c.is_ascii_digit()) {
                    self.error_help(
                        *span,
                        format!("cannot use `{}` on a tuple component", name.name),
                        "a tuple is a fixed group of values with no place to write into — \
bind the component to a `var`, or use a struct when the parts have names worth writing",
                    );
                    return;
                }
            }
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
            Type::I64 | Type::Int(_) | Type::F64 | Type::Str | Type::Var(_) | Type::Error => true,
            Type::Tuple(parts) => !parts.is_empty() && parts.iter().all(|p| self.is_ordered(p)),
            _ => false,
        }
    }

    /// As `float_aware_lhs`, and says whether the right operand was already
    /// inferred — so the caller does not walk it a second time.
    ///
    /// Walking it twice made a right-nested chain exponential: `2^2^…^2`
    /// thirty terms long doubled the work per term, and `check` never
    /// finished on a program `[G-NEST-LIMIT]` promises a diagnostic for.
    /// A literal written beside a fixed-width integer takes its type, if it
    /// fits: `b + 1` and `1 + b` are both `u8` for `b: u8` ([T-INT-FIXED]).
    fn literal_adopts(&mut self, lit: &Expr, k: IntKind) {
        let (v, sp) = match lit {
            Expr::Int(v, sp) => (self.lit_value(k, *v, *sp, false), *sp),
            Expr::Unary {
                op: UnaryOp::Neg,
                expr,
                span,
            } => match **expr {
                Expr::Int(v, isp) => (self.lit_value(k, v, isp, true), *span),
                _ => return,
            },
            _ => return,
        };
        if !k.fits(v) {
            self.literal_out_of_range(sp, v, k);
        }
        self.resolved_types.insert((sp.start, sp.end), Type::Int(k));
    }

    /// The number a literal stands for against `k`: [`literal_value`], except
    /// that a decimal literal past `i64` is the large number it was written
    /// as whatever the width ([T-U64]).
    fn lit_value(&self, k: IntKind, v: i64, at: Span, negated: bool) -> i128 {
        if self.wide_ints.contains(&at.start) {
            let n = i128::from(v as u64);
            return if negated { -n } else { n };
        }
        literal_value(k, v, negated)
    }

    /// A decimal literal past `i64`, somewhere other than a `u64`.
    fn wide_literal(&mut self, at: Span, ty: &str) {
        self.error_help(
            at,
            format!("this literal is too large for `{ty}`"),
            "only a `u64` holds a number past 9223372036854775807; write it where a `u64` is expected -- an annotation, a parameter, a field, or beside another `u64` ([T-U64])",
        );
    }

    /// Whether `e` is a decimal literal past `i64`, or its negation.
    fn is_wide_literal(&self, e: &Expr) -> bool {
        match e {
            Expr::Int(_, sp) => self.wide_ints.contains(&sp.start),
            Expr::Unary {
                op: UnaryOp::Neg,
                expr,
                ..
            } => self.is_wide_literal(expr),
            _ => false,
        }
    }

    fn literal_out_of_range(&mut self, span: Span, v: i128, k: IntKind) {
        self.error_help(
            span,
            format!("`{v}` does not fit in `{}`", k.name()),
            format!(
                "`{}` holds {} to {}; a value outside that is not written as a literal of it ([{}])",
                k.name(),
                k.min(),
                k.max(),
                if k == IntKind::U64 { "T-U64" } else { "T-INT-FIXED" }
            ),
        );
    }

    fn float_aware_lhs_seen(&mut self, lhs: &Expr, rhs: &Expr) -> (Type, Option<Type>) {
        // `18446744073709551615 - x` for a `u64` x: the literal has no type
        // of its own to infer, so the other side is asked first ([T-U64]).
        if self.is_wide_literal(lhs) {
            let probe = self.infer_expr(rhs);
            if let Type::Int(k) = self.uni.shallow_resolve(&probe) {
                self.literal_adopts(lhs, k);
                return (Type::Int(k), Some(probe));
            }
            let lhs_ty = self.infer_expr(lhs);
            return (lhs_ty, Some(probe));
        }
        let lhs_ty = self.infer_expr(lhs);
        let literal_span = match lhs {
            Expr::Int(..) => Some(lhs.span().start),
            Expr::Unary {
                op: UnaryOp::Neg,
                expr,
                ..
            } if matches!(**expr, Expr::Int(..)) => Some(expr.span().start),
            _ => None,
        };
        if let Some(at) = literal_span {
            if self.uni.shallow_resolve(&lhs_ty) == Type::I64 {
                let probe = self.infer_expr(rhs);
                if self.uni.shallow_resolve(&probe) == Type::F64 {
                    self.float_literals.insert(at);
                    return (Type::F64, Some(probe));
                }
                if let Type::Int(k) = self.uni.shallow_resolve(&probe) {
                    self.literal_adopts(lhs, k);
                    return (Type::Int(k), Some(probe));
                }
                return (lhs_ty, Some(probe));
            }
        }
        (lhs_ty, None)
    }

    fn float_aware_lhs(&mut self, lhs: &Expr, rhs: &Expr) -> Type {
        if self.is_wide_literal(lhs) {
            let (t, _) = self.float_aware_lhs_seen(lhs, rhs);
            return t;
        }
        let lhs_ty = self.infer_expr(lhs);
        // A negated literal is a literal: `-3` is `Neg` applied to `3`, and
        // matching only the bare shape meant `2.5 * -3` worked while
        // `-3 * 2.5` was a type error — with help that said to "use a
        // literal, which does adopt the other side" about one
        // ([E-NUM-PROMOTE]: "on either side").
        let literal_span = match lhs {
            Expr::Int(..) => Some(lhs.span().start),
            Expr::Unary {
                op: UnaryOp::Neg,
                expr,
                ..
            } if matches!(**expr, Expr::Int(..)) => Some(expr.span().start),
            _ => None,
        };
        if let Some(at) = literal_span {
            if self.uni.shallow_resolve(&lhs_ty) == Type::I64 {
                let probe = self.infer_expr(rhs);
                if self.uni.shallow_resolve(&probe) == Type::F64 {
                    self.float_literals.insert(at);
                    return Type::F64;
                }
                if let Type::Int(k) = self.uni.shallow_resolve(&probe) {
                    self.literal_adopts(lhs, k);
                    return Type::Int(k);
                }
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
                    self.error(
                        span,
                        format!("cannot compare `{shown}` values with `<`/`>`"),
                    );
                }
                Type::Bool
            }
            Coalesce => {
                let lhs_ty = self.infer_expr(lhs);
                // A call to a `let`-bound closure has the closure's body's
                // type, and those bodies wait for their parameter types.
                // Here the call itself has just supplied them, so the body
                // can be checked now — without this, `f(1) ?? 0` for
                // `let f = |x| risky(x)` was refused for a left side whose
                // type the checker was about to work out.
                self.force_deferred(&lhs_ty);
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
                let (lhs_ty, rhs_seen) = self.float_aware_lhs_seen(lhs, rhs);
                match rhs_seen {
                    // Already walked, and walking it again is what made a
                    // right-nested arithmetic chain exponential.
                    Some(rt) => self.unify_or_report(rhs.span(), &rt, &lhs_ty),
                    None => self.check_expr(rhs, &lhs_ty),
                }
                let r = self.uni.shallow_resolve(&lhs_ty);
                match r {
                    Type::I64 | Type::F64 | Type::Int(_) => r,
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
        // As in `??`: a call to a `let`-bound closure has just fixed that
        // closure's parameter types, so its body can be checked now — and
        // `try` needs the shape of the answer, not just to unify with it.
        self.force_deferred(&t);
        match self.uni.shallow_resolve(&t) {
            Type::Fallible(ok, err) => {
                match self.fn_err.clone() {
                    Some(fn_err) => {
                        if !self.err_fits(&err, &fn_err) {
                            let e = self.show(&err);
                            let fe = self.show(&fn_err);
                            self.error_help(
                                span,
                                format!(
                                    "`try` propagates `{e}` but this function only declares `{fe}`"
                                ),
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
        // The patterns are matched against the *shape* of this type, so a
        // closure call's deferred body has to be checked first.
        self.force_deferred(&scrut_ty);
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
        let mut deferred_literals: Vec<&Expr> = Vec::new();
        for arm in arms {
            // Each arm starts from the state before the match: at most one of
            // them runs, so a move in an earlier arm has not happened here.
            self.restore_moved(&before);
            self.scopes.push(HashMap::new());
            // Closures deferred by this arm belong to *this* scope, where the
            // arm's pattern bindings live. Left on the enclosing block's list
            // they were checked after this scope was popped, so
            // `match o { Some(n) -> |x| x + n  ... }` reported "unknown name
            // `n`" about the binding two tokens to its left, and a correct
            // program had no spelling ([E-CLOSURE-CAPTURE]).
            //
            // The same defect, in the same words, was already fixed twice: once
            // for a closure nested in a closure body, once for a block. A match
            // arm is the third place that opens a scope, and it was the one
            // that never got a frame of its own.
            self.deferred_closures.push(Vec::new());
            self.deferred_fields.push(Vec::new());
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
            // Integer literals last, as in a list or a map's values: whatever
            // fixes the type does it first, so `match n { 1 -> 2  _ -> 3.5 }`
            // and the same two arms the other way round both produce `f64`.
            // Checked in order, the first arm decided for all of them — the
            // asymmetry [T-INT-TO-FLOAT-CARRIED] says is fixed, and was, for
            // every carrier except this one.
            if Self::defers_to_its_context(&arm.body) {
                deferred_literals.push(&arm.body);
            } else {
                // One arm may build the whole `T ! E` while another gives the
                // ok half, which a fallible body may do either way
                // ([T-OK-ERR-RETURN]). The `if` spelling of the same two
                // branches was accepted and this one was not.
                let arm_ty = self.infer_expr(&arm.body);
                if !self.uni.unify(&arm_ty, &result) {
                    match self.merge_fallible_branches(&arm_ty, &result) {
                        Some(t) => {
                            // The arm that gave the bare half is the one
                            // wrapped; the other already built the whole.
                            if !matches!(self.uni.shallow_resolve(&arm_ty), Type::Fallible(..)) {
                                self.ok_wrapped
                                    .insert((arm.body.span().start, arm.body.span().end));
                                if self.record_tir {
                                    self.tir.wraps.insert(
                                        (arm.body.span().start, arm.body.span().end),
                                        t.clone(),
                                    );
                                }
                            }
                            let _ = self.uni.unify(&result, &t);
                        }
                        None => {
                            // The arm may still fit once the expected type
                            // reaches the literals *inside* it. Inference
                            // alone gave `(3.5, 4)` the type `(f64, i64)`, so
                            // a mixed tuple in a `match` arm was refused —
                            // while the same tuple in every other position,
                            // and a mixed *list* in the same arm, promoted
                            // ([T-INT-TO-FLOAT-CARRIED] names both "a list,
                            // tuple or map literal" and "a `match`'s arms").
                            let mark = self.diags.len();
                            self.check_expr(&arm.body, &result);
                            if self.diags.len() > mark {
                                self.diags.truncate(mark);
                                self.unify_or_report(arm.body.span(), &arm_ty, &result);
                            }
                        }
                    }
                }
            }
            // Drained before the scope goes, as in a block and in a closure
            // body. Checking one deferred body can defer another, so this is a
            // loop rather than a pass.
            while let Some(next) = self.deferred_closures.last_mut().and_then(|v| v.pop()) {
                self.check_deferred_closure(next);
            }
            self.deferred_closures.pop();
            self.settle_deferred_fields();
            self.scopes.pop();
            arm_states.push(self.moved_snapshot());
        }
        // A body that *builds* the whole `T ! E` decides the match's type; a
        // bare one adopts whatever that turns out to be. Both defer — `Ok(2)`
        // is a carrier of literals like `[2]` is — so with every arm deferred
        // the bare one went first, fixed the result to the ok half, and the
        // `Ok` arm was then refused against it. Only nesting reached this:
        // written directly at a fallible position the result is seeded from
        // the outside and there is nothing left to decide.
        deferred_literals.sort_by_key(|body| {
            !matches!(body, Expr::Call { callee, .. }
                if matches!(&**callee, Expr::Ident(id)
                    if matches!(id.name.as_str(), "Ok" | "Err")))
        });
        for body in deferred_literals {
            // As above: a literal arm beside an `Err(e)` arm is the ok half —
            // unless it *is* the whole fallible, which `Ok(3)` is. A body may
            // give either ([T-OK-ERR-RETURN]), and taking the ok half for both
            // reported "this expression can fail but `f64` is required" about
            // an arm whose whole job is to say it did not.
            let r = self.uni.shallow_resolve(&result);
            if let Type::Fallible(ok, _) = &r {
                let whole = matches!(body, Expr::Call { callee, .. }
                    if matches!(&**callee, Expr::Ident(id)
                        if matches!(id.name.as_str(), "Ok" | "Err")));
                if !whole {
                    let ok = (**ok).clone();
                    // Accepted as the bare half, so its value is wrapped where
                    // it lands ([T-OK-ERR-RETURN]).
                    self.ok_wrapped.insert((body.span().start, body.span().end));
                    if self.record_tir {
                        self.tir
                            .wraps
                            .insert((body.span().start, body.span().end), r.clone());
                    }
                    self.check_expr(body, &ok);
                    continue;
                }
            }
            self.check_expr(body, &result);
        }
        self.restore_moved(&before);
        let voters = arm_states
            .into_iter()
            .zip(arms.iter().map(|a| expr_leaves(&a.body)))
            .collect();
        self.merge_branches(voters, span);
        // Exhaustiveness.
        if !has_irrefutable {
            let missing = self.missing_for(&scrut_ty, &covered);
            let missing: Vec<String> = missing
                .into_iter()
                .filter(|m| !covered.contains(m))
                .collect();
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
                    && members.iter().all(|m| self.uni.resolve(m) == Type::Bool) =>
            {
                let n = members.len();
                let mut missing = Vec::new();
                for bits in 0..(1u32 << n) {
                    let combo: Vec<&str> = (0..n)
                        .map(|i| {
                            if bits & (1 << i) != 0 {
                                "true"
                            } else {
                                "false"
                            }
                        })
                        .collect();
                    let is_covered = covered.iter().any(|tag| {
                        tag.strip_prefix('(')
                            .and_then(|t| t.strip_suffix(')'))
                            .map(|inner| {
                                let parts: Vec<&str> = inner.split(", ").collect();
                                parts.len() == n
                                    && parts
                                        .iter()
                                        .zip(&combo)
                                        .all(|(p, want)| *p == "_" || p == want)
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
        missing
            .into_iter()
            .filter(|m| !covered.contains(m))
            .collect()
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
            Pattern::Int(_, span) if self.wide_ints.contains(&span.start) => {
                self.error_help(
                    *span,
                    "a pattern names a value from 0 to 9223372036854775807",
                    "a `u64` past that is matched with a guard: `n if n == 18446744073709551615` ([T-U64])",
                );
                Some("<int>".into())
            }
            Pattern::Int(v, span) => {
                if let Type::Int(k) = self.uni.shallow_resolve(expected) {
                    // A pattern's literal arrives already negated, so a
                    // `u64` pattern is read as the `i64` it was written as:
                    // `-1` is refused rather than taken for 2^64 - 1, and a
                    // value from 2^63 up is matched in a guard ([T-U64]).
                    if k == IntKind::U64 && *v < 0 {
                        self.error_help(
                            *span,
                            "a pattern on a `u64` names a value from 0 to 9223372036854775807",
                            "a negative number is no `u64`, and one from 2^63 up is matched with a guard: `n if n == 0xFFFF_FFFF_FFFF_FFFF` ([T-U64])",
                        );
                    } else if !k.fits(i128::from(*v)) {
                        self.literal_out_of_range(*span, i128::from(*v), k);
                    }
                    return Some("<int>".into());
                }
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
                    self.error(
                        *span,
                        "struct values are matched with bindings, not variants",
                    );
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
                params: want_params,
                ..
            }) if want_params.len() == params.len() => want_params.clone(),
            _ => params.iter().map(|_| self.uni.fresh()).collect(),
        };
        // Keyed by the closure and by its body, for the same reason the return
        // type is: the backend identifies a closure by where its *body* starts,
        // and one with parameters begins earlier than that.
        self.closure_params.push((span.start, param_tys.clone()));
        self.closure_params
            .push((body.span().start, param_tys.clone()));
        if let Some(Type::Fn {
            params: want_params,
            ..
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
                    at: span.start,
                });
                self.closure_types.insert(span.start, ret.clone());
                // Keyed by the body too. The backend identifies a closure by
                // where its *body* starts, and a closure with parameters
                // begins earlier than that — so the guard that refuses `fail`
                // inside a `map` closure never matched one, and the `fail`
                // was compiled as a return from the enclosing function:
                // `main` reported an unhandled error where the interpreter
                // produced an `Err` element.
                self.closure_types.insert(body.span().start, ret.clone());
                // As below: the value carries what its body performs, even
                // when the body itself waits for a call site.
                return Type::Fn {
                    params: param_tys,
                    ret: Box::new(ret),
                    row: self.closure_rows.get(&span.start).cloned(),
                };
            }
        }
        self.scopes.push(HashMap::new());
        self.closure_floors.push(self.scopes.len() - 1);
        // A `go` must be lexically inside its block, not inside something
        // that could run after the block has ended.
        let saved_conc = std::mem::replace(&mut self.concurrent_depth, 0);
        // Per closure: an inner one reaching a `ref` says nothing about the
        // one around it.
        let saved_touch = std::mem::replace(&mut self.closure_touches_outside_ref, false);
        for (p, t) in params.iter().zip(&param_tys) {
            self.bind(p, t.clone(), false);
        }
        // Closures deferred by this body belong to this scope, where this
        // closure's parameters are bound. On the enclosing block's list they
        // were checked after it was gone, so an annotated
        // `|n| |m| n + m` reported "unknown name `n`" ([E-CLOSURE-CAPTURE]).
        self.deferred_closures.push(Vec::new());
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
        // As above: the closure's own return type is what `return` means
        // here, and a loop outside it is out of reach.
        let closure_ret = match &want_ret {
            Some(Type::Fallible(ok, _)) => (**ok).clone(),
            Some(t) => t.clone(),
            None => self.uni.fresh(),
        };
        let saved_fn_ret = std::mem::replace(&mut self.fn_ret, closure_ret);
        let saved_loop = std::mem::replace(&mut self.loop_depth, 0);
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
            // A closure that is not fallible has no error branch, so a `fail`
            // inside it belongs to nobody: charged to the enclosing function
            // instead, `Box{ f: |x| if c { fail(e) } else { x } }` built in a
            // fallible function and *called* from one that declares no error
            // type let the failure out of `main` at run time.
            if !self.hof_closures.contains(&span.start) {
                self.fn_err = None;
            }
            // Checked against the written result type, not merely inferred:
            // "a closure literal is checked the same way a named function is"
            // ([T-FN-TYPE-ROW]), and a named function's body *is* checked
            // against its declared return. Inferred instead, the expected type
            // never reached the literals in the tail, so `|x| 3` against
            // `fn(i64) -> f64` was refused while `fn named(x: i64) -> f64 { 3 }`
            // — the same body, one closure apart — was accepted, and so was
            // `|x| { return 3 }`. That is the asymmetry
            // [T-INT-TO-FLOAT-CARRIED] records as fixed for named functions,
            // still present here.
            match want_ret
                .clone()
                .filter(|t| !matches!(self.uni.shallow_resolve(t), Type::Var(_) | Type::Error))
            {
                Some(want) => {
                    let mark = self.diags.len();
                    self.check_expr(body, &want);
                    if self.diags.len() > mark {
                        // Not a fit: report it the way the caller would, from
                        // the type the body actually has.
                        //
                        // Only when re-inferring actually says something. It is
                        // memoized, so a second visit to the same expression
                        // answers from the cache and reports nothing — and this
                        // threw the first pass's diagnostics away regardless.
                        // An unknown name inside a closure with an expected
                        // type was reported, discarded, and never reported
                        // again: `let f: fn(i64) -> () = |v| nosuchname(v)`
                        // passed `grasm check`, panicked under `grasm run`, and
                        // was refused by `--native` as a backend limitation
                        // (F44).
                        let checked = self.diags[mark..].to_vec();
                        self.diags.truncate(mark);
                        let inferred = self.infer_expr(body);
                        if self.diags.len() == mark {
                            self.diags.extend(checked);
                        }
                        inferred
                    } else {
                        want
                    }
                }
                None => self.infer_expr(body),
            }
        };
        self.fn_err = saved_err;
        self.fn_ret = saved_fn_ret;
        self.loop_depth = saved_loop;
        while let Some(next) = self.deferred_closures.last_mut().and_then(|v| v.pop()) {
            self.check_deferred_closure(next);
        }
        self.deferred_closures.pop();
        self.concurrent_depth = saved_conc;
        self.closure_floors.pop();
        self.scopes.pop();
        if self.closure_touches_outside_ref {
            self.ref_touching_closures.insert(span.start);
        }
        self.closure_touches_outside_ref = saved_touch;
        self.closure_types.insert(span.start, ret.clone());
        self.closure_types.insert(body.span().start, ret.clone());
        Type::Fn {
            params: param_tys,
            ret: Box::new(ret),
            // What this closure body actually performs, so the value carries
            // it wherever it goes. `None` means *unconstrained*, which is
            // right for the built-in higher-order methods and wrong for the
            // value itself: one unannotated `let` between the literal and its
            // destination erased the row, so `let c = |m| print(m)` then
            // `Box{ f: c }` put a printing closure in a written `fn(str)` — a
            // declared empty row ([T-FN-TYPE-ROW]) — while the literal written
            // in place was refused. The same `let` carried a handled effect
            // out of its `with` block through a `push`, reaching the
            // missing-handler panic [E-HANDLER-MISSING] calls a backstop that
            // well-typed programs never reach.
            row: self.closure_rows.get(&span.start).cloned(),
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

/// Whether a tail expression can only produce a fallible function's *ok* half
/// — that is, whether every branch of it is an ordinary value rather than an
/// `Ok(..)`/`Err(..)` that builds the whole `T ! E`.
fn yields_ok_half(e: &Expr) -> bool {
    fn block_tail(b: &Block) -> Option<&Expr> {
        match b.stmts.last() {
            Some(Stmt::Expr(e)) => Some(e),
            _ => None,
        }
    }
    fn ok(e: &Expr) -> bool {
        match e {
            Expr::Call { callee, .. } => !matches!(&**callee, Expr::Ident(id)
                if id.name == "Ok" || id.name == "Err"),
            Expr::If {
                then_block,
                else_branch,
                ..
            } => {
                let t = block_tail(then_block).map(ok).unwrap_or(true);
                let f = match else_branch.as_deref() {
                    Some(ElseBranch::Else(b)) => block_tail(b).map(ok).unwrap_or(true),
                    Some(ElseBranch::ElseIf(inner)) => ok(inner),
                    None => true,
                };
                t && f
            }
            Expr::Match { arms, .. } => arms.iter().all(|a| ok(&a.body)),
            _ => true,
        }
    }
    matches!(e, Expr::If { .. } | Expr::Match { .. }) && ok(e)
}

/// Whether every branch of a tail `if`/`match` builds the whole `T ! E`.
/// Whether this expression builds the whole `T ! E` rather than the ok half.
fn yields_whole_expr(e: &Expr) -> bool {
    match e {
        Expr::Call { callee, .. } => matches!(&**callee, Expr::Ident(id)
            if id.name == "Ok" || id.name == "Err" || id.name == "fail"),
        Expr::If { .. } | Expr::Match { .. } => yields_whole(e),
        Expr::Try { .. } => false,
        // A branch whose tail is a block is worth what the block is worth.
        Expr::Block(b) => match b.stmts.last() {
            Some(Stmt::Expr(inner)) => yields_whole_expr(inner),
            _ => false,
        },
        _ => false,
    }
}

/// Whether *some* branch inside `e` builds the whole `T ! E`.
///
/// `yields_whole` asks whether every branch does, which is the question for
/// "is this expression already the whole thing". This is the question for
/// "does the expected type have to reach it as the whole thing": an `if` or
/// `match` that mixes the halves merges to the whole, and each of its own
/// branches may then give either ([T-OK-ERR-RETURN]). Asked the first
/// question, a *nested* mixing branch was handed the ok half and its `Ok(2)`
/// was refused — while the same computation with the two constructs swapped
/// was accepted, one keyword apart.
fn yields_some_whole(e: &Expr) -> bool {
    fn tail(b: &Block) -> Option<&Expr> {
        match b.stmts.last() {
            Some(Stmt::Expr(e)) => Some(e),
            _ => None,
        }
    }
    match e {
        Expr::Call { callee, .. } => matches!(&**callee, Expr::Ident(id)
            if id.name == "Ok" || id.name == "Err" || id.name == "fail"),
        Expr::If {
            then_block,
            else_branch,
            ..
        } => {
            tail(then_block).is_some_and(yields_some_whole)
                || match else_branch.as_deref() {
                    Some(ElseBranch::Else(b)) => tail(b).is_some_and(yields_some_whole),
                    Some(ElseBranch::ElseIf(inner)) => yields_some_whole(inner),
                    None => false,
                }
        }
        Expr::Match { arms, .. } => arms.iter().any(|a| yields_some_whole(&a.body)),
        Expr::Block(b) => tail(b).is_some_and(yields_some_whole),
        _ => false,
    }
}

fn yields_whole(e: &Expr) -> bool {
    fn block_tail(b: &Block) -> Option<&Expr> {
        match b.stmts.last() {
            Some(Stmt::Expr(e)) => Some(e),
            _ => None,
        }
    }
    fn whole(e: &Expr) -> bool {
        match e {
            Expr::Call { callee, .. } => matches!(&**callee, Expr::Ident(id)
                if id.name == "Ok" || id.name == "Err" || id.name == "fail"),
            Expr::If {
                then_block,
                else_branch,
                ..
            } => {
                let t = block_tail(then_block).map(whole).unwrap_or(false);
                let f = match else_branch.as_deref() {
                    Some(ElseBranch::Else(b)) => block_tail(b).map(whole).unwrap_or(false),
                    Some(ElseBranch::ElseIf(inner)) => whole(inner),
                    None => false,
                };
                t && f
            }
            Expr::Match { arms, .. } => arms.iter().all(|a| whole(&a.body)),
            _ => false,
        }
    }
    matches!(e, Expr::If { .. } | Expr::Match { .. }) && whole(e)
}

/// Closure literals passed directly to a built-in that calls them at once.
fn collect_hof_closures_block(b: &Block, out: &mut std::collections::HashSet<u32>) {
    for stmt in &b.stmts {
        walk_stmt_exprs(stmt, &mut |e| collect_hof_closures(e, out));
    }
}

fn collect_hof_closures(e: &Expr, out: &mut std::collections::HashSet<u32>) {
    if let Expr::MethodCall { name, args, .. } = e {
        if matches!(
            name.name.as_str(),
            "map"
                | "par_map"
                | "filter"
                | "par_filter"
                | "fold"
                | "any"
                | "all"
                | "find"
                | "sort_by"
                | "each"
        ) {
            for a in args {
                if let Expr::Closure { span, .. } = a {
                    out.insert(span.start);
                }
            }
        }
    }
}

/// Applies `f` to every expression a statement contains, recursively.
fn walk_stmt_exprs(s: &Stmt, f: &mut dyn FnMut(&Expr)) {
    let walk_expr = |e: &Expr, f: &mut dyn FnMut(&Expr)| walk_exprs(e, f);
    match s {
        Stmt::Let { init, .. } => walk_expr(init, f),
        Stmt::LetTuple { init, .. } => walk_expr(init, f),
        Stmt::Assign { target, value, .. } => {
            walk_expr(target, f);
            walk_expr(value, f);
        }
        Stmt::Return(Some(e), _) | Stmt::Expr(e) => walk_expr(e, f),
        Stmt::While { cond, body, .. } => {
            walk_expr(cond, f);
            for st in &body.stmts {
                walk_stmt_exprs(st, f);
            }
        }
        Stmt::For { iter, body, .. } => {
            walk_expr(iter, f);
            for st in &body.stmts {
                walk_stmt_exprs(st, f);
            }
        }
        Stmt::With { handler, body, .. } => {
            walk_expr(handler, f);
            for st in &body.stmts {
                walk_stmt_exprs(st, f);
            }
        }
        Stmt::Parallel { body, .. } | Stmt::Spawn { body, .. } => {
            for st in &body.stmts {
                walk_stmt_exprs(st, f);
            }
        }
        _ => {}
    }
}

/// Every name a block might change: assigned through (`x = v`, `x.f = v`,
/// `x[i] = v`), rebound by a `let`, the receiver of any method call, or a
/// bare argument (which could be `inout`). Deliberately generous: a name not
/// in it is certainly left alone for the whole block.
pub fn possibly_mutated(
    b: &Block,
    reads_only: &dyn Fn(&str) -> bool,
) -> std::collections::HashSet<String> {
    fn root(e: &Expr) -> Option<&str> {
        match e {
            Expr::Ident(id) => Some(&id.name),
            Expr::Field { recv, .. } | Expr::Index { recv, .. } => root(recv),
            _ => None,
        }
    }
    fn block(b: &Block, out: &mut std::collections::HashSet<String>, ro: &dyn Fn(&str) -> bool) {
        for st in &b.stmts {
            match st {
                Stmt::Assign { target, .. } => {
                    if let Some(r) = root(target) {
                        out.insert(r.to_string());
                    }
                }
                Stmt::Let { name, .. } => {
                    out.insert(name.name.clone());
                }
                Stmt::LetTuple { names, .. } => {
                    for n in names {
                        out.insert(n.name.clone());
                    }
                }
                Stmt::While { body, .. }
                | Stmt::For { body, .. }
                | Stmt::Parallel { body, .. }
                | Stmt::Spawn { body, .. }
                | Stmt::With { body, .. } => block(body, out, ro),
                _ => {}
            }
            walk_stmt_exprs(st, &mut |e| expr(e, out, ro));
        }
    }
    fn expr(e: &Expr, out: &mut std::collections::HashSet<String>, ro: &dyn Fn(&str) -> bool) {
        match e {
            // A method the caller vouches reads only (`xs.len()`) changes
            // nothing; any other may.
            Expr::MethodCall { recv, name, .. } => {
                if !ro(&name.name) {
                    if let Some(r) = root(recv) {
                        out.insert(r.to_string());
                    }
                }
            }
            Expr::Call { args, .. } => {
                for a in args {
                    if let Some(r) = root(a) {
                        out.insert(r.to_string());
                    }
                }
            }
            Expr::Block(b) | Expr::Unsafe(b) | Expr::Region(b) => block(b, out, ro),
            Expr::If {
                then_block,
                else_branch,
                ..
            } => {
                block(then_block, out, ro);
                if let Some(ElseBranch::Else(b)) = else_branch.as_deref() {
                    block(b, out, ro);
                }
            }
            _ => {}
        }
    }
    let mut out = std::collections::HashSet::new();
    block(b, &mut out, reads_only);
    out
}

/// Whether a block can leave before its end: a `break` or `return` anywhere
/// in it, nested blocks included.
pub fn block_can_exit(b: &Block) -> bool {
    fn block(b: &Block) -> bool {
        b.stmts.iter().any(|st| match st {
            Stmt::Break(_) | Stmt::Return(..) => true,
            Stmt::While { body, .. }
            | Stmt::For { body, .. }
            | Stmt::Parallel { body, .. }
            | Stmt::Spawn { body, .. }
            | Stmt::With { body, .. } => block(body) || stmt_exprs_exit(st),
            _ => stmt_exprs_exit(st),
        })
    }
    fn stmt_exprs_exit(st: &Stmt) -> bool {
        let mut found = false;
        walk_stmt_exprs(st, &mut |e| match e {
            Expr::Block(b) | Expr::Unsafe(b) | Expr::Region(b) => found |= block(b),
            Expr::If {
                then_block,
                else_branch,
                ..
            } => {
                found |= block(then_block);
                if let Some(ElseBranch::Else(b)) = else_branch.as_deref() {
                    found |= block(b);
                }
            }
            _ => {}
        });
        found
    }
    block(b)
}

/// Every float literal written anywhere in a block.
pub fn block_float_literals(b: &Block) -> Vec<f64> {
    let mut out = Vec::new();
    for st in &b.stmts {
        walk_stmt_exprs(st, &mut |x| {
            if let Expr::Float(v, _) = x {
                out.push(*v);
            }
        });
    }
    out
}

/// Whether `e` names the variable `name` anywhere inside it.
pub fn expr_mentions(e: &Expr, name: &str) -> bool {
    let mut found = false;
    walk_exprs(e, &mut |x| {
        if let Expr::Ident(id) = x {
            found |= id.name == name;
        }
    });
    found
}

fn walk_exprs(e: &Expr, f: &mut dyn FnMut(&Expr)) {
    f(e);
    match e {
        Expr::Unary { expr, .. } | Expr::Try { expr, .. } => walk_exprs(expr, f),
        Expr::Binary { lhs, rhs, .. } => {
            walk_exprs(lhs, f);
            walk_exprs(rhs, f);
        }
        Expr::Field { recv, .. } => walk_exprs(recv, f),
        Expr::Index { recv, index, .. } => {
            walk_exprs(recv, f);
            walk_exprs(index, f);
        }
        Expr::Call { callee, args, .. } => {
            walk_exprs(callee, f);
            for a in args {
                walk_exprs(a, f);
            }
        }
        Expr::MethodCall { recv, args, .. } => {
            walk_exprs(recv, f);
            for a in args {
                walk_exprs(a, f);
            }
        }
        Expr::List(items, _) | Expr::Tuple(items, _) => {
            for i in items {
                walk_exprs(i, f);
            }
        }
        Expr::Map(entries, _) => {
            for (k, v) in entries {
                walk_exprs(k, f);
                walk_exprs(v, f);
            }
        }
        Expr::Str(pieces, _) => {
            for p in pieces {
                if let StrPiece::Expr(inner) = p {
                    walk_exprs(inner, f);
                }
            }
        }
        Expr::Closure { body, .. } => walk_exprs(body, f),
        Expr::StructLit { fields, base, .. } => {
            if let Some(b) = base {
                walk_exprs(b, f);
            }
            for (_, v) in fields {
                walk_exprs(v, f);
            }
        }
        Expr::Block(b) | Expr::Unsafe(b) | Expr::Region(b) => {
            for st in &b.stmts {
                walk_stmt_exprs(st, f);
            }
        }
        Expr::If {
            cond,
            then_block,
            else_branch,
            ..
        } => {
            walk_exprs(cond, f);
            for st in &then_block.stmts {
                walk_stmt_exprs(st, f);
            }
            match else_branch.as_deref() {
                Some(ElseBranch::Else(b)) => {
                    for st in &b.stmts {
                        walk_stmt_exprs(st, f);
                    }
                }
                Some(ElseBranch::ElseIf(inner)) => walk_exprs(inner, f),
                None => {}
            }
        }
        Expr::Match {
            scrutinee, arms, ..
        } => {
            walk_exprs(scrutinee, f);
            for a in arms {
                walk_exprs(&a.body, f);
                if let Some(g) = &a.guard {
                    walk_exprs(g, f);
                }
            }
        }
        _ => {}
    }
}

/// One step of a place path. An index is a wildcard: `xs[i]` and `xs[j]` may
/// be the same element and this pass cannot know.
#[derive(PartialEq)]
enum PlaceSeg {
    Field(String),
    Index,
}

/// A place as root name plus the steps taken from it.
/// Free functions a call reaches before it consults any declaration.
///
/// Method names are deliberately absent: `xs.len()` is a method, and a free
/// `fn len(xs: [i64])` is the ordinary UFCS extension [L-UFCS] describes.
/// These are the names written as a plain call.
/// Names a call resolves to before it consults any declaration.
///
/// Derived from `BUILTIN_FREE_FNS` rather than written out again. It *was*
/// written out again, and the two lists drifted: `tcp_listen` and its whole
/// family were callable builtins that a program was none the less allowed to
/// declare, and adding `run` for [L-LIB-PROC] put a name in one list and not
/// the other — so `fn run` compiled, was never called, and the program silently
/// did something else. Two tables that must agree and nothing checking them is
/// the same defect twice (see ProjectGold F17).
///
/// The few names here that are not free functions — `map`, `fail`, `ref`,
/// `release`, `alive` — are handled elsewhere in `infer_call` and are listed
/// because a call still reaches them first.
fn is_builtin_fn_name(name: &str) -> bool {
    BUILTIN_FREE_FNS.contains(&name) || matches!(name, "fail" | "map" | "ref" | "release" | "alive")
}

/// Help for two integer types that met where one was wanted: they do not
/// convert on their own, in either direction ([T-INT-FIXED]).
fn int_mix_help(got: &Type, want: &Type) -> Option<String> {
    let name = |t: &Type| match t {
        Type::I64 => Some("i64"),
        Type::Int(k) => Some(k.name()),
        _ => None,
    };
    let (g, w) = (name(got)?, name(want)?);
    if g == w {
        return None;
    }
    if w == "i64" && g == "u64" {
        return Some(
            "`u64` and `i64` are different types and do not convert on their own -- `.try_i64()` gives the same value or `None` past `i64`, `.as_i64()` gives the bits ([T-U64])".to_string(),
        );
    }
    if w == "i64" {
        return Some(format!(
            "`{g}` and `i64` are different types and do not convert on their own -- `.as_i64()` gives the same value as an `i64`"
        ));
    }
    Some(format!(
        "`{g}` and `{w}` are different types and do not convert on their own -- convert with `.as_{w}()`, which wraps to the width, or `.try_{w}()`, which answers `None` when the value does not fit"
    ))
}

/// Names `resolve_type` answers for before it consults any declaration.
///
/// Kept in step with that function on purpose: a name it handles and this one
/// does not is a name a program may declare and never reach.
fn is_builtin_type_name(name: &str) -> bool {
    matches!(
        name,
        "i8" | "i16"
            | "i32"
            | "i64"
            | "u8"
            | "u16"
            | "u32"
            | "u64"
            | "usize"
            | "int"
            | "f32"
            | "f64"
            | "float"
            | "bool"
            | "str"
            | "Map"
    )
}

fn place_path(e: &Expr) -> Option<(String, Vec<PlaceSeg>)> {
    match e {
        Expr::Ident(id) => Some((id.name.clone(), Vec::new())),
        Expr::Field { recv, name, .. } => {
            let (root, mut segs) = place_path(recv)?;
            segs.push(PlaceSeg::Field(name.name.clone()));
            Some((root, segs))
        }
        Expr::Index { recv, .. } => {
            let (root, mut segs) = place_path(recv)?;
            segs.push(PlaceSeg::Index);
            Some((root, segs))
        }
        _ => None,
    }
}

/// Whether two places can name the same storage.
///
/// One being a prefix of the other is an overlap — `p` covers `p.xs`. Two
/// different *fields* of one value are disjoint, and refusing them was a
/// false alarm on the most ordinary shape there is: `two(p.xs, p.ys)` was
/// "`p` is passed as `inout` twice in one call", about two places that share
/// nothing but a root ([T-INOUT-EXCLUSIVE] is about the same value).
fn places_overlap(a: &(String, Vec<PlaceSeg>), b: &(String, Vec<PlaceSeg>)) -> bool {
    if a.0 != b.0 {
        return false;
    }
    a.1.iter().zip(b.1.iter()).all(|(x, y)| x == y)
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

/// Whether a block always leaves before its end: its last statement returns,
/// breaks, continues, or calls something that does not come back.
fn block_leaves(b: &Block) -> bool {
    match b.stmts.last() {
        Some(Stmt::Return(..)) | Some(Stmt::Break(_)) | Some(Stmt::Continue(_)) => true,
        Some(Stmt::Expr(e)) => expr_leaves(e),
        _ => false,
    }
}

fn expr_leaves(e: &Expr) -> bool {
    match e {
        Expr::Block(b) => block_leaves(b),
        Expr::Call { callee, .. } => {
            matches!(&**callee, Expr::Ident(id) if matches!(id.name.as_str(), "panic" | "exit" | "fail"))
        }
        _ => false,
    }
}

/// Every name an expression mentions.
pub(crate) fn idents_in(e: &Expr, out: &mut Vec<String>) {
    match e {
        Expr::Ident(id) => out.push(id.name.clone()),
        Expr::Unary { expr, .. } => idents_in(expr, out),
        Expr::Binary { lhs, rhs, .. } => {
            idents_in(lhs, out);
            idents_in(rhs, out);
        }
        Expr::Call { callee, args, .. } => {
            idents_in(callee, out);
            for a in args {
                idents_in(a, out);
            }
        }
        _ => {}
    }
}

/// The number an integer literal written against `k` stands for, for its
/// range check ([T-INT-FIXED-RANGE]). A hex or binary literal is a bit
/// pattern, so `0xFFFF_FFFF_FFFF_FFFF` arrives as the `i64` -1; against a
/// `u64` it is read as the pattern, 2^64 - 1, and only zero survives a
/// negation. Every other width reads the `i64` as it always has.
fn literal_value(k: IntKind, v: i64, negated: bool) -> i128 {
    match (k, negated) {
        (IntKind::U64, false) => k.value(v),
        (IntKind::U64, true) => -k.value(v),
        (_, false) => i128::from(v),
        (_, true) => i128::from(v.wrapping_neg()),
    }
}
