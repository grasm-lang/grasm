//! The `grasm` CLI. Phase 1: `tokens` subcommand (debug lexing).
//! Later phases add: run, check, test, fmt, repl.

use std::process::ExitCode;

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    match args.first().map(String::as_str) {
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
            println!("grasm 0.1.0-dev (phase 1)");
            ExitCode::SUCCESS
        }
        _ => {
            eprintln!("grasm — the Grasm language toolchain (pre-alpha)");
            eprintln!();
            eprintln!("usage: grasm <command>");
            eprintln!();
            eprintln!("commands:");
            eprintln!("  tokens <file.gr>   dump lexer output (debug)");
            eprintln!("  version            print version");
            ExitCode::FAILURE
        }
    }
}
