//! Typechecker integration tests.

use grasm_sema::analyze;

/// The stack the compiler gives its own front end (`COMPILER_STACK` in
/// `crates/grasm/src/main.rs`), which these tests have to match.
///
/// `analyze` recurses once per level of nesting, and the depth a program may
/// reach is decided by the checker's own "code is nested too deeply" limit —
/// not by whichever stack the caller happened to have. `grasm` establishes
/// that by running on a thread of its own; a test calling `analyze` directly
/// gets cargo's thread instead, and in a *debug* build the thousand-deep
/// projection chain in `a_chain_of_field_reads_is_linear` overflowed it and
/// aborted the whole test binary. Release passed, so every gate passed, and
/// only `cargo test` without `--release` ever saw it.
const COMPILER_STACK: usize = 256 * 1024 * 1024;

/// Runs `f` with the stack the compiler would have given it.
fn with_compiler_stack<T: Send + 'static>(f: impl FnOnce() -> T + Send + 'static) -> T {
    let spawned = std::thread::Builder::new()
        .stack_size(COMPILER_STACK)
        .spawn(f);
    match spawned {
        Ok(h) => match h.join() {
            Ok(v) => v,
            Err(panic) => std::panic::resume_unwind(panic),
        },
        // A thread that cannot be spawned leaves nothing to fall back to but
        // this one, and the caller would rather see the real failure than a
        // message about threads.
        Err(e) => panic!("could not spawn a checker thread: {e}"),
    }
}

fn check_ok(src: &str) {
    let owned = src.to_string();
    let diags = with_compiler_stack(move || analyze(&owned).1);
    assert!(
        diags.is_empty(),
        "expected no diagnostics for:\n{src}\ngot:\n{diags:#?}"
    );
}

/// Asserts at least one diagnostic contains `needle`.
fn check_err(src: &str, needle: &str) {
    let owned = src.to_string();
    let diags = with_compiler_stack(move || analyze(&owned).1);
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
    check_err(
        "fn f() {\n    let x = true + false\n}\n",
        "cannot apply `+`",
    );
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
    check_err("fn f() {\n    let x = 1\n    x = 2\n}\n", "it is immutable");
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
    check_ok(
        "fn add(a: i64, b: i64) -> i64 {\n    a + b\n}\n\nfn main() {\n    let x = add(1, 2)\n}\n",
    );
    check_err(
        "fn add(a: i64, b: i64) -> i64 {\n    a + b\n}\n\nfn main() {\n    let x = add(1)\n}\n",
        "expects 2 argument(s)",
    );
    check_err(
        "fn add(a: i64, b: i64) -> i64 {\n    a + b\n}\n\nfn main() {\n    let x = add(1, \"two\")\n}\n",
        "type mismatch",
    );
    check_err("fn bad() -> i64 {\n    \"nope\"\n}\n", "must return `i64`");
}

#[test]
fn explicit_return() {
    check_ok(
        "fn f(x: i64) -> i64 {\n    if x > 0 {\n        return x\n    }\n    return 0 - x\n}\n",
    );
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
    check_ok(
        "fn f(b: bool) -> i64 {\n    match b {\n        true -> 1\n        false -> 0\n    }\n}\n",
    );
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
    check_ok(
        "fn f() {\n    let x = Some(3)\n    let y = x ?? 0\n    let n: Option[str] = None\n}\n",
    );
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
    check_err("fn f() {\n    let s = [true].sum()\n}\n", "cannot sum");
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
    check_err(
        "fn main() {\n    for i in 3.5 {\n    }\n}\n",
        "cannot iterate",
    );
    check_err("fn main() {\n    break\n}\n", "outside of a loop");
}

#[test]
fn if_as_expression() {
    check_ok("fn f(x: i64) -> str {\n    if x > 0 { \"pos\" } else { \"neg\" }\n}\n");
    check_ok("fn f(x: i64) -> i64 {\n    let y = if x > 0 { 1 } else { 0 - 1 }\n    y\n}\n");
    check_err(
        "fn f(x: i64) {\n    if x { print(\"hm\") }\n}\n",
        "type mismatch",
    );
}

#[test]
fn tests_can_use_try() {
    check_ok("type E = | Bad\n\nfn risky() -> i64 ! E {\n    1\n}\n\ntest \"works\" {\n    let v = try risky()\n    assert_eq(v, 1)\n}\n");
}

#[test]
fn duplicate_names_diagnosed() {
    check_err("fn f() {\n}\n\nfn f() {\n}\n", "duplicate function name");
    check_err(
        "type A = { x: i64 }\ntype A = { y: i64 }\n",
        "duplicate type name",
    );
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
    check_err("fn f() / {Bogus} {\n}\n", "unknown effect `Bogus`");
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
    check_ok(&format!(
        "{bump}\nfn main() {{\n    var n = 0\n    bump(n)\n    print(\"{{n}}\")\n}}\n"
    ));
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
    check_err("test \"p\" (n: i64) {\n    assert(m == 0)\n}\n", "unknown");
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
    check_err("test \"p\" (n: i64) {\n    n = 1\n}\n", "immutable");
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

// net_workers ([L-LIB-NET-WORKERS]): a parallel operation whose function may
// perform `net`, and nothing else.

#[test]
fn net_workers_allows_net_and_answers_a_list() {
    check_ok("fn probe(i: i64) -> bool / {net} {\n    (tcp_listen(0) ?? -1) >= 0\n}\nfn main() / {io, net} {\n    let a: [bool] = net_workers(2, probe)\n    let b: [i64] = net_workers(2, |i| i + 1)\n    print(\"{a} {b}\")\n}\n");
}

#[test]
fn net_workers_refuses_every_effect_but_net() {
    check_err(
        "fn loud(i: i64) -> i64 / {io} {\n    print(\"hi\")\n    i\n}\nfn main() / {io, net} {\n    let a = net_workers(2, loud)\n}\n",
        "may perform only `net`",
    );
    check_err(
        "fn main() / {io, net} {\n    let a = net_workers(2, |i| {\n        print(\"x\")\n        i\n    })\n}\n",
        "may perform only `net`",
    );
}

#[test]
fn net_workers_refuses_a_ref_from_outside() {
    check_err(
        "type Acc = { n: i64 }\nfn main() / {io, net} {\n    let acc = ref(Acc{ n: 0 })\n    let a = net_workers(2, |i| acc.n + i)\n}\n",
        "reaches a `ref` from outside",
    );
}

#[test]
fn net_workers_performs_net() {
    check_err(
        "fn main() / {io} {\n    let a = net_workers(2, |i| i)\n}\n",
        "performs effect(s) `net`",
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
    // Named against the branch that disagrees, rather than against the `if`:
    // the declared return type is what the branches are checked against, so
    // the mismatch has a side.
    check_err(
        "fn f(n: i64) -> i64 ! str {\n    if n == 1 {\n        \"x\"\n    } else {\n        Ok(n * 3)\n    }\n}\nfn main() {\n    print(\"{f(2) ?? 0 - 1}\")\n}\n",
        "expected `i64`, found `str`",
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
    check_ok("fn main() {\n    var m = map()\n    m.set(\"a\", 1)\n    print(\"{m.len()}\")\n}\n");
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
        "type P = { xs: [i64] }\nfn mix(inout a: P, inout b: [i64]) {\n    b.push(1)\n    a.xs.push(2)\n}\nfn main() {\n    var p = P{ xs: [] }\n    mix(p, p.xs)\n    print(\"{p.xs}\")\n}\n",
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
    // A genuinely different type is still a mismatch, named against the
    // branch that disagrees rather than against the `if`.
    check_err(
        "type E = | Bad\nfn f(c: bool) -> f64 ! E {\n    if c {\n        \"x\"\n    } else {\n        Ok(4.5)\n    }\n}\nfn main() {\n    print(\"{f(true) ?? 0.0}\")\n}\n",
        "found `str`",
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
        &format!(
            "{base}fn main() {{\n    let fs = [shout]\n    print(\"{{fs.par_map(call)}}\")\n}}\n"
        ),
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

/// A parallel operation's function may not reach a `ref` from outside — read
/// or written ([T-PAR-MAP-PURE]).
///
/// An empty effect row is only half of purity. Mutation is not an effect, so
/// nothing in the row system saw every element's call reaching the same slot —
/// which is the interleaving the clause promises is a compile error.
///
/// Reading was refused later than writing, and had to be: a read through a
/// reference takes a count on what it finds, and that count is not atomic. Once
/// the native backend compiled writes through a reference, a program that only
/// *read* one — `[1, 2].par_map(|x| x + acc.n)` — died with SIGTRAP, or
/// panicked inside the runtime, or printed a different wrong answer each run.
#[test]
fn a_parallel_function_may_not_reach_a_ref_from_outside() {
    let base = "type Acc = { n: i64, seen: [i64] }\n";
    check_err(
        &format!(
            "{base}fn main() {{\n    let acc = ref(Acc{{ n: 0, seen: [] }})\n    \
let ys = [1, 2].par_map(|x| {{\n        acc.n = acc.n + x\n        x\n    }})\n    \
print(\"{{ys}}\")\n}}\n"
        ),
        "inside a parallel operation",
    );
    // A mutating method through the same reference, and `par_filter` too.
    check_err(
        &format!(
            "{base}fn main() {{\n    let acc = ref(Acc{{ n: 0, seen: [] }})\n    \
let ys = [1, 2].par_map(|x| {{\n        acc.seen.push(x)\n        x\n    }})\n    \
print(\"{{ys}}\")\n}}\n"
        ),
        "inside a parallel operation",
    );
    check_err(
        &format!(
            "{base}fn main() {{\n    let acc = ref(Acc{{ n: 0, seen: [] }})\n    \
let ys = [1, 2].par_filter(|x| {{\n        acc.n = acc.n + x\n        x > 0\n    }})\n    \
print(\"{{ys}}\")\n}}\n"
        ),
        "inside a parallel operation",
    );
    // A `ref` created inside the function is one per element.
    check_ok(&format!(
        "{base}fn main() {{\n    let ys = [1, 2].par_map(|x| {{\n        \
let mine = ref(Acc{{ n: x, seen: [] }})\n        mine.n = mine.n * 2\n        \
mine.n\n    }})\n    print(\"{{ys}}\")\n}}\n"
    ));
    // Reading a captured one was allowed, on the reasoning that a read shares
    // nothing. It shares the *reference count*: a read through a reference
    // takes one on what it finds, and it is not atomic. This assertion was
    // this test asserting the defect — harmless only while the native backend
    // refused writes through a reference, and a SIGTRAP the day it stopped.
    check_err(
        &format!(
            "{base}fn main() {{\n    let acc = ref(Acc{{ n: 7, seen: [] }})\n    \
let ys = [1, 2].par_map(|x| x + acc.n)\n    print(\"{{ys}}\")\n}}\n"
        ),
        "inside a parallel operation",
    );
    // The element type reaching a reference is refused at the operation, since
    // there is no mention of an outside binding to point at: every element
    // *is* the shared thing.
    for (elem, build) in [
        ("[ref Acc]", "var rs: [ref Acc] = []\n    rs.push(acc)"),
        (
            "[Holder]",
            "var rs: [Holder] = []\n    rs.push(Holder{ r: acc })",
        ),
    ] {
        let _ = elem;
        for op in ["par_map(|r| 1)", "par_filter(|r| true)"] {
            check_err(
                &format!(
                    "{base}type Holder = {{ r: ref Acc }}\n\nfn main() {{\n    \
let acc = ref(Acc{{ n: 7, seen: [] }})\n    {build}\n    \
let ys = rs.{op}\n    print(\"{{ys.len()}}\")\n}}\n"
                ),
                "reaches a `ref`",
            );
        }
    }
    // `map` is sequential, so the write is exactly what [T-REF-CAPTURE] allows.
    check_ok(&format!(
        "{base}fn main() {{\n    let acc = ref(Acc{{ n: 0, seen: [] }})\n    \
let ys = [1, 2].map(|x| {{\n        acc.n = acc.n + x\n        x\n    }})\n    \
print(\"{{ys}} {{acc.n}}\")\n}}\n"
    ));
}

/// The builtin capability names cannot be declared as effects
/// ([T-EFFECT-BUILTIN]).
///
/// `io` and `async` are not keywords, so `effect io { … }` parsed — and the
/// `with io handled_by …` it made available discharged the *builtin* `io` for
/// everything in the block. A function declaring `/ {}` printed, `par_map`
/// ran a printing function on several cores, and a `capabilities = []`
/// package read and wrote files while `grasm deps` certified it as performing
/// nothing.
#[test]
fn a_builtin_capability_cannot_be_declared_as_an_effect() {
    for name in ["io", "async"] {
        check_err(
            &format!("effect {name} {{\n    fn nothing() -> i64\n}}\n\nfn main() {{\n}}\n"),
            "is reserved by the language",
        );
    }
    // The hole it opened, in each of the three places it defeated.
    let decl = "effect io {\n    fn nothing() -> i64\n}\n";
    check_err(
        &format!(
            "{decl}fn quiet(n: i64) -> i64 / {{}} {{\n    \
with io handled_by ioHandler{{ nothing: || 1 }} {{\n        print(\"x\")\n    }}\n    n\n}}\n\n\
fn main() {{\n    print(\"{{quiet(1)}}\")\n}}\n"
        ),
        "is reserved by the language",
    );
    check_err(
        &format!(
            "{decl}fn shout(n: i64) -> i64 / {{}} {{\n    \
with io handled_by ioHandler{{ nothing: || 1 }} {{\n        print(\"x\")\n    }}\n    n\n}}\n\n\
fn main() {{\n    print(\"{{[1, 2].par_map(shout)}}\")\n}}\n"
        ),
        "is reserved by the language",
    );
    // And there is no `with io handled_by` even without the declaration.
    check_err(
        "fn main() {\n    with io handled_by ioHandler{ nothing: || 1 } {\n        print(\"x\")\n    }\n}\n",
        "unknown effect `io`",
    );
    // An effect merely *named* like one of them is fine.
    check_ok(
        "effect Io {\n    fn nothing() -> i64\n}\n\nfn main() {\n    \
with Io handled_by IoHandler{ nothing: || 1 } {\n        print(\"{Io.nothing()}\")\n    }\n}\n",
    );
}

/// A container of failures in `main`'s tail is discarded like anywhere else
/// ([T-FALLIBLE-UNHANDLED]).
///
/// `main` has an open error slot, so a directly fallible tail there is
/// propagated ([E-MAIN-ERR]) — and the whole check was skipped for `main` on
/// that account. The slot carries one failure, not a list of them, so
/// `fn main() { xs.map(risky) }` threw away every one: silent, exit 0. Adding
/// any line after it caught the same expression.
#[test]
fn a_container_of_failures_in_mains_tail_is_not_discarded() {
    let base = "type E = | Bad\nfn risky(x: i64) -> i64 ! E {\n    \
if x == 0 {\n        fail(Bad)\n    } else {\n        Ok(x)\n    }\n}\n";
    for tail in ["[1, 2].map(risky)", "[risky(1)]", "Some(risky(1))"] {
        check_err(
            &format!("{base}fn main() {{\n    {tail}\n}}\n"),
            "holds failures",
        );
    }
    // A directly fallible tail still propagates: that is what the slot is for.
    check_ok(&format!("{base}fn main() {{\n    risky(0)\n}}\n"));
    check_ok(&format!(
        "{base}fn main() {{\n    let r = [1, 2].map(risky)\n    \
match r[0] {{\n        Ok(v) -> print(\"{{v}}\")\n        Err(e) -> print(\"bad\")\n    }}\n}}\n"
    ));
}

/// A field read waits for a receiver whose type is not settled yet.
///
/// `let f = |x| S{ c: x }` defers its body until its call sites are seen, so
/// `f(2).c` one line later asked an inference variable for a field and was
/// told ``_` has no field `c`` — naming a type no program can write, about a
/// value every other use of the same call inferred correctly.
#[test]
fn a_field_read_waits_for_a_deferred_closures_result() {
    let base = "type S = { c: i64 }\n";
    check_ok(&format!(
        "{base}fn main() {{\n    let f = |x| S{{ c: x }}\n    print(\"{{f(2).c}}\")\n}}\n"
    ));
    // Through a binding, and for a tuple component.
    check_ok(&format!(
        "{base}fn main() {{\n    let f = |x| S{{ c: x }}\n    let s = f(2)\n    \
print(\"{{s.c}}\")\n}}\n"
    ));
    check_ok("fn main() {\n    let f = |x| (x, x + 1)\n    print(\"{f(2).0} {f(2).1}\")\n}\n");
    // A field the type does not have is still refused, and named by its type.
    check_err(
        &format!(
            "{base}fn main() {{\n    let f = |x| S{{ c: x }}\n    print(\"{{f(2).nope}}\")\n}}\n"
        ),
        "`S` has no field `nope`",
    );
    check_err(
        "fn main() {\n    let f = |x| x\n    print(\"{f(2).nope}\")\n}\n",
        "`i64` has no field `nope`",
    );
}

/// A builtin type name cannot be redeclared.
///
/// `resolve_type` answers for `i64`, `str`, `Map` and the rest before it ever
/// consults a declaration, and those names are not in the definition table —
/// so nothing collided with them. `type Map = { x: i64 }` and `type i64 = str`
/// were accepted and then silently ignored, which reads to a reviewer as a
/// definition that took effect.
#[test]
fn a_builtin_type_name_cannot_be_redeclared() {
    for decl in [
        "type Map = { x: i64 }",
        "type i64 = str",
        "type str = i64",
        "type bool = i64",
        "type f64 = i64",
        "type float = i64",
        "type int = str",
    ] {
        check_err(
            &format!("{decl}\n\nfn main() {{\n    print(\"x\")\n}}\n"),
            "is built in",
        );
    }
    // The prelude's own types keep their own message, and an ordinary name
    // is unaffected.
    check_err(
        "type Option = { x: i64 }\n\nfn main() {\n}\n",
        "reserved by the prelude",
    );
    check_ok("type Good = { x: i64 }\n\nfn main() {\n    print(\"{Good{ x: 1 }.x}\")\n}\n");
}

/// Every binder carries a function value's effect row, not just `let`
/// ([T-FN-VALUE-ROW], [T-FN-TYPE-ROW-CARRIED]).
///
/// `let f = fs[0]` was charged correctly and `for f in fs` was charged
/// nothing — the same program, one keyword apart. So were a `match` payload,
/// a tuple destructuring and a closure parameter, each of which carried a
/// printing function past a `/ {}` row, `par_map`'s purity proof and a
/// package's `capabilities = []`.
#[test]
fn every_binder_carries_a_function_values_row() {
    let base = "fn shout(n: i64) -> i64 {\n    print(\"shout {n}\")\n    n\n}\n";
    let main = "fn main() {\n    print(\"{quiet([shout])}\")\n}\n";
    let bodies = [
        "fn quiet(fs: [fn(i64) -> i64 / {io}]) -> i64 / {} {\n    let f = fs[0]\n    f(5)\n}\n",
        "fn quiet(fs: [fn(i64) -> i64 / {io}]) -> i64 / {} {\n    for f in fs {\n        return f(5)\n    }\n    0\n}\n",
        "fn quiet(fs: [fn(i64) -> i64 / {io}]) -> i64 / {} {\n    match fs.find(|g| true) {\n        Some(f) -> f(5)\n        None -> 0\n    }\n}\n",
        "fn quiet(fs: [fn(i64) -> i64 / {io}]) -> i64 / {} {\n    fs.map(|f| f(5))[0]\n}\n",
    ];
    for body in bodies {
        check_err(&format!("{body}{base}{main}"), "not in its declared row");
    }
    // And through a parallel operation, which the clause calls a compile
    // error rather than a race.
    check_err(
        &format!(
            "fn quiet(fs: [fn(i64) -> i64 / {{io}}]) -> [i64] / {{}} {{\n    \
fs.par_map(|f| f(5))\n}}\n{base}{main}"
        ),
        "not in its declared row",
    );
    // An element type that declares no effects is not charged for a row
    // written somewhere else in the program.
    check_ok(
        "type Cb = fn(i64) -> i64 / {io}\n\
fn shout(n: i64) -> i64 {\n    print(\"{n}\")\n    n\n}\n\
fn loud(f: Cb) -> i64 / {io} {\n    f(1)\n}\n\
fn dbl(n: i64) -> i64 {\n    n * 2\n}\n\
fn pure_sum(fs: [fn(i64) -> i64]) -> i64 / {} {\n    var t = 0\n    \
for f in fs {\n        t = t + f(2)\n    }\n    t\n}\n\
fn main() {\n    print(\"{loud(shout)}\")\n    print(\"{pure_sum([dbl])}\")\n}\n",
    );
}

/// A type alias for a function type carries its row ([T-TYPE-ALIAS]: "it is
/// not a new type, and the two are interchangeable everywhere").
///
/// This pass reads written syntax rather than resolved types, so a row behind
/// a name was not there: `f: Cb` seeded an *empty* declared row, which then
/// shadowed every fallback. One `type` line laundered `io`.
#[test]
fn a_type_alias_carries_its_functions_row() {
    let tail = "fn shout(n: i64) -> i64 {\n    print(\"shout {n}\")\n    n\n}\n\n\
fn main() {\n    print(\"{quiet(shout, 5)}\")\n}\n";
    check_err(
        &format!(
            "type Cb = fn(i64) -> i64 / {{io}}\n\n\
fn quiet(f: Cb, x: i64) -> i64 / {{}} {{\n    f(x)\n}}\n\n{tail}"
        ),
        "not in its declared row",
    );
    // Through a second alias, and inside a container.
    check_err(
        &format!(
            "type Cb = fn(i64) -> i64 / {{io}}\ntype Cb2 = Cb\n\n\
fn quiet(f: Cb2, x: i64) -> i64 / {{}} {{\n    f(x)\n}}\n\n{tail}"
        ),
        "not in its declared row",
    );
    check_err(
        "type Cb = fn(i64) -> i64 / {io}\n\n\
fn quiet(fs: [Cb]) -> i64 / {} {\n    for f in fs {\n        return f(5)\n    }\n    0\n}\n\n\
fn shout(n: i64) -> i64 {\n    print(\"shout {n}\")\n    n\n}\n\n\
fn main() {\n    print(\"{quiet([shout])}\")\n}\n",
        "not in its declared row",
    );
    // An alias for a pure function type is still pure.
    check_ok(
        "type Pure = fn(i64) -> i64\n\n\
fn quiet(f: Pure, x: i64) -> i64 / {} {\n    f(x)\n}\n\n\
fn dbl(n: i64) -> i64 {\n    n * 2\n}\n\n\
fn main() {\n    print(\"{quiet(dbl, 5)}\")\n}\n",
    );
}

/// A builtin free function's name cannot be redeclared.
///
/// A call resolves to the builtin before it consults any declaration, so
/// `fn print(x: str)` was accepted and then never ran — the builtin printed
/// instead, and the reader had written a function nothing calls.
#[test]
fn a_builtin_function_name_cannot_be_redeclared() {
    for name in [
        "print",
        "panic",
        "env",
        "file_exists",
        "fail",
        "exit",
        "read_file",
    ] {
        check_err(
            &format!("fn {name}(x: str) -> i64 {{\n    99\n}}\n\nfn main() {{\n}}\n"),
            "is built in",
        );
    }
    // A free function used in method position is the ordinary UFCS
    // extension, and a *method* name is not reserved.
    check_ok(
        "fn describe(xs: [i64]) -> str {\n    \"n={xs.len()}\"\n}\n\n\
fn main() {\n    print(\"{[1, 2].describe()}\")\n}\n",
    );
}

/// A row written inside a declared type travels with that type's name
/// ([T-FN-TYPE-ROW-CARRIED], [T-TYPE-ALIAS]).
///
/// A written type is read for the rows inside it, and a *name* stood for
/// nothing: a struct field spelled with an alias, and a sum variant's payload
/// reached through a `match`, both contributed an empty row. The same program
/// with the row written inline was refused, so the guarantee turned on how the
/// type was spelled.
#[test]
fn a_row_inside_a_declared_type_travels_with_its_name() {
    let base = "fn shout(n: i64) -> i64 {\n    print(\"shout {n}\")\n    n\n}\n";
    // A sum variant's payload, read back through a `match`.
    check_err(
        &format!(
            "{base}type V = | Hold(f: fn(i64) -> i64 / {{io}}) | Nil\n\n\
fn pure(v: V) -> i64 / {{}} {{\n    match v {{\n        Hold(g) -> g(7)\n        Nil -> 0\n    }}\n}}\n\n\
fn main() {{\n    print(\"{{pure(Hold(shout))}}\")\n}}\n"
        ),
        "not in its declared row",
    );
    // A struct field whose type is an alias, and the same inline.
    for field in ["Cb", "fn(i64) -> i64 / {io}"] {
        check_err(
            &format!(
                "type Cb = fn(i64) -> i64 / {{io}}\ntype Box = {{ f: {field} }}\n{base}\n\
fn pure(b: Box) -> i64 / {{}} {{\n    b.f(5)\n}}\n\n\
fn main() {{\n    print(\"{{pure(Box{{ f: shout }})}}\")\n}}\n"
            ),
            "not in its declared row",
        );
    }
    // Through a parallel operation, which the clause calls a compile error
    // rather than a race — the constructor carries the value there.
    check_err(
        &format!(
            "{base}type V = | Hold(f: fn(i64) -> i64 / {{io}}) | Nil\n\n\
fn main() {{\n    let v = Hold(shout)\n    \
print(\"{{[1, 2].par_map(|x| match v {{ Hold(g) -> g(x) Nil -> 0 }})}}\")\n}}\n"
        ),
        "effect-free function",
    );
    // A named type whose fields declare no effects is charged nothing.
    check_ok(
        "type P = | Keep(f: fn(i64) -> i64) | Skip\n\n\
fn dbl(n: i64) -> i64 {\n    n * 2\n}\n\n\
fn pure(p: P) -> i64 / {} {\n    match p {\n        Keep(g) -> g(7)\n        Skip -> 0\n    }\n}\n\n\
fn main() {\n    print(\"{pure(Keep(dbl))}\")\n}\n",
    );
}

/// An integer literal reaches its expected type through a constructor in a
/// `match` arm or an `if` branch ([T-INT-TO-FLOAT-CARRIED]).
///
/// The deferral looked for a *bare* literal, so `Some(3)` beside `Some(4.5)`
/// was inferred as `Option[i64]` and reported a mismatch against a program the
/// clause calls well typed — while `Some(3)` in every other carrier position
/// was accepted, and the same two branches spelled `3` and `4.5` were too.
#[test]
fn a_literal_inside_a_constructor_adopts_the_expected_float() {
    check_ok(
        "fn g(n: i64) -> Option[f64] {\n    match n {\n        1 -> Some(4.5)\n        \
_ -> Some(3)\n    }\n}\n\nfn main() {\n    print(\"{g(1) ?? -1.0} {g(2) ?? -1.0}\")\n}\n",
    );
    let e = "type E = { m: str }\n";
    check_ok(&format!(
        "{e}fn g(n: i64) -> f64 ! E {{\n    match n {{\n        1 -> Ok(4.5)\n        \
_ -> Ok(3)\n    }}\n}}\n\nfn main() {{\n    print(\"{{g(1) ?? -1.0}} {{g(2) ?? -1.0}}\")\n}}\n"
    ));
    // The literal on either side of an `if`, inside the constructor or not.
    for (a, b) in [("Ok(3)", "4.5"), ("3", "Ok(4.5)"), ("Ok(3)", "Ok(4.5)")] {
        check_ok(&format!(
            "{e}fn g(c: bool) -> f64 ! E {{\n    if c {{\n        {a}\n    }} else {{\n        {b}\n    }}\n}}\n\n\
fn main() {{\n    print(\"{{g(true) ?? -1.0}} {{g(false) ?? -1.0}}\")\n}}\n"
        ));
    }
    // A genuine mismatch is still a mismatch.
    check_err(
        "fn g(n: i64) -> Option[f64] {\n    match n {\n        1 -> Some(4.5)\n        \
_ -> Some(\"x\")\n    }\n}\n\nfn main() {\n    print(\"{g(1) ?? -1.0}\")\n}\n",
        "type mismatch",
    );
}

/// A closure value carries what its body performs, through an unannotated
/// `let` as well as in place ([T-FN-TYPE-ROW], [T-FN-TYPE-ROW-CARRIED]).
///
/// The literal's row was recorded against its own span and consulted only
/// where the literal appeared, so one `let` between it and its destination
/// erased it: `let c = |m| print(m)` then `Box{ f: c }` put a printing closure
/// into a written `fn(str)` — a declared empty row — while the same literal
/// written in place was refused.
#[test]
fn a_closure_carries_its_row_through_a_let() {
    let head = "type Box = { f: fn(str) }\n\n";
    for build in [
        "Box{ f: |m| print(\"x {m}\") }",
        "{\n        let c = |m| print(\"x {m}\")\n        Box{ f: c }\n    }",
    ] {
        check_err(
            &format!(
                "{head}fn build() -> Box / {{io}} {{\n    {build}\n}}\n\n\
fn pure_caller(b: Box) / {{}} {{\n    b.f(\"hello\")\n}}\n\n\
fn main() {{\n    pure_caller(build())\n}}\n"
            ),
            "performs",
        );
    }
    // Through a `push` into an annotated list, and a `set` into a map.
    check_err(
        "fn main() {\n    var fs: [fn(str)] = []\n    let c = |m| print(\"x {m}\")\n    \
fs.push(c)\n    fs[0](\"hi\")\n}\n",
        "performs",
    );
    // A handled effect cannot leave its `with` block that way either
    // ([E-HANDLER-MISSING]).
    check_err(
        "effect Log {\n    fn info(m: str)\n}\n\nfn main() {\n    \
var fs: [fn(str) / {io}] = []\n    \
with Log handled_by LogHandler{ info: |m| print(\"shout {m}\") } {\n        \
let c = |m| Log.info(m)\n        fs.push(c)\n    }\n    fs[0](\"late\")\n}\n",
        "performs",
    );
    // A pure closure still fits a pure written type.
    check_ok(
        "type Box = { f: fn(str) -> i64 }\n\nfn main() {\n    \
let c = |m| m.len()\n    let b = Box{ f: c }\n    print(\"{b.f(\"hi\")}\")\n}\n",
    );
}

/// A `type` alias that stands for a shape holding itself is refused, naming
/// itself.
///
/// It has no base case and no indirection to stop at, unlike a struct or a sum
/// ([T-RECURSIVE]). Every pass that walked it recursed forever: the process
/// aborted with a Rust stack overflow and exit 134 and no diagnostic at all —
/// and a dependency carrying the line took its consumer's toolchain down with
/// it. [G-NEST-LIMIT]: exceeding a cap is a diagnostic, never a crash.
#[test]
fn a_self_referential_alias_is_refused_not_a_crash() {
    for body in [
        "[A]",
        "(A, i64)",
        "Map[str, A]",
        "fn(A) -> i64",
        "Option[A]",
        "i64 ! A",
        "ref A",
    ] {
        check_err(
            &format!("type A = {body}\n\nfn main() {{\n}}\n"),
            "defined in terms of itself",
        );
    }
    // A mutually recursive pair is the same thing spelled twice.
    check_err(
        "type A = [B]\ntype B = [A]\n\nfn main() {\n}\n",
        "defined in terms of itself",
    );
    // The nominal forms [T-RECURSIVE] allows are unaffected.
    check_ok(
        "type A = { x: Option[A] }\ntype S = | W(x: [S]) | Z\n\n\
fn main() {\n    let a = A{ x: None }\n    print(\"{a.x.is_none()}\")\n}\n",
    );
}

/// Diagnostics come out in source order, every run.
///
/// The effect pass walks a hash map, so the same file reported its errors in a
/// different order on every run — and `grasm run` and `grasm build` could
/// disagree about the order of the same two, which makes compiler output
/// undiffable in CI and in any `--check` gate.
#[test]
fn diagnostics_come_out_in_source_order() {
    let src = "fn a() / {} {\n    print(\"a\")\n}\n\nfn b() / {} {\n    print(\"b\")\n}\n\n\
fn c() / {} {\n    print(\"c\")\n}\n\nfn main() {\n    a()\n    b()\n    c()\n}\n";
    let (_, diags) = grasm_sema::analyze(src);
    let spans: Vec<u32> = diags.iter().map(|d| d.span.start).collect();
    let mut sorted = spans.clone();
    sorted.sort();
    assert_eq!(
        spans, sorted,
        "diagnostics are not in source order: {spans:?}"
    );
    assert!(spans.len() >= 3, "expected one per function: {spans:?}");
}

/// A fallible function's declared return reaches a tail that *mixes* the two
/// halves ([T-OK-ERR-RETURN], [T-INT-TO-FLOAT-CARRIED]).
///
/// A branching tail giving `Err(e)` on one side and a bare value on the other
/// reached no expectation at all, so the merge had only the branches to go on:
/// `-> f64 ! E` with `if c { Err(Bad) } else { 4 }` produced `i64 ! E` and was
/// refused — while the identical body under a `let v: f64 ! E =` annotation
/// was accepted. The clause says the rule is about the *position*.
#[test]
fn a_declared_fallible_return_reaches_a_mixed_tail() {
    let e = "type E = | Bad\n";
    for body in [
        "if c { Err(Bad) } else { 4 }",
        "if c { 4 } else { Err(Bad) }",
        "if c { 6 } else { Ok(7) }",
        "if c { 6.0 } else { Ok(7) }",
        "if c { Ok(6) } else { Ok(7) }",
        "if c { fail(Bad) } else { 4 }",
    ] {
        check_ok(&format!(
            "{e}fn m(c: bool) -> f64 ! E {{\n    {body}\n}}\n\n\
fn main() {{\n    print(\"{{m(false) ?? -1.0}}\")\n}}\n"
        ));
    }
    check_ok(&format!(
        "{e}fn m(n: i64) -> f64 ! E {{\n    match n {{\n        1 -> Err(Bad)\n        _ -> 4\n    }}\n}}\n\n\
fn main() {{\n    print(\"{{m(2) ?? -1.0}}\")\n}}\n"
    ));
    check_ok(&format!(
        "{e}fn m(c: bool) -> f64 ! E {{\n    match c {{\n        true -> 6.0\n        false -> Ok(7)\n    }}\n}}\n\n\
fn main() {{\n    print(\"{{m(false) ?? -1.0}}\")\n}}\n"
    ));
    // A plain ok-half tail is unaffected — the ordinary case.
    check_ok(
        "type MathError = | DivByZero\n\nfn safe_div(a: i64, b: i64) -> i64 ! MathError {\n    \
if b == 0 {\n        fail(DivByZero)\n    }\n    a / b\n}\n\n\
fn main() {\n    print(\"{safe_div(10, 2) ?? 0}\")\n}\n",
    );
}

/// A mixed-literal tuple in a `match` arm adopts the expected type.
///
/// Inference alone gave `(3.5, 4)` the type `(f64, i64)`, so it was refused —
/// while the same tuple in every other position, and a mixed *list* in the
/// same arm, promoted. [T-INT-TO-FLOAT-CARRIED] names both "a list, tuple or
/// map literal" and "a `match`'s arms".
#[test]
fn a_mixed_tuple_in_a_match_arm_promotes() {
    check_ok(
        "fn main() {\n    let t: (f64, f64) = match 1 {\n        _ -> (3.5, 4)\n    }\n    \
print(\"{t}\")\n}\n",
    );
    check_ok(
        "type P = { x: f64, y: f64 }\n\nfn main() {\n    \
let p: P = match 1 {\n        _ -> P{ x: 3.5, y: 4 }\n    }\n    print(\"{p.x} {p.y}\")\n}\n",
    );
    // A genuine mismatch inside the tuple is still one.
    check_err(
        "fn main() {\n    let t: (f64, f64) = match 1 {\n        _ -> (3.5, \"x\")\n    }\n    \
print(\"{t}\")\n}\n",
        "type mismatch",
    );
}

/// `??` carries a function value's effect row like every other route
/// ([T-FN-VALUE-ROW], [T-FN-TYPE-ROW-CARRIED]).
///
/// A binding with *some* recognised source and one unrecognised operand
/// looked fully accounted for, so the signature fallback never fired:
/// `let g = o ?? nop` recorded only the pure `nop` and charged nothing, while
/// `o.unwrap_or(nop)` and the `match` spelling of the same thing were both
/// charged. One operator erased a row that `/ {}`, [T-PAR-MAP-PURE] and a
/// package's `capabilities = []` all rest on.
#[test]
fn a_coalesced_function_value_keeps_its_row() {
    let head = "fn nop(x: i64) {\n}\n\n";
    for body in [
        "let g = o ?? nop\n    g(1)",
        "let g = o.unwrap_or(nop)\n    g(1)",
        "match o {\n        Some(g) -> g(1)\n        None -> nop(1)\n    }",
    ] {
        check_err(
            &format!(
                "{head}fn pure(o: Option[fn(i64) -> () / {{io}}]) / {{}} {{\n    {body}\n}}\n\n\
fn main() {{\n    pure(Some(|x| print(\"{{x}}\")))\n}}\n"
            ),
            "not in its declared row",
        );
    }
    // And through a parallel operation, where the row comes from a `let`
    // annotation rather than from the signature.
    check_err(
        "fn nop(x: i64) -> i64 {\n    x\n}\n\nfn main() {\n    \
let o: Option[fn(i64) -> i64 / {io}] = Some(|x| {\n        print(\"{x}\")\n        x\n    })\n    \
let g = o ?? nop\n    print(\"{[1, 2].par_map(g)}\")\n}\n",
        "effect-free function",
    );
    // A pure option coalesced with a pure default is charged nothing.
    check_ok(
        "fn dbl(n: i64) -> i64 {\n    n * 2\n}\n\n\
fn pure(o: Option[fn(i64) -> i64]) -> i64 / {} {\n    let g = o ?? dbl\n    g(3)\n}\n\n\
fn main() {\n    print(\"{pure(None)}\")\n}\n",
    );
}

/// A local spelled like a top-level function is a different thing — scope
/// decides, at a *call* as well as at a mention ([T-FN-VALUE-ROW]).
///
/// The mention arm checked shadowing and the call arm did not, so a parameter
/// named after a private helper was charged that helper's row: a
/// `capabilities = []` package broke in every consumer, and `par_map`'s
/// purity proof failed, over one parameter's spelling.
#[test]
fn a_local_spelled_like_a_function_is_not_that_function() {
    let head = "fn shout(n: i64) -> i64 {\n    print(\"shout {n}\")\n    n\n}\n\n";
    check_ok(&format!(
        "{head}fn quiet(shout: fn(i64) -> i64) -> i64 / {{}} {{\n    shout(1)\n}}\n\n\
fn main() {{\n    print(\"{{quiet(|x| x + 1)}}\")\n}}\n"
    ));
    check_ok(&format!(
        "{head}fn twice(shout: fn(i64) -> i64) -> [i64] {{\n    [1, 2].par_map(shout)\n}}\n\n\
fn main() {{\n    print(\"{{twice(|x| x * 2)}}\")\n}}\n"
    ));
    // The real function is still charged, called and named.
    check_err(
        &format!("{head}fn quiet() -> i64 / {{}} {{\n    shout(1)\n}}\n\nfn main() {{\n    print(\"{{quiet()}}\")\n}}\n"),
        "not in its declared row",
    );
    check_err(
        &format!("{head}fn main() {{\n    print(\"{{[1, 2].par_map(shout)}}\")\n}}\n"),
        "effect-free function",
    );
}

/// Reading an element out of a known binding hands on *that* binding's
/// sources, not every function value in the body.
///
/// `let g = fs[0]` fell through to the whole-body pool, so an unrelated
/// closure elsewhere charged its effect to a call through `g` — while the same
/// program with `for g in fs` was accepted, which is the pair
/// [T-FN-TYPE-ROW-CARRIED] names as the defect it was written to close.
#[test]
fn an_element_read_carries_only_its_own_bindings_sources() {
    check_ok(
        "effect Cfg {\n    fn value() -> i64\n}\n\nfn main() {\n    \
with Cfg handled_by CfgHandler{ value: || 21 } {\n        \
let cf = |x| x + Cfg.value()\n        print(\"{cf(0)}\")\n    }\n    \
let fs: [fn(i64) -> i64] = [|x| x + 1]\n    let g = fs[0]\n    print(\"{g(1)}\")\n}\n",
    );
    // A list that really does hold an effectful function is still charged.
    let head = "fn shout(n: i64) -> i64 {\n    print(\"x\")\n    n\n}\n\n";
    check_err(
        &format!("{head}fn quiet() -> i64 / {{}} {{\n    let fs = [shout]\n    let g = fs[0]\n    g(1)\n}}\n\nfn main() {{\n    print(\"{{quiet()}}\")\n}}\n"),
        "not in its declared row",
    );
    check_err(
        &format!(
            "{head}fn quiet(fs: [fn(i64) -> i64 / {{io}}]) -> i64 / {{}} {{\n    \
let g = fs[0]\n    g(1)\n}}\n\nfn main() {{\n    print(\"{{quiet([shout])}}\")\n}}\n"
        ),
        "not in its declared row",
    );
}

/// A closure's body is checked against the written result type, the same way
/// a named function's is ([T-FN-TYPE-ROW], [T-INT-TO-FLOAT-CARRIED]).
///
/// Inferred instead, the expected type never reached the literals in the
/// tail: `|x| 3` against `fn(i64) -> f64` was refused while
/// `fn named(x: i64) -> f64 { 3 }` — the same body, one closure apart — was
/// accepted, and so was `|x| { return 3 }`.
#[test]
fn a_closures_tail_adopts_its_written_result_type() {
    check_ok(
        "type E = | Bad\ntype Box = { f: fn(i64) -> f64 }\n\n\
fn take(f: fn(i64) -> f64) -> f64 {\n    f(1)\n}\n\nfn main() {\n    \
let a: fn(i64) -> f64 = |x| 3\n    let b = Box{ f: |x| 3 }\n    \
let cs: [fn(i64) -> f64] = [|x| 3]\n    let d: fn(i64) -> [f64] = |x| [1, 2]\n    \
let e: fn(i64) -> Option[f64] = |x| Some(3)\n    let g: fn() -> f64 = || 3\n    \
let h: fn(i64) -> f64 ! E = |x| 3\n    \
print(\"{a(0)} {b.f(0)} {cs[0](0)} {d(0)} {e(0) ?? -1.0} {g()} {h(0) ?? -1.0}\")\n    \
print(\"{take(|x| if x == 1 { 3 } else { 2.5 })}\")\n}\n",
    );
    // A genuine mismatch is still refused.
    check_err(
        "fn take(f: fn(i64) -> f64) -> f64 {\n    f(1)\n}\n\n\
fn main() {\n    print(\"{take(|x| \"oops\")}\")\n}\n",
        "type mismatch",
    );
}

/// An `if` merges an integer literal against a float branch, the way a
/// `match` already does ([T-INT-TO-FLOAT-CARRIED]).
///
/// With no expected type the branches have only each other, and the literal
/// is the one that adopts — but only a `match`'s arms did, so a two-arm
/// `match` on a `bool` and the `if` that says the same thing disagreed.
#[test]
fn an_if_merges_a_literal_branch_against_a_float() {
    check_ok(
        "fn main() {\n    let y = if true {\n        3\n    } else {\n        2.5\n    }\n    \
let z = match 1 {\n        1 -> 3\n        _ -> 2.5\n    }\n    print(\"{y} {z}\")\n}\n",
    );
    // Either side.
    check_ok(
        "fn main() {\n    let y = if true {\n        2.5\n    } else {\n        3\n    }\n    \
print(\"{y}\")\n}\n",
    );
    // A genuinely different type is still a mismatch.
    check_err(
        "fn main() {\n    let y = if true {\n        3\n    } else {\n        \"x\"\n    }\n    \
print(\"{y}\")\n}\n",
        "different types",
    );
}

/// `??` carries a row whatever its left operand is, and wherever the result
/// goes ([T-FN-VALUE-ROW], [T-FN-TYPE-ROW-CARRIED]).
///
/// Only a bare identifier on the left was marked as unaccounted, so a field
/// read or a method call there left the pure default on the right looking like
/// the whole story — and a `??` handed straight to `map` or `par_map`, without
/// a `let` in between, was not looked at as a function value at all.
#[test]
fn a_coalesce_carries_its_row_from_every_operand() {
    let head = "fn shout(n: i64) -> i64 {\n    print(\"shout {n}\")\n    n\n}\n\n\
fn dflt(n: i64) -> i64 {\n    n\n}\n\n";
    // A field read on the left.
    check_err(
        &format!(
            "{head}type Bx = {{ opt: Option[fn(i64) -> i64 / {{io}}] }}\n\n\
fn pure(b: Bx) / {{}} {{\n    let f = b.opt ?? dflt\n    let r = f(1)\n}}\n\n\
fn main() {{\n    pure(Bx{{ opt: Some(shout) }})\n}}\n"
        ),
        "not in its declared row",
    );
    // Straight into a higher-order method, with no binding in between.
    for m in ["map", "par_map"] {
        check_err(
            &format!(
                "{head}fn pure(o: Option[fn(i64) -> i64 / {{io}}]) / {{}} {{\n    \
let r = [1, 2].{m}(o ?? dflt)\n}}\n\nfn main() {{\n    pure(Some(shout))\n}}\n"
            ),
            "performs",
        );
    }
    // A pure option coalesced with a pure default, through a field, is fine.
    check_ok(
        "fn dflt(n: i64) -> i64 {\n    n\n}\n\ntype Bx = { opt: Option[fn(i64) -> i64] }\n\n\
fn pure(b: Bx) -> i64 / {} {\n    let f = b.opt ?? dflt\n    f(1)\n}\n\n\
fn main() {\n    print(\"{pure(Bx{ opt: None })}\")\n}\n",
    );
}

/// A branch that gives the bare ok half is wrapped where it lands
/// ([T-OK-ERR-RETURN]: "the branches giving only the ok half are the ones
/// wrapped").
///
/// A fallible *function* re-wraps on the way out, which masked this everywhere
/// except the positions that do not — a `let` annotation, a struct field, a
/// list element — where the interpreter stored the raw value. The engines then
/// disagreed, and the normative one was wrong: `let a: i64 ! E = if true { 1 }
/// else { Ok(2) }` printed `1`, compared unequal to `Ok(1)`, and matched no arm
/// of a `match` over it.
#[test]
fn a_bare_ok_half_is_wrapped_where_it_lands() {
    let e = "type E = | Bad\n";
    check_ok(&format!(
        "{e}fn main() {{\n    let a: i64 ! E = if true {{ 1 }} else {{ Ok(2) }}\n    \
match a {{\n        Ok(n) -> print(\"Ok({{n}})\")\n        Err(x) -> print(\"Err\")\n    }}\n}}\n"
    ));
    check_ok(&format!(
        "{e}fn main() {{\n    let b: i64 ! E = match 1 {{\n        1 -> 7\n        _ -> Ok(8)\n    }}\n    \
match b {{\n        Ok(n) -> print(\"Ok({{n}})\")\n        Err(x) -> print(\"Err\")\n    }}\n}}\n"
    ));
    // A binary tail: the wrap belongs to the whole branch, not to its leftmost
    // leaf, which begins at the same offset.
    check_ok(&format!(
        "{e}fn f(n: i64) -> i64 ! E {{\n    let v: i64 ! E = if n == 0 {{\n        fail(Bad)\n    }} else {{\n        0 - 7\n    }}\n    v\n}}\n\n\
fn main() {{\n    print(\"{{f(1) ?? 0 - 1}}\")\n}}\n"
    ));
}

/// A mixing `if`/`match` merges to the whole `T ! E` however deeply it is
/// nested ([T-OK-ERR-RETURN]: "a rule about the *position*, not about how it
/// was reached").
///
/// The expected type reached only the *immediate* expression at the fallible
/// position, so the same computation was accepted written
/// `match a { true -> if b { 1 } else { Ok(2) } … }` and refused written
/// `if a { if b { 1 } else { Ok(2) } } else { Ok(3) }` — one keyword apart.
#[test]
fn a_mixing_branch_merges_however_it_is_nested() {
    let e = "type E = | Bad\n";
    for body in [
        "if a {\n        if b { 1 } else { Ok(2) }\n    } else {\n        Ok(3)\n    }",
        "if a {\n        match b {\n            true -> 1\n            false -> Ok(2)\n        }\n    } else {\n        Ok(3)\n    }",
        "match a {\n        true -> match b {\n            true -> 1\n            false -> Ok(2)\n        }\n        false -> Ok(3)\n    }",
        "match a {\n        true -> if b { 1 } else { Ok(2) }\n        false -> Ok(3)\n    }",
        "{\n        match b {\n            true -> 1\n            false -> Ok(2)\n        }\n    }",
    ] {
        check_ok(&format!(
            "{e}fn f(a: bool, b: bool) -> i64 ! E {{\n    {body}\n}}\n\n\
fn main() {{\n    print(\"{{f(true, true) ?? 0 - 1}}\")\n}}\n"
        ));
    }
}

/// A `let` copy of a parameter hands on *that* parameter's row, not a
/// sibling's.
///
/// [T-FN-TYPE-ROW-CARRIED] scopes the fallback to positions "with no field
/// name", and a plain named parameter is not one — but `let f = step` fell
/// through to it, so an unrelated `log: fn(str) / {io}` in the same signature
/// charged `io` to a call through `step`. Writing `xs.map(step)` was accepted:
/// the same program one `let` apart, which is the pair this clause exists to
/// close.
#[test]
fn a_let_copy_of_a_parameter_keeps_that_parameters_row() {
    let tail = "fn shout(m: str) {\n    print(m)\n}\n\nfn dbl(n: i64) -> i64 {\n    n * 2\n}\n\n\
fn main() {\n    print(\"{run(shout, dbl, [1, 2, 3])}\")\n}\n";
    for body in [
        "let f = step\n    xs.map(f)",
        "xs.map(step)",
        "let f = step\n    xs.par_map(f)",
    ] {
        check_ok(&format!(
            "fn run(log: fn(str) / {{io}}, step: fn(i64) -> i64, xs: [i64]) -> [i64] / {{}} {{\n    \
{body}\n}}\n\n{tail}"
        ));
    }
    // An effectful parameter copied the same way is still charged.
    check_err(
        "fn run(step: fn(i64) -> i64 / {io}, xs: [i64]) -> [i64] / {} {\n    \
let f = step\n    xs.map(f)\n}\n\nfn shout(n: i64) -> i64 {\n    print(\"x\")\n    n\n}\n\n\
fn main() {\n    print(\"{run(shout, [1])}\")\n}\n",
        "not in its declared row",
    );
}

/// A block hands its tail through, so `par_map`'s purity proof sees the same
/// function `let f = loud` would give.
///
/// [T-PAR-MAP-PURE] calls an effectful function at a parallel operation "a
/// compile error, not a race" — but `let f = { loud }` recorded no source at
/// all, so the proof saw an empty row and `grasm run` really did `print` from
/// inside `par_map`. One pair of braces from the spelling that was refused.
/// `unsafe { sqrt }` is the same shape, and it is how a declared C function
/// got onto the workers past [T-EXTERN-VALUE].
#[test]
fn a_block_hands_its_tail_to_the_purity_proof() {
    let fns = "fn loud(n: i64) -> i64 {\n    print(\"{n}\")\n    n\n}\n\n\
fn dbl(n: i64) -> i64 {\n    n * 2\n}\n\n";
    for op in ["par_map", "par_filter"] {
        for held in ["{ loud }", "{ { loud } }", "unsafe { loud }"] {
            check_err(
                &format!(
                    "{fns}fn main() {{\n    let f = {held}\n    \
let out = [1, 2].{op}(f)\n}}\n"
                ),
                "needs an effect-free function",
            );
            check_err(
                &format!("{fns}fn main() {{\n    let out = [1, 2].{op}({held})\n}}\n"),
                "needs an effect-free function",
            );
        }
        // A pure one held the same way is still accepted. `par_filter`
        // wants a predicate, so each takes the function of its own type.
        let pure = if op == "par_filter" { "keep" } else { "dbl" };
        check_ok(&format!(
            "{fns}fn keep(n: i64) -> bool {{\n    n > 0\n}}\n\n\
fn main() {{\n    let f = {{ {pure} }}\n    \
print(\"{{[1, 2].{op}(f)}}\")\n}}\n"
        ));
    }
    // A C function named through a block carries the `io` a call does.
    check_err(
        "extern fn sqrt(x: f64) -> f64\n\nfn main() {\n    let f = unsafe { sqrt }\n    \
let out = [16.0].par_map(f)\n}\n",
        "needs an effect-free function",
    );
}

/// A C function called in method position is the same call, and carries the
/// same row.
///
/// [T-UNSAFE-UFCS] gated the `unsafe` half of `x.f()` and left the `io` half
/// behind, so `n.labs()` charged nothing while `labs(n)` charged `io` — and a
/// package declaring `capabilities = ["unsafe"]` was reported as reaching no
/// `io` while its binary called libc.
#[test]
fn a_c_function_in_method_position_charges_the_same_row() {
    let ext = "extern fn labs(x: i64) -> i64\n\n";
    for call in ["labs(n)", "n.labs()"] {
        check_err(
            &format!(
                "{ext}fn quiet(n: i64) -> i64 / {{unsafe}} {{\n    unsafe {{\n        \
{call}\n    }}\n}}\n\nfn main() {{\n    print(\"{{quiet(0 - 5)}}\")\n}}\n"
            ),
            "not in its declared row",
        );
        check_ok(&format!(
            "{ext}fn loud(n: i64) -> i64 / {{io, unsafe}} {{\n    unsafe {{\n        \
{call}\n    }}\n}}\n\nfn main() {{\n    print(\"{{loud(0 - 5)}}\")\n}}\n"
        ));
    }
}

/// Reading a part out of a binding hands on *that* binding's row, including
/// when the row it carries is empty.
///
/// [T-FN-TYPE-ROW-CARRIED] scopes the fallback pool to positions "with no
/// field name", but an empty carried row was not recorded at all, so
/// `let f = fs[0]` for a pure `fs` looked unaccounted and drew the whole
/// signature pool — and an unrelated parameter whose *named field* holds an
/// `io` function refused a `/ {}` body that performs nothing.
#[test]
fn a_pure_carrier_is_not_charged_a_siblings_row() {
    let decls = "type Loud = { g: fn(i64) -> i64 / {io} }\n\n\
fn inc(n: i64) -> i64 {\n    n + 1\n}\n\n";
    let tail = "\nfn main() {\n    print(\"{go([inc], Loud{ g: inc })}\")\n}\n";
    for body in [
        "let f = fs[0]\n    f(1)",
        "let f = fs[0]\n    [1].map(f)[0]",
        "var t = 0\n    for f in fs {\n        t = t + f(1)\n    }\n    t",
    ] {
        check_ok(&format!(
            "{decls}fn go(fs: [fn(i64) -> i64], l: Loud) -> i64 / {{}} {{\n    {body}\n}}\n{tail}"
        ));
    }
    // The effectful carrier is still charged, through each of the same shapes.
    let loud = "fn shout(n: i64) -> i64 {\n    print(\"{n}\")\n    n\n}\n\n";
    for body in [
        "let f = fs[0]\n    f(1)",
        "let f = fs[0]\n    [1].map(f)[0]",
        "var t = 0\n    for f in fs {\n        t = t + f(1)\n    }\n    t",
    ] {
        check_err(
            &format!(
                "{loud}fn go(fs: [fn(i64) -> i64 / {{io}}]) -> i64 / {{}} {{\n    {body}\n}}\n\
\nfn main() {{\n    print(\"{{go([shout])}}\")\n}}\n"
            ),
            "not in its declared row",
        );
    }
}

/// The help for a write to an immutable name says something that parses.
///
/// A parameter cannot be `var` — the grammar has no such parameter — so
/// "declare it with `var n` instead of `let`" produced `fn f(var n: i64)`,
/// which is a syntax error, and left `inout` and the local copy unsaid. A
/// property test's parameter has no caller to write back to at all.
#[test]
fn the_immutability_help_names_a_fix_that_parses() {
    check_err(
        "fn f(n: i64) -> i64 {\n    n = 5\n    n\n}\n\nfn main() {\n    print(\"{f(1)}\")\n}\n",
        "cannot assign to `n`",
    );
    let (_, diags) = analyze(
        "fn f(n: i64) -> i64 {\n    n = 5\n    n\n}\n\nfn main() {\n    print(\"{f(1)}\")\n}\n",
    );
    let help = diags[0].help.clone().expect("the write has help");
    assert!(
        help.contains("inout n") && help.contains("var n = n"),
        "{help}"
    );
    // Following it compiles.
    check_ok(
        "fn f(inout n: i64) {\n    n = 5\n}\n\nfn main() {\n    var x = 1\n    \
f(x)\n    print(\"{x}\")\n}\n",
    );
    // A local still gets the `var` advice, which is right for a local.
    let (_, diags) = analyze("fn main() {\n    let a = 1\n    a = 2\n    print(\"{a}\")\n}\n");
    assert!(
        diags[0]
            .help
            .as_deref()
            .is_some_and(|h| h.contains("var a")),
        "{:?}",
        diags[0].help
    );
}

/// Where every part of what was read is a binding this pass knows, the
/// fallback pool is the wrong answer.
///
/// [T-FN-TYPE-ROW-CARRIED] scopes the pool to rows written "at a position with
/// no field name", and reaches for it only where nothing is known about the
/// part. A tuple component and a `??` between two known carriers drew it
/// anyway, so a `/ {}` body performing nothing was refused on the strength of
/// an unrelated parameter in the same signature — including a parameter whose
/// own row is exactly accounted for at every mention of it.
#[test]
fn a_known_carrier_is_read_by_its_own_type() {
    let decls = "type Cb = fn(i64) -> i64\ntype Loud = fn(i64) -> i64 / {io}\n\n\
fn inc(n: i64) -> i64 {\n    n + 1\n}\n\n";
    let sig = "fn go(fs: [Cb], o: Option[Cb], t: (Cb, i64), l: Loud) -> i64 / {} {\n    ";
    let tail = "\n}\n\nfn main() {\n    print(\"{go([inc], Some(inc), (inc, 1), inc)}\")\n}\n";
    for body in [
        "let a = fs[0]\n    a(1)",
        "let d = t.0\n    d(1)",
        "t.0(1)",
        "let b = o ?? fs[0]\n    b(1)",
        "let b = o ?? t.0\n    b(1)",
        "(o ?? t.0)(1)",
        "let c = { fs[0] }\n    c(1)",
    ] {
        check_ok(&format!("{decls}{sig}{body}{tail}"));
    }
    // A declared type that holds no row says so, rather than saying nothing:
    // the program-wide field table keys `f` across every type that has one,
    // so a pure `B { f: ... }` was charged an unrelated `A { f: ... / {io} }`.
    check_ok(
        "type A = { f: fn(i64) -> i64 / {io} }\ntype B = { f: fn(i64) -> i64 }\n\n\
fn inc(n: i64) -> i64 {\n    n + 1\n}\n\n\
fn ok(b: B) -> i64 / {} {\n    b.f(1)\n}\n\n\
fn main() {\n    print(\"{ok(B{ f: inc })}\")\n}\n",
    );
    // …and the type that does hold one is still charged, through a generic
    // carrier and a sum payload as well as directly.
    for (decls, sig, body, arg) in [
        (
            "type A = { f: fn(i64) -> i64 / {io} }\n",
            "fn go(a: A) -> i64 / {} {",
            "a.f(1)",
            "A{ f: shout }",
        ),
        (
            "type Cb = fn(i64) -> i64 / {io}\ntype Box[T] = { v: T }\n",
            "fn go(b: Box[Cb]) -> i64 / {} {",
            "b.v(1)",
            "Box{ v: shout }",
        ),
        (
            "type V = | Hold(f: fn(i64) -> i64 / {io}) | Nil\n",
            "fn go(v: V) -> i64 / {} {",
            "match v {\n        Hold(f) -> f(1)\n        Nil -> 0\n    }",
            "Hold(shout)",
        ),
    ] {
        check_err(
            &format!(
                "{decls}\nfn shout(n: i64) -> i64 {{\n    print(\"{{n}}\")\n    n\n}}\n\n\
{sig}\n    {body}\n}}\n\nfn main() {{\n    print(\"{{go({arg})}}\")\n}}\n"
            ),
            "not in its declared row",
        );
    }

    // The same reads through an effectful carrier are still charged.
    let loud = "type Cb = fn(i64) -> i64 / {io}\n\n\
fn shout(n: i64) -> i64 {\n    print(\"{n}\")\n    n\n}\n\n";
    let lsig = "fn go(fs: [Cb], o: Option[Cb], t: (Cb, i64)) -> i64 / {} {\n    ";
    let ltail = "\n}\n\nfn main() {\n    print(\"{go([shout], Some(shout), (shout, 1))}\")\n}\n";
    for body in [
        "let a = fs[0]\n    a(1)",
        "let d = t.0\n    d(1)",
        "t.0(1)",
        "let b = o ?? fs[0]\n    b(1)",
        "(o ?? t.0)(1)",
        "let c = { fs[0] }\n    c(1)",
    ] {
        check_err(
            &format!("{loud}{lsig}{body}{ltail}"),
            "not in its declared row",
        );
    }
}

/// An effect operation's declared *result* carries its row.
///
/// [E-HANDLER-OWN-ROW] exempts the handler's own operations from the
/// installing function's row — not the value an operation hands back. An
/// operation answering `-> fn(i64) -> i64 / {io}` gave out a printing function
/// that nothing charged, so a function declaring `/ {Mk}` printed, `par_map`'s
/// purity proof passed, and a package permitting only `Mk` reached `io`. The
/// same shape written as an ordinary `fn` was refused.
#[test]
fn an_operation_result_carries_its_row() {
    let handler = "\nfn main() {\n    with Mk handled_by MkHandler{ get: || noisy } {\n        \
print(\"{quiet()}\")\n    }\n}\n\nfn noisy(x: i64) -> i64 {\n    print(\"{x}\")\n    x\n}\n";
    for (ret, body) in [
        ("fn(i64) -> i64 / {io}", "let f = Mk.get()\n    f(4)"),
        ("fn(i64) -> i64 / {io}", "Mk.get()(4)"),
        ("[fn(i64) -> i64 / {io}]", "let fs = Mk.get()\n    fs[0](4)"),
        (
            "Option[fn(i64) -> i64 / {io}]",
            "let f = Mk.get() ?? noisy\n    f(4)",
        ),
    ] {
        check_err(
            &format!(
                "effect Mk {{\n    fn get() -> {ret}\n}}\n\n\
fn quiet() -> i64 / {{Mk}} {{\n    {body}\n}}\n{handler}"
            ),
            "not in its declared row",
        );
    }
    // The parallel proof sees it too, through a name and written at the call.
    for body in [
        "let f = Mk.get()\n    [1, 2].par_map(f)",
        "[1, 2].par_map(Mk.get())",
    ] {
        check_err(
            &format!(
                "effect Mk {{\n    fn get() -> fn(i64) -> i64 / {{io}}\n}}\n\n\
fn quiet() -> [i64] / {{Mk}} {{\n    {body}\n}}\n\nfn main() {{\n    \
with Mk handled_by MkHandler{{ get: || |x| x }} {{\n        print(\"{{quiet()}}\")\n    }}\n}}\n"
            ),
            "performs",
        );
    }
    // An operation answering a *pure* function type is not charged.
    check_ok(
        "effect Mk {\n    fn get() -> fn(i64) -> i64\n}\n\n\
fn quiet() -> i64 / {Mk} {\n    let f = Mk.get()\n    f(4)\n}\n\nfn main() {\n    \
with Mk handled_by MkHandler{ get: || |x| x * 2 } {\n        print(\"{quiet()}\")\n    }\n}\n",
    );
}

/// Scope decides for a module alias and an effect name, as it already does for
/// a function name.
///
/// [T-FN-VALUE-ROW] says so in as many words — "scope decides, here as
/// everywhere else" — but only the function-name arm honoured it. A binding
/// spelled like an imported module made `binding.field` take the *module
/// function's* row: an `io` field looked pure, so a printing function reached
/// `par_map` and a `capabilities = []` package; and a pure field was charged
/// an unreachable private helper's `io`, so a package performing nothing
/// failed to build. A parameter spelled like an effect demanded a handler for
/// an effect the body never performs.
#[test]
fn scope_decides_for_a_module_alias_and_an_effect_name() {
    // The effect name, shadowed by a parameter: no handler is demanded.
    check_ok(
        "effect Clock {\n    fn now() -> i64\n}\n\ntype C = { now: fn() -> i64 }\n\n\
fn quiet(Clock: C) -> i64 {\n    Clock.now()\n}\n\nfn main() {\n    \
print(\"{quiet(C{ now: || 42 })}\")\n}\n",
    );
    // …and the unshadowed spelling still is.
    check_err(
        "effect Clock {\n    fn now() -> i64\n}\n\nfn quiet() -> i64 {\n    Clock.now()\n}\n\n\
fn main() {\n    print(\"{quiet()}\")\n}\n",
        "unhandled effect",
    );
    // A local spelled like a same-file type's field, holding an `io` function,
    // is charged by the field's written row.
    check_err(
        "type P = { shout: fn(i64) -> i64 / {io} }\n\n\
fn noisy(n: i64) -> i64 {\n    print(\"{n}\")\n    n\n}\n\n\
fn quiet() -> [i64] / {} {\n    let p = P{ shout: noisy }\n    [1, 2].par_map(p.shout)\n}\n\n\
fn main() {\n    print(\"{quiet()}\")\n}\n",
        "effect-free function",
    );
}

/// A binder the effect pass does not model is still charged correctly,
/// because the row comes from the type rather than from the binder.
///
/// The pass used to guess where a function value came from, and reached for a
/// fallback pool wherever it could not: a `match` payload, a tuple
/// destructuring and a closure parameter were charged every row written
/// anywhere in the enclosing signature. So a `/ {}` body performing nothing
/// was refused because an unrelated parameter of the same function declared
/// `io`. The checker already carries a function's row in its type and
/// propagates it through every carrier by unification; asking it is one
/// answer instead of two.
#[test]
fn a_binder_the_effect_pass_does_not_model_is_charged_by_type() {
    let pure = "type Cb = fn(i64) -> i64\ntype Loud = fn(i64) -> i64 / {io}\n\
type V = | Hold(f: Cb) | Nil\n\nfn inc(n: i64) -> i64 {\n    n + 1\n}\n\n";
    for (sig, body, arg) in [
        (
            "fn go(v: V, l: Loud) -> i64 / {} {",
            "match v {\n        Hold(f) -> f(1)\n        Nil -> 0\n    }",
            "Hold(inc), inc",
        ),
        (
            "fn go(p: (Cb, i64), l: Loud) -> i64 / {} {",
            "let (f, n) = p\n    f(n)",
            "(inc, 1), inc",
        ),
        (
            "fn go(fs: [Cb], l: Loud) -> i64 / {} {",
            "var t = 0\n    for f in fs {\n        t = t + f(1)\n    }\n    t",
            "[inc], inc",
        ),
    ] {
        check_ok(&format!(
            "{pure}{sig}\n    {body}\n}}\n\nfn main() {{\n    print(\"{{go({arg})}}\")\n}}\n"
        ));
    }
    // The same binders carrying an effectful value are still charged.
    let loud = "type Cb = fn(i64) -> i64 / {io}\ntype V = | Hold(f: Cb) | Nil\n\n\
fn shout(n: i64) -> i64 {\n    print(\"{n}\")\n    n\n}\n\n";
    for (sig, body, arg) in [
        (
            "fn go(v: V) -> i64 / {} {",
            "match v {\n        Hold(f) -> f(1)\n        Nil -> 0\n    }",
            "Hold(shout)",
        ),
        (
            "fn go(p: (Cb, i64)) -> i64 / {} {",
            "let (f, n) = p\n    f(n)",
            "(shout, 1)",
        ),
        (
            "fn go(fs: [Cb]) -> i64 / {} {",
            "var t = 0\n    for f in fs {\n        t = t + f(1)\n    }\n    t",
            "[shout]",
        ),
    ] {
        check_err(
            &format!(
                "{loud}{sig}\n    {body}\n}}\n\nfn main() {{\n    print(\"{{go({arg})}}\")\n}}\n"
            ),
            "not in its declared row",
        );
    }
}

/// A built-in free function is not a method, and the refusal says which
/// spelling does work.
///
/// [L-UFCS] resolves method syntax against the receiver type's methods and
/// then against functions a *program* declares. `print` is a method of
/// nothing — but the help offered "a free function `fn print(str, ...)` would
/// also work here (UFCS)", which is exactly the call it had just refused.
/// Following help that cannot work is worse than no help.
#[test]
fn a_builtin_is_not_offered_as_a_method() {
    for (name, call) in [
        ("print", "\"hi\".print()"),
        ("read_file", "\"p\".read_file()"),
        ("panic", "\"boom\".panic()"),
        ("exit", "1.exit()"),
    ] {
        let (_, diags) = analyze(&format!("fn main() {{\n    let x = {call}\n}}\n"));
        let d = &diags[0];
        assert!(
            d.message.contains("built-in function, not a method"),
            "{call}: {}",
            d.message
        );
        let help = d.help.clone().unwrap_or_default();
        assert!(help.contains(&format!("|> {name}")), "{call}: {help}");
        assert!(!help.contains("UFCS"), "{call}: {help}");
    }
    // The pipeline the help names compiles and runs.
    check_ok("fn main() {\n    \"hi\" |> print\n}\n");
    // A function the *program* declares is still reachable in method position.
    check_ok(
        "fn double(n: i64) -> i64 {\n    n * 2\n}\n\nfn main() {\n    \
print(\"{5.double()}\")\n}\n",
    );
}

/// A generic does not launder a `ref` into a parallel operation.
///
/// [T-PAR-MAP-PURE] refuses an element type that reaches a reference, but a
/// type *parameter* says nothing at the operation — `fn go[T](xs: [T], f:
/// fn(T) -> i64) { xs.par_map(f) }` hid `[ref Acc]` from the check entirely,
/// which is the door back to the race the clause describes. Settled at the
/// call sites, exactly as [T-MAP-KEY-GENERIC] settles map keys, and through a
/// generic that forwards to a generic.
#[test]
fn a_generic_does_not_launder_a_ref_into_a_parallel_operation() {
    let base = "type Acc = { n: i64 }\n\n\
fn hit(q: ref Acc) -> i64 {\n    q.n\n}\n\nfn dbl(n: i64) -> i64 {\n    n * 2\n}\n\n";
    let one = "fn go[T](xs: [T], f: fn(T) -> i64) -> [i64] {\n    xs.par_map(f)\n}\n\n";
    // Forwarding one hop further still lands the constraint on the caller.
    let two = "fn go[T](xs: [T], f: fn(T) -> i64) -> [i64] {\n    xs.par_map(f)\n}\n\n\
fn outer[T](xs: [T], f: fn(T) -> i64) -> [i64] {\n    go(xs, f)\n}\n\n";
    for (decls, call) in [(one, "go"), (two, "outer")] {
        check_err(
            &format!(
                "{base}{decls}fn main() {{\n    let r = ref(Acc{{ n: 0 }})\n    \
let rs = [r]\n    print(\"{{{call}(rs, hit).len()}}\")\n}}\n"
            ),
            "reaches a `ref`",
        );
        // The same generic at a type that holds none is untouched.
        check_ok(&format!(
            "{base}{decls}fn main() {{\n    print(\"{{{call}([1, 2], dbl)}}\")\n}}\n"
        ));
    }
}

/// The `ref`-in-parallel rule refuses the shared shapes and nothing else.
///
/// Three refusals were added at once — an element type reaching a reference,
/// a mention of an outside reference, and a generic carrying one in — and a
/// rule that also refuses correct programs costs a user a workaround for a
/// race they did not write. Sequential `map`, `filter` and `fold` may still
/// write through a captured reference ([T-REF-CAPTURE]); a reference created
/// *inside* a parallel function is one per element; and a reference merely in
/// scope constrains nothing.
#[test]
fn the_parallel_ref_rule_refuses_only_what_is_shared() {
    let base = "type Acc = { n: i64 }\ntype Pair = { a: i64, b: str }\n\n";
    for body in [
        // Sequential combinators, writing through a captured reference.
        "let acc = ref(Acc{ n: 0 })\n    let ys = [1, 2].map(|x| {\n        \
acc.n = acc.n + x\n        x\n    })\n    print(\"{ys} {acc.n}\")",
        "let acc = ref(Acc{ n: 1 })\n    let zs = [1, 2].filter(|x| acc.n > 0)\n    \
print(\"{zs}\")",
        "let acc = ref(Acc{ n: 1 })\n    let s = [1, 2].fold(0, |a, b| a + b + acc.n)\n    \
print(\"{s}\")",
        // A reference created inside the parallel function: one per element.
        "let ps = [1, 2].par_map(|x| {\n        let mine = ref(Acc{ n: x })\n        \
mine.n = mine.n * 2\n        mine.n\n    })\n    print(\"{ps}\")",
        // One merely in scope.
        "let acc = ref(Acc{ n: 5 })\n    let qs = [1, 2].par_map(|x| x * 2)\n    \
print(\"{qs} {acc.n}\")",
        // Element types that hold no reference, through every carrier.
        "let ps = [Pair{ a: 1, b: \"x\" }].par_map(|p| p.a * 10)\n    print(\"{ps}\")",
        "let ls = [[1], [2]].par_map(|l| l.sum())\n    print(\"{ls}\")",
        "let os: [Option[i64]] = [Some(1), None]\n    \
print(\"{os.par_map(|o| o ?? 0)}\")",
        "let ts = [(1, \"a\")].par_map(|t| t.0)\n    print(\"{ts}\")",
    ] {
        check_ok(&format!("{base}fn main() {{\n    {body}\n}}\n"));
    }
    // A generic whose parallel parameter is instantiated at a plain type.
    check_ok(
        "fn go[T](xs: [T]) -> [i64] {\n    xs.par_map(|x| 1)\n}\n\nfn main() {\n    \
print(\"{go([1, 2])}\")\n    print(\"{go([\"a\"])}\")\n}\n",
    );
    // And every carrier that *does* reach one is refused — the refusal walks
    // the whole type, not just its outermost shape.
    let decls = "type Acc = { n: i64 }\ntype V = | Hold(r: ref Acc) | Nil\ntype E = | Bad\n\
type W = { inner: Option[ref Acc] }\n\n";
    for build in [
        "let vs = [Hold(acc), Nil]\n    print(\"{vs.par_map(|v| 1)}\")",
        "let fs: [ref Acc ! E] = [Ok(acc)]\n    print(\"{fs.par_map(|f| 1)}\")",
        "let ms: [Map[str, ref Acc]] = [{\"a\": acc}]\n    print(\"{ms.par_map(|m| 1)}\")",
        "let ws = [W{ inner: Some(acc) }]\n    print(\"{ws.par_map(|w| 1)}\")",
        "let ts = [(acc, 1)]\n    print(\"{ts.par_map(|t| 1)}\")",
        "let ls = [[acc]]\n    print(\"{ls.par_map(|l| 1)}\")",
    ] {
        check_err(
            &format!("{decls}fn main() {{\n    let acc = ref(Acc{{ n: 1 }})\n    {build}\n}}\n"),
            "reaches a `ref`",
        );
    }
}

/// A closure bound to a name carries its `ref` with it.
///
/// The last shape [T-PAR-MAP-PURE] listed as reaching past the rule: the
/// purity proof looks at the function written *at* the operation, and a name
/// is not one. It was held closed only by the native backend refusing
/// `par_map` with a name-bound closure — the same "held closed by a gap
/// elsewhere" argument that stopped being true the day the backend started
/// compiling writes through a reference.
///
/// Settled at the end of the analysis, because a closure whose parameter types
/// nothing fixes has its body checked after the `let` that binds it — and
/// after the call that fixed them, which may be the parallel operation itself.
#[test]
fn a_name_bound_closure_carries_its_ref_into_a_parallel_operation() {
    let base = "type Acc = { n: i64 }\n\n";
    for (bind, op) in [
        (
            "let f = |x| {\n        acc.n = acc.n + x\n        x\n    }",
            "par_map(f)",
        ),
        ("let f = |x| x + acc.n", "par_map(f)"),
        ("let f = |x| acc.n > x", "par_filter(f)"),
    ] {
        check_err(
            &format!(
                "{base}fn main() {{\n    let acc = ref(Acc{{ n: 0 }})\n    {bind}\n    \
print(\"{{[1, 2].{op}}}\")\n}}\n"
            ),
            "cannot run in parallel",
        );
    }
    // A name-bound closure that touches no reference is untouched…
    check_ok(&format!(
        "{base}fn main() {{\n    let g = |x| x * 2\n    print(\"{{[1, 2].par_map(g)}}\")\n}}\n"
    ));
    // …and one that does may still be used sequentially ([T-REF-CAPTURE]).
    check_ok(&format!(
        "{base}fn main() {{\n    let acc = ref(Acc{{ n: 1 }})\n    let h = |x| x + acc.n\n    \
print(\"{{[1, 2].map(h)}}\")\n}}\n"
    ));
}

/// A struct field of function type is reachable even when a free function
/// shares its name.
///
/// [L-UFCS] takes a free function in method position only when "its first
/// parameter accepts the receiver", and [T-FIELD-CALL] says a field is what
/// remains when no method and no such candidate exists. Sharing a name was
/// enough, so declaring `fn len(xs: [i64])` anywhere in the module made every
/// `n.len()` on a struct with a `len` field a type error naming `[i64]` — a
/// type that appears nowhere near the call. Any struct with a field called
/// `len`, `get`, `run` or `parse` was one same-named free function away from
/// being uncallable.
#[test]
fn a_field_call_survives_a_free_function_of_the_same_name() {
    // The free function cannot take the receiver, so the field is the call.
    check_ok(
        "type Node = { len: fn() -> i64 }\n\nfn len(xs: [i64]) -> i64 {\n    999\n}\n\n\
fn main() {\n    let n = Node{ len: || 7 }\n    print(\"{n.len()}\")\n}\n",
    );
    // Through a reference and a tuple component, the other two routes
    // [T-FIELD-CALL] names.
    check_ok(
        "type Node = { len: fn() -> i64 }\n\nfn len(xs: [i64]) -> i64 {\n    999\n}\n\n\
fn main() {\n    let r = ref(Node{ len: || 7 })\n    let p = (Node{ len: || 8 }, 1)\n    \
print(\"{r.len()} {p.0.len()}\")\n}\n",
    );
    // A free function that *does* accept the receiver still wins, which is the
    // half of the rule that was already right.
    check_ok(
        "type Node = { len: fn() -> i64 }\n\nfn len(n: Node) -> i64 {\n    999\n}\n\n\
fn main() {\n    let n = Node{ len: || 7 }\n    print(\"{n.len()}\")\n}\n",
    );
    // And a builtin beats both.
    check_ok(
        "fn get(n: i64) -> i64 {\n    999\n}\n\nfn main() {\n    let xs = [1, 2]\n    \
print(\"{xs.get(0) ?? 0}\")\n    print(\"{5.get()}\")\n}\n",
    );
}

/// A chain of field reads is checked in linear time.
///
/// `infer_field` inferred its receiver twice — once for the handler-operation
/// check and once for the read itself — so the cost doubled per link. Forty
/// `.next`s never finished, at one core and 13 MB: not a stack overflow, not a
/// memory limit, and well under the nesting cap that [G-NEST-LIMIT] says must
/// be *reported* rather than hung on. Twenty-two links took 1.3 seconds and
/// twenty-six took twenty-one.
///
/// A thousand links check instantly now. The test is a timing one only in the
/// sense that the old code could not have finished it in the age of the
/// universe: 2^1000.
#[test]
fn a_chain_of_field_reads_is_linear() {
    let chain = ".next".repeat(1000);
    check_ok(&format!(
        "type N = {{ next: N, v: i64 }}\n\nfn depth(n: N) -> i64 {{\n    n{chain}.v\n}}\n\n\
fn main() {{\n    print(\"ok\")\n}}\n"
    ));
    // The same for an index chain and a tuple chain, the other two shapes a
    // projection can take.
    let idx = ".xs[0]".repeat(200);
    check_ok(&format!(
        "type W = {{ xs: [W], v: i64 }}\n\nfn f(w: W) -> i64 {{\n    w{idx}.v\n}}\n\n\
fn main() {{\n    print(\"ok\")\n}}\n"
    ));
    check_ok("fn main() {\n    let t = (((1, 2), 2), 3)\n    print(\"{t.0.0.0}\")\n}\n");
}

/// A chain of UFCS method calls is checked in linear time.
///
/// The same defect as the field chain, one path over: a method call infers its
/// receiver to resolve the method, and then `check_fn_call` checks that same
/// receiver against the parameter it fills. Twenty `.bump(1)`s took 0.7
/// seconds, forty never finished — under the nesting cap that [G-NEST-LIMIT]
/// says must be *reported* rather than hung on.
///
/// Fixed by memoizing inference for the shapes that cannot hold a closure. A
/// closure literal's type depends on the position it fills and its body is
/// checked against the enclosing function's error type, so for those a second
/// visit is not a repeat — memoizing them lost the refusal of a stored closure
/// that fails inside a function declaring no error type, which is why the
/// cache is narrow.
#[test]
fn a_chain_of_method_calls_is_linear() {
    let chain = ".bump(1)".repeat(400);
    check_ok(&format!(
        "type B = {{ v: i64 }}\n\nfn bump(b: B, n: i64) -> B {{\n    B{{ v: b.v + n }}\n}}\n\n\
fn main() {{\n    let b = B{{ v: 0 }}\n    print(\"{{b{chain}.v}}\")\n}}\n"
    ));
    // The narrowness is the point: a closure in an argument is still walked
    // where it is checked, so this stays refused.
    check_err(
        "type E = { c: str }\ntype Box = { f: fn(i64) -> i64 }\n\n\
fn build() -> Box ! E {\n    Box{ f: |x| if x == 0 { fail(E{ c: \"z\" }) } else { x } }\n}\n\n\
fn main() {\n    let b = build() ?? Box{ f: |x| x }\n    print(\"{b.f(0)}\")\n}\n",
        "declares no error type",
    );
}

/// A handler operation's *result* is row-checked, even though the operation
/// itself is not.
///
/// A handler field's type carries an unconstrained row, which is what
/// [E-HANDLER-OWN-ROW] requires: a handler's operations are charged to whoever
/// wrote the handler, not to whoever installs it. That exemption is about the
/// operation; it is not about the value the operation hands back
/// ([T-EFFECT-OP-RESULT]).
///
/// `check_expr` returned early for a closure after checking the closure's *own*
/// row, so the rows nested in its result type were never compared — every other
/// expression got that check by falling through one line further. So
/// `MkHandler{ make: || shout }` for `fn make() -> fn(i64) -> i64` handed a
/// printing function through an empty-row result: a function declaring `/ {Mk}`
/// printed, and a `capabilities = []` package reached `io`, while the same
/// signature written as an ordinary `fn` was refused.
#[test]
fn a_handler_operations_result_carries_its_row() {
    let base = "fn shout(n: i64) -> i64 {\n    print(\"x\")\n    n\n}\n\n\
fn quiet(n: i64) -> i64 {\n    n * 2\n}\n\n";
    for (ret, give) in [
        ("fn(i64) -> i64", "shout"),
        ("[fn(i64) -> i64]", "[shout]"),
        ("Option[fn(i64) -> i64]", "Some(shout)"),
        ("(fn(i64) -> i64, i64)", "(shout, 1)"),
    ] {
        check_err(
            &format!(
                "{base}effect Mk {{\n    fn make() -> {ret}\n}}\n\n\
fn main() {{\n    with Mk handled_by MkHandler{{ make: || {give} }} {{\n        \
print(\"in\")\n    }}\n}}\n"
            ),
            "performs nothing, but this one performs `io`",
        );
        // The pure value through the same result type is accepted.
        check_ok(&format!(
            "{base}effect Mk {{\n    fn make() -> {ret}\n}}\n\n\
fn main() {{\n    with Mk handled_by MkHandler{{ make: || {} }} {{\n        \
print(\"in\")\n    }}\n}}\n",
            give.replace("shout", "quiet")
        ));
    }
    // A handler whose *operation* performs `io` is still fine — that is the
    // exemption, and it is the whole reason handlers exist.
    check_ok(
        "effect Log {\n    fn write(s: str)\n}\n\nfn main() {\n    \
with Log handled_by LogHandler{ write: |s| print(s) } {\n        \
Log.write(\"hello\")\n    }\n}\n",
    );
    // …and a result that *declares* the row accepts the effectful value.
    check_ok(&format!(
        "{base}effect Mk {{\n    fn make() -> fn(i64) -> i64 / {{io}}\n}}\n\n\
fn main() {{\n    with Mk handled_by MkHandler{{ make: || shout }} {{\n        \
print(\"in\")\n    }}\n}}\n"
    ));
}

/// A deeply nested tuple literal is checked in near-linear time.
///
/// `record_value_row` runs on every expression and called `resolve`, which
/// copies the whole type. For a value whose type is deep — a nested tuple
/// literal is the natural way to get one — that copy costs O(n) at each of n
/// levels, on top of a check that was already superlinear: 400 deep took 0.46
/// seconds, 800 took 3.9, 1000 took 7.8. Only a function type is wanted there,
/// and its head is visible without copying anything below it.
///
/// A thousand levels now checks in about 30 ms. Like the field- and
/// method-chain tests, this is a timing test only in the sense that the old
/// code could not have passed it.
/// Run on a thread with the stack the CLI's main thread has. A test harness
/// thread gets 2 MiB and `main` gets 8, so a depth the toolchain handles in
/// all three engines aborts the test binary — which would be the test
/// measuring the harness rather than the compiler. The deepest nesting the
/// language accepts is 1022; 1023 is a clean `code is nested too deeply`
/// ([G-NEST-LIMIT]), and both were checked by hand against `check`, `run` and
/// `build --native` when this was written.
#[test]
fn a_deeply_nested_tuple_literal_is_near_linear() {
    std::thread::Builder::new()
        .stack_size(8 * 1024 * 1024)
        .spawn(|| {
            let mut e = String::from("1");
            for _ in 0..1000 {
                e = format!("({e}, 2)");
            }
            check_ok(&format!(
                "fn main() {{\n    let t = {e}\n    print(\"ok\")\n}}\n"
            ));
        })
        .expect("spawns")
        .join()
        .expect("the checker should not abort at a depth the language accepts");
}

/// A `ref` receiver does not make a free function a UFCS candidate.
///
/// [L-UFCS] takes a free function in method position only when "its first
/// parameter accepts the receiver", and [T-FIELD-CALL] says "a `ref` is read
/// through, so `c.f(3)` calls the field of the referent". Reading through one
/// is [T-REF-READ]; being *passed* as one is not, and `fn step(b: Box, …)`
/// cannot take a `ref Box`. Treating them as interchangeable made that function
/// a candidate, which then failed its own argument check — so a field call
/// through a reference was refused with "expected `Box`, found `ref Box`"
/// rather than falling back to the field.
#[test]
fn a_ref_receiver_does_not_make_a_free_function_a_candidate() {
    let base = "type Box = { step: fn(i64) -> i64 }\n\nfn twice(n: i64) -> i64 {\n    n * 2\n}\n\n";
    // The free function takes the referent, so it is not a candidate for a
    // `ref` receiver: the field answers.
    check_ok(&format!(
        "{base}fn step(b: Box, x: i64) -> i64 {{\n    x + 2\n}}\n\nfn main() {{\n    \
let r = ref(Box{{ step: twice }})\n    print(\"{{r.step(5)}}\")\n}}\n"
    ));
    // The mirror: a free function over `ref Box` is not a candidate for a plain
    // receiver either.
    check_ok(&format!(
        "{base}fn step(b: ref Box, x: i64) -> i64 {{\n    x + 2\n}}\n\nfn main() {{\n    \
let b = Box{{ step: twice }}\n    print(\"{{b.step(5)}}\")\n}}\n"
    ));
    // And when it *does* accept the receiver, it still wins.
    check_ok(&format!(
        "{base}fn step(b: ref Box, x: i64) -> i64 {{\n    x + 2\n}}\n\nfn main() {{\n    \
let r = ref(Box{{ step: twice }})\n    print(\"{{r.step(5)}}\")\n}}\n"
    ));
}

/// The methods a range answers are the ones its diagnostic names.
///
/// `RANGE_METHODS` in the checker is a hand-written list, and a help text that
/// names a method the compiler dropped is worse than no help at all. So each
/// name in it is typed against a real range here. The list is not importable —
/// it is private — which is the point: this test would fail if the two drifted.
#[test]
fn range_methods_are_the_ones_the_help_names() {
    // Named exactly as the diagnostic names them, so a rename in one place
    // fails here rather than going out in a help text.
    for (call, ok) in [
        ("(0..3).to_list()", true),
        ("(0..3).map(|x| x + 1)", true),
        ("(0..3).filter(|x| x > 1)", true),
        ("(0..3).sum()", true),
        ("(0..3).len()", true),
    ] {
        let src = format!("fn main() {{\n    let v = {call}\n    print(\"{{v}}\")\n}}\n");
        let (_, diags) = grasm_sema::analyze(&src);
        assert_eq!(
            diags.is_empty(),
            ok,
            "`{call}` is named by the range help, so it must type; got:\n{diags:#?}"
        );
    }
}

/// A range's "no method" help must not name a type nobody can write.
///
/// `range` is not in the type grammar — `fn f(r: range)` is "unknown type
/// `range`" — so the ordinary UFCS help proposed a signature that cannot be
/// typed. Third instance of that mistake; the first two are guarded beside it.
#[test]
fn the_range_help_does_not_propose_an_unwriteable_signature() {
    let (_, diags) =
        grasm_sema::analyze("fn main() {\n    let r = 0..3\n    print(\"{r.count()}\")\n}\n");
    let d = diags
        .iter()
        .find(|d| d.message.contains("count"))
        .expect("a range has no `count`, so this must be refused");
    let help = d.help.clone().unwrap_or_default();
    assert!(
        !help.contains("fn count(range"),
        "the help proposes `fn count(range, ...)`, a signature nobody can write: {help}"
    );
    assert!(
        help.contains("to_list") && help.contains("len"),
        "the help should name the methods a range does have: {help}"
    );
    // And the message itself should not print `range` as if it were a type
    // name a reader could use.
    assert!(
        d.message.contains("on a range"),
        "the message should say `on a range`, not name it as a written type: {}",
        d.message
    );
}

/// A match arm's pattern bindings are in scope for closures written in it.
///
/// Closure bodies are checked at the end of the scope that holds them, and a
/// match arm opens a scope without opening a frame for them — so they were
/// drained after the arm's bindings were gone and `Some(n) -> |x| x + n` said
/// "unknown name `n`" about the binding two tokens to its left. A correct
/// program had no spelling at all.
///
/// The same defect was fixed twice before, for a closure nested in a closure
/// and for one in a block. This is the third scope-opener.
#[test]
fn a_closure_in_a_match_arm_sees_the_arms_bindings() {
    // Bare closure as the arm body.
    check_ok(
        "fn main() {\n    let o: Option[i64] = Some(3)\n    \
let g: fn(i64) -> i64 = match o { Some(n) -> |x| x + n  None -> |x| x }\n    \
print(\"{g(1)}\")\n}\n",
    );
    // Closure inside a variant constructor in the arm body.
    check_ok(
        "fn main() {\n    let o: Option[i64] = Some(3)\n    \
let g: Option[fn(i64) -> i64] = match o {\n        Some(n) -> Some(|x| x + n)\n        \
None -> None\n    }\n    \
match g { Some(f) -> print(\"{f(1)}\")  None -> print(\"none\") }\n}\n",
    );
    // Closure as an argument in the arm body — this one always worked, and is
    // here so a fix that breaks it is caught.
    check_ok(
        "fn take(f: fn(i64) -> i64) -> i64 {\n    f(1)\n}\n\nfn main() {\n    \
let o: Option[i64] = Some(3)\n    \
let r = match o { Some(n) -> take(|x| x + n)  None -> 0 }\n    print(\"{r}\")\n}\n",
    );
    // Nested one level further: a closure inside a closure inside an arm.
    check_ok(
        "fn main() {\n    let o: Option[i64] = Some(3)\n    \
let g: fn(i64) -> fn(i64) -> i64 = match o {\n        Some(n) -> |x| |y| x + y + n\n        \
None -> |x| |y| x + y\n    }\n    print(\"{g(1)(2)}\")\n}\n",
    );
}

/// `&&` and `||` are refused, and say what to write instead.
///
/// The first thing anyone arriving from C, Rust, Go, JavaScript or Java types.
/// It used to be "unexpected character `&`", which sends a reader hunting for a
/// typo they did not make — nothing is misspelled, the operator has a different
/// name.
#[test]
fn the_c_spellings_of_and_or_name_the_grasm_ones() {
    for (wrote, meant) in [("&&", "and"), ("||", "or")] {
        let src = format!(
            "fn main() {{\n    let x = 1\n    if x > 0 {wrote} x < 5 {{\n        \
print(\"yes\")\n    }}\n}}\n"
        );
        let (_, diags) = grasm_sema::analyze(&src);
        assert!(
            diags.iter().any(|d| d
                .message
                .contains(&format!("`{wrote}` is written `{meant}`"))),
            "`{wrote}` should name `{meant}`; got:\n{diags:#?}"
        );
        // And exactly one error: reported where the precedence is known, the
        // operator is consumed as the one it means and the rest of the file
        // parses. Rejected as a character it left the parser hunting for the
        // `{` of the `if` and inventing a second error at the closing brace.
        assert_eq!(
            diags.len(),
            1,
            "one mistake should give one error, not a cascade; got:\n{diags:#?}"
        );
    }
    // Both spellings in one condition: two mistakes, two errors, nothing else.
    let (_, diags) = grasm_sema::analyze(
        "fn main() {\n    let x = 1\n    let y = 9\n    if x > 0 && x < 5 || y == 9 {\n        \
print(\"yes\")\n    }\n}\n",
    );
    assert_eq!(diags.len(), 2, "expected exactly two; got:\n{diags:#?}");
    // The correct spellings still work, and `|` is still a token: a sum type
    // and a pipe both parse.
    check_ok(
        "type S = | A(v: i64) | B\n\nfn double(n: i64) -> i64 {\n    n * 2\n}\n\n\
fn main() {\n    let x = 1\n    if x > 0 and x < 5 or x == 9 {\n        \
print(\"{2 |> double}\")\n    }\n    match A(1) { A(v) -> print(\"{v}\")  B -> print(\"b\") }\n}\n",
    );
}

/// Every diagnostic points at real, non-whitespace source.
///
/// A span that is empty, reversed, past the end of the file, or sitting on a
/// space is an off-by-one, and an off-by-one in a span is invisible in a test
/// that only asserts on the message. This checks the property directly over a
/// corpus of wrong programs, so the whole class is caught rather than the one
/// instance somebody happened to notice.
#[test]
fn every_diagnostic_span_covers_real_source() {
    let wrong = [
        "fn main() {\n    let x: i64 = \"s\"\n}\n",
        "fn main() {\n    let x = 1 + \"a\"\n}\n",
        "fn main() {\n    print(\"{nope}\")\n}\n",
        "fn main() {\n    let r = 0..3\n    print(\"{r.count()}\")\n}\n",
        "fn main() {\n    let x = 1\n    if x > 0 && x < 5 {\n        print(\"y\")\n    }\n}\n",
        "fn main() {\n    let t = (1, \"a\")\n    print(\"{t.9}\")\n}\n",
        "fn main() {\n    let d = \"abc\".index_of(\"b\", 1)\n    print(\"{d}\")\n}\n",
        "fn g() -> i64 ! IoError {\n    Ok(1)\n}\n\nfn main() {\n    let c = g()\n    print(\"{c}\")\n}\n",
        "fn main() {\n    var xs = [1]\n    xs.nope()\n}\n",
        "type S = { a: i64 }\n\nfn main() {\n    let s = S{ a: 1 }\n    print(\"{s.b}\")\n}\n",
        "fn main() {\n    \"hi\".print()\n}\n",
        "fn f(n: i64) -> i64 {\n    n\n}\n\nfn main() {\n    print(\"{f(1, 2)}\")\n}\n",
    ];
    for src in wrong {
        let (_, diags) = grasm_sema::analyze(src);
        assert!(
            !diags.is_empty(),
            "this program is wrong and should be refused:\n{src}"
        );
        for d in &diags {
            let (lo, hi) = (d.span.start as usize, d.span.end as usize);
            assert!(
                lo < hi,
                "empty or reversed span {lo}..{hi} for `{}` in:\n{src}",
                d.message
            );
            assert!(
                hi <= src.len(),
                "span {lo}..{hi} runs past the {} bytes of:\n{src}",
                src.len()
            );
            assert!(
                src.is_char_boundary(lo) && src.is_char_boundary(hi),
                "span {lo}..{hi} splits a character in:\n{src}"
            );
            let text = &src[lo..hi];
            assert!(
                text.chars().any(|c| !c.is_whitespace()),
                "span for `{}` covers only whitespace ({text:?}) in:\n{src}",
                d.message
            );
            // A caret is drawn under one line. A span crossing a newline draws
            // it under text the reader is not being pointed at.
            assert!(
                !text.contains('\n') || text.trim_end().contains('\n'),
                "span for `{}` ends on a newline in:\n{src}",
                d.message
            );
        }
    }
}

/// Every callable builtin must also be a *reserved* name.
///
/// These were two hand-written lists. They drifted: the whole `tcp_*` family
/// was callable and none of it was reserved, so a program could declare
/// `fn tcp_read` and watch the builtin answer instead. Adding `run` for
/// [L-LIB-PROC] hit the same seam from the other side — the name became
/// callable, two programs that already had a `fn run` kept compiling, and their
/// own function stopped being reached.
///
/// The lists are one list now, so this cannot drift again by construction. The
/// test is here because "cannot drift by construction" is a property of today's
/// code and this is what makes it a property of tomorrow's.
#[test]
fn every_callable_builtin_is_a_reserved_name() {
    for name in grasm_sema::check::BUILTIN_FREE_FNS {
        let src = format!("fn {name}() {{\n}}\n\nfn main() {{\n}}\n");
        check_err(&src, "is built in");
    }
}

/// No builtin may take a name a program in this repository already uses.
///
/// The two rules above are judgement, and judgement missed twice. `run` was
/// added and two ProjectGold programs already had a `fn run`. `run_pipeline`
/// was added — `verb_noun`, which the rule below asks for — and
/// `examples/function_values.gr` already had a `fn run_pipeline` folding a list
/// of integers. "Pipeline" is a word programs use for their own abstractions,
/// and no naming convention was going to predict that.
///
/// This one is not judgement. It reads the corpus.
#[test]
fn no_builtin_takes_a_name_the_corpus_already_uses() {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(|p| p.parent())
        .expect("crates/grasm_sema is two levels down")
        .to_path_buf();

    let mut sources = Vec::new();
    let mut stack = vec![root.clone()];
    while let Some(dir) = stack.pop() {
        let Ok(entries) = std::fs::read_dir(&dir) else {
            continue;
        };
        for e in entries.flatten() {
            let path = e.path();
            let name = path.file_name().and_then(|n| n.to_str()).unwrap_or("");
            // `target` is build output and `.git` is history; neither is code
            // anybody wrote.
            if path.is_dir() {
                if name != "target" && name != ".git" {
                    stack.push(path);
                }
            } else if path.extension().is_some_and(|x| x == "gr") {
                sources.push(path);
            }
        }
    }
    assert!(
        sources.len() > 100,
        "expected to find the .gr corpus, found {} file(s)",
        sources.len()
    );

    let mut clashes = Vec::new();
    for path in &sources {
        let Ok(text) = std::fs::read_to_string(path) else {
            continue;
        };
        for line in text.lines() {
            let t = line.trim_start();
            let Some(rest) = t.strip_prefix("fn ").or_else(|| t.strip_prefix("pub fn ")) else {
                continue;
            };
            let declared: String = rest
                .chars()
                .take_while(|c| c.is_alphanumeric() || *c == '_')
                .collect();
            if grasm_sema::check::BUILTIN_FREE_FNS.contains(&declared.as_str()) {
                let shown = path.strip_prefix(&root).unwrap_or(path);
                clashes.push(format!("{}: fn {declared}", shown.display()));
            }
        }
    }
    assert!(
        clashes.is_empty(),
        "a builtin has taken a name a program already uses — the program stops \
compiling and the error says nothing about a name being reserved:\n  {}",
        clashes.join("\n  ")
    );
}

/// A builtin's name must be one nobody would reasonably want.
///
/// Reserving a name is permanent and total: a program cannot declare it, cannot
/// work around it, and gets a compile error rather than a deprecation. `run` was
/// added as a builtin and immediately took a name two programs on this repo's
/// own list were already using — which is the cheapest possible warning that the
/// name was wrong, and it only arrived because those programs are in the gate.
///
/// So a builtin names the thing outside the program that it touches, usually
/// with a domain prefix: `tcp_listen`, `udp_bind`, `net_wait`, `proc_run`,
/// `proc_pipeline`, `read_file`, `write_bytes`. The bare verbs a program is
/// likely to want — `run`, `send`, `open`, `close`, `read`, `write`, `start`,
/// `stop`, `wait`, `execute` — stay available.
///
/// `verb_noun` alone was not enough: `run_pipeline` fitted it and still took a
/// name a program in this repository was using. The test above is what
/// actually catches that.
#[test]
fn builtins_do_not_take_names_a_program_would_want() {
    const WANTED: &[&str] = &[
        "run",
        "send",
        "open",
        "close",
        "read",
        "write",
        "start",
        "stop",
        "wait",
        "execute",
        "step",
        "next",
        "get",
        "set",
        "add",
        "push",
        "pop",
        "find",
        "parse",
        "format",
        "log",
        "init",
        "update",
        "check",
        "process",
        "handle",
        "make",
        "build",
        "run_pipeline",
        "pipeline",
        "emit",
        "apply",
        "reduce",
        "walk",
        "visit",
        "render",
    ];
    let taken: Vec<&str> = WANTED
        .iter()
        .copied()
        .filter(|w| grasm_sema::check::BUILTIN_FREE_FNS.contains(w))
        .collect();
    assert!(
        taken.is_empty(),
        "these builtin names are ordinary words a program would want for its \
own functions, and reserving one is permanent: {taken:?}"
    );
}

// ---------------------------------------------------------------------------
// The builtin table is the only place a builtin's row is written down
// ---------------------------------------------------------------------------
//
// `read_bytes` and `write_bytes` were in the checker's list of builtins and not
// in the effect system's, so a function declaring `/ {}` could read any file on
// the disk and a package declaring `capabilities = []` could too — for as long
// as `bytes` had existed (ProjectGold F35). Two hand-maintained lists, and no
// test could see the difference because each test was written against one of
// them.
//
// These three make the second list impossible.

#[test]
fn builtin_names_agree_with_the_table() {
    use std::collections::BTreeSet;
    let listed: BTreeSet<&str> = grasm_sema::check::BUILTIN_FREE_FNS
        .iter()
        .copied()
        .collect();
    let tabled: BTreeSet<&str> = grasm_sema::builtins::BUILTINS
        .iter()
        .map(|b| b.name)
        .collect();

    let missing_row: Vec<&&str> = listed.difference(&tabled).collect();
    assert!(
        missing_row.is_empty(),
        "these builtins have no row in crate::builtins, so nothing charges them \
         an effect — which is exactly how F35 happened:\n  {missing_row:?}"
    );
    let missing_name: Vec<&&str> = tabled.difference(&listed).collect();
    assert!(
        missing_name.is_empty(),
        "these have a row but are not builtins the checker knows:\n  {missing_name:?}"
    );
}

#[test]
fn every_builtin_that_reaches_the_world_is_charged() {
    // A builtin whose name says it touches something, with an empty row, is
    // either a mistake or a decision — and a decision belongs in the table's
    // comment rather than in nobody's head.
    let suspicious = [
        "read", "write", "file", "dir", "tcp", "udp", "net", "proc", "env", "now", "input",
        "print", "args",
    ];
    for b in grasm_sema::builtins::BUILTINS {
        if b.row.is_empty() && suspicious.iter().any(|s| b.name.contains(s)) {
            panic!(
                "`{}` reaches outside the program by its name and has an empty \
                 effect row. If that is deliberate, say why in the table.",
                b.name
            );
        }
    }
}

#[test]
fn every_effectful_builtin_is_named_in_conformance() {
    // T-EFFECT-BUILTIN.gr names each one inside a function declaring its row.
    // Those functions never run: naming a builtin in a row that declares the
    // effect is what makes the checker charge it, so a builtin that stopped
    // being charged would make that file fail to compile.
    //
    // Which means this test's job is only to notice a builtin that was added to
    // the table and never named there.
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../spec/conformance/T-EFFECT-BUILTIN.gr");
    let src = std::fs::read_to_string(&path)
        .unwrap_or_else(|e| panic!("cannot read {}: {e}", path.display()));
    let mut absent = Vec::new();
    for b in grasm_sema::builtins::effectful() {
        if !src.contains(b.name) {
            absent.push(b.name);
        }
    }
    assert!(
        absent.is_empty(),
        "these builtins perform an effect and are not named in \
         spec/conformance/T-EFFECT-BUILTIN.gr, so nothing would notice if they \
         stopped being charged:\n  {absent:?}"
    );
}

/// Every builtin that reaches the world is either journaled or deliberately
/// repeated — never silently neither.
///
/// A new effectful builtin added without a thought about replay would default
/// to `Untouched`, and a recording would then be quietly incomplete: the
/// program would reach the world during a replay and nobody would be told. The
/// classification is not optional, so the test makes it not optional.
#[test]
fn every_effectful_builtin_says_what_a_replay_owes_it() {
    use grasm_sema::builtins::{Journal, ASYNC, BUILTINS};
    for b in BUILTINS {
        // `async` alone does not reach the world: a channel operation talks
        // to another task of the same program, under a schedule that is fixed,
        // so a replay does it again and gets the same answer.
        if b.row.iter().all(|e| *e == ASYNC) {
            assert_eq!(
                b.journal,
                Journal::Untouched,
                "`{}` reaches nothing outside the program, so a replay has \
                 nothing to hand it back",
                b.name
            );
        } else {
            assert_ne!(
                b.journal,
                Journal::Untouched,
                "`{}` performs {:?} but is marked as touching nothing — a \
                 recording of a program that calls it would be incomplete, and \
                 the replay would reach the world instead",
                b.name,
                b.row
            );
        }
    }
}

/// Every journaled builtin has a program in the replay gate.
///
/// `scripts/replaytest.sh` proves a hook exists by running one call under an
/// empty journal. A builtin with no program there is not checked by it, and
/// the script's own count of "no test program" only appears when someone runs
/// it — this fails in `cargo test`, where the gate list starts.
#[test]
fn every_journaled_builtin_is_named_in_the_replay_gate() {
    use grasm_sema::builtins::recorded;
    let script = include_str!("../../../scripts/replaytest.sh");
    for b in recorded() {
        assert!(
            script.contains(&format!("    {})", b.name)),
            "`{}` is journaled but has no program in scripts/replaytest.sh, so \
             nothing checks that its hook is still there",
            b.name
        );
    }
}

/// A constructor that mentions one type parameter and not the other has to be
/// annotated ([T-GENERIC-TYPE]).
///
/// The clause said this was the language's rule while one engine of three
/// enforced it: `grasm check` passed, `grasm run` ran it, and only the compiled
/// backend refused — which made it look like a backend limitation rather than
/// what the clause says it is.
#[test]
fn a_constructor_may_not_leave_a_parameter_open() {
    check_err(
        "type Res[T, E] = | Good(v: T) | Bad(e: E)\nfn main() {\n    let r = Good(1)\n    match r {\n        Good(v) -> print(\"{v}\")\n        Bad(e) -> print(\"bad\")\n    }\n}\n",
        "leaves a type parameter undetermined",
    );
}

/// Determined by a *later* line is determined. The check waits for inference to
/// finish for the same reason the empty-literal one does.
#[test]
fn a_parameter_a_later_line_fixes_is_determined() {
    check_ok(
        "type Res[T, E] = | Good(v: T) | Bad(e: E)\nfn take(r: Res[i64, str]) -> i64 {\n    match r {\n        Good(v) -> v\n        Bad(e) -> -1\n    }\n}\nfn main() {\n    let a = Good(1)\n    print(\"{take(a)}\")\n}\n",
    );
}

/// A closure with an *expected* type still has its body checked.
///
/// Checking the body against the wanted return type and then, on any
/// diagnostic, discarding all of them and re-inferring — so the mismatch reads
/// from the type the body actually has — lost every unrelated error with them.
/// Inference is memoized, so the second visit answered from the cache and said
/// nothing at all (F44).
#[test]
fn an_unknown_name_in_a_typed_closure_is_reported() {
    for src in [
        "fn main() {\n    let f: fn(i64) -> () = |v| nosuchname(v)\n    f(1)\n}\n",
        "fn call(f: fn(i64) -> ()) {\n    f(1)\n}\nfn main() {\n    call(|v| nosuchname(v))\n}\n",
        "type B = { f: fn(i64) -> () }\nfn main() {\n    let b = B{ f: |v| nosuchname(v) }\n    b.f(1)\n}\n",
        "effect S {\n    fn put(v: i64)\n}\nfn main() {\n    with S handled_by SHandler{ put: |v| nosuchname(v) } {\n        S.put(1)\n    }\n}\n",
    ] {
        check_err(src, "unknown name `nosuchname`");
    }
}

/// And an unknown *method*, which is the same hole one resolution apart.
#[test]
fn an_unknown_method_in_a_typed_closure_is_reported() {
    check_err(
        "fn main() {\n    let f: fn(i64) -> i64 = |v| v.nomethod()\n    print(\"{f(1)}\")\n}\n",
        "no method `nomethod` on `i64`",
    );
}

/// The reason the rollback existed is preserved: a body that does not fit is
/// still reported from the type it actually has, not as a nested mismatch.
#[test]
fn a_body_that_does_not_fit_still_reads_from_its_own_type() {
    check_err(
        "fn main() {\n    let f: fn(i64) -> i64 = |v| v + \"s\"\n    print(\"{f(1)}\")\n}\n",
        "expected `i64`, found `str`",
    );
}

/// A write to a captured binding is refused inside a *handler operation* too.
///
/// [T-CAPTURE-WRITE] says the rule exists because of "a handler that appears to
/// accumulate state and accumulates none", and its conformance program uses an
/// unannotated closure — which is deferred, and was checked. A handler
/// operation has an expected type, which is the position F44 made deaf: this
/// exact program compiled, and the handler quietly accumulated nothing.
#[test]
fn a_handler_operation_may_not_write_to_a_captured_binding() {
    check_err(
        "effect S {\n    fn put(k: str, v: i64)\n}\nfn main() {\n    var kv = map()\n    kv.set(\"seed\", 0)\n    with S handled_by SHandler{ put: |k, v| kv.set(k, v) } {\n        S.put(\"a\", 1)\n    }\n}\n",
        "captured from outside",
    );
}

/// Every rule, in every position a closure body can occupy.
///
/// F44 was one rule going quiet in one position: a closure whose type is known
/// from context had its diagnostics discarded, so unknown names and unknown
/// methods inside an annotated `let`, a function parameter, a struct field or a
/// handler operation were accepted. Each of those rules was tested — in a
/// *statement*, which was never the deaf position.
///
/// So the rules are crossed with the positions. A rule that fires in one place
/// and not another is the shape of that defect, and this fails for any of them
/// rather than for the one that happened to be found.
#[test]
fn a_rule_that_fires_anywhere_fires_everywhere() {
    // (name, offending fragment, what the diagnostic must say, legal alone?)
    let rules: &[(&str, &str, &str, bool)] = &[
        ("unknown name", "nosuchname(1)", "unknown name", true),
        ("unknown method", "(1).nomethod()", "no method", true),
        ("type mismatch", "let z: i64 = \"s\"", "mismatch", true),
        (
            "unhandled fallible",
            "risky()",
            "failure is discarded",
            true,
        ),
        // Legal as a statement — it is only a defect when the binding is
        // captured — so that position is not asked of it.
        (
            "capture write",
            "cell.push(1)",
            "captured from outside",
            false,
        ),
        (
            "undeclared effect",
            "print(\"x\")",
            "not in its declared row",
            false,
        ),
    ];

    const PRE: &str = "type E = { m: str }\nfn risky() -> i64 ! E {\n    Ok(1)\n}\n";

    for (rule, frag, needle, in_statement) in rules {
        // Every wrapper declares `/ {}`, so the effect rule has a row to break,
        // and the fragment is never the tail — a tail propagates rather than
        // discards, which is a different rule.
        let mut cases: Vec<(&str, String)> = vec![
            ("unannotated closure", format!("fn plain() / {{}} {{\n    var cell: [i64] = []\n    let f = || {{\n        {frag}\n        0\n    }}\n    let _r = f()\n}}\nfn main() {{ plain() }}\n")),
            ("annotated closure", format!("fn plain() / {{}} {{\n    var cell: [i64] = []\n    let f: fn() -> i64 = || {{\n        {frag}\n        0\n    }}\n    let _r = f()\n}}\nfn main() {{ plain() }}\n")),
            ("fn parameter", format!("fn call(f: fn() -> i64) -> i64 {{\n    f()\n}}\nfn plain() / {{}} {{\n    var cell: [i64] = []\n    let _r = call(|| {{\n        {frag}\n        0\n    }})\n}}\nfn main() {{ plain() }}\n")),
            ("struct field", format!("type B = {{ f: fn() -> i64 }}\nfn plain() / {{}} {{\n    var cell: [i64] = []\n    let b = B{{ f: || {{\n        {frag}\n        0\n    }} }}\n    let _r = b.f()\n}}\nfn main() {{ plain() }}\n")),
            ("handler operation", format!("effect S {{\n    fn op() -> i64\n}}\nfn plain() / {{}} {{\n    var cell: [i64] = []\n    with S handled_by SHandler{{ op: || {{\n        {frag}\n        0\n    }} }} {{\n        let _r = S.op()\n    }}\n}}\nfn main() {{ plain() }}\n")),
            ("map closure", format!("fn plain() / {{}} {{\n    var cell: [i64] = []\n    let _r = [1].map(|x| {{\n        {frag}\n        x\n    }})\n}}\nfn main() {{ plain() }}\n")),
        ];
        if *in_statement {
            cases.insert(0, ("statement", format!("fn plain() / {{}} {{\n    var cell: [i64] = []\n    {frag}\n    let _k = cell.len()\n}}\nfn main() {{ plain() }}\n")));
        }

        for (position, body) in cases {
            let src = format!("{PRE}{body}");
            let (_, diags) = analyze(&src);
            let text = diags
                .iter()
                .map(|d| d.message.clone())
                .collect::<Vec<_>>()
                .join("\n");
            // The needle first. A parse-failure guard that ran before it
            // matched the word "expected" inside a perfectly good
            // `type mismatch: expected \`i64\`, found \`str\`` and called the
            // probe malformed.
            if text.contains(needle) {
                continue;
            }
            // A program that does not parse tests nothing. Say which, rather
            // than reporting it as a missing rule.
            assert!(
                !text.contains("expected a declaration")
                    && !text.contains("expected an expression")
                    && !text.contains("expected end of line"),
                "the probe for `{rule}` in {position} does not parse:\n{text}\n{src}"
            );
            assert!(
                text.contains(needle),
                "`{rule}` is not reported in {position} — a rule that fires in one \
place and not another is F44's shape\n  wanted: {needle}\n  got: {text:?}\n{src}"
            );
        }
    }
}

#[test]
fn a_reference_to_a_sum_is_read_and_written_through_value() {
    // [T-REF-VALUE] offers `.value` when the referent has no way to name its
    // parts. A sum has none — but a sum is a `Named` type just as a struct is,
    // and asking whether the referent was named excluded it along with the
    // structs. Rebinding a `var` only moves the local, so nothing in the
    // language could write through a `ref` to a sum at all (F48).
    check_ok(
        "type Col = | Red | Green\n\
         fn main() {\n\
         \x20   let r = ref(Red)\n\
         \x20   let c = r\n\
         \x20   c.value = Green\n\
         \x20   match r.value {\n\
         \x20       Red -> print(\"red\")\n\
         \x20       Green -> print(\"green\")\n\
         \x20   }\n\
         }\n",
    );
    // An `Option` is a sum like any other.
    check_ok(
        "fn main() {\n\
         \x20   let r = ref(Some(1))\n\
         \x20   r.value = None\n\
         \x20   print(\"{r.value}\")\n\
         }\n",
    );
    // The two exclusions the clause does name still hold, so this widened the
    // rule to what it says rather than to everything.
    check_err(
        "type P = { n: i64 }\n\
         fn main() {\n\
         \x20   let r = ref(P{ n: 1 })\n\
         \x20   print(\"{r.value}\")\n\
         }\n",
        "has no field `value`",
    );
    check_err(
        "fn main() {\n\
         \x20   let r = ref((1, 2))\n\
         \x20   print(\"{r.value}\")\n\
         }\n",
        "a tuple has no field `value`",
    );
}

/// An interface method's effect row bounds its implementations, and a call
/// through a bound charges the caller that row ([T-INTERFACE-EFFECTS]).
///
/// Both halves are refusals, and a conformance program has one answer, so
/// they live here. `spec/conformance/T-INTERFACE-EFFECTS-ROW.gr` shows the
/// accepted shape.
#[test]
fn an_interface_effect_row_bounds_its_impls_and_charges_its_callers() {
    // An `impl` may not declare an effect its interface does not: a bounded
    // call contributes the *interface's* row, so the extra one would be
    // reached through a row that never mentions it -- and a package's
    // capabilities are checked against those rows.
    check_err(
        "interface Sink {\n\
         \x20   fn emit(self, line: str) / {io}\n\
         }\n\
         type Net = { host: str }\n\
         impl Sink for Net {\n\
         \x20   fn emit(self, line: str) / {io, net} {\n\
         \x20       print(\"{self.host} {line}\")\n\
         \x20   }\n\
         }\n\
         fn main() / {io} {\n\
         \x20   Net{ host: \"h\" }.emit(\"x\")\n\
         }\n",
        "declares `net`, which `Sink` does not",
    );
    // And a generic body whose declared row omits what its bound performs is
    // refused where it is written, not where it is called.
    check_err(
        "interface Sink {\n\
         \x20   fn emit(self, line: str) / {io}\n\
         }\n\
         fn send[T: Sink](s: T) / {} {\n\
         \x20   s.emit(\"x\")\n\
         }\n\
         fn main() {\n\
         }\n",
        "performs effect(s) `io` not in its declared row",
    );
    // An interface that declares no row still admits no effect at all.
    check_err(
        "interface Show {\n\
         \x20   fn show(self) -> str\n\
         }\n\
         type P = { x: i64 }\n\
         impl Show for P {\n\
         \x20   fn show(self) -> str / {io} { \"{self.x}\" }\n\
         }\n\
         fn main() {\n\
         }\n",
        "declares `io`, which `Show` does not",
    );
}

/// An `impl` is for a shape, and the parameters it introduces are what the
/// shape is written in terms of ([T-INTERFACE-GENERIC]).
///
/// The accepted shapes are in `spec/conformance/T-INTERFACE-GENERIC.gr`;
/// these are the three refusals, which a conformance program cannot be.
#[test]
fn a_generic_impl_is_for_a_shape_and_its_own_parameters() {
    // An element is only what its bound says it is, exactly as in a generic
    // function: without `: Show` the `impl` cannot call `show` on an element,
    // even though every type it will be reached with happens to have one.
    check_err(
        "interface Show {\n\
         \x20   fn show(self) -> str\n\
         }\n\
         impl Show for i64 {\n\
         \x20   fn show(self) -> str { \"{self}\" }\n\
         }\n\
         impl[T] Show for [T] {\n\
         \x20   fn show(self) -> str { self[0].show() }\n\
         }\n\
         fn main() {\n\
         }\n",
        "has no method `show`",
    );
    // A declared type is given the parameters it was declared with, so an
    // `impl` cannot quietly introduce one the type has no room for.
    check_err(
        "interface Show {\n\
         \x20   fn show(self) -> str\n\
         }\n\
         type Pair[A] = { left: A, right: A }\n\
         impl[A, B] Show for Pair[A] {\n\
         \x20   fn show(self) -> str { \"x\" }\n\
         }\n\
         fn main() {\n\
         }\n",
        "takes 1 type parameter(s) and this `impl` introduces 2",
    );
    // And a list shape is for every element type: writing one for `[i64]`
    // alone would be an `impl` two shapes could claim, so it is refused at
    // the type rather than left to overlap.
    check_err(
        "interface Show {\n\
         \x20   fn show(self) -> str\n\
         }\n\
         impl Show for [i64] {\n\
         \x20   fn show(self) -> str { \"x\" }\n\
         }\n\
         fn main() {\n\
         }\n",
        "for every element type",
    );
}
