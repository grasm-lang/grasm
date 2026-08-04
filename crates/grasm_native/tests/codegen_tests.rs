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
    let e = compile("fn main() {\n    let xs = [1, 2]\n    print(\"{xs.len()}\")\n}\n")
        .expect_err("lists unsupported in N1");
    assert!(e.contains("does not yet support"), "got: {e}");
    let e2 = compile("type T = { x: i64 }\n\nfn main() {\n}\n")
        .expect_err("types unsupported in N1");
    assert!(e2.contains("does not yet support"), "got: {e2}");
}
