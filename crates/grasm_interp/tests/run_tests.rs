//! End-to-end interpreter tests: source in, stdout + exit code out.

fn run(src: &str) -> (i32, String) {
    let mut out = Vec::new();
    let (code, diags) = grasm_interp::run_src(src, &mut out);
    assert!(
        diags.is_empty(),
        "unexpected diagnostics:\n{diags:#?}\nfor:\n{src}"
    );
    (code, String::from_utf8(out).unwrap())
}

fn run_expect(src: &str, expected_out: &str) {
    let (code, out) = run(src);
    assert_eq!(code, 0, "non-zero exit; output:\n{out}");
    assert_eq!(out, expected_out, "wrong output for:\n{src}");
}

#[test]
fn hello_world() {
    run_expect("fn main() {\n    print(\"hello, world\")\n}\n", "hello, world\n");
}

#[test]
fn arithmetic_and_interpolation() {
    run_expect(
        "fn main() {\n    let x = 2 + 3 * 4\n    let y = 2.0 ^ 10.0\n    print(\"x={x} y={y}\")\n}\n",
        "x=14 y=1024.0\n",
    );
}

#[test]
fn string_ops() {
    run_expect(
        "fn main() {\n    let s = \"Hello\" + \", \" + \"World\"\n    print(s.to_upper())\n    print(\"{s.len()}\")\n    print(s.split(\", \").join(\"|\"))\n}\n",
        "HELLO, WORLD\n12\nHello|World\n",
    );
}

#[test]
fn control_flow() {
    run_expect(
        "fn main() {\n    var total = 0\n    for i in 1..=10 {\n        if i % 2 == 0 {\n            total += i\n        }\n    }\n    print(\"{total}\")\n}\n",
        "30\n",
    );
}

#[test]
fn while_break_continue() {
    run_expect(
        "fn main() {\n    var n = 0\n    var i = 0\n    while true {\n        i += 1\n        if i > 100 {\n            break\n        }\n        if i % 3 != 0 {\n            continue\n        }\n        n += i\n    }\n    print(\"{n}\")\n}\n",
        "1683\n",
    );
}

#[test]
fn functions_and_recursion() {
    run_expect(
        "fn fib(n: i64) -> i64 {\n    if n <= 1 {\n        return n\n    }\n    fib(n - 1) + fib(n - 2)\n}\n\nfn main() {\n    print(\"{fib(20)}\")\n}\n",
        "6765\n",
    );
}

#[test]
fn value_semantics_no_aliasing() {
    // Assignment copies: mutating `b` must not affect `a`.
    run_expect(
        "fn main() {\n    var a = [1, 2, 3]\n    var b = a\n    b.push(4)\n    print(\"{a.len()} {b.len()}\")\n}\n",
        "3 4\n",
    );
}

#[test]
fn structs_and_field_assignment() {
    run_expect(
        "type Point = { x: i64, y: i64 }\n\nfn main() {\n    var p = Point{ x: 1, y: 2 }\n    p.x = 10\n    print(\"{p.x},{p.y}\")\n    print(\"{p}\")\n}\n",
        "10,2\nPoint{x: 10, y: 2}\n",
    );
}

#[test]
fn nested_place_assignment() {
    run_expect(
        "type Board = { cells: [i64] }\n\nfn main() {\n    var b = Board{ cells: [0, 0, 0] }\n    b.cells[1] = 5\n    print(\"{b.cells}\")\n}\n",
        "[0, 5, 0]\n",
    );
}

#[test]
fn sum_types_and_match() {
    run_expect(
        r#"type Shape =
    | Circle(r: f64)
    | Rect(w: f64, h: f64)

fn area(s: Shape) -> f64 {
    match s {
        Circle(r) -> 3.0 * r * r
        Rect(w, h) -> w * h
    }
}

fn main() {
    let shapes = [Circle(2.0), Rect(3.0, 4.0)]
    var total = 0.0
    for s in shapes {
        total += area(s)
    }
    print("{total}")
}
"#,
        "24.0\n",
    );
}

#[test]
fn match_guards_and_literals() {
    run_expect(
        "fn label(n: i64) -> str {\n    match n {\n        0 -> \"zero\"\n        x if x < 0 -> \"negative\"\n        _ -> \"positive\"\n    }\n}\n\nfn main() {\n    print(label(0) + \" \" + label(0 - 5) + \" \" + label(9))\n}\n",
        "zero negative positive\n",
    );
}

#[test]
fn option_and_coalesce() {
    run_expect(
        "fn main() {\n    let xs = [10, 20]\n    let a = xs.get(1) ?? 0\n    let b = xs.get(9) ?? 0\n    let n = \"42\".parse_int() ?? 0\n    let bad = \"nope\".parse_int() ?? 7\n    print(\"{a} {b} {n} {bad}\")\n}\n",
        "20 0 42 7\n",
    );
}

#[test]
fn fallible_try_and_match() {
    run_expect(
        r#"type MathError = | DivByZero

fn safe_div(a: i64, b: i64) -> i64 ! MathError {
    if b == 0 {
        fail(DivByZero)
    }
    a / b
}

fn main() {
    match safe_div(10, 2) {
        Ok(v) -> print("ok {v}")
        Err(e) -> print("err {e}")
    }
    match safe_div(1, 0) {
        Ok(v) -> print("ok {v}")
        Err(e) -> print("err {e}")
    }
}
"#,
        "ok 5\nerr DivByZero\n",
    );
}

#[test]
fn fallible_error_propagates_through_try() {
    run_expect(
        r#"type ParseError = | Empty | BadChar(c: str)

fn parse_num(s: str) -> i64 ! ParseError {
    if s == "" {
        fail(Empty)
    }
    match s.parse_int() {
        Some(v) -> v
        None -> fail(BadChar(s))
    }
}

fn sum_csv(line: str) -> i64 ! ParseError {
    var total = 0
    for part in line.split(",") {
        total += try parse_num(part.trim())
    }
    total
}

fn main() {
    match sum_csv("1, 2, 3") {
        Ok(v) -> print("sum {v}")
        Err(e) -> print("err {e}")
    }
    match sum_csv("1, x, 3") {
        Ok(v) -> print("sum {v}")
        Err(e) -> print("err {e}")
    }
    match sum_csv("") {
        Ok(v) -> print("sum {v}")
        Err(e) -> print("err {e}")
    }
}
"#,
        "sum 6\nerr BadChar(x)\nerr Empty\n",
    );
}

#[test]
fn closures_and_higher_order() {
    run_expect(
        "fn main() {\n    let add = |a, b| a + b\n    let inc = |x| add(x, 1)\n    let xs = [1, 2, 3].map(|x| x * x).filter(|x| x > 2)\n    print(\"{inc(41)} {xs} {xs.sum()}\")\n}\n",
        "42 [4, 9] 13\n",
    );
}

#[test]
fn pipelines() {
    run_expect(
        "fn double_all(xs: [i64]) -> [i64] {\n    xs.map(|x| x * 2)\n}\n\nfn total(xs: [i64]) -> i64 {\n    xs.sum()\n}\n\nfn main() {\n    let r = [1, 2, 3] |> double_all |> total\n    print(\"{r}\")\n}\n",
        "12\n",
    );
}

#[test]
fn ufcs_runtime() {
    run_expect(
        "fn double(x: i64) -> i64 {\n    x * 2\n}\n\nfn main() {\n    print(\"{21.double()}\")\n}\n",
        "42\n",
    );
}

#[test]
fn panic_exit_code() {
    let (code, out) = run("fn main() {\n    let xs = [1]\n    let x = xs[5]\n}\n");
    assert_eq!(code, 101);
    assert!(out.contains("out of bounds"), "output: {out}");
}

#[test]
fn division_by_zero_panics() {
    let (code, out) = run("fn main() {\n    let a = 1\n    let b = 0\n    let c = a / b\n}\n");
    assert_eq!(code, 101);
    assert!(out.contains("division by zero"), "output: {out}");
}

#[test]
fn assert_eq_failure() {
    let (code, out) = run("fn main() {\n    assert_eq(1 + 1, 3)\n}\n");
    assert_eq!(code, 101);
    assert!(out.contains("assertion failed"), "output: {out}");
}

#[test]
fn test_runner() {
    let src = "fn add(a: i64, b: i64) -> i64 {\n    a + b\n}\n\ntest \"passes\" {\n    assert_eq(add(2, 2), 4)\n}\n\ntest \"fails\" {\n    assert_eq(add(2, 2), 5)\n}\n";
    let mut out = Vec::new();
    let (p, f, diags) = grasm_interp::test_src(src, &mut out);
    assert!(diags.is_empty());
    assert_eq!((p, f), (1, 1));
    let text = String::from_utf8(out).unwrap();
    assert!(text.contains("\"passes\" ... ok"));
    assert!(text.contains("\"fails\" ... FAILED"));
}

#[test]
fn qualified_variants() {
    run_expect(
        "type Color = | Red | Green | Blue\n\nfn main() {\n    let c = Color.Green\n    let name = match c {\n        Red -> \"red\"\n        Green -> \"green\"\n        Blue -> \"blue\"\n    }\n    print(name)\n}\n",
        "green\n",
    );
}

#[test]
fn sort_and_rev() {
    run_expect(
        "fn main() {\n    var xs = [3, 1, 2]\n    xs.sort()\n    print(\"{xs} {xs.rev()}\")\n}\n",
        "[1, 2, 3] [3, 2, 1]\n",
    );
}

#[test]
fn mixed_numeric_promotion() {
    run_expect(
        "fn main() {\n    let x: f64 = 3\n    let y = 2.0 * x\n    print(\"{y}\")\n}\n",
        "6.0\n",
    );
}

#[test]
fn tuples_and_match() {
    run_expect(
        "fn main() {\n    let pair = (1, \"one\")\n    match pair {\n        (n, name) -> print(\"{n} is {name}\")\n    }\n}\n",
        "1 is one\n",
    );
}

#[test]
fn full_program_grades() {
    let src = r#"
type Grade = | Pass(score: i64) | Fail(reason: str)

fn grade(score: i64) -> Grade {
    if score >= 50 {
        Pass(score)
    } else {
        Fail("too low")
    }
}

fn describe(g: Grade) -> str {
    match g {
        Pass(s) if s >= 90 -> "excellent"
        Pass(s) -> "passed with {s}"
        Fail(r) -> "failed: {r}"
    }
}

fn main() {
    for s in [95, 72, 33] {
        print(describe(grade(s)))
    }
}
"#;
    run_expect(src, "excellent\npassed with 72\nfailed: too low\n");
}
