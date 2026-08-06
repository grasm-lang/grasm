//! Parser integration tests.

use grasm_syntax::ast::*;
use grasm_syntax::parse;

fn parse_ok(src: &str) -> Module {
    let out = parse(src);
    assert!(
        out.diagnostics.is_empty(),
        "unexpected diagnostics for:\n{src}\n{:#?}",
        out.diagnostics
    );
    out.module
}

fn parse_err(src: &str) -> (Module, Vec<grasm_syntax::Diagnostic>) {
    let out = parse(src);
    assert!(
        !out.diagnostics.is_empty(),
        "expected diagnostics for:\n{src}"
    );
    (out.module, out.diagnostics)
}

/// First declaration as a function.
fn first_fn(m: &Module) -> &FnDecl {
    match &m.decls[0] {
        Decl::Fn(f) => f,
        other => panic!("expected fn, got {other:?}"),
    }
}

#[test]
fn empty_module() {
    let m = parse_ok("");
    assert!(m.decls.is_empty());
    let m = parse_ok("\n\n\n");
    assert!(m.decls.is_empty());
}

#[test]
fn minimal_fn() {
    let m = parse_ok("fn main() {\n}\n");
    let f = first_fn(&m);
    assert_eq!(f.name.name, "main");
    assert!(!f.is_pub);
    assert!(f.params.is_empty());
    assert!(f.ret.is_none());
    assert!(f.body.stmts.is_empty());
}

#[test]
fn fn_with_everything() {
    let src = "pub fn load[T](path: str, inout buf: [u8], own conn: Conn) -> Config ! (IoError | ParseError) / {io.fs, io.net} {\n    return\n}\n";
    let m = parse_ok(src);
    let f = first_fn(&m);
    assert!(f.is_pub);
    assert_eq!(f.generics.len(), 1);
    assert_eq!(f.params.len(), 3);
    assert_eq!(f.params[0].mode, ParamMode::Let);
    assert_eq!(f.params[1].mode, ParamMode::Inout);
    assert_eq!(f.params[2].mode, ParamMode::Own);
    assert!(matches!(f.params[1].ty, TypeExpr::List(..)));
    assert!(f.ret.is_some());
    assert!(matches!(f.err, Some(TypeExpr::Union(ref ms, _)) if ms.len() == 2));
    let fx = f.effects.as_ref().unwrap();
    assert_eq!(fx.len(), 2);
    assert_eq!(fx[0].dotted(), "io.fs");
    assert_eq!(fx[1].dotted(), "io.net");
}

#[test]
fn let_var_and_assign() {
    let src = "fn f() {\n    let x = 1\n    var y: f64 = 2.5\n    y = 3.0\n    y += 1.0\n    x.field[0] = 9\n}\n";
    let m = parse_ok(src);
    let f = first_fn(&m);
    assert_eq!(f.body.stmts.len(), 5);
    assert!(matches!(
        f.body.stmts[0],
        Stmt::Let { mutable: false, .. }
    ));
    assert!(matches!(
        &f.body.stmts[1],
        Stmt::Let {
            mutable: true,
            ty: Some(_),
            ..
        }
    ));
    assert!(matches!(
        f.body.stmts[2],
        Stmt::Assign {
            op: AssignOp::Assign,
            ..
        }
    ));
    assert!(matches!(
        f.body.stmts[3],
        Stmt::Assign {
            op: AssignOp::Add,
            ..
        }
    ));
    assert!(matches!(
        &f.body.stmts[4],
        Stmt::Assign {
            target: Expr::Index { .. },
            ..
        }
    ));
}

#[test]
fn precedence() {
    let m = parse_ok("fn f() {\n    let x = 1 + 2 * 3\n}\n");
    let f = first_fn(&m);
    let Stmt::Let { init, .. } = &f.body.stmts[0] else {
        panic!()
    };
    // 1 + (2 * 3)
    let Expr::Binary {
        op: BinaryOp::Add,
        rhs,
        ..
    } = init
    else {
        panic!("expected top-level +, got {init:?}")
    };
    assert!(matches!(
        **rhs,
        Expr::Binary {
            op: BinaryOp::Mul,
            ..
        }
    ));
}

#[test]
fn power_is_right_assoc() {
    let m = parse_ok("fn f() {\n    let x = 2 ^ 3 ^ 2\n}\n");
    let f = first_fn(&m);
    let Stmt::Let { init, .. } = &f.body.stmts[0] else {
        panic!()
    };
    let Expr::Binary {
        op: BinaryOp::Pow,
        rhs,
        ..
    } = init
    else {
        panic!()
    };
    assert!(matches!(
        **rhs,
        Expr::Binary {
            op: BinaryOp::Pow,
            ..
        }
    ));
}

#[test]
fn comparison_and_logic() {
    let m = parse_ok("fn f() {\n    let ok = a < b and c >= d or not e\n}\n");
    let f = first_fn(&m);
    let Stmt::Let { init, .. } = &f.body.stmts[0] else {
        panic!()
    };
    // (a<b and c>=d) or (not e)
    let Expr::Binary {
        op: BinaryOp::Or, ..
    } = init
    else {
        panic!("expected or at top: {init:?}")
    };
}

#[test]
fn pipeline_desugars_to_call() {
    let m = parse_ok("fn f() {\n    let r = xs |> filter(pred) |> sum\n}\n");
    let f = first_fn(&m);
    let Stmt::Let { init, .. } = &f.body.stmts[0] else {
        panic!()
    };
    // sum(filter(xs, pred))
    let Expr::Call { callee, args, .. } = init else {
        panic!("expected call, got {init:?}")
    };
    assert!(matches!(&**callee, Expr::Ident(id) if id.name == "sum"));
    assert_eq!(args.len(), 1);
    let Expr::Call {
        callee: inner_callee,
        args: inner_args,
        ..
    } = &args[0]
    else {
        panic!()
    };
    assert!(matches!(&**inner_callee, Expr::Ident(id) if id.name == "filter"));
    assert_eq!(inner_args.len(), 2);
    assert!(matches!(&inner_args[0], Expr::Ident(id) if id.name == "xs"));
}

#[test]
fn method_calls_and_chains() {
    let m = parse_ok("fn f() {\n    let n = user.name.len()\n    let x = xs.map(|v| v * 2)\n}\n");
    let f = first_fn(&m);
    let Stmt::Let { init, .. } = &f.body.stmts[0] else {
        panic!()
    };
    let Expr::MethodCall { recv, name, .. } = init else {
        panic!("expected method call: {init:?}")
    };
    assert_eq!(name.name, "len");
    assert!(matches!(&**recv, Expr::Field { .. }));

    let Stmt::Let { init, .. } = &f.body.stmts[1] else {
        panic!()
    };
    let Expr::MethodCall { args, .. } = init else {
        panic!()
    };
    assert!(matches!(&args[0], Expr::Closure { params, .. } if params.len() == 1));
}

#[test]
fn if_else_chain() {
    let src = "fn f() {\n    if a { b() } else if c { d() } else { e() }\n}\n";
    let m = parse_ok(src);
    let f = first_fn(&m);
    let Stmt::Expr(Expr::If { else_branch, .. }) = &f.body.stmts[0] else {
        panic!()
    };
    let Some(eb) = else_branch else { panic!() };
    let ElseBranch::ElseIf(Expr::If {
        else_branch: inner, ..
    }) = &**eb
    else {
        panic!()
    };
    assert!(matches!(inner.as_deref(), Some(ElseBranch::Else(_))));
}

#[test]
fn else_after_newline() {
    let src = "fn f() {\n    if a {\n        b()\n    }\n    else {\n        c()\n    }\n}\n";
    let m = parse_ok(src);
    let f = first_fn(&m);
    assert_eq!(f.body.stmts.len(), 1);
    let Stmt::Expr(Expr::If { else_branch, .. }) = &f.body.stmts[0] else {
        panic!()
    };
    assert!(else_branch.is_some());
}

#[test]
fn if_condition_no_struct_literal_ambiguity() {
    // `x` followed by block must parse as condition + block, not struct lit.
    let src = "fn f() {\n    if x { y() }\n    while n { z() }\n    for i in xs { w() }\n}\n";
    let m = parse_ok(src);
    let f = first_fn(&m);
    assert_eq!(f.body.stmts.len(), 3);
}

#[test]
fn struct_literal_in_normal_position() {
    let src = "fn f() {\n    let u = User{ id: 1, name: \"ada\" }\n    let v = Point{\n        x: 1.0\n        y: 2.0\n    }\n}\n";
    let m = parse_ok(src);
    let f = first_fn(&m);
    let Stmt::Let { init, .. } = &f.body.stmts[0] else {
        panic!()
    };
    let Expr::StructLit { fields, .. } = init else {
        panic!("expected struct lit: {init:?}")
    };
    assert_eq!(fields.len(), 2);
    let Stmt::Let { init, .. } = &f.body.stmts[1] else {
        panic!()
    };
    assert!(matches!(init, Expr::StructLit { fields, .. } if fields.len() == 2));
}

#[test]
fn match_with_patterns_and_guards() {
    let src = r#"fn f(s: Shape) -> f64 {
    match s {
        Circle(r) -> 3.14 * r ^ 2
        Rect(w, h) if w > 0 -> w * h
        Shape.Line -> 0.0
        _ -> 0.0
    }
}
"#;
    let m = parse_ok(src);
    let f = first_fn(&m);
    let Stmt::Expr(Expr::Match { arms, .. }) = &f.body.stmts[0] else {
        panic!()
    };
    assert_eq!(arms.len(), 4);
    assert!(matches!(&arms[0].pattern, Pattern::Variant { fields, .. } if fields.len() == 1));
    assert!(arms[1].guard.is_some());
    assert!(
        matches!(&arms[2].pattern, Pattern::Variant { path, fields, .. } if path.dotted() == "Shape.Line" && fields.is_empty())
    );
    assert!(matches!(&arms[3].pattern, Pattern::Wildcard(_)));
}

#[test]
fn sum_type_decl_multiline() {
    let src = "type Shape =\n    | Circle(r: f64)\n    | Rect(w: f64, h: f64)\n    | Line\n";
    let m = parse_ok(src);
    let Decl::Type(t) = &m.decls[0] else { panic!() };
    let TypeBody::Sum(variants) = &t.body else {
        panic!()
    };
    assert_eq!(variants.len(), 3);
    assert_eq!(variants[0].fields.len(), 1);
    assert_eq!(variants[0].fields[0].name.name, "r");
    assert!(variants[2].fields.is_empty());
}

#[test]
fn struct_type_decl() {
    let src = "pub type User = {\n    id: u64\n    name: str,\n    email: Option[str]\n}\n";
    let m = parse_ok(src);
    let Decl::Type(t) = &m.decls[0] else { panic!() };
    assert!(t.is_pub);
    let TypeBody::Struct(fields) = &t.body else {
        panic!()
    };
    assert_eq!(fields.len(), 3);
    assert!(matches!(&fields[2].ty, TypeExpr::Name { args, .. } if args.len() == 1));
}

#[test]
fn alias_type_decl() {
    let m = parse_ok("type Port = i32\n");
    let Decl::Type(t) = &m.decls[0] else { panic!() };
    assert!(matches!(t.body, TypeBody::Alias(_)));
}

#[test]
fn effect_decl() {
    let src = "effect Clock {\n    fn now() -> Instant\n    fn sleep(ms: i64)\n}\n";
    let m = parse_ok(src);
    let Decl::Effect(e) = &m.decls[0] else {
        panic!()
    };
    assert_eq!(e.name.name, "Clock");
    assert_eq!(e.ops.len(), 2);
    assert_eq!(e.ops[0].name.name, "now");
    assert!(e.ops[0].ret.is_some());
    assert_eq!(e.ops[1].params.len(), 1);
}

#[test]
fn test_decl_and_import() {
    let src = "import math\nimport net.http\n\ntest \"adds\" {\n    assert_eq(1 + 1, 2)\n}\n";
    let m = parse_ok(src);
    assert_eq!(m.decls.len(), 3);
    assert!(matches!(&m.decls[0], Decl::Import(i) if i.path.dotted() == "math"));
    assert!(matches!(&m.decls[1], Decl::Import(i) if i.path.dotted() == "net.http"));
    assert!(matches!(&m.decls[2], Decl::Test(t) if t.name == "adds"));
}

#[test]
fn string_interpolation_parses_exprs() {
    let src = "fn f() {\n    print(\"hi {name}, next {age + 1}\")\n}\n";
    let m = parse_ok(src);
    let f = first_fn(&m);
    let Stmt::Expr(Expr::Call { args, .. }) = &f.body.stmts[0] else {
        panic!()
    };
    let Expr::Str(pieces, _) = &args[0] else {
        panic!()
    };
    assert_eq!(pieces.len(), 4);
    assert!(matches!(&pieces[1], StrPiece::Expr(Expr::Ident(id)) if id.name == "name"));
    assert!(matches!(
        &pieces[3],
        StrPiece::Expr(Expr::Binary {
            op: BinaryOp::Add,
            ..
        })
    ));
}

#[test]
fn ranges_and_for() {
    let src = "fn f() {\n    for i in 0..10 { g(i) }\n    for j in 1..=n { g(j) }\n}\n";
    let m = parse_ok(src);
    let f = first_fn(&m);
    let Stmt::For { iter, .. } = &f.body.stmts[0] else {
        panic!()
    };
    assert!(matches!(
        iter,
        Expr::Range {
            inclusive: false,
            ..
        }
    ));
    let Stmt::For { iter, .. } = &f.body.stmts[1] else {
        panic!()
    };
    assert!(matches!(iter, Expr::Range { inclusive: true, .. }));
}

#[test]
fn try_and_coalesce_and_optional_chain() {
    let src = "fn f() {\n    let cfg = try load(path)\n    let port = cfg.port ?? 8080\n}\n";
    let m = parse_ok(src);
    let f = first_fn(&m);
    assert!(matches!(
        &f.body.stmts[0],
        Stmt::Let {
            init: Expr::Try { .. },
            ..
        }
    ));
    assert!(matches!(
        &f.body.stmts[1],
        Stmt::Let {
            init: Expr::Binary {
                op: BinaryOp::Coalesce,
                ..
            },
            ..
        }
    ));
}

#[test]
fn multiline_expressions_inside_parens() {
    let src = "fn f() {\n    let x = g(\n        1,\n        2,\n    )\n    let ys = [\n        1,\n        2,\n    ]\n}\n";
    let m = parse_ok(src);
    let f = first_fn(&m);
    assert_eq!(f.body.stmts.len(), 2);
}

#[test]
fn operator_at_line_end_continues() {
    let src = "fn f() {\n    let x = 1 +\n        2 +\n        3\n}\n";
    let m = parse_ok(src);
    let f = first_fn(&m);
    assert_eq!(f.body.stmts.len(), 1);
}

#[test]
fn closures() {
    let src = "fn f() {\n    let id = |x| x\n    let add = |a, b| a + b\n    let block = |v| {\n        let d = v * 2\n        d\n    }\n    let thunk = || 42\n}\n";
    let m = parse_ok(src);
    let f = first_fn(&m);
    assert_eq!(f.body.stmts.len(), 4);
    let Stmt::Let { init, .. } = &f.body.stmts[3] else {
        panic!()
    };
    assert!(matches!(init, Expr::Closure { params, .. } if params.is_empty()));
}

#[test]
fn error_recovery_continues_parsing() {
    let src = "fn broken() {\n    let = 5\n}\n\nfn good() {\n    let x = 1\n}\n";
    let (m, diags) = parse_err(src);
    assert!(!diags.is_empty());
    // `good` still parsed.
    assert!(m
        .decls
        .iter()
        .any(|d| matches!(d, Decl::Fn(f) if f.name.name == "good")));
}

#[test]
fn error_messages_are_specific() {
    let (_, diags) = parse_err("fn f( {\n}\n");
    assert!(
        diags[0].message.contains("identifier"),
        "got: {}",
        diags[0].message
    );
}

#[test]
fn invalid_assignment_target_diagnosed() {
    let (_, diags) = parse_err("fn f() {\n    1 + 2 = 3\n}\n");
    assert!(diags
        .iter()
        .any(|d| d.message.contains("invalid assignment target")));
}

#[test]
fn full_realistic_program() {
    let src = r#"// stats.gr
import math

type Stats = { mean: f64, sd: f64 }

type Shape =
    | Circle(r: f64)
    | Rect(w: f64, h: f64)

fn area(s: Shape) -> f64 {
    match s {
        Circle(r) -> math.pi * r ^ 2
        Rect(w, h) -> w * h
    }
}

pub fn stats(xs: [f64]) -> Stats {
    let n = xs.len()
    let mean = xs.sum() / n
    var acc = 0.0
    for x in xs {
        acc += (x - mean) ^ 2
    }
    Stats{ mean: mean, sd: math.sqrt(acc / n) }
}

fn main() {
    let shapes = [Circle(1.0), Rect(2.0, 3.0)]
    var total = 0.0
    for s in shapes {
        total += area(s)
    }
    print("total area: {total}")
}

test "area of rect" {
    assert_eq(area(Rect(2.0, 3.0)), 6.0)
}
"#;
    let m = parse_ok(src);
    assert_eq!(m.decls.len(), 7);
}

#[test]
fn recovery_from_unbalanced_parens_resets_newline_mode() {
    // Error inside `(` — recovery must not leave newline-skipping mode on,
    // otherwise `good`'s newline-terminated statements would misparse.
    let src = "fn broken() {\n    let x = g(1, +\n}\n\nfn good() {\n    let a = 1\n    let b = 2\n}\n";
    let out = grasm_syntax::parse(src);
    assert!(!out.diagnostics.is_empty());
    let good = out.module.decls.iter().find_map(|d| match d {
        Decl::Fn(f) if f.name.name == "good" => Some(f),
        _ => None,
    });
    let good = good.expect("`good` fn should have been parsed after recovery");
    assert_eq!(good.body.stmts.len(), 2);
}

#[test]
fn parallel_and_spawn_are_contextual_keywords() {
    // As concurrency scopes when a block follows...
    let m = parse_ok(
        "fn main() {\n    parallel {\n        spawn {\n            f()\n        }\n    }\n}\n",
    );
    let f = first_fn(&m);
    let Stmt::Parallel { body, .. } = &f.body.stmts[0] else {
        panic!("expected a parallel scope, got {:?}", f.body.stmts[0])
    };
    assert!(matches!(body.stmts[0], Stmt::Spawn { .. }));

    // ...and as ordinary names everywhere else — existing code must not break.
    let m = parse_ok(
        "fn spawn(n: i64) -> i64 {\n    n\n}\n\nfn main() {\n    let parallel = 1\n    let x = spawn(parallel)\n    let y = obj.spawn(2)\n}\n",
    );
    assert!(matches!(&m.decls[0], Decl::Fn(f) if f.name.name == "spawn"));
}

// ---------------------------------------------------------------------------
// Property tests and benchmarks
// ---------------------------------------------------------------------------

#[test]
fn a_test_without_parameters_is_a_unit_test() {
    let m = parse_ok("test \"plain\" {\n    assert(true)\n}\n");
    let Decl::Test(t) = &m.decls[0] else {
        panic!("expected a test declaration");
    };
    assert_eq!(t.name, "plain");
    assert!(t.params.is_empty());
}

#[test]
fn parameters_turn_a_test_into_a_property() {
    let m = parse_ok("test \"prop\" (n: i64, xs: [str]) {\n    assert(true)\n}\n");
    let Decl::Test(t) = &m.decls[0] else {
        panic!("expected a test declaration");
    };
    assert_eq!(t.params.len(), 2);
    assert_eq!(t.params[0].name.name, "n");
    assert_eq!(t.params[1].name.name, "xs");
}

#[test]
fn bench_declarations_parse() {
    let m = parse_ok("bench \"hot loop\" {\n    let x = 1\n    assert(x == 1)\n}\n");
    let Decl::Bench(b) = &m.decls[0] else {
        panic!("expected a bench declaration");
    };
    assert_eq!(b.name, "hot loop");
    assert_eq!(b.body.stmts.len(), 2);
}

#[test]
fn bench_is_contextual_and_still_usable_as_a_name() {
    // Only `bench "..."` at declaration position is the keyword; everywhere
    // else `bench` is an ordinary identifier.
    let m = parse_ok(
        "fn bench(n: i64) -> i64 {\n    n\n}\n\nbench \"b\" {\n    let bench = 1\n    assert(bench == 1)\n}\n",
    );
    assert!(matches!(&m.decls[0], Decl::Fn(f) if f.name.name == "bench"));
    assert!(matches!(&m.decls[1], Decl::Bench(_)));
}

#[test]
fn a_bench_without_a_name_is_an_error() {
    // `bench` alone is an identifier, so this fails as a bad declaration
    // rather than silently parsing as something else.
    let (_, diags) = parse_err("bench {\n}\n");
    assert!(
        diags[0].message.contains("expected a declaration"),
        "got: {}",
        diags[0].message
    );
}

#[test]
fn declaration_recovery_always_makes_progress() {
    // Every one of these fails at declaration position. If recovery ever
    // stops consuming tokens the parser spins forever, so this test is a
    // liveness check as much as a diagnostics check.
    for src in [
        "bench {\n}\n",
        "bench\n",
        "bench bench\n",
        "42\n",
        "let x = 1\n",
        "]\n",
        "bench {\n}\nfn main() {\n}\n",
    ] {
        let out = grasm_syntax::parse(src);
        assert!(
            !out.diagnostics.is_empty(),
            "expected diagnostics for: {src:?}"
        );
    }
    // Recovery still finds the declaration after the bad one.
    let out = grasm_syntax::parse("bench {\n}\n\nfn after() {\n}\n");
    assert!(
        out.module
            .decls
            .iter()
            .any(|d| matches!(d, Decl::Fn(f) if f.name.name == "after")),
        "recovery should resume at the next declaration"
    );
}

// ---------------------------------------------------------------------------
// Documentation comments
// ---------------------------------------------------------------------------

#[test]
fn doc_comments_attach_to_the_following_declaration() {
    let m = parse_ok(
        "/// Adds two numbers.\n/// Wraps on overflow.\nfn add(a: i64, b: i64) -> i64 {\n    a + b\n}\n",
    );
    assert_eq!(
        m.doc_for(&m.decls[0]),
        Some("Adds two numbers.\nWraps on overflow.")
    );
}

#[test]
fn a_blank_line_ends_a_doc_block() {
    // The first block documents nothing that follows it directly, so only the
    // adjacent block reaches the declaration.
    let m = parse_ok("/// Stray note.\n\n/// The real doc.\nfn f() {\n}\n");
    assert_eq!(m.doc_for(&m.decls[0]), Some("The real doc."));
}

#[test]
fn plain_comments_are_not_documentation() {
    let m = parse_ok("// Just a comment.\n//// Also just a comment.\nfn f() {\n}\n");
    assert_eq!(m.doc_for(&m.decls[0]), None);
}

#[test]
fn doc_comments_work_on_every_declaration_kind() {
    let m = parse_ok(
        "/// A type.\ntype T = { x: i64 }\n\n/// An effect.\neffect E {\n    fn op() -> i64\n}\n\n/// A test.\ntest \"t\" {\n    assert(true)\n}\n\n/// A benchmark.\nbench \"b\" {\n    assert(true)\n}\n",
    );
    let docs: Vec<Option<&str>> = m.decls.iter().map(|d| m.doc_for(d)).collect();
    assert_eq!(
        docs,
        vec![
            Some("A type."),
            Some("An effect."),
            Some("A test."),
            Some("A benchmark."),
        ]
    );
}

#[test]
fn a_trailing_doc_comment_is_dropped_not_misattached() {
    let m = parse_ok("fn f() {\n}\n\n/// Documents nothing.\n");
    assert_eq!(m.doc_for(&m.decls[0]), None);
}

#[test]
fn slashes_inside_a_string_are_not_documentation() {
    let m = parse_ok("fn f() -> str {\n    \"/// not a doc\"\n}\n");
    assert_eq!(m.doc_for(&m.decls[0]), None);
    assert!(m.docs.is_empty());
}

/// `{` in expression position is a map literal or a block, and the two are
/// told apart by whether a `:` sits where a first entry's colon would. These
/// pin that split, because getting it wrong silently changes what a program
/// means rather than failing to parse.
#[test]
fn braces_in_expression_position_split_into_maps_and_blocks() {
    fn body_expr(src: &str) -> Expr {
        let m = parse_ok(src);
        let Decl::Fn(f) = &m.decls[0] else {
            panic!("expected a function")
        };
        match &f.body.stmts[0] {
            Stmt::Let { init, .. } => init.clone(),
            other => panic!("expected a let, got {other:?}"),
        }
    }

    // Entries make a map, with or without a trailing comma or newlines.
    assert!(matches!(
        body_expr("fn f() {\n    let m = {\"a\": 1}\n}\n"),
        Expr::Map(ref e, _) if e.len() == 1
    ));
    assert!(matches!(
        body_expr("fn f() {\n    let m = {\n        \"a\": 1,\n        \"b\": 2,\n    }\n}\n"),
        Expr::Map(ref e, _) if e.len() == 2
    ));
    // Nothing at all is the empty map, not an empty block.
    assert!(matches!(
        body_expr("fn f() {\n    let m = {}\n}\n"),
        Expr::Map(ref e, _) if e.is_empty()
    ));
    // Statements make a block.
    assert!(matches!(
        body_expr("fn f() {\n    let b = { let n = 1\n        n }\n}\n"),
        Expr::Block(_)
    ));
    // A single expression with no colon is a block, not a one-entry map.
    assert!(matches!(
        body_expr("fn f() {\n    let b = { 1 + 2 }\n}\n"),
        Expr::Block(_)
    ));
    // An annotation's colon belongs to a statement, so this stays a block —
    // the case where a naive scan for the first `:` reads a block as a map.
    assert!(matches!(
        body_expr("fn f() {\n    let b = { let n: i64 = 1\n        n }\n}\n"),
        Expr::Block(_)
    ));
}
