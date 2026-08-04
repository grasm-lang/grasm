//! N1 codegen tests: object emission for the scalar subset.

use std::collections::HashMap;

fn compile(src: &str) -> Result<Vec<u8>, String> {
    let map: HashMap<String, String> = [("main".to_string(), src.to_string())].into();
    let provider = move |name: &str| map.get(name).cloned();
    let program = grasm_sema::loader::load_with("main", "test.gr", &provider);
    assert!(program.diagnostics.is_empty());
    let analysis = grasm_sema::check::check_program(&program);
    assert!(analysis.diagnostics.is_empty(), "{:#?}", analysis.diagnostics);
    let tmp = std::env::temp_dir().join(format!(
        "grasm_cg_test_{}_{:p}.o",
        std::process::id(),
        &program as *const _
    ));
    let r = grasm_native::compile_object(&program, &tmp).map(|()| std::fs::read(&tmp).unwrap());
    let _ = std::fs::remove_file(&tmp);
    r
}

#[test]
fn scalar_program_compiles_to_object() {
    let obj = compile(
        "fn fib(n: i64) -> i64 {\n    if n <= 1 {\n        return n\n    }\n    fib(n - 1) + fib(n - 2)\n}\n\nfn main() {\n    print(\"{fib(10)}\")\n}\n",
    )
    .expect("should compile");
    assert!(obj.len() > 100, "object too small");
}

#[test]
fn loops_and_floats_compile() {
    compile(
        "fn main() {\n    var t = 0.0\n    for i in 0..10 {\n        t += 1.5\n    }\n    while t > 0.0 {\n        t -= 4.0\n    }\n    print(\"{t}\")\n}\n",
    )
    .expect("should compile");
}

#[test]
fn unsupported_features_reported_cleanly() {
    let e = compile("fn main() {\n    let f = |x| x\n    print(\"{f(1)}\")\n}\n")
        .expect_err("closures unsupported yet");
    assert!(e.contains("does not yet support"), "got: {e}");
    let e2 = compile("fn main() {\n    let t = (1, 2)\n    print(\"{t}\")\n}\n")
        .expect_err("tuples unsupported yet");
    assert!(e2.contains("does not yet support"), "got: {e2}");
}

#[test]
fn n2_strings_and_lists_compile() {
    compile(
        "fn shout(s: str) -> str {\n    s + \"!\"\n}\n\nfn main() {\n    let g = \"hi \" + \"there\"\n    print(shout(g))\n    var xs = [1, 2]\n    xs.push(3)\n    for x in xs {\n        print(\"{x}\")\n    }\n    print(\"{xs.len()} {xs[0]} {g.len()} {g == \"hi there\"}\")\n}\n",
    )
    .expect("should compile");
}

#[test]
fn n2_list_of_strings_compiles() {
    compile(
        "fn main() {\n    var ws = [\"a\", \"b\"]\n    ws.push(\"c\")\n    print(ws)\n}\n",
    )
    .expect("should compile");
}

#[test]
fn n3_structs_sums_match_compile() {
    compile(
        "type P = { x: i64, y: i64 }\n\ntype S = | A(v: i64) | B\n\nfn pick(s: S) -> i64 {\n    match s {\n        A(v) -> v\n        B -> 0\n    }\n}\n\nfn main() {\n    var p = P{ x: 1, y: 2 }\n    p.x = 5\n    print(\"{p.x} {pick(A(3))} {pick(B)} {p}\")\n}\n",
    )
    .expect("should compile");
}

#[test]
fn n3_fallibles_and_option_compile() {
    compile(
        "type E = | Bad(n: i64)\n\nfn f(b: i64) -> i64 ! E {\n    if b == 0 {\n        fail(Bad(b))\n    }\n    b\n}\n\nfn g(b: i64) -> i64 ! E {\n    let v = try f(b)\n    v + 1\n}\n\nfn main() {\n    match g(2) {\n        Ok(v) -> print(\"ok {v}\")\n        Err(Bad(n)) -> print(\"bad {n}\")\n        Err(e) -> print(\"other\")\n    }\n    let xs = [1, 2]\n    print(\"{xs.get(0) ?? 0} {Some(3).unwrap_or(0)}\")\n}\n",
    )
    .expect("should compile");
}

#[test]
fn n3_guards_and_literal_patterns_compile() {
    compile(
        "fn label(n: i64) -> str {\n    match n {\n        0 -> \"zero\"\n        x if x < 0 -> \"neg\"\n        _ -> \"pos\"\n    }\n}\n\nfn main() {\n    print(label(0) + label(1))\n}\n",
    )
    .expect("should compile");
}

#[test]
fn n4_maps_and_methods_compile() {
    compile(
        "fn main() {\n    var m = map()\n    m.set(\"a\", 1)\n    print(\"{m.get(\"a\") ?? 0} {m.has(\"a\")} {m.len()} {m.keys()} {m.values()}\")\n    let s = \" Hi \"\n    print(\"{s.trim().to_upper()} {s.contains(\"H\")} {\"7\".parse_int() ?? 0}\")\n    var xs = [3, 1]\n    xs.sort()\n    print(\"{xs.rev()} {xs.first() ?? 0} {(0 - 2).abs()} {(4.0).sqrt()}\")\n}\n",
    )
    .expect("should compile");
}

#[test]
fn n4_map_filter_inline_closures_compile() {
    compile(
        "fn main() {\n    let xs = [1, 2, 3]\n    print(\"{xs.map(|x| x * 2)} {xs.filter(|x| x > 1)}\")\n}\n",
    )
    .expect("should compile");
}

#[test]
fn n4_closure_outside_map_filter_reported() {
    let e = compile("fn main() {\n    let f = |x| x + 1\n    print(\"{f(1)}\")\n}\n")
        .expect_err("first-class closures not supported yet");
    assert!(e.contains("closures outside"), "got: {e}");
}

#[test]
fn n4_annotated_empty_literals_compile() {
    compile(
        "fn main() {\n    var xs: [i64] = []\n    xs.push(1)\n    var m: Map[str, i64] = map()\n    m.set(\"k\", 1)\n    print(\"{xs.len()} {m.len()}\")\n}\n",
    )
    .expect("should compile");
}
