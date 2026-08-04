//! The `grasm` CLI: run / test / check / fmt / build / tokens.
//!
//! `grasm build` produces self-contained executables by appending a source
//! bundle to a copy of this binary; at startup we check our own tail for the
//! bundle magic and, if present, run it directly.

use std::io::Write as _;
use std::path::PathBuf;
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
            let (code, trace) =
                grasm_interp::run_program_traced(&program, &prog_args, &mut stdout);
            let _ = stdout.flush();
            print_trace(&program, &trace);
            ExitCode::from(code.clamp(0, 255) as u8)
        }
        Some("test") => {
            let Some(path) = args.get(1).filter(|a| !a.starts_with("--")) else {
                eprintln!("usage: grasm test <file.gr> [--coverage] [--seed N] [--cases N] [--filter TEXT]");
                return ExitCode::FAILURE;
            };
            let path = path.clone();
            let opts = match parse_test_opts(&args[2..]) {
                Ok(o) => o,
                Err(e) => {
                    eprintln!("error: {e}");
                    return ExitCode::FAILURE;
                }
            };
            let program = match load_and_check(&path) {
                Ok(p) => p,
                Err(code) => return code,
            };
            let mut stdout = std::io::stdout();
            let summary = grasm_interp::test_program_with(&program, &opts, &mut stdout);
            println!("{} passed, {} failed", summary.passed, summary.failed);
            if let Some(cov) = &summary.coverage {
                print_coverage(cov);
            }
            if summary.failed > 0 {
                ExitCode::FAILURE
            } else {
                ExitCode::SUCCESS
            }
        }
        Some("bench") => {
            let Some(path) = args.get(1).filter(|a| !a.starts_with("--")) else {
                eprintln!("usage: grasm bench <file.gr> [--filter TEXT]");
                return ExitCode::FAILURE;
            };
            let path = path.clone();
            let filter = flag_value(&args[2..], "--filter").unwrap_or_default();
            let program = match load_and_check(&path) {
                Ok(p) => p,
                Err(code) => return code,
            };
            let mut stdout = std::io::stdout();
            let results = grasm_interp::bench_program(&program, &filter, &mut stdout);
            if results.is_empty() {
                println!("no benchmarks found (declare one with `bench \"name\" {{ ... }}`)");
                return ExitCode::SUCCESS;
            }
            let width = results.iter().map(|r| r.name.len()).max().unwrap_or(0);
            let mut failed = false;
            for r in &results {
                match &r.error {
                    Some(e) => {
                        println!("{:<width$}  FAILED: {e}", r.name, width = width);
                        failed = true;
                    }
                    None => println!(
                        "{:<width$}  {:>12}  ({} iterations)",
                        r.name,
                        format_duration(r.ns_per_op),
                        r.iterations,
                        width = width
                    ),
                }
            }
            if failed {
                ExitCode::FAILURE
            } else {
                ExitCode::SUCCESS
            }
        }
        Some("doc") => {
            let Some(path) = args.get(1).filter(|a| !a.starts_with("--")) else {
                eprintln!("usage: grasm doc <file.gr> [-o out.md]");
                return ExitCode::FAILURE;
            };
            let path = path.clone();
            let program = match load_and_check(&path) {
                Ok(p) => p,
                Err(code) => return code,
            };
            let modules = grasm_sema::docs::collect(&program);
            let title = std::path::Path::new(&path)
                .file_stem()
                .map(|s| s.to_string_lossy().to_string())
                .unwrap_or_else(|| "API".to_string());
            let text = grasm_sema::docs::render_markdown(&title, &modules);
            match flag_value(&args[2..], "-o") {
                Some(out) => match std::fs::write(&out, &text) {
                    Ok(()) => {
                        let items: usize = modules.iter().map(|m| m.items.len()).sum();
                        println!("wrote {out} ({items} item(s))");
                        ExitCode::SUCCESS
                    }
                    Err(e) => {
                        eprintln!("error: cannot write {out}: {e}");
                        ExitCode::FAILURE
                    }
                },
                None => {
                    print!("{text}");
                    ExitCode::SUCCESS
                }
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
                let target = args
                    .iter()
                    .position(|a| a == "--target")
                    .and_then(|i| args.get(i + 1))
                    .cloned();
                if let Some(t) = &target {
                    // Cross-target: emit the object only — linking needs a
                    // cross-linker and a runtime built for that target.
                    let obj = format!("{out_path}.o");
                    return match grasm_native::compile_object_for(
                        &program,
                        std::path::Path::new(&obj),
                        Some(t),
                    ) {
                        Ok(()) => {
                            println!("emitted `{obj}` for {t} (link it with a {t} toolchain + libgrasm_rt.a)");
                            ExitCode::SUCCESS
                        }
                        Err(e) => {
                            eprintln!("error: {e}");
                            ExitCode::FAILURE
                        }
                    };
                }
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
        Some("deps") => {
            let dir = args
                .get(1)
                .map(PathBuf::from)
                .unwrap_or_else(|| PathBuf::from("."));
            match cmd_deps(&dir) {
                Ok(()) => ExitCode::SUCCESS,
                Err(e) => {
                    eprintln!("error: {e}");
                    ExitCode::FAILURE
                }
            }
        }
        Some("lsp") => {
            let stdin = std::io::stdin();
            let mut input = stdin.lock();
            let mut output = std::io::stdout();
            match grasm_lsp::Server::new().run(&mut input, &mut output) {
                Ok(()) => ExitCode::SUCCESS,
                Err(e) => {
                    eprintln!("lsp error: {e}");
                    ExitCode::FAILURE
                }
            }
        }
        Some("dap") => {
            let stdin = std::io::stdin();
            let input = std::io::BufReader::new(stdin);
            let output = std::io::stdout();
            let result = grasm_dap::serve(input, output, |path| {
                let program = grasm_sema::loader::load_from_disk(std::path::Path::new(path))
                    .map_err(|e| format!("cannot load `{path}`: {e}"))?;
                let mut diags = program.diagnostics.clone();
                diags.extend(grasm_sema::check::check_program(&program).diagnostics);
                if let Some(first) = diags.first() {
                    return Err(format!("`{path}` has errors: {}", first.message));
                }
                Ok(program)
            });
            match result {
                Ok(_) => ExitCode::SUCCESS,
                Err(e) => {
                    eprintln!("dap error: {e}");
                    ExitCode::FAILURE
                }
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
            eprintln!("  test <file.gr>               run test blocks (unit and property)");
            eprintln!("      --coverage               report which lines the tests reached");
            eprintln!("      --seed N                 seed for property generation");
            eprintln!("      --cases N                property cases per test (default 100)");
            eprintln!("      --filter TEXT            only tests whose name contains TEXT");
            eprintln!("  bench <file.gr>              run bench blocks, report ns/op");
            eprintln!("      --filter TEXT            only benchmarks whose name contains TEXT");
            eprintln!("  check <file.gr>              parse + typecheck");
            eprintln!("  build <file.gr> [-o out]     self-contained executable");
            eprintln!("      --native                 compile to machine code (Cranelift)");
            eprintln!("      --target <triple>        emit a cross-target object file");
            eprintln!("  doc <file.gr> [-o out.md]    API documentation from `///` comments");
            eprintln!("  fmt <file.gr> [--check]      canonical formatting");
            eprintln!("  lsp                          language server (stdio)");
            eprintln!("  dap                          debug adapter (stdio) — breakpoints, stepping");
            eprintln!("  deps [dir]                   resolve dependencies, check capabilities, write grasm.lock");
            eprintln!("  tokens <file.gr>             dump lexer output (debug)");
            eprintln!("  version                      print version");
            ExitCode::FAILURE
        }
    }
}

/// `--flag value` lookup.
fn flag_value(args: &[String], flag: &str) -> Option<String> {
    let i = args.iter().position(|a| a == flag)?;
    args.get(i + 1).cloned()
}

fn parse_test_opts(args: &[String]) -> Result<grasm_interp::testing::TestOptions, String> {
    let mut opts = grasm_interp::testing::TestOptions {
        coverage: args.iter().any(|a| a == "--coverage"),
        filter: flag_value(args, "--filter").unwrap_or_default(),
        ..Default::default()
    };
    if let Some(v) = flag_value(args, "--seed") {
        opts.seed = v
            .parse()
            .map_err(|_| format!("--seed expects a number, got `{v}`"))?;
    }
    if let Some(v) = flag_value(args, "--cases") {
        let n: u32 = v
            .parse()
            .map_err(|_| format!("--cases expects a number, got `{v}`"))?;
        if n == 0 {
            return Err("--cases must be at least 1".to_string());
        }
        opts.cases = n;
    }
    Ok(opts)
}

fn print_coverage(cov: &grasm_interp::testing::Coverage) {
    println!();
    println!("coverage: {:.1}% of statement lines", cov.percent());
    for (file, lines) in cov.misses() {
        println!(
            "  {file}: {} uncovered — {}",
            lines.len(),
            grasm_interp::testing::format_ranges(&lines)
        );
    }
}

/// Human-readable ns/op: picks the unit that keeps 3-4 significant digits.
fn format_duration(ns: f64) -> String {
    if ns < 1_000.0 {
        format!("{ns:.1} ns/op")
    } else if ns < 1_000_000.0 {
        format!("{:.2} µs/op", ns / 1_000.0)
    } else {
        format!("{:.2} ms/op", ns / 1_000_000.0)
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
    let (code, trace) = grasm_interp::run_program_traced(&program, program_args, &mut stdout);
    let _ = stdout.flush();
    print_trace(&program, &trace);
    ExitCode::from(code.clamp(0, 255) as u8)
}

/// Prints a panic's call stack, innermost frame first.
///
/// Goes to stderr so a panicking program's stdout stays exactly what the
/// program produced — differential testing compares that byte for byte.
fn print_trace(program: &Program, trace: &[grasm_interp::Frame]) {
    if trace.is_empty() {
        return;
    }
    eprintln!("stack trace (innermost first):");
    for frame in trace {
        let i = program.file_of(frame.at);
        let file = &program.files[i];
        let index = grasm_syntax::span::LineIndex::new(&file.src);
        let lc = index.line_col(frame.at.start.saturating_sub(file.base));
        eprintln!(
            "  {} at {}:{}:{}",
            frame.function, file.path, lc.line, lc.col
        );
    }
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

// ---------------------------------------------------------------------------
// Package management (`grasm deps`)
// ---------------------------------------------------------------------------

/// Resolves the dependency graph, enforces each package's capability ceiling,
/// and writes the lockfile.
fn cmd_deps(dir: &std::path::Path) -> Result<(), String> {
    let resolution = grasm_pkg::resolve(dir)?;
    let root = resolution.root();

    println!(
        "{} {} — {} package(s) in the graph",
        root.name,
        root.version,
        resolution.packages.len()
    );

    // Capability enforcement: a package that performs more than its manifest
    // permits fails the build for everyone who depends on it.
    let mut violations = Vec::new();
    for pkg in &resolution.packages {
        let performed = match grasm_pkg::effects_of_package(pkg) {
            Ok(e) => e,
            Err(e) => {
                eprintln!("warning: cannot analyze `{}`: {e}", pkg.name);
                continue;
            }
        };
        let shown: Vec<&str> = performed.iter().map(|s| s.as_str()).collect();
        let declared = match &pkg.capabilities {
            Some(c) => c.iter().cloned().collect::<Vec<_>>().join(", "),
            None => "(unrestricted)".to_string(),
        };
        println!(
            "  {:<16} performs [{}]  permits [{}]",
            pkg.name,
            shown.join(", "),
            declared
        );
        if let Some(v) = grasm_pkg::check_capabilities(pkg, &performed) {
            violations.push(v);
        }
    }

    if !violations.is_empty() {
        for v in &violations {
            eprintln!("error: {v}");
            eprintln!(
                "  help: either add the capability to that package's manifest, \
or stop it from performing the effect"
            );
        }
        return Err(format!(
            "{} package(s) exceed their declared capabilities",
            violations.len()
        ));
    }

    let entries = grasm_pkg::build_lock(&resolution)?;
    let lock_path = dir.join(grasm_pkg::LOCKFILE_NAME);
    if let Ok(previous) = std::fs::read_to_string(&lock_path) {
        for change in grasm_pkg::diff_lock(&previous, &entries) {
            println!("  note: {change}");
        }
    }
    std::fs::write(&lock_path, grasm_pkg::render_lock(&entries))
        .map_err(|e| format!("cannot write {}: {e}", lock_path.display()))?;
    println!("wrote {}", lock_path.display());
    Ok(())
}
