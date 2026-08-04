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
}

impl VType {
    fn cl(&self) -> types::Type {
        match self {
            VType::I64 => types::I64,
            VType::F64 => types::F64,
            VType::Bool => types::I8,
            VType::Unit => types::I8,
            // Boxed values travel as 64-bit pointers.
            VType::Str | VType::List(_) => types::I64,
        }
    }

    fn boxed(&self) -> bool {
        matches!(self, VType::Str | VType::List(_))
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
}

impl<'m> Codegen<'m> {
    fn new(module: &'m mut ObjectModule) -> Self {
        Codegen {
            module,
            fns: HashMap::new(),
            runtime: HashMap::new(),
            strings: HashMap::new(),
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
                if path.segments.len() == 1 && args.is_empty() {
                    match path.segments[0].name.as_str() {
                        "i8" | "i16" | "i32" | "i64" | "u8" | "u16" | "u32" | "u64" | "usize"
                        | "int" => return Ok(VType::I64),
                        "f32" | "f64" | "float" => return Ok(VType::F64),
                        "bool" => return Ok(VType::Bool),
                        "str" => return Ok(VType::Str),
                        _ => {}
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
        for decl in &module_ast.decls {
            match decl {
                Decl::Fn(f) => {
                    if !f.generics.is_empty() {
                        return nope(f.span, "generic functions");
                    }
                    if f.err.is_some() {
                        return nope(f.span, "fallible functions (`! E`)");
                    }
                    let mut params = Vec::new();
                    for p in &f.params {
                        if p.mode != ParamMode::Let {
                            return nope(p.span, "`inout`/`own` parameters");
                        }
                        params.push(self.vtype_of(&p.ty)?);
                    }
                    let ret = match &f.ret {
                        Some(t) => self.vtype_of(t)?,
                        None => VType::Unit,
                    };
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
                Decl::Test(_) | Decl::Import(_) => {}
                Decl::Type(t) => return nope(t.span, "type declarations"),
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
        let fref = self.module.declare_func_in_func(g_main, b.func);
        b.ins().call(fref, &[]);
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
                    if *want == VType::F64 && vt == VType::I64 {
                        v = fcx.b.ins().fcvt_from_sint(types::F64, v);
                    }
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
            VType::Str | VType::List(_) => Ok((boxed, want.clone())),
            VType::Unit => nope(Span::new(0, 1), "unit list elements"),
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
                None => nope(id.span, format!("the name `{}` here", id.name)),
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
            other => nope(
                other.span(),
                match other {
                    Expr::Tuple(..) => "tuples",
                    Expr::StructLit { .. } => "structs",
                    Expr::Field { .. } => "field access",
                    Expr::Match { .. } => "match",
                    Expr::Range { .. } => "range values outside for",
                    Expr::Try { .. } => "try",
                    Expr::Closure { .. } => "closures",
                    _ => "this construct",
                },
            ),
        }
    }

    fn list_elem_type(&mut self, items: &[Expr], span: Span) -> CResult<VType> {
        // Peek the first element's static shape; empty lists need context we
        // don't track yet.
        let Some(first) = items.first() else {
            return nope(span, "empty list literals (annotate via a function param for now)");
        };
        // Cheap structural probe: literals and idents cover the N2 tests.
        Ok(match first {
            Expr::Int(..) => VType::I64,
            Expr::Float(..) => VType::F64,
            Expr::Bool(..) => VType::Bool,
            Expr::Str(..) => VType::Str,
            Expr::Ident(id) => match self.lookup(&id.name) {
                Some((_, vt)) => vt,
                None => return nope(span, "this list literal's element type"),
            },
            _ => return nope(span, "complex list literal elements (bind them to a variable first)"),
        })
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
                        VType::List(_) => {
                            // Stringify via display: reuse out path? Build str
                            // from list display not exposed; N2: unsupported.
                            return nope(inner.span(), "interpolating lists");
                        }
                        VType::Unit => return nope(inner.span(), "interpolating unit"),
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
        let (rv, rt) = self.eval_expr_val(recv)?;
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
            return nope(span, "operators on lists");
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
            _ => {}
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
                                VType::Str | VType::List(_) => {
                                    self.call_rt("grasm_out_val", &[v]);
                                    self.call_rt("grasm_val_dec", &[v]);
                                }
                                VType::Unit => return nope(inner.span(), "printing unit"),
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
                    VType::Str | VType::List(_) => {
                        self.call_rt("grasm_out_val", &[v]);
                        self.call_rt("grasm_val_dec", &[v]);
                    }
                    VType::Unit => return nope(other.span(), "printing unit"),
                }
            }
        }
        self.call_rt("grasm_out_nl", &[]);
        Ok(())
    }
}
