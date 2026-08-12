//! The `grasm` CLI: run / test / check / fmt / build / tokens.
//!
//! `grasm build` produces self-contained executables by appending a source
//! bundle to a copy of this binary; at startup we check our own tail for the
//! bundle magic and, if present, run it directly.

use std::collections::HashMap;
use std::io::Write as _;
use std::path::PathBuf;
use std::process::ExitCode;

use grasm_sema::loader::{self, Program};

const BUNDLE_MAGIC: &[u8; 8] = b"GRASMBDL";



/// The stack the whole tool runs on.
///
/// Every pass after the parser — checking, formatting, the interpreter's own
/// compile step, the native backend — walks the syntax tree by recursion, so
/// the depth a program is allowed to reach and the stack available to walk it
/// are one budget. The platform default is 8 MiB, which a flat chain of a few
/// thousand terms exhausted: `let x = 1+1+…+1` aborted the process with
/// `fatal runtime error: stack overflow` and no diagnostic, against
/// [G-NEST-LIMIT]. The parser's cap is what keeps input inside this.
const COMPILER_STACK: usize = 256 * 1024 * 1024;

/// The ceiling breach, said with the place the effect happens.
///
/// The message named the effect and stopped there, so a `capabilities = []`
/// package that printed told you only that it printed — finding the `print`
/// was then a manual search. Where an effect has a syntactic site, it is
/// quoted; where it only arrives through a call the package makes, there is
/// no honest line to point at and the name stands alone.
fn ceiling_help(pkg: &grasm_pkg::Manifest, v: &grasm_pkg::CapabilityViolation) -> String {
    let mut where_at: Vec<String> = Vec::new();
    if let Ok(sites) = grasm_pkg::effect_sites_of_package(pkg) {
        for e in &v.exceeded {
            if let Some(at) = sites.get(e) {
                where_at.push(format!("`{e}` at {at}"));
            }
        }
    }
    let mut help = String::from(
        "either add the capability to that package's manifest, or stop it from performing the effect",
    );
    if !where_at.is_empty() {
        help.push_str(&format!("\n  performed: {}", where_at.join(", ")));
    }
    help
}

fn main() -> ExitCode {
    match std::thread::Builder::new()
        .stack_size(COMPILER_STACK)
        .spawn(run)
    {
        Ok(h) => h.join().unwrap_or(ExitCode::FAILURE),
        // If the thread cannot be spawned there is nothing left but to try on
        // whatever stack we already have.
        Err(_) => run(),
    }
}

fn run() -> ExitCode {
    // Bundled-executable mode: our own binary carries a program.
    if let Some(bundle) = read_own_bundle() {
        let args: Vec<String> = std::env::args().skip(1).collect();
        return run_bundle(&bundle, &args);
    }

    let args: Vec<String> = std::env::args().skip(1).collect();
    // A flag this command does not know is a mistake, not a comment. A typo
    // in `--native` produced a 10 MB interpreter bundle and reported success,
    // and `--filter` misspelt ran the whole suite.
    if let Some(cmd) = args.first().map(String::as_str) {
        // The flags each command takes, and which of them take a value.
        let (flags, valued): (&[&str], &[&str]) = match cmd {
            "run" => (&[], &[]),
            "test" => (
                &["--coverage"],
                &["--seed", "--cases", "--filter"],
            ),
            "check" => (&[], &[]),
            "build" => (&["--native"], &["-o", "--target"]),
            "doc" => (&[], &["-o"]),
            "api" => (&[], &["--against"]),
            "fmt" => (&["--check"], &[]),
            "deps" => (&["--locked"], &[]),
            "publish" => (&[], &["--registry"]),
            _ => (&[], &[]),
        };
        // Everything after `--` belongs to the program, not to the toolchain.
        let mut i = 1;
        let mut bad: Option<String> = None;
        while i < args.len() {
            let a = &args[i];
            if a == "--" {
                break;
            }
            if a.starts_with('-') && a != "-" {
                if valued.contains(&a.as_str()) {
                    // A value-taking flag whose value is missing is a lost
                    // operand, not a flag on its own: `--against "$BASE"`
                    // with `BASE` unset dropped the comparison and exited 0.
                    if args.get(i + 1).is_none_or(|v| v.starts_with('-')) {
                        eprintln!("error: `{a}` needs a value");
                        return ExitCode::FAILURE;
                    }
                    i += 2;
                    continue;
                }
                if !flags.contains(&a.as_str()) {
                    bad = Some(a.clone());
                    break;
                }
            }
            i += 1;
        }
        if let Some(flag) = bad {
            eprintln!("error: `grasm {cmd}` has no option `{flag}`");
            let known: Vec<String> = flags
                .iter()
                .chain(valued.iter())
                .map(|f| format!("`{f}`"))
                .collect();
            if known.is_empty() {
                eprintln!("  help: `grasm {cmd}` takes no options");
            } else {
                eprintln!("  help: it takes {}", known.join(", "));
            }
            return ExitCode::FAILURE;
        }
    }
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
            let (code, trace, _fault) =
                grasm_interp::run_program_traced(&program, &prog_args, &mut stdout);
            let _ = stdout.flush();
            print_trace(&program, &trace);
            ExitCode::from((code & 0xff) as u8)
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
            // The package's name, when the file is in one. The entry file's
            // stem is `main` in every package laid out the ordinary way, so
            // every generated document was titled `main` regardless of what
            // the package was called.
            let title = enclosing_package(std::path::Path::new(&path))
                .map(|m| m.name)
                .or_else(|| {
                    std::path::Path::new(&path)
                        .file_stem()
                        .map(|s| s.to_string_lossy().to_string())
                })
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
            // The first argument that is not a flag, so `build --native f.gr`
            // works as well as `build f.gr --native`. Taking argument one
            // literally made the first form report `cannot read '--native'`,
            // which reads like a missing file.
            let Some(path) = args[1..]
                .iter()
                .find(|a| !a.starts_with('-'))
                .filter(|a| args.iter().position(|x| x == *a) == Some(1) || a.ends_with(".gr"))
            else {
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
            // An output path that is not a regular file cannot become an
            // executable. `-o /dev/null` used to report `built ... (0 bytes)`
            // and exit 0 — in a build script, a green light for something that
            // will never run. Checked here so the message names the cause
            // rather than surfacing whatever errno the linker happened to hit.
            if let Ok(meta) = std::fs::metadata(&out_path) {
                if !meta.is_file() {
                    eprintln!(
                        "error: `{out_path}` is not a regular file, so it cannot be an executable",
                    );
                    return ExitCode::FAILURE;
                }
            }
            // `--target` only means anything to the compiled backend. Without
            // `--native` it was accepted and ignored, so asking for a Linux
            // build on a Mac produced a Mach-O bundle that ran locally and
            // reported success — a silent wrong answer from the toolchain.
            if args.iter().any(|a| a == "--target") && !args.iter().any(|a| a == "--native") {
                eprintln!("error: `--target` needs `--native` — a bundled build carries the interpreter, which is this machine's");
                eprintln!("  help: add `--native`, or drop `--target` to build for this machine");
                return ExitCode::FAILURE;
            }
            let program = match load_and_check(path) {
                Ok(p) => p,
                Err(code) => return code,
            };
            // A program with no entry point is not a backend limitation
            // ([E-ENTRY]). Leaving it to the native backend produced "does
            // not yet support programs without a `main` function — use
            // `grasm run`", which sends a reader to try something that fails
            // there too, with a clearer message.
            if !program.files.iter().any(|f| {
                f.name == "main"
                    && f.module.decls.iter().any(|d| {
                        matches!(d, grasm_syntax::ast::Decl::Fn(fd) if fd.name.name == "main")
                    })
            }) {
                eprintln!("error: no `main` function");
                return ExitCode::FAILURE;
            }
            if args.iter().any(|a| a == "--native") {
                let target = args
                    .iter()
                    .position(|a| a == "--target")
                    .and_then(|i| args.get(i + 1))
                    .cloned();
                if let Some(t) = &target {
                    // Cross-target: emit the object only — linking needs a
                    // cross-linker and a runtime built for that target.
                    // `-o out.o` means that file, not `out.o.o`.
                    let obj = if out_path.ends_with(".o") {
                        out_path.clone()
                    } else {
                        format!("{out_path}.o")
                    };
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
                let whole = args.iter().any(|a| a == "--no-incremental");
                return match if whole {
                    build_native_whole(&program, &out_path)
                } else {
                    build_native(&program, &out_path)
                } {
                    // A linker that writes nothing has not built anything.
                    // `-o /dev/null` reported "built ... (0 bytes)" and exited
                    // 0, which in a build script is a green light for a file
                    // that will never run.
                    Ok(0) => {
                        eprintln!(
                            "error: linking produced an empty file at `{out_path}` — it is not an executable",
                        );
                        ExitCode::FAILURE
                    }
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
            // A program with no `main` is an error ([E-ENTRY]), and the
            // other two build modes say so. The bundle used to ship a 9.7 MB
            // executable, report success, and fail on startup.
            if !program.files.first().is_some_and(|f| {
                f.module.decls.iter().any(|d| {
                    matches!(d, grasm_syntax::ast::Decl::Fn(fd) if fd.name.name == "main")
                })
            }) {
                eprintln!("error: no `main` function");
                eprintln!("  help: a bundled build needs an entry point, the same as a native one");
                return ExitCode::FAILURE;
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
        // The public surface of a package, and what changed in it.
        Some("api") => {
            let Some(path) = args.get(1).filter(|a| !a.starts_with("--")) else {
                eprintln!("usage: grasm api <file.gr> [--against <saved.api>]");
                return ExitCode::FAILURE;
            };
            let program = match load_and_check(path) {
                Ok(p) => p,
                Err(code) => return code,
            };
            let surface = api_surface(&program);
            let against = args
                .iter()
                .position(|a| a == "--against")
                .and_then(|i| args.get(i + 1));
            match against {
                None => {
                    print!("{surface}");
                    ExitCode::SUCCESS
                }
                Some(saved) => {
                    let old = match std::fs::read_to_string(saved) {
                        Ok(t) => t,
                        Err(e) => {
                            eprintln!("error: cannot read `{saved}`: {e}");
                            return ExitCode::FAILURE;
                        }
                    };
                    // A baseline with no entries cannot be a baseline: every
                    // export reads as "added" and a removal becomes a MINOR
                    // release. A truncated file, a wrong path that happens to
                    // exist, or binary junk all landed there and exited 0 —
                    // the one exit code a release gate reads.
                    let entries = old
                        .lines()
                        .filter(|l| l.split('\t').count() == 3)
                        .count();
                    if entries == 0 && !surface.trim().is_empty() {
                        eprintln!(
                            "error: `{saved}` has no API entries — a baseline that says nothing would report every export as added"
                        );
                        eprintln!(
                            "  help: regenerate it with `grasm api <the previous version> > {saved}`"
                        );
                        return ExitCode::FAILURE;
                    }
                    report_api_change(&old, &surface)
                }
            }
        }
        Some("fmt") => {
            // The path is whichever argument is not a flag, so
            // `fmt --check f.gr` and `fmt f.gr --check` both work. Only the
            // second used to: the first read `--check` as the file name and
            // reported `cannot read '--check'`, which says nothing about the
            // real mistake.
            let Some(path) = args.iter().skip(1).find(|a| !a.starts_with("--")) else {
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
            // Formatting a file that does not parse cannot mean anything:
            // the formatter works from the token stream, so it rewrote what it
            // could and reported success on source the compiler rejects.
            let parsed = grasm_syntax::parse(&src);
            if !parsed.diagnostics.is_empty() {
                eprintln!("error: `{path}` does not parse, so there is nothing to format");
                for d in parsed.diagnostics.iter().take(3) {
                    eprintln!("  {}", d.message);
                }
                return ExitCode::FAILURE;
            }
            let formatted = grasm_syntax::fmt::format_src(&src);
            if args.iter().any(|a| a == "--check") {
                if formatted == src {
                    println!("ok: `{path}` is formatted");
                    ExitCode::SUCCESS
                } else {
                    eprintln!("`{path}` is not canonically indented (run `grasm fmt {path}`)");
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
            let locked = args.iter().any(|a| a == "--locked");
            let dir = args
                .get(1)
                .filter(|a| !a.starts_with("--"))
                .map(PathBuf::from)
                .unwrap_or_else(|| PathBuf::from("."));
            match cmd_deps(&dir, locked) {
                Ok(()) => ExitCode::SUCCESS,
                Err(e) => {
                    eprintln!("error: {e}");
                    ExitCode::FAILURE
                }
            }
        }
        Some("publish") => {
            let dir = args
                .get(1)
                .filter(|a| !a.starts_with("--"))
                .map(PathBuf::from)
                .unwrap_or_else(|| PathBuf::from("."));
            match cmd_publish(&dir, flag_value(&args[1..], "--registry")) {
                Ok(()) => ExitCode::SUCCESS,
                Err(e) => {
                    eprintln!("error: {e}");
                    ExitCode::FAILURE
                }
            }
        }
        Some("add") => {
            let (Some(name), Some(version)) = (args.get(1), args.get(2)) else {
                eprintln!("usage: grasm add <name> <version> [dir]");
                return ExitCode::FAILURE;
            };
            let dir = args
                .get(3)
                .filter(|a| !a.starts_with("--"))
                .map(PathBuf::from)
                .unwrap_or_else(|| PathBuf::from("."));
            match cmd_add(&dir, name, version) {
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
        // Asking for help is not an error; an unknown command is. Printing
        // usage and exiting 1 for `--help` breaks any script that checks.
        other => {
            let asked = matches!(other, Some("help") | Some("--help") | Some("-h") | None);
            if let Some(cmd) = other.filter(|_| !asked) {
                eprintln!("error: unknown command `{cmd}`");
                eprintln!();
            }
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
            eprintln!("  bench <file.gr>              run bench blocks, report time per op");
            eprintln!("      --filter TEXT            only benchmarks whose name contains TEXT");
            eprintln!("  check <file.gr>              parse + typecheck");
            eprintln!("  build <file.gr> [-o out]     self-contained executable");
            eprintln!("      --native                 compile to machine code (Cranelift)");
            eprintln!("      --target <triple>        emit a cross-target object file");
            eprintln!("      --no-incremental         one object, no cache (best for a single build)");
            eprintln!("  doc <file.gr> [-o out.md]    API documentation from `///` comments");
            eprintln!("  api <file.gr> [--against F]  public surface, and what changed in it");
            eprintln!("  fmt <file.gr> [--check]      re-indent (4 spaces per level); line contents are left alone");
            eprintln!("  lsp                          language server (stdio)");
            eprintln!("  dap                          debug adapter (stdio) — breakpoints, stepping");
            eprintln!("  deps [dir]                   resolve dependencies, check capabilities, write grasm.lock");
            eprintln!("      --locked                 fail if anything changed since grasm.lock (CI)");
            eprintln!("  publish [dir]                copy a package into a registry (versions are immutable)");
            eprintln!("      --registry <dir>         registry root (else GRASM_REGISTRY, else .grasm/registry)");
            eprintln!("  add <name> <version> [dir]   add a registry dependency to grasm.toml");
            eprintln!("  tokens <file.gr>             dump lexer output (debug)");
            eprintln!("  version                      print version");
            if asked {
                ExitCode::SUCCESS
            } else {
                ExitCode::FAILURE
            }
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

/// Package directories for a file's registry dependencies, if it has any.
///
/// Path dependencies the loader finds on its own. A registry dependency needs
/// The manifest of the package a file belongs to, if it is in one.
///
/// Walks up from the file, so any file in a package finds it — the same rule
/// the loader uses for resolving a package's dependencies.
fn enclosing_package(path: &std::path::Path) -> Option<grasm_pkg::Manifest> {
    let mut here = path.parent().map(std::path::Path::to_path_buf);
    loop {
        match here {
            Some(ref d) if d.join(grasm_pkg::MANIFEST_NAME).is_file() => {
                return grasm_pkg::Manifest::load(d).ok();
            }
            Some(ref d) => here = d.parent().map(std::path::Path::to_path_buf),
            None => return None,
        }
    }
}

/// the whole graph resolved before its version is known, so that happens here
/// and the answer is handed down. Packages with no registry dependency pay
/// nothing: there is nothing to resolve.
/// Whether two paths name the same directory, following symlinks when they
/// both exist.
fn same_dir(a: &std::path::Path, b: &std::path::Path) -> bool {
    match (std::fs::canonicalize(a), std::fs::canonicalize(b)) {
        (Ok(x), Ok(y)) => x == y,
        _ => a == b,
    }
}

/// The package graph, as name → directory — *excluding* the package being
/// built, whose own modules are named relative to it rather than under it.
fn registry_packages(path: &std::path::Path) -> Result<HashMap<String, PathBuf>, String> {
    let mut here = path.parent().map(std::path::Path::to_path_buf);
    let pkg_dir = loop {
        match here {
            Some(ref d) if d.join(grasm_pkg::MANIFEST_NAME).is_file() => break d.clone(),
            Some(ref d) => here = d.parent().map(std::path::Path::to_path_buf),
            None => return Ok(HashMap::new()),
        }
    };
    let manifest = grasm_pkg::Manifest::load(&pkg_dir)?;
    if manifest.dependencies.iter().all(|d| d.path.is_some()) {
        // A *nested* package may still want the registry: this package's own
        // dependencies being paths says nothing about theirs. Attempted, not
        // required — a graph that is entirely paths must not start needing a
        // registry to build.
        return Ok(grasm_pkg::resolve(&pkg_dir)
            .map(|r| {
                r.packages
                    .iter()
                    .filter(|p| !same_dir(&p.dir, &pkg_dir))
                    .map(|p| (p.name.clone(), p.dir.clone()))
                    .collect()
            })
            .unwrap_or_default());
    }
    let resolution = grasm_pkg::resolve(&pkg_dir)?;
    Ok(resolution
        .packages
        .iter()
        .filter(|p| !same_dir(&p.dir, &pkg_dir))
        .map(|p| (p.name.clone(), p.dir.clone()))
        .collect())
}

/// Load (following imports), typecheck, render diagnostics per file.
fn load_and_check(path: &str) -> Result<Program, ExitCode> {
    let entry = std::path::Path::new(path);
    let resolved = match registry_packages(entry) {
        Ok(r) => r,
        Err(e) => {
            eprintln!("error: {e}");
            return Err(ExitCode::FAILURE);
        }
    };
    let program = match loader::load_from_disk_with(entry, &resolved) {
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
    if !diags.is_empty() {
        render_diags(&program, &diags);
        return Err(ExitCode::FAILURE);
    }
    // A dependency's capability ceiling is a *build* guarantee, so it is
    // checked here — on the path `check`, `run` and `build` all take — and not
    // only by `grasm deps`. Documented as "a compile error, not a runtime
    // surprise, and not something a reviewer has to happen to notice", which
    // is only true if the compiler is the one noticing.
    if let Err(violations) = check_dependency_ceilings(entry, Some(&program)) {
        for (message, help) in violations {
            eprintln!("error: {message}");
            if let Some(h) = help {
                eprintln!("  help: {h}");
            }
        }
        return Err(ExitCode::FAILURE);
    }
    Ok(program)
}

/// Every dependency's effects against its declared ceiling.
///
/// Path dependencies included: a package being local is a reason the ceiling
/// matters more, not less, since it is the one changing under you.
fn check_dependency_ceilings(
    entry: &std::path::Path,
    program: Option<&Program>,
) -> Result<(), Vec<(String, Option<String>)>> {
    let mut here = entry.parent().map(std::path::Path::to_path_buf);
    let pkg_dir = loop {
        match here {
            Some(ref d) if d.join(grasm_pkg::MANIFEST_NAME).is_file() => break d.clone(),
            Some(ref d) => here = d.parent().map(std::path::Path::to_path_buf),
            None => return Ok(()),
        }
    };
    // A resolution failure is a build failure. Swallowing it meant a version
    // requirement nothing could satisfy — `version = "=9.9.9"` against a 0.1.0
    // dependency — was caught by `grasm deps` and by nothing else, while the
    // capability ceiling right beside it was enforced everywhere.
    let resolution = match grasm_pkg::resolve(&pkg_dir) {
        Ok(r) => r,
        Err(e) => return Err(vec![(e, None)]),
    };
    let mut violations: Vec<(String, Option<String>)> = Vec::new();
    for pkg in &resolution.packages {
        if pkg.capabilities.is_none() {
            continue;
        }
        let Ok(performed) = grasm_pkg::effects_of_package(pkg) else {
            violations.push((
                format!(
                    "cannot check the capability ceiling of `{}` — refusing to \
build against a dependency whose reach is unknown",
                    pkg.name
                ),
                None,
            ));
            continue;
        };
        // What the package performs *in this program* counts too: a library
        // that installs a handler it did not construct performs nothing on
        // its own, and printed through one the consumer handed it.
        let mut performed = performed;
        if let Some(p) = program {
            let by_pkg = grasm_sema::effects::effects_by_package(p);
            if let Some(extra) = by_pkg.get(&pkg.name) {
                performed.extend(extra.iter().cloned());
            }
        }
        if let Some(v) = grasm_pkg::check_capabilities(pkg, &performed) {
            let help = ceiling_help(pkg, &v);
            violations.push((v.to_string(), Some(help)));
        }
    }
    if violations.is_empty() {
        Ok(())
    } else {
        Err(violations)
    }
}


/// A package's public surface, one item per line, sorted.
///
/// Sorted because a diff should report what *changed*, not where a
/// declaration moved to. One line per item because that makes the comparison
/// a set operation on names and signatures rather than a parse.
/// Replaces every type alias with what it stands for.
///
/// `[T-TYPE-ALIAS]`: an alias is not a new type, and the two are
/// interchangeable everywhere. Comparing the *written* signatures made
/// `-> str` becoming `-> Text` (with `type Text = str`) a changed export and
/// so a MAJOR release — exit 1 on a release gate for a change no consumer can
/// observe. Expanded first, both spellings render the same.
fn expand_aliases(sig: &str, aliases: &std::collections::BTreeMap<String, String>) -> String {
    if aliases.is_empty() {
        return sig.to_string();
    }
    // Bounded: an alias chain is finite, and a cyclic one (rejected by the
    // checker, but this runs on text) must not spin here.
    let mut out = sig.to_string();
    for _ in 0..8 {
        let mut next = String::with_capacity(out.len());
        let mut word = String::new();
        let mut changed = false;
        for ch in out.chars().chain(std::iter::once('\0')) {
            if ch.is_alphanumeric() || ch == '_' {
                word.push(ch);
                continue;
            }
            if !word.is_empty() {
                match aliases.get(&word) {
                    Some(target) => {
                        next.push_str(target);
                        changed = true;
                    }
                    None => next.push_str(&word),
                }
                word.clear();
            }
            if ch != '\0' {
                next.push(ch);
            }
        }
        out = next;
        if !changed {
            break;
        }
    }
    out
}

fn api_surface(program: &Program) -> String {
    use grasm_syntax::ast::Decl;
    let const_types = grasm_sema::check::check_program(program).const_types;
    // Aliases are program-wide: types share one namespace ([T-MODULE-NS]).
    let mut aliases: std::collections::BTreeMap<String, String> =
        std::collections::BTreeMap::new();
    for file in &program.files {
        for decl in &file.module.decls {
            if let Decl::Type(t) = decl {
                if let grasm_syntax::ast::TypeBody::Alias(target) = &t.body {
                    if t.generics.is_empty() {
                        aliases.insert(
                            t.name.name.clone(),
                            grasm_sema::docs::type_text(target),
                        );
                    }
                }
            }
        }
    }
    let mut lines: Vec<String> = Vec::new();
    for file in &program.files {
        // A dependency's exports are that package's surface, not this one's.
        // Including them meant upgrading a dependency reported a breaking
        // change in *your* API — the one signal this command exists to keep
        // clean. The dependency tracks its own surface with its own `api`.
        let root = file.name.split('.').next().unwrap_or(&file.name);
        if program.deps.contains(root) {
            continue;
        }
        for decl in &file.module.decls {
            // Exactly what another package can reach. The documentation
            // collector was the wrong source: it shows the *entry* module's
            // private items too, so a renamed private helper read as a
            // breaking change while dropping `pub` from a real export read as
            // no change at all — backwards in both directions.
            let (name, sig) = match decl {
                Decl::Fn(f) if f.is_pub => (
                    f.name.name.clone(),
                    grasm_sema::docs::fn_signature(f),
                ),
                Decl::Extern(e) if e.is_pub => (
                    e.name.name.clone(),
                    grasm_sema::docs::extern_signature(e),
                ),
                Decl::Const(c) if c.is_pub => {
                    // The inferred type, not just the name: `pub let W = 42`
                    // becoming `= "x"` is a breaking change and the source
                    // alone does not show it.
                    let key = format!("{}::{}", file.name, c.name.name);
                    let sig = match const_types.get(&key) {
                        Some(t) => format!("pub let {}: {t}", c.name.name),
                        None => grasm_sema::docs::const_signature(c),
                    };
                    (c.name.name.clone(), sig)
                }
                // Types and effects share one namespace across the program
                // ([T-MODULE-NS]), so every one of them is surface.
                Decl::Type(t) => (
                    t.name.name.clone(),
                    grasm_sema::docs::type_signature(t),
                ),
                Decl::Effect(e) => (
                    e.name.name.clone(),
                    grasm_sema::docs::effect_signature(e),
                ),
                _ => continue,
            };
            // An alias declaration keeps its own text — `type Text = str`
            // disappearing *is* a change to the surface. Only uses expand.
            let sig = if matches!(decl, Decl::Type(_)) {
                sig
            } else {
                expand_aliases(&sig, &aliases)
            };
            lines.push(format!("{}\t{}\t{}", file.name, name, sig));
        }
    }
    lines.sort();
    lines.dedup();
    let mut out = String::new();
    for l in lines {
        out.push_str(&l);
        out.push('\n');
    }
    out
}

/// Compares two surfaces and says what kind of release this is.
///
/// The question a version number is supposed to answer is "will this break
/// me", and a number a human typed cannot answer it. This one is derived from
/// the surface itself: an item that disappeared or changed shape is breaking,
/// whatever the changelog says.
fn report_api_change(old: &str, new: &str) -> ExitCode {
    use std::collections::BTreeMap;
    let parse = |text: &str| -> BTreeMap<String, String> {
        text.lines()
            .filter_map(|l| {
                let mut parts = l.splitn(3, '\t');
                let m = parts.next()?;
                let n = parts.next()?;
                let sig = parts.next()?;
                Some((format!("{m}::{n}"), sig.to_string()))
            })
            .collect()
    };
    let (before, after) = (parse(old), parse(new));

    let mut removed: Vec<&String> = Vec::new();
    let mut changed: Vec<(&String, &String, &String)> = Vec::new();
    for (key, sig) in &before {
        match after.get(key) {
            None => removed.push(key),
            Some(now) if now != sig => changed.push((key, sig, now)),
            Some(_) => {}
        }
    }
    let added: Vec<&String> = after.keys().filter(|k| !before.contains_key(*k)).collect();

    for k in &removed {
        println!("REMOVED  {k}");
    }
    for (k, was, now) in &changed {
        println!("CHANGED  {k}");
        println!("    was  {was}");
        println!("    now  {now}");
    }
    for k in &added {
        println!("ADDED    {k}");
    }

    let breaking = !removed.is_empty() || !changed.is_empty();
    println!();
    if breaking {
        println!(
            "breaking: {} removed, {} changed, {} added — this is a MAJOR release",
            removed.len(),
            changed.len(),
            added.len()
        );
        ExitCode::FAILURE
    } else if !added.is_empty() {
        println!("compatible: {} added — this is a MINOR release", added.len());
        ExitCode::SUCCESS
    } else {
        println!("no change to the public surface — this is a PATCH release");
        ExitCode::SUCCESS
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
    /// Which of those names are *dependency packages*.
    ///
    /// A bundle reloads its own sources at startup, and without this the
    /// reload had no idea a module came from a dependency — so a library's
    /// `import util` was resolved as a bare name and found the application's
    /// `util` instead. The bundled program then printed a different answer
    /// from the same source run any other way, with nothing reported. This is
    /// C21 for the third time: the loader takes a dependency set, and every
    /// place that reloads a program has to carry it.
    deps: Vec<String>,
    /// (importing module, written path) → the module name the build chose.
    imports: Vec<(String, String, String)>,
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
    // Older payloads stop here. A bundle carries the runtime that reads it, so
    // this only matters while a build is in flight; an absent list means no
    // dependencies, which is what every bundle before this could express.
    let deps = match read_u32(&mut pos) {
        Some(d) => {
            let mut out = Vec::with_capacity(d as usize);
            for _ in 0..d {
                let len = read_u32(&mut pos)? as usize;
                out.push(String::from_utf8(payload.get(pos..pos + len)?.to_vec()).ok()?);
                pos += len;
            }
            out
        }
        None => Vec::new(),
    };
    // Older payloads stop before this, as they did before `deps`.
    let mut imports = Vec::new();
    if let Some(n) = read_u32(&mut pos) {
        for _ in 0..n {
            let mut part = || -> Option<String> {
                let len = read_u32(&mut pos)? as usize;
                let out = String::from_utf8(payload.get(pos..pos + len)?.to_vec()).ok()?;
                pos += len;
                Some(out)
            };
            let (Some(m), Some(a), Some(t)) = (part(), part(), part()) else {
                break;
            };
            imports.push((m, a, t));
        }
    }
    Some(Bundle {
        files,
        deps,
        imports,
    })
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
    let mut deps: Vec<&String> = program.deps.iter().collect();
    deps.sort();
    payload.extend((deps.len() as u32).to_le_bytes());
    for d in deps {
        payload.extend((d.len() as u32).to_le_bytes());
        payload.extend(d.as_bytes());
    }
    // Where each import already resolved to. The module names a build gives a
    // package graph come from the manifests on disk, which a bundle does not
    // carry — deriving them again made a three-package chain look for a module
    // under a name the build never used.
    let mut imports: Vec<(&str, &str, &str)> = Vec::new();
    for f in &program.files {
        for (alias, target) in &f.imports {
            imports.push((f.name.as_str(), alias.as_str(), target.as_str()));
        }
    }
    imports.sort();
    payload.extend((imports.len() as u32).to_le_bytes());
    for (module, alias, target) in imports {
        for part in [module, alias, target] {
            payload.extend((part.len() as u32).to_le_bytes());
            payload.extend(part.as_bytes());
        }
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
    let deps: std::collections::HashSet<String> = bundle.deps.iter().cloned().collect();
    let resolved: HashMap<(String, String), String> = bundle
        .imports
        .iter()
        .map(|(m, a, t)| ((m.clone(), a.clone()), t.clone()))
        .collect();
    let program = loader::load_with_resolved_imports(
        "main",
        "<bundled>",
        &provider,
        &|_| Vec::new(),
        &deps,
        None,
        &|_, _| None,
        &resolved,
    );
    let mut diags = program.diagnostics.clone();
    if diags.is_empty() {
        diags.extend(grasm_sema::check::check_program(&program).diagnostics);
    }
    if !diags.is_empty() {
        render_diags(&program, &diags);
        return ExitCode::FAILURE;
    }
    let mut stdout = std::io::stdout();
    let (code, trace, _fault) = grasm_interp::run_program_traced(&program, program_args, &mut stdout);
    let _ = stdout.flush();
    print_trace(&program, &trace);
    ExitCode::from((code & 0xff) as u8)
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
    // A stack overflow has one frame per level: at the interpreter's bound
    // that is 100,000 lines and about 5 MB of stderr, which buries the
    // message that matters under the repetition that does not. The innermost
    // frames are where the recursion is, so those are the ones kept.
    const KEEP: usize = 20;
    let elided = trace.len().saturating_sub(KEEP);
    for frame in trace.iter().take(KEEP) {
        let i = program.file_of(frame.at);
        let file = &program.files[i];
        let index = grasm_syntax::span::LineIndex::new(&file.src);
        let lc = index.line_col(frame.at.start.saturating_sub(file.base));
        eprintln!(
            "  {} at {}:{}:{}",
            frame.function, file.path, lc.line, lc.col
        );
    }
    if elided > 0 {
        eprintln!("  ... {elided} more frame(s)");
    }
}

// ---------------------------------------------------------------------------
// Native builds (`grasm build --native`)
// ---------------------------------------------------------------------------

/// Locates `libgrasm_rt.a`, which native builds link against.
///
/// Checked in the order a real installation makes sensible, so a downloaded
/// release, a `make install` layout, and a cargo checkout all work without
/// configuration. `grasm build --native` is the headline feature; requiring a
/// new user to already know where the runtime lives would make it useless to
/// exactly the people trying the language for the first time.
fn find_runtime() -> Result<PathBuf, String> {
    if let Ok(explicit) = std::env::var("GRASM_RUNTIME") {
        let p = PathBuf::from(&explicit);
        if p.exists() {
            return Ok(p);
        }
        return Err(format!(
            "GRASM_RUNTIME is set to `{explicit}`, but no file is there"
        ));
    }
    let exe = std::env::current_exe().map_err(|e| e.to_string())?;
    let dir = exe.parent().ok_or("cannot locate the grasm binary")?;
    let candidates = [
        dir.join("libgrasm_rt.a"),               // release archive, cargo target
        dir.join("../lib/grasm/libgrasm_rt.a"),  // bin/ + lib/ install layout
        dir.join("lib/libgrasm_rt.a"),
        dir.join("../lib/libgrasm_rt.a"),
    ];
    for c in &candidates {
        if c.exists() {
            return Ok(c.clone());
        }
    }
    Err(format!(
        "cannot find the Grasm runtime (libgrasm_rt.a).\n\
         \n\
         Looked next to `{}` and in ../lib.\n\
         \n\
         If you downloaded a release, keep `grasm` and `libgrasm_rt.a` in the\n\
         same directory. From a source checkout, run:\n\
         \n\
             cargo build --release --workspace\n\
         \n\
         Or point at it directly:  GRASM_RUNTIME=/path/to/libgrasm_rt.a\n\
         \n\
         `grasm run` needs none of this — only `--native` links the runtime.",
        dir.display()
    ))
}

/// Cache key for one module's object file.
///
/// Two things can invalidate it: the module's own source (its bodies), and the
/// *signature surface* of the whole program (any function signature, type, or
/// effect anywhere, since types share a global namespace). Editing a body in
/// another module changes neither, which is the whole point — that rebuild
/// reuses this object.
fn module_cache_key(program: &Program, index: usize, world_sig: &str) -> String {
    let f = &program.files[index];
    let mut buf = Vec::new();
    buf.extend(f.name.as_bytes());
    buf.push(0);
    buf.extend(f.src.as_bytes());
    buf.push(0);
    buf.extend(world_sig.as_bytes());
    grasm_pkg::hash::sha256_hex(&buf)
}

/// The program's whole signature surface, in a stable order.
fn world_signature(program: &Program) -> String {
    let mut sigs: Vec<String> = program
        .files
        .iter()
        .map(|f| format!("{}\n{}", f.name, grasm_sema::docs::module_signature(&f.module)))
        .collect();
    sigs.sort();
    sigs.join("\n")
}

fn build_native(program: &Program, out_path: &str) -> Result<u64, String> {
    build_native_incremental(program, out_path, true)
}

/// Whole-program build: one object, no cache. Best for a single build (CI),
/// where there is no later rebuild to amortize the split against.
fn build_native_whole(program: &Program, out_path: &str) -> Result<u64, String> {
    build_native_incremental(program, out_path, false)
}

/// Compiles each module to its own object, reusing cached ones whose inputs
/// have not changed, then links them together.
fn build_native_incremental(
    program: &Program,
    out_path: &str,
    incremental: bool,
) -> Result<u64, String> {
    let scratch = std::env::temp_dir().join(format!("grasm_native_{}", std::process::id()));
    std::fs::create_dir_all(&scratch).map_err(|e| e.to_string())?;

    let mut objects: Vec<std::path::PathBuf> = Vec::new();
    if incremental && program.files.len() > 1 {
        // The cache lives beside the output so a clean is `rm -rf`.
        let cache = std::path::Path::new(out_path)
            .parent()
            .unwrap_or(std::path::Path::new("."))
            .join(".grasm-cache");
        std::fs::create_dir_all(&cache).map_err(|e| e.to_string())?;
        let world = world_signature(program);
        let planned: Vec<(String, std::path::PathBuf)> = program
            .files
            .iter()
            .enumerate()
            .map(|(i, f)| {
                let key = module_cache_key(program, i, &world);
                (
                    f.name.clone(),
                    cache.join(format!("{}-{key}.o", f.name.replace('.', "_"))),
                )
            })
            .collect();

        // Modules are independent once signatures are known, so compile the
        // stale ones concurrently. Without this a cold build pays for setting
        // up a code generator per module in series, which costs more than the
        // whole-program path it replaces.
        let stale: Vec<&(String, std::path::PathBuf)> =
            planned.iter().filter(|(_, o)| !o.exists()).collect();
        if !stale.is_empty() {
            let workers = std::thread::available_parallelism()
                .map(|p| p.get())
                .unwrap_or(1)
                .min(stale.len());
            let errors: std::sync::Mutex<Vec<String>> = std::sync::Mutex::new(Vec::new());
            let next = std::sync::atomic::AtomicUsize::new(0);
            std::thread::scope(|scope| {
                for _ in 0..workers {
                    scope.spawn(|| loop {
                        let i = next.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                        let Some((name, obj)) = stale.get(i) else {
                            return;
                        };
                        if let Err(e) =
                            grasm_native::compile_module_object(program, name, obj, None)
                        {
                            errors.lock().expect("no panics inside").push(e);
                        }
                    });
                }
            });
            // Report the first failure deterministically rather than whichever
            // thread happened to finish first.
            let mut errs = errors.into_inner().expect("no panics inside");
            if !errs.is_empty() {
                errs.sort();
                return Err(errs.remove(0));
            }
        }
        objects.extend(planned.into_iter().map(|(_, o)| o));
    } else {
        let obj = scratch.join("out.o");
        grasm_native::compile_object(program, &obj)?;
        objects.push(obj);
    }

    let rt = find_runtime()?;
    let mut link = std::process::Command::new("cc");
    for o in &objects {
        link.arg(o);
    }
    link.arg(&rt).arg("-o").arg(out_path);
    // Strip runtime code the program never calls. The archive carries the
    // whole stdlib surface; a hello-world only needs a sliver of it, and
    // without this every binary pays for all of it (1.5 MB -> ~0.5 MB).
    if cfg!(target_os = "macos") {
        link.arg("-Wl,-dead_strip");
    } else if cfg!(target_os = "linux") {
        link.arg("-Wl,--gc-sections");
    }
    // Captured rather than inherited: the linker's own text names a temporary
    // object file, an internal symbol and an architecture, and four lines of
    // it arrived before grasm's own one-line diagnostic. The one thing in it
    // a reader needs is which symbol was missing.
    let out = link
        .output()
        .map_err(|e| format!("failed to run `cc`: {e}"))?;
    if !out.status.success() {
        let text = String::from_utf8_lossy(&out.stderr);
        let missing: Vec<String> = text
            .lines()
            .filter_map(|l| {
                let l = l.trim();
                // `"_sym", referenced from:` (macOS) and
                // `undefined reference to \`sym'` (GNU ld).
                let name = if let Some(rest) = l.strip_prefix('"') {
                    rest.split('"').next()?.trim_start_matches('_').to_string()
                } else if let Some(rest) = l.split("undefined reference to").nth(1) {
                    rest.trim().trim_matches(|c| c == '`' || c == '\'').to_string()
                } else {
                    return None;
                };
                (!name.is_empty() && !name.starts_with("g_")).then_some(name)
            })
            .collect();
        if !missing.is_empty() {
            let mut names: Vec<String> = missing;
            names.sort();
            names.dedup();
            return Err(format!(
                "cannot find the C function(s) {} — an `extern fn` is resolved by the system linker, so the library defining it has to be on the link line",
                names
                    .iter()
                    .map(|n| format!("`{n}`"))
                    .collect::<Vec<_>>()
                    .join(", ")
            ));
        }
        // Something else went wrong; the linker's own words are all there is.
        let detail = text.trim();
        if detail.is_empty() {
            return Err("linking failed".into());
        }
        return Err(format!("linking failed:\n{detail}"));
    }
    let size = std::fs::metadata(out_path).map_err(|e| e.to_string())?.len();
    Ok(size)
}

// ---------------------------------------------------------------------------
// Package management (`grasm deps`)
// ---------------------------------------------------------------------------

/// Resolves the dependency graph, enforces each package's capability ceiling,
/// and writes the lockfile.
/// Where to publish to: `--registry`, else `GRASM_REGISTRY`, else the nearest
/// `.grasm/registry`. Creating one is the publisher's decision, so a missing
/// registry says how to make one rather than making it.
fn publish_registry(
    dir: &std::path::Path,
    flag: Option<String>,
) -> Result<grasm_pkg::Registry, String> {
    if let Some(p) = flag {
        return Ok(grasm_pkg::Registry::new(PathBuf::from(p)));
    }
    grasm_pkg::Registry::discover(dir).ok_or_else(|| {
        "no registry found\n  \
         pass --registry <dir>, set GRASM_REGISTRY, or create .grasm/registry"
            .to_string()
    })
}

fn cmd_publish(dir: &std::path::Path, registry: Option<String>) -> Result<(), String> {
    let reg = publish_registry(dir, registry)?;
    let out = grasm_pkg::publish(dir, &reg)?;
    println!(
        "published {} {} — {} file(s) to {}",
        out.name,
        out.version,
        out.files,
        out.dir.display()
    );
    Ok(())
}

fn cmd_add(dir: &std::path::Path, name: &str, version: &str) -> Result<(), String> {
    let req = grasm_pkg::VersionReq::parse(version)?;
    let into = grasm_pkg::add_dependency(dir, name, &req)?;
    println!("added `{name}` {req} to {into}");
    println!("  run `grasm deps` to resolve it");
    Ok(())
}

fn cmd_deps(dir: &std::path::Path, locked: bool) -> Result<(), String> {
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
    let mut unknown: Vec<grasm_pkg::UnknownCapability> = Vec::new();
    let mut unanalyzable = Vec::new();
    for pkg in &resolution.packages {
        // A package that cannot be analyzed used to warn and carry on, which
        // meant the ceiling silently stopped being enforced for it while the
        // command still exited 0 and wrote the lockfile. There is no safe
        // reading of "I could not check this dependency": whatever the reason,
        // the guarantee the ceiling exists to give is not being given.
        let performed = match grasm_pkg::effects_of_package(pkg) {
            Ok(e) => e,
            Err(e) => {
                println!("  {:<16} CANNOT ANALYZE", pkg.name);
                unanalyzable.push(format!("`{}`: {e}", pkg.name));
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
            let help = ceiling_help(pkg, &v);
            violations.push((v, help));
        }
        // A ceiling only means something if its names do. A capability naming
        // no real effect reads to a reviewer as a granted permission while
        // granting nothing at all.
        if let Ok(vocab) = grasm_pkg::effect_vocabulary_of_package(pkg) {
            unknown.extend(grasm_pkg::check_capability_names(pkg, &vocab));
        }
    }

    if !unanalyzable.is_empty() {
        for u in &unanalyzable {
            eprintln!("error: cannot analyze {u}");
        }
        eprintln!(
            "  help: a package whose effects cannot be inferred cannot be held \
to its ceiling,\n        so its dependents get no guarantee at all"
        );
        return Err(format!(
            "{} package(s) could not be checked",
            unanalyzable.len()
        ));
    }

    if !unknown.is_empty() {
        for u in &unknown {
            eprintln!("error: {u}");
        }
        eprintln!(
            "  help: a capability must be `io`, `async`, `unsafe`, or an effect the \
program declares"
        );
        return Err(format!(
            "{} capability name(s) do not exist",
            unknown.len()
        ));
    }

    if !violations.is_empty() {
        for (v, help) in &violations {
            eprintln!("error: {v}");
            eprintln!("  help: {help}");
        }
        return Err(format!(
            "{} package(s) exceed their declared capabilities",
            violations.len()
        ));
    }

    let entries = grasm_pkg::build_lock(&resolution)?;
    let lock_path = dir.join(grasm_pkg::LOCKFILE_NAME);
    let changes = match std::fs::read_to_string(&lock_path) {
        Ok(previous) => grasm_pkg::diff_lock(&previous, &entries),
        Err(_) => Vec::new(),
    };

    if locked {
        // CI mode: a dependency that is not what the lockfile recorded is the
        // case the lockfile exists to catch, so it fails rather than being
        // silently rewritten into agreement.
        if !changes.is_empty() {
            for c in &changes {
                eprintln!("error: {c}");
            }
            eprintln!(
                "  help: run `grasm deps` without --locked to accept these and \
update {}",
                grasm_pkg::LOCKFILE_NAME
            );
            return Err(format!(
                "{} change(s) since {}",
                changes.len(),
                grasm_pkg::LOCKFILE_NAME
            ));
        }
        println!("{} is up to date", grasm_pkg::LOCKFILE_NAME);
        return Ok(());
    }

    for c in &changes {
        println!("  note: {c}");
    }
    std::fs::write(&lock_path, grasm_pkg::render_lock(&entries))
        .map_err(|e| format!("cannot write {}: {e}", lock_path.display()))?;
    println!("wrote {}", lock_path.display());
    Ok(())
}
