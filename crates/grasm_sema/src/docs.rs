//! `grasm doc` — API documentation extracted from the code that ships.
//!
//! Signatures come from the AST rather than from prose, so a documented type
//! or effect row cannot drift from the real one. Examples inside `///` blocks
//! are *executable*: `grasm test` runs them, so documentation that stops
//! being true fails the build instead of quietly misleading people.

use crate::loader::Program;
use grasm_syntax::ast::{
    decl_span, Decl, EffectDecl, FnDecl, Module, Param, ParamMode, TypeBody, TypeDecl, TypeExpr,
};

/// One documented item.
pub struct Item {
    pub kind: &'static str,
    pub name: String,
    /// The declaration as it appears in source, without a body.
    pub signature: String,
    pub doc: Option<String>,
}

pub struct ModuleDocs {
    pub module: String,
    /// Text of a leading `//!`-free doc block that precedes no declaration is
    /// not collected; a module summary is the doc on its first item.
    pub items: Vec<Item>,
}

/// A runnable example lifted out of a doc comment.
#[derive(Debug, PartialEq)]
pub struct Example {
    /// `module::item` — used as the test name so a failure points somewhere.
    pub owner: String,
    pub source: String,
}

/// Collects public items from every file in the program.
///
/// Only `pub` items are documented: the entry module's own definitions are
/// included regardless, because a program's `main` file has no consumers to
/// hide anything from.
pub fn collect(program: &Program) -> Vec<ModuleDocs> {
    let mut out = Vec::new();
    for (i, file) in program.files.iter().enumerate() {
        let is_entry = i == 0;
        let mut items = Vec::new();
        for decl in &file.module.decls {
            let Some(item) = describe(decl, &file.module, is_entry) else {
                continue;
            };
            items.push(item);
        }
        if items.is_empty() {
            continue;
        }
        out.push(ModuleDocs {
            module: file.name.clone(),
            items,
        });
    }
    out
}

fn describe(decl: &Decl, module: &Module, is_entry: bool) -> Option<Item> {
    let doc = module.doc_for(decl).map(str::to_string);
    match decl {
        Decl::Fn(f) => {
            if !f.is_pub && !is_entry {
                return None;
            }
            Some(Item {
                kind: "fn",
                name: f.name.name.clone(),
                signature: fn_signature(f),
                doc,
            })
        }
        Decl::Type(t) => {
            if !t.is_pub && !is_entry {
                return None;
            }
            Some(Item {
                kind: "type",
                name: t.name.name.clone(),
                signature: type_signature(t),
                doc,
            })
        }
        Decl::Effect(e) => Some(Item {
            kind: "effect",
            name: e.name.name.clone(),
            signature: effect_signature(e),
            doc,
        }),
        // A constant is API too. `pub let` exports, `grasm api` lists it, and
        // this dropped both the item and its `///` comment — so a library's
        // documented layout constants were simply absent from its docs.
        Decl::Const(c) => {
            if !c.is_pub && !is_entry {
                return None;
            }
            Some(Item {
                kind: "let",
                name: c.name.name.clone(),
                signature: const_signature(c),
                doc,
            })
        }
        Decl::Extern(e) => {
            if !e.is_pub && !is_entry {
                return None;
            }
            Some(Item {
                kind: "extern fn",
                name: e.name.name.clone(),
                signature: extern_signature(e),
                doc,
            })
        }
        // Tests, benchmarks, and imports are not API.
        _ => None,
    }
}

pub fn fn_signature(f: &FnDecl) -> String {
    let mut s = String::new();
    if f.is_pub {
        s.push_str("pub ");
    }
    s.push_str("fn ");
    s.push_str(&f.name.name);
    if !f.generics.is_empty() {
        s.push('[');
        s.push_str(
            &f.generics
                .iter()
                .map(|g| g.name.clone())
                .collect::<Vec<_>>()
                .join(", "),
        );
        s.push(']');
    }
    s.push('(');
    s.push_str(
        &f.params
            .iter()
            .map(param_text)
            .collect::<Vec<_>>()
            .join(", "),
    );
    s.push(')');
    if let Some(r) = &f.ret {
        s.push_str(" -> ");
        s.push_str(&type_text(r));
    }
    if let Some(e) = &f.err {
        s.push_str(" ! ");
        s.push_str(&type_text(e));
    }
    if let Some(row) = &f.effects {
        s.push_str(" / {");
        s.push_str(
            &row.iter()
                .map(|p| p.dotted())
                .collect::<Vec<_>>()
                .join(", "),
        );
        s.push('}');
    }
    s
}

fn param_text(p: &Param) -> String {
    let mode = match p.mode {
        ParamMode::Let => "",
        ParamMode::Inout => "inout ",
        ParamMode::Own => "own ",
    };
    format!("{mode}{}: {}", p.name.name, type_text(&p.ty))
}

pub fn type_signature(t: &TypeDecl) -> String {
    let mut s = String::new();
    if t.is_pub {
        s.push_str("pub ");
    }
    s.push_str("type ");
    s.push_str(&t.name.name);
    if !t.generics.is_empty() {
        s.push('[');
        s.push_str(
            &t.generics
                .iter()
                .map(|g| g.name.clone())
                .collect::<Vec<_>>()
                .join(", "),
        );
        s.push(']');
    }
    s.push_str(" = ");
    match &t.body {
        TypeBody::Struct(fields) => {
            s.push_str("{ ");
            s.push_str(
                &fields
                    .iter()
                    .map(|f| format!("{}: {}", f.name.name, type_text(&f.ty)))
                    .collect::<Vec<_>>()
                    .join(", "),
            );
            s.push_str(" }");
        }
        TypeBody::Sum(variants) => {
            let rendered: Vec<String> = variants
                .iter()
                .map(|v| {
                    if v.fields.is_empty() {
                        v.name.name.clone()
                    } else {
                        format!(
                            "{}({})",
                            v.name.name,
                            v.fields
                                .iter()
                                .map(|f| format!("{}: {}", f.name.name, type_text(&f.ty)))
                                .collect::<Vec<_>>()
                                .join(", ")
                        )
                    }
                })
                .collect();
            // The leading `|` is part of the grammar, so without it the
            // rendered declaration does not parse back in ([G-SUM]).
            s.push_str("| ");
            s.push_str(&rendered.join(" | "));
        }
        TypeBody::Alias(te) => s.push_str(&type_text(te)),
    }
    s
}

pub fn effect_signature(e: &EffectDecl) -> String {
    let ops: Vec<String> = e.ops.iter().map(fn_sig_text).collect();
    format!("effect {} {{ {} }}", e.name.name, ops.join("; "))
}

fn fn_sig_text(op: &grasm_syntax::ast::FnSig) -> String {
    let mut s = format!(
        "fn {}({})",
        op.name.name,
        op.params
            .iter()
            .map(param_text)
            .collect::<Vec<_>>()
            .join(", ")
    );
    if let Some(r) = &op.ret {
        s.push_str(" -> ");
        s.push_str(&type_text(r));
    }
    // An operation's `! E` is part of its contract, and dropping it made the
    // documented signature disagree with the real one — the exact drift the
    // generator exists to prevent. Plain functions kept theirs; operations
    // did not.
    if let Some(e) = &op.err {
        s.push_str(" ! ");
        s.push_str(&type_text(e));
    }
    s
}

pub fn type_text(t: &TypeExpr) -> String {
    match t {
        TypeExpr::Ref(inner, _) => format!("ref {}", type_text(inner)),
        TypeExpr::Name { path, args, .. } => {
            if args.is_empty() {
                path.dotted()
            } else {
                format!(
                    "{}[{}]",
                    path.dotted(),
                    args.iter().map(type_text).collect::<Vec<_>>().join(", ")
                )
            }
        }
        TypeExpr::Fn {
            params,
            ret,
            effects,
            ..
        } => {
            let row = if effects.is_empty() {
                String::new()
            } else {
                format!(
                    " / {{{}}}",
                    effects
                        .iter()
                        .map(|p| p.dotted())
                        .collect::<Vec<_>>()
                        .join(", ")
                )
            };
            format!(
                "fn({}) -> {}{row}",
                params.iter().map(type_text).collect::<Vec<_>>().join(", "),
                type_text(ret)
            )
        }
        TypeExpr::List(inner, _) => format!("[{}]", type_text(inner)),
        TypeExpr::Fallible(ok, err, _) => format!("{} ! {}", type_text(ok), type_text(err)),
        TypeExpr::Tuple(items, _) => format!(
            "({})",
            items.iter().map(type_text).collect::<Vec<_>>().join(", ")
        ),
        TypeExpr::Union(items, _) => format!(
            "({})",
            items.iter().map(type_text).collect::<Vec<_>>().join(" | ")
        ),
    }
}

/// Every declaration in a module rendered without its body.
///
/// This is the part of a module other modules can depend on. Hashing it lets
/// an incremental build tell "someone changed a function body" (rebuild only
/// that module) from "someone changed a signature or a type" (rebuild
/// everything that could see it). Types and effects share one global
/// namespace, so they are included regardless of `pub`.
pub fn module_signature(module: &Module) -> String {
    let mut out = String::new();
    for decl in &module.decls {
        match decl {
            Decl::Fn(f) => {
                out.push_str(&fn_signature(f));
                out.push('\n');
            }
            Decl::Const(c) => {
                out.push_str(&const_signature(c));
                out.push('\n');
            }
            Decl::Extern(e) => {
                out.push_str(&extern_signature(e));
                out.push('\n');
            }
            Decl::Type(t) => {
                out.push_str(&type_signature(t));
                out.push('\n');
            }
            Decl::Effect(e) => {
                out.push_str(&effect_signature(e));
                out.push('\n');
            }
            Decl::Import(i) => {
                out.push_str("import ");
                out.push_str(&i.path.dotted());
                out.push('\n');
            }
            // Tests and benchmarks are not part of any other module's view.
            Decl::Test(_) | Decl::Bench(_) => {}
        }
    }
    out
}

// ---------------------------------------------------------------------------
// Rendering
// ---------------------------------------------------------------------------

/// Renders the whole program's documentation as Markdown.
pub fn render_markdown(title: &str, modules: &[ModuleDocs]) -> String {
    let mut out = format!("# {title}\n");
    for m in modules {
        out.push_str(&format!("\n## Module `{}`\n", m.module));
        for item in &m.items {
            out.push_str(&format!("\n### `{}` {}\n\n", item.kind, item.name));
            out.push_str("```\n");
            out.push_str(&item.signature);
            out.push_str("\n```\n");
            if let Some(doc) = &item.doc {
                out.push('\n');
                out.push_str(&without_hidden_lines(doc));
                out.push('\n');
            }
        }
    }
    out
}

// ---------------------------------------------------------------------------
// Executable examples
// ---------------------------------------------------------------------------

/// A doc comment with its hidden setup lines removed.
///
/// `# ` inside a fenced block is setup: it runs, and it is not part of what a
/// reader sees. The example *runner* already stripped it; the renderer did
/// not, so every hidden line appeared verbatim in the published API docs —
/// the one place the feature exists to keep them out of.
fn without_hidden_lines(doc: &str) -> String {
    let mut out: Vec<&str> = Vec::new();
    let mut in_block = false;
    for line in doc.lines() {
        let trimmed = line.trim_start();
        if trimmed.starts_with("```") {
            in_block = !in_block;
            out.push(line);
            continue;
        }
        if in_block && trimmed.starts_with("# ") {
            continue;
        }
        out.push(line);
    }
    out.join("\n")
}

/// Extracts fenced code blocks from doc comments.
///
/// A block is a run of lines between ``` fences. Lines starting with `# ` are
/// hidden setup: they run but do not appear in rendered documentation, which
/// is how an example can stay short without being a fragment that cannot run.
pub fn examples(program: &Program) -> Vec<Example> {
    let mut out = Vec::new();
    for file in &program.files {
        for decl in &file.module.decls {
            let Some(doc) = file.module.docs.get(&decl_span(decl).start) else {
                continue;
            };
            let name = decl_name(decl);
            for (i, body) in fenced_blocks(doc).into_iter().enumerate() {
                let owner = if i == 0 {
                    format!("{}::{name}", file.name)
                } else {
                    format!("{}::{name} #{}", file.name, i + 1)
                };
                out.push(Example {
                    owner,
                    source: body,
                });
            }
        }
    }
    out
}

/// `let NAME: T` — the value is not part of the signature, only the promise
/// that the name exists and what type it has.
pub fn const_signature(c: &grasm_syntax::ast::ConstDecl) -> String {
    // `pub` is part of the signature: it is the difference between something
    // another module can read and something it cannot.
    let vis = if c.is_pub { "pub " } else { "" };
    match &c.ty {
        Some(t) => format!("{vis}let {}: {}", c.name.name, type_text(t)),
        None => format!("{vis}let {}", c.name.name),
    }
}

/// `extern fn name(params) -> T` — the contract, which is all there is.
pub fn extern_signature(e: &grasm_syntax::ast::ExternDecl) -> String {
    let params: Vec<String> = e.params.iter().map(param_text).collect();
    let ret = match &e.ret {
        Some(t) => format!(" -> {}", type_text(t)),
        None => String::new(),
    };
    format!("extern fn {}({}){}", e.name.name, params.join(", "), ret)
}

fn decl_name(d: &Decl) -> String {
    match d {
        Decl::Fn(f) => f.name.name.clone(),
        Decl::Const(c) => c.name.name.clone(),
        Decl::Extern(e) => e.name.name.clone(),
        Decl::Type(t) => t.name.name.clone(),
        Decl::Effect(e) => e.name.name.clone(),
        Decl::Test(t) => t.name.clone(),
        Decl::Bench(b) => b.name.clone(),
        Decl::Import(i) => i.path.dotted(),
    }
}

/// Bodies of ``` fenced blocks, with `# ` hidden-line markers stripped.
fn fenced_blocks(doc: &str) -> Vec<String> {
    let mut blocks = Vec::new();
    let mut current: Option<Vec<String>> = None;
    for line in doc.lines() {
        let trimmed = line.trim_start();
        if trimmed.starts_with("```") {
            match current.take() {
                // Closing fence.
                Some(lines) => blocks.push(lines.join("\n")),
                // Opening fence; an info string like ```grasm is ignored.
                None => current = Some(Vec::new()),
            }
            continue;
        }
        if let Some(lines) = &mut current {
            let visible = trimmed.strip_prefix("# ").unwrap_or(line);
            lines.push(visible.to_string());
        }
    }
    // An unterminated block is still an example; dropping it silently would
    // let a typo disable a test.
    if let Some(lines) = current {
        blocks.push(lines.join("\n"));
    }
    blocks
}

/// Builds a runnable program for one example.
///
/// The example becomes a synthetic entry module that imports every module of
/// the original program, so a doc comment can call `mathlib.square(4)` — the
/// way a consumer actually writes it — instead of an unqualified name that
/// only works because everything was flattened into one file.
pub fn example_program(program: &Program, example: &Example) -> Program {
    let mut sources: Vec<(String, String)> = program
        .files
        .iter()
        .map(|f| (f.name.clone(), f.src.clone()))
        .collect();

    // The example is compiled in the context of the module that *owns* it:
    // its own items unqualified, everything else imported. Compiling it in the
    // program's entry module instead made an example unwritable — a library's
    // own suite wanted `describe(...)` and a consumer's wanted
    // `statlib.describe(...)`, and no spelling passed in both.
    let owner_module = example
        .owner
        .split("::")
        .next()
        .unwrap_or("main")
        .to_string();
    // The example gets exactly the owner's own context: its declarations
    // inlined, its imports as it wrote them. Adding an import for every other
    // module in the program produced `duplicate import alias` the moment the
    // owner imported one of them — which is any module that imports anything.
    let mut entry = String::new();
    let owner_src = program
        .files
        .iter()
        .find(|f| f.name == owner_module)
        .or_else(|| program.files.first());
    if let Some(owner) = owner_src {
        for line in owner.src.lines() {
            // The owner's own imports come along, so an example may use
            // anything the documented module itself can — but spelled the way
            // the *loader* resolved them, not the way the owner wrote them.
            // Inside a dependency, `import util` means that dependency's
            // `util`; re-reading it here as a bare name found the consumer's
            // file of the same name instead, so a library's doc example ran
            // against the application's code and reported the wrong answer as
            // a failure of the library. That is C21 again, one layer up.
            if line.trim_start().starts_with("import ") {
                let written = line.trim_start().trim_start_matches("import ").trim();
                let alias = written.rsplit('.').next().unwrap_or(written);
                match owner.imports.get(alias) {
                    Some(full) => {
                        entry.push_str("import ");
                        entry.push_str(full);
                    }
                    None => entry.push_str(line),
                }
                entry.push('\n');
                continue;
            }
            entry.push_str(line);
            entry.push('\n');
        }
    }
    entry.push_str("\ntest \"");
    entry.push_str(&example.owner.replace('"', "'"));
    entry.push_str("\" {\n");
    for line in example.source.lines() {
        entry.push_str("    ");
        entry.push_str(line);
        entry.push('\n');
    }
    entry.push_str("}\n");

    // The program's original entry module is kept, under a name nothing else
    // uses, and imported. Dropping it lost every type it declared — and types
    // are one namespace across the whole program ([T-MODULE-NS]), so a doc
    // example on a *library* function failed to compile on a type it never
    // mentions, because the type was declared in the application. The failure
    // came from the signature, so every example in that module died with it.
    const HOST: &str = "__doc_host";
    let host = sources
        .iter()
        .find(|(n, _)| n == "main")
        .map(|(_, src)| src.clone());
    sources.retain(|(n, _)| n != "main");
    let mut entry_with_host = String::new();
    if let Some(src) = host {
        // Only when the owner is not itself the entry — otherwise its
        // declarations are already inlined above and this would duplicate them.
        if owner_module != "main" {
            sources.push((HOST.to_string(), src));
            entry_with_host.push_str("import ");
            entry_with_host.push_str(HOST);
            entry_with_host.push('\n');
        }
    }
    entry_with_host.push_str(&entry);
    sources.push(("main".to_string(), entry_with_host));

    // The dependency set comes along too, so every *other* module reloaded
    // here namespaces its imports the way it did the first time.
    crate::loader::load_with_paths_in(
        "main",
        "<doc example>",
        &|name| {
            sources
                .iter()
                .find(|(n, _)| n == name)
                .map(|(_, src)| src.clone())
        },
        &|_| Vec::new(),
        &program.deps,
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fenced_blocks_are_extracted() {
        let doc = "Adds.\n\n```\nassert(add(1, 2) == 3)\n```\n";
        assert_eq!(fenced_blocks(doc), vec!["assert(add(1, 2) == 3)"]);
    }

    #[test]
    fn hidden_setup_lines_run_but_are_marked() {
        let doc = "```\n# let x = 1\nassert(x == 1)\n```";
        assert_eq!(fenced_blocks(doc), vec!["let x = 1\nassert(x == 1)"]);
    }

    #[test]
    fn several_blocks_in_one_comment() {
        let doc = "```\na\n```\n\ntext\n\n```\nb\n```";
        assert_eq!(fenced_blocks(doc), vec!["a", "b"]);
    }

    #[test]
    fn an_info_string_is_ignored() {
        assert_eq!(fenced_blocks("```grasm\nx\n```"), vec!["x"]);
    }

    #[test]
    fn an_unterminated_block_is_still_collected() {
        // Otherwise a missing closing fence would silently skip the test.
        assert_eq!(fenced_blocks("```\nx"), vec!["x"]);
    }

    #[test]
    fn a_comment_without_code_yields_nothing() {
        assert!(fenced_blocks("Just prose.").is_empty());
    }
}
