//! grasm_lsp — a Language Server Protocol server built on the same checker
//! the compiler uses, so editor diagnostics and `grasm check` never disagree.
//!
//! Supported: publishDiagnostics on open/change/save, hover (types and
//! signatures), go-to-definition, completion, and document symbols.

pub mod json;

use std::collections::HashMap;
use std::io::{BufRead, Write};

use grasm_syntax::ast::*;
use grasm_syntax::span::{LineIndex, Span};

use json::Json;

/// A file the editor has open, plus its analysis.
struct Doc {
    text: String,
    index: LineIndex,
}

impl Doc {
    fn new(text: String) -> Doc {
        let index = LineIndex::new(&text);
        Doc { text, index }
    }

    /// LSP positions are 0-based line/character; spans are byte offsets.
    fn offset_at(&self, line: u32, character: u32) -> u32 {
        let mut off = 0u32;
        for (i, l) in self.text.split_inclusive('\n').enumerate() {
            if i as u32 == line {
                // Character index is UTF-16 in the spec; for ASCII sources
                // (the common case) this is the same as the byte offset.
                let clamped = (character as usize).min(l.trim_end_matches('\n').len());
                return off + clamped as u32;
            }
            off += l.len() as u32;
        }
        off
    }

    fn range_of(&self, span: Span) -> Json {
        let start = self.index.line_col(span.start.min(self.text.len() as u32));
        let end = self.index.line_col(span.end.min(self.text.len() as u32));
        Json::obj(vec![
            (
                "start",
                Json::obj(vec![
                    ("line", Json::int(start.line as i64 - 1)),
                    ("character", Json::int(start.col as i64 - 1)),
                ]),
            ),
            (
                "end",
                Json::obj(vec![
                    ("line", Json::int(end.line as i64 - 1)),
                    ("character", Json::int(end.col as i64 - 1)),
                ]),
            ),
        ])
    }
}

pub struct Server {
    docs: HashMap<String, Doc>,
    shutdown: bool,
}

impl Default for Server {
    fn default() -> Self {
        Self::new()
    }
}

impl Server {
    pub fn new() -> Server {
        Server {
            docs: HashMap::new(),
            shutdown: false,
        }
    }

    /// Runs the stdio message loop until the client says exit.
    pub fn run(mut self, input: &mut impl BufRead, output: &mut impl Write) -> std::io::Result<()> {
        while let Some(msg) = read_message(input)? {
            let Ok(req) = json::parse(&msg) else {
                continue; // malformed frame: ignore rather than crash the editor
            };
            let method = req.get("method").and_then(|m| m.as_str()).unwrap_or("");
            let id = req.get("id").cloned();

            let response = self.handle(method, &req);

            if let (Some(id), Some(result)) = (id, response.result) {
                write_message(
                    output,
                    &Json::obj(vec![
                        ("jsonrpc", Json::str("2.0")),
                        ("id", id),
                        ("result", result),
                    ]),
                )?;
            }
            for note in response.notifications {
                write_message(output, &note)?;
            }
            if self.shutdown && method == "exit" {
                break;
            }
        }
        Ok(())
    }

    fn handle(&mut self, method: &str, req: &Json) -> Reply {
        match method {
            "initialize" => Reply::result(Json::obj(vec![
                (
                    "capabilities",
                    Json::obj(vec![
                        // 1 = full document sync
                        ("textDocumentSync", Json::int(1)),
                        ("hoverProvider", Json::Bool(true)),
                        ("definitionProvider", Json::Bool(true)),
                        ("documentSymbolProvider", Json::Bool(true)),
                        (
                            "completionProvider",
                            Json::obj(vec![("triggerCharacters", Json::Arr(vec![Json::str(".")]))]),
                        ),
                    ]),
                ),
                (
                    "serverInfo",
                    Json::obj(vec![
                        ("name", Json::str("grasm-lsp")),
                        ("version", Json::str(env!("CARGO_PKG_VERSION"))),
                    ]),
                ),
            ])),
            "shutdown" => {
                self.shutdown = true;
                Reply::result(Json::Null)
            }
            "textDocument/didOpen" => {
                let uri = req
                    .path("params.textDocument.uri")
                    .and_then(|u| u.as_str())
                    .unwrap_or("")
                    .to_string();
                let text = req
                    .path("params.textDocument.text")
                    .and_then(|t| t.as_str())
                    .unwrap_or("")
                    .to_string();
                self.docs.insert(uri.clone(), Doc::new(text));
                Reply::notify(self.diagnostics_for(&uri))
            }
            "textDocument/didChange" => {
                let uri = req
                    .path("params.textDocument.uri")
                    .and_then(|u| u.as_str())
                    .unwrap_or("")
                    .to_string();
                // Full sync: the last content change carries the whole file.
                let text = req
                    .path("params.contentChanges")
                    .and_then(|c| match c {
                        Json::Arr(items) => items.last(),
                        _ => None,
                    })
                    .and_then(|c| c.get("text"))
                    .and_then(|t| t.as_str())
                    .unwrap_or("")
                    .to_string();
                self.docs.insert(uri.clone(), Doc::new(text));
                Reply::notify(self.diagnostics_for(&uri))
            }
            "textDocument/didSave" => {
                let uri = req
                    .path("params.textDocument.uri")
                    .and_then(|u| u.as_str())
                    .unwrap_or("")
                    .to_string();
                Reply::notify(self.diagnostics_for(&uri))
            }
            "textDocument/hover" => Reply::result(self.hover(req)),
            "textDocument/definition" => Reply::result(self.definition(req)),
            "textDocument/completion" => Reply::result(self.completion(req)),
            "textDocument/documentSymbol" => Reply::result(self.symbols(req)),
            _ => Reply::none(),
        }
    }

    /// Analyzes a document and builds a publishDiagnostics notification.
    fn diagnostics_for(&self, uri: &str) -> Json {
        let Some(doc) = self.docs.get(uri) else {
            return Json::Null;
        };
        // Resolved the way `grasm check` resolves it, with the buffer
        // standing in for the entry file — an editor holds text that is not on
        // disk yet. Analysing the buffer alone left every import unresolved,
        // so a file that `grasm check` passes showed twenty-two errors and the
        // real one was indistinguishable from the noise.
        let diags = match uri.strip_prefix("file://").map(std::path::PathBuf::from) {
            Some(path) => {
                match grasm_sema::loader::load_from_disk_in(
                    &path,
                    &std::collections::HashMap::new(),
                    Some(doc.text.clone()),
                ) {
                    Ok(program) => {
                        let mut ds = program.diagnostics.clone();
                        if ds.is_empty() {
                            ds = grasm_sema::check::check_program(&program).diagnostics;
                        }
                        // Only this file's. The others belong to their own URIs
                        // and would land at meaningless offsets here.
                        let end = program
                            .files
                            .first()
                            .map(|f| f.src.len() as u32)
                            .unwrap_or(u32::MAX);
                        ds.retain(|d| d.span.start < end);
                        ds
                    }
                    // A path that will not load at all — unsaved buffer with no
                    // file yet, say. One file is better than none.
                    Err(_) => grasm_sema::analyze(&doc.text).1,
                }
            }
            None => grasm_sema::analyze(&doc.text).1,
        };
        let items: Vec<Json> = diags
            .iter()
            .map(|d| {
                let mut msg = d.message.clone();
                if let Some(help) = &d.help {
                    msg.push_str("\nhelp: ");
                    msg.push_str(help);
                }
                Json::obj(vec![
                    ("range", doc.range_of(d.span)),
                    ("severity", Json::int(1)), // 1 = Error
                    ("source", Json::str("grasm")),
                    ("message", Json::str(msg)),
                ])
            })
            .collect();
        Json::obj(vec![
            ("jsonrpc", Json::str("2.0")),
            ("method", Json::str("textDocument/publishDiagnostics")),
            (
                "params",
                Json::obj(vec![
                    ("uri", Json::str(uri)),
                    ("diagnostics", Json::Arr(items)),
                ]),
            ),
        ])
    }

    fn doc_and_offset(&self, req: &Json) -> Option<(&Doc, u32)> {
        let uri = req.path("params.textDocument.uri")?.as_str()?;
        let doc = self.docs.get(uri)?;
        let line = req.path("params.position.line")?.as_i64()? as u32;
        let ch = req.path("params.position.character")?.as_i64()? as u32;
        Some((doc, doc.offset_at(line, ch)))
    }

    fn hover(&self, req: &Json) -> Json {
        let Some((doc, off)) = self.doc_and_offset(req) else {
            return Json::Null;
        };
        let module = grasm_syntax::parse(&doc.text).module;
        let Some(word) = word_at(&doc.text, off) else {
            return Json::Null;
        };
        // A declaration's `///` block, shown under its signature on hover.
        let with_doc = |decl: &Decl, signature: String| -> Json {
            match module.doc_for(decl) {
                Some(d) => hover_markdown(&format!("{signature}\n\n---\n\n{d}")),
                None => hover_markdown(&signature),
            }
        };

        // Look for a declaration with this name and render its signature.
        for decl in &module.decls {
            match decl {
                Decl::Fn(f) if f.name.name == word => {
                    return with_doc(decl, render_fn_signature(f));
                }
                Decl::Type(t) if t.name.name == word => {
                    return with_doc(decl, render_type_decl(t));
                }
                Decl::Effect(e) if e.name.name == word => {
                    return with_doc(
                        decl,
                        format!(
                            "effect {} ({} operation{})",
                            e.name.name,
                            e.ops.len(),
                            if e.ops.len() == 1 { "" } else { "s" }
                        ),
                    );
                }
                Decl::Type(t) => {
                    // Variant of a sum type?
                    if let TypeBody::Sum(vs) = &t.body {
                        if let Some(v) = vs.iter().find(|v| v.name.name == word) {
                            let fields: Vec<String> = v
                                .fields
                                .iter()
                                .map(|f| format!("{}: {}", f.name.name, type_text(&f.ty)))
                                .collect();
                            return hover_markdown(&format!(
                                "{}.{}({})",
                                t.name.name,
                                v.name.name,
                                fields.join(", ")
                            ));
                        }
                    }
                }
                _ => {}
            }
        }
        if let Some(doc_text) = builtin_doc(&word) {
            return hover_markdown(doc_text);
        }
        Json::Null
    }

    fn definition(&self, req: &Json) -> Json {
        let Some((doc, off)) = self.doc_and_offset(req) else {
            return Json::Null;
        };
        let uri = req
            .path("params.textDocument.uri")
            .and_then(|u| u.as_str())
            .unwrap_or("");
        let module = grasm_syntax::parse(&doc.text).module;
        let Some(word) = word_at(&doc.text, off) else {
            return Json::Null;
        };
        let target: Option<Span> = module.decls.iter().find_map(|d| match d {
            Decl::Fn(f) if f.name.name == word => Some(f.name.span),
            Decl::Type(t) if t.name.name == word => Some(t.name.span),
            Decl::Effect(e) if e.name.name == word => Some(e.name.span),
            Decl::Type(t) => match &t.body {
                TypeBody::Sum(vs) => vs.iter().find(|v| v.name.name == word).map(|v| v.name.span),
                _ => None,
            },
            _ => None,
        });
        match target {
            Some(span) => Json::obj(vec![("uri", Json::str(uri)), ("range", doc.range_of(span))]),
            None => Json::Null,
        }
    }

    fn completion(&self, req: &Json) -> Json {
        let Some((doc, _off)) = self.doc_and_offset(req) else {
            return Json::Null;
        };
        let module = grasm_syntax::parse(&doc.text).module;
        let mut items: Vec<Json> = Vec::new();

        // Declarations in this file.
        for decl in &module.decls {
            match decl {
                Decl::Fn(f) => items.push(completion_item(
                    &f.name.name,
                    3, // Function
                    &render_fn_signature(f),
                )),
                Decl::Type(t) => {
                    items.push(completion_item(&t.name.name, 7, &render_type_decl(t)));
                    if let TypeBody::Sum(vs) = &t.body {
                        for v in vs {
                            items.push(completion_item(
                                &v.name.name,
                                4, // Constructor
                                &format!("variant of {}", t.name.name),
                            ));
                        }
                    }
                }
                Decl::Effect(e) => {
                    items.push(completion_item(&e.name.name, 8, "effect"));
                }
                _ => {}
            }
        }
        // Keywords and builtins.
        for kw in KEYWORDS {
            items.push(completion_item(kw, 14, "keyword"));
        }
        for (name, doc_text) in BUILTINS {
            items.push(completion_item(name, 3, doc_text));
        }
        Json::Arr(items)
    }

    fn symbols(&self, req: &Json) -> Json {
        let Some(uri) = req.path("params.textDocument.uri").and_then(|u| u.as_str()) else {
            return Json::Null;
        };
        let Some(doc) = self.docs.get(uri) else {
            return Json::Null;
        };
        let module = grasm_syntax::parse(&doc.text).module;
        let mut out = Vec::new();
        for decl in &module.decls {
            let (name, kind, span) = match decl {
                Decl::Fn(f) => (f.name.name.clone(), 12, f.span), // Function
                Decl::Const(c) => (c.name.name.clone(), 14, c.span), // Constant
                Decl::Extern(e) => (e.name.name.clone(), 12, e.span), // Function
                Decl::Type(t) => (t.name.name.clone(), 23, t.span), // Struct
                Decl::Effect(e) => (e.name.name.clone(), 11, e.span), // Interface
                Decl::Test(t) => (format!("test {:?}", t.name), 12, t.span),
                Decl::Bench(b) => (format!("bench {:?}", b.name), 12, b.span),
                Decl::Import(i) => (format!("import {}", i.path.dotted()), 2, i.span),
                Decl::Interface(i) => (i.name.name.clone(), 11, i.span), // Interface
                Decl::Impl(i) => (
                    format!("impl {} for {}", i.iface.dotted(), i.type_name),
                    5, // Class
                    i.span,
                ),
            };
            out.push(Json::obj(vec![
                ("name", Json::str(name)),
                ("kind", Json::int(kind)),
                ("range", doc.range_of(span)),
                ("selectionRange", doc.range_of(span)),
            ]));
        }
        Json::Arr(out)
    }
}

struct Reply {
    result: Option<Json>,
    notifications: Vec<Json>,
}

impl Reply {
    fn result(v: Json) -> Reply {
        Reply {
            result: Some(v),
            notifications: Vec::new(),
        }
    }
    fn notify(n: Json) -> Reply {
        Reply {
            result: None,
            notifications: if n == Json::Null { vec![] } else { vec![n] },
        }
    }
    fn none() -> Reply {
        Reply {
            result: None,
            notifications: Vec::new(),
        }
    }
}

// ---------------------------------------------------------------------------
// Rendering helpers
// ---------------------------------------------------------------------------

fn hover_markdown(text: &str) -> Json {
    Json::obj(vec![(
        "contents",
        Json::obj(vec![
            ("kind", Json::str("markdown")),
            ("value", Json::str(format!("```grasm\n{text}\n```"))),
        ]),
    )])
}

fn completion_item(label: &str, kind: i64, detail: &str) -> Json {
    Json::obj(vec![
        ("label", Json::str(label)),
        ("kind", Json::int(kind)),
        ("detail", Json::str(detail)),
    ])
}

pub fn render_fn_signature(f: &FnDecl) -> String {
    let params: Vec<String> = f
        .params
        .iter()
        .map(|p| {
            let mode = match p.mode {
                ParamMode::Let => "",
                ParamMode::Inout => "inout ",
                ParamMode::Own => "own ",
            };
            format!("{mode}{}: {}", p.name.name, type_text(&p.ty))
        })
        .collect();
    let mut out = String::new();
    if f.is_pub {
        out.push_str("pub ");
    }
    out.push_str("fn ");
    out.push_str(&f.name.name);
    if !f.generics.is_empty() {
        let gs: Vec<String> = f.generics.iter().map(|g| g.name.clone()).collect();
        out.push('[');
        out.push_str(&gs.join(", "));
        out.push(']');
    }
    out.push('(');
    out.push_str(&params.join(", "));
    out.push(')');
    if let Some(r) = &f.ret {
        out.push_str(" -> ");
        out.push_str(&type_text(r));
    }
    if let Some(e) = &f.err {
        out.push_str(" ! ");
        out.push_str(&type_text(e));
    }
    if let Some(fx) = &f.effects {
        let names: Vec<String> = fx.iter().map(|p| p.dotted()).collect();
        out.push_str(" / {");
        out.push_str(&names.join(", "));
        out.push('}');
    }
    out
}

pub fn render_type_decl(t: &TypeDecl) -> String {
    match &t.body {
        TypeBody::Struct(fields) => {
            let fs: Vec<String> = fields
                .iter()
                .map(|f| format!("{}: {}", f.name.name, type_text(&f.ty)))
                .collect();
            format!("type {} = {{ {} }}", t.name.name, fs.join(", "))
        }
        TypeBody::Sum(vs) => {
            let names: Vec<String> = vs
                .iter()
                .map(|v| {
                    if v.fields.is_empty() {
                        v.name.name.clone()
                    } else {
                        let fs: Vec<String> = v
                            .fields
                            .iter()
                            .map(|f| format!("{}: {}", f.name.name, type_text(&f.ty)))
                            .collect();
                        format!("{}({})", v.name.name, fs.join(", "))
                    }
                })
                .collect();
            format!("type {} = | {}", t.name.name, names.join(" | "))
        }
        TypeBody::Alias(a) => format!("type {} = {}", t.name.name, type_text(a)),
    }
}

pub fn type_text(t: &TypeExpr) -> String {
    match t {
        TypeExpr::Ref(inner, _) => format!("ref {}", type_text(inner)),
        TypeExpr::Name { path, args, .. } => {
            if args.is_empty() {
                path.dotted()
            } else {
                let a: Vec<String> = args.iter().map(type_text).collect();
                format!("{}[{}]", path.dotted(), a.join(", "))
            }
        }
        TypeExpr::Fn { params, ret, .. } => format!(
            "fn({}) -> {}",
            params.iter().map(type_text).collect::<Vec<_>>().join(", "),
            type_text(ret)
        ),
        TypeExpr::List(inner, _) => format!("[{}]", type_text(inner)),
        TypeExpr::Fallible(ok, err, _) => format!("{} ! {}", type_text(ok), type_text(err)),
        TypeExpr::Tuple(items, _) => {
            let a: Vec<String> = items.iter().map(type_text).collect();
            format!("({})", a.join(", "))
        }
        TypeExpr::Union(members, _) => {
            let a: Vec<String> = members.iter().map(type_text).collect();
            format!("({})", a.join(" | "))
        }
    }
}

/// The identifier under a byte offset, if any.
fn word_at(text: &str, off: u32) -> Option<String> {
    let bytes = text.as_bytes();
    let off = (off as usize).min(bytes.len());
    let is_word = |b: u8| b.is_ascii_alphanumeric() || b == b'_';
    if off >= bytes.len() && off == 0 {
        return None;
    }
    let mut start = off;
    while start > 0 && is_word(bytes[start - 1]) {
        start -= 1;
    }
    let mut end = off;
    while end < bytes.len() && is_word(bytes[end]) {
        end += 1;
    }
    if start == end {
        return None;
    }
    Some(text[start..end].to_string())
}

const KEYWORDS: &[&str] = &[
    "fn",
    "pub",
    "let",
    "var",
    "own",
    "inout",
    "type",
    "effect",
    "with",
    "handled_by",
    "match",
    "if",
    "else",
    "for",
    "while",
    "in",
    "return",
    "try",
    "true",
    "false",
    "break",
    "continue",
    "and",
    "or",
    "not",
    "import",
    "test",
];

const BUILTINS: &[(&str, &str)] = &[
    ("print", "print(value) — write a line to stdout (io effect)"),
    ("assert", "assert(cond: bool) — panic unless true"),
    ("assert_eq", "assert_eq(a, b) — panic unless equal"),
    ("panic", "panic(msg: str) — abort with a message"),
    ("fail", "fail(err) — raise the error branch of `T ! E`"),
    ("map", "map() — a new empty Map[K, V]"),
    ("args", "args() -> [str] — program arguments (io effect)"),
    (
        "input",
        "input() -> str — read a line from stdin (io effect)",
    ),
    (
        "read_file",
        "read_file(path: str) -> str ! IoError (io effect)",
    ),
    (
        "write_file",
        "write_file(path: str, content: str) -> () ! IoError (io effect)",
    ),
    ("file_exists", "file_exists(path: str) -> bool (io effect)"),
];

fn builtin_doc(name: &str) -> Option<&'static str> {
    BUILTINS.iter().find(|(n, _)| *n == name).map(|(_, d)| *d)
}

// ---------------------------------------------------------------------------
// Framing (Content-Length headers, as the protocol specifies)
// ---------------------------------------------------------------------------

pub fn read_message(input: &mut impl BufRead) -> std::io::Result<Option<String>> {
    let mut content_length: Option<usize> = None;
    loop {
        let mut line = String::new();
        let n = input.read_line(&mut line)?;
        if n == 0 {
            return Ok(None); // clean EOF
        }
        let trimmed = line.trim_end();
        if trimmed.is_empty() {
            break; // end of headers
        }
        if let Some(rest) = trimmed.strip_prefix("Content-Length:") {
            content_length = rest.trim().parse().ok();
        }
    }
    let Some(len) = content_length else {
        return Ok(Some(String::new()));
    };
    let mut buf = vec![0u8; len];
    input.read_exact(&mut buf)?;
    Ok(Some(String::from_utf8_lossy(&buf).into_owned()))
}

pub fn write_message(out: &mut impl Write, msg: &Json) -> std::io::Result<()> {
    let body = msg.to_text();
    write!(out, "Content-Length: {}\r\n\r\n{}", body.len(), body)?;
    out.flush()
}
