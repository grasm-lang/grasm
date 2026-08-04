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
}

pub fn check(module: &Module) -> Analysis {
    let mut c = Checker::new();
    c.collect_types(module);
    c.collect_fns(module);
    c.check_module(module);
    let mut diagnostics = c.diags;
    diagnostics.extend(crate::effects::check_effects(module));
    Analysis {
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
    /// effect name → (ops, handler struct TypeDefId).
    effects: HashMap<String, (Vec<EffectOp>, TypeDefId)>,
    /// variant name → (typedef, variant index); ambiguous names keep all.
    variants: HashMap<String, Vec<(TypeDefId, usize)>>,
    uni: Unifier,
    diags: Vec<Diagnostic>,
    scopes: Vec<HashMap<String, Binding>>,
    /// (ret, err) of the function being checked.
    fn_ret: Type,
    fn_err: Option<Type>,
    /// True for main/tests: any error type may propagate (runtime reports).
    err_open: bool,
    loop_depth: u32,
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
            effects: HashMap::new(),
            variants,
            uni: Unifier::new(),
            diags: Vec::new(),
            scopes: Vec::new(),
            fn_ret: Type::Unit,
            fn_err: None,
            err_open: false,
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
        for decl in &module.decls {
            if let Decl::Type(t) = decl {
                if let TypeBody::Alias(te) = &t.body {
                    let generics: Vec<String> = t.generics.iter().map(|g| g.name.clone()).collect();
                    let ty = self.resolve_type(te, &generics);
                    if !t.generics.is_empty() {
                        self.error(
                            t.name.span,
                            "generic type aliases are not supported yet".to_string(),
                        );
                    }
                    self.aliases.insert(t.name.name.clone(), ty);
                }
            }
        }
        // Effects: register ops + auto-generate `<Name>Handler` struct types.
        for decl in &module.decls {
            if let Decl::Effect(e) = decl {
                if self.effects.contains_key(&e.name.name) {
                    self.error(e.name.span, format!("duplicate effect `{}`", e.name.name));
                    continue;
                }
                let mut ops = Vec::new();
                let mut handler_fields = Vec::new();
                for op in &e.ops {
                    let params: Vec<Type> =
                        op.params.iter().map(|p| self.resolve_type(&p.ty, &[])).collect();
                    let ret = op
                        .ret
                        .as_ref()
                        .map(|t| self.resolve_type(t, &[]))
                        .unwrap_or(Type::Unit);
                    handler_fields.push((
                        op.name.name.clone(),
                        Type::Fn {
                            params: params.clone(),
                            ret: Box::new(ret.clone()),
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
        for (di, id) in ids {
            let Decl::Type(t) = &module.decls[di] else {
                continue;
            };
            let generics: Vec<String> = t.generics.iter().map(|g| g.name.clone()).collect();
            let kind = match &t.body {
                TypeBody::Struct(fields) => {
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

    fn collect_fns(&mut self, module: &Module) {
        for decl in &module.decls {
            if let Decl::Fn(f) = decl {
                if self.fns.contains_key(&f.name.name) {
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
                self.fns.insert(
                    f.name.name.clone(),
                    FnInfo {
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
            TypeExpr::Name { path, args, span } => {
                if path.segments.len() > 1 {
                    return self.error(*span, "qualified type paths are not supported yet");
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
                    self.scopes.push(HashMap::new());
                    self.check_block(&t.body);
                    self.scopes.pop();
                }
                _ => {}
            }
        }
    }

    fn check_fn(&mut self, f: &FnDecl) {
        let info = match self.fns.get(&f.name.name) {
            Some(i) => i.clone(),
            None => return,
        };
        // Generic params stay opaque (`Generic(i)`) while checking the body.
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
        let body_ty = self.check_block(&f.body);
        let ret = info.ret.clone();
        let ends_with_return = matches!(f.body.stmts.last(), Some(Stmt::Return(..)));
        if ret != Type::Unit && !ends_with_return && !self.uni.unify(&body_ty, &ret) {
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
    }

    // ------------------------------------------------------------------
    // Statements & blocks
    // ------------------------------------------------------------------

    /// Returns the block's value type (last expression statement, else unit).
    fn check_block(&mut self, b: &Block) -> Type {
        self.scopes.push(HashMap::new());
        let mut ty = Type::Unit;
        let n = b.stmts.len();
        for (i, stmt) in b.stmts.iter().enumerate() {
            ty = self.check_stmt(stmt);
            if i + 1 < n {
                ty = Type::Unit;
            }
        }
        self.scopes.pop();
        ty
    }

    fn check_stmt(&mut self, s: &Stmt) -> Type {
        match s {
            Stmt::Let {
                mutable,
                name,
                ty,
                init,
                ..
            } => {
                let bound_ty = if let Some(te) = ty {
                    let want = self.resolve_type(te, &[]);
                    self.check_expr(init, &want);
                    want
                } else {
                    let t = self.infer_expr(init);
                    let resolved = self.uni.shallow_resolve(&t);
                    if let Type::Fallible(..) = resolved {
                        self.error_help(
                            init.span(),
                            "this expression can fail — its error must be handled",
                            "use `try` to propagate it or `match` to handle both cases",
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
                Type::Unit
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
                Type::Unit
            }
            Stmt::While { cond, body, .. } => {
                self.check_expr(cond, &Type::Bool);
                self.loop_depth += 1;
                self.check_block(body);
                self.loop_depth -= 1;
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
                self.check_block(body);
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
                            "iterate over a list (`[T]`), a range (`0..n`), or a string",
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
    fn mark_moved(&mut self, id: &Ident) {
        let loop_depth = self.loop_depth;
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
            Type::I64 | Type::F64 | Type::Bool | Type::Unit | Type::Range
        )
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
    fn check_assign_target(&mut self, target: &Expr) {
        let mut root = target;
        while let Expr::Field { recv, .. } | Expr::Index { recv, .. } = root {
            root = recv;
        }
        match root {
            Expr::Ident(id) => match self.lookup(&id.name) {
                Some(b) if !b.mutable => {
                    self.error_help(
                        id.span,
                        format!("cannot assign to `{}` — it is immutable", id.name),
                        format!("declare it with `var {}` instead of `let`", id.name),
                    );
                }
                Some(_) => {}
                None => {
                    self.error(id.span, format!("unknown variable `{}`", id.name));
                }
            },
            other => {
                self.error(other.span(), "invalid assignment target");
            }
        }
    }

    // ------------------------------------------------------------------
    // Expressions
    // ------------------------------------------------------------------

    fn check_expr(&mut self, e: &Expr, expected: &Type) {
        let want = self.uni.shallow_resolve(expected);
        // Int literal adapts to f64.
        if let (Expr::Int(..), Type::F64) = (e, &want) {
            return;
        }
        if let (Expr::Unary { op: UnaryOp::Neg, expr, .. }, Type::F64) = (e, &want) {
            if matches!(**expr, Expr::Int(..)) {
                return;
            }
        }
        // Closures check against an expected fn type for param inference.
        if let (Expr::Closure { .. }, Type::Fn { .. }) = (e, &want) {
            let got = self.infer_closure(e, Some(&want));
            self.unify_or_report(e.span(), &got, &want);
            return;
        }
        let got = self.infer_expr(e);
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
            } else {
                self.error(span, format!("type mismatch: expected `{w}`, found `{g}`"));
            }
        }
    }

    fn infer_expr(&mut self, e: &Expr) -> Type {
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
            Expr::List(items, _) => {
                let elem = self.uni.fresh();
                for item in items {
                    self.check_expr(item, &elem);
                }
                Type::List(Box::new(elem))
            }
            Expr::Tuple(items, _) => {
                if items.is_empty() {
                    Type::Unit
                } else {
                    Type::Tuple(items.iter().map(|i| self.infer_expr(i)).collect())
                }
            }
            Expr::StructLit { name, fields, span } => self.infer_struct_lit(name, fields, *span),
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
                let then_ty = self.check_block(then_block);
                match else_branch {
                    Some(eb) => {
                        let else_ty = match &**eb {
                            ElseBranch::Else(b) => self.check_block(b),
                            ElseBranch::ElseIf(e) => self.infer_expr(e),
                        };
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
            Expr::Try { expr, span } => self.infer_try(expr, *span),
            Expr::Closure { .. } => self.infer_closure(e, None),
        }
    }

    fn infer_ident(&mut self, id: &Ident) -> Type {
        if let Some(b) = self.lookup(&id.name) {
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
        // Function referenced as a value.
        if let Some(info) = self.fns.get(&id.name).cloned() {
            if info.generics > 0 {
                let fresh: Vec<Type> = (0..info.generics).map(|_| self.uni.fresh()).collect();
                return Type::Fn {
                    params: info
                        .params
                        .iter()
                        .map(|p| substitute_generics(p, &fresh))
                        .collect(),
                    ret: Box::new(substitute_generics(&info.ret, &fresh)),
                };
            }
            return Type::Fn {
                params: info.params.clone(),
                ret: Box::new(info.ret.clone()),
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

    fn infer_struct_lit(&mut self, name: &Path, fields: &[(Ident, Expr)], span: Span) -> Type {
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
                    "input" => {
                        if !args.is_empty() {
                            return self.error(span, "input takes no arguments");
                        }
                        return Type::Str;
                    }
                    "fail" => {
                        if args.len() != 1 {
                            return self.error(span, "fail takes exactly 1 argument (the error value)");
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
                if let Some(info) = self.fns.get(&id.name).cloned() {
                    return self.check_fn_call(&id.name, &info, args, span);
                }
            }
        }
        // Qualified variant: `Shape.Circle(...)` parses as MethodCall on type
        // name and lands in infer_method_call, not here. General case:
        let callee_ty = self.infer_expr(callee);
        match self.uni.shallow_resolve(&callee_ty) {
            Type::Fn { params, ret } => {
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

    fn check_fn_call(&mut self, name: &str, info: &FnInfo, args: &[Expr], span: Span) -> Type {
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
                            match self.lookup(&id.name) {
                                Some(b) if !b.mutable => {
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
        let recv_ty = self.infer_expr(recv);
        match self.uni.shallow_resolve(&recv_ty) {
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
        // Effect operation call `Clock.now(...)`.
        if let Expr::Ident(tid) = recv {
            if self.lookup(&tid.name).is_none() {
                if let Some((ops, _)) = self.effects.get(&tid.name).cloned() {
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
        if let Some(info) = self.fns.get(&name.name).cloned() {
            let mut full_args: Vec<Expr> = Vec::with_capacity(args.len() + 1);
            full_args.push(recv.clone());
            full_args.extend(args.iter().cloned());
            return self.check_fn_call(&name.name, &info, &full_args, span);
        }
        let shown = self.show(&recv_ty);
        self.error_help(
            name.span,
            format!("no method `{}` on `{shown}`", name.name),
            format!("a free function `fn {}({shown}, ...)` would also work here (UFCS)", name.name),
        )
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
                        self.require_mutable(recv);
                        if check_arity(self, 1) {
                            self.check_expr(&args[0], &elem);
                        }
                        Type::Unit
                    }
                    "pop" => {
                        self.require_mutable(recv);
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
                            };
                            self.check_expr(&args[0], &f);
                        }
                        Type::List(Box::new(out))
                    }
                    "filter" => {
                        if check_arity(self, 1) {
                            let f = Type::Fn {
                                params: vec![elem.clone()],
                                ret: Box::new(Type::Bool),
                            };
                            self.check_expr(&args[0], &f);
                        }
                        Type::List(Box::new(elem))
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
                        self.require_mutable(recv);
                        check_arity(self, 0);
                        Type::Unit
                    }
                    _ => return None,
                }
            }
            Type::Str => match name.name.as_str() {
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
                "chars" => {
                    check_arity(self, 0);
                    Type::List(Box::new(Type::Str))
                }
                "trim" | "to_upper" | "to_lower" => {
                    check_arity(self, 0);
                    Type::Str
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
            Type::I64 => match name.name.as_str() {
                "as_f64" => {
                    check_arity(self, 0);
                    Type::F64
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
                "as_i64" => {
                    check_arity(self, 0);
                    Type::I64
                }
                "abs" | "sqrt" | "floor" | "ceil" | "round" => {
                    check_arity(self, 0);
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
                    _ => return None,
                }
            }
            _ => return None,
        };
        Some(t)
    }

    /// Mutating methods need a mutable receiver root.
    fn require_mutable(&mut self, recv: &Expr) {
        let mut root = recv;
        while let Expr::Field { recv, .. } | Expr::Index { recv, .. } = root {
            root = recv;
        }
        if let Expr::Ident(id) = root {
            if let Some(b) = self.lookup(&id.name) {
                if !b.mutable {
                    self.error_help(
                        id.span,
                        format!("cannot mutate `{}` — it is immutable", id.name),
                        format!("declare it with `var {}`", id.name),
                    );
                }
            }
        }
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
                let t = self.infer_expr(lhs);
                self.check_expr(rhs, &t);
                Type::Bool
            }
            Lt | LtEq | Gt | GtEq => {
                let t = self.infer_expr(lhs);
                self.check_expr(rhs, &t);
                let r = self.uni.shallow_resolve(&t);
                if let Type::Var(_) = r {
                    self.uni.unify(&t, &Type::I64);
                } else if !matches!(r, Type::I64 | Type::F64 | Type::Str | Type::Error) {
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
                    Type::Error => Type::Error,
                    _ => {
                        let shown = self.show(&lhs_ty);
                        self.error_help(
                            lhs.span(),
                            format!("`??` needs an `Option` on the left, found `{shown}`"),
                            "`a ?? b` unwraps `a` or falls back to `b`",
                        )
                    }
                }
            }
            Add | Sub | Mul | Div | Rem | Pow => {
                let lhs_ty = self.infer_expr(lhs);
                self.check_expr(rhs, &lhs_ty);
                let r = self.uni.shallow_resolve(&lhs_ty);
                match r {
                    Type::I64 | Type::F64 => r,
                    Type::Str if op == Add => Type::Str,
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
        let scrut_ty = self.infer_expr(scrutinee);
        let result = self.uni.fresh();
        let mut covered: Vec<String> = Vec::new();
        let mut has_irrefutable = false;
        for arm in arms {
            self.scopes.push(HashMap::new());
            let tag = self.check_pattern(&arm.pattern, &scrut_ty);
            if let Some(guard) = &arm.guard {
                self.check_expr(guard, &Type::Bool);
            } else {
                match tag {
                    None => has_irrefutable = true,
                    Some(t) => covered.push(t),
                }
            }
            self.check_expr(&arm.body, &result);
            self.scopes.pop();
        }
        // Exhaustiveness.
        if !has_irrefutable {
            let missing: Vec<String> = match self.uni.resolve(&scrut_ty) {
                Type::Named { id, .. } => match &self.defs.get(id).kind {
                    TypeDefKind::Sum { variants } => variants
                        .iter()
                        .map(|v| v.name.clone())
                        .filter(|n| !covered.contains(n))
                        .collect(),
                    TypeDefKind::Struct { .. } => vec!["_".into()],
                },
                Type::Bool => ["true", "false"]
                    .iter()
                    .map(|s| s.to_string())
                    .filter(|n| !covered.contains(n))
                    .collect(),
                Type::Fallible(..) => ["Ok", "Err"]
                    .iter()
                    .map(|s| s.to_string())
                    .filter(|n| !covered.contains(n))
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

    /// Checks a pattern against the expected type, binding names.
    /// Returns the covered constructor tag, or None if irrefutable.
    fn check_pattern(&mut self, pat: &Pattern, expected: &Type) -> Option<String> {
        match pat {
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
                            if fields.len() == 1 {
                                self.check_pattern(&fields[0], &ok);
                            } else {
                                self.error(*span, "`Ok` pattern takes exactly 1 binding");
                            }
                            return Some("Ok".into());
                        }
                        "Err" => {
                            if fields.len() == 1 {
                                self.check_pattern(&fields[0], &err);
                            } else {
                                self.error(*span, "`Err` pattern takes exactly 1 binding");
                            }
                            return Some("Err".into());
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
                        for (fpat, (_, fty)) in fields.iter().zip(&v.fields) {
                            let want = substitute_generics(fty, &targs);
                            self.check_pattern(fpat, &want);
                        }
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
        self.scopes.push(HashMap::new());
        for (p, t) in params.iter().zip(&param_tys) {
            self.bind(p, t.clone(), false);
        }
        let ret = self.infer_expr(body);
        self.scopes.pop();
        Type::Fn {
            params: param_tys,
            ret: Box::new(ret),
        }
    }
}

/// Root identifier of a place expression (`x`, `x.f`, `x[i].g`), if any.
fn place_root(e: &Expr) -> Option<&Ident> {
    match e {
        Expr::Ident(id) => Some(id),
        Expr::Field { recv, .. } | Expr::Index { recv, .. } => place_root(recv),
        _ => None,
    }
}

fn stmt_span(s: &Stmt) -> Span {
    match s {
        Stmt::Let { span, .. }
        | Stmt::Assign { span, .. }
        | Stmt::Return(_, span)
        | Stmt::Break(span)
        | Stmt::Continue(span)
        | Stmt::While { span, .. }
        | Stmt::For { span, .. }
        | Stmt::With { span, .. } => *span,
        Stmt::Expr(e) => e.span(),
    }
}
