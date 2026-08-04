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
                | VType::Any
        )
    }
}

struct FnSig {
    params: Vec<VType>,
    ret: VType,
    id: FuncId,
}

pub fn compile_object(
    program: &grasm_sema::loader::Program,
    obj_out: &std::path::Path,
) -> Result<(), String> {
    if program.files.len() > 1 {
        return Err("native backend does not yet support modules (imports)".into());
    }
    let module_ast = &program.files[0].module;

    let mut flags = settings::builder();
    flags.set("is_pic", "true").map_err(|e| e.to_string())?;
    flags.set("opt_level", "speed").map_err(|e| e.to_string())?;
    let isa = cranelift_native::builder()
        .map_err(|e| e.to_string())?
        .finish(settings::Flags::new(flags))
        .map_err(|e| e.to_string())?;
    let builder = ObjectBuilder::new(isa, "grasm", cranelift_module::default_libcall_names())
        .map_err(|e| e.to_string())?;
    let mut module = ObjectModule::new(builder);

    let mut cg = Codegen::new(&mut module);
    cg.declare_runtime()?;
    match cg.compile(module_ast) {
        Ok(()) => {}
        Err(u) => {
            let f = &program.files[0];
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
                }
                nope(*span, format!("the type `{}`", path.dotted()))
            }
            TypeExpr::List(inner, _) => Ok(VType::List(Box::new(self.vtype_of(inner)?))),
            TypeExpr::Tuple(items, _) if items.is_empty() => Ok(VType::Unit),
            other => nope(other.span(), "non-scalar types"),
        }
    }

    fn compile(&mut self, module_ast: &AstModule) -> CResult<()> {
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
        for decl in &module_ast.decls {
            match decl {
                Decl::Fn(f) => {
                    if !f.generics.is_empty() {
                        return nope(f.span, "generic functions");
                    }
                    let mut params = Vec::new();
                    for p in &f.params {
                        if p.mode != ParamMode::Let {
                            return nope(p.span, "`inout`/`own` parameters");
                        }
                        params.push(self.vtype_of(&p.ty)?);
                    }
                    let mut ret = match &f.ret {
                        Some(t) => self.vtype_of(t)?,
                        None => VType::Unit,
                    };
                    if f.err.is_some() {
                        ret = VType::Fallible(Box::new(ret));
                    }
                    let mut sig = self.module.make_signature();
                    for p in &params {
                        sig.params.push(AbiParam::new(p.cl()));
                    }
                    if ret != VType::Unit {
                        sig.returns.push(AbiParam::new(ret.cl()));
                    }
                    let id = self
                        .module
                        .declare_function(&format!("g_{}", f.name.name), Linkage::Local, &sig)
                        .map_err(|e| Unsupported {
                            span: f.span,
                            what: format!("(module error: {e})"),
                        })?;
                    self.fns
                        .insert(f.name.name.clone(), FnSig { params, ret, id });
                }
                Decl::Test(_) | Decl::Import(_) | Decl::Type(_) => {}
                Decl::Effect(e) => return nope(e.span, "effects"),
            }
        }
        if !self.fns.contains_key("main") {
            return nope(Span::new(0, 1), "programs without a `main` function");
        }
        for decl in &module_ast.decls {
            if let Decl::Fn(f) = decl {
                self.compile_fn(f)?;
            }
        }
        self.emit_c_main()?;
        Ok(())
    }

    fn emit_c_main(&mut self) -> CResult<()> {
        let mut sig = self.module.make_signature();
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
        b.switch_to_block(entry);
        let g_main = self.fns["main"].id;
        let main_fallible = matches!(self.fns["main"].ret, VType::Fallible(_));
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
        let sig_params = self.fns[&f.name.name].params.clone();
        let sig_ret = self.fns[&f.name.name].ret.clone();
        let func_id = self.fns[&f.name.name].id;

        let mut ctx = self.module.make_context();
        for p in &sig_params {
            ctx.func.signature.params.push(AbiParam::new(p.cl()));
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
        };

        let entry = fcx.b.create_block();
        fcx.b.append_block_params_for_function_params(entry);
        fcx.b.switch_to_block(entry);
        fcx.push_scope();
        let entry_params: Vec<Value> = fcx.b.block_params(entry).to_vec();
        for (i, p) in f.params.iter().enumerate() {
            let vt = sig_params[i].clone();
            let var = fcx.new_var(&vt);
            fcx.b.def_var(var, entry_params[i]);
            fcx.bind(&p.name.name, var, vt);
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

    /// Before a `return`: release every boxed local in every scope.
    fn emit_scope_decs_all(&mut self) {
        let all: Vec<(Variable, VType)> = self
            .vars
            .iter()
            .flat_map(|s| s.values().cloned())
            .collect();
        for (var, vt) in all {
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
                let (mut v, mut vt) = self.eval_expr_val(init)?;
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
                // Single-level field assignment on a variable: `p.x = v`.
                if let Expr::Field { recv, name, .. } = target {
                    if *op != AssignOp::Assign {
                        return nope(target.span(), "compound assignment to fields");
                    }
                    let Expr::Ident(root) = &**recv else {
                        return nope(target.span(), "nested field assignment");
                    };
                    let Some((var, vt)) = self.lookup(&root.name) else {
                        return nope(root.span, "assignment to unknown variable");
                    };
                    let VType::Struct(sname) = vt.clone() else {
                        return nope(target.span(), "field assignment on non-structs");
                    };
                    let field_t = self.struct_field_type(&sname, &name.name, target.span())?;
                    let (mut fv, fvt) = self.eval_expr_val(value)?;
                    if field_t == VType::F64 && fvt == VType::I64 {
                        fv = self.b.ins().fcvt_from_sint(types::F64, fv);
                    }
                    let boxed = self.box_value(fv, &field_t);
                    let cur = self.b.use_var(var);
                    let (np, nl) = self.str_ptr_len(&name.name);
                    let newp = self
                        .call_rt("grasm_struct_set_field", &[cur, np, nl, boxed])
                        .unwrap();
                    self.b.def_var(var, newp);
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
                        let (mut val, vt) = self.eval_expr_val(e)?;
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
                    }
                    None => {
                        self.emit_scope_decs_all();
                        self.b.ins().return_(&[]);
                    }
                }
                Ok(StmtOut::Terminated)
            }
            Stmt::Break(span) => {
                let Some(&(_, break_blk)) = self.loops.last() else {
                    return nope(*span, "break outside loop");
                };
                self.b.ins().jump(break_blk, &[]);
                Ok(StmtOut::Terminated)
            }
            Stmt::Continue(span) => {
                let Some(&(cont_blk, _)) = self.loops.last() else {
                    return nope(*span, "continue outside loop");
                };
                self.b.ins().jump(cont_blk, &[]);
                Ok(StmtOut::Terminated)
            }
            Stmt::While { cond, body, .. } => {
                let header = self.b.create_block();
                let body_blk = self.b.create_block();
                let exit = self.b.create_block();
                self.b.ins().jump(header, &[]);
                self.b.switch_to_block(header);
                let (c, ct) = self.eval_expr_val(cond)?;
                if ct != VType::Bool {
                    return nope(cond.span(), "non-bool while condition");
                }
                self.b.ins().brif(c, body_blk, &[], exit, &[]);
                self.b.switch_to_block(body_blk);
                self.loops.push((header, exit));
                let (_, terminated) = self.eval_block_inner(body)?;
                self.loops.pop();
                if !terminated {
                    self.b.ins().jump(header, &[]);
                }
                self.b.switch_to_block(exit);
                Ok(StmtOut::Value(None))
            }
            Stmt::For {
                var: loop_var,
                iter,
                body,
                span,
            } => self.eval_for(loop_var, iter, body, *span),
            Stmt::With { span, .. } => nope(*span, "effect handlers"),
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
            self.b.switch_to_block(header);
            let iv = self.b.use_var(ivar);
            let ev = self.b.use_var(end_var);
            let cc = if *inclusive {
                IntCC::SignedLessThanOrEqual
            } else {
                IntCC::SignedLessThan
            };
            let c = self.b.ins().icmp(cc, iv, ev);
            self.b.ins().brif(c, body_blk, &[], exit, &[]);

            self.b.switch_to_block(body_blk);
            self.push_scope();
            self.bind(&loop_var.name, ivar, VType::I64);
            self.loops.push((step, exit));
            let (_, terminated) = self.eval_block_inner(body)?;
            self.loops.pop();
            self.pop_scope_no_decs(); // loop var is scalar
            if !terminated {
                self.b.ins().jump(step, &[]);
            }
            self.b.switch_to_block(step);
            let iv2 = self.b.use_var(ivar);
            let one = self.b.ins().iconst(types::I64, 1);
            let next = self.b.ins().iadd(iv2, one);
            self.b.def_var(ivar, next);
            self.b.ins().jump(header, &[]);
            self.b.switch_to_block(exit);
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
        self.b.switch_to_block(header);
        let lv = self.b.use_var(list_var);
        let len = self.call_rt("grasm_list_len", &[lv]).unwrap();
        let iv = self.b.use_var(ivar);
        let c = self.b.ins().icmp(IntCC::SignedLessThan, iv, len);
        self.b.ins().brif(c, body_blk, &[], exit, &[]);

        self.b.switch_to_block(body_blk);
        let lv2 = self.b.use_var(list_var);
        let iv2 = self.b.use_var(ivar);
        let boxed_elem = self.call_rt("grasm_list_get", &[lv2, iv2]).unwrap();
        let (elem_val, elem_vt) = self.unbox(boxed_elem, &elem_t)?;
        let evar = self.new_var(&elem_vt);
        self.b.def_var(evar, elem_val);
        self.push_scope();
        self.bind(&loop_var.name, evar, elem_vt.clone());
        self.loops.push((step, exit));
        let (_, terminated) = self.eval_block_inner(body)?;
        self.loops.pop();
        if terminated {
            self.pop_scope_no_decs();
        } else {
            self.pop_scope_with_decs();
        }
        if !terminated {
            self.b.ins().jump(step, &[]);
        }
        self.b.switch_to_block(step);
        let iv3 = self.b.use_var(ivar);
        let one = self.b.ins().iconst(types::I64, 1);
        let next = self.b.ins().iadd(iv3, one);
        self.b.def_var(ivar, next);
        self.b.ins().jump(header, &[]);
        self.b.switch_to_block(exit);
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
            VType::Unit => nope(Span::new(0, 1), "unit values in boxes"),
            // All other boxed kinds pass through, keeping the +1.
            other => Ok((boxed, other.clone())),
        }
    }

    fn box_value(&mut self, v: Value, vt: &VType) -> Value {
        match vt {
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
                span,
            } => self.eval_if(cond, then_block, else_branch.as_deref(), *span),
            Expr::Call { callee, args, span } => self.eval_call(callee, args, *span),
            Expr::Block(b) => {
                let (last, terminated) = self.eval_block_inner(b)?;
                if terminated {
                    Ok(None)
                } else {
                    Ok(last)
                }
            }
            Expr::StructLit { name, fields, span } => {
                self.eval_struct_lit(name, fields, *span).map(Some)
            }
            Expr::Field { recv, name, span } => self.eval_field(recv, name, *span),
            Expr::Match {
                scrutinee, arms, span,
            } => self.eval_match(scrutinee, arms, *span),
            Expr::Try { expr, span } => self.eval_try(expr, *span).map(Some),
            other => nope(
                other.span(),
                match other {
                    Expr::Tuple(..) => "tuples",
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
                    let fake_lhs_span = span;
                    let _ = fake_lhs_span;
                    return self
                        .coalesce_values(rv, &payload_t, &args[0], span)
                        .map(Some);
                }
                _ => {}
            }
        }
        match (&rt, name.name.as_str()) {
            (VType::Str, "len") => {
                let v = self.call_rt("grasm_str_len", &[rv]).unwrap();
                self.call_rt("grasm_val_dec", &[rv]);
                Ok(Some((v, VType::I64)))
            }
            (VType::List(_), "len") => {
                let v = self.call_rt("grasm_list_len", &[rv]).unwrap();
                self.call_rt("grasm_val_dec", &[rv]);
                Ok(Some((v, VType::I64)))
            }
            _ => nope(span, format!("the method `{}`", name.name)),
        }
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
            }
            BinaryOp::Or => {
                self.b.ins().brif(lv, merge, &[lv], rhs_blk, &[]);
            }
            _ => unreachable!(),
        }
        self.b.switch_to_block(rhs_blk);
        let (rv, rt) = self.eval_expr_val(rhs)?;
        if rt != VType::Bool {
            return nope(rhs.span(), "non-bool logic operand");
        }
        self.b.ins().jump(merge, &[rv]);
        self.b.switch_to_block(merge);
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
        self.b.switch_to_block(bad_blk);
        self.call_rt(panic_fn, &[]);
        self.trap_here();
        self.b.switch_to_block(ok_blk);
    }

    fn eval_if(
        &mut self,
        cond: &Expr,
        then_block: &grasm_syntax::ast::Block,
        else_branch: Option<&ElseBranch>,
        span: Span,
    ) -> CResult<Option<(Value, VType)>> {
        let (c, ct) = self.eval_expr_val(cond)?;
        if ct != VType::Bool {
            return nope(cond.span(), "non-bool if condition");
        }
        let then_blk = self.b.create_block();
        let else_blk = self.b.create_block();
        let merge = self.b.create_block();
        self.b.ins().brif(c, then_blk, &[], else_blk, &[]);

        self.b.switch_to_block(then_blk);
        let (then_val, then_term) = self.eval_block_inner(then_block)?;

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

        self.b.switch_to_block(else_blk);
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
        if !else_term {
            match (&merge_ty, &else_val) {
                (Some(want), Some((v, vt))) => {
                    let mut v = *v;
                    if *want == VType::F64 && *vt == VType::I64 {
                        v = self.b.ins().fcvt_from_sint(types::F64, v);
                    }
                    self.b.ins().jump(merge, &[v]);
                }
                (Some(_), None) => {
                    return nope(span, "if-expressions with a valueless else branch");
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
                }
            }
        }

        self.b.switch_to_block(merge);
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
            return nope(id.span, "calling closures");
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
                        let dead = self.b.create_block();
                        self.b.switch_to_block(dead);
                        return Ok(None);
                    }
                }
                return nope(span, "panic with a non-literal message");
            }
            "fail" => {
                let (ev, evt) = self.eval_expr_val(&args[0])?;
                if !evt.boxed() {
                    return nope(args[0].span(), "failing with unboxed values");
                }
                let wrapped = self.call_rt("grasm_err_new", &[ev]).unwrap();
                self.emit_scope_decs_all();
                self.b.ins().return_(&[wrapped]);
                let dead = self.b.create_block();
                self.b.switch_to_block(dead);
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
        let Some(sig) = self.cg.fns.get(&id.name) else {
            return nope(id.span, format!("the function `{}` here", id.name));
        };
        let (fid, params, ret) = (sig.id, sig.params.clone(), sig.ret.clone());
        let mut vals = Vec::with_capacity(args.len());
        for (a, want) in args.iter().zip(&params) {
            let (mut v, vt) = self.eval_expr_val(a)?;
            if *want == VType::F64 && vt == VType::I64 {
                v = self.b.ins().fcvt_from_sint(types::F64, v);
            }
            // Boxed args transfer ownership to the callee (it decs at exit).
            vals.push(v);
        }
        let fref = self.cg.module.declare_func_in_func(fid, self.b.func);
        let inst = self.b.ins().call(fref, &vals);
        let results = self.b.inst_results(inst);
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
        self.b.switch_to_block(bad_blk);
        self.call_rt("grasm_panic_assert", &[]);
        self.trap_here();
        self.b.switch_to_block(ok_blk);
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
            let (mut av, avt) = self.eval_expr_val(a)?;
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
            let (mut fv, fvt) = self.eval_expr_val(fexpr)?;
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

        self.b.switch_to_block(err_blk);
        let zero = self.b.ins().iconst(types::I64, 0);
        let epay = self.call_rt("grasm_variant_field", &[fv, zero]).unwrap();
        self.call_rt("grasm_val_dec", &[fv]);
        let wrapped = self.call_rt("grasm_err_new", &[epay]).unwrap();
        self.emit_scope_decs_all();
        if matches!(self.ret, VType::Fallible(_)) {
            self.b.ins().return_(&[wrapped]);
        } else {
            // Checker prevents this; guard anyway.
            self.call_rt("grasm_report_main_err", &[wrapped]);
            self.trap_here();
        }

        self.b.switch_to_block(ok_blk);
        let zero2 = self.b.ins().iconst(types::I64, 0);
        let payload = self.call_rt("grasm_variant_field", &[fv, zero2]).unwrap();
        self.call_rt("grasm_val_dec", &[fv]);
        let (v, vt) = self.unbox(payload, &ok_t)?;
        Ok((v, vt))
    }

    fn eval_coalesce(&mut self, lhs: &Expr, rhs: &Expr, span: Span) -> CResult<(Value, VType)> {
        let (lv, lt) = self.eval_expr_val(lhs)?;
        let VType::Option(payload_t) = lt else {
            return nope(span, "?? on non-Option values");
        };
        self.coalesce_values(lv, &payload_t, rhs, span)
    }

    /// Shared by `??` and `unwrap_or`: consumes the owned Option `lv`.
    fn coalesce_values(
        &mut self,
        lv: Value,
        payload_t: &VType,
        rhs: &Expr,
        _span: Span,
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

        self.b.switch_to_block(some_blk);
        let zero = self.b.ins().iconst(types::I64, 0);
        let payload = self.call_rt("grasm_variant_field", &[lv, zero]).unwrap();
        self.call_rt("grasm_val_dec", &[lv]);
        let (v, _) = self.unbox(payload, &out_t)?;
        self.b.ins().jump(merge, &[v]);

        self.b.switch_to_block(none_blk);
        self.call_rt("grasm_val_dec", &[lv]);
        let (mut rv, rvt) = self.eval_expr_val(rhs)?;
        if out_t == VType::F64 && rvt == VType::I64 {
            rv = self.b.ins().fcvt_from_sint(types::F64, rv);
        }
        self.b.ins().jump(merge, &[rv]);

        self.b.switch_to_block(merge);
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
                    } else {
                        return nope(id.span, "inferring this type");
                    }
                }
            },
            Expr::Call { callee, args, .. } => {
                if let Expr::Ident(id) = &**callee {
                    if let Some(sig) = self.cg.fns.get(&id.name) {
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
                    if let Some(sig) = self.cg.fns.get(&name.name) {
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
        let out_t: Option<VType> = match self.probe_type(&arms[0].body) {
            Ok(t) if t != VType::Unit => Some(t),
            _ => None,
        };
        if let Some(t) = &out_t {
            self.b.append_block_param(merge, t.cl());
        }

        let mut next_test = self.b.create_block();
        self.b.ins().jump(next_test, &[]);

        for (ai, arm) in arms.iter().enumerate() {
            self.b.switch_to_block(next_test);
            next_test = self.b.create_block();
            let body_blk = self.b.create_block();
            let is_last = ai + 1 == arms.len();

            self.push_scope();
            // Pattern test + binds.
            let matched = self.lower_pattern_test(&arm.pattern, svar, &st, body_blk, next_test)?;
            let _ = matched;

            self.b.switch_to_block(body_blk);
            if let Some(guard) = &arm.guard {
                let (gv, gt) = self.eval_expr_val(guard)?;
                if gt != VType::Bool {
                    return nope(guard.span(), "non-bool guards");
                }
                let cont_blk = self.b.create_block();
                // Guard false: release binds, try next arm.
                let cleanup = self.b.create_block();
                self.b.ins().brif(gv, cont_blk, &[], cleanup, &[]);
                self.b.switch_to_block(cleanup);
                self.release_current_scope_binds();
                self.b.ins().jump(next_test, &[]);
                self.b.switch_to_block(cont_blk);
            }
            let (bv, bterm) = match self.eval_expr(&arm.body)? {
                Some(v) => (Some(v), false),
                None => (None, false),
            };
            if !bterm {
                match (&out_t, bv) {
                    (Some(want), Some((mut v, vt))) => {
                        if *want == VType::F64 && vt == VType::I64 {
                            v = self.b.ins().fcvt_from_sint(types::F64, v);
                        }
                        self.pop_scope_with_decs();
                        self.b.ins().jump(merge, &[v]);
                    }
                    (None, maybe) => {
                        if let Some((v, vt)) = maybe {
                            self.dec_if_boxed(v, &vt);
                        }
                        self.pop_scope_with_decs();
                        self.b.ins().jump(merge, &[]);
                    }
                    (Some(_), None) => {
                        return nope(arm.span, "this arm producing no value");
                    }
                }
            } else {
                self.pop_scope_no_decs();
            }

            if is_last {
                // Exhaustive per checker: the final next_test is unreachable.
                self.b.switch_to_block(next_test);
                self.call_rt("grasm_panic_assert", &[]);
                self.trap_here();
            }
        }

        self.b.switch_to_block(merge);
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
                Ok(true)
            }
            Pattern::Bind(id) => {
                // Nullary-variant name check.
                if let Some((_, 0)) = self.cg.variant_index.get(&id.name).cloned() {
                    let sv = self.b.use_var(svar);
                    let (vp, vl) = self.str_ptr_len(&id.name);
                    let c = self.call_rt("grasm_variant_is", &[sv, vp, vl]).unwrap();
                    self.b.ins().brif(c, body_blk, &[], next_test, &[]);
                    return Ok(true);
                }
                if id.name == "None" {
                    let sv = self.b.use_var(svar);
                    let (vp, vl) = self.str_ptr_len("None");
                    let c = self.call_rt("grasm_variant_is", &[sv, vp, vl]).unwrap();
                    self.b.ins().brif(c, body_blk, &[], next_test, &[]);
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
                Ok(true)
            }
            Pattern::Int(want, _) => {
                let sv = self.b.use_var(svar);
                let w = self.b.ins().iconst(types::I64, *want);
                let c = self.b.ins().icmp(IntCC::Equal, sv, w);
                self.b.ins().brif(c, body_blk, &[], next_test, &[]);
                Ok(true)
            }
            Pattern::Bool(want, _) => {
                let sv = self.b.use_var(svar);
                let w = self.b.ins().iconst(types::I8, i64::from(*want));
                let c = self.b.ins().icmp(IntCC::Equal, sv, w);
                self.b.ins().brif(c, body_blk, &[], next_test, &[]);
                Ok(true)
            }
            Pattern::Str(want, _) => {
                let sv = self.b.use_var(svar);
                let (p, l) = self.str_ptr_len(want);
                let lit = self.call_rt("grasm_str_lit", &[p, l]).unwrap();
                let c = self.call_rt("grasm_str_eq", &[sv, lit]).unwrap();
                self.call_rt("grasm_val_dec", &[lit]);
                self.b.ins().brif(c, body_blk, &[], next_test, &[]);
                Ok(true)
            }
            Pattern::Variant { path, fields, span } => {
                let vname = path.segments.last().unwrap().name.clone();
                let sv = self.b.use_var(svar);
                let (vp, vl) = self.str_ptr_len(&vname);
                let c = self.call_rt("grasm_variant_is", &[sv, vp, vl]).unwrap();
                let bind_blk = self.b.create_block();
                self.b.ins().brif(c, bind_blk, &[], next_test, &[]);
                self.b.switch_to_block(bind_blk);
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
                            self.b.switch_to_block(cont);
                        }
                    }
                }
                self.b.ins().jump(body_blk, &[]);
                Ok(true)
            }
            Pattern::Tuple(_, span) => nope(*span, "tuple patterns"),
        }
    }
}
