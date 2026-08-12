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
    // Both of these used to be rejected and now compile; what is still worth
    // checking is that a genuine gap is reported as one rather than crashing.
    compile("fn main() {\n    let f = |x| x\n    print(\"{f(1)}\")\n}\n")
        .expect("first-class closures compile");
    compile(
        "fn twice(x: i64) -> i64 {\n    x * 2\n}\n\nfn main() {\n    let fs = [twice]\n    print(\"{fs.len()}\")\n}\n",
    )
    .expect("function values compile");

    // An empty literal nothing determines is now a *checker* error rather
    // than a backend refusal, which is what [T-EMPTY-LIT] says it is — and it
    // removes a divergence, since `grasm run` used to accept it. So it is the
    // checker that has to report it, before the backend sees the program.
    let src = "fn main() {\n    var xs = []\n    print(\"{xs.len()}\")\n}\n";
    let map: HashMap<String, String> = [("main".to_string(), src.to_string())].into();
    let provider = move |name: &str| map.get(name).cloned();
    let program = grasm_sema::loader::load_with("main", "test.gr", &provider);
    let diags = grasm_sema::check::check_program(&program).diagnostics;
    assert!(
        diags.iter().any(|d| d.message.contains("not determined by anything")),
        "got: {diags:#?}"
    );
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
fn a_closure_outside_map_and_filter_compiles() {
    // This used to be rejected: closures were only ever inlined into the
    // methods that take one. They are now lifted to real functions.
    compile("fn main() {\n    let f = |x| x + 1\n    print(\"{f(1)}\")\n}\n")
        .expect("a first-class closure should compile");
}

#[test]
fn n4_annotated_empty_literals_compile() {
    compile(
        "fn main() {\n    var xs: [i64] = []\n    xs.push(1)\n    var m: Map[str, i64] = map()\n    m.set(\"k\", 1)\n    print(\"{xs.len()} {m.len()}\")\n}\n",
    )
    .expect("should compile");
}

#[test]
fn n5_effects_compile() {
    compile(
        "effect Clock {\n    fn now() -> i64\n}\n\nfn late(t: i64) -> bool {\n    Clock.now() > t\n}\n\nfn main() {\n    with Clock handled_by ClockHandler{ now: || 100 } {\n        print(\"{late(50)}\")\n    }\n}\n",
    )
    .expect("should compile");
}

#[test]
fn n5_inout_and_own_compile() {
    compile(
        "fn bump(inout n: i64) {\n    n += 1\n}\n\nfn add(inout xs: [i64], v: i64) {\n    xs.push(v)\n}\n\nfn eat(own s: str) -> i64 {\n    s.len()\n}\n\nfn main() {\n    var n = 0\n    bump(n)\n    var xs = [1]\n    add(xs, 2)\n    print(\"{n} {xs.len()} {eat(\"abc\")}\")\n}\n",
    )
    .expect("should compile");
}

#[test]
fn n5_fs_and_args_compile() {
    compile(
        "fn main() ! IoError {\n    let p = \"/tmp/grasm_cg.txt\"\n    try write_file(p, \"x\")\n    let c = try read_file(p)\n    print(\"{c.len()} {file_exists(p)} {args().len()}\")\n}\n",
    )
    .expect("should compile");
}

/// Cross-target object emission must work for every architecture we claim.
/// (Linking needs a target toolchain; see scripts/crosstest.sh for that.)
#[test]
fn emits_objects_for_all_supported_targets() {
    use std::collections::HashMap;
    let src = "fn main() {\n    print(\"portable\")\n}\n";
    let map: HashMap<String, String> = [("main".to_string(), src.to_string())].into();
    let provider = move |name: &str| map.get(name).cloned();
    let program = grasm_sema::loader::load_with("main", "t.gr", &provider);
    assert!(grasm_sema::check::check_program(&program)
        .diagnostics
        .is_empty());

    for target in [
        "x86_64-unknown-linux-gnu",
        "aarch64-unknown-linux-gnu",
        "riscv64gc-unknown-linux-gnu",
        "s390x-unknown-linux-gnu",
        "x86_64-apple-darwin",
    ] {
        let tmp = std::env::temp_dir().join(format!(
            "grasm_xtarget_{}_{}.o",
            std::process::id(),
            target
        ));
        let r = grasm_native::compile_object_for(&program, &tmp, Some(target));
        assert!(r.is_ok(), "target {target} failed: {:?}", r.err());
        let bytes = std::fs::read(&tmp).unwrap();
        assert!(bytes.len() > 100, "{target} produced a tiny object");
        let _ = std::fs::remove_file(&tmp);
    }
}

#[test]
fn unknown_target_is_reported_clearly() {
    use std::collections::HashMap;
    let map: HashMap<String, String> =
        [("main".to_string(), "fn main() {\n}\n".to_string())].into();
    let provider = move |name: &str| map.get(name).cloned();
    let program = grasm_sema::loader::load_with("main", "t.gr", &provider);
    let tmp = std::env::temp_dir().join("grasm_bad_target.o");
    let e = grasm_native::compile_object_for(&program, &tmp, Some("banana-unknown-none"))
        .expect_err("bogus target must fail");
    assert!(e.contains("unsupported target"), "got: {e}");
}

// ---------------------------------------------------------------------------
// Structured concurrency
// ---------------------------------------------------------------------------

#[test]
fn parallel_and_spawn_compile() {
    compile(
        "fn main() {\n    var log: [str] = []\n    parallel {\n        spawn {\n            log.push(\"a\")\n        }\n        spawn {\n            log.push(\"b\")\n        }\n    }\n    print(log.join(\" \"))\n}\n",
    )
    .expect("structured concurrency should compile");
}

#[test]
fn a_spawn_site_inside_a_loop_compiles() {
    // The site is lowered once but may fire many times, so the emitted code
    // records indices rather than unrolling.
    compile(
        "fn main() {\n    var hits: [str] = []\n    var i = 0\n    parallel {\n        while i < 3 {\n            spawn {\n                hits.push(\"x\")\n            }\n            i = i + 1\n        }\n    }\n    print(\"{hits.len()}\")\n}\n",
    )
    .expect("a dynamic spawn count should compile");
}

#[test]
fn nested_parallel_scopes_compile() {
    compile(
        "fn main() {\n    var log: [str] = []\n    parallel {\n        spawn {\n            log.push(\"outer\")\n            parallel {\n                spawn {\n                    log.push(\"inner\")\n                }\n            }\n        }\n    }\n    print(log.join(\" \"))\n}\n",
    )
    .expect("nested scopes should compile");
}

#[test]
fn an_empty_parallel_scope_compiles() {
    // No spawn sites means no dispatch loop to emit; the block still has to
    // produce valid code.
    compile("fn main() {\n    parallel {\n    }\n    print(\"done\")\n}\n")
        .expect("an empty scope should compile");
}

#[test]
fn a_return_inside_a_task_compiles() {
    compile(
        "fn pick() -> i64 {\n    parallel {\n        spawn {\n            return 7\n        }\n    }\n    0\n}\n\nfn main() {\n    print(\"{pick()}\")\n}\n",
    )
    .expect("a task body may return from its function");
}

// ---------------------------------------------------------------------------
// Generic functions (monomorphized)
// ---------------------------------------------------------------------------

#[test]
fn a_generic_function_compiles_per_instantiation() {
    compile(
        "fn first[T](xs: [T]) -> Option[T] {\n    xs.get(0)\n}\n\nfn main() {\n    print(\"{first([1, 2]) ?? 0} {first([\"a\"]) ?? \"none\"}\")\n}\n",
    )
    .expect("two instantiations of one generic should compile");
}

#[test]
fn a_generic_with_several_parameters_compiles() {
    compile(
        "fn pair[A, B](a: A, b: B) -> str {\n    \"{a}/{b}\"\n}\n\nfn main() {\n    print(pair(1, \"x\"))\n    print(pair(true, 2.5))\n}\n",
    )
    .expect("multiple type parameters should compile");
}

#[test]
fn a_generic_calling_a_generic_compiles() {
    // Instantiating `twice` discovers `wrap`, so the worklist has to keep
    // draining rather than being walked once.
    compile(
        "fn wrap[T](x: T) -> [T] {\n    [x]\n}\n\nfn twice[T](x: T) -> [T] {\n    let a = wrap(x)\n    [a[0], a[0]]\n}\n\nfn main() {\n    print(\"{twice(4)}\")\n}\n",
    )
    .expect("a generic reached only through another generic should compile");
}

#[test]
fn an_unused_generic_needs_no_instantiation() {
    // A generic that is never called has no concrete types, so there is
    // nothing to emit — and that must not be an error.
    compile(
        "fn unused[T](x: T) -> T {\n    x\n}\n\nfn main() {\n    print(\"fine\")\n}\n",
    )
    .expect("an uncalled generic should not need a symbol");
}

#[test]
fn a_type_parameter_the_arguments_do_not_determine_is_reported() {
    // `T` appears only in the return type, so there is nothing to infer from
    // the call. Guessing would emit the wrong function.
    let err = compile(
        "fn make[T]() -> Option[T] {\n    None\n}\n\nfn main() {\n    print(\"{make().is_none()}\")\n}\n",
    )
    .expect_err("should be reported, not guessed");
    assert!(
        err.contains("not determined by the arguments"),
        "expected a clear reason, got: {err}"
    );
}

// ---------------------------------------------------------------------------
// Function values
// ---------------------------------------------------------------------------

#[test]
fn a_named_function_used_as_a_value_compiles() {
    compile(
        "fn inc(n: i64) -> i64 {\n    n + 1\n}\n\nfn main() {\n    let f = inc\n    print(\"{f(3)}\")\n}\n",
    )
    .expect("a named function should be usable as a value");
}

#[test]
fn a_list_of_functions_compiles() {
    // What the router needs: functions in a list, iterated, called indirectly.
    compile(
        "fn inc(n: i64) -> i64 {\n    n + 1\n}\n\nfn dbl(n: i64) -> i64 {\n    n * 2\n}\n\nfn main() {\n    let fs = [inc, dbl]\n    for g in fs {\n        print(\"{g(10)}\")\n    }\n}\n",
    )
    .expect("a list of function values should compile");
}

#[test]
fn a_closure_bound_to_a_name_compiles() {
    compile("fn main() {\n    let c = |x| x + 100\n    print(\"{c(1)}\")\n}\n")
        .expect("a closure used as a value should compile");
}

#[test]
fn a_closure_capturing_its_environment_compiles() {
    compile(
        "fn main() {\n    let base = 10\n    let add = |n| n + base\n    print(\"{add(5)}\")\n}\n",
    )
    .expect("captures should compile");
}

#[test]
fn a_closure_argument_declared_after_it_still_types() {
    // The call names `i`, which is declared *after* the closure. The closure
    // is lowered first, so its parameter type has to come from the binding's
    // initializer rather than the binding itself.
    compile(
        "fn main() {\n    let f = |x| x + 1\n    var i = 0\n    print(\"{f(i)}\")\n}\n",
    )
    .expect("a later binding should still give the parameter its type");
}

#[test]
fn a_function_value_printed_shows_what_it_is() {
    compile(
        "fn inc(n: i64) -> i64 {\n    n + 1\n}\n\nfn main() {\n    let f = inc\n    print(\"{f}\")\n}\n",
    )
    .expect("printing a function value should compile");
}

#[test]
fn an_uncalled_closure_with_parameters_is_reported() {
    // Nothing says what `x` is, so the parameter type has nowhere to come
    // from. Guessing would compile the wrong function.
    let err = compile(
        "fn main() {\n    let f = |x| x + 1\n    print(\"{f}\")\n}\n",
    )
    .expect_err("should be reported, not guessed");
    assert!(
        err.contains("nothing to come from")
            || err.contains("no type says what its parameters are"),
        "expected a clear reason, got: {err}"
    );
}

// ---------------------------------------------------------------------------
// par_map
// ---------------------------------------------------------------------------

#[test]
fn par_map_compiles_to_the_threaded_path() {
    compile("fn main() {\n    let xs = [1, 2, 3]\n    print(\"{xs.par_map(|x| x * x)}\")\n}\n")
        .expect("scalar elements and a scalar result should compile");
}

#[test]
fn par_map_with_a_scalar_capture_compiles() {
    // Captures travel as raw bits copied per worker, so nothing refcounted
    // ever crosses a thread.
    compile(
        "fn main() {\n    let k = 3\n    let xs = [1, 2]\n    print(\"{xs.par_map(|x| x * k)}\")\n}\n",
    )
    .expect("a scalar capture should compile");
}

#[test]
fn par_map_over_boxed_elements_falls_back_rather_than_failing() {
    // Strings cannot cross a thread safely with this representation, so the
    // sequential lowering is used. Slower, never wrong.
    compile(
        "fn main() {\n    let ws = [\"a\", \"bb\"]\n    print(\"{ws.par_map(|w| w.len())}\")\n}\n",
    )
    .expect("boxed elements should fall back, not error");
}

#[test]
fn chained_par_maps_get_distinct_symbols() {
    // Both method calls start at the same source offset, so naming the lifted
    // functions after the span produced a duplicate-symbol link error.
    compile(
        "fn main() {\n    let xs = [1, 2, 3]\n    print(\"{xs.par_map(|x| x + 1).par_map(|y| y * 2)}\")\n}\n",
    )
    .expect("chained par_maps should compile");
}

#[test]
fn par_filter_compiles_to_the_threaded_path() {
    compile("fn main() {\n    let xs = [1, 2, 3]\n    print(\"{xs.par_filter(|x| x > 1)}\")\n}\n")
        .expect("scalar elements should take the parallel path");
}

#[test]
fn par_filter_over_boxed_elements_falls_back() {
    compile(
        "fn main() {\n    let ws = [\"a\", \"bb\"]\n    print(\"{ws.par_filter(|w| w.len() > 1)}\")\n}\n",
    )
    .expect("boxed elements should fall back, not error");
}
