//! Effect-row inference: a syntactic call-graph fixpoint.
//!
//! Every function gets an inferred effect row (set of effect names; `io` is
//! the built-in effect of `print`). `with E handled_by …` blocks discharge
//! effect `E` for everything inside them. Declared rows (`/ {io, Clock}`)
//! are upper bounds; `main` and tests must end up with nothing but `io`.

use std::collections::{BTreeSet, HashMap, HashSet};

use grasm_syntax::ast::*;
use grasm_syntax::diag::Diagnostic;

pub const IO: &str = "io";

pub fn check_effects(module: &Module) -> Vec<Diagnostic> {
    let mut fns: HashMap<&str, &FnDecl> = HashMap::new();
    let mut effect_names: HashSet<&str> = HashSet::new();
    for d in &module.decls {
        match d {
            Decl::Fn(f) => {
                fns.insert(f.name.name.as_str(), f);
            }
            Decl::Effect(e) => {
                effect_names.insert(e.name.name.as_str());
            }
            _ => {}
        }
    }

    // Fixpoint over fn rows.
    let mut rows: HashMap<String, BTreeSet<String>> = fns
        .keys()
        .map(|k| (k.to_string(), BTreeSet::new()))
        .collect();
    loop {
        let mut changed = false;
        for (name, f) in &fns {
            let mut row = BTreeSet::new();
            let mut active = Vec::new();
            walk_block(&f.body, &mut active, &rows, &effect_names, &fns, &mut row);
            let entry = rows.get_mut(*name).unwrap();
            if &row != entry {
                *entry = row;
                changed = true;
            }
        }
        if !changed {
            break;
        }
    }

    let mut diags = Vec::new();

    // Declared rows are upper bounds.
    for (name, f) in &fns {
        if let Some(declared) = &f.effects {
            let declared_set: BTreeSet<String> =
                declared.iter().map(|p| p.dotted()).collect();
            for d in &declared_set {
                let base = d.split('.').next().unwrap_or(d);
                if base != IO && !effect_names.contains(base) {
                    diags.push(Diagnostic::error(
                        f.name.span,
                        format!("function `{name}` declares unknown effect `{d}`"),
                    ));
                }
            }
            let inferred = &rows[*name];
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
                        format!("function `{name}` performs effect(s) {list} not in its declared row"),
                    )
                    .with_help("add them to the `/ { ... }` clause or handle them inside"),
                );
            }
        }
    }

    // main: only `io` may remain unhandled.
    if let Some(main) = fns.get("main") {
        let leftover: Vec<&String> = rows["main"].iter().filter(|e| *e != IO).collect();
        if !leftover.is_empty() {
            let list = leftover
                .iter()
                .map(|m| format!("`{m}`"))
                .collect::<Vec<_>>()
                .join(", ");
            diags.push(
                Diagnostic::error(
                    main.name.span,
                    format!("`main` has unhandled effect(s) {list}"),
                )
                .with_help(
                    "wrap the calls in `with <Effect> handled_by <handler> { ... }`",
                ),
            );
        }
    }

    // Tests: same rule as main.
    for d in &module.decls {
        if let Decl::Test(t) = d {
            let mut row = BTreeSet::new();
            let mut active = Vec::new();
            walk_block(&t.body, &mut active, &rows, &effect_names, &fns, &mut row);
            let leftover: Vec<&String> = row.iter().filter(|e| *e != IO).collect();
            if !leftover.is_empty() {
                let list = leftover
                    .iter()
                    .map(|m| format!("`{m}`"))
                    .collect::<Vec<_>>()
                    .join(", ");
                diags.push(
                    Diagnostic::error(
                        t.span,
                        format!("test \"{}\" has unhandled effect(s) {list}", t.name),
                    )
                    .with_help("install a test handler with `with <Effect> handled_by ...`"),
                );
            }
        }
    }

    diags
}

fn contribute(
    callee: &str,
    active: &[String],
    rows: &HashMap<String, BTreeSet<String>>,
    out: &mut BTreeSet<String>,
) {
    if let Some(row) = rows.get(callee) {
        for e in row {
            if !active.contains(e) {
                out.insert(e.clone());
            }
        }
    }
}

fn walk_block(
    b: &Block,
    active: &mut Vec<String>,
    rows: &HashMap<String, BTreeSet<String>>,
    effects: &HashSet<&str>,
    fns: &HashMap<&str, &FnDecl>,
    out: &mut BTreeSet<String>,
) {
    for s in &b.stmts {
        walk_stmt(s, active, rows, effects, fns, out);
    }
}

fn walk_stmt(
    s: &Stmt,
    active: &mut Vec<String>,
    rows: &HashMap<String, BTreeSet<String>>,
    effects: &HashSet<&str>,
    fns: &HashMap<&str, &FnDecl>,
    out: &mut BTreeSet<String>,
) {
    match s {
        Stmt::Let { init, .. } => walk_expr(init, active, rows, effects, fns, out),
        Stmt::Assign { target, value, .. } => {
            walk_expr(target, active, rows, effects, fns, out);
            walk_expr(value, active, rows, effects, fns, out);
        }
        Stmt::Expr(e) => walk_expr(e, active, rows, effects, fns, out),
        Stmt::Return(v, _) => {
            if let Some(e) = v {
                walk_expr(e, active, rows, effects, fns, out);
            }
        }
        Stmt::Break(_) | Stmt::Continue(_) => {}
        Stmt::While { cond, body, .. } => {
            walk_expr(cond, active, rows, effects, fns, out);
            walk_block(body, active, rows, effects, fns, out);
        }
        Stmt::For { iter, body, .. } => {
            walk_expr(iter, active, rows, effects, fns, out);
            walk_block(body, active, rows, effects, fns, out);
        }
        Stmt::With {
            effect,
            handler,
            body,
            ..
        } => {
            walk_expr(handler, active, rows, effects, fns, out);
            active.push(effect.dotted());
            walk_block(body, active, rows, effects, fns, out);
            active.pop();
        }
    }
}

fn walk_expr(
    e: &Expr,
    active: &mut Vec<String>,
    rows: &HashMap<String, BTreeSet<String>>,
    effects: &HashSet<&str>,
    fns: &HashMap<&str, &FnDecl>,
    out: &mut BTreeSet<String>,
) {
    match e {
        Expr::Int(..) | Expr::Float(..) | Expr::Bool(..) | Expr::Ident(_) => {}
        Expr::Str(pieces, _) => {
            for p in pieces {
                if let StrPiece::Expr(inner) = p {
                    walk_expr(inner, active, rows, effects, fns, out);
                }
            }
        }
        Expr::List(items, _) | Expr::Tuple(items, _) => {
            for i in items {
                walk_expr(i, active, rows, effects, fns, out);
            }
        }
        Expr::StructLit { fields, .. } => {
            for (_, v) in fields {
                walk_expr(v, active, rows, effects, fns, out);
            }
        }
        Expr::Call { callee, args, .. } => {
            if let Expr::Ident(id) = &**callee {
                let io_builtin = matches!(
                    id.name.as_str(),
                    "print" | "read_file" | "write_file" | "file_exists" | "input" | "args"
                );
                if io_builtin && !active.iter().any(|a| a == IO) {
                    out.insert(IO.to_string());
                }
                if fns.contains_key(id.name.as_str()) {
                    contribute(&id.name, active, rows, out);
                }
            } else {
                walk_expr(callee, active, rows, effects, fns, out);
            }
            for a in args {
                walk_expr(a, active, rows, effects, fns, out);
            }
        }
        Expr::MethodCall {
            recv, name, args, ..
        } => {
            if let Expr::Ident(tid) = &**recv {
                if effects.contains(tid.name.as_str()) {
                    let ename = tid.name.clone();
                    if !active.contains(&ename) {
                        out.insert(ename);
                    }
                } else {
                    walk_expr(recv, active, rows, effects, fns, out);
                }
            } else {
                walk_expr(recv, active, rows, effects, fns, out);
            }
            // UFCS or list.map(f): the named fn contributes too.
            if fns.contains_key(name.name.as_str()) {
                contribute(&name.name, active, rows, out);
            }
            for a in args {
                walk_expr(a, active, rows, effects, fns, out);
            }
        }
        Expr::Field { recv, .. } => walk_expr(recv, active, rows, effects, fns, out),
        Expr::Index { recv, index, .. } => {
            walk_expr(recv, active, rows, effects, fns, out);
            walk_expr(index, active, rows, effects, fns, out);
        }
        Expr::Unary { expr, .. } | Expr::Try { expr, .. } => {
            walk_expr(expr, active, rows, effects, fns, out)
        }
        Expr::Binary { lhs, rhs, .. } => {
            walk_expr(lhs, active, rows, effects, fns, out);
            walk_expr(rhs, active, rows, effects, fns, out);
        }
        Expr::Range { start, end, .. } => {
            walk_expr(start, active, rows, effects, fns, out);
            walk_expr(end, active, rows, effects, fns, out);
        }
        Expr::If {
            cond,
            then_block,
            else_branch,
            ..
        } => {
            walk_expr(cond, active, rows, effects, fns, out);
            walk_block(then_block, active, rows, effects, fns, out);
            if let Some(eb) = else_branch {
                match &**eb {
                    ElseBranch::Else(b) => walk_block(b, active, rows, effects, fns, out),
                    ElseBranch::ElseIf(e) => walk_expr(e, active, rows, effects, fns, out),
                }
            }
        }
        Expr::Match {
            scrutinee, arms, ..
        } => {
            walk_expr(scrutinee, active, rows, effects, fns, out);
            for arm in arms {
                if let Some(g) = &arm.guard {
                    walk_expr(g, active, rows, effects, fns, out);
                }
                walk_expr(&arm.body, active, rows, effects, fns, out);
            }
        }
        Expr::Block(b) => walk_block(b, active, rows, effects, fns, out),
        Expr::Closure { body, .. } => walk_expr(body, active, rows, effects, fns, out),
    }
}
