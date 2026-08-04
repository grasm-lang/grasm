//! The `grasm` CLI: run / test / check / fmt / build / tokens.
//!
//! `grasm build` produces self-contained executables by appending a source
//! bundle to a copy of this binary; at startup we check our own tail for the
//! bundle magic and, if present, run it directly.

use std::io::Write as _;
use std::process::ExitCode;

use grasm_sema::loader::{self, Program};

const BUNDLE_MAGIC: &[u8; 8] = b"GRASMBDL";

fn main() -> ExitCode {
    // Bundled-executable mode: our own binary carries a program.
    if let Some(bundle) = read_own_bundle() {
        let args: Vec<String> = std::env::args().skip(1).collect();
        return run_bundle(&bundle, &args);
    }

    let args: Vec<String> = std::env::args().skip(1).collect();
    match args.first().map(String::as_str) {
        Some("run") => {
            let Some(path) = args.get(1) else {
                eprintln!("usage: grasm run <file.gr> [-- args...]");
                return ExitCode::FAILURE;
            };
            let prog_args: Vec<String> = args
                .iter()
                .skip_while(|a| *a != "--")
                .skip(1)
                .cloned()
                .collect();
            let program = match load_and_check(path) {
                Ok(p) => p,
                Err(code) => return code,
            };
            let mut stdout = std::io::stdout();
            let code = grasm_interp::run_program(&program, &prog_args, &mut stdout);
            ExitCode::from(code.clamp(0, 255) as u8)
        }
        Some("test") => {
            let Some(path) = args.get(1) else {
                eprintln!("usage: grasm test <file.gr>");
                return ExitCode::FAILURE;
            };
            let program = match load_and_check(path) {
                Ok(p) => p,
                Err(code) => return code,
            };
            let mut stdout = std::io::stdout();
            let (passed, failed) = grasm_interp::test_program(&program, &mut stdout);
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
            match load_and_check(path) {
                Ok(_) => {
                    println!("ok: `{path}` has no errors");
                    ExitCode::SUCCESS
                }
                Err(code) => code,
            }
        }
        Some("build") => {
            let Some(path) = args.get(1) else {
                eprintln!("usage: grasm build <file.gr> [-o output]");
                return ExitCode::FAILURE;
            };
            let out_path = args
                .iter()
                .position(|a| a == "-o")
                .and_then(|i| args.get(i + 1))
                .cloned()
                .unwrap_or_else(|| {
                    std::path::Path::new(path)
                        .file_stem()
                        .map(|s| s.to_string_lossy().to_string())
                        .unwrap_or_else(|| "app".to_string())
                });
            let program = match load_and_check(path) {
                Ok(p) => p,
                Err(code) => return code,
            };
            if args.iter().any(|a| a == "--native") {
                return match build_native(&program, &out_path) {
                    Ok(size) => {
                        println!(
                            "built `{out_path}` natively ({size} bytes, Cranelift)",
                        );
                        ExitCode::SUCCESS
                    }
                    Err(e) => {
                        eprintln!("error: {e}");
                        ExitCode::FAILURE
                    }
                };
            }
            match write_bundle(&program, &out_path) {
                Ok(size) => {
                    println!("built `{out_path}` ({size} bytes, {} module(s))", program.files.len());
                    ExitCode::SUCCESS
                }
                Err(e) => {
                    eprintln!("error: {e}");
                    ExitCode::FAILURE
                }
            }
        }
        Some("fmt") => {
            let Some(path) = args.get(1) else {
                eprintln!("usage: grasm fmt <file.gr> [--check]");
                return ExitCode::FAILURE;
            };
            let src = match std::fs::read_to_string(path) {
                Ok(s) => s,
                Err(e) => {
                    eprintln!("error: cannot read `{path}`: {e}");
                    return ExitCode::FAILURE;
                }
            };
            let formatted = grasm_syntax::fmt::format_src(&src);
            if args.iter().any(|a| a == "--check") {
                if formatted == src {
                    println!("ok: `{path}` is formatted");
                    ExitCode::SUCCESS
                } else {
                    eprintln!("`{path}` is not canonically formatted (run `grasm fmt {path}`)");
                    ExitCode::FAILURE
                }
            } else if formatted != src {
                if let Err(e) = std::fs::write(path, &formatted) {
                    eprintln!("error: cannot write `{path}`: {e}");
                    return ExitCode::FAILURE;
                }
                println!("formatted `{path}`");
                ExitCode::SUCCESS
            } else {
                println!("`{path}` already formatted");
                ExitCode::SUCCESS
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
            println!("grasm 0.2.0-dev (phase 2: modules, maps, build)");
            ExitCode::SUCCESS
        }
        _ => {
            eprintln!("grasm — the Grasm language toolchain (pre-alpha)");
            eprintln!();
            eprintln!("usage: grasm <command>");
            eprintln!();
            eprintln!("commands:");
            eprintln!("  run <file.gr> [-- args...]   check + execute main (follows imports)");
            eprintln!("  test <file.gr>               run test blocks");
            eprintln!("  check <file.gr>              parse + typecheck");
            eprintln!("  build <file.gr> [-o out]     self-contained executable (--native: Cranelift AOT)");
            eprintln!("  fmt <file.gr> [--check]      canonical formatting");
            eprintln!("  tokens <file.gr>             dump lexer output (debug)");
            eprintln!("  version                      print version");
            ExitCode::FAILURE
        }
    }
}

/// Load (following imports), typecheck, render diagnostics per file.
fn load_and_check(path: &str) -> Result<Program, ExitCode> {
    let program = match loader::load_from_disk(std::path::Path::new(path)) {
        Ok(p) => p,
        Err(e) => {
            eprintln!("error: {e}");
            return Err(ExitCode::FAILURE);
        }
    };
    let mut diags = program.diagnostics.clone();
    if diags.is_empty() {
        diags.extend(grasm_sema::check::check_program(&program).diagnostics);
    }
    if diags.is_empty() {
        Ok(program)
    } else {
        render_diags(&program, &diags);
        Err(ExitCode::FAILURE)
    }
}

fn render_diags(program: &Program, diags: &[grasm_syntax::Diagnostic]) {
    for d in diags {
        let idx = program.file_of(d.span);
        let f = &program.files[idx];
        let local = grasm_syntax::Span::new(
            d.span.start.saturating_sub(f.base),
            d.span.end.saturating_sub(f.base),
        );
        let local_d = grasm_syntax::Diagnostic {
            severity: d.severity,
            span: local,
            message: d.message.clone(),
            help: d.help.clone(),
        };
        let index = grasm_syntax::LineIndex::new(&f.src);
        eprintln!("{}", local_d.render(&f.path, &f.src, &index));
    }
    eprintln!("{} error(s) found", diags.len());
}

// ---------------------------------------------------------------------------
// Bundles (`grasm build`)
// ---------------------------------------------------------------------------

struct Bundle {
    /// module name → source
    files: Vec<(String, String)>,
}

fn read_own_bundle() -> Option<Bundle> {
    let exe = std::env::current_exe().ok()?;
    let data = std::fs::read(exe).ok()?;
    parse_bundle(&data)
}

fn parse_bundle(data: &[u8]) -> Option<Bundle> {
    if data.len() < 16 || &data[data.len() - 8..] != BUNDLE_MAGIC {
        return None;
    }
    let len_bytes: [u8; 8] = data[data.len() - 16..data.len() - 8].try_into().ok()?;
    let payload_len = u64::from_le_bytes(len_bytes) as usize;
    if payload_len + 16 > data.len() {
        return None;
    }
    let payload = &data[data.len() - 16 - payload_len..data.len() - 16];
    let mut pos = 0usize;
    let read_u32 = |p: &mut usize| -> Option<u32> {
        let b: [u8; 4] = payload.get(*p..*p + 4)?.try_into().ok()?;
        *p += 4;
        Some(u32::from_le_bytes(b))
    };
    let n = read_u32(&mut pos)? as usize;
    let mut files = Vec::with_capacity(n);
    for _ in 0..n {
        let name_len = read_u32(&mut pos)? as usize;
        let name = String::from_utf8(payload.get(pos..pos + name_len)?.to_vec()).ok()?;
        pos += name_len;
        let src_len = read_u32(&mut pos)? as usize;
        let src = String::from_utf8(payload.get(pos..pos + src_len)?.to_vec()).ok()?;
        pos += src_len;
        files.push((name, src));
    }
    Some(Bundle { files })
}

fn write_bundle(program: &Program, out_path: &str) -> Result<u64, String> {
    let exe = std::env::current_exe().map_err(|e| e.to_string())?;
    let mut data = std::fs::read(&exe).map_err(|e| e.to_string())?;
    // If this runtime itself is a bundle (unlikely), strip its payload.
    while parse_bundle(&data).is_some() {
        let len_bytes: [u8; 8] = data[data.len() - 16..data.len() - 8].try_into().unwrap();
        let payload_len = u64::from_le_bytes(len_bytes) as usize;
        data.truncate(data.len() - 16 - payload_len);
    }
    let mut payload = Vec::new();
    payload.extend((program.files.len() as u32).to_le_bytes());
    for f in &program.files {
        payload.extend((f.name.len() as u32).to_le_bytes());
        payload.extend(f.name.as_bytes());
        payload.extend((f.src.len() as u32).to_le_bytes());
        payload.extend(f.src.as_bytes());
    }
    let payload_len = payload.len() as u64;
    data.extend(payload);
    data.extend(payload_len.to_le_bytes());
    data.extend(BUNDLE_MAGIC);
    let size = data.len() as u64;
    std::fs::write(out_path, data).map_err(|e| e.to_string())?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mut perms = std::fs::metadata(out_path)
            .map_err(|e| e.to_string())?
            .permissions();
        perms.set_mode(0o755);
        std::fs::set_permissions(out_path, perms).map_err(|e| e.to_string())?;
    }
    Ok(size)
}

fn run_bundle(bundle: &Bundle, program_args: &[String]) -> ExitCode {
    let files = bundle.files.clone();
    let provider = move |name: &str| -> Option<String> {
        files
            .iter()
            .find(|(n, _)| n == name)
            .map(|(_, s)| s.clone())
    };
    let program = loader::load_with("main", "<bundled>", &provider);
    let mut diags = program.diagnostics.clone();
    if diags.is_empty() {
        diags.extend(grasm_sema::check::check_program(&program).diagnostics);
    }
    if !diags.is_empty() {
        render_diags(&program, &diags);
        return ExitCode::FAILURE;
    }
    let mut stdout = std::io::stdout();
    let code = grasm_interp::run_program(&program, program_args, &mut stdout);
    let _ = stdout.flush();
    ExitCode::from(code.clamp(0, 255) as u8)
}

// ---------------------------------------------------------------------------
// Native builds (`grasm build --native`)
// ---------------------------------------------------------------------------

fn build_native(program: &Program, out_path: &str) -> Result<u64, String> {
    let scratch = std::env::temp_dir().join(format!("grasm_native_{}", std::process::id()));
    std::fs::create_dir_all(&scratch).map_err(|e| e.to_string())?;
    let obj = scratch.join("out.o");
    grasm_native::compile_object(program, &obj)?;

    // libgrasm_rt.a sits next to the grasm binary (cargo target dir).
    let exe = std::env::current_exe().map_err(|e| e.to_string())?;
    let exe_dir = exe.parent().ok_or("no exe dir")?;
    let rt = exe_dir.join("libgrasm_rt.a");
    if !rt.exists() {
        return Err(format!(
            "runtime library not found at `{}` — build it with `cargo build -p grasm_rt`",
            rt.display()
        ));
    }
    let status = std::process::Command::new("cc")
        .arg(&obj)
        .arg(&rt)
        .arg("-o")
        .arg(out_path)
        .status()
        .map_err(|e| format!("failed to run `cc`: {e}"))?;
    if !status.success() {
        return Err("linking failed".into());
    }
    let size = std::fs::metadata(out_path).map_err(|e| e.to_string())?.len();
    Ok(size)
}
