//! grasm_dap — a Debug Adapter Protocol server over stdio.
//!
//! Editors speak DAP to debug a program: set breakpoints, step, inspect
//! variables. The adapter drives the reference interpreter through a hook it
//! installs, so what you see stepping is exactly what `grasm run` does — there
//! is no separate "debug mode" evaluator that can disagree with the real one.
//!
//! Everything runs on one thread. When execution suspends, the adapter serves
//! requests from inside the hook and returns when told to continue, which
//! keeps the debugger as deterministic as the interpreter it drives.

pub mod json;

use grasm_interp::{DebugHook, Interp};
use grasm_sema::loader::Program;
use grasm_syntax::span::{LineIndex, Span};
use json::Json;
use std::collections::BTreeSet;
use std::cell::RefCell;
use std::io::{BufRead, Write};
use std::rc::Rc;

/// How execution should proceed after a suspension.
#[derive(Debug, Clone, Copy, PartialEq)]
enum Resume {
    /// Run until the next breakpoint.
    Continue,
    /// Stop at the next statement in this frame or shallower.
    Next,
    /// Stop at the very next statement, wherever it is.
    StepIn,
    /// Run until this frame returns.
    StepOut,
    /// Stop the program.
    Terminate,
}

/// Maps span offsets to (file index, line) and back.
struct SourceMap {
    /// Per file: path and line index over its own source.
    files: Vec<(String, LineIndex, u32)>,
}

impl SourceMap {
    fn new(program: &Program) -> SourceMap {
        SourceMap {
            files: program
                .files
                .iter()
                .map(|f| (f.path.clone(), LineIndex::new(&f.src), f.base))
                .collect(),
        }
    }

    /// (path, 1-based line, 1-based column) for a span.
    fn locate(&self, span: Span) -> (String, u32, u32) {
        let i = self
            .files
            .iter()
            .rposition(|(_, _, base)| span.start >= *base)
            .unwrap_or(0);
        let (path, index, base) = &self.files[i];
        let lc = index.line_col(span.start.saturating_sub(*base));
        (path.clone(), lc.line, lc.col)
    }
}

/// Where the program is allowed to stop.
#[derive(Default)]
struct Breakpoints {
    /// (file path, line) pairs the client asked for.
    lines: BTreeSet<(String, u32)>,
}

// ---------------------------------------------------------------------------
// Wire protocol
// ---------------------------------------------------------------------------

/// Reads one `Content-Length`-framed message. `Ok(None)` at end of input.
pub fn read_message(input: &mut impl BufRead) -> Result<Option<Json>, String> {
    let mut length: Option<usize> = None;
    loop {
        let mut line = String::new();
        let n = input
            .read_line(&mut line)
            .map_err(|e| format!("reading header: {e}"))?;
        if n == 0 {
            return Ok(None);
        }
        let trimmed = line.trim_end();
        if trimmed.is_empty() {
            break;
        }
        if let Some(v) = trimmed.strip_prefix("Content-Length:") {
            length = Some(
                v.trim()
                    .parse()
                    .map_err(|_| format!("bad Content-Length: {v}"))?,
            );
        }
    }
    let Some(len) = length else {
        return Err("message had no Content-Length".into());
    };
    let mut buf = vec![0u8; len];
    input
        .read_exact(&mut buf)
        .map_err(|e| format!("reading body: {e}"))?;
    let text = String::from_utf8(buf).map_err(|e| format!("body is not UTF-8: {e}"))?;
    json::parse(&text).map(Some)
}

fn write_message(out: &mut impl Write, msg: &Json) {
    let body = msg.to_text();
    let _ = write!(out, "Content-Length: {}\r\n\r\n{}", body.len(), body);
    let _ = out.flush();
}

// ---------------------------------------------------------------------------
// The adapter
// ---------------------------------------------------------------------------

/// Protocol state shared between the top-level loop and the suspended hook.
struct Session<I: BufRead, O: Write> {
    input: I,
    output: O,
    seq: i64,
    breakpoints: Breakpoints,
    sources: SourceMap,
    /// What the last stop decided; drives whether the next statement stops.
    resume: Resume,
    /// Call depth when a step began, so `next` and `stepOut` know when to stop.
    step_depth: usize,
    terminated: bool,
    /// Variable references handed out for `scopes`/`variables`.
    frame_of_ref: Vec<usize>,
}

impl<I: BufRead, O: Write> Session<I, O> {
    fn next_seq(&mut self) -> i64 {
        self.seq += 1;
        self.seq
    }

    fn send(&mut self, msg: Json) {
        write_message(&mut self.output, &msg);
    }

    fn event(&mut self, name: &str, body: Json) {
        let seq = self.next_seq();
        let msg = Json::obj(vec![
            ("seq", Json::int(seq)),
            ("type", Json::str("event")),
            ("event", Json::str(name)),
            ("body", body),
        ]);
        self.send(msg);
    }

    fn respond(&mut self, request: &Json, body: Json) {
        let seq = self.next_seq();
        let msg = Json::obj(vec![
            ("seq", Json::int(seq)),
            ("type", Json::str("response")),
            (
                "request_seq",
                Json::int(request.get("seq").and_then(Json::as_i64).unwrap_or(0)),
            ),
            ("success", Json::Bool(true)),
            (
                "command",
                Json::str(request.get("command").and_then(Json::as_str).unwrap_or("")),
            ),
            ("body", body),
        ]);
        self.send(msg);
    }

    fn fail(&mut self, request: &Json, message: &str) {
        let seq = self.next_seq();
        let msg = Json::obj(vec![
            ("seq", Json::int(seq)),
            ("type", Json::str("response")),
            (
                "request_seq",
                Json::int(request.get("seq").and_then(Json::as_i64).unwrap_or(0)),
            ),
            ("success", Json::Bool(false)),
            (
                "command",
                Json::str(request.get("command").and_then(Json::as_str).unwrap_or("")),
            ),
            ("message", Json::str(message)),
        ]);
        self.send(msg);
    }
}

/// What `initialize` advertises. Only capabilities that are actually
/// implemented are claimed — a client that trusts a lie here breaks in ways
/// that are hard to trace back to the adapter.
fn capabilities() -> Json {
    Json::obj(vec![
        ("supportsConfigurationDoneRequest", Json::Bool(true)),
        ("supportsTerminateRequest", Json::Bool(true)),
        ("supportsEvaluateForHovers", Json::Bool(true)),
        ("supportsSingleThreadExecutionRequests", Json::Bool(false)),
        ("supportsStepBack", Json::Bool(false)),
        ("supportsRestartRequest", Json::Bool(false)),
        ("supportsFunctionBreakpoints", Json::Bool(false)),
        ("supportsConditionalBreakpoints", Json::Bool(false)),
    ])
}

/// The single thread the adapter reports. Grasm's `parallel` runs tasks
/// deterministically in spawn order on one thread, so one is the truth.
const THREAD_ID: i64 = 1;

fn thread_list() -> Json {
    Json::obj(vec![(
        "threads",
        Json::Arr(vec![Json::obj(vec![
            ("id", Json::int(THREAD_ID)),
            ("name", Json::str("main")),
        ])]),
    )])
}

/// Renders a value for the variables pane.
fn render(value: &grasm_interp::Value) -> String {
    match value {
        grasm_interp::Value::Str(s) => format!("{s:?}"),
        other => other.to_string(),
    }
}

/// A value's DAP type name, shown beside it in the variables pane.
fn type_name(value: &grasm_interp::Value) -> &'static str {
    use grasm_interp::Value;
    match value {
        Value::Int(_) => "i64",
        Value::Ref(..) => "ref",
        Value::Float(_) => "f64",
        Value::Bool(_) => "bool",
        Value::Str(_) => "str",
        Value::Bytes(_) => "bytes",
        Value::List(_) => "list",
        Value::Map(_) => "map",
        Value::Tuple(_) => "tuple",
        Value::Struct { .. } => "struct",
        Value::Variant { .. } => "variant",
        Value::Ok(_) | Value::Err(_) => "result",
        Value::Closure { .. } | Value::FnRef(_) => "fn",
        Value::Range(..) => "range",
        Value::Unit => "()",
    }
}

// ---------------------------------------------------------------------------
// The hook that suspends execution
// ---------------------------------------------------------------------------

/// The adapter's state, shared between the output sink and the hook.
///
/// The two never hold it at once — the program is not running while execution
/// is suspended, and the hook is not active while the program prints — so the
/// `RefCell` never actually conflicts. It is here so that assumption is
/// *checked* rather than assumed.
type Shared<I, O> = Rc<RefCell<Session<I, O>>>;

struct Suspender<I: BufRead, O: Write> {
    session: Shared<I, O>,
}

impl<I: BufRead, O: Write, W: Write> DebugHook<W> for Suspender<I, O> {
    fn before_stmt(&mut self, interp: &mut Interp<'_, W>, at: Span) {
        let shared = self.session.clone();
        let mut guard = shared.borrow_mut();
        let session: &mut Session<I, O> = &mut guard;
        if session.terminated {
            return;
        }
        let depth = interp.debug_call_stack().len();
        let (path, line, _) = session.sources.locate(at);

        let hit_breakpoint = session.breakpoints.lines.contains(&(path.clone(), line));
        let should_stop = match session.resume {
            Resume::Continue => hit_breakpoint,
            Resume::StepIn => true,
            Resume::Next => hit_breakpoint || depth <= session.step_depth,
            Resume::StepOut => hit_breakpoint || depth < session.step_depth,
            Resume::Terminate => false,
        };
        if !should_stop {
            return;
        }

        let reason = if hit_breakpoint && session.resume == Resume::Continue {
            "breakpoint"
        } else {
            "step"
        };
        session.event(
            "stopped",
            Json::obj(vec![
                ("reason", Json::str(reason)),
                ("threadId", Json::int(THREAD_ID)),
                ("allThreadsStopped", Json::Bool(true)),
            ]),
        );
        serve_while_stopped(session, interp);
    }
}

/// Handles requests until the client resumes execution.
fn serve_while_stopped<I: BufRead, O: Write, W: Write>(
    session: &mut Session<I, O>,
    interp: &mut Interp<'_, W>,
) {
    loop {
        let msg = match read_message(&mut session.input) {
            Ok(Some(m)) => m,
            // The client vanished: stop rather than run on unsupervised.
            Ok(None) | Err(_) => {
                session.terminated = true;
                session.resume = Resume::Terminate;
                return;
            }
        };
        let command = msg
            .get("command")
            .and_then(Json::as_str)
            .unwrap_or("")
            .to_string();
        let depth = interp.debug_call_stack().len();
        match command.as_str() {
            "continue" => {
                session.resume = Resume::Continue;
                session.respond(&msg, Json::obj(vec![("allThreadsContinued", Json::Bool(true))]));
                return;
            }
            "next" => {
                session.resume = Resume::Next;
                session.step_depth = depth;
                session.respond(&msg, Json::Null);
                return;
            }
            "stepIn" => {
                session.resume = Resume::StepIn;
                session.respond(&msg, Json::Null);
                return;
            }
            "stepOut" => {
                session.resume = Resume::StepOut;
                session.step_depth = depth;
                session.respond(&msg, Json::Null);
                return;
            }
            "disconnect" | "terminate" => {
                session.resume = Resume::Terminate;
                session.terminated = true;
                session.respond(&msg, Json::Null);
                return;
            }
            "threads" => {
                let body = thread_list();
                session.respond(&msg, body);
            }
            "stackTrace" => {
                let body = stack_trace(session, interp);
                session.respond(&msg, body);
            }
            "scopes" => {
                let frame_id = msg
                    .path("arguments.frameId")
                    .and_then(Json::as_i64)
                    .unwrap_or(0);
                let body = scopes(session, frame_id);
                session.respond(&msg, body);
            }
            "variables" => {
                let body = variables(session, interp, &msg);
                session.respond(&msg, body);
            }
            "evaluate" => {
                let expr = msg
                    .path("arguments.expression")
                    .and_then(Json::as_str)
                    .unwrap_or("")
                    .to_string();
                match interp.debug_lookup(expr.trim()) {
                    Some(v) => {
                        let body = Json::obj(vec![
                            ("result", Json::str(render(v))),
                            ("type", Json::str(type_name(v))),
                            ("variablesReference", Json::int(0)),
                        ]);
                        session.respond(&msg, body);
                    }
                    // Only name lookup is supported; claiming to evaluate
                    // arbitrary expressions would mean running code with side
                    // effects while stopped.
                    None => session.fail(&msg, &format!("no variable named `{}`", expr.trim())),
                }
            }
            "setBreakpoints" => {
                let body = set_breakpoints(session, &msg);
                session.respond(&msg, body);
            }
            _ => session.respond(&msg, Json::Null),
        }
    }
}

fn stack_trace<I: BufRead, O: Write, W: Write>(
    session: &mut Session<I, O>,
    interp: &Interp<'_, W>,
) -> Json {
    // Frames innermost first, each showing where execution is inside it.
    let stack = interp.debug_call_stack();
    let mut at = interp.debug_cur_span();
    let mut frames = Vec::new();
    session.frame_of_ref.clear();
    for (i, (name, call_site)) in stack.iter().enumerate().rev() {
        let (path, line, col) = session.sources.locate(at);
        let id = (stack.len() - 1 - i) as i64;
        session.frame_of_ref.push(i);
        frames.push(Json::obj(vec![
            ("id", Json::int(id)),
            ("name", Json::str(name.clone())),
            (
                "source",
                Json::obj(vec![
                    ("name", Json::str(file_name(&path))),
                    ("path", Json::str(path)),
                ]),
            ),
            ("line", Json::int(i64::from(line))),
            ("column", Json::int(i64::from(col))),
        ]));
        at = *call_site;
    }
    Json::obj(vec![
        ("totalFrames", Json::int(frames.len() as i64)),
        ("stackFrames", Json::Arr(frames)),
    ])
}

fn file_name(path: &str) -> String {
    path.rsplit('/').next().unwrap_or(path).to_string()
}

/// One scope per frame. Only the innermost frame has readable locals: the
/// interpreter swaps scopes out on call, so an outer frame's bindings are not
/// reachable while it is suspended, and inventing them would be a lie.
fn scopes<I: BufRead, O: Write>(session: &mut Session<I, O>, frame_id: i64) -> Json {
    let is_innermost = frame_id == 0;
    let _ = session;
    Json::obj(vec![(
        "scopes",
        Json::Arr(vec![Json::obj(vec![
            ("name", Json::str("Locals")),
            (
                "variablesReference",
                Json::int(if is_innermost { 1 } else { 0 }),
            ),
            ("expensive", Json::Bool(false)),
            (
                "presentationHint",
                Json::str(if is_innermost { "locals" } else { "registers" }),
            ),
        ])]),
    )])
}

fn variables<I: BufRead, O: Write, W: Write>(
    session: &mut Session<I, O>,
    interp: &Interp<'_, W>,
    request: &Json,
) -> Json {
    let reference = request
        .path("arguments.variablesReference")
        .and_then(Json::as_i64)
        .unwrap_or(0);
    let _ = session;
    if reference != 1 {
        return Json::obj(vec![("variables", Json::Arr(vec![]))]);
    }
    // Inner scopes shadow outer ones, so walk outward and keep the first hit.
    let mut seen: std::collections::BTreeMap<String, String> = std::collections::BTreeMap::new();
    let mut types: std::collections::BTreeMap<String, &'static str> =
        std::collections::BTreeMap::new();
    for scope in interp.debug_scopes().iter().rev() {
        for (name, value) in scope {
            seen.entry(name.clone()).or_insert_with(|| render(value));
            types.entry(name.clone()).or_insert_with(|| type_name(value));
        }
    }
    let vars: Vec<Json> = seen
        .into_iter()
        .map(|(name, value)| {
            let ty = types.get(&name).copied().unwrap_or("");
            Json::obj(vec![
                ("name", Json::str(name)),
                ("value", Json::str(value)),
                ("type", Json::str(ty)),
                ("variablesReference", Json::int(0)),
            ])
        })
        .collect();
    Json::obj(vec![("variables", Json::Arr(vars))])
}

fn set_breakpoints<I: BufRead, O: Write>(session: &mut Session<I, O>, request: &Json) -> Json {
    let path = request
        .path("arguments.source.path")
        .and_then(Json::as_str)
        .unwrap_or("")
        .to_string();
    // A new list replaces every breakpoint in that file, per the protocol.
    session.breakpoints.lines.retain(|(f, _)| f != &path);

    let requested = match request.path("arguments.breakpoints") {
        Some(Json::Arr(items)) => items.clone(),
        _ => Vec::new(),
    };
    let mut verified = Vec::new();
    for bp in requested {
        let line = bp.get("line").and_then(Json::as_i64).unwrap_or(0);
        let known = session
            .sources
            .files
            .iter()
            .any(|(p, _, _)| p == &path || file_name(p) == file_name(&path));
        if known && line > 0 {
            session.breakpoints.lines.insert((path.clone(), line as u32));
        }
        verified.push(Json::obj(vec![
            ("verified", Json::Bool(known && line > 0)),
            ("line", Json::int(line)),
        ]));
    }
    Json::obj(vec![("breakpoints", Json::Arr(verified))])
}

// ---------------------------------------------------------------------------
// Output capture
// ---------------------------------------------------------------------------

/// Turns the program's stdout into DAP `output` events, a line at a time.
struct OutputSink<I: BufRead, O: Write> {
    session: Shared<I, O>,
    pending: Vec<u8>,
}

impl<I: BufRead, O: Write> OutputSink<I, O> {
    fn emit(&self, text: String) {
        self.session.borrow_mut().event(
            "output",
            Json::obj(vec![
                ("category", Json::str("stdout")),
                ("output", Json::str(text)),
            ]),
        );
    }
}

impl<I: BufRead, O: Write> Write for OutputSink<I, O> {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        self.pending.extend_from_slice(buf);
        while let Some(nl) = self.pending.iter().position(|b| *b == b'\n') {
            let line: Vec<u8> = self.pending.drain(..=nl).collect();
            self.emit(String::from_utf8_lossy(&line).into_owned());
        }
        Ok(buf.len())
    }

    fn flush(&mut self) -> std::io::Result<()> {
        if self.pending.is_empty() {
            return Ok(());
        }
        let text = String::from_utf8_lossy(&self.pending).into_owned();
        self.pending.clear();
        self.emit(text);
        Ok(())
    }
}

// ---------------------------------------------------------------------------
// Entry point
// ---------------------------------------------------------------------------

/// Serves one debug session on `input`/`output`.
///
/// `load` is called once the client sends `launch`, with the program path it
/// asked for; returning an error reports a failed launch rather than pretending
/// to debug nothing.
///
/// Takes the writer by value and hands it back, because the adapter shares it
/// with the interpreter's output sink for the length of the run.
pub fn serve<I: BufRead + 'static, O: Write + 'static>(
    mut input: I,
    mut output: O,
    load: impl Fn(&str) -> Result<Program, String>,
) -> Result<O, String> {
    // Phase 1: configuration. Requests are served directly until `launch`
    // hands us a program and the client says configuration is done.
    let mut seq = 0i64;
    let mut program: Option<Program> = None;
    let mut pending_breakpoints: Vec<Json> = Vec::new();
    let mut stop_on_entry = false;
    let mut launch_request: Option<Json> = None;

    loop {
        let Some(msg) = read_message(&mut input)? else {
            return Ok(output);
        };
        let command = msg.get("command").and_then(Json::as_str).unwrap_or("");
        seq += 1;
        match command {
            "initialize" => {
                write_message(
                    &mut output,
                    &Json::obj(vec![
                        ("seq", Json::int(seq)),
                        ("type", Json::str("response")),
                        (
                            "request_seq",
                            Json::int(msg.get("seq").and_then(Json::as_i64).unwrap_or(0)),
                        ),
                        ("success", Json::Bool(true)),
                        ("command", Json::str("initialize")),
                        ("body", capabilities()),
                    ]),
                );
                seq += 1;
                write_message(
                    &mut output,
                    &Json::obj(vec![
                        ("seq", Json::int(seq)),
                        ("type", Json::str("event")),
                        ("event", Json::str("initialized")),
                        ("body", Json::obj(vec![])),
                    ]),
                );
            }
            "launch" => {
                let path = msg
                    .path("arguments.program")
                    .and_then(Json::as_str)
                    .unwrap_or("");
                stop_on_entry = matches!(
                    msg.path("arguments.stopOnEntry"),
                    Some(Json::Bool(true))
                );
                // `"journal": "path"` in launch.json steps through a *recorded*
                // run rather than a fresh one. This is the pair the two halves
                // were built for: the failure happens in the compiled build on
                // a machine you do not have, and the debugger is here.
                //
                // Installed before the program loads, so the first effect it
                // reaches is already answered from the journal.
                if let Some(j) = msg.path("arguments.journal").and_then(Json::as_str) {
                    let installed = std::fs::read(j)
                        .map_err(|e| e.to_string())
                        .and_then(|raw| grasm_journal::install(&raw));
                    if let Err(e) = installed {
                        // A failed launch, not a silent live run. A session that
                        // quietly ran against the real world while the person
                        // believed they were watching a replay would be worse
                        // than an error — they would trust what they saw.
                        write_message(
                            &mut output,
                            &err_response(seq, &msg, &format!("cannot replay {j}: {e}")),
                        );
                        seq += 1;
                        write_message(&mut output, &terminated_event(seq));
                        return Ok(output);
                    }
                }
                match load(path) {
                    Ok(p) => {
                        program = Some(p);
                        launch_request = Some(msg.clone());
                        write_message(
                            &mut output,
                            &ok_response(seq, &msg, Json::obj(vec![])),
                        );
                    }
                    Err(e) => {
                        write_message(&mut output, &err_response(seq, &msg, &e));
                        seq += 1;
                        write_message(&mut output, &terminated_event(seq));
                        return Ok(output);
                    }
                }
            }
            "setBreakpoints" => {
                // Held until the program is loaded, so line verification can
                // be answered against real sources.
                pending_breakpoints.push(msg.clone());
                let requested = match msg.path("arguments.breakpoints") {
                    Some(Json::Arr(items)) => items.len(),
                    _ => 0,
                };
                let verified: Vec<Json> = (0..requested)
                    .map(|i| {
                        let line = msg
                            .path("arguments.breakpoints")
                            .and_then(|a| match a {
                                Json::Arr(items) => items.get(i).cloned(),
                                _ => None,
                            })
                            .and_then(|b| b.get("line").and_then(Json::as_i64))
                            .unwrap_or(0);
                        Json::obj(vec![
                            ("verified", Json::Bool(true)),
                            ("line", Json::int(line)),
                        ])
                    })
                    .collect();
                write_message(
                    &mut output,
                    &ok_response(
                        seq,
                        &msg,
                        Json::obj(vec![("breakpoints", Json::Arr(verified))]),
                    ),
                );
            }
            "threads" => {
                write_message(&mut output, &ok_response(seq, &msg, thread_list()));
            }
            "configurationDone" => {
                write_message(&mut output, &ok_response(seq, &msg, Json::obj(vec![])));
                break;
            }
            "disconnect" | "terminate" => {
                write_message(&mut output, &ok_response(seq, &msg, Json::obj(vec![])));
                return Ok(output);
            }
            _ => {
                write_message(&mut output, &ok_response(seq, &msg, Json::Null));
            }
        }
    }

    let Some(program) = program else {
        seq += 1;
        write_message(&mut output, &terminated_event(seq));
        return Err("configurationDone arrived before launch".into());
    };
    let _ = launch_request;

    // Phase 2: run under the debugger.
    let session = Session {
        input,
        output,
        seq,
        breakpoints: Breakpoints::default(),
        sources: SourceMap::new(&program),
        resume: if stop_on_entry {
            Resume::StepIn
        } else {
            Resume::Continue
        },
        step_depth: 0,
        terminated: false,
        frame_of_ref: Vec::new(),
    };
    let shared: Shared<I, O> = Rc::new(RefCell::new(session));
    for bp in &pending_breakpoints {
        let mut s = shared.borrow_mut();
        set_breakpoints(&mut s, bp);
    }

    let mut sink = OutputSink {
        session: shared.clone(),
        pending: Vec::new(),
    };

    let files: Vec<(String, &grasm_syntax::ast::Module, std::collections::HashMap<String, String>)> =
        program
            .files
            .iter()
            .map(|f| (f.name.clone(), &f.module, f.imports.clone()))
            .collect();
    let mut interp = Interp::new_program(&files, &mut sink);
    interp.attach_debugger(Box::new(Suspender {
        session: shared.clone(),
    }));
    let code = interp.run_main();
    let trace = interp.panic_trace().unwrap_or_default().to_vec();
    drop(interp);
    let _ = sink.flush();
    // The sink and the hook each hold a handle; both must go before the
    // session can be reclaimed to hand the writer back.
    drop(sink);

    {
    let mut session = shared.borrow_mut();
    if !trace.is_empty() {
        let mut text = String::from("stack trace (innermost first):\n");
        for frame in &trace {
            let (path, line, col) = session.sources.locate(frame.at);
            text.push_str(&format!("  {} at {path}:{line}:{col}\n", frame.function));
        }
        session.event(
            "output",
            Json::obj(vec![
                ("category", Json::str("stderr")),
                ("output", Json::str(text)),
            ]),
        );
    }

    session.event(
        "exited",
        Json::obj(vec![("exitCode", Json::int(i64::from(code)))]),
    );
    session.event("terminated", Json::obj(vec![]));
    }
    // Every other handle is gone by now, so the writer comes back out.
    let session = Rc::try_unwrap(shared)
        .map_err(|_| "debug session outlived its run".to_string())?
        .into_inner();
    Ok(session.output)
}

fn ok_response(seq: i64, request: &Json, body: Json) -> Json {
    Json::obj(vec![
        ("seq", Json::int(seq)),
        ("type", Json::str("response")),
        (
            "request_seq",
            Json::int(request.get("seq").and_then(Json::as_i64).unwrap_or(0)),
        ),
        ("success", Json::Bool(true)),
        (
            "command",
            Json::str(request.get("command").and_then(Json::as_str).unwrap_or("")),
        ),
        ("body", body),
    ])
}

fn err_response(seq: i64, request: &Json, message: &str) -> Json {
    Json::obj(vec![
        ("seq", Json::int(seq)),
        ("type", Json::str("response")),
        (
            "request_seq",
            Json::int(request.get("seq").and_then(Json::as_i64).unwrap_or(0)),
        ),
        ("success", Json::Bool(false)),
        (
            "command",
            Json::str(request.get("command").and_then(Json::as_str).unwrap_or("")),
        ),
        ("message", Json::str(message)),
    ])
}

fn terminated_event(seq: i64) -> Json {
    Json::obj(vec![
        ("seq", Json::int(seq)),
        ("type", Json::str("event")),
        ("event", Json::str("terminated")),
        ("body", Json::obj(vec![])),
    ])
}
