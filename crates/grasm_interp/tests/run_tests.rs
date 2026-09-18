//! End-to-end interpreter tests: source in, stdout + exit code out.

/// The program's output, with a panic or unhandled `main` error appended.
///
/// Those go to stderr now ([E-PANIC-EXIT]), so a test that wants to read one
/// asks for it rather than finding it mixed into what the program printed.
fn run(src: &str) -> (i32, String) {
    let mut out = Vec::new();
    let (code, fault, diags) = grasm_interp::run_src_capturing_fault(src, &mut out);
    assert!(
        diags.is_empty(),
        "unexpected diagnostics:\n{diags:#?}\nfor:\n{src}"
    );
    let mut text = String::from_utf8(out).unwrap();
    if let Some(f) = fault {
        text.push_str(&f);
        text.push('\n');
    }
    (code, text)
}

fn run_expect(src: &str, expected_out: &str) {
    let (code, out) = run(src);
    assert_eq!(code, 0, "non-zero exit; output:\n{out}");
    assert_eq!(out, expected_out, "wrong output for:\n{src}");
}

#[test]
fn hello_world() {
    run_expect(
        "fn main() {\n    print(\"hello, world\")\n}\n",
        "hello, world\n",
    );
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

// ---------------------------------------------------------------------------
// Phase 5: effects at runtime
// ---------------------------------------------------------------------------

#[test]
fn effect_handler_dispatch() {
    run_expect(
        r#"effect Clock {
    fn now() -> i64
}

fn is_late(t: i64) -> bool {
    Clock.now() > t
}

fn main() {
    with Clock handled_by ClockHandler{ now: || 100 } {
        print("{is_late(50)} {is_late(200)}")
    }
}
"#,
        "true false\n",
    );
}

#[test]
fn innermost_handler_wins() {
    run_expect(
        r#"effect Clock {
    fn now() -> i64
}

fn main() {
    with Clock handled_by ClockHandler{ now: || 1 } {
        print("{Clock.now()}")
        with Clock handled_by ClockHandler{ now: || 2 } {
            print("{Clock.now()}")
        }
        print("{Clock.now()}")
    }
}
"#,
        "1\n2\n1\n",
    );
}

#[test]
fn effect_ops_with_params() {
    run_expect(
        r#"effect Greeter {
    fn greet(name: str) -> str
}

fn welcome(who: str) -> str {
    Greeter.greet(who)
}

fn main() {
    with Greeter handled_by GreeterHandler{ greet: |n| "hello, " + n } {
        print(welcome("ada"))
    }
    with Greeter handled_by GreeterHandler{ greet: |n| n.to_upper() + "!" } {
        print(welcome("ada"))
    }
}
"#,
        "hello, ada\nADA!\n",
    );
}

#[test]
fn handler_swapping_enables_testing_pattern() {
    // The DI-without-frameworks pattern from the master plan.
    run_expect(
        r#"effect Rand {
    fn next() -> i64
}

fn roll() -> i64 {
    Rand.next() % 6 + 1
}

fn main() {
    with Rand handled_by RandHandler{ next: || 3 } {
        print("{roll()}")
    }
    with Rand handled_by RandHandler{ next: || 10 } {
        print("{roll()}")
    }
}
"#,
        "4\n5\n",
    );
}

// ---------------------------------------------------------------------------
// Phase 6: ownership at runtime — inout copy-in/copy-out
// ---------------------------------------------------------------------------

#[test]
fn inout_writeback_scalar() {
    run_expect(
        "fn bump(inout x: i64) {\n    x += 1\n}\n\nfn main() {\n    var n = 41\n    bump(n)\n    print(\"{n}\")\n}\n",
        "42\n",
    );
}

#[test]
fn inout_writeback_list_and_field() {
    run_expect(
        r#"type Bag = { items: [i64] }

fn add_item(inout xs: [i64], v: i64) {
    xs.push(v)
}

fn main() {
    var bag = Bag{ items: [1] }
    add_item(bag.items, 2)
    add_item(bag.items, 3)
    print("{bag.items}")
}
"#,
        "[1, 2, 3]\n",
    );
}

#[test]
fn inout_ufcs_receiver() {
    run_expect(
        "fn double_all(inout xs: [i64]) {\n    var i = 0\n    while i < xs.len() {\n        xs[i] *= 2\n        i += 1\n    }\n}\n\nfn main() {\n    var xs = [1, 2, 3]\n    xs.double_all()\n    print(\"{xs}\")\n}\n",
        "[2, 4, 6]\n",
    );
}

#[test]
fn let_params_still_copy() {
    // Non-inout param mutations must NOT leak to the caller.
    run_expect(
        "fn tweak(xs: [i64]) -> i64 {\n    xs.len()\n}\n\nfn main() {\n    var a = [1, 2]\n    let n = tweak(a)\n    print(\"{a.len()} {n}\")\n}\n",
        "2 2\n",
    );
}

// ---------------------------------------------------------------------------
// Phase 7: stdlib — fs, args, new str/list methods
// ---------------------------------------------------------------------------

#[test]
fn fs_roundtrip_with_args() {
    let dir = std::env::temp_dir().join("grasm_test_fs");
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("note.txt");
    let path_str = path.to_string_lossy().to_string();
    let src = r#"fn main() ! IoError {
    let path = args().get(0) ?? "missing"
    try write_file(path, "line one\nline two")
    let content = try read_file(path)
    print("{content.split("\n").len()} lines")
    print("{file_exists(path)} {file_exists(path + ".nope")}")
}
"#;
    let mut out = Vec::new();
    let (code, diags) = grasm_interp::run_src_with_args(src, &[path_str], &mut out);
    assert!(diags.is_empty(), "{diags:#?}");
    assert_eq!(code, 0, "output: {}", String::from_utf8_lossy(&out));
    assert_eq!(String::from_utf8(out).unwrap(), "2 lines\ntrue false\n");
}

#[test]
fn read_missing_file_is_notfound() {
    run_expect(
        r#"fn main() {
    match read_file("/definitely/not/here.txt") {
        Ok(c) -> print("found?")
        Err(NotFound(p)) -> print("not found")
        Err(IoFailed(m)) -> print("io failed")
    }
}
"#,
        "not found\n",
    );
}

#[test]
fn new_string_methods() {
    run_expect(
        r#"fn main() {
    let s = "hello world"
    print(s.replace("world", "grasm"))
    print(s.substr(0, 5))
    print("{s.index_of("world") ?? 0 - 1}")
    print("{s.index_of("zzz").is_none()}")
}
"#,
        "hello grasm\nhello\n6\ntrue\n",
    );
}

#[test]
fn new_list_methods() {
    run_expect(
        r#"fn main() {
    let xs = [10, 20, 30, 40]
    print("{xs.slice(1, 3)}")
    print("{xs.first() ?? 0} {xs.last() ?? 0}")
    let empty: [i64] = []
    print("{empty.first().is_none()}")
}
"#,
        "[20, 30]\n10 40\ntrue\n",
    );
}

// ---------------------------------------------------------------------------
// Phase 2a: maps
// ---------------------------------------------------------------------------

#[test]
fn map_basics() {
    run_expect(
        r#"fn main() {
    var ages = map()
    ages.set("ada", 36)
    ages.set("alan", 41)
    ages.set("ada", 37)
    print("{ages.len()} {ages.get("ada") ?? 0} {ages.get("zzz") ?? -1}")
    print("{ages.has("alan")} {ages.delete("alan")} {ages.has("alan")}")
    print("{ages}")
}
"#,
        "2 37 -1\ntrue true false\n{ada: 37}\n",
    );
}

#[test]
fn map_keys_values_and_iteration() {
    run_expect(
        r#"fn main() {
    var m = map()
    m.set(3, "three")
    m.set(1, "one")
    m.set(2, "two")
    print(m.keys().map(|k| k.to_str()).join(","))
    print(m.values().join(","))
    var total = 0
    for k in m.keys() {
        total += k
    }
    print("{total}")
}
"#,
        "1,2,3\none,two,three\n6\n",
    );
}

#[test]
fn map_typed_annotation_and_word_count() {
    // The validation-round pain point, now idiomatic.
    run_expect(
        r#"fn word_counts(text: str) -> Map[str, i64] {
    var counts: Map[str, i64] = map()
    for w in text.split(" ").filter(|w| w.len() > 0) {
        counts.set(w, (counts.get(w) ?? 0) + 1)
    }
    counts
}

fn main() {
    let c = word_counts("a b a c a b")
    print("{c.get("a") ?? 0} {c.get("b") ?? 0} {c.get("c") ?? 0}")
}
"#,
        "3 2 1\n",
    );
}

#[test]
fn map_value_semantics() {
    run_expect(
        r#"fn main() {
    var a = map()
    a.set("k", 1)
    var b = a
    b.set("k", 2)
    print("{a.get("k") ?? 0} {b.get("k") ?? 0}")
}
"#,
        "1 2\n",
    );
}

#[test]
fn map_nested_in_struct() {
    run_expect(
        r#"type Cache = { entries: Map[str, i64] }

fn main() {
    var c = Cache{ entries: map() }
    c.entries.set("hits", 10)
    c.entries.set("misses", 2)
    print("{c.entries.get("hits") ?? 0} {c.entries.len()}")
}
"#,
        "10 2\n",
    );
}

// ---------------------------------------------------------------------------
// C3: structured concurrency (deterministic scheduling, not parallelism)
// ---------------------------------------------------------------------------

#[test]
fn parallel_joins_every_spawned_task() {
    run_expect(
        r#"fn main() {
    var xs: [i64] = []
    parallel {
        spawn {
            xs.push(1)
        }
        spawn {
            xs.push(2)
        }
        spawn {
            xs.push(3)
        }
    }
    print("{xs} {xs.len()}")
}
"#,
        "[1, 2, 3] 3\n",
    );
}

#[test]
fn tasks_run_in_spawn_order_deterministically() {
    // Same program, same output, every run — the scheduler is deterministic.
    let src = r#"fn main() {
    var log: [str] = []
    parallel {
        spawn {
            log.push("a")
        }
        spawn {
            log.push("b")
        }
        spawn {
            log.push("c")
        }
    }
    print(log.join(""))
}
"#;
    for _ in 0..5 {
        run_expect(src, "abc\n");
    }
}

#[test]
fn spawned_tasks_see_enclosing_state() {
    run_expect(
        r#"fn main() {
    let base = 10
    var total = 0
    parallel {
        spawn {
            total += base
        }
        spawn {
            total += base * 2
        }
    }
    print("{total}")
}
"#,
        "30\n",
    );
}

#[test]
fn nested_parallel_scopes_join_inside_out() {
    run_expect(
        r#"fn main() {
    var log: [str] = []
    parallel {
        spawn {
            log.push("outer-start")
            parallel {
                spawn {
                    log.push("inner")
                }
            }
            log.push("outer-end")
        }
    }
    print(log.join(" "))
}
"#,
        "outer-start inner outer-end\n",
    );
}

#[test]
fn a_panicking_task_aborts_the_scope() {
    let (code, out) = run(
        "fn main() {\n    parallel {\n        spawn {\n            panic(\"task failed\")\n        }\n        spawn {\n            print(\"never runs\")\n        }\n    }\n}\n",
    );
    assert_eq!(code, 101);
    assert!(out.contains("task failed"), "output: {out}");
    assert!(!out.contains("never runs"), "later tasks must not run");
}

#[test]
fn empty_parallel_scope_is_a_no_op() {
    run_expect(
        "fn main() {\n    parallel {\n    }\n    print(\"done\")\n}\n",
        "done\n",
    );
}

// ---------------------------------------------------------------------------
// Standard library: searching, extremes, string shaping, OS access
// ---------------------------------------------------------------------------

#[test]
fn any_all_and_find_search_a_list() {
    run_expect(
        r#"fn main() {
    let xs = [3, 1, 4, 1, 5]
    print("{xs.any(|x| x > 4)} {xs.any(|x| x > 9)}")
    print("{xs.all(|x| x > 0)} {xs.all(|x| x > 3)}")
    print("{xs.find(|x| x > 3) ?? -1} {xs.find(|x| x > 99) ?? -1}")
}
"#,
        "true false\ntrue false\n4 -1\n",
    );
}

#[test]
fn any_and_all_short_circuit() {
    // The predicate stops running once the answer is settled, so `any` prints
    // only up to its first hit and `all` only up to its first miss.
    run_expect(
        r#"fn main() {
    let xs = [1, 2, 3, 4]
    let hit = xs.any(|x| {
        print("any saw {x}")
        x == 2
    })
    print("{hit}")
    let ok = xs.all(|x| {
        print("all saw {x}")
        x < 3
    })
    print("{ok}")
}
"#,
        "any saw 1\nany saw 2\ntrue\nall saw 1\nall saw 2\nall saw 3\nfalse\n",
    );
}

#[test]
fn find_on_an_empty_list_is_none() {
    run_expect(
        "fn main() {\n    let xs: [i64] = []\n    print(\"{xs.find(|x| x > 0).is_none()} {xs.any(|x| x > 0)} {xs.all(|x| x > 0)}\")\n}\n",
        "true false true\n",
    );
}

#[test]
fn min_and_max_return_option() {
    run_expect(
        r#"fn main() {
    print("{[3, 1, 4].min() ?? 0} {[3, 1, 4].max() ?? 0}")
    print("{["pear", "fig"].min() ?? "?"} {["pear", "fig"].max() ?? "?"}")
    let none: [i64] = []
    print("{none.min().is_none()} {none.max().is_none()}")
}
"#,
        "1 4\nfig pear\ntrue true\n",
    );
}

#[test]
fn is_empty_on_lists_strings_and_maps() {
    run_expect(
        r#"fn main() {
    let none: [i64] = []
    print("{none.is_empty()} {[1].is_empty()}")
    print("{"".is_empty()} {"x".is_empty()}")
    var m = map()
    print("{m.is_empty()}")
    m.set("k", 1)
    print("{m.is_empty()}")
}
"#,
        "true false\ntrue false\ntrue\nfalse\n",
    );
}

#[test]
fn lines_splits_on_newlines() {
    run_expect(
        r#"fn main() {
    let doc = "alpha\nbeta\ngamma"
    print("{doc.lines().len()} {doc.lines()[1]}")
    print("{"solo".lines().len()} {"".lines().len()}")
    // A trailing newline does not produce a phantom final line.
    print("{"a\nb\n".lines().len()}")
}
"#,
        "3 beta\n1 0\n2\n",
    );
}

#[test]
fn padding_widens_to_a_target_length() {
    run_expect(
        r#"fn main() {
    print("[{"7".pad_start(3, "0")}] [{"ab".pad_end(5, ".")}]")
    // Already at or over the width, the string is returned unchanged.
    print("[{"wide".pad_start(2, "-")}] [{"four".pad_end(4, "-")}]")
    // Multi-character fill repeats and is cut to fit.
    print("[{"x".pad_start(5, "ab")}]")
}
"#,
        "[007] [ab...]\n[wide] [four]\n[ababx]\n",
    );
}

#[test]
fn now_millis_moves_forward() {
    run_expect(
        "fn main() {\n    let a = now_millis()\n    print(\"{a > 1577836800000}\")\n}\n",
        "true\n",
    );
}

#[test]
fn env_reads_the_process_environment() {
    // Only the absent case is deterministic without touching the harness's env.
    run_expect(
        "fn main() {\n    print(\"{env(\"GRASM_DEFINITELY_UNSET_VAR\") ?? \"unset\"}\")\n}\n",
        "unset\n",
    );
}

#[test]
fn exit_stops_the_program_with_a_status() {
    let (code, out) =
        run("fn main() {\n    print(\"before\")\n    exit(3)\n    print(\"after\")\n}\n");
    assert_eq!(code, 3);
    assert_eq!(out, "before\n");
}

#[test]
fn tasks_run_after_the_block_in_spawn_order() {
    run_expect(
        r#"fn main() {
    var log: [str] = []
    parallel {
        log.push("body-a")
        spawn {
            log.push("task-1")
        }
        log.push("body-b")
        spawn {
            log.push("task-2")
        }
    }
    print(log.join(" "))
}
"#,
        "body-a body-b task-1 task-2\n",
    );
}

#[test]
fn a_spawn_site_in_a_loop_fires_once_per_iteration() {
    run_expect(
        r#"fn main() {
    var hits: [str] = []
    var i = 0
    parallel {
        while i < 3 {
            spawn {
                hits.push("x")
            }
            i = i + 1
        }
    }
    print("{hits.len()}")
}
"#,
        "3\n",
    );
}

#[test]
fn a_conditional_spawn_only_fires_when_taken() {
    run_expect(
        r#"fn main() {
    var picked: [str] = []
    parallel {
        if 1 < 2 {
            spawn {
                picked.push("yes")
            }
        }
        if 2 < 1 {
            spawn {
                picked.push("no")
            }
        }
    }
    print(picked.join(","))
}
"#,
        "yes\n",
    );
}

#[test]
fn a_closure_captures_by_value_at_creation() {
    // A later change to the captured binding is not visible to the closure:
    // capture is a copy, like every other binding ([E-VALUE-SEMANTICS]).
    run_expect(
        "fn main() {\n    var n = 1\n    let f = |x| x + n\n    n = 100\n    print(\"{f(1)} {n}\")\n}\n",
        "2 100\n",
    );
}

#[test]
fn function_values_print_as_what_they_are() {
    run_expect(
        "fn inc(n: i64) -> i64 {\n    n + 1\n}\n\nfn main() {\n    let f = inc\n    let c = |x| x\n    print(\"{f}\")\n    print(\"{c(1)}\")\n}\n",
        "<fn main::inc>\n1\n",
    );
}

#[test]
fn par_filter_keeps_input_order() {
    // The output order is the input order, which is what makes a parallel
    // filter interchangeable with a sequential one.
    run_expect(
        "fn main() {\n    let xs = [5, 2, 8, 1, 9, 4]\n    print(\"{xs.par_filter(|x| x > 3)}\")\n}\n",
        "[5, 8, 9, 4]\n",
    );
}

#[test]
fn par_filter_matches_filter() {
    run_expect(
        "fn main() {\n    let xs = [1, 2, 3, 4, 5, 6, 7]\n    print(\"{xs.par_filter(|x| x % 3 == 1) == xs.filter(|x| x % 3 == 1)}\")\n}\n",
        "true\n",
    );
}
