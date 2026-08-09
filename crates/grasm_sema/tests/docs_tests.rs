//! Documentation extraction: what `grasm doc` renders and what `grasm test`
//! lifts out of `///` comments to run.

use grasm_sema::docs;
use std::io::Write;

struct Fixture {
    dir: std::path::PathBuf,
}

impl Fixture {
    /// `files` is (module name, source); `main` is the entry.
    fn new(name: &str, files: &[(&str, &str)]) -> Fixture {
        let dir = std::env::temp_dir().join(format!("grasm_docs_{}_{}", name, std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        for (module, src) in files {
            let mut f = std::fs::File::create(dir.join(format!("{module}.gr"))).unwrap();
            f.write_all(src.as_bytes()).unwrap();
        }
        Fixture { dir }
    }

    fn program(&self) -> grasm_sema::loader::Program {
        let program =
            grasm_sema::loader::load_from_disk(&self.dir.join("main.gr")).expect("loads");
        assert!(
            program.diagnostics.is_empty(),
            "fixture failed to parse: {:#?}",
            program.diagnostics
        );
        program
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.dir);
    }
}

#[test]
fn signatures_come_from_the_ast_not_the_prose() {
    let fx = Fixture::new(
        "sig",
        &[(
            "main",
            "/// Reads a file.\nfn read(path: str) -> str ! str / {io} {\n    fail(\"nope\")\n}\n\nfn main() {\n}\n",
        )],
    );
    let modules = docs::collect(&fx.program());
    let item = modules[0]
        .items
        .iter()
        .find(|i| i.name == "read")
        .expect("read is documented");
    // The error type and the effect row are part of the signature, so they
    // cannot drift from the declaration the way hand-written prose would.
    assert_eq!(item.signature, "fn read(path: str) -> str ! str / {io}");
    assert_eq!(item.doc.as_deref(), Some("Reads a file."));
}

#[test]
fn struct_sum_and_effect_signatures_render() {
    let fx = Fixture::new(
        "shapes",
        &[(
            "main",
            "type Point = { x: f64, y: f64 }\n\ntype Shape =\n    | Circle(r: f64)\n    | Dot\n\neffect Clock {\n    fn now() -> i64\n}\n\nfn main() {\n}\n",
        )],
    );
    let modules = docs::collect(&fx.program());
    let sig = |name: &str| {
        modules[0]
            .items
            .iter()
            .find(|i| i.name == name)
            .unwrap_or_else(|| panic!("{name} missing"))
            .signature
            .clone()
    };
    assert_eq!(sig("Point"), "type Point = { x: f64, y: f64 }");
    // Variant field names are API, so they survive into the rendering. The
    // leading `|` is part of the grammar: what `grasm doc` prints has to parse
    // back in, and without it this did not.
    assert_eq!(sig("Shape"), "type Shape = | Circle(r: f64) | Dot");
    assert_eq!(sig("Clock"), "effect Clock { fn now() -> i64 }");
}

/// An operation's `! E` is part of its contract and must survive into the
/// rendering — dropping it made the documented signature disagree with the
/// real one, which is the drift the generator exists to prevent.
#[test]
fn an_effect_operations_error_type_is_documented() {
    let fx = Fixture::new(
        "diag",
        &[(
            "main",
            "type Diag = { why: str }

effect Docs {
    fn load(name: str) -> str ! Diag
    fn note(m: str)
}

fn main() {
}
",
        )],
    );
    let modules = docs::collect(&fx.program());
    let sig = modules[0]
        .items
        .iter()
        .find(|i| i.name == "Docs")
        .expect("Docs missing")
        .signature
        .clone();
    assert_eq!(
        sig,
        "effect Docs { fn load(name: str) -> str ! Diag; fn note(m: str) }"
    );
}

#[test]
fn private_items_of_an_imported_module_are_not_documented() {
    let fx = Fixture::new(
        "private",
        &[
            (
                "lib",
                "/// Exported.\npub fn shown() -> i64 {\n    1\n}\n\n/// Internal.\nfn hidden() -> i64 {\n    2\n}\n",
            ),
            ("main", "import lib\n\nfn main() {\n    print(\"{lib.shown()}\")\n}\n"),
        ],
    );
    let modules = docs::collect(&fx.program());
    let lib = modules
        .iter()
        .find(|m| m.module == "lib")
        .expect("lib is documented");
    let names: Vec<&str> = lib.items.iter().map(|i| i.name.as_str()).collect();
    assert_eq!(names, vec!["shown"]);
}

#[test]
fn the_entry_module_documents_its_own_private_items() {
    // A program's `main` file has no consumers, so `pub` there means nothing.
    let fx = Fixture::new(
        "entry",
        &[("main", "/// Helper.\nfn helper() -> i64 {\n    1\n}\n\nfn main() {\n}\n")],
    );
    let modules = docs::collect(&fx.program());
    assert!(modules[0].items.iter().any(|i| i.name == "helper"));
}

#[test]
fn markdown_contains_the_signature_and_the_prose() {
    let fx = Fixture::new(
        "md",
        &[("main", "/// Adds.\nfn add(a: i64, b: i64) -> i64 {\n    a + b\n}\n\nfn main() {\n}\n")],
    );
    let text = docs::render_markdown("demo", &docs::collect(&fx.program()));
    assert!(text.starts_with("# demo\n"), "got: {text}");
    assert!(text.contains("### `fn` add"), "got: {text}");
    assert!(text.contains("fn add(a: i64, b: i64) -> i64"), "got: {text}");
    assert!(text.contains("Adds."), "got: {text}");
}

#[test]
fn examples_are_lifted_from_fenced_blocks() {
    let fx = Fixture::new(
        "ex",
        &[(
            "main",
            "/// Adds.\n///\n/// ```\n/// assert(add(1, 2) == 3)\n/// ```\nfn add(a: i64, b: i64) -> i64 {\n    a + b\n}\n\nfn main() {\n}\n",
        )],
    );
    let examples = docs::examples(&fx.program());
    assert_eq!(examples.len(), 1);
    assert_eq!(examples[0].owner, "main::add");
    assert_eq!(examples[0].source, "assert(add(1, 2) == 3)");
}

#[test]
fn several_examples_on_one_item_are_numbered() {
    let fx = Fixture::new(
        "multi",
        &[(
            "main",
            "/// Doc.\n///\n/// ```\n/// assert(true)\n/// ```\n///\n/// ```\n/// assert(1 == 1)\n/// ```\nfn f() {\n}\n\nfn main() {\n}\n",
        )],
    );
    let examples = docs::examples(&fx.program());
    assert_eq!(examples.len(), 2);
    assert_eq!(examples[0].owner, "main::f");
    assert_eq!(examples[1].owner, "main::f #2");
}

#[test]
fn an_example_program_keeps_module_structure() {
    // The example is compiled in the module that owns it, so it calls
    // `square` the way that module's own code does. Writing `lib.square` here
    // used to be the rule, and it could not be: the same example then failed
    // when the library was tested through a consumer, where `lib` is not a
    // name the example's own module binds. One spelling has to work in both
    // positions, and this is the one.
    let fx = Fixture::new(
        "modstruct",
        &[
            (
                "lib",
                "/// Squares.\n///\n/// ```\n/// assert(square(4) == 16)\n/// ```\npub fn square(n: i64) -> i64 {\n    n * n\n}\n",
            ),
            ("main", "import lib\n\nfn main() {\n    print(\"{lib.square(2)}\")\n}\n"),
        ],
    );
    let program = fx.program();
    let examples = docs::examples(&program);
    assert_eq!(examples.len(), 1);
    let ex_program = docs::example_program(&program, &examples[0]);
    assert!(
        ex_program.diagnostics.is_empty(),
        "example program should parse: {:#?}",
        ex_program.diagnostics
    );
    let diags = grasm_sema::check::check_program(&ex_program).diagnostics;
    assert!(
        diags.is_empty(),
        "the owning module's own names must resolve in an example: {diags:#?}"
    );
    assert!(
        ex_program
            .files
            .iter()
            .any(|f| f.src.contains("fn square")),
        "the documented module's items must be in the example program"
    );
}

#[test]
fn an_example_program_has_exactly_one_main() {
    // The entry module is inlined rather than imported; importing it would
    // produce a duplicate `main`.
    let fx = Fixture::new(
        "onemain",
        &[(
            "main",
            "/// Doc.\n///\n/// ```\n/// assert(helper() == 1)\n/// ```\nfn helper() -> i64 {\n    1\n}\n\nfn main() {\n}\n",
        )],
    );
    let program = fx.program();
    let examples = docs::examples(&program);
    let ex_program = docs::example_program(&program, &examples[0]);
    let diags = grasm_sema::check::check_program(&ex_program).diagnostics;
    assert!(diags.is_empty(), "{diags:#?}");
    let mains = ex_program
        .files
        .iter()
        .flat_map(|f| &f.module.decls)
        .filter(|d| matches!(d, grasm_syntax::ast::Decl::Fn(f) if f.name.name == "main"))
        .count();
    assert_eq!(mains, 1);
}

#[test]
fn an_item_without_examples_contributes_none() {
    let fx = Fixture::new(
        "noex",
        &[("main", "/// Just prose.\nfn f() {\n}\n\nfn main() {\n}\n")],
    );
    assert!(docs::examples(&fx.program()).is_empty());
}

/// A doc example inside a dependency resolves that dependency's imports.
///
/// The harness reloads the program to compile the example, and the reload had
/// lost which modules were packages. So a library's `import util` was looked
/// up as a bare name and found the *consumer's* `util.gr` — the example ran
/// against the application's code and reported the library as broken. This is
/// the C21 bug one layer up, which is why the package set now travels on
/// `Program` instead of being rederived.
#[test]
fn a_dependency_doc_example_resolves_inside_the_dependency() {
    let root = std::env::temp_dir().join(format!("grasm_docs_dep_{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
    let write = |path: std::path::PathBuf, src: &str| {
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, src).unwrap();
    };
    write(
        root.join("dep/grasm.toml"),
        "[package]\nname = \"dep\"\nversion = \"0.1.0\"\nentry = \"src/main.gr\"\ncapabilities = []\n",
    );
    write(
        root.join("dep/src/main.gr"),
        "import util\n\n/// ```\n/// assert(twice(3) == 6)\n/// ```\npub fn twice(n: i64) -> i64 {\n    util.scale(n)\n}\n",
    );
    write(root.join("dep/src/util.gr"), "pub fn scale(n: i64) -> i64 {\n    n * 2\n}\n");
    write(
        root.join("app/grasm.toml"),
        "[package]\nname = \"app\"\nversion = \"0.1.0\"\nentry = \"src/main.gr\"\ncapabilities = [\"io\"]\n\n[dependencies.dep]\npath = \"../dep\"\nversion = \"^0.1.0\"\n",
    );
    // Same module name, different answer. Without the fix the example picks
    // this one and computes 3000.
    write(root.join("app/src/util.gr"), "pub fn scale(n: i64) -> i64 {\n    n * 1000\n}\n");
    write(
        root.join("app/src/main.gr"),
        "import dep\n\nfn main() {\n    print(\"{dep.twice(3)}\")\n}\n",
    );

    let program = grasm_sema::loader::load_from_disk(&root.join("app/src/main.gr")).expect("loads");
    assert!(program.diagnostics.is_empty(), "{:#?}", program.diagnostics);
    let examples = docs::examples(&program);
    let example = examples
        .iter()
        .find(|e| e.owner.starts_with("dep::"))
        .expect("the dependency's example is collected");
    let ex = docs::example_program(&program, example);
    assert!(ex.diagnostics.is_empty(), "{:#?}", ex.diagnostics);
    assert_eq!(ex.files[0].imports.get("util").map(String::as_str), Some("dep.util"));

    let _ = std::fs::remove_dir_all(&root);
}

/// A doc example sees types declared in other modules.
///
/// Types are one namespace across a program ([T-MODULE-NS]), but the example
/// harness dropped the program's entry module when it built the example, so a
/// library function whose signature named a type declared in the *application*
/// failed to compile on a type its example never mentions. The failure comes
/// from the signature, so every example in that module died with it.
#[test]
fn a_doc_example_sees_types_from_other_modules() {
    let fx = Fixture::new(
        "globaltype",
        &[
            (
                "main",
                "import b\n\ntype Oops = { why: str }\n\nfn main() {\n    print(\"{b.twice(2) ?? 0}\")\n}\n",
            ),
            (
                "b",
                "/// ```\n/// assert((twice(2) ?? 0) == 4)\n/// ```\npub fn twice(n: i64) -> i64 ! Oops {\n    n * 2\n}\n",
            ),
        ],
    );
    let program = fx.program();
    let examples = docs::examples(&program);
    let example = examples
        .iter()
        .find(|e| e.owner.starts_with("b::"))
        .expect("the library's example is collected");
    let ex = docs::example_program(&program, example);
    let diags = grasm_sema::check::check_program(&ex).diagnostics;
    assert!(diags.is_empty(), "{diags:#?}");
}
