//! Typechecker integration tests.

use grasm_sema::analyze;

fn check_ok(src: &str) {
    let (_, diags) = analyze(src);
    assert!(
        diags.is_empty(),
        "expected no diagnostics for:\n{src}\ngot:\n{diags:#?}"
    );
}

/// Asserts at least one diagnostic contains `needle`.
fn check_err(src: &str, needle: &str) {
    let (_, diags) = analyze(src);
    assert!(
        diags.iter().any(|d| d.message.contains(needle)),
        "expected a diagnostic containing `{needle}` for:\n{src}\ngot:\n{diags:#?}"
    );
}

#[test]
fn hello_world() {
    check_ok("fn main() {\n    print(\"hello\")\n}\n");
}

#[test]
fn arithmetic_and_inference() {
    check_ok("fn f() {\n    let x = 1 + 2 * 3\n    let y = 2.5 * 4.0\n    let z: f64 = 2\n    let s = \"a\" + \"b\"\n}\n");
    check_err("fn f() {\n    let x = 1 + \"a\"\n}\n", "type mismatch");
    check_err("fn f() {\n    let x = true + false\n}\n", "cannot apply `+`");
}

#[test]
fn int_literal_adapts_to_float() {
    check_ok("fn f() {\n    let x: f64 = 3\n    let y = 2.0 * x\n}\n");
}

#[test]
fn unknown_names() {
    check_err("fn f() {\n    let x = nope\n}\n", "unknown name `nope`");
    check_err("fn f() {\n    let x: Nope = 1\n}\n", "unknown type `Nope`");
}

#[test]
fn immutability_enforced() {
    check_err(
        "fn f() {\n    let x = 1\n    x = 2\n}\n",
        "it is immutable",
    );
    check_ok("fn f() {\n    var x = 1\n    x = 2\n    x += 3\n}\n");
}

#[test]
fn mutating_method_needs_var() {
    check_err(
        "fn f() {\n    let xs = [1]\n    xs.push(2)\n}\n",
        "cannot mutate `xs`",
    );
    check_ok("fn f() {\n    var xs = [1]\n    xs.push(2)\n}\n");
}

#[test]
fn inout_param_is_mutable() {
    check_ok("fn add(inout xs: [i64], v: i64) {\n    xs.push(v)\n}\n");
    check_err(
        "fn add(xs: [i64], v: i64) {\n    xs.push(v)\n}\n",
        "cannot mutate `xs`",
    );
}

#[test]
fn function_calls_and_returns() {
    check_ok("fn add(a: i64, b: i64) -> i64 {\n    a + b\n}\n\nfn main() {\n    let x = add(1, 2)\n}\n");
    check_err(
        "fn add(a: i64, b: i64) -> i64 {\n    a + b\n}\n\nfn main() {\n    let x = add(1)\n}\n",
        "expects 2 argument(s)",
    );
    check_err(
        "fn add(a: i64, b: i64) -> i64 {\n    a + b\n}\n\nfn main() {\n    let x = add(1, \"two\")\n}\n",
        "type mismatch",
    );
    check_err(
        "fn bad() -> i64 {\n    \"nope\"\n}\n",
        "must return `i64`",
    );
}

#[test]
fn explicit_return() {
    check_ok("fn f(x: i64) -> i64 {\n    if x > 0 {\n        return x\n    }\n    return 0 - x\n}\n");
    check_err("fn f() -> i64 {\n    return \"s\"\n}\n", "type mismatch");
}

#[test]
fn structs() {
    let prefix = "type User = { id: i64, name: str }\n";
    check_ok(&format!(
        "{prefix}fn f() {{\n    let u = User{{ id: 1, name: \"ada\" }}\n    let n = u.name\n    print(\"{{n}}\")\n}}\n"
    ));
    check_err(
        &format!("{prefix}fn f() {{\n    let u = User{{ id: 1 }}\n}}\n"),
        "missing field(s) `name`",
    );
    check_err(
        &format!("{prefix}fn f() {{\n    let u = User{{ id: 1, name: \"x\", extra: 2 }}\n}}\n"),
        "no field `extra`",
    );
    check_err(
        &format!("{prefix}fn f(u: User) {{\n    let x = u.email\n}}\n"),
        "no field `email`",
    );
}

#[test]
fn sum_types_and_match() {
    let prefix = "type Shape =\n    | Circle(r: f64)\n    | Rect(w: f64, h: f64)\n    | Dot\n";
    check_ok(&format!(
        "{prefix}fn area(s: Shape) -> f64 {{\n    match s {{\n        Circle(r) -> 3.14 * r * r\n        Rect(w, h) -> w * h\n        Dot -> 0.0\n    }}\n}}\n"
    ));
    check_err(
        &format!(
            "{prefix}fn area(s: Shape) -> f64 {{\n    match s {{\n        Circle(r) -> 3.14\n        Rect(w, h) -> w * h\n    }}\n}}\n"
        ),
        "missing `Dot`",
    );
    check_ok(&format!(
        "{prefix}fn f() {{\n    let s = Circle(1.0)\n    let d = Shape.Dot\n}}\n"
    ));
    check_err(
        &format!("{prefix}fn f() {{\n    let s = Circle(1.0, 2.0)\n}}\n"),
        "takes 1 argument(s)",
    );
    check_err(
        &format!("{prefix}fn f(s: Shape) {{\n    match s {{\n        Square(x) -> 1\n        _ -> 2\n    }}\n}}\n"),
        "no variant `Square`",
    );
}

#[test]
fn wildcard_covers_match() {
    check_ok("fn f(x: i64) -> str {\n    match x {\n        0 -> \"zero\"\n        _ -> \"other\"\n    }\n}\n");
    check_err(
        "fn f(x: i64) -> str {\n    match x {\n        0 -> \"zero\"\n    }\n}\n",
        "not exhaustive",
    );
}

#[test]
fn bool_match_exhaustive() {
    check_ok("fn f(b: bool) -> i64 {\n    match b {\n        true -> 1\n        false -> 0\n    }\n}\n");
    check_err(
        "fn f(b: bool) -> i64 {\n    match b {\n        true -> 1\n    }\n}\n",
        "missing `false`",
    );
}

#[test]
fn guards_do_not_count_for_exhaustiveness() {
    check_err(
        "fn f(b: bool) -> i64 {\n    match b {\n        true -> 1\n        false if b -> 0\n    }\n}\n",
        "not exhaustive",
    );
}

#[test]
fn option_prelude() {
    check_ok("fn f() {\n    let x = Some(3)\n    let y = x ?? 0\n    let n: Option[str] = None\n}\n");
    check_ok("fn f(o: Option[i64]) -> i64 {\n    match o {\n        Some(v) -> v\n        None -> 0\n    }\n}\n");
    check_err(
        "fn f(o: Option[i64]) -> i64 {\n    match o {\n        Some(v) -> v\n    }\n}\n",
        "missing `None`",
    );
    check_err("fn f() {\n    let y = 3 ?? 0\n}\n", "needs an `Option`");
}

#[test]
fn fallible_functions_and_try() {
    let prefix = "type LoadError = | Missing | Denied\n\nfn load(p: str) -> str ! LoadError {\n    if p == \"\" {\n        return \"x\"\n    }\n    \"data\"\n}\n";
    // try propagates when declared.
    check_ok(&format!(
        "{prefix}fn use_it() -> str ! LoadError {{\n    let d = try load(\"a\")\n    d\n}}\n"
    ));
    // try without declaring the error.
    check_err(
        &format!("{prefix}fn use_it() -> str {{\n    let d = try load(\"a\")\n    d\n}}\n"),
        "declares no error type",
    );
    // unhandled fallible value.
    check_err(
        &format!("{prefix}fn use_it() {{\n    let d = load(\"a\")\n}}\n"),
        "can fail",
    );
    // match handles both.
    check_ok(&format!(
        "{prefix}fn use_it() -> str {{\n    match load(\"a\") {{\n        Ok(v) -> v\n        Err(e) -> \"fallback\"\n    }}\n}}\n"
    ));
    // match must cover Err.
    check_err(
        &format!(
            "{prefix}fn use_it() -> str {{\n    match load(\"a\") {{\n        Ok(v) -> v\n    }}\n}}\n"
        ),
        "missing `Err`",
    );
}

#[test]
fn error_union_subset() {
    let prefix = "type AError = | A\ntype BError = | B\n\nfn fa() -> i64 ! AError {\n    1\n}\nfn fboth() -> i64 ! (AError | BError) {\n    1\n}\n";
    check_ok(&format!(
        "{prefix}fn wide() -> i64 ! (AError | BError) {{\n    let x = try fa()\n    let y = try fboth()\n    x + y\n}}\n"
    ));
    check_err(
        &format!("{prefix}fn narrow() -> i64 ! AError {{\n    let y = try fboth()\n    y\n}}\n"),
        "only declares",
    );
}

#[test]
fn generics_basic() {
    check_ok("fn first[T](xs: [T]) -> Option[T] {\n    xs.get(0)\n}\n\nfn main() {\n    let x = first([1, 2, 3]) ?? 0\n    let s = first([\"a\"]) ?? \"none\"\n}\n");
    check_err(
        "fn first[T](xs: [T]) -> Option[T] {\n    xs.get(0)\n}\n\nfn main() {\n    let x: i64 = first([\"a\"]) ?? 0\n}\n",
        "type mismatch",
    );
}

#[test]
fn list_methods() {
    check_ok("fn f() {\n    let xs = [1, 2, 3]\n    let n = xs.len()\n    let d = xs.map(|x| x * 2)\n    let e = xs.filter(|x| x > 1)\n    let s = xs.sum()\n    let names = [\"a\", \"b\"].join(\", \")\n}\n");
    check_err(
        "fn f() {\n    let s = [true].sum()\n}\n",
        "cannot sum",
    );
    check_err(
        "fn f() {\n    let xs = [1]\n    let y = xs.frobnicate()\n}\n",
        "no method `frobnicate`",
    );
}

#[test]
fn str_methods_and_interp() {
    check_ok("fn f() {\n    let s = \"Hello World\"\n    let parts = s.split(\" \")\n    let up = s.to_upper()\n    let n = s.len()\n    print(\"{n} chars: {up}\")\n    let num = \"42\".parse_int() ?? 0\n}\n");
}

#[test]
fn ufcs() {
    check_ok("fn double(x: i64) -> i64 {\n    x * 2\n}\n\nfn main() {\n    let y = 21.double()\n    assert_eq(y, 42)\n}\n");
}

#[test]
fn closures_and_pipelines() {
    check_ok("fn main() {\n    let add = |a, b| a + b\n    let r = add(1, 2)\n    let total = [1, 2, 3] |> sumup\n}\n\nfn sumup(xs: [i64]) -> i64 {\n    xs.sum()\n}\n");
}

#[test]
fn loops_and_ranges() {
    check_ok("fn main() {\n    var total = 0\n    for i in 0..10 {\n        total += i\n    }\n    for x in [1.5, 2.5] {\n        print(\"{x}\")\n    }\n    while total > 0 {\n        total -= 1\n    }\n}\n");
    check_err("fn main() {\n    for i in 3.5 {\n    }\n}\n", "cannot iterate");
    check_err("fn main() {\n    break\n}\n", "outside of a loop");
}

#[test]
fn if_as_expression() {
    check_ok("fn f(x: i64) -> str {\n    if x > 0 { \"pos\" } else { \"neg\" }\n}\n");
    check_ok("fn f(x: i64) -> i64 {\n    let y = if x > 0 { 1 } else { 0 - 1 }\n    y\n}\n");
    check_err("fn f(x: i64) {\n    if x { print(\"hm\") }\n}\n", "type mismatch");
}

#[test]
fn tests_can_use_try() {
    check_ok("type E = | Bad\n\nfn risky() -> i64 ! E {\n    1\n}\n\ntest \"works\" {\n    let v = try risky()\n    assert_eq(v, 1)\n}\n");
}

#[test]
fn duplicate_names_diagnosed() {
    check_err("fn f() {\n}\n\nfn f() {\n}\n", "duplicate function name");
    check_err("type A = { x: i64 }\ntype A = { y: i64 }\n", "duplicate type name");
}

#[test]
fn realistic_program_checks() {
    let src = r#"
type Grade = | Pass(score: i64) | Fail(reason: str)

fn grade(score: i64) -> Grade {
    if score >= 50 {
        Pass(score)
    } else {
        Fail("below passing threshold")
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
    let scores = [95, 72, 33]
    for s in scores {
        print(describe(grade(s)))
    }
    let avg = scores.sum() / scores.len()
    print("average: {avg}")
}

test "grading" {
    assert_eq(describe(grade(95)), "excellent")
    assert_eq(describe(grade(10)), "failed: below passing threshold")
}
"#;
    check_ok(src);
}

#[test]
fn fail_builtin() {
    check_ok("type E = | Boom\n\nfn f(x: i64) -> i64 ! E {\n    if x < 0 {\n        fail(Boom)\n    }\n    x\n}\n");
    check_err(
        "type E = | Boom\n\nfn f(x: i64) -> i64 {\n    fail(Boom)\n}\n",
        "declares no error type",
    );
    check_err(
        "type E = | Boom\ntype F = | Bang\n\nfn f(x: i64) -> i64 ! E {\n    fail(Bang)\n}\n",
        "declares",
    );
}

// ---------------------------------------------------------------------------
// Phase 5: effects
// ---------------------------------------------------------------------------

const CLOCK: &str = "effect Clock {\n    fn now() -> i64\n}\n\nfn is_late(t: i64) -> bool {\n    Clock.now() > t\n}\n";

#[test]
fn effect_handled_in_main_ok() {
    check_ok(&format!(
        "{CLOCK}\nfn main() {{\n    with Clock handled_by ClockHandler{{ now: || 100 }} {{\n        print(\"{{is_late(50)}}\")\n    }}\n}}\n"
    ));
}

#[test]
fn unhandled_effect_in_main_rejected() {
    check_err(
        &format!("{CLOCK}\nfn main() {{\n    print(\"{{is_late(50)}}\")\n}}\n"),
        "unhandled effect",
    );
}

#[test]
fn declared_row_is_upper_bound() {
    // Declares only io but performs Clock → error.
    check_err(
        &format!(
            "{CLOCK}\nfn f() / {{io}} {{\n    let x = is_late(1)\n    print(\"{{x}}\")\n}}\n\nfn main() {{\n    with Clock handled_by ClockHandler{{ now: || 1 }} {{\n        f()\n    }}\n}}\n"
        ),
        "not in its declared row",
    );
    // Declaring both is fine.
    check_ok(&format!(
        "{CLOCK}\nfn f() / {{io, Clock}} {{\n    let x = is_late(1)\n    print(\"{{x}}\")\n}}\n\nfn main() {{\n    with Clock handled_by ClockHandler{{ now: || 1 }} {{\n        f()\n    }}\n}}\n"
    ));
}

#[test]
fn unknown_effect_in_with() {
    check_err(
        "fn main() {\n    with Nope handled_by x {\n    }\n}\n",
        "unknown effect",
    );
}

#[test]
fn unknown_effect_in_declared_row() {
    check_err(
        "fn f() / {Bogus} {\n}\n",
        "unknown effect `Bogus`",
    );
}

#[test]
fn handler_type_checked() {
    // Handler op closure with wrong return type.
    check_err(
        &format!(
            "{CLOCK}\nfn main() {{\n    with Clock handled_by ClockHandler{{ now: || \"soon\" }} {{\n        print(\"{{is_late(1)}}\")\n    }}\n}}\n"
        ),
        "type mismatch",
    );
}

#[test]
fn effect_op_arity_and_types() {
    let logger = "effect Logger {\n    fn log(msg: str)\n}\n";
    check_err(
        &format!("{logger}\nfn f() {{\n    Logger.log()\n}}\n"),
        "takes 1 argument(s)",
    );
    check_err(
        &format!("{logger}\nfn f() {{\n    Logger.log(42)\n}}\n"),
        "type mismatch",
    );
    check_err(
        &format!("{logger}\nfn f() {{\n    Logger.warn(\"x\")\n}}\n"),
        "no operation `warn`",
    );
}

// ---------------------------------------------------------------------------
// Phase 6: ownership Tier-0
// ---------------------------------------------------------------------------

const SINK: &str = "fn consume(own s: str) -> i64 {\n    s.len()\n}\n";

#[test]
fn use_after_move_rejected() {
    check_err(
        &format!("{SINK}\nfn main() {{\n    let name = \"ada\"\n    let n = consume(name)\n    print(name)\n}}\n"),
        "use of moved value `name`",
    );
}

#[test]
fn double_move_rejected() {
    check_err(
        &format!("{SINK}\nfn main() {{\n    let name = \"ada\"\n    let a = consume(name)\n    let b = consume(name)\n}}\n"),
        "use of moved value `name`",
    );
}

#[test]
fn copy_types_do_not_move() {
    check_ok(
        "fn consume_int(own x: i64) -> i64 {\n    x + 1\n}\n\nfn main() {\n    let n = 5\n    let a = consume_int(n)\n    let b = consume_int(n)\n    print(\"{a} {b}\")\n}\n",
    );
}

#[test]
fn move_in_loop_rejected() {
    check_err(
        &format!("{SINK}\nfn main() {{\n    let name = \"ada\"\n    for i in 0..3 {{\n        let n = consume(name)\n    }}\n}}\n"),
        "next loop iteration",
    );
}

#[test]
fn move_of_loop_local_ok() {
    check_ok(&format!(
        "{SINK}\nfn main() {{\n    for i in 0..3 {{\n        let name = \"a\".repeat(i)\n        let n = consume(name)\n    }}\n}}\n"
    ));
}

#[test]
fn conditional_move_is_conservative() {
    check_err(
        &format!("{SINK}\nfn main() {{\n    let name = \"ada\"\n    if name.len() > 2 {{\n        let n = consume(name)\n    }}\n    print(name)\n}}\n"),
        "use of moved value `name`",
    );
}

#[test]
fn reassignment_revives_moved_var() {
    check_ok(&format!(
        "{SINK}\nfn main() {{\n    var name = \"ada\"\n    let n = consume(name)\n    name = \"lovelace\"\n    print(name)\n}}\n"
    ));
}

#[test]
fn inout_arg_must_be_mutable_place() {
    let bump = "fn bump(inout x: i64) {\n    x += 1\n}\n";
    check_ok(&format!("{bump}\nfn main() {{\n    var n = 0\n    bump(n)\n    print(\"{{n}}\")\n}}\n"));
    check_err(
        &format!("{bump}\nfn main() {{\n    let n = 0\n    bump(n)\n}}\n"),
        "it is immutable",
    );
    check_err(
        &format!("{bump}\nfn main() {{\n    bump(41)\n}}\n"),
        "must be a variable",
    );
    check_err(
        &format!("{bump}\nfn main() {{\n    var n = 1\n    bump(n + 1)\n}}\n"),
        "must be a variable",
    );
}

#[test]
fn inout_exclusivity() {
    let swapish = "fn combine(inout a: [i64], inout b: [i64]) {\n    a.push(1)\n    b.push(2)\n}\n";
    check_ok(&format!(
        "{swapish}\nfn main() {{\n    var xs = [1]\n    var ys = [2]\n    combine(xs, ys)\n}}\n"
    ));
    check_err(
        &format!("{swapish}\nfn main() {{\n    var xs = [1]\n    combine(xs, xs)\n}}\n"),
        "passed as `inout` twice",
    );
}

#[test]
fn own_plus_inout_same_var_rejected() {
    check_err(
        "fn weird(own a: [i64], inout b: [i64]) {\n    b.push(1)\n}\n\nfn main() {\n    var xs = [1]\n    weird(xs, xs)\n}\n",
        "moved (`own`) and mutated (`inout`)",
    );
}

#[test]
fn tuple_patterns_with_literals_are_refutable() {
    // Regression: `("GET", x)` must not count as a catch-all.
    check_err(
        "fn f(m: str, p: str) -> i64 {\n    match (m, p) {\n        (\"GET\", x) -> 1\n    }\n}\n",
        "not exhaustive",
    );
    // All-binding tuple pattern still irrefutable.
    check_ok(
        "fn f(m: str, p: str) -> str {\n    match (m, p) {\n        (a, b) -> a + b\n    }\n}\n",
    );
    // Literal combos + catch-all is fine.
    check_ok(
        "fn f(m: str, p: str) -> i64 {\n    match (m, p) {\n        (\"GET\", \"x\") -> 1\n        _ -> 0\n    }\n}\n",
    );
}

// ---------------------------------------------------------------------------
// Phase 2a: maps (static)
// ---------------------------------------------------------------------------

#[test]
fn map_type_checking() {
    check_ok("fn f() {\n    var m: Map[str, i64] = map()\n    m.set(\"a\", 1)\n    let v = m.get(\"a\") ?? 0\n    let n: i64 = m.len()\n}\n");
    check_err(
        "fn f() {\n    var m: Map[str, i64] = map()\n    m.set(1, 1)\n}\n",
        "type mismatch",
    );
    check_err(
        "fn f() {\n    var m: Map[str, i64] = map()\n    let v: str = m.get(\"a\") ?? 0\n}\n",
        "type mismatch",
    );
    check_err(
        "fn f() {\n    let m = map()\n    m.set(\"a\", 1)\n}\n",
        "cannot mutate",
    );
}

#[test]
fn map_key_restriction() {
    check_err(
        "fn f() {\n    var m: Map[[i64], str] = map()\n}\n",
        "map keys must be",
    );
    check_err(
        "fn f() {\n    var m = map()\n    m.set([1], \"x\")\n}\n",
        "map keys must be",
    );
    check_ok("fn f() {\n    var a: Map[i64, str] = map()\n    var b: Map[bool, i64] = map()\n}\n");
}
