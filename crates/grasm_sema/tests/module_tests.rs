//! Multi-file module system tests (in-memory provider).

use std::collections::HashMap;

fn analyze_program(files: &[(&str, &str)]) -> Vec<grasm_syntax::Diagnostic> {
    let map: HashMap<String, String> = files
        .iter()
        .map(|(n, s)| (n.to_string(), s.to_string()))
        .collect();
    let provider = move |name: &str| map.get(name).cloned();
    let program = grasm_sema::loader::load_with("main", "main.gr", &provider);
    let mut diags = program.diagnostics.clone();
    if diags.is_empty() {
        diags.extend(grasm_sema::check::check_program(&program).diagnostics);
    }
    diags
}

fn ok(files: &[(&str, &str)]) {
    let diags = analyze_program(files);
    assert!(diags.is_empty(), "unexpected diagnostics:\n{diags:#?}");
}

fn err(files: &[(&str, &str)], needle: &str) {
    let diags = analyze_program(files);
    assert!(
        diags.iter().any(|d| d.message.contains(needle)),
        "expected `{needle}`, got:\n{diags:#?}"
    );
}

#[test]
fn cross_module_calls() {
    ok(&[
        ("main", "import utils\n\nfn main() {\n    print(\"{utils.double(21)}\")\n}\n"),
        ("utils", "pub fn double(x: i64) -> i64 {\n    x * 2\n}\n"),
    ]);
}

#[test]
fn privacy_enforced() {
    err(
        &[
            ("main", "import utils\n\nfn main() {\n    print(\"{utils.secret()}\")\n}\n"),
            ("utils", "fn secret() -> i64 {\n    1\n}\n"),
        ],
        "is private",
    );
}

#[test]
fn missing_module_reported() {
    err(
        &[("main", "import nowhere\n\nfn main() {\n}\n")],
        "cannot load module `nowhere`",
    );
}

#[test]
fn missing_fn_in_module() {
    err(
        &[
            ("main", "import utils\n\nfn main() {\n    utils.nope()\n}\n"),
            ("utils", "pub fn yep() {\n}\n"),
        ],
        "has no function `nope`",
    );
}

#[test]
fn type_errors_cross_module() {
    err(
        &[
            ("main", "import utils\n\nfn main() {\n    let x: str = utils.double(2)\n}\n"),
            ("utils", "pub fn double(x: i64) -> i64 {\n    x * 2\n}\n"),
        ],
        "type mismatch",
    );
}

#[test]
fn same_fn_name_in_two_modules_ok() {
    ok(&[
        ("main", "import a\nimport b\n\nfn main() {\n    print(\"{a.get() + b.get()}\")\n}\n"),
        ("a", "pub fn get() -> i64 {\n    1\n}\n"),
        ("b", "pub fn get() -> i64 {\n    2\n}\n"),
    ]);
}

#[test]
fn dotted_imports_use_last_segment_alias() {
    ok(&[
        ("main", "import lib.math\n\nfn main() {\n    print(\"{math.sq(3)}\")\n}\n"),
        ("lib.math", "pub fn sq(x: i64) -> i64 {\n    x * x\n}\n"),
    ]);
}

#[test]
fn effects_flow_across_modules() {
    // logger.log_it performs io — main declares nothing, fine; but a
    // restricted-row fn calling across modules must still be caught.
    err(
        &[
            (
                "main",
                "import logger\n\nfn quiet() / {} {\n    logger.log_it()\n}\n\nfn main() {\n    quiet()\n}\n",
            ),
            ("logger", "pub fn log_it() {\n    print(\"hi\")\n}\n"),
        ],
        "not in its declared row",
    );
}

#[test]
fn diamond_imports_load_once() {
    ok(&[
        ("main", "import a\nimport b\n\nfn main() {\n    print(\"{a.fa() + b.fb()}\")\n}\n"),
        ("a", "import shared\n\npub fn fa() -> i64 {\n    shared.base()\n}\n"),
        ("b", "import shared\n\npub fn fb() -> i64 {\n    shared.base() * 2\n}\n"),
        ("shared", "pub fn base() -> i64 {\n    10\n}\n"),
    ]);
}

#[test]
fn duplicate_type_across_modules_rejected() {
    err(
        &[
            ("main", "import other\ntype Thing = { x: i64 }\n\nfn main() {\n}\n"),
            ("other", "pub type Thing = { y: i64 }\n"),
        ],
        "duplicate type name",
    );
}

/// An effect operation may name a type declared in another module.
///
/// Effects used to be registered in the same pass as type declarations, so
/// whether this worked depended on which file the loader happened to hand
/// over first. The error was `unknown type` with a help saying to declare it
/// with `type` — which then said `duplicate type name`. Two rules, and no way
/// to satisfy both.
#[test]
fn an_effect_operation_may_name_a_type_from_another_module() {
    ok(&[
        (
            "main",
            "import lib\n\n\
             effect Docs {\n\
             \x20   fn take(d: Diag) -> str\n\
             }\n\n\
             fn describe() -> str / {Docs} {\n\
             \x20   Docs.take(Diag{ msg: \"hello\" })\n\
             }\n\n\
             fn main() {\n\
             \x20   with Docs handled_by DocsHandler{ take: |d| \"got {d.msg}\" } {\n\
             \x20       print(\"{describe()}\")\n\
             \x20   }\n\
             }\n",
        ),
        ("lib", "pub type Diag = { msg: str }\n"),
    ]);
}

/// The same, with the operation's *error* type coming from another module.
#[test]
fn an_effect_operations_error_type_may_come_from_another_module() {
    ok(&[
        (
            "main",
            "import lib\n\n\
             effect Store {\n\
             \x20   fn read(k: str) -> str ! Missing\n\
             }\n\n\
             fn main() {\n\
             \x20   print(\"ok\")\n\
             }\n",
        ),
        ("lib", "pub type Missing = { key: str }\n"),
    ]);
}
