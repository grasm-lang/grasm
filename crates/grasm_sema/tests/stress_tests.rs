//! Adversarial + stress tests: the compiler must diagnose, never crash.

use grasm_sema::analyze;

/// The stack every entry point gives the compiler (`COMPILER_STACK` in the
/// binary). The parser's nesting limit is set against this number, so a test
/// that runs the checker on the harness's own small stack is not testing what
/// ships: 300 nested blocks — well inside the limit, and fine from the
/// command line — aborted the test binary with a stack overflow.
const COMPILER_STACK: usize = 256 * 1024 * 1024;

/// Analyze must not panic, whatever the input.
fn survives(src: &str) {
    let src = src.to_string();
    std::thread::Builder::new()
        .stack_size(COMPILER_STACK)
        .spawn(move || {
            let _ = analyze(&src);
        })
        .expect("spawn")
        .join()
        .expect("the checker must diagnose, not panic");
}

#[test]
fn garbage_inputs_do_not_crash() {
    let cases = [
        "",
        "\0\0\0",
        "fn",
        "fn (",
        "fn main( {",
        "}}}}}}",
        "((((((((",
        "let x = ",
        "type = |||",
        "effect effect effect",
        "\"unterminated",
        "fn main() { \"{unclosed\" }",
        "match { -> }",
        "with with with",
        "🎉🎉🎉 fn é() {}",
        "fn main() { 999999999999999999999999999 }",
        "x" ,
        "// only a comment",
        "/* unclosed block comment",
        "fn main() {\n    let x = 1 +\n",
        "test \"t\" { ] }",
        "fn f(own own own: own) {}",
        "fn f() -> ! {}",
        "fn f() / {} {}",
        "type T = { x: }",
        "fn f() { a.b.c.d.e.f.g.h.i.j() }",
        "fn f() { ((((((((((1)))))))))) }",
    ];
    for c in cases {
        survives(c);
    }
}

#[test]
fn truncation_fuzz_does_not_crash() {
    // Every prefix of a real program must parse-or-diagnose without panic.
    let src = r#"type Shape = | Circle(r: f64) | Rect(w: f64, h: f64)

effect Clock {
    fn now() -> i64
}

fn area(s: Shape) -> f64 ! IoError / {io} {
    match s {
        Circle(r) if r > 0.0 -> 3.14 * r ^ 2
        _ -> 0.0
    }
}

fn main() {
    with Clock handled_by ClockHandler{ now: || 1 } {
        print("area: {area(Circle(2.0))}")
    }
}
"#;
    for i in 0..src.len() {
        if src.is_char_boundary(i) {
            survives(&src[..i]);
        }
    }
}

#[test]
fn deep_nesting_does_not_crash() {
    // 300 nested blocks.
    let mut src = String::from("fn main() {\n");
    for _ in 0..300 {
        src.push_str("if true {\n");
    }
    src.push_str("print(\"deep\")\n");
    for _ in 0..300 {
        src.push_str("}\n");
    }
    src.push('}');
    survives(&src);

    // 200 nested parens in one expression.
    let mut e = String::from("fn f() -> i64 {\n    ");
    for _ in 0..200 {
        e.push('(');
    }
    e.push('1');
    for _ in 0..200 {
        e.push(')');
    }
    e.push_str("\n}\n");
    survives(&e);
}

#[test]
fn large_generated_program_checks_fast() {
    // 2000 functions calling each other in a chain.
    let mut src = String::new();
    for i in 0..2000 {
        let call = if i == 0 {
            "0".to_string()
        } else {
            format!("f{}(x) + 1", i - 1)
        };
        src.push_str(&format!("fn f{i}(x: i64) -> i64 {{\n    {call}\n}}\n\n"));
    }
    src.push_str("fn main() {\n    print(\"{f1999(0)}\")\n}\n");
    let start = std::time::Instant::now();
    let (_, diags) = analyze(&src);
    let elapsed = start.elapsed();
    assert!(diags.is_empty(), "{diags:#?}");
    // ~90k lines/s floor: 8k-line file must check well under a second.
    assert!(
        elapsed.as_millis() < 5000,
        "checking took {elapsed:?} — perf regression"
    );
}
