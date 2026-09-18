//! End-to-end LSP tests: drive the server over its real stdio protocol and
//! assert on the frames it writes back.

use std::io::Cursor;

use grasm_lsp::json::{self, Json};
use grasm_lsp::Server;

/// Frames a batch of JSON-RPC messages the way a client would.
fn frame(msgs: &[&str]) -> Vec<u8> {
    let mut out = Vec::new();
    for m in msgs {
        out.extend(format!("Content-Length: {}\r\n\r\n{m}", m.len()).into_bytes());
    }
    out
}

/// Runs the server over the given messages and returns the parsed replies.
fn run(msgs: &[&str]) -> Vec<Json> {
    let input = frame(msgs);
    let mut reader = Cursor::new(input);
    let mut output: Vec<u8> = Vec::new();
    Server::new().run(&mut reader, &mut output).unwrap();
    parse_frames(&String::from_utf8(output).unwrap())
}

fn parse_frames(text: &str) -> Vec<Json> {
    let mut out = Vec::new();
    let mut rest = text;
    while let Some(idx) = rest.find("Content-Length: ") {
        rest = &rest[idx + "Content-Length: ".len()..];
        let end = rest.find("\r\n").unwrap();
        let len: usize = rest[..end].parse().unwrap();
        let body_start = rest.find("\r\n\r\n").unwrap() + 4;
        let body = &rest[body_start..body_start + len];
        out.push(json::parse(body).unwrap());
        rest = &rest[body_start + len..];
    }
    out
}

fn open_doc(uri: &str, text: &str) -> String {
    Json::obj(vec![
        ("jsonrpc", Json::str("2.0")),
        ("method", Json::str("textDocument/didOpen")),
        (
            "params",
            Json::obj(vec![(
                "textDocument",
                Json::obj(vec![
                    ("uri", Json::str(uri)),
                    ("languageId", Json::str("grasm")),
                    ("version", Json::int(1)),
                    ("text", Json::str(text)),
                ]),
            )]),
        ),
    ])
    .to_text()
}

fn request(id: i64, method: &str, params: Json) -> String {
    Json::obj(vec![
        ("jsonrpc", Json::str("2.0")),
        ("id", Json::int(id)),
        ("method", Json::str(method)),
        ("params", params),
    ])
    .to_text()
}

fn position_params(uri: &str, line: i64, ch: i64) -> Json {
    Json::obj(vec![
        ("textDocument", Json::obj(vec![("uri", Json::str(uri))])),
        (
            "position",
            Json::obj(vec![
                ("line", Json::int(line)),
                ("character", Json::int(ch)),
            ]),
        ),
    ])
}

const SAMPLE: &str = "type Shape =\n    | Circle(r: f64)\n    | Dot\n\npub fn area(s: Shape) -> f64 {\n    match s {\n        Circle(r) -> 3.0 * r * r\n        Dot -> 0.0\n    }\n}\n\nfn main() {\n    print(\"{area(Circle(2.0))}\")\n}\n";

#[test]
fn initialize_advertises_capabilities() {
    let out = run(&[&request(1, "initialize", Json::obj(vec![]))]);
    let caps = out[0].path("result.capabilities").unwrap();
    assert_eq!(caps.get("hoverProvider"), Some(&Json::Bool(true)));
    assert_eq!(caps.get("definitionProvider"), Some(&Json::Bool(true)));
    assert_eq!(caps.get("documentSymbolProvider"), Some(&Json::Bool(true)));
    assert!(caps.get("completionProvider").is_some());
    assert_eq!(
        out[0].path("result.serverInfo.name").unwrap().as_str(),
        Some("grasm-lsp")
    );
}

#[test]
fn clean_file_publishes_no_diagnostics() {
    let out = run(&[&open_doc("file:///a.gr", SAMPLE)]);
    assert_eq!(
        out[0].get("method").unwrap().as_str(),
        Some("textDocument/publishDiagnostics")
    );
    let diags = out[0].path("params.diagnostics").unwrap();
    assert_eq!(diags, &Json::Arr(vec![]), "clean file should be quiet");
}

#[test]
fn errors_are_published_with_ranges_and_help() {
    let bad = "fn main() {\n    let x = 1 + \"oops\"\n}\n";
    let out = run(&[&open_doc("file:///b.gr", bad)]);
    let Json::Arr(diags) = out[0].path("params.diagnostics").unwrap() else {
        panic!("expected an array")
    };
    assert_eq!(diags.len(), 1);
    let d = &diags[0];
    assert!(d
        .get("message")
        .unwrap()
        .as_str()
        .unwrap()
        .contains("type mismatch"));
    assert_eq!(d.get("severity").unwrap().as_i64(), Some(1));
    assert_eq!(d.get("source").unwrap().as_str(), Some("grasm"));
    // Line 1 (0-based), pointing at the string literal.
    assert_eq!(d.path("range.start.line").unwrap().as_i64(), Some(1));
}

#[test]
fn diagnostics_carry_the_help_line() {
    let bad = "fn main() {\n    let x = nope\n}\n";
    let out = run(&[&open_doc("file:///c.gr", bad)]);
    let Json::Arr(diags) = out[0].path("params.diagnostics").unwrap() else {
        panic!()
    };
    let msg = diags[0].get("message").unwrap().as_str().unwrap();
    assert!(msg.contains("unknown name"), "got: {msg}");
    assert!(msg.contains("help:"), "help text should be included: {msg}");
}

#[test]
fn edits_refresh_diagnostics() {
    let broken = "fn main() {\n    let x = nope\n}\n";
    let fixed = "fn main() {\n    let x = 1\n}\n";
    let change = Json::obj(vec![
        ("jsonrpc", Json::str("2.0")),
        ("method", Json::str("textDocument/didChange")),
        (
            "params",
            Json::obj(vec![
                (
                    "textDocument",
                    Json::obj(vec![
                        ("uri", Json::str("file:///d.gr")),
                        ("version", Json::int(2)),
                    ]),
                ),
                (
                    "contentChanges",
                    Json::Arr(vec![Json::obj(vec![("text", Json::str(fixed))])]),
                ),
            ]),
        ),
    ])
    .to_text();
    let out = run(&[&open_doc("file:///d.gr", broken), &change]);
    assert_eq!(out.len(), 2);
    // First publish has the error, second is clean.
    let Json::Arr(first) = out[0].path("params.diagnostics").unwrap() else {
        panic!()
    };
    assert_eq!(first.len(), 1);
    assert_eq!(
        out[1].path("params.diagnostics").unwrap(),
        &Json::Arr(vec![])
    );
}

#[test]
fn hover_shows_function_signature() {
    // Line 4 is `pub fn area(s: Shape) -> f64 {`, "area" starts at column 7.
    let out = run(&[
        &open_doc("file:///e.gr", SAMPLE),
        &request(
            2,
            "textDocument/hover",
            position_params("file:///e.gr", 4, 8),
        ),
    ]);
    let hover = out.iter().find(|m| m.get("id").is_some()).unwrap();
    let text = hover
        .path("result.contents.value")
        .unwrap()
        .as_str()
        .unwrap();
    assert!(text.contains("pub fn area(s: Shape) -> f64"), "got: {text}");
}

#[test]
fn hover_shows_variant_and_builtin() {
    let out = run(&[
        &open_doc("file:///f.gr", SAMPLE),
        // "Circle" inside the match arm (line 6).
        &request(
            2,
            "textDocument/hover",
            position_params("file:///f.gr", 6, 10),
        ),
        // "print" on line 12.
        &request(
            3,
            "textDocument/hover",
            position_params("file:///f.gr", 12, 6),
        ),
    ]);
    let replies: Vec<&Json> = out.iter().filter(|m| m.get("id").is_some()).collect();
    let variant = replies[0]
        .path("result.contents.value")
        .unwrap()
        .as_str()
        .unwrap();
    assert!(variant.contains("Shape.Circle(r: f64)"), "got: {variant}");
    let builtin = replies[1]
        .path("result.contents.value")
        .unwrap()
        .as_str()
        .unwrap();
    assert!(builtin.contains("io effect"), "got: {builtin}");
}

#[test]
fn goto_definition_finds_the_declaration() {
    // Jump from the `area(...)` call on line 12 to its declaration on line 4.
    let out = run(&[
        &open_doc("file:///g.gr", SAMPLE),
        &request(
            2,
            "textDocument/definition",
            position_params("file:///g.gr", 12, 13),
        ),
    ]);
    let reply = out.iter().find(|m| m.get("id").is_some()).unwrap();
    assert_eq!(
        reply.path("result.uri").unwrap().as_str(),
        Some("file:///g.gr")
    );
    assert_eq!(
        reply.path("result.range.start.line").unwrap().as_i64(),
        Some(4)
    );
}

#[test]
fn completion_offers_locals_keywords_and_builtins() {
    let out = run(&[
        &open_doc("file:///h.gr", SAMPLE),
        &request(
            2,
            "textDocument/completion",
            position_params("file:///h.gr", 12, 4),
        ),
    ]);
    let reply = out.iter().find(|m| m.get("id").is_some()).unwrap();
    let Json::Arr(items) = reply.get("result").unwrap() else {
        panic!("expected an array")
    };
    let labels: Vec<&str> = items
        .iter()
        .filter_map(|i| i.get("label").and_then(|l| l.as_str()))
        .collect();
    for expected in ["area", "Shape", "Circle", "match", "print", "fail"] {
        assert!(
            labels.contains(&expected),
            "missing `{expected}` in {labels:?}"
        );
    }
}

#[test]
fn document_symbols_list_declarations() {
    let out = run(&[
        &open_doc("file:///i.gr", SAMPLE),
        &request(
            2,
            "textDocument/documentSymbol",
            Json::obj(vec![(
                "textDocument",
                Json::obj(vec![("uri", Json::str("file:///i.gr"))]),
            )]),
        ),
    ]);
    let reply = out.iter().find(|m| m.get("id").is_some()).unwrap();
    let Json::Arr(syms) = reply.get("result").unwrap() else {
        panic!()
    };
    let names: Vec<&str> = syms
        .iter()
        .filter_map(|s| s.get("name").and_then(|n| n.as_str()))
        .collect();
    assert_eq!(names, vec!["Shape", "area", "main"]);
}

#[test]
fn shutdown_then_exit_terminates_cleanly() {
    let exit = Json::obj(vec![
        ("jsonrpc", Json::str("2.0")),
        ("method", Json::str("exit")),
    ])
    .to_text();
    let out = run(&[&request(1, "shutdown", Json::Null), &exit]);
    assert_eq!(out[0].get("result"), Some(&Json::Null));
}

#[test]
fn malformed_frames_do_not_crash_the_server() {
    // A garbage body between two valid messages must be skipped, not fatal.
    let out = run(&[
        "{not json at all",
        &request(1, "initialize", Json::obj(vec![])),
    ]);
    assert!(
        out.iter().any(|m| m.path("result.capabilities").is_some()),
        "server should still answer after a bad frame"
    );
}

#[test]
fn requests_for_unknown_documents_are_safe() {
    let out = run(&[&request(
        1,
        "textDocument/hover",
        position_params("file:///never-opened.gr", 0, 0),
    )]);
    assert_eq!(out[0].get("result"), Some(&Json::Null));
}

#[test]
fn hover_includes_the_doc_comment() {
    let src = "/// Adds two integers.\n/// Wraps on overflow.\nfn add(a: i64, b: i64) -> i64 {\n    a + b\n}\n";
    let out = run(&[
        &open_doc("file:///d.gr", src),
        // "add" on line 2 (0-based), column 3.
        &request(
            2,
            "textDocument/hover",
            position_params("file:///d.gr", 2, 4),
        ),
    ]);
    let hover = out.iter().find(|m| m.get("id").is_some()).unwrap();
    let text = hover
        .path("result.contents.value")
        .unwrap()
        .as_str()
        .unwrap();
    assert!(
        text.contains("fn add(a: i64, b: i64) -> i64"),
        "got: {text}"
    );
    assert!(text.contains("Adds two integers."), "got: {text}");
    assert!(text.contains("Wraps on overflow."), "got: {text}");
}

#[test]
fn hover_without_a_doc_comment_shows_only_the_signature() {
    let src = "fn add(a: i64, b: i64) -> i64 {\n    a + b\n}\n";
    let out = run(&[
        &open_doc("file:///nd.gr", src),
        &request(
            2,
            "textDocument/hover",
            position_params("file:///nd.gr", 0, 4),
        ),
    ]);
    let hover = out.iter().find(|m| m.get("id").is_some()).unwrap();
    let text = hover
        .path("result.contents.value")
        .unwrap()
        .as_str()
        .unwrap();
    assert!(text.contains("fn add"), "got: {text}");
    assert!(!text.contains("---"), "no separator without a doc: {text}");
}

/// A file with an `import` is analysed as part of its program.
///
/// The server used to analyse the buffer alone, with no imports resolved, so
/// every use of an imported name came back "unknown name" — twenty-two errors
/// on a file `grasm check` passes clean, with any real error indistinguishable
/// among them. An editor showed every multi-module file solid red.
#[test]
fn a_file_with_imports_resolves_them() {
    let dir = std::env::temp_dir().join(format!("grasm_lsp_imports_{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(
        dir.join("util.gr"),
        "pub fn scale(n: i64) -> i64 {\n    n * 2\n}\n",
    )
    .unwrap();
    let main = "import util\n\nfn main() {\n    print(\"{util.scale(3)}\")\n}\n";
    let main_path = dir.join("main.gr");
    std::fs::write(&main_path, main).unwrap();

    let uri = format!("file://{}", main_path.display());
    let out = run(&[&open_doc(&uri, main)]);
    let diags = out[0].path("params.diagnostics").unwrap();
    assert_eq!(diags, &Json::Arr(vec![]), "imports should resolve");

    // And a genuine error in the same file is still reported.
    let broken =
        "import util\n\nfn main() {\n    let s: str = util.scale(3)\n    print(\"{s}\")\n}\n";
    std::fs::write(&main_path, broken).unwrap();
    let out = run(&[&open_doc(&uri, broken)]);
    let Json::Arr(diags) = out[0].path("params.diagnostics").unwrap() else {
        panic!("expected an array")
    };
    assert_eq!(diags.len(), 1, "{diags:?}");

    let _ = std::fs::remove_dir_all(&dir);
}
