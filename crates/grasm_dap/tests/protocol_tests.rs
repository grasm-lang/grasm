//! Debug Adapter Protocol tests, driven the way an editor drives it: framed
//! messages in, framed messages out.

use grasm_dap::json::{self, Json};
use std::io::Write;

/// A program on disk, because the adapter loads by path the way a client does.
struct Fixture {
    dir: std::path::PathBuf,
    entry: std::path::PathBuf,
}

impl Fixture {
    fn new(name: &str, src: &str) -> Fixture {
        let dir = std::env::temp_dir().join(format!("grasm_dap_{}_{}", name, std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let entry = dir.join("main.gr");
        std::fs::File::create(&entry)
            .unwrap()
            .write_all(src.as_bytes())
            .unwrap();
        Fixture { dir, entry }
    }

    fn path(&self) -> String {
        self.entry.to_string_lossy().to_string()
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.dir);
    }
}

fn frame(body: &str) -> String {
    format!("Content-Length: {}\r\n\r\n{body}", body.len())
}

fn request(seq: i64, command: &str, arguments: Json) -> String {
    frame(
        &Json::obj(vec![
            ("seq", Json::int(seq)),
            ("type", Json::str("request")),
            ("command", Json::str(command)),
            ("arguments", arguments),
        ])
        .to_text(),
    )
}

/// Runs a scripted session and returns every message the adapter emitted.
fn run(fixture: &Fixture, messages: &[String]) -> Vec<Json> {
    let input = messages.concat();
    let path = fixture.path();
    let result = grasm_dap::serve(
        std::io::BufReader::new(std::io::Cursor::new(input.into_bytes())),
        Vec::<u8>::new(),
        move |p| {
            let program = grasm_sema::loader::load_from_disk(std::path::Path::new(p))
                .map_err(|e| format!("cannot load `{p}`: {e}"))?;
            let mut diags = program.diagnostics.clone();
            diags.extend(grasm_sema::check::check_program(&program).diagnostics);
            if let Some(first) = diags.first() {
                return Err(format!("`{p}` has errors: {}", first.message));
            }
            let _ = &path;
            Ok(program)
        },
    );
    let output = result.expect("adapter should finish cleanly");
    decode(&output)
}

fn decode(bytes: &[u8]) -> Vec<Json> {
    let text = String::from_utf8_lossy(bytes).into_owned();
    let mut out = Vec::new();
    let mut rest = text.as_str();
    while let Some(header_end) = rest.find("\r\n\r\n") {
        let header = &rest[..header_end];
        let len: usize = header
            .lines()
            .find_map(|l| l.strip_prefix("Content-Length:"))
            .expect("framed message must carry Content-Length")
            .trim()
            .parse()
            .expect("Content-Length must be a number");
        let body_start = header_end + 4;
        let body = &rest[body_start..body_start + len];
        out.push(json::parse(body).expect("adapter must emit valid JSON"));
        rest = &rest[body_start + len..];
    }
    out
}

fn events<'a>(messages: &'a [Json], name: &str) -> Vec<&'a Json> {
    messages
        .iter()
        .filter(|m| m.get("event").and_then(Json::as_str) == Some(name))
        .collect()
}

fn responses<'a>(messages: &'a [Json], command: &str) -> Vec<&'a Json> {
    messages
        .iter()
        .filter(|m| {
            m.get("type").and_then(Json::as_str) == Some("response")
                && m.get("command").and_then(Json::as_str) == Some(command)
        })
        .collect()
}

const COUNTER: &str = "fn add(a: i64, b: i64) -> i64 {\n    let sum = a + b\n    sum\n}\n\nfn main() {\n    let x = 2\n    let y = 3\n    let total = add(x, y)\n    print(\"{total}\")\n}\n";

fn init(seq: i64) -> String {
    request(seq, "initialize", Json::obj(vec![]))
}

fn launch(seq: i64, fixture: &Fixture, stop_on_entry: bool) -> String {
    request(
        seq,
        "launch",
        Json::obj(vec![
            ("program", Json::str(fixture.path())),
            ("stopOnEntry", Json::Bool(stop_on_entry)),
        ]),
    )
}

fn breakpoints(seq: i64, fixture: &Fixture, lines: &[i64]) -> String {
    request(
        seq,
        "setBreakpoints",
        Json::obj(vec![
            (
                "source",
                Json::obj(vec![("path", Json::str(fixture.path()))]),
            ),
            (
                "breakpoints",
                Json::Arr(
                    lines
                        .iter()
                        .map(|l| Json::obj(vec![("line", Json::int(*l))]))
                        .collect(),
                ),
            ),
        ]),
    )
}

#[test]
fn initialize_advertises_only_what_is_implemented() {
    let fx = Fixture::new("init", COUNTER);
    let out = run(&fx, &[init(1), request(2, "disconnect", Json::obj(vec![]))]);
    let reply = responses(&out, "initialize")[0];
    assert_eq!(
        reply.path("body.supportsConfigurationDoneRequest"),
        Some(&Json::Bool(true))
    );
    // Claiming step-back would break any client that trusted it.
    assert_eq!(
        reply.path("body.supportsStepBack"),
        Some(&Json::Bool(false))
    );
    assert_eq!(events(&out, "initialized").len(), 1);
}

#[test]
fn a_program_runs_to_completion_and_reports_its_output() {
    let fx = Fixture::new("run", COUNTER);
    let out = run(
        &fx,
        &[
            init(1),
            launch(2, &fx, false),
            request(3, "configurationDone", Json::obj(vec![])),
        ],
    );
    let printed: String = events(&out, "output")
        .iter()
        .filter_map(|e| e.path("body.output").and_then(Json::as_str))
        .collect();
    assert!(printed.contains('5'), "program printed: {printed:?}");
    let exited = events(&out, "exited");
    assert_eq!(exited.len(), 1);
    assert_eq!(exited[0].path("body.exitCode"), Some(&Json::Num(0.0)));
    assert_eq!(events(&out, "terminated").len(), 1);
}

#[test]
fn a_breakpoint_stops_execution_on_its_line() {
    let fx = Fixture::new("bp", COUNTER);
    let out = run(
        &fx,
        &[
            init(1),
            // Line 9 is `let total = add(x, y)`.
            breakpoints(2, &fx, &[9]),
            launch(3, &fx, false),
            request(4, "configurationDone", Json::obj(vec![])),
            request(5, "stackTrace", Json::obj(vec![])),
            request(6, "continue", Json::obj(vec![])),
        ],
    );
    let stopped = events(&out, "stopped");
    assert_eq!(stopped.len(), 1, "expected exactly one stop");
    assert_eq!(
        stopped[0].path("body.reason"),
        Some(&Json::str("breakpoint"))
    );
    let frames = responses(&out, "stackTrace")[0]
        .path("body.stackFrames")
        .unwrap();
    let Json::Arr(frames) = frames else {
        panic!("stackFrames must be an array");
    };
    assert_eq!(frames[0].get("name"), Some(&Json::str("main::main")));
    assert_eq!(frames[0].get("line"), Some(&Json::Num(9.0)));
    assert_eq!(events(&out, "terminated").len(), 1);
}

#[test]
fn variables_show_the_locals_in_scope() {
    let fx = Fixture::new("vars", COUNTER);
    let out = run(
        &fx,
        &[
            init(1),
            breakpoints(2, &fx, &[9]),
            launch(3, &fx, false),
            request(4, "configurationDone", Json::obj(vec![])),
            request(5, "stackTrace", Json::obj(vec![])),
            request(6, "scopes", Json::obj(vec![("frameId", Json::int(0))])),
            request(
                7,
                "variables",
                Json::obj(vec![("variablesReference", Json::int(1))]),
            ),
            request(8, "continue", Json::obj(vec![])),
        ],
    );
    let vars = responses(&out, "variables")[0]
        .path("body.variables")
        .unwrap();
    let Json::Arr(vars) = vars else {
        panic!("variables must be an array");
    };
    let named: Vec<(String, String)> = vars
        .iter()
        .map(|v| {
            (
                v.get("name")
                    .and_then(Json::as_str)
                    .unwrap_or("")
                    .to_string(),
                v.get("value")
                    .and_then(Json::as_str)
                    .unwrap_or("")
                    .to_string(),
            )
        })
        .collect();
    assert!(
        named.contains(&("x".to_string(), "2".to_string())),
        "got: {named:?}"
    );
    assert!(
        named.contains(&("y".to_string(), "3".to_string())),
        "got: {named:?}"
    );
    // `total` is not bound until the statement we are stopped *before* runs.
    assert!(
        !named.iter().any(|(n, _)| n == "total"),
        "a not-yet-executed binding must not appear: {named:?}"
    );
}

#[test]
fn step_in_enters_a_called_function() {
    let fx = Fixture::new("stepin", COUNTER);
    let out = run(
        &fx,
        &[
            init(1),
            breakpoints(2, &fx, &[9]),
            launch(3, &fx, false),
            request(4, "configurationDone", Json::obj(vec![])),
            request(5, "stepIn", Json::obj(vec![])),
            request(6, "stackTrace", Json::obj(vec![])),
            request(7, "continue", Json::obj(vec![])),
        ],
    );
    let frames = responses(&out, "stackTrace")[0]
        .path("body.stackFrames")
        .unwrap();
    let Json::Arr(frames) = frames else {
        panic!("stackFrames must be an array");
    };
    assert_eq!(
        frames[0].get("name"),
        Some(&Json::str("main::add")),
        "stepIn should be inside `add`: {frames:?}"
    );
    assert_eq!(
        frames.len(),
        2,
        "the caller must still be on the stack: {frames:?}"
    );
    assert_eq!(frames[1].get("name"), Some(&Json::str("main::main")));
}

#[test]
fn next_steps_over_a_call_rather_than_into_it() {
    let fx = Fixture::new("next", COUNTER);
    let out = run(
        &fx,
        &[
            init(1),
            breakpoints(2, &fx, &[9]),
            launch(3, &fx, false),
            request(4, "configurationDone", Json::obj(vec![])),
            request(5, "next", Json::obj(vec![])),
            request(6, "stackTrace", Json::obj(vec![])),
            request(7, "continue", Json::obj(vec![])),
        ],
    );
    let frames = responses(&out, "stackTrace")[0]
        .path("body.stackFrames")
        .unwrap();
    let Json::Arr(frames) = frames else {
        panic!("stackFrames must be an array");
    };
    assert_eq!(
        frames.len(),
        1,
        "`next` must not descend into `add`: {frames:?}"
    );
    // Line 10 is the `print` after the call.
    assert_eq!(frames[0].get("line"), Some(&Json::Num(10.0)));
}

#[test]
fn step_out_returns_to_the_caller() {
    let fx = Fixture::new("stepout", COUNTER);
    let out = run(
        &fx,
        &[
            init(1),
            // Line 2 is `let sum = a + b`, inside `add`.
            breakpoints(2, &fx, &[2]),
            launch(3, &fx, false),
            request(4, "configurationDone", Json::obj(vec![])),
            request(5, "stepOut", Json::obj(vec![])),
            request(6, "stackTrace", Json::obj(vec![])),
            request(7, "continue", Json::obj(vec![])),
        ],
    );
    let frames = responses(&out, "stackTrace")[0]
        .path("body.stackFrames")
        .unwrap();
    let Json::Arr(frames) = frames else {
        panic!("stackFrames must be an array");
    };
    assert_eq!(frames.len(), 1, "back in main: {frames:?}");
    assert_eq!(frames[0].get("name"), Some(&Json::str("main::main")));
}

#[test]
fn stop_on_entry_suspends_before_the_first_statement() {
    let fx = Fixture::new("entry", COUNTER);
    let out = run(
        &fx,
        &[
            init(1),
            launch(2, &fx, true),
            request(3, "configurationDone", Json::obj(vec![])),
            request(4, "stackTrace", Json::obj(vec![])),
            request(5, "continue", Json::obj(vec![])),
        ],
    );
    let stopped = events(&out, "stopped");
    assert_eq!(stopped.len(), 1);
    assert_eq!(stopped[0].path("body.reason"), Some(&Json::str("step")));
    let frames = responses(&out, "stackTrace")[0]
        .path("body.stackFrames")
        .unwrap();
    let Json::Arr(frames) = frames else {
        panic!("stackFrames must be an array");
    };
    // Line 7 is `let x = 2`, the first statement of `main`.
    assert_eq!(frames[0].get("line"), Some(&Json::Num(7.0)));
}

#[test]
fn evaluate_looks_up_a_variable_and_refuses_to_invent_one() {
    let fx = Fixture::new("eval", COUNTER);
    let out = run(
        &fx,
        &[
            init(1),
            breakpoints(2, &fx, &[9]),
            launch(3, &fx, false),
            request(4, "configurationDone", Json::obj(vec![])),
            request(
                5,
                "evaluate",
                Json::obj(vec![("expression", Json::str("x"))]),
            ),
            request(
                6,
                "evaluate",
                Json::obj(vec![("expression", Json::str("nope"))]),
            ),
            request(7, "continue", Json::obj(vec![])),
        ],
    );
    let replies = responses(&out, "evaluate");
    assert_eq!(replies.len(), 2);
    assert_eq!(replies[0].path("body.result"), Some(&Json::str("2")));
    assert_eq!(replies[0].path("body.type"), Some(&Json::str("i64")));
    assert_eq!(replies[1].get("success"), Some(&Json::Bool(false)));
}

#[test]
fn a_panic_reports_its_stack_trace_before_exiting() {
    let fx = Fixture::new(
        "panic",
        "fn boom(xs: [i64]) -> i64 {\n    xs[10]\n}\n\nfn main() {\n    print(\"{boom([1])}\")\n}\n",
    );
    let out = run(
        &fx,
        &[
            init(1),
            launch(2, &fx, false),
            request(3, "configurationDone", Json::obj(vec![])),
        ],
    );
    let stderr: String = events(&out, "output")
        .iter()
        .filter(|e| e.path("body.category").and_then(Json::as_str) == Some("stderr"))
        .filter_map(|e| e.path("body.output").and_then(Json::as_str))
        .collect();
    assert!(stderr.contains("main::boom"), "trace was: {stderr:?}");
    assert!(stderr.contains("main::main"), "trace was: {stderr:?}");
    let exited = events(&out, "exited");
    assert_eq!(exited[0].path("body.exitCode"), Some(&Json::Num(101.0)));
}

#[test]
fn a_program_that_does_not_compile_fails_the_launch() {
    let fx = Fixture::new("broken", "fn main() {\n    let x: i64 = \"text\"\n}\n");
    let out = run(&fx, &[init(1), launch(2, &fx, false)]);
    let reply = responses(&out, "launch")[0];
    assert_eq!(reply.get("success"), Some(&Json::Bool(false)));
    // The client is told the run is over rather than left waiting.
    assert_eq!(events(&out, "terminated").len(), 1);
}

#[test]
fn breakpoints_in_a_file_are_replaced_not_accumulated() {
    let fx = Fixture::new("replace", COUNTER);
    let out = run(
        &fx,
        &[
            init(1),
            breakpoints(2, &fx, &[7, 8]),
            // A second call for the same file supersedes the first.
            breakpoints(3, &fx, &[9]),
            launch(4, &fx, false),
            request(5, "configurationDone", Json::obj(vec![])),
            request(6, "continue", Json::obj(vec![])),
        ],
    );
    assert_eq!(
        events(&out, "stopped").len(),
        1,
        "only the line from the second call should stop"
    );
}

#[test]
fn a_client_that_disconnects_mid_stop_ends_the_run() {
    let fx = Fixture::new("hangup", COUNTER);
    // Input ends while execution is suspended at the breakpoint.
    let out = run(
        &fx,
        &[
            init(1),
            breakpoints(2, &fx, &[7]),
            launch(3, &fx, false),
            request(4, "configurationDone", Json::obj(vec![])),
        ],
    );
    assert_eq!(events(&out, "stopped").len(), 1);
    assert_eq!(
        events(&out, "terminated").len(),
        1,
        "the adapter must finish rather than run on unsupervised"
    );
}

/// A program whose behaviour depends entirely on when it ran.
const CLOCK: &str = "fn main() / {io} {\n    let t = now_millis()\n    let doubled = t * 2\n    print(\"{doubled}\")\n}\n";

/// A program that reads a file it needs.
const READER: &str = "fn main() / {io} {\n    match read_file(\"config.toml\") {\n        Ok(text) -> print(\"read {text.len()} bytes\")\n        Err(e) -> print(\"could not read it\")\n    }\n}\n";

fn launch_with_journal(seq: i64, fixture: &Fixture, journal: &str) -> String {
    request(
        seq,
        "launch",
        Json::obj(vec![
            ("program", Json::str(fixture.path())),
            ("stopOnEntry", Json::Bool(false)),
            ("journal", Json::str(journal)),
        ]),
    )
}

/// Writes a journal into the fixture's directory and answers its path.
fn journal(fixture: &Fixture, records: &[(&str, grasm_journal::Wire)]) -> String {
    let owned: Vec<(String, grasm_journal::Wire)> = records
        .iter()
        .map(|(n, w)| ((*n).to_string(), w.clone()))
        .collect();
    let path = fixture.dir.join("j");
    std::fs::write(&path, grasm_journal::framed(&owned)).unwrap();
    path.to_string_lossy().to_string()
}

/// Stepping through a *recorded* run: the two halves this was built for.
///
/// The failure happens on a machine you do not have, and the debugger is here.
/// So the value a breakpoint shows must be the one from the journal, not the
/// one this machine's clock would give — and a clock is the clearest case,
/// because the two cannot coincide by accident.
#[test]
fn a_breakpoint_shows_the_recorded_value_and_not_this_machines() {
    let fx = Fixture::new("replay", CLOCK);
    let recorded = 1_700_000_000_000i64;
    let j = journal(&fx, &[("now_millis", grasm_journal::Wire::Int(recorded))]);

    let out = run(
        &fx,
        &[
            init(1),
            breakpoints(2, &fx, &[3]),
            launch_with_journal(3, &fx, &j),
            request(4, "configurationDone", Json::obj(vec![])),
            request(5, "stackTrace", Json::obj(vec![])),
            request(6, "scopes", Json::obj(vec![("frameId", Json::int(0))])),
            request(
                7,
                "variables",
                Json::obj(vec![("variablesReference", Json::int(1))]),
            ),
            request(8, "continue", Json::obj(vec![])),
        ],
    );

    let Some(Json::Arr(vars)) = responses(&out, "variables")[0].path("body.variables") else {
        panic!("variables must be an array: {out:?}");
    };
    let t = vars
        .iter()
        .find(|v| v.get("name").and_then(Json::as_str) == Some("t"))
        .and_then(|v| v.get("value"))
        .and_then(Json::as_str)
        .unwrap_or_default()
        .to_string();
    assert_eq!(
        t, "1700000000000",
        "the breakpoint showed this machine's clock rather than the journal's"
    );
}

/// A journal written by hand decides what the world says, and the disk does
/// not get a vote.
///
/// The file named here does not exist, so a live run takes the `Err` branch.
/// The journal says it read twelve bytes, so the run takes the `Ok` branch and
/// prints twelve — which no live run on any machine could produce. That is the
/// direction worth testing: a journal that merely *agreed* with the disk would
/// pass whether or not it was consulted, and this one cannot.
#[test]
fn a_written_journal_decides_what_the_program_read() {
    let fx = Fixture::new("inject", READER);
    let j = journal(
        &fx,
        &[(
            "read_file",
            grasm_journal::Wire::ok(grasm_journal::Wire::Str("port = 8080\n".to_string())),
        )],
    );
    let out = run(
        &fx,
        &[
            init(1),
            launch_with_journal(2, &fx, &j),
            request(3, "configurationDone", Json::obj(vec![])),
        ],
    );
    let printed: String = events(&out, "output")
        .iter()
        .filter_map(|e| e.path("body.output").and_then(Json::as_str))
        .collect();
    assert!(
        printed.contains("read 12 bytes"),
        "the journal, not the missing file, should have decided this: {printed:?}"
    );
}

/// A journal that cannot be read fails the launch.
///
/// Not a live run with a warning: a session that quietly ran against the real
/// world while the person believed they were watching a replay would be worse
/// than an error, because they would trust what they saw.
#[test]
fn an_unreadable_journal_fails_the_launch() {
    let fx = Fixture::new("badjournal", CLOCK);
    let bad = fx.dir.join("not-a-journal");
    std::fs::write(&bad, b"NOTAJOURNAL").unwrap();
    let out = run(
        &fx,
        &[
            init(1),
            launch_with_journal(2, &fx, &bad.to_string_lossy()),
            request(3, "configurationDone", Json::obj(vec![])),
        ],
    );
    let launch = responses(&out, "launch");
    assert_eq!(
        launch[0].path("success"),
        Some(&Json::Bool(false)),
        "the launch must fail: {out:?}"
    );
    let message = launch[0]
        .path("message")
        .and_then(Json::as_str)
        .unwrap_or_default();
    assert!(
        message.contains("not a Grasm journal"),
        "the reason must say what was wrong with the journal: {message:?}"
    );
    assert_eq!(
        events(&out, "terminated").len(),
        1,
        "a failed launch must terminate the session"
    );
}
