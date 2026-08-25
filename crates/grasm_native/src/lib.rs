//! grasm_native — Cranelift AOT backend.
//!
//! N1: scalar subset (i64/f64/bool, control flow, functions, print).
//! N2: strings + lists as refcounted boxed values (`*mut RtVal`) with
//! copy-on-write, matching interpreter value semantics.
//!
//! Ownership discipline in generated code: every expression producing a boxed
//! value yields an OWNED reference (+1). Consumers either transfer ownership
//! (stores, calls, list pushes) or call `grasm_val_dec` after use. Scope exits
//! dec their boxed locals; early `return` skips sibling decs (bounded leak,
//! hardened later). Unsupported constructs produce file:line diagnostics.

use std::collections::HashMap;

use cranelift_codegen::ir::condcodes::{FloatCC, IntCC};
use cranelift_codegen::ir::{types, AbiParam, Block, InstBuilder, Value};
use cranelift_codegen::settings::{self, Configurable};
use cranelift_frontend::{FunctionBuilder, FunctionBuilderContext, Variable};
use cranelift_module::{DataDescription, FuncId, Linkage, Module};
use cranelift_object::{ObjectBuilder, ObjectModule};

use grasm_syntax::ast::Module as AstModule;
use grasm_syntax::ast::*;
use grasm_syntax::span::Span;

/// Marks an `Unsupported` that is really a backend bug, not a gap.
const INTERNAL: &str = "\u{1}internal\u{1}";

/// Whether `region` frees its allocations in one sweep at the closing brace.
///
/// The sweep is only sound if [T-REGION]'s escape rule is airtight, and it is
/// not yet: a `ref` written through a call, and an effect handler written
/// outside the block, both carry a value allocated inside past the brace. With
/// the sweep on, those are use-after-free rather than a wrong answer. The
/// runtime half is written and tested; this turns it on once the checker
/// closes the two routes. See MASTERPLAN §8.3.
const REGION_SWEEP: bool = false;

/// How deeply a type argument to a generic may nest before the backend calls
/// it polymorphic recursion. Generous: 32 nested lists is far past anything
/// written on purpose, and the shape that hits it grows without bound.
const MAX_TYPE_DEPTH: usize = 32;

pub struct Unsupported {
    pub span: Span,
    pub what: String,
}

type CResult<T> = Result<T, Unsupported>;

/// What an expression is, in the words the tutorial uses for it. A refusal
/// that says "inferring this type" tells the reader nothing they can act on;
/// naming the shape at least says which line to rewrite.
fn expr_kind(e: &Expr) -> &'static str {
    match e {
        Expr::Ident(_) => "a name",
        Expr::Int(..) | Expr::Float(..) | Expr::Str(..) | Expr::Bool(..) => "a literal",
        Expr::Call { .. } => "a call",
        Expr::MethodCall { .. } => "a method call",
        Expr::Field { .. } => "a field read",
        Expr::Index { .. } => "an index",
        Expr::Binary { .. } => "an operator expression",
        Expr::Unary { .. } => "a unary expression",
        Expr::Closure { .. } => "a closure",
        Expr::StructLit { .. } => "a struct literal",
        Expr::List(..) => "a list literal",
        Expr::Map(..) => "a map literal",
        Expr::Tuple(..) => "a tuple",
        Expr::If { .. } => "an `if`",
        Expr::Match { .. } => "a `match`",
        Expr::Block(..) => "a block",
        Expr::Try { .. } => "a `try`",
        Expr::Range { .. } => "a range",
        _ => "this expression",
    }
}

fn nope<T>(span: Span, what: impl Into<String>) -> CResult<T> {
    Err(Unsupported {
        span,
        what: what.into(),
    })
}

#[derive(Clone, PartialEq, Debug)]
enum VType {
    I64,
    F64,
    Bool,
    Unit,
    Str,
    /// A contiguous run of bytes ([L-LIB-BYTES]). Carried as a boxed runtime
    /// value, like `Str` — a byte buffer has no scalar representation.
    Bytes,
    List(Box<VType>),
    /// An effect handler carried as a value.
    ///
    /// A reference-counted value holding, per operation, a code pointer and a
    /// heap environment. It was a pointer into the frame that built it, which
    /// made passing one down sound and returning or storing one unsound — so
    /// those were refused. The environment is on the heap now and counted like
    /// anything else, and the question "is this still alive?" has the same
    /// answer here as everywhere.
    Handler(String),
    /// `a..b` held as a value, rather than walked by a `for`.
    ///
    /// Boxed: three fields do not fit a machine word, and a range is rare
    /// enough in a hot loop that it is not worth a second representation. The
    /// counted-loop compilation of `for i in 0..n` does not go through this
    /// and is unchanged.
    Range,
    /// Tier 2 — a generational reference, one i64: slot index in the high
    /// half, generation in the low half. A plain scalar, so it copies freely
    /// and needs no refcount of its own; the slab owns what it points at.
    Ref(Box<VType>),
    Struct(String),
    Sum(String),
    Option(Box<VType>),
    /// `T ! E` — boxed Ok/Err variant wrapper. Both halves are carried:
    /// dropping `E` meant a `match` could bind an `Err` payload and have no
    /// idea what it was, so reading a field off it was rejected.
    Fallible(Box<VType>, Box<VType>),
    Map(Box<VType>, Box<VType>),
    Tuple(Vec<VType>),
    /// Dynamically-typed boxed value (error payloads); printable + variant-matchable.
    Any,
    /// A function value: a code pointer plus its captured environment.
    /// Named functions and closures share one representation, so a list can
    /// hold both.
    Fn(Vec<VType>, Box<VType>),
}

impl VType {
    /// A symbol-safe name, for monomorphized function symbols.
    /// How deeply the type nests. Monomorphization instantiates one machine
    /// copy per type argument, so polymorphic recursion — `fn f[T](x: T) { f([x]) }`
    /// — asks for `f[i64]`, `f[[i64]]`, `f[[[i64]]]` and never stops. The
    /// build ran until it was killed, with no diagnostic and unbounded
    /// memory, where §11 promises a refusal by name.
    fn depth(&self) -> usize {
        1 + match self {
            VType::List(t) | VType::Ref(t) | VType::Option(t) => t.depth(),
            VType::Fallible(a, b) | VType::Map(a, b) => a.depth().max(b.depth()),
            VType::Tuple(ts) => ts.iter().map(|t| t.depth()).max().unwrap_or(0),
            VType::Fn(ps, r) => {
                ps.iter().map(|t| t.depth()).max().unwrap_or(0).max(r.depth())
            }
            _ => 0,
        }
    }

    fn mangle(&self) -> String {
        match self {
            VType::I64 => "i64".into(),
            VType::Ref(inner) => format!("ref_{}", inner.mangle()),
            VType::F64 => "f64".into(),
            VType::Bool => "bool".into(),
            VType::Unit => "unit".into(),
            VType::Str => "str".into(),
            VType::Bytes => "bytes".into(),
            VType::Range => "range".into(),
            VType::Handler(e) => format!("h{e}"),
            VType::Any => "any".into(),
            VType::List(e) => format!("l{}", e.mangle()),
            VType::Option(e) => format!("o{}", e.mangle()),
            VType::Fallible(o, e) => format!("f{}_{}", o.mangle(), e.mangle()),
            VType::Struct(n) | VType::Sum(n) => format!("n{n}"),
            VType::Map(k, v) => format!("m{}_{}", k.mangle(), v.mangle()),
            VType::Fn(ps, r) => format!(
                "fn{}_{}",
                ps.iter().map(VType::mangle).collect::<Vec<_>>().join("_"),
                r.mangle()
            ),
            VType::Tuple(ts) => format!(
                "t{}",
                ts.iter().map(VType::mangle).collect::<Vec<_>>().join("_")
            ),
        }
    }

    /// The type as a reader would write it. §11 promises a refusal that names
    /// what it refused; "this operator on these types" names nothing, and the
    /// reader had no way to tell which of the two operands was the problem.
    fn show(&self) -> String {
        match self {
            VType::I64 => "i64".into(),
            VType::F64 => "f64".into(),
            VType::Bool => "bool".into(),
            VType::Unit => "()".into(),
            VType::Str => "str".into(),
            VType::Bytes => "bytes".into(),
            VType::Range => "a range".into(),
            VType::Handler(e) => format!("{e}Handler"),
            VType::Any => "an inferred type".into(),
            VType::Ref(inner) => format!("ref {}", inner.show()),
            VType::List(e) => format!("[{}]", e.show()),
            VType::Option(e) => format!("{}?", e.show()),
            VType::Fallible(o, e) => format!("{} ! {}", o.show(), e.show()),
            VType::Struct(n) | VType::Sum(n) => n.clone(),
            VType::Map(k, v) => format!("[{}: {}]", k.show(), v.show()),
            VType::Fn(ps, r) => format!(
                "fn({}) -> {}",
                ps.iter().map(VType::show).collect::<Vec<_>>().join(", "),
                r.show()
            ),
            VType::Tuple(ts) => format!(
                "({})",
                ts.iter().map(VType::show).collect::<Vec<_>>().join(", ")
            ),
        }
    }

    fn cl(&self) -> types::Type {
        match self {
            VType::I64 => types::I64,
            // One word: slot index and generation packed together.
            VType::Ref(_) => types::I64,
            VType::F64 => types::F64,
            VType::Bool => types::I8,
            VType::Unit => types::I8,
            // Boxed values travel as 64-bit pointers.
            VType::Handler(_)
            | VType::Str
            | VType::Bytes
            | VType::Range
            | VType::List(_)
            | VType::Struct(_)
            | VType::Sum(_)
            | VType::Option(_)
            | VType::Fallible(_, _)
            | VType::Map(_, _)
            | VType::Tuple(_)
            | VType::Fn(_, _)
            | VType::Any => types::I64,
        }
    }

    fn boxed(&self) -> bool {
        matches!(
            self,
            VType::Handler(_)
                | VType::Str
                | VType::Bytes
                | VType::Range
                | VType::List(_)
                | VType::Struct(_)
                | VType::Sum(_)
                | VType::Option(_)
                | VType::Fallible(_, _)
                | VType::Map(_, _)
                | VType::Tuple(_)
                | VType::Fn(_, _)
                | VType::Any
        )
    }

}

#[derive(Clone)]
struct FnSig {
    params: Vec<VType>,
    /// A declared C function. Its arguments cross as C representations rather
    /// than as boxed values, and it releases nothing, so the caller does.
    is_extern: bool,
    /// `inout` params are passed as pointers to the caller's slot.
    modes: Vec<ParamMode>,
    ret: VType,
    id: FuncId,
}

pub fn compile_object(
    program: &grasm_sema::loader::Program,
    obj_out: &std::path::Path,
) -> Result<(), String> {
    compile_object_for(program, obj_out, None)
}

/// Compiles to an object file for `target` (a triple such as
/// `x86_64-unknown-linux-gnu`), or for the host when `None`.
///
/// Note: emitting a cross-target object works anywhere; *linking* it still
/// needs a cross-linker and a `libgrasm_rt.a` built for that target.
pub fn compile_object_for(
    program: &grasm_sema::loader::Program,
    obj_out: &std::path::Path,
    target: Option<&str>,
) -> Result<(), String> {
    compile_object_scoped(program, obj_out, target, None)
}

/// Compiles just `only_module`'s functions into an object, declaring every
/// other module's as imports for the linker to resolve.
///
/// This is what makes rebuilds incremental: edit one module of a large program
/// and only that module is re-emitted.
pub fn compile_module_object(
    program: &grasm_sema::loader::Program,
    module: &str,
    obj_out: &std::path::Path,
    target: Option<&str>,
) -> Result<(), String> {
    compile_object_scoped(program, obj_out, target, Some(module))
}

fn compile_object_scoped(
    program: &grasm_sema::loader::Program,
    obj_out: &std::path::Path,
    target: Option<&str>,
    only_module: Option<&str>,
) -> Result<(), String> {

    let mut flags = settings::builder();
    flags.set("is_pic", "true").map_err(|e| e.to_string())?;
    flags.set("opt_level", "speed").map_err(|e| e.to_string())?;
    let isa_builder = match target {
        Some(name) => {
            // Parse first: `lookup_by_name` panics on a malformed triple, so a
            // user typo must never reach it.
            let parsed: target_lexicon::Triple = name
                .parse()
                .map_err(|e| format!("unsupported target `{name}`: {e}"))?;
            cranelift_codegen::isa::lookup(parsed)
                .map_err(|e| format!("unsupported target `{name}`: {e}"))?
        }
        None => cranelift_native::builder().map_err(|e| e.to_string())?,
    };
    let isa = isa_builder
        .finish(settings::Flags::new(flags))
        .map_err(|e| e.to_string())?;
    let builder = ObjectBuilder::new(isa, "grasm", cranelift_module::default_libcall_names())
        .map_err(|e| e.to_string())?;
    let mut module = ObjectModule::new(builder);

    let mut cg = Codegen::new(&mut module);
    let analysis = grasm_sema::check::check_program(program);
    cg.float_literals = analysis.float_literals;
    cg.map_get_value_kind = analysis.map_get_value_kind;
    cg.field_calls = analysis.field_calls;
    cg.fallible_closures = analysis.fallible_closures;
    cg.empty_types = analysis.empty_types;
    cg.closure_param_types = analysis.closure_param_types;
    cg.generic_ctor_types = analysis.generic_ctor_types;
    cg.only_module = only_module.map(str::to_string);
    cg.declare_runtime()?;
    let views: Vec<(String, &AstModule, std::collections::HashMap<String, String>)> = program
        .files
        .iter()
        .map(|f| (f.name.clone(), &f.module, f.imports.clone()))
        .collect();
    // A panic inside the code generator is a bug in the backend, and it used
    // to reach the user as raw Rust text — a Cranelift assertion, the file it
    // fired in, and an absolute path from the machine that built the
    // compiler, with no `error:` line of grasm's own. Caught here so it
    // arrives as a diagnostic that says what happened and what still works.
    let compiled = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        cg.compile_program(&views)
    }));
    let compiled = match compiled {
        Ok(r) => r,
        Err(_) => {
            return Err(
                "internal compiler error — the backend panicked while generating code.
  This is a bug in the backend, not a limitation of the language.
  `grasm run` will still work. Please report it with this program."
                    .to_string(),
            );
        }
    };
    match compiled {
        Ok(()) => {}
        Err(u) => {
            let idx = program.file_of(u.span);
            let f = &program.files[idx];
            let index = grasm_syntax::LineIndex::new(&f.src);
            let local = Span::new(
                u.span.start.saturating_sub(f.base),
                u.span.end.saturating_sub(f.base),
            );
            let lc = index.line_col(local.start.min(f.src.len() as u32));
            // A construct the backend has not implemented and a construct it
            // implemented wrongly are different reports. The first names a
            // feature and points at the interpreter; the second is a bug and
            // should say so rather than sending the reader looking for a
            // language limitation with no name.
            if let Some(detail) = u.what.strip_prefix(INTERNAL) {
                return Err(format!(
                    "{}:{}:{}: internal compiler error — the generated code did not verify.\n  This is a bug in the backend, not a limitation of the language.\n  `grasm run` will still work. Please report it with this program.\n{}",
                    f.path, lc.line, lc.col, detail
                ));
            }
            return Err(format!(
                "{}:{}:{}: native backend does not yet support {} — use `grasm run` (interpreter) for now",
                f.path, lc.line, lc.col, u.what
            ));
        }
    }

    let product = module.finish();
    let bytes = product.emit().map_err(|e| e.to_string())?;
    std::fs::write(obj_out, bytes).map_err(|e| e.to_string())?;
    Ok(())
}

/// What a handler operation's code is.
///
/// A closure is lifted into a function of its own by the pass that walks the
/// program. A named function already exists, and needs only the wrapper that
/// gives it the `(env, params…)` shape every function value has — built where
/// wrappers are built, not in the walking pass.
#[derive(Clone)]
enum HandlerOpCode {
    Lifted(FuncId),
    Named(String),
}

struct Codegen<'m> {
    module: &'m mut ObjectModule,
    fns: HashMap<String, FnSig>,
    runtime: HashMap<&'static str, FuncId>,
    strings: HashMap<String, cranelift_module::DataId>,
    /// One type descriptor per struct shape, keyed by the type name and the
    /// field names it is built with. Keyed by both because a literal that does
    /// not name every declared field has a different shape and would otherwise
    /// read its names out of the wrong descriptor.
    struct_descs: HashMap<String, cranelift_module::DataId>,
    /// One cache slot per distinct string literal — see `str_lit_slot`.
    str_lit_slots: HashMap<String, cranelift_module::DataId>,
    /// struct name → ordered fields (name, type)
    structs: HashMap<String, Vec<(String, VType)>>,
    /// `type Name = str` — the aliased type expression, resolved on demand.
    /// Kept as the AST rather than a `VType` because an alias may name a type
    /// declared later in the file.
    aliases: HashMap<String, TypeExpr>,
    /// sum name → variants (name, field types)
    sums: HashMap<String, Vec<(String, Vec<VType>)>>,
    /// variant name → (parent sum, arity) — unambiguous per checker
    variant_index: HashMap<String, (String, usize)>,
    /// effect name → ops (op name, param types, return type)
    effects: HashMap<String, Vec<(String, Vec<VType>, VType)>>,
    /// `with` site (by span start) → op name → lifted handler FuncId
    handler_sites: HashMap<u32, Vec<(String, HandlerOpCode, Vec<String>)>>,
    /// Captures each lifted handler operation ended up with, typed at the
    /// `with` site and read back when the operation itself is compiled.
    handler_captures: HashMap<FuncId, Vec<(String, VType)>>,
    /// Lifted handler bodies to compile after the main functions.
    pending_handlers: Vec<PendingHandler>,
    /// Handler struct literals bound to a name, so `handled_by store` finds
    /// the literal this pass has to lift closures out of.
    handler_lits: HashMap<String, Expr>,
    /// Lifted closure bodies, likewise.
    pending_closures: Vec<PendingClosure>,
    /// Lifted `par_map` bodies, compiled with the scalar-bits ABI.
    pending_par_maps: Vec<PendingParMap>,
    /// Serial number for lifted `par_map` symbols. A span start is not unique
    /// enough: `xs.par_map(f).par_map(g)` gives both method calls the same
    /// start offset, and a generic can be instantiated more than once.
    par_map_seq: usize,
    /// Named function → the wrapper that gives it the function-value ABI.
    /// One wrapper per function, however many times it is used as a value.
    fn_wrappers: HashMap<String, FuncId>,
    current_module: String,
    /// module → (alias → target module)
    module_imports: HashMap<String, HashMap<String, String>>,
    /// When set, only this module's bodies are emitted; every other module's
    /// functions are declared as imports and resolved by the linker. That is
    /// what makes a rebuild proportional to what changed rather than to the
    /// size of the program.
    only_module: Option<String>,
    /// Generic functions, kept aside rather than declared. There is no such
    /// thing as a generic machine-code symbol, so each *instantiation* becomes
    /// its own concrete function.
    generic_fns: HashMap<String, FnDecl>,
    /// Generic *type* declarations, kept whole until a use site fixes their
    /// parameters — the same treatment generic functions get, for the same
    /// reason: `type Pair[A, B]` describes a family, and a machine
    /// representation needs one member of it.
    generic_types: HashMap<String, grasm_syntax::ast::TypeDecl>,
    /// An instantiated type's name → the arguments it was instantiated at.
    ///
    /// The name is mangled for uniqueness and is not meant to be read back, so
    /// matching `Holder[T]` against `Holder[i64]` asks here rather than parsing
    /// it. Without this, a generic function over a generic type had `T`
    /// determined by nothing.
    type_args: HashMap<String, (String, Vec<VType>)>,
    /// A variant name → the generic sum that declares it. One variant belongs
    /// to one family, and which *member* of it is a question for the use site.
    generic_variant_owner: HashMap<String, String>,
    /// Module-level constants, by `module::NAME`. Their initializers are
    /// effect-free, so each use compiles the expression again rather than
    /// reading a global — which keeps the two engines doing the same thing.
    consts: HashMap<String, grasm_syntax::ast::ConstDecl>,
    /// Functions that can reach themselves, directly or through others. Only
    /// these need a stack check.
    recursive: std::collections::HashSet<String>,
    /// Instantiations discovered at call sites and not yet compiled.
    pending_generics: Vec<PendingGeneric>,
    /// Bindings for the generic parameters of the function being compiled.
    /// Consulted by `vtype_of`, so a `T` in a signature resolves like any
    /// other type name while that instantiation is being emitted.
    generic_subst: HashMap<String, VType>,
    /// Integer literals the checker resolved to `f64`, by span start.
    ///
    /// The backend used to convert these itself, at each place it happened to
    /// remember to: a list element, a function argument, an `if` merge. Where
    /// it forgot — a `Map[str, f64]` value, a `contains` argument — the
    /// literal stayed an `i64` and the program either panicked with
    /// `unbox: not an f64` or answered `false` to a comparison that was true.
    /// The checker already knows which literals are floats; asking it once is
    /// both correct and the same answer the interpreter gets.
    float_literals: std::collections::HashSet<u32>,
    /// The value type of each `m.get(k)`, from the checker. See
    /// `Analysis::map_get_value_kind`.
    map_get_value_kind: std::collections::HashMap<(u32, u32), u8>,
    /// Method calls the checker resolved to a struct field holding a function.
    /// See the UFCS branch of the method-call lowering.
    field_calls: grasm_sema::check::SpanSet,
    /// Closure literals whose own type is fallible, by start offset.
    fallible_closures: std::collections::HashSet<u32>,
    /// Start offset of each empty literal → the type the checker settled on.
    empty_types: std::collections::HashMap<u32, String>,
    closure_param_types: std::collections::HashMap<u32, Vec<String>>,
    generic_ctor_types: std::collections::HashMap<u32, String>,
}

/// One monomorphized copy of a generic function.
struct PendingGeneric {
    key: String,
    module: String,
    decl: FnDecl,
    subst: HashMap<String, VType>,
}

/// A `par_map` function lifted for threaded execution: it takes captured
/// scalars as raw bits and returns raw bits, so a worker thread touches
/// nothing refcounted.
struct PendingParMap {
    id: FuncId,
    module: String,
    captures: Vec<(String, VType)>,
    param: String,
    param_t: VType,
    ret: VType,
    body: Expr,
}

/// A closure body lifted to a real function.
///
/// Its first parameter is the captured environment (a boxed list, or null when
/// nothing was captured); the rest are the closure's own parameters. Named
/// functions used as values get the same shape through a thin wrapper, so one
/// calling convention covers both and a list can hold either.
struct PendingClosure {
    id: FuncId,
    module: String,
    /// Captured names, in the order they occupy the environment list.
    captures: Vec<(String, VType)>,
    params: Vec<(String, VType)>,
    ret: VType,
    /// The body produces the ok half and the declared type is `T ! E`, so the
    /// value is wrapped on the way out — the same rule a named fallible
    /// function follows.
    wrap_ok: bool,
    body: Expr,
}

struct PendingHandler {
    id: FuncId,
    module: String,
    params: Vec<(String, VType)>,
    /// Names the operation's body reads from the scope that installed the
    /// handler. Passed as a pointer to a slot in that frame, which is sound
    /// because the frame cannot return while the handler is installed.
    captures: Vec<(String, VType)>,
    ret: VType,
    body: Expr,
}

impl<'m> Codegen<'m> {
    fn new(module: &'m mut ObjectModule) -> Self {
        Codegen {
            module,
            fns: HashMap::new(),
            runtime: HashMap::new(),
            strings: HashMap::new(),
            struct_descs: HashMap::new(),
            str_lit_slots: HashMap::new(),
            structs: HashMap::new(),
            aliases: HashMap::new(),
            sums: HashMap::new(),
            variant_index: HashMap::new(),
            effects: HashMap::new(),
            handler_sites: HashMap::new(),
            handler_captures: HashMap::new(),
            pending_handlers: Vec::new(),
            handler_lits: HashMap::new(),
            pending_closures: Vec::new(),
            pending_par_maps: Vec::new(),
            par_map_seq: 0,
            fn_wrappers: HashMap::new(),
            current_module: "main".to_string(),
            module_imports: HashMap::new(),
            only_module: None,
            generic_fns: HashMap::new(),
            generic_types: HashMap::new(),
            type_args: HashMap::new(),
            generic_variant_owner: HashMap::new(),
            consts: HashMap::new(),
            recursive: std::collections::HashSet::new(),
            pending_generics: Vec::new(),
            generic_subst: HashMap::new(),
            float_literals: std::collections::HashSet::new(),
            map_get_value_kind: std::collections::HashMap::new(),
            field_calls: Default::default(),
            fallible_closures: std::collections::HashSet::new(),
            empty_types: std::collections::HashMap::new(),
            closure_param_types: std::collections::HashMap::new(),
            generic_ctor_types: std::collections::HashMap::new(),
        }
    }

    fn declare_runtime(&mut self) -> Result<(), String> {
        let ptr = self.module.target_config().pointer_type();
        let i64t = types::I64;
        let i8t = types::I8;
        let f64t = types::F64;
        // (name, params, ret)
        let decls: &[(&str, Vec<types::Type>, Option<types::Type>)] = &[
            ("grasm_out_text", vec![ptr, i64t], None),
            ("grasm_out_i64", vec![i64t], None),
            ("grasm_out_f64", vec![f64t], None),
            ("grasm_out_bool", vec![i8t], None),
            ("grasm_out_val", vec![i64t], None),
            ("grasm_out_nl", vec![], None),
            ("grasm_out_to_stderr", vec![i8t], None),
            ("grasm_panic_div_zero", vec![], None),
            ("grasm_panic_rem_zero", vec![], None),
            ("grasm_panic_assert", vec![], None),
            ("grasm_panic_msg", vec![ptr, i64t], None),
            ("grasm_pow_i64", vec![i64t, i64t], Some(i64t)),
            ("grasm_pow_f64", vec![f64t, f64t], Some(f64t)),
            ("grasm_f64_rem", vec![f64t, f64t], Some(f64t)),
            ("grasm_val_inc", vec![i64t], None),
            ("grasm_val_dec", vec![i64t], None),
            ("grasm_str_lit", vec![ptr, i64t], Some(i64t)),
            ("grasm_str_lit_immortal", vec![ptr, i64t], Some(i64t)),
            ("grasm_str_concat", vec![i64t, i64t], Some(i64t)),
            ("grasm_str_append", vec![i64t, i64t], Some(i64t)),
            ("grasm_list_concat", vec![i64t, i64t], Some(i64t)),
            ("grasm_list_sort_by", vec![i64t, i64t], Some(i64t)),
            ("grasm_list_remove", vec![i64t, i64t, ptr], Some(i64t)),
            ("grasm_list_enumerate", vec![i64t], Some(i64t)),
            ("grasm_list_zip", vec![i64t, i64t], Some(i64t)),
            ("grasm_str_code_at", vec![i64t, i64t], Some(i64t)),
            ("grasm_str_codes", vec![i64t], Some(i64t)),
            ("grasm_str_words", vec![i64t], Some(i64t)),
            ("grasm_i64_to_char", vec![i64t], Some(i64t)),
            ("grasm_f64_to_fixed", vec![f64t, i64t], Some(i64t)),
            ("grasm_f64_is_nan", vec![f64t], Some(i8t)),
            ("grasm_f64_is_finite", vec![f64t], Some(i8t)),
            ("grasm_val_cmp", vec![i64t, i64t], Some(i64t)),
            ("grasm_assert_eq", vec![i64t, i64t], None),
            ("grasm_stack_check", vec![i64t], None),
            ("grasm_str_from_i64", vec![i64t], Some(i64t)),
            ("grasm_str_from_f64", vec![f64t], Some(i64t)),
            ("grasm_str_from_bool", vec![i8t], Some(i64t)),
            ("grasm_str_len", vec![i64t], Some(i64t)),
            ("grasm_str_byte_len", vec![i64t], Some(i64t)),
            ("grasm_i64_to_base", vec![i64t, i64t], Some(i64t)),
            ("grasm_str_parse_int_base", vec![i64t, i64t], Some(i64t)),
            ("grasm_str_eq", vec![i64t, i64t], Some(i8t)),
            ("grasm_str_cmp", vec![i64t, i64t], Some(i64t)),
            ("grasm_list_new", vec![], Some(i64t)),
            ("grasm_tuple_seal", vec![i64t], Some(i64t)),
            ("grasm_list_push", vec![i64t, i64t], Some(i64t)),
            ("grasm_list_len", vec![i64t], Some(i64t)),
            ("grasm_list_get", vec![i64t, i64t], Some(i64t)),
            ("grasm_box_i64", vec![i64t], Some(i64t)),
            ("grasm_box_unit", vec![], Some(i64t)),
            ("grasm_box_ref", vec![i64t], Some(i64t)),
            ("grasm_box_f64", vec![f64t], Some(i64t)),
            ("grasm_box_bool", vec![i8t], Some(i64t)),
            ("grasm_unbox_i64", vec![i64t], Some(i64t)),
            ("grasm_unbox_f64", vec![i64t], Some(f64t)),
            ("grasm_unbox_bool", vec![i64t], Some(i8t)),
            ("grasm_struct_make", vec![ptr, ptr], Some(i64t)),
            ("grasm_struct_get", vec![i64t, ptr, i64t], Some(i64t)),
            ("grasm_struct_get_at", vec![i64t, i64t], Some(i64t)),
            ("grasm_struct_scalar_at", vec![i64t, i64t], Some(i64t)),
            ("grasm_struct_borrow_at", vec![i64t, i64t], Some(i64t)),
            ("grasm_struct_set_field", vec![i64t, ptr, i64t, i64t], Some(i64t)),
            ("grasm_struct_set_field_at", vec![i64t, i64t, i64t], Some(i64t)),
            ("grasm_variant_new", vec![ptr, i64t, ptr, i64t], Some(i64t)),
            ("grasm_variant_push", vec![i64t, i64t], None),
            ("grasm_variant_is", vec![i64t, ptr, i64t], Some(i8t)),
            ("grasm_variant_field", vec![i64t, i64t], Some(i64t)),
            ("grasm_ok_new", vec![i64t], Some(i64t)),
            ("grasm_err_new", vec![i64t], Some(i64t)),
            ("grasm_is_ok", vec![i64t], Some(i8t)),
            ("grasm_report_main_err", vec![i64t], None),
            ("grasm_val_eq", vec![i64t, i64t], Some(i8t)),
            ("grasm_list_get_opt", vec![i64t, i64t], Some(i64t)),
            ("grasm_str_from_val", vec![i64t], Some(i64t)),
            ("grasm_map_new", vec![], Some(i64t)),
            ("grasm_map_set", vec![i64t, i64t, i64t], Some(i64t)),
            ("grasm_map_get", vec![i64t, i64t], Some(i64t)),
            ("grasm_map_has", vec![i64t, i64t], Some(i8t)),
            ("grasm_map_delete", vec![i64t, i64t, ptr], Some(i64t)),
            ("grasm_map_len", vec![i64t], Some(i64t)),
            ("grasm_map_get_ptr", vec![i64t, i64t], Some(i64t)),
            ("grasm_map_keys", vec![i64t], Some(i64t)),
            ("grasm_map_values", vec![i64t], Some(i64t)),
            ("grasm_map_entries", vec![i64t], Some(i64t)),
            ("grasm_panic_value", vec![i64t], None),
            ("grasm_str_contains", vec![i64t, i64t], Some(i8t)),
            ("grasm_str_starts_with", vec![i64t, i64t], Some(i8t)),
            ("grasm_str_ends_with", vec![i64t, i64t], Some(i8t)),
            ("grasm_str_split", vec![i64t, i64t], Some(i64t)),
            ("grasm_str_chars", vec![i64t], Some(i64t)),
            ("grasm_str_trim", vec![i64t], Some(i64t)),
            ("grasm_str_trim_start", vec![i64t], Some(i64t)),
            ("grasm_str_trim_end", vec![i64t], Some(i64t)),
            ("grasm_str_upper", vec![i64t], Some(i64t)),
            ("grasm_str_lower", vec![i64t], Some(i64t)),
            ("grasm_str_repeat", vec![i64t, i64t], Some(i64t)),
            ("grasm_str_replace", vec![i64t, i64t, i64t], Some(i64t)),
            ("grasm_str_substr", vec![i64t, i64t, i64t], Some(i64t)),
            ("grasm_str_index_of", vec![i64t, i64t], Some(i64t)),
            ("grasm_str_parse_int", vec![i64t], Some(i64t)),
            ("grasm_str_parse_float", vec![i64t], Some(i64t)),
            ("grasm_list_join", vec![i64t, i64t], Some(i64t)),
            ("grasm_list_rev", vec![i64t], Some(i64t)),
            ("grasm_list_sort", vec![i64t], Some(i64t)),
            ("grasm_list_slice", vec![i64t, i64t, i64t], Some(i64t)),
            ("grasm_list_contains", vec![i64t, i64t], Some(i8t)),
            ("grasm_list_pop", vec![i64t, ptr], Some(i64t)),
            ("grasm_list_first", vec![i64t], Some(i64t)),
            ("grasm_list_last", vec![i64t], Some(i64t)),
            ("grasm_list_set", vec![i64t, i64t, i64t], Some(i64t)),
            ("grasm_list_sum_i64", vec![i64t], Some(i64t)),
            ("grasm_list_sum_f64", vec![i64t], Some(f64t)),
            ("grasm_i64_abs", vec![i64t], Some(i64t)),
            ("grasm_i64_min", vec![i64t, i64t], Some(i64t)),
            ("grasm_i64_max", vec![i64t, i64t], Some(i64t)),
            ("grasm_f64_abs", vec![f64t], Some(f64t)),
            ("grasm_f64_sqrt", vec![f64t], Some(f64t)),
            ("grasm_f64_sin", vec![f64t], Some(f64t)),
            ("grasm_f64_cos", vec![f64t], Some(f64t)),
            ("grasm_f64_tan", vec![f64t], Some(f64t)),
            ("grasm_f64_asin", vec![f64t], Some(f64t)),
            ("grasm_f64_acos", vec![f64t], Some(f64t)),
            ("grasm_f64_atan", vec![f64t], Some(f64t)),
            ("grasm_f64_sinh", vec![f64t], Some(f64t)),
            ("grasm_f64_cosh", vec![f64t], Some(f64t)),
            ("grasm_f64_tanh", vec![f64t], Some(f64t)),
            ("grasm_f64_exp", vec![f64t], Some(f64t)),
            ("grasm_f64_ln", vec![f64t], Some(f64t)),
            ("grasm_f64_log2", vec![f64t], Some(f64t)),
            ("grasm_f64_log10", vec![f64t], Some(f64t)),
            ("grasm_f64_cbrt", vec![f64t], Some(f64t)),
            ("grasm_f64_to_radians", vec![f64t], Some(f64t)),
            ("grasm_f64_to_degrees", vec![f64t], Some(f64t)),
            ("grasm_f64_atan2", vec![f64t, f64t], Some(f64t)),
            ("grasm_f64_hypot", vec![f64t, f64t], Some(f64t)),
            ("grasm_f64_log", vec![f64t, f64t], Some(f64t)),
            ("grasm_f64_floor", vec![f64t], Some(f64t)),
            ("grasm_f64_ceil", vec![f64t], Some(f64t)),
            ("grasm_f64_round", vec![f64t], Some(f64t)),
            ("grasm_f64_min", vec![f64t, f64t], Some(f64t)),
            ("grasm_f64_max", vec![f64t, f64t], Some(f64t)),
            ("grasm_handler_push", vec![ptr, i64t], None),
            ("grasm_handler_op", vec![ptr, i64t, ptr, ptr], None),
            ("grasm_handler_pop", vec![], None),
            ("grasm_fn_new", vec![i64t, i64t, ptr, i64t], Some(i64t)),
            ("grasm_fn_code", vec![i64t], Some(i64t)),
            ("grasm_fn_env", vec![i64t], Some(i64t)),
            ("grasm_handler_lookup", vec![ptr, i64t, ptr, i64t], Some(ptr)),
            ("grasm_handler_env", vec![ptr, i64t, ptr, i64t], Some(ptr)),
            ("grasm_args", vec![], Some(i64t)),
            ("grasm_input", vec![], Some(i64t)),
            ("grasm_read_file", vec![i64t], Some(i64t)),
            // The `net` capability. Handles and payloads are ordinary boxed
            // values, so these look like any other runtime call.
            ("grasm_tcp_listen", vec![i64t], Some(i64t)),
            ("grasm_tcp_connect", vec![i64t, i64t], Some(i64t)),
            ("grasm_tcp_accept", vec![i64t], Some(i64t)),
            ("grasm_tcp_read", vec![i64t], Some(i64t)),
            ("grasm_tcp_read_bytes", vec![i64t], Some(i64t)),
            ("grasm_random_bytes", vec![i64t], Some(i64t)),
            ("grasm_tcp_write_bytes", vec![i64t, i64t], Some(i64t)),
            ("grasm_tcp_write", vec![i64t, i64t], Some(i64t)),
            ("grasm_net_close", vec![i64t], None),
            ("grasm_net_wait", vec![i64t, i64t], Some(i64t)),
            ("grasm_net_port", vec![i64t], Some(i64t)),
            ("grasm_map_add_i64", vec![i64t, i64t, i64t, i64t], Some(i64t)),
            ("grasm_proc_run", vec![i64t, i64t], Some(i64t)),
            ("grasm_proc_pipeline", vec![i64t, i64t], Some(i64t)),
            // The C boundary ([T-EXTERN-POINTER]).
            ("grasm_str_c_new", vec![i64t], Some(i64t)),
            ("grasm_str_c_free", vec![i64t], None),
            ("grasm_bytes_ptr", vec![i64t], Some(i64t)),
            ("grasm_udp_bind", vec![i64t], Some(i64t)),
            ("grasm_udp_recv", vec![i64t], Some(i64t)),
            ("grasm_udp_send", vec![i64t, i64t, i64t, i64t], Some(i64t)),
            ("grasm_write_file", vec![i64t, i64t], Some(i64t)),
            ("grasm_file_exists", vec![i64t], Some(i8t)),
            ("grasm_remove_file", vec![i64t], Some(i64t)),
            ("grasm_make_dir", vec![i64t], Some(i64t)),
            ("grasm_bytes_of", vec![i64t], Some(i64t)),
            ("grasm_bytes_len", vec![i64t], Some(i64t)),
            ("grasm_bytes_at", vec![i64t, i64t], Some(i64t)),
            ("grasm_bytes_slice", vec![i64t, i64t, i64t], Some(i64t)),
            ("grasm_bytes_concat", vec![i64t, i64t], Some(i64t)),
            ("grasm_bytes_to_list", vec![i64t], Some(i64t)),
            ("grasm_bytes_to_str", vec![i64t], Some(i64t)),
            ("grasm_str_to_bytes", vec![i64t], Some(i64t)),
            ("grasm_read_bytes", vec![i64t], Some(i64t)),
            ("grasm_write_bytes", vec![i64t, i64t], Some(i64t)),
            ("grasm_list_dir", vec![i64t], Some(i64t)),
            ("grasm_set_args", vec![i64t, ptr], None),
            ("grasm_now_millis", vec![], Some(i64t)),
            ("grasm_check_leaks", vec![], None),
            ("grasm_struct_take", vec![ptr, ptr, i64t], Some(ptr)),
            ("grasm_list_take", vec![ptr, i64t], Some(ptr)),
            ("grasm_region_push", vec![], None),
            ("grasm_region_pop", vec![], None),
            ("grasm_out_ref", vec![i64t], None),
            ("grasm_ref_new", vec![i64t], Some(i64t)),
            ("grasm_ref_get", vec![i64t], Some(i64t)),
            ("grasm_ref_set", vec![i64t, i64t], None),
            ("grasm_ref_make_unique", vec![i64t], Some(i64t)),
            ("grasm_ref_release", vec![i64t], None),
            ("grasm_ref_alive", vec![i64t], Some(i8t)),
            ("grasm_env", vec![i64t], Some(i64t)),
            ("grasm_exit", vec![i64t], None),
            ("grasm_str_lines", vec![i64t], Some(i64t)),
            ("grasm_str_pad", vec![i64t, i64t, i64t, i8t], Some(i64t)),
            ("grasm_str_is_empty", vec![i64t], Some(i8t)),
            ("grasm_list_is_empty", vec![i64t], Some(i8t)),
            ("grasm_map_is_empty", vec![i64t], Some(i8t)),
            ("grasm_list_min_max", vec![i64t, i8t], Some(i64t)),
            ("grasm_list_new_scalar", vec![i64t], Some(i64t)),
            ("grasm_range_new", vec![i64t, i64t, i8t], Some(i64t)),
            ("grasm_env_new", vec![i64t], Some(i64t)),
            ("grasm_env_set", vec![i64t, i64t, i64t, i8t], None),
            ("grasm_handler_new", vec![ptr, i64t], Some(i64t)),
            ("grasm_handler_add", vec![i64t, ptr, i64t, ptr, i64t], None),
            ("grasm_handler_install", vec![i64t], None),
            ("grasm_range_to_list", vec![i64t], Some(i64t)),
            ("grasm_range_len", vec![i64t], Some(i64t)),
            ("grasm_range_start", vec![i64t], Some(i64t)),
            ("grasm_range_end", vec![i64t], Some(i64t)),
            ("grasm_range_inclusive", vec![i64t], Some(i8t)),
            ("grasm_list_push_scalar", vec![i64t, i64t, i64t], Some(i64t)),
            ("grasm_list_get_scalar", vec![i64t, i64t, i64t], Some(i64t)),
            ("grasm_list_set_scalar", vec![i64t, i64t, i64t, i64t], Some(i64t)),
            ("grasm_list_pin", vec![i64t, i64t, ptr, ptr], Some(i64t)),
            ("grasm_panic_index", vec![i64t, i64t], None),
            ("grasm_par_map_scalar", vec![i64t, ptr, ptr, i64t, i64t], Some(i64t)),
            ("grasm_par_map_mixed", vec![i64t, ptr, ptr, i64t, i64t, i64t], Some(i64t)),
            ("grasm_par_filter_scalar", vec![i64t, ptr, ptr, i64t], Some(i64t)),
            ("grasm_par_filter_boxed", vec![i64t, ptr, ptr, i64t], Some(i64t)),
        ];
        for (name, params, ret) in decls {
            let mut sig = self.module.make_signature();
            for p in params {
                sig.params.push(AbiParam::new(*p));
            }
            if let Some(r) = ret {
                sig.returns.push(AbiParam::new(*r));
            }
            let id = self
                .module
                .declare_function(name, Linkage::Import, &sig)
                .map_err(|e| e.to_string())?;
            self.runtime.insert(name, id);
        }
        Ok(())
    }

    fn vtype_of(&mut self, te: &TypeExpr) -> CResult<VType> {
        match te {
            TypeExpr::Ref(inner, _) => Ok(VType::Ref(Box::new(self.vtype_of(inner)?))),
            TypeExpr::Name { path, args, span } => {
                if path.segments.len() == 1 {
                    let n = path.segments[0].name.as_str();
                    if args.is_empty() {
                        // A generic parameter of the instantiation being
                        // emitted resolves to whatever it was bound to.
                        if let Some(bound) = self.generic_subst.get(n) {
                            return Ok(bound.clone());
                        }
                        match n {
                            "i8" | "i16" | "i32" | "i64" | "u8" | "u16" | "u32" | "u64"
                            | "usize" | "int" => return Ok(VType::I64),
                            "f32" | "f64" | "float" => return Ok(VType::F64),
                            "bool" => return Ok(VType::Bool),
                            "str" => return Ok(VType::Str),
                            "bytes" => return Ok(VType::Bytes),
                            "range" => return Ok(VType::Range),
                            _ if n
                                .strip_suffix("Handler")
                                .is_some_and(|e| self.effects.contains_key(e)) =>
                            {
                                return Ok(VType::Handler(
                                    n.strip_suffix("Handler").expect("just checked").to_string(),
                                ))
                            }
                            _ => {}
                        }
                        // An alias is its target, resolved here so nothing
                        // downstream has to know aliases exist.
                        if let Some(target) = self.aliases.get(n).cloned() {
                            return self.vtype_of(&target);
                        }
                        if self.structs.contains_key(n) {
                            return Ok(VType::Struct(n.to_string()));
                        }
                        if self.sums.contains_key(n) {
                            return Ok(VType::Sum(n.to_string()));
                        }
                    }
                    // `Pair[i64, str]` — a member of a generic family, named
                    // for its arguments and registered on demand.
                    if !args.is_empty()
                        && self
                            .generic_types
                            .contains_key(&format!("{}::{}", self.current_module, n))
                    {
                        let mut ats = Vec::with_capacity(args.len());
                        for a in args {
                            ats.push(self.vtype_of(a)?);
                        }
                        let module = self.current_module.clone();
                        let key = self.instantiate_type(&module, n, &ats, *span)?;
                        return Ok(if self.sums.contains_key(&key) {
                            VType::Sum(key)
                        } else {
                            VType::Struct(key)
                        });
                    }
                    if n == "Option" && args.len() == 1 {
                        return Ok(VType::Option(Box::new(self.vtype_of(&args[0])?)));
                    }
                    if n == "Map" && args.len() == 2 {
                        return Ok(VType::Map(
                            Box::new(self.vtype_of(&args[0])?),
                            Box::new(self.vtype_of(&args[1])?),
                        ));
                    }
                }
                nope(*span, format!("the type `{}`", path.dotted()))
            }
            TypeExpr::Fn { params, ret, .. } => {
                let mut ps = Vec::with_capacity(params.len());
                for p in params {
                    ps.push(self.vtype_of(p)?);
                }
                Ok(VType::Fn(ps, Box::new(self.vtype_of(ret)?)))
            }
            TypeExpr::List(inner, _) => Ok(VType::List(Box::new(self.vtype_of(inner)?))),
            TypeExpr::Fallible(ok, err, _) => Ok(VType::Fallible(
                Box::new(self.vtype_of(ok)?),
                Box::new(self.vtype_of(err)?),
            )),
            TypeExpr::Tuple(items, _) if items.is_empty() => Ok(VType::Unit),
            TypeExpr::Tuple(items, _) => {
                // `()` is the unit type, not a zero-length tuple. Reading it
                // as the latter gave a `let nothing: () = ()` a boxed-pointer
                // variable holding unit's one-byte value, and Cranelift
                // rejected the function with "declared type of variable var0
                // doesn't match type of value".
                if items.is_empty() {
                    return Ok(VType::Unit);
                }
                let mut ts = Vec::with_capacity(items.len());
                for i in items {
                    ts.push(self.vtype_of(i)?);
                }
                Ok(VType::Tuple(ts))
            }
            other => nope(other.span(), "non-scalar types"),
        }
    }

    /// Compiles every module of a program. Types/effects share one global
    /// namespace (as in the checker); functions are per-module symbols.
    fn compile_program(
        &mut self,
        views: &[(String, &AstModule, std::collections::HashMap<String, String>)],
    ) -> CResult<()> {
        for (name, m, imports) in views {
            self.module_imports.insert(name.clone(), imports.clone());
            self.collect_types(m)?;
            let _ = name;
        }
        // Effects before type bodies, because a field may be of handler type
        // and `XHandler` only means something once the effect `X` is known.
        // The other way round, `type B = { h: SHandler }` was refused as an
        // unknown type. Effects need only type *names*, which `collect_types`
        // has already registered.
        for (_, m, _) in views {
            self.collect_effects(m)?;
        }
        for (_, m, _) in views {
            self.resolve_type_bodies(m)?;
        }
        for (name, m, _) in views {
            self.current_module = name.clone();
            self.declare_fns(m)?;
        }
        self.recursive = recursive_functions(views);
        if !self.fns.contains_key("main::main") {
            // Not a backend limitation: a program without `main` has no
            // entry point in any engine ([E-ENTRY]). The CLI checks for it
            // before getting here, so this is a backstop.
            return nope(Span::new(0, 1), "programs without a `main` function");
        }
        for (name, m, _) in views {
            self.current_module = name.clone();
            if !self.emitting_current() {
                continue;
            }
            for decl in &m.decls {
                if let Decl::Fn(f) = decl {
                    self.collect_handlers_block(&f.body)?;
                }
            }
        }
        for (name, m, _) in views {
            self.current_module = name.clone();
            if !self.emitting_current() {
                continue;
            }
            for decl in &m.decls {
                if let Decl::Fn(f) = decl {
                    self.compile_fn(f)?;
                }
            }
        }
        // Instantiating a generic can discover another one, so drain until
        // the worklist is empty rather than iterating it once.
        while let Some(g) = self.pending_generics.pop() {
            self.current_module = g.module.clone();
            let saved = std::mem::replace(&mut self.generic_subst, g.subst);
            let result = self.compile_fn_as(&g.decl, &g.key);
            self.generic_subst = saved;
            result?;
        }

        // A lifted closure can itself create another, so drain the worklist.
        // Until both queues are empty: compiling a handler operation can
        // create closures, and compiling a closure can create handlers.
        // Draining closures once and then handlers left a closure declared
        // inside a handler operation never emitted, and the *linker* reported
        // it — as a missing C function named `gc_main_161`, for a program
        // with no `extern fn` in it.
        loop {
            let mut moved = false;
            while let Some(c) = self.pending_closures.pop() {
                moved = true;
                self.current_module = c.module.clone();
                self.compile_closure(c)?;
            }
            let pending = std::mem::take(&mut self.pending_handlers);
            for h in pending {
                moved = true;
                self.current_module = h.module.clone();
                self.compile_handler(h)?;
            }
            // In the same loop, for the reason above: a `par_map` written
            // inside a closure body — or inside a handler operation — is
            // discovered while that body is compiled. Drained once beforehand
            // instead, its lifted worker was declared and never defined, and
            // the *linker* reported it as a missing C function named
            // `gp_main_1`, for a program with no `extern fn` in it. The same
            // shape, and the same wrong diagnostic, as the closure case this
            // loop already exists to prevent.
            while let Some(c) = self.pending_par_maps.pop() {
                moved = true;
                self.current_module = c.module.clone();
                self.compile_par_map(c)?;
            }
            if !moved {
                break;
            }
        }
        // `main` is emitted once, into the entry module's object.
        if self.only_module.as_deref().unwrap_or("main") == "main" {
            self.emit_c_main()?;
        }
        Ok(())
    }

    /// Effect declarations, in a pass of their own.
    ///
    /// An operation's signature may name a type declared in another module,
    /// and doing this inside `collect_types` meant the answer depended on
    /// which file happened to be compiled first.
    fn collect_effects(&mut self, module_ast: &AstModule) -> CResult<()> {
        for decl in &module_ast.decls {
            if let Decl::Effect(e) = decl {
                let mut ops = Vec::new();
                for op in &e.ops {
                    let mut pts = Vec::new();
                    for p in &op.params {
                        pts.push(self.vtype_of(&p.ty)?);
                    }
                    let mut rt = match &op.ret {
                        Some(t) => self.vtype_of(t)?,
                        None => VType::Unit,
                    };
                    // An operation may declare an error type, in which case
                    // its result is fallible like any other — the checker
                    // treats it that way, so the backend has to as well or
                    // `try Cfg.get(x)` is rejected here and accepted there.
                    if let Some(et) = &op.err {
                        let e = self.vtype_of(et)?;
                        rt = VType::Fallible(Box::new(rt), Box::new(e));
                    }
                    ops.push((op.name.name.clone(), pts, rt));
                }
                self.effects.insert(e.name.name.clone(), ops);
            }
        }
        Ok(())
    }

    fn collect_types(&mut self, module_ast: &AstModule) -> CResult<()> {
        // The prelude's `IoError`, read from the checker's own table rather
        // than copied. Nobody declares it in source, so nothing else here would
        // ever register it — and matching its variants was a build-time
        // refusal for that reason alone, in a language where every `io` call
        // hands one back.
        if !self.sums.contains_key("IoError") {
            let mut vs = Vec::new();
            for (name, _) in grasm_sema::types::IO_ERROR_VARIANTS {
                self.variant_index
                    .insert((*name).to_string(), ("IoError".to_string(), 1));
                vs.push(((*name).to_string(), vec![VType::Str]));
            }
            self.sums.insert("IoError".to_string(), vs);
        }

        // Pass 0: type declarations (structs + sums; aliases unsupported).
        for decl in &module_ast.decls {
            if let Decl::Type(t) = decl {
                if !t.generics.is_empty() {
                    // Kept, not refused. A generic type has no layout until a
                    // use site says what its parameters are, exactly as a
                    // generic function has no symbol until a call site does.
                    let gkey = format!("{}::{}", self.current_module, t.name.name);
                    if let TypeBody::Sum(variants) = &t.body {
                        for v in variants {
                            self.generic_variant_owner
                                .insert(v.name.name.clone(), gkey.clone());
                        }
                    }
                    self.generic_types.insert(gkey, t.clone());
                    continue;
                }
                match &t.body {
                    TypeBody::Struct(_) => {
                        self.structs.insert(t.name.name.clone(), Vec::new());
                    }
                    TypeBody::Sum(_) => {
                        self.sums.insert(t.name.name.clone(), Vec::new());
                    }
                    TypeBody::Alias(te) => {
                        // Recorded, not rejected: `vtype_of` resolves an alias
                        // to its target, so nothing downstream sees one.
                        self.aliases.insert(t.name.name.clone(), te.clone());
                    }
                }
            }
        }
        Ok(())
    }

    /// Fills in field types, once every module's type *names* are registered.
    ///
    /// A field's type may live in any module, so resolving bodies inside
    /// `collect_types` made the answer depend on which file was compiled
    /// first — the same ordering the checker had.
    fn resolve_type_bodies(&mut self, module_ast: &AstModule) -> CResult<()> {
        for decl in &module_ast.decls {
            if let Decl::Type(t) = decl {
                // A generic declaration has no body to resolve until it is
                // instantiated; `instantiate_type` resolves one per set of
                // arguments.
                if !t.generics.is_empty() {
                    continue;
                }
                match &t.body {
                    TypeBody::Struct(fields) => {
                        let mut fs = Vec::new();
                        for f in fields {
                            fs.push((f.name.name.clone(), self.vtype_of(&f.ty)?));
                        }
                        self.structs.insert(t.name.name.clone(), fs);
                    }
                    TypeBody::Sum(variants) => {
                        let mut vs = Vec::new();
                        for v in variants {
                            let mut fts = Vec::new();
                            for f in &v.fields {
                                fts.push(self.vtype_of(&f.ty)?);
                            }
                            self.variant_index.insert(
                                v.name.name.clone(),
                                (t.name.name.clone(), v.fields.len()),
                            );
                            vs.push((v.name.name.clone(), fts));
                        }
                        self.sums.insert(t.name.name.clone(), vs);
                    }
                    TypeBody::Alias(te) => {
                        self.aliases.insert(t.name.name.clone(), te.clone());
                    }
                }
            }
        }
        Ok(())
    }

    /// Binds a generic function's type parameters by matching its declared
    /// parameter types against the concrete types of the arguments at a call
    /// site.
    ///
    /// Only what the arguments determine can be inferred, which is exactly the
    /// checker's rule too; a parameter that appears solely in the return type
    /// leaves nothing to instantiate and is reported rather than guessed.
    fn infer_generic_subst(
        &self,
        decl: &FnDecl,
        arg_types: &[VType],
        span: Span,
    ) -> CResult<HashMap<String, VType>> {
        let names: std::collections::HashSet<&str> =
            decl.generics.iter().map(|g| g.name.as_str()).collect();
        let mut subst = HashMap::new();
        for (p, actual) in decl.params.iter().zip(arg_types) {
            unify_generic_with(&p.ty, actual, &names, &mut subst, &self.type_args);
        }
        for g in &decl.generics {
            if !subst.contains_key(&g.name) {
                return nope(
                    span,
                    format!(
                        "calling `{}` — `{}` is not determined by the arguments",
                        decl.name.name, g.name
                    ),
                );
            }
        }
        Ok(subst)
    }

    /// Declares (once) the instantiation of `name` for `arg_types` and returns
    /// its key in `self.fns`.
    /// One member of a generic type family, named for its arguments.
    ///
    /// `Pair[i64, str]` becomes a struct called `Pair[i64,str]` with its fields
    /// resolved under that binding. The name is what every later stage keys on
    /// — field lookup, equality, display — so nothing downstream has to know
    /// that generic types exist.
    fn instantiate_type(
        &mut self,
        module: &str,
        name: &str,
        args: &[VType],
        span: Span,
    ) -> CResult<String> {
        let generic_key = format!("{module}::{name}");
        let Some(decl) = self.generic_types.get(&generic_key).cloned() else {
            return nope(span, format!("the type `{name}` here"));
        };
        if decl.generics.len() != args.len() {
            return nope(
                span,
                format!(
                    "`{name}` takes {} type argument(s) and was given {}",
                    decl.generics.len(),
                    args.len()
                ),
            );
        }
        if let Some(deep) = args.iter().find(|t| t.depth() > MAX_TYPE_DEPTH) {
            return nope(
                span,
                format!(
                    "a generic type whose argument keeps growing (`{name}` here, reached a type nested {} deep)",
                    deep.depth()
                ),
            );
        }
        let suffix: Vec<String> = args.iter().map(VType::mangle).collect();
        let key = format!("{name}[{}]", suffix.join(","));
        self.type_args
            .insert(key.clone(), (name.to_string(), args.to_vec()));
        if self.structs.contains_key(&key) || self.sums.contains_key(&key) {
            return Ok(key);
        }
        let subst: HashMap<String, VType> = decl
            .generics
            .iter()
            .map(|g| g.name.clone())
            .zip(args.iter().cloned())
            .collect();
        // Registered before the body is resolved, so a type that mentions
        // itself — `type Node[T] = { next: Option[Node[T]] }` — finds the name
        // rather than recursing forever.
        match &decl.body {
            TypeBody::Struct(_) => {
                self.structs.insert(key.clone(), Vec::new());
            }
            TypeBody::Sum(_) => {
                self.sums.insert(key.clone(), Vec::new());
            }
            TypeBody::Alias(_) => {}
        }
        let saved = std::mem::replace(&mut self.generic_subst, subst);
        let resolved = (|| -> CResult<()> {
            match &decl.body {
                TypeBody::Struct(fields) => {
                    let mut fs = Vec::new();
                    for f in fields {
                        fs.push((f.name.name.clone(), self.vtype_of(&f.ty)?));
                    }
                    self.structs.insert(key.clone(), fs);
                }
                TypeBody::Sum(variants) => {
                    let mut vs = Vec::new();
                    for v in variants {
                        let mut fts = Vec::new();
                        for f in &v.fields {
                            fts.push(self.vtype_of(&f.ty)?);
                        }
                        // Deliberately *not* in `variant_index`. That maps a
                        // variant name to one parent, and a generic sum has one
                        // parent per instantiation — registering them made
                        // `Has("text")` find the `Opt[i64]` that `Has(5)` had
                        // left there, build an `Opt[i64]` holding a string, and
                        // print its address as an integer. Which member a
                        // constructor names is a question for the use site.
                        vs.push((v.name.name.clone(), fts));
                    }
                    self.sums.insert(key.clone(), vs);
                }
                TypeBody::Alias(te) => {
                    let t = self.vtype_of(te)?;
                    self.aliases.insert(key.clone(), grasm_syntax::ast::TypeExpr::Name {
                        path: grasm_syntax::ast::Path {
                            segments: vec![Ident { name: t.show(), span }],
                            span,
                        },
                        args: Vec::new(),
                        span,
                    });
                }
            }
            Ok(())
        })();
        self.generic_subst = saved;
        resolved?;
        Ok(key)
    }

    fn instantiate_generic(
        &mut self,
        module: &str,
        name: &str,
        arg_types: &[VType],
        span: Span,
    ) -> CResult<String> {
        let generic_key = format!("{module}::{name}");
        let Some(decl) = self.generic_fns.get(&generic_key).cloned() else {
            return nope(span, format!("the function `{name}` here"));
        };
        if decl.params.len() != arg_types.len() {
            return nope(span, format!("calling `{name}` with a different arity"));
        }
        let subst = self.infer_generic_subst(&decl, arg_types, span)?;
        // A type argument that keeps growing is polymorphic recursion, and
        // one machine copy per type means the worklist never empties.
        if let Some(deep) = subst.values().find(|t| t.depth() > MAX_TYPE_DEPTH) {
            return nope(
                span,
                format!(
                    "a generic function whose type argument keeps growing (`{name}` here, reached a type nested {} deep) — polymorphic recursion needs one compiled copy per type, and there is no end to them",
                    deep.depth()
                ),
            );
        }

        let suffix: Vec<String> = decl
            .generics
            .iter()
            .map(|g| subst[&g.name].mangle())
            .collect();
        let key = format!("{generic_key}[{}]", suffix.join(","));
        if self.fns.contains_key(&key) {
            return Ok(key);
        }

        // Resolve the signature with the type parameters bound.
        let saved = std::mem::replace(&mut self.generic_subst, subst.clone());
        let signature = (|| -> CResult<(Vec<VType>, Vec<ParamMode>, VType)> {
            let mut params = Vec::new();
            let mut modes = Vec::new();
            for p in &decl.params {
                params.push(self.vtype_of(&p.ty)?);
                modes.push(p.mode);
            }
            let mut ret = match &decl.ret {
                Some(t) => self.vtype_of(t)?,
                None => VType::Unit,
            };
            if let Some(et) = &decl.err {
                let e = self.vtype_of(et).unwrap_or(VType::Str);
                ret = VType::Fallible(Box::new(ret), Box::new(e));
            }
            Ok((params, modes, ret))
        })();
        self.generic_subst = saved;
        let (params, modes, ret) = signature?;

        let ptr_ty = self.module.target_config().pointer_type();
        let mut sig = self.module.make_signature();
        for (p, m) in params.iter().zip(&modes) {
            sig.params.push(AbiParam::new(if *m == ParamMode::Inout {
                ptr_ty
            } else {
                p.cl()
            }));
        }
        if ret != VType::Unit {
            sig.returns.push(AbiParam::new(ret.cl()));
        }
        let sym = format!(
            "g_{}_{}__{}",
            module.replace('.', "_"),
            name,
            suffix.join("_")
        );
        let id = self
            .module
            .declare_function(&sym, Linkage::Local, &sig)
            .map_err(|e| Unsupported {
                span,
                what: format!("(module error: {e})"),
            })?;
        self.fns.insert(
            key.clone(),
            FnSig {
                is_extern: false,
                params,
                modes,
                ret,
                id,
            },
        );
        self.pending_generics.push(PendingGeneric {
            key: key.clone(),
            module: module.to_string(),
            decl,
            subst,
        });
        Ok(key)
    }

    /// Linkage for a function defined in `current_module`.
    ///
    /// Whole-program builds keep everything local. A per-module build exports
    /// what it defines and imports the rest, so the objects link together.
    fn linkage_for_current(&self) -> Linkage {
        match &self.only_module {
            None => Linkage::Local,
            Some(m) if *m == self.current_module => Linkage::Export,
            Some(_) => Linkage::Import,
        }
    }

    fn emitting_current(&self) -> bool {
        match &self.only_module {
            None => true,
            Some(m) => *m == self.current_module,
        }
    }

    fn declare_fns(&mut self, module_ast: &AstModule) -> CResult<()> {
        for decl in &module_ast.decls {
            match decl {
                // A C function: declared as an imported symbol and called
                // directly. The linker resolves it against libc or whatever
                // else the platform provides.
                Decl::Extern(e) => {
                    let mut sig = self.module.make_signature();
                    let mut param_ts = Vec::new();
                    for p in &e.params {
                        let vt = self.vtype_of(&p.ty)?;
                        sig.params.push(AbiParam::new(vt.cl()));
                        param_ts.push(vt);
                    }
                    let ret_t = match &e.ret {
                        Some(t) => self.vtype_of(t)?,
                        None => VType::Unit,
                    };
                    if ret_t != VType::Unit {
                        sig.returns.push(AbiParam::new(ret_t.cl()));
                    }
                    let id = self
                        .module
                        .declare_function(&e.name.name, Linkage::Import, &sig)
                        .map_err(|err| Unsupported {
                            span: e.span,
                            what: format!("declaring `{}`: {err}", e.name.name),
                        })?;
                    self.fns.insert(
                        format!("{}::{}", self.current_module, e.name.name),
                        FnSig {
                            id,
                            is_extern: true,
                            params: param_ts,
                            modes: e.params.iter().map(|p| p.mode).collect(),
                            ret: ret_t,
                        },
                    );
                }
                // Constants are inlined at each use, so there is no symbol to
                // declare — see `const_inits`.
                Decl::Const(c) => {
                    self.consts
                        .insert(format!("{}::{}", self.current_module, c.name.name), c.clone());
                }
                Decl::Fn(f) => {
                    if !f.generics.is_empty() {
                        // No signature until a call site says what the type
                        // parameters are.
                        self.generic_fns.insert(
                            format!("{}::{}", self.current_module, f.name.name),
                            f.clone(),
                        );
                        continue;
                    }
                    let mut params = Vec::new();
                    let mut modes = Vec::new();
                    for p in &f.params {
                        // `own` and `let` share one native ABI: the caller's
                        // +1 transfers to the callee, which releases at exit.
                        // The distinction is purely static (use-after-move is
                        // a checker error).
                        params.push(self.vtype_of(&p.ty)?);
                        modes.push(p.mode);
                    }
                    let mut ret = match &f.ret {
                        Some(t) => self.vtype_of(t)?,
                        None => VType::Unit,
                    };
                    if let Some(et) = &f.err {
                        let e = self.vtype_of(et).unwrap_or(VType::Str);
                        ret = VType::Fallible(Box::new(ret), Box::new(e));
                    }
                    let ptr_ty = self.module.target_config().pointer_type();
                    let mut sig = self.module.make_signature();
                    for (p, m) in params.iter().zip(&modes) {
                        sig.params.push(AbiParam::new(if *m == ParamMode::Inout {
                            ptr_ty
                        } else {
                            p.cl()
                        }));
                    }
                    if ret != VType::Unit {
                        sig.returns.push(AbiParam::new(ret.cl()));
                    }
                    let sym = format!(
                        "g_{}_{}",
                        self.current_module.replace('.', "_"),
                        f.name.name
                    );
                    let linkage = self.linkage_for_current();
                    let id = self
                        .module
                        .declare_function(&sym, linkage, &sig)
                        .map_err(|e| Unsupported {
                            span: f.span,
                            what: format!("(module error: {e})"),
                        })?;
                    let key = format!("{}::{}", self.current_module, f.name.name);
                    self.fns.insert(
                        key,
                        FnSig {
                            is_extern: false,
                            params,
                            modes,
                            ret,
                            id,
                        },
                    );
                }
                Decl::Test(_) | Decl::Bench(_) | Decl::Import(_) | Decl::Type(_) | Decl::Effect(_) => {}
            }
        }
        Ok(())
    }

    fn emit_c_main(&mut self) -> CResult<()> {
        let ptr_ty = self.module.target_config().pointer_type();
        let mut sig = self.module.make_signature();
        sig.params.push(AbiParam::new(types::I32));
        sig.params.push(AbiParam::new(ptr_ty));
        sig.returns.push(AbiParam::new(types::I32));
        let main_id = self
            .module
            .declare_function("main", Linkage::Export, &sig)
            .map_err(|e| Unsupported {
                span: Span::new(0, 1),
                what: format!("(module error: {e})"),
            })?;
        let mut ctx = self.module.make_context();
        ctx.func.signature = sig;
        let mut fbcx = FunctionBuilderContext::new();
        let mut b = FunctionBuilder::new(&mut ctx.func, &mut fbcx);
        let entry = b.create_block();
        b.append_block_params_for_function_params(entry);
        b.switch_to_block(entry);
        // Publish argv (skipping argv[0]) for the `args()` builtin.
        let params: Vec<Value> = b.block_params(entry).to_vec();
        let argc = b.ins().sextend(types::I64, params[0]);
        let one = b.ins().iconst(types::I64, 1);
        let user_argc = b.ins().isub(argc, one);
        let eight = b.ins().iconst(types::I64, 8);
        let argv_shift = b.ins().iadd(params[1], eight);
        let set_args = self.runtime["grasm_set_args"];
        let sa_ref = self.module.declare_func_in_func(set_args, b.func);
        b.ins().call(sa_ref, &[user_argc, argv_shift]);
        let g_main = self.fns["main::main"].id;
        let main_fallible = matches!(self.fns["main::main"].ret, VType::Fallible(_, _));
        let fref = self.module.declare_func_in_func(g_main, b.func);
        let inst = b.ins().call(fref, &[]);
        if main_fallible {
            let res = b.inst_results(inst)[0];
            let ok_blk = b.create_block();
            let err_blk = b.create_block();
            let is_ok_id = self.runtime["grasm_is_ok"];
            let is_ok_ref = self.module.declare_func_in_func(is_ok_id, b.func);
            let c_inst = b.ins().call(is_ok_ref, &[res]);
            let c = b.inst_results(c_inst)[0];
            b.ins().brif(c, ok_blk, &[], err_blk, &[]);
            b.switch_to_block(err_blk);
            let rep_id = self.runtime["grasm_report_main_err"];
            let rep_ref = self.module.declare_func_in_func(rep_id, b.func);
            b.ins().call(rep_ref, &[res]);
            b.ins().trap(cranelift_codegen::ir::TrapCode::unwrap_user(1));
            b.switch_to_block(ok_blk);
            // The `Ok(())` a fallible `main` returns is an allocation like any
            // other, and nothing released it — so every program whose entry
            // point declares an error type leaked one, forever, in a place no
            // amount of looking at the *program* would find.
            let dec_id = self.runtime["grasm_val_dec"];
            let dec_ref = self.module.declare_func_in_func(dec_id, b.func);
            b.ins().call(dec_ref, &[res]);
        }
        // Leak check, silent unless `GRASM_LEAK_CHECK` is set. A leak
        // produces exactly the right output, so the differential harness
        // cannot see one; this is what makes it observable.
        let leak_id = self.runtime["grasm_check_leaks"];
        let leak_ref = self.module.declare_func_in_func(leak_id, b.func);
        b.ins().call(leak_ref, &[]);
        let zero = b.ins().iconst(types::I32, 0);
        b.ins().return_(&[zero]);
        b.seal_all_blocks();
        b.finalize();
        self.module
            .define_function(main_id, &mut ctx)
            .map_err(|e| Unsupported {
                span: Span::new(0, 1),
                // A verifier failure is a compiler bug, not a missing
                // feature, and reporting it through the "unsupported" channel
                // tells a reader to use the interpreter for a construct that
                // has no name. Say which it is.
                what: format!("{INTERNAL}  {e}"),
            })?;
        Ok(())
    }

    fn compile_fn(&mut self, f: &FnDecl) -> CResult<()> {
        // Generic declarations have no signature of their own; their
        // instantiations are compiled from the worklist.
        if !f.generics.is_empty() {
            return Ok(());
        }
        let key = format!("{}::{}", self.current_module, f.name.name);
        self.compile_fn_as(f, &key)
    }

    fn compile_fn_as(&mut self, f: &FnDecl, key: &str) -> CResult<()> {
        let key = key.to_string();
        let sig_params = self.fns[&key].params.clone();
        let sig_modes = self.fns[&key].modes.clone();
        let sig_ret = self.fns[&key].ret.clone();
        let func_id = self.fns[&key].id;
        let ptr_ty = self.module.target_config().pointer_type();

        let mut ctx = self.module.make_context();
        for (p, m) in sig_params.iter().zip(&sig_modes) {
            ctx.func.signature.params.push(AbiParam::new(
                if *m == ParamMode::Inout { ptr_ty } else { p.cl() },
            ));
        }
        if sig_ret != VType::Unit {
            ctx.func.signature.returns.push(AbiParam::new(sig_ret.cl()));
        }

        let mut fbcx = FunctionBuilderContext::new();
        let builder = FunctionBuilder::new(&mut ctx.func, &mut fbcx);
        let mut fcx = FnCx {
            cg: self,
            b: builder,
            vars: Vec::new(),
            var_count: 0,
            loops: Vec::new(),
            handler_arg_held: Vec::new(),
            fail_redirect: Vec::new(),
            ret: sig_ret.clone(),
            inout_slots: Vec::new(),
            filled: false,
            closure_hints: Vec::new(),
            pinned: Vec::new(),
            tail_hint: None,
            probe_types: Vec::new(),
            probing_args: false,
            index_vals: HashMap::new(),
            wrote_through_ref: false,
            task_scopes: Vec::new(),
        };

        let entry = fcx.b.create_block();
        fcx.b.append_block_params_for_function_params(entry);
        fcx.b.switch_to_block(entry);
        // A function that can recurse checks how much stack is left.
        // Cranelift emits no such check, so a deep enough recursion took
        // SIGSEGV with nothing reported, while [E-RECURSION-DEPTH] requires a
        // panic and exit 101 and says outright that it "does not abort,
        // crash, or corrupt the process". Only recursive functions pay for
        // it, and the address of a scratch slot stands in for the stack
        // pointer.
        if fcx.cg.recursive.contains(&key) {
            let slot = fcx.b.create_sized_stack_slot(
                cranelift_codegen::ir::StackSlotData::new(
                    cranelift_codegen::ir::StackSlotKind::ExplicitSlot,
                    8,
                    3,
                ),
            );
            let here = fcx.b.ins().stack_addr(ptr_ty, slot, 0);
            fcx.call_rt("grasm_stack_check", &[here]);
        }
        fcx.push_scope();
        let entry_params: Vec<Value> = fcx.b.block_params(entry).to_vec();
        for (i, p) in f.params.iter().enumerate() {
            let vt = sig_params[i].clone();
            if sig_modes[i] == ParamMode::Inout {
                // Load through the caller's pointer; store back at exit.
                let loaded = fcx.b.ins().load(
                    vt.cl(),
                    cranelift_codegen::ir::MemFlags::new(),
                    entry_params[i],
                    0,
                );
                let var = fcx.new_var(&vt);
                fcx.b.def_var(var, loaded);
                fcx.bind(&p.name.name, var, vt.clone());
                fcx.inout_slots.push((entry_params[i], var, vt));
            } else {
                let var = fcx.new_var(&vt);
                fcx.b.def_var(var, entry_params[i]);
                fcx.bind(&p.name.name, var, vt);
            }
        }

        fcx.tail_hint = match &sig_ret {
            // Unit produces no value, so an expectation of it would force
            // one where there is none.
            VType::Fallible(ok, _) if **ok != VType::Unit => Some((**ok).clone()),
            VType::Fallible(_, _) | VType::Unit => None,
            other => Some(other.clone()),
        };
        let (last, terminated) = fcx.eval_block_inner(&f.body)?;
        // A body whose last statement is a bare `{ … return … }` block leaves
        // through that `return`, and the block it left in is already closed —
        // but `eval_block_inner` reports the *statement* as non-terminating,
        // so the epilogue added a second terminator and Cranelift refused the
        // function. The refusal named nothing ("codegen bug in `main`:
        // Verifier errors") and covered a quarter of every well-typed program
        // containing a `return`.
        let terminated = terminated || fcx.is_filled();
        if !terminated {
            match (&sig_ret, last) {
                (VType::Unit, last) => {
                    // A discarded fallible tail in `main`. Everywhere else
                    // the checker refuses one; `main` may propagate, and
                    // [E-MAIN-ERR] says the failure prints and exits 1.
                    // Dropped instead, `fn main() { write_file(p, s) }`
                    // printed the line before it, exited 0, wrote nothing to
                    // stderr and left no file — in the compiled build only.
                    if let Some((v, vt)) = &last {
                        if matches!(vt, VType::Fallible(..)) {
                            let v = *v;
                            let ok_blk = fcx.b.create_block();
                            let err_blk = fcx.b.create_block();
                            let c = fcx.call_rt("grasm_is_ok", &[v]).unwrap();
                            fcx.b.ins().brif(c, ok_blk, &[], err_blk, &[]);
                            fcx.goto_block(err_blk);
                            fcx.call_rt("grasm_report_main_err", &[v]);
                            fcx.trap_here();
                            fcx.goto_block(ok_blk);
                        }
                    }
                    // Unused trailing boxed value must be released.
                    if let Some((v, vt)) = last {
                        if vt.boxed() {
                            fcx.call_rt("grasm_val_dec", &[v]);
                        }
                    }
                    fcx.emit_scope_decs_all();
                    fcx.b.ins().return_(&[]);
                }
                (want, Some((mut v, vt))) => {
                    // A body that already produced a `T ! E` is the result;
                    // wrapping it again would make an `Ok(Ok(..))`. Falls
                    // through to the shared return below.
                    let already_fallible =
                        matches!(want, VType::Fallible(..)) && matches!(vt, VType::Fallible(..));
                    if !already_fallible {
                        if let VType::Fallible(ok_t, _) = want.clone() {
                            let mut inner = v;
                            if *ok_t == VType::F64 && vt == VType::I64 {
                                inner = fcx.b.ins().fcvt_from_sint(types::F64, inner);
                            }
                            let boxed = fcx.box_value(inner, &ok_t);
                            v = fcx.call_rt("grasm_ok_new", &[boxed]).unwrap();
                        } else if *want == VType::F64 && vt == VType::I64 {
                            v = fcx.b.ins().fcvt_from_sint(types::F64, v);
                        }
                    }
                    fcx.emit_scope_decs_all();
                    fcx.b.ins().return_(&[v]);
                }
                (VType::Fallible(_, _), None) => {
                    // Unit-valued fallible function: Ok(()).
                    let z = fcx.b.ins().iconst(types::I64, 0);
                    let boxed = fcx.call_rt("grasm_box_i64", &[z]).unwrap();
                    let v = fcx.call_rt("grasm_ok_new", &[boxed]).unwrap();
                    fcx.emit_scope_decs_all();
                    fcx.b.ins().return_(&[v]);
                }
                (want, None) => {
                    // A body that leaves — `panic(m)`, `exit(n)` — produces no
                    // value because control never reaches the end. The trap is
                    // already emitted and what follows is a dead block, so the
                    // function needs a terminator and nothing else:
                    // `fn unreachable() -> T { panic(…) }` is ordinary code and
                    // §11 lists no row for it, so refusing it made the gap
                    // table wrong either way.
                    let z = fcx.zero_of(want);
                    fcx.b.ins().return_(&[z]);
                }
            }
        }
        fcx.b.seal_all_blocks();
        fcx.b.finalize();

        // The IR a function compiled to, for reading what the backend actually
        // emitted rather than guessing from a timing. On demand, because it is
        // thousands of lines for a real program.
        if std::env::var("GRASM_DUMP_IR").is_ok() {
            eprintln!("--- {} ---\n{}", f.name.name, ctx.func);
        }
        self.module
            .define_function(func_id, &mut ctx)
            .map_err(|e| Unsupported {
                span: f.span,
                what: {
                    if std::env::var("GRASM_DUMP_IR").is_ok() {
                        eprintln!(
                            "--- verifier detail for `{}` ---\n{:?}\n--- IR ---\n{}",
                            f.name.name, e, ctx.func
                        );
                    }
                    format!("(codegen bug in `{}`: {e})", f.name.name)
                },
            })?;
        Ok(())
    }

    // ------------------------------------------------------------------
    // N5: effect handler lifting
    // ------------------------------------------------------------------

    fn collect_handlers_block(&mut self, b: &grasm_syntax::ast::Block) -> CResult<()> {
        for st in &b.stmts {
            // A handler bound to a name before `handled_by` uses it. This pass
            // lifts each op's closure into its own function, so it needs the
            // literal itself, not the name standing for it. Statements are
            // walked in order, so the binding is always recorded first.
            if let Stmt::Let { name, init, .. } = st {
                if matches!(init, Expr::StructLit { .. }) {
                    self.handler_lits.insert(name.name.clone(), init.clone());
                }
            }
            self.collect_handlers_stmt(st)?;
        }
        Ok(())
    }

    /// Whether an expression is a handler struct literal — `EHandler{ .. }`
    /// for a declared effect `E`.
    fn is_handler_literal(&self, e: &Expr) -> bool {
        let Expr::StructLit { name, .. } = e else {
            return false;
        };
        let n = &name.segments[0].name;
        match n.strip_suffix("Handler") {
            Some(eff) => self.effects.contains_key(eff),
            None => false,
        }
    }

    /// The struct literal a handler expression stands for, if it is a name
    /// bound to one.
    fn handler_literal(&self, handler: &Expr) -> Option<Expr> {
        match handler {
            Expr::StructLit { .. } => Some(handler.clone()),
            Expr::Ident(id) => self.handler_lits.get(&id.name).cloned(),
            _ => None,
        }
    }

    /// Lifts a handler literal's operations into top-level functions.
    ///
    /// Shared by a `with` site and by a handler literal passed as an argument,
    /// because they need exactly the same thing: one function per operation,
    /// plus the names its body reads from outside itself. Two copies of this
    /// would be two places for the capture list to drift, and a capture list
    /// that drifts is a handler reading the wrong value.
    fn lift_handler_ops(
        &mut self,
        ename: &str,
        ops: &[(String, Vec<VType>, VType)],
        fields: &[(Ident, Expr)],
        span: Span,
    ) -> CResult<Vec<(String, HandlerOpCode, Vec<String>)>> {
        let mut site = Vec::new();
        for (fname, fexpr) in fields {
            let Some((_, param_ts, ret_t)) =
                ops.iter().find(|(o, _, _)| *o == fname.name).cloned()
            else {
                return nope(fname.span, format!("the operation `{}`", fname.name));
            };
            // A named function as an operation. Function values already travel
            // as `(env, params…)`, and the wrapper that gives a named function
            // that shape is the same one `[inc, inc]` uses — so the operation
            // is that wrapper with an environment it never reads. Writing the
            // closure out by hand was the advice, and it produced a different
            // program only in how much of it there was.
            let Expr::Closure {
                params: cparams,
                body: cbody,
                ..
            } = fexpr
            else {
                if let Expr::Ident(fid) = fexpr {
                    let key = format!("{}::{}", self.current_module, fid.name);
                    if let Some(sig) = self.fns.get(&key).cloned() {
                        if sig.params.len() != param_ts.len() || sig.ret != ret_t {
                            return nope(
                                fexpr.span(),
                                format!(
                                    "`{}` as the operation `{}` — its type is not the \
operation's",
                                    fid.name, fname.name
                                ),
                            );
                        }
                        // Recorded, not emitted. This pass runs before any body
                        // is compiled, and emitting a function from inside it
                        // left the next declaration compiling against a
                        // half-written module: one named operation worked and
                        // the second crashed the program it produced. The
                        // wrapper is built where every other one is, when the
                        // handler is installed.
                        site.push((
                            fname.name.clone(),
                            HandlerOpCode::Named(key),
                            Vec::new(),
                        ));
                        continue;
                    }
                }
                return nope(fexpr.span(), "handler ops that aren't closures");
            };
            if cparams.len() != param_ts.len() {
                return nope(fexpr.span(), "a handler op with the wrong arity");
            }
            let mut sig = self.module.make_signature();
            for p in &param_ts {
                sig.params.push(AbiParam::new(p.cl()));
            }
            if ret_t != VType::Unit {
                sig.returns.push(AbiParam::new(ret_t.cl()));
            }
            let sym = format!(
                "g_h_{}_{}_{}",
                ename.replace('.', "_"),
                fname.name,
                self.pending_handlers.len()
            );
            let id = self
                .module
                .declare_function(&sym, Linkage::Local, &sig)
                .map_err(|e| Unsupported {
                    span,
                    what: format!("(module error: {e})"),
                })?;
            let named_params: Vec<(String, VType)> = cparams
                .iter()
                .map(|p| p.name.clone())
                .zip(param_ts.iter().cloned())
                .collect();
            // Names the body reads that are neither its parameters nor
            // anything global. Their types are unknown at this pass —
            // it runs before any function body is compiled — so they
            // are recorded by name and typed at the `with` site.
            let mut free = Vec::new();
            collect_idents(cbody, &mut free);
            free.retain(|n| !cparams.iter().any(|p| p.name == *n));
            free.dedup();
            self.pending_handlers.push(PendingHandler {
                id,
                module: self.current_module.clone(),
                params: named_params,
                captures: Vec::new(),
                ret: ret_t,
                body: (**cbody).clone(),
            });
            site.push((fname.name.clone(), HandlerOpCode::Lifted(id), free));
        }
        Ok(site)
    }

    fn collect_handlers_stmt(&mut self, st: &Stmt) -> CResult<()> {
        match st {
            Stmt::With {
                effect,
                handler,
                body,
                span,
            } => {
                let ename = effect.dotted();
                let Some(ops) = self.effects.get(&ename).cloned() else {
                    return nope(*span, format!("the effect `{ename}`"));
                };
                let resolved = self.handler_literal(handler);
                let Some(Expr::StructLit { fields, .. }) = &resolved else {
                    // No literal to lift *here*. That is not yet an error: the
                    // handler may have arrived as a value, whose operations
                    // were lifted where the literal was written. This pass runs
                    // before any body is typed and cannot tell the two apart,
                    // so it says nothing and leaves the judgement to the point
                    // that knows — which either finds a handler value or
                    // refuses by name.
                    return self.collect_handlers_block(body);
                };
                let site = self.lift_handler_ops(&ename, &ops, fields, *span)?;
                self.handler_sites.insert(span.start, site);
                self.collect_handlers_block(body)?;
                Ok(())
            }
            Stmt::While { body, .. } | Stmt::For { body, .. } => self.collect_handlers_block(body),
            Stmt::Expr(e) | Stmt::Let { init: e, .. } => self.collect_handlers_expr(e),
            Stmt::Assign { value, .. } => self.collect_handlers_expr(value),
            Stmt::Return(Some(e), _) => self.collect_handlers_expr(e),
            _ => Ok(()),
        }
    }

    fn collect_handlers_expr(&mut self, e: &Expr) -> CResult<()> {
        match e {
            Expr::Block(b) | Expr::Unsafe(b) | Expr::Region(b) => self.collect_handlers_block(b),
            Expr::If {
                then_block,
                else_branch,
                ..
            } => {
                self.collect_handlers_block(then_block)?;
                match else_branch.as_deref() {
                    Some(ElseBranch::Else(b)) => self.collect_handlers_block(b),
                    Some(ElseBranch::ElseIf(e)) => self.collect_handlers_expr(e),
                    None => Ok(()),
                }
            }
            Expr::Match { arms, .. } => {
                for a in arms {
                    self.collect_handlers_expr(&a.body)?;
                }
                Ok(())
            }
            // A handler literal passed as an argument. Its operations are
            // lifted here, keyed by where the literal is written, so the call
            // site can hand the callee code pointers rather than a syntax tree
            // it has no way to compile.
            Expr::Call { args, .. } | Expr::MethodCall { args, .. } => {
                for a in args {
                    // Resolved through a name, so a `let`-bound handler can be
                    // passed as well as written inline. Lifted here rather than
                    // at the `let`, because a handler that is only ever
                    // *installed* is lifted at its `with` — and lifting it in
                    // both places produced a second copy whose captures nothing
                    // registered, which failed to compile by name.
                    if let Some(lit) = self.handler_literal(a) {
                        if self.is_handler_literal(&lit) {
                            self.lift_handler_literal(&lit)?;
                        }
                    }
                    self.collect_handlers_expr(a)?;
                }
                Ok(())
            }
            _ => Ok(()),
        }
    }

    /// Lifts a handler literal written in a value position, keyed by its own
    /// span rather than by a `with`.
    fn lift_handler_literal(&mut self, lit: &Expr) -> CResult<()> {
        let Expr::StructLit { name, fields, span, .. } = lit else {
            return Ok(());
        };
        if self.handler_sites.contains_key(&span.start) {
            return Ok(());
        }
        let ename = name.segments[0]
            .name
            .strip_suffix("Handler")
            .unwrap_or_default()
            .to_string();
        let Some(ops) = self.effects.get(&ename).cloned() else {
            return Ok(());
        };
        let site = self.lift_handler_ops(&ename, &ops, fields, *span)?;
        self.handler_sites.insert(span.start, site);
        Ok(())
    }

    /// Emits the wrapper that lets a named function be used as a value.
    ///
    /// Function values all share one shape — `(env, params…)` — so a wrapper
    /// takes the environment it will never use and forwards the rest. One per
    /// function, cached, so `[inc, inc]` does not emit two.
    fn fn_value_wrapper(&mut self, key: &str, span: Span) -> CResult<FuncId> {
        self.wrapper_for(key, span, true)
    }

    /// The same wrapper, for a named function used as a *handler operation*.
    ///
    /// One difference, and it is not optional: a function value's environment
    /// is a reference the wrapper owns and releases, while a handler
    /// operation's is the address of a stack slot in the frame that installed
    /// it. Releasing that is a decrement through a pointer into a live frame —
    /// which produced the right answer and then SIGBUS, because the value came
    /// back before the damage was read.
    fn handler_op_wrapper(&mut self, key: &str, span: Span) -> CResult<FuncId> {
        self.wrapper_for(key, span, false)
    }

    fn wrapper_for(&mut self, key: &str, span: Span, owns_env: bool) -> CResult<FuncId> {
        let cache_key = if owns_env {
            key.to_string()
        } else {
            format!("{key}#handlerop")
        };
        let key_for_sym = cache_key.clone();
        if let Some(id) = self.fn_wrappers.get(&cache_key) {
            return Ok(*id);
        }
        let sig_info = self.fns[key].clone();
        if sig_info.modes.contains(&ParamMode::Inout) {
            return nope(span, "`inout` functions as values");
        }
        let ptr_ty = self.module.target_config().pointer_type();
        let mut sig = self.module.make_signature();
        sig.params.push(AbiParam::new(ptr_ty)); // environment (unused)
        for p in &sig_info.params {
            sig.params.push(AbiParam::new(p.cl()));
        }
        if sig_info.ret != VType::Unit {
            sig.returns.push(AbiParam::new(sig_info.ret.cl()));
        }
        let sym = format!("gv_{}", key_for_sym.replace([':', '.', '[', ']', ',', '#'], "_"));
        let id = self
            .module
            .declare_function(&sym, Linkage::Local, &sig)
            .map_err(|e| Unsupported {
                span,
                what: format!("(module error: {e})"),
            })?;

        let mut ctx = self.module.make_context();
        ctx.func.signature = sig;
        let mut fbcx = FunctionBuilderContext::new();
        let mut b = FunctionBuilder::new(&mut ctx.func, &mut fbcx);
        let entry = b.create_block();
        b.append_block_params_for_function_params(entry);
        b.switch_to_block(entry);
        let params: Vec<Value> = b.block_params(entry).to_vec();
        let env = params[0];
        // A function value's environment is a reference this wrapper owns.
        // A handler operation's is a stack address, and releasing one of those
        // is a decrement through a live frame.
        if owns_env {
            let dec_ref =
                self.module.declare_func_in_func(self.runtime["grasm_val_dec"], b.func);
            b.ins().call(dec_ref, &[env]);
        } else {
            let _ = env;
        }
        let target = self.module.declare_func_in_func(sig_info.id, b.func);
        // A C function reached through a value is the same call as a direct
        // one, so a `str` or `bytes` parameter still crosses as a pointer
        // ([T-EXTERN-POINTER]). Without this the wrapper forwarded the boxed
        // value's own address: `[s].map(strlen)` handed C an `RtVal` to walk
        // and answered 1 for every string, while `grasm run` answered
        // correctly.
        let mut call_args: Vec<Value> = Vec::with_capacity(params.len() - 1);
        let mut c_temps: Vec<(Value, bool)> = Vec::new();
        for (i, p) in params[1..].iter().enumerate() {
            match sig_info.params.get(i) {
                Some(VType::Str) if sig_info.is_extern => {
                    let f = self
                        .module
                        .declare_func_in_func(self.runtime["grasm_str_c_new"], b.func);
                    let call = b.ins().call(f, &[*p]);
                    let ptr = b.inst_results(call)[0];
                    call_args.push(ptr);
                    c_temps.push((ptr, true));
                    c_temps.push((*p, false));
                }
                Some(VType::Bytes) if sig_info.is_extern => {
                    let f = self
                        .module
                        .declare_func_in_func(self.runtime["grasm_bytes_ptr"], b.func);
                    let call = b.ins().call(f, &[*p]);
                    call_args.push(b.inst_results(call)[0]);
                    c_temps.push((*p, false));
                }
                _ => call_args.push(*p),
            }
        }
        let inst = b.ins().call(target, &call_args);
        let results: Vec<Value> = b.inst_results(inst).to_vec();
        // The wrapper owns the arguments it was handed, and C released
        // nothing: free the string copies and release the boxes.
        for (v, is_copy) in c_temps {
            if is_copy {
                let f = self
                    .module
                    .declare_func_in_func(self.runtime["grasm_str_c_free"], b.func);
                b.ins().call(f, &[v]);
            } else {
                let f = self
                    .module
                    .declare_func_in_func(self.runtime["grasm_val_dec"], b.func);
                b.ins().call(f, &[v]);
            }
        }
        if sig_info.ret == VType::Unit {
            b.ins().return_(&[]);
        } else {
            b.ins().return_(&results);
        }
        b.seal_all_blocks();
        b.finalize();
        self.module
            .define_function(id, &mut ctx)
            .map_err(|e| Unsupported {
                span,
                // A verifier failure is a compiler bug, not a missing
                // feature, and reporting it through the "unsupported" channel
                // tells a reader to use the interpreter for a construct that
                // has no name. Say which it is.
                what: format!("{INTERNAL}  {e}"),
            })?;
        self.fn_wrappers.insert(cache_key, id);
        Ok(id)
    }

    /// Compiles a lifted `par_map` body: captured scalars arrive as an array
    /// of raw bits, the element as bits, and the result leaves as bits.
    fn compile_par_map(&mut self, c: PendingParMap) -> CResult<()> {
        let ptr_ty = self.module.target_config().pointer_type();
        let mut ctx = self.module.make_context();
        ctx.func.signature.params.push(AbiParam::new(ptr_ty));
        ctx.func.signature.params.push(AbiParam::new(types::I64));
        ctx.func.signature.returns.push(AbiParam::new(types::I64));

        let mut fbcx = FunctionBuilderContext::new();
        let builder = FunctionBuilder::new(&mut ctx.func, &mut fbcx);
        let mut fcx = FnCx {
            cg: self,
            b: builder,
            vars: Vec::new(),
            var_count: 0,
            loops: Vec::new(),
            handler_arg_held: Vec::new(),
            fail_redirect: Vec::new(),
            ret: c.ret.clone(),
            inout_slots: Vec::new(),
            filled: false,
            closure_hints: Vec::new(),
            pinned: Vec::new(),
            tail_hint: None,
            probe_types: Vec::new(),
            probing_args: false,
            index_vals: HashMap::new(),
            wrote_through_ref: false,
            task_scopes: Vec::new(),
        };
        let entry = fcx.b.create_block();
        fcx.b.append_block_params_for_function_params(entry);
        fcx.b.switch_to_block(entry);
        fcx.push_scope();
        let ps: Vec<Value> = fcx.b.block_params(entry).to_vec();

        for (i, (name, vt)) in c.captures.iter().enumerate() {
            let bits = fcx.b.ins().load(
                types::I64,
                cranelift_codegen::ir::MemFlags::trusted(),
                ps[0],
                (i * 8) as i32,
            );
            let v = fcx.bits_to_value(bits, vt);
            let var = fcx.new_var(vt);
            fcx.b.def_var(var, v);
            fcx.bind(name, var, vt.clone());
            // A boxed capture is *borrowed*: the caller holds it for the whole
            // of the parallel call, and every worker reads the same pointer.
            // The scope decs at the end of this body would release one
            // reference per element, so one is taken per element here. This is
            // the traffic that made the count have to be atomic.
            fcx.inc_if_boxed(v, vt);
        }
        let x = fcx.bits_to_value(ps[1], &c.param_t);
        let pvar = fcx.new_var(&c.param_t);
        fcx.b.def_var(pvar, x);
        fcx.bind(&c.param, pvar, c.param_t.clone());
        // As above: the element belongs to the input list, which outlives the
        // call, so this body borrows it and balances its own release.
        fcx.inc_if_boxed(x, &c.param_t);

        let val = fcx.eval_expr(&c.body)?;
        let Some((mut r, rt)) = val else {
            return nope(c.body.span(), "a par_map body with no value");
        };
        if c.ret == VType::F64 && rt == VType::I64 {
            r = fcx.b.ins().fcvt_from_sint(types::F64, r);
        }
        let bits = fcx.value_to_bits(r, &c.ret);
        fcx.emit_scope_decs_all();
        if !fcx.is_filled() {
            fcx.b.ins().return_(&[bits]);
        }
        fcx.b.seal_all_blocks();
        fcx.b.finalize();
        self.module
            .define_function(c.id, &mut ctx)
            .map_err(|e| Unsupported {
                span: c.body.span(),
                // A verifier failure is a compiler bug, not a missing
                // feature, and reporting it through the "unsupported" channel
                // tells a reader to use the interpreter for a construct that
                // has no name. Say which it is.
                what: format!("{INTERNAL}  {e}"),
            })?;
        let _ = c.module;
        Ok(())
    }

    /// Compiles a lifted closure body.
    fn compile_closure(&mut self, c: PendingClosure) -> CResult<()> {
        let ptr_ty = self.module.target_config().pointer_type();
        let mut ctx = self.module.make_context();
        ctx.func.signature.params.push(AbiParam::new(ptr_ty));
        for (_, t) in &c.params {
            ctx.func.signature.params.push(AbiParam::new(t.cl()));
        }
        if c.ret != VType::Unit {
            ctx.func.signature.returns.push(AbiParam::new(c.ret.cl()));
        }
        let mut fbcx = FunctionBuilderContext::new();
        let builder = FunctionBuilder::new(&mut ctx.func, &mut fbcx);
        let mut fcx = FnCx {
            cg: self,
            b: builder,
            vars: Vec::new(),
            var_count: 0,
            loops: Vec::new(),
            handler_arg_held: Vec::new(),
            fail_redirect: Vec::new(),
            ret: c.ret.clone(),
            inout_slots: Vec::new(),
            filled: false,
            closure_hints: Vec::new(),
            pinned: Vec::new(),
            tail_hint: None,
            probe_types: Vec::new(),
            probing_args: false,
            index_vals: HashMap::new(),
            wrote_through_ref: false,
            task_scopes: Vec::new(),
        };
        let entry = fcx.b.create_block();
        fcx.b.append_block_params_for_function_params(entry);
        fcx.b.switch_to_block(entry);
        fcx.push_scope();
        // A lifted body recurses through no name, so the call-graph analysis
        // that guards named functions never sees it.
        if body_can_call(&c.body) {
            let ptr_ty = fcx.cg.module.target_config().pointer_type();
            let slot = fcx.b.create_sized_stack_slot(
                cranelift_codegen::ir::StackSlotData::new(
                    cranelift_codegen::ir::StackSlotKind::ExplicitSlot,
                    8,
                    3,
                ),
            );
            let here = fcx.b.ins().stack_addr(ptr_ty, slot, 0);
            fcx.call_rt("grasm_stack_check", &[here]);
        }
        let ps: Vec<Value> = fcx.b.block_params(entry).to_vec();

        // Unpack the captured environment, which was built in this order.
        let env = ps[0];
        for (i, (name, vt)) in c.captures.iter().enumerate() {
            let idx = fcx.b.ins().iconst(types::I64, i as i64);
            let boxed = fcx.call_rt("grasm_list_get", &[env, idx]).unwrap();
            let (v, _) = fcx.unbox(boxed, vt)?;
            let var = fcx.new_var(vt);
            fcx.b.def_var(var, v);
            fcx.bind(name, var, vt.clone());
        }
        fcx.call_rt("grasm_val_dec", &[env]);

        for (i, (pname, pt)) in c.params.iter().enumerate() {
            let var = fcx.new_var(pt);
            fcx.b.def_var(var, ps[i + 1]);
            fcx.bind(pname, var, pt.clone());
        }

        let val = fcx.eval_expr(&c.body)?;
        match (&c.ret, val) {
            (VType::Unit, v) => {
                if let Some((x, xt)) = v {
                    fcx.dec_if_boxed(x, &xt);
                }
                fcx.emit_scope_decs_all();
                if !fcx.is_filled() {
                    fcx.b.ins().return_(&[]);
                }
            }
            (want, Some((mut x, xt))) => {
                if c.wrap_ok {
                    // The body gave the ok half; the declaration says the
                    // value is `T ! E`. Without this the caller's `??` saw a
                    // bare value where an `Ok` was expected and took the
                    // fallback on the *success* path.
                    let boxed = fcx.box_value(x, &xt);
                    x = fcx.call_rt("grasm_ok_new", &[boxed]).unwrap();
                } else if *want == VType::F64 && xt == VType::I64 {
                    x = fcx.b.ins().fcvt_from_sint(types::F64, x);
                }
                fcx.emit_scope_decs_all();
                if !fcx.is_filled() {
                    fcx.b.ins().return_(&[x]);
                }
            }
            (_, None) => {
                // A body that produces nothing, where the declaration says
                // `() ! E`: the value is `Ok(())`. Refused instead, an
                // ordinary `|x| { }` filling a fallible slot could not be
                // compiled — and before that it crashed with SIGTRAP.
                if c.wrap_ok && !fcx.is_filled() {
                    let z = fcx.b.ins().iconst(types::I64, 0);
                    let boxed = fcx.call_rt("grasm_box_unit", &[]).unwrap_or(z);
                    let v = fcx.call_rt("grasm_ok_new", &[boxed]).unwrap();
                    fcx.emit_scope_decs_all();
                    fcx.b.ins().return_(&[v]);
                } else if !fcx.is_filled() {
                    return nope(c.body.span(), "a closure body with no value");
                }
            }
        }
        fcx.b.seal_all_blocks();
        fcx.b.finalize();
        self.module
            .define_function(c.id, &mut ctx)
            .map_err(|e| Unsupported {
                span: c.body.span(),
                // A verifier failure is a compiler bug, not a missing
                // feature, and reporting it through the "unsupported" channel
                // tells a reader to use the interpreter for a construct that
                // has no name. Say which it is.
                what: format!("{INTERNAL}  {e}"),
            })?;
        let _ = c.module;
        Ok(())
    }

    fn compile_handler(&mut self, mut h: PendingHandler) -> CResult<()> {
        h.captures = self.handler_captures.get(&h.id).cloned().unwrap_or_default();
        let mut ctx = self.module.make_context();
        let ptr_ty = self.module.target_config().pointer_type();
        // The environment is always the first parameter, even when empty, so
        // every lifted operation has one signature shape.
        ctx.func.signature.params.push(AbiParam::new(ptr_ty));
        for (_, t) in &h.params {
            ctx.func.signature.params.push(AbiParam::new(t.cl()));
        }
        if h.ret != VType::Unit {
            ctx.func.signature.returns.push(AbiParam::new(h.ret.cl()));
        }
        let mut fbcx = FunctionBuilderContext::new();
        let builder = FunctionBuilder::new(&mut ctx.func, &mut fbcx);
        let mut fcx = FnCx {
            cg: self,
            b: builder,
            vars: Vec::new(),
            var_count: 0,
            loops: Vec::new(),
            handler_arg_held: Vec::new(),
            fail_redirect: Vec::new(),
            ret: h.ret.clone(),
            inout_slots: Vec::new(),
            filled: false,
            closure_hints: Vec::new(),
            pinned: Vec::new(),
            tail_hint: None,
            probe_types: Vec::new(),
            probing_args: false,
            index_vals: HashMap::new(),
            wrote_through_ref: false,
            task_scopes: Vec::new(),
        };
        let entry = fcx.b.create_block();
        fcx.b.append_block_params_for_function_params(entry);
        fcx.b.switch_to_block(entry);
        fcx.push_scope();
        // A lifted body recurses through no name, so the call-graph analysis
        // that guards named functions never sees it.
        if body_can_call(&h.body) {
            let ptr_ty = fcx.cg.module.target_config().pointer_type();
            let slot = fcx.b.create_sized_stack_slot(
                cranelift_codegen::ir::StackSlotData::new(
                    cranelift_codegen::ir::StackSlotKind::ExplicitSlot,
                    8,
                    3,
                ),
            );
            let here = fcx.b.ins().stack_addr(ptr_ty, slot, 0);
            fcx.call_rt("grasm_stack_check", &[here]);
        }
        let ps: Vec<Value> = fcx.b.block_params(entry).to_vec();
        let env = ps[0];
        for (i, (cname, ct)) in h.captures.iter().enumerate() {
            let flags = cranelift_codegen::ir::MemFlags::trusted();
            let raw = fcx.b.ins().load(types::I64, flags, env, (i * 8) as i32);
            let v = fcx.bits_to_value(raw, ct);
            // The environment lends its value; leaving this scope releases
            // whatever it holds. Without a matching reference the first call
            // would free a capture the second call still needs — which showed
            // up as a captured map working once and then panicking with
            // "get on non-map".
            fcx.inc_if_boxed(v, ct);
            let var = fcx.new_var(ct);
            fcx.b.def_var(var, v);
            fcx.bind(cname, var, ct.clone());
        }
        for (i, (pname, pt)) in h.params.iter().enumerate() {
            let var = fcx.new_var(pt);
            fcx.b.def_var(var, ps[i + 1]);
            fcx.bind(pname, var, pt.clone());
        }
        fcx.tail_hint = match &h.ret {
            // Unit produces no value, so an expectation of it would force
            // one where there is none.
            VType::Fallible(ok, _) if **ok != VType::Unit => Some((**ok).clone()),
            VType::Fallible(_, _) | VType::Unit => None,
            other => Some(other.clone()),
        };
        let val = fcx.eval_expr(&h.body)?;
        // A body that already left — `|| panic(m)` — filled its block, and
        // Cranelift takes one terminator per block. The named-function path
        // has always guarded this; a lifted handler body did not.
        if fcx.is_filled() {
            fcx.b.seal_all_blocks();
            fcx.b.finalize();
            return self
                .module
                .define_function(h.id, &mut ctx)
                .map_err(|e| Unsupported {
                    span: h.body.span(),
                    what: format!("{INTERNAL}  {e}"),
                });
        }
        match (&h.ret, val) {
            (VType::Unit, v) => {
                if let Some((x, xt)) = v {
                    fcx.dec_if_boxed(x, &xt);
                }
                fcx.emit_scope_decs_all();
                fcx.b.ins().return_(&[]);
            }
            // An operation that declares `! E`: a body producing the plain
            // value is producing `Ok(value)`, the same rule a fallible
            // function's body follows. Without this the two halves of an
            // `if` — one a value, one a `fail` — are different types and the
            // generated code does not verify.
            (VType::Fallible(ok_t, _), Some((x, xt))) => {
                let v = if matches!(xt, VType::Fallible(_, _)) {
                    x
                } else {
                    let mut inner = x;
                    if **ok_t == VType::F64 && xt == VType::I64 {
                        inner = fcx.b.ins().fcvt_from_sint(types::F64, inner);
                    }
                    let boxed = fcx.box_value(inner, ok_t);
                    fcx.call_rt("grasm_ok_new", &[boxed]).unwrap()
                };
                fcx.emit_scope_decs_all();
                fcx.b.ins().return_(&[v]);
            }
            (want, Some((mut x, xt))) => {
                if *want == VType::F64 && xt == VType::I64 {
                    x = fcx.b.ins().fcvt_from_sint(types::F64, x);
                }
                fcx.emit_scope_decs_all();
                fcx.b.ins().return_(&[x]);
            }
            // An operation declared `-> () ! E` whose body gives the bare ok
            // half. Unit has no value to carry, so the merge of `()` against
            // `fail(e)` produces nothing at all — and the arm below returned
            // `zero_of(Fallible)`, a null pointer, which the caller then read.
            // The build succeeded and the program died with SIGSEGV and no
            // message, which is the one thing §11 promises never happens.
            // This is [T-FALLIBLE-UNIT]'s "the value it merges the two halves
            // through does not exist", in the one position that had no arm
            // for it; the named-function path has had one all along.
            (VType::Fallible(_, _), None) => {
                let z = fcx.b.ins().iconst(types::I64, 0);
                let boxed = fcx.call_rt("grasm_box_i64", &[z]).unwrap();
                let v = fcx.call_rt("grasm_ok_new", &[boxed]).unwrap();
                fcx.emit_scope_decs_all();
                fcx.b.ins().return_(&[v]);
            }
            (want, None) => {
                // A handler operation whose body leaves — `|| fail(e)` for a
                // fallible op — produces no value because control never
                // reaches the end. The refusal depended on the *shape* of the
                // closure body rather than on the operation's type, so
                // `|b| if b { 1 } else { fail(e) }` compiled and the bare
                // `fail` did not.
                let z = fcx.zero_of(want);
                fcx.b.ins().return_(&[z]);
            }
        }
        fcx.b.seal_all_blocks();
        fcx.b.finalize();
        self.module
            .define_function(h.id, &mut ctx)
            .map_err(|e| Unsupported {
                span: h.body.span(),
                // A verifier failure is a compiler bug, not a missing
                // feature, and reporting it through the "unsupported" channel
                // tells a reader to use the interpreter for a construct that
                // has no name. Say which it is.
                what: format!("{INTERNAL}  {e}"),
            })?;
        Ok(())
    }

    /// The type descriptor for a struct shape: its name and its field names,
    /// written into the object file's data section so that building a value
    /// does not have to discover them.
    ///
    /// The layout is the one `StructDesc` in the runtime documents — a cache
    /// word the runtime fills in on first use, the name, the field count, then
    /// a pointer and a length per field. The pointers are relocations, so the
    /// text itself is the same interned bytes every other mention of that name
    /// uses.
    fn struct_desc(
        &mut self,
        name: &str,
        fields: &[String],
    ) -> cranelift_module::DataId {
        let key = format!("{name}\u{0}{}", fields.join("\u{0}"));
        if let Some(id) = self.struct_descs.get(&key) {
            return *id;
        }
        let name_id = self.intern_string(name);
        let field_ids: Vec<cranelift_module::DataId> =
            fields.iter().map(|f| self.intern_string(f)).collect();

        let size = 32 + fields.len() * 16;
        let mut bytes = vec![0u8; size];
        // Little-endian: the backend targets the host, and every target it can
        // reach is little-endian.
        bytes[16..24].copy_from_slice(&(name.len() as u64).to_le_bytes());
        bytes[24..32].copy_from_slice(&(fields.len() as u64).to_le_bytes());
        for (i, f) in fields.iter().enumerate() {
            let at = 32 + i * 16 + 8;
            bytes[at..at + 8].copy_from_slice(&(f.len() as u64).to_le_bytes());
        }

        // Writable: the runtime stores the built `StructTy` into word 0.
        let id = self
            .module
            .declare_anonymous_data(true, false)
            .expect("data");
        let mut d = DataDescription::new();
        d.define(bytes.into_boxed_slice());
        d.set_align(8);
        let gv = self.module.declare_data_in_data(name_id, &mut d);
        d.write_data_addr(8, gv, 0);
        for (i, fid) in field_ids.iter().enumerate() {
            let gv = self.module.declare_data_in_data(*fid, &mut d);
            d.write_data_addr((32 + i * 16) as u32, gv, 0);
        }
        self.module.define_data(id, &d).expect("data define");
        self.struct_descs.insert(key, id);
        id
    }

    /// A one-word slot holding a literal's value once it has been made.
    ///
    /// One per distinct text, so `"hello"` written in ten places shares a
    /// single value. The slot is zero until the first time that literal is
    /// reached; after that every occurrence is a load and a branch, against the
    /// 24 ns an allocation cost.
    fn str_lit_slot(&mut self, s: &str) -> cranelift_module::DataId {
        if let Some(id) = self.str_lit_slots.get(s) {
            return *id;
        }
        let id = self
            .module
            .declare_anonymous_data(true, false)
            .expect("data");
        let mut d = DataDescription::new();
        d.define(vec![0u8; 8].into_boxed_slice());
        d.set_align(8);
        self.module.define_data(id, &d).expect("data define");
        self.str_lit_slots.insert(s.to_string(), id);
        id
    }

    fn intern_string(&mut self, s: &str) -> cranelift_module::DataId {
        if let Some(id) = self.strings.get(s) {
            return *id;
        }
        let id = self
            .module
            .declare_anonymous_data(false, false)
            .expect("data");
        let mut desc = DataDescription::new();
        desc.define(s.as_bytes().to_vec().into_boxed_slice());
        self.module.define_data(id, &desc).expect("data define");
        self.strings.insert(s.to_string(), id);
        id
    }
}

struct FnCx<'a, 'm> {
    cg: &'a mut Codegen<'m>,
    b: FunctionBuilder<'a>,
    vars: Vec<HashMap<String, (Variable, VType)>>,
    var_count: usize,
    /// `(continue target, break target, scope depth at loop entry)`.
    ///
    /// The depth is what lets `break` and `continue` release the locals the
    /// loop body declared. Without it they jumped straight out and the body's
    /// scope decs — emitted only on the fall-through path — never ran, so
    /// every `break` past a boxed local leaked it.
    loops: Vec<(Block, Block, usize)>,
    /// Capture slots built for handler arguments in this frame, and what is in
    /// them, so the boxed ones can be released when the frame ends.
    ///
    /// Released at the end of the function rather than after the call: a
    /// handler block is a pointer into *this* frame, and the frame outliving
    /// every call it makes is exactly what makes passing one down sound.
    handler_arg_held: Vec<(cranelift_codegen::ir::StackSlot, Vec<(String, VType)>)>,
    /// Where a `fail` should go instead of returning from this function.
    ///
    /// A closure passed to `map` is *inlined* into the loop, so a `fail` in its
    /// body is lexically inside this function and belongs to the closure. It
    /// lowered as a return, and `[0, 1].map(|x| fail(e))` held as `[i64 ! E]`
    /// reported an unhandled error from `main` where the interpreter produced a
    /// list of `Err`s.
    ///
    /// `scopes` is how deep the scope stack was when the redirect was
    /// installed: a `fail` releases what the closure body bound and nothing
    /// the loop owns, which it still needs.
    fail_redirect: Vec<(Block, Variable, usize)>,
    ret: VType,
    /// (caller pointer, local var, type) for `inout` params — stored back
    /// through the pointer before every return.
    inout_slots: Vec<(Value, Variable, VType)>,
    /// True when the current block already ends in a terminator (a `break`,
    /// `continue`, `return`, or `fail` was just emitted). Cleared whenever we
    /// switch to a fresh block.
    filled: bool,
    /// Per enclosing block: the argument expressions seen at call sites of a
    /// `let`-bound closure, and every binding's initializer.
    ///
    /// Without a function-type syntax a closure's parameter types can only
    /// come from how it is called — and the call may name a binding declared
    /// *after* the closure, so the initializers are needed to type it.
    closure_hints: Vec<BlockHints>,
    /// Scalar lists pinned for the enclosing loop: name → (data pointer,
    /// length, element type). While pinned, `xs[i]` is a load rather than a
    /// call.
    pinned: Vec<HashMap<String, (Value, Value, VType)>>,
    /// What the value of a block in tail position should be. Only consulted
    /// for the shapes that cannot type themselves — an empty collection
    /// literal — so a stale hint cannot change a program's meaning.
    tail_hint: Option<VType>,
    /// Types for names a *probe* knows about, kept apart from real bindings
    /// because a probe must not create a Cranelift variable — one declared and
    /// never defined is read as garbage if anything later uses it.
    probe_types: Vec<HashMap<String, VType>>,
    /// Guards the one retry `probe_arg_type` makes with the block's hints
    /// bound, so probing a hint cannot re-enter it.
    probing_args: bool,
    /// Assignment-target index expressions, evaluated before the value.
    ///
    /// `xs[f()] = g()` must run `f` before `g` ([E-ORDER-LTR]), and the walk
    /// that writes the value is what evaluates the indices — so doing it
    /// after left the order reversed. Keyed by span, cleared per statement.
    index_vals: HashMap<u32, (Value, VType)>,
    /// Set when a write reached the slab through a `ref`, so no enclosing
    /// container needs updating. See `begin_write`.
    wrote_through_ref: bool,
    /// One entry per enclosing `parallel` block: the runtime list that records
    /// which spawn sites fired, and how many sites have been lowered so far.
    ///
    /// Tasks are collected during the block and run in spawn order after it,
    /// so the list holds *site indices* rather than closures — the bodies are
    /// lexically known, and [T-TASK-CAPTURE] guarantees they only touch
    /// bindings that outlive the block.
    task_scopes: Vec<(Variable, usize)>,
}

/// What a block knows that a closure inside it needs.
#[derive(Default)]
struct BlockHints {
    /// `let f = |x| …` → the arguments of a later `f(...)` call.
    calls: HashMap<String, Vec<Expr>>,
    /// Every binding's initializer, so an argument naming a binding declared
    /// after the closure can still be typed.
    inits: HashMap<String, Expr>,
    /// A `for` variable and the thing it iterates. The variable is not a
    /// `let`, so it had no initializer to probe: `let f = |x| x * 2` then
    /// `for v in [1, 2] { f(v) }` was refused as "the type of `v`" — about a
    /// binding whose type the list two lines up fixes.
    loop_over: HashMap<String, Expr>,
    /// A binding's written type, which beats its initializer when the
    /// initializer is an empty literal.
    declared_ty: HashMap<String, grasm_syntax::ast::TypeExpr>,
    /// The declared type of a closure literal that is not bound to a name of
    /// its own — an arm of a `match` or an `if` whose *result* is annotated.
    /// Keyed by the literal's start offset, since there is no name to key on.
    declared_at: HashMap<u32, grasm_syntax::ast::TypeExpr>,
    /// `apply(f, 5)` where `apply` declares `fn(i64) -> i64`: the signature
    /// says what `f`'s parameters are, and no call of `f` by name ever will.
    declared: HashMap<String, Vec<VType>>,
    /// And what it returns. A closure's own body cannot say that it is
    /// *fallible*: `|n| if c { fail(e) } else { v }` probes as `v`'s type, so
    /// the lifted function returned a bare value and a `fail` inside it
    /// returned an error object through a slot typed for the value —
    /// `f(1) ?? "d"` then took the fallback on the success path.
    declared_ret: HashMap<String, VType>,
}

/// The call-graph node standing for a module-level constant's initializer.
/// Named so no function key can collide with it.
fn const_node(module: &str, name: &str) -> String {
    format!("{module}::<const {name}>")
}

/// Every function that can reach itself, directly or through others.
///
/// Only these need a stack check, so a straight-line program pays nothing for
/// it. A function taking a function *parameter* counts too: the call it makes
/// through that parameter is opaque here and can close a cycle this cannot
/// see.
fn recursive_functions(
    views: &[(String, &AstModule, std::collections::HashMap<String, String>)],
) -> std::collections::HashSet<String> {
    let mut calls: HashMap<String, Vec<String>> = HashMap::new();
    let mut out: std::collections::HashSet<String> = std::collections::HashSet::new();
    // Every function name in the program, unqualified — enough to tell a
    // mention of a function from a mention of a local.
    let fn_names: std::collections::HashSet<String> = views
        .iter()
        .flat_map(|(_, m, _)| m.decls.iter())
        .filter_map(|d| match d {
            Decl::Fn(f) => Some(f.name.name.clone()),
            _ => None,
        })
        .collect();
    // A module-level constant is computed by evaluating its initializer, so
    // the initializer is a node in this graph like any body — and a function
    // that *reads* the constant can re-enter it. `let A = f()` beside
    // `fn f() -> i64 { A }` is a cycle that runs off the stack for real:
    // SIGSEGV, exit 139, empty stderr, which is what [E-RECURSION-DEPTH]
    // rules out. Neither half of the cycle is a call this graph could see
    // without the constant in it.
    let const_names: std::collections::HashSet<String> = views
        .iter()
        .flat_map(|(_, m, _)| m.decls.iter())
        .filter_map(|d| match d {
            Decl::Const(c) => Some(c.name.name.clone()),
            _ => None,
        })
        .collect();
    for (module, m, imports) in views {
        for decl in &m.decls {
            if let Decl::Const(c) = decl {
                let mut named: Vec<(String, Vec<Expr>)> = Vec::new();
                collect_named_calls_expr(&c.init, &mut named);
                let callees: Vec<String> = named
                    .into_iter()
                    .map(|(name, _)| match name.split_once('.') {
                        Some((alias, callee)) => match imports.get(alias) {
                            Some(target) => format!("{target}::{callee}"),
                            None => format!("{module}::{name}"),
                        },
                        None => format!("{module}::{name}"),
                    })
                    .collect();
                calls.insert(const_node(module, &c.name.name), callees);
                continue;
            }
            let Decl::Fn(f) = decl else { continue };
            let key = format!("{module}::{}", f.name.name);
            if f.params
                .iter()
                .any(|p| matches!(p.ty, grasm_syntax::ast::TypeExpr::Fn { .. }))
            {
                out.insert(key.clone());
            }
            let mut named = Vec::new();
            collect_named_calls_block(&f.body, &mut named);
            // A function *value* handed somewhere — `Box{ f: down }`,
            // `[shout]`, a return — can be called back through later, and that
            // call is invisible to this graph. Without it, recursion through a
            // struct field ran off the stack for real: SIGSEGV, exit 139,
            // empty stderr, which is what [E-RECURSION-DEPTH] rules out.
            // Naming one is enough to need the check; calling it by name is
            // the ordinary case the graph already sees.
            let mut idents = Vec::new();
            collect_idents_block(&f.body, &mut idents);
            let called: std::collections::HashSet<&str> =
                named.iter().map(|(n, _)| n.as_str()).collect();
            if idents.iter().any(|i| !called.contains(i.as_str()) && fn_names.contains(i)) {
                out.insert(key.clone());
            }
            let mut callees: Vec<String> = named
                .into_iter()
                .map(|(name, _)| match name.split_once('.') {
                    // A qualified call names another module through an alias.
                    Some((alias, callee)) => match imports.get(alias) {
                        Some(target) => format!("{target}::{callee}"),
                        None => format!("{module}::{name}"),
                    },
                    None => format!("{module}::{name}"),
                })
                .collect();
            // Reading a constant runs its initializer, so the read is an edge.
            for id in &idents {
                if const_names.contains(id) {
                    callees.push(const_node(module, id));
                }
            }
            calls.insert(key, callees);
        }
    }
    for start in calls.keys() {
        let mut seen: std::collections::HashSet<&str> = std::collections::HashSet::new();
        let mut stack: Vec<&str> = calls[start].iter().map(String::as_str).collect();
        while let Some(next) = stack.pop() {
            if next == start.as_str() {
                out.insert(start.clone());
                break;
            }
            if !seen.insert(next) {
                continue;
            }
            if let Some(more) = calls.get(next) {
                stack.extend(more.iter().map(String::as_str));
            }
        }
    }
    out
}

/// Every `f(args...)` in a block where `f` is a plain name, innermost first.
/// Every struct literal a statement contains, as (type path, fields).
fn collect_struct_lits_stmt<'e>(
    s: &'e Stmt,
    out: &mut Vec<(&'e Path, &'e Vec<(Ident, Expr)>)>,
) {
    match s {
        Stmt::Let { init, .. } => collect_struct_lits(init, out),
        Stmt::Assign { value, .. } => collect_struct_lits(value, out),
        Stmt::Return(Some(e), _) | Stmt::Expr(e) => collect_struct_lits(e, out),
        Stmt::While { body, .. }
        | Stmt::For { body, .. }
        | Stmt::With { body, .. }
        | Stmt::Parallel { body, .. }
        | Stmt::Spawn { body, .. } => {
            for st in &body.stmts {
                collect_struct_lits_stmt(st, out);
            }
        }
        _ => {}
    }
}

fn collect_struct_lits<'e>(
    e: &'e Expr,
    out: &mut Vec<(&'e Path, &'e Vec<(Ident, Expr)>)>,
) {
    match e {
        Expr::StructLit { name, fields, base, .. } => {
            out.push((name, fields));
            if let Some(b) = base {
                collect_struct_lits(b, out);
            }
            for (_, v) in fields {
                collect_struct_lits(v, out);
            }
        }
        Expr::Call { args, .. } | Expr::MethodCall { args, .. } => {
            for a in args {
                collect_struct_lits(a, out);
            }
        }
        Expr::List(items, _) | Expr::Tuple(items, _) => {
            for i in items {
                collect_struct_lits(i, out);
            }
        }
        _ => {}
    }
}

fn collect_named_calls_block(b: &grasm_syntax::ast::Block, out: &mut Vec<(String, Vec<Expr>)>) {
    for st in &b.stmts {
        collect_named_calls_stmt(st, out);
    }
}

fn collect_named_calls_stmt(st: &Stmt, out: &mut Vec<(String, Vec<Expr>)>) {
    match st {
        Stmt::Let { init, .. } => collect_named_calls_expr(init, out),
        Stmt::LetTuple { init, .. } => collect_named_calls_expr(init, out),
        Stmt::Assign { value, .. } => collect_named_calls_expr(value, out),
        Stmt::Return(Some(e), _) | Stmt::Expr(e) => collect_named_calls_expr(e, out),
        Stmt::While { cond, body, .. } => {
            collect_named_calls_expr(cond, out);
            collect_named_calls_block(body, out);
        }
        Stmt::For { iter, body, .. } => {
            collect_named_calls_expr(iter, out);
            collect_named_calls_block(body, out);
        }
        _ => {}
    }
}

fn collect_named_calls_expr(e: &Expr, out: &mut Vec<(String, Vec<Expr>)>) {
    match e {
        Expr::Call { callee, args, .. } => {
            if let Expr::Ident(id) = &**callee {
                out.push((id.name.clone(), args.to_vec()));
            }
            for a in args {
                collect_named_calls_expr(a, out);
            }
        }
        Expr::MethodCall { recv, args, .. } => {
            collect_named_calls_expr(recv, out);
            for a in args {
                collect_named_calls_expr(a, out);
            }
        }
        Expr::Binary { lhs, rhs, .. } => {
            collect_named_calls_expr(lhs, out);
            collect_named_calls_expr(rhs, out);
        }
        Expr::Unary { expr, .. } => collect_named_calls_expr(expr, out),
        Expr::Block(b) | Expr::Unsafe(b) | Expr::Region(b) => collect_named_calls_block(b, out),
        Expr::Str(pieces, _) => {
            for p in pieces {
                if let StrPiece::Expr(inner) = p {
                    collect_named_calls_expr(inner, out);
                }
            }
        }
        // The rest of the expression forms. Missing `if` here meant the most
        // ordinary recursive function in the world — `if n <= 0 { 0 } else {
        // f(n - 1) }` — did not look recursive at all.
        Expr::If {
            cond,
            then_block,
            else_branch,
            ..
        } => {
            collect_named_calls_expr(cond, out);
            collect_named_calls_block(then_block, out);
            match else_branch.as_deref() {
                Some(grasm_syntax::ast::ElseBranch::Else(b)) => {
                    collect_named_calls_block(b, out)
                }
                Some(grasm_syntax::ast::ElseBranch::ElseIf(e)) => {
                    collect_named_calls_expr(e, out)
                }
                None => {}
            }
        }
        Expr::Match { scrutinee, arms, .. } => {
            collect_named_calls_expr(scrutinee, out);
            for a in arms {
                collect_named_calls_expr(&a.body, out);
                if let Some(g) = &a.guard {
                    collect_named_calls_expr(g, out);
                }
            }
        }
        Expr::List(items, _) | Expr::Tuple(items, _) => {
            for i in items {
                collect_named_calls_expr(i, out);
            }
        }
        Expr::Map(entries, _) => {
            for (k, v) in entries {
                collect_named_calls_expr(k, out);
                collect_named_calls_expr(v, out);
            }
        }
        Expr::StructLit { base, fields, .. } => {
            if let Some(b) = base {
                collect_named_calls_expr(b, out);
            }
            for (_, v) in fields {
                collect_named_calls_expr(v, out);
            }
        }
        Expr::Index { recv, index, .. } => {
            collect_named_calls_expr(recv, out);
            collect_named_calls_expr(index, out);
        }
        Expr::Field { recv, .. } => collect_named_calls_expr(recv, out),
        Expr::Closure { body, .. } => collect_named_calls_expr(body, out),
        Expr::Try { expr, .. } => collect_named_calls_expr(expr, out),
        _ => {}
    }
}

fn collect_block_hints(blk: &grasm_syntax::ast::Block) -> BlockHints {
    let mut hints = BlockHints::default();
    let mut bound: Vec<String> = Vec::new();
    for stmt in &blk.stmts {
        if let Stmt::Let { name, init, ty, .. } = stmt {
            hints
                .inits
                .entry(name.name.clone())
                .or_insert_with(|| init.clone());
            // The annotation says more than the initializer when the
            // initializer is an empty literal: `var m: Map[str, i64] = map()`
            // probes as a map of nothing without it.
            if let Some(t) = ty {
                hints
                    .declared_ty
                    .entry(name.name.clone())
                    .or_insert_with(|| t.clone());
            }
            if matches!(init, Expr::Closure { params, .. } if !params.is_empty()) {
                bound.push(name.name.clone());
            }
            // `let f: fn(i64) -> i64 = match n { 1 -> |x| x + 1  _ -> … }`.
            // The annotation is on the binding and the closures are in the
            // arms, so nothing connected the two: the refusal said "no type
            // says what its parameters are" about a program whose binding says
            // exactly that, and its suggested fix was what the source already
            // contained.
            if let Some(t) = ty {
                for (at, want) in arm_closure_hints(init, t) {
                    hints.declared_at.entry(at).or_insert(want);
                }
            }
        }
        if let Stmt::For { var, iter, .. } = stmt {
            hints
                .loop_over
                .entry(var.name.clone())
                .or_insert_with(|| iter.clone());
        }
    }
    if bound.is_empty() {
        return hints;
    }
    for stmt in &blk.stmts {
        collect_calls_stmt(stmt, &bound, &mut hints.calls);
    }
    hints
}

fn collect_calls_stmt(stmt: &Stmt, bound: &[String], out: &mut HashMap<String, Vec<Expr>>) {
    match stmt {
        Stmt::Let { init, .. } => collect_calls_expr(init, bound, out),
        Stmt::Assign { value, .. } => collect_calls_expr(value, bound, out),
        Stmt::Return(Some(e), _) | Stmt::Expr(e) => collect_calls_expr(e, bound, out),
        Stmt::While { cond, body, .. } => {
            collect_calls_expr(cond, bound, out);
            for s in &body.stmts {
                collect_calls_stmt(s, bound, out);
            }
        }
        Stmt::For { body, .. }
        | Stmt::Parallel { body, .. }
        | Stmt::Spawn { body, .. }
        | Stmt::With { body, .. } => {
            for s in &body.stmts {
                collect_calls_stmt(s, bound, out);
            }
        }
        _ => {}
    }
}

fn collect_calls_expr(e: &Expr, bound: &[String], out: &mut HashMap<String, Vec<Expr>>) {
    if let Expr::Call { callee, args, .. } = e {
        if let Expr::Ident(id) = &**callee {
            if bound.contains(&id.name) {
                out.entry(id.name.clone()).or_insert_with(|| args.to_vec());
            }
        }
    }
    // `xs.filter(f)` never calls `f` by name, so the rule above sees nothing
    // and the closure has no parameter types. But the receiver says what they
    // are: one element of `xs`. Recording `xs[0]` as a stand-in argument is
    // enough — the hint is only ever *probed*, never evaluated, so an index
    // that might be out of range costs nothing.
    if let Expr::MethodCall { recv, name, args, span } = e {
        let per_element = matches!(
            name.name.as_str(),
            "map" | "par_map" | "filter" | "par_filter" | "any" | "all" | "find" | "sort_by"
        );
        if per_element || name.name == "fold" {
            let elem = Expr::Index {
                recv: recv.clone(),
                index: Box::new(Expr::Int(0, *span)),
                span: *span,
            };
            for (i, a) in args.iter().enumerate() {
                let Expr::Ident(id) = a else { continue };
                if !bound.contains(&id.name) {
                    continue;
                }
                // `fold(init, f)` takes the accumulator first.
                let stand_in = if name.name == "fold" {
                    if i == 0 {
                        continue;
                    }
                    vec![args[0].clone(), elem.clone()]
                } else {
                    vec![elem.clone()]
                };
                out.entry(id.name.clone()).or_insert(stand_in);
            }
        }
    }
    match e {
        Expr::Call { callee, args, .. } => {
            collect_calls_expr(callee, bound, out);
            for a in args {
                collect_calls_expr(a, bound, out);
            }
        }
        Expr::MethodCall { recv, args, .. } => {
            collect_calls_expr(recv, bound, out);
            for a in args {
                collect_calls_expr(a, bound, out);
            }
        }
        Expr::Binary { lhs, rhs, .. } => {
            collect_calls_expr(lhs, bound, out);
            collect_calls_expr(rhs, bound, out);
        }
        Expr::Unary { expr, .. } | Expr::Try { expr, .. } => collect_calls_expr(expr, bound, out),
        Expr::List(items, _) | Expr::Tuple(items, _) => {
            for i in items {
                collect_calls_expr(i, bound, out);
            }
        }
        Expr::Map(entries, _) => {
            for (k, v) in entries {
                collect_calls_expr(k, bound, out);
                collect_calls_expr(v, bound, out);
            }
        }
        Expr::Str(pieces, _) => {
            for p in pieces {
                if let StrPiece::Expr(inner) = p {
                    collect_calls_expr(inner, bound, out);
                }
            }
        }
        // The shapes a call can hide in. A closure whose only call site was a
        // `match` scrutinee, an `if` branch, or a field read had no parameter
        // types to be found, and the backend refused it for having none —
        // `let h = |x| risky(x)` followed by `match h(-1) { ... }`.
        Expr::Field { recv, .. } => collect_calls_expr(recv, bound, out),
        Expr::Index { recv, index, .. } => {
            collect_calls_expr(recv, bound, out);
            collect_calls_expr(index, bound, out);
        }
        Expr::Block(b) | Expr::Unsafe(b) | Expr::Region(b) => {
            for st in &b.stmts {
                collect_calls_stmt(st, bound, out);
            }
        }
        Expr::If {
            cond,
            then_block,
            else_branch,
            ..
        } => {
            collect_calls_expr(cond, bound, out);
            for st in &then_block.stmts {
                collect_calls_stmt(st, bound, out);
            }
            match else_branch.as_deref() {
                Some(grasm_syntax::ast::ElseBranch::Else(b)) => {
                    for st in &b.stmts {
                        collect_calls_stmt(st, bound, out);
                    }
                }
                Some(grasm_syntax::ast::ElseBranch::ElseIf(e)) => {
                    collect_calls_expr(e, bound, out)
                }
                None => {}
            }
        }
        Expr::Match { scrutinee, arms, .. } => {
            collect_calls_expr(scrutinee, bound, out);
            for a in arms {
                collect_calls_expr(&a.body, bound, out);
                if let Some(g) = &a.guard {
                    collect_calls_expr(g, bound, out);
                }
            }
        }
        _ => {}
    }
}

/// Every identifier appearing in an expression, in source order.
/// Whether a lifted body can call anything, and so can recurse.
///
/// A named function gets its stack check from the call-graph analysis, which
/// only knows names. A handler operation calling its own effect —
/// `CfgHandler{ value: |n| Cfg.value(n - 1) }` — recurses through no name at
/// all, and the compiled program died with SIGSEGV and an empty stderr where
/// [E-RECURSION-DEPTH] promises a panic and exit 101.
fn body_can_call(e: &Expr) -> bool {
    let mut names = Vec::new();
    collect_idents(e, &mut names);
    if !names.is_empty() {
        return true;
    }
    let mut calls: Vec<(String, Vec<Expr>)> = Vec::new();
    collect_named_calls_expr(e, &mut calls);
    !calls.is_empty()
}

/// Whether this closure body can fail, and so cannot run on a parallel worker.
///
/// The lifted worker ABI carries one `i64` of result bits and no error
/// channel, so a `fail` inside it used to leave the error's boxed pointer in
/// the result slot: `[1, 2, 3].par_map(|x| if x == 2 { fail(e) } else { x })`
/// built natively printed a raw address as the middle element and exited 0.
/// The sequential lowering propagates the failure correctly, and `par_map` is
/// free to run sequentially, so a body like this takes that path.
fn body_can_fail(e: &Expr) -> bool {
    let mut names = Vec::new();
    collect_idents(e, &mut names);
    names.iter().any(|n| n == "fail")
}

fn collect_idents(e: &Expr, out: &mut Vec<String>) {
    match e {
        Expr::Ident(id) => out.push(id.name.clone()),
        Expr::Unary { expr, .. } | Expr::Try { expr, .. } => collect_idents(expr, out),
        Expr::Binary { lhs, rhs, .. } => {
            collect_idents(lhs, out);
            collect_idents(rhs, out);
        }
        Expr::Call { callee, args, .. } => {
            collect_idents(callee, out);
            for a in args {
                collect_idents(a, out);
            }
        }
        Expr::MethodCall { recv, args, .. } => {
            collect_idents(recv, out);
            for a in args {
                collect_idents(a, out);
            }
        }
        Expr::Field { recv, .. } => collect_idents(recv, out),
        Expr::Index { recv, index, .. } => {
            collect_idents(recv, out);
            collect_idents(index, out);
        }
        Expr::List(items, _) | Expr::Tuple(items, _) => {
            for i in items {
                collect_idents(i, out);
            }
        }
        Expr::Map(entries, _) => {
            for (k, v) in entries {
                collect_idents(k, out);
                collect_idents(v, out);
            }
        }
        Expr::Str(pieces, _) => {
            for p in pieces {
                if let StrPiece::Expr(inner) = p {
                    collect_idents(inner, out);
                }
            }
        }
        Expr::Closure { body, .. } => collect_idents(body, out),
        // A struct literal's field values were not walked at all, so
        // `Box{ f: down }` mentioned nothing as far as this could tell.
        Expr::StructLit { base, fields, .. } => {
            if let Some(b) = base {
                collect_idents(b, out);
            }
            for (_, v) in fields {
                collect_idents(v, out);
            }
        }
        Expr::Block(b) | Expr::Unsafe(b) | Expr::Region(b) => collect_idents_block(b, out),
        Expr::If {
            cond,
            then_block,
            else_branch,
            ..
        } => {
            collect_idents(cond, out);
            collect_idents_block(then_block, out);
            match else_branch.as_deref() {
                Some(grasm_syntax::ast::ElseBranch::Else(b)) => collect_idents_block(b, out),
                Some(grasm_syntax::ast::ElseBranch::ElseIf(inner)) => collect_idents(inner, out),
                None => {}
            }
        }
        Expr::Match {
            scrutinee, arms, ..
        } => {
            collect_idents(scrutinee, out);
            for arm in arms {
                if let Some(g) = &arm.guard {
                    collect_idents(g, out);
                }
                collect_idents(&arm.body, out);
            }
        }
        _ => {}
    }
}

fn collect_idents_block(b: &grasm_syntax::ast::Block, out: &mut Vec<String>) {
    for stmt in &b.stmts {
        match stmt {
            Stmt::Let { init, .. } => collect_idents(init, out),
            Stmt::Assign { target, value, .. } => {
                collect_idents(target, out);
                collect_idents(value, out);
            }
            Stmt::Return(Some(e), _) | Stmt::Expr(e) => collect_idents(e, out),
            Stmt::While { cond, body, .. } => {
                collect_idents(cond, out);
                collect_idents_block(body, out);
            }
            Stmt::For { iter, body, .. } => {
                collect_idents(iter, out);
                collect_idents_block(body, out);
            }
            Stmt::Parallel { body, .. } | Stmt::Spawn { body, .. } => {
                collect_idents_block(body, out)
            }
            Stmt::With { handler, body, .. } => {
                collect_idents(handler, out);
                collect_idents_block(body, out);
            }
            _ => {}
        }
    }
}

/// Every `spawn` body inside a `parallel` block, in source order.
///
/// The walk mirrors codegen exactly — each `spawn` *statement* is lowered
/// once, however many times it runs — so the index assigned here is the index
/// the emitted code records. A nested `parallel` owns its own sites, so the
/// walk stops there.
fn collect_spawn_sites(block: &grasm_syntax::ast::Block) -> Vec<&grasm_syntax::ast::Block> {
    let mut out = Vec::new();
    walk_spawns(block, &mut out);
    out
}

fn walk_spawns<'b>(
    block: &'b grasm_syntax::ast::Block,
    out: &mut Vec<&'b grasm_syntax::ast::Block>,
) {
    for stmt in &block.stmts {
        match stmt {
            Stmt::Spawn { body, .. } => out.push(body),
            Stmt::While { body, .. } | Stmt::For { body, .. } | Stmt::With { body, .. } => {
                walk_spawns(body, out)
            }
            // A nested `parallel` collects and joins its own tasks.
            Stmt::Parallel { .. } => {}
            Stmt::Let { init, .. } => walk_spawns_expr(init, out),
            Stmt::Assign { value, .. } => walk_spawns_expr(value, out),
            Stmt::Return(Some(e), _) | Stmt::Expr(e) => walk_spawns_expr(e, out),
            _ => {}
        }
    }
}

fn walk_spawns_expr<'b>(e: &'b Expr, out: &mut Vec<&'b grasm_syntax::ast::Block>) {
    use grasm_syntax::ast::ElseBranch;
    match e {
        Expr::If {
            then_block,
            else_branch,
            ..
        } => {
            walk_spawns(then_block, out);
            match else_branch.as_deref() {
                Some(ElseBranch::Else(b)) => walk_spawns(b, out),
                Some(ElseBranch::ElseIf(inner)) => walk_spawns_expr(inner, out),
                None => {}
            }
        }
        Expr::Match { arms, .. } => {
            for arm in arms {
                walk_spawns_expr(&arm.body, out);
            }
        }
        Expr::Block(b) | Expr::Unsafe(b) | Expr::Region(b) => walk_spawns(b, out),
        _ => {}
    }
}

/// Matches a declared type against a concrete one, binding generic names.
///
/// Structural and best-effort: a mismatch simply binds nothing, and the caller
/// reports any parameter left undetermined. The checker has already accepted
/// the call, so this is inference, not validation.
fn unify_generic(
    declared: &TypeExpr,
    actual: &VType,
    names: &std::collections::HashSet<&str>,
    subst: &mut HashMap<String, VType>,
) {
    unify_generic_with(declared, actual, names, subst, &HashMap::new())
}

/// As [`unify_generic`], and able to see through an instantiated generic type.
///
/// `fn first[T](h: Holder[T])` called with a `Holder[i64]`: the declared side
/// says `Holder` of one argument, the actual side is a struct whose name was
/// mangled for uniqueness. `type_args` is what that name was instantiated at.
fn unify_generic_with(
    declared: &TypeExpr,
    actual: &VType,
    names: &std::collections::HashSet<&str>,
    subst: &mut HashMap<String, VType>,
    type_args: &HashMap<String, (String, Vec<VType>)>,
) {
    match declared {
        TypeExpr::Name { path, args, .. } if path.segments.len() == 1 => {
            let n = path.segments[0].name.as_str();
            if args.is_empty() && names.contains(n) {
                subst.entry(n.to_string()).or_insert_with(|| actual.clone());
                return;
            }
            match (n, actual) {
                ("Option", VType::Option(inner)) if args.len() == 1 => {
                    unify_generic_with(&args[0], inner, names, subst, type_args)
                }
                ("Map", VType::Map(k, v)) if args.len() == 2 => {
                    unify_generic_with(&args[0], k, names, subst, type_args);
                    unify_generic_with(&args[1], v, names, subst, type_args);
                }
                (_, VType::Struct(key) | VType::Sum(key)) if !args.is_empty() => {
                    if let Some((base, ats)) = type_args.get(key) {
                        if base == n && ats.len() == args.len() {
                            for (d, a) in args.iter().zip(ats) {
                                unify_generic_with(d, a, names, subst, type_args);
                            }
                        }
                    }
                }
                _ => {}
            }
        }
        TypeExpr::List(inner, _) => {
            if let VType::List(e) = actual {
                unify_generic_with(inner, e, names, subst, type_args);
            }
        }
        TypeExpr::Tuple(items, _) => {
            if let VType::Tuple(ts) = actual {
                for (d, a) in items.iter().zip(ts) {
                    unify_generic_with(d, a, names, subst, type_args);
                }
            }
        }
        // `fn(T) -> K`. Without this, a type parameter that appears only in a
        // function-typed parameter's *result* was determined by nothing:
        // `key_by[T, K](xs: [T], f: fn(T) -> K)` bound `T` from `xs` and left
        // `K` unbound, so there was no type to monomorphize the return at.
        //
        // The closure passed for `f` has always known its own type; nothing
        // looked inside it.
        TypeExpr::Fn { params, ret, .. } => {
            if let VType::Fn(ps, r) = actual {
                for (d, a) in params.iter().zip(ps) {
                    unify_generic_with(d, a, names, subst, type_args);
                }
                unify_generic_with(ret, r, names, subst, type_args);
            }
        }
        TypeExpr::Ref(inner, _) => {
            if let VType::Ref(t) = actual {
                unify_generic_with(inner, t, names, subst, type_args);
            }
        }
        TypeExpr::Fallible(ok, err, _) => {
            if let VType::Fallible(o, e) = actual {
                unify_generic_with(ok, o, names, subst, type_args);
                unify_generic_with(err, e, names, subst, type_args);
            }
        }
        _ => {}
    }
}

/// Bytes per element in a pinned scalar list: bools pack to a byte.
/// Whether an expression has no type of its own and needs one supplied.
///
/// Only these shapes. A hint applied more widely forces a value out of an
/// expression that produces none — an arm that ends in `fail`, say — which is
/// how the first version of this broke three conformance programs.
/// Whether a list element needs its type supplied by the list's annotation.
///
/// A closure with parameters is the case: nothing in `|n| n + 100` says what
/// `n` is, and inside a list there is no call site to learn it from.
/// Whether probing this expression is known to emit nothing.
///
/// `probe_type` is documented as a static probe, and most of it is, but some
/// arms create Cranelift variables as scratch — harmless where the probe was
/// originally called from and not harmless everywhere. Rather than trust the
/// whole of it, the block probe only looks at shapes whose probe bottoms out
/// in literals and names.
/// Names the place shape the backend cannot write through.
///
/// §11 promises every native refusal is "by name". This one said "this
/// assignment target's shape", which names nothing — a reader could not tell
/// whether it was the `ref`, the map, or the depth that was the problem, and
/// the refusal for a handler type in the same program did name its type.
/// Start offsets of closure literals sitting in the arms of a `match` or an
/// `if` that is the whole of an expression — the positions where the type is
/// written on what *holds* the closure rather than beside it.
/// The payload type a `Some`/`Ok`/`Err` constructor puts its argument at.
///
/// `Some(|x| …)` against `Option[fn(i64) -> i64]` puts the closure at
/// `fn(i64) -> i64`, and it is that type the closure's parameters come from —
/// not the annotation as written, which is an `Option`. Without the unwrap the
/// hint recorded `Option[fn(i64) -> i64]` for the closure and the backend, on
/// finding no parameters in it, refused with the message this exists to stop.
fn carrier_payload(callee: &Expr, ty: &TypeExpr) -> Option<TypeExpr> {
    let Expr::Ident(id) = callee else {
        return None;
    };
    match (id.name.as_str(), ty) {
        ("Some", TypeExpr::Name { path, args, .. })
            if path.segments.len() == 1
                && path.segments[0].name == "Option"
                && args.len() == 1 =>
        {
            Some(args[0].clone())
        }
        ("Ok", TypeExpr::Fallible(ok, _, _)) => Some((**ok).clone()),
        ("Err", TypeExpr::Fallible(_, err, _)) => Some((**err).clone()),
        _ => None,
    }
}

/// Closures inside a branching initialiser, each paired with the type the
/// binding's annotation says it has.
///
/// The annotation is on the binding and the closures are in the arms, so
/// nothing connects the two by name — this is what connects them by position.
fn arm_closure_hints(e: &Expr, ty: &TypeExpr) -> Vec<(u32, TypeExpr)> {
    let mut out = Vec::new();
    fn walk(e: &Expr, ty: &TypeExpr, out: &mut Vec<(u32, TypeExpr)>) {
        match e {
            Expr::Closure { params, span, .. } if !params.is_empty() => {
                out.push((span.start, ty.clone()))
            }
            Expr::Match { arms, .. } => {
                for a in arms {
                    walk(&a.body, ty, out);
                }
            }
            // `Some(|x| x + n)` in an arm. The closure is one level inside the
            // constructor, so the annotation has to come one level in with it:
            // `Option[fn(i64) -> i64]` becomes `fn(i64) -> i64`. Without this
            // the refusal said "no type says what its parameters are" about a
            // program whose binding said exactly that, and offered as its fix
            // the annotation the source already had.
            Expr::Call { callee, args, .. } if args.len() == 1 => {
                if let Some(inner) = carrier_payload(callee, ty) {
                    walk(&args[0], &inner, out);
                }
            }
            Expr::If {
                then_block,
                else_branch,
                ..
            } => {
                if let Some(Stmt::Expr(t)) = then_block.stmts.last() {
                    walk(t, ty, out);
                }
                match else_branch.as_deref() {
                    Some(ElseBranch::Else(b)) => {
                        if let Some(Stmt::Expr(t)) = b.stmts.last() {
                            walk(t, ty, out);
                        }
                    }
                    Some(ElseBranch::ElseIf(inner)) => walk(inner, ty, out),
                    None => {}
                }
            }
            Expr::Block(b) => {
                if let Some(Stmt::Expr(t)) = b.stmts.last() {
                    walk(t, ty, out);
                }
            }
            _ => {}
        }
    }
    // Only *inside* a branching shape: a closure written directly is already
    // connected to its binding by name.
    match e {
        Expr::Match { .. } | Expr::If { .. } | Expr::Block(_) => walk(e, ty, &mut out),
        _ => {}
    }
    out
}

fn place_shape(root_t: &VType, seg: &PlaceSeg<'_>) -> String {
    let what = match root_t {
        VType::Ref(_) => "a write through a `ref`",
        VType::Map(..) => "a write into a map through a place",
        VType::Tuple(_) => "a write into a tuple component",
        VType::Str => "a write into a string",
        _ => match seg {
            PlaceSeg::Field(f) => return format!("a write to field `{f}` of this type"),
            PlaceSeg::Index(_) => "an indexed write into this type",
        },
    };
    what.to_string()
}

fn is_probe_safe(e: &Expr) -> bool {
    match e {
        Expr::Int(..) | Expr::Float(..) | Expr::Bool(..) | Expr::Str(..) | Expr::Ident(_) => true,
        Expr::List(items, _) => items.first().is_some_and(is_probe_safe),
        Expr::Tuple(items, _) => items.iter().all(is_probe_safe),
        Expr::Map(entries, _) => entries
            .first()
            .is_some_and(|(k, v)| is_probe_safe(k) && is_probe_safe(v)),
        Expr::Unary { expr, .. } => is_probe_safe(expr),
        Expr::Binary { lhs, rhs, .. } => is_probe_safe(lhs) && is_probe_safe(rhs),
        Expr::StructLit { .. } => true,
        _ => false,
    }
}

/// Whether this arm body always leaves — by returning, breaking, continuing,
/// or failing — and so never reaches the match's merge point.
fn arm_diverges(body: &Expr) -> bool {
    match body {
        // A block that *ends* in something that leaves, by any spelling: a
        // `{ panic("x") }` arm read as an ordinary block yielding nothing, so
        // a `match` with one live arm became a statement and the value was
        // thrown away — `()` printed where the answer was `7`, and `false`
        // where it was `true`.
        Expr::Block(b) | Expr::Unsafe(b) | Expr::Region(b) => match b.stmts.last() {
            Some(Stmt::Return(..) | Stmt::Break(_) | Stmt::Continue(_)) => true,
            Some(Stmt::Expr(e)) => arm_diverges(e),
            _ => false,
        },
        // `exit(n)` ends the program, so an arm that calls it produces no
        // value and says nothing about what the match is worth.
        Expr::Call { callee, .. } => {
            matches!(&**callee, Expr::Ident(id)
                if id.name == "fail" || id.name == "panic" || id.name == "exit")
        }
        _ => false,
    }
}

fn needs_element_hint(e: &Expr) -> bool {
    match e {
        Expr::Closure { params, .. } => !params.is_empty(),
        // An empty container has no element to infer a type from, so the
        // annotation has to reach it. `[[]]` is one element long, so the outer
        // list looked complete, stopped passing the type down, and the inner
        // literal had nowhere left to get it — `let g: [[i64]] = [[]]` was
        // refused with its own annotation in view.
        Expr::List(items, _) => items.is_empty() || items.iter().any(needs_element_hint),
        Expr::Map(entries, _) => {
            entries.is_empty() || entries.iter().any(|(_, v)| needs_element_hint(v))
        }
        Expr::Tuple(items, _) => items.iter().any(needs_element_hint),
        // `Some(|n| n + 1)` is the closure one level in. The payload's type
        // is the annotation's, and passing it through is what lets the
        // closure be written where the type is already named.
        Expr::Call { callee, args, .. } => {
            matches!(&**callee, Expr::Ident(id)
                if matches!(id.name.as_str(), "Some" | "Ok" | "Err"))
                && args.len() == 1
                && needs_element_hint(&args[0])
        }
        _ => false,
    }
}

fn needs_type_hint(e: &Expr) -> bool {
    match e {
        Expr::List(items, _) => items.is_empty(),
        Expr::Map(entries, _) => entries.is_empty(),
        Expr::Call { callee, args, .. } => {
            args.is_empty() && matches!(&**callee, Expr::Ident(id) if id.name == "map")
        }
        _ => false,
    }
}

fn stride_of(vt: &VType) -> i64 {
    match vt {
        VType::Bool => 1,
        _ => 8,
    }
}

/// Names of list locals that appear in `block` **only** as the receiver of an
/// index expression.
///
/// Any other appearance — an argument, a method receiver, a closure capture,
/// the target of a whole-variable assignment — can change the list's refcount
/// or its length, either of which invalidates a raw pointer into it. So the
/// rule is deliberately strict: index it, or it is not pinned.
/// Whether `Any` appears anywhere in a type — the placeholder a value that
/// cannot name its element type carries.
fn any_inside(t: &VType) -> bool {
    match t {
        VType::Any => true,
        VType::List(e) | VType::Option(e) | VType::Ref(e) => any_inside(e),
        VType::Fallible(a, b) | VType::Map(a, b) => any_inside(a) || any_inside(b),
        VType::Tuple(ts) => ts.iter().any(any_inside),
        VType::Fn(ps, r) => ps.iter().any(any_inside) || any_inside(r),
        _ => false,
    }
}

/// Combines two branches' types when both produce a value.
///
/// The only interesting case is a fallible pair where one side is `Err(e)` or
/// `Ok(v)` alone: `Err(e)` has no ok half to name and carries `Any` there,
/// `Ok(v)` has no error half and carries `Any` there. Each side supplies what
/// the other cannot.
fn merge_fallible_halves(a: VType, b: VType) -> VType {
    let pick = |x: &VType, y: &VType| -> VType {
        if matches!(x, VType::Any | VType::Unit) && !matches!(y, VType::Any | VType::Unit) {
            y.clone()
        } else {
            x.clone()
        }
    };
    match (&a, &b) {
        (VType::Fallible(a_ok, a_err), VType::Fallible(b_ok, b_err)) => VType::Fallible(
            Box::new(pick(a_ok, b_ok)),
            Box::new(pick(a_err, b_err)),
        ),
        // One side gives the whole `T ! E` and the other only the ok half —
        // `if n == 1 { 10 } else { Ok(n * 3) }` in a fallible function
        // ([T-OK-ERR-RETURN]). The whole type is the wider of the two, and
        // taking the *first* branch's answer regardless meant the merge was
        // typed `i64`: the `Ok(...)` branch's pointer travelled as a bare
        // integer and was wrapped a second time on the way out, so the caller
        // matched `Ok(v)` and printed a heap address where the number was.
        (VType::Fallible(ok, err), other) => {
            VType::Fallible(Box::new(pick(ok, other)), err.clone())
        }
        (other, VType::Fallible(ok, err)) => {
            VType::Fallible(Box::new(pick(ok, other)), err.clone())
        }
        _ => a,
    }
}

fn indexed_only_lists(block: &grasm_syntax::ast::Block) -> Vec<String> {
    let mut indexed: Vec<String> = Vec::new();
    let mut disqualified: Vec<String> = Vec::new();
    scan_uses_block(block, &mut indexed, &mut disqualified);
    indexed.sort();
    indexed.dedup();
    indexed.retain(|n| !disqualified.contains(n));
    indexed
}

fn scan_uses_block(
    b: &grasm_syntax::ast::Block,
    indexed: &mut Vec<String>,
    bad: &mut Vec<String>,
) {
    for stmt in &b.stmts {
        match stmt {
            Stmt::Let { init, .. } => scan_uses_expr(init, indexed, bad),
            Stmt::Assign { target, value, .. } => {
                // `xs[i] = v` keeps xs eligible; `xs = other` does not.
                match target {
                    Expr::Index { recv, index, .. } => {
                        if let Expr::Ident(id) = &**recv {
                            indexed.push(id.name.clone());
                        } else {
                            scan_uses_expr(recv, indexed, bad);
                        }
                        scan_uses_expr(index, indexed, bad);
                    }
                    other => scan_uses_expr(other, indexed, bad),
                }
                if let Expr::Ident(id) = target {
                    bad.push(id.name.clone());
                }
                scan_uses_expr(value, indexed, bad);
            }
            Stmt::Return(Some(e), _) | Stmt::Expr(e) => scan_uses_expr(e, indexed, bad),
            Stmt::While { cond, body, .. } => {
                scan_uses_expr(cond, indexed, bad);
                scan_uses_block(body, indexed, bad);
            }
            Stmt::For { iter, body, .. } => {
                scan_uses_expr(iter, indexed, bad);
                scan_uses_block(body, indexed, bad);
            }
            Stmt::Parallel { body, .. } | Stmt::Spawn { body, .. } => {
                scan_uses_block(body, indexed, bad)
            }
            Stmt::With { handler, body, .. } => {
                scan_uses_expr(handler, indexed, bad);
                scan_uses_block(body, indexed, bad);
            }
            _ => {}
        }
    }
}

fn scan_uses_expr(e: &Expr, indexed: &mut Vec<String>, bad: &mut Vec<String>) {
    match e {
        // The one shape that keeps a list eligible.
        Expr::Index { recv, index, .. } => {
            if let Expr::Ident(id) = &**recv {
                indexed.push(id.name.clone());
            } else {
                scan_uses_expr(recv, indexed, bad);
            }
            scan_uses_expr(index, indexed, bad);
        }
        // Everything below is a use that could alias, resize, or retain.
        Expr::Ident(id) => bad.push(id.name.clone()),
        Expr::Unary { expr, .. } | Expr::Try { expr, .. } => scan_uses_expr(expr, indexed, bad),
        Expr::Binary { lhs, rhs, .. } => {
            scan_uses_expr(lhs, indexed, bad);
            scan_uses_expr(rhs, indexed, bad);
        }
        Expr::Call { callee, args, .. } => {
            scan_uses_expr(callee, indexed, bad);
            for a in args {
                scan_uses_expr(a, indexed, bad);
            }
        }
        Expr::MethodCall { recv, args, .. } => {
            scan_uses_expr(recv, indexed, bad);
            for a in args {
                scan_uses_expr(a, indexed, bad);
            }
        }
        Expr::Field { recv, .. } => scan_uses_expr(recv, indexed, bad),
        Expr::List(items, _) | Expr::Tuple(items, _) => {
            for i in items {
                scan_uses_expr(i, indexed, bad);
            }
        }
        Expr::Str(pieces, _) => {
            for p in pieces {
                if let StrPiece::Expr(inner) = p {
                    scan_uses_expr(inner, indexed, bad);
                }
            }
        }
        Expr::Closure { body, .. } => scan_uses_expr(body, indexed, bad),
        Expr::Block(b) | Expr::Unsafe(b) | Expr::Region(b) => scan_uses_block(b, indexed, bad),
        Expr::If {
            cond,
            then_block,
            else_branch,
            ..
        } => {
            scan_uses_expr(cond, indexed, bad);
            scan_uses_block(then_block, indexed, bad);
            match else_branch.as_deref() {
                Some(ElseBranch::Else(b)) => scan_uses_block(b, indexed, bad),
                Some(ElseBranch::ElseIf(inner)) => scan_uses_expr(inner, indexed, bad),
                None => {}
            }
        }
        Expr::Match {
            scrutinee, arms, ..
        } => {
            scan_uses_expr(scrutinee, indexed, bad);
            for arm in arms {
                if let Some(g) = &arm.guard {
                    scan_uses_expr(g, indexed, bad);
                }
                scan_uses_expr(&arm.body, indexed, bad);
            }
        }
        _ => {}
    }
}

/// The runtime tag for a type stored inline in a scalar list
/// (0 = i64, 1 = f64, 2 = bool), or None when elements must be boxed.
fn scalar_kind_i64(vt: &VType) -> Option<i64> {
    match vt {
        VType::I64 => Some(0),
        // A reference is one word, so a list of them is an unboxed scalar
        // list like any other — which is what makes an object graph cheap.
        // Its own kind, not `i64`: stored the same, printed as the slot it
        // names rather than as the packed word.
        VType::Ref(_) => Some(3),
        VType::F64 => Some(1),
        VType::Bool => Some(2),
        _ => None,
    }
}

/// One step of a place path (`p.field`, `xs[i]`).
#[derive(Clone, Copy)]
enum PlaceSeg<'e> {
    Field(&'e str),
    Index(&'e Expr),
}

enum StmtOut {
    Value(Option<(Value, VType)>),
    Terminated,
}

impl FnCx<'_, '_> {
    fn push_scope(&mut self) {
        self.vars.push(HashMap::new());
    }

    /// Emits rc-decs for the current scope's boxed locals, then pops it.
    /// Call only on non-terminated paths.
    fn pop_scope_with_decs(&mut self) {
        let scope = self.vars.pop().unwrap();
        for (_, (var, vt)) in scope {
            if vt.boxed() {
                let v = self.b.use_var(var);
                self.call_rt("grasm_val_dec", &[v]);
            }
        }
    }

    fn pop_scope_no_decs(&mut self) {
        self.vars.pop();
    }

    /// Stores `inout` locals back through the caller's pointers.
    fn emit_inout_writeback(&mut self) {
        let slots = self.inout_slots.clone();
        for (ptr, var, _) in slots {
            let v = self.b.use_var(var);
            self.b
                .ins()
                .store(cranelift_codegen::ir::MemFlags::new(), v, ptr, 0);
        }
    }

    /// Releases boxed locals in every scope deeper than `depth`.
    ///
    /// What `break` and `continue` need: they leave the loop body without
    /// running its fall-through scope decs, so anything it declared has to be
    /// released here. The scopes are not popped — the code after this is
    /// unreachable, and the enclosing lowering still owns the bookkeeping.
    fn emit_decs_above(&mut self, depth: usize) {
        let live: Vec<(Variable, VType)> = self
            .vars
            .iter()
            .skip(depth)
            .flat_map(|sc| sc.values().cloned())
            .collect();
        for (var, vt) in live {
            if vt.boxed() {
                let v = self.b.use_var(var);
                self.call_rt("grasm_val_dec", &[v]);
            }
        }
    }

    /// Releases boxed locals in scopes deeper than `depth`, leaving the rest.
    ///
    /// A `return` releases everything; a `fail` that is a closure's own failure
    /// releases only what the closure bound, because the loop around it still
    /// owns the list it is walking and the one it is building.
    fn emit_scope_decs_below(&mut self, depth: usize) {
        let deeper: Vec<(Variable, VType)> = self
            .vars
            .iter()
            .skip(depth)
            .flat_map(|s| s.values().cloned())
            .collect();
        let inout_vars: Vec<Variable> = self.inout_slots.iter().map(|(_, v, _)| *v).collect();
        for (var, vt) in deeper {
            if inout_vars.contains(&var) {
                continue;
            }
            if vt.boxed() {
                let v = self.b.use_var(var);
                self.call_rt("grasm_val_dec", &[v]);
            }
        }
    }

    /// Before a `return`: release every boxed local in every scope.
    fn emit_scope_decs_all(&mut self) {
        self.emit_inout_writeback();
        // Captures held for a handler passed to a call. They are released with
        // the frame rather than after the call, because the block they live in
        // is a pointer into this frame and stays reachable for as long as the
        // callee can install it — which is the whole reason a handler may be
        // passed down and not returned.
        let held = std::mem::take(&mut self.handler_arg_held);
        for (slot, captures) in &held {
            for (i, (_, cvt)) in captures.iter().enumerate() {
                if !cvt.boxed() {
                    continue;
                }
                let bits = self.b.ins().stack_load(types::I64, *slot, (i * 8) as i32);
                self.call_rt("grasm_val_dec", &[bits]);
            }
        }
        self.handler_arg_held = held;
        let all: Vec<(Variable, VType)> = self
            .vars
            .iter()
            .flat_map(|s| s.values().cloned())
            .collect();
        let inout_vars: Vec<Variable> = self.inout_slots.iter().map(|(_, v, _)| *v).collect();
        for (var, vt) in all {
            if inout_vars.contains(&var) {
                continue;
            }
            if vt.boxed() {
                let v = self.b.use_var(var);
                self.call_rt("grasm_val_dec", &[v]);
            }
        }
    }

    fn new_var(&mut self, vt: &VType) -> Variable {
        let var = Variable::from_u32(self.var_count as u32);
        self.var_count += 1;
        self.b.declare_var(var, vt.cl());
        var
    }

    /// Updates the recorded static type of an existing binding.
    fn rebind(&mut self, name: &str, var: Variable, vt: VType) {
        for scope in self.vars.iter_mut().rev() {
            if scope.contains_key(name) {
                scope.insert(name.to_string(), (var, vt));
                return;
            }
        }
    }

    fn bind(&mut self, name: &str, var: Variable, vt: VType) {
        self.vars
            .last_mut()
            .unwrap()
            .insert(name.to_string(), (var, vt));
    }

    /// Whether `e` is a name bound to a range.
    ///
    /// Deliberately only a name: it is asked *before* the receiver is
    /// evaluated, so it cannot evaluate anything itself, and a name is the
    /// shape a held range actually takes. Everything else falls through to the
    /// paths that were already there.
    fn holds_a_range(&self, e: &Expr) -> bool {
        match e {
            Expr::Ident(id) => matches!(self.lookup(&id.name), Some((_, VType::Range))),
            _ => false,
        }
    }

    fn lookup(&self, name: &str) -> Option<(Variable, VType)> {
        for scope in self.vars.iter().rev() {
            if let Some(v) = scope.get(name) {
                return Some(v.clone());
            }
        }
        None
    }

    /// Function lookup in the module currently being compiled.
    fn lookup_fn(&self, name: &str) -> Option<&FnSig> {
        self.cg
            .fns
            .get(&format!("{}::{}", self.cg.current_module, name))
    }

    /// Looks up a pinned list, if `name` is one in the innermost loop.
    fn pinned_list(&self, name: &str) -> Option<(Value, Value, VType)> {
        self.pinned.iter().rev().find_map(|s| s.get(name).cloned())
    }

    /// Pins every scalar list the loop body only ever indexes.
    ///
    /// Returns the names pinned, so the caller can pop them afterwards. A list
    /// used any other way — passed to a function, pushed to, captured by a
    /// closure — is left alone, because any of those can change its refcount
    /// or its length and invalidate the pointer.
    /// Builds each module constant a loop mentions, once, before the loop.
    ///
    /// A constant is its initializer, and the initializer is effect-free, so
    /// building it per mention is only ever a cost — an O(n) rebuild inside
    /// an O(k) loop. [E-CONST] recommends a constant for a dispatch table,
    /// which is read in a loop by definition. Bound under a name no program
    /// can write, so the ordinary scope teardown releases it on every exit
    /// path (`break`, `return`, falling out) without a second mechanism.
    fn pin_loop_consts(&mut self, body: &grasm_syntax::ast::Block) -> CResult<()> {
        let mut names: Vec<String> = Vec::new();
        collect_idents_block(body, &mut names);
        names.sort();
        names.dedup();
        for name in names {
            // A local of the same name shadows the constant.
            if self.lookup(&name).is_some() {
                continue;
            }
            let pin = format!(" const {name}");
            if self.lookup(&pin).is_some() {
                continue;
            }
            let Some(init) = self.const_of(&name) else {
                continue;
            };
            // A constant whose initializer this backend cannot compile is
            // left alone: the mention itself will report it, at its own span.
            let Ok((v, vt)) = self.eval_expr_val(&init) else {
                continue;
            };
            let var = self.new_var(&vt);
            self.b.def_var(var, v);
            self.bind(&pin, var, vt);
        }
        Ok(())
    }

    fn pin_loop_lists(&mut self, body: &grasm_syntax::ast::Block) -> CResult<()> {
        let mut scope: HashMap<String, (Value, Value, VType)> = HashMap::new();
        let candidates: Vec<String> = indexed_only_lists(body);
        for name in candidates {
            let Some((var, VType::List(elem))) = self.lookup(&name) else {
                continue;
            };
            let Some(kind) = scalar_kind_i64(&elem) else {
                continue;
            };
            let ptr_ty = self.cg.module.target_config().pointer_type();
            let slot = self.b.create_sized_stack_slot(
                cranelift_codegen::ir::StackSlotData::new(
                    cranelift_codegen::ir::StackSlotKind::ExplicitSlot,
                    16,
                    3,
                ),
            );
            let data_out = self.b.ins().stack_addr(ptr_ty, slot, 0);
            let len_out = self.b.ins().stack_addr(ptr_ty, slot, 8);
            let cur = self.b.use_var(var);
            let kc = self.b.ins().iconst(types::I64, kind);
            let newlist = self
                .call_rt("grasm_list_pin", &[cur, kc, data_out, len_out])
                .unwrap();
            self.b.def_var(var, newlist);
            let flags = cranelift_codegen::ir::MemFlags::trusted();
            let data = self.b.ins().load(types::I64, flags, data_out, 0);
            let len = self.b.ins().load(types::I64, flags, len_out, 0);
            scope.insert(name, (data, len, (*elem).clone()));
        }
        self.pinned.push(scope);
        Ok(())
    }

    /// Inline bounds check, then a raw load or the address to store to.
    fn pinned_address(&mut self, data: Value, len: Value, idx: Value, stride: i64) -> Value {
        let ok = self.b.create_block();
        let oob = self.b.create_block();
        // Unsigned compare catches negative indices in the same test.
        let bad = self.b.ins().icmp(IntCC::UnsignedGreaterThanOrEqual, idx, len);
        self.b.ins().brif(bad, oob, &[], ok, &[]);
        self.filled = true;

        self.goto_block(oob);
        self.call_rt("grasm_panic_index", &[idx, len]);
        self.trap_here();
        self.filled = true;

        self.goto_block(ok);
        let w = self.b.ins().iconst(types::I64, stride);
        let off = self.b.ins().imul(idx, w);
        self.b.ins().iadd(data, off)
    }

    /// A zero value of any native type — probing binds dummies with this.
    /// `iconst` panics on float types, which is easy to hit and hard to trace.
    fn zero_of(&mut self, vt: &VType) -> Value {
        match vt.cl() {
            types::F64 => self.b.ins().f64const(0.0),
            t => self.b.ins().iconst(t, 0),
        }
    }

    /// A new empty list whose elements have type `elem_t` — inline scalar
    /// storage when the type allows it, boxed cells otherwise.
    fn new_list_of(&mut self, elem_t: &VType) -> Value {
        match scalar_kind_i64(elem_t) {
            Some(k) => {
                let kc = self.b.ins().iconst(types::I64, k);
                self.call_rt("grasm_list_new_scalar", &[kc]).unwrap()
            }
            None => self.call_rt("grasm_list_new", &[]).unwrap(),
        }
    }

    /// One element out of a list: the typed inline read when the element type
    /// is scalar, the boxed read + unbox otherwise.
    fn list_get_elem(&mut self, list: Value, idx: Value, elem_t: &VType) -> CResult<(Value, VType)> {
        if let Some(k) = scalar_kind_i64(elem_t) {
            let kc = self.b.ins().iconst(types::I64, k);
            let bits = self
                .call_rt("grasm_list_get_scalar", &[list, idx, kc])
                .unwrap();
            let v = self.scalar_of_bits(bits, elem_t);
            return Ok((v, elem_t.clone()));
        }
        let boxed = self.call_rt("grasm_list_get", &[list, idx]).unwrap();
        self.unbox(boxed, elem_t)
    }

    /// Appends `v` (a native value of type `vt`) to `list`, choosing the
    /// inline or the boxed path. Consumes `v`'s reference either way.
    fn list_push_elem(&mut self, list: Value, v: Value, vt: &VType) -> Value {
        if let Some(k) = scalar_kind_i64(vt) {
            let bits = self.scalar_bits_of(v, vt);
            let kc = self.b.ins().iconst(types::I64, k);
            return self
                .call_rt("grasm_list_push_scalar", &[list, bits, kc])
                .unwrap();
        }
        let bx = self.box_value(v, vt);
        self.call_rt("grasm_list_push", &[list, bx]).unwrap()
    }

    /// A scalar as the i64 bit pattern a `ListScalar` stores.
    fn scalar_bits_of(&mut self, v: Value, vt: &VType) -> Value {
        match vt {
            // A reference is already the word it is stored as.
            VType::I64 | VType::Ref(_) => v,
            VType::F64 => {
                self.b
                    .ins()
                    .bitcast(types::I64, cranelift_codegen::ir::MemFlags::new(), v)
            }
            VType::Bool => self.b.ins().uextend(types::I64, v),
            _ => unreachable!("only scalars are stored inline"),
        }
    }

    /// Eight bytes standing for any value: a scalar's bits, or a boxed
    /// value's pointer. Used for a handler's captured environment, which is a
    /// flat slot array rather than a typed structure.
    fn value_to_bits(&mut self, v: Value, vt: &VType) -> Value {
        match vt {
            VType::I64 | VType::F64 | VType::Bool | VType::Ref(_) => {
                self.scalar_bits_of(v, vt)
            }
            _ => v,
        }
    }

    fn bits_to_value(&mut self, bits: Value, vt: &VType) -> Value {
        match vt {
            VType::I64 | VType::F64 | VType::Bool | VType::Ref(_) => {
                self.scalar_of_bits(bits, vt)
            }
            _ => bits,
        }
    }

    fn scalar_of_bits(&mut self, bits: Value, vt: &VType) -> Value {
        match vt {
            VType::I64 | VType::Ref(_) => bits,
            VType::F64 => {
                self.b
                    .ins()
                    .bitcast(types::F64, cranelift_codegen::ir::MemFlags::new(), bits)
            }
            VType::Bool => self.b.ins().ireduce(types::I8, bits),
            _ => unreachable!("only scalars are stored inline"),
        }
    }

    /// Switches to `blk`, resetting the terminator flag.
    fn goto_block(&mut self, blk: Block) {
        self.b.switch_to_block(blk);
        self.filled = false;
    }

    fn is_filled(&self) -> bool {
        self.filled
    }

    /// `a..b` as a value: three scalars in one boxed cell.
    ///
    /// `for i in 0..n` never reaches here — the backend compiles that shape
    /// into a counted loop, which is why it costs nothing and still does. This
    /// is the other case: a range bound to a name, passed, printed, compared.
    fn eval_range_value(
        &mut self,
        start: &Expr,
        end: &Expr,
        inclusive: bool,
        span: Span,
    ) -> CResult<(Value, VType)> {
        let (s, st) = self.eval_expr_val(start)?;
        let (e, et) = self.eval_expr_val(end)?;
        if st != VType::I64 || et != VType::I64 {
            return nope(span, "non-integer ranges");
        }
        let incl = self.b.ins().iconst(types::I8, i64::from(inclusive));
        let v = self.call_rt("grasm_range_new", &[s, e, incl]).unwrap();
        Ok((v, VType::Range))
    }

    fn call_rt(&mut self, name: &'static str, args: &[Value]) -> Option<Value> {
        let id = self.cg.runtime[name];
        let fref = self.cg.module.declare_func_in_func(id, self.b.func);
        // The runtime is a static archive linked into this same binary, so
        // every one of these is a PC-relative branch and not a hop through the
        // PLT. Declared `Import`, cranelift-module assumes the other side may
        // be in another object and emits the indirection — which is a load, a
        // dependent branch, and a full clobber the register allocator has to
        // spill around.
        //
        // Measured on `fib(32)`, which calls `grasm_stack_check` once per
        // invocation: the whole program is 22.9 ms with the indirect call and
        // 11.6 ms with no call at all, so the call — not the check it performs
        // — was half the runtime of every recursive function in the language.
        self.b.func.dfg.ext_funcs[fref].colocated = true;
        let inst = self.b.ins().call(fref, args);
        self.b.inst_results(inst).first().copied()
    }

    fn dec_if_boxed(&mut self, v: Value, vt: &VType) {
        if vt.boxed() {
            self.call_rt("grasm_val_dec", &[v]);
        }
    }

    fn inc_if_boxed(&mut self, v: Value, vt: &VType) {
        if vt.boxed() {
            self.call_rt("grasm_val_inc", &[v]);
        }
    }

    /// A pure field path rooted at a plain local, borrowed at every step.
    ///
    /// `o.i.v` fetched `i` and incremented it, read `v`, then released `i` —
    /// and that pair exists only to protect a value the root already protects.
    /// Every intermediate in a path like this is reachable from the local, so
    /// none of them can be freed during the read.
    ///
    /// Answers `None` for anything that is not a chain of field names over a
    /// struct local. An index or a call in the middle could run code that frees
    /// what an earlier step borrowed, so those keep the owning path.
    fn borrow_struct_path(&mut self, e: &Expr) -> CResult<Option<(Value, String)>> {
        match e {
            Expr::Ident(id) => Ok(match self.lookup(&id.name) {
                Some((var, VType::Struct(sn))) => Some((self.b.use_var(var), sn)),
                _ => None,
            }),
            Expr::Field { recv, name, span } => {
                let Some((base, sname)) = self.borrow_struct_path(recv)? else {
                    return Ok(None);
                };
                // A field this cannot resolve is not this shape. Propagating
                // the refusal would let a *borrowing optimisation* decide the
                // program cannot be compiled, when the ordinary path below it
                // both compiles the read and produces the right diagnostic if
                // there is one — the same rule as F42.
                let Ok(VType::Struct(inner)) = self.struct_field_type(&sname, &name.name, *span)
                else {
                    return Ok(None);
                };
                let Some(i) = self.struct_field_index(&sname, &name.name) else {
                    return Ok(None);
                };
                let iv = self.b.ins().iconst(types::I64, i);
                let v = self
                    .call_rt("grasm_struct_borrow_at", &[base, iv])
                    .unwrap();
                Ok(Some((v, inner)))
            }
            _ => Ok(None),
        }
    }

    /// A string literal, made once and cached in a slot of its own.
    ///
    /// The value is `IMMORTAL`, so the caller does not have to know it is
    /// shared: `inc` and `dec` skip it, and nothing frees it. That is also what
    /// makes this safe against `grasm_str_append`, which mutates in place only
    /// when the count is exactly 1 — an immortal's never is, so appending to a
    /// literal copies it.
    fn str_literal(&mut self, text: &str) -> Value {
        let ptr_ty = self.cg.module.target_config().pointer_type();
        let slot_id = self.cg.str_lit_slot(text);
        let gv = self.cg.module.declare_data_in_func(slot_id, self.b.func);
        let slot = self.b.ins().global_value(ptr_ty, gv);
        let flags = cranelift_codegen::ir::MemFlags::trusted();
        let cached = self.b.ins().load(types::I64, flags, slot, 0);

        let make_blk = self.b.create_block();
        let done_blk = self.b.create_block();
        self.b.append_block_param(done_blk, types::I64);
        self.b.ins().brif(cached, done_blk, &[cached], make_blk, &[]);

        self.goto_block(make_blk);
        let (p, l) = self.str_ptr_len(text);
        let made = self.call_rt("grasm_str_lit_immortal", &[p, l]).unwrap();
        self.b.ins().store(flags, made, slot, 0);
        self.b.ins().jump(done_blk, &[made]);

        self.goto_block(done_blk);
        self.b.block_params(done_blk)[0]
    }

    fn str_ptr_len(&mut self, s: &str) -> (Value, Value) {
        let data_id = self.cg.intern_string(s);
        let gv = self.cg.module.declare_data_in_func(data_id, self.b.func);
        let ptr_ty = self.cg.module.target_config().pointer_type();
        let ptr = self.b.ins().global_value(ptr_ty, gv);
        let len = self.b.ins().iconst(types::I64, s.len() as i64);
        (ptr, len)
    }

    fn trap_here(&mut self) {
        self.b
            .ins()
            .trap(cranelift_codegen::ir::TrapCode::unwrap_user(1));
    }

    // ------------------------------------------------------------------
    // Blocks & statements
    // ------------------------------------------------------------------

    fn eval_block_inner(
        &mut self,
        blk: &grasm_syntax::ast::Block,
    ) -> CResult<(Option<(Value, VType)>, bool)> {
        self.push_scope();
        let mut hints = collect_block_hints(blk);
        self.declared_fn_arg_types(blk, &mut hints);
        self.closure_hints.push(hints);
        // The hint belongs to this block's tail and to no other. Taking it
        // means a nested block sees nothing — a `fail` inside an `if` is not
        // the function's return value, and typing it as one is how this first
        // went wrong.
        let tail_hint = self.tail_hint.take();
        let mut last: Option<(Value, VType)> = None;
        let mut terminated = false;
        for (i, stmt) in blk.stmts.iter().enumerate() {
            if terminated {
                break;
            }
            let is_last = i + 1 == blk.stmts.len();
            if is_last {
                self.tail_hint = tail_hint.clone();
            }
            let out = self.eval_stmt(stmt, is_last)?;
            self.tail_hint = None;
            match out {
                StmtOut::Value(v) => {
                    // Non-final boxed temporaries were already released inside
                    // eval_stmt; only the final value flows out.
                    last = v;
                }
                StmtOut::Terminated => terminated = true,
            }
        }
        if terminated {
            self.pop_scope_no_decs();
        } else {
            self.pop_scope_with_decs();
        }
        self.closure_hints.pop();
        Ok((last, terminated))
    }

    fn eval_stmt(&mut self, s: &Stmt, is_last: bool) -> CResult<StmtOut> {
        match s {
            // `let (a, b) = pair` — the tuple is a list underneath, so each
            // name is bound to one element read out of it.
            Stmt::LetTuple {
                names, init, span, ..
            } => {
                let (tv, tt) = self.eval_expr_val(init)?;
                let VType::Tuple(part_ts) = tt else {
                    return nope(*span, "destructuring a non-tuple");
                };
                if part_ts.len() != names.len() {
                    return nope(*span, "a tuple destructured into a different number of names");
                }
                for (i, (n, pt)) in names.iter().zip(&part_ts).enumerate() {
                    let idx = self.b.ins().iconst(types::I64, i as i64);
                    let (v, vt) = self.list_get_elem(tv, idx, pt)?;
                    let var = self.new_var(&vt);
                    self.b.def_var(var, v);
                    self.bind(&n.name, var, vt);
                }
                self.dec_if_boxed(tv, &VType::Tuple(part_ts));
                Ok(StmtOut::Value(None))
            }
            Stmt::Let {
                name, ty, init, ..
            } => {
                // A handler bound to a name is consumed by the pass that lifts
                // its operations into functions, not by this one. There is no
                // runtime value to produce: `EHandler` is a type the effect
                // declaration conjures, not a struct anyone can build.
                if self.cg.is_handler_literal(init) {
                    // No runtime value to produce — `EHandler` is a type the
                    // effect declaration conjures, not a struct anyone can
                    // build. But its operations are closures, and a closure
                    // captures the values it uses *at the moment it is
                    // created* ([E-CLOSURE-CAPTURE]). The environment was
                    // filled at the `with` instead, so a binding changed
                    // between the two was read at its new value: `var n = 1`,
                    // build the handler, `n = 100`, install it — the
                    // interpreter answered 1 and the compiled binary answered
                    // 100, with matching exit codes and nothing on stderr.
                    // Copied here, the `with` finds what the literal saw.
                    self.snapshot_handler_captures(init);
                    return Ok(StmtOut::Value(None));
                }
                // An annotation gives empty collection literals their type.
                let expected = match ty {
                    Some(te) => Some(self.cg.vtype_of(te)?),
                    None => None,
                };
                // A closure needs to know what it was bound to: that name is
                // how its parameter types are found at a call site.
                let (mut v, mut vt) = match init {
                    Expr::Closure {
                        params,
                        body,
                        span: cspan,
                    } => self.make_closure_value(params, body, Some(&name.name), *cspan)?,
                    // A `match` reads the annotation as its tail hint, which
                    // is how the remedy §11 names — "lift it into an
                    // annotated `let`" — actually reaches the arms.
                    Expr::Match { .. } => {
                        let saved = std::mem::replace(&mut self.tail_hint, expected.clone());
                        let out = self.eval_expr_expecting(init, expected.as_ref());
                        self.tail_hint = saved;
                        out?
                    }
                    _ => self.eval_expr_expecting(init, expected.as_ref())?,
                };
                if let Some(te) = ty {
                    let want = self.cg.vtype_of(te)?;
                    if want == VType::F64 && vt == VType::I64 {
                        v = self.b.ins().fcvt_from_sint(types::F64, v);
                    }
                    // A binding annotated `T ! E` may be given either half,
                    // exactly as a fallible function's return position is
                    // ([T-OK-ERR-RETURN]) — `let v: str ! E = if c { "a" }
                    // else { "b" }` produced a bare string and bound it under
                    // a type that says it is a boxed variant, so reading it
                    // back dereferenced a `str` as one: a wrong answer for
                    // some payloads and a misaligned-pointer abort for others.
                    if let VType::Fallible(ok_t, _) = want.clone() {
                        if !matches!(vt, VType::Fallible(..)) {
                            let mut inner = v;
                            if *ok_t == VType::F64 && vt == VType::I64 {
                                inner = self.b.ins().fcvt_from_sint(types::F64, inner);
                            }
                            let boxed = self.box_value(inner, &ok_t);
                            v = self.call_rt("grasm_ok_new", &[boxed]).unwrap();
                        }
                    }
                    vt = want;
                }
                let var = self.new_var(&vt);
                self.b.def_var(var, v);
                self.bind(&name.name, var, vt);
                Ok(StmtOut::Value(None))
            }
            Stmt::Assign {
                target,
                op,
                value,
                span,
                ..
            } => {
                self.index_vals.clear();
                // Field/index places (any depth, plain or compound).
                if matches!(target, Expr::Field { .. } | Expr::Index { .. }) {
                    let (root, path) = self.place_path(target)?;
                    // The target's own subexpressions run first, before the
                    // value: `xs[f()] = g()` calls `f` then `g`
                    // ([E-ORDER-LTR]). The walk that writes is what evaluates
                    // them, so without this they ran last.
                    let index_spans: Vec<Span> = path
                        .iter()
                        .filter_map(|seg| match seg {
                            PlaceSeg::Index(ix) => Some(ix.span()),
                            _ => None,
                        })
                        .collect();
                    if !index_spans.is_empty() {
                        let exprs: Vec<&Expr> = path
                            .iter()
                            .filter_map(|seg| match seg {
                                PlaceSeg::Index(ix) => Some(*ix),
                                _ => None,
                            })
                            .collect();
                        for (ix, sp) in exprs.iter().zip(&index_spans) {
                            let v = self.eval_expr_val(ix)?;
                            self.index_vals.insert(sp.start, v);
                        }
                    }
                    let Some((var, root_t)) = self.lookup(&root.name) else {
                        return nope(root.span, "assignment to unknown variable");
                    };
                    // `xs[i] = v` on a scalar-element list is a load and a
                    // store on inline data — the box/write-back machinery
                    // below would allocate in the hot loop this exists for.
                    if let (VType::List(eb), [PlaceSeg::Index(ix)]) = (&root_t, path.as_slice()) {
                        // Pinned: read-modify-write is a load and a store.
                        if let Some((data, len, pt)) = self.pinned_list(&root.name) {
                            if pt == **eb {
                                let elem_t = (**eb).clone();
                                let (mut val, val_t) =
                                    self.eval_expr_expecting(value, Some(&elem_t))?;
                                if elem_t == VType::F64 && val_t == VType::I64 {
                                    val = self.b.ins().fcvt_from_sint(types::F64, val);
                                }
                                let (iv, _) = self.index_value(ix)?;
                                let addr = self.pinned_address(data, len, iv, stride_of(&elem_t));
                                let flags = cranelift_codegen::ir::MemFlags::trusted();
                                let final_v = if *op == AssignOp::Assign {
                                    val
                                } else {
                                    let old = self.b.ins().load(elem_t.cl(), flags, addr, 0);
                                    let bop = match op {
                                        AssignOp::Add => BinaryOp::Add,
                                        AssignOp::Sub => BinaryOp::Sub,
                                        AssignOp::Mul => BinaryOp::Mul,
                                        AssignOp::Div => BinaryOp::Div,
                                        AssignOp::Assign => unreachable!(),
                                    };
                                    self.binop_values(
                                        bop,
                                        old,
                                        elem_t.clone(),
                                        val,
                                        val_t,
                                        target.span(),
                                    )?
                                    .0
                                };
                                self.b.ins().store(flags, final_v, addr, 0);
                                return Ok(StmtOut::Value(None));
                            }
                        }
                        if let Some(kind) = scalar_kind_i64(eb) {
                            let elem_t = (**eb).clone();
                            let (mut val, val_t) =
                                self.eval_expr_expecting(value, Some(&elem_t))?;
                            if elem_t == VType::F64 && val_t == VType::I64 {
                                val = self.b.ins().fcvt_from_sint(types::F64, val);
                            }
                            let final_v = if *op == AssignOp::Assign {
                                val
                            } else {
                                let cur = self.b.use_var(var);
                                let (iv, _) = self.index_value(ix)?;
                                let kc = self.b.ins().iconst(types::I64, kind);
                                let bits = self
                                    .call_rt("grasm_list_get_scalar", &[cur, iv, kc])
                                    .unwrap();
                                let old = self.scalar_of_bits(bits, &elem_t);
                                let bop = match op {
                                    AssignOp::Add => BinaryOp::Add,
                                    AssignOp::Sub => BinaryOp::Sub,
                                    AssignOp::Mul => BinaryOp::Mul,
                                    AssignOp::Div => BinaryOp::Div,
                                    AssignOp::Assign => unreachable!(),
                                };
                                self.binop_values(
                                    bop,
                                    old,
                                    elem_t.clone(),
                                    val,
                                    val_t,
                                    target.span(),
                                )?
                                .0
                            };
                            let (iv2, _) = self.index_value(ix)?;
                            let bits = self.scalar_bits_of(final_v, &elem_t);
                            let kc = self.b.ins().iconst(types::I64, kind);
                            let cur2 = self.b.use_var(var);
                            let updated = self
                                .call_rt("grasm_list_set_scalar", &[cur2, iv2, bits, kc])
                                .unwrap();
                            self.b.def_var(var, updated);
                            return Ok(StmtOut::Value(None));
                        }
                    }
                    let elem_t = self.place_type(&root_t, &path, target.span())?;
                    let (mut val, val_t) = self.eval_expr_expecting(value, Some(&elem_t))?;
                    if elem_t == VType::F64 && val_t == VType::I64 {
                        val = self.b.ins().fcvt_from_sint(types::F64, val);
                    }
                    let final_v = if *op == AssignOp::Assign {
                        val
                    } else {
                        let cur = self.b.use_var(var);
                        let old = self.read_place(cur, &path, &root_t, &elem_t)?;
                        let bop = match op {
                            AssignOp::Add => BinaryOp::Add,
                            AssignOp::Sub => BinaryOp::Sub,
                            AssignOp::Mul => BinaryOp::Mul,
                            AssignOp::Div => BinaryOp::Div,
                            AssignOp::Assign => unreachable!(),
                        };
                        self.binop_values(
                            bop,
                            old,
                            elem_t.clone(),
                            val,
                            val_t,
                            target.span(),
                        )?
                        .0
                    };
                    let boxed = self.box_value(final_v, &elem_t);
                    let cur = self.b.use_var(var);
                    self.begin_write();
                    let updated = self.write_place(*span, cur, &path, &root_t, boxed)?;
                    self.b.def_var(var, updated);
                    return Ok(StmtOut::Value(None));
                }
                let Expr::Ident(id) = target else {
                    return nope(target.span(), "assignment to fields/indexes");
                };
                let Some((var, vt)) = self.lookup(&id.name) else {
                    return nope(id.span, "assignment to unknown variable");
                };
                // `acc = acc + piece` and `acc += piece` are the same
                // statement, and both are the loop that builds a string. The
                // variable's own reference is handed to the append rather than
                // copied, so the accumulator grows in place instead of being
                // copied whole every step.
                if vt == VType::Str {
                    let rhs = match (op, value) {
                        (AssignOp::Add, v) => Some(v),
                        (
                            AssignOp::Assign,
                            Expr::Binary {
                                op: BinaryOp::Add,
                                lhs,
                                rhs,
                                ..
                            },
                        ) if matches!(&**lhs, Expr::Ident(l) if l.name == id.name) => {
                            Some(&**rhs)
                        }
                        _ => None,
                    };
                    if let Some(piece) = rhs {
                        let (rv, rt) = self.eval_expr_val(piece)?;
                        if rt != VType::Str {
                            return nope(piece.span(), "adding a non-string to a string");
                        }
                        let old = self.b.use_var(var);
                        let joined =
                            self.call_rt("grasm_str_append", &[old, rv]).unwrap();
                        self.call_rt("grasm_val_dec", &[rv]);
                        self.b.def_var(var, joined);
                        return Ok(StmtOut::Value(None));
                    }
                }
                let (mut val, val_t) = self.eval_expr_val(value)?;
                if vt == VType::F64 && val_t == VType::I64 {
                    val = self.b.ins().fcvt_from_sint(types::F64, val);
                }
                let final_v = if *op == AssignOp::Assign {
                    // Release the previous boxed value in the slot.
                    if vt.boxed() {
                        let old = self.b.use_var(var);
                        self.call_rt("grasm_val_dec", &[old]);
                    }
                    val
                } else {
                    let old = self.b.use_var(var);
                    let bop = match op {
                        AssignOp::Add => BinaryOp::Add,
                        AssignOp::Sub => BinaryOp::Sub,
                        AssignOp::Mul => BinaryOp::Mul,
                        AssignOp::Div => BinaryOp::Div,
                        AssignOp::Assign => unreachable!(),
                    };
                    // For str += : concat then release old.
                    let (nv, _) =
                        self.binop_values(bop, old, vt.clone(), val, val_t, target.span())?;
                    if vt.boxed() {
                        // binop released operands (incl. old use) appropriately.
                    }
                    nv
                };
                self.b.def_var(var, final_v);
                Ok(StmtOut::Value(None))
            }
            Stmt::Expr(e) => {
                // In tail position the function's return type is what an
                // empty collection literal has to take its element type from.
                // Nothing else says what `fn f() -> [str] { [] }` returns, and
                // the checker has already agreed it is a `[str]` — this only
                // tells the backend what the checker worked out.
                let v = match self.tail_hint.clone() {
                    Some(t) if is_last && needs_type_hint(e) => {
                        self.eval_expr_expecting(e, Some(&t)).map(Some)?
                    }
                    _ => self.eval_expr(e)?,
                };
                if is_last {
                    Ok(StmtOut::Value(v))
                } else {
                    if let Some((val, vt)) = v {
                        self.dec_if_boxed(val, &vt);
                    }
                    Ok(StmtOut::Value(None))
                }
            }
            Stmt::Return(v, _) => {
                match v {
                    Some(e) => {
                        // `return Ok(v)` / `return Err(e)` build the *whole*
                        // `T ! E`, so they are not wrapped again — expecting
                        // the ok half made the returned value a pointer read
                        // as an `i64` and printed as one ([T-OK-ERR-RETURN]).
                        // An `if` or `match` merges its branches to the whole
                        // `T ! E` when any of them builds one, so the whole
                        // type is the hint that reaches them — the same rule
                        // the checker follows.
                        let wrapped = matches!(e, Expr::Call { callee, .. }
                            if matches!(&**callee, Expr::Ident(id)
                                if id.name == "Ok" || id.name == "Err"))
                            || matches!(e, Expr::If { .. } | Expr::Match { .. });
                        let expect = match (&self.ret, wrapped) {
                            (VType::Fallible(ok, _), false) => (**ok).clone(),
                            (other, _) => other.clone(),
                        };
                        let (mut val, vt) = self.eval_expr_expecting(e, Some(&expect))?;
                        if self.ret == VType::F64 && vt == VType::I64 {
                            val = self.b.ins().fcvt_from_sint(types::F64, val);
                        }
                        // Wrapped by what it *produced*, not by how it was
                        // written: `return if c { 3 } else { Ok(4) }` gives the
                        // whole thing already, and wrapping it a second time
                        // returned a pointer the caller printed as a number.
                        if let VType::Fallible(ok_t, _) = self.ret.clone() {
                            if !matches!(vt, VType::Fallible(..)) {
                                let mut inner = val;
                                if *ok_t == VType::F64 && vt == VType::I64 {
                                    inner = self.b.ins().fcvt_from_sint(types::F64, inner);
                                }
                                let boxed = self.box_value(inner, &ok_t);
                                val = self.call_rt("grasm_ok_new", &[boxed]).unwrap();
                            }
                        }
                        // Returned value stays owned; release the rest.
                        // (Locals holding the same box are fine: their dec is
                        // balanced by the +1 the Ident eval added.)
                        self.emit_scope_decs_all();
                        self.b.ins().return_(&[val]);
                self.filled = true;
                    }
                    None => {
                        if let VType::Fallible(ok_t, _) = self.ret.clone() {
                            // Bare `return` in a fallible function is `Ok(())`.
                            let z = self.b.ins().iconst(types::I64, 0);
                            let inner = if *ok_t == VType::Unit {
                                self.call_rt("grasm_box_i64", &[z]).unwrap()
                            } else {
                                self.box_value(z, &ok_t)
                            };
                            let wrapped = self.call_rt("grasm_ok_new", &[inner]).unwrap();
                            self.emit_scope_decs_all();
                            self.b.ins().return_(&[wrapped]);
                        } else {
                            self.emit_scope_decs_all();
                            self.b.ins().return_(&[]);
                        }
                        self.filled = true;
                    }
                }
                Ok(StmtOut::Terminated)
            }
            Stmt::Break(span) => {
                let Some(&(_, break_blk, depth)) = self.loops.last() else {
                    return nope(*span, "break outside loop");
                };
                self.emit_decs_above(depth);
                self.b.ins().jump(break_blk, &[]);
                self.filled = true;
                Ok(StmtOut::Terminated)
            }
            Stmt::Continue(span) => {
                let Some(&(cont_blk, _, depth)) = self.loops.last() else {
                    return nope(*span, "continue outside loop");
                };
                self.emit_decs_above(depth);
                self.b.ins().jump(cont_blk, &[]);
                self.filled = true;
                Ok(StmtOut::Terminated)
            }
            Stmt::While { cond, body, .. } => {
                // Pinning emits code, so it has to happen while the current
                // block is still open — before the jump that closes it.
                // Lists the body only indexes get a raw pointer for the whole
                // loop, turning each access from a call into a load.
                self.pin_loop_consts(body)?;
                self.pin_loop_lists(body)?;
                let header = self.b.create_block();
                let body_blk = self.b.create_block();
                let exit = self.b.create_block();
                self.b.ins().jump(header, &[]);
                self.filled = true;
                self.goto_block(header);
                let (c, ct) = self.eval_expr_val(cond)?;
                if ct != VType::Bool {
                    self.pinned.pop();
                    return nope(cond.span(), "non-bool while condition");
                }
                self.b.ins().brif(c, body_blk, &[], exit, &[]);
                self.filled = true;
                self.goto_block(body_blk);
                self.loops.push((header, exit, self.vars.len()));
                let inner = self.eval_block_inner(body);
                self.loops.pop();
                self.pinned.pop();
                let (_, mut terminated) = inner?;
                terminated |= self.is_filled();
                if !terminated {
                    self.b.ins().jump(header, &[]);
                    self.filled = true;
                }
                self.goto_block(exit);
                Ok(StmtOut::Value(None))
            }
            Stmt::For {
                var: loop_var,
                iter,
                body,
                span,
            } => self.eval_for(loop_var, iter, body, *span),
            Stmt::Parallel { body, .. } => {
                self.emit_parallel(body)?;
                Ok(StmtOut::Value(None))
            }
            Stmt::Spawn { body: _, span } => {
                // Records that this site fired. The body itself is lowered by
                // the enclosing `parallel`, after the block finishes.
                let Some((list_var, next_index)) = self.task_scopes.last_mut() else {
                    // The checker rejects this; a backstop, not a path.
                    return nope(*span, "`spawn` outside a `parallel` block");
                };
                let (list_var, index) = (*list_var, *next_index);
                self.task_scopes.last_mut().expect("checked above").1 += 1;
                let idx = self.b.ins().iconst(types::I64, index as i64);
                let cur = self.b.use_var(list_var);
                let next = self.list_push_elem(cur, idx, &VType::I64);
                self.b.def_var(list_var, next);
                Ok(StmtOut::Value(None))
            }
            Stmt::With {
                effect,
                handler,
                body,
                span,
            } => {
                let ename = effect.dotted();
                // Where the handler literal was *written*, which is where its
                // operations captured. An inline literal is its own site.
                let resolved_span = self
                    .cg
                    .handler_literal(handler)
                    .map(|lit| lit.span().start)
                    .unwrap_or(span.start);
                // A handler that arrived as a *value* — a parameter. Its
                // operations were lifted where the literal was written, in the
                // caller, and it carries their code and environment pointers.
                // Installing them needs no literal here, which is the whole
                // point: this function does not know which handler it got.
                // Anything whose *type* is this effect's handler: a parameter,
                // a loop variable, a list element, a field, the result of a
                // call. Only a literal written here — or a name bound to one —
                // takes the lifted path below, and that is the fast one.
                if self.cg.handler_literal(handler).is_none()
                    || !self.cg.handler_sites.contains_key(&span.start)
                {
                    let (hv, ht) = self.eval_expr_val(handler)?;
                    if ht == VType::Handler(ename.clone()) {
                        let hvar = self.new_var(&ht);
                        self.b.def_var(hvar, hv);
                        // Bound into a scope of its own so the reference is
                        // released however the block is left — including by a
                        // `return` from inside it.
                        self.push_scope();
                        self.bind(&format!(" with.handler{}", span.start), hvar, ht);
                        let out = self.with_handler_value(&ename, hvar, body, *span);
                        self.pop_scope_with_decs();
                        return out;
                    }
                    return nope(
                        handler.span(),
                        format!("a `with {ename}` whose handler is `{}`", ht.show()),
                    );
                }
                let Some(site) = self.cg.handler_sites.get(&span.start).cloned() else {
                    // Two ways to get here, and they want different messages.
                    // A handler expression that is not a literal and not a
                    // handler value is the first; the second is a `with` inside
                    // a `parallel`/`spawn` body, whose statements are compiled
                    // into a task rather than into this function, so the
                    // lifting pass never reached it.
                    if self.cg.handler_literal(handler).is_none() {
                        return nope(
                            handler.span(),
                            "a handler that is not a struct literal, a `let` bound \
to one, or a parameter of handler type — a loop variable or a list element is \
this case",
                        );
                    }
                    return nope(
                        *span,
                        format!("a `with {ename}` block inside `parallel` or `spawn`"),
                    );
                };
                let (ep, el) = self.str_ptr_len(&ename);
                self.call_rt("grasm_handler_push", &[ep, el]);
                let ptr_ty = self.cg.module.target_config().pointer_type();
                let mut held: Vec<(cranelift_codegen::ir::StackSlot, Vec<(String, VType)>)> =
                    Vec::new();
                for (opname, opcode, free) in site {
                    let fid = self.handler_op_func(&opcode, *span)?;
                    // Free names that resolve to something in scope here are
                    // the operation's captures. They are typed now, at the
                    // `with` site, because the pass that lifted the operation
                    // ran before any body was compiled and had no types yet.
                    // Read through the snapshot taken where the literal was
                    // written, when there is one. The name recorded is still
                    // the original — that is what the operation's body reads.
                    let lit_span = resolved_span;
                    let captures: Vec<(String, VType)> = free
                        .iter()
                        .filter_map(|n| {
                            self.lookup(&Self::snapshot_name(lit_span, n))
                                .or_else(|| self.lookup(n))
                                .map(|(_, vt)| (n.clone(), vt))
                        })
                        .collect();
                    // A slot in this frame. The frame cannot return while the
                    // handler is installed, so the pointer stays good for as
                    // long as anything can call the operation — including from
                    // functions several calls deep.
                    let slot = self.b.create_sized_stack_slot(
                        cranelift_codegen::ir::StackSlotData::new(
                            cranelift_codegen::ir::StackSlotKind::ExplicitSlot,
                            (captures.len().max(1) * 8) as u32,
                            3,
                        ),
                    );
                    for (i, (cname, cvt)) in captures.iter().enumerate() {
                        let Some((var, _)) = self
                            .lookup(&Self::snapshot_name(lit_span, cname))
                            .or_else(|| self.lookup(cname))
                        else {
                            continue;
                        };
                        let v = self.b.use_var(var);
                        let bits = self.value_to_bits(v, cvt);
                        // The capture is a *value*, and for a boxed one that
                        // means holding a reference to it ([E-CLOSURE-CAPTURE]).
                        // Storing the bare pointer did not: the binding stayed
                        // uniquely owned, so a later `xs.push(v)` mutated the
                        // very list the handler had captured. `[E-COW]` copies
                        // on mutation of a *shared* value, and this made it
                        // look unshared. The interpreter, which clones its
                        // reference-counted value, got this right, so the two
                        // engines disagreed about what the program computed —
                        // and reassigning the binding freed what the handler
                        // still pointed at, which crashed.
                        if cvt.boxed() {
                            self.call_rt("grasm_val_inc", &[v]);
                        }
                        self.b.ins().stack_store(bits, slot, (i * 8) as i32);
                    }
                    // Released when the handler goes out of scope, below.
                    held.push((slot, captures.clone()));
                    let env = self.b.ins().stack_addr(ptr_ty, slot, 0);
                    self.cg.handler_captures.insert(fid, captures);
                    let (op_p, op_l) = self.str_ptr_len(&opname);
                    let fref = self.cg.module.declare_func_in_func(fid, self.b.func);
                    let addr = self.b.ins().func_addr(ptr_ty, fref);
                    self.call_rt("grasm_handler_op", &[op_p, op_l, addr, env]);
                }
                let (_, mut terminated) = self.eval_block_inner(body)?;
                terminated |= self.is_filled();
                if !terminated {
                    self.call_rt("grasm_handler_pop", &[]);
                    for (slot, captures) in &held {
                        for (i, (_, cvt)) in captures.iter().enumerate() {
                            if !cvt.boxed() {
                                continue;
                            }
                            let bits =
                                self.b.ins().stack_load(types::I64, *slot, (i * 8) as i32);
                            self.call_rt("grasm_val_dec", &[bits]);
                        }
                    }
                }
                Ok(StmtOut::Value(None))
            }
        }
    }

    /// Installs a handler that arrived as a value, then runs the block.
    ///
    /// The pairs go onto the same runtime stack a literal handler uses, so an
    /// operation call inside the body resolves identically either way — it was
    /// always a lookup by effect and operation name, never a direct call.
    ///
    /// The handler is a live value for the whole block, which is what keeps the
    /// environment addresses the stack was handed good.
    fn with_handler_value(
        &mut self,
        ename: &str,
        hvar: Variable,
        body: &grasm_syntax::ast::Block,
        span: Span,
    ) -> CResult<StmtOut> {
        if !self.cg.effects.contains_key(ename) {
            return nope(span, format!("the effect `{ename}`"));
        }
        let h = self.b.use_var(hvar);
        self.call_rt("grasm_handler_install", &[h]);
        let (_, mut terminated) = self.eval_block_inner(body)?;
        terminated |= self.is_filled();
        if !terminated {
            self.call_rt("grasm_handler_pop", &[]);
        }
        Ok(StmtOut::Value(None))
    }

    /// The hidden local a handler literal's capture is copied into.
    ///
    /// Keyed by where the literal was written, so two handlers over the same
    /// binding keep their own values.
    fn snapshot_name(lit_span: u32, name: &str) -> String {
        format!("__hcap{lit_span}#{name}")
    }

    /// Copies every binding a handler literal's operations read into a hidden
    /// local, at the point the literal is written.
    ///
    /// A closure captures values, not bindings ([E-CLOSURE-CAPTURE]), and the
    /// environment was filled at the `with` instead — so a binding changed
    /// between the two was read at its new value.
    fn snapshot_handler_captures(&mut self, lit: &Expr) {
        let Expr::StructLit { fields, span, .. } = lit else {
            return;
        };
        let mut free = Vec::new();
        for (_, fexpr) in fields {
            let Expr::Closure { params, body, .. } = fexpr else {
                continue;
            };
            let mut names = Vec::new();
            collect_idents(body, &mut names);
            names.retain(|n| !params.iter().any(|p| p.name == *n));
            free.extend(names);
        }
        free.sort();
        free.dedup();
        for n in free {
            let Some((var, vt)) = self.lookup(&n) else {
                continue;
            };
            let v = self.b.use_var(var);
            // The copy holds a reference of its own, the same way the `with`
            // site's does: the original binding may be reassigned or dropped
            // before the handler is ever installed.
            self.inc_if_boxed(v, &vt);
            let copy = self.new_var(&vt);
            self.b.def_var(copy, v);
            self.bind(&Self::snapshot_name(span.start, &n), copy, vt);
        }
    }

    fn eval_for(
        &mut self,
        loop_var: &Ident,
        iter: &Expr,
        body: &grasm_syntax::ast::Block,
        _span: Span,
    ) -> CResult<StmtOut> {
        // Range iteration.
        if let Expr::Range {
            start,
            end,
            inclusive,
            ..
        } = iter
        {
            let (s, st) = self.eval_expr_val(start)?;
            let (e, et) = self.eval_expr_val(end)?;
            if st != VType::I64 || et != VType::I64 {
                return nope(iter.span(), "non-integer ranges");
            }
            let ivar = self.new_var(&VType::I64);
            self.b.def_var(ivar, s);
            let end_var = self.new_var(&VType::I64);
            self.b.def_var(end_var, e);
            // Same as the while loop: lists the body only indexes are pinned
            // for its whole extent so each access is a load, not a call.
            self.pin_loop_lists(body)?;

            let header = self.b.create_block();
            let body_blk = self.b.create_block();
            let step = self.b.create_block();
            let exit = self.b.create_block();
            self.b.ins().jump(header, &[]);
            self.filled = true;
            self.goto_block(header);
            let iv = self.b.use_var(ivar);
            let ev = self.b.use_var(end_var);
            let cc = if *inclusive {
                IntCC::SignedLessThanOrEqual
            } else {
                IntCC::SignedLessThan
            };
            let c = self.b.ins().icmp(cc, iv, ev);
            self.b.ins().brif(c, body_blk, &[], exit, &[]);
            self.filled = true;

            self.goto_block(body_blk);
            self.push_scope();
            self.bind(&loop_var.name, ivar, VType::I64);
            self.loops.push((step, exit, self.vars.len()));
            let inner = self.eval_block_inner(body);
            self.loops.pop();
            self.pinned.pop();
            let (_, mut terminated) = inner?;
            terminated |= self.is_filled();
            self.pop_scope_no_decs(); // loop var is scalar
            if !terminated {
                self.b.ins().jump(step, &[]);
                self.filled = true;
            }
            self.goto_block(step);
            let iv2 = self.b.use_var(ivar);
            // An inclusive range stops *before* incrementing past its end.
            // Incrementing first and re-testing `i <= end` spins forever when
            // the end is `i64::MAX`: the add wraps to `i64::MIN`, which is
            // still `<= end`. The exclusive form cannot reach that, because
            // its last value is one below the end.
            if *inclusive {
                let ev2 = self.b.use_var(end_var);
                let done = self.b.ins().icmp(IntCC::Equal, iv2, ev2);
                let bump = self.b.create_block();
                self.b.ins().brif(done, exit, &[], bump, &[]);
                self.filled = true;
                self.goto_block(bump);
            }
            let one = self.b.ins().iconst(types::I64, 1);
            let next = self.b.ins().iadd(iv2, one);
            self.b.def_var(ivar, next);
            self.b.ins().jump(header, &[]);
            self.filled = true;
            self.goto_block(exit);
            return Ok(StmtOut::Value(None));
        }

        // List iteration: index-based over an owned temp.
        //
        // The temp lives in a scope of its own, so every way out of the loop
        // releases it — including a `return` from the body, which jumps
        // straight past the loop's own teardown. It used to be a bare
        // temporary, so returning out of a `for` leaked the list being
        // iterated, which is the shape any search function has.
        self.push_scope();
        let (mut list_v, mut list_t) = self.eval_expr_val(iter)?;
        // A range *held as a value* — `let r = 0..n` then `for i in r`. The
        // literal `for i in 0..n` never gets here; it is compiled above into a
        // counted loop, which is why it costs nothing and still does. Walking
        // the elements this one names is the same thing the interpreter does,
        // so the two agree on what the loop sees.
        if list_t == VType::Range {
            let elems = self.call_rt("grasm_range_to_list", &[list_v]).unwrap();
            self.call_rt("grasm_val_dec", &[list_v]);
            list_v = elems;
            list_t = VType::List(Box::new(VType::I64));
        }
        let VType::List(elem_t) = list_t.clone() else {
            return nope(iter.span(), "for over non-range, non-list iterables");
        };
        let list_var = self.new_var(&list_t);
        self.b.def_var(list_var, list_v);
        self.bind(" for", list_var, list_t.clone());
        let ivar = self.new_var(&VType::I64);
        let zero = self.b.ins().iconst(types::I64, 0);
        self.b.def_var(ivar, zero);

        let header = self.b.create_block();
        let body_blk = self.b.create_block();
        let step = self.b.create_block();
        let exit = self.b.create_block();
        self.b.ins().jump(header, &[]);
        self.filled = true;
        self.goto_block(header);
        let lv = self.b.use_var(list_var);
        let len = self.call_rt("grasm_list_len", &[lv]).unwrap();
        let iv = self.b.use_var(ivar);
        let c = self.b.ins().icmp(IntCC::SignedLessThan, iv, len);
        self.b.ins().brif(c, body_blk, &[], exit, &[]);
        self.filled = true;

        self.goto_block(body_blk);
        let lv2 = self.b.use_var(list_var);
        let iv2 = self.b.use_var(ivar);
        let (elem_val, elem_vt) = self.list_get_elem(lv2, iv2, &elem_t)?;
        let evar = self.new_var(&elem_vt);
        self.b.def_var(evar, elem_val);
        // The depth is recorded *before* the loop variable's scope, so
        // `continue` releases it: the element was read with a +1 and the next
        // iteration binds a fresh one. Recording it after meant
        // `emit_decs_above` skipped that scope, and every `continue` in a loop
        // over boxed elements leaked the element it skipped.
        self.loops.push((step, exit, self.vars.len()));
        self.push_scope();
        self.bind(&loop_var.name, evar, elem_vt.clone());
        let (_, mut terminated) = self.eval_block_inner(body)?;
        self.loops.pop();
        terminated |= self.is_filled();
        if terminated {
            self.pop_scope_no_decs();
        } else {
            self.pop_scope_with_decs();
        }
        if !terminated {
            self.b.ins().jump(step, &[]);
            self.filled = true;
        }
        self.goto_block(step);
        let iv3 = self.b.use_var(ivar);
        let one = self.b.ins().iconst(types::I64, 1);
        let next = self.b.ins().iadd(iv3, one);
        self.b.def_var(ivar, next);
        self.b.ins().jump(header, &[]);
        self.filled = true;
        self.goto_block(exit);
        // Released by popping the scope it was bound into, so a `return` out
        // of the body — which calls `emit_scope_decs_all` and never arrives
        // here — releases it too.
        self.pop_scope_with_decs();
        Ok(StmtOut::Value(None))
    }

    /// Unbox a list element into its unboxed form (scalars) or pass through
    /// (boxed types keep the +1 from list_get).
    fn unbox(&mut self, boxed: Value, want: &VType) -> CResult<(Value, VType)> {
        match want {
            VType::I64 => {
                let v = self.call_rt("grasm_unbox_i64", &[boxed]).unwrap();
                self.call_rt("grasm_val_dec", &[boxed]);
                Ok((v, VType::I64))
            }
            // The same word, kept at its own type so a later field read still
            // knows to go through the slab.
            VType::Ref(inner) => {
                let v = self.call_rt("grasm_unbox_i64", &[boxed]).unwrap();
                self.call_rt("grasm_val_dec", &[boxed]);
                Ok((v, VType::Ref(inner.clone())))
            }
            VType::F64 => {
                let v = self.call_rt("grasm_unbox_f64", &[boxed]).unwrap();
                self.call_rt("grasm_val_dec", &[boxed]);
                Ok((v, VType::F64))
            }
            VType::Bool => {
                let v = self.call_rt("grasm_unbox_bool", &[boxed]).unwrap();
                self.call_rt("grasm_val_dec", &[boxed]);
                Ok((v, VType::Bool))
            }
            VType::Unit => {
                // Unit has no payload; release the placeholder box.
                self.call_rt("grasm_val_dec", &[boxed]);
                let dummy = self.b.ins().iconst(types::I8, 0);
                Ok((dummy, VType::Unit))
            }
            // All other boxed kinds pass through, keeping the +1.
            other => Ok((boxed, other.clone())),
        }
    }

    fn box_value(&mut self, v: Value, vt: &VType) -> Value {
        match vt {
            // Unit has no runtime payload; box a placeholder so `Ok(())` and
            // unit-typed containers still hold a valid pointer.
            // Boxed as itself, not as the integer 0: a container holding
            // units printed `[0, 0]` where the interpreter printed
            // `[(), ()]`, which is a wrong answer rather than a refusal.
            VType::Unit => self.call_rt("grasm_box_unit", &[]).unwrap(),
            // A reference is a packed word, not a pointer. Falling through to
            // "already boxed" below stored the packed value *as* a pointer,
            // and reading it back dereferenced an integer. Boxed as an
            // integer instead, it printed as the packed word — the slab slot
            // and generation, as a number nobody can read.
            VType::Ref(_) => self.call_rt("grasm_box_ref", &[v]).unwrap(),
            VType::I64 => self.call_rt("grasm_box_i64", &[v]).unwrap(),
            VType::F64 => self.call_rt("grasm_box_f64", &[v]).unwrap(),
            VType::Bool => self.call_rt("grasm_box_bool", &[v]).unwrap(),
            // Already boxed: ownership transfers into the container.
            _ => v,
        }
    }

    // ------------------------------------------------------------------
    // Expressions
    // ------------------------------------------------------------------

    fn eval_expr_val(&mut self, e: &Expr) -> CResult<(Value, VType)> {
        match self.eval_expr(e)? {
            Some(v) => Ok(v),
            // An expression that produces nothing *is* the unit value —
            // `let u = noop()` binds it, and it renders as `()`. Refusing
            // here made a shape the checker accepts one the backend would not
            // take, for a type that has exactly one value to hand over.
            None => Ok((self.zero_of(&VType::Unit), VType::Unit)),
        }
    }

    /// Like `eval_expr_val`, but an expected type supplies the element type
    /// for empty collection literals (`let xs: [i64] = []`).
    /// The function that implements one handler operation.
    ///
    /// A lifted closure is already a function. A named one needs the wrapper
    /// that gives it the `(env, params…)` shape, and this is where it is built
    /// — during body compilation, like every other wrapper.
    fn handler_op_func(&mut self, code: &HandlerOpCode, span: Span) -> CResult<FuncId> {
        match code {
            HandlerOpCode::Lifted(id) => Ok(*id),
            HandlerOpCode::Named(key) => self.cg.handler_op_wrapper(key, span),
        }
    }

    /// Builds a handler value: one code pointer and one heap environment per
    /// operation.
    ///
    /// The operations were lifted where the literal was written. What is built
    /// here is the value that carries them, and it is reference-counted, so it
    /// may be returned, stored or put in a list — the environment outlives this
    /// frame because nothing about it is in this frame.
    fn handler_value(&mut self, e: &Expr, eff: &str) -> CResult<(Value, VType)> {
        // Already a handler value — a parameter passed straight on, or a name
        // bound to one.
        if let Expr::Ident(id) = e {
            if let Some((var, VType::Handler(got))) = self.lookup(&id.name) {
                if got == eff {
                    let v = self.b.use_var(var);
                    self.call_rt("grasm_val_inc", &[v]);
                    return Ok((v, VType::Handler(eff.to_string())));
                }
            }
        }
        let Some(lit) = self.cg.handler_literal(e) else {
            return nope(
                e.span(),
                format!("a `{eff}Handler` that is not a handler literal or a name bound to one"),
            );
        };
        let Expr::StructLit { fields, span: lit_span, .. } = &lit else {
            return nope(e.span(), format!("a `{eff}Handler` value"));
        };
        self.cg.lift_handler_literal(&lit)?;
        let Some(site) = self.cg.handler_sites.get(&lit_span.start).cloned() else {
            return nope(e.span(), format!("a `{eff}Handler` that was not lifted"));
        };
        let ptr_ty = self.cg.module.target_config().pointer_type();
        let (ep, el) = self.str_ptr_len(eff);
        let handler = self.call_rt("grasm_handler_new", &[ep, el]).unwrap();
        // The literal's field order, which is the order the interpreter prints
        // a handler in. Iterating the effect's declaration order instead would
        // agree on behaviour and disagree on `print("{h}")`.
        for (fname, _) in fields.iter() {
            let Some((_, opcode, free)) = site.iter().find(|(o, _, _)| *o == fname.name).cloned()
            else {
                return nope(fname.span, format!("the operation `{}`", fname.name));
            };
            let fid = self.handler_op_func(&opcode, *lit_span)?;
            let captures: Vec<(String, VType)> = free
                .iter()
                .filter_map(|n| {
                    self.lookup(&Self::snapshot_name(lit_span.start, n))
                        .or_else(|| self.lookup(n))
                        .map(|(_, vt)| (n.clone(), vt))
                })
                .collect();
            let n = self.b.ins().iconst(types::I64, captures.len() as i64);
            let env = self.call_rt("grasm_env_new", &[n]).unwrap();
            for (i, (cname, cvt)) in captures.iter().enumerate() {
                let Some((var, _)) = self
                    .lookup(&Self::snapshot_name(lit_span.start, cname))
                    .or_else(|| self.lookup(cname))
                else {
                    continue;
                };
                let v = self.b.use_var(var);
                let bits = self.value_to_bits(v, cvt);
                // A capture is a value, not a binding ([E-CLOSURE-CAPTURE]), so
                // a boxed one is held for as long as the environment is. The
                // environment releases it, which is the difference from the
                // frame-scoped path: there, the `with` released them.
                if cvt.boxed() {
                    self.call_rt("grasm_val_inc", &[v]);
                }
                let idx = self.b.ins().iconst(types::I64, i as i64);
                let flag = self.b.ins().iconst(types::I8, i64::from(cvt.boxed()));
                self.call_rt("grasm_env_set", &[env, idx, bits, flag]);
            }
            self.cg.handler_captures.insert(fid, captures);
            let (op_p, op_l) = self.str_ptr_len(&fname.name);
            let fref = self.cg.module.declare_func_in_func(fid, self.b.func);
            let addr = self.b.ins().func_addr(ptr_ty, fref);
            self.call_rt("grasm_handler_add", &[handler, op_p, op_l, addr, env]);
        }
        Ok((handler, VType::Handler(eff.to_string())))
    }

    fn eval_expr_expecting(
        &mut self,
        e: &Expr,
        expected: Option<&VType>,
    ) -> CResult<(Value, VType)> {
        match self.eval_expr_expecting_opt(e, expected)? {
            Some(v) => Ok(v),
            // As in `eval_expr_val`: an expression that produces nothing is
            // the unit value, and unit has exactly one.
            None => Ok((self.zero_of(&VType::Unit), VType::Unit)),
        }
    }

    /// As `eval_expr_expecting`, but `None` for an expression that produces no
    /// value — because it is unit, or because control left before it could.
    fn eval_expr_expecting_opt(
        &mut self,
        e: &Expr,
        expected: Option<&VType>,
    ) -> CResult<Option<(Value, VType)>> {
        // A variant of a generic sum with the member already decided by the
        // type in view: `let r: Res[i64, str] = Good(1)`. The arguments alone
        // cannot say — `Good` mentions `T` and never `E` — and an annotation
        // is the only thing that can, which is why one is required there.
        if let (Expr::Call { callee, args, .. }, Some(VType::Sum(key))) = (e, expected) {
            if let Expr::Ident(id) = &**callee {
                if self.lookup(&id.name).is_none()
                    && self.cg.sums.get(key).is_some_and(|vs| {
                        vs.iter().any(|(n, ts)| n == &id.name && ts.len() == args.len())
                    })
                {
                    let key = key.clone();
                    let args = args.clone();
                    let v = self.build_variant_call(&key, &id.name, &args)?;
                    return Ok(Some((v, VType::Sum(key))));
                }
            }
        }
        // A nullary variant of a generic sum, which member decided by the type
        // in view: `let n: Opt[i64] = Nothing`. The by-name index cannot answer
        // it — one name, one entry, many instantiations — so the expected type
        // is the only thing that can.
        if let (Expr::Ident(id), Some(VType::Sum(key))) = (e, expected) {
            if self.lookup(&id.name).is_none()
                && self
                    .cg
                    .sums
                    .get(key)
                    .is_some_and(|vs| vs.iter().any(|(n, ts)| n == &id.name && ts.is_empty()))
            {
                let key = key.clone();
                let v = self.make_variant(&key, &id.name, &[])?;
                return Ok(Some((v, VType::Sum(key))));
            }
        }
        // A handler meeting a parameter of handler type. The block of
        // `(code, env)` pairs is built here, in *this* frame, and the callee
        // is handed a pointer to it — which is why a handler may be passed
        // down and not returned or stored: this frame outlives the call and
        // nothing else.
        if let Some(VType::Handler(eff)) = expected {
            let eff = eff.clone();
            return self.handler_value(e, &eff).map(Some);
        }
        // A closure literal in argument position gets its parameter types from
        // the parameter it is being passed to. Before function types existed
        // there was nowhere for them to come from, and the backend said so.
        if let (Expr::Closure { params, body, span }, Some(VType::Fn(want_ps, want_ret))) =
            (e, expected)
        {
            if params.len() == want_ps.len() {
                // The declared *return* type too. Dropped, a closure literal
                // meeting `fn(i64) -> str ! E` in argument, struct-field or
                // list-element position was lifted as if it returned the ok
                // half alone: the caller's `??` then read a bare value as a
                // fallible one — a wrong answer for a boxed half, SIGSEGV for
                // an `i64`, SIGTRAP for `()`.
                return self
                    .closure_value_with_ret(
                        params,
                        body,
                        want_ps,
                        Some((**want_ret).clone()),
                        *span,
                    )
                    .map(Some);
            }
        }
        if let (Expr::List(items, _), Some(VType::List(elem_t))) = (e, expected) {
            if items.is_empty() {
                let v = self.new_list_of(elem_t);
                return Ok(Some((v, VType::List(elem_t.clone()))));
            }
            // A non-empty list whose element type is known passes it down.
            // A closure among the elements has nowhere else to get its
            // parameter types from — `[dbl, |n| n + 100]` annotated as
            // `[fn(i64) -> i64]` names them exactly once, at the annotation.
            if items.iter().any(needs_element_hint) {
                let mut list = self.new_list_of(elem_t);
                for item in items {
                    let (mut v, vt) = self.eval_expr_expecting(item, Some(elem_t))?;
                    let vt = if **elem_t == VType::F64 && vt == VType::I64 {
                        v = self.b.ins().fcvt_from_sint(types::F64, v);
                        VType::F64
                    } else {
                        vt
                    };
                    list = self.list_push_elem(list, v, &vt);
                }
                return Ok(Some((list, VType::List(elem_t.clone()))));
            }
        }
        if let (Expr::Map(entries, _), Some(VType::Map(k, v))) = (e, expected) {
            if entries.is_empty() {
                let m = self.call_rt("grasm_map_new", &[]).unwrap();
                return Ok(Some((m, VType::Map(k.clone(), v.clone()))));
            }
            // A value that cannot say its own type — a closure, an empty
            // container — takes it from the annotation, exactly as a list's
            // elements do. `let m: Map[str, fn(i64) -> i64] = {"inc": |n| n +
            // 1}` was refused for having no type to read the parameters from,
            // with the type written on the line above.
            if entries.iter().any(|(_, ve)| needs_element_hint(ve)) {
                let (k, v) = (k.clone(), v.clone());
                let mut m = self.call_rt("grasm_map_new", &[]).unwrap();
                for (ke, ve) in entries {
                    let (kv, kvt) = self.eval_expr_expecting(ke, Some(&k))?;
                    let kb = self.box_value(kv, &kvt);
                    let (vv, vvt) = self.eval_expr_expecting(ve, Some(&v))?;
                    let vb = self.box_value(vv, &vvt);
                    m = self.call_rt("grasm_map_set", &[m, kb, vb]).unwrap();
                }
                return Ok(Some((m, VType::Map(k, v))));
            }
        }
        if let (Expr::Tuple(items, _), Some(VType::Tuple(ts))) = (e, expected) {
            if !items.is_empty()
                && items.len() == ts.len()
                && items.iter().any(needs_element_hint)
            {
                let ts = ts.clone();
                let mut list = self.call_rt("grasm_list_new", &[]).unwrap();
                let mut got = Vec::with_capacity(items.len());
                for (item, want) in items.iter().zip(&ts) {
                    let (v, vt) = self.eval_expr_expecting(item, Some(want))?;
                    let bx = self.box_value(v, &vt);
                    list = self.call_rt("grasm_list_push", &[list, bx]).unwrap();
                    got.push(vt);
                }
                let sealed = self.call_rt("grasm_tuple_seal", &[list]).unwrap();
                return Ok(Some((sealed, VType::Tuple(got))));
            }
        }
        // `Some(x)` / `Ok(x)` / `Err(x)`: the payload's expected type is the
        // one written on the container.
        if let Expr::Call { callee, args, .. } = e {
            if let Expr::Ident(id) = &**callee {
                let payload = match (id.name.as_str(), expected) {
                    ("Some", Some(VType::Option(t))) => Some((*t).clone()),
                    ("Ok", Some(VType::Fallible(t, _))) => Some((*t).clone()),
                    ("Err", Some(VType::Fallible(_, t))) => Some((*t).clone()),
                    _ => None,
                };
                if let Some(want) = payload {
                    if args.len() == 1
                        && needs_element_hint(&args[0])
                        && self.lookup(&id.name).is_none()
                    {
                        let (pv, pvt) = self.eval_expr_expecting(&args[0], Some(&want))?;
                        let boxed = self.box_value(pv, &pvt);
                        let out = match id.name.as_str() {
                            "Some" => (
                                self.make_variant("Option", "Some", &[boxed])?,
                                VType::Option(Box::new(pvt)),
                            ),
                            "Ok" => (
                                self.call_rt("grasm_ok_new", &[boxed]).unwrap(),
                                VType::Fallible(Box::new(pvt), Box::new(VType::Any)),
                            ),
                            _ => (
                                self.call_rt("grasm_err_new", &[boxed]).unwrap(),
                                VType::Fallible(Box::new(VType::Any), Box::new(pvt)),
                            ),
                        };
                        return Ok(Some(out));
                    }
                }
            }
        }
        if let (Expr::Call { callee, args, .. }, Some(VType::Map(k, v))) = (e, expected) {
            if args.is_empty() {
                if let Expr::Ident(id) = &**callee {
                    if id.name == "map" && self.lookup("map").is_none() {
                        let m = self.call_rt("grasm_map_new", &[]).unwrap();
                        return Ok(Some((m, VType::Map(k.clone(), v.clone()))));
                    }
                }
            }
        }
        self.eval_expr(e)
    }

    fn eval_expr(&mut self, e: &Expr) -> CResult<Option<(Value, VType)>> {
        match e {
            // A literal the checker placed in an `f64` position *is* a float
            // ([T-INT-TO-FLOAT]), everywhere and not only where this backend
            // remembered to convert it.
            Expr::Int(v, sp) if self.cg.float_literals.contains(&sp.start) => Ok(Some((
                self.b.ins().f64const(*v as f64),
                VType::F64,
            ))),
            Expr::Int(v, _) => Ok(Some((self.b.ins().iconst(types::I64, *v), VType::I64))),
            Expr::Map(entries, _) => {
                let mut m = self.call_rt("grasm_map_new", &[]).unwrap();
                let (mut kt, mut vt) = (VType::Any, VType::Any);
                for (ke, ve) in entries {
                    let (kv, kvt) = self.eval_expr_val(ke)?;
                    let kb = self.box_value(kv, &kvt);
                    let (vv, vvt) = self.eval_expr_val(ve)?;
                    let vb = self.box_value(vv, &vvt);
                    m = self.call_rt("grasm_map_set", &[m, kb, vb]).unwrap();
                    kt = kvt;
                    vt = vvt;
                }
                Ok(Some((m, VType::Map(Box::new(kt), Box::new(vt)))))
            }
            Expr::Float(v, _) => Ok(Some((self.b.ins().f64const(*v), VType::F64))),
            Expr::Bool(v, _) => Ok(Some((
                self.b.ins().iconst(types::I8, i64::from(*v)),
                VType::Bool,
            ))),
            // Built once before the enclosing loop, if there was one.
            Expr::Ident(id)
                if self.lookup(&id.name).is_none()
                    && self.lookup(&format!(" const {}", id.name)).is_some() =>
            {
                let (var, vt) = self
                    .lookup(&format!(" const {}", id.name))
                    .expect("just checked");
                let v = self.b.use_var(var);
                if vt.boxed() {
                    self.call_rt("grasm_val_inc", &[v]);
                }
                Ok(Some((v, vt)))
            }
            Expr::Ident(id) if self.lookup(&id.name).is_none() && self.const_of(&id.name).is_some() => {
                // A module-level constant. The initializer is effect-free, so
                // compiling it here is the whole of what the name means — no
                // global, and the same rule the interpreter follows.
                let init = self.const_of(&id.name).expect("just checked");
                self.eval_expr(&init)
            }
            Expr::Ident(id) => match self.lookup(&id.name) {
                Some((var, vt)) => {
                    let v = self.b.use_var(var);
                    if vt.boxed() {
                        self.call_rt("grasm_val_inc", &[v]);
                    }
                    Ok(Some((v, vt)))
                }
                None if self.lookup_fn(&id.name).is_some() => {
                    let key = format!("{}::{}", self.cg.current_module, id.name);
                    let (v, vt) = self.make_named_fn_value(&key, id.span)?;
                    Ok(Some((v, vt)))
                }
                None => {
                    // Nullary variant as a value (None, Dot, Red, ...).
                    if let Some((parent, 0)) = self.cg.variant_index.get(&id.name).cloned() {
                        let v = self.make_variant(&parent, &id.name, &[])?;
                        let vt = self.sum_vtype(&parent);
                        return Ok(Some((v, vt)));
                    }
                    if id.name == "None" {
                        let v = self.make_variant("Option", "None", &[])?;
                        return Ok(Some((v, VType::Option(Box::new(VType::Any)))));
                    }
                    // A handler bound to a name, used as a value rather than
                    // installed. It builds the value now — the literal is
                    // recorded, and a handler value is a value like any other.
                    if let Some(lit) = self.cg.handler_lits.get(&id.name).cloned() {
                        if let Expr::StructLit { name, .. } = &lit {
                            if let Some(eff) = name.segments[0].name.strip_suffix("Handler") {
                                if self.cg.effects.contains_key(eff) {
                                    let eff = eff.to_string();
                                    return self.handler_value(&lit, &eff).map(Some);
                                }
                            }
                        }
                    }
                    nope(id.span, format!("the name `{}` here", id.name))
                }
            },
            Expr::Closure { params, body, span } => {
                let (v, vt) = self.make_closure_value(params, body, None, *span)?;
                Ok(Some((v, vt)))
            }
            Expr::Str(pieces, _) => Ok(Some(self.build_string(pieces)?)),
            Expr::List(items, span) => {
                let elem_t = self.list_elem_type(items, *span)?;
                let mut list = self.new_list_of(&elem_t);
                for item in items {
                    // The element type reaches every element, not only the
                    // annotated case: in `[double, |n| n + 1]` the named
                    // function is what says what the closure's parameter is.
                    let (mut v, vt) = self.eval_expr_expecting(item, Some(&elem_t))?;
                    // Int-literal → f64 adaptation inside float lists.
                    let vt = if elem_t == VType::F64 && vt == VType::I64 {
                        v = self.b.ins().fcvt_from_sint(types::F64, v);
                        VType::F64
                    } else {
                        vt
                    };
                    list = self.list_push_elem(list, v, &vt);
                }
                Ok(Some((list, VType::List(Box::new(elem_t)))))
            }
            Expr::Index { recv, index, span } => {
                // A pinned list is read without ever producing the list value:
                // `pin` already took the borrow for the whole loop, so
                // evaluating the receiver here would be an inc/dec round trip
                // per element for a value the load does not use.
                if let Expr::Ident(id) = &**recv {
                    if let Some((data, len, pt)) = self.pinned_list(&id.name) {
                        let (iv, it) = self.eval_expr_val(index)?;
                        if it != VType::I64 {
                            return nope(index.span(), "non-integer index");
                        }
                        let addr = self.pinned_address(data, len, iv, stride_of(&pt));
                        let flags = cranelift_codegen::ir::MemFlags::trusted();
                        let v = self.b.ins().load(pt.cl(), flags, addr, 0);
                        return Ok(Some((v, pt)));
                    }
                }
                // As for a field read: indexing a plain local **borrows** it.
                // The variable holds a reference for the whole of the read, so
                // the increment and the release around it cancel exactly. Any
                // other receiver produced a temporary this expression owns.
                let borrowed = match &**recv {
                    Expr::Ident(id) => match self.lookup(&id.name) {
                        Some((var, vt @ VType::List(_))) => Some((var, vt)),
                        _ => None,
                    },
                    _ => None,
                };
                let (rv, rt, owned) = match borrowed {
                    Some((var, vt)) => (self.b.use_var(var), vt, false),
                    None => {
                        let (v, t) = self.eval_expr_val(recv)?;
                        (v, t, true)
                    }
                };
                let VType::List(elem_t) = rt else {
                    return nope(*span, "indexing non-lists");
                };
                let (iv, it) = self.eval_expr_val(index)?;
                if it != VType::I64 {
                    return nope(index.span(), "non-integer index");
                }
                let (v, vt) = self.list_get_elem(rv, iv, &elem_t)?;
                if owned {
                    self.call_rt("grasm_val_dec", &[rv]);
                }
                Ok(Some((v, vt)))
            }
            Expr::MethodCall {
                recv, name, args, span,
            } => self.eval_method(recv, name, args, *span),
            Expr::Unary { op, expr, span } => {
                let (v, vt) = self.eval_expr_val(expr)?;
                match (op, &vt) {
                    (UnaryOp::Neg, VType::I64) => Ok(Some((self.b.ins().ineg(v), VType::I64))),
                    (UnaryOp::Neg, VType::F64) => Ok(Some((self.b.ins().fneg(v), VType::F64))),
                    (UnaryOp::Not, VType::Bool) => {
                        let one = self.b.ins().iconst(types::I8, 1);
                        Ok(Some((self.b.ins().bxor(v, one), VType::Bool)))
                    }
                    _ => nope(*span, "this unary operation"),
                }
            }
            Expr::Binary { op, lhs, rhs, span } => {
                if matches!(op, BinaryOp::And | BinaryOp::Or) {
                    return self.short_circuit(*op, lhs, rhs);
                }
                if *op == BinaryOp::Coalesce {
                    return self.eval_coalesce(lhs, rhs, *span).map(Some);
                }
                // Whichever side is not an empty literal fixes the type, so
                // `xs + []` and `[] + xs` both have somewhere to get one from.
                // Taking the empty one second cannot be observed: a literal
                // has no effects, so [E-ORDER-LTR] still holds.
                // The first operand is held in a scope while the second is
                // evaluated, so a second operand that diverges — `a + "{try
                // f()}"` — releases the first instead of stranding it. `try`
                // unwinds through `emit_scope_decs_all`, which sees scope
                // bindings and not loose temporaries.
                self.push_scope();
                let ((lv, lt), (rv, rt)) = if needs_type_hint(lhs) {
                    let r = self.eval_expr_val(rhs)?;
                    let r = self.hold_operand(r);
                    let l = self.eval_expr_expecting(lhs, Some(&r.1))?;
                    (l, r)
                } else {
                    let l = self.eval_expr_val(lhs)?;
                    let l = self.hold_operand(l);
                    let r = self.eval_expr_expecting(rhs, Some(&l.1))?;
                    (l, r)
                };
                self.pop_scope_no_decs();
                Ok(Some(self.binop_values(*op, lv, lt, rv, rt, *span)?))
            }
            Expr::If {
                cond,
                then_block,
                else_branch,
                ..
            } => self.eval_if(cond, then_block, else_branch.as_deref()),
            Expr::Call { callee, args, span } => self.eval_call(callee, args, *span),
            Expr::Block(b) | Expr::Unsafe(b) => {
                let (last, terminated) = self.eval_block_inner(b)?;
                if terminated {
                    Ok(None)
                } else {
                    Ok(last)
                }
            }
            // Tier 1. Allocations made inside are arena-owned: no reference
            // counting while the region is open, and the whole set is freed in
            // one sweep at the closing brace. Sound because the checker proves
            // nothing allocated inside is reachable afterwards, and the result
            // is a scalar, which is not an allocation ([T-REGION]).
            Expr::Region(b) => {
                if REGION_SWEEP {
                    self.call_rt("grasm_region_push", &[]);
                }
                let (last, terminated) = self.eval_block_inner(b)?;
                if REGION_SWEEP {
                    self.call_rt("grasm_region_pop", &[]);
                }
                if terminated {
                    Ok(None)
                } else {
                    Ok(last)
                }
            }
            Expr::Tuple(items, _) => {
                // `()` is unit, not a zero-length tuple — and unit is a
                // value, not an allocation, so nothing is built for it.
                if items.is_empty() {
                    let nothing = self.zero_of(&VType::Unit);
                    return Ok(Some((nothing, VType::Unit)));
                }
                let mut list = self.call_rt("grasm_list_new", &[]).unwrap();
                let mut ts = Vec::with_capacity(items.len());
                for item in items {
                    let (v, vt) = self.eval_expr_val(item)?;
                    let bx = self.box_value(v, &vt);
                    list = self.call_rt("grasm_list_push", &[list, bx]).unwrap();
                    ts.push(vt);
                }
                // A tuple is a list until it is sealed. The seal exists only
                // so it renders with parentheses, which [L-DISPLAY] makes
                // part of what the program is judged on.
                let sealed = self.call_rt("grasm_tuple_seal", &[list]).unwrap();
                Ok(Some((sealed, VType::Tuple(ts))))
            }
            Expr::StructLit {
                name,
                base,
                fields,
                span,
            } => self
                .eval_struct_lit(name, base.as_deref(), fields, *span)
                .map(Some),
            Expr::Field { recv, name, span } => self.eval_field(recv, name, *span),
            Expr::Match {
                scrutinee, arms, span,
            } => self.eval_match(scrutinee, arms, *span),
            Expr::Try { expr, span } => self.eval_try(expr, *span).map(Some),
            Expr::Range {
                start,
                end,
                inclusive,
                span,
            } => self
                .eval_range_value(start, end, *inclusive, *span)
                .map(Some),
            // No catch-all. Every expression form the parser can produce has
            // an arm above, and that is worth keeping: a new one added later
            // has to be handled here rather than falling into a refusal that
            // says "this construct" and names nothing.
        }
    }

    /// The type the checker recorded for an empty literal at this span.
    fn empty_vtype(&mut self, span: Span) -> Option<VType> {
        let text = self.cg.empty_types.get(&span.start)?.clone();
        let te = grasm_syntax::parse_type_src(&text)?;
        self.cg.vtype_of(&te).ok()
    }

    fn list_elem_type(&mut self, items: &[Expr], span: Span) -> CResult<VType> {
        // The first element that can say its own type. A closure cannot, so
        // `[|n| n + 1, double]` has to read the answer off `double`.
        let first = items
            .iter()
            .find(|i| !needs_element_hint(i))
            .or_else(|| items.first());
        let Some(first) = first else {
            // The checker resolved this one against the whole body, which is
            // the only place the answer exists: an empty literal has nothing
            // to read a type off.
            if let Some(vt) = self.empty_vtype(span) {
                return Ok(match vt {
                    VType::List(e) => *e,
                    other => other,
                });
            }
            // The advice used to say "annotate via a function param", which
            // was both narrower than the truth and, for `[[]]`, useless: the
            // annotation was already there and still refused. Any annotation
            // that reaches the literal works — a `let`, a parameter, a field.
            return nope(
                span,
                "an empty list literal with no type in view (annotate the binding, as in `let xs: [i64] = []`)",
            );
        };
        self.probe_type(first)
    }

    /// Interpolated string as a value: fold pieces with concat.
    fn build_string(&mut self, pieces: &[StrPiece]) -> CResult<(Value, VType)> {
        let mut acc: Option<Value> = None;
        for piece in pieces {
            let part: Value = match piece {
                StrPiece::Text(t) => self.str_literal(t),
                StrPiece::Expr(inner) => {
                    let (v, vt) = self.eval_expr_val(inner)?;
                    match vt {
                        VType::I64 => self.call_rt("grasm_str_from_i64", &[v]).unwrap(),
                        VType::F64 => self.call_rt("grasm_str_from_f64", &[v]).unwrap(),
                        VType::Bool => self.call_rt("grasm_str_from_bool", &[v]).unwrap(),
                        VType::Str => v,
                        ref vt if vt.boxed() => {
                            let sv = self.call_rt("grasm_str_from_val", &[v]).unwrap();
                            self.call_rt("grasm_val_dec", &[v]);
                            sv
                        }
                        // A unit carries nothing, so there is nothing to
                        // read: the text is a constant. It reached here
                        // unboxed — the boxed arm above runs first, which is
                        // why the same expression under an annotation always
                        // compiled and only the inferred one was refused.
                        VType::Unit => self.str_literal("()"),
                        _ => unreachable!(),
                    }
                }
            };
            acc = Some(match acc {
                None => part,
                Some(prev) => {
                    // `prev` is this loop's own intermediate and nothing else
                    // holds it, so it is handed over rather than copied:
                    // `"{a}{b}{c}"` builds one string instead of one per
                    // piece.
                    let joined = self.call_rt("grasm_str_append", &[prev, part]).unwrap();
                    self.call_rt("grasm_val_dec", &[part]);
                    joined
                }
            });
        }
        let v = match acc {
            Some(v) => v,
            None => self.str_literal(""),
        };
        Ok((v, VType::Str))
    }

    /// `a..b` as the expression `{ var §: [i64] = []; for § in a..b { §.push(§) }; § }`.
    ///
    /// Synthesised rather than emitted by hand so that the loop, the list and
    /// the push are the ones this backend already compiles and the differential
    /// gate already covers. The names carry the receiver's span, so a
    /// diagnostic inside points at the range the reader wrote.
    fn range_to_list_expr(&self, range: &Expr, span: Span) -> Expr {
        let acc = Ident {
            name: "__range_list".to_string(),
            span,
        };
        let item = Ident {
            name: "__range_item".to_string(),
            span,
        };
        let elem_ty = grasm_syntax::ast::TypeExpr::List(
            Box::new(grasm_syntax::ast::TypeExpr::Name {
                path: grasm_syntax::ast::Path {
                    segments: vec![Ident {
                        name: "i64".to_string(),
                        span,
                    }],
                    span,
                },
                args: Vec::new(),
                span,
            }),
            span,
        );
        Expr::Block(grasm_syntax::ast::Block {
            stmts: vec![
                Stmt::Let {
                    mutable: true,
                    name: acc.clone(),
                    ty: Some(elem_ty),
                    init: Expr::List(Vec::new(), span),
                    span,
                },
                Stmt::For {
                    var: item.clone(),
                    iter: range.clone(),
                    body: grasm_syntax::ast::Block {
                        stmts: vec![Stmt::Expr(Expr::MethodCall {
                            recv: Box::new(Expr::Ident(acc.clone())),
                            name: Ident {
                                name: "push".to_string(),
                                span,
                            },
                            args: vec![Expr::Ident(item)],
                            span,
                        })],
                        span,
                    },
                    span,
                },
                Stmt::Expr(Expr::Ident(acc)),
            ],
            span,
        })
    }

    /// `(a..b).map(f)` and `(a..b).filter(p)` as one loop, with no intermediate
    /// list of the range's own elements.
    ///
    /// Only for a closure written at the call site: the parameter is bound by
    /// the `for`, so the closure body becomes the loop body and there is no
    /// call at all. A named function or a stored closure takes the ordinary
    /// path, where the call has to happen anyway.
    ///
    /// Returns `None` whenever the shape does not match, so nothing here can
    /// change what a program means — only how long it takes.
    /// `m.set(k, (m.get(k) ?? D) + N)` — the counting idiom — as
    /// `(delta, default)`, when fusing it into one lookup cannot change what
    /// the program does.
    ///
    /// Deliberately narrow. It fires only when the map and the key are spelled
    /// the same on both sides, the key is a *simple* expression, and both the
    /// default and the addend are integer literals. Every one of those is
    /// there so that evaluating the key once instead of twice, and evaluating
    /// the default before the lookup instead of after it, cannot be observed:
    /// a key expression that called something would run once instead of twice,
    /// and a non-literal default would lose `??`'s laziness.
    fn counter_bump(map_name: &str, key: &Expr, value: &Expr) -> Option<(i64, i64)> {
        // A key whose evaluation nothing can notice happening once.
        fn simple(e: &Expr) -> bool {
            match e {
                Expr::Ident(_) | Expr::Int(..) | Expr::Str(..) => true,
                Expr::Index { recv, index, .. } => simple(recv) && simple(index),
                Expr::Field { recv, .. } => simple(recv),
                Expr::Binary { lhs, rhs, .. } => simple(lhs) && simple(rhs),
                _ => false,
            }
        }
        fn same(a: &Expr, b: &Expr) -> bool {
            match (a, b) {
                (Expr::Ident(x), Expr::Ident(y)) => x.name == y.name,
                (Expr::Int(x, ..), Expr::Int(y, ..)) => x == y,
                (Expr::Str(..), Expr::Str(..)) => format!("{a:?}") == format!("{b:?}"),
                (
                    Expr::Index { recv: r1, index: i1, .. },
                    Expr::Index { recv: r2, index: i2, .. },
                ) => same(r1, r2) && same(i1, i2),
                (Expr::Field { recv: r1, name: n1, .. }, Expr::Field { recv: r2, name: n2, .. }) => {
                    n1.name == n2.name && same(r1, r2)
                }
                (
                    Expr::Binary { op: o1, lhs: l1, rhs: h1, .. },
                    Expr::Binary { op: o2, lhs: l2, rhs: h2, .. },
                ) => o1 == o2 && same(l1, l2) && same(h1, h2),
                _ => false,
            }
        }

        if !simple(key) {
            return None;
        }
        // `<something> + N`, with N a literal.
        let Expr::Binary { op: BinaryOp::Add, lhs, rhs, .. } = value else {
            return None;
        };
        let Expr::Int(delta, ..) = &**rhs else {
            return None;
        };
        // The left side is `m.get(k) ?? D`.
        let Expr::Binary { op: BinaryOp::Coalesce, lhs: got, rhs: dflt, .. } = &**lhs else {
            return None;
        };
        let Expr::Int(default, ..) = &**dflt else {
            return None;
        };
        let Expr::MethodCall { recv, name, args, .. } = &**got else {
            return None;
        };
        if name.name != "get" || args.len() != 1 {
            return None;
        }
        let Expr::Ident(m) = &**recv else {
            return None;
        };
        if m.name != map_name || !same(&args[0], key) {
            return None;
        }
        Some((*delta, *default))
    }

    fn fused_range_build(
        &self,
        range: &Expr,
        which: &str,
        f: &Expr,
        span: Span,
    ) -> Option<Expr> {
        let Expr::Closure { params, body, .. } = f else {
            return None;
        };
        if params.len() != 1 {
            return None;
        }
        // The accumulator needs a written element type: an empty list literal
        // with nothing in view is refused, and the closure body's type is not
        // knowable here — the loop variable it mentions is not bound yet. The
        // checker knows it and publishes it.
        let elem_name = if which == "filter" {
            "i64"
        } else {
            match self.cg.map_get_value_kind.get(&(span.start, span.end))? {
                0 => "i64",
                1 => "f64",
                2 => "bool",
                3 => "str",
                4 => "bytes",
                _ => return None,
            }
        };
        let elem_ty = grasm_syntax::ast::TypeExpr::List(
            Box::new(grasm_syntax::ast::TypeExpr::Name {
                path: grasm_syntax::ast::Path {
                    segments: vec![Ident { name: elem_name.to_string(), span }],
                    span,
                },
                args: Vec::new(),
                span,
            }),
            span,
        );
        let acc = Ident { name: "__fused_acc".to_string(), span };
        let item = params[0].clone();

        // `acc.push(<body>)` for map; `if <body> { acc.push(item) }` for filter.
        let push_mapped = Stmt::Expr(Expr::MethodCall {
            recv: Box::new(Expr::Ident(acc.clone())),
            name: Ident { name: "push".to_string(), span },
            args: vec![if which == "map" {
                (**body).clone()
            } else {
                Expr::Ident(item.clone())
            }],
            span,
        });
        let body_stmt = if which == "map" {
            push_mapped
        } else {
            Stmt::Expr(Expr::If {
                cond: Box::new((**body).clone()),
                then_block: grasm_syntax::ast::Block { stmts: vec![push_mapped], span },
                else_branch: None,
                span,
            })
        };

        Some(Expr::Block(grasm_syntax::ast::Block {
            stmts: vec![
                Stmt::Let {
                    mutable: true,
                    name: acc.clone(),
                    ty: Some(elem_ty),
                    init: Expr::List(Vec::new(), span),
                    span,
                },
                Stmt::For {
                    var: item,
                    iter: range.clone(),
                    body: grasm_syntax::ast::Block { stmts: vec![body_stmt], span },
                    span,
                },
                Stmt::Expr(Expr::Ident(acc)),
            ],
            span,
        }))
    }

    fn eval_method(
        &mut self,
        recv: &Expr,
        name: &Ident,
        args: &[Expr],
        span: Span,
    ) -> CResult<Option<(Value, VType)>> {
        // A combinator on a range: `(0..n).map(f)`. The backend has no range
        // *value* (§11), and it does not need one — a range is the bounds of a
        // loop, and the loop that builds a list from those bounds is a shape
        // this backend already compiles. So the call is rewritten into it and
        // the list methods take it from there.
        //
        // [G11] budgets Grasm at or below Python's token count and it was
        // 1.41×; the pattern this replaces — `var xs: [i64] = []`, a `for`, a
        // `push` and two braces — is where most of the difference was.
        // A range *held as a value* — `let r = 0..n` then `r.map(f)`. The
        // literal form below is fused into a single loop and this one is not:
        // the elements are built first and the list methods take it from there,
        // exactly as the interpreter does. So there is one definition of what a
        // range contains, and the fast path stays on the shape that can use it.
        if matches!(
            name.name.as_str(),
            "to_list" | "map" | "filter" | "sum" | "len"
        ) && !matches!(recv, Expr::Range { .. })
            && self.holds_a_range(recv)
        {
            let (rv, _) = self.eval_expr_val(recv)?;
            if name.name == "len" {
                // Answered from the bounds. Building five million integers in
                // order to count them would be the one case where a range
                // costs more than the list it replaced.
                let n = self.call_rt("grasm_range_len", &[rv]).unwrap();
                // Reading a name retains it; the count is a scalar and holds
                // nothing, so the retained range goes back here.
                self.call_rt("grasm_val_dec", &[rv]);
                return Ok(Some((n, VType::I64)));
            }
            let list = self.call_rt("grasm_range_to_list", &[rv]).unwrap();
            // The list is built from the bounds and does not borrow them.
            self.call_rt("grasm_val_dec", &[rv]);
            let lt = VType::List(Box::new(VType::I64));
            if name.name == "to_list" {
                return Ok(Some((list, lt)));
            }
            // Bound to a name the source cannot spell, so the list-method
            // paths — which dispatch on the shape of the receiver expression,
            // not on a value — take it from here unchanged.
            let tmp = self.new_var(&lt);
            self.b.def_var(tmp, list);
            let hidden = format!("__range_list_{}", span.start);
            self.push_scope();
            self.bind(&hidden, tmp, lt);
            let call = Expr::MethodCall {
                recv: Box::new(Expr::Ident(Ident { name: hidden, span })),
                name: name.clone(),
                args: args.to_vec(),
                span,
            };
            let out = self.eval_expr(&call);
            self.pop_scope_with_decs();
            return out;
        }

        if matches!(recv, Expr::Range { .. })
            && matches!(
                name.name.as_str(),
                "to_list" | "map" | "filter" | "sum" | "len"
            )
        {
            let as_list = self.range_to_list_expr(recv, span);
            if name.name == "to_list" {
                return self.eval_expr(&as_list);
            }
            // `map` and `filter` are *fused* into the same loop rather than
            // run over the list the range was first turned into. Building the
            // intermediate is two passes and an allocation the size of the
            // input: on `(0..=2_000_000).map(|_| true)` — how a sieve makes its
            // flags — the intermediate cost more than the sieve that followed
            // it, 20 ms against 5 ms of reading and 3 ms of writing.
            if name.name == "map" || name.name == "filter" {
                if let Some(fused) = self.fused_range_build(recv, &name.name, &args[0], span) {
                    return self.eval_expr(&fused);
                }
            }
            let call = Expr::MethodCall {
                recv: Box::new(as_list),
                name: name.clone(),
                args: args.to_vec(),
                span,
            };
            return self.eval_expr(&call);
        }
        // Mutating map/list methods on a variable receiver.
        if matches!(name.name.as_str(), "set" | "delete" | "pop" | "sort" | "sort_by" | "remove") {
            if let Expr::Ident(id) = recv {
                if let Some((var, vt)) = self.lookup(&id.name) {
                    match (&vt, name.name.as_str()) {
                        (VType::Map(k_t, v_t), "set") => {
                            // The counting idiom, fused into one lookup. It
                            // hashes the key once where writing it out hashes
                            // twice, which is what the C program this is
                            // measured against already did.
                            if matches!(**v_t, VType::I64 | VType::Any) {
                                if let Some((delta, default)) =
                                    Self::counter_bump(&id.name, &args[0], &args[1])
                                {
                                    let want_k: Option<VType> = if **k_t == VType::Any {
                                        None
                                    } else {
                                        Some((**k_t).clone())
                                    };
                                    let kt2 = k_t.clone();
                                    let (kv, kvt) =
                                        self.eval_expr_expecting(&args[0], want_k.as_ref())?;
                                    let kb = self.box_value(kv, &kvt);
                                    let d = self.b.ins().iconst(types::I64, delta);
                                    let z = self.b.ins().iconst(types::I64, default);
                                    let cur = self.b.use_var(var);
                                    let np = self
                                        .call_rt("grasm_map_add_i64", &[cur, kb, d, z])
                                        .unwrap();
                                    let refined = VType::Map(
                                        if *kt2 == VType::Any {
                                            Box::new(kvt)
                                        } else {
                                            kt2
                                        },
                                        Box::new(VType::I64),
                                    );
                                    self.rebind(&id.name, var, refined);
                                    self.b.def_var(var, np);
                                    return Ok(None);
                                }
                            }
                            let (kt, vt2) = (k_t.clone(), v_t.clone());
                            let want_k: Option<VType> = if **k_t == VType::Any { None } else { Some((**k_t).clone()) };
                            let want_v: Option<VType> = if **v_t == VType::Any { None } else { Some((**v_t).clone()) };
                            // The key is held while the value is evaluated,
                            // so a value that diverges — `m.set(k, try f())` —
                            // releases the key instead of stranding it. That
                            // is a parser's inner loop: read a key, then fail
                            // reading its value.
                            self.push_scope();
                            let (kv, kvt) = self.eval_expr_expecting(&args[0], want_k.as_ref())?;
                            let kb = self.box_value(kv, &kvt);
                            let kb = self.hold_operand((kb, VType::Any)).0;
                            let (vv, vvt) = self.eval_expr_expecting(&args[1], want_v.as_ref())?;
                            self.pop_scope_no_decs();
                            let vb = self.box_value(vv, &vvt);
                            let cur = self.b.use_var(var);
                            let np = self.call_rt("grasm_map_set", &[cur, kb, vb]).unwrap();
                            // Refine the map's element types once known.
                            let refined = VType::Map(
                                if *kt == VType::Any { Box::new(kvt) } else { kt },
                                if *vt2 == VType::Any { Box::new(vvt) } else { vt2 },
                            );
                            self.rebind(&id.name, var, refined);
                            self.b.def_var(var, np);
                            return Ok(None);
                        }
                        (VType::Map(_, _), "delete") => {
                            let (kv, kvt) = self.eval_expr_val(&args[0])?;
                            let kb = self.box_value(kv, &kvt);
                            let slot = self.b.create_sized_stack_slot(
                                cranelift_codegen::ir::StackSlotData::new(
                                    cranelift_codegen::ir::StackSlotKind::ExplicitSlot,
                                    8,
                                    3,
                                ),
                            );
                            let ptr_ty = self.cg.module.target_config().pointer_type();
                            let addr = self.b.ins().stack_addr(ptr_ty, slot, 0);
                            let cur = self.b.use_var(var);
                            let np = self
                                .call_rt("grasm_map_delete", &[cur, kb, addr])
                                .unwrap();
                            self.b.def_var(var, np);
                            self.call_rt("grasm_val_dec", &[kb]);
                            let found = self.b.ins().stack_load(types::I8, slot, 0);
                            return Ok(Some((found, VType::Bool)));
                        }
                        (VType::List(e), "pop") => {
                            let et = e.clone();
                            let ptr_ty = self.cg.module.target_config().pointer_type();
                            let slot = self.b.create_sized_stack_slot(
                                cranelift_codegen::ir::StackSlotData::new(
                                    cranelift_codegen::ir::StackSlotKind::ExplicitSlot,
                                    8,
                                    3,
                                ),
                            );
                            let addr = self.b.ins().stack_addr(ptr_ty, slot, 0);
                            let cur = self.b.use_var(var);
                            let np = self.call_rt("grasm_list_pop", &[cur, addr]).unwrap();
                            self.b.def_var(var, np);
                            let opt = self.b.ins().stack_load(types::I64, slot, 0);
                            return Ok(Some((opt, VType::Option(et))));
                        }
                        (VType::List(e), "remove") => {
                            let et = (**e).clone();
                            let (idx, _) = self.eval_expr_val(&args[0])?;
                            let ptr_ty = self.cg.module.target_config().pointer_type();
                            let slot = self.b.create_sized_stack_slot(
                                cranelift_codegen::ir::StackSlotData::new(
                                    cranelift_codegen::ir::StackSlotKind::ExplicitSlot,
                                    8,
                                    3,
                                ),
                            );
                            let addr = self.b.ins().stack_addr(ptr_ty, slot, 0);
                            let cur = self.b.use_var(var);
                            let np = self
                                .call_rt("grasm_list_remove", &[cur, idx, addr])
                                .unwrap();
                            self.b.def_var(var, np);
                            let boxed = self.b.ins().stack_load(types::I64, slot, 0);
                            let (v, vt) = self.unbox(boxed, &et)?;
                            return Ok(Some((v, vt)));
                        }
                        (VType::List(_), "sort") => {
                            let cur = self.b.use_var(var);
                            let np = self.call_rt("grasm_list_sort", &[cur]).unwrap();
                            self.b.def_var(var, np);
                            return Ok(None);
                        }
                        // Sorting by an extracted key: build the keys with the
                        // same loop `map` uses, then sort by them. The
                        // comparison stays in the runtime, so the key function
                        // runs exactly once per element.
                        (VType::List(e), "sort_by") => {
                            let elem_t = (**e).clone();
                            let (param, body) = match self.desugar_fn_arg(&args[0], false) {
                                Some(pb) => pb,
                                None => match &args[0] {
                                    Expr::Closure { params, body, .. } if params.len() == 1 => {
                                        (params[0].clone(), (**body).clone())
                                    }
                                    other => {
                                        return nope(
                                            other.span(),
                                            "sort_by with something other than a one-argument function",
                                        )
                                    }
                                },
                            };
                            let cur = self.b.use_var(var);
                            self.call_rt("grasm_val_inc", &[cur]);
                            let (keys, _) =
                                self.emit_list_transform(cur, &elem_t, &param, &body, true)?;
                            let cur2 = self.b.use_var(var);
                            let np = self.call_rt("grasm_list_sort_by", &[cur2, keys]).unwrap();
                            self.b.def_var(var, np);
                            self.call_rt("grasm_val_dec", &[keys]);
                            return Ok(None);
                        }
                        _ => {}
                    }
                }
            }
        }
        // The same mutating methods on a nested place: `bag.seen.set(k, v)`.
        // Reached only when the receiver is not a plain variable, so the
        // variable case above still takes the shorter path.
        if matches!(
            name.name.as_str(),
            "set" | "delete" | "sort" | "sort_by" | "pop" | "remove"
        ) && matches!(recv, Expr::Field { .. } | Expr::Index { .. })
        {
            let (root, path) = self.place_path(recv)?;
            self.pin_place_indices(&path)?;
            let Some((var, root_t)) = self.lookup(&root.name) else {
                return nope(root.span, "a mutating method on an unknown variable");
            };
            let inner_t = self.place_type(&root_t, &path, span)?;
            match (&inner_t, name.name.as_str()) {
                (VType::Map(k_t, v_t), "set") => {
                    let want_k: Option<VType> =
                        if **k_t == VType::Any { None } else { Some((**k_t).clone()) };
                    let want_v: Option<VType> =
                        if **v_t == VType::Any { None } else { Some((**v_t).clone()) };
                    // The key is held while the value is evaluated, so a
                    // value that diverges — `m.set(k, try f())` — releases the
                    // key instead of stranding it. That is the shape a parser
                    // has: read a key, then fail reading its value.
                    self.push_scope();
                    let (kv, kvt) = self.eval_expr_expecting(&args[0], want_k.as_ref())?;
                    let kb = self.box_value(kv, &kvt);
                    let kb = self.hold_operand((kb, VType::Any)).0;
                    let (vv, vvt) = self.eval_expr_expecting(&args[1], want_v.as_ref())?;
                    self.pop_scope_no_decs();
                    let vb = self.box_value(vv, &vvt);
                    let cur = self.b.use_var(var);
                    let mapv = self.read_place_boxed(cur, &path, &root_t, true)?;
                    let newmap = self.call_rt("grasm_map_set", &[mapv, kb, vb]).unwrap();
                    let cur2 = self.b.use_var(var);
                    self.begin_write();
                    let updated = self.write_place(span, cur2, &path, &root_t, newmap)?;
                    self.b.def_var(var, updated);
                    return Ok(None);
                }
                (VType::Map(_, _), "delete") => {
                    let (kv, kvt) = self.eval_expr_val(&args[0])?;
                    let kb = self.box_value(kv, &kvt);
                    let slot = self.b.create_sized_stack_slot(
                        cranelift_codegen::ir::StackSlotData::new(
                            cranelift_codegen::ir::StackSlotKind::ExplicitSlot,
                            8,
                            3,
                        ),
                    );
                    let ptr_ty = self.cg.module.target_config().pointer_type();
                    let addr = self.b.ins().stack_addr(ptr_ty, slot, 0);
                    let cur = self.b.use_var(var);
                    let mapv = self.read_place_boxed(cur, &path, &root_t, true)?;
                    let newmap = self.call_rt("grasm_map_delete", &[mapv, kb, addr]).unwrap();
                    let cur2 = self.b.use_var(var);
                    self.begin_write();
                    let updated = self.write_place(span, cur2, &path, &root_t, newmap)?;
                    self.b.def_var(var, updated);
                    self.call_rt("grasm_val_dec", &[kb]);
                    let found = self.b.ins().stack_load(types::I8, slot, 0);
                    return Ok(Some((found, VType::Bool)));
                }
                (VType::List(_), "sort") => {
                    let cur = self.b.use_var(var);
                    let listv = self.read_place_boxed(cur, &path, &root_t, true)?;
                    let newlist = self.call_rt("grasm_list_sort", &[listv]).unwrap();
                    let cur2 = self.b.use_var(var);
                    self.begin_write();
                    let updated = self.write_place(span, cur2, &path, &root_t, newlist)?;
                    self.b.def_var(var, updated);
                    return Ok(None);
                }
                // `vm.stack.pop()` — the core operation of anything holding a
                // stack in a struct, and the reason a whole VM had to use
                // `slice` instead.
                // The same sort as on a plain variable. Left out, this fell
                // through to the UFCS path and was refused as "a UFCS call of
                // a generic function", naming neither the shape nor the
                // reason — for `s.rows.sort_by(key)`, which is where a sort
                // by an extracted key is usually written.
                (VType::List(e), "sort_by") => {
                    let elem_t = (**e).clone();
                    let (param, body) = match self.desugar_fn_arg(&args[0], false) {
                        Some(pb) => pb,
                        None => match &args[0] {
                            Expr::Closure { params, body, .. } if params.len() == 1 => {
                                (params[0].clone(), (**body).clone())
                            }
                            other => {
                                return nope(
                                    other.span(),
                                    "sort_by with something other than a one-argument function",
                                )
                            }
                        },
                    };
                    let cur = self.b.use_var(var);
                    let listv = self.read_place_boxed(cur, &path, &root_t, true)?;
                    self.call_rt("grasm_val_inc", &[listv]);
                    let (keys, _) =
                        self.emit_list_transform(listv, &elem_t, &param, &body, true)?;
                    let newlist = self.call_rt("grasm_list_sort_by", &[listv, keys]).unwrap();
                    self.call_rt("grasm_val_dec", &[keys]);
                    let cur2 = self.b.use_var(var);
                    self.begin_write();
                    let updated = self.write_place(span, cur2, &path, &root_t, newlist)?;
                    self.b.def_var(var, updated);
                    return Ok(None);
                }
                (VType::List(e), "pop") => {
                    let et = e.clone();
                    let ptr_ty = self.cg.module.target_config().pointer_type();
                    let slot = self.b.create_sized_stack_slot(
                        cranelift_codegen::ir::StackSlotData::new(
                            cranelift_codegen::ir::StackSlotKind::ExplicitSlot,
                            8,
                            3,
                        ),
                    );
                    let addr = self.b.ins().stack_addr(ptr_ty, slot, 0);
                    let cur = self.b.use_var(var);
                    let listv = self.read_place_boxed(cur, &path, &root_t, true)?;
                    let newlist = self.call_rt("grasm_list_pop", &[listv, addr]).unwrap();
                    let cur2 = self.b.use_var(var);
                    self.begin_write();
                    let updated = self.write_place(span, cur2, &path, &root_t, newlist)?;
                    self.b.def_var(var, updated);
                    let opt = self.b.ins().stack_load(types::I64, slot, 0);
                    return Ok(Some((opt, VType::Option(et))));
                }
                (VType::List(e), "remove") => {
                    let et = (**e).clone();
                    let (idx, _) = self.eval_expr_val(&args[0])?;
                    let ptr_ty = self.cg.module.target_config().pointer_type();
                    let slot = self.b.create_sized_stack_slot(
                        cranelift_codegen::ir::StackSlotData::new(
                            cranelift_codegen::ir::StackSlotKind::ExplicitSlot,
                            8,
                            3,
                        ),
                    );
                    let addr = self.b.ins().stack_addr(ptr_ty, slot, 0);
                    let cur = self.b.use_var(var);
                    let listv = self.read_place_boxed(cur, &path, &root_t, true)?;
                    let newlist = self
                        .call_rt("grasm_list_remove", &[listv, idx, addr])
                        .unwrap();
                    let cur2 = self.b.use_var(var);
                    self.begin_write();
                    let updated = self.write_place(span, cur2, &path, &root_t, newlist)?;
                    self.b.def_var(var, updated);
                    let boxed = self.b.ins().stack_load(types::I64, slot, 0);
                    let (v, vt) = self.unbox(boxed, &et)?;
                    return Ok(Some((v, vt)));
                }
                _ => return nope(span, "this mutating method on this place"),
            }
        }
        // Mutating list push on a nested place: `bag.items.push(v)`.
        if name.name == "push" && matches!(recv, Expr::Field { .. } | Expr::Index { .. }) {
            let (root, path) = self.place_path(recv)?;
            self.pin_place_indices(&path)?;
            let Some((var, root_t)) = self.lookup(&root.name) else {
                return nope(root.span, "push on an unknown variable");
            };
            let list_t = self.place_type(&root_t, &path, span)?;
            let VType::List(elem_t) = list_t.clone() else {
                return nope(span, "push on non-lists");
            };
            let (av, _) = self.eval_expr_expecting(&args[0], Some(&elem_t))?;
            let boxed = self.box_value(av, &elem_t);
            let cur = self.b.use_var(var);
            let list_val = self.read_place_boxed(cur, &path, &root_t, true)?;
            let newlist = self.call_rt("grasm_list_push", &[list_val, boxed]).unwrap();
            let cur2 = self.b.use_var(var);
            self.begin_write();
            let updated = self.write_place(span, cur2, &path, &root_t, newlist)?;
            self.b.def_var(var, updated);
            return Ok(None);
        }
        // A module alias is not a receiver: `bag.push(xs, 7)` is a call to
        // that module's `push`, not a list push. Treating every `push` as the
        // builtin refused it as "push on unknown variable" — which names
        // neither the construct nor the fix, and made the *name* of a `pub fn`
        // decide whether a package could be built natively.
        let via_module = matches!(recv, Expr::Ident(tid)
            if self.lookup(&tid.name).is_none()
                && self
                    .cg
                    .module_imports
                    .get(&self.cg.current_module)
                    .is_some_and(|m| m.contains_key(&tid.name)));
        // Mutating list push on a variable.
        if name.name == "push" && !via_module {
            let Expr::Ident(id) = recv else {
                return nope(span, "push on non-variable receivers");
            };
            let Some((var, vt)) = self.lookup(&id.name) else {
                return nope(id.span, "push on unknown variable");
            };
            let VType::List(elem_t) = vt else {
                return nope(span, "push on non-lists");
            };
            // The list's element type is what an empty literal needs, and it
            // is right here: `succ.push([])` on a `[[i64]]` said the type
            // could not be inferred while naming the one place it came from.
            let want = if *elem_t == VType::Any {
                None
            } else {
                Some((*elem_t).clone())
            };
            let (mut av, avt) = self.eval_expr_expecting(&args[0], want.as_ref())?;
            let avt = if *elem_t == VType::F64 && avt == VType::I64 {
                av = self.b.ins().fcvt_from_sint(types::F64, av);
                VType::F64
            } else {
                avt
            };
            let cur = self.b.use_var(var);
            let newp = self.list_push_elem(cur, av, &avt);
            self.b.def_var(var, newp);
            return Ok(None);
        }
        // Cross-module call: `utils.helper(...)`.
        if let Expr::Ident(tid) = recv {
            if self.lookup(&tid.name).is_none() {
                let target = self
                    .cg
                    .module_imports
                    .get(&self.cg.current_module)
                    .and_then(|m| m.get(&tid.name))
                    .cloned();
                if let Some(target_mod) = target {
                    let mut qual = format!("{target_mod}::{}", name.name);
                    // A generic in another module instantiates the same way,
                    // just under that module's namespace.
                    if !self.cg.fns.contains_key(&qual)
                        && self.cg.generic_fns.contains_key(&qual)
                    {
                        let arg_types = self.generic_arg_types(&qual, args)?;
                        qual = self.cg.instantiate_generic(
                            &target_mod,
                            &name.name,
                            &arg_types,
                            span,
                        )?;
                    }
                    if let Some(sig) = self.cg.fns.get(&qual) {
                        let (fid, params, modes, ret) = (
                            sig.id,
                            sig.params.clone(),
                            sig.modes.clone(),
                            sig.ret.clone(),
                        );
                        // `inout` marshals the same way here as in a
                        // same-module call: the caller's variable goes through
                        // a stack slot and is read back after. Refusing it
                        // across a module boundary made the documented way to
                        // thread mutable state stop at the file edge.
                        let ptr_ty = self.cg.module.target_config().pointer_type();
                        let mut vals = Vec::with_capacity(args.len());
                        let mut inout_args: Vec<(cranelift_codegen::ir::StackSlot, Variable, VType)> =
                            Vec::new();
                        for (i, (a, want)) in args.iter().zip(&params).enumerate() {
                            if modes.get(i).copied() == Some(ParamMode::Inout) {
                                let Expr::Ident(aid) = a else {
                                    return nope(
                                        a.span(),
                                        "`inout` arguments that aren't plain variables",
                                    );
                                };
                                let Some((avar, avt)) = self.lookup(&aid.name) else {
                                    return nope(aid.span, "an unknown `inout` argument");
                                };
                                let slot = self.b.create_sized_stack_slot(
                                    cranelift_codegen::ir::StackSlotData::new(
                                        cranelift_codegen::ir::StackSlotKind::ExplicitSlot,
                                        8,
                                        3,
                                    ),
                                );
                                let cur = self.b.use_var(avar);
                                self.b.ins().stack_store(cur, slot, 0);
                                let addr = self.b.ins().stack_addr(ptr_ty, slot, 0);
                                vals.push(addr);
                                inout_args.push((slot, avar, avt));
                                continue;
                            }
                            let (mut v, vt) = self.eval_expr_expecting(a, Some(want))?;
                            if *want == VType::F64 && vt == VType::I64 {
                                v = self.b.ins().fcvt_from_sint(types::F64, v);
                            }
                            vals.push(v);
                        }
                        let fref = self.cg.module.declare_func_in_func(fid, self.b.func);
                        let inst = self.b.ins().call(fref, &vals);
                        let results: Vec<Value> = self.b.inst_results(inst).to_vec();
                        for (slot, var, vt) in inout_args {
                            let updated = self.b.ins().stack_load(vt.cl(), slot, 0);
                            self.b.def_var(var, updated);
                        }
                        return Ok(if ret == VType::Unit {
                            None
                        } else {
                            Some((results[0], ret))
                        });
                    }
                    // Variant constructor through the module alias.
                    if let Some((parent, _)) = self.cg.variant_index.get(&name.name).cloned() {
                        let v = self.build_variant_call(&parent, &name.name, args)?;
                        let vt = self.sum_vtype(&parent);
                        return Ok(Some((v, vt)));
                    }
                    return nope(span, format!("`{}` in module `{target_mod}`", name.name));
                }
            }
        }
        // Effect operation: `Clock.now(...)` — innermost handler wins.
        if let Expr::Ident(tid) = recv {
            if self.lookup(&tid.name).is_none() {
                if let Some(ops) = self.cg.effects.get(&tid.name).cloned() {
                    let Some((_, param_ts, ret_t)) =
                        ops.iter().find(|(o, _, _)| *o == name.name).cloned()
                    else {
                        return nope(name.span, format!("the operation `{}`", name.name));
                    };
                    let (ep, el) = self.str_ptr_len(&tid.name);
                    let (op_p, op_l) = self.str_ptr_len(&name.name);
                    let code = self
                        .call_rt("grasm_handler_lookup", &[ep, el, op_p, op_l])
                        .unwrap();
                    let env = self
                        .call_rt("grasm_handler_env", &[ep, el, op_p, op_l])
                        .unwrap();
                    let ptr_ty = self.cg.module.target_config().pointer_type();
                    let mut sig = self.cg.module.make_signature();
                    sig.params.push(AbiParam::new(ptr_ty));
                    for p in &param_ts {
                        sig.params.push(AbiParam::new(p.cl()));
                    }
                    if ret_t != VType::Unit {
                        sig.returns.push(AbiParam::new(ret_t.cl()));
                    }
                    let sig_ref = self.b.import_signature(sig);
                    let mut vals = Vec::with_capacity(args.len() + 1);
                    vals.push(env);
                    for (a, want) in args.iter().zip(&param_ts) {
                        let (mut v, vt) = self.eval_expr_expecting(a, Some(want))?;
                        if *want == VType::F64 && vt == VType::I64 {
                            v = self.b.ins().fcvt_from_sint(types::F64, v);
                        }
                        vals.push(v);
                    }
                    let inst = self.b.ins().call_indirect(sig_ref, code, &vals);
                    let results = self.b.inst_results(inst);
                    return Ok(if ret_t == VType::Unit {
                        None
                    } else {
                        Some((results[0], ret_t))
                    });
                }
            }
        }
        // Qualified variant: `Shape.Circle(...)` / `Color.Red`.
        if let Expr::Ident(tid) = recv {
            if self.lookup(&tid.name).is_none()
                && (self.cg.sums.contains_key(&tid.name) || tid.name == "Option")
            {
                let parent = tid.name.clone();
                let v = self.build_variant_call(&parent, &name.name, args)?;
                let vt = self.sum_vtype(&parent);
                return Ok(Some((v, vt)));
            }
        }
        // A read-only method on a plain local borrows its receiver: the
        // variable already holds a reference for the whole call, so the
        // ordinary +1/-1 pair around it is pure traffic. `xs.len()` in a loop
        // was three runtime calls where one was needed.
        //
        // Only methods that take the receiver, read it, and keep nothing are
        // eligible — the general path below hands `rv` to things that consume
        // it, and borrowing there would free it early.
        if let Some(out) = self.borrowed_read(recv, name, args)? {
            return Ok(Some(out));
        }
        let (rv, rt) = self.eval_expr_val(recv)?;
        // list.get(i) -> Option[elem]
        if let (VType::List(elem_t), "get") = (&rt, name.name.as_str()) {
            let elem_t = elem_t.clone();
            let (iv, it) = self.eval_expr_val(&args[0])?;
            if it != VType::I64 {
                return nope(args[0].span(), "non-integer index");
            }
            let opt = self.call_rt("grasm_list_get_opt", &[rv, iv]).unwrap();
            self.call_rt("grasm_val_dec", &[rv]);
            return Ok(Some((opt, VType::Option(elem_t))));
        }
        // Option methods.
        if let VType::Option(payload_t) = rt.clone() {
            match name.name.as_str() {
                "is_some" | "is_none" => {
                    let (sp, sl) = self.str_ptr_len("Some");
                    let is_some = self.call_rt("grasm_variant_is", &[rv, sp, sl]).unwrap();
                    self.call_rt("grasm_val_dec", &[rv]);
                    let out = if name.name == "is_none" {
                        let one = self.b.ins().iconst(types::I8, 1);
                        self.b.ins().bxor(is_some, one)
                    } else {
                        is_some
                    };
                    return Ok(Some((out, VType::Bool)));
                }
                "unwrap_or" => {
                    return self.coalesce_values(rv, &payload_t, &args[0]).map(Some);
                }
                "expect" => {
                    let some_blk = self.b.create_block();
                    let none_blk = self.b.create_block();
                    let (sp, sl) = self.str_ptr_len("Some");
                    let c = self.call_rt("grasm_variant_is", &[rv, sp, sl]).unwrap();
                    self.b.ins().brif(c, some_blk, &[], none_blk, &[]);
                    self.filled = true;

                    // The message is only built on the failing path, so a
                    // computed one costs nothing when the Option is Some.
                    self.goto_block(none_blk);
                    let (msg, _) = self.eval_expr_val(&args[0])?;
                    self.call_rt("grasm_panic_value", &[msg]);
                    self.trap_here();
                    self.filled = true;

                    self.goto_block(some_blk);
                    let zero = self.b.ins().iconst(types::I64, 0);
                    let payload = self.call_rt("grasm_variant_field", &[rv, zero]).unwrap();
                    self.call_rt("grasm_val_dec", &[rv]);
                    let (v, vt) = self.unbox(payload, &payload_t)?;
                    return Ok(Some((v, vt)));
                }
                _ => {}
            }
        }
        self.dispatch_method(rv, rt, recv, name, args, span)
    }

    /// Full builtin-method surface over an owned receiver value. `recv` is the
    /// receiver *expression* (needed for `inout` UFCS calls).
    #[allow(clippy::too_many_arguments)]
    /// `xs.len()` and friends on a plain local, without touching its
    /// refcount. `None` means "not one of these", and the caller falls
    /// through to the owning path.
    fn borrowed_read(
        &mut self,
        recv: &Expr,
        name: &Ident,
        args: &[Expr],
    ) -> CResult<Option<(Value, VType)>> {
        if !args.is_empty() {
            return Ok(None);
        }
        let Expr::Ident(id) = recv else {
            return Ok(None);
        };
        let Some((var, vt)) = self.lookup(&id.name) else {
            return Ok(None);
        };
        if !vt.boxed() {
            return Ok(None);
        }
        let f = match (&vt, name.name.as_str()) {
            (VType::List(_), "len") => "grasm_list_len",
            (VType::List(_), "is_empty") => "grasm_list_is_empty",
            (VType::Map(_, _), "len") => "grasm_map_len",
            (VType::Map(_, _), "is_empty") => "grasm_map_is_empty",
            (VType::Str, "len") => "grasm_str_len",
            (VType::Str, "byte_len") => "grasm_str_byte_len",
            (VType::Str, "is_empty") => "grasm_str_is_empty",
            // Each of these takes the receiver, reads it, and keeps nothing.
            (VType::List(_), "rev") => "grasm_list_rev",
            (VType::Map(_, _), "keys") => "grasm_map_keys",
            (VType::Map(_, _), "values") => "grasm_map_values",
            (VType::Map(_, _), "entries") => "grasm_map_entries",
            (VType::Str, "chars") => "grasm_str_chars",
            (VType::Str, "lines") => "grasm_str_lines",
            (VType::Str, "codes") => "grasm_str_codes",
            (VType::Str, "words") => "grasm_str_words",
            (VType::Str, "trim") => "grasm_str_trim",
            (VType::Str, "trim_start") => "grasm_str_trim_start",
            (VType::Str, "trim_end") => "grasm_str_trim_end",
            (VType::Str, "to_upper") => "grasm_str_upper",
            (VType::Str, "to_lower") => "grasm_str_lower",
            _ => return Ok(None),
        };
        let borrowed = self.b.use_var(var);
        let v = self.call_rt(f, &[borrowed]).unwrap();
        // This match must name every method the table above accepts. It used to
        // end in `(VType::Str, _) => VType::Str`, and that catch-all is a trap:
        // `byte_len` was added to the table above, returned an `i64`, and was
        // typed here as a boxed string. The compiled program then called `.len()`
        // on the integer 10 as though it were a pointer and took SIGSEGV, while
        // `grasm run` printed the right answer. Naming each method turns that
        // silent miscompile into a fall-through to the owning path, which is
        // slower and correct.
        let out_t = match (&vt, name.name.as_str()) {
            (_, "len") => VType::I64,
            (VType::Str, "byte_len") => VType::I64,
            (_, "is_empty") => VType::Bool,
            (VType::List(_), "rev") => vt.clone(),
            (VType::Map(k, _), "keys") => VType::List(k.clone()),
            (VType::Map(_, v), "values") => VType::List(v.clone()),
            (VType::Map(k, v), "entries") => VType::List(Box::new(VType::Tuple(vec![
                (**k).clone(),
                (**v).clone(),
            ]))),
            (VType::Str, "chars" | "lines" | "words") => VType::List(Box::new(VType::Str)),
            (VType::Str, "codes") => VType::List(Box::new(VType::I64)),
            (VType::Str, "trim" | "trim_start" | "trim_end" | "to_upper" | "to_lower") => {
                VType::Str
            }
            _ => return Ok(None),
        };
        Ok(Some((v, out_t)))
    }

    fn dispatch_method(
        &mut self,
        rv: Value,
        rt: VType,
        recv: &Expr,
        name: &Ident,
        args: &[Expr],
        span: Span,
    ) -> CResult<Option<(Value, VType)>> {
        let m = name.name.as_str();
        // Helper closures can't borrow self; inline the arg evaluation.
        macro_rules! arg_boxed {
            ($i:expr) => {{
                let (av, avt) = self.eval_expr_val(&args[$i])?;
                self.box_value(av, &avt)
            }};
        }
        macro_rules! arg_scalar {
            ($i:expr, $want:expr) => {{
                let (av, avt) = self.eval_expr_val(&args[$i])?;
                if avt != $want {
                    return nope(args[$i].span(), "an argument of a different type");
                }
                av
            }};
        }
        let out: (Value, VType) = match (&rt, m) {
            // ---- str ----
            (VType::Str, "len") => {
                let v = self.call_rt("grasm_str_len", &[rv]).unwrap();
                self.call_rt("grasm_val_dec", &[rv]);
                (v, VType::I64)
            }
            // Bytes, not code points: what a `Content-Length` needs.
            (VType::Str, "byte_len") => {
                let v = self.call_rt("grasm_str_byte_len", &[rv]).unwrap();
                self.call_rt("grasm_val_dec", &[rv]);
                (v, VType::I64)
            }
            (VType::Str, "contains" | "starts_with" | "ends_with") => {
                let a = arg_boxed!(0);
                let f = match m {
                    "contains" => "grasm_str_contains",
                    "starts_with" => "grasm_str_starts_with",
                    _ => "grasm_str_ends_with",
                };
                let v = self.call_rt(f, &[rv, a]).unwrap();
                self.call_rt("grasm_val_dec", &[rv]);
                self.call_rt("grasm_val_dec", &[a]);
                (v, VType::Bool)
            }
            (VType::Str, "split") => {
                let a = arg_boxed!(0);
                let v = self.call_rt("grasm_str_split", &[rv, a]).unwrap();
                self.call_rt("grasm_val_dec", &[rv]);
                self.call_rt("grasm_val_dec", &[a]);
                (v, VType::List(Box::new(VType::Str)))
            }
            (VType::Str, "chars" | "lines") => {
                let f = if m == "chars" {
                    "grasm_str_chars"
                } else {
                    "grasm_str_lines"
                };
                let v = self.call_rt(f, &[rv]).unwrap();
                self.call_rt("grasm_val_dec", &[rv]);
                (v, VType::List(Box::new(VType::Str)))
            }
            (VType::Str, "is_empty") => {
                let v = self.call_rt("grasm_str_is_empty", &[rv]).unwrap();
                self.call_rt("grasm_val_dec", &[rv]);
                (v, VType::Bool)
            }
            (VType::Str, "pad_start" | "pad_end") => {
                let at_start = i64::from(m == "pad_start");
                let w = arg_scalar!(0, VType::I64);
                let fill = arg_boxed!(1);
                let flag = self.b.ins().iconst(types::I8, at_start);
                let v = self.call_rt("grasm_str_pad", &[rv, w, fill, flag]).unwrap();
                self.call_rt("grasm_val_dec", &[rv]);
                self.call_rt("grasm_val_dec", &[fill]);
                (v, VType::Str)
            }
            (VType::Str, "trim" | "trim_start" | "trim_end" | "to_upper" | "to_lower") => {
                let f = match m {
                    "trim" => "grasm_str_trim",
                    "trim_start" => "grasm_str_trim_start",
                    "trim_end" => "grasm_str_trim_end",
                    "to_upper" => "grasm_str_upper",
                    _ => "grasm_str_lower",
                };
                let v = self.call_rt(f, &[rv]).unwrap();
                self.call_rt("grasm_val_dec", &[rv]);
                (v, VType::Str)
            }
            (VType::Str, "code_at") => {
                let n = arg_scalar!(0, VType::I64);
                let v = self.call_rt("grasm_str_code_at", &[rv, n]).unwrap();
                self.call_rt("grasm_val_dec", &[rv]);
                (v, VType::I64)
            }
            (VType::Str, "words") => {
                let v = self.call_rt("grasm_str_words", &[rv]).unwrap();
                self.call_rt("grasm_val_dec", &[rv]);
                (v, VType::List(Box::new(VType::Str)))
            }
            (VType::Str, "codes") => {
                let v = self.call_rt("grasm_str_codes", &[rv]).unwrap();
                self.call_rt("grasm_val_dec", &[rv]);
                (v, VType::List(Box::new(VType::I64)))
            }
            (VType::F64, "is_nan") => (
                self.call_rt("grasm_f64_is_nan", &[rv]).unwrap(),
                VType::Bool,
            ),
            (VType::F64, "is_finite") => (
                self.call_rt("grasm_f64_is_finite", &[rv]).unwrap(),
                VType::Bool,
            ),
            (VType::F64, "to_fixed") => {
                let n = arg_scalar!(0, VType::I64);
                let v = self.call_rt("grasm_f64_to_fixed", &[rv, n]).unwrap();
                (v, VType::Str)
            }
            (VType::I64, "to_char") => {
                let v = self.call_rt("grasm_i64_to_char", &[rv]).unwrap();
                (v, VType::Str)
            }
            (VType::Str, "repeat") => {
                let n = arg_scalar!(0, VType::I64);
                let v = self.call_rt("grasm_str_repeat", &[rv, n]).unwrap();
                self.call_rt("grasm_val_dec", &[rv]);
                (v, VType::Str)
            }
            (VType::Str, "replace") => {
                let a = arg_boxed!(0);
                let b = arg_boxed!(1);
                let v = self.call_rt("grasm_str_replace", &[rv, a, b]).unwrap();
                self.call_rt("grasm_val_dec", &[rv]);
                self.call_rt("grasm_val_dec", &[a]);
                self.call_rt("grasm_val_dec", &[b]);
                (v, VType::Str)
            }
            (VType::Str, "substr") => {
                let a = arg_scalar!(0, VType::I64);
                let b = arg_scalar!(1, VType::I64);
                let v = self.call_rt("grasm_str_substr", &[rv, a, b]).unwrap();
                self.call_rt("grasm_val_dec", &[rv]);
                (v, VType::Str)
            }
            (VType::Str, "index_of") => {
                let a = arg_boxed!(0);
                let v = self.call_rt("grasm_str_index_of", &[rv, a]).unwrap();
                self.call_rt("grasm_val_dec", &[rv]);
                self.call_rt("grasm_val_dec", &[a]);
                (v, VType::Option(Box::new(VType::I64)))
            }
            (VType::Str, "parse_int") => {
                let v = self.call_rt("grasm_str_parse_int", &[rv]).unwrap();
                self.call_rt("grasm_val_dec", &[rv]);
                (v, VType::Option(Box::new(VType::I64)))
            }
            (VType::Str, "parse_int_base") => {
                let base = arg_scalar!(0, VType::I64);
                let v = self
                    .call_rt("grasm_str_parse_int_base", &[rv, base])
                    .unwrap();
                self.call_rt("grasm_val_dec", &[rv]);
                (v, VType::Option(Box::new(VType::I64)))
            }
            (VType::Str, "parse_float") => {
                let v = self.call_rt("grasm_str_parse_float", &[rv]).unwrap();
                self.call_rt("grasm_val_dec", &[rv]);
                (v, VType::Option(Box::new(VType::F64)))
            }
            // ---- list ----
            (VType::List(_), "len") => {
                let v = self.call_rt("grasm_list_len", &[rv]).unwrap();
                self.call_rt("grasm_val_dec", &[rv]);
                (v, VType::I64)
            }
            (VType::List(_), "is_empty") => {
                let v = self.call_rt("grasm_list_is_empty", &[rv]).unwrap();
                self.call_rt("grasm_val_dec", &[rv]);
                (v, VType::Bool)
            }
            (VType::List(e), "min" | "max") => {
                let et = e.clone();
                let flag = self.b.ins().iconst(types::I8, i64::from(m == "max"));
                let v = self.call_rt("grasm_list_min_max", &[rv, flag]).unwrap();
                self.call_rt("grasm_val_dec", &[rv]);
                (v, VType::Option(et))
            }
            (VType::List(e), "join") => {
                let _ = e;
                let a = arg_boxed!(0);
                let v = self.call_rt("grasm_list_join", &[rv, a]).unwrap();
                self.call_rt("grasm_val_dec", &[rv]);
                self.call_rt("grasm_val_dec", &[a]);
                (v, VType::Str)
            }
            (VType::List(e), "rev") => {
                let et = e.clone();
                let v = self.call_rt("grasm_list_rev", &[rv]).unwrap();
                self.call_rt("grasm_val_dec", &[rv]);
                (v, VType::List(et))
            }
            (VType::List(e), "slice") => {
                let et = e.clone();
                let a = arg_scalar!(0, VType::I64);
                let b = arg_scalar!(1, VType::I64);
                let v = self.call_rt("grasm_list_slice", &[rv, a, b]).unwrap();
                self.call_rt("grasm_val_dec", &[rv]);
                (v, VType::List(et))
            }
            (VType::List(_), "contains") => {
                let a = arg_boxed!(0);
                let v = self.call_rt("grasm_list_contains", &[rv, a]).unwrap();
                self.call_rt("grasm_val_dec", &[rv]);
                self.call_rt("grasm_val_dec", &[a]);
                (v, VType::Bool)
            }
            (VType::List(e), "first" | "last") => {
                let et = e.clone();
                let f = if m == "first" {
                    "grasm_list_first"
                } else {
                    "grasm_list_last"
                };
                let v = self.call_rt(f, &[rv]).unwrap();
                self.call_rt("grasm_val_dec", &[rv]);
                (v, VType::Option(et))
            }
            (VType::List(e), "sum") => {
                let is_float = **e == VType::F64;
                let f = if is_float {
                    "grasm_list_sum_f64"
                } else {
                    "grasm_list_sum_i64"
                };
                let v = self.call_rt(f, &[rv]).unwrap();
                self.call_rt("grasm_val_dec", &[rv]);
                (v, if is_float { VType::F64 } else { VType::I64 })
            }
            (VType::List(e), "any" | "all" | "find") => {
                let elem_t = (**e).clone();
                if let Some((param, body)) = self.desugar_fn_arg(&args[0], false) {
                    return self
                        .emit_list_search(rv, &elem_t, &param, &body, m)
                        .map(Some);
                }
                let Expr::Closure {
                    params: cparams,
                    body: cbody,
                    ..
                } = &args[0]
                else {
                    return nope(args[0].span(), "any/all/find with a non-literal closure");
                };
                if cparams.len() != 1 {
                    return nope(args[0].span(), "predicates with multiple params");
                }
                self.emit_list_search(rv, &elem_t, &cparams[0], cbody, m)?
            }
            (VType::List(e), "par_filter") => {
                let elem_t = (**e).clone();
                if let Some((param, body)) = self.desugar_fn_arg(&args[0], true) {
                    return self
                        .emit_par_filter(rv, &elem_t, &param, &body, span)
                        .map(Some);
                }
                let Expr::Closure {
                    params: cparams,
                    body: cbody,
                    ..
                } = &args[0]
                else {
                    return nope(
                        args[0].span(),
                        "par_filter with a closure bound to a name (write it inline, or as a top-level `fn`, which parallel workers can call)",
                    );
                };
                if cparams.len() != 1 {
                    return nope(args[0].span(), "par_filter with multiple parameters");
                }
                self.emit_par_filter(rv, &elem_t, &cparams[0], cbody, span)?
            }
            (VType::List(e), "par_map") => {
                let elem_t = (**e).clone();
                if let Some((param, body)) = self.desugar_fn_arg(&args[0], true) {
                    return self.emit_par_map(rv, &elem_t, &param, &body, span).map(Some);
                }
                let Expr::Closure {
                    params: cparams,
                    body: cbody,
                    ..
                } = &args[0]
                else {
                    return nope(
                        args[0].span(),
                        "par_map with a closure bound to a name (write it inline, or as a top-level `fn`, which parallel workers can call)",
                    );
                };
                if cparams.len() != 1 {
                    return nope(args[0].span(), "par_map with multiple parameters");
                }
                self.emit_par_map(rv, &elem_t, &cparams[0], cbody, span)?
            }
            (VType::List(e), "fold") => {
                let elem_t = (**e).clone();
                let (ap, ep, fbody) = match &args[1] {
                    Expr::Closure { params, body, span } => {
                        if params.len() != 2 {
                            return nope(*span, "a fold function that does not take two parameters");
                        }
                        (params[0].clone(), params[1].clone(), (**body).clone())
                    }
                    other => {
                        // A named function: `xs.fold(0, add)` becomes
                        // `xs.fold(0, |a, b| add(a, b))`.
                        let Expr::Ident(id) = other else {
                            return nope(other.span(), "fold with a non-literal function");
                        };
                        if self.lookup_fn(&id.name).is_none()
                            && !matches!(self.lookup(&id.name), Some((_, VType::Fn(_, _))))
                        {
                            return nope(other.span(), "fold with a non-literal function");
                        }
                        let sp = id.span;
                        let a = Ident { name: "__fold_acc".into(), span: sp };
                        let b = Ident { name: "__fold_elem".into(), span: sp };
                        let call = Expr::Call {
                            callee: Box::new(Expr::Ident(id.clone())),
                            args: vec![Expr::Ident(a.clone()), Expr::Ident(b.clone())],
                            span: sp,
                        };
                        (a, b, call)
                    }
                };
                return self
                    .emit_list_fold(rv, &elem_t, &args[0], &ap, &ep, &fbody)
                    .map(Some);
            }
            (VType::List(e), "enumerate") => {
                let pair = VType::Tuple(vec![VType::I64, (**e).clone()]);
                let v = self.call_rt("grasm_list_enumerate", &[rv]).unwrap();
                self.call_rt("grasm_val_dec", &[rv]);
                (v, VType::List(Box::new(pair)))
            }
            (VType::List(e), "zip") => {
                let (ov, ovt) = self.eval_expr_val(&args[0])?;
                let VType::List(oe) = ovt.clone() else {
                    return nope(args[0].span(), "zip with a non-list");
                };
                let pair = VType::Tuple(vec![(**e).clone(), (*oe).clone()]);
                let v = self.call_rt("grasm_list_zip", &[rv, ov]).unwrap();
                self.call_rt("grasm_val_dec", &[rv]);
                self.call_rt("grasm_val_dec", &[ov]);
                (v, VType::List(Box::new(pair)))
            }
            (VType::List(e), "map" | "filter") => {
                let elem_t = (**e).clone();
                if let Some((param, body)) = self.desugar_fn_arg(&args[0], false) {
                    let is_map = m == "map";
                    return self
                        .emit_list_transform(rv, &elem_t, &param, &body, is_map)
                        .map(Some);
                }
                let Expr::Closure {
                    params: cparams,
                    body: cbody,
                    ..
                } = &args[0]
                else {
                    return nope(args[0].span(), "map/filter with a non-literal closure");
                };
                if cparams.len() != 1 {
                    return nope(args[0].span(), "map/filter closures with multiple params");
                }
                let is_map = m == "map";
                self.emit_list_transform(rv, &elem_t, &cparams[0], cbody, is_map)?
            }
            // ---- map ----
            (VType::Map(_, v_t), "get") => {
                let vt = v_t.clone();
                let k = arg_boxed!(0);
                let v = self.call_rt("grasm_map_get", &[rv, k]).unwrap();
                self.call_rt("grasm_val_dec", &[rv]);
                self.call_rt("grasm_val_dec", &[k]);
                (v, VType::Option(vt))
            }
            (VType::Map(_, _), "has") => {
                let k = arg_boxed!(0);
                let v = self.call_rt("grasm_map_has", &[rv, k]).unwrap();
                self.call_rt("grasm_val_dec", &[rv]);
                self.call_rt("grasm_val_dec", &[k]);
                (v, VType::Bool)
            }
            (VType::Map(_, _), "len") => {
                let v = self.call_rt("grasm_map_len", &[rv]).unwrap();
                self.call_rt("grasm_val_dec", &[rv]);
                (v, VType::I64)
            }
            (VType::Map(_, _), "is_empty") => {
                let v = self.call_rt("grasm_map_is_empty", &[rv]).unwrap();
                self.call_rt("grasm_val_dec", &[rv]);
                (v, VType::Bool)
            }
            (VType::Map(k_t, _), "keys") => {
                let kt = k_t.clone();
                let v = self.call_rt("grasm_map_keys", &[rv]).unwrap();
                self.call_rt("grasm_val_dec", &[rv]);
                (v, VType::List(kt))
            }
            (VType::Map(_, v_t), "values") => {
                let vt = v_t.clone();
                let v = self.call_rt("grasm_map_values", &[rv]).unwrap();
                self.call_rt("grasm_val_dec", &[rv]);
                (v, VType::List(vt))
            }
            (VType::Map(k_t, v_t), "entries") => {
                let pair = VType::Tuple(vec![(**k_t).clone(), (**v_t).clone()]);
                let v = self.call_rt("grasm_map_entries", &[rv]).unwrap();
                self.call_rt("grasm_val_dec", &[rv]);
                (v, VType::List(Box::new(pair)))
            }
            // ---- numeric ----
            // The same total order sorting uses ([L-LIB-SORT-NAN]), callable.
            (VType::I64 | VType::F64 | VType::Str | VType::Tuple(_), "cmp") => {
                let rt2 = rt.clone();
                let ab = self.box_value(rv, &rt2);
                let (ov, ovt) = self.eval_expr_val(&args[0])?;
                let bb = self.box_value(ov, &ovt);
                let v = self.call_rt("grasm_val_cmp", &[ab, bb]).unwrap();
                self.call_rt("grasm_val_dec", &[ab]);
                self.call_rt("grasm_val_dec", &[bb]);
                (v, VType::I64)
            }
            (VType::I64, "as_f64") => {
                (self.b.ins().fcvt_from_sint(types::F64, rv), VType::F64)
            }
            (VType::I64, "to_str") => (
                self.call_rt("grasm_str_from_i64", &[rv]).unwrap(),
                VType::Str,
            ),
            (VType::I64, "to_base") => {
                let base = arg_scalar!(0, VType::I64);
                (
                    self.call_rt("grasm_i64_to_base", &[rv, base]).unwrap(),
                    VType::Str,
                )
            }
            (VType::I64, "abs") => (self.call_rt("grasm_i64_abs", &[rv]).unwrap(), VType::I64),
            // [L-LIB-BITS]. Emitted as instructions rather than runtime calls:
            // these are single machine operations and a call would cost more
            // than the work.
            //
            // The shift count is masked to 0..63 first. Cranelift's shifts
            // leave an out-of-range count as undefined behaviour, and the
            // interpreter masks — so without this, `1.shl(64)` would be one
            // answer under `grasm run` and whatever the hardware happened to do
            // under `--native`, which is precisely the class of disagreement
            // this project exists to prevent.
            // Methods on `bytes` ([L-LIB-BYTES]). Each is one runtime call, and
            // the receiver is already a boxed pointer, so there is nothing to
            // box or unbox here.
            // Each of these reads the receiver and keeps nothing, so each
            // releases it — the convention every other boxed-receiver arm
            // above follows. Omitting it leaks one allocation per call, which
            // is invisible in the output and which `scripts/leakcheck.sh`
            // caught on the first run.
            (VType::Bytes, "len") => {
                let n = self.call_rt("grasm_bytes_len", &[rv]).unwrap();
                self.call_rt("grasm_val_dec", &[rv]);
                (n, VType::I64)
            }
            // No `uextend` here: `icmp` already answers in `i8`, which is what
            // a `bool` is, and widening to the width it already has is a
            // verifier error rather than a no-op. Every `b.is_empty()` in the
            // language failed to compile — reported as a "codegen bug", which
            // is the backend's own word for "not a missing feature".
            (VType::Bytes, "is_empty") => {
                let n = self.call_rt("grasm_bytes_len", &[rv]).unwrap();
                self.call_rt("grasm_val_dec", &[rv]);
                let zero = self.b.ins().iconst(types::I64, 0);
                (self.b.ins().icmp(IntCC::Equal, n, zero), VType::Bool)
            }
            (VType::Bytes, "at") => {
                let i = arg_scalar!(0, VType::I64);
                let v = self.call_rt("grasm_bytes_at", &[rv, i]).unwrap();
                self.call_rt("grasm_val_dec", &[rv]);
                (v, VType::I64)
            }
            (VType::Bytes, "slice") => {
                let from = arg_scalar!(0, VType::I64);
                let count = arg_scalar!(1, VType::I64);
                let v = self
                    .call_rt("grasm_bytes_slice", &[rv, from, count])
                    .unwrap();
                self.call_rt("grasm_val_dec", &[rv]);
                (v, VType::Bytes)
            }
            (VType::Bytes, "concat") => {
                let (other, _) = self.eval_expr_val(&args[0])?;
                let out = self.call_rt("grasm_bytes_concat", &[rv, other]).unwrap();
                self.call_rt("grasm_val_dec", &[rv]);
                self.call_rt("grasm_val_dec", &[other]);
                (out, VType::Bytes)
            }
            (VType::Bytes, "to_list") => {
                let v = self.call_rt("grasm_bytes_to_list", &[rv]).unwrap();
                self.call_rt("grasm_val_dec", &[rv]);
                (v, VType::List(Box::new(VType::I64)))
            }
            (VType::Bytes, "to_str") => {
                let v = self.call_rt("grasm_bytes_to_str", &[rv]).unwrap();
                self.call_rt("grasm_val_dec", &[rv]);
                (v, VType::Str)
            }
            (VType::Str, "to_bytes") => {
                let v = self.call_rt("grasm_str_to_bytes", &[rv]).unwrap();
                self.call_rt("grasm_val_dec", &[rv]);
                (v, VType::Bytes)
            }
            (VType::I64, "bit_and" | "bit_or" | "bit_xor") => {
                let a = arg_scalar!(0, VType::I64);
                let out = match m {
                    "bit_and" => self.b.ins().band(rv, a),
                    "bit_or" => self.b.ins().bor(rv, a),
                    _ => self.b.ins().bxor(rv, a),
                };
                (out, VType::I64)
            }
            (VType::I64, "bit_not") => (self.b.ins().bnot(rv), VType::I64),
            (VType::I64, "shl" | "shr" | "ushr") => {
                let a = arg_scalar!(0, VType::I64);
                let sixty_three = self.b.ins().iconst(types::I64, 63);
                let count = self.b.ins().band(a, sixty_three);
                let out = match m {
                    "shl" => self.b.ins().ishl(rv, count),
                    "shr" => self.b.ins().sshr(rv, count),
                    _ => self.b.ins().ushr(rv, count),
                };
                (out, VType::I64)
            }
            (VType::I64, "min" | "max") => {
                let a = arg_scalar!(0, VType::I64);
                let f = if m == "min" {
                    "grasm_i64_min"
                } else {
                    "grasm_i64_max"
                };
                (self.call_rt(f, &[rv, a]).unwrap(), VType::I64)
            }
            // Saturating, not trapping. `"1e400".parse_float()` succeeds and
            // gives `inf`, so an out-of-range float reaches this from
            // ordinary data — and the trapping conversion killed the process
            // with SIGILL, no message, while the interpreter saturated. Rust's
            // `as` does the same thing this does: NaN to 0, the infinities to
            // the ends.
            (VType::F64, "as_i64") => (
                self.b.ins().fcvt_to_sint_sat(types::I64, rv),
                VType::I64,
            ),
            (VType::F64, "to_str") => (
                self.call_rt("grasm_str_from_f64", &[rv]).unwrap(),
                VType::Str,
            ),
            (VType::F64, "abs" | "sqrt" | "floor" | "ceil" | "round") => {
                let f = match m {
                    "abs" => "grasm_f64_abs",
                    "sqrt" => "grasm_f64_sqrt",
                    "floor" => "grasm_f64_floor",
                    "ceil" => "grasm_f64_ceil",
                    _ => "grasm_f64_round",
                };
                (self.call_rt(f, &[rv]).unwrap(), VType::F64)
            }
            // The math library. Every one is `f64 -> f64` with no allocation
            // and no effect, so it lowers to a direct runtime call.
            (VType::F64, "sin" | "cos" | "tan" | "asin" | "acos" | "atan" | "sinh" | "cosh" | "tanh" | "exp" | "ln" | "log2" | "log10" | "cbrt" | "to_radians" | "to_degrees") => {
                let f = match m {
                    "sin" => "grasm_f64_sin",
                    "cos" => "grasm_f64_cos",
                    "tan" => "grasm_f64_tan",
                    "asin" => "grasm_f64_asin",
                    "acos" => "grasm_f64_acos",
                    "atan" => "grasm_f64_atan",
                    "sinh" => "grasm_f64_sinh",
                    "cosh" => "grasm_f64_cosh",
                    "tanh" => "grasm_f64_tanh",
                    "exp" => "grasm_f64_exp",
                    "ln" => "grasm_f64_ln",
                    "log2" => "grasm_f64_log2",
                    "log10" => "grasm_f64_log10",
                    "cbrt" => "grasm_f64_cbrt",
                    "to_radians" => "grasm_f64_to_radians",
                    _ => "grasm_f64_to_degrees",
                };
                (self.call_rt(f, &[rv]).unwrap(), VType::F64)
            }
            (VType::F64, "atan2" | "hypot" | "log") => {
                let a = arg_scalar!(0, VType::F64);
                let f = match m {
                    "atan2" => "grasm_f64_atan2",
                    "hypot" => "grasm_f64_hypot",
                    _ => "grasm_f64_log",
                };
                (self.call_rt(f, &[rv, a]).unwrap(), VType::F64)
            }
            (VType::F64, "min" | "max") => {
                let a = arg_scalar!(0, VType::F64);
                let f = if m == "min" {
                    "grasm_f64_min"
                } else {
                    "grasm_f64_max"
                };
                (self.call_rt(f, &[rv, a]).unwrap(), VType::F64)
            }
            _ => {
                // UFCS: x.f(a) → f(x, a) — unless the checker resolved this
                // call to a field of the receiver holding a function. This
                // backend resolves by name, like the interpreter, and
                // [L-UFCS] takes a free function only when its first parameter
                // accepts the receiver: an unrelated `fn len(xs: [i64])` made
                // `n.len()` on a struct with a `len` field call the free
                // function, so the three engines printed two different
                // answers for a program that typechecks.
                // A *generic* free function called this way has no symbol until
                // the call site fixes its type parameters, and `lookup_fn` does
                // not carry generics — so `[1, 2].describe()` fell past this
                // arm and was refused while `describe([1, 2])` compiled. The
                // receiver is the first argument ([L-UFCS]), which is exactly
                // what the type parameters are read from.
                let ufcs_generic = if self.lookup_fn(&name.name).is_none()
                    && !self.cg.field_calls.contains(&(span.start, span.end))
                {
                    let generic_key = format!("{}::{}", self.cg.current_module, name.name);
                    if self.cg.generic_fns.contains_key(&generic_key) {
                        let mut all: Vec<Expr> = Vec::with_capacity(args.len() + 1);
                        all.push(recv.clone());
                        all.extend(args.iter().cloned());
                        let arg_types = self.generic_arg_types(&generic_key, &all)?;
                        let module = self.cg.current_module.clone();
                        Some(self.cg.instantiate_generic(
                            &module,
                            &name.name,
                            &arg_types,
                            span,
                        )?)
                    } else {
                        None
                    }
                } else {
                    None
                };
                let resolved: Option<FnSig> = match &ufcs_generic {
                    Some(k) => self.cg.fns.get(k).cloned(),
                    None => self
                        .lookup_fn(&name.name)
                        .filter(|_| !self.cg.field_calls.contains(&(span.start, span.end)))
                        .cloned(),
                };
                if let Some(sig) = resolved {
                    let (fid, params, modes, ret, is_extern) = (
                        sig.id,
                        sig.params.clone(),
                        sig.modes.clone(),
                        sig.ret.clone(),
                        sig.is_extern,
                    );
                    let ptr_ty = self.cg.module.target_config().pointer_type();
                    let mut vals: Vec<Value> = Vec::with_capacity(args.len() + 1);
                    // `s.strlen()` is the same call as `strlen(s)` ([L-UFCS]),
                    // so the pointer conversion has to happen here too. It did
                    // not, and the boxed value's own address was handed to C:
                    // `strlen` walked an `RtVal` and answered 1 for every
                    // string, while `grasm run` answered correctly.
                    let mut c_args: Vec<(Value, VType, Value)> = Vec::new();
                    // The receiver is the first parameter; honor `inout` by
                    // passing a pointer to the caller's slot.
                    let mut recv_slot: Option<(cranelift_codegen::ir::StackSlot, Variable, VType)> =
                        None;
                    if modes.first().copied() == Some(ParamMode::Inout) {
                        let Expr::Ident(rid) = recv else {
                            return nope(span, "`inout` receivers that aren't plain variables");
                        };
                        let Some((rvar, rvt)) = self.lookup(&rid.name) else {
                            return nope(rid.span, "an unknown `inout` receiver");
                        };
                        // `rv` was an owned +1 read of the same value; release it.
                        self.dec_if_boxed(rv, &rvt);
                        let slot = self.b.create_sized_stack_slot(
                            cranelift_codegen::ir::StackSlotData::new(
                                cranelift_codegen::ir::StackSlotKind::ExplicitSlot,
                                8,
                                3,
                            ),
                        );
                        let cur = self.b.use_var(rvar);
                        self.b.ins().stack_store(cur, slot, 0);
                        let addr = self.b.ins().stack_addr(ptr_ty, slot, 0);
                        vals.push(addr);
                        recv_slot = Some((slot, rvar, rvt));
                    } else if is_extern && matches!(params.first(), Some(VType::Str | VType::Bytes))
                    {
                        let want = params[0].clone();
                        let p = match want {
                            VType::Str => self.call_rt("grasm_str_c_new", &[rv]).unwrap(),
                            _ => self.call_rt("grasm_bytes_ptr", &[rv]).unwrap(),
                        };
                        c_args.push((rv, want, p));
                        vals.push(p);
                    } else {
                        vals.push(rv);
                    }
                    let mut arg_slots: Vec<(cranelift_codegen::ir::StackSlot, Variable, VType)> =
                        Vec::new();
                    for (i, (a, want)) in args.iter().zip(params.iter().skip(1)).enumerate() {
                        if modes.get(i + 1).copied() == Some(ParamMode::Inout) {
                            let Expr::Ident(aid) = a else {
                                return nope(
                                    a.span(),
                                    "`inout` arguments that aren't plain variables",
                                );
                            };
                            let Some((avar, avt)) = self.lookup(&aid.name) else {
                                return nope(aid.span, "an unknown `inout` argument");
                            };
                            let slot = self.b.create_sized_stack_slot(
                                cranelift_codegen::ir::StackSlotData::new(
                                    cranelift_codegen::ir::StackSlotKind::ExplicitSlot,
                                    8,
                                    3,
                                ),
                            );
                            let cur = self.b.use_var(avar);
                            self.b.ins().stack_store(cur, slot, 0);
                            let addr = self.b.ins().stack_addr(ptr_ty, slot, 0);
                            vals.push(addr);
                            arg_slots.push((slot, avar, avt));
                            continue;
                        }
                        let (mut v, vt) = self.eval_expr_expecting(a, Some(want))?;
                        if *want == VType::F64 && vt == VType::I64 {
                            v = self.b.ins().fcvt_from_sint(types::F64, v);
                        }
                        if is_extern && matches!(want, VType::Str | VType::Bytes) {
                            let p = match want {
                                VType::Str => self.call_rt("grasm_str_c_new", &[v]).unwrap(),
                                _ => self.call_rt("grasm_bytes_ptr", &[v]).unwrap(),
                            };
                            c_args.push((v, want.clone(), p));
                            vals.push(p);
                            continue;
                        }
                        vals.push(v);
                    }
                    let fref = self.cg.module.declare_func_in_func(fid, self.b.func);
                    let inst = self.b.ins().call(fref, &vals);
                    let results: Vec<Value> = self.b.inst_results(inst).to_vec();
                    if let Some((slot, rvar, rvt)) = recv_slot {
                        let updated = self.b.ins().stack_load(rvt.cl(), slot, 0);
                        self.b.def_var(rvar, updated);
                    }
                    for (slot, avar, avt) in arg_slots {
                        let updated = self.b.ins().stack_load(avt.cl(), slot, 0);
                        self.b.def_var(avar, updated);
                    }
                    // The C call is over: free the temporary copies and release
                    // the boxed values that backed the pointers.
                    for (boxed, vt, p) in c_args {
                        if vt == VType::Str {
                            self.call_rt("grasm_str_c_free", &[p]);
                        }
                        self.call_rt("grasm_val_dec", &[boxed]);
                    }
                    return Ok(if ret == VType::Unit {
                        None
                    } else {
                        Some((results[0], ret))
                    });
                }
                // A field holding a function, called through the field —
                // *only* when the checker resolved this call that way.
                //
                // The table is authoritative in both directions, and it has to
                // be. This backend resolves a free function with `lookup_fn`,
                // which does not carry generics: a *generic* free function of
                // the same name as a function-typed field left `lookup_fn`
                // empty, so the call fell through to here and ran the field,
                // while the checker had resolved it to the function. `grasm
                // run` printed `77` and the compiled binary printed a string
                // and then multiplied its heap address by two — exit 0, a
                // different number every run, nothing on stderr.
                //
                // Reading the table only to *suppress* this arm was half a
                // rule. It suppresses and it permits.
                if !self.cg.field_calls.contains(&(span.start, span.end)) {
                    return nope(
                        span,
                        format!(
                            "a call of `{}` that this backend cannot resolve the way the \
checker did",
                            name.name
                        ),
                    );
                }
                //
                // The receiver was already evaluated above for method
                // dispatch, and `eval_field` evaluates it again — so the first
                // one has to be released here. Before this arm existed the
                // path ended in a compile error, where leaking it cost
                // nothing; now it runs, and it leaked the receiver on every
                // call through a field.
                if rt.boxed() {
                    self.call_rt("grasm_val_dec", &[rv]);
                }
                if let Some((fv, fvt)) = self.eval_field(recv, name, span)? {
                    if let VType::Fn(param_ts, ret_t) = fvt.clone() {
                        // `call_fn_value` reads its callee out of a variable,
                        // so the field's value gets one.
                        let var = self.new_var(&fvt);
                        self.b.def_var(var, fv);
                        let out = self.call_fn_value(var, &param_ts, &ret_t, args, span)?;
                        // `eval_field` handed over an owned reference; the
                        // call borrowed it. Without this every call through a
                        // field leaked the function value and its environment.
                        let f2 = self.b.use_var(var);
                        self.call_rt("grasm_val_dec", &[f2]);
                        return Ok(out);
                    }
                }
                return nope(span, format!("the method `{}`", name.name));
            }
        };
        Ok(Some(out))
    }

    fn short_circuit(
        &mut self,
        op: BinaryOp,
        lhs: &Expr,
        rhs: &Expr,
    ) -> CResult<Option<(Value, VType)>> {
        let (lv, lt) = self.eval_expr_val(lhs)?;
        if lt != VType::Bool {
            return nope(lhs.span(), "non-bool logic operand");
        }
        let rhs_blk = self.b.create_block();
        let merge = self.b.create_block();
        self.b.append_block_param(merge, types::I8);
        match op {
            BinaryOp::And => {
                self.b.ins().brif(lv, rhs_blk, &[], merge, &[lv]);
                self.filled = true;
            }
            BinaryOp::Or => {
                self.b.ins().brif(lv, merge, &[lv], rhs_blk, &[]);
                self.filled = true;
            }
            _ => unreachable!(),
        }
        self.goto_block(rhs_blk);
        let (rv, rt) = self.eval_expr_val(rhs)?;
        if rt != VType::Bool {
            return nope(rhs.span(), "non-bool logic operand");
        }
        self.b.ins().jump(merge, &[rv]);
        self.filled = true;
        self.goto_block(merge);
        let out = self.b.block_params(merge)[0];
        Ok(Some((out, VType::Bool)))
    }

    fn binop_values(
        &mut self,
        op: BinaryOp,
        mut lv: Value,
        mut lt: VType,
        mut rv: Value,
        mut rt: VType,
        span: Span,
    ) -> CResult<(Value, VType)> {
        let opname = match op {
            Add => "+",
            Sub => "-",
            Mul => "*",
            Div => "/",
            Rem => "%",
            Pow => "^",
            Eq => "==",
            NotEq => "!=",
            Lt => "<",
            LtEq => "<=",
            Gt => ">",
            GtEq => ">=",
            And => "and",
            Or => "or",
            Coalesce => "??",
        };
        if lt == VType::I64 && rt == VType::F64 {
            lv = self.b.ins().fcvt_from_sint(types::F64, lv);
            lt = VType::F64;
        }
        if rt == VType::I64 && lt == VType::F64 {
            rv = self.b.ins().fcvt_from_sint(types::F64, rv);
            rt = VType::F64;
        }
        // `Any` is what a value with nothing to say about its element type
        // carries — `None`, an `Err` with no ok half. Comparing it with a
        // side that does know is the same comparison, done in the runtime on
        // boxed values either way: `Some(()) == None` was refused as a
        // "mixed-type operation" for two values of the same type.
        if lt != rt && lt.boxed() && rt.boxed() && (any_inside(&lt) || any_inside(&rt)) {
            let unified = if any_inside(&lt) { rt.clone() } else { lt.clone() };
            lt = unified.clone();
            rt = unified;
        }
        if lt != rt {
            return nope(
                span,
                format!("`{}` between `{}` and `{}`", opname, lt.show(), rt.show()),
            );
        }
        use BinaryOp::*;
        // Tuples compare component by component; the runtime already knows
        // how, because that is what sorts a list of them.
        if let (VType::Tuple(_), Lt | LtEq | Gt | GtEq) = (&lt, op) {
            let ord = self.call_rt("grasm_val_cmp", &[lv, rv]).unwrap();
            self.call_rt("grasm_val_dec", &[lv]);
            self.call_rt("grasm_val_dec", &[rv]);
            let zero = self.b.ins().iconst(types::I64, 0);
            let cc = match op {
                Lt => IntCC::SignedLessThan,
                LtEq => IntCC::SignedLessThanOrEqual,
                Gt => IntCC::SignedGreaterThan,
                _ => IntCC::SignedGreaterThanOrEqual,
            };
            let v = self.b.ins().icmp(cc, ord, zero);
            return Ok((v, VType::Bool));
        }
        // Lists concatenate with `+`, the way strings do.
        if let (VType::List(_), Add) = (&lt, op) {
            let v = self.call_rt("grasm_list_concat", &[lv, rv]).unwrap();
            self.call_rt("grasm_val_dec", &[lv]);
            self.call_rt("grasm_val_dec", &[rv]);
            return Ok((v, lt));
        }
        // Strings.
        if lt == VType::Str {
            let out = match op {
                Add => {
                    let v = self.call_rt("grasm_str_concat", &[lv, rv]).unwrap();
                    (v, VType::Str)
                }
                Eq | NotEq => {
                    let e = self.call_rt("grasm_str_eq", &[lv, rv]).unwrap();
                    let v = if op == NotEq {
                        let one = self.b.ins().iconst(types::I8, 1);
                        self.b.ins().bxor(e, one)
                    } else {
                        e
                    };
                    (v, VType::Bool)
                }
                Lt | LtEq | Gt | GtEq => {
                    let c = self.call_rt("grasm_str_cmp", &[lv, rv]).unwrap();
                    let zero = self.b.ins().iconst(types::I64, 0);
                    let cc = match op {
                        Lt => IntCC::SignedLessThan,
                        LtEq => IntCC::SignedLessThanOrEqual,
                        Gt => IntCC::SignedGreaterThan,
                        GtEq => IntCC::SignedGreaterThanOrEqual,
                        _ => unreachable!(),
                    };
                    (self.b.ins().icmp(cc, c, zero), VType::Bool)
                }
                _ => return nope(span, "this operator on strings"),
            };
            self.call_rt("grasm_val_dec", &[lv]);
            self.call_rt("grasm_val_dec", &[rv]);
            return Ok(out);
        }
        if lt.boxed() {
            // Structural equality for any boxed values.
            if matches!(op, Eq | NotEq) {
                let e = self.call_rt("grasm_val_eq", &[lv, rv]).unwrap();
                let v = if op == NotEq {
                    let one = self.b.ins().iconst(types::I8, 1);
                    self.b.ins().bxor(e, one)
                } else {
                    e
                };
                self.call_rt("grasm_val_dec", &[lv]);
                self.call_rt("grasm_val_dec", &[rv]);
                return Ok((v, VType::Bool));
            }
            return nope(span, "this operator on boxed values");
        }
        let out = match (&lt, op) {
            (VType::I64, Add) => (self.b.ins().iadd(lv, rv), VType::I64),
            (VType::I64, Sub) => (self.b.ins().isub(lv, rv), VType::I64),
            (VType::I64, Mul) => (self.b.ins().imul(lv, rv), VType::I64),
            // `i64::MIN / -1` overflows, and the hardware divide instruction
            // traps on it rather than wrapping — the process died with SIGFPE
            // and no message. Substituting 1 for a -1 divisor avoids the trap;
            // the true result is then the negation, which wraps like every
            // other overflow ([E-ARITH-INT]). Three extra instructions on an
            // operation already tens of cycles deep.
            (VType::I64, Div) => {
                self.guard_zero(rv, "grasm_panic_div_zero");
                let minus_one = self.b.ins().iconst(types::I64, -1);
                let is_neg1 = self.b.ins().icmp(IntCC::Equal, rv, minus_one);
                let one = self.b.ins().iconst(types::I64, 1);
                let safe = self.b.ins().select(is_neg1, one, rv);
                let quotient = self.b.ins().sdiv(lv, safe);
                let negated = self.b.ins().ineg(lv);
                (self.b.ins().select(is_neg1, negated, quotient), VType::I64)
            }
            // `x % -1` is 0 for every x, and `x % 1` is 0 too, so the same
            // substitution gives the right answer with no fixup.
            (VType::I64, Rem) => {
                self.guard_zero(rv, "grasm_panic_rem_zero");
                let minus_one = self.b.ins().iconst(types::I64, -1);
                let is_neg1 = self.b.ins().icmp(IntCC::Equal, rv, minus_one);
                let one = self.b.ins().iconst(types::I64, 1);
                let safe = self.b.ins().select(is_neg1, one, rv);
                (self.b.ins().srem(lv, safe), VType::I64)
            }
            (VType::I64, Pow) => (
                self.call_rt("grasm_pow_i64", &[lv, rv]).unwrap(),
                VType::I64,
            ),
            (VType::F64, Add) => (self.b.ins().fadd(lv, rv), VType::F64),
            (VType::F64, Sub) => (self.b.ins().fsub(lv, rv), VType::F64),
            (VType::F64, Mul) => (self.b.ins().fmul(lv, rv), VType::F64),
            (VType::F64, Div) => (self.b.ins().fdiv(lv, rv), VType::F64),
            // Cranelift has no floating remainder instruction.
            (VType::F64, Rem) => (
                self.call_rt("grasm_f64_rem", &[lv, rv]).unwrap(),
                VType::F64,
            ),
            (VType::F64, Pow) => (
                self.call_rt("grasm_pow_f64", &[lv, rv]).unwrap(),
                VType::F64,
            ),
            (VType::I64, Eq) => (self.b.ins().icmp(IntCC::Equal, lv, rv), VType::Bool),
            (VType::I64, NotEq) => (self.b.ins().icmp(IntCC::NotEqual, lv, rv), VType::Bool),
            (VType::I64, Lt) => (
                self.b.ins().icmp(IntCC::SignedLessThan, lv, rv),
                VType::Bool,
            ),
            (VType::I64, LtEq) => (
                self.b.ins().icmp(IntCC::SignedLessThanOrEqual, lv, rv),
                VType::Bool,
            ),
            (VType::I64, Gt) => (
                self.b.ins().icmp(IntCC::SignedGreaterThan, lv, rv),
                VType::Bool,
            ),
            (VType::I64, GtEq) => (
                self.b.ins().icmp(IntCC::SignedGreaterThanOrEqual, lv, rv),
                VType::Bool,
            ),
            (VType::F64, Eq) => (self.b.ins().fcmp(FloatCC::Equal, lv, rv), VType::Bool),
            (VType::F64, NotEq) => (self.b.ins().fcmp(FloatCC::NotEqual, lv, rv), VType::Bool),
            (VType::F64, Lt) => (self.b.ins().fcmp(FloatCC::LessThan, lv, rv), VType::Bool),
            (VType::F64, LtEq) => (
                self.b.ins().fcmp(FloatCC::LessThanOrEqual, lv, rv),
                VType::Bool,
            ),
            (VType::F64, Gt) => (self.b.ins().fcmp(FloatCC::GreaterThan, lv, rv), VType::Bool),
            (VType::F64, GtEq) => (
                self.b.ins().fcmp(FloatCC::GreaterThanOrEqual, lv, rv),
                VType::Bool,
            ),
            // Unit has one value, so two of them are always equal.
            (VType::Unit, Eq) => {
                let t = self.b.ins().iconst(types::I8, 1);
                (t, VType::Bool)
            }
            (VType::Unit, NotEq) => {
                let f = self.b.ins().iconst(types::I8, 0);
                (f, VType::Bool)
            }
            (VType::Bool, Eq) => (self.b.ins().icmp(IntCC::Equal, lv, rv), VType::Bool),
            (VType::Bool, NotEq) => (self.b.ins().icmp(IntCC::NotEqual, lv, rv), VType::Bool),
            // Two `ref`s are equal when they name the same slot at the same
            // generation — identity, not the values they point at, which is
            // what the interpreter answers and the only thing a reference can
            // usefully be asked. A `ref` is that pair packed into one word, so
            // the comparison is the word.
            //
            // This compiled already for a *list* of refs, because the runtime's
            // deep equality reaches the same conclusion element by element. It
            // was only two bare ones that had nowhere to go.
            (VType::Ref(_), Eq) => (self.b.ins().icmp(IntCC::Equal, lv, rv), VType::Bool),
            (VType::Ref(_), NotEq) => (self.b.ins().icmp(IntCC::NotEqual, lv, rv), VType::Bool),
            _ => return nope(span, format!("`{}` on `{}`", opname, lt.show())),
        };
        Ok(out)
    }

    fn guard_zero(&mut self, divisor: Value, panic_fn: &'static str) {
        let ok_blk = self.b.create_block();
        let bad_blk = self.b.create_block();
        self.b.ins().brif(divisor, ok_blk, &[], bad_blk, &[]);
        self.filled = true;
        self.goto_block(bad_blk);
        self.call_rt(panic_fn, &[]);
        self.trap_here();
        self.filled = true;
        self.goto_block(ok_blk);
    }

    fn eval_if(
        &mut self,
        cond: &Expr,
        then_block: &grasm_syntax::ast::Block,
        else_branch: Option<&ElseBranch>,
    ) -> CResult<Option<(Value, VType)>> {
        let (c, ct) = self.eval_expr_val(cond)?;
        if ct != VType::Bool {
            return nope(cond.span(), "non-bool if condition");
        }
        let then_blk = self.b.create_block();
        let else_blk = self.b.create_block();
        let merge = self.b.create_block();
        self.b.ins().brif(c, then_blk, &[], else_blk, &[]);
        self.filled = true;

        self.goto_block(then_blk);
        let (then_val, mut then_term) = self.eval_block_inner(then_block)?;
        then_term |= self.is_filled();
        // Where the branch's conversion and jump go. The merge type is not
        // known until both branches have been compiled, so the jump cannot be
        // written yet — but leaving the branch's last block unterminated and
        // switching away from it breaks Cranelift's contract ("you have to
        // fill your block before switching"), which a release build does not
        // check and an SSA construction is entitled to mis-seal. A jump to an
        // empty block of its own fills it honestly; the conversion is written
        // into that block afterwards, where the branch's values still
        // dominate.
        let then_fix = if then_term {
            None
        } else {
            let fix = self.b.create_block();
            self.b.ins().jump(fix, &[]);
            self.filled = true;
            Some(fix)
        };

        self.goto_block(else_blk);
        let mut else_term = false;
        let mut else_val: Option<(Value, VType)> = None;
        match else_branch {
            Some(ElseBranch::Else(b)) => {
                let (v, t) = self.eval_block_inner(b)?;
                else_val = v;
                else_term = t;
            }
            Some(ElseBranch::ElseIf(e)) => {
                else_val = self.eval_expr(e)?;
            }
            None => {}
        }
        else_term |= self.is_filled();
        let else_fix = if else_term {
            None
        } else {
            let fix = self.b.create_block();
            self.b.ins().jump(fix, &[]);
            self.filled = true;
            Some(fix)
        };

        // What the whole `if` is worth, decided from the branches themselves
        // rather than from a probe of one of them.
        //
        // Taking it from the *then* branch alone was wrong whenever that
        // branch leaves — `if c { fail(e) } else { v }` in tail position then
        // had no merge parameter at all, so the else branch's value was
        // dropped and the function returned a zero. Probing the else branch
        // instead only moved the problem: the probe is a guess made without
        // compiling anything, and it cannot type a `match` whose arms bind
        // patterns, so `if c { fail(e) } else { match f() { Ok(t) -> Ok(t) …
        // } }` printed `0` in place of the string, and SIGSEGV with an empty
        // stderr one nesting level up. Both branches are compiled first now,
        // and each says what it actually produced.
        let mut merge_ty: Option<VType> = None;
        for (val, term) in [(&then_val, then_term), (&else_val, else_term)] {
            let Some((_, vt)) = val else { continue };
            if term || *vt == VType::Unit {
                continue;
            }
            merge_ty = Some(match merge_ty {
                None => vt.clone(),
                // A branch that is `Err(e)` has no ok half to give, so its
                // type is `() ! E`; the other branch is the one that knows.
                // `xs.map(|x| if p { Err(e) } else { Ok(x) })` produced a list
                // whose elements `??` unboxed as the wrong thing.
                Some(prev) => merge_fallible_halves(prev, vt.clone()),
            });
        }
        if merge_ty.is_none() && else_branch.is_none() {
            // No else: the `if` is a statement, and produces nothing.
        }
        if let Some(vt) = &merge_ty {
            self.b.append_block_param(merge, vt.cl());
        }

        // Now the terminators, in each branch's own landing block.
        if let Some(blk) = then_fix {
            self.goto_block(blk);
            match (&merge_ty, &then_val) {
                (Some(want), Some((v, vt))) => {
                    let mut v = *v;
                    // A fallible function's branch may give the whole `T ! E`
                    // or just the ok half ([T-OK-ERR-RETURN]).
                    // `if b { Err(e) } else { 3 }` jumped a bare integer into
                    // a merge holding a pointer: SIGSEGV, empty stderr.
                    if matches!(want, VType::Fallible(..))
                        && !matches!(vt, VType::Fallible(..))
                    {
                        let boxed = self.box_value(v, vt);
                        v = self.call_rt("grasm_ok_new", &[boxed]).unwrap();
                    } else if *want == VType::F64 && *vt == VType::I64 {
                        v = self.b.ins().fcvt_from_sint(types::F64, v);
                    }
                    self.b.ins().jump(merge, &[v]);
                }
                (Some(want), None) => {
                    // The branch produced nothing where the other produced a
                    // value. Feed the merge a placeholder so the CFG stays
                    // well-formed.
                    let z = match want {
                        VType::F64 => self.b.ins().f64const(0.0),
                        VType::Bool => self.b.ins().iconst(types::I8, 0),
                        other => self.b.ins().iconst(other.cl(), 0),
                    };
                    self.b.ins().jump(merge, &[z]);
                }
                _ => {
                    // Unused boxed then-value must be released.
                    if let Some((v, vt)) = &then_val {
                        if vt.boxed() && merge_ty.is_none() {
                            let v = *v;
                            let vt = vt.clone();
                            self.dec_if_boxed(v, &vt);
                        }
                    }
                    self.b.ins().jump(merge, &[]);
                }
            }
        }
        if let Some(blk) = else_fix {
            self.goto_block(blk);
            match (&merge_ty, &else_val) {
                (Some(want), Some((v, vt))) => {
                    let mut v = *v;
                    // A fallible function's branch may give the whole `T ! E`
                    // or just the ok half ([T-OK-ERR-RETURN]).
                    // `if b { Err(e) } else { 3 }` jumped a bare integer into
                    // a merge holding a pointer: SIGSEGV, empty stderr.
                    if matches!(want, VType::Fallible(..))
                        && !matches!(vt, VType::Fallible(..))
                    {
                        let boxed = self.box_value(v, vt);
                        v = self.call_rt("grasm_ok_new", &[boxed]).unwrap();
                    } else if *want == VType::F64 && *vt == VType::I64 {
                        v = self.b.ins().fcvt_from_sint(types::F64, v);
                    }
                    self.b.ins().jump(merge, &[v]);
                }
                (Some(want), None) => {
                    let z = match want {
                        VType::F64 => self.b.ins().f64const(0.0),
                        VType::Bool => self.b.ins().iconst(types::I8, 0),
                        other => self.b.ins().iconst(other.cl(), 0),
                    };
                    self.b.ins().jump(merge, &[z]);
                }
                _ => {
                    if let Some((v, vt)) = &else_val {
                        if vt.boxed() && merge_ty.is_none() {
                            let v = *v;
                            let vt = vt.clone();
                            self.dec_if_boxed(v, &vt);
                        }
                    }
                    self.b.ins().jump(merge, &[]);
                }
            }
        }
        self.filled = true;
        self.goto_block(merge);
        if let Some(vt) = merge_ty {
            let out = self.b.block_params(merge)[0];
            Ok(Some((out, vt)))
        } else {
            Ok(None)
        }
    }

    fn eval_call(
        &mut self,
        callee: &Expr,
        args: &[Expr],
        _span: Span,
    ) -> CResult<Option<(Value, VType)>> {
        // `x |> utils.helper` desugars to a call whose callee is qualified.
        // That is the same thing as `utils.helper(x)`, which the method path
        // already handles.
        if let Expr::Field { recv, name, span } = callee {
            if let Expr::Ident(tid) = &**recv {
                if self.lookup(&tid.name).is_none()
                    && self
                        .cg
                        .module_imports
                        .get(&self.cg.current_module)
                        .is_some_and(|m| m.contains_key(&tid.name))
                {
                    return self.eval_method(recv, name, args, *span);
                }
            }
        }
        let Expr::Ident(id) = callee else {
            // A callee that is not a name — `TABLE[0](21)`, `(m.get(k) ?? d)(x)`.
            // Evaluated once, bound to a name the source cannot spell, and then
            // called by the path a named function value already uses. Binding
            // it is not ceremony: `call_fn_value` reads the value twice, once
            // for the code pointer and once for the environment, and evaluating
            // the callee twice would run its side effects twice.
            let (fv, ft) = self.eval_expr_val(callee)?;
            let VType::Fn(param_ts, ret_t) = ft.clone() else {
                return nope(callee.span(), "calling a value that is not a function");
            };
            let tmp = self.new_var(&ft);
            self.b.def_var(tmp, fv);
            self.push_scope();
            self.bind(&format!("__callee_{}", callee.span().start), tmp, ft);
            let out = self.call_fn_value(tmp, &param_ts, &ret_t, args, callee.span());
            self.pop_scope_with_decs();
            return out;
        };
        // A local of function type is called indirectly; a local of any other
        // type is not callable at all.
        if let Some((var, vt)) = self.lookup(&id.name) {
            let VType::Fn(param_ts, ret_t) = vt else {
                return nope(id.span, "calling a value that is not a function");
            };
            return self.call_fn_value(var, &param_ts, &ret_t, args, id.span);
        }
        match id.name.as_str() {
            "print" => {
                self.lower_print(&args[0])?;
                return Ok(None);
            }
            // The same lowering, bracketed by a flag the runtime reads
            // ([L-LIB-EPRINT]). One bit of difference, so there is one code
            // path and no second copy of the interpolation logic to drift.
            "eprint" => {
                let on = self.b.ins().iconst(types::I8, 1);
                self.call_rt("grasm_out_to_stderr", &[on]);
                let r = self.lower_print(&args[0]);
                let off = self.b.ins().iconst(types::I8, 0);
                self.call_rt("grasm_out_to_stderr", &[off]);
                r?;
                return Ok(None);
            }
            "assert" => {
                let (c, ct) = self.eval_expr_val(&args[0])?;
                if ct != VType::Bool {
                    return nope(args[0].span(), "non-bool assert");
                }
                self.assert_true(c);
                return Ok(None);
            }
            "assert_eq" => {
                // Boxed rather than reduced to a bool, so the failure can name
                // both values the way the interpreter does. A test that says
                // only "assertion failed" has told you nothing.
                let (a, at) = self.eval_expr_val(&args[0])?;
                let ab = self.box_value(a, &at);
                let (bv, bt) = self.eval_expr_val(&args[1])?;
                let bb = self.box_value(bv, &bt);
                self.call_rt("grasm_assert_eq", &[ab, bb]);
                return Ok(None);
            }
            "panic" => {
                if let Expr::Str(pieces, _) = &args[0] {
                    if let [StrPiece::Text(t)] = pieces.as_slice() {
                        let (p, l) = self.str_ptr_len(t);
                        self.call_rt("grasm_panic_msg", &[p, l]);
                        self.trap_here();
                        self.filled = true;
                        let dead = self.b.create_block();
                        self.goto_block(dead);
                        return Ok(None);
                    }
                }
                // A computed message: build the string and hand it over.
                let (msg, _) = self.eval_expr_val(&args[0])?;
                self.call_rt("grasm_panic_value", &[msg]);
                self.trap_here();
                self.filled = true;
                let dead = self.b.create_block();
                self.goto_block(dead);
                return Ok(None);
            }
            "args" => {
                let v = self.call_rt("grasm_args", &[]).unwrap();
                return Ok(Some((v, VType::List(Box::new(VType::Str)))));
            }
            "input" => {
                let v = self.call_rt("grasm_input", &[]).unwrap();
                return Ok(Some((v, VType::Str)));
            }
            // Guarded on the user's own namespace, the way the checker and
            // the interpreter guard them: `e` is a common enough identifier
            // that a builtin taking it outright would break working programs.
            "pi" | "e"
                if !self
                    .cg
                    .fns
                    .contains_key(&format!("{}::{}", self.cg.current_module, id.name)) =>
            {
                let c = if id.name == "pi" {
                    std::f64::consts::PI
                } else {
                    std::f64::consts::E
                };
                return Ok(Some((self.b.ins().f64const(c), VType::F64)));
            }
            // Tier 2.
            "ref" if !self.cg.fns.contains_key(&format!("{}::ref", self.cg.current_module)) => {
                let (v, vt) = self.eval_expr_val(&args[0])?;
                let boxed = self.box_value(v, &vt);
                let r = self.call_rt("grasm_ref_new", &[boxed]).unwrap();
                return Ok(Some((r, VType::Ref(Box::new(vt)))));
            }
            "release"
                if !self
                    .cg
                    .fns
                    .contains_key(&format!("{}::release", self.cg.current_module)) =>
            {
                let (v, _) = self.eval_expr_val(&args[0])?;
                self.call_rt("grasm_ref_release", &[v]);
                return Ok(None);
            }
            "alive"
                if !self
                    .cg
                    .fns
                    .contains_key(&format!("{}::alive", self.cg.current_module)) =>
            {
                let (v, _) = self.eval_expr_val(&args[0])?;
                let b = self.call_rt("grasm_ref_alive", &[v]).unwrap();
                return Ok(Some((b, VType::Bool)));
            }
            "now_millis" => {
                let v = self.call_rt("grasm_now_millis", &[]).unwrap();
                return Ok(Some((v, VType::I64)));
            }
            "env" => {
                let (nv, _) = self.eval_expr_val(&args[0])?;
                let v = self.call_rt("grasm_env", &[nv]).unwrap();
                self.call_rt("grasm_val_dec", &[nv]);
                return Ok(Some((v, VType::Option(Box::new(VType::Str)))));
            }
            "exit" => {
                let (cv, _) = self.eval_expr_val(&args[0])?;
                self.call_rt("grasm_exit", &[cv]);
                self.trap_here();
                let dead = self.b.create_block();
                self.goto_block(dead);
                return Ok(None);
            }
            // The five `net` builtins. A handle is an `i64` and a payload is a
            // `str`, so nothing here needs a pointer type — which is why the
            // network could be added to the language without adding one.
            "tcp_listen" | "tcp_accept" | "tcp_read" | "tcp_read_bytes"
            | "net_port" | "udp_bind" | "udp_recv" => {
                let (hv, ht) = self.eval_expr_val(&args[0])?;
                let boxed = self.box_value(hv, &ht);
                let (sym, ok_t) = match id.name.as_str() {
                    "tcp_listen" => ("grasm_tcp_listen", VType::I64),
                    "tcp_accept" => ("grasm_tcp_accept", VType::I64),
                    "net_port" => ("grasm_net_port", VType::I64),
                    "udp_bind" => ("grasm_udp_bind", VType::I64),
                    "udp_recv" => ("grasm_udp_recv", VType::Bytes),
                    "tcp_read_bytes" => ("grasm_tcp_read_bytes", VType::Bytes),
                    _ => ("grasm_tcp_read", VType::Str),
                };
                let v = self.call_rt(sym, &[boxed]).unwrap();
                self.call_rt("grasm_val_dec", &[boxed]);
                return Ok(Some((
                    v,
                    VType::Fallible(Box::new(ok_t), Box::new(VType::Sum("IoError".into()))),
                )));
            }
            // Two boxed arguments, a host and a port, and an `i64` handle
            // back — the same shape as `tcp_write` rather than `tcp_listen`.
            "tcp_connect" => {
                let (hostv, hostt) = self.eval_expr_val(&args[0])?;
                let boxed_host = self.box_value(hostv, &hostt);
                let (portv, portt) = self.eval_expr_val(&args[1])?;
                let boxed_port = self.box_value(portv, &portt);
                let v = self
                    .call_rt("grasm_tcp_connect", &[boxed_host, boxed_port])
                    .unwrap();
                self.call_rt("grasm_val_dec", &[boxed_host]);
                self.call_rt("grasm_val_dec", &[boxed_port]);
                return Ok(Some((
                    v,
                    VType::Fallible(
                        Box::new(VType::I64),
                        Box::new(VType::Sum("IoError".into())),
                    ),
                )));
            }
            "proc_pipeline" => {
                let (sv, st) = self.eval_expr_val(&args[0])?;
                let boxed_s = self.box_value(sv, &st);
                let (iv, it) = self.eval_expr_val(&args[1])?;
                let boxed_i = self.box_value(iv, &it);
                let v = self
                    .call_rt("grasm_proc_pipeline", &[boxed_s, boxed_i])
                    .unwrap();
                self.call_rt("grasm_val_dec", &[boxed_s]);
                self.call_rt("grasm_val_dec", &[boxed_i]);
                return Ok(Some((
                    v,
                    VType::Fallible(
                        Box::new(VType::Tuple(vec![VType::I64, VType::Str, VType::Str])),
                        Box::new(VType::Sum("IoError".into())),
                    ),
                )));
            }
            "proc_run" => {
                let (cv, ct) = self.eval_expr_val(&args[0])?;
                let boxed_c = self.box_value(cv, &ct);
                let (av, at) = self.eval_expr_val(&args[1])?;
                let boxed_a = self.box_value(av, &at);
                let v = self.call_rt("grasm_proc_run", &[boxed_c, boxed_a]).unwrap();
                self.call_rt("grasm_val_dec", &[boxed_c]);
                self.call_rt("grasm_val_dec", &[boxed_a]);
                return Ok(Some((
                    v,
                    VType::Fallible(
                        Box::new(VType::Tuple(vec![VType::I64, VType::Str, VType::Str])),
                        Box::new(VType::Sum("IoError".into())),
                    ),
                )));
            }
            // Readiness. The handles arrive as an ordinary `[i64]`, which the
            // backend represents as an *unboxed scalar* list, and the runtime
            // hands one back in the same shape — so the answer can be indexed
            // and iterated like any other list a program built itself.
            "net_wait" => {
                let (hsv, hst) = self.eval_expr_val(&args[0])?;
                let boxed_h = self.box_value(hsv, &hst);
                let (tv, tt) = self.eval_expr_val(&args[1])?;
                let boxed_t = self.box_value(tv, &tt);
                let v = self.call_rt("grasm_net_wait", &[boxed_h, boxed_t]).unwrap();
                self.call_rt("grasm_val_dec", &[boxed_h]);
                self.call_rt("grasm_val_dec", &[boxed_t]);
                return Ok(Some((
                    v,
                    VType::Fallible(
                        Box::new(VType::List(Box::new(VType::I64))),
                        Box::new(VType::Sum("IoError".into())),
                    ),
                )));
            }
            // Four arguments, because a datagram names its destination at send
            // time rather than at connect time.
            "udp_send" => {
                let (hv, ht) = self.eval_expr_val(&args[0])?;
                let boxed_h = self.box_value(hv, &ht);
                let (host, hostt) = self.eval_expr_val(&args[1])?;
                let boxed_host = self.box_value(host, &hostt);
                let (pv, pt) = self.eval_expr_val(&args[2])?;
                let boxed_p = self.box_value(pv, &pt);
                let (dv, _) = self.eval_expr_val(&args[3])?;
                let v = self
                    .call_rt("grasm_udp_send", &[boxed_h, boxed_host, boxed_p, dv])
                    .unwrap();
                self.call_rt("grasm_val_dec", &[boxed_h]);
                self.call_rt("grasm_val_dec", &[boxed_host]);
                self.call_rt("grasm_val_dec", &[boxed_p]);
                self.call_rt("grasm_val_dec", &[dv]);
                return Ok(Some((
                    v,
                    VType::Fallible(Box::new(VType::I64), Box::new(VType::Sum("IoError".into()))),
                )));
            }
            "tcp_write" | "tcp_write_bytes" => {
                let (hv, ht) = self.eval_expr_val(&args[0])?;
                let boxed_h = self.box_value(hv, &ht);
                let (dv, _) = self.eval_expr_val(&args[1])?;
                let sym = if id.name == "tcp_write" {
                    "grasm_tcp_write"
                } else {
                    "grasm_tcp_write_bytes"
                };
                let v = self.call_rt(sym, &[boxed_h, dv]).unwrap();
                self.call_rt("grasm_val_dec", &[boxed_h]);
                self.call_rt("grasm_val_dec", &[dv]);
                return Ok(Some((
                    v,
                    VType::Fallible(Box::new(VType::Unit), Box::new(VType::Sum("IoError".into()))),
                )));
            }
            "net_close" => {
                let (hv, ht) = self.eval_expr_val(&args[0])?;
                let boxed = self.box_value(hv, &ht);
                self.call_rt("grasm_net_close", &[boxed]);
                self.call_rt("grasm_val_dec", &[boxed]);
                return Ok(None);
            }
            "read_file" => {
                let (pv, _) = self.eval_expr_val(&args[0])?;
                let v = self.call_rt("grasm_read_file", &[pv]).unwrap();
                self.call_rt("grasm_val_dec", &[pv]);
                return Ok(Some((v, VType::Fallible(Box::new(VType::Str), Box::new(VType::Sum("IoError".into()))))));
            }
            "write_file" => {
                let (pv, _) = self.eval_expr_val(&args[0])?;
                let (cv, _) = self.eval_expr_val(&args[1])?;
                let v = self.call_rt("grasm_write_file", &[pv, cv]).unwrap();
                self.call_rt("grasm_val_dec", &[pv]);
                self.call_rt("grasm_val_dec", &[cv]);
                return Ok(Some((v, VType::Fallible(Box::new(VType::Unit), Box::new(VType::Sum("IoError".into()))))));
            }
            // [L-LIB-FS]. Both answer with the sorted listing / unit the
            // runtime produces, so the two engines cannot disagree on order.
            "bytes_of" => {
                let (lv, _) = self.eval_expr_val(&args[0])?;
                let v = self.call_rt("grasm_bytes_of", &[lv]).unwrap();
                self.call_rt("grasm_val_dec", &[lv]);
                return Ok(Some((v, VType::Bytes)));
            }
            "random_bytes" => {
                let (nv, nt) = self.eval_expr_val(&args[0])?;
                let boxed = self.box_value(nv, &nt);
                let v = self.call_rt("grasm_random_bytes", &[boxed]).unwrap();
                self.call_rt("grasm_val_dec", &[boxed]);
                return Ok(Some((
                    v,
                    VType::Fallible(Box::new(VType::Bytes), Box::new(VType::Sum("IoError".into()))),
                )));
            }
            "read_bytes" => {
                let (pv, _) = self.eval_expr_val(&args[0])?;
                let v = self.call_rt("grasm_read_bytes", &[pv]).unwrap();
                self.call_rt("grasm_val_dec", &[pv]);
                return Ok(Some((
                    v,
                    VType::Fallible(Box::new(VType::Bytes), Box::new(VType::Sum("IoError".into()))),
                )));
            }
            "write_bytes" => {
                let (pv, _) = self.eval_expr_val(&args[0])?;
                let (bv, _) = self.eval_expr_val(&args[1])?;
                let v = self.call_rt("grasm_write_bytes", &[pv, bv]).unwrap();
                self.call_rt("grasm_val_dec", &[pv]);
                self.call_rt("grasm_val_dec", &[bv]);
                return Ok(Some((
                    v,
                    VType::Fallible(Box::new(VType::Unit), Box::new(VType::Sum("IoError".into()))),
                )));
            }
            "make_dir" | "remove_file" => {
                let (pv, _) = self.eval_expr_val(&args[0])?;
                let sym = if id.name == "make_dir" { "grasm_make_dir" } else { "grasm_remove_file" };
                let v = self.call_rt(sym, &[pv]).unwrap();
                self.call_rt("grasm_val_dec", &[pv]);
                return Ok(Some((
                    v,
                    VType::Fallible(Box::new(VType::Unit), Box::new(VType::Sum("IoError".into()))),
                )));
            }
            "list_dir" => {
                let (pv, _) = self.eval_expr_val(&args[0])?;
                let v = self.call_rt("grasm_list_dir", &[pv]).unwrap();
                self.call_rt("grasm_val_dec", &[pv]);
                return Ok(Some((
                    v,
                    VType::Fallible(
                        Box::new(VType::List(Box::new(VType::Str))),
                        Box::new(VType::Sum("IoError".into())),
                    ),
                )));
            }
            "file_exists" => {
                let (pv, _) = self.eval_expr_val(&args[0])?;
                let v = self.call_rt("grasm_file_exists", &[pv]).unwrap();
                self.call_rt("grasm_val_dec", &[pv]);
                return Ok(Some((v, VType::Bool)));
            }
            "Ok" | "Err" => {
                let (v, vt) = self.eval_expr_val(&args[0])?;
                let boxed = self.box_value(v, &vt);
                let (f, ty) = if id.name == "Ok" {
                    ("grasm_ok_new", VType::Fallible(Box::new(vt), Box::new(VType::Any)))
                } else {
                    ("grasm_err_new", VType::Fallible(Box::new(VType::Any), Box::new(vt)))
                };
                let out = self.call_rt(f, &[boxed]).unwrap();
                return Ok(Some((out, ty)));
            }
            "map" => {
                let m = self.call_rt("grasm_map_new", &[]).unwrap();
                // An empty map has nothing to read a type off, exactly as an
                // empty list does — and the checker already resolved it against
                // the whole body and wrote the answer down ([T-EMPTY-LIT]). It
                // was the list that asked and the map that did not, so
                // `xs.fold(map(), add)` was refused with a message saying the
                // seed and the function differed when they were written the
                // same.
                let ty = self
                    .empty_vtype(_span)
                    .filter(|t| matches!(t, VType::Map(_, _)))
                    .unwrap_or(VType::Map(Box::new(VType::Any), Box::new(VType::Any)));
                return Ok(Some((m, ty)));
            }
            "fail" => {
                let (ev, evt) = self.eval_expr_val(&args[0])?;
                if !evt.boxed() {
                    return nope(args[0].span(), "failing with unboxed values");
                }
                let wrapped = self.call_rt("grasm_err_new", &[ev]).unwrap();
                // Inside a closure that was inlined into a loop, this `fail` is
                // the *closure's* — it produces an `Err` element and the loop
                // goes on. Releasing every scope, as a return does, would free
                // the list being built and the one being walked.
                if let Some((blk, slot, depth)) = self.fail_redirect.last().copied() {
                    self.emit_scope_decs_below(depth);
                    self.b.def_var(slot, wrapped);
                    self.b.ins().jump(blk, &[]);
                    self.filled = true;
                    let dead = self.b.create_block();
                    self.goto_block(dead);
                    return Ok(None);
                }
                self.emit_scope_decs_all();
                if matches!(self.ret, VType::Fallible(_, _)) {
                    self.b.ins().return_(&[wrapped]);
                } else {
                    // `fn main() { fail(e) }` — `main` may propagate without
                    // declaring `! E`, and its signature then returns nothing.
                    // Returning the error value from it anyway failed the
                    // Cranelift verifier and surfaced as "codegen bug",
                    // naming nothing. The same report `try` makes from here.
                    self.call_rt("grasm_report_main_err", &[wrapped]);
                    self.trap_here();
                }
                self.filled = true;
                let dead = self.b.create_block();
                self.goto_block(dead);
                // fail() has bottom type; produce a dummy unit.
                return Ok(None);
            }
            _ => {}
        }
        // Variant constructor call.
        if let Some(key) = self.generic_variant_key(&id.name, args, id.span)? {
            let val = self.build_variant_call(&key, &id.name, args)?;
            return Ok(Some((val, VType::Sum(key))));
        }
        if let Some((parent, arity)) = self.cg.variant_index.get(&id.name).cloned() {
            if args.len() == arity {
                let v = self.build_variant_call(&parent, &id.name, args)?;
                let vt = self.sum_vtype(&parent);
                return Ok(Some((v, vt)));
            }
        }
        if id.name == "Some" && args.len() == 1 {
            let (av, avt) = self.eval_expr_val(&args[0])?;
            let boxed = self.box_value(av, &avt);
            let v = self.make_variant("Option", "Some", &[boxed])?;
            return Ok(Some((v, VType::Option(Box::new(avt)))));
        }
        // A generic function has no symbol until a call site fixes its type
        // parameters, so probe the arguments and instantiate on demand.
        let sig_key = if self.lookup_fn(&id.name).is_none() {
            let generic_key = format!("{}::{}", self.cg.current_module, id.name);
            if self.cg.generic_fns.contains_key(&generic_key) {
                let arg_types = self.generic_arg_types(&generic_key, args)?;
                let module = self.cg.current_module.clone();
                Some(self.cg.instantiate_generic(&module, &id.name, &arg_types, id.span)?)
            } else {
                None
            }
        } else {
            None
        };
        let sig = match &sig_key {
            Some(k) => self.cg.fns.get(k),
            None => self.lookup_fn(&id.name),
        };
        let Some(sig) = sig else {
            // A variant of a *generic* sum. The declaration is kept whole
            // until a use site fixes its parameters, and a bare constructor
            // call does not: `Has(5)` says `T` is `i64`, and threading that
            // through the variant index is not done. Named, because "the
            // function `Has` here" says nothing about which shape was hit.
            for (gkey, decl) in &self.cg.generic_types {
                if let TypeBody::Sum(variants) = &decl.body {
                    if variants.iter().any(|v| v.name.name == id.name) {
                        let base = gkey.rsplit("::").next().unwrap_or(gkey);
                        return nope(
                            id.span,
                            format!(
                                "a variant of the generic sum type `{base}` (`{}` here) — \
generic *structs* compile; a generic sum's constructor does not yet fix its \
type parameters",
                                id.name
                            ),
                        );
                    }
                }
            }
            return nope(id.span, format!("the function `{}` here", id.name));
        };
        let (fid, params, modes, ret, is_extern) = (
            sig.id,
            sig.params.clone(),
            sig.modes.clone(),
            sig.ret.clone(),
            sig.is_extern,
        );
        let ptr_ty = self.cg.module.target_config().pointer_type();
        let mut vals = Vec::with_capacity(args.len());
        // Arguments are evaluated one at a time and their ownership passes to
        // the callee — but only if the call happens. A later argument that
        // diverges (`f(key, try boom())`) leaves the earlier ones owned by
        // nobody: `try` unwinds through `emit_scope_decs_all`, which sees
        // scope bindings and not loose temporaries. So they are bound into a
        // scope of their own, and that scope is dropped *without* releasing
        // them once the call is about to happen.
        self.push_scope();
        // (arg index, stack slot, variable to refresh) for inout write-back.
        let mut inout_args: Vec<(usize, cranelift_codegen::ir::StackSlot, Variable, VType)> =
            Vec::new();
        // (arg index, boxed value, its type, the C pointer) for a C call's
        // pointer arguments, cleaned up after the call returns.
        let mut c_args: Vec<(usize, Value, VType, Value)> = Vec::new();
        // (arg index, slot, root variable, root type, path) for an `inout`
        // argument that is a *place* rather than a plain variable.
        let mut inout_places: Vec<(
            cranelift_codegen::ir::StackSlot,
            Variable,
            VType,
            Vec<PlaceSeg<'_>>,
            VType,
        )> = Vec::new();
        for (i, (a, want)) in args.iter().zip(&params).enumerate() {
            if modes.get(i).copied() == Some(ParamMode::Inout) {
                // `bump(s.xs)` — the argument names a place inside something,
                // not a binding of its own. `grasm run` reads the place, calls,
                // and writes back; this did not, and refused to compile a
                // program the interpreter ran, which is the same read-modify-
                // write the mutating methods already do through a place.
                if !matches!(a, Expr::Ident(_)) {
                    let (root_id, path) = self.place_path(a)?;
                    let Some((var, root_t)) = self.lookup(&root_id.name) else {
                        return nope(root_id.span, "an unknown `inout` argument");
                    };
                    let cur = self.b.use_var(var);
                    // Taken rather than copied: the write-back below puts it
                    // straight back, and a copy would make every `inout`
                    // through a place O(n) in the thing it points at.
                    let held = self.read_place_boxed(cur, &path, &root_t, true)?;
                    // A place holds a *boxed* value; an `inout` parameter is a
                    // pointer to a slot holding what the callee's type is —
                    // a raw integer for `inout n: i64`. Stored boxed, the
                    // callee incremented the address of the box and wrote the
                    // result back into the field, and reading the field then
                    // said `unbox: not an i64`. `grasm run` printed the answer,
                    // so this was a disagreement, not a refusal.
                    let (raw, _) = self.unbox(held, want)?;
                    let slot = self.b.create_sized_stack_slot(
                        cranelift_codegen::ir::StackSlotData::new(
                            cranelift_codegen::ir::StackSlotKind::ExplicitSlot,
                            8,
                            3,
                        ),
                    );
                    self.b.ins().stack_store(raw, slot, 0);
                    let addr = self.b.ins().stack_addr(ptr_ty, slot, 0);
                    vals.push(addr);
                    inout_places.push((slot, var, root_t, path, want.clone()));
                    continue;
                }
                let Expr::Ident(id) = a else {
                    return nope(a.span(), "`inout` arguments that aren't plain variables");
                };
                let Some((var, vt)) = self.lookup(&id.name) else {
                    return nope(id.span, "an unknown `inout` argument");
                };
                let slot = self.b.create_sized_stack_slot(
                    cranelift_codegen::ir::StackSlotData::new(
                        cranelift_codegen::ir::StackSlotKind::ExplicitSlot,
                        8,
                        3,
                    ),
                );
                let cur = self.b.use_var(var);
                self.b.ins().stack_store(cur, slot, 0);
                let addr = self.b.ins().stack_addr(ptr_ty, slot, 0);
                vals.push(addr);
                inout_args.push((i, slot, var, vt));
                continue;
            }
            let (mut v, vt) = self.eval_expr_expecting(a, Some(want))?;
            if *want == VType::F64 && vt == VType::I64 {
                v = self.b.ins().fcvt_from_sint(types::F64, v);
            }
            // A `str` or `bytes` argument to a C function crosses as a pointer
            // ([T-EXTERN-POINTER]). The boxed value stays held in the argument
            // scope so it outlives the call, and — unlike a Grasm callee, which
            // releases what it was handed — nothing on the other side releases
            // anything, so the release happens here afterwards.
            if is_extern && matches!(want, VType::Str | VType::Bytes) {
                let hold = self.new_var(&vt);
                self.b.def_var(hold, v);
                self.bind(&format!(" cptr{i}"), hold, vt.clone());
                let boxed = self.b.use_var(hold);
                let p = match want {
                    // A NUL-terminated copy: Grasm strings carry their length
                    // and C looks for the terminator. Freed after the call.
                    VType::Str => self.call_rt("grasm_str_c_new", &[boxed]).unwrap(),
                    // No copy: the buffer is already contiguous and appending a
                    // NUL would change its length.
                    _ => self.call_rt("grasm_bytes_ptr", &[boxed]).unwrap(),
                };
                c_args.push((i, boxed, want.clone(), p));
                vals.push(p);
                continue;
            }
            // Boxed args transfer ownership to the callee (it decs at exit),
            // but are held in scope until then so a diverging later argument
            // releases them.
            if vt.boxed() {
                let hold = self.new_var(&vt);
                self.b.def_var(hold, v);
                self.bind(&format!(" arg{i}"), hold, vt.clone());
                v = self.b.use_var(hold);
            }
            vals.push(v);
        }
        // The call is happening: ownership passes, so no release here.
        self.pop_scope_no_decs();
        let fref = self.cg.module.declare_func_in_func(fid, self.b.func);
        let inst = self.b.ins().call(fref, &vals);
        let results: Vec<Value> = self.b.inst_results(inst).to_vec();
        for (_, slot, var, vt) in inout_args {
            let updated = self.b.ins().stack_load(vt.cl(), slot, 0);
            self.b.def_var(var, updated);
        }
        // An `inout` place is written back where it came from, the same way a
        // mutating method through a place is.
        for (slot, var, root_t, path, want) in inout_places {
            let updated = self.b.ins().stack_load(want.cl(), slot, 0);
            // Boxed again on the way back, because that is what a place holds.
            let updated = self.box_value(updated, &want);
            let cur = self.b.use_var(var);
            self.begin_write();
            let new_root = self.write_place(id.span, cur, &path, &root_t, updated)?;
            self.b.def_var(var, new_root);
        }
        // The C call is over, so the temporary string copies are freed and the
        // boxed values that backed the pointers are released. A Grasm callee
        // would have done the second half itself; C does not know how.
        for (_, boxed, vt, p) in c_args {
            if vt == VType::Str {
                self.call_rt("grasm_str_c_free", &[p]);
            }
            self.call_rt("grasm_val_dec", &[boxed]);
        }
        if ret == VType::Unit {
            Ok(None)
        } else {
            Ok(Some((results[0], ret)))
        }
    }

    /// Binds a boxed operand into the current scope so an unwind releases it.
    ///
    /// Operands and arguments are evaluated one at a time and their ownership
    /// passes onward only when the operation actually happens. A later operand
    /// that diverges leaves the earlier ones owned by nobody, and `try`
    /// unwinds through `emit_scope_decs_all`, which sees scope bindings rather
    /// than loose temporaries. The caller pops the scope *without* releasing
    /// once the operation is committed.
    fn hold_operand(&mut self, (v, t): (Value, VType)) -> (Value, VType) {
        if !t.boxed() {
            return (v, t);
        }
        let hold = self.new_var(&t);
        self.b.def_var(hold, v);
        let name = format!(" hold{}", self.vars.last().map(|s| s.len()).unwrap_or(0));
        self.bind(&name, hold, t.clone());
        (self.b.use_var(hold), t)
    }

    fn assert_true(&mut self, c: Value) {
        let ok_blk = self.b.create_block();
        let bad_blk = self.b.create_block();
        self.b.ins().brif(c, ok_blk, &[], bad_blk, &[]);
        self.filled = true;
        self.goto_block(bad_blk);
        self.call_rt("grasm_panic_assert", &[]);
        self.trap_here();
        self.filled = true;
        self.goto_block(ok_blk);
    }

    fn lower_print(&mut self, arg: &Expr) -> CResult<()> {
        match arg {
            Expr::Str(pieces, _) => {
                // Every interpolated expression is evaluated before anything
                // is written. Writing each piece as it was computed was one
                // less allocation, and it cost two guarantees: a panic partway
                // along a line left the earlier pieces on stdout, which
                // [E-PANIC-EXIT] says never happens, and a `print` *inside* an
                // interpolation landed in the middle of the outer line rather
                // than before it, which is where the interpreter puts it.
                let mut vals = Vec::new();
                for piece in pieces {
                    if let StrPiece::Expr(inner) = piece {
                        let (v, vt) = self.eval_expr_val(inner)?;
                        if false {
                            return nope(inner.span(), "printing unit");
                        }
                        vals.push((v, vt));
                    }
                }
                let mut next = 0;
                for piece in pieces {
                    match piece {
                        StrPiece::Text(t) => {
                            if !t.is_empty() {
                                let (p, l) = self.str_ptr_len(t);
                                self.call_rt("grasm_out_text", &[p, l]);
                            }
                        }
                        StrPiece::Expr(_) => {
                            let (v, vt) = vals[next].clone();
                            next += 1;
                            match vt {
                                VType::I64 => {
                                    self.call_rt("grasm_out_i64", &[v]);
                                }
                                VType::F64 => {
                                    self.call_rt("grasm_out_f64", &[v]);
                                }
                                VType::Bool => {
                                    self.call_rt("grasm_out_bool", &[v]);
                                }
                                // A reference prints as the slot it names, the
                                // same as in the interpreter. Falling through
                                // to `unreachable!` here made interpolating a
                                // bare `ref` a Rust panic out of the compiler,
                                // which is neither a refusal by name nor at
                                // build time.
                                VType::Ref(_) => {
                                    self.call_rt("grasm_out_ref", &[v]);
                                }
                                ref vt2 if vt2.boxed() => {
                                    self.call_rt("grasm_out_val", &[v]);
                                    self.call_rt("grasm_val_dec", &[v]);
                                }
                                // Unit has one value and renders as `()`.
                                VType::Unit => {
                                    let (p, l) = self.str_ptr_len("()");
                                    self.call_rt("grasm_out_text", &[p, l]);
                                }
                                _ => unreachable!(),
                            }
                        }
                    }
                }
            }
            other => {
                let (v, vt) = self.eval_expr_val(other)?;
                match vt {
                    VType::I64 => {
                        self.call_rt("grasm_out_i64", &[v]);
                    }
                    VType::F64 => {
                        self.call_rt("grasm_out_f64", &[v]);
                    }
                    VType::Bool => {
                        self.call_rt("grasm_out_bool", &[v]);
                    }
                    ref vt2 if vt2.boxed() => {
                        self.call_rt("grasm_out_val", &[v]);
                        self.call_rt("grasm_val_dec", &[v]);
                    }
                    VType::Unit => {
                        let (p, l) = self.str_ptr_len("()");
                        self.call_rt("grasm_out_text", &[p, l]);
                    }
                    _ => unreachable!(),
                }
            }
        }
        self.call_rt("grasm_out_nl", &[]);
        Ok(())
    }

    // ------------------------------------------------------------------
    // N3: variants, structs, match, fallibles
    // ------------------------------------------------------------------

    fn sum_vtype(&self, parent: &str) -> VType {
        if parent == "Option" {
            VType::Option(Box::new(VType::Any))
        } else {
            VType::Sum(parent.to_string())
        }
    }

    /// Where a field sits in its struct's declaration.
    ///
    /// Construction sorts a literal into this order, so the position is fixed
    /// at compile time and a read needs no name comparison.
    fn struct_field_index(&mut self, sname: &str, fname: &str) -> Option<i64> {
        let fields = self.cg.structs.get(sname)?;
        fields
            .iter()
            .position(|(n, _)| n == fname)
            .map(|i| i as i64)
    }

    fn struct_field_type(&mut self, sname: &str, fname: &str, span: Span) -> CResult<VType> {
        let Some(fields) = self.cg.structs.get(sname) else {
            return nope(span, format!("the struct `{sname}`"));
        };
        for (n, t) in fields {
            if n == fname {
                return Ok(t.clone());
            }
        }
        nope(span, format!("the field `{fname}`"))
    }

    fn variant_field_types(&mut self, parent: &str, vname: &str, span: Span) -> CResult<Vec<VType>> {
        if parent == "Option" {
            return Ok(if vname == "Some" {
                vec![VType::Any]
            } else {
                vec![]
            });
        }
        let Some(variants) = self.cg.sums.get(parent) else {
            return nope(span, format!("the sum type `{parent}`"));
        };
        for (n, ts) in variants {
            if n == vname {
                return Ok(ts.clone());
            }
        }
        nope(span, format!("the variant `{vname}`"))
    }

    /// Builds a variant from already-boxed field values (ownership transfers).
    fn make_variant(&mut self, parent: &str, vname: &str, fields: &[Value]) -> CResult<Value> {
        let (tp, tl) = self.str_ptr_len(parent);
        let (vp, vl) = self.str_ptr_len(vname);
        let v = self.call_rt("grasm_variant_new", &[tp, tl, vp, vl]).unwrap();
        for f in fields {
            self.call_rt("grasm_variant_push", &[v, *f]);
        }
        Ok(v)
    }

    fn build_variant_call(&mut self, parent: &str, vname: &str, args: &[Expr]) -> CResult<Value> {
        let span = args.first().map(|a| a.span()).unwrap_or(Span::new(0, 1));
        let field_ts = self.variant_field_types(parent, vname, span)?;
        let mut boxed_fields = Vec::with_capacity(args.len());
        for (a, ft) in args.iter().zip(&field_ts) {
            let (mut av, avt) = self.eval_expr_expecting(a, Some(ft))?;
            if *ft == VType::F64 && avt == VType::I64 {
                av = self.b.ins().fcvt_from_sint(types::F64, av);
            }
            let bx = if *ft == VType::Any {
                self.box_value(av, &avt)
            } else {
                self.box_value(av, ft)
            };
            boxed_fields.push(bx);
        }
        self.make_variant(parent, vname, &boxed_fields)
    }

    /// The instantiated name of a generic struct literal, from the fields it
    /// gives.
    ///
    /// `Pair{ fst: 1, snd: "x" }` is `Pair[i64,str]`. Used both where the
    /// literal is compiled and where it is probed: probing returned the written
    /// name, so a nested literal came back as `Pair` — a family, not a type —
    /// and the enclosing instantiation could not be resolved.
    /// The instantiated sum a generic variant constructor names.
    ///
    /// `Has(5)` is `Opt[i64]`'s. Which member of the family it is comes from
    /// the arguments, exactly as a generic struct literal reads it off the
    /// fields — and for the same reason, that nothing else at this point says.
    ///
    /// `None` when the name is not a generic variant. An error when it is and
    /// the arguments leave a parameter undetermined: `Good(1)` for
    /// `Res[T, E]` says nothing about `E`, and only an annotation can.
    fn generic_variant_key(
        &mut self,
        vname: &str,
        args: &[Expr],
        span: Span,
    ) -> CResult<Option<String>> {
        let Some(gkey) = self.cg.generic_variant_owner.get(vname).cloned() else {
            return Ok(None);
        };
        // The checker's answer first. It reaches the whole program, so a
        // constructor whose parameters a *later* line fixes — `let a = Good(1)`
        // then `take(a)` — is determined there and determined by nothing here.
        if let Some(text) = self.cg.generic_ctor_types.get(&span.start).cloned() {
            if let Some(te) = grasm_syntax::parse_type_src(&text) {
                if let Ok(VType::Sum(key)) = self.cg.vtype_of(&te) {
                    return Ok(Some(key));
                }
            }
        }
        let decl = self.cg.generic_types[&gkey].clone();
        let TypeBody::Sum(variants) = &decl.body else {
            return Ok(None);
        };
        let Some(v) = variants.iter().find(|v| v.name.name == vname) else {
            return Ok(None);
        };
        if v.fields.len() != args.len() {
            return Ok(None);
        }
        let names: std::collections::HashSet<&str> =
            decl.generics.iter().map(|g| g.name.as_str()).collect();
        let mut subst: HashMap<String, VType> = HashMap::new();
        for (f, a) in v.fields.iter().zip(args) {
            if let Ok(t) = self.probe_type(a) {
                unify_generic_with(&f.ty, &t, &names, &mut subst, &self.cg.type_args);
            }
        }
        let mut ats = Vec::with_capacity(decl.generics.len());
        for g in &decl.generics {
            let Some(t) = subst.get(&g.name) else {
                return nope(
                    span,
                    format!(
                        "`{vname}` here — the arguments say nothing about `{}`, so \
annotate the binding",
                        g.name
                    ),
                );
            };
            ats.push(t.clone());
        }
        let base = gkey.rsplit("::").next().unwrap_or(&gkey).to_string();
        let module = self.cg.current_module.clone();
        Ok(Some(self.cg.instantiate_type(&module, &base, &ats, span)?))
    }

    fn generic_struct_key(
        &mut self,
        sname: &str,
        fields: &[(Ident, Expr)],
        span: Span,
    ) -> CResult<Option<String>> {
        let gkey = format!("{}::{}", self.cg.current_module, sname);
        if !self.cg.generic_types.contains_key(&gkey) {
            return Ok(None);
        }
        let decl = self.cg.generic_types[&gkey].clone();
        let TypeBody::Struct(decl_fields) = &decl.body else {
            return nope(span, format!("`{sname}` as a struct literal"));
        };
        let names: std::collections::HashSet<&str> =
            decl.generics.iter().map(|g| g.name.as_str()).collect();
        let mut subst: HashMap<String, VType> = HashMap::new();
        for (fname, fexpr) in fields {
            let Some(df) = decl_fields.iter().find(|d| d.name.name == fname.name) else {
                return nope(fname.span, format!("the field `{}`", fname.name));
            };
            if let Ok(t) = self.probe_type(fexpr) {
                unify_generic(&df.ty, &t, &names, &mut subst);
            }
        }
        let mut ats = Vec::with_capacity(decl.generics.len());
        for g in &decl.generics {
            match subst.get(&g.name) {
                Some(t) => ats.push(t.clone()),
                None => {
                    return nope(
                        span,
                        format!("`{sname}` here — the fields say nothing about `{}`", g.name),
                    )
                }
            }
        }
        let module = self.cg.current_module.clone();
        Ok(Some(self.cg.instantiate_type(&module, sname, &ats, span)?))
    }

    fn eval_struct_lit(
        &mut self,
        name: &Path,
        base: Option<&Expr>,
        fields: &[(Ident, Expr)],
        span: Span,
    ) -> CResult<(Value, VType)> {
        let sname = name.segments[0].name.clone();
        // A handler literal is not a struct, whatever it looks like. Written in
        // a value position — returned, put in a list, stored in a field — it
        // builds a handler value; written at a `with`, it never reaches here at
        // all, because that path lifts it.
        if let Some(eff) = sname.strip_suffix("Handler") {
            if self.cg.effects.contains_key(eff) {
                let eff = eff.to_string();
                let lit = Expr::StructLit {
                    name: name.clone(),
                    base: base.cloned().map(Box::new),
                    fields: fields.to_vec(),
                    span,
                };
                return self.handler_value(&lit, &eff);
            }
        }
        let sname = match self.generic_struct_key(&sname, fields, span)? {
            Some(k) => k,
            None => sname,
        };
        // `Point{ ..p, x: 1 }` — start from a copy of `p` and overwrite. The
        // base is evaluated first, as it is written ([E-ORDER-LTR]).
        let sv = match base {
            Some(b) => {
                let (bv, _) = self.eval_expr_val(b)?;
                bv
            }
            // No base: the value is built in one call once every field has
            // been evaluated, so there is nothing to make yet.
            None => self.b.ins().iconst(types::I64, 0),
        };
        let mut sv = sv;
        // Evaluated in written order ([E-ORDER-LTR]) and *stored* in declared
        // order. Stored as written instead, `P{ z: 1, m: "a" }` and
        // `P{ m: "a", z: 1 }` were two different values: they rendered
        // differently and compared unequal, against [E-EQ-STRUCTURAL]'s "two
        // values of the same shape and contents are equal".
        let mut built: Vec<(String, Value)> = Vec::with_capacity(fields.len());
        for (fname, fexpr) in fields {
            let ft = self.struct_field_type(&sname, &fname.name, span)?;
            let (mut fv, fvt) = self.eval_expr_expecting(fexpr, Some(&ft))?;
            if ft == VType::F64 && fvt == VType::I64 {
                fv = self.b.ins().fcvt_from_sint(types::F64, fv);
            }
            let boxed = self.box_value(fv, &ft);
            if base.is_some() {
                let (fp, fl) = self.str_ptr_len(&fname.name);
                sv = self
                    .call_rt("grasm_struct_set_field", &[sv, fp, fl, boxed])
                    .unwrap();
                continue;
            }
            built.push((fname.name.clone(), boxed));
        }
        if base.is_none() {
            let order: Vec<String> = self
                .cg
                .structs
                .get(&sname)
                .map(|fs| fs.iter().map(|(n, _)| n.clone()).collect())
                .unwrap_or_default();
            built.sort_by_key(|(n, _)| {
                order.iter().position(|d| d == n).unwrap_or(usize::MAX)
            });
            let names: Vec<String> =
                built.iter().map(|(n, _)| n.clone()).collect();
            // The field values go into a stack slot and the whole value is
            // built by one call. A slot of nothing is not a slot, so an empty
            // struct still gets a word; the callee reads none of it.
            let ptr_ty = self.cg.module.target_config().pointer_type();
            let words = built.len().max(1);
            let slot = self.b.create_sized_stack_slot(
                cranelift_codegen::ir::StackSlotData::new(
                    cranelift_codegen::ir::StackSlotKind::ExplicitSlot,
                    (words * 8) as u32,
                    3,
                ),
            );
            for (i, (_, boxed)) in built.iter().enumerate() {
                self.b.ins().stack_store(*boxed, slot, (i * 8) as i32);
            }
            let argv = self.b.ins().stack_addr(ptr_ty, slot, 0);
            // The *written* name, not the instantiated one. `Pair[i64,str]` is
            // how this backend keeps two members of a family apart; it is not
            // what the type is called, and printing one showed
            // `Pair[i64,str]{fst: 1, snd: x}` where every other engine shows
            // `Pair{...}`. The mangling is an implementation detail and has to
            // stop at the value's edge.
            let display_name = self
                .cg
                .type_args
                .get(&sname)
                .map(|(base, _)| base.clone())
                .unwrap_or_else(|| sname.clone());
            let did = self.cg.struct_desc(&display_name, &names);
            let gv = self.cg.module.declare_data_in_func(did, self.b.func);
            let dp = self.b.ins().global_value(ptr_ty, gv);
            sv = self.call_rt("grasm_struct_make", &[dp, argv]).unwrap();
        }
        Ok((sv, VType::Struct(sname)))
    }

    fn eval_field(
        &mut self,
        recv: &Expr,
        name: &Ident,
        span: Span,
    ) -> CResult<Option<(Value, VType)>> {
        // `Type.Variant` nullary through the field path.
        if let Expr::Ident(tid) = recv {
            if self.lookup(&tid.name).is_none()
                && (self.cg.sums.contains_key(&tid.name) || tid.name == "Option")
            {
                let parent = tid.name.clone();
                let v = self.build_variant_call(&parent, &name.name, &[])?;
                let vt = self.sum_vtype(&parent);
                return Ok(Some((v, vt)));
            }
        }
        // `utils.helper` as a value: the same function a qualified call would
        // reach, built as a function value.
        if let Expr::Ident(tid) = recv {
            if self.lookup(&tid.name).is_none() {
                let target = self
                    .cg
                    .module_imports
                    .get(&self.cg.current_module)
                    .and_then(|m| m.get(&tid.name))
                    .cloned();
                if let Some(target_mod) = target {
                    let key = format!("{target_mod}::{}", name.name);
                    // An exported constant is inlined here, in the module that
                    // declared it.
                    if let Some(c) = self.cg.consts.get(&key).cloned() {
                        let saved =
                            std::mem::replace(&mut self.cg.current_module, target_mod.clone());
                        let out = self.eval_expr(&c.init);
                        self.cg.current_module = saved;
                        return out;
                    }
                    if self.cg.fns.contains_key(&key) {
                        return self.make_named_fn_value(&key, span).map(Some);
                    }
                }
            }
        }
        // A field read of a plain local **borrows** its receiver.
        //
        // `p.a` evaluated `p` — which increments — and released it again
        // afterwards, a pair that cancels exactly. The variable holds a
        // reference for the whole of the read, so the count cannot reach zero
        // during it; this is the same argument [E-BORROWED-READ] already makes
        // for a read-only method, applied where it was not.
        //
        // Only a plain identifier. Anything else produced a temporary that this
        // expression owns and must release.
        let (rv, rt, owned) = match self.borrow_struct_path(recv)? {
            Some((v, sname)) => (v, VType::Struct(sname), false),
            None => {
                let (v, t) = self.eval_expr_val(recv)?;
                (v, t, true)
            }
        };
        // Tier 2: reading through a reference goes via the slab, which checks
        // the generation and panics if the slot has been released. The value
        // comes back boxed, so it is unboxed to the pointee's own shape.
        let through_ref = matches!(rt, VType::Ref(_));
        let (rv, rt) = match rt {
            VType::Ref(inner) => {
                let boxed = self.call_rt("grasm_ref_get", &[rv]).unwrap();
                self.unbox(boxed, &inner)?
            }
            other => (rv, other),
        };
        // `.value` reads what a reference points at, when that has no fields of
        // its own ([T-REF-VALUE]). The slab read above already produced it, so
        // there is nothing further to do.
        if through_ref
            && name.name == "value"
            && !matches!(rt, VType::Struct(_) | VType::Tuple(_))
        {
            return Ok(Some((rv, rt)));
        }
        // `pair.0` — a tuple's components are named by position.
        if let VType::Tuple(parts) = &rt {
            let Ok(i) = name.name.parse::<usize>() else {
                return nope(span, "a named field on a tuple");
            };
            let Some(pt) = parts.get(i).cloned() else {
                return nope(span, "a tuple component past the end");
            };
            let idx = self.b.ins().iconst(types::I64, i as i64);
            let boxed = self.call_rt("grasm_list_get", &[rv, idx]).unwrap();
            let (out, out_t) = self.unbox(boxed, &pt)?;
            self.call_rt("grasm_val_dec", &[rv]);
            return Ok(Some((out, out_t)));
        }
        let VType::Struct(sname) = rt else {
            // A UFCS call — `xs.describe()` for a free `fn describe[T]` —
            // reaches here as a field read, so refusing it as "field access
            // on non-structs" named neither the construct nor the fix. §11
            // says a refusal is by name.
            let what = if matches!(rt, VType::List(_) | VType::Map(..) | VType::Str) {
                format!(
                    "a UFCS call of a generic function (`{}` here) — call it as `{}(x)` instead",
                    name.name, name.name
                )
            } else {
                "field access on non-structs".to_string()
            };
            return nope(span, &what);
        };
        let ft = self.struct_field_type(&sname, &name.name, span)?;
        // By position, not by name. Fields are stored in declaration order and
        // the index is known here, so the read is a load instead of a walk
        // comparing strings — the name form falls back only for a struct type
        // this backend somehow has no table for, which the line above would
        // already have refused.
        // A scalar field is read in one call and no reference counting. The
        // general path is four calls — fetch and increment, release the
        // receiver, unbox, release the field — and the increment and decrement
        // on the field cancel exactly, because bits copied out cannot outlive
        // anything.
        if let (Some(i), true) = (
            self.struct_field_index(&sname, &name.name),
            scalar_kind_i64(&ft).is_some(),
        ) {
            let iv = self.b.ins().iconst(types::I64, i);
            let bits = self.call_rt("grasm_struct_scalar_at", &[rv, iv]).unwrap();
            if owned {
                self.call_rt("grasm_val_dec", &[rv]);
            }
            let v = self.scalar_of_bits(bits, &ft);
            return Ok(Some((v, ft)));
        }
        let boxed = match self.struct_field_index(&sname, &name.name) {
            Some(i) => {
                let iv = self.b.ins().iconst(types::I64, i);
                self.call_rt("grasm_struct_get_at", &[rv, iv]).unwrap()
            }
            None => {
                let (fp, fl) = self.str_ptr_len(&name.name);
                self.call_rt("grasm_struct_get", &[rv, fp, fl]).unwrap()
            }
        };
        if owned {
            self.call_rt("grasm_val_dec", &[rv]);
        }
        let (v, vt) = self.unbox(boxed, &ft)?;
        Ok(Some((v, vt)))
    }

    fn eval_try(&mut self, inner: &Expr, span: Span) -> CResult<(Value, VType)> {
        let (fv, ft) = self.eval_expr_val(inner)?;
        let VType::Fallible(ok_t, _) = ft else {
            return nope(span, "try on non-fallible values");
        };
        let ok_blk = self.b.create_block();
        let err_blk = self.b.create_block();
        let c = self.call_rt("grasm_is_ok", &[fv]).unwrap();
        self.b.ins().brif(c, ok_blk, &[], err_blk, &[]);
        self.filled = true;

        self.goto_block(err_blk);
        let zero = self.b.ins().iconst(types::I64, 0);
        let epay = self.call_rt("grasm_variant_field", &[fv, zero]).unwrap();
        self.call_rt("grasm_val_dec", &[fv]);
        let wrapped = self.call_rt("grasm_err_new", &[epay]).unwrap();
        self.emit_scope_decs_all();
        if matches!(self.ret, VType::Fallible(_, _)) {
            self.b.ins().return_(&[wrapped]);
                self.filled = true;
        } else {
            // Checker prevents this; guard anyway.
            self.call_rt("grasm_report_main_err", &[wrapped]);
            self.trap_here();
            self.filled = true;
        }

        self.goto_block(ok_blk);
        let zero2 = self.b.ins().iconst(types::I64, 0);
        let payload = self.call_rt("grasm_variant_field", &[fv, zero2]).unwrap();
        self.call_rt("grasm_val_dec", &[fv]);
        if *ok_t == VType::Unit {
            // `try write_file(..)` — nothing to unwrap.
            self.call_rt("grasm_val_dec", &[payload]);
            let dummy = self.b.ins().iconst(types::I8, 0);
            return Ok((dummy, VType::Unit));
        }
        let (v, vt) = self.unbox(payload, &ok_t)?;
        Ok((v, vt))
    }

    fn eval_coalesce(&mut self, lhs: &Expr, rhs: &Expr, span: Span) -> CResult<(Value, VType)> {
        // `m.get(k) ?? d` is fused into one call that never builds the
        // `Option`. Constructing it costs an `RtVal` *and* a `Vec` for its one
        // field, both freed again by the very next operation — measured at
        // ~85 ns per lookup over two million of them, three times the cost of
        // the lookup itself. Counting loops and table reads with a fallback are
        // among the most common things anyone writes.
        if let Some(fused) = self.fused_map_get_or(lhs, rhs)? {
            return Ok(fused);
        }
        let (lv, lt) = self.eval_expr_val(lhs)?;
        match lt {
            VType::Option(payload_t) => self.coalesce_values(lv, &payload_t, rhs),
            // `T ! E` takes the same fallback: `Ok(v)` yields `v`, `Err(_)`
            // yields the right-hand side. The runtime tag differs from an
            // Option's, so the test does too.
            VType::Fallible(ok_t, _) => self.coalesce_fallible(lv, &ok_t, rhs),
            _ => nope(span, "?? on values that are neither Option nor fallible"),
        }
    }

    /// Recognises `m.get(k) ?? d` and lowers it without building an `Option`.
    ///
    /// Returns `None` when the shape does not match, so every other `??` takes
    /// the ordinary path. The receiver must actually be a map: `??` also applies
    /// to fallible values and to `Option`s from elsewhere.
    ///
    /// The branch is kept. `??` is lazy — the ordinary lowering evaluates the
    /// default only in the `none` block — so an eagerly evaluated default would
    /// change what `m.get(k) ?? panic("missing")` does. The runtime answers with
    /// a null pointer instead of an `Option`, and the branch is on that.
    fn fused_map_get_or(
        &mut self,
        lhs: &Expr,
        rhs: &Expr,
    ) -> CResult<Option<(Value, VType)>> {
        let Expr::MethodCall { recv, name, args, .. } = lhs else {
            return Ok(None);
        };
        if name.name != "get" || args.len() != 1 {
            return Ok(None);
        }
        // The checker's answer first: its type is the real one, and for a map
        // built by `var m = map()` the structural probe below sees only
        // `map()` and learns nothing.
        let from_checker = self
            .cg
            .map_get_value_kind
            .get(&(lhs.span().start, lhs.span().end))
            .and_then(|k| match k {
                0 => Some(VType::I64),
                1 => Some(VType::F64),
                2 => Some(VType::Bool),
                3 => Some(VType::Str),
                4 => Some(VType::Bytes),
                _ => None,
            });
        let out_t = match from_checker {
            Some(t) => t,
            // A receiver this cannot type is not this shape. Propagating the
            // failure made an *optimization recogniser* refuse to compile the
            // program: `Src.get(1) ?? 0` is an effect operation that happens to
            // be called `get` and take one argument, so it reached here, and
            // the probe of `Src` — which is an effect, not a value — became
            // "does not yet support the type of `Src`".
            //
            // Nothing else could have caught it. `match Src.get(1)` compiled,
            // an annotated `let` compiled, and only the fused spelling did not.
            None => match self.probe_type(recv) {
                Ok(VType::Map(_, v)) if *v != VType::Any => *v,
                _ => return Ok(None),
            },
        };

        let (mv, _) = self.eval_expr_val(recv)?;
        let (kv, kt) = self.eval_expr_val(&args[0])?;
        let boxed_k = self.box_value(kv, &kt);
        let found = self
            .call_rt("grasm_map_get_ptr", &[mv, boxed_k])
            .unwrap();
        self.call_rt("grasm_val_dec", &[mv]);
        self.call_rt("grasm_val_dec", &[boxed_k]);

        let present = self.b.create_block();
        let absent = self.b.create_block();
        let merge = self.b.create_block();
        self.b.append_block_param(merge, out_t.cl());
        let null = self.b.ins().iconst(types::I64, 0);
        let is_some = self.b.ins().icmp(IntCC::NotEqual, found, null);
        self.b.ins().brif(is_some, present, &[], absent, &[]);
        self.filled = true;

        self.goto_block(present);
        let (v, _) = self.unbox(found, &out_t)?;
        self.b.ins().jump(merge, &[v]);
        self.filled = true;

        // The default is lowered here and nowhere else, so it runs only when
        // the key was missing.
        //
        // It may also *leave* rather than produce a value — `m.get(k) ?? fail(e)`
        // and `?? return d` are the shape most fallible bodies take — in which
        // case the block is already terminated and nothing reaches the merge.
        // Emitting the jump anyway is a verifier error, which is what
        // `spec/conformance/E-COALESCE-DIVERGE.gr` caught.
        self.goto_block(absent);
        match self.eval_expr_expecting_opt(rhs, Some(&out_t))? {
            Some((mut dv, dvt)) => {
                if out_t == VType::F64 && dvt == VType::I64 {
                    dv = self.b.ins().fcvt_from_sint(types::F64, dv);
                }
                self.b.ins().jump(merge, &[dv]);
                self.filled = true;
            }
            None => {
                if !self.filled {
                    let zero = self.zero_of(&out_t);
                    self.b.ins().jump(merge, &[zero]);
                    self.filled = true;
                }
            }
        }

        self.goto_block(merge);
        let out = self.b.block_params(merge)[0];
        Ok(Some((out, out_t)))
    }

    /// `e ?? d` where `e` is fallible. Consumes the owned value `lv`.
    fn coalesce_fallible(
        &mut self,
        lv: Value,
        ok_t: &VType,
        rhs: &Expr,
    ) -> CResult<(Value, VType)> {
        // A value whose ok half nothing determined — `xs.map(|x| Err(e))`,
        // where the closure never says what a success would hold — takes it
        // from the fallback, which is the only side that knows. Left as
        // `Any`, the merge held a boxed value on one edge and an immediate on
        // the other, and reading it back dereferenced the fallback: a
        // misaligned-pointer abort with nothing on stdout or stderr.
        let ok_t = &match (ok_t, self.probe_type(rhs)) {
            (VType::Any, Ok(t)) if t != VType::Any => t,
            _ => ok_t.clone(),
        };
        let ok_blk = self.b.create_block();
        let err_blk = self.b.create_block();
        let merge = self.b.create_block();
        // `() ! E` carries no value, so the merge takes no parameter. Giving
        // it one anyway produced a Cranelift verifier error, which surfaced as
        // "does not yet support (codegen bug in `main`)" — a refusal naming
        // nothing, which then masked the *real* refusal elsewhere in the same
        // program. `f() ?? ()` is the natural way to say a setup call's
        // failure is deliberately ignored, so this is a shape people write.
        let unit = *ok_t == VType::Unit;
        if !unit {
            self.b.append_block_param(merge, ok_t.cl());
        }
        let c = self.call_rt("grasm_is_ok", &[lv]).unwrap();
        self.b.ins().brif(c, ok_blk, &[], err_blk, &[]);
        self.filled = true;

        self.goto_block(ok_blk);
        let zero = self.b.ins().iconst(types::I64, 0);
        let payload = self.call_rt("grasm_variant_field", &[lv, zero]).unwrap();
        self.call_rt("grasm_val_dec", &[lv]);
        if unit {
            self.call_rt("grasm_val_dec", &[payload]);
            self.b.ins().jump(merge, &[]);
        } else {
            let (v, _) = self.unbox(payload, ok_t)?;
            self.b.ins().jump(merge, &[v]);
        }
        self.filled = true;

        self.goto_block(err_blk);
        self.call_rt("grasm_val_dec", &[lv]);
        // `x ?? fail(e)` and `x ?? return d`: the fallback leaves rather than
        // producing a value, so there is nothing to carry to the merge and the
        // block is already terminated.
        match self.eval_expr_expecting_opt(rhs, Some(ok_t))? {
            Some((mut rv, rvt)) => {
                if unit {
                    // The fallback's value goes nowhere — `()` is written as
                    // an empty tuple, which allocates — so it is released
                    // rather than stranded.
                    if rvt.boxed() {
                        self.call_rt("grasm_val_dec", &[rv]);
                    }
                    self.b.ins().jump(merge, &[]);
                } else {
                    if *ok_t == VType::F64 && rvt == VType::I64 {
                        rv = self.b.ins().fcvt_from_sint(types::F64, rv);
                    }
                    self.b.ins().jump(merge, &[rv]);
                }
                self.filled = true;
            }
            None => {
                if !self.filled {
                    if unit {
                        self.b.ins().jump(merge, &[]);
                    } else {
                        let zero = self.zero_of(ok_t);
                        self.b.ins().jump(merge, &[zero]);
                    }
                    self.filled = true;
                }
            }
        }

        self.goto_block(merge);
        if unit {
            let nothing = self.zero_of(&VType::Unit);
            return Ok((nothing, VType::Unit));
        }
        let out = self.b.block_params(merge)[0];
        Ok((out, ok_t.clone()))
    }

    /// Shared by `??` and `unwrap_or`: consumes the owned Option `lv`.
    fn coalesce_values(
        &mut self,
        lv: Value,
        payload_t: &VType,
        rhs: &Expr,
    ) -> CResult<(Value, VType)> {
        // Determine result type: payload type, or the rhs type if unknown.
        let mut out_t = payload_t.clone();
        if out_t == VType::Any {
            // Probe rhs type cheaply by lowering it in the else branch only;
            // we need the type NOW for the merge param. Structural probe:
            out_t = self.probe_type(rhs)?;
        }
        let some_blk = self.b.create_block();
        let none_blk = self.b.create_block();
        let merge = self.b.create_block();
        self.b.append_block_param(merge, out_t.cl());
        let (sp, sl) = self.str_ptr_len("Some");
        let c = self.call_rt("grasm_variant_is", &[lv, sp, sl]).unwrap();
        self.b.ins().brif(c, some_blk, &[], none_blk, &[]);
        self.filled = true;

        self.goto_block(some_blk);
        let zero = self.b.ins().iconst(types::I64, 0);
        let payload = self.call_rt("grasm_variant_field", &[lv, zero]).unwrap();
        self.call_rt("grasm_val_dec", &[lv]);
        let (v, _) = self.unbox(payload, &out_t)?;
        self.b.ins().jump(merge, &[v]);
        self.filled = true;

        self.goto_block(none_blk);
        self.call_rt("grasm_val_dec", &[lv]);
        // The fallback is evaluated against the Option's payload type, so an
        // empty collection literal — `m.get(k) ?? []`, the natural way to
        // write a default — has the annotation it needs.
        // `m.get(k) ?? fail(e)` and `?? return d`: the fallback may leave
        // rather than produce a value, in which case the block is already
        // terminated and nothing reaches the merge.
        match self.eval_expr_expecting_opt(rhs, Some(&out_t))? {
            Some((mut rv, rvt)) => {
                if out_t == VType::F64 && rvt == VType::I64 {
                    rv = self.b.ins().fcvt_from_sint(types::F64, rv);
                }
                self.b.ins().jump(merge, &[rv]);
                self.filled = true;
            }
            None => {
                if !self.filled {
                    let zero = self.zero_of(&out_t);
                    self.b.ins().jump(merge, &[zero]);
                    self.filled = true;
                }
            }
        }

        self.goto_block(merge);
        let out = self.b.block_params(merge)[0];
        Ok((out, out_t))
    }

    /// Argument types for a call to a generic function.
    ///
    /// A closure argument has no type of its own: its parameter types come
    /// from the parameter it is passed to, and for a generic function that
    /// parameter is not known until the *other* arguments have fixed the type
    /// parameters. So probe what can be probed, solve the type parameters from
    /// those, and read the remaining arguments off the signature.
    fn generic_arg_types(
        &mut self,
        generic_key: &str,
        args: &[Expr],
    ) -> CResult<Vec<VType>> {
        let mut probed: Vec<Option<VType>> = Vec::with_capacity(args.len());
        let mut first_err: Option<Unsupported> = None;
        for a in args {
            match self.probe_type(a) {
                Ok(t) => probed.push(Some(t)),
                Err(e) => {
                    if first_err.is_none() {
                        first_err = Some(e);
                    }
                    probed.push(None);
                }
            }
        }
        if probed.iter().all(Option::is_some) {
            return Ok(probed.into_iter().flatten().collect());
        }
        let Some(decl) = self.cg.generic_fns.get(generic_key).cloned() else {
            return Err(first_err.expect("a None probe recorded an error"));
        };
        let names: std::collections::HashSet<&str> =
            decl.generics.iter().map(|g| g.name.as_str()).collect();
        let mut subst = HashMap::new();
        for (param, actual) in decl.params.iter().zip(&probed) {
            if let Some(a) = actual {
                unify_generic(&param.ty, a, &names, &mut subst);
            }
        }
        let saved = std::mem::replace(&mut self.cg.generic_subst, subst);
        let mut out = Vec::with_capacity(args.len());
        let mut fill_err: Option<Unsupported> = None;
        for (i, probe) in probed.into_iter().enumerate() {
            match probe {
                Some(t) => out.push(t),
                None => match decl.params.get(i).map(|p| self.cg.vtype_of(&p.ty)) {
                    Some(Ok(t)) => out.push(t),
                    _ => {
                        fill_err = Some(first_err.take().expect("recorded"));
                        break;
                    }
                },
            }
        }
        self.cg.generic_subst = saved;
        match fill_err {
            Some(e) => Err(e),
            None => Ok(out),
        }
    }

    /// The type a block evaluates to, with its own `let`/`var` bindings
    /// probed into the current scope so the tail can name them.
    fn probe_block_tail(&mut self, b: &grasm_syntax::ast::Block) -> CResult<VType> {
        // Only the tail decides the type, so nothing else is probed — probing
        // intermediate statements would repeat their side effects on scope for
        // no answer. The bindings are walked, though, because the tail may
        // name them.
        let Some((last, rest)) = b.stmts.split_last() else {
            return Ok(VType::Unit);
        };
        let Stmt::Expr(tail) = last else {
            return Ok(VType::Unit);
        };
        let mut env: HashMap<String, VType> = HashMap::new();
        for st in rest {
            if let Stmt::Let { name, ty, init, .. } = st {
                let t = match ty {
                    Some(te) => self.cg.vtype_of(te)?,
                    // Only shapes whose probe cannot reach an emitting arm.
                    // A probe is meant to be free of side effects and is not
                    // yet provably so; restricting what is probed here keeps
                    // that unproven part out of the path.
                    None if is_probe_safe(init) => {
                        self.probe_types.push(env.clone());
                        let probed = self.probe_type(init);
                        self.probe_types.pop();
                        probed?
                    }
                    None => continue,
                };
                env.insert(name.name.clone(), t);
            }
        }
        self.probe_types.push(env);
        let out = self.probe_type(tail);
        self.probe_types.pop();
        out
    }

    /// The initializer of a module-level constant with this name, if any.
    fn const_of(&self, name: &str) -> Option<Expr> {
        self.cg
            .consts
            .get(&format!("{}::{}", self.cg.current_module, name))
            .map(|c| c.init.clone())
    }

    /// Static type probe without emitting code. Best-effort but covers the
    /// realistic expression shapes; used for merge-block param types.
    fn probe_type(&mut self, e: &Expr) -> CResult<VType> {
        use BinaryOp::*;
        Ok(match e {
            // The checker decided which integer literals are floats, and the
            // probe has to agree with the lowering or a match's merge block
            // gets one type declared and the other jumped to it
            // ([T-INT-TO-FLOAT-CARRIED]).
            Expr::Int(_, sp) if self.cg.float_literals.contains(&sp.start) => VType::F64,
            Expr::Int(..) => VType::I64,
            Expr::Float(..) => VType::F64,
            Expr::Bool(..) => VType::Bool,
            Expr::Str(..) => VType::Str,
            Expr::List(items, span) => VType::List(Box::new(self.list_elem_type(items, *span)?)),
            Expr::Map(entries, _) => match entries.first() {
                Some((k, v)) => VType::Map(
                    Box::new(self.probe_type(k)?),
                    Box::new(self.probe_type(v)?),
                ),
                None => VType::Map(Box::new(VType::Any), Box::new(VType::Any)),
            },
            // A block's type is its tail expression's, and the tail may name
            // the block's own bindings — so those are probed into a scope
            // first. Nothing is emitted; the scope is dropped without decs.
            Expr::Block(b) | Expr::Unsafe(b) | Expr::Region(b) => {
                self.push_scope();
                let probed = self.probe_block_tail(b);
                self.pop_scope_no_decs();
                probed?
            }
            // An `if` is worth the same as whichever branch can be typed.
            // Without this a closure body like
            // `|x| if p { Err(e) } else { Ok(x) }` could not be typed, and
            // `map` built a list whose element type was decided elsewhere —
            // `xs.map(...)[0] ?? d` then read a boxed value as a machine word
            // and took SIGSEGV with an empty stderr.
            Expr::If {
                then_block,
                else_branch,
                ..
            } => {
                self.push_scope();
                let t = self.probe_block_tail(then_block);
                self.pop_scope_no_decs();
                let e = match else_branch.as_deref() {
                    Some(ElseBranch::Else(b)) => {
                        self.push_scope();
                        let e = self.probe_block_tail(b);
                        self.pop_scope_no_decs();
                        e.ok()
                    }
                    Some(ElseBranch::ElseIf(inner)) => self.probe_type(inner).ok(),
                    None => None,
                };
                // Each branch of a fallible `if` knows one half: `Err(e)` has
                // no ok type to give and `Ok(v)` has no error type. Taking the
                // first branch that could be typed made
                // `|n| if c { Err(e) } else { Ok(n) }` probe as `_ ! E`, so a
                // caller that needed the ok half read its own fallback as a
                // pointer — SIGSEGV with an empty stderr.
                match (
                    t.ok().filter(|t| *t != VType::Unit),
                    e.clone().filter(|t| *t != VType::Unit),
                ) {
                    (Some(a), Some(b)) => merge_fallible_halves(a, b),
                    (Some(t), None) => t,
                    (None, Some(t)) => t,
                    (None, None) => e.unwrap_or(VType::Unit),
                }
            }
            Expr::Closure { params, body, span } => {
                // Probing must not lift anything; only the shape is needed.
                let param_types = self.closure_param_types(params, None, *span)?;
                let _ = body;
                // The parameters go into the probe's own environment rather
                // than becoming real variables: a variable declared here and
                // never defined is read as garbage if anything later uses it,
                // which is how probing a `let` init changed what a program
                // computed.
                let env: HashMap<String, VType> = params
                    .iter()
                    .zip(&param_types)
                    .map(|(p, pt)| (p.name.clone(), pt.clone()))
                    .collect();
                self.probe_types.push(env);
                let ret = self.probe_type(body);
                self.probe_types.pop();
                VType::Fn(param_types, Box::new(ret?))
            }
            Expr::Index { recv, span, .. } => match self.probe_type(recv)? {
                VType::List(e) => *e,
                VType::Map(_, v) => VType::Option(v),
                VType::Str => VType::Str,
                VType::Tuple(ts) => ts.first().cloned().unwrap_or(VType::Any),
                other => return nope(*span, format!("indexing into `{}`", other.show())),
            },
            Expr::Unary { op, expr, .. } => match op {
                UnaryOp::Not => VType::Bool,
                UnaryOp::Neg => self.probe_type(expr)?,
            },
            Expr::Binary { op, lhs, rhs, .. } => match op {
                Eq | NotEq | Lt | LtEq | Gt | GtEq | And | Or => VType::Bool,
                Coalesce => {
                    let lt = self.probe_type(lhs)?;
                    if let VType::Option(p) = lt {
                        if *p != VType::Any {
                            *p
                        } else {
                            self.probe_type(rhs)?
                        }
                    } else {
                        self.probe_type(rhs)?
                    }
                }
                _ => {
                    // Arithmetic: float wins the promotion. An empty literal
                    // on either side has no type of its own, so the other
                    // side is what says what the whole thing is.
                    if needs_type_hint(lhs) {
                        return self.probe_type(rhs);
                    }
                    let lt = self.probe_type(lhs)?;
                    if lt == VType::I64 {
                        if let Ok(rt) = self.probe_type(rhs) {
                            if rt == VType::F64 {
                                return Ok(VType::F64);
                            }
                        }
                    }
                    lt
                }
            },
            Expr::Ident(id) if self.lookup(&id.name).is_none() && self.const_of(&id.name).is_some() => {
                let init = self.const_of(&id.name).expect("just checked");
                self.probe_type(&init)?
            }
            Expr::Ident(id) => match self.lookup(&id.name) {
                Some((_, vt)) => vt,
                None if self
                    .probe_types
                    .iter()
                    .rev()
                    .any(|e| e.contains_key(&id.name)) =>
                {
                    self.probe_types
                        .iter()
                        .rev()
                        .find_map(|e| e.get(&id.name))
                        .cloned()
                        .expect("just checked")
                }
                None => {
                    if let Some((parent, 0)) = self.cg.variant_index.get(&id.name).cloned() {
                        self.sum_vtype(&parent)
                    } else if id.name == "None" {
                        VType::Option(Box::new(VType::Any))
                    } else if let Some(sig) = self.lookup_fn(&id.name) {
                        VType::Fn(sig.params.clone(), Box::new(sig.ret.clone()))
                    } else if let Some(eff) = self
                        .cg
                        .handler_lits
                        .get(&id.name)
                        .and_then(|lit| match lit {
                            Expr::StructLit { name, .. } => name.segments[0]
                                .name
                                .strip_suffix("Handler")
                                .map(str::to_string),
                            _ => None,
                        })
                        .filter(|eff| self.cg.effects.contains_key(eff))
                    {
                        VType::Handler(eff)
                    } else {
                        return nope(id.span, format!("the type of `{}`", id.name));
                    }
                }
            },
            Expr::Call { callee, args, .. } => {
                // A variant of a generic sum, in a position where the type has
                // to be worked out rather than read: a list element, say.
                if let Expr::Ident(id) = &**callee {
                    if self.lookup(&id.name).is_none() {
                        if let Some(key) =
                            self.generic_variant_key(&id.name, args, id.span)?
                        {
                            return Ok(VType::Sum(key));
                        }
                    }
                }
                // `Src.get(1)` — an effect operation, which parses as a call
                // through a field and not as a method call. The `MethodCall`
                // arm below already handles the other spelling; this one was
                // missing, so probing the call probed `Src` as if it were a
                // value and refused with "the type of `Src`".
                //
                // It only showed where the probe has to run at all: `match
                // Src.get(1)` and an annotated `let` both compiled, and
                // `Src.get(1) ?? 0` did not.
                if let Expr::Field { recv, name, .. } = &**callee {
                    if let Expr::Ident(tid) = &**recv {
                        if self.lookup(&tid.name).is_none() {
                            if let Some(ops) = self.cg.effects.get(&tid.name).cloned() {
                                return match ops.iter().find(|(o, _, _)| *o == name.name) {
                                    Some((_, _, ret)) => Ok(ret.clone()),
                                    None => nope(
                                        name.span,
                                        format!(
                                            "`{}` is not an operation of effect `{}`",
                                            name.name, tid.name
                                        ),
                                    ),
                                };
                            }
                        }
                    }
                }
                if let Expr::Ident(id) = &**callee {
                    // A local holding a function value shadows a function of
                    // the same name, so it is checked first.
                    if let Some((_, VType::Fn(_, ret))) = self.lookup(&id.name) {
                        return Ok(*ret);
                    }
                    // Builtins that answer with nothing. Their absence here
                    // meant a closure whose body is `print(x)` had no type at
                    // all, so a void callback could not be passed anywhere.
                    if matches!(
                        id.name.as_str(),
                        "print" | "assert" | "assert_eq" | "exit" | "panic" | "release"
                    ) && self.lookup(&id.name).is_none()
                        && self.lookup_fn(&id.name).is_none()
                    {
                        // `release` belongs here too: without it a statement
                        // `match r { A -> release(r)  B(n) -> release(r) }`
                        // was refused as "a match whose arms it cannot type",
                        // advising an annotation that a unit arm has no
                        // spelling for — and freeing a `ref` tree by matching
                        // on its shape is the ordinary way to write it.
                        return Ok(VType::Unit);
                    }
                    // And the ones that answer with something. Every builtin
                    // free function was missing here, and the consequence was
                    // not a refusal: a match none of whose arms could be typed
                    // compiled as a *statement* match, so a fallible function
                    // whose arms were `args()` returned `0`. Silently. The
                    // list is kept beside the one above so the next builtin
                    // added has to pick a side.
                    if self.lookup(&id.name).is_none() && self.lookup_fn(&id.name).is_none() {
                        let t = match id.name.as_str() {
                            "pi" | "e" => Some(VType::F64),
                            "now_millis" => Some(VType::I64),
                            "input" => Some(VType::Str),
                            "file_exists" => Some(VType::Bool),
                            "args" => Some(VType::List(Box::new(VType::Str))),
                            "env" => Some(VType::Option(Box::new(VType::Str))),
                            "tcp_listen" | "tcp_connect" | "tcp_accept" | "net_port"
                            | "udp_bind" | "udp_send" => Some(VType::Fallible(
                                Box::new(VType::I64),
                                Box::new(VType::Sum("IoError".into())),
                            )),
                            "tcp_read" => Some(VType::Fallible(
                                Box::new(VType::Str),
                                Box::new(VType::Sum("IoError".into())),
                            )),
                            "tcp_read_bytes" => Some(VType::Fallible(
                                Box::new(VType::Bytes),
                                Box::new(VType::Sum("IoError".into())),
                            )),
                            "proc_run" | "proc_pipeline" => Some(VType::Fallible(
                                Box::new(VType::Tuple(vec![
                                    VType::I64,
                                    VType::Str,
                                    VType::Str,
                                ])),
                                Box::new(VType::Sum("IoError".into())),
                            )),
                            "udp_recv" => Some(VType::Fallible(
                                Box::new(VType::Bytes),
                                Box::new(VType::Sum("IoError".into())),
                            )),
                            "net_wait" => Some(VType::Fallible(
                                Box::new(VType::List(Box::new(VType::I64))),
                                Box::new(VType::Sum("IoError".into())),
                            )),
                            "tcp_write" | "tcp_write_bytes" => Some(VType::Fallible(
                                Box::new(VType::Unit),
                                Box::new(VType::Sum("IoError".into())),
                            )),
                            "read_file" => Some(VType::Fallible(
                                Box::new(VType::Str),
                                Box::new(VType::Sum("IoError".into())),
                            )),
                            "write_file" | "remove_file" | "make_dir" => Some(VType::Fallible(
                                Box::new(VType::Unit),
                                Box::new(VType::Sum("IoError".into())),
                            )),
                            "bytes_of" => Some(VType::Bytes),
                            "random_bytes" => Some(VType::Fallible(
                                Box::new(VType::Bytes),
                                Box::new(VType::Sum("IoError".into())),
                            )),
                            "read_bytes" => Some(VType::Fallible(
                                Box::new(VType::Bytes),
                                Box::new(VType::Sum("IoError".into())),
                            )),
                            "write_bytes" => Some(VType::Fallible(
                                Box::new(VType::Unit),
                                Box::new(VType::Sum("IoError".into())),
                            )),
                            "list_dir" => Some(VType::Fallible(
                                Box::new(VType::List(Box::new(VType::Str))),
                                Box::new(VType::Sum("IoError".into())),
                            )),
                            _ => None,
                        };
                        if let Some(t) = t {
                            return Ok(t);
                        }
                    }
                    if matches!(id.name.as_str(), "Ok" | "Err")
                        && args.len() == 1
                        && self.lookup(&id.name).is_none()
                    {
                        let inner = self.probe_type(&args[0])?;
                        return Ok(if id.name == "Ok" {
                            VType::Fallible(Box::new(inner), Box::new(VType::Any))
                        } else {
                            VType::Fallible(Box::new(VType::Any), Box::new(inner))
                        });
                    }
                    if let Some(sig) = self.lookup_fn(&id.name) {
                        return Ok(sig.ret.clone());
                    }
                    // A generic call's type is its instantiation's return type,
                    // which means resolving the instantiation here too.
                    let generic_key = format!("{}::{}", self.cg.current_module, id.name);
                    if self.cg.generic_fns.contains_key(&generic_key) {
                        let arg_types = self.generic_arg_types(&generic_key, args)?;
                        let module = self.cg.current_module.clone();
                        let key =
                            self.cg
                                .instantiate_generic(&module, &id.name, &arg_types, id.span)?;
                        return Ok(self.cg.fns[&key].ret.clone());
                    }
                    if let Some((parent, _)) = self.cg.variant_index.get(&id.name).cloned() {
                        return Ok(self.sum_vtype(&parent));
                    }
                    if id.name == "Some" && args.len() == 1 {
                        let inner = self.probe_type(&args[0])?;
                        return Ok(VType::Option(Box::new(inner)));
                    }
                    // `Ok(v)` and `Err(e)` each name one half. Without these
                    // a closure returning them probed as `_ ! _`, so a caller
                    // that had to know the ok half — `??` does, `match` does
                    // not — read the fallback it produced as a pointer and
                    // the program died with an empty stderr.
                    // `ref(v)` — a Tier-2 reference to whatever it is given.
                    // Without this a list literal of them could not be typed,
                    // so `let rs: [ref N] = [ref(N{ v: 3 })]` was refused
                    // while the same list built with `push` compiled.
                    if id.name == "ref"
                        && args.len() == 1
                        && !self
                            .cg
                            .fns
                            .contains_key(&format!("{}::ref", self.cg.current_module))
                    {
                        let inner = self.probe_type(&args[0])?;
                        return Ok(VType::Ref(Box::new(inner)));
                    }
                    if id.name == "Ok" && args.len() == 1 {
                        let inner = self.probe_type(&args[0])?;
                        return Ok(VType::Fallible(
                            Box::new(inner),
                            Box::new(VType::Any),
                        ));
                    }
                    if id.name == "Err" && args.len() == 1 {
                        let inner = self.probe_type(&args[0])?;
                        return Ok(VType::Fallible(
                            Box::new(VType::Any),
                            Box::new(inner),
                        ));
                    }
                }
                let what = match &**callee {
                    Expr::Ident(id) => format!("the result type of `{}`", id.name),
                    Expr::Field { name, .. } | Expr::MethodCall { name, .. } => {
                        format!("the result type of `{}`", name.name)
                    }
                    _ => "the result type of this call".to_string(),
                };
                return nope(e.span(), what);
            }
            Expr::Tuple(items, _) => {
                // `()` is unit, not a zero-length tuple — the same rule the
                // type syntax follows. Probed as an empty tuple it was a
                // boxed pointer, so a generic instantiated at it built a
                // signature taking `i64` while the call site passed the unit
                // value's `i8`: `idg(())` failed the verifier, and the
                // refusal that surfaced said "codegen bug" and named nothing.
                if items.is_empty() {
                    return Ok(VType::Unit);
                }
                let mut ts = Vec::with_capacity(items.len());
                for i in items {
                    ts.push(self.probe_type(i)?);
                }
                VType::Tuple(ts)
            }
            Expr::StructLit { name, fields, .. } => {
                // A handler literal is a handler, not a struct with that name.
                // Probed as a struct, a list of them took `Struct("SHandler")`
                // as its element type, and the `with` that walked the list then
                // refused a handler for not being one.
                let n = name.segments[0].name.clone();
                match n.strip_suffix("Handler") {
                    Some(eff) if self.cg.effects.contains_key(eff) => {
                        VType::Handler(eff.to_string())
                    }
                    _ => match self.generic_struct_key(&n, fields, e.span())? {
                        Some(k) => VType::Struct(k),
                        None => VType::Struct(n),
                    },
                }
            }
            Expr::Field { recv, name, span } => {
                if let Expr::Ident(tid) = &**recv {
                    if self.lookup(&tid.name).is_none() {
                        let target = self
                            .cg
                            .module_imports
                            .get(&self.cg.current_module)
                            .and_then(|m| m.get(&tid.name))
                            .cloned();
                        if let Some(target_mod) = target {
                            let key = format!("{target_mod}::{}", name.name);
                            if let Some(c) = self.cg.consts.get(&key).cloned() {
                                let saved = std::mem::replace(
                                    &mut self.cg.current_module,
                                    target_mod.clone(),
                                );
                                let out = self.probe_type(&c.init);
                                self.cg.current_module = saved;
                                return out;
                            }
                            if let Some(sig) = self.cg.fns.get(&key) {
                                return Ok(VType::Fn(
                                    sig.params.clone(),
                                    Box::new(sig.ret.clone()),
                                ));
                            }
                        }
                    }
                }
                let rt = self.probe_type(recv)?;
                match rt {
                    VType::Struct(sname) => self.struct_field_type(&sname, &name.name, *span)?,
                    // Reading *through* a reference is an ordinary field
                    // read, and compiles — but the probe did not know that,
                    // so `let f = |_u| r.v` was refused for a type it could
                    // work out, while the same read outside a closure built.
                    VType::Ref(inner) => match *inner {
                        VType::Struct(sname) => {
                            self.struct_field_type(&sname, &name.name, *span)?
                        }
                        other => {
                            return nope(
                                *span,
                                format!("reading `.{}` through a `ref {}`", name.name, other.show()),
                            )
                        }
                    },
                    VType::Tuple(parts) => match name.name.parse::<usize>() {
                        Ok(i) if i < parts.len() => parts[i].clone(),
                        _ => {
                            return nope(
                                *span,
                                format!(
                                    "`.{}` on a {}-part tuple",
                                    name.name,
                                    parts.len()
                                ),
                            )
                        }
                    },
                    other => {
                        // `Type.Variant` value probe.
                        if let Expr::Ident(tid) = &**recv {
                            if self.cg.sums.contains_key(&tid.name) {
                                return Ok(self.sum_vtype(&tid.name));
                            }
                        }
                        return nope(
                            *span,
                            format!("`.{}` on `{}`", name.name, other.show()),
                        );
                    }
                }
            }
            // `try e` is the ok half of `e`. Without this a `match` with a
            // `try` arm could not be typed, and the backend refused it as "a
            // match whose arms it cannot type" — about an arm whose type it
            // was one step from working out.
            Expr::Try { expr, .. } => match self.probe_type(expr)? {
                VType::Fallible(ok, _) => *ok,
                other => other,
            },
            Expr::Match { arms, .. } => {
                // Every arm that can say something, for the same reason as an
                // `if`'s two branches: one arm's `Err(e)` and another's
                // `Ok(v)` each know a different half.
                let mut found: Option<VType> = None;
                for arm in arms {
                    // An arm that leaves — `return`, `fail`, `panic` — never
                    // reaches the merge and says nothing about what the match
                    // is worth. Folding it in made an inner
                    // `match x { 0 -> return 7  _ -> x * 3 }` probe as
                    // nothing, so the enclosing function returned whatever
                    // was in the register: `0` where the answer was `9`.
                    if arm_diverges(&arm.body) {
                        continue;
                    }
                    let Ok(t) = self.probe_type(&arm.body) else {
                        continue;
                    };
                    if t == VType::Unit {
                        continue;
                    }
                    found = Some(match found {
                        None => t,
                        Some(prev) => merge_fallible_halves(prev, t),
                    });
                    // Both halves known: no later arm can add anything, and a
                    // long match should not be probed twice over.
                    if let Some(VType::Fallible(ok, err)) = &found {
                        if !matches!(**ok, VType::Any | VType::Unit)
                            && !matches!(**err, VType::Any | VType::Unit)
                        {
                            break;
                        }
                    }
                }
                match found {
                    Some(t) => t,
                    None => {
                        return nope(e.span(), "a `match` whose arms say nothing about its type")
                    }
                }
            }
            // The builtin methods, typed from `spec/library.md`. Anything
            // missing here still compiles wherever the type is already known
            // and fails only where it has to be inferred — as a list element,
            // say — which is a hole that reads as "the backend cannot do this"
            // while looking nothing like it.
            // `Cfg.base()` is an effect operation, not a method: its type is
            // the operation's declared result. Left to the builtin table it
            // was "`.base()` on a receiver whose type is not known here",
            // which named an effect a receiver — and refused the operation
            // inside a list literal while every other position compiled.
            Expr::MethodCall { recv, name, .. }
                if matches!(&**recv, Expr::Ident(tid)
                    if self.lookup(&tid.name).is_none()
                        && self.cg.effects.contains_key(&tid.name)) =>
            {
                let Expr::Ident(tid) = &**recv else {
                    unreachable!("just matched")
                };
                let ops = self.cg.effects.get(&tid.name).cloned().unwrap_or_default();
                match ops.iter().find(|(o, _, _)| *o == name.name) {
                    Some((_, _, ret)) => ret.clone(),
                    None => {
                        return nope(
                            name.span,
                            format!("`{}` is not an operation of effect `{}`", name.name, tid.name),
                        )
                    }
                }
            }
            // `b.f(x)` where `f` is a struct field of function type
            // ([T-FIELD-CALL]). Left to the builtin table it was "`.f()` on a
            // receiver whose type is not known here" — about a receiver whose
            // type a struct literal two lines up had fixed — and refused the
            // ordinary spelling wherever the probe, rather than the compiler,
            // had to work it out.
            Expr::MethodCall { recv, name, .. }
                if matches!(
                    self.probe_type(recv),
                    Ok(VType::Struct(ref sn))
                        if self
                            .cg
                            .structs
                            .get(sn)
                            .is_some_and(|fs| fs.iter().any(|(f, t)| {
                                f == &name.name && matches!(t, VType::Fn(..))
                            }))
                ) =>
            {
                let VType::Struct(sname) = self.probe_type(recv)? else {
                    unreachable!("just matched")
                };
                match self.struct_field_type(&sname, &name.name, e.span())? {
                    VType::Fn(_, ret) => *ret,
                    other => other,
                }
            }
            Expr::MethodCall { recv, name, args, .. } => match name.name.as_str() {
                "len" | "byte_len" => VType::I64,
                "is_nan" | "is_finite" => VType::Bool,
                "is_some" | "is_none" | "is_empty" | "contains" | "starts_with"
                | "ends_with" | "has" | "delete" | "any" | "all" => VType::Bool,
                "to_base" => VType::Str,
                "to_str" | "join" | "trim" | "trim_start" | "trim_end"
                | "to_upper" | "to_lower" | "replace"
                | "substr" | "repeat" | "pad_start" | "pad_end" | "to_char"
                | "to_fixed" => VType::Str,
                "code_at" | "cmp" => VType::I64,
                "codes" => VType::List(Box::new(VType::I64)),
                "as_f64" => VType::F64,
                "as_i64" => VType::I64,
                // Numeric methods. Their absence here did not merely make a
                // program unsupported: inside a fallible function it produced
                // wrong code, because a match whose arms could not be typed
                // fell through to a representation that then unboxed an f64
                // as if it were a pointer.
                "sqrt" | "floor" | "ceil" | "round" => VType::F64,
                "sin" | "cos" | "tan" | "asin" | "acos" | "atan" | "sinh" | "cosh" | "tanh" | "exp" | "ln" | "log2" | "log10" | "cbrt" | "to_radians" | "to_degrees" => VType::F64,
                "atan2" | "hypot" | "log" => VType::F64,
                "abs" => self.probe_type(recv)?,
                // Bit manipulation always answers `i64` ([L-LIB-BITS]).
                // Without this the probe could not type `(v.ushr(8)).bit_and(0xFF)`
                // — a chain that every little-endian field in a binary format
                // is built from.
                "bit_and" | "bit_or" | "bit_xor" | "bit_not" | "shl" | "shr" | "ushr" => {
                    VType::I64
                }
                // Bytes ([L-LIB-BYTES]).
                "to_bytes" => VType::Bytes,
                "at" => VType::I64,
                "concat" => VType::Bytes,
                "chars" | "lines" | "split" | "words" | "keys" => VType::List(Box::new(VType::Str)),
                "entries" => match self.probe_type(recv)? {
                    VType::Map(k, v) => VType::List(Box::new(VType::Tuple(vec![*k, *v]))),
                    _ => return nope(e.span(), format!("`.{}()` on a receiver whose type is not known here", name.name)),
                },
                "values" => match self.probe_type(recv)? {
                    VType::Map(_, v) => VType::List(v),
                    _ => return nope(e.span(), format!("`.{}()` on a receiver whose type is not known here", name.name)),
                },
                "parse_int" => VType::Option(Box::new(VType::I64)),
                "parse_float" => VType::Option(Box::new(VType::F64)),
                "index_of" => VType::Option(Box::new(VType::I64)),
                // Element-wise: the result follows the receiver's elements.
                "sum" => match self.probe_type(recv)? {
                    VType::List(elem) => *elem,
                    _ => return nope(e.span(), format!("`.{}()` on a receiver whose type is not known here", name.name)),
                },
                // `min`/`max` are two methods sharing a name: the reduction
                // over a list yields an Option, the two-argument numeric form
                // yields the receiver's own type.
                "min" | "max" => match self.probe_type(recv)? {
                    VType::List(elem) => VType::Option(elem),
                    n @ (VType::I64 | VType::F64) => n,
                    _ => return nope(e.span(), format!("`.{}()` on a receiver whose type is not known here", name.name)),
                },
                "first" | "last" | "pop" => {
                    match self.probe_type(recv)? {
                        VType::List(elem) => VType::Option(elem),
                        _ => return nope(e.span(), format!("`.{}()` on a receiver whose type is not known here", name.name)),
                    }
                }
                "rev" | "slice" | "filter" | "par_filter" => self.probe_type(recv)?,
                // Mutating methods answer with nothing. `sort` used to probe
                // as the list's own type, which is only harmless while the
                // value is discarded.
                // Mutating methods answer with nothing. `sort` probing as
                // the list's own type is only harmless while the value is
                // discarded.
                "sort" | "sort_by" | "push" | "set" => VType::Unit,
                "remove" => match self.probe_type(recv)? {
                    VType::List(elem) => *elem,
                    _ => return nope(e.span(), format!("`.{}()` on a receiver whose type is not known here", name.name)),
                },
                "enumerate" => match self.probe_type(recv)? {
                    VType::List(elem) => {
                        VType::List(Box::new(VType::Tuple(vec![VType::I64, *elem])))
                    }
                    _ => return nope(e.span(), format!("`.{}()` on a receiver whose type is not known here", name.name)),
                },
                "zip" => match (self.probe_type(recv)?, args.first()) {
                    (VType::List(a), Some(other)) => match self.probe_type(other)? {
                        VType::List(b) => VType::List(Box::new(VType::Tuple(vec![*a, *b]))),
                        _ => return nope(e.span(), format!("`.{}()` on a receiver whose type is not known here", name.name)),
                    },
                    _ => return nope(e.span(), format!("`.{}()` on a receiver whose type is not known here", name.name)),
                },
                "find" => match self.probe_type(recv)? {
                    VType::List(elem) => VType::Option(elem),
                    _ => return nope(e.span(), format!("`.{}()` on a receiver whose type is not known here", name.name)),
                },
                // `map`/`par_map` change the element type, which needs the
                // function's result — only knowable when it is a closure whose
                // body can itself be probed.
                "map" | "par_map" => {
                    let inner = match args.first() {
                        Some(Expr::Closure { params, body, .. }) => {
                            // The closure's parameter is one element of the
                            // receiver. Without binding it the body could not
                            // be typed at all, so `count([1, 2].map(|x| x + 1))`
                            // — a generic applied to a mapped list — was
                            // refused for a type one step away.
                            let elem = match self.probe_type(recv)? {
                                VType::List(t) => Some(*t),
                                _ => None,
                            };
                            match (params.first(), elem) {
                                (Some(p), Some(t)) => {
                                    let env: HashMap<String, VType> =
                                        std::iter::once((p.name.clone(), t)).collect();
                                    self.probe_types.push(env);
                                    let out = self.probe_type(body);
                                    self.probe_types.pop();
                                    out?
                                }
                                _ => self.probe_type(body)?,
                            }
                        }
                        _ => return nope(e.span(), format!("`.{}()` on a receiver whose type is not known here", name.name)),
                    };
                    VType::List(Box::new(inner))
                }
                "get" => {
                    let rt = self.probe_type(recv)?;
                    match rt {
                        VType::List(elem) => VType::Option(elem),
                        VType::Map(_, v) => VType::Option(v),
                        _ => return nope(e.span(), format!("`.{}()` on a receiver whose type is not known here", name.name)),
                    }
                }
                _ => {
                    if let Expr::Ident(tid) = &**recv {
                        if let Some(target) = self
                            .cg
                            .module_imports
                            .get(&self.cg.current_module)
                            .and_then(|m| m.get(&tid.name))
                            .cloned()
                        {
                            if let Some(sig) =
                                self.cg.fns.get(&format!("{target}::{}", name.name))
                            {
                                return Ok(sig.ret.clone());
                            }
                            // `shapes.Circle(..)` — a variant through the alias.
                            if let Some((parent, _)) =
                                self.cg.variant_index.get(&name.name).cloned()
                            {
                                return Ok(self.sum_vtype(&parent));
                            }
                        }
                    }
                    if let Some(sig) = self.lookup_fn(&name.name) {
                        sig.ret.clone()
                    } else if let Expr::Ident(tid) = &**recv {
                        if self.cg.sums.contains_key(&tid.name) {
                            self.sum_vtype(&tid.name)
                        } else {
                            return nope(e.span(), format!("`.{}()` on a receiver whose type is not known here", name.name));
                        }
                    } else {
                        return nope(e.span(), format!("`.{}()` on a receiver whose type is not known here", name.name));
                    }
                }
            },
            other => {
                return nope(e.span(), format!("working out the type of {}", expr_kind(other)))
            }
        })
    }

    fn eval_match(
        &mut self,
        scrutinee: &Expr,
        arms: &[Arm],
        span: Span,
    ) -> CResult<Option<(Value, VType)>> {
        let (sv, st) = self.eval_expr_val(scrutinee)?;
        // Matching through a reference matches what it points at — the same
        // unwrap a field read does, and for the same reason.
        let (sv, st) = match st {
            VType::Ref(inner) => {
                let boxed = self.call_rt("grasm_ref_get", &[sv]).unwrap();
                self.unbox(boxed, &inner)?
            }
            other => (sv, other),
        };
        // The scrutinee lives in a scope of its own, so *every* way out of
        // the match releases it. It used to be released only at the merge
        // block, which an arm that `return`s never reaches — so a match with
        // an early return leaked its scrutinee, and that is the shape a
        // `classify`-style function has.
        self.push_scope();
        // Hold the scrutinee in a variable for repeated tests.
        let svar = self.new_var(&st);
        self.b.def_var(svar, sv);
        // Bound under a name no source can write, so scope release covers it.
        self.bind(" match", svar, st.clone());

        let merge = self.b.create_block();
        // Merge param type discovered from the first value-producing arm; we
        // pre-scan arm bodies structurally via probe where possible. Simpler:
        // decide after lowering first arm — but block params must exist before
        // jumps. Strategy: lower arms into (blocks, value) then wire jumps.
        // Pragmatic approach: probe the first arm body type.
        // Probe the result type. Pattern binds aren't live yet, so bind their
        // *types* in a scratch scope first, and try each arm until one probes.
        let mut out_t: Option<VType> = None;
        // "No arm could be typed" and "every arm yields nothing" used to be
        // the same answer here — both left `out_t` as `None`, which compiles a
        // statement match with no merge parameter. So a *value* match whose
        // arms the probe did not understand silently became a match that
        // produces no value, and the enclosing function returned whatever
        // happened to be there: `fn f() -> [str] ! str { match p { 0 -> args()
        // _ -> args() } }` returned `0`. Not a panic, not a refusal — a wrong
        // answer. These are now distinguished, and the first one refuses.
        let mut probed_any = false;
        let mut live_arms = false;
        for arm in arms {
            // An arm that leaves the function (or the loop) never reaches the
            // merge, so it says nothing about what the match is worth. Reading
            // it as "yields nothing" made `0 -> return false` turn the whole
            // match into a statement, and the function then had no value to
            // return.
            if arm_diverges(&arm.body) {
                continue;
            }
            live_arms = true;
            self.push_scope();
            let bound = self.bind_pattern_types(&arm.pattern, &st).is_ok();
            let probed = if bound {
                self.probe_type(&arm.body).ok()
            } else {
                None
            };
            self.pop_scope_no_decs();
            match probed {
                // Every arm that can say something, not the first one that
                // can. `match n { 1 -> 10  2 -> Ok(n * 3)  _ -> Err("no") }`
                // took `i64` from the first arm, so the other two jumped a
                // `T ! E` pointer into a merge holding a machine word and the
                // caller printed a heap address where the number was.
                Some(t) if t != VType::Unit => {
                    probed_any = true;
                    out_t = Some(match out_t {
                        None => t,
                        Some(prev) => merge_fallible_halves(prev, t),
                    });
                    // Once both halves of a `T ! E` are known no later arm can
                    // add anything, and probing the rest of a long match is
                    // work with no answer in it.
                    if let Some(VType::Fallible(ok, err)) = &out_t {
                        if !matches!(**ok, VType::Any | VType::Unit)
                            && !matches!(**err, VType::Any | VType::Unit)
                        {
                            break;
                        }
                    }
                }
                // An arm that yields nothing (e.g. `-> print(..)`) makes this
                // a statement match: no merge parameter, no value. Only while
                // no arm has yielded a value — one that has is the checker's
                // business, and dropping the value here is not the answer.
                Some(_) => {
                    if out_t.is_none() {
                        probed_any = true;
                        break;
                    }
                }
                None => continue,
            }
        }
        // A written type on the binding this match initialises says what the
        // arms are worth when none of them can say it themselves — which is
        // exactly the remedy §11's table names, and it did not work:
        // `let m: Map[str, i64] = match p { 0 -> map()  _ -> map() }` was
        // refused with its own annotation in view.
        if live_arms && !probed_any {
            if let Some(hint) = self.tail_hint.clone() {
                if hint != VType::Unit {
                    out_t = Some(hint);
                    probed_any = true;
                }
            }
        }
        if live_arms && !probed_any {
            return nope(
                span,
                "a match here whose arms it cannot type (annotate one of them, or lift it into a `let` with a type)",
            );
        }
        // A value-match needs EVERY arm to yield a value; if any arm is a bare
        // statement call, fall back to statement form.
        if out_t.is_some() {
            for arm in arms {
                if arm_diverges(&arm.body) {
                    continue;
                }
                if matches!(&arm.body, Expr::Call { callee, .. }
                    if matches!(&**callee, Expr::Ident(id)
                        if id.name == "print" || id.name == "assert"
                            || id.name == "assert_eq"))
                {
                    out_t = None;
                    break;
                }
            }
        }
        if let Some(t) = &out_t {
            self.b.append_block_param(merge, t.cl());
        }

        let mut next_test = self.b.create_block();
        self.b.ins().jump(next_test, &[]);
        self.filled = true;

        for (ai, arm) in arms.iter().enumerate() {
            self.goto_block(next_test);
            next_test = self.b.create_block();
            let body_blk = self.b.create_block();
            let is_last = ai + 1 == arms.len();

            self.push_scope();
            // Pattern test + binds.
            let matched = self.lower_pattern_test(&arm.pattern, svar, &st, body_blk, next_test)?;
            let _ = matched;

            self.goto_block(body_blk);
            if let Some(guard) = &arm.guard {
                let (gv, gt) = self.eval_expr_val(guard)?;
                if gt != VType::Bool {
                    return nope(guard.span(), "non-bool guards");
                }
                let cont_blk = self.b.create_block();
                // Guard false: release binds, try next arm.
                let cleanup = self.b.create_block();
                self.b.ins().brif(gv, cont_blk, &[], cleanup, &[]);
                self.filled = true;
                self.goto_block(cleanup);
                self.release_current_scope_binds();
                self.b.ins().jump(next_test, &[]);
                self.filled = true;
                self.goto_block(cont_blk);
            }
            // The match's own result type is what an arm's empty collection
            // literal takes its element type from — `match n { 0 -> [] ... }`
            // has nothing else to go on.
            let bv = match &out_t {
                Some(t) if needs_type_hint(&arm.body) => {
                    self.eval_expr_expecting(&arm.body, Some(&t.clone())).map(Some)?
                }
                _ => self.eval_expr(&arm.body)?,
            };
            // An arm body may itself terminate (`-> break`, `-> return x`).
            let bterm = self.is_filled();
            if !bterm {
                match (&out_t, bv) {
                    (Some(want), Some((mut v, vt))) => {
                        // An arm of a fallible function may give the whole
                        // `T ! E` or just the ok half ([T-OK-ERR-RETURN]),
                        // exactly as an `if` branch may.
                        if matches!(want, VType::Fallible(..))
                            && !matches!(vt, VType::Fallible(..))
                        {
                            let boxed = self.box_value(v, &vt);
                            v = self.call_rt("grasm_ok_new", &[boxed]).unwrap();
                        } else if *want == VType::F64 && vt == VType::I64 {
                            v = self.b.ins().fcvt_from_sint(types::F64, v);
                        }
                        self.pop_scope_with_decs();
                        self.b.ins().jump(merge, &[v]);
                        self.filled = true;
                    }
                    (None, maybe) => {
                        if let Some((v, vt)) = maybe {
                            self.dec_if_boxed(v, &vt);
                        }
                        self.pop_scope_with_decs();
                        self.b.ins().jump(merge, &[]);
                        self.filled = true;
                    }
                    (Some(want), None) => {
                        // Defensive: emit a zero of the merge type so the CFG
                        // stays well-formed (checker rejects real mismatches).
                        let z = match want {
                            VType::F64 => self.b.ins().f64const(0.0),
                            VType::Bool => self.b.ins().iconst(types::I8, 0),
                            other => self.b.ins().iconst(other.cl(), 0),
                        };
                        self.pop_scope_with_decs();
                        self.b.ins().jump(merge, &[z]);
                        self.filled = true;
                    }
                }
            } else {
                self.pop_scope_no_decs();
            }

            if is_last {
                // Exhaustive per checker: the final next_test is unreachable.
                self.goto_block(next_test);
                self.call_rt("grasm_panic_assert", &[]);
                self.trap_here();
                self.filled = true;
            }
        }

        self.goto_block(merge);
        // Released by popping the scope it was bound into, so a `return` out
        // of an arm — which calls `emit_scope_decs_all` and never arrives
        // here — releases it too.
        self.pop_scope_with_decs();
        if let Some(t) = out_t {
            let out = self.b.block_params(merge)[0];
            let _ = span;
            Ok(Some((out, t)))
        } else {
            Ok(None)
        }
    }

    fn release_current_scope_binds(&mut self) {
        let binds: Vec<(Variable, VType)> = self
            .vars
            .last()
            .map(|s| s.values().cloned().collect())
            .unwrap_or_default();
        for (var, vt) in binds {
            if vt.boxed() {
                let v = self.b.use_var(var);
                self.call_rt("grasm_val_dec", &[v]);
            }
        }
    }

    /// Splits a place expression into its root variable and access path.
    /// Evaluates a place path's index expressions now, so everything that
    /// walks the path afterwards reuses the values.
    ///
    /// A mutating method reads its receiver and writes it back, each walking
    /// the path — so `g[f()].push(v)` called `f` twice, and once more for the
    /// argument's own walk. It also has to happen *before* the argument is
    /// evaluated, since the receiver is written first ([E-ORDER-LTR]).
    fn pin_place_indices(&mut self, path: &[PlaceSeg<'_>]) -> CResult<()> {
        for seg in path {
            if let PlaceSeg::Index(ix) = seg {
                let key = ix.span().start;
                if !self.index_vals.contains_key(&key) {
                    let v = self.eval_expr_val(ix)?;
                    self.index_vals.insert(key, v);
                }
            }
        }
        Ok(())
    }

    /// An index expression's value, taken from the pre-evaluated cache when
    /// this is an assignment target.
    fn index_value(&mut self, ix: &Expr) -> CResult<(Value, VType)> {
        if let Some(v) = self.index_vals.get(&ix.span().start) {
            return Ok(v.clone());
        }
        self.eval_expr_val(ix)
    }

    /// Whether the write just performed landed in the slab through a
    /// reference, so no enclosing container needs updating. Set by
    /// `write_place`, read by the level above it, and cleared before each
    /// top-level write.
    fn begin_write(&mut self) {
        self.wrote_through_ref = false;
    }

    fn place_path<'e>(&mut self, e: &'e Expr) -> CResult<(&'e Ident, Vec<PlaceSeg<'e>>)> {
        let mut segs: Vec<PlaceSeg<'e>> = Vec::new();
        let mut cur = e;
        loop {
            match cur {
                Expr::Field { recv, name, .. } => {
                    segs.push(PlaceSeg::Field(&name.name));
                    cur = recv;
                }
                Expr::Index { recv, index, .. } => {
                    segs.push(PlaceSeg::Index(index));
                    cur = recv;
                }
                Expr::Ident(id) => {
                    segs.reverse();
                    return Ok((id, segs));
                }
                other => return nope(other.span(), "this assignment target"),
            }
        }
    }

    /// Static type of the value a place path designates.
    fn place_type(&mut self, root_t: &VType, path: &[PlaceSeg<'_>], span: Span) -> CResult<VType> {
        let mut t = root_t.clone();
        for seg in path {
            // A reference reads as the value it points at ([T-REF-READ]), and
            // it consumes no segment of the path: `h.r.v` is the field `v` of
            // what `h.r` points at.
            let mut was_ref = false;
            while let VType::Ref(inner) = &t {
                t = (**inner).clone();
                was_ref = true;
            }
            // `.value` names the referent itself ([T-REF-VALUE]), so it
            // designates the type the dereference just produced.
            if was_ref
                && matches!(seg, PlaceSeg::Field("value"))
                && !matches!(t, VType::Struct(_) | VType::Tuple(_))
            {
                continue;
            }
            t = match (&t, seg) {
                (VType::Struct(sname), PlaceSeg::Field(f)) => {
                    let sname = sname.clone();
                    self.struct_field_type(&sname, f, span)?
                }
                (VType::List(e), PlaceSeg::Index(_)) => (**e).clone(),
                _ => return nope(span, place_shape(&t, seg)),
            };
        }
        Ok(t)
    }

    /// Reads the (boxed-then-unboxed) value at a place, for compound assigns.
    fn read_place(
        &mut self,
        root: Value,
        path: &[PlaceSeg<'_>],
        root_t: &VType,
        elem_t: &VType,
    ) -> CResult<Value> {
        let mut cur = root;
        let mut cur_t = root_t.clone();
        // The accessors *increment* what they hand back, so an intermediate
        // is released once the walk has moved past it — the comment here used
        // to say they borrow, and they do not: `d[0][1] += 1` leaked the
        // inner list on every execution.
        let mut owned = false;
        for (i, seg) in path.iter().enumerate() {
            let is_last = i + 1 == path.len();
            // A reference reads as what it points at and consumes no segment.
            // Before `prev` is taken, because `grasm_ref_get` increments what
            // it hands back: recorded as owned, the walk releases it a step
            // later like any other intermediate. Left out of that bookkeeping
            // it leaked the referent on every `r.items.push(x)`.
            while let VType::Ref(inner) = &cur_t {
                let inner = (**inner).clone();
                // Raw at the root, boxed once a getter has handed it over.
                let raw = if owned {
                    let r = self.call_rt("grasm_unbox_i64", &[cur]).unwrap();
                    self.call_rt("grasm_val_dec", &[cur]);
                    r
                } else {
                    cur
                };
                cur = self.call_rt("grasm_ref_get", &[raw]).unwrap();
                cur_t = inner;
                owned = true;
            }
            // `.value` names the referent itself, which the dereference above
            // already produced ([T-REF-VALUE]) — so the segment consumes no
            // accessor. As the last one it *is* the answer, unboxed the same
            // way any other final step is.
            if matches!(seg, PlaceSeg::Field("value"))
                && !matches!(cur_t, VType::Struct(_) | VType::Tuple(_))
            {
                if is_last {
                    let (v, _) = self.unbox(cur, elem_t)?;
                    if elem_t.boxed() {
                        self.call_rt("grasm_val_dec", &[cur]);
                    }
                    return Ok(v);
                }
                continue;
            }
            let prev = cur;
            let prev_owned = owned;
            let prev_t = cur_t.clone();
            let boxed = match (&cur_t, seg) {
                (VType::Struct(sname), PlaceSeg::Field(f)) => {
                    let sname = sname.clone();
                    let ft = self.struct_field_type(&sname, f, Span::new(0, 1))?;
                    let (fp, fl) = self.str_ptr_len(f);
                    let v = self.call_rt("grasm_struct_get", &[cur, fp, fl]).unwrap();
                    cur_t = ft;
                    v
                }
                (VType::List(e), PlaceSeg::Index(ix)) => {
                    let et = (**e).clone();
                    let (iv, _) = self.index_value(ix)?;
                    let v = self.call_rt("grasm_list_get", &[cur, iv]).unwrap();
                    cur_t = et;
                    v
                }
                _ => return nope(Span::new(0, 1), "this assignment target's shape"),
            };
            owned = true;
            if prev_owned && prev_t.boxed() {
                self.call_rt("grasm_val_dec", &[prev]);
            }
            if is_last {
                let (v, _) = self.unbox(boxed, elem_t)?;
                if elem_t.boxed() {
                    // The unboxed value is the caller's; the box it came from
                    // is this walk's and is finished with.
                    self.call_rt("grasm_val_dec", &[boxed]);
                }
                return Ok(v);
            }
            cur = boxed;
        }
        nope(Span::new(0, 1), "an empty assignment path")
    }

    /// Reads a place without unboxing the final value (for container places).
    /// Calls the taking accessor, falling back to the copying one when it
    /// answers null — which is how the runtime says the parent is shared.
    fn take_or_get(
        &mut self,
        take_fn: &'static str,
        get_fn: &'static str,
        args: &[Value],
        take: bool,
    ) -> Value {
        if !take {
            return self.call_rt(get_fn, args).unwrap();
        }
        let took = self.call_rt(take_fn, args).unwrap();
        let zero = self.b.ins().iconst(types::I64, 0);
        let is_null = self.b.ins().icmp(IntCC::Equal, took, zero);
        let copy_blk = self.b.create_block();
        let done = self.b.create_block();
        let ptr_ty = self.cg.module.target_config().pointer_type();
        self.b.append_block_param(done, ptr_ty);
        self.b.ins().brif(is_null, copy_blk, &[], done, &[took]);
        self.filled = true;
        self.goto_block(copy_blk);
        let got = self.call_rt(get_fn, args).unwrap();
        self.b.ins().jump(done, &[got]);
        self.filled = true;
        self.goto_block(done);
        self.b.block_params(done)[0]
    }

    /// Reads the value at a place.
    ///
    /// With `take`, each step *moves* the child out of its parent instead of
    /// copying a reference to it, so the caller is the sole owner and a
    /// mutation happens where the value lies. The value is put back by
    /// `write_place`, and the runtime recognises a value returning to the
    /// slot it came from. Without this `s.xs.push(i)` was O(n) per push — the
    /// read incremented the field's count, and the push then found the list
    /// shared and copied it. Taking is only possible when the parent is
    /// unshared; the runtime answers null when it is not, and the copying
    /// accessor runs instead.
    fn read_place_boxed(
        &mut self,
        root: Value,
        path: &[PlaceSeg<'_>],
        root_t: &VType,
        take: bool,
    ) -> CResult<Value> {
        let mut cur = root;
        let mut cur_t = root_t.clone();
        // Whether `cur` is a reference this function took. The root is the
        // caller's — borrowed — and everything after it comes from an
        // accessor that increments.
        let mut owned = false;
        for (i, seg) in path.iter().enumerate() {
            // Only the last step may take. `write_place` walks the path again
            // from the root, so an intermediate moved out of its parent would
            // leave a placeholder there for that walk to find —
            // `rows[0].stack.pop()` reached `set_field` with a `bool` where
            // the struct used to be. The last step's parent is re-read by
            // that walk and still holds the placeholder, which is exactly
            // what the write replaces.
            let take = take && i + 1 == path.len();
            // As in `read_place`: dereference before `prev` is taken, and
            // record the result as owned so it is released a step later.
            while let VType::Ref(inner) = &cur_t {
                let inner = (**inner).clone();
                let raw = if owned {
                    let r = self.call_rt("grasm_unbox_i64", &[cur]).unwrap();
                    self.call_rt("grasm_val_dec", &[cur]);
                    r
                } else {
                    cur
                };
                cur = self.call_rt("grasm_ref_get", &[raw]).unwrap();
                cur_t = inner;
                owned = true;
            }
            // `.value` names the referent itself, which the dereference above
            // already produced ([T-REF-VALUE]).
            if matches!(seg, PlaceSeg::Field("value"))
                && !matches!(cur_t, VType::Struct(_) | VType::Tuple(_))
            {
                continue;
            }
            let prev = cur;
            let prev_owned = owned;
            let prev_t = cur_t.clone();
            match (&cur_t, seg) {
                (VType::Struct(sname), PlaceSeg::Field(f)) => {
                    let sname = sname.clone();
                    let ft = self.struct_field_type(&sname, f, Span::new(0, 1))?;
                    let (fp, fl) = self.str_ptr_len(f);
                    cur = self.take_or_get(
                        "grasm_struct_take",
                        "grasm_struct_get",
                        &[cur, fp, fl],
                        take,
                    );
                    cur_t = ft;
                }
                (VType::List(e), PlaceSeg::Index(ix)) => {
                    let et = (**e).clone();
                    let (iv, _) = self.index_value(ix)?;
                    cur = self.take_or_get(
                        "grasm_list_take",
                        "grasm_list_get",
                        &[cur, iv],
                        take,
                    );
                    cur_t = et;
                }
                _ => return nope(Span::new(0, 1), "this place's shape"),
            }
            owned = true;
            // The intermediate we just walked past is still held by its own
            // parent, and this function's reference to it is finished with —
            // only the last one is handed to the caller. Nothing released
            // them, so every mutating method through a multi-segment place
            // (`rows[0].stack.sort()`) leaked one per intermediate.
            if prev_owned && prev_t.boxed() {
                self.call_rt("grasm_val_dec", &[prev]);
            }
        }
        Ok(cur)
    }

    /// Writes `boxed_val` at the place, rebuilding containers copy-on-write.
    /// Returns the (possibly new) root value.
    fn write_place(
        &mut self,
        span: Span,
        root: Value,
        path: &[PlaceSeg<'_>],
        root_t: &VType,
        boxed_val: Value,
    ) -> CResult<Value> {
        // A write whose path passes through a reference has to land in the
        // slab, not in a rebuilt copy: every other holder of that reference
        // must see it ([T-REF-GENERATIONAL]). The reference consumes no
        // segment — it reads as the value it points at — so the whole
        // remaining path is applied to the referent, and the reference itself
        // is what the caller stores back, unchanged.
        if let VType::Ref(inner) = root_t {
            let inner = (**inner).clone();
            if path.is_empty() {
                // `r = ref(...)` — rebinding the reference itself.
                return Ok(boxed_val);
            }
            // `root` is a local's own value here, which for a reference is
            // the packed slot-and-generation word itself. A reference read out
            // of a struct or a list arrives boxed instead, and that case is
            // unboxed where it is read, below.
            //
            // The referent is made unique and then written *in place*: every
            // other holder of the reference has to see the write, and a cycle
            // proves that rebuilding is not enough — `a.out[0].out[0].name`
            // comes back to `a`, so an outer copy stored afterwards erased the
            // inner write.
            // `r.value = x` replaces the referent itself ([T-REF-VALUE]).
            // There is no field to descend into, so this is the slab write and
            // not a write inside a struct that lives there.
            if path.len() == 1
                && matches!(path[0], PlaceSeg::Field("value"))
                && !matches!(inner, VType::Struct(_) | VType::Tuple(_))
            {
                self.call_rt("grasm_ref_set", &[root, boxed_val]);
                self.wrote_through_ref = true;
                return Ok(root);
            }
            let referent = self.call_rt("grasm_ref_make_unique", &[root]).unwrap();
            self.write_place(span, referent, path, &inner, boxed_val)?;
            self.wrote_through_ref = true;
            return Ok(root);
        }
        match path.split_first() {
            None => Ok(boxed_val),
            Some((seg, [])) => match (root_t, seg) {
                // By position where the struct's type is known, which is
                // everywhere the checker resolved a field — the name form
                // stays for the case it did not.
                (VType::Struct(sn), PlaceSeg::Field(f)) => {
                    let sn = sn.clone();
                    Ok(match self.struct_field_index(&sn, f) {
                        Some(i) => {
                            let iv = self.b.ins().iconst(types::I64, i);
                            self.call_rt("grasm_struct_set_field_at", &[root, iv, boxed_val])
                                .unwrap()
                        }
                        None => {
                            let (fp, fl) = self.str_ptr_len(f);
                            self.call_rt("grasm_struct_set_field", &[root, fp, fl, boxed_val])
                                .unwrap()
                        }
                    })
                }
                (VType::List(_), PlaceSeg::Index(ix)) => {
                    let (iv, _) = self.index_value(ix)?;
                    Ok(self
                        .call_rt("grasm_list_set", &[root, iv, boxed_val])
                        .unwrap())
                }
                _ => nope(span, place_shape(root_t, seg)),
            },
            Some((seg, rest)) => {
                // Descend: read the child, recurse, then write it back.
                let (child, child_t, writeback): (Value, VType, PlaceSeg<'_>) =
                    match (root_t, seg) {
                        (VType::Struct(sname), PlaceSeg::Field(f)) => {
                            let sname = sname.clone();
                            let ft = self.struct_field_type(&sname, f, Span::new(0, 1))?;
                            let (fp, fl) = self.str_ptr_len(f);
                            let c = self.call_rt("grasm_struct_get", &[root, fp, fl]).unwrap();
                            (c, ft, PlaceSeg::Field(f))
                        }
                        (VType::List(e), PlaceSeg::Index(ix)) => {
                            let et = (**e).clone();
                            let (iv, _) = self.index_value(ix)?;
                            let c = self.call_rt("grasm_list_get", &[root, iv]).unwrap();
                            let _ = iv;
                            (c, et, PlaceSeg::Index(ix))
                        }
                        _ => return nope(span, place_shape(root_t, seg)),
                    };
                // A reference child is written *through*: the write lands in
                // the slab, so every other holder of it sees the change and
                // nothing above needs updating — not this parent, and not any
                // of its own. Saying so is what keeps the walk balanced: the
                // setters skip their release when handed back the pointer they
                // already hold, so a chain of no-op write-backs leaked one
                // reference per level.
                if let VType::Ref(inner) = &child_t {
                    let inner = (**inner).clone();
                    let raw = self.call_rt("grasm_unbox_i64", &[child]).unwrap();
                    self.call_rt("grasm_val_dec", &[child]);
                    // `xs[i].value = v` replaces the referent itself
                    // ([T-REF-VALUE]), the same as `r.value = v` does at the
                    // root. A reference reached through a list or a struct is
                    // still a reference, and this is the same slab write — no
                    // `make_unique`, because nothing inside the referent is
                    // being edited.
                    if rest.len() == 1
                        && matches!(rest[0], PlaceSeg::Field("value"))
                        && !matches!(inner, VType::Struct(_) | VType::Tuple(_))
                    {
                        self.call_rt("grasm_ref_set", &[raw, boxed_val]);
                        self.wrote_through_ref = true;
                        return Ok(root);
                    }
                    let referent = self.call_rt("grasm_ref_make_unique", &[raw]).unwrap();
                    self.write_place(span, referent, rest, &inner, boxed_val)?;
                    self.wrote_through_ref = true;
                    return Ok(root);
                }
                let new_child = self.write_place(span, child, rest, &child_t, boxed_val)?;
                if self.wrote_through_ref {
                    // Something below reached a reference and wrote through
                    // it, so this level's child is unchanged and there is
                    // nothing to write back — but the getter that produced it
                    // incremented, and only the write-back would have released
                    // it. One leaked reference per level of the path.
                    if child_t.boxed() {
                        self.call_rt("grasm_val_dec", &[new_child]);
                    }
                    return Ok(root);
                }
                match (root_t, writeback) {
                    (VType::Struct(sn), PlaceSeg::Field(f)) => {
                        let sn = sn.clone();
                        Ok(match self.struct_field_index(&sn, f) {
                            Some(i) => {
                                let iv = self.b.ins().iconst(types::I64, i);
                                self.call_rt(
                                    "grasm_struct_set_field_at",
                                    &[root, iv, new_child],
                                )
                                .unwrap()
                            }
                            None => {
                                let (fp, fl) = self.str_ptr_len(f);
                                self.call_rt(
                                    "grasm_struct_set_field",
                                    &[root, fp, fl, new_child],
                                )
                                .unwrap()
                            }
                        })
                    }
                    (VType::List(_), PlaceSeg::Index(ix)) => {
                        let (iv, _) = self.index_value(ix)?;
                        Ok(self
                            .call_rt("grasm_list_set", &[root, iv, new_child])
                            .unwrap())
                    }
                    _ => nope(span, place_shape(root_t, seg)),
                }
            }
        }
    }

    /// Calls a function value held in `var`: unpack the code pointer and the
    /// environment, then call indirectly with the environment first.
    fn call_fn_value(
        &mut self,
        var: Variable,
        param_ts: &[VType],
        ret_t: &VType,
        args: &[Expr],
        span: Span,
    ) -> CResult<Option<(Value, VType)>> {
        if args.len() != param_ts.len() {
            return nope(span, "calling a function value with a different arity");
        }
        let fv = self.b.use_var(var);
        let code = self.call_rt("grasm_fn_code", &[fv]).unwrap();
        let env = self.call_rt("grasm_fn_env", &[fv]).unwrap();

        let ptr_ty = self.cg.module.target_config().pointer_type();
        let mut sig = self.cg.module.make_signature();
        sig.params.push(AbiParam::new(ptr_ty));
        for p in param_ts {
            sig.params.push(AbiParam::new(p.cl()));
        }
        if *ret_t != VType::Unit {
            sig.returns.push(AbiParam::new(ret_t.cl()));
        }
        let sig_ref = self.b.import_signature(sig);

        let mut vals = vec![env];
        for (a, want) in args.iter().zip(param_ts) {
            let (mut v, vt) = self.eval_expr_expecting(a, Some(want))?;
            if *want == VType::F64 && vt == VType::I64 {
                v = self.b.ins().fcvt_from_sint(types::F64, v);
            }
            vals.push(v);
        }
        let inst = self.b.ins().call_indirect(sig_ref, code, &vals);
        let results: Vec<Value> = self.b.inst_results(inst).to_vec();
        Ok(if *ret_t == VType::Unit {
            None
        } else {
            Some((results[0], ret_t.clone()))
        })
    }

    /// Builds a function value from a named function.
    fn make_named_fn_value(&mut self, key: &str, span: Span) -> CResult<(Value, VType)> {
        let sig_info = self.cg.fns[key].clone();
        let wrapper = self.cg.fn_value_wrapper(key, span)?;
        let fref = self.cg.module.declare_func_in_func(wrapper, self.b.func);
        let ptr_ty = self.cg.module.target_config().pointer_type();
        let code = self.b.ins().func_addr(ptr_ty, fref);
        // No captures, so the environment is an empty list rather than null:
        // one shape for every function value keeps the call site simple.
        let env = self.call_rt("grasm_list_new", &[]).unwrap();
        let (name_p, name_l) = self.str_ptr_len(key);
        let v = self
            .call_rt("grasm_fn_new", &[code, env, name_p, name_l])
            .unwrap();
        Ok((v, VType::Fn(sig_info.params, Box::new(sig_info.ret))))
    }

    /// Builds a function value from a closure literal, lifting its body.
    ///
    /// Free variables are captured *by value* at this point, matching the
    /// interpreter: a closure that mutates a captured binding cannot be
    /// observed doing so from outside ([E-VALUE-SEMANTICS]).
    fn make_closure_value(
        &mut self,
        params: &[Ident],
        body: &Expr,
        binding: Option<&str>,
        span: Span,
    ) -> CResult<(Value, VType)> {
        let param_types = self.closure_param_types(params, binding, span)?;
        let want_ret = binding.and_then(|name| {
            self.closure_hints
                .iter()
                .rev()
                .find_map(|scope| scope.declared_ret.get(name).cloned())
        });
        self.closure_value_with_ret(params, body, &param_types, want_ret, span)
    }

    /// `closure_value_typed`, told what the closure is declared to return.
    fn closure_value_with_ret(
        &mut self,
        params: &[Ident],
        body: &Expr,
        param_types: &[VType],
        want_ret: Option<VType>,
        span: Span,
    ) -> CResult<(Value, VType)> {
        let param_types = param_types.to_vec();
        let captures = self.closure_captures(params, body);

        // Probe the body's type with the parameters and captures bound, so the
        // lifted function's return type is known before it is compiled.
        self.push_scope();
        for (name, vt) in &captures {
            let var = self.new_var(vt);
            let zero = self.zero_of(vt);
            self.b.def_var(var, zero);
            self.bind(name, var, vt.clone());
        }
        for (p, pt) in params.iter().zip(&param_types) {
            let var = self.new_var(pt);
            let zero = self.zero_of(pt);
            self.b.def_var(var, zero);
            self.bind(&p.name, var, pt.clone());
        }
        let ret = self.probe_type(body);
        self.pop_scope_no_decs();
        let probed = ret?;
        // A declaration wins: the body of a fallible closure probes as its ok
        // half, and only the declaration says the other half exists.
        let (ret, wrap_ok) = match &want_ret {
            Some(w @ VType::Fallible(..)) if !matches!(probed, VType::Fallible(..)) => {
                (w.clone(), true)
            }
            Some(w) if *w != VType::Unit && probed == VType::Unit => (w.clone(), false),
            _ => (probed, false),
        };

        let ptr_ty = self.cg.module.target_config().pointer_type();
        let mut sig = self.cg.module.make_signature();
        sig.params.push(AbiParam::new(ptr_ty));
        for t in &param_types {
            sig.params.push(AbiParam::new(t.cl()));
        }
        if ret != VType::Unit {
            sig.returns.push(AbiParam::new(ret.cl()));
        }
        let sym = format!("gc_{}_{}", self.cg.current_module.replace('.', "_"), span.start);
        let id = self
            .cg
            .module
            .declare_function(&sym, Linkage::Local, &sig)
            .map_err(|e| Unsupported {
                span,
                what: format!("(module error: {e})"),
            })?;
        self.cg.pending_closures.push(PendingClosure {
            id,
            module: self.cg.current_module.clone(),
            wrap_ok,
            captures: captures.clone(),
            params: params
                .iter()
                .map(|p| p.name.clone())
                .zip(param_types.iter().cloned())
                .collect(),
            ret: ret.clone(),
            body: body.clone(),
        });

        // Build the environment in the same order the lifted body unpacks it.
        let mut env = self.call_rt("grasm_list_new", &[]).unwrap();
        for (name, vt) in &captures {
            let (var, _) = self.lookup(name).expect("capture was resolved above");
            let v = self.b.use_var(var);
            if vt.boxed() {
                self.call_rt("grasm_val_inc", &[v]);
            }
            let boxed = self.box_value(v, vt);
            env = self.call_rt("grasm_list_push", &[env, boxed]).unwrap();
        }
        let fref = self.cg.module.declare_func_in_func(id, self.b.func);
        let code = self.b.ins().func_addr(ptr_ty, fref);
        let empty = self.b.ins().iconst(types::I64, 0);
        let v = self
            .call_rt("grasm_fn_new", &[code, env, empty, empty])
            .unwrap();
        Ok((v, VType::Fn(param_types, Box::new(ret))))
    }

    /// Parameter types of a closure literal, inferred from how the body uses
    /// them. Only a few shapes are inferable without a function-type syntax,
    /// so anything else is reported rather than assumed.
    /// Records, for each closure bound in this block, the parameter types any
    /// function it is *passed to* declares. `collect_block_hints` cannot do
    /// this: it is a free function and the signatures live on the compiler.
    fn declared_fn_arg_types(&mut self, blk: &grasm_syntax::ast::Block, hints: &mut BlockHints) {
        let bound: Vec<String> = hints
            .inits
            .iter()
            .filter(|(_, init)| matches!(init, Expr::Closure { params, .. } if !params.is_empty()))
            .map(|(n, _)| n.clone())
            .collect();
        if bound.is_empty() {
            return;
        }
        // The annotation on the binding itself, which is what the refusal
        // tells the reader to write: `let f: fn(i64) -> i64 = |x| x + 1`.
        // Without this the advice named a form the backend then refused.
        for stmt in &blk.stmts {
            let Stmt::Let { name, ty: Some(ty), init, .. } = stmt else {
                continue;
            };
            if !matches!(init, Expr::Closure { params, .. } if !params.is_empty()) {
                continue;
            }
            if let Ok(VType::Fn(ps, ret)) = self.cg.vtype_of(ty) {
                hints.declared.entry(name.name.clone()).or_insert(ps);
                hints
                    .declared_ret
                    .entry(name.name.clone())
                    .or_insert(*ret);
            }
        }
        // A struct field's declared type says what a closure put in it takes:
        // `type Box = { f: fn(i64) -> i64 }` and `Box{ f: g }` for a
        // `let`-bound `g`. Without this the backend refused `g` for having
        // "no type to say what its parameters are", with the type written on
        // the field.
        for stmt in &blk.stmts {
            let mut lits: Vec<(&Path, &Vec<(Ident, Expr)>)> = Vec::new();
            collect_struct_lits_stmt(stmt, &mut lits);
            for (name, fields) in lits {
                let sname = name.segments.last().map(|s| s.name.clone()).unwrap_or_default();
                for (fname, value) in fields {
                    let Expr::Ident(id) = value else { continue };
                    if !bound.contains(&id.name) {
                        continue;
                    }
                    let Ok(VType::Fn(ps, ret)) =
                        self.struct_field_type(&sname, &fname.name, id.span)
                    else {
                        continue;
                    };
                    hints.declared.entry(id.name.clone()).or_insert(ps);
                    hints.declared_ret.entry(id.name.clone()).or_insert(*ret);
                }
            }
        }
        let mut calls: Vec<(String, Vec<Expr>)> = Vec::new();
        collect_named_calls_block(blk, &mut calls);
        for (fname, args) in calls {
            let Some(sig) = self.lookup_fn(&fname).map(|s| s.params.clone()) else {
                continue;
            };
            for (i, a) in args.iter().enumerate() {
                let Expr::Ident(id) = a else { continue };
                if !bound.contains(&id.name) {
                    continue;
                }
                if let Some(VType::Fn(ps, ret)) = sig.get(i) {
                    hints.declared.entry(id.name.clone()).or_insert(ps.clone());
                    hints
                        .declared_ret
                        .entry(id.name.clone())
                        .or_insert((**ret).clone());
                }
            }
        }
    }

    fn closure_param_types(
        &mut self,
        params: &[Ident],
        binding: Option<&str>,
        span: Span,
    ) -> CResult<Vec<VType>> {
        if params.is_empty() {
            return Ok(Vec::new());
        }
        // The checker settled these, and asking it is not one more heuristic —
        // it is the answer the rest of this function tries to reconstruct.
        //
        // A closure whose parameters nothing local fixes learns them from its
        // call site, and inference reaches both. Everything below reconstructs
        // that from the shapes it can see here, which is why `let f = |p| p.0 +
        // p.1` called as `f((i, v))` was refused: the call site is in a
        // different statement, and no shape here reaches it.
        //
        // Kept as a fallback rather than deleted. It handles closures the
        // checker leaves with a type variable — genuinely unresolved, where a
        // refusal is right — and it also runs while probing, where a partial
        // answer is still useful.
        if let Some(ts) = self.cg.closure_param_types.get(&span.start).cloned() {
            if ts.len() == params.len() {
                let mut out = Vec::with_capacity(ts.len());
                let mut all = true;
                for t in &ts {
                    match grasm_syntax::parse_type_src(t)
                        .and_then(|te| self.cg.vtype_of(&te).ok())
                    {
                        Some(vt) => out.push(vt),
                        None => {
                            all = false;
                            break;
                        }
                    }
                }
                if all {
                    return Ok(out);
                }
            }
        }
        // The parameter types come from a call site of whatever the closure
        // was bound to. Nothing else in the language says what they are.
        // A declared signature is the surer source, so it is asked first.
        if let Some(name) = binding {
            let declared = self
                .closure_hints
                .iter()
                .rev()
                .find_map(|scope| scope.declared.get(name).cloned());
            if let Some(ts) = declared {
                if ts.len() == params.len() {
                    return Ok(ts);
                }
            }
        }
        // No name of its own: an arm of an annotated `match` or `if`.
        if let Some(t) = self
            .closure_hints
            .iter()
            .rev()
            .find_map(|scope| scope.declared_at.get(&span.start).cloned())
        {
            if let Ok(VType::Fn(ps, _)) = self.cg.vtype_of(&t) {
                if ps.len() == params.len() {
                    return Ok(ps);
                }
            }
        }
        let hint = binding.and_then(|name| {
            self.closure_hints
                .iter()
                .rev()
                .find_map(|scope| scope.calls.get(name).cloned())
        });
        let Some(args) = hint else {
            // A function whose declared return type *is* a function type says
            // exactly what the parameters are, and saying so is all this shape
            // ever needed: the refusal named the return, while what was
            // missing was the parameter types the return had already written
            // down.
            if let VType::Fn(ps, _) = &self.ret {
                if ps.len() == params.len() {
                    return Ok(ps.clone());
                }
            }
            return nope(
                span,
                "a closure with parameters used as a value where no type says \
what its parameters are (annotate the binding, as in \
`let f: fn(i64) -> i64 = |x| x + 1`, or make it a top-level `fn` — a closure's \
own parameters cannot be annotated)",
            );
        };
        if args.len() != params.len() {
            return nope(span, "a closure called with a different arity");
        }
        let mut out = Vec::with_capacity(params.len());
        for a in &args {
            out.push(self.probe_arg_type(a)?);
        }
        Ok(out)
    }

    /// Types a call-site argument that may name a binding declared later in
    /// the block, which is legal for the *call* even though the closure is
    /// lowered earlier.
    fn probe_arg_type(&mut self, a: &Expr) -> CResult<VType> {
        if let Ok(t) = self.probe_type(a) {
            return Ok(t);
        }
        // The argument may *contain* a binding declared after the closure —
        // `f(xs[0])`, `f(m.get(k) ?? 0)`, `f(t.0)`. Bind every hint this
        // block carries into a probe scope and try once more, so the shape is
        // worked out from what the block already says rather than refused as
        // "the type of `xs`" about a list whose literal is two lines down.
        if !self.probing_args {
            self.probing_args = true;
            let hinted: Vec<(String, Expr)> = self
                .closure_hints
                .iter()
                .rev()
                .flat_map(|scope| {
                    scope
                        .inits
                        .iter()
                        .map(|(n, e)| (n.clone(), e.clone()))
                        .chain(scope.loop_over.iter().map(|(n, e)| (n.clone(), e.clone())))
                })
                .collect();
            let loop_names: std::collections::HashSet<String> = self
                .closure_hints
                .iter()
                .flat_map(|scope| scope.loop_over.keys().cloned())
                .collect();
            let mut env: HashMap<String, VType> = HashMap::new();
            let declared: Vec<(String, grasm_syntax::ast::TypeExpr)> = self
                .closure_hints
                .iter()
                .rev()
                .flat_map(|scope| {
                    scope.declared_ty.iter().map(|(n, t)| (n.clone(), t.clone()))
                })
                .collect();
            for (n, t) in declared {
                if let Ok(vt) = self.cg.vtype_of(&t) {
                    env.insert(n, vt);
                }
            }
            for (n, e) in hinted {
                if env.contains_key(&n) {
                    continue;
                }
                if let Ok(t) = self.probe_type(&e) {
                    let t = if loop_names.contains(&n) {
                        match t {
                            VType::List(el) => *el,
                            VType::Map(k, v) => VType::Tuple(vec![*k, *v]),
                            other => other,
                        }
                    } else {
                        t
                    };
                    env.insert(n, t);
                }
            }
            self.probe_types.push(env);
            let out = self.probe_type(a);
            self.probe_types.pop();
            self.probing_args = false;
            if let Ok(t) = out {
                return Ok(t);
            }
        }
        if let Expr::Ident(id) = a {
            let init = self
                .closure_hints
                .iter()
                .rev()
                .find_map(|scope| scope.inits.get(&id.name).cloned());
            if let Some(init) = init {
                return self.probe_type(&init);
            }
            // A `for` variable: its type is the element type of what the loop
            // walks.
            let over = self
                .closure_hints
                .iter()
                .rev()
                .find_map(|scope| scope.loop_over.get(&id.name).cloned());
            if let Some(over) = over {
                if let Ok(t) = self.probe_type(&over) {
                    return Ok(match t {
                        VType::List(e) => *e,
                        VType::Map(k, v) => VType::Tuple(vec![*k, *v]),
                        VType::Str => VType::Str,
                        other => other,
                    });
                }
            }
            let _ = id;
        }
        // The retry did not settle it either. Name the shape rather than the
        // argument: "the type of `v`" pointed at a binding whose type the
        // line above fixes, and sent the reader to the one place the fix does
        // not go.
        nope(
            a.span(),
            "a closure with parameters whose types nothing here fixes (annotate the \
binding, as in `let f: fn(i64) -> i64 = |x| x + 1`, or make it a top-level `fn`)",
        )
    }

    /// Locals the closure body refers to, in a stable order.
    fn closure_captures(&mut self, params: &[Ident], body: &Expr) -> Vec<(String, VType)> {
        let mut names = Vec::new();
        collect_idents(body, &mut names);
        let mut out: Vec<(String, VType)> = Vec::new();
        for n in names {
            if params.iter().any(|p| p.name == n) || out.iter().any(|(o, _)| *o == n) {
                continue;
            }
            if let Some((_, vt)) = self.lookup(&n) {
                out.push((n, vt));
            }
        }
        out
    }

    /// Lowers `xs.par_map(|x| body)` onto real threads.
    ///
    /// `xs.par_map(f)` as `xs.par_map(|x| f(x))`.
    ///
    /// The parallel lowering needs a body to compile into the worker, and a
    /// bare function name is not one. Rather than teach the lowering a second
    /// shape, the call is desugared into the shape it already handles — which
    /// is also exactly what the effect checker treats the two spellings as.
    ///
    /// Returns `None` when the argument is already a closure, or is neither.
    fn desugar_fn_arg(&self, arg: &Expr, parallel: bool) -> Option<(Ident, Expr)> {
        // `utils.helper` passed as a value is the same thing as a bare name,
        // wrapped one level deeper. Rewriting it to `|x| utils.helper(x)` is
        // exactly what a caller had to write by hand.
        if let Expr::Field { recv, name, span } = arg {
            if let Expr::Ident(tid) = &**recv {
                if self.lookup(&tid.name).is_none()
                    && self
                        .cg
                        .module_imports
                        .get(&self.cg.current_module)
                        .is_some_and(|m| m.contains_key(&tid.name))
                {
                    let param = Ident {
                        name: "__par_arg".to_string(),
                        span: *span,
                    };
                    let call = Expr::MethodCall {
                        recv: recv.clone(),
                        name: name.clone(),
                        args: vec![Expr::Ident(param.clone())],
                        span: *span,
                    };
                    return Some((param, call));
                }
            }
        }
        let Expr::Ident(id) = arg else {
            return None;
        };
        // A top-level function works everywhere. A *value* of function type —
        // a parameter, or a local — needs an indirect call, which the
        // sequential loops can make and the parallel workers cannot.
        let is_named = self.lookup_fn(&id.name).is_some();
        // A *value* of function type — a parameter, or a local — needs an
        // indirect call. The sequential loops make one directly; a parallel
        // operation makes it inside `emit_par_map`, which routes anything
        // whose captures are not scalars to its own sequential fallback
        // ("slower, never wrong"). Refusing it outright instead named a
        // closure in programs that contain none — `fn go(xs: [i64], f:
        // fn(i64) -> i64) { xs.par_map(f) }` and `let f = sq` — and the help
        // told the reader to pass a top-level `fn`, which is what they had
        // passed.
        let is_fn_value = matches!(self.lookup(&id.name), Some((_, VType::Fn(_, _))));
        let _ = parallel;
        if !is_named && !is_fn_value {
            return None;
        }
        let span = id.span;
        let param = Ident {
            name: "__par_arg".to_string(),
            span,
        };
        let call = Expr::Call {
            callee: Box::new(Expr::Ident(id.clone())),
            args: vec![Expr::Ident(param.clone())],
            span,
        };
        Some((param, call))
    }

    /// Safe to run on several cores because two separate things hold: the
    /// checker has proved the function effect-free, so there is no observable
    /// interleaving; and both the elements and the captured values are
    /// *scalars*, so nothing refcounted crosses a thread. Each worker writes a
    /// disjoint index range, which is why the result does not depend on how
    /// many cores ran it ([E-PAR-MAP-ORDER]).
    ///
    /// Anything outside that — boxed elements, boxed captures — falls back to
    /// the sequential inline loop. Slower, never wrong.
    fn emit_par_map(
        &mut self,
        list: Value,
        elem_t: &VType,
        param: &Ident,
        body: &Expr,
        span: Span,
    ) -> CResult<(Value, VType)> {
        let captures = self.closure_captures(std::slice::from_ref(param), body);
        let out_t = {
            self.push_scope();
            let dummy = self.new_var(elem_t);
            let z = self.zero_of(elem_t);
            self.b.def_var(dummy, z);
            self.bind(&param.name, dummy, elem_t.clone());
            let probed = self.probe_type(body);
            self.pop_scope_no_decs();
            probed
        };
        // A body whose result type cannot be determined here is still a
        // perfectly good sequential map; only the threading needs the type.
        let Ok(out_t) = out_t else {
            return self.emit_list_transform(list, elem_t, param, body, true);
        };

        // A word or a pointer: either crosses a thread as eight bytes. Boxed
        // elements were refused here until the reference count became atomic,
        // which is the only thing that made a worker touching one unsafe.
        let ok = |t: &VType| scalar_kind_i64(t).is_some() || t.boxed();
        let all_scalar = scalar_kind_i64(elem_t).is_some()
            && scalar_kind_i64(&out_t).is_some()
            && captures.iter().all(|(_, t)| scalar_kind_i64(t).is_some());
        if !(ok(elem_t) && ok(&out_t) && captures.iter().all(|(_, t)| ok(t)))
            || body_can_fail(body)
        {
            // Not parallelizable with this representation; the sequential
            // lowering produces the same list.
            return self.emit_list_transform(list, elem_t, param, body, true);
        }

        // Lift the body to `extern "C" fn(*const i64 env, i64 x) -> i64`,
        // trafficking in raw scalar bits so the worker shares nothing.
        let ptr_ty = self.cg.module.target_config().pointer_type();
        let mut sig = self.cg.module.make_signature();
        sig.params.push(AbiParam::new(ptr_ty));
        sig.params.push(AbiParam::new(types::I64));
        sig.returns.push(AbiParam::new(types::I64));
        self.cg.par_map_seq += 1;
        let sym = format!(
            "gp_{}_{}",
            self.cg.current_module.replace('.', "_"),
            self.cg.par_map_seq
        );
        let id = self
            .cg
            .module
            .declare_function(&sym, Linkage::Local, &sig)
            .map_err(|e| Unsupported {
                span,
                what: format!("(module error: {e})"),
            })?;
        self.cg.pending_par_maps.push(PendingParMap {
            id,
            module: self.cg.current_module.clone(),
            captures: captures.clone(),
            param: param.name.clone(),
            param_t: elem_t.clone(),
            ret: out_t.clone(),
            body: body.clone(),
        });

        // The captured scalars go into a stack array the runtime copies per
        // worker — no shared mutable state at all.
        let slot_bytes = (captures.len().max(1) * 8) as u32;
        let slot = self.b.create_sized_stack_slot(cranelift_codegen::ir::StackSlotData::new(
            cranelift_codegen::ir::StackSlotKind::ExplicitSlot,
            slot_bytes,
            3,
        ));
        for (i, (name, vt)) in captures.iter().enumerate() {
            let (var, _) = self.lookup(name).expect("capture resolved above");
            let v = self.b.use_var(var);
            let bits = self.value_to_bits(v, vt);
            self.b.ins().stack_store(bits, slot, (i * 8) as i32);
        }
        let env_ptr = self.b.ins().stack_addr(ptr_ty, slot, 0);
        let fref = self.cg.module.declare_func_in_func(id, self.b.func);
        let code = self.b.ins().func_addr(ptr_ty, fref);
        let env_len = self.b.ins().iconst(types::I64, captures.len() as i64);
        let out = if all_scalar {
            let out_kind = self
                .b
                .ins()
                .iconst(types::I64, scalar_kind_i64(&out_t).expect("checked above"));
            self.call_rt(
                "grasm_par_map_scalar",
                &[list, code, env_ptr, env_len, out_kind],
            )
            .unwrap()
        } else {
            // Both representations are the compiler's to state: they follow
            // the static type, and a boxed list of integers exists.
            let in_boxed = self
                .b
                .ins()
                .iconst(types::I64, i64::from(elem_t.boxed()));
            let out_kind = self
                .b
                .ins()
                .iconst(types::I64, scalar_kind_i64(&out_t).unwrap_or(-1));
            self.call_rt(
                "grasm_par_map_mixed",
                &[list, code, env_ptr, env_len, in_boxed, out_kind],
            )
            .unwrap()
        };
        self.call_rt("grasm_val_dec", &[list]);
        Ok((out, VType::List(Box::new(out_t))))
    }

    /// Lowers `xs.par_filter(|x| pred)` onto real threads.
    ///
    /// The predicate runs concurrently and the surviving elements are then
    /// compacted in input order, so the result is independent of core count.
    /// Falls back to the sequential lowering when the elements or captures are
    /// not scalars, for the same reason `par_map` does: nothing refcounted may
    /// cross a thread.
    fn emit_par_filter(
        &mut self,
        list: Value,
        elem_t: &VType,
        param: &Ident,
        body: &Expr,
        span: Span,
    ) -> CResult<(Value, VType)> {
        let captures = self.closure_captures(std::slice::from_ref(param), body);
        // As in `par_map`: a word or a pointer, either crosses a thread as
        // eight bytes, and boxed elements stopped being unsafe when the
        // reference count became atomic.
        let ok = |t: &VType| scalar_kind_i64(t).is_some() || t.boxed();
        let elems_inline = scalar_kind_i64(elem_t).is_some();
        if !(ok(elem_t) && captures.iter().all(|(_, t)| ok(t))) || body_can_fail(body) {
            return self.emit_list_transform(list, elem_t, param, body, false);
        }

        let ptr_ty = self.cg.module.target_config().pointer_type();
        let mut sig = self.cg.module.make_signature();
        sig.params.push(AbiParam::new(ptr_ty));
        sig.params.push(AbiParam::new(types::I64));
        sig.returns.push(AbiParam::new(types::I64));
        self.cg.par_map_seq += 1;
        let sym = format!(
            "gpf_{}_{}",
            self.cg.current_module.replace('.', "_"),
            self.cg.par_map_seq
        );
        let id = self
            .cg
            .module
            .declare_function(&sym, Linkage::Local, &sig)
            .map_err(|e| Unsupported {
                span,
                what: format!("(module error: {e})"),
            })?;
        // The lifted predicate returns bits like any other par_map body; the
        // runtime only cares whether they are zero.
        self.cg.pending_par_maps.push(PendingParMap {
            id,
            module: self.cg.current_module.clone(),
            captures: captures.clone(),
            param: param.name.clone(),
            param_t: elem_t.clone(),
            ret: VType::Bool,
            body: body.clone(),
        });

        let slot_bytes = (captures.len().max(1) * 8) as u32;
        let slot = self.b.create_sized_stack_slot(cranelift_codegen::ir::StackSlotData::new(
            cranelift_codegen::ir::StackSlotKind::ExplicitSlot,
            slot_bytes,
            3,
        ));
        for (i, (name, vt)) in captures.iter().enumerate() {
            let (var, _) = self.lookup(name).expect("capture resolved above");
            let v = self.b.use_var(var);
            let bits = self.value_to_bits(v, vt);
            self.b.ins().stack_store(bits, slot, (i * 8) as i32);
        }
        let env_ptr = self.b.ins().stack_addr(ptr_ty, slot, 0);
        let fref = self.cg.module.declare_func_in_func(id, self.b.func);
        let code = self.b.ins().func_addr(ptr_ty, fref);
        let env_len = self.b.ins().iconst(types::I64, captures.len() as i64);
        let out = self
            .call_rt(
                if elems_inline {
                    "grasm_par_filter_scalar"
                } else {
                    "grasm_par_filter_boxed"
                },
                &[list, code, env_ptr, env_len],
            )
            .unwrap();
        self.call_rt("grasm_val_dec", &[list]);
        Ok((out, VType::List(Box::new(elem_t.clone()))))
    }

    /// Lowers `parallel { … }`.
    ///
    /// The block runs first and only *records* which `spawn` sites fired, in
    /// order; the task bodies run afterwards, which is what makes the order
    /// deterministic ([E-SPAWN-ORDER]) and the scope joined ([E-PARALLEL-JOIN]).
    /// Since the bodies are lexically known, the runtime list holds site
    /// indices and the dispatch is a switch — no closures, no environment
    /// capture, and no allocation per task beyond an interned index.
    fn emit_parallel(&mut self, body: &grasm_syntax::ast::Block) -> CResult<()> {
        let sites = collect_spawn_sites(body);

        let list_var = self.new_var(&VType::List(Box::new(VType::I64)));
        let empty = self.new_list_of(&VType::I64);
        self.b.def_var(list_var, empty);
        self.task_scopes.push((list_var, 0));

        let (_, collect_terminated) = self.eval_block_inner(body)?;
        let assigned = self.task_scopes.pop().expect("pushed above").1;
        debug_assert_eq!(assigned, sites.len(), "spawn sites must be visited once");

        // A `return` inside the collection pass leaves the scope before any
        // task could run, exactly as the interpreter does.
        if collect_terminated || sites.is_empty() {
            if !self.is_filled() {
                let lv = self.b.use_var(list_var);
                self.call_rt("grasm_val_dec", &[lv]);
            }
            return Ok(());
        }

        let header = self.b.create_block();
        let dispatch = self.b.create_block();
        let step = self.b.create_block();
        let exit = self.b.create_block();
        let ivar = self.new_var(&VType::I64);
        let zero = self.b.ins().iconst(types::I64, 0);
        self.b.def_var(ivar, zero);
        self.b.ins().jump(header, &[]);
        self.filled = true;

        self.goto_block(header);
        let lv = self.b.use_var(list_var);
        let len = self.call_rt("grasm_list_len", &[lv]).unwrap();
        let iv = self.b.use_var(ivar);
        let more = self.b.ins().icmp(IntCC::SignedLessThan, iv, len);
        self.b.ins().brif(more, dispatch, &[], exit, &[]);
        self.filled = true;

        self.goto_block(dispatch);
        let lv2 = self.b.use_var(list_var);
        let iv2 = self.b.use_var(ivar);
        let (site, _) = self.list_get_elem(lv2, iv2, &VType::I64)?;

        // One block per site, chained on equality. The count is the number of
        // `spawn` statements in the source, so this stays small.
        let task_blocks: Vec<Block> = sites.iter().map(|_| self.b.create_block()).collect();
        for (k, target) in task_blocks.iter().enumerate() {
            let want = self.b.ins().iconst(types::I64, k as i64);
            let hit = self.b.ins().icmp(IntCC::Equal, site, want);
            if k + 1 == task_blocks.len() {
                // The last site is the only remaining possibility; a mismatch
                // would mean the recorded index came from nowhere.
                self.b.ins().brif(hit, *target, &[], step, &[]);
            } else {
                let next_test = self.b.create_block();
                self.b.ins().brif(hit, *target, &[], next_test, &[]);
                self.filled = true;
                self.goto_block(next_test);
            }
        }
        self.filled = true;

        for (task_body, block) in sites.iter().zip(&task_blocks) {
            self.goto_block(*block);
            let (_, terminated) = self.eval_block_inner(task_body)?;
            if !terminated && !self.is_filled() {
                self.b.ins().jump(step, &[]);
                self.filled = true;
            }
        }

        self.goto_block(step);
        let iv3 = self.b.use_var(ivar);
        let one = self.b.ins().iconst(types::I64, 1);
        let next = self.b.ins().iadd(iv3, one);
        self.b.def_var(ivar, next);
        self.b.ins().jump(header, &[]);
        self.filled = true;

        self.goto_block(exit);
        let lv3 = self.b.use_var(list_var);
        self.call_rt("grasm_val_dec", &[lv3]);
        Ok(())
    }

    /// Inlines `list.any/all/find(|x| pred)` as a loop with early exit.
    /// Consumes the owned `list`; returns (value, type).
    fn emit_list_search(
        &mut self,
        list: Value,
        elem_t: &VType,
        param: &Ident,
        body: &Expr,
        mode: &str,
    ) -> CResult<(Value, VType)> {
        let is_find = mode == "find";
        let is_all = mode == "all";
        let out_t = if is_find {
            VType::Option(Box::new(elem_t.clone()))
        } else {
            VType::Bool
        };

        let list_var = self.new_var(&VType::List(Box::new(elem_t.clone())));
        self.b.def_var(list_var, list);
        let out_var = self.new_var(&out_t);
        // `all` starts true, `any` false, `find` None.
        if is_find {
            let none = self.make_variant("Option", "None", &[])?;
            self.b.def_var(out_var, none);
        } else {
            let init = self.b.ins().iconst(types::I8, i64::from(is_all));
            self.b.def_var(out_var, init);
        }
        let done_var = self.new_var(&VType::Bool);
        let no = self.b.ins().iconst(types::I8, 0);
        self.b.def_var(done_var, no);
        let ivar = self.new_var(&VType::I64);
        let zero = self.b.ins().iconst(types::I64, 0);
        self.b.def_var(ivar, zero);

        let header = self.b.create_block();
        let body_blk = self.b.create_block();
        let join = self.b.create_block();
        let step = self.b.create_block();
        let exit = self.b.create_block();
        self.b.ins().jump(header, &[]);
        self.filled = true;

        self.goto_block(header);
        let lv = self.b.use_var(list_var);
        let len = self.call_rt("grasm_list_len", &[lv]).unwrap();
        let iv = self.b.use_var(ivar);
        let c = self.b.ins().icmp(IntCC::SignedLessThan, iv, len);
        self.b.ins().brif(c, body_blk, &[], exit, &[]);
        self.filled = true;

        self.goto_block(body_blk);
        let lv2 = self.b.use_var(list_var);
        let iv2 = self.b.use_var(ivar);
        let (ev, evt) = self.list_get_elem(lv2, iv2, elem_t)?;
        let evar = self.new_var(&evt);
        self.b.def_var(evar, ev);
        self.push_scope();
        self.bind(&param.name, evar, evt.clone());
        let (pred, pred_t) = self.eval_expr_val(body)?;
        if pred_t != VType::Bool {
            return nope(body.span(), "a non-bool predicate");
        }

        let hit = self.b.create_block();
        // `all` stops early on a FALSE predicate; `any`/`find` on a true one.
        if is_all {
            self.b.ins().brif(pred, join, &[], hit, &[]);
        } else {
            self.b.ins().brif(pred, hit, &[], join, &[]);
        }
        self.filled = true;

        self.goto_block(hit);
        if is_find {
            // The result started as a freshly allocated `None`. Overwriting it
            // without releasing it leaked that allocation on every `find` that
            // actually found something — which is most of them.
            let prev = self.b.use_var(out_var);
            self.call_rt("grasm_val_dec", &[prev]);
            let e2 = self.b.use_var(evar);
            if evt.boxed() {
                self.call_rt("grasm_val_inc", &[e2]);
            }
            let bx = self.box_value(e2, &evt);
            let some = self.make_variant("Option", "Some", &[bx])?;
            self.b.def_var(out_var, some);
        } else {
            // `any` found a match (true); `all` found a counterexample (false).
            let v = self.b.ins().iconst(types::I8, i64::from(!is_all));
            self.b.def_var(out_var, v);
        }
        let yes = self.b.ins().iconst(types::I8, 1);
        self.b.def_var(done_var, yes);
        self.b.ins().jump(join, &[]);
        self.filled = true;

        // One scope pop for one push, on the path both branches share.
        self.goto_block(join);
        self.pop_scope_with_decs();
        let d = self.b.use_var(done_var);
        self.b.ins().brif(d, exit, &[], step, &[]);
        self.filled = true;

        self.goto_block(step);
        let iv3 = self.b.use_var(ivar);
        let one = self.b.ins().iconst(types::I64, 1);
        let next = self.b.ins().iadd(iv3, one);
        self.b.def_var(ivar, next);
        self.b.ins().jump(header, &[]);
        self.filled = true;

        self.goto_block(exit);
        let lv3 = self.b.use_var(list_var);
        self.call_rt("grasm_val_dec", &[lv3]);
        let out = self.b.use_var(out_var);
        Ok((out, out_t))
    }

    /// Inlines `list.map(|x| body)` / `list.filter(|x| body)` as a loop.
    /// Consumes the owned `list`; returns (new list value, its type).
    /// `xs.fold(init, f)` as an inline loop over an accumulator variable.
    ///
    /// The accumulator's type comes from `init`, not from the element type,
    /// which is what makes `fold` the one combinator whose result is not a
    /// function of the list alone — `[i64].fold("", ..)` is a `str`.
    fn emit_list_fold(
        &mut self,
        list: Value,
        elem_t: &VType,
        init: &Expr,
        acc_param: &Ident,
        elem_param: &Ident,
        body: &Expr,
    ) -> CResult<(Value, VType)> {
        let (init_v, acc_t) = self.eval_expr_val(init)?;
        let acc_var = self.new_var(&acc_t);
        self.b.def_var(acc_var, init_v);

        let list_var = self.new_var(&VType::List(Box::new(elem_t.clone())));
        self.b.def_var(list_var, list);
        let ivar = self.new_var(&VType::I64);
        let zero = self.b.ins().iconst(types::I64, 0);
        self.b.def_var(ivar, zero);

        let header = self.b.create_block();
        let body_blk = self.b.create_block();
        let step = self.b.create_block();
        let exit = self.b.create_block();
        self.b.ins().jump(header, &[]);
        self.filled = true;

        self.goto_block(header);
        let lv = self.b.use_var(list_var);
        let len = self.call_rt("grasm_list_len", &[lv]).unwrap();
        let iv = self.b.use_var(ivar);
        let c = self.b.ins().icmp(IntCC::SignedLessThan, iv, len);
        self.b.ins().brif(c, body_blk, &[], exit, &[]);
        self.filled = true;

        self.goto_block(body_blk);
        let lv2 = self.b.use_var(list_var);
        let iv2 = self.b.use_var(ivar);
        let (ev, evt) = self.list_get_elem(lv2, iv2, elem_t)?;
        let evar = self.new_var(&evt);
        self.b.def_var(evar, ev);
        // The accumulator is rebound each round so the body reads the value
        // from the previous one. The binding gets its own reference, because
        // leaving the scope releases it — without that the round's explicit
        // release below would be the second one on a single +1, which is a
        // double free the moment the accumulator is boxed.
        let cur_acc = self.b.use_var(acc_var);
        self.inc_if_boxed(cur_acc, &acc_t);
        let bound_acc = self.new_var(&acc_t);
        self.b.def_var(bound_acc, cur_acc);
        self.push_scope();
        self.bind(&acc_param.name, bound_acc, acc_t.clone());
        self.bind(&elem_param.name, evar, evt.clone());
        // A `fail` in a fold whose accumulator is fallible is the closure's own
        // failure: it becomes the accumulator, and the next round receives it.
        // Without this it lowered as a return from the enclosing function, so
        // `[1, 2].fold(seed, |a, x| if x == 2 { fail(e) } else { a })` reported
        // an unhandled error from `main` where the interpreter finished and
        // printed the fallback (F43).
        let fold_fail = matches!(acc_t, VType::Fallible(..))
            && body_can_fail(body)
            && self.cg.fallible_closures.contains(&body.span().start);
        let fail_blk = if fold_fail {
            let blk = self.b.create_block();
            let slot = self.new_var(&VType::Any);
            // One *below* the current depth, so the scope holding the
            // closure's own bindings is released by the `fail` as well. The
            // ordinary path releases it with `pop_scope_with_decs`, and a fail
            // path that skipped it leaked whatever they held — invisible with
            // an `i64` element and one allocation with a `str`.
            self.fail_redirect.push((blk, slot, self.vars.len().saturating_sub(1)));
            Some((blk, slot))
        } else {
            None
        };
        let (bv, bvt) = self.eval_expr_val(body)?;
        if fail_blk.is_some() {
            self.fail_redirect.pop();
        }
        if bvt != acc_t {
            // Naming both is the difference between a refusal a reader can
            // act on and one that is simply false: an unannotated `map()`
            // seed reached here as a map with an unknown key, and the message
            // said the two types differed when they were written identically.
            return nope(
                body.span(),
                format!(
                    "a `fold` over `{}` whose function answers `{}`",
                    acc_t.show(),
                    bvt.show()
                ),
            );
        }
        // The previous accumulator is consumed by this round.
        let old = self.b.use_var(acc_var);
        self.dec_if_boxed(old, &acc_t);
        self.b.def_var(acc_var, bv);
        self.pop_scope_with_decs();
        self.b.ins().jump(step, &[]);
        self.filled = true;

        if let Some((blk, slot)) = fail_blk {
            // The failure is the new accumulator. The old one is released here
            // for the same reason it is on the ordinary path: this round
            // consumed it.
            self.goto_block(blk);
            let prev = self.b.use_var(acc_var);
            self.dec_if_boxed(prev, &acc_t);
            let errv = self.b.use_var(slot);
            self.b.def_var(acc_var, errv);
            self.b.ins().jump(step, &[]);
            self.filled = true;
        }

        self.goto_block(step);
        let iv3 = self.b.use_var(ivar);
        let one = self.b.ins().iconst(types::I64, 1);
        let next = self.b.ins().iadd(iv3, one);
        self.b.def_var(ivar, next);
        self.b.ins().jump(header, &[]);
        self.filled = true;

        self.goto_block(exit);
        let lv3 = self.b.use_var(list_var);
        self.call_rt("grasm_val_dec", &[lv3]);
        let out = self.b.use_var(acc_var);
        Ok((out, acc_t))
    }

    fn emit_list_transform(
        &mut self,
        list: Value,
        elem_t: &VType,
        param: &Ident,
        body: &Expr,
        is_map: bool,
    ) -> CResult<(Value, VType)> {
        let closure_at = body.span().start;
        // The body is inlined into the loop, and `fail` there lowers as a
        // return from the *enclosing* function — so `[1, 2].map(|x| if x == 2
        // { fail(e) } else { Ok(x) })` reported an unhandled error from
        // `main` where the interpreter produced `[Ok(1), Err(e)]`. A `fail`
        // inside a closure is that closure's own failure, which needs a value
        // per element rather than a return; until the loop can carry one,
        // this is a refusal rather than a wrong answer.
        // A `fail` in the body is the *closure's* failure and belongs in the
        // element, which only `map` has: `filter` answers a bool and `fold` a
        // single accumulator, and neither has a place to put an `Err` per item.
        let redirect_fail = body_can_fail(body) && self.cg.fallible_closures.contains(&closure_at);
        if redirect_fail && !is_map {
            return nope(
                body.span(),
                "`fail` inside a *fallible* closure passed to `filter` or `fold` (there is no per-element result to hold the failure — produce `Err(e)` instead of failing, or pass a named function)",
            );
        }
        let list_var = self.new_var(&VType::List(Box::new(elem_t.clone())));
        self.b.def_var(list_var, list);
        let out_var = self.new_var(&VType::List(Box::new(elem_t.clone())));
        // The output holds `filter`'s input type, or whatever `map`'s body
        // produces — probed so a scalar result gets inline storage. A body the
        // probe cannot type falls back to a boxed list rather than failing:
        // it compiled before this optimization existed.
        //
        // `xs.map(|x| print(x))` builds a list of nothings, and that used to be
        // refused here on the grounds that there was no native representation
        // for one. There is — `RtPayload::Unit` exists precisely so that
        // `[(), ()]` does not print as `[0, 0]` — and the rest of the refusal
        // was advice: *write a `for` loop, which is what a list of nothings was
        // standing in for*. That is good advice and it is not a limitation, and
        // dressing one as the other made two engines disagree about which
        // programs exist.
        let out_hint = if is_map {
            self.push_scope();
            let dummy = self.new_var(elem_t);
            let zero = self.zero_of(elem_t);
            self.b.def_var(dummy, zero);
            self.bind(&param.name, dummy, elem_t.clone());
            let probed = self.probe_type(body);
            self.pop_scope_no_decs();
            probed.ok()
        } else {
            Some(elem_t.clone())
        };
        let out0 = match out_hint {
            Some(t) => self.new_list_of(&t),
            None => self.call_rt("grasm_list_new", &[]).unwrap(),
        };
        self.b.def_var(out_var, out0);
        let ivar = self.new_var(&VType::I64);
        let zero = self.b.ins().iconst(types::I64, 0);
        self.b.def_var(ivar, zero);

        // The input and the half-built output are live across the whole loop,
        // and the body can leave without reaching the exit block — a `fail`
        // inside the closure returns straight out of the enclosing function.
        // Binding them into a scope is what releases them on that path;
        // without it `xs.par_map(|x| if p { fail(e) } else { x })` leaked the
        // input list and every element pushed before the failure.
        self.push_scope();
        let lt = VType::List(Box::new(elem_t.clone()));
        self.bind(" transform.in", list_var, lt.clone());
        self.bind(" transform.out", out_var, lt);

        let header = self.b.create_block();
        let body_blk = self.b.create_block();
        let step = self.b.create_block();
        let exit = self.b.create_block();
        self.b.ins().jump(header, &[]);
        self.filled = true;

        self.goto_block(header);
        let lv = self.b.use_var(list_var);
        let len = self.call_rt("grasm_list_len", &[lv]).unwrap();
        let iv = self.b.use_var(ivar);
        let c = self.b.ins().icmp(IntCC::SignedLessThan, iv, len);
        self.b.ins().brif(c, body_blk, &[], exit, &[]);
        self.filled = true;

        self.goto_block(body_blk);
        let lv2 = self.b.use_var(list_var);
        let iv2 = self.b.use_var(ivar);
        let (ev, evt) = self.list_get_elem(lv2, iv2, elem_t)?;
        let evar = self.new_var(&evt);
        self.b.def_var(evar, ev);
        self.push_scope();
        self.bind(&param.name, evar, evt.clone());
        // The closure's own `fail` goes here rather than out of this function:
        // it pushes an `Err` element and the loop carries on.
        let fail_blk = if redirect_fail {
            let blk = self.b.create_block();
            let slot = self.new_var(&VType::Any);
            // One *below* the current depth, so the scope holding the
            // closure's own bindings is released by the `fail` as well. The
            // ordinary path releases it with `pop_scope_with_decs`, and a fail
            // path that skipped it leaked whatever they held — invisible with
            // an `i64` element and one allocation with a `str`.
            self.fail_redirect.push((blk, slot, self.vars.len().saturating_sub(1)));
            Some((blk, slot))
        } else {
            None
        };
        let (bv, bvt) = self.eval_expr_val(body)?;
        if fail_blk.is_some() {
            self.fail_redirect.pop();
        }
        let out_elem_t = if is_map { bvt.clone() } else { elem_t.clone() };
        if is_map {
            let cur = self.b.use_var(out_var);
            let np = self.list_push_elem(cur, bv, &bvt);
            self.b.def_var(out_var, np);
            self.pop_scope_with_decs();
            self.b.ins().jump(step, &[]);
            self.filled = true;
            if let Some((blk, slot)) = fail_blk {
                // The element the failure produced. Boxed already — `fail`
                // wraps it — so it is pushed as the boxed value it is.
                self.goto_block(blk);
                let errv = self.b.use_var(slot);
                let cur = self.b.use_var(out_var);
                let np = self.list_push_elem(cur, errv, &VType::Any);
                self.b.def_var(out_var, np);
                self.b.ins().jump(step, &[]);
                self.filled = true;
            }
        } else {
            if bvt != VType::Bool {
                return nope(body.span(), "a non-bool filter predicate");
            }
            let keep = self.b.create_block();
            let skip = self.b.create_block();
            self.b.ins().brif(bv, keep, &[], skip, &[]);
            self.filled = true;
            self.goto_block(keep);
            let e2 = self.b.use_var(evar);
            if evt.boxed() {
                self.call_rt("grasm_val_inc", &[e2]);
            }
            let cur = self.b.use_var(out_var);
            let np = self.list_push_elem(cur, e2, &evt);
            self.b.def_var(out_var, np);
            self.b.ins().jump(skip, &[]);
            self.filled = true;
            self.goto_block(skip);
            self.pop_scope_with_decs();
            self.b.ins().jump(step, &[]);
            self.filled = true;
        }

        self.goto_block(step);
        let iv3 = self.b.use_var(ivar);
        let one = self.b.ins().iconst(types::I64, 1);
        let next = self.b.ins().iadd(iv3, one);
        self.b.def_var(ivar, next);
        self.b.ins().jump(header, &[]);
        self.filled = true;

        self.goto_block(exit);
        // Reaching here means the loop finished: the output is the value of
        // the expression and belongs to whoever asked for it, so it is not
        // released — only the input is.
        self.pop_scope_no_decs();
        let lv3 = self.b.use_var(list_var);
        self.call_rt("grasm_val_dec", &[lv3]);
        let out = self.b.use_var(out_var);
        Ok((out, VType::List(Box::new(out_elem_t))))
    }

    /// Whether a bare identifier in a pattern *tests* for a variant rather
    /// than binding a name.
    ///
    /// [G-PAT-BIND]: it tests only when it names a nullary variant of **the
    /// scrutinee's type**. Asked of every type in the program instead — and
    /// of the literal name `None` whatever the scrutinee — `match n { None ->
    /// … }` over an `i64` compiled as a variant test on a scalar. The arm's
    /// body then read a name nothing had bound, and the binary died with
    /// SIGSEGV and no output, or with `panic: assertion failed`, while the
    /// interpreter printed the answer. §11 promises the backend refuses what
    /// it cannot compile rather than miscompiling it.
    fn pattern_names_variant(&self, name: &str, st: &VType) -> bool {
        match st {
            VType::Option(_) => name == "None",
            VType::Fallible(..) => name == "Ok" || name == "Err",
            // Looked up inside *this* sum rather than in the by-name index.
            // A generic sum has one entry per instantiation and the index holds
            // one parent per variant name, so `Nothing` in an `Opt[i64]` was
            // matched against whichever `Opt[..]` was registered last.
            VType::Sum(parent) => self
                .cg
                .sums
                .get(parent)
                .is_some_and(|vs| vs.iter().any(|(n, ts)| n == name && ts.is_empty())),
            // A type the probe could not pin down. Every variant name is a
            // candidate, which is what this arm used to do for every
            // scrutinee.
            VType::Any => {
                matches!(self.cg.variant_index.get(name), Some((_, 0))) || name == "None"
            }
            _ => false,
        }
    }

    /// Declares the *types* a pattern would bind, without emitting any code.
    /// Used to type-probe match arm bodies before lowering.
    fn bind_pattern_types(&mut self, pat: &Pattern, st: &VType) -> CResult<()> {
        match pat {
            // Alternatives cannot bind, so there are no types to declare.
            Pattern::Or(..) => Ok(()),
            Pattern::Wildcard(_) | Pattern::Int(..) | Pattern::Bool(..) | Pattern::Str(..) => {
                Ok(())
            }
            Pattern::Bind(id) => {
                if !self.pattern_names_variant(&id.name, st) {
                    let var = self.new_var(st);
                    self.bind(&id.name, var, st.clone());
                }
                Ok(())
            }
            Pattern::Variant { path, fields, span } => {
                let vname = path.segments.last().unwrap().name.clone();
                let field_ts: Vec<VType> = match st {
                    VType::Option(p) => {
                        if vname == "Some" {
                            vec![(**p).clone()]
                        } else {
                            vec![]
                        }
                    }
                    VType::Sum(parent) => {
                        let parent = parent.clone();
                        self.variant_field_types(&parent, &vname, *span)?
                    }
                    // Both halves are known, so `Err(d)` binds `d` at the
                    // declared error type instead of `Any` — which is what
                    // made reading a field off it "field access on non-structs".
                    VType::Fallible(ok_t, err_t) => {
                        if vname == "Ok" {
                            vec![(**ok_t).clone()]
                        } else {
                            vec![(**err_t).clone()]
                        }
                    }
                    VType::Any => fields.iter().map(|_| VType::Any).collect(),
                    _ => return nope(*span, "variant patterns on this type"),
                };
                for (fpat, ft) in fields.iter().zip(&field_ts) {
                    self.bind_pattern_types(fpat, ft)?;
                }
                Ok(())
            }
            Pattern::Tuple(items, span) => {
                let VType::Tuple(elem_ts) = st.clone() else {
                    return nope(*span, "tuple patterns on this type");
                };
                for (p, t) in items.iter().zip(&elem_ts) {
                    self.bind_pattern_types(p, t)?;
                }
                Ok(())
            }
        }
    }

    /// Emits the test for one pattern; on match falls into `body_blk` (with
    /// binds defined), otherwise jumps to `next_test`.
    fn lower_pattern_test(
        &mut self,
        pat: &Pattern,
        svar: Variable,
        st: &VType,
        body_blk: Block,
        next_test: Block,
    ) -> CResult<bool> {
        match pat {
            // Each alternative is tested in turn; the first to match jumps to
            // the body, and only after all of them fail does control reach the
            // next arm.
            Pattern::Or(alts, _) => {
                let mut all_irrefutable = true;
                for (i, alt) in alts.iter().enumerate() {
                    let last = i + 1 == alts.len();
                    let fallthrough = if last {
                        next_test
                    } else {
                        self.b.create_block()
                    };
                    let refutable =
                        self.lower_pattern_test(alt, svar, st, body_blk, fallthrough)?;
                    all_irrefutable &= !refutable;
                    if !last {
                        self.goto_block(fallthrough);
                    }
                }
                Ok(!all_irrefutable)
            }
            Pattern::Wildcard(_) => {
                self.b.ins().jump(body_blk, &[]);
                self.filled = true;
                Ok(true)
            }
            Pattern::Bind(id) => {
                // A nullary variant *of the scrutinee's type* tests; anything
                // else binds ([G-PAT-BIND]).
                if self.pattern_names_variant(&id.name, st) {
                    let sv = self.b.use_var(svar);
                    let (vp, vl) = self.str_ptr_len(&id.name);
                    let c = self.call_rt("grasm_variant_is", &[sv, vp, vl]).unwrap();
                    self.b.ins().brif(c, body_blk, &[], next_test, &[]);
                    self.filled = true;
                    return Ok(true);
                }
                let sv = self.b.use_var(svar);
                if st.boxed() {
                    self.call_rt("grasm_val_inc", &[sv]);
                }
                let var = self.new_var(st);
                self.b.def_var(var, sv);
                self.bind(&id.name, var, st.clone());
                self.b.ins().jump(body_blk, &[]);
                self.filled = true;
                Ok(true)
            }
            Pattern::Int(want, _) => {
                let sv = self.b.use_var(svar);
                let w = self.b.ins().iconst(types::I64, *want);
                let c = self.b.ins().icmp(IntCC::Equal, sv, w);
                self.b.ins().brif(c, body_blk, &[], next_test, &[]);
                self.filled = true;
                Ok(true)
            }
            Pattern::Bool(want, _) => {
                let sv = self.b.use_var(svar);
                let w = self.b.ins().iconst(types::I8, i64::from(*want));
                let c = self.b.ins().icmp(IntCC::Equal, sv, w);
                self.b.ins().brif(c, body_blk, &[], next_test, &[]);
                self.filled = true;
                Ok(true)
            }
            Pattern::Str(want, _) => {
                let sv = self.b.use_var(svar);
                let (p, l) = self.str_ptr_len(want);
                let lit = self.call_rt("grasm_str_lit", &[p, l]).unwrap();
                let c = self.call_rt("grasm_str_eq", &[sv, lit]).unwrap();
                self.call_rt("grasm_val_dec", &[lit]);
                self.b.ins().brif(c, body_blk, &[], next_test, &[]);
                self.filled = true;
                Ok(true)
            }
            Pattern::Variant { path, fields, span } => {
                let vname = path.segments.last().unwrap().name.clone();
                let sv = self.b.use_var(svar);
                let (vp, vl) = self.str_ptr_len(&vname);
                let c = self.call_rt("grasm_variant_is", &[sv, vp, vl]).unwrap();
                let bind_blk = self.b.create_block();
                self.b.ins().brif(c, bind_blk, &[], next_test, &[]);
                self.filled = true;
                self.goto_block(bind_blk);
                // Field types by scrutinee static type.
                let field_ts: Vec<VType> = match st {
                    VType::Option(p) => {
                        if vname == "Some" {
                            vec![(**p).clone()]
                        } else {
                            vec![]
                        }
                    }
                    VType::Sum(parent) => {
                        let parent = parent.clone();
                        self.variant_field_types(&parent, &vname, *span)?
                    }
                    // Both halves are known, so `Err(d)` binds `d` at the
                    // declared error type instead of `Any` — which is what
                    // made reading a field off it "field access on non-structs".
                    VType::Fallible(ok_t, err_t) => {
                        if vname == "Ok" {
                            vec![(**ok_t).clone()]
                        } else {
                            vec![(**err_t).clone()]
                        }
                    }
                    VType::Any => fields.iter().map(|_| VType::Any).collect(),
                    _ => return nope(*span, "variant patterns on this type"),
                };
                for (i, (fpat, ft)) in fields.iter().zip(&field_ts).enumerate() {
                    let sv2 = self.b.use_var(svar);
                    let idx = self.b.ins().iconst(types::I64, i as i64);
                    let boxed = self.call_rt("grasm_variant_field", &[sv2, idx]).unwrap();
                    match fpat {
                        Pattern::Bind(fid)
                            if !matches!(
                                self.cg.variant_index.get(&fid.name),
                                Some((_, 0))
                            ) && fid.name != "None" =>
                        {
                            let (v, vt) = self.unbox(boxed, ft)?;
                            let var = self.new_var(&vt);
                            self.b.def_var(var, v);
                            self.bind(&fid.name, var, vt);
                        }
                        Pattern::Wildcard(_) => {
                            self.call_rt("grasm_val_dec", &[boxed]);
                        }
                        nested => {
                            // Recursive sub-pattern on the (boxed) field, held
                            // as a typed temp that owns the +1 from
                            // `grasm_variant_field`.
                            let (fv, fvt) = self.unbox(boxed, ft)?;
                            let fvar = self.new_var(&fvt);
                            self.b.def_var(fvar, fv);
                            // The temp is released on *both* ways out: on a
                            // match, because any name the sub-pattern bound
                            // took its own reference; on a mismatch, because
                            // the arm is abandoned. Nothing released it
                            // before, so every nested variant pattern —
                            // `Wrap(A)`, the shape [T-MATCH-NESTED] is about —
                            // leaked its payload, once per arm *tried*.
                            let cont = self.b.create_block();
                            let fail = self.b.create_block();
                            self.lower_pattern_test(nested, fvar, &fvt, cont, fail)?;
                            self.goto_block(fail);
                            if fvt.boxed() {
                                let t = self.b.use_var(fvar);
                                self.call_rt("grasm_val_dec", &[t]);
                            }
                            self.b.ins().jump(next_test, &[]);
                            self.filled = true;
                            self.goto_block(cont);
                            if fvt.boxed() {
                                let t = self.b.use_var(fvar);
                                self.call_rt("grasm_val_dec", &[t]);
                            }
                        }
                    }
                }
                self.b.ins().jump(body_blk, &[]);
                self.filled = true;
                Ok(true)
            }
            Pattern::Tuple(items, span) => {
                let VType::Tuple(elem_ts) = st.clone() else {
                    return nope(*span, "tuple patterns on this type");
                };
                if items.len() != elem_ts.len() {
                    return nope(*span, "a tuple pattern of the wrong arity");
                }
                if items.is_empty() {
                    self.b.ins().jump(body_blk, &[]);
                    self.filled = true;
                    return Ok(true);
                }
                // Bind each element to a temp, then chain the sub-tests.
                //
                // `grasm_list_get` hands back a reference the temp now owns.
                // A `Bind` sub-pattern takes that ownership directly, the way
                // the variant path above does: routing it through
                // `lower_pattern_test` instead incremented a *second* time —
                // that arm assumes a borrowed receiver — and only one
                // decrement ever ran, so every tuple pattern binding a boxed
                // component leaked it. A wildcard owns nothing and releases
                // immediately.
                let mut chain: Vec<(Variable, VType, &Pattern)> = Vec::new();
                // Components bound to a name. On the success path the arm's
                // scope releases them; on the *failure* path that scope is
                // never entered, so nothing did — every failing arm that binds
                // a name leaked one reference per boxed component, unbounded
                // in a loop.
                let mut bound: Vec<(Variable, VType)> = Vec::new();
                for (i, (pat_i, t_i)) in items.iter().zip(&elem_ts).enumerate() {
                    let sv = self.b.use_var(svar);
                    let idx = self.b.ins().iconst(types::I64, i as i64);
                    let boxed = self.call_rt("grasm_list_get", &[sv, idx]).unwrap();
                    match pat_i {
                        Pattern::Bind(fid)
                            if !matches!(self.cg.variant_index.get(&fid.name), Some((_, 0)))
                                && fid.name != "None" =>
                        {
                            let (v, vt) = self.unbox(boxed, t_i)?;
                            let var = self.new_var(&vt);
                            self.b.def_var(var, v);
                            bound.push((var, vt.clone()));
                            self.bind(&fid.name, var, vt);
                        }
                        Pattern::Wildcard(_) => {
                            self.call_rt("grasm_val_dec", &[boxed]);
                        }
                        _ => {
                            let (v, vt) = self.unbox(boxed, t_i)?;
                            let var = self.new_var(&vt);
                            self.b.def_var(var, v);
                            chain.push((var, vt, pat_i));
                        }
                    }
                }
                if chain.is_empty() {
                    self.b.ins().jump(body_blk, &[]);
                    self.filled = true;
                    return Ok(true);
                }
                // Each chained temp owns the +1 from `grasm_list_get`, and a
                // sub-pattern that only *tests* — a string, an integer, a
                // nullary variant — never takes it. So it has to be released
                // here, on the way out of the test either way: on success
                // before the body runs, on failure before the next arm is
                // tried. `match (req.method, head) { ("GET", "users") -> ... }`
                // leaked both components on every call, which is what a router
                // does once per request.
                let n = chain.len();
                let temps: Vec<(Variable, VType)> =
                    chain.iter().map(|(v, t, _)| (*v, t.clone())).collect();
                for (k, (var, vt, pat_i)) in chain.into_iter().enumerate() {
                    let cont = self.b.create_block();
                    let fail = self.b.create_block();
                    self.lower_pattern_test(pat_i, var, &vt, cont, fail)?;
                    // Failure: release *every* temp, not just the ones
                    // tested so far. All the components are read up front,
                    // before any of them is tested, so a mismatch at the first
                    // one still leaves the rest holding a reference — which is
                    // why the arms *after* the first two leaked and the first
                    // two did not.
                    self.goto_block(fail);
                    for (tv, tt) in temps.iter().chain(bound.iter()) {
                        if tt.boxed() {
                            let v = self.b.use_var(*tv);
                            self.call_rt("grasm_val_dec", &[v]);
                        }
                    }
                    self.b.ins().jump(next_test, &[]);
                    self.filled = true;
                    self.goto_block(cont);
                    if k + 1 == n {
                        // All components matched: release them and enter the
                        // body. A `Bind` component never reaches here — those
                        // are bound directly above and own their reference.
                        for (tv, tt) in temps.iter() {
                            if tt.boxed() {
                                let v = self.b.use_var(*tv);
                                self.call_rt("grasm_val_dec", &[v]);
                            }
                        }
                        self.b.ins().jump(body_blk, &[]);
                        self.filled = true;
                    }
                }
                Ok(true)
            }
        }
    }
}
