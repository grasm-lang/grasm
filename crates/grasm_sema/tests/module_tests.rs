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
            ("other", "type Thing = { y: i64 }\n"),
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
        ("lib", "type Diag = { msg: str }\n"),
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
        ("lib", "type Missing = { key: str }\n"),
    ]);
}

/// An imported function can be used as a *value*, not only called.
///
/// `[1].map(util.dbl)` reported `unknown name \`util\`` — the message denying
/// the module exists while `util.dbl(1)` on the next line worked. Every
/// `map`/`sort_by`/`fold` over an imported function had to be wrapped in a
/// closure to get past it.
#[test]
fn an_imported_function_may_be_used_as_a_value() {
    ok(&[
        (
            "main",
            "import util\n\n\
             fn apply(f: fn(i64) -> i64, v: i64) -> i64 {\n\
             \x20   f(v)\n\
             }\n\n\
             fn main() {\n\
             \x20   print(\"{[1, 2].map(util.dbl)}\")\n\
             \x20   var xs = [1, 3, 2]\n\
             \x20   xs.sort_by(util.key)\n\
             \x20   print(\"{apply(util.dbl, 5)} {5 |> util.dbl}\")\n\
             }\n",
        ),
        (
            "util",
            "pub fn dbl(n: i64) -> i64 {\n    n * 2\n}\n\
             pub fn key(n: i64) -> i64 {\n    0 - n\n}\n",
        ),
    ]);
}

/// Privacy still applies to a function used as a value.
#[test]
fn a_private_function_may_not_be_used_as_a_value_either() {
    err(
        &[
            (
                "main",
                "import util\n\nfn main() {\n    print(\"{[1].map(util.hidden)}\")\n}\n",
            ),
            ("util", "fn hidden(n: i64) -> i64 {\n    n\n}\n"),
        ],
        "is private",
    );
}

/// A closure's body names things where the closure is *written*.
///
/// The interpreter resolved them in whatever module was calling, so
/// `engine.apply(|| helper.value())` panicked with `unknown name \`helper\``
/// for a program the compiled binary ran correctly — the reference
/// implementation being the wrong one, and `difftest.sh` unable to see it
/// because it takes two modules and a closure crossing between them.
#[test]
fn a_closure_resolves_names_where_it_was_written() {
    ok(&[
        (
            "main",
            "import engine\nimport helper\n\n\
             fn local_value() -> str {\n    \"local\"\n}\n\n\
             fn main() {\n\
             \x20   print(\"{engine.apply(|| helper.value())}\")\n\
             \x20   print(\"{engine.apply(|| local_value())}\")\n\
             }\n",
        ),
        (
            "engine",
            "pub fn apply(f: fn() -> str) -> str {\n    f()\n}\n",
        ),
        ("helper", "pub fn value() -> str {\n    \"from helper\"\n}\n"),
    ]);
}

/// A `type` declaration may name a type from another module.
///
/// Type *bodies* were resolved in the same pass that registers type names, so
/// whether a field could name a type came down to which file the loader
/// handed over first. It worked in function signatures and `let` annotations
/// and nowhere else — and the error said to declare it with `type`, naming
/// the thing that was already declared. The same ordering effects had.
#[test]
fn a_type_declaration_may_name_a_type_from_another_module() {
    ok(&[
        (
            "main",
            "import lib\n\n\
             type Wrap = { inner: Thing }\n\
             type Holder = | One(t: Thing) | None2\n\n\
             fn main() {\n\
             \x20   let w = Wrap{ inner: Thing{ n: 5 } }\n\
             \x20   print(\"{w.inner.n}\")\n\
             }\n",
        ),
        ("lib", "type Thing = { n: i64 }\n"),
    ]);
}

/// A *generic* imported function may be piped and passed as a value.
///
/// Its type parameters were left uninstantiated when it was used as a value,
/// so `xs |> lib.uniq` reported `expected [T0], found [str]` while
/// `lib.uniq(xs)` on the next line was fine. A local generic piped correctly
/// and a qualified non-generic passed correctly; only the combination broke.
#[test]
fn a_qualified_generic_may_be_piped_and_passed() {
    ok(&[
        (
            "main",
            "import lib\n\n\
             fn main() {\n\
             \x20   let n = lib.uniq([1, 1, 2]).len()\n\
             \x20   let piped = [1, 1] |> lib.uniq\n\
             \x20   let mapped = [[1, 1]].map(lib.uniq)\n\
             \x20   print(\"{n} {piped} {mapped}\")\n\
             }\n",
        ),
        (
            "lib",
            "pub fn uniq[T](xs: [T]) -> [T] {\n\
             \x20   var out: [T] = []\n\
             \x20   for x in xs {\n\
             \x20       if not out.contains(x) {\n\
             \x20           out.push(x)\n\
             \x20       }\n\
             \x20   }\n\
             \x20   out\n\
             }\n",
        ),
    ]);
}

/// `pub let` crosses a module boundary; a bare `let` does not.
///
/// `pub` on a constant parsed and checked and did nothing: no other module
/// could read one, and both `doc` and `api` omitted it. Syntax with no meaning
/// is worse than an error, because it looks like it worked.
#[test]
fn a_pub_constant_is_readable_from_another_module() {
    ok(&[
        (
            "main",
            "import lib\n\nfn main() {\n    print(\"{lib.WIDTH}\")\n}\n",
        ),
        ("lib", "pub let WIDTH = 42\n"),
    ]);
}

#[test]
fn a_private_constant_is_not() {
    err(
        &[
            (
                "main",
                "import lib\n\nfn main() {\n    print(\"{lib.HIDDEN}\")\n}\n",
            ),
            ("lib", "let HIDDEN = 7\npub fn w() -> i64 {\n    HIDDEN\n}\n"),
        ],
        "is private",
    );
}

/// A diagnostic in a dependency names a path that exists.
///
/// The loader derived a display path from the module name, which is right only
/// for a sibling file: a dependency's `liba.helper` became `liba/helper.gr`,
/// which exists nowhere — not relative to the working directory, not relative
/// to the manifest. Every editor and CI annotation that followed it landed on
/// nothing.
#[test]
fn a_diagnostic_in_a_dependency_names_a_real_file() {
    let root = std::env::temp_dir().join(format!("grasm_diagpath_{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
    let write = |p: std::path::PathBuf, s: &str| {
        std::fs::create_dir_all(p.parent().unwrap()).unwrap();
        std::fs::write(p, s).unwrap();
    };
    write(
        root.join("liba/grasm.toml"),
        "[package]\nname = \"liba\"\nversion = \"0.1.0\"\nentry = \"src/main.gr\"\ncapabilities = []\n",
    );
    write(
        root.join("liba/src/main.gr"),
        "import helper\n\npub fn go() -> str {\n    helper.bad()\n}\n",
    );
    write(root.join("liba/src/helper.gr"), "pub fn bad() -> str {\n    42\n}\n");
    write(
        root.join("app/grasm.toml"),
        "[package]\nname = \"app\"\nversion = \"0.1.0\"\nentry = \"src/main.gr\"\ncapabilities = [\"io\"]\n\n[dependencies.liba]\npath = \"../liba\"\nversion = \"^0.1.0\"\n",
    );
    write(
        root.join("app/src/main.gr"),
        "import liba\n\nfn main() {\n    print(\"{liba.go()}\")\n}\n",
    );

    let program = grasm_sema::loader::load_from_disk(&root.join("app/src/main.gr")).expect("loads");
    for f in &program.files {
        assert!(
            std::path::Path::new(&f.path).is_file(),
            "module `{}` reports path `{}`, which does not exist",
            f.name,
            f.path
        );
    }
    let _ = std::fs::remove_dir_all(&root);
}

/// The end-of-analysis map-key pass is keyed by the callee, not the caller
/// ([T-MAP-KEY-GENERIC]).
///
/// A qualified call recorded the calling module, so `mk.tally([1.5])` over a
/// `Map[T, _]` in `mk` went through the checker and into a runtime
/// `invalid map key`, while the identical body called unqualified was refused.
#[test]
fn a_key_generic_is_checked_at_a_qualified_call_site() {
    let tally = "pub fn tally[T](xs: [T]) -> i64 {\n    var m: Map[T, i64] = {}\n    for x in xs {\n        m.set(x, 1)\n    }\n    m.len()\n}\n";
    err(
        &[
            ("main", "import mk\n\nfn main() {\n    print(\"{mk.tally([1.5, 2.5])}\")\n}\n"),
            ("mk", tally),
        ],
        "map keys must be",
    );
    ok(&[
        ("main", "import mk\n\nfn main() {\n    let n = mk.tally([\"a\"])\n    print(\"{n}\")\n}\n"),
        ("mk", tally),
    ]);
}

/// A call made from inside a region is refused however it is spelled
/// ([T-REGION-FOREIGN]).
///
/// The row was looked up under the *calling* module, so a qualified call
/// carried none and `region { lib.cfg_user() }` was accepted — the handler is
/// written outside the region and may keep what it is given.
#[test]
fn a_qualified_call_inside_a_region_is_refused() {
    let lib = "effect Cfg {\n    fn value() -> i64\n}\npub fn cfg_user() -> i64 / {Cfg} {\n    Cfg.value()\n}\npub fn plain(n: i64) -> i64 {\n    n\n}\n";
    err(
        &[
            ("main", "import lib\n\nfn main() {\n    with Cfg handled_by CfgHandler{ value: || 5 } {\n        let n = region {\n            lib.cfg_user()\n        }\n        print(\"{n}\")\n    }\n}\n"),
            ("lib", lib),
        ],
        "inside a region",
    );
    // A call that performs nothing is ordinary.
    ok(&[
        ("main", "import lib\n\nfn main() {\n    let n = region {\n        lib.plain(5)\n    }\n    print(\"{n}\")\n}\n"),
        ("lib", lib),
    ]);
}

/// Two modules that each declare a private error type with an `Io` variant.
///
/// Both compile alone. Before this, a program importing both broke *their*
/// sources — with an error naming a file the program's author does not own and
/// cannot fix, about a name that was never ambiguous inside the module that
/// declared it. A package became uncompilable because of what was standing
/// next to it, which is the opposite of what separate packages are for.
///
/// Found by writing a store: `sstable` and `wal` each had an `Io`.
#[test]
fn a_modules_own_variant_wins_over_an_imported_one() {
    let a = "type AError =\n    | Io(why: str)\n\npub fn go(n: i64) -> i64 ! AError {\n    if n < 0 {\n        fail(Io(\"negative\"))\n    }\n    n\n}\n";
    let b = "type BError =\n    | Io(why: str)\n\npub fn go(n: i64) -> i64 ! BError {\n    if n < 0 {\n        fail(Io(\"negative\"))\n    }\n    n\n}\n";
    let main = "import a\nimport b\n\nfn main() {\n    match a.go(1) {\n        Ok(v) -> print(\"{v}\")\n        Err(e) -> print(\"no\")\n    }\n    match b.go(2) {\n        Ok(v) -> print(\"{v}\")\n        Err(e) -> print(\"no\")\n    }\n}\n";
    ok(&[("main", main), ("a", a), ("b", b)]);

    // And the case that really is ambiguous still is: a module with no `Io` of
    // its own, looking at two.
    let caller = "import a\nimport b\n\nfn pick() -> i64 ! AError {\n    fail(Io(\"which?\"))\n}\n\nfn main() {\n    match pick() {\n        Ok(v) -> print(\"{v}\")\n        Err(e) -> print(\"no\")\n    }\n}\n";
    err(&[("main", caller), ("a", a), ("b", b)], "ambiguous");

    // Qualifying it is the way out, and still works.
    let fixed = caller.replace("fail(Io(", "fail(AError.Io(");
    ok(&[("main", &fixed), ("a", a), ("b", b)]);
}

/// A mutating call inside another call's argument list, where both write
/// through the same `inout` place.
///
/// The inner write happened and was then erased by the outer call's writeback,
/// which put back a snapshot taken before the inner call ran. Two engines said
/// `[100]` and the compiled backend said `[10, 100]` — the reference
/// implementation being the wrong one (F90).
///
/// Checked here as a *type* question only: the program is legal, and it is
/// `difftest` over `packages/gc-demo` that holds the three engines to one
/// answer. Found by writing a heap allocator, where
/// `alloc(h, Cons(alloc(h, ...), tail))` is the ordinary spelling.
#[test]
fn a_mutating_call_inside_another_calls_arguments_is_legal() {
    ok(&[(
        "main",
        "fn push_one(inout xs: [i64], v: i64) -> i64 {\n    xs.push(v)\n    xs.len() - 1\n}\n\nfn main() {\n    var xs: [i64] = []\n    let outer = push_one(xs, push_one(xs, 10) + 100)\n    print(\"{xs} {outer}\")\n}\n",
    )]);
}
