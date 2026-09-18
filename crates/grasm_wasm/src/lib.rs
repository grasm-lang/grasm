//! Grasm in a browser: the reference interpreter, compiled to wasm32.
//!
//! Why this exists. Every claim this project makes is checked by running
//! programs, and until now running one meant cloning a repository and building
//! a Rust workspace. That is a barrier in front of the only evidence the
//! project has. This module removes it: the language is a link.
//!
//! Why the *interpreter* rather than a wasm code generator. The interpreter is
//! the normative engine — `grasm run` is what the other two are checked against
//! — and it depends on nothing but `grasm_syntax` and `grasm_sema`, neither of
//! which touches the host. So this is the whole frontend and the whole
//! semantics with no reimplementation, and nothing here can disagree with
//! `grasm run`, because it *is* `grasm run`. A wasm backend would be a fourth
//! engine and would have to enter the agreement gate to be trusted; this is the
//! same engine on a different machine.
//!
//! What is not available. wasm32-unknown-unknown has no threads, no sockets and
//! no filesystem, so `par_map` runs on one core, `net` cannot open a socket, and
//! `read_file`/`write_file` fail as they would on a machine with no such file.
//! Each is a refusal at run time with the ordinary error, never a wrong answer.
//! The stack is fixed at link time, so [E-RECURSION-DEPTH]'s depth is not
//! reachable here either.
//!
//! # The ABI
//!
//! Four exports, and no dependency on a binding generator.
//!
//! * `grasm_alloc(len) -> ptr` — reserve `len` bytes for the caller to write.
//! * `grasm_free(ptr, len)` — release what `grasm_alloc` or `grasm_run` gave.
//! * `grasm_run(ptr, len) -> ptr` — run the UTF-8 source at `ptr`, and answer
//!   with a buffer laid out as little-endian `u32`s:
//! * `grasm_run_with_stdin(ptr, len, in_ptr, in_len) -> ptr` — the same, with
//!   the lines `input()` should read. wasm32 has no stdin, so without this an
//!   interactive program cannot run in a browser at all.
//!
//! ```text
//! [0..4)   total byte length of this buffer, including these four
//! [4..8)   exit code, as an unsigned two's-complement i32
//! [8..12)  length of stdout
//! [12..)   stdout, UTF-8
//! then     length of the diagnostic text, then the text, UTF-8
//! ```
//!
//! The caller must hand the pointer back to `grasm_free` with the total length
//! it read out of the first field.

use grasm_syntax::LineIndex;

/// Reserves `len` bytes and hands back a pointer the caller may write to.
///
/// # Safety
/// The caller must eventually pass the pointer back to [`grasm_free`] with the
/// same `len`, and must not write past it.
#[no_mangle]
pub extern "C" fn grasm_alloc(len: usize) -> *mut u8 {
    let mut buf = Vec::<u8>::with_capacity(len);
    let ptr = buf.as_mut_ptr();
    std::mem::forget(buf);
    ptr
}

/// Releases a buffer from [`grasm_alloc`] or [`grasm_run`].
///
/// # Safety
/// `ptr` must have come from one of those two, with this exact `len`, and must
/// not be used afterwards.
#[no_mangle]
pub unsafe extern "C" fn grasm_free(ptr: *mut u8, len: usize) {
    if !ptr.is_null() && len > 0 {
        drop(Vec::from_raw_parts(ptr, len, len));
    }
}

/// Runs Grasm source and answers with the framed result described above.
///
/// # Safety
/// `ptr` must point at `len` initialised bytes of UTF-8.
#[no_mangle]
pub unsafe extern "C" fn grasm_run(ptr: *const u8, len: usize) -> *mut u8 {
    run_with(ptr, len, std::ptr::null(), 0)
}

/// Runs Grasm source with the lines `input()` should read.
///
/// wasm32 has no stdin, so without this every program built around `input()` is
/// unrunnable in a browser — which is most of the interactive ones. The lines
/// are a single UTF-8 buffer split on `\n`, because that is what a `<textarea>`
/// hands over and a second length-prefixed array would buy nothing.
///
/// # Safety
/// Both pointers must point at that many initialised bytes of UTF-8; the stdin
/// pointer may be null when `stdin_len` is 0.
#[no_mangle]
pub unsafe extern "C" fn grasm_run_with_stdin(
    ptr: *const u8,
    len: usize,
    stdin_ptr: *const u8,
    stdin_len: usize,
) -> *mut u8 {
    run_with(ptr, len, stdin_ptr, stdin_len)
}

/// Runs Grasm source against a **recorded journal**, so a failure that happened
/// somewhere else happens again here.
///
/// The journal is the bytes `GRASM_RECORD` wrote. On a host that path names a
/// file; wasm32 has no filesystem, which is the only reason this export exists
/// — and it is the reason worth having, because a browser tab is somewhere a
/// person can be handed a production failure and step through it without
/// installing anything.
///
/// A journal that cannot be read is a diagnostic rather than a silent normal
/// run: a page that quietly ran the program against the real world while the
/// reader believed they were watching a replay would be worse than an error.
///
/// # Safety
/// `ptr` must point at `len` initialised bytes of UTF-8, and `jrnl_ptr` at
/// `jrnl_len` initialised bytes.
#[no_mangle]
pub unsafe extern "C" fn grasm_run_with_journal(
    ptr: *const u8,
    len: usize,
    jrnl_ptr: *const u8,
    jrnl_len: usize,
) -> *mut u8 {
    let raw = if jrnl_ptr.is_null() || jrnl_len == 0 {
        &[][..]
    } else {
        std::slice::from_raw_parts(jrnl_ptr, jrnl_len)
    };
    if let Err(e) = grasm_journal::install(raw) {
        return frame(7, "", &format!("cannot replay: {e}"));
    }
    run_with(ptr, len, std::ptr::null(), 0)
}

unsafe fn run_with(ptr: *const u8, len: usize, stdin_ptr: *const u8, stdin_len: usize) -> *mut u8 {
    let src = std::slice::from_raw_parts(ptr, len);
    // Invalid UTF-8 is replaced rather than refused, for the same reason
    // `tcp_read` replaces it ([L-LIB-NET]): the caller is a browser and a
    // mangled byte should produce a diagnostic, not a trap with no message.
    let src = String::from_utf8_lossy(src).into_owned();
    // Always `Some`, even when empty. This embedder has no stdin to fall back
    // to — on wasm32 there is none, and on the host a test would block on the
    // real one — so "no lines supplied" must mean end of input, not "go and
    // read the process's stdin".
    let stdin = if stdin_ptr.is_null() || stdin_len == 0 {
        Some(Vec::new())
    } else {
        let raw = std::slice::from_raw_parts(stdin_ptr, stdin_len);
        let text = String::from_utf8_lossy(raw).into_owned();
        // A trailing newline would otherwise add an empty final line, which
        // every program in the corpus reads as "stop" one line early.
        let text = text.strip_suffix('\n').unwrap_or(&text).to_string();
        Some(text.split('\n').map(|l| l.to_string()).collect())
    };
    let (code, out, diags) = run_to_strings(&src, stdin);
    frame(code, &out, &diags)
}

/// The whole of running a program, with nothing that can panic across the ABI.
///
/// A panic unwinding out of an `extern "C"` function is undefined behaviour, and
/// a Grasm program is *allowed* to panic — that is what [E-PANIC] specifies. So
/// the fault is caught and returned as text, exactly as the conformance harness
/// reads it, rather than being allowed to reach the boundary.
fn run_to_strings(src: &str, stdin: Option<Vec<String>>) -> (i32, String, String) {
    let mut out: Vec<u8> = Vec::new();
    let (code, fault, diags) =
        grasm_interp::run_src_capturing_fault_with_stdin(src, &mut out, stdin);
    let stdout = String::from_utf8_lossy(&out).into_owned();
    if !diags.is_empty() {
        let index = LineIndex::new(src);
        let text = diags
            .iter()
            .map(|d| d.render("<browser>", src, &index))
            .collect::<Vec<_>>()
            .join("\n");
        return (code, stdout, text);
    }
    (code, stdout, fault.unwrap_or_default())
}

/// Packs the result into the single buffer the ABI describes.
fn frame(code: i32, out: &str, err: &str) -> *mut u8 {
    let total = 4 + 4 + 4 + out.len() + 4 + err.len();
    let mut buf = Vec::<u8>::with_capacity(total);
    buf.extend_from_slice(&(total as u32).to_le_bytes());
    buf.extend_from_slice(&(code as u32).to_le_bytes());
    buf.extend_from_slice(&(out.len() as u32).to_le_bytes());
    buf.extend_from_slice(out.as_bytes());
    buf.extend_from_slice(&(err.len() as u32).to_le_bytes());
    buf.extend_from_slice(err.as_bytes());
    debug_assert_eq!(buf.len(), total);
    let ptr = buf.as_mut_ptr();
    std::mem::forget(buf);
    ptr
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The framing is the contract with the browser, so it is checked here
    /// rather than only in JavaScript — a test that runs on every `cargo test`
    /// on the host, not only when somebody opens a page.
    fn unframe(ptr: *mut u8) -> (i32, String, String) {
        unsafe {
            let u32_at = |off: usize| {
                let mut b = [0u8; 4];
                b.copy_from_slice(std::slice::from_raw_parts(ptr.add(off), 4));
                u32::from_le_bytes(b)
            };
            let total = u32_at(0) as usize;
            let code = u32_at(4) as i32;
            let out_len = u32_at(8) as usize;
            let out = std::str::from_utf8(std::slice::from_raw_parts(ptr.add(12), out_len))
                .unwrap()
                .to_string();
            let err_off = 12 + out_len;
            let err_len = u32_at(err_off) as usize;
            let err =
                std::str::from_utf8(std::slice::from_raw_parts(ptr.add(err_off + 4), err_len))
                    .unwrap()
                    .to_string();
            assert_eq!(
                total,
                err_off + 4 + err_len,
                "the length field must be true"
            );
            grasm_free(ptr, total);
            (code, out, err)
        }
    }

    fn run(src: &str) -> (i32, String, String) {
        unsafe { unframe(grasm_run(src.as_ptr(), src.len())) }
    }

    /// `input()` reads the lines the embedder supplied.
    ///
    /// wasm32 has no stdin. Checked here on the host, because the alternative
    /// is finding out in a browser that half the corpus cannot run.
    #[test]
    fn supplied_stdin_reaches_input() {
        let src = "fn main() {\n    var going = true\n    while going {\n        \
let line = input()\n        if line == \"\" {\n            going = false\n        \
} else {\n            print(\"got {line}\")\n        }\n    }\n}\n";
        let stdin = "one\ntwo\nthree\n";
        let ptr =
            unsafe { grasm_run_with_stdin(src.as_ptr(), src.len(), stdin.as_ptr(), stdin.len()) };
        let (code, out, err) = unframe(ptr);
        assert_eq!(code, 0, "{err}");
        assert_eq!(out, "got one\ngot two\ngot three\n");
    }

    /// With no stdin supplied, `input()` sees end of input rather than hanging
    /// or trapping.
    #[test]
    fn absent_stdin_is_end_of_input() {
        let src = "fn main() {\n    let line = input()\n    print(\"[{line}]\")\n}\n";
        let ptr = unsafe { grasm_run(src.as_ptr(), src.len()) };
        let (code, out, _) = unframe(ptr);
        assert_eq!(code, 0);
        assert_eq!(out, "[]\n");
    }

    #[test]
    fn a_program_that_prints_comes_back_framed() {
        let (code, out, err) = run("fn main() {\n    print(\"hello\")\n}\n");
        assert_eq!(code, 0);
        assert_eq!(out, "hello\n");
        assert!(err.is_empty(), "unexpected: {err}");
    }

    #[test]
    fn a_diagnostic_comes_back_as_text_rather_than_as_a_trap() {
        let (code, _, err) = run("fn main() {\n    let x: i64 = \"s\"\n}\n");
        assert_ne!(code, 0);
        assert!(err.contains("type mismatch"), "got: {err}");
    }

    /// A Grasm panic is specified behaviour ([E-PANIC]), and unwinding out of
    /// an `extern "C"` function is undefined. So it must arrive as text.
    #[test]
    fn a_panic_arrives_as_text_and_not_across_the_abi() {
        let (code, _, err) = run("fn main() {\n    let xs = [1]\n    print(\"{xs[5]}\")\n}\n");
        assert_ne!(code, 0);
        assert!(!err.is_empty(), "a panic must come back with its message");
    }

    /// The exit code survives the trip unchanged.
    ///
    /// `exit(-1)` is **255**, not `-1`: both engines already narrow an exit code
    /// the way a shell does, and this boundary must not be the place that
    /// second-guesses them. Collapsing an exit code is a defect this project has
    /// shipped once and caught — `exit_status(ExitCode)` turned every program's
    /// code into 1 — so what is checked here is agreement with `grasm run`
    /// rather than any opinion of this module's own.
    #[test]
    fn an_exit_code_survives_the_round_trip_exactly_as_grasm_run_gives_it() {
        let (code, _, _) = run("fn main() {\n    exit(-1)\n}\n");
        assert_eq!(code, 255, "`grasm run` and `--native` both give 255 here");
        let (code, _, _) = run("fn main() {\n    exit(3)\n}\n");
        assert_eq!(code, 3);
        let (code, _, _) = run("fn main() {\n    print(\"ok\")\n}\n");
        assert_eq!(code, 0);
    }

    #[test]
    fn an_empty_program_is_not_a_crash() {
        let (code, _, err) = run("");
        assert_ne!(code, 0);
        assert!(!err.is_empty());
    }
}
