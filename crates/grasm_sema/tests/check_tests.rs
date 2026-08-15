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

// ---------------------------------------------------------------------------
// C3: structured concurrency
// ---------------------------------------------------------------------------

#[test]
fn spawn_requires_a_parallel_scope() {
    check_err(
        "fn main() {\n    spawn {\n        print(\"nope\")\n    }\n}\n",
        "`spawn` outside a `parallel` block",
    );
    check_ok(
        "fn main() {\n    parallel {\n        spawn {\n            print(\"ok\")\n        }\n    }\n}\n",
    );
}

#[test]
fn task_bodies_are_typechecked() {
    check_err(
        "fn main() {\n    parallel {\n        spawn {\n            let x: i64 = \"wrong\"\n        }\n    }\n}\n",
        "type mismatch",
    );
}

#[test]
fn break_cannot_cross_a_task_boundary() {
    check_err(
        "fn main() {\n    for i in 0..3 {\n        parallel {\n            spawn {\n                break\n            }\n        }\n    }\n}\n",
        "outside of a loop",
    );
}

#[test]
fn concurrency_is_the_async_effect() {
    // A function that spawns performs `async`; declaring only `io` is an error.
    check_err(
        "fn worker() / {io} {\n    parallel {\n        spawn {\n            print(\"x\")\n        }\n    }\n}\n\nfn main() {\n    worker()\n}\n",
        "not in its declared row",
    );
    check_ok(
        "fn worker() / {io, async} {\n    parallel {\n        spawn {\n            print(\"x\")\n        }\n    }\n}\n\nfn main() {\n    worker()\n}\n",
    );
}

#[test]
fn nested_parallel_scopes_are_allowed() {
    check_ok(
        "fn main() {\n    parallel {\n        spawn {\n            parallel {\n                spawn {\n                    print(\"deep\")\n                }\n            }\n        }\n    }\n}\n",
    );
}

// ---------------------------------------------------------------------------
// Standard library typing
// ---------------------------------------------------------------------------

#[test]
fn search_methods_require_a_bool_predicate() {
    check_ok("fn main() {\n    print(\"{[1].any(|x| x > 0)}\")\n}\n");
    check_err(
        "fn main() {\n    print(\"{[1].all(|x| x + 1)}\")\n}\n",
        "bool",
    );
}

#[test]
fn find_min_and_max_yield_options() {
    check_ok("fn main() {\n    let x: i64 = [1, 2].find(|v| v > 1) ?? 0\n    let y: i64 = [1, 2].min() ?? 0\n    let z: i64 = [1, 2].max() ?? 0\n    print(\"{x}{y}{z}\")\n}\n");
    check_err(
        "fn main() {\n    let x: i64 = [1, 2].min()\n    print(\"{x}\")\n}\n",
        "type mismatch",
    );
}

#[test]
fn env_returns_an_optional_string() {
    check_ok("fn main() {\n    let home: str = env(\"HOME\") ?? \"\"\n    print(home)\n}\n");
    check_err(
        "fn main() {\n    let home: str = env(\"HOME\")\n    print(home)\n}\n",
        "type mismatch",
    );
}

#[test]
fn the_clock_and_the_environment_are_io_effects() {
    check_err(
        "fn stamp() -> i64 / {} {\n    now_millis()\n}\n\nfn main() {\n    print(\"{stamp()}\")\n}\n",
        "not in its declared row",
    );
    check_ok(
        "fn stamp() -> i64 / {io} {\n    now_millis()\n}\n\nfn main() {\n    print(\"{stamp()}\")\n}\n",
    );
}

#[test]
fn is_empty_works_on_every_container() {
    check_ok(
        "fn main() {\n    var m = map()\n    m.set(\"k\", 1)\n    print(\"{[1].is_empty()} {\"s\".is_empty()} {m.is_empty()}\")\n}\n",
    );
    check_err(
        "fn main() {\n    print(\"{(1).is_empty()}\")\n}\n",
        "no method",
    );
}

// ---------------------------------------------------------------------------
// Property tests and benchmarks
// ---------------------------------------------------------------------------

#[test]
fn property_parameters_are_bound_in_the_body() {
    check_ok("test \"p\" (n: i64, s: str) {\n    assert(n == n)\n    assert(s.len() >= 0)\n}\n");
    check_err(
        "test \"p\" (n: i64) {\n    assert(m == 0)\n}\n",
        "unknown",
    );
}

#[test]
fn property_parameters_must_be_generatable() {
    check_ok("test \"p\" (a: i64, b: f64, c: bool, d: str, e: [i64], f: Option[str]) {\n    assert(true)\n}\n");
    check_err(
        "type Point = { x: i64, y: i64 }\n\ntest \"p\" (p: Point) {\n    assert(true)\n}\n",
        "cannot generate values of type",
    );
}

#[test]
fn a_property_parameter_cannot_be_inout_or_own() {
    check_err(
        "test \"p\" (inout n: i64) {\n    assert(true)\n}\n",
        "supplied by the runner",
    );
}

#[test]
fn property_parameters_are_immutable() {
    // The runner owns the value; reassigning it would silently discard the
    // input the failure will be reported with.
    check_err(
        "test \"p\" (n: i64) {\n    n = 1\n}\n",
        "immutable",
    );
}

#[test]
fn bench_bodies_are_typechecked() {
    check_ok("bench \"b\" {\n    let x = 1 + 1\n    assert(x == 2)\n}\n");
    check_err(
        "bench \"b\" {\n    let x: i64 = \"text\"\n    assert(x == 2)\n}\n",
        "type mismatch",
    );
}

#[test]
fn bench_bodies_carry_effects_like_tests() {
    check_err(
        "effect Clock {\n    fn now() -> i64\n}\n\nbench \"b\" {\n    let t = Clock.now()\n    assert(t >= 0)\n}\n",
        "unhandled effect",
    );
}

// ---------------------------------------------------------------------------
// Task capture
// ---------------------------------------------------------------------------

#[test]
fn a_task_may_use_bindings_from_outside_the_parallel_block() {
    check_ok(
        "fn main() {\n    var log: [str] = []\n    let n = 7\n    parallel {\n        spawn {\n            log.push(\"{n}\")\n        }\n    }\n    print(log.join(\",\"))\n}\n",
    );
}

#[test]
fn a_task_cannot_use_a_binding_declared_inside_the_block() {
    // Tasks run after the block finishes, so `shared` is gone. The
    // interpreter used to panic at run time with "unknown name".
    check_err(
        "fn main() {\n    var log: [str] = []\n    parallel {\n        let shared = 1\n        spawn {\n            log.push(\"{shared}\")\n        }\n    }\n    print(log.join(\",\"))\n}\n",
        "declared inside the `parallel` block",
    );
}

#[test]
fn a_task_cannot_use_a_loop_variable_from_the_block() {
    check_err(
        "fn main() {\n    var log: [str] = []\n    parallel {\n        for i in 0..3 {\n            spawn {\n                log.push(\"{i}\")\n            }\n        }\n    }\n    print(log.join(\",\"))\n}\n",
        "declared inside the `parallel` block",
    );
}

#[test]
fn a_task_may_use_its_own_locals() {
    check_ok(
        "fn main() {\n    var log: [str] = []\n    parallel {\n        spawn {\n            let mine = 3\n            log.push(\"{mine}\")\n        }\n    }\n    print(log.join(\",\"))\n}\n",
    );
}

#[test]
fn the_capture_rule_does_not_leak_past_the_parallel_block() {
    // A binding declared inside the block is still perfectly usable by the
    // block's own statements — only tasks run too late to see it.
    check_ok(
        "fn main() {\n    parallel {\n        let shared = 1\n        print(\"{shared}\")\n    }\n}\n",
    );
    // And a later, unrelated block is unaffected.
    check_ok(
        "fn main() {\n    parallel {\n        spawn {\n            print(\"a\")\n        }\n    }\n    let after = 1\n    print(\"{after}\")\n}\n",
    );
}

// ---------------------------------------------------------------------------
// Function values
// ---------------------------------------------------------------------------

#[test]
fn a_function_can_be_used_as_a_value() {
    check_ok("fn inc(n: i64) -> i64 {\n    n + 1\n}\n\nfn main() {\n    let f = inc\n    print(\"{f(3)}\")\n}\n");
}

#[test]
fn a_value_that_is_not_a_function_is_not_callable() {
    check_err(
        "fn main() {\n    let x = 5\n    print(\"{x(1)}\")\n}\n",
        "not callable",
    );
}

#[test]
fn a_list_may_hold_functions_of_the_same_shape() {
    check_ok(
        "fn inc(n: i64) -> i64 {\n    n + 1\n}\n\nfn dbl(n: i64) -> i64 {\n    n * 2\n}\n\nfn main() {\n    let fs = [inc, dbl]\n    for g in fs {\n        print(\"{g(1)}\")\n    }\n}\n",
    );
}

// ---------------------------------------------------------------------------
// par_map
// ---------------------------------------------------------------------------

#[test]
fn par_map_types_like_map() {
    check_ok("fn main() {\n    let xs = [1, 2, 3]\n    let ys: [i64] = xs.par_map(|x| x * 2)\n    print(\"{ys}\")\n}\n");
    check_ok("fn main() {\n    let xs = [1, 2]\n    let ys: [bool] = xs.par_map(|x| x > 1)\n    print(\"{ys}\")\n}\n");
}

#[test]
fn par_map_rejects_an_effectful_function() {
    // Elements run concurrently, so an effect would interleave differently
    // between runs. The effect system already knows, so this is a compile
    // error rather than a race.
    check_err(
        "fn main() {\n    let xs = [1, 2]\n    let ys = xs.par_map(|x| {\n        print(\"{x}\")\n        x\n    })\n    print(\"{ys}\")\n}\n",
        "effect-free",
    );
}

#[test]
fn par_map_rejects_an_effect_reached_indirectly() {
    // The impurity is a call away, which is exactly the case a hand-written
    // "does this closure print?" check would miss.
    check_err(
        "fn shout(n: i64) -> i64 {\n    print(\"{n}\")\n    n\n}\n\nfn main() {\n    let xs = [1, 2]\n    print(\"{xs.par_map(|x| shout(x))}\")\n}\n",
        "effect-free",
    );
}

#[test]
fn par_map_rejects_a_user_effect_too() {
    check_err(
        "effect Clock {\n    fn now() -> i64\n}\n\nfn main() {\n    let xs = [1, 2]\n    let ys = xs.par_map(|x| x + Clock.now())\n    with Clock handled_by ClockHandler{ now: || 1 } {\n        print(\"{ys}\")\n    }\n}\n",
        "effect-free",
    );
}

#[test]
fn a_pure_par_map_is_accepted() {
    check_ok(
        "fn double(n: i64) -> i64 {\n    n * 2\n}\n\nfn main() {\n    let xs = [1, 2]\n    print(\"{xs.par_map(|x| double(x))}\")\n}\n",
    );
}

#[test]
fn par_filter_types_like_filter_and_demands_purity() {
    check_ok("fn main() {\n    let xs = [1, 2, 3]\n    let ys: [i64] = xs.par_filter(|x| x > 1)\n    print(\"{ys}\")\n}\n");
    check_err(
        "fn main() {\n    let xs = [1, 2]\n    let ys = xs.par_filter(|x| {\n        print(\"{x}\")\n        x > 1\n    })\n    print(\"{ys}\")\n}\n",
        "effect-free",
    );
}

#[test]
fn par_filter_rejects_an_effect_reached_indirectly() {
    check_err(
        "fn noisy(n: i64) -> bool {\n    print(\"{n}\")\n    n > 1\n}\n\nfn main() {\n    print(\"{[1, 2].par_filter(|x| noisy(x))}\")\n}\n",
        "effect-free",
    );
}

// The purity check used to fire only on a closure written inline at the call
// site. Every other way of naming the same effectful function compiled, and
// ran it on several cores — including with the effect row declared in plain
// sight. A cold-model run found this; these hold the fix.

#[test]
fn par_map_rejects_a_named_effectful_function() {
    check_err(
        "fn announce(x: i64) -> i64 {\n    print(\"{x}\")\n    x * 2\n}\n\nfn main() {\n    print(\"{[1, 2].par_map(announce)}\")\n}\n",
        "effect-free",
    );
}

#[test]
fn par_map_rejects_a_named_function_whose_row_is_declared() {
    // The worst spelling of the three: the row is written on the function,
    // so the compiler was being told exactly what it failed to notice.
    check_err(
        "fn announce(x: i64) -> i64 / {io} {\n    print(\"{x}\")\n    x * 2\n}\n\nfn main() {\n    print(\"{[1, 2].par_map(announce)}\")\n}\n",
        "effect-free",
    );
}

#[test]
fn par_map_rejects_a_closure_given_a_name_first() {
    check_err(
        "fn main() {\n    let f = |x| {\n        print(\"{x}\")\n        x * 2\n    }\n    print(\"{[1, 2].par_map(f)}\")\n}\n",
        "effect-free",
    );
}

#[test]
fn par_filter_rejects_a_named_effectful_function() {
    check_err(
        "fn loud(x: i64) -> bool {\n    print(\"{x}\")\n    x > 1\n}\n\nfn main() {\n    print(\"{[1, 2].par_filter(loud)}\")\n}\n",
        "effect-free",
    );
}

#[test]
fn par_map_rejects_a_named_function_effectful_two_calls_down() {
    check_err(
        "fn inner(x: i64) -> i64 {\n    print(\"{x}\")\n    x\n}\n\nfn outer(x: i64) -> i64 {\n    inner(x) + 1\n}\n\nfn main() {\n    print(\"{[1, 2].par_map(outer)}\")\n}\n",
        "effect-free",
    );
}

#[test]
fn par_map_still_accepts_pure_functions_by_name() {
    // The fix must not make the feature unusable: a named pure function and a
    // named pure closure are the two most natural ways to write this.
    check_ok("fn dbl(x: i64) -> i64 {\n    x * 2\n}\n\nfn main() {\n    print(\"{[1, 2].par_map(dbl)}\")\n}\n");
    check_ok("fn main() {\n    let f = |x| x * 2\n    print(\"{[1, 2].par_map(f)}\")\n}\n");
}

#[test]
fn plain_map_still_allows_effects() {
    // `map` is sequential, so an effect in it is ordinary code, not a race.
    check_ok("fn announce(x: i64) -> i64 {\n    print(\"{x}\")\n    x\n}\n\nfn main() {\n    print(\"{[1, 2].map(announce)}\")\n}\n");
}

// ---------------------------------------------------------------------------
// Adversarial: effects must not escape through a function value.
//
// Effect inference was written before function values existed, so every check
// that asks "what does this expression perform?" answered "nothing" for a bare
// name. A declared row is supposed to be an upper bound ([T-EFFECT-ROW]); these
// are the ways it was not.
// ---------------------------------------------------------------------------

#[test]
fn a_declared_empty_row_cannot_print_through_a_bound_function_name() {
    check_err(
        "fn shout(n: i64) -> i64 {\n    print(\"{n}\")\n    n\n}\n\nfn claims_pure(n: i64) -> i64 / {} {\n    let f = shout\n    f(n)\n}\n\nfn main() {\n    print(\"{claims_pure(1)}\")\n}\n",
        "io",
    );
}

#[test]
fn a_declared_empty_row_cannot_print_through_a_bound_closure() {
    check_err(
        "fn claims_pure(n: i64) -> i64 / {} {\n    let f = |x| {\n        print(\"{x}\")\n        x\n    }\n    f(n)\n}\n\nfn main() {\n    print(\"{claims_pure(1)}\")\n}\n",
        "io",
    );
}

#[test]
fn a_declared_empty_row_cannot_print_through_a_list_of_functions() {
    check_err(
        "fn shout(n: i64) -> i64 {\n    print(\"{n}\")\n    n\n}\n\nfn claims_pure(n: i64) -> i64 / {} {\n    let fs = [shout]\n    var acc = 0\n    for f in fs {\n        acc = f(n)\n    }\n    acc\n}\n\nfn main() {\n    print(\"{claims_pure(1)}\")\n}\n",
        "io",
    );
}

#[test]
fn par_map_rejects_a_function_value_taken_from_a_list() {
    check_err(
        "fn shout(n: i64) -> i64 {\n    print(\"{n}\")\n    n\n}\n\nfn main() {\n    let fs = [shout]\n    var out = 0\n    for f in fs {\n        out = [1, 2].par_map(f).len()\n    }\n    print(\"{out}\")\n}\n",
        "effect-free",
    );
}

#[test]
fn a_user_effect_cannot_escape_through_a_function_value_either() {
    check_err(
        "effect Clock {\n    fn now() -> i64\n}\n\nfn stamped(n: i64) -> i64 {\n    n + Clock.now()\n}\n\nfn claims_pure(n: i64) -> i64 / {} {\n    let f = stamped\n    f(n)\n}\n\nfn main() {\n    with Clock handled_by ClockHandler{ now: || 1 } {\n        print(\"{claims_pure(1)}\")\n    }\n}\n",
        "Clock",
    );
}

#[test]
fn a_function_value_that_is_pure_still_works_through_a_binding() {
    // The fix must not reject the legitimate uses: naming a pure function and
    // calling it through the name is ordinary code.
    check_ok("fn dbl(n: i64) -> i64 {\n    n * 2\n}\n\nfn stays_pure(n: i64) -> i64 / {} {\n    let f = dbl\n    f(n)\n}\n\nfn main() {\n    print(\"{stays_pure(1)}\")\n}\n");
    check_ok("fn stays_pure(n: i64) -> i64 / {} {\n    let f = |x| x * 2\n    f(n)\n}\n\nfn main() {\n    print(\"{stays_pure(1)}\")\n}\n");
}

/// Naming an effectful function counts, even without calling it.
///
/// This test used to assert the opposite, on the reasoning that mentioning a
/// function performs nothing and only calling does. That is true of the
/// mention and false of the program: a cold evaluation turned it into a hole
/// through every capability guarantee at once.
///
/// `[1, 2].map(shout)` was the easy half — a built-in higher-order method's
/// parameter is deliberately unconstrained, so `map` accepts any row and the
/// enclosing function's declared `/ {}` had nothing to check against. Hiding
/// it one hop further, `let fs = [uses]` then `fs.map(call)`, defeated even
/// `par_map`'s own hand-written purity check: the effectful function is only
/// ever *mentioned*, and something else calls it.
///
/// So the rule is now the one already applied to a declared C function —
/// holding one is the permission to call it. It is conservative: it rejects
/// storing an effectful function and never calling it, which this test used to
/// pin as legitimate. That is a real cost, paid to keep `/ {}`,
/// [T-PAR-MAP-PURE], [E-HANDLER-MISSING] and `capabilities = []` from being
/// one indirection away from meaningless. Declaring the row is the way out.
#[test]
fn naming_an_effectful_function_counts_as_performing_it() {
    check_err(
        "fn shout(n: i64) -> i64 {\n    print(\"{n}\")\n    n\n}\n\nfn stays_pure() -> i64 / {} {\n    let fs = [shout]\n    fs.len()\n}\n\nfn main() {\n    print(\"{stays_pure()}\")\n}\n",
        "io",
    );
    // A pure function named as a value is still pure — the row travels, it is
    // not invented.
    check_ok("fn dbl(n: i64) -> i64 {\n    n * 2\n}\n\nfn stays_pure() -> i64 / {} {\n    let fs = [dbl]\n    fs.len()\n}\n\nfn main() {\n    print(\"{stays_pure()}\")\n}\n");
}

/// Passing a captured binding as `inout` is the quiet member of the
/// [T-CAPTURE-WRITE] family: the call runs, and the callee's writes land in a
/// copy the caller cannot read, so it reads as a counter that never moves.
#[test]
fn passing_a_captured_binding_as_inout_is_rejected() {
    check_err(
        "fn track(inout calls: i64, n: i64) -> i64 {\n\
         \x20   calls += 1\n\
         \x20   n\n\
         }\n\
         fn main() {\n\
         \x20   var calls = 0\n\
         \x20   let f = |n| track(calls, n)\n\
         \x20   print(\"{f(7)} {calls}\")\n\
         }\n",
        "cannot pass `calls` as `inout`",
    );
}

/// A closure's *own* `var` is not captured, so threading it through `inout`
/// stays legal — the rule is about the boundary, not about `inout`.
#[test]
fn a_closures_own_var_may_still_be_passed_as_inout() {
    check_ok(
        "fn track(inout c: i64, n: i64) -> i64 {\n\
         \x20   c += 1\n\
         \x20   n\n\
         }\n\
         fn main() {\n\
         \x20   let f = |n| { var c = 0\n\
         \x20       track(c, n) + c }\n\
         \x20   print(\"{f(7)}\")\n\
         }\n",
    );
}

/// A parse error in a map literal's *value* is reported where it is.
///
/// `{` opens either a block or a map, and the map reading rewound on any
/// failure — so `{"k": *}` was reported as "expected end of line after
/// statement" at the colon, a parse error about the wrong thing. Once a
/// `key :` has been read it cannot be a block.
#[test]
fn a_map_literal_reports_its_own_error() {
    let (_, diags) = analyze("fn main() {\n    let m = {\"k\": *}\n    print(\"{m}\")\n}\n");
    let text = diags
        .iter()
        .map(|d| d.message.clone())
        .collect::<Vec<_>>()
        .join("\n");
    assert!(
        text.contains("expected an expression"),
        "the error should be the one inside the literal, got: {text}"
    );
    assert!(
        !text.contains("end of line after statement"),
        "the block reading should not be what is reported: {text}"
    );
}

/// A cascade from an earlier error says nothing twice.
///
/// `p.zz.len()` reported that `P` has no field `zz` and then that `<error>`
/// has no method `len` — with help suggesting `fn len(<error>, ...)`, a
/// signature nobody can type.
#[test]
fn a_broken_receiver_does_not_cascade() {
    let (_, diags) = analyze(
        "type P = { a: i64 }\nfn main() {\n    let p = P{ a: 1 }\n    print(\"{p.zz.len()}\")\n}\n",
    );
    assert_eq!(diags.len(), 1, "{diags:#?}");
    assert!(!format!("{diags:?}").contains("<error>"), "{diags:#?}");
}

/// An integer literal in a `match` arm adopts `f64` whichever arm it is in.
///
/// [T-INT-TO-FLOAT-CARRIED] says a list, a map's values and a `match`'s arms
/// are checked with their integer literals last. Lists and maps were; arms
/// decided left to right, so one order compiled and the other did not.
#[test]
fn match_arms_check_integer_literals_last() {
    check_ok("fn main() {\n    let x = match 1 { 1 -> 3  _ -> 3.5 }\n    print(\"{x}\")\n}\n");
    check_ok("fn main() {\n    let x = match 1 { 1 -> 3.5  _ -> 3 }\n    print(\"{x}\")\n}\n");
}

/// A closure's failure is a failure, whichever order the checker learns it in.
///
/// A closure with parameters waits for a call site to say what they are, so
/// its return type is unknown when the statement is checked. `f(-1)` on its
/// own line printed the next line and exited 0.
#[test]
fn a_discarded_closure_failure_is_refused() {
    check_err(
        "type E = { m: str }\nfn risky(n: i64) -> i64 ! E {\n    if n < 0 {\n        fail(E{ m: \"x\" })\n    }\n    n\n}\nfn main() {\n    let f = |x| risky(x)\n    f(-1)\n    print(\"after\")\n}\n",
        "failure is discarded",
    );
}

/// And the three shapes that need the *shape* of a closure's return type.
#[test]
fn a_closure_call_can_be_coalesced_matched_and_tried() {
    check_ok(
        "type E = { m: str }\nfn risky(n: i64) -> i64 ! E {\n    if n < 0 {\n        fail(E{ m: \"x\" })\n    }\n    n\n}\nfn main() ! E {\n    let f = |x| risky(x)\n    print(\"{f(1) ?? -1}\")\n    let g = |x| risky(x)\n    match g(1) {\n        Ok(v) -> print(\"{v}\")\n        Err(e) -> print(\"{e.m}\")\n    }\n    let h = |x| risky(x)\n    print(\"{try h(1)}\")\n}\n",
    );
}

/// A closure that returns a closure keeps its captured binding in scope.
#[test]
fn a_closure_may_return_a_closure() {
    check_ok(
        "fn main() {\n    let mk = |n| |m| n + m\n    let add3 = mk(3)\n    print(\"{add3(4)}\")\n}\n",
    );
}

/// A type parameter that reaches a map only inside a body is still a key.
#[test]
fn a_body_only_map_key_is_checked_at_the_call_site() {
    let head = "fn tally[T](xs: [T]) -> i64 {\n    var m: Map[T, i64] = {}\n    for x in xs {\n        m.set(x, 1)\n    }\n    m.len()\n}\nfn main() {\n";
    check_err(
        &format!("{head}    print(\"{{tally([1.5, 2.5])}}\")\n}}\n"),
        "map keys must be",
    );
    check_ok(&format!("{head}    print(\"{{tally([1, 2])}}\")\n}}\n"));
}

/// A handler's discharge may not leave its block through a generic call.
#[test]
fn a_generic_call_cannot_carry_a_handler_row_out() {
    check_err(
        "effect Cfg {\n    fn value() -> i64\n}\nfn keep[T](inout xs: [T], v: T) {\n    xs.push(v)\n}\nfn main() {\n    var fs: [fn() -> i64 / {Cfg}] = []\n    with Cfg handled_by CfgHandler{ value: || 1 } {\n        keep(fs, || Cfg.value())\n    }\n}\n",
        "cannot pass a function that performs `Cfg`",
    );
}

/// `main` produces no value: a return type on it read as a status code and
/// was discarded ([E-ENTRY]).
#[test]
fn main_produces_no_value() {
    check_err(
        "fn main() -> i64 {\n    print(\"x\")\n    3\n}\n",
        "`main` produces no value",
    );
    // A `! E` on main is a different thing and is allowed.
    check_ok(
        "type E = { m: str }\nfn boom() -> i64 ! E {\n    fail(E{ m: \"x\" })\n}\nfn main() ! E {\n    print(\"{try boom()}\")\n}\n",
    );
}

/// A negated literal is a literal ([E-NUM-PROMOTE]: "on either side").
#[test]
fn a_negated_literal_promotes_to_float() {
    check_ok("fn main() {\n    print(\"{-3 * 2.5}\")\n    print(\"{2.5 * -3}\")\n    print(\"{-1 < 0.5}\")\n}\n");
}

/// An expected type reaches an `if`'s branches through a fallible return type,
/// and a branch that gives the ok half merges with one that builds the whole
/// `T ! E` ([T-INT-TO-FLOAT-CARRIED], [T-OK-ERR-RETURN]).
#[test]
fn a_fallible_body_may_end_in_an_if() {
    check_ok("type E = | Bad\nfn f(n: i64) -> f64 ! E {\n    if n == 0 {\n        4.5\n    } else {\n        6\n    }\n}\nfn main() {\n    print(\"{f(0) ?? -1.0}\")\n}\n");
    check_ok("type E = | Bad\nfn f(b: bool) -> i64 ! E {\n    if b {\n        Err(Bad)\n    } else {\n        3\n    }\n}\nfn main() {\n    print(\"{f(false) ?? -1}\")\n}\n");
}

/// A closure with parameters that nothing ever uses has nothing to infer from
/// ([T-FN-TYPE]).
#[test]
fn an_unusable_closure_is_refused() {
    check_err(
        "fn main() {\n    let f = |x| x\n    print(\"done\")\n}\n",
        "nothing says what `x` is",
    );
}

/// A handler's discharge may not leave its block inside an `Option`.
#[test]
fn some_cannot_carry_a_handler_row_out() {
    check_err(
        "effect Cfg {\n    fn value() -> i64\n}\nfn main() {\n    var o: Option[fn() -> i64 / {Cfg}] = None\n    with Cfg handled_by CfgHandler{ value: || 111 } {\n        o = Some(|| Cfg.value())\n    }\n    print(\"{(o ?? (|| 0))()}\")\n}\n",
        "cannot store a function that performs `Cfg`",
    );
}

/// A function value laundered through `Some(f) ?? f` still carries its row
/// ([T-PAR-MAP-PURE]).
#[test]
fn a_coalesced_function_keeps_its_row() {
    check_err(
        "fn shout(n: i64) -> i64 {\n    print(\"noisy\")\n    n\n}\nfn main() {\n    let o = Some(shout)\n    let g = o ?? shout\n    print(\"{[1, 2].par_map(g)}\")\n}\n",
        "needs an effect-free function",
    );
}

/// A function that returns nothing throws its tail away, and throwing away a
/// failure is using a fallible value as if it were `T`
/// ([T-FALLIBLE-UNHANDLED]).
#[test]
fn a_discarded_tail_is_refused() {
    check_err(
        "fn write_report(p: str, b: str) {\n    write_file(p, b)\n}\nfn main() {\n    write_report(\"/x\", \"y\")\n}\n",
        "failure is discarded",
    );
    // `exit`/`panic`/`fail` in that position produce no value to discard.
    check_ok("fn main() {\n    print(\"bye\")\n    exit(2)\n}\n");
    // And `main` may propagate, which is [E-MAIN-ERR], not a discard.
    check_ok("fn main() {\n    write_file(\"/x\", \"y\")\n}\n");
}

/// A closure body is checked against the closure, not the enclosing function.
#[test]
fn a_closure_body_is_its_own_function() {
    check_err(
        "fn f() -> str {\n    let g: fn(i64) -> i64 = |x| {\n        return \"oops\"\n    }\n    print(\"{g(1)}\")\n    \"done\"\n}\nfn main() {\n    print(f())\n}\n",
        "type mismatch",
    );
    check_err(
        "fn main() {\n    for i in [1] {\n        let f = |x| {\n            break\n        }\n        print(\"{i}\")\n    }\n}\n",
        "`break` outside of a loop",
    );
    // A `return` that agrees with the closure's own type is fine.
    check_ok(
        "fn main() {\n    let g: fn(i64) -> i64 = |x| {\n        if x < 0 {\n            return 0\n        }\n        x * 2\n    }\n    print(\"{g(3)}\")\n}\n",
    );
}

/// A value stored under a written function type must fit that type's row.
#[test]
fn a_written_row_is_checked_where_a_value_is_stored() {
    check_err(
        "fn id[T](x: T) -> T {\n    x\n}\nfn main() {\n    let f: fn() -> i64 = id(|| {\n        print(\"IO\")\n        1\n    })\n    print(\"{f()}\")\n}\n",
        "but the value performs `io`",
    );
    check_err(
        "effect Cfg {\n    fn value() -> i64\n}\nfn main() {\n    var keep: [fn() -> i64] = []\n    with Cfg handled_by CfgHandler{ value: || 1 } {\n        keep = [1].map(|_x| || Cfg.value())\n    }\n    print(\"{keep[0]()}\")\n}\n",
        "but the value performs `Cfg`",
    );
}

/// A `fail` in a closure that is not fallible belongs to the enclosing
/// function, and is sound only while the closure cannot outlive the call.
#[test]
fn a_stored_closure_may_not_fail() {
    check_err(
        "type E = { c: str }\ntype Box = { f: fn(i64) -> i64 }\nfn build() -> Box ! E {\n    Box{ f: |x| if x == 0 { fail(E{ c: \"z\" }) } else { x } }\n}\nfn main() {\n    let b = build() ?? Box{ f: |x| x }\n    print(\"{b.f(0)}\")\n}\n",
        "declares no error type",
    );
    // A built-in that calls the closure at once keeps working ([E-PAR-FAIL]).
    check_ok(
        "type E = | Bad\nfn go(xs: [i64]) -> [i64] ! E {\n    xs.par_map(|x| if x == 2 {\n        fail(Bad)\n    } else {\n        x\n    })\n}\nfn main() {\n    print(\"{go([1, 3]) ?? []}\")\n}\n",
    );
}

/// An `if` or `match` may give the bare ok half in one branch and the whole
/// `T ! E` in another; what it is worth is the whole type ([T-OK-ERR-RETURN]).
#[test]
fn the_two_halves_may_be_mixed_across_branches() {
    check_ok(
        "fn f(n: i64) -> i64 ! str {\n    if n == 1 {\n        10\n    } else {\n        Ok(n * 3)\n    }\n}\nfn main() {\n    print(\"{f(2) ?? 0 - 1}\")\n}\n",
    );
    check_ok(
        "fn f(n: i64) -> i64 ! str {\n    match n {\n        1 -> 10\n        2 -> Ok(n * 3)\n        _ -> Err(\"no\")\n    }\n}\nfn main() {\n    print(\"{f(2) ?? 0 - 1}\")\n}\n",
    );
    // Mixing halves is not mixing *types*: the ok halves still have to agree.
    check_err(
        "fn f(n: i64) -> i64 ! str {\n    if n == 1 {\n        \"x\"\n    } else {\n        Ok(n * 3)\n    }\n}\nfn main() {\n    print(\"{f(2) ?? 0 - 1}\")\n}\n",
        "different types",
    );
}

/// `main` takes no parameters ([E-ENTRY]).
///
/// The return type was refused and the parameters were not, so
/// `fn main(x: i64)` checked clean and then panicked at run time with a
/// compiler-internal message — and the backend could not generate code for it
/// at all.
#[test]
fn main_takes_no_parameters() {
    check_err(
        "fn main(x: i64) {\n    print(\"{x}\")\n}\n",
        "`main` takes no parameters",
    );
    check_err(
        "fn main(x: i64) {\n    print(\"hello\")\n}\n",
        "`main` takes no parameters",
    );
    // A `main` in another module is an ordinary function.
    check_ok("fn main() {\n    print(\"hello\")\n}\n");
}

/// A type alias is a type declaration: it may not repeat a name, and `pub`
/// means nothing on it ([T-MODULE-NS], [T-TYPE-NO-PUB]).
///
/// Aliases are resolved a pass later than struct and sum bodies, so nothing
/// had registered the first one when the second was collected — two
/// `type Id = ...` lines checked clean and the later one silently won.
#[test]
fn an_alias_is_a_type_declaration() {
    check_err(
        "type Id = i64\ntype Id = str\nfn main() {\n    print(\"x\")\n}\n",
        "duplicate type name `Id`",
    );
    check_err(
        "type Id = i64\ntype Id = { n: i64 }\nfn main() {\n    print(\"x\")\n}\n",
        "duplicate type name `Id`",
    );
    check_err(
        "pub type N = i64\nfn main() {\n    print(\"x\")\n}\n",
        "`pub` means nothing on type `N`",
    );
    check_ok("type Id = i64\nfn main() {\n    let x: Id = 1\n    print(\"{x}\")\n}\n");
}

/// Failures held inside a container that nothing reads are discarded
/// ([T-FALLIBLE-UNHANDLED]).
///
/// One pair of brackets was enough to get the old silence back.
#[test]
fn failures_inside_a_discarded_container_are_refused() {
    let prelude = "type E = | Bad\nfn risky(n: i64) -> i64 ! E {\n    if n < 0 {\n        fail(Bad)\n    } else {\n        n\n    }\n}\n";
    check_err(
        &format!("{prelude}fn main() {{\n    [risky(0 - 1)]\n    print(\"done\")\n}}\n"),
        "holds failures",
    );
    check_err(
        &format!("{prelude}fn main() {{\n    (risky(0 - 1), 1)\n    print(\"done\")\n}}\n"),
        "holds failures",
    );
    check_err(
        &format!("{prelude}fn main() {{\n    [1, 2].map(risky)\n    print(\"done\")\n}}\n"),
        "holds failures",
    );
    // Bound, they are a value somebody can read ([T-FALLIBLE-VALUE]).
    check_ok(&format!(
        "{prelude}fn main() {{\n    let rs = [1, 2].map(risky)\n    print(\"{{rs.len()}}\")\n}}\n"
    ));
}

/// A tail `if` whose branches call a fallible function produces the whole
/// `T ! E`, exactly as the same `match` does ([T-OK-ERR-RETURN]).
#[test]
fn a_tail_if_may_call_fallible_functions() {
    let prelude = "type E = | Bad\nfn risky(n: i64) -> i64 ! E {\n    if n < 0 {\n        fail(Bad)\n    } else {\n        n\n    }\n}\n";
    check_ok(&format!(
        "{prelude}fn pick(n: i64) -> i64 ! E {{\n    if n == 0 {{\n        risky(1)\n    }} else {{\n        risky(n)\n    }}\n}}\nfn main() {{\n    print(\"{{pick(2) ?? 0 - 1}}\")\n}}\n"
    ));
    check_ok(&format!(
        "{prelude}fn pick(n: i64) -> i64 ! E {{\n    if n == 0 {{\n        risky(1)\n    }} else {{\n        fail(Bad)\n    }}\n}}\nfn main() {{\n    print(\"{{pick(2) ?? 0 - 1}}\")\n}}\n"
    ));
    // And an int literal beside a float branch still promotes
    // ([T-INT-TO-FLOAT-CARRIED]).
    check_ok(
        "type E = | Bad\nfn f(c: bool) -> f64 ! E {\n    if c {\n        4.5\n    } else {\n        6\n    }\n}\nfn main() {\n    print(\"{f(false) ?? 0.0}\")\n}\n",
    );
}

/// A one-armed `if` throws its branch's value away, so a failure in that tail
/// is a discarded failure ([T-FALLIBLE-UNHANDLED]).
///
/// The `if` itself is `()`, so the statement rule saw nothing to complain
/// about: `fn save(p, b) { if c { write_file(p, b) } }` printed the line after
/// it, exited 0, wrote nothing to stderr and left no file.
#[test]
fn a_one_armed_if_does_not_swallow_a_failure() {
    let prelude = "type E = | Bad\nfn risky() -> i64 ! E {\n    fail(Bad)\n}\n";
    check_err(
        &format!("{prelude}fn caller() {{\n    if true {{\n        risky()\n    }}\n}}\nfn main() {{\n    caller()\n}}\n"),
        "its failure is discarded",
    );
    check_err(
        "fn save(path: str, body: str) {\n    if not path.is_empty() {\n        write_file(path, body)\n    }\n}\nfn main() {\n    save(\"/x\", \"y\")\n}\n",
        "its failure is discarded",
    );
    // Handled, it is fine.
    check_ok(
        &format!("{prelude}fn caller() {{\n    if true {{\n        risky() ?? 0\n    }}\n}}\nfn main() {{\n    caller()\n}}\n"),
    );
}

/// A generic forwarding to a generic makes its own parameter a map-key
/// parameter, however many hops away ([T-MAP-KEY-GENERIC]).
#[test]
fn a_key_generic_travels_through_forwarding() {
    let inner = "fn inner[T](xs: [T]) -> i64 {\n    var m: Map[T, i64] = {}\n    for x in xs {\n        m.set(x, 1)\n    }\n    m.len()\n}\n";
    check_err(
        &format!("{inner}fn outer[T](xs: [T]) -> i64 {{\n    inner(xs)\n}}\nfn main() {{\n    print(\"{{outer([1.5])}}\")\n}}\n"),
        "map keys must be",
    );
    check_err(
        &format!("{inner}fn h2[T](xs: [T]) -> i64 {{\n    inner(xs)\n}}\nfn h3[T](xs: [T]) -> i64 {{\n    h2(xs)\n}}\nfn main() {{\n    print(\"{{h3([1.5])}}\")\n}}\n"),
        "map keys must be",
    );
    check_ok(&format!(
        "{inner}fn outer[T](xs: [T]) -> i64 {{\n    inner(xs)\n}}\nfn main() {{\n    let n = outer([\"a\"])\n    print(\"{{n}}\")\n}}\n"
    ));
}

/// A tuple has no place to write into ([G-PLACE]).
#[test]
fn a_tuple_component_is_not_a_place() {
    check_err(
        "fn main() {\n    var t = (1, \"x\")\n    t.0 = 5\n    print(\"{t}\")\n}\n",
        "cannot assign to a tuple component",
    );
    check_err(
        "fn main() {\n    var t = ([1], \"x\")\n    t.0.push(5)\n    print(\"{t}\")\n}\n",
        "cannot use `push` on a tuple component",
    );
    // Reading one is ordinary.
    check_ok("fn main() {\n    let t = ([1], \"x\")\n    print(\"{t.0.len()}\")\n}\n");
}

/// `map()` is an empty literal like `[]` ([T-EMPTY-LIT]).
#[test]
fn an_undetermined_map_is_refused() {
    check_err(
        "fn main() {\n    let m = map()\n    print(\"{m.values().all(|x| x != 0)}\")\n}\n",
        "not determined by anything",
    );
    check_ok(
        "fn main() {\n    var m: Map[str, i64] = map()\n    m.set(\"a\", 1)\n    print(\"{m.len()}\")\n}\n",
    );
    check_ok(
        "fn main() {\n    var m = map()\n    m.set(\"a\", 1)\n    print(\"{m.len()}\")\n}\n",
    );
}

/// The "must return" template belongs to the tail expression and to no other.
#[test]
fn a_let_mismatch_is_not_blamed_on_the_return_type() {
    check_err(
        "fn g() -> i64 {\n    let r: str = 1\n    2\n}\nfn main() {\n    print(\"{g()}\")\n}\n",
        "type mismatch: expected `str`, found `i64`",
    );
    check_err(
        "fn g() -> str {\n    1\n}\nfn main() {\n    print(\"{g()}\")\n}\n",
        "function `g` must return `str` but its body produces `i64`",
    );
}

/// A value-bearing branch has to end in an expression ([E-IF-VALUE]).
///
/// `check_block_with` pushes the expected type into a tail *expression* and
/// has nothing to push it into otherwise, so a branch ending in a `with`
/// statement sailed past the demand: the checker accepted it, the interpreter
/// printed `()` where an integer belonged, and a native build printed `0`.
#[test]
fn a_value_branch_may_not_end_in_a_statement() {
    check_err(
        "effect D {\n    fn depth() -> i64\n}\nfn go(n: i64) -> i64 {\n    if n == 0 {\n        7\n    } else {\n        with D handled_by DHandler{ depth: || n * 10 } {\n            n + 1\n        }\n    }\n}\nfn main() {\n    with D handled_by DHandler{ depth: || 0 } {\n        print(\"{go(3)}\")\n    }\n}\n",
        "must produce `i64` but ends in a statement",
    );
    // A branch that leaves delivers its value by not coming back.
    check_ok(
        "fn g(n: i64) -> i64 {\n    if n < 0 {\n        return 0\n    } else {\n        n + 1\n    }\n}\nfn main() {\n    print(\"{g(2)}\")\n}\n",
    );
    // A statement followed by a value is the ordinary shape.
    check_ok(
        "fn g(n: i64) -> i64 {\n    if n == 0 {\n        7\n    } else {\n        var t = 0\n        for i in 0..n {\n            t = t + i\n        }\n        t\n    }\n}\nfn main() {\n    print(\"{g(4)}\")\n}\n",
    );
}

/// `inout` twice is about the same *place*, not the same root
/// ([T-INOUT-EXCLUSIVE]).
///
/// Comparing roots alone refused `two(p.xs, p.ys)` — two fields that share
/// nothing but the struct they live in.
#[test]
fn two_disjoint_fields_may_both_be_inout() {
    let two = "fn two(inout a: [i64], inout b: [i64]) {\n    a.push(1)\n    b.push(2)\n}\n";
    check_ok(&format!(
        "type P = {{ xs: [i64], ys: [i64] }}\n{two}fn main() {{\n    var p = P{{ xs: [], ys: [] }}\n    two(p.xs, p.ys)\n    print(\"{{p.xs}}\")\n}}\n"
    ));
    // The same field twice is the same place.
    check_err(
        &format!(
            "type P = {{ xs: [i64] }}\n{two}fn main() {{\n    var p = P{{ xs: [] }}\n    two(p.xs, p.xs)\n    print(\"{{p.xs}}\")\n}}\n"
        ),
        "passed as `inout` twice",
    );
    // A whole value beside one of its fields overlaps.
    check_err(
        &format!(
            "type P = {{ xs: [i64] }}\nfn mix(inout a: P, inout b: [i64]) {{\n    b.push(1)\n    a.xs.push(2)\n}}\nfn main() {{\n    var p = P{{ xs: [] }}\n    mix(p, p.xs)\n    print(\"{{p.xs}}\")\n}}\n"
        ),
        "passed as `inout` twice",
    );
    let _ = two;
    // Two indices may be the same element, so they are treated as one place.
    check_err(
        &format!(
            "{two}fn main() {{\n    var g = [[1], [2]]\n    two(g[0], g[1])\n    print(\"{{g}}\")\n}}\n"
        ),
        "passed as `inout` twice",
    );
}

/// An integer literal beside a whole `T ! E` adopts the ok half's float
/// ([T-INT-TO-FLOAT-CARRIED]).
///
/// The same `if` with *both* halves bare compiled; mixing a bare literal with
/// an `Ok(4.5)` was "branches of different types".
#[test]
fn an_int_literal_promotes_against_a_whole_fallible_float() {
    check_ok(
        "type E = | Bad\nfn f(c: bool) -> f64 ! E {\n    if c {\n        3\n    } else {\n        Ok(4.5)\n    }\n}\nfn main() {\n    print(\"{f(true) ?? 0.0}\")\n}\n",
    );
    // Either order.
    check_ok(
        "type E = | Bad\nfn f(c: bool) -> f64 ! E {\n    if c {\n        Ok(4.5)\n    } else {\n        3\n    }\n}\nfn main() {\n    print(\"{f(true) ?? 0.0}\")\n}\n",
    );
    // And across an `else if` chain that also carries an `Err`.
    check_ok(
        "type E = | Bad\nfn f(n: i64) -> f64 ! E {\n    if n == 0 {\n        3\n    } else if n == 1 {\n        Ok(4.5)\n    } else {\n        Err(Bad)\n    }\n}\nfn main() {\n    print(\"{f(0) ?? 0.0}\")\n}\n",
    );
    // A genuinely different type is still a mismatch.
    check_err(
        "type E = | Bad\nfn f(c: bool) -> f64 ! E {\n    if c {\n        \"x\"\n    } else {\n        Ok(4.5)\n    }\n}\nfn main() {\n    print(\"{f(true) ?? 0.0}\")\n}\n",
        "different types",
    );
}

/// Under an annotation naming `T ! E`, each branch may still give either half
/// ([T-OK-ERR-RETURN]).
///
/// Demanding the whole type of every branch made
/// `let v: f64 ! E = if c { 3.0 } else { Ok(4.5) }` a type mismatch, while the
/// same expression *without* the annotation was an unhandled failure whose
/// help says to annotate it — two diagnostics pointing at each other.
#[test]
fn an_annotated_fallible_branch_may_give_either_half() {
    check_ok(
        "type E = | Bad\nfn main() {\n    let c = true\n    let v: f64 ! E = if c {\n        3.0\n    } else {\n        Ok(4.5)\n    }\n    print(\"{v ?? 0.0}\")\n}\n",
    );
    check_ok(
        "type E = | Bad\nfn main() {\n    let c = true\n    let v: i64 ! E = if c {\n        3\n    } else {\n        Ok(4)\n    }\n    print(\"{v ?? 0 - 1}\")\n}\n",
    );
    // A real mismatch is still one.
    check_err(
        "type E = | Bad\nfn main() {\n    let c = true\n    let v: i64 ! E = if c {\n        \"x\"\n    } else {\n        Ok(4)\n    }\n    print(\"{v ?? 0 - 1}\")\n}\n",
        "expected `i64`, found `str`",
    );
}

/// `return` takes an `if` or `match` that mixes the two halves
/// ([T-OK-ERR-RETURN]).
///
/// The rule was written for a direct `Ok(..)`/`Err(..)`, so
/// `return if c { 3 } else { Ok(4) }` was "this expression can fail but `i64`
/// is required" — the message the rule replaced, one shape further out.
#[test]
fn return_takes_a_branching_expression_of_either_half() {
    let e = "type E = | Bad\n";
    check_ok(&format!(
        "{e}fn pick(c: bool) -> i64 ! E {{\n    return if c {{\n        3\n    }} else {{\n        Ok(4)\n    }}\n}}\nfn main() {{\n    print(\"{{pick(true) ?? 0 - 1}}\")\n}}\n"
    ));
    check_ok(&format!(
        "{e}fn pick(c: bool) -> i64 ! E {{\n    return if c {{\n        Err(Bad)\n    }} else {{\n        Ok(4)\n    }}\n}}\nfn main() {{\n    print(\"{{pick(true) ?? 0 - 1}}\")\n}}\n"
    ));
    check_ok(&format!(
        "{e}fn pick(c: bool) -> i64 ! E {{\n    return if c {{\n        3\n    }} else {{\n        4\n    }}\n}}\nfn main() {{\n    print(\"{{pick(true) ?? 0 - 1}}\")\n}}\n"
    ));
    check_ok(&format!(
        "{e}fn pick(n: i64) -> i64 ! E {{\n    return match n {{\n        0 -> 3\n        1 -> Ok(4)\n        _ -> Err(Bad)\n    }}\n}}\nfn main() {{\n    print(\"{{pick(0) ?? 0 - 1}}\")\n}}\n"
    ));
}

/// A failure held inside any container is discarded with it
/// ([T-FALLIBLE-UNHANDLED]).
///
/// The recursion stopped at the type-argument boundary, so one constructor —
/// `Some(save())`, a struct literal, a sum variant — laundered a failure past
/// the rule, and so did a list of them.
#[test]
fn a_constructor_does_not_launder_a_discarded_failure() {
    let prelude = "type E = | Bad\ntype Report = { written: () ! E }\ntype Wrapped = | W(c: () ! E) | N\nfn save() -> () ! E {\n    fail(Bad)\n}\n";
    for held in ["Some(save())", "Report{ written: save() }", "W(save())"] {
        check_err(
            &format!("{prelude}fn main() {{\n    {held}\n    print(\"done\")\n}}\n"),
            "holds failures",
        );
    }
    // Bound, they are a value somebody can read.
    check_ok(&format!(
        "{prelude}fn main() {{\n    let r = Some(save())\n    print(\"{{r.is_some()}}\")\n}}\n"
    ));
}

/// A tuple component is not a place, in the `inout` spelling too ([G-PLACE]).
#[test]
fn a_tuple_component_is_not_an_inout_argument() {
    check_err(
        "fn bump(inout n: i64) {\n    n += 1\n}\nfn main() {\n    var t = (1, 2)\n    bump(t.0)\n    print(\"{t}\")\n}\n",
        "cannot pass a tuple component as `inout`",
    );
    check_ok(
        "fn bump(inout n: i64) {\n    n += 1\n}\nfn main() {\n    var n = 1\n    bump(n)\n    print(\"{n}\")\n}\n",
    );
}

/// An effect and a type may not share a name ([T-MODULE-NS]).
///
/// An effect declares `EHandler` too, so a type of that name collides with
/// something the reader never wrote.
#[test]
fn an_effect_may_not_collide_with_a_type() {
    check_err(
        "type Clock = i64\neffect Clock {\n    fn now() -> i64\n}\nfn main() {\n    print(\"x\")\n}\n",
        "collides with a type of that name",
    );
    check_err(
        "type ClockHandler = i64\neffect Clock {\n    fn now() -> i64\n}\nfn main() {\n    print(\"x\")\n}\n",
        "declares `ClockHandler`",
    );
    check_ok(
        "effect Clock {\n    fn now() -> i64\n}\nfn main() {\n    with Clock handled_by ClockHandler{ now: || 1 } {\n        print(\"{Clock.now()}\")\n    }\n}\n",
    );
}

/// A C function is a link-time symbol of the package that declares it, so
/// `pub` on one is refused ([T-UNSAFE-FFI]).
///
/// Exported, the name escaped every gate that makes Tier 3 visible: a
/// qualified call needed no `unsafe`, charged no capability, panicked under
/// the interpreter and printed an answer from a native build.
#[test]
fn an_extern_may_not_be_pub() {
    check_err(
        "pub extern fn getpid() -> i64\nfn main() {\n    unsafe {\n        print(\"{getpid() > 0}\")\n    }\n}\n",
        "cannot be `pub`",
    );
    // The wrapper is how a package offers one.
    check_ok(
        "extern fn sqrt(x: f64) -> f64\npub fn root(x: f64) -> f64 {\n    unsafe {\n        sqrt(x)\n    }\n}\nfn main() {\n    print(\"{root(16.0)}\")\n}\n",
    );
}

/// The tail of a `()`-returning function throws its value away, so a
/// container of failures there is discarded ([T-FALLIBLE-UNHANDLED]).
///
/// The container rule was checked for statements only: adding one line after
/// `paths.map(|p| write_file(p, body))` caught it and leaving it last did
/// not — "wrote the report", exit 0, nothing on stderr, no file.
#[test]
fn a_unit_functions_tail_may_not_hold_failures() {
    check_err(
        "fn write_all(paths: [str], body: str) / {io} {\n    paths.map(|p| write_file(p, body))\n}\nfn main() {\n    write_all([\"/x/a.txt\"], \"hi\")\n}\n",
        "holds failures",
    );
    let prelude = "type E = | Bad\ntype R = { w: () ! E }\nfn s() -> () ! E {\n    fail(Bad)\n}\n";
    for held in ["[s()]", "Some(s())", "R{ w: s() }", "(s(), 1)"] {
        check_err(
            &format!("{prelude}fn go() {{\n    {held}\n}}\nfn main() {{\n    go()\n}}\n"),
            "holds failures",
        );
    }
    // Handled, it is fine.
    check_ok(&format!(
        "{prelude}fn go() {{\n    let r = [s()]\n    print(\"{{r.len()}}\")\n}}\nfn main() {{\n    go()\n}}\n"
    ));
}

/// A local shadows a function of the same name, for effect rows too
/// ([T-FN-VALUE-ROW] charges the row for *naming a function*).
///
/// A parameter spelled like a private helper charged its row: it broke a
/// `capabilities = []` package in every consumer, and failed a `par_map`
/// purity proof over a closure parameter's name.
#[test]
fn a_local_shadows_a_function_for_effect_rows() {
    let shout = "fn shout(n: i64) -> i64 / {io} {\n    print(\"{n}\")\n    n\n}\n";
    check_ok(&format!(
        "{shout}fn bump(shout: i64) -> i64 / {{}} {{\n    shout + 1\n}}\nfn main() {{\n    print(\"{{bump(1)}}\")\n}}\n"
    ));
    check_ok(&format!(
        "{shout}fn f() -> i64 / {{}} {{\n    let xs = [1, 2].par_map(|shout| shout * 2)\n    xs.len()\n}}\nfn main() {{\n    print(\"{{f()}}\")\n}}\n"
    ));
    // A real mention before the shadowing binding still counts.
    check_err(
        &format!(
            "{shout}fn f() -> i64 / {{}} {{\n    let a = shout(1)\n    let shout = 5\n    a + shout\n}}\nfn main() {{\n    print(\"{{f()}}\")\n}}\n"
        ),
        "not in its declared row",
    );
}

/// A `with` discharges the effect for a function *value* named inside it, not
/// only for a call ([T-EFFECT-DISCHARGE]).
#[test]
fn a_with_discharges_a_named_function_value() {
    let base = "effect Log {\n    fn info(m: str)\n}\nfn shout(m: str) / {Log} {\n    Log.info(\"!{m}\")\n}\n";
    for body in [
        "let f: fn(str) -> () / {Log} = shout\n        f(\"z\")",
        "let f = shout\n        f(\"z\")",
        "[\"a\"].map(shout)",
    ] {
        check_ok(&format!(
            "{base}fn main() {{\n    with Log handled_by LogHandler{{ info: |m| print(\"L: {{m}}\") }} {{\n        {body}\n    }}\n}}\n"
        ));
    }
    // Outside the `with`, it is still unhandled.
    check_err(
        &format!("{base}fn main() {{\n    let f = shout\n    f(\"z\")\n}}\n"),
        "unhandled effect",
    );
}

/// A function handed to a higher-order builtin must accept what that builtin
/// will give it, rows included ([T-PAR-MAP-PURE], [T-FN-TYPE-ROW]).
///
/// Parameters were checked covariantly, so a `call` taking a pure
/// `fn(i64) -> i64` accepted a list of `/ {io}` ones and `par_map` ran the
/// effect on several cores.
#[test]
fn a_callbacks_parameter_row_is_checked_the_other_way() {
    let base = "fn shout(n: i64) -> i64 / {io} {\n    print(\"x\")\n    n\n}\nfn call(f: fn(i64) -> i64) -> i64 {\n    f(1)\n}\n";
    check_err(
        &format!("{base}fn main() {{\n    let fs = [shout]\n    print(\"{{fs.par_map(call)}}\")\n}}\n"),
        "performs nothing, but this one performs `io`",
    );
    check_err(
        &format!("{base}fn main() {{\n    let fs = [shout]\n    print(\"{{fs.map(call)}}\")\n}}\n"),
        "performs nothing, but this one performs `io`",
    );
    // A pure element is fine.
    check_ok(
        "fn dbl(n: i64) -> i64 {\n    n * 2\n}\nfn call(f: fn(i64) -> i64) -> i64 {\n    f(1)\n}\nfn main() {\n    let fs = [dbl]\n    print(\"{fs.par_map(call)}\")\n}\n",
    );
}

/// A parallel operation's function may not write through a `ref` it captured
/// ([T-PAR-MAP-PURE]).
///
/// An empty effect row is only half of purity. Mutation is not an effect, so
/// nothing in the row system saw every element's call writing the same slot —
/// which is the interleaving the clause promises is a compile error.
#[test]
fn a_parallel_function_may_not_write_through_a_captured_ref() {
    let base = "type Acc = { n: i64, seen: [i64] }\n";
    check_err(
        &format!(
            "{base}fn main() {{\n    let acc = ref(Acc{{ n: 0, seen: [] }})\n    \
let ys = [1, 2].par_map(|x| {{\n        acc.n = acc.n + x\n        x\n    }})\n    \
print(\"{{ys}}\")\n}}\n"
        ),
        "every element writes the same slot",
    );
    // A mutating method through the same reference, and `par_filter` too.
    check_err(
        &format!(
            "{base}fn main() {{\n    let acc = ref(Acc{{ n: 0, seen: [] }})\n    \
let ys = [1, 2].par_map(|x| {{\n        acc.seen.push(x)\n        x\n    }})\n    \
print(\"{{ys}}\")\n}}\n"
        ),
        "every element writes the same slot",
    );
    check_err(
        &format!(
            "{base}fn main() {{\n    let acc = ref(Acc{{ n: 0, seen: [] }})\n    \
let ys = [1, 2].par_filter(|x| {{\n        acc.n = acc.n + x\n        x > 0\n    }})\n    \
print(\"{{ys}}\")\n}}\n"
        ),
        "every element writes the same slot",
    );
    // A `ref` created inside the function is one per element.
    check_ok(
        &format!(
            "{base}fn main() {{\n    let ys = [1, 2].par_map(|x| {{\n        \
let mine = ref(Acc{{ n: x, seen: [] }})\n        mine.n = mine.n * 2\n        \
mine.n\n    }})\n    print(\"{{ys}}\")\n}}\n"
        ),
    );
    // Reading a captured one shares nothing.
    check_ok(
        &format!(
            "{base}fn main() {{\n    let acc = ref(Acc{{ n: 7, seen: [] }})\n    \
let ys = [1, 2].par_map(|x| x + acc.n)\n    print(\"{{ys}}\")\n}}\n"
        ),
    );
    // `map` is sequential, so the write is exactly what [T-REF-CAPTURE] allows.
    check_ok(
        &format!(
            "{base}fn main() {{\n    let acc = ref(Acc{{ n: 0, seen: [] }})\n    \
let ys = [1, 2].map(|x| {{\n        acc.n = acc.n + x\n        x\n    }})\n    \
print(\"{{ys}} {{acc.n}}\")\n}}\n"
        ),
    );
}
