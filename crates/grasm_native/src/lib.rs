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

pub struct Unsupported {
    pub span: Span,
    pub what: String,
}

type CResult<T> = Result<T, Unsupported>;

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
    List(Box<VType>),
    Struct(String),
    Sum(String),
    Option(Box<VType>),
    /// `T ! E` — boxed Ok/Err variant wrapper around T.
    Fallible(Box<VType>),
    Map(Box<VType>, Box<VType>),
    Tuple(Vec<VType>),
    /// Dynamically-typed boxed value (error payloads); printable + variant-matchable.
    Any,
}

impl VType {
    fn cl(&self) -> types::Type {
        match self {
            VType::I64 => types::I64,
            VType::F64 => types::F64,
            VType::Bool => types::I8,
            VType::Unit => types::I8,
            // Boxed values travel as 64-bit pointers.
            VType::Str
            | VType::List(_)
            | VType::Struct(_)
            | VType::Sum(_)
            | VType::Option(_)
            | VType::Fallible(_)
            | VType::Map(_, _)
            | VType::Tuple(_)
            | VType::Any => types::I64,
        }
    }

    fn boxed(&self) -> bool {
        matches!(
            self,
            VType::Str
                | VType::List(_)
                | VType::Struct(_)
                | VType::Sum(_)
                | VType::Option(_)
                | VType::Fallible(_)
                | VType::Map(_, _)
                | VType::Tuple(_)
                | VType::Any
        )
    }

}

struct FnSig {
    params: Vec<VType>,
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
    cg.declare_runtime()?;
    let views: Vec<(String, &AstModule, std::collections::HashMap<String, String>)> = program
        .files
        .iter()
        .map(|f| (f.name.clone(), &f.module, f.imports.clone()))
        .collect();
    match cg.compile_program(&views) {
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

struct Codegen<'m> {
    module: &'m mut ObjectModule,
    fns: HashMap<String, FnSig>,
    runtime: HashMap<&'static str, FuncId>,
    strings: HashMap<String, cranelift_module::DataId>,
    /// struct name → ordered fields (name, type)
    structs: HashMap<String, Vec<(String, VType)>>,
    /// sum name → variants (name, field types)
    sums: HashMap<String, Vec<(String, Vec<VType>)>>,
    /// variant name → (parent sum, arity) — unambiguous per checker
    variant_index: HashMap<String, (String, usize)>,
    /// effect name → ops (op name, param types, return type)
    effects: HashMap<String, Vec<(String, Vec<VType>, VType)>>,
    /// `with` site (by span start) → op name → lifted handler FuncId
    handler_sites: HashMap<u32, Vec<(String, FuncId)>>,
    /// Lifted handler bodies to compile after the main functions.
    pending_handlers: Vec<PendingHandler>,
    current_module: String,
    /// module → (alias → target module)
    module_imports: HashMap<String, HashMap<String, String>>,
}

struct PendingHandler {
    id: FuncId,
    module: String,
    params: Vec<(String, VType)>,
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
            structs: HashMap::new(),
            sums: HashMap::new(),
            variant_index: HashMap::new(),
            effects: HashMap::new(),
            handler_sites: HashMap::new(),
            pending_handlers: Vec::new(),
            current_module: "main".to_string(),
            module_imports: HashMap::new(),
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
            ("grasm_panic_div_zero", vec![], None),
            ("grasm_panic_rem_zero", vec![], None),
            ("grasm_panic_assert", vec![], None),
            ("grasm_panic_msg", vec![ptr, i64t], None),
            ("grasm_pow_i64", vec![i64t, i64t], Some(i64t)),
            ("grasm_pow_f64", vec![f64t, f64t], Some(f64t)),
            ("grasm_val_inc", vec![i64t], None),
            ("grasm_val_dec", vec![i64t], None),
            ("grasm_str_lit", vec![ptr, i64t], Some(i64t)),
            ("grasm_str_concat", vec![i64t, i64t], Some(i64t)),
            ("grasm_str_from_i64", vec![i64t], Some(i64t)),
            ("grasm_str_from_f64", vec![f64t], Some(i64t)),
            ("grasm_str_from_bool", vec![i8t], Some(i64t)),
            ("grasm_str_len", vec![i64t], Some(i64t)),
            ("grasm_str_eq", vec![i64t, i64t], Some(i8t)),
            ("grasm_str_cmp", vec![i64t, i64t], Some(i64t)),
            ("grasm_list_new", vec![], Some(i64t)),
            ("grasm_list_push", vec![i64t, i64t], Some(i64t)),
            ("grasm_list_len", vec![i64t], Some(i64t)),
            ("grasm_list_get", vec![i64t, i64t], Some(i64t)),
            ("grasm_box_i64", vec![i64t], Some(i64t)),
            ("grasm_box_f64", vec![f64t], Some(i64t)),
            ("grasm_box_bool", vec![i8t], Some(i64t)),
            ("grasm_unbox_i64", vec![i64t], Some(i64t)),
            ("grasm_unbox_f64", vec![i64t], Some(f64t)),
            ("grasm_unbox_bool", vec![i64t], Some(i8t)),
            ("grasm_struct_new", vec![ptr, i64t], Some(i64t)),
            ("grasm_struct_push_field", vec![i64t, ptr, i64t, i64t], None),
            ("grasm_struct_get", vec![i64t, ptr, i64t], Some(i64t)),
            ("grasm_struct_set_field", vec![i64t, ptr, i64t, i64t], Some(i64t)),
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
            ("grasm_closure_new", vec![ptr, i64t], Some(i64t)),
            ("grasm_closure_push", vec![i64t, i64t], None),
            ("grasm_closure_code", vec![i64t], Some(ptr)),
            ("grasm_closure_env", vec![i64t, i64t], Some(i64t)),
            ("grasm_closure_inc", vec![i64t], None),
            ("grasm_closure_dec", vec![i64t], None),
            ("grasm_map_new", vec![], Some(i64t)),
            ("grasm_map_set", vec![i64t, i64t, i64t], Some(i64t)),
            ("grasm_map_get", vec![i64t, i64t], Some(i64t)),
            ("grasm_map_has", vec![i64t, i64t], Some(i8t)),
            ("grasm_map_delete", vec![i64t, i64t, ptr], Some(i64t)),
            ("grasm_map_len", vec![i64t], Some(i64t)),
            ("grasm_map_keys", vec![i64t], Some(i64t)),
            ("grasm_map_values", vec![i64t], Some(i64t)),
            ("grasm_str_contains", vec![i64t, i64t], Some(i8t)),
            ("grasm_str_starts_with", vec![i64t, i64t], Some(i8t)),
            ("grasm_str_ends_with", vec![i64t, i64t], Some(i8t)),
            ("grasm_str_split", vec![i64t, i64t], Some(i64t)),
            ("grasm_str_chars", vec![i64t], Some(i64t)),
            ("grasm_str_trim", vec![i64t], Some(i64t)),
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
            ("grasm_f64_floor", vec![f64t], Some(f64t)),
            ("grasm_f64_ceil", vec![f64t], Some(f64t)),
            ("grasm_f64_round", vec![f64t], Some(f64t)),
            ("grasm_f64_min", vec![f64t, f64t], Some(f64t)),
            ("grasm_f64_max", vec![f64t, f64t], Some(f64t)),
            ("grasm_handler_push", vec![ptr, i64t], None),
            ("grasm_handler_op", vec![ptr, i64t, ptr], None),
            ("grasm_handler_pop", vec![], None),
            ("grasm_handler_lookup", vec![ptr, i64t, ptr, i64t], Some(ptr)),
            ("grasm_args", vec![], Some(i64t)),
            ("grasm_input", vec![], Some(i64t)),
            ("grasm_read_file", vec![i64t], Some(i64t)),
            ("grasm_write_file", vec![i64t, i64t], Some(i64t)),
            ("grasm_file_exists", vec![i64t], Some(i8t)),
            ("grasm_set_args", vec![i64t, ptr], None),
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

    fn vtype_of(&self, te: &TypeExpr) -> CResult<VType> {
        match te {
            TypeExpr::Name { path, args, span } => {
                if path.segments.len() == 1 {
                    let n = path.segments[0].name.as_str();
                    if args.is_empty() {
                        match n {
                            "i8" | "i16" | "i32" | "i64" | "u8" | "u16" | "u32" | "u64"
                            | "usize" | "int" => return Ok(VType::I64),
                            "f32" | "f64" | "float" => return Ok(VType::F64),
                            "bool" => return Ok(VType::Bool),
                            "str" => return Ok(VType::Str),
                            _ => {}
                        }
                        if self.structs.contains_key(n) {
                            return Ok(VType::Struct(n.to_string()));
                        }
                        if self.sums.contains_key(n) {
                            return Ok(VType::Sum(n.to_string()));
                        }
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
            TypeExpr::List(inner, _) => Ok(VType::List(Box::new(self.vtype_of(inner)?))),
            TypeExpr::Tuple(items, _) if items.is_empty() => Ok(VType::Unit),
            TypeExpr::Tuple(items, _) => {
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
        for (name, m, _) in views {
            self.current_module = name.clone();
            self.declare_fns(m)?;
        }
        if !self.fns.contains_key("main::main") {
            return nope(Span::new(0, 1), "programs without a `main` function");
        }
        for (name, m, _) in views {
            self.current_module = name.clone();
            for decl in &m.decls {
                if let Decl::Fn(f) = decl {
                    self.collect_handlers_block(&f.body)?;
                }
            }
        }
        for (name, m, _) in views {
            self.current_module = name.clone();
            for decl in &m.decls {
                if let Decl::Fn(f) = decl {
                    self.compile_fn(f)?;
                }
            }
        }
        let pending = std::mem::take(&mut self.pending_handlers);
        for h in pending {
            self.current_module = h.module.clone();
            self.compile_handler(h)?;
        }
        self.emit_c_main()?;
        Ok(())
    }

    fn collect_types(&mut self, module_ast: &AstModule) -> CResult<()> {
        // Pass 0: type declarations (structs + sums; aliases unsupported).
        for decl in &module_ast.decls {
            if let Decl::Type(t) = decl {
                if !t.generics.is_empty() {
                    return nope(t.span, "generic types");
                }
                match &t.body {
                    TypeBody::Struct(_) => {
                        self.structs.insert(t.name.name.clone(), Vec::new());
                    }
                    TypeBody::Sum(_) => {
                        self.sums.insert(t.name.name.clone(), Vec::new());
                    }
                    TypeBody::Alias(_) => return nope(t.span, "type aliases"),
                }
            }
        }
        for decl in &module_ast.decls {
            if let Decl::Type(t) = decl {
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
                    TypeBody::Alias(_) => {}
                }
            }
        }
        // Effect declarations: op signatures drive handler lowering.
        for decl in &module_ast.decls {
            if let Decl::Effect(e) = decl {
                let mut ops = Vec::new();
                for op in &e.ops {
                    let mut pts = Vec::new();
                    for p in &op.params {
                        pts.push(self.vtype_of(&p.ty)?);
                    }
                    let rt = match &op.ret {
                        Some(t) => self.vtype_of(t)?,
                        None => VType::Unit,
                    };
                    ops.push((op.name.name.clone(), pts, rt));
                }
                self.effects.insert(e.name.name.clone(), ops);
            }
        }
        Ok(())
    }

    fn declare_fns(&mut self, module_ast: &AstModule) -> CResult<()> {
        for decl in &module_ast.decls {
            match decl {
                Decl::Fn(f) => {
                    if !f.generics.is_empty() {
                        return nope(f.span, "generic functions");
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
                    if f.err.is_some() {
                        ret = VType::Fallible(Box::new(ret));
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
                    let id = self
                        .module
                        .declare_function(&sym, Linkage::Local, &sig)
                        .map_err(|e| Unsupported {
                            span: f.span,
                            what: format!("(module error: {e})"),
                        })?;
                    let key = format!("{}::{}", self.current_module, f.name.name);
                    self.fns.insert(
                        key,
                        FnSig {
                            params,
                            modes,
                            ret,
                            id,
                        },
                    );
                }
                Decl::Test(_) | Decl::Import(_) | Decl::Type(_) | Decl::Effect(_) => {}
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
        let main_fallible = matches!(self.fns["main::main"].ret, VType::Fallible(_));
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
        }
        let zero = b.ins().iconst(types::I32, 0);
        b.ins().return_(&[zero]);
        b.seal_all_blocks();
        b.finalize();
        self.module
            .define_function(main_id, &mut ctx)
            .map_err(|e| Unsupported {
                span: Span::new(0, 1),
                what: format!("(define error: {e})"),
            })?;
        Ok(())
    }

    fn compile_fn(&mut self, f: &FnDecl) -> CResult<()> {
        let key = format!("{}::{}", self.current_module, f.name.name);
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
            ret: sig_ret.clone(),
            inout_slots: Vec::new(),
            filled: false,
        };

        let entry = fcx.b.create_block();
        fcx.b.append_block_params_for_function_params(entry);
        fcx.b.switch_to_block(entry);
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

        let (last, terminated) = fcx.eval_block_inner(&f.body)?;
        if !terminated {
            match (&sig_ret, last) {
                (VType::Unit, last) => {
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
                    if let VType::Fallible(ok_t) = want.clone() {
                        let mut inner = v;
                        if *ok_t == VType::F64 && vt == VType::I64 {
                            inner = fcx.b.ins().fcvt_from_sint(types::F64, inner);
                        }
                        let boxed = fcx.box_value(inner, &ok_t);
                        v = fcx.call_rt("grasm_ok_new", &[boxed]).unwrap();
                    } else if *want == VType::F64 && vt == VType::I64 {
                        v = fcx.b.ins().fcvt_from_sint(types::F64, v);
                    }
                    fcx.emit_scope_decs_all();
                    fcx.b.ins().return_(&[v]);
                }
                (VType::Fallible(_), None) => {
                    // Unit-valued fallible function: Ok(()).
                    let z = fcx.b.ins().iconst(types::I64, 0);
                    let boxed = fcx.call_rt("grasm_box_i64", &[z]).unwrap();
                    let v = fcx.call_rt("grasm_ok_new", &[boxed]).unwrap();
                    fcx.emit_scope_decs_all();
                    fcx.b.ins().return_(&[v]);
                }
                (_, None) => {
                    return nope(f.span, "a missing return value here");
                }
            }
        }
        fcx.b.seal_all_blocks();
        fcx.b.finalize();

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
            self.collect_handlers_stmt(st)?;
        }
        Ok(())
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
                let Expr::StructLit { fields, .. } = handler else {
                    return nope(handler.span(), "handlers that aren't struct literals");
                };
                let mut site = Vec::new();
                for (fname, fexpr) in fields {
                    let Some((_, param_ts, ret_t)) =
                        ops.iter().find(|(o, _, _)| *o == fname.name).cloned()
                    else {
                        return nope(fname.span, format!("the operation `{}`", fname.name));
                    };
                    let Expr::Closure {
                        params: cparams,
                        body: cbody,
                        ..
                    } = fexpr
                    else {
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
                            span: *span,
                            what: format!("(module error: {e})"),
                        })?;
                    let named_params: Vec<(String, VType)> = cparams
                        .iter()
                        .map(|p| p.name.clone())
                        .zip(param_ts.iter().cloned())
                        .collect();
                    self.pending_handlers.push(PendingHandler {
                        id,
                        module: self.current_module.clone(),
                        params: named_params,
                        ret: ret_t,
                        body: (**cbody).clone(),
                    });
                    site.push((fname.name.clone(), id));
                }
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
            Expr::Block(b) => self.collect_handlers_block(b),
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
            _ => Ok(()),
        }
    }

    fn compile_handler(&mut self, h: PendingHandler) -> CResult<()> {
        let mut ctx = self.module.make_context();
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
            ret: h.ret.clone(),
            inout_slots: Vec::new(),
            filled: false,
        };
        let entry = fcx.b.create_block();
        fcx.b.append_block_params_for_function_params(entry);
        fcx.b.switch_to_block(entry);
        fcx.push_scope();
        let ps: Vec<Value> = fcx.b.block_params(entry).to_vec();
        for (i, (pname, pt)) in h.params.iter().enumerate() {
            let var = fcx.new_var(pt);
            fcx.b.def_var(var, ps[i]);
            fcx.bind(pname, var, pt.clone());
        }
        let val = fcx.eval_expr(&h.body)?;
        match (&h.ret, val) {
            (VType::Unit, v) => {
                if let Some((x, xt)) = v {
                    fcx.dec_if_boxed(x, &xt);
                }
                fcx.emit_scope_decs_all();
                fcx.b.ins().return_(&[]);
            }
            (want, Some((mut x, xt))) => {
                if *want == VType::F64 && xt == VType::I64 {
                    x = fcx.b.ins().fcvt_from_sint(types::F64, x);
                }
                fcx.emit_scope_decs_all();
                fcx.b.ins().return_(&[x]);
            }
            (_, None) => {
                return nope(h.body.span(), "a handler op with no value");
            }
        }
        fcx.b.seal_all_blocks();
        fcx.b.finalize();
        self.module
            .define_function(h.id, &mut ctx)
            .map_err(|e| Unsupported {
                span: h.body.span(),
                what: format!("(define error: {e})"),
            })?;
        Ok(())
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
    loops: Vec<(Block, Block)>,
    ret: VType,
    /// (caller pointer, local var, type) for `inout` params — stored back
    /// through the pointer before every return.
    inout_slots: Vec<(Value, Variable, VType)>,
    /// True when the current block already ends in a terminator (a `break`,
    /// `continue`, `return`, or `fail` was just emitted). Cleared whenever we
    /// switch to a fresh block.
    filled: bool,
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

    /// Before a `return`: release every boxed local in every scope.
    fn emit_scope_decs_all(&mut self) {
        self.emit_inout_writeback();
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

    /// Switches to `blk`, resetting the terminator flag.
    fn goto_block(&mut self, blk: Block) {
        self.b.switch_to_block(blk);
        self.filled = false;
    }

    fn is_filled(&self) -> bool {
        self.filled
    }

    fn call_rt(&mut self, name: &'static str, args: &[Value]) -> Option<Value> {
        let id = self.cg.runtime[name];
        let fref = self.cg.module.declare_func_in_func(id, self.b.func);
        let inst = self.b.ins().call(fref, args);
        self.b.inst_results(inst).first().copied()
    }

    fn dec_if_boxed(&mut self, v: Value, vt: &VType) {
        if vt.boxed() {
            self.call_rt("grasm_val_dec", &[v]);
        }
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
        let mut last: Option<(Value, VType)> = None;
        let mut terminated = false;
        for (i, stmt) in blk.stmts.iter().enumerate() {
            if terminated {
                break;
            }
            let is_last = i + 1 == blk.stmts.len();
            match self.eval_stmt(stmt, is_last)? {
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
        Ok((last, terminated))
    }

    fn eval_stmt(&mut self, s: &Stmt, is_last: bool) -> CResult<StmtOut> {
        match s {
            Stmt::Let {
                name, ty, init, ..
            } => {
                // An annotation gives empty collection literals their type.
                let expected = match ty {
                    Some(te) => Some(self.cg.vtype_of(te)?),
                    None => None,
                };
                let (mut v, mut vt) = self.eval_expr_expecting(init, expected.as_ref())?;
                if let Some(te) = ty {
                    let want = self.cg.vtype_of(te)?;
                    if want == VType::F64 && vt == VType::I64 {
                        v = self.b.ins().fcvt_from_sint(types::F64, v);
                    }
                    vt = want;
                }
                let var = self.new_var(&vt);
                self.b.def_var(var, v);
                self.bind(&name.name, var, vt);
                Ok(StmtOut::Value(None))
            }
            Stmt::Assign {
                target, op, value, ..
            } => {
                // Field/index places (any depth, plain or compound).
                if matches!(target, Expr::Field { .. } | Expr::Index { .. }) {
                    let (root, path) = self.place_path(target)?;
                    let Some((var, root_t)) = self.lookup(&root.name) else {
                        return nope(root.span, "assignment to unknown variable");
                    };
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
                    let updated = self.write_place(cur, &path, &root_t, boxed)?;
                    self.b.def_var(var, updated);
                    return Ok(StmtOut::Value(None));
                }
                let Expr::Ident(id) = target else {
                    return nope(target.span(), "assignment to fields/indexes");
                };
                let Some((var, vt)) = self.lookup(&id.name) else {
                    return nope(id.span, "assignment to unknown variable");
                };
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
                let v = self.eval_expr(e)?;
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
                        let expect = match &self.ret {
                            VType::Fallible(ok) => (**ok).clone(),
                            other => other.clone(),
                        };
                        let (mut val, vt) = self.eval_expr_expecting(e, Some(&expect))?;
                        if self.ret == VType::F64 && vt == VType::I64 {
                            val = self.b.ins().fcvt_from_sint(types::F64, val);
                        }
                        if let VType::Fallible(ok_t) = self.ret.clone() {
                            let mut inner = val;
                            if *ok_t == VType::F64 && vt == VType::I64 {
                                inner = self.b.ins().fcvt_from_sint(types::F64, inner);
                            }
                            let boxed = self.box_value(inner, &ok_t);
                            val = self.call_rt("grasm_ok_new", &[boxed]).unwrap();
                        }
                        // Returned value stays owned; release the rest.
                        // (Locals holding the same box are fine: their dec is
                        // balanced by the +1 the Ident eval added.)
                        self.emit_scope_decs_all();
                        self.b.ins().return_(&[val]);
                self.filled = true;
                    }
                    None => {
                        if let VType::Fallible(ok_t) = self.ret.clone() {
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
                let Some(&(_, break_blk)) = self.loops.last() else {
                    return nope(*span, "break outside loop");
                };
                self.b.ins().jump(break_blk, &[]);
                self.filled = true;
                Ok(StmtOut::Terminated)
            }
            Stmt::Continue(span) => {
                let Some(&(cont_blk, _)) = self.loops.last() else {
                    return nope(*span, "continue outside loop");
                };
                self.b.ins().jump(cont_blk, &[]);
                self.filled = true;
                Ok(StmtOut::Terminated)
            }
            Stmt::While { cond, body, .. } => {
                let header = self.b.create_block();
                let body_blk = self.b.create_block();
                let exit = self.b.create_block();
                self.b.ins().jump(header, &[]);
                self.filled = true;
                self.goto_block(header);
                let (c, ct) = self.eval_expr_val(cond)?;
                if ct != VType::Bool {
                    return nope(cond.span(), "non-bool while condition");
                }
                self.b.ins().brif(c, body_blk, &[], exit, &[]);
                self.filled = true;
                self.goto_block(body_blk);
                self.loops.push((header, exit));
                let (_, mut terminated) = self.eval_block_inner(body)?;
                self.loops.pop();
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
            Stmt::Parallel { span, .. } | Stmt::Spawn { span, .. } => nope(
                *span,
                "structured concurrency (`parallel`/`spawn`) — the interpreter runs these",
            ),
            Stmt::With {
                effect,
                body,
                span,
                ..
            } => {
                let ename = effect.dotted();
                let Some(site) = self.cg.handler_sites.get(&span.start).cloned() else {
                    return nope(*span, "this handler block");
                };
                let (ep, el) = self.str_ptr_len(&ename);
                self.call_rt("grasm_handler_push", &[ep, el]);
                let ptr_ty = self.cg.module.target_config().pointer_type();
                for (opname, fid) in site {
                    let (op_p, op_l) = self.str_ptr_len(&opname);
                    let fref = self.cg.module.declare_func_in_func(fid, self.b.func);
                    let addr = self.b.ins().func_addr(ptr_ty, fref);
                    self.call_rt("grasm_handler_op", &[op_p, op_l, addr]);
                }
                let (_, mut terminated) = self.eval_block_inner(body)?;
                terminated |= self.is_filled();
                if !terminated {
                    self.call_rt("grasm_handler_pop", &[]);
                }
                Ok(StmtOut::Value(None))
            }
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
            self.loops.push((step, exit));
            let (_, mut terminated) = self.eval_block_inner(body)?;
            self.loops.pop();
            terminated |= self.is_filled();
            self.pop_scope_no_decs(); // loop var is scalar
            if !terminated {
                self.b.ins().jump(step, &[]);
                self.filled = true;
            }
            self.goto_block(step);
            let iv2 = self.b.use_var(ivar);
            let one = self.b.ins().iconst(types::I64, 1);
            let next = self.b.ins().iadd(iv2, one);
            self.b.def_var(ivar, next);
            self.b.ins().jump(header, &[]);
            self.filled = true;
            self.goto_block(exit);
            return Ok(StmtOut::Value(None));
        }

        // List iteration: index-based over an owned temp.
        let (list_v, list_t) = self.eval_expr_val(iter)?;
        let VType::List(elem_t) = list_t.clone() else {
            return nope(iter.span(), "for over non-range, non-list iterables");
        };
        let list_var = self.new_var(&list_t);
        self.b.def_var(list_var, list_v);
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
        let boxed_elem = self.call_rt("grasm_list_get", &[lv2, iv2]).unwrap();
        let (elem_val, elem_vt) = self.unbox(boxed_elem, &elem_t)?;
        let evar = self.new_var(&elem_vt);
        self.b.def_var(evar, elem_val);
        self.push_scope();
        self.bind(&loop_var.name, evar, elem_vt.clone());
        self.loops.push((step, exit));
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
        // Release the list temp.
        let lv3 = self.b.use_var(list_var);
        self.call_rt("grasm_val_dec", &[lv3]);
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
            VType::Unit => {
                let z = self.b.ins().iconst(types::I64, 0);
                self.call_rt("grasm_box_i64", &[z]).unwrap()
            }
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
            None => nope(e.span(), "using a unit value"),
        }
    }

    /// Like `eval_expr_val`, but an expected type supplies the element type
    /// for empty collection literals (`let xs: [i64] = []`).
    fn eval_expr_expecting(
        &mut self,
        e: &Expr,
        expected: Option<&VType>,
    ) -> CResult<(Value, VType)> {
        if let (Expr::List(items, _), Some(VType::List(elem_t))) = (e, expected) {
            if items.is_empty() {
                let v = self.call_rt("grasm_list_new", &[]).unwrap();
                return Ok((v, VType::List(elem_t.clone())));
            }
        }
        if let (Expr::Call { callee, args, .. }, Some(VType::Map(k, v))) = (e, expected) {
            if args.is_empty() {
                if let Expr::Ident(id) = &**callee {
                    if id.name == "map" && self.lookup("map").is_none() {
                        let m = self.call_rt("grasm_map_new", &[]).unwrap();
                        return Ok((m, VType::Map(k.clone(), v.clone())));
                    }
                }
            }
        }
        self.eval_expr_val(e)
    }

    fn eval_expr(&mut self, e: &Expr) -> CResult<Option<(Value, VType)>> {
        match e {
            Expr::Int(v, _) => Ok(Some((self.b.ins().iconst(types::I64, *v), VType::I64))),
            Expr::Float(v, _) => Ok(Some((self.b.ins().f64const(*v), VType::F64))),
            Expr::Bool(v, _) => Ok(Some((
                self.b.ins().iconst(types::I8, i64::from(*v)),
                VType::Bool,
            ))),
            Expr::Ident(id) => match self.lookup(&id.name) {
                Some((var, vt)) => {
                    let v = self.b.use_var(var);
                    if vt.boxed() {
                        self.call_rt("grasm_val_inc", &[v]);
                    }
                    Ok(Some((v, vt)))
                }
                None if self.lookup_fn(&id.name).is_some() => nope(
                    id.span,
                    "function values (passing a function as data)",
                ),
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
                    nope(id.span, format!("the name `{}` here", id.name))
                }
            },
            Expr::Str(pieces, _) => Ok(Some(self.build_string(pieces)?)),
            Expr::List(items, span) => {
                let elem_t = self.list_elem_type(items, *span)?;
                let mut list = self.call_rt("grasm_list_new", &[]).unwrap();
                for item in items {
                    let (v, vt) = self.eval_expr_val(item)?;
                    let mut bv = self.box_value(v, &vt);
                    // Int-literal → f64 adaptation inside float lists.
                    if elem_t == VType::F64 && vt == VType::I64 {
                        let f = self.b.ins().fcvt_from_sint(types::F64, v);
                        bv = self.call_rt("grasm_box_f64", &[f]).unwrap();
                    }
                    list = self.call_rt("grasm_list_push", &[list, bv]).unwrap();
                }
                Ok(Some((list, VType::List(Box::new(elem_t)))))
            }
            Expr::Index { recv, index, span } => {
                let (rv, rt) = self.eval_expr_val(recv)?;
                let VType::List(elem_t) = rt else {
                    return nope(*span, "indexing non-lists");
                };
                let (iv, it) = self.eval_expr_val(index)?;
                if it != VType::I64 {
                    return nope(index.span(), "non-integer index");
                }
                let boxed = self.call_rt("grasm_list_get", &[rv, iv]).unwrap();
                self.call_rt("grasm_val_dec", &[rv]);
                let (v, vt) = self.unbox(boxed, &elem_t)?;
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
                let (lv, lt) = self.eval_expr_val(lhs)?;
                let (rv, rt) = self.eval_expr_val(rhs)?;
                Ok(Some(self.binop_values(*op, lv, lt, rv, rt, *span)?))
            }
            Expr::If {
                cond,
                then_block,
                else_branch,
                ..
            } => self.eval_if(cond, then_block, else_branch.as_deref()),
            Expr::Call { callee, args, span } => self.eval_call(callee, args, *span),
            Expr::Block(b) => {
                let (last, terminated) = self.eval_block_inner(b)?;
                if terminated {
                    Ok(None)
                } else {
                    Ok(last)
                }
            }
            Expr::Tuple(items, _) => {
                let mut list = self.call_rt("grasm_list_new", &[]).unwrap();
                let mut ts = Vec::with_capacity(items.len());
                for item in items {
                    let (v, vt) = self.eval_expr_val(item)?;
                    let bx = self.box_value(v, &vt);
                    list = self.call_rt("grasm_list_push", &[list, bx]).unwrap();
                    ts.push(vt);
                }
                Ok(Some((list, VType::Tuple(ts))))
            }
            Expr::StructLit { name, fields, span } => {
                self.eval_struct_lit(name, fields, *span).map(Some)
            }
            Expr::Field { recv, name, span } => self.eval_field(recv, name, *span),
            Expr::Match {
                scrutinee, arms, span,
            } => self.eval_match(scrutinee, arms, *span),
            Expr::Try { expr, span } => self.eval_try(expr, *span).map(Some),
            Expr::Closure { span, .. } => nope(
                *span,
                "closures outside `map`/`filter` (those are inlined natively)",
            ),
            other => nope(
                other.span(),
                match other {
                    Expr::Range { .. } => "range values outside for",
                    Expr::Closure { .. } => "closures",
                    _ => "this construct",
                },
            ),
        }
    }

    fn list_elem_type(&mut self, items: &[Expr], span: Span) -> CResult<VType> {
        let Some(first) = items.first() else {
            return nope(span, "empty list literals (annotate via a function param for now)");
        };
        self.probe_type(first)
    }

    /// Interpolated string as a value: fold pieces with concat.
    fn build_string(&mut self, pieces: &[StrPiece]) -> CResult<(Value, VType)> {
        let mut acc: Option<Value> = None;
        for piece in pieces {
            let part: Value = match piece {
                StrPiece::Text(t) => {
                    let (p, l) = self.str_ptr_len(t);
                    self.call_rt("grasm_str_lit", &[p, l]).unwrap()
                }
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
                        VType::Unit => return nope(inner.span(), "interpolating unit"),
                        _ => unreachable!(),
                    }
                }
            };
            acc = Some(match acc {
                None => part,
                Some(prev) => {
                    let joined = self.call_rt("grasm_str_concat", &[prev, part]).unwrap();
                    self.call_rt("grasm_val_dec", &[prev]);
                    self.call_rt("grasm_val_dec", &[part]);
                    joined
                }
            });
        }
        let v = match acc {
            Some(v) => v,
            None => {
                let (p, l) = self.str_ptr_len("");
                self.call_rt("grasm_str_lit", &[p, l]).unwrap()
            }
        };
        Ok((v, VType::Str))
    }

    fn eval_method(
        &mut self,
        recv: &Expr,
        name: &Ident,
        args: &[Expr],
        span: Span,
    ) -> CResult<Option<(Value, VType)>> {
        // Mutating map/list methods on a variable receiver.
        if matches!(name.name.as_str(), "set" | "delete" | "pop" | "sort") {
            if let Expr::Ident(id) = recv {
                if let Some((var, vt)) = self.lookup(&id.name) {
                    match (&vt, name.name.as_str()) {
                        (VType::Map(k_t, v_t), "set") => {
                            let (kt, vt2) = (k_t.clone(), v_t.clone());
                            let (kv, kvt) = self.eval_expr_val(&args[0])?;
                            let kb = self.box_value(kv, &kvt);
                            let (vv, vvt) = self.eval_expr_val(&args[1])?;
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
                        (VType::List(_), "sort") => {
                            let cur = self.b.use_var(var);
                            let np = self.call_rt("grasm_list_sort", &[cur]).unwrap();
                            self.b.def_var(var, np);
                            return Ok(None);
                        }
                        _ => {}
                    }
                }
            }
        }
        // Mutating list push on a nested place: `bag.items.push(v)`.
        if name.name == "push" && matches!(recv, Expr::Field { .. } | Expr::Index { .. }) {
            let (root, path) = self.place_path(recv)?;
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
            let list_val = self.read_place_boxed(cur, &path, &root_t)?;
            let newlist = self.call_rt("grasm_list_push", &[list_val, boxed]).unwrap();
            let cur2 = self.b.use_var(var);
            let updated = self.write_place(cur2, &path, &root_t, newlist)?;
            self.b.def_var(var, updated);
            return Ok(None);
        }
        // Mutating list push on a variable.
        if name.name == "push" {
            let Expr::Ident(id) = recv else {
                return nope(span, "push on non-variable receivers");
            };
            let Some((var, vt)) = self.lookup(&id.name) else {
                return nope(id.span, "push on unknown variable");
            };
            let VType::List(elem_t) = vt else {
                return nope(span, "push on non-lists");
            };
            let (av, avt) = self.eval_expr_val(&args[0])?;
            let mut boxed = self.box_value(av, &avt);
            if *elem_t == VType::F64 && avt == VType::I64 {
                let f = self.b.ins().fcvt_from_sint(types::F64, av);
                boxed = self.call_rt("grasm_box_f64", &[f]).unwrap();
            }
            let cur = self.b.use_var(var);
            let newp = self.call_rt("grasm_list_push", &[cur, boxed]).unwrap();
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
                    let qual = format!("{target_mod}::{}", name.name);
                    if let Some(sig) = self.cg.fns.get(&qual) {
                        let (fid, params, modes, ret) = (
                            sig.id,
                            sig.params.clone(),
                            sig.modes.clone(),
                            sig.ret.clone(),
                        );
                        if modes.contains(&ParamMode::Inout) {
                            return nope(span, "`inout` across module boundaries");
                        }
                        let mut vals = Vec::with_capacity(args.len());
                        for (a, want) in args.iter().zip(&params) {
                            let (mut v, vt) = self.eval_expr_val(a)?;
                            if *want == VType::F64 && vt == VType::I64 {
                                v = self.b.ins().fcvt_from_sint(types::F64, v);
                            }
                            vals.push(v);
                        }
                        let fref = self.cg.module.declare_func_in_func(fid, self.b.func);
                        let inst = self.b.ins().call(fref, &vals);
                        let results: Vec<Value> = self.b.inst_results(inst).to_vec();
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
                    let mut sig = self.cg.module.make_signature();
                    for p in &param_ts {
                        sig.params.push(AbiParam::new(p.cl()));
                    }
                    if ret_t != VType::Unit {
                        sig.returns.push(AbiParam::new(ret_t.cl()));
                    }
                    let sig_ref = self.b.import_signature(sig);
                    let mut vals = Vec::with_capacity(args.len());
                    for (a, want) in args.iter().zip(&param_ts) {
                        let (mut v, vt) = self.eval_expr_val(a)?;
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
                _ => {}
            }
        }
        self.dispatch_method(rv, rt, recv, name, args, span)
    }

    /// Full builtin-method surface over an owned receiver value. `recv` is the
    /// receiver *expression* (needed for `inout` UFCS calls).
    #[allow(clippy::too_many_arguments)]
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
            (VType::Str, "chars") => {
                let v = self.call_rt("grasm_str_chars", &[rv]).unwrap();
                self.call_rt("grasm_val_dec", &[rv]);
                (v, VType::List(Box::new(VType::Str)))
            }
            (VType::Str, "trim" | "to_upper" | "to_lower") => {
                let f = match m {
                    "trim" => "grasm_str_trim",
                    "to_upper" => "grasm_str_upper",
                    _ => "grasm_str_lower",
                };
                let v = self.call_rt(f, &[rv]).unwrap();
                self.call_rt("grasm_val_dec", &[rv]);
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
            (VType::List(e), "map" | "filter") => {
                let elem_t = (**e).clone();
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
            // ---- numeric ----
            (VType::I64, "as_f64") => {
                (self.b.ins().fcvt_from_sint(types::F64, rv), VType::F64)
            }
            (VType::I64, "to_str") => (
                self.call_rt("grasm_str_from_i64", &[rv]).unwrap(),
                VType::Str,
            ),
            (VType::I64, "abs") => (self.call_rt("grasm_i64_abs", &[rv]).unwrap(), VType::I64),
            (VType::I64, "min" | "max") => {
                let a = arg_scalar!(0, VType::I64);
                let f = if m == "min" {
                    "grasm_i64_min"
                } else {
                    "grasm_i64_max"
                };
                (self.call_rt(f, &[rv, a]).unwrap(), VType::I64)
            }
            (VType::F64, "as_i64") => (self.b.ins().fcvt_to_sint(types::I64, rv), VType::I64),
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
                // UFCS: x.f(a) → f(x, a).
                if let Some(sig) = self.lookup_fn(&name.name) {
                    let (fid, params, modes, ret) = (
                        sig.id,
                        sig.params.clone(),
                        sig.modes.clone(),
                        sig.ret.clone(),
                    );
                    let ptr_ty = self.cg.module.target_config().pointer_type();
                    let mut vals: Vec<Value> = Vec::with_capacity(args.len() + 1);
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
                    return Ok(if ret == VType::Unit {
                        None
                    } else {
                        Some((results[0], ret))
                    });
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
        if lt == VType::I64 && rt == VType::F64 {
            lv = self.b.ins().fcvt_from_sint(types::F64, lv);
            lt = VType::F64;
        }
        if rt == VType::I64 && lt == VType::F64 {
            rv = self.b.ins().fcvt_from_sint(types::F64, rv);
            rt = VType::F64;
        }
        if lt != rt {
            return nope(span, "mixed-type operation");
        }
        use BinaryOp::*;
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
            (VType::I64, Div) => {
                self.guard_zero(rv, "grasm_panic_div_zero");
                (self.b.ins().sdiv(lv, rv), VType::I64)
            }
            (VType::I64, Rem) => {
                self.guard_zero(rv, "grasm_panic_rem_zero");
                (self.b.ins().srem(lv, rv), VType::I64)
            }
            (VType::I64, Pow) => (
                self.call_rt("grasm_pow_i64", &[lv, rv]).unwrap(),
                VType::I64,
            ),
            (VType::F64, Add) => (self.b.ins().fadd(lv, rv), VType::F64),
            (VType::F64, Sub) => (self.b.ins().fsub(lv, rv), VType::F64),
            (VType::F64, Mul) => (self.b.ins().fmul(lv, rv), VType::F64),
            (VType::F64, Div) => (self.b.ins().fdiv(lv, rv), VType::F64),
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
            (VType::Bool, Eq) => (self.b.ins().icmp(IntCC::Equal, lv, rv), VType::Bool),
            (VType::Bool, NotEq) => (self.b.ins().icmp(IntCC::NotEqual, lv, rv), VType::Bool),
            _ => return nope(span, "this operator on these types"),
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

        let mut merge_ty: Option<VType> = None;
        if let (Some((_, vt)), Some(_)) = (&then_val, &else_branch) {
            if *vt != VType::Unit {
                merge_ty = Some(vt.clone());
            }
        }
        if let Some(vt) = &merge_ty {
            self.b.append_block_param(merge, vt.cl());
        }
        if !then_term {
            match (&merge_ty, &then_val) {
                (Some(_), Some((v, _))) => {
                    let v = *v;
                    self.b.ins().jump(merge, &[v]);
                    self.filled = true;
                }
                (Some(want), None) => {
                    let z = match want {
                        VType::F64 => self.b.ins().f64const(0.0),
                        VType::Bool => self.b.ins().iconst(types::I8, 0),
                        other => self.b.ins().iconst(other.cl(), 0),
                    };
                    self.b.ins().jump(merge, &[z]);
                    self.filled = true;
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
                    self.filled = true;
                }
            }
        }

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
        if !else_term {
            match (&merge_ty, &else_val) {
                (Some(want), Some((v, vt))) => {
                    let mut v = *v;
                    if *want == VType::F64 && *vt == VType::I64 {
                        v = self.b.ins().fcvt_from_sint(types::F64, v);
                    }
                    self.b.ins().jump(merge, &[v]);
                    self.filled = true;
                }
                (Some(want), None) => {
                    // The branch diverged (`fail(..)`, `panic(..)`). Feed the
                    // merge a placeholder so the CFG stays well-formed; the
                    // value is unreachable.
                    let z = match want {
                        VType::F64 => self.b.ins().f64const(0.0),
                        VType::Bool => self.b.ins().iconst(types::I8, 0),
                        other => self.b.ins().iconst(other.cl(), 0),
                    };
                    self.b.ins().jump(merge, &[z]);
                    self.filled = true;
                }
                _ => {
                    if let Some((v, vt)) = &else_val {
                        if vt.boxed() {
                            let v = *v;
                            let vt = vt.clone();
                            self.dec_if_boxed(v, &vt);
                        }
                    }
                    self.b.ins().jump(merge, &[]);
                    self.filled = true;
                }
            }
        }

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
        span: Span,
    ) -> CResult<Option<(Value, VType)>> {
        let Expr::Ident(id) = callee else {
            return nope(callee.span(), "indirect calls");
        };
        if self.lookup(&id.name).is_some() {
            return nope(id.span, "calling closure values");
        }
        match id.name.as_str() {
            "print" => {
                self.lower_print(&args[0])?;
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
                let (a, at) = self.eval_expr_val(&args[0])?;
                let (bv, bt) = self.eval_expr_val(&args[1])?;
                let (eq, _) = self.binop_values(BinaryOp::Eq, a, at, bv, bt, span)?;
                self.assert_true(eq);
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
                return nope(span, "panic with a non-literal message");
            }
            "args" => {
                let v = self.call_rt("grasm_args", &[]).unwrap();
                return Ok(Some((v, VType::List(Box::new(VType::Str)))));
            }
            "input" => {
                let v = self.call_rt("grasm_input", &[]).unwrap();
                return Ok(Some((v, VType::Str)));
            }
            "read_file" => {
                let (pv, _) = self.eval_expr_val(&args[0])?;
                let v = self.call_rt("grasm_read_file", &[pv]).unwrap();
                self.call_rt("grasm_val_dec", &[pv]);
                return Ok(Some((v, VType::Fallible(Box::new(VType::Str)))));
            }
            "write_file" => {
                let (pv, _) = self.eval_expr_val(&args[0])?;
                let (cv, _) = self.eval_expr_val(&args[1])?;
                let v = self.call_rt("grasm_write_file", &[pv, cv]).unwrap();
                self.call_rt("grasm_val_dec", &[pv]);
                self.call_rt("grasm_val_dec", &[cv]);
                return Ok(Some((v, VType::Fallible(Box::new(VType::Unit)))));
            }
            "file_exists" => {
                let (pv, _) = self.eval_expr_val(&args[0])?;
                let v = self.call_rt("grasm_file_exists", &[pv]).unwrap();
                self.call_rt("grasm_val_dec", &[pv]);
                return Ok(Some((v, VType::Bool)));
            }
            "map" => {
                let m = self.call_rt("grasm_map_new", &[]).unwrap();
                return Ok(Some((
                    m,
                    VType::Map(Box::new(VType::Any), Box::new(VType::Any)),
                )));
            }
            "fail" => {
                let (ev, evt) = self.eval_expr_val(&args[0])?;
                if !evt.boxed() {
                    return nope(args[0].span(), "failing with unboxed values");
                }
                let wrapped = self.call_rt("grasm_err_new", &[ev]).unwrap();
                self.emit_scope_decs_all();
                self.b.ins().return_(&[wrapped]);
                self.filled = true;
                let dead = self.b.create_block();
                self.goto_block(dead);
                // fail() has bottom type; produce a dummy unit.
                return Ok(None);
            }
            _ => {}
        }
        // Variant constructor call.
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
        let Some(sig) = self.lookup_fn(&id.name) else {
            return nope(id.span, format!("the function `{}` here", id.name));
        };
        let (fid, params, modes, ret) = (
            sig.id,
            sig.params.clone(),
            sig.modes.clone(),
            sig.ret.clone(),
        );
        let ptr_ty = self.cg.module.target_config().pointer_type();
        let mut vals = Vec::with_capacity(args.len());
        // (arg index, stack slot, variable to refresh) for inout write-back.
        let mut inout_args: Vec<(usize, cranelift_codegen::ir::StackSlot, Variable, VType)> =
            Vec::new();
        for (i, (a, want)) in args.iter().zip(&params).enumerate() {
            if modes.get(i).copied() == Some(ParamMode::Inout) {
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
            // Boxed args transfer ownership to the callee (it decs at exit).
            vals.push(v);
        }
        let fref = self.cg.module.declare_func_in_func(fid, self.b.func);
        let inst = self.b.ins().call(fref, &vals);
        let results: Vec<Value> = self.b.inst_results(inst).to_vec();
        for (_, slot, var, vt) in inout_args {
            let updated = self.b.ins().stack_load(vt.cl(), slot, 0);
            self.b.def_var(var, updated);
        }
        if ret == VType::Unit {
            Ok(None)
        } else {
            Ok(Some((results[0], ret)))
        }
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
                for piece in pieces {
                    match piece {
                        StrPiece::Text(t) => {
                            if !t.is_empty() {
                                let (p, l) = self.str_ptr_len(t);
                                self.call_rt("grasm_out_text", &[p, l]);
                            }
                        }
                        StrPiece::Expr(inner) => {
                            let (v, vt) = self.eval_expr_val(inner)?;
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
                                VType::Unit => return nope(inner.span(), "printing unit"),
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
                    VType::Unit => return nope(other.span(), "printing unit"),
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

    fn eval_struct_lit(
        &mut self,
        name: &Path,
        fields: &[(Ident, Expr)],
        span: Span,
    ) -> CResult<(Value, VType)> {
        let sname = name.segments[0].name.clone();
        let (np, nl) = self.str_ptr_len(&sname);
        let sv = self.call_rt("grasm_struct_new", &[np, nl]).unwrap();
        for (fname, fexpr) in fields {
            let ft = self.struct_field_type(&sname, &fname.name, span)?;
            let (mut fv, fvt) = self.eval_expr_expecting(fexpr, Some(&ft))?;
            if ft == VType::F64 && fvt == VType::I64 {
                fv = self.b.ins().fcvt_from_sint(types::F64, fv);
            }
            let boxed = self.box_value(fv, &ft);
            let (fp, fl) = self.str_ptr_len(&fname.name);
            self.call_rt("grasm_struct_push_field", &[sv, fp, fl, boxed]);
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
        let (rv, rt) = self.eval_expr_val(recv)?;
        let VType::Struct(sname) = rt else {
            return nope(span, "field access on non-structs");
        };
        let ft = self.struct_field_type(&sname, &name.name, span)?;
        let (fp, fl) = self.str_ptr_len(&name.name);
        let boxed = self.call_rt("grasm_struct_get", &[rv, fp, fl]).unwrap();
        self.call_rt("grasm_val_dec", &[rv]);
        let (v, vt) = self.unbox(boxed, &ft)?;
        Ok(Some((v, vt)))
    }

    fn eval_try(&mut self, inner: &Expr, span: Span) -> CResult<(Value, VType)> {
        let (fv, ft) = self.eval_expr_val(inner)?;
        let VType::Fallible(ok_t) = ft else {
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
        if matches!(self.ret, VType::Fallible(_)) {
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
        let (lv, lt) = self.eval_expr_val(lhs)?;
        let VType::Option(payload_t) = lt else {
            return nope(span, "?? on non-Option values");
        };
        self.coalesce_values(lv, &payload_t, rhs)
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
        let (mut rv, rvt) = self.eval_expr_val(rhs)?;
        if out_t == VType::F64 && rvt == VType::I64 {
            rv = self.b.ins().fcvt_from_sint(types::F64, rv);
        }
        self.b.ins().jump(merge, &[rv]);
        self.filled = true;

        self.goto_block(merge);
        let out = self.b.block_params(merge)[0];
        Ok((out, out_t))
    }

    /// Static type probe without emitting code. Best-effort but covers the
    /// realistic expression shapes; used for merge-block param types.
    fn probe_type(&mut self, e: &Expr) -> CResult<VType> {
        use BinaryOp::*;
        Ok(match e {
            Expr::Int(..) => VType::I64,
            Expr::Float(..) => VType::F64,
            Expr::Bool(..) => VType::Bool,
            Expr::Str(..) => VType::Str,
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
                    // Arithmetic: float wins the promotion.
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
            Expr::Ident(id) => match self.lookup(&id.name) {
                Some((_, vt)) => vt,
                None => {
                    if let Some((parent, 0)) = self.cg.variant_index.get(&id.name).cloned() {
                        self.sum_vtype(&parent)
                    } else if id.name == "None" {
                        VType::Option(Box::new(VType::Any))
                    } else if self.lookup_fn(&id.name).is_some() {
                        return nope(
                            id.span,
                            "function values (passing a function as data)",
                        );
                    } else {
                        return nope(id.span, "inferring this type");
                    }
                }
            },
            Expr::Call { callee, args, .. } => {
                if let Expr::Ident(id) = &**callee {
                    if let Some(sig) = self.lookup_fn(&id.name) {
                        return Ok(sig.ret.clone());
                    }
                    if let Some((parent, _)) = self.cg.variant_index.get(&id.name).cloned() {
                        return Ok(self.sum_vtype(&parent));
                    }
                    if id.name == "Some" && args.len() == 1 {
                        let inner = self.probe_type(&args[0])?;
                        return Ok(VType::Option(Box::new(inner)));
                    }
                }
                return nope(e.span(), "inferring this type");
            }
            Expr::Tuple(items, _) => {
                let mut ts = Vec::with_capacity(items.len());
                for i in items {
                    ts.push(self.probe_type(i)?);
                }
                VType::Tuple(ts)
            }
            Expr::StructLit { name, .. } => VType::Struct(name.segments[0].name.clone()),
            Expr::Field { recv, name, span } => {
                let rt = self.probe_type(recv)?;
                match rt {
                    VType::Struct(sname) => self.struct_field_type(&sname, &name.name, *span)?,
                    _ => {
                        // `Type.Variant` value probe.
                        if let Expr::Ident(tid) = &**recv {
                            if self.cg.sums.contains_key(&tid.name) {
                                return Ok(self.sum_vtype(&tid.name));
                            }
                        }
                        return nope(*span, "inferring this type");
                    }
                }
            }
            Expr::If { then_block, .. } => {
                match then_block.stmts.last() {
                    Some(Stmt::Expr(inner)) => self.probe_type(inner)?,
                    _ => return nope(e.span(), "inferring this type"),
                }
            }
            Expr::Match { arms, .. } => {
                let mut found: Option<VType> = None;
                for arm in arms {
                    if let Ok(t) = self.probe_type(&arm.body) {
                        found = Some(t);
                        break;
                    }
                }
                match found {
                    Some(t) => t,
                    None => return nope(e.span(), "inferring this type"),
                }
            }
            Expr::Block(b) => match b.stmts.last() {
                Some(Stmt::Expr(inner)) => self.probe_type(inner)?,
                _ => VType::Unit,
            },
            Expr::MethodCall { recv, name, .. } => match name.name.as_str() {
                "len" => VType::I64,
                "is_some" | "is_none" | "contains" => VType::Bool,
                "get" => {
                    let rt = self.probe_type(recv)?;
                    match rt {
                        VType::List(elem) => VType::Option(elem),
                        _ => return nope(e.span(), "inferring this type"),
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
                            return nope(e.span(), "inferring this type");
                        }
                    } else {
                        return nope(e.span(), "inferring this type");
                    }
                }
            },
            _ => return nope(e.span(), "inferring this type"),
        })
    }

    fn eval_match(
        &mut self,
        scrutinee: &Expr,
        arms: &[Arm],
        span: Span,
    ) -> CResult<Option<(Value, VType)>> {
        let (sv, st) = self.eval_expr_val(scrutinee)?;
        // Hold the scrutinee in a variable for repeated tests.
        let svar = self.new_var(&st);
        self.b.def_var(svar, sv);

        let merge = self.b.create_block();
        // Merge param type discovered from the first value-producing arm; we
        // pre-scan arm bodies structurally via probe where possible. Simpler:
        // decide after lowering first arm — but block params must exist before
        // jumps. Strategy: lower arms into (blocks, value) then wire jumps.
        // Pragmatic approach: probe the first arm body type.
        // Probe the result type. Pattern binds aren't live yet, so bind their
        // *types* in a scratch scope first, and try each arm until one probes.
        let mut out_t: Option<VType> = None;
        for arm in arms {
            self.push_scope();
            let bound = self.bind_pattern_types(&arm.pattern, &st).is_ok();
            let probed = if bound {
                self.probe_type(&arm.body).ok()
            } else {
                None
            };
            self.pop_scope_no_decs();
            match probed {
                Some(t) if t != VType::Unit => {
                    out_t = Some(t);
                    break;
                }
                // An arm that yields nothing (e.g. `-> print(..)`) makes this
                // a statement match: no merge parameter, no value.
                Some(_) => {
                    out_t = None;
                    break;
                }
                None => continue,
            }
        }
        // A value-match needs EVERY arm to yield a value; if any arm is a bare
        // statement call, fall back to statement form.
        if out_t.is_some() {
            for arm in arms {
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
            let bv = self.eval_expr(&arm.body)?;
            // An arm body may itself terminate (`-> break`, `-> return x`).
            let bterm = self.is_filled();
            if !bterm {
                match (&out_t, bv) {
                    (Some(want), Some((mut v, vt))) => {
                        if *want == VType::F64 && vt == VType::I64 {
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
        // Release the scrutinee.
        if st.boxed() {
            let sv2 = self.b.use_var(svar);
            self.call_rt("grasm_val_dec", &[sv2]);
        }
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
            t = match (&t, seg) {
                (VType::Struct(sname), PlaceSeg::Field(f)) => {
                    let sname = sname.clone();
                    self.struct_field_type(&sname, f, span)?
                }
                (VType::List(e), PlaceSeg::Index(_)) => (**e).clone(),
                _ => return nope(span, "this assignment target's shape"),
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
        // Intermediate reads borrow; only the final read is unboxed.
        for (i, seg) in path.iter().enumerate() {
            let is_last = i + 1 == path.len();
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
                    let (iv, _) = self.eval_expr_val(ix)?;
                    let v = self.call_rt("grasm_list_get", &[cur, iv]).unwrap();
                    cur_t = et;
                    v
                }
                _ => return nope(Span::new(0, 1), "this assignment target's shape"),
            };
            if is_last {
                let (v, _) = self.unbox(boxed, elem_t)?;
                return Ok(v);
            }
            cur = boxed;
        }
        nope(Span::new(0, 1), "an empty assignment path")
    }

    /// Reads a place without unboxing the final value (for container places).
    fn read_place_boxed(
        &mut self,
        root: Value,
        path: &[PlaceSeg<'_>],
        root_t: &VType,
    ) -> CResult<Value> {
        let mut cur = root;
        let mut cur_t = root_t.clone();
        for seg in path {
            match (&cur_t, seg) {
                (VType::Struct(sname), PlaceSeg::Field(f)) => {
                    let sname = sname.clone();
                    let ft = self.struct_field_type(&sname, f, Span::new(0, 1))?;
                    let (fp, fl) = self.str_ptr_len(f);
                    cur = self.call_rt("grasm_struct_get", &[cur, fp, fl]).unwrap();
                    cur_t = ft;
                }
                (VType::List(e), PlaceSeg::Index(ix)) => {
                    let et = (**e).clone();
                    let (iv, _) = self.eval_expr_val(ix)?;
                    cur = self.call_rt("grasm_list_get", &[cur, iv]).unwrap();
                    cur_t = et;
                }
                _ => return nope(Span::new(0, 1), "this place's shape"),
            }
        }
        Ok(cur)
    }

    /// Writes `boxed_val` at the place, rebuilding containers copy-on-write.
    /// Returns the (possibly new) root value.
    fn write_place(
        &mut self,
        root: Value,
        path: &[PlaceSeg<'_>],
        root_t: &VType,
        boxed_val: Value,
    ) -> CResult<Value> {
        match path.split_first() {
            None => Ok(boxed_val),
            Some((seg, [])) => match (root_t, seg) {
                (VType::Struct(_), PlaceSeg::Field(f)) => {
                    let (fp, fl) = self.str_ptr_len(f);
                    Ok(self
                        .call_rt("grasm_struct_set_field", &[root, fp, fl, boxed_val])
                        .unwrap())
                }
                (VType::List(_), PlaceSeg::Index(ix)) => {
                    let (iv, _) = self.eval_expr_val(ix)?;
                    Ok(self
                        .call_rt("grasm_list_set", &[root, iv, boxed_val])
                        .unwrap())
                }
                _ => nope(Span::new(0, 1), "this assignment target's shape"),
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
                            let (iv, _) = self.eval_expr_val(ix)?;
                            let c = self.call_rt("grasm_list_get", &[root, iv]).unwrap();
                            let _ = iv;
                            (c, et, PlaceSeg::Index(ix))
                        }
                        _ => return nope(Span::new(0, 1), "this assignment target's shape"),
                    };
                let new_child = self.write_place(child, rest, &child_t, boxed_val)?;
                match (root_t, writeback) {
                    (VType::Struct(_), PlaceSeg::Field(f)) => {
                        let (fp, fl) = self.str_ptr_len(f);
                        Ok(self
                            .call_rt("grasm_struct_set_field", &[root, fp, fl, new_child])
                            .unwrap())
                    }
                    (VType::List(_), PlaceSeg::Index(ix)) => {
                        let (iv, _) = self.eval_expr_val(ix)?;
                        Ok(self
                            .call_rt("grasm_list_set", &[root, iv, new_child])
                            .unwrap())
                    }
                    _ => nope(Span::new(0, 1), "this assignment target's shape"),
                }
            }
        }
    }

    /// Inlines `list.map(|x| body)` / `list.filter(|x| body)` as a loop.
    /// Consumes the owned `list`; returns (new list value, its type).
    fn emit_list_transform(
        &mut self,
        list: Value,
        elem_t: &VType,
        param: &Ident,
        body: &Expr,
        is_map: bool,
    ) -> CResult<(Value, VType)> {
        let list_var = self.new_var(&VType::List(Box::new(elem_t.clone())));
        self.b.def_var(list_var, list);
        let out_var = self.new_var(&VType::List(Box::new(elem_t.clone())));
        let out0 = self.call_rt("grasm_list_new", &[]).unwrap();
        self.b.def_var(out_var, out0);
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
        let boxed = self.call_rt("grasm_list_get", &[lv2, iv2]).unwrap();
        let (ev, evt) = self.unbox(boxed, elem_t)?;
        let evar = self.new_var(&evt);
        self.b.def_var(evar, ev);
        self.push_scope();
        self.bind(&param.name, evar, evt.clone());
        let (bv, bvt) = self.eval_expr_val(body)?;
        let out_elem_t = if is_map { bvt.clone() } else { elem_t.clone() };
        if is_map {
            let bx = self.box_value(bv, &bvt);
            let cur = self.b.use_var(out_var);
            let np = self.call_rt("grasm_list_push", &[cur, bx]).unwrap();
            self.b.def_var(out_var, np);
            self.pop_scope_with_decs();
            self.b.ins().jump(step, &[]);
            self.filled = true;
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
            let bx = self.box_value(e2, &evt);
            let cur = self.b.use_var(out_var);
            let np = self.call_rt("grasm_list_push", &[cur, bx]).unwrap();
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
        let lv3 = self.b.use_var(list_var);
        self.call_rt("grasm_val_dec", &[lv3]);
        let out = self.b.use_var(out_var);
        Ok((out, VType::List(Box::new(out_elem_t))))
    }

    /// Declares the *types* a pattern would bind, without emitting any code.
    /// Used to type-probe match arm bodies before lowering.
    fn bind_pattern_types(&mut self, pat: &Pattern, st: &VType) -> CResult<()> {
        match pat {
            Pattern::Wildcard(_) | Pattern::Int(..) | Pattern::Bool(..) | Pattern::Str(..) => {
                Ok(())
            }
            Pattern::Bind(id) => {
                let is_variant = matches!(self.cg.variant_index.get(&id.name), Some((_, 0)))
                    || id.name == "None";
                if !is_variant {
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
                    VType::Fallible(ok_t) => {
                        if vname == "Ok" {
                            vec![(**ok_t).clone()]
                        } else {
                            vec![VType::Any]
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
            Pattern::Wildcard(_) => {
                self.b.ins().jump(body_blk, &[]);
                self.filled = true;
                Ok(true)
            }
            Pattern::Bind(id) => {
                // Nullary-variant name check.
                if let Some((_, 0)) = self.cg.variant_index.get(&id.name).cloned() {
                    let sv = self.b.use_var(svar);
                    let (vp, vl) = self.str_ptr_len(&id.name);
                    let c = self.call_rt("grasm_variant_is", &[sv, vp, vl]).unwrap();
                    self.b.ins().brif(c, body_blk, &[], next_test, &[]);
                    self.filled = true;
                    return Ok(true);
                }
                if id.name == "None" {
                    let sv = self.b.use_var(svar);
                    let (vp, vl) = self.str_ptr_len("None");
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
                    VType::Fallible(ok_t) => {
                        if vname == "Ok" {
                            vec![(**ok_t).clone()]
                        } else {
                            vec![VType::Any]
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
                            // Recursive sub-pattern on the (boxed) field. Held
                            // as a typed temp var; failure path may leak the
                            // temp (hardened later).
                            let (fv, fvt) = self.unbox(boxed, ft)?;
                            let fvar = self.new_var(&fvt);
                            self.b.def_var(fvar, fv);
                            let cont = self.b.create_block();
                            self.lower_pattern_test(nested, fvar, &fvt, cont, next_test)?;
                            self.goto_block(cont);
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
                let mut chain: Vec<(Variable, VType, &Pattern)> = Vec::new();
                for (i, (pat_i, t_i)) in items.iter().zip(&elem_ts).enumerate() {
                    let sv = self.b.use_var(svar);
                    let idx = self.b.ins().iconst(types::I64, i as i64);
                    let boxed = self.call_rt("grasm_list_get", &[sv, idx]).unwrap();
                    let (v, vt) = self.unbox(boxed, t_i)?;
                    let var = self.new_var(&vt);
                    self.b.def_var(var, v);
                    chain.push((var, vt, pat_i));
                }
                let n = chain.len();
                for (k, (var, vt, pat_i)) in chain.into_iter().enumerate() {
                    let cont = if k + 1 == n {
                        body_blk
                    } else {
                        self.b.create_block()
                    };
                    self.lower_pattern_test(pat_i, var, &vt, cont, next_test)?;
                    if k + 1 < n {
                        self.goto_block(cont);
                    }
                }
                Ok(true)
            }
        }
    }
}
