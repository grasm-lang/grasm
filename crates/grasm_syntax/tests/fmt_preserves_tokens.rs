//! The one property a formatter must have: it does not change the program.
//!
//! Layout is the only thing `grasm fmt` may touch, so the token stream it
//! produces must equal the token stream it was given — every file in the tree,
//! not a handful of cases someone thought of. A spacing rule that is merely
//! ugly shows up in review; one that moves a token compiles to something else
//! and shows up nowhere, which is why this is checked mechanically.
//!
//! Idempotence is checked in the same pass, since it costs one more format and
//! `fmt --check` is worthless without it.

use grasm_syntax::fmt::format_src;
use grasm_syntax::lexer::lex;
use grasm_syntax::token::{StrPart, TokenKind};
use std::path::{Path, PathBuf};

fn gr_files(dir: &Path, out: &mut Vec<PathBuf>) {
    let Ok(rd) = std::fs::read_dir(dir) else {
        return;
    };
    for e in rd.flatten() {
        let p = e.path();
        if p.is_dir() {
            if !matches!(
                p.file_name().and_then(|s| s.to_str()),
                Some("target" | ".git")
            ) {
                gr_files(&p, out);
            }
        } else if p.extension().and_then(|s| s.to_str()) == Some("gr") {
            out.push(p);
        }
    }
}

/// Token kinds with position stripped. An interpolation slot records the byte
/// offset of its expression so sub-expression spans stay accurate, and moving
/// a line's indentation moves that offset by design — it is where the token
/// is, not what it is. Everything else must match exactly.
fn kinds(src: &str) -> Vec<TokenKind> {
    lex(src)
        .into_iter()
        .map(|t| match t.kind {
            TokenKind::Str(parts) => TokenKind::Str(
                parts
                    .into_iter()
                    .map(|p| match p {
                        StrPart::Interp { src, .. } => StrPart::Interp { src, offset: 0 },
                        other => other,
                    })
                    .collect(),
            ),
            other => other,
        })
        .collect()
}

#[test]
fn formatting_never_changes_the_token_stream() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .canonicalize()
        .unwrap();
    let mut files = Vec::new();
    gr_files(&root, &mut files);
    assert!(
        files.len() > 200,
        "expected the whole corpus, found {}",
        files.len()
    );

    let mut changed = 0usize;
    let mut bad: Vec<String> = Vec::new();
    for f in &files {
        let Ok(src) = std::fs::read_to_string(f) else {
            continue;
        };
        let once = format_src(&src);
        let before = kinds(&src);
        let after = kinds(&once);
        if before != after {
            let at = before
                .iter()
                .zip(after.iter())
                .position(|(a, b)| a != b)
                .unwrap_or(before.len().min(after.len()));
            let ctx = |v: &Vec<TokenKind>| {
                v.iter()
                    .skip(at.saturating_sub(3))
                    .take(7)
                    .map(|k| format!("{k:?}"))
                    .collect::<Vec<_>>()
                    .join(" ")
            };
            bad.push(format!(
                "{}\n     was: {}\n     now: {}",
                f.strip_prefix(&root).unwrap_or(f).display(),
                ctx(&before),
                ctx(&after)
            ));
        }
        if once != src {
            changed += 1;
        }
        let twice = format_src(&once);
        if twice != once {
            bad.push(format!(
                "{}: not idempotent",
                f.strip_prefix(&root).unwrap_or(f).display()
            ));
        }
    }
    eprintln!("fmt: {} files, {} would change", files.len(), changed);
    assert!(
        bad.is_empty(),
        "{} files broken by fmt:\n  {}",
        bad.len(),
        bad.join("\n  ")
    );
}
