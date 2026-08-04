//! The `grasm` CLI. Phase 3: `check` + `tokens`.
//! Later phases add: run, test, fmt, repl.

use std::process::ExitCode;

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    match args.first().map(String::as_str) {
        Some("run") => {
            let Some(path) = args.get(1) else {
                eprintln!("usage: grasm run <file.gr>");
                return ExitCode::FAILURE;
            };
            let src = match std::fs::read_to_string(path) {
                Ok(s) => s,
                Err(e) => {
                    eprintln!("error: cannot read `{path}`: {e}");
                    return ExitCode::FAILURE;
                }
            };
            let mut stdout = std::io::stdout();
            let (code, diags) = grasm_interp::run_src(&src, &mut stdout);
            report(path, &src, &diags);
            ExitCode::from(code.clamp(0, 255) as u8)
        }
        Some("test") => {
            let Some(path) = args.get(1) else {
                eprintln!("usage: grasm test <file.gr>");
                return ExitCode::FAILURE;
            };
            let src = match std::fs::read_to_string(path) {
                Ok(s) => s,
                Err(e) => {
                    eprintln!("error: cannot read `{path}`: {e}");
                    return ExitCode::FAILURE;
                }
            };
            let mut stdout = std::io::stdout();
            let (passed, failed, diags) = grasm_interp::test_src(&src, &mut stdout);
            report(path, &src, &diags);
            if !diags.is_empty() {
                return ExitCode::FAILURE;
            }
            println!("{passed} passed, {failed} failed");
            if failed > 0 {
                ExitCode::FAILURE
            } else {
                ExitCode::SUCCESS
            }
        }
        Some("check") => {
            let Some(path) = args.get(1) else {
                eprintln!("usage: grasm check <file.gr>");
                return ExitCode::FAILURE;
            };
            let src = match std::fs::read_to_string(path) {
                Ok(s) => s,
                Err(e) => {
                    eprintln!("error: cannot read `{path}`: {e}");
                    return ExitCode::FAILURE;
                }
            };
            let (_, diags) = grasm_sema::analyze(&src);
            report(path, &src, &diags);
            if diags.is_empty() {
                println!("ok: `{path}` has no errors");
                ExitCode::SUCCESS
            } else {
                ExitCode::FAILURE
            }
        }
        Some("tokens") => {
            let Some(path) = args.get(1) else {
                eprintln!("usage: grasm tokens <file.gr>");
                return ExitCode::FAILURE;
            };
            let src = match std::fs::read_to_string(path) {
                Ok(s) => s,
                Err(e) => {
                    eprintln!("error: cannot read `{path}`: {e}");
                    return ExitCode::FAILURE;
                }
            };
            let index = grasm_syntax::LineIndex::new(&src);
            let mut had_error = false;
            for tok in grasm_syntax::lex(&src) {
                let lc = index.line_col(tok.span.start);
                if let grasm_syntax::TokenKind::Error(msg) = &tok.kind {
                    had_error = true;
                    eprintln!("{path}:{}:{}: error: {msg}", lc.line, lc.col);
                } else {
                    println!("{}:{}\t{:?}", lc.line, lc.col, tok.kind);
                }
            }
            if had_error {
                return ExitCode::FAILURE;
            }
            ExitCode::SUCCESS
        }
        Some("version") | Some("--version") => {
            println!("grasm 0.1.0-dev (phase 3)");
            ExitCode::SUCCESS
        }
        _ => {
            eprintln!("grasm — the Grasm language toolchain (pre-alpha)");
            eprintln!();
            eprintln!("usage: grasm <command>");
            eprintln!();
            eprintln!("commands:");
            eprintln!("  check <file.gr>    parse + typecheck");
            eprintln!("  tokens <file.gr>   dump lexer output (debug)");
            eprintln!("  version            print version");
            ExitCode::FAILURE
        }
    }
}

fn report(path: &str, src: &str, diags: &[grasm_syntax::Diagnostic]) {
    let index = grasm_syntax::LineIndex::new(src);
    for d in diags {
        eprintln!("{}", d.render(path, src, &index));
    }
    if !diags.is_empty() {
        let n = diags.len();
        eprintln!("{n} error(s) found");
    }
}
