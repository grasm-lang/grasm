//! Recursive-descent parser with error recovery.
//!
//! Newline discipline: newlines are statement terminators. Inside `(` and `[`
//! groupings they are insignificant (auto-skipped); inside `{` they matter
//! again. A mode stack tracks this. Binary operators at end-of-line continue
//! the expression.

use crate::ast::*;
use crate::diag::Diagnostic;
use crate::lexer::lex;
use crate::span::Span;
use crate::token::{StrPart, Token, TokenKind};

pub struct ParseOutput {
    pub module: Module,
    pub diagnostics: Vec<Diagnostic>,
}

pub fn parse(src: &str) -> ParseOutput {
    let (tokens, docs) = crate::lexer::lex_with_docs(src);
    let mut p = Parser::new(tokens);
    let mut module = p.parse_module();
    attach_docs(&mut module, src, 0, &docs);
    ParseOutput {
        module,
        diagnostics: p.diags,
    }
}

/// Files each run of consecutive `///` lines under the declaration that
/// follows it. A run not followed by a declaration is dropped, so a trailing
/// comment at the end of a file is never silently attached to something.
fn attach_docs(module: &mut Module, src: &str, offset: u32, docs: &[(Span, String)]) {
    if docs.is_empty() {
        return;
    }
    let mut starts: Vec<u32> = module.decls.iter().map(|d| decl_span(d).start).collect();
    starts.sort_unstable();

    // Two doc lines belong to the same block when only whitespace separates
    // them — a blank line or any code in between starts a new block.
    let adjacent = |prev_end: u32, next_start: u32| -> bool {
        let (a, b) = ((prev_end - offset) as usize, (next_start - offset) as usize);
        let between = src.get(a..b).unwrap_or("");
        between.chars().all(char::is_whitespace)
            && between.chars().filter(|c| *c == '\n').count() == 1
    };

    let mut i = 0;
    while i < docs.len() {
        let mut j = i + 1;
        while j < docs.len() && adjacent(docs[j - 1].0.end, docs[j].0.start) {
            j += 1;
        }
        let end = docs[j - 1].0.end;
        if let Some(start) = starts.iter().copied().find(|s| *s >= end) {
            let text = docs[i..j]
                .iter()
                .map(|(_, t)| t.as_str())
                .collect::<Vec<_>>()
                .join("\n");
            module.docs.insert(start, text);
        }
        i = j;
    }
}

/// Parse a whole file with all spans shifted by `offset` — modules live in
/// one virtual span space so diagnostics stay a single flat list.
pub fn parse_with_offset(src: &str, offset: u32) -> ParseOutput {
    let (mut tokens, mut docs) = crate::lexer::lex_with_docs(src);
    for d in &mut docs {
        d.0 = Span::new(d.0.start + offset, d.0.end + offset);
    }
    for t in &mut tokens {
        t.span = Span::new(t.span.start + offset, t.span.end + offset);
    }
    // Interpolation sub-parses re-lex from stored offsets; shift those too by
    // patching StrPart offsets.
    for t in &mut tokens {
        if let TokenKind::Str(parts) = &mut t.kind {
            for p in parts {
                if let crate::token::StrPart::Interp { offset: o, .. } = p {
                    *o += offset;
                }
            }
        }
    }
    let mut p = Parser::new(tokens);
    let mut module = p.parse_module();
    attach_docs(&mut module, src, offset, &docs);
    ParseOutput {
        module,
        diagnostics: p.diags,
    }
}

/// Parse a single type, as written in source.
///
/// The checker knows types the backend cannot re-derive — the element type of
/// an empty literal, for one — and hands them over as the text a person would
/// write. This is how that text comes back as a tree.
pub fn parse_type_src(src: &str) -> Option<TypeExpr> {
    let mut p = Parser::new(lex(src));
    p.skip_newlines();
    let t = p.parse_type_nested().ok()?;
    p.skip_newlines();
    if !p.at(&TokenKind::Eof) || !p.diags.is_empty() {
        return None;
    }
    Some(t)
}

/// Parse a single expression (used by the REPL and interpolation).
pub fn parse_expr_src(src: &str, offset: u32) -> (Option<Expr>, Vec<Diagnostic>) {
    parse_expr_src_at(src, offset, 0)
}

/// The same, starting from a depth already spent.
///
/// An interpolation is parsed by a *new* parser, which started counting from
/// zero — so nesting `"{ … }"` inside itself was the one shape the cap could
/// not see, and 60 000 levels aborted the process with a stack overflow while
/// 40 000 were accepted with no error at all ([G-NEST-LIMIT]).
fn parse_expr_src_at(src: &str, offset: u32, depth: u32) -> (Option<Expr>, Vec<Diagnostic>) {
    let mut tokens = lex(src);
    for t in &mut tokens {
        t.span = Span::new(t.span.start + offset, t.span.end + offset);
    }
    let mut p = Parser::new(tokens);
    p.depth = depth;
    p.skip_newlines();
    let expr = p.parse_expr().ok();
    // Everything between the braces has to be one expression. Parsing a
    // prefix and discarding the rest meant `"A{1 2}B"` printed `A1B` — a typo
    // inside `{...}` silently deleted part of the output, and both engines
    // agreed on the wrong answer so no differential test could see it.
    p.skip_newlines();
    if expr.is_some() && !p.at(&TokenKind::Eof) {
        let leftover = p.cur_span();
        p.diags.push(
            Diagnostic::error(
                leftover,
                "unexpected extra input inside `{...}` — an interpolation holds one expression",
            )
            .with_help("close the interpolation before this, or join the parts with an operator"),
        );
    }
    (expr, p.diags)
}

#[derive(Clone, Copy, PartialEq)]
enum NlMode {
    Keep,
    Skip,
}

type PResult<T> = Result<T, ()>;

/// Hard recursion cap: keeps deeply nested (possibly adversarial) input from
/// overflowing the native stack (each level costs ~13 recursive-descent
/// frames). Applied to both expressions and blocks.
///
/// The cap is on the *depth of the tree*, not on how many brackets were
/// typed. `1 + 1 + … + 1` needs no brackets at all and parses without
/// recursing, but it builds a left spine as deep as it is long, and every
/// pass after the parser walks that spine by recursion: 8000 terms aborted
/// the process with a stack overflow and no diagnostic. So a folded binary
/// operator counts as a level, exactly as a bracket does.
///
/// The number is generous because it is not the real safety margin — the
/// stack the tool runs on is (see `COMPILER_STACK`). It is here so that
/// adversarial input gets a diagnostic instead of finding whatever the
/// margin happens to be.
const MAX_NESTING: u32 = 2048;

struct Parser {
    toks: Vec<Token>,
    i: usize,
    modes: Vec<NlMode>,
    diags: Vec<Diagnostic>,
    /// When true, `Ident {` is NOT a struct literal (condition positions).
    no_struct_lit: bool,
    depth: u32,
    /// Token positions already found not to start a map literal.
    ///
    /// `{…}` is tried as a map and, failing that, re-parsed as a block. Both
    /// attempts descend into the same nested braces, so each level of nesting
    /// doubled the work: `"{{{{…1…}}}}"` at depth 26 — an 81-byte file — took
    /// 27 seconds, and every two further levels quadrupled it. The nesting cap
    /// could never fire, because reaching it took geometric time. The verdict
    /// for a position does not depend on how it was reached, so recording it
    /// makes the second attempt a single pass.
    not_a_map: std::collections::HashSet<usize>,
    /// Whether the map reading of `{...}` has read a `key :` and so cannot be
    /// anything else.
    map_committed: bool,
    /// Functions an `impl` was lowered to, waiting to follow its header into
    /// the module ([T-INTERFACE]).
    lowered: Vec<Decl>,
}

/// What to write instead, for an operator this language does not have.
///
/// [L-MAXMUNCH] lists them precisely because they are what someone arriving
/// from another language types first.
fn spelled_differently(t: &TokenKind) -> Option<&'static str> {
    Some(match t {
        TokenKind::ColonColon => "paths use `.`, as in `utils.helper`",
        TokenKind::FatArrow => "match arms use `->`, not `=>`",
        TokenKind::At => "there is no `@`; bind a name with `let`",
        TokenKind::Question | TokenKind::QuestionDot => {
            "an `Option` is opened with `??` or `match`, and a fallible value with `try`"
        }
        TokenKind::Bang => "negation is `not`, as in `not done`",
        _ => return None,
    })
}

impl Parser {
    fn new(toks: Vec<Token>) -> Self {
        Parser {
            toks,
            i: 0,
            modes: vec![NlMode::Keep],
            diags: Vec::new(),
            no_struct_lit: false,
            depth: 0,
            not_a_map: std::collections::HashSet::new(),
            map_committed: false,
            lowered: Vec::new(),
        }
    }

    fn enter_nested(&mut self) -> PResult<()> {
        if self.depth >= MAX_NESTING {
            // Not a level count: a block costs more than one, and quoting a
            // number that does not match where it fired is worse than
            // quoting none.
            self.error_here("code is nested too deeply".to_string());
            // Skip to EOF: no useful recovery from pathological nesting.
            while !matches!(self.toks[self.i].kind, TokenKind::Eof) {
                self.i += 1;
            }
            return Err(());
        }
        self.depth += 1;
        Ok(())
    }

    // ------------------------------------------------------------------
    // Token stream helpers
    // ------------------------------------------------------------------

    fn mode(&self) -> NlMode {
        *self.modes.last().unwrap()
    }

    /// Advances past newlines when the current mode ignores them.
    fn normalize(&mut self) {
        if self.mode() == NlMode::Skip {
            while matches!(self.toks[self.i].kind, TokenKind::Newline) {
                self.i += 1;
            }
        }
    }

    fn cur(&mut self) -> &TokenKind {
        self.normalize();
        &self.toks[self.i].kind
    }

    fn cur_span(&mut self) -> Span {
        self.normalize();
        self.toks[self.i].span
    }

    fn bump(&mut self) -> Token {
        self.normalize();
        let tok = self.toks[self.i].clone();
        if !matches!(tok.kind, TokenKind::Eof) {
            self.i += 1;
        }
        match tok.kind {
            TokenKind::LParen | TokenKind::LBracket => self.modes.push(NlMode::Skip),
            TokenKind::LBrace => self.modes.push(NlMode::Keep),
            // The outermost mode is the file's own and is never popped: a
            // stray closer would otherwise leave the parser with no mode at
            // all.
            TokenKind::RParen | TokenKind::RBracket | TokenKind::RBrace if self.modes.len() > 1 => {
                self.modes.pop();
            }
            _ => {}
        }
        tok
    }

    fn at(&mut self, kind: &TokenKind) -> bool {
        self.cur() == kind
    }

    fn eat(&mut self, kind: &TokenKind) -> bool {
        if self.at(kind) {
            self.bump();
            true
        } else {
            false
        }
    }

    fn expect(&mut self, kind: &TokenKind) -> PResult<Token> {
        if self.at(kind) {
            Ok(self.bump())
        } else {
            let span = self.cur_span();
            let found = self.cur().describe();
            let mut d = Diagnostic::error(
                span,
                format!("expected {} but found {}", kind.describe(), found),
            );
            if let Some(op) = self.wrapped_operator_ahead() {
                d = d.with_help(format!(
                    "the next line starts with `{op}`, so this statement ended here — a newline ends a statement unless the line *ends* in an operator. Move the `{op}` up to the end of this line, or wrap the whole expression in parentheses"
                ));
            }
            self.diags.push(d);
            Err(())
        }
    }

    /// Whether the token just consumed began a line — i.e. a newline token sits
    /// immediately before it.
    fn at_line_start(&self) -> bool {
        let mut j = self.i;
        // `self.i` has already advanced past the offending token.
        while j > 0 {
            j -= 1;
            match self.toks.get(j).map(|t| &t.kind) {
                Some(TokenKind::Newline) => return true,
                Some(_) if j + 1 == self.i => continue,
                _ => return false,
            }
        }
        false
    }

    /// The binary operator that begins the next line, if one does.
    ///
    /// A newline ends a statement unless the line ends in an operator or a
    /// bracket is open — the same rule Go uses, and a consistent one. What was
    /// missing was any way for a reader to find that out: wrapping a long
    /// condition the natural way
    ///
    /// ```text
    /// if a.contains(x)
    ///     or b.contains(x) {
    /// ```
    ///
    /// reported "expected `{` but found end of line" and then a second,
    /// invented error at the `or`. Neither said what to move. Found by writing
    /// an ordinary program (ProjectGold 06).
    fn wrapped_operator_ahead(&self) -> Option<&'static str> {
        // Shared with the expression-position diagnostic below.
        if !matches!(
            self.toks.get(self.i).map(|t| &t.kind),
            Some(TokenKind::Newline)
        ) {
            return None;
        }
        let mut j = self.i;
        while matches!(self.toks.get(j).map(|t| &t.kind), Some(TokenKind::Newline)) {
            j += 1;
        }
        match self.toks.get(j).map(|t| &t.kind)? {
            TokenKind::KwAnd => Some("and"),
            TokenKind::KwOr => Some("or"),
            TokenKind::Plus => Some("+"),
            TokenKind::Minus => Some("-"),
            TokenKind::Star => Some("*"),
            TokenKind::Slash => Some("/"),
            TokenKind::Percent => Some("%"),
            TokenKind::EqEq => Some("=="),
            TokenKind::NotEq => Some("!="),
            TokenKind::Lt => Some("<"),
            TokenKind::Gt => Some(">"),
            TokenKind::LtEq => Some("<="),
            TokenKind::GtEq => Some(">="),
            TokenKind::PipeGt => Some("|>"),
            _ => None,
        }
    }

    /// True when the token after the current one is `{` — used to decide
    /// whether a contextual keyword introduces a block.
    fn block_follows(&self) -> bool {
        let mut j = self.i + 1;
        while matches!(self.toks.get(j).map(|t| &t.kind), Some(TokenKind::Newline)) {
            j += 1;
        }
        matches!(self.toks.get(j).map(|t| &t.kind), Some(TokenKind::LBrace))
    }

    fn skip_newlines(&mut self) {
        while matches!(self.toks[self.i].kind, TokenKind::Newline) {
            self.i += 1;
        }
    }

    fn error_here(&mut self, msg: impl Into<String>) {
        let span = self.cur_span();
        let mut d = Diagnostic::error(span, msg);
        // The operators [L-MAXMUNCH] says this language deliberately does not
        // have are the ones a newcomer types by reflex. Naming the token was
        // already done; saying what to reach for instead is the part that
        // helps, and only `=>` had it.
        if let Some(h) = spelled_differently(self.cur()) {
            d = d.with_help(h);
        }
        self.diags.push(d);
    }

    /// Panic-mode recovery: skip to the next statement boundary.
    fn sync_stmt(&mut self) {
        loop {
            match &self.toks[self.i].kind {
                TokenKind::Newline | TokenKind::Eof => break,
                TokenKind::RBrace => break,
                _ => {
                    self.bump();
                }
            }
        }
    }

    /// Recovery at declaration level: skip to next top-level keyword.
    /// Resets the newline-mode stack — recovery may have jumped out of
    /// unbalanced delimiters.
    fn sync_decl(&mut self) {
        loop {
            match &self.toks[self.i].kind {
                TokenKind::Eof
                | TokenKind::KwFn
                | TokenKind::KwPub
                | TokenKind::KwType
                | TokenKind::KwEffect
                | TokenKind::KwTest
                | TokenKind::KwImport => break,
                // Only a real `bench "..."` header ends recovery. A bare
                // `bench` is an ordinary identifier, and breaking on it would
                // hand the same token back to parse_decl and spin forever.
                TokenKind::Ident(n)
                    if n == "bench" && matches!(&self.toks[self.i + 1].kind, TokenKind::Str(_)) =>
                {
                    break
                }
                _ => {
                    self.i += 1;
                }
            }
        }
        self.modes.clear();
        self.modes.push(NlMode::Keep);
    }

    fn ident(&mut self) -> PResult<Ident> {
        let span = self.cur_span();
        if let TokenKind::Ident(name) = self.cur().clone() {
            self.bump();
            Ok(Ident { name, span })
        } else {
            let found = self.cur().describe();
            self.diags.push(Diagnostic::error(
                span,
                format!("expected an identifier but found {found}"),
            ));
            Err(())
        }
    }

    /// A name in *binding* position, where `_` means "do not name this".
    ///
    /// `_` was accepted in a `match` pattern and refused everywhere else, so
    /// `for _ in 0..n`, `let _ = f()` and `let (_, b) = pair` were all errors —
    /// while `spec/core.md` classifies `_` as a pattern form and `let (a, b)` as
    /// sugar for projections, which is exactly what leads a reader to expect it
    /// to work in all four places.
    ///
    /// It matters beyond tidiness. [T-FALLIBLE-UNHANDLED] requires a failure to
    /// be handled, and `let _ = risky()` is how a reader writes "I have seen
    /// this and I am dropping it on purpose". Without it the only option is to
    /// invent a name, which reads identically to having forgotten one.
    ///
    /// Each `_` becomes a distinct unwritable name, using the same `\u{1}`
    /// convention as the `for (a, b)` desugaring below: distinct so that
    /// `let (_, _) = pair` does not bind one name twice, and unwritable so that
    /// nothing can read the value back.
    fn binding_ident(&mut self) -> PResult<Ident> {
        if self.at(&TokenKind::Underscore) {
            let span = self.cur_span();
            self.bump();
            return Ok(Ident {
                name: format!("\u{1}wild{}", span.start),
                span,
            });
        }
        self.ident()
    }

    // ------------------------------------------------------------------
    // Module & declarations
    // ------------------------------------------------------------------

    fn parse_module(&mut self) -> Module {
        let mut module = Module::default();
        loop {
            self.skip_newlines();
            if matches!(self.cur(), TokenKind::Eof) {
                break;
            }
            let before = self.i;
            match self.parse_decl() {
                Ok(d) => {
                    module.decls.push(d);
                    module.decls.append(&mut self.lowered);
                }
                Err(()) => {
                    self.sync_decl();
                    // Recovery must consume something. If it did not, the same
                    // token would come back around forever — a hang is a worse
                    // failure than a second diagnostic.
                    if self.i == before {
                        self.i += 1;
                    }
                }
            }
        }
        module
    }

    fn parse_decl(&mut self) -> PResult<Decl> {
        let is_pub = self.eat(&TokenKind::KwPub);
        if self.at_bench_decl() {
            return Ok(Decl::Bench(self.parse_bench_decl()?));
        }
        match self.cur() {
            TokenKind::KwFn => Ok(Decl::Fn(self.parse_fn(is_pub)?)),
            // `extern fn abs(n: i64) -> i64` — a C function.
            TokenKind::Ident(n) if n == "extern" => Ok(Decl::Extern(self.parse_extern(is_pub)?)),
            // `let WIDTH = 77` at module level. Without it every layout
            // constant became a zero-argument function.
            TokenKind::KwLet => Ok(Decl::Const(self.parse_const(is_pub)?)),
            TokenKind::KwType => Ok(Decl::Type(self.parse_type_decl(is_pub)?)),
            TokenKind::KwEffect => Ok(Decl::Effect(self.parse_effect_decl()?)),
            TokenKind::KwTest => Ok(Decl::Test(self.parse_test_decl()?)),
            TokenKind::KwImport => Ok(Decl::Import(self.parse_import()?)),
            TokenKind::KwInterface => Ok(Decl::Interface(self.parse_interface(is_pub)?)),
            TokenKind::KwImpl => Ok(Decl::Impl(self.parse_impl()?)),
            _ => {
                let found = self.cur().describe();
                self.error_here(format!(
                    "expected a declaration (`fn`, `let`, `type`, `effect`, `interface`, `impl`, `test`, `bench`, `import`) but found {found}"
                ));
                Err(())
            }
        }
    }

    fn parse_extern(&mut self, is_pub: bool) -> PResult<ExternDecl> {
        let start = self.cur_span();
        self.bump(); // `extern`
        self.expect(&TokenKind::KwFn)?;
        let name = self.ident()?;
        let params = self.parse_params()?;
        let ret = if self.eat(&TokenKind::Arrow) {
            Some(self.parse_type()?)
        } else {
            None
        };
        let span = start.merge(self.toks[self.i - 1].span);
        Ok(ExternDecl {
            is_pub,
            name,
            params,
            ret,
            span,
        })
    }

    fn parse_const(&mut self, is_pub: bool) -> PResult<ConstDecl> {
        let start = self.cur_span();
        self.expect(&TokenKind::KwLet)?;
        let name = self.ident()?;
        let ty = if self.eat(&TokenKind::Colon) {
            Some(self.parse_type_nested()?)
        } else {
            None
        };
        self.expect(&TokenKind::Eq)?;
        self.skip_newlines_if_continuation();
        let init = self.parse_expr()?;
        let span = start.merge(init.span());
        Ok(ConstDecl {
            is_pub,
            name,
            ty,
            init,
            span,
        })
    }

    fn parse_fn(&mut self, is_pub: bool) -> PResult<FnDecl> {
        self.parse_fn_with_self(is_pub, None)
    }

    /// A function, or -- given the type `self` stands for -- a method of an
    /// `impl`, whose first parameter may be the bare word `self`.
    fn parse_fn_with_self(&mut self, is_pub: bool, self_ty: Option<&TypeExpr>) -> PResult<FnDecl> {
        let start = self.cur_span();
        self.expect(&TokenKind::KwFn)?;
        let name = self.ident()?;
        let (generics, bounds) = self.parse_generics()?;
        let params = self.parse_params_self(self_ty)?;
        let (ret, err, effects) = self.parse_fn_tail()?;
        let body = self.parse_block()?;
        let span = start.merge(body.span);
        Ok(FnDecl {
            is_pub,
            name,
            generics,
            bounds,
            params,
            ret,
            err,
            effects,
            body,
            span,
        })
    }

    /// `[T, U]`, each optionally bounded: `[T: Show + Ord, U]`
    /// ([T-INTERFACE-BOUND]). The bounds come back aligned with the names.
    fn parse_generics(&mut self) -> PResult<(Vec<Ident>, Vec<Vec<Path>>)> {
        let mut generics = Vec::new();
        let mut bounds = Vec::new();
        if self.eat(&TokenKind::LBracket) {
            loop {
                if self.at(&TokenKind::RBracket) {
                    break;
                }
                generics.push(self.ident()?);
                let mut these = Vec::new();
                if self.eat(&TokenKind::Colon) {
                    loop {
                        these.push(self.parse_path()?);
                        if !self.eat(&TokenKind::Plus) {
                            break;
                        }
                    }
                }
                bounds.push(these);
                if !self.eat(&TokenKind::Comma) {
                    break;
                }
            }
            self.expect(&TokenKind::RBracket)?;
        }
        Ok((generics, bounds))
    }

    /// `interface Show { fn show(self) -> str }`. A signature's `self` is a
    /// parameter of type `Self`.
    fn parse_interface(&mut self, is_pub: bool) -> PResult<InterfaceDecl> {
        let start = self.cur_span();
        self.expect(&TokenKind::KwInterface)?;
        let name = self.ident()?;
        self.expect(&TokenKind::LBrace)?;
        let self_ty = TypeExpr::Name {
            path: Path {
                segments: vec![Ident {
                    name: "Self".to_string(),
                    span: name.span,
                }],
                span: name.span,
            },
            args: Vec::new(),
            span: name.span,
        };
        let mut methods = Vec::new();
        loop {
            self.skip_newlines();
            if self.at(&TokenKind::RBrace) {
                break;
            }
            let m_start = self.cur_span();
            self.expect(&TokenKind::KwFn)?;
            let m_name = self.ident()?;
            let params = self.parse_params_self(Some(&self_ty))?;
            let (ret, err, fx) = self.parse_fn_tail()?;
            if fx.is_some() {
                self.error_here(
                    "an interface method declares no effects ([T-INTERFACE-PURE])",
                );
                return Err(());
            }
            let span = m_start.merge(self.toks[self.i - 1].span);
            methods.push(FnSig {
                name: m_name,
                params,
                ret,
                err,
                span,
            });
        }
        let rb = self.expect(&TokenKind::RBrace)?;
        Ok(InterfaceDecl {
            is_pub,
            name,
            methods,
            span: start.merge(rb.span),
        })
    }

    /// `impl Show for Point { fn show(self) -> str { ... } }`.
    ///
    /// Each method is lowered here into an ordinary function named
    /// `method@Type` with `self` as its first parameter and every `Self`
    /// in its signature replaced by the implementing type, and queued in
    /// `lowered` to follow the header into the module. So every engine sees
    /// plain functions and needs to know nothing about `impl` except where a
    /// method call resolves to one.
    fn parse_impl(&mut self) -> PResult<ImplDecl> {
        let start = self.cur_span();
        self.expect(&TokenKind::KwImpl)?;
        let iface = self.parse_path()?;
        match self.cur() {
            TokenKind::KwFor => {
                self.bump();
            }
            _ => {
                let found = self.cur().describe();
                self.error_here(format!(
                    "expected `for` after the interface name, as in `impl Show for Point`, but found {found}"
                ));
                return Err(());
            }
        }
        let ty = self.parse_type()?;
        let type_name = match &ty {
            TypeExpr::Name { path, args, .. } if args.is_empty() && path.segments.len() == 1 => {
                path.segments[0].name.clone()
            }
            other => {
                self.diags.push(Diagnostic::error(
                    other.span(),
                    "an `impl` is for a declared, non-generic type or for `i64`, `f64`, `bool` or `str` ([T-INTERFACE])",
                ));
                return Err(());
            }
        };
        self.expect(&TokenKind::LBrace)?;
        let mut methods = Vec::new();
        loop {
            self.skip_newlines();
            if self.at(&TokenKind::RBrace) {
                break;
            }
            let mut f = self.parse_fn_with_self(false, Some(&ty))?;
            for p in &mut f.params {
                p.ty = replace_self(&p.ty, &ty);
            }
            f.ret = f.ret.as_ref().map(|t| replace_self(t, &ty));
            f.err = f.err.as_ref().map(|t| replace_self(t, &ty));
            // An interface method performs nothing, so its `impl` may not
            // either ([T-INTERFACE-PURE]). Declaring the empty row lets the
            // ordinary row check say so, at the line that performs.
            if f.effects.is_some() {
                self.diags.push(Diagnostic::error(
                    f.name.span,
                    "an `impl` method declares no effects: its interface's methods perform none ([T-INTERFACE-PURE])",
                ));
            }
            f.effects = Some(Vec::new());
            let lowered = impl_fn_name(&f.name.name, &type_name);
            methods.push((f.name.clone(), lowered.clone()));
            f.name = Ident {
                name: lowered,
                span: f.name.span,
            };
            // Visible wherever the type is: an impl method is reached through
            // a value of its type, not by importing its name.
            f.is_pub = true;
            self.lowered.push(Decl::Fn(f));
        }
        let rb = self.expect(&TokenKind::RBrace)?;
        Ok(ImplDecl {
            iface,
            ty,
            type_name,
            methods,
            span: start.merge(rb.span),
        })
    }

    fn parse_params(&mut self) -> PResult<Vec<Param>> {
        self.parse_params_self(None)
    }

    /// A parameter list. With `self_ty`, the first parameter may be the bare
    /// word `self`, which stands for a parameter of that type.
    fn parse_params_self(&mut self, self_ty: Option<&TypeExpr>) -> PResult<Vec<Param>> {
        self.expect(&TokenKind::LParen)?;
        let mut params = Vec::new();
        loop {
            if self.at(&TokenKind::RParen) {
                break;
            }
            let start = self.cur_span();
            if matches!(self.cur(), TokenKind::Ident(n) if n == "self")
                && !matches!(self.toks.get(self.i + 1).map(|t| &t.kind), Some(TokenKind::Colon))
            {
                let Some(st) = self_ty else {
                    self.error_here(
                        "`self` is a parameter of an interface or `impl` method only ([T-INTERFACE])",
                    );
                    return Err(());
                };
                if !params.is_empty() {
                    self.error_here("`self` must be the first parameter");
                    return Err(());
                }
                let name = self.ident()?;
                params.push(Param {
                    mode: ParamMode::Let,
                    name,
                    ty: st.clone(),
                    span: start,
                });
                if !self.eat(&TokenKind::Comma) {
                    break;
                }
                continue;
            }
            let mode = if self.eat(&TokenKind::KwInout) {
                ParamMode::Inout
            } else if self.eat(&TokenKind::KwOwn) {
                ParamMode::Own
            } else {
                ParamMode::Let
            };
            let name = self.ident()?;
            self.expect(&TokenKind::Colon)?;
            let ty = self.parse_type_nested()?;
            let span = start.merge(ty.span());
            params.push(Param {
                mode,
                name,
                ty,
                span,
            });
            if !self.eat(&TokenKind::Comma) {
                break;
            }
        }
        self.expect(&TokenKind::RParen)?;
        Ok(params)
    }

    /// One entry in an effect row.
    ///
    /// `unsafe` is a keyword, and it is also a capability that belongs in a
    /// row — `/ {io, unsafe}` is what a function calling C declares. Parsing
    /// rows as plain paths made that unwritable, so the only way to satisfy
    /// the checker was to leave the row off and let it be inferred.
    fn parse_row_entry(&mut self) -> PResult<Path> {
        if self.at(&TokenKind::KwUnsafe) {
            let t = self.bump();
            return Ok(Path {
                segments: vec![Ident {
                    name: "unsafe".to_string(),
                    span: t.span,
                }],
                span: t.span,
            });
        }
        self.parse_path()
    }

    /// `-> Ret ! Err / {effects}` — every piece optional.
    #[allow(clippy::type_complexity)]
    fn parse_fn_tail(
        &mut self,
    ) -> PResult<(Option<TypeExpr>, Option<TypeExpr>, Option<Vec<Path>>)> {
        let mut ret = None;
        let mut err = None;
        let mut effects = None;
        if self.eat(&TokenKind::Arrow) {
            ret = Some(self.parse_type()?);
        }
        if self.eat(&TokenKind::Bang) {
            err = Some(self.parse_type()?);
        }
        if self.eat(&TokenKind::Slash) {
            self.expect(&TokenKind::LBrace)?;
            let mut paths = Vec::new();
            loop {
                self.skip_newlines();
                if self.at(&TokenKind::RBrace) {
                    break;
                }
                paths.push(self.parse_row_entry()?);
                if !self.eat(&TokenKind::Comma) {
                    break;
                }
            }
            self.skip_newlines();
            self.expect(&TokenKind::RBrace)?;
            effects = Some(paths);
        }
        Ok((ret, err, effects))
    }

    fn parse_path(&mut self) -> PResult<Path> {
        let first = self.ident()?;
        let mut span = first.span;
        let mut segments = vec![first];
        while self.at(&TokenKind::Dot) {
            self.bump();
            let seg = self.ident()?;
            span = span.merge(seg.span);
            segments.push(seg);
        }
        Ok(Path { segments, span })
    }

    fn parse_type_decl(&mut self, is_pub: bool) -> PResult<TypeDecl> {
        let start = self.cur_span();
        self.expect(&TokenKind::KwType)?;
        let name = self.ident()?;
        let (generics, bounds) = self.parse_generics()?;
        if bounds.iter().any(|b| !b.is_empty()) {
            self.error_here("a type's parameters take no bounds; bound the functions that use it");
            return Err(());
        }
        self.expect(&TokenKind::Eq)?;
        self.skip_newlines();
        let (body, end_span) = if self.at(&TokenKind::LBrace) {
            let fields = self.parse_field_block()?;
            let end = self.toks[self.i - 1].span;
            (TypeBody::Struct(fields), end)
        } else if self.at(&TokenKind::Pipe) {
            let mut variants = Vec::new();
            while self.eat(&TokenKind::Pipe) {
                let vname = self.ident()?;
                let mut vspan = vname.span;
                let mut fields = Vec::new();
                if self.eat(&TokenKind::LParen) {
                    loop {
                        if self.at(&TokenKind::RParen) {
                            break;
                        }
                        let fname = self.ident()?;
                        self.expect(&TokenKind::Colon)?;
                        let fty = self.parse_type_nested()?;
                        let fspan = fname.span.merge(fty.span());
                        fields.push(FieldDef {
                            name: fname,
                            ty: fty,
                            span: fspan,
                        });
                        if !self.eat(&TokenKind::Comma) {
                            break;
                        }
                    }
                    let rp = self.expect(&TokenKind::RParen)?;
                    vspan = vspan.merge(rp.span);
                }
                variants.push(VariantDef {
                    name: vname,
                    fields,
                    span: vspan,
                });
                // Allow newline(s) before the next `|`.
                let save = self.i;
                self.skip_newlines();
                if !self.at(&TokenKind::Pipe) {
                    self.i = save;
                    break;
                }
            }
            let end = self.toks[self.i - 1].span;
            (TypeBody::Sum(variants), end)
        } else {
            let ty = self.parse_type_nested()?;
            let end = ty.span();
            (TypeBody::Alias(ty), end)
        };
        Ok(TypeDecl {
            is_pub,
            name,
            generics,
            body,
            span: start.merge(end_span),
        })
    }

    /// `{ field: T, other: U }` — commas and/or newlines separate.
    fn parse_field_block(&mut self) -> PResult<Vec<FieldDef>> {
        self.expect(&TokenKind::LBrace)?;
        let mut fields = Vec::new();
        loop {
            self.skip_newlines();
            if self.at(&TokenKind::RBrace) {
                break;
            }
            let name = self.ident()?;
            self.expect(&TokenKind::Colon)?;
            let ty = self.parse_type_nested()?;
            let span = name.span.merge(ty.span());
            fields.push(FieldDef { name, ty, span });
            let had_sep = self.eat(&TokenKind::Comma);
            let at_nl = matches!(self.toks[self.i].kind, TokenKind::Newline);
            if !had_sep && !at_nl {
                break;
            }
        }
        self.skip_newlines();
        self.expect(&TokenKind::RBrace)?;
        Ok(fields)
    }

    fn parse_effect_decl(&mut self) -> PResult<EffectDecl> {
        let start = self.cur_span();
        self.expect(&TokenKind::KwEffect)?;
        let name = self.ident()?;
        self.expect(&TokenKind::LBrace)?;
        let mut ops = Vec::new();
        loop {
            self.skip_newlines();
            if self.at(&TokenKind::RBrace) {
                break;
            }
            let op_start = self.cur_span();
            self.expect(&TokenKind::KwFn)?;
            let op_name = self.ident()?;
            let params = self.parse_params()?;
            let (ret, err, _fx) = self.parse_fn_tail()?;
            let span = op_start.merge(self.toks[self.i - 1].span);
            ops.push(FnSig {
                name: op_name,
                params,
                ret,
                err,
                span,
            });
        }
        let rb = self.expect(&TokenKind::RBrace)?;
        Ok(EffectDecl {
            name,
            ops,
            span: start.merge(rb.span),
        })
    }

    fn parse_test_decl(&mut self) -> PResult<TestDecl> {
        let start = self.cur_span();
        self.expect(&TokenKind::KwTest)?;
        let name = match self.cur().clone() {
            TokenKind::Str(parts) => {
                self.bump();
                match parts.as_slice() {
                    [StrPart::Text(t)] => t.clone(),
                    _ => {
                        self.error_here("test names cannot contain interpolation");
                        return Err(());
                    }
                }
            }
            _ => {
                self.error_here("expected a string name after `test`");
                return Err(());
            }
        };
        // A parameter list turns a unit test into a property test.
        let params = if matches!(self.cur(), TokenKind::LParen) {
            self.parse_params()?
        } else {
            Vec::new()
        };
        let body = self.parse_block()?;
        let span = start.merge(body.span);
        Ok(TestDecl {
            name,
            params,
            body,
            span,
        })
    }

    /// `bench "name" { ... }`. `bench` is contextual: it is only a keyword
    /// directly before a string literal at declaration position, so ordinary
    /// code may still use `bench` as a name.
    fn parse_bench_decl(&mut self) -> PResult<BenchDecl> {
        let start = self.cur_span();
        self.bump(); // `bench`
        let name = match self.cur().clone() {
            TokenKind::Str(parts) => {
                self.bump();
                match parts.as_slice() {
                    [StrPart::Text(t)] => t.clone(),
                    _ => {
                        self.error_here("benchmark names cannot contain interpolation");
                        return Err(());
                    }
                }
            }
            _ => {
                self.error_here("expected a string name after `bench`");
                return Err(());
            }
        };
        let body = self.parse_block()?;
        let span = start.merge(body.span);
        Ok(BenchDecl { name, body, span })
    }

    /// True when the cursor sits on a `bench "..."` declaration header.
    fn at_bench_decl(&mut self) -> bool {
        matches!(self.cur(), TokenKind::Ident(n) if n == "bench")
            && matches!(&self.toks[self.i + 1].kind, TokenKind::Str(_))
    }

    fn parse_import(&mut self) -> PResult<ImportDecl> {
        let start = self.cur_span();
        self.expect(&TokenKind::KwImport)?;
        let path = self.parse_path()?;
        let span = start.merge(path.span);
        Ok(ImportDecl { path, span })
    }

    // ------------------------------------------------------------------
    // Types
    // ------------------------------------------------------------------

    /// A type in a nested position, where `T ! E` is unambiguous.
    ///
    /// Everywhere a type is written *inside* something else: a parameter, a
    /// field, a type argument, a tuple member, a `let` annotation. Not at a
    /// function declaration's return type — there the `! E` belongs to the
    /// declaration, alongside the effect row, and is parsed there.
    fn parse_type_nested(&mut self) -> PResult<TypeExpr> {
        let inner = self.parse_type()?;
        if !self.at(&TokenKind::Bang) {
            return Ok(inner);
        }
        self.bump();
        let err = self.parse_type()?;
        let span = inner.span().merge(err.span());
        if self.at(&TokenKind::Bang) {
            // `T ! E ! E` used to be accepted, because the second `!` was
            // read as a function declaration's error type — so the fix
            // suggested for a mis-parsed `ref T ! E` compiled, as a different
            // type from the one asked for.
            self.error_here("a type can carry only one `!`".to_string());
            return Err(());
        }
        Ok(TypeExpr::Fallible(Box::new(inner), Box::new(err), span))
    }

    fn parse_type(&mut self) -> PResult<TypeExpr> {
        // Types nest, and everything that walks one recurses. `[[[…i64…]]]`
        // at 250 000 deep aborted the process, and a 40 000-deep
        // `Map[str, …]` took 45 seconds of `check` with nothing reported.
        self.enter_nested()?;
        let r = self.parse_type_atom();
        self.depth -= 1;
        r
    }

    fn parse_type_atom(&mut self) -> PResult<TypeExpr> {
        let start = self.cur_span();
        match self.cur().clone() {
            TokenKind::Ident(_) => {
                let path = self.parse_path()?;
                let mut args = Vec::new();
                let mut span = path.span;
                if self.at(&TokenKind::LBracket) {
                    self.bump();
                    loop {
                        if self.at(&TokenKind::RBracket) {
                            break;
                        }
                        args.push(self.parse_type_nested()?);
                        if !self.eat(&TokenKind::Comma) {
                            break;
                        }
                    }
                    let rb = self.expect(&TokenKind::RBracket)?;
                    span = span.merge(rb.span);
                }
                Ok(TypeExpr::Name { path, args, span })
            }
            // `fn(A, B) -> R / {io}` — a function type. Parameters have no
            // names here: a caller of a function-typed parameter names nothing.
            TokenKind::KwFn => {
                self.bump();
                self.expect(&TokenKind::LParen)?;
                let mut params = Vec::new();
                loop {
                    if self.at(&TokenKind::RParen) {
                        break;
                    }
                    params.push(self.parse_type_nested()?);
                    if !self.eat(&TokenKind::Comma) {
                        break;
                    }
                }
                let rp = self.expect(&TokenKind::RParen)?;
                let mut span = start.merge(rp.span);
                // Unlike `ref`, a function type's return *absorbs* the `!`:
                // `fn(i64) -> i64 ! E` is a function that can fail, which is
                // what a struct field holding one means. Binding it outward
                // made that field a *fallible function value* instead — a
                // type that displays identically and unifies with nothing, so
                // `Box{ f: twice }` was refused with "expected
                // `fn(i64) -> i64 ! E`, found `fn(i64) -> i64 ! E`". A
                // fallible function value, if anyone wants one, is
                // `(fn(i64) -> i64) ! E`.
                let ret = if self.eat(&TokenKind::Arrow) {
                    let t = self.parse_type_nested()?;
                    span = span.merge(t.span());
                    Box::new(t)
                } else {
                    Box::new(TypeExpr::Tuple(Vec::new(), span))
                };
                let mut effects = Vec::new();
                if self.eat(&TokenKind::Slash) {
                    self.expect(&TokenKind::LBrace)?;
                    loop {
                        self.skip_newlines();
                        if self.at(&TokenKind::RBrace) {
                            break;
                        }
                        effects.push(self.parse_row_entry()?);
                        if !self.eat(&TokenKind::Comma) {
                            break;
                        }
                    }
                    self.skip_newlines();
                    let rb = self.expect(&TokenKind::RBrace)?;
                    span = span.merge(rb.span);
                }
                Ok(TypeExpr::Fn {
                    params,
                    ret,
                    effects,
                    span,
                })
            }
            TokenKind::LBracket => {
                self.bump();
                let inner = self.parse_type_nested()?;
                let rb = self.expect(&TokenKind::RBracket)?;
                Ok(TypeExpr::List(Box::new(inner), start.merge(rb.span)))
            }
            // `ref T` — Tier 2.
            //
            // The referent is parsed *without* the fallible suffix, so
            // `-> ref Node ! E` is `(ref Node) ! E` and not `ref (Node ! E)`.
            // It used to be the latter, and the two errors that followed
            // contradicted each other: "`fail` used in a function that
            // declares no error type" alongside "must return `ref Node ! E`
            // but its body produces `ref Node`". Applying the help verbatim
            // gave `! E ! E`, which parsed as something else again.
            TokenKind::KwRef => {
                self.bump();
                let inner = self.parse_type()?;
                let sp = start.merge(inner.span());
                Ok(TypeExpr::Ref(Box::new(inner), sp))
            }
            TokenKind::LParen => {
                self.bump();
                if self.at(&TokenKind::RParen) {
                    let rp = self.bump();
                    return Ok(TypeExpr::Tuple(Vec::new(), start.merge(rp.span)));
                }
                let first = self.parse_type_nested()?;
                if self.at(&TokenKind::Pipe) {
                    let mut members = vec![first];
                    while self.eat(&TokenKind::Pipe) {
                        members.push(self.parse_type_nested()?);
                    }
                    let rp = self.expect(&TokenKind::RParen)?;
                    Ok(TypeExpr::Union(members, start.merge(rp.span)))
                } else if self.at(&TokenKind::Comma) {
                    let mut members = vec![first];
                    while self.eat(&TokenKind::Comma) {
                        if self.at(&TokenKind::RParen) {
                            break;
                        }
                        members.push(self.parse_type_nested()?);
                    }
                    let rp = self.expect(&TokenKind::RParen)?;
                    Ok(TypeExpr::Tuple(members, start.merge(rp.span)))
                } else {
                    self.expect(&TokenKind::RParen)?;
                    Ok(first)
                }
            }
            other => {
                self.diags.push(Diagnostic::error(
                    start,
                    format!("expected a type but found {}", other.describe()),
                ));
                Err(())
            }
        }
    }

    // ------------------------------------------------------------------
    // Blocks & statements
    // ------------------------------------------------------------------

    fn parse_block(&mut self) -> PResult<Block> {
        self.enter_nested()?;
        let r = self.parse_block_inner();
        self.depth -= 1;
        r
    }

    fn parse_block_inner(&mut self) -> PResult<Block> {
        let start = self.cur_span();
        self.expect(&TokenKind::LBrace)?;
        let mut stmts = Vec::new();
        loop {
            self.skip_newlines();
            if self.at(&TokenKind::RBrace) || matches!(self.cur(), TokenKind::Eof) {
                break;
            }
            match self.parse_stmt() {
                Ok(s) => {
                    stmts.push(s);
                    // Terminator: newline, `}` next, or EOF.
                    if !matches!(self.toks[self.i].kind, TokenKind::Newline)
                        && !self.at(&TokenKind::RBrace)
                        && !matches!(self.cur(), TokenKind::Eof)
                    {
                        let found = self.cur().describe();
                        self.error_here(format!(
                            "expected end of line after statement but found {found}"
                        ));
                        self.sync_stmt();
                    }
                }
                Err(()) => self.sync_stmt(),
            }
        }
        let rb = self.expect(&TokenKind::RBrace)?;
        Ok(Block {
            stmts,
            span: start.merge(rb.span),
        })
    }

    fn parse_stmt(&mut self) -> PResult<Stmt> {
        let start = self.cur_span();
        // `parallel {` and `spawn {` are contextual keywords: they introduce a
        // concurrency scope only when a block follows, so existing code using
        // those words as ordinary names keeps compiling.
        if let TokenKind::Ident(name) = self.cur() {
            let is_parallel = name == "parallel";
            let is_spawn = name == "spawn";
            if (is_parallel || is_spawn) && self.block_follows() {
                self.bump();
                let body = self.parse_block()?;
                let span = start.merge(body.span);
                return Ok(if is_parallel {
                    Stmt::Parallel { body, span }
                } else {
                    Stmt::Spawn { body, span }
                });
            }
        }
        match self.cur() {
            TokenKind::KwLet | TokenKind::KwVar => {
                let mutable = matches!(self.cur(), TokenKind::KwVar);
                self.bump();
                // `let (a, b) = pair` binds a tuple's components by name.
                if self.at(&TokenKind::LParen) {
                    self.bump();
                    let mut names = Vec::new();
                    loop {
                        if self.at(&TokenKind::RParen) {
                            break;
                        }
                        names.push(self.binding_ident()?);
                        if !self.eat(&TokenKind::Comma) {
                            break;
                        }
                    }
                    self.expect(&TokenKind::RParen)?;
                    self.expect(&TokenKind::Eq)?;
                    self.skip_newlines_if_continuation();
                    let init = self.parse_expr()?;
                    let span = start.merge(init.span());
                    return Ok(Stmt::LetTuple {
                        mutable,
                        names,
                        init,
                        span,
                    });
                }
                let name = self.binding_ident()?;
                let ty = if self.eat(&TokenKind::Colon) {
                    Some(self.parse_type_nested()?)
                } else {
                    None
                };
                self.expect(&TokenKind::Eq)?;
                self.skip_newlines_if_continuation();
                let init = self.parse_expr()?;
                let span = start.merge(init.span());
                Ok(Stmt::Let {
                    mutable,
                    name,
                    ty,
                    init,
                    span,
                })
            }
            TokenKind::KwReturn => {
                self.bump();
                let value = if matches!(self.toks[self.i].kind, TokenKind::Newline)
                    || self.at(&TokenKind::RBrace)
                {
                    None
                } else {
                    Some(self.parse_expr()?)
                };
                let span = match &value {
                    Some(e) => start.merge(e.span()),
                    None => start,
                };
                Ok(Stmt::Return(value, span))
            }
            TokenKind::KwBreak => {
                self.bump();
                Ok(Stmt::Break(start))
            }
            TokenKind::KwContinue => {
                self.bump();
                Ok(Stmt::Continue(start))
            }
            TokenKind::KwWhile => {
                self.bump();
                let cond = self.parse_cond_expr()?;
                let body = self.parse_block()?;
                let span = start.merge(body.span);
                Ok(Stmt::While { cond, body, span })
            }
            TokenKind::KwFor => {
                self.bump();
                // `for (a, b) in pairs` — the same split `let (a, b) = p`
                // does, done once per element. It becomes exactly that: a
                // loop over whole elements whose body opens with the split,
                // so nothing downstream has to know about it.
                let mut names: Vec<Ident> = Vec::new();
                let var = if self.at(&TokenKind::LParen) {
                    self.bump();
                    loop {
                        if self.at(&TokenKind::RParen) {
                            break;
                        }
                        names.push(self.binding_ident()?);
                        if !self.eat(&TokenKind::Comma) {
                            break;
                        }
                    }
                    let rp = self.cur_span();
                    self.expect(&TokenKind::RParen)?;
                    Ident {
                        // Unwritable, so it cannot shadow anything the program
                        // named, and cannot be read by mistake.
                        name: "\u{1}each".to_string(),
                        span: rp,
                    }
                } else {
                    self.binding_ident()?
                };
                self.expect(&TokenKind::KwIn)?;
                let iter = self.parse_cond_expr()?;
                let mut body = self.parse_block()?;
                if !names.is_empty() {
                    let span = var.span;
                    body.stmts.insert(
                        0,
                        Stmt::LetTuple {
                            mutable: false,
                            names,
                            init: Expr::Ident(var.clone()),
                            span,
                        },
                    );
                }
                let span = start.merge(body.span);
                Ok(Stmt::For {
                    var,
                    iter,
                    body,
                    span,
                })
            }
            TokenKind::KwWith => {
                self.bump();
                let effect = self.parse_path()?;
                self.expect(&TokenKind::KwHandledBy)?;
                let handler = self.parse_expr()?;
                let body = self.parse_block()?;
                let span = start.merge(body.span);
                Ok(Stmt::With {
                    effect,
                    handler,
                    body,
                    span,
                })
            }
            _ => {
                let expr = self.parse_expr()?;
                let op = match self.cur() {
                    TokenKind::Eq => Some(AssignOp::Assign),
                    TokenKind::PlusEq => Some(AssignOp::Add),
                    TokenKind::MinusEq => Some(AssignOp::Sub),
                    TokenKind::StarEq => Some(AssignOp::Mul),
                    TokenKind::SlashEq => Some(AssignOp::Div),
                    _ => None,
                };
                if let Some(op) = op {
                    self.bump();
                    self.skip_newlines_if_continuation();
                    let value = self.parse_expr()?;
                    let span = expr.span().merge(value.span());
                    if !is_place_expr(&expr) {
                        self.diags.push(
                            Diagnostic::error(expr.span(), "invalid assignment target").with_help(
                                "only variables, fields, and index expressions can be assigned to",
                            ),
                        );
                    }
                    Ok(Stmt::Assign {
                        target: expr,
                        op,
                        value,
                        span,
                    })
                } else {
                    Ok(Stmt::Expr(expr))
                }
            }
        }
    }

    /// After `=` or a binary operator at end of line, the expression continues.
    fn skip_newlines_if_continuation(&mut self) {
        self.skip_newlines();
    }

    /// Condition position: struct literals need parens (`if (User{..}).ok {}`).
    fn parse_cond_expr(&mut self) -> PResult<Expr> {
        let saved = self.no_struct_lit;
        self.no_struct_lit = true;
        let r = self.parse_expr();
        self.no_struct_lit = saved;
        r
    }

    // ------------------------------------------------------------------
    // Expressions (precedence climbing)
    // ------------------------------------------------------------------

    pub(crate) fn parse_expr(&mut self) -> PResult<Expr> {
        self.enter_nested()?;
        // Folding a binary operator deepens the tree, and `fold` charges for
        // it without a matching release — so the whole chain is paid for and
        // released together, here, when the expression is done.
        let saved = self.depth;
        let r = self.parse_or();
        self.depth = saved - 1;
        r
    }

    /// Two `|` tokens written side by side, which is `||`.
    ///
    /// Two tokens rather than one, because the lexer must not merge them: `||`
    /// is also a closure with no parameters. Adjacency is required — `| |` with
    /// a space is not what anybody types for `or`, and in operator position it
    /// is a different mistake.
    fn at_double_pipe(&mut self) -> bool {
        if !self.at(&TokenKind::Pipe) {
            return false;
        }
        let here = self.toks[self.i].span;
        matches!(self.toks.get(self.i + 1),
            Some(t) if t.kind == TokenKind::Pipe && t.span.start == here.end)
    }

    /// Reports `&&`/`||` where `and`/`or` was meant, if that is what is here.
    ///
    /// An error, not a warning: accepting both spellings would give the language
    /// two ways to write one thing, which principle 5 of the master plan
    /// forbids. But it is a *recoverable* error — the operator is consumed as
    /// the one it plainly means, so a file full of `&&` reports one mistake per
    /// `&&` and nothing else.
    fn wrong_bool_spelling(&mut self, kind: &TokenKind, wrote: &str, meant: &str) {
        if self.at(kind) {
            let span = self.cur_span();
            self.diags.push(
                Diagnostic::error(span, format!("`{wrote}` is written `{meant}` in Grasm"))
                    .with_help(format!("replace `{wrote}` with `{meant}`")),
            );
        }
    }

    /// Charges one level of tree depth for a folded binary operator.
    fn fold(&mut self, e: Expr) -> PResult<Expr> {
        self.enter_nested()?;
        Ok(e)
    }

    fn parse_or(&mut self) -> PResult<Expr> {
        let mut lhs = self.parse_and()?;
        while self.at(&TokenKind::KwOr) || self.at_double_pipe() {
            // `||` is caught here rather than in the lexer for two reasons. It
            // can be *recovered* from: the operator it means is unambiguous and
            // its precedence is this one, so the mistake is reported once and
            // the rest of the file parses — rejected as a character it left the
            // parser looking for the `{` of an `if` and inventing a second error
            // at the closing brace. And it can be *told apart* here: `||` is
            // also a closure with no parameters, and only a parser that knows
            // an operator is expected can say which this is.
            if self.at_double_pipe() {
                let span = self.cur_span().merge(self.toks[self.i + 1].span);
                self.diags.push(
                    Diagnostic::error(span, "`||` is written `or` in Grasm")
                        .with_help("replace `||` with `or`"),
                );
                self.bump();
            }
            self.bump();
            self.skip_newlines_if_continuation();
            let rhs = self.parse_and()?;
            let span = lhs.span().merge(rhs.span());
            lhs = self.fold(Expr::Binary {
                op: BinaryOp::Or,
                lhs: Box::new(lhs),
                rhs: Box::new(rhs),
                span,
            })?;
        }
        Ok(lhs)
    }

    fn parse_and(&mut self) -> PResult<Expr> {
        let mut lhs = self.parse_comparison()?;
        while self.at(&TokenKind::KwAnd) || self.at(&TokenKind::AmpAmp) {
            // `&&` *is* merged by the lexer, unlike `||`: `&` has no other use
            // anywhere in Grasm, so there is nothing for it to be confused with.
            self.wrong_bool_spelling(&TokenKind::AmpAmp, "&&", "and");
            self.bump();
            self.skip_newlines_if_continuation();
            let rhs = self.parse_comparison()?;
            let span = lhs.span().merge(rhs.span());
            lhs = self.fold(Expr::Binary {
                op: BinaryOp::And,
                lhs: Box::new(lhs),
                rhs: Box::new(rhs),
                span,
            })?;
        }
        Ok(lhs)
    }

    fn parse_comparison(&mut self) -> PResult<Expr> {
        let mut lhs = self.parse_range()?;
        loop {
            let op = match self.cur() {
                TokenKind::EqEq => BinaryOp::Eq,
                TokenKind::NotEq => BinaryOp::NotEq,
                TokenKind::Lt => BinaryOp::Lt,
                TokenKind::LtEq => BinaryOp::LtEq,
                TokenKind::Gt => BinaryOp::Gt,
                TokenKind::GtEq => BinaryOp::GtEq,
                _ => break,
            };
            self.bump();
            self.skip_newlines_if_continuation();
            let rhs = self.parse_range()?;
            let span = lhs.span().merge(rhs.span());
            lhs = self.fold(Expr::Binary {
                op,
                lhs: Box::new(lhs),
                rhs: Box::new(rhs),
                span,
            })?;
        }
        Ok(lhs)
    }

    fn parse_range(&mut self) -> PResult<Expr> {
        let lhs = self.parse_coalesce()?;
        let inclusive = match self.cur() {
            TokenKind::DotDot => false,
            TokenKind::DotDotEq => true,
            _ => return Ok(lhs),
        };
        self.bump();
        let rhs = self.parse_coalesce()?;
        let span = lhs.span().merge(rhs.span());
        Ok(Expr::Range {
            start: Box::new(lhs),
            end: Box::new(rhs),
            inclusive,
            span,
        })
    }

    /// `a ?? b ?? c` groups to the **right**: `a ?? (b ?? c)`.
    ///
    /// Left-associative, the middle operand had to be a plain value, so
    /// `first() ?? second() ?? fallback` did not compile — the `??` between
    /// two `Option`s saw an `Option` where it wanted the element type. For an
    /// operator whose whole job is "or this default", chaining defaults is
    /// the obvious thing to write, and parentheses were the only way to say
    /// it.
    fn parse_coalesce(&mut self) -> PResult<Expr> {
        let lhs = self.parse_pipeline()?;
        if !self.at(&TokenKind::QuestionQuestion) {
            return Ok(lhs);
        }
        self.bump();
        self.skip_newlines_if_continuation();
        let rhs = self.parse_coalesce()?;
        let span = lhs.span().merge(rhs.span());
        self.fold(Expr::Binary {
            op: BinaryOp::Coalesce,
            lhs: Box::new(lhs),
            rhs: Box::new(rhs),
            span,
        })
    }

    /// `x |> f(a)` desugars to `f(x, a)`; `x |> obj.m(a)` to `obj.m(x, a)`.
    fn parse_pipeline(&mut self) -> PResult<Expr> {
        let mut lhs = self.parse_additive()?;
        while self.at(&TokenKind::PipeGt) {
            // A pipeline stage deepens the tree exactly as a folded operator
            // does; 50 000 stages passed the cap without touching it.
            self.enter_nested()?;
            self.bump();
            self.skip_newlines_if_continuation();
            let rhs = self.parse_additive()?;
            let span = lhs.span().merge(rhs.span());
            lhs = match rhs {
                Expr::Call {
                    callee, mut args, ..
                } => {
                    args.insert(0, lhs);
                    Expr::Call { callee, args, span }
                }
                Expr::MethodCall {
                    recv,
                    name,
                    mut args,
                    ..
                } => {
                    args.insert(0, lhs);
                    Expr::MethodCall {
                        recv,
                        name,
                        args,
                        span,
                    }
                }
                callee @ (Expr::Ident(_) | Expr::Field { .. }) => Expr::Call {
                    callee: Box::new(callee),
                    args: vec![lhs],
                    span,
                },
                other => {
                    self.diags.push(
                        Diagnostic::error(
                            other.span(),
                            "right side of `|>` must be a function or call",
                        )
                        .with_help("write `value |> f` or `value |> f(more, args)`"),
                    );
                    other
                }
            };
        }
        Ok(lhs)
    }

    fn parse_additive(&mut self) -> PResult<Expr> {
        let mut lhs = self.parse_multiplicative()?;
        loop {
            let op = match self.cur() {
                TokenKind::Plus => BinaryOp::Add,
                TokenKind::Minus => BinaryOp::Sub,
                _ => break,
            };
            self.bump();
            self.skip_newlines_if_continuation();
            let rhs = self.parse_multiplicative()?;
            let span = lhs.span().merge(rhs.span());
            lhs = self.fold(Expr::Binary {
                op,
                lhs: Box::new(lhs),
                rhs: Box::new(rhs),
                span,
            })?;
        }
        Ok(lhs)
    }

    fn parse_multiplicative(&mut self) -> PResult<Expr> {
        let mut lhs = self.parse_power()?;
        loop {
            let op = match self.cur() {
                TokenKind::Star => BinaryOp::Mul,
                TokenKind::Slash => BinaryOp::Div,
                TokenKind::Percent => BinaryOp::Rem,
                _ => break,
            };
            self.bump();
            self.skip_newlines_if_continuation();
            let rhs = self.parse_power()?;
            let span = lhs.span().merge(rhs.span());
            lhs = self.fold(Expr::Binary {
                op,
                lhs: Box::new(lhs),
                rhs: Box::new(rhs),
                span,
            })?;
        }
        Ok(lhs)
    }

    /// Right-associative: `2^3^2` = `2^(3^2)`.
    fn parse_power(&mut self) -> PResult<Expr> {
        let lhs = self.parse_unary()?;
        if self.at(&TokenKind::Caret) {
            self.bump();
            self.skip_newlines_if_continuation();
            let rhs = self.parse_power()?;
            let span = lhs.span().merge(rhs.span());
            return Ok(Expr::Binary {
                op: BinaryOp::Pow,
                lhs: Box::new(lhs),
                rhs: Box::new(rhs),
                span,
            });
        }
        Ok(lhs)
    }

    fn parse_unary(&mut self) -> PResult<Expr> {
        // A prefix operator deepens the tree and recurses to do it, so it
        // costs a level like everything else. `not not not …` at 220 000 and
        // unary `-` at 600 000 aborted the process with a stack overflow and
        // nothing on stderr — the failure [G-NEST-LIMIT] was written after,
        // reached through the one operator position it did not count.
        self.enter_nested()?;
        let r = self.parse_unary_inner();
        self.depth -= 1;
        r
    }

    fn parse_unary_inner(&mut self) -> PResult<Expr> {
        let start = self.cur_span();
        match self.cur() {
            TokenKind::Minus => {
                self.bump();
                let expr = self.parse_unary()?;
                let span = start.merge(expr.span());
                Ok(Expr::Unary {
                    op: UnaryOp::Neg,
                    expr: Box::new(expr),
                    span,
                })
            }
            TokenKind::KwNot => {
                self.bump();
                let expr = self.parse_unary()?;
                let span = start.merge(expr.span());
                Ok(Expr::Unary {
                    op: UnaryOp::Not,
                    expr: Box::new(expr),
                    span,
                })
            }
            TokenKind::KwTry => {
                self.bump();
                let expr = self.parse_unary()?;
                let span = start.merge(expr.span());
                Ok(Expr::Try {
                    expr: Box::new(expr),
                    span,
                })
            }
            _ => self.parse_postfix(),
        }
    }

    fn parse_postfix(&mut self) -> PResult<Expr> {
        let mut expr = self.parse_primary()?;
        loop {
            // A postfix chain deepens the tree exactly as a folded binary
            // operator does — `(1).max(1).max(1)…` needs no brackets and
            // recurses nowhere, and still built a spine deep enough to
            // overflow every pass that walks it.
            if matches!(
                self.cur(),
                TokenKind::LParen | TokenKind::Dot | TokenKind::LBracket
            ) {
                self.enter_nested()?;
            }
            match self.cur() {
                TokenKind::LParen => {
                    self.bump();
                    let args = self.parse_call_args()?;
                    let rp = self.expect(&TokenKind::RParen)?;
                    let span = expr.span().merge(rp.span);
                    expr = match expr {
                        Expr::Field { recv, name, .. } => Expr::MethodCall {
                            recv,
                            name,
                            args,
                            span,
                        },
                        other => Expr::Call {
                            callee: Box::new(other),
                            args,
                            span,
                        },
                    };
                }
                TokenKind::LBracket => {
                    self.bump();
                    let index = self.parse_expr()?;
                    let rb = self.expect(&TokenKind::RBracket)?;
                    let span = expr.span().merge(rb.span);
                    expr = Expr::Index {
                        recv: Box::new(expr),
                        index: Box::new(index),
                        span,
                    };
                }
                TokenKind::Dot => {
                    self.bump();
                    // `pair.0` — a tuple's components are named by position,
                    // and the name is the digits themselves.
                    let name = if let TokenKind::Int(n) = self.cur().clone() {
                        let sp = self.cur_span();
                        self.bump();
                        Ident {
                            name: n.to_string(),
                            span: sp,
                        }
                    } else {
                        self.ident()?
                    };
                    let span = expr.span().merge(name.span);
                    expr = Expr::Field {
                        recv: Box::new(expr),
                        name,
                        span,
                    };
                }
                _ => break,
            }
        }
        Ok(expr)
    }

    fn parse_call_args(&mut self) -> PResult<Vec<Expr>> {
        let saved = self.no_struct_lit;
        self.no_struct_lit = false;
        let mut args = Vec::new();
        loop {
            if self.at(&TokenKind::RParen) {
                break;
            }
            match self.parse_expr() {
                Ok(e) => args.push(e),
                Err(()) => {
                    self.no_struct_lit = saved;
                    return Err(());
                }
            }
            if !self.eat(&TokenKind::Comma) {
                break;
            }
        }
        self.no_struct_lit = saved;
        Ok(args)
    }

    fn parse_primary(&mut self) -> PResult<Expr> {
        let start = self.cur_span();
        match self.cur().clone() {
            TokenKind::Int(v) => {
                self.bump();
                Ok(Expr::Int(v, start))
            }
            TokenKind::Float(v) => {
                self.bump();
                Ok(Expr::Float(v, start))
            }
            TokenKind::KwTrue => {
                self.bump();
                Ok(Expr::Bool(true, start))
            }
            TokenKind::KwFalse => {
                self.bump();
                Ok(Expr::Bool(false, start))
            }
            TokenKind::Str(parts) => {
                self.bump();
                let mut pieces = Vec::new();
                for part in parts {
                    match part {
                        StrPart::Text(t) => pieces.push(StrPiece::Text(t)),
                        StrPart::Interp { src, offset } => {
                            let (expr, mut diags) = parse_expr_src_at(&src, offset, self.depth + 1);
                            self.diags.append(&mut diags);
                            match expr {
                                Some(e) => pieces.push(StrPiece::Expr(e)),
                                None => {
                                    self.diags.push(Diagnostic::error(
                                        start,
                                        "invalid expression inside string interpolation",
                                    ));
                                }
                            }
                        }
                    }
                }
                Ok(Expr::Str(pieces, start))
            }
            // `region {` is a *contextual* keyword, like `parallel` and
            // `spawn` ([L-CONTEXTUAL]). Making it a reserved word broke a
            // program in this repository that used `region` as a field name,
            // which is exactly the compatibility the contextual rule exists
            // for.
            // In a condition, an iterator, or a `match` scrutinee, a
            // following `{` opens the *body* — which is exactly what
            // `no_struct_lit` marks, and why `if User{..}.ok` needs parens
            // ([G-STRUCT-AMBIG]). `region` is an expression form and was not
            // asking, so `for x in region { … }` read the loop's own brace as
            // a region body and `let region = [1, 2]` stopped being a usable
            // name ([L-CONTEXTUAL]).
            TokenKind::Ident(n) if n == "region" && !self.no_struct_lit && self.block_follows() => {
                self.bump();
                Ok(Expr::Region(self.parse_block()?))
            }
            TokenKind::Ident(name) => {
                self.bump();
                let ident = Ident { name, span: start };
                // Struct literal: `Name{ ... }` — only when allowed here.
                if !self.no_struct_lit && self.at(&TokenKind::LBrace) {
                    return self.parse_struct_lit_or_ident(ident);
                }
                Ok(Expr::Ident(ident))
            }
            TokenKind::LParen => {
                self.bump();
                if self.at(&TokenKind::RParen) {
                    let rp = self.bump();
                    return Ok(Expr::Tuple(Vec::new(), start.merge(rp.span)));
                }
                let saved = self.no_struct_lit;
                self.no_struct_lit = false;
                let first = self.parse_expr()?;
                if self.at(&TokenKind::Comma) {
                    let mut items = vec![first];
                    while self.eat(&TokenKind::Comma) {
                        if self.at(&TokenKind::RParen) {
                            break;
                        }
                        items.push(self.parse_expr()?);
                    }
                    let rp = self.expect(&TokenKind::RParen)?;
                    self.no_struct_lit = saved;
                    Ok(Expr::Tuple(items, start.merge(rp.span)))
                } else {
                    self.expect(&TokenKind::RParen)?;
                    self.no_struct_lit = saved;
                    Ok(first)
                }
            }
            TokenKind::LBracket => {
                self.bump();
                let saved = self.no_struct_lit;
                self.no_struct_lit = false;
                let mut items = Vec::new();
                loop {
                    if self.at(&TokenKind::RBracket) {
                        break;
                    }
                    items.push(self.parse_expr()?);
                    if !self.eat(&TokenKind::Comma) {
                        break;
                    }
                }
                let rb = self.expect(&TokenKind::RBracket)?;
                self.no_struct_lit = saved;
                Ok(Expr::List(items, start.merge(rb.span)))
            }
            TokenKind::KwIf => self.parse_if(),
            TokenKind::KwMatch => self.parse_match(),
            TokenKind::KwUnsafe => {
                self.bump();
                Ok(Expr::Unsafe(self.parse_block()?))
            }

            // `ref(v)` — Tier 2. `ref` is a keyword because it is also a type
            // prefix, so it cannot arrive here as an ordinary identifier the
            // way `release` and `alive` do.
            TokenKind::KwRef => {
                let kw = self.bump();
                self.expect(&TokenKind::LParen)?;
                let saved = self.no_struct_lit;
                self.no_struct_lit = false;
                let inner = self.parse_expr()?;
                self.no_struct_lit = saved;
                let rp = self.expect(&TokenKind::RParen)?;
                Ok(Expr::Call {
                    callee: Box::new(Expr::Ident(Ident {
                        name: "ref".to_string(),
                        span: kw.span,
                    })),
                    args: vec![inner],
                    span: kw.span.merge(rp.span),
                })
            }
            TokenKind::LBrace => self.parse_brace_expr(),
            TokenKind::Pipe => self.parse_closure(),
            other => {
                // Operators other languages spell differently are the ones a
                // newcomer types by reflex, and "expected an expression" says
                // nothing about which word to reach for instead.
                let hint: Option<String> = match other {
                    TokenKind::Bang => {
                        Some("negation is `not`, as in `not done`".to_string())
                    }
                    // A binary operator where an expression belongs, at the
                    // start of a line, is a wrapped expression written the way
                    // most languages allow and this one does not: a newline ends
                    // a statement unless the line *ends* in an operator.
                    //
                    // This is the common shape of that mistake — the previous
                    // line parsed cleanly as a complete statement, so nothing
                    // failed until the operator itself. The `expect` path
                    // catches the other shape, where the statement was still
                    // open.
                    _ if self.at_line_start() => binary_operator_name(&other).map(|op| {
                        format!(
                            "a newline ends a statement unless the line *ends* in an operator, so the previous line was complete. Move this `{op}` up to the end of it, or wrap the whole expression in parentheses"
                        )
                    }),
                    _ => None,
                };
                let mut d = Diagnostic::error(
                    start,
                    format!("expected an expression but found {}", other.describe()),
                );
                if let Some(h) = hint {
                    d = d.with_help(h);
                }
                self.diags.push(d);
                Err(())
            }
        }
    }

    /// Attempts `Name{ field: v, ... }`. If the braces turn out not to be a
    /// struct literal (e.g. a following block), rolls back and returns the
    /// plain identifier — this keeps `handled_by h { ... }` unambiguous.
    fn parse_struct_lit_or_ident(&mut self, name: Ident) -> PResult<Expr> {
        let save_i = self.i;
        let save_modes = self.modes.len();
        let save_diags = self.diags.len();
        // Lookahead after `{`: struct literal iff `ident :` follows.
        // (Empty `{}` counts as a block, so `handled_by h { }` parses.)
        self.bump(); // LBrace
        self.skip_newlines();
        let is_struct = match self.cur().clone() {
            TokenKind::Ident(_) => {
                // Peek one past the ident for a `:` (raw scan, skipping nothing).
                let mut j = self.i + 1;
                while matches!(self.toks[j].kind, TokenKind::Newline) {
                    j += 1;
                }
                matches!(self.toks[j].kind, TokenKind::Colon)
            }
            // `Point{ ..p }` — a struct update. Nothing else in the language
            // starts a brace with `..`.
            TokenKind::DotDot => true,
            _ => false,
        };
        // Roll back to before the `{` either way; parse_struct_lit re-consumes.
        self.i = save_i;
        self.modes.truncate(save_modes);
        self.diags.truncate(save_diags);
        if is_struct {
            self.parse_struct_lit(name)
        } else {
            Ok(Expr::Ident(name))
        }
    }

    fn parse_struct_lit(&mut self, name: Ident) -> PResult<Expr> {
        let start = name.span;
        self.expect(&TokenKind::LBrace)?;
        let mut fields = Vec::new();
        // `Point{ ..p, x: 1 }` — the fields not written come from `p`. An
        // accumulator otherwise restates every field at every step, which is
        // where a wrong one hides.
        let mut base: Option<Box<Expr>> = None;
        self.skip_newlines();
        if self.at(&TokenKind::DotDot) {
            self.bump();
            let saved = self.no_struct_lit;
            self.no_struct_lit = false;
            base = Some(Box::new(self.parse_expr()?));
            self.no_struct_lit = saved;
            self.eat(&TokenKind::Comma);
        }
        loop {
            self.skip_newlines();
            if self.at(&TokenKind::RBrace) {
                break;
            }
            let fname = self.ident()?;
            self.expect(&TokenKind::Colon)?;
            self.skip_newlines();
            let saved = self.no_struct_lit;
            self.no_struct_lit = false;
            let value = self.parse_expr()?;
            self.no_struct_lit = saved;
            fields.push((fname, value));
            let had_sep = self.eat(&TokenKind::Comma);
            let at_nl = matches!(self.toks[self.i].kind, TokenKind::Newline);
            if !had_sep && !at_nl {
                break;
            }
        }
        self.skip_newlines();
        let rb = self.expect(&TokenKind::RBrace)?;
        let path = Path {
            segments: vec![name.clone()],
            span: name.span,
        };
        Ok(Expr::StructLit {
            name: path,
            base,
            fields,
            span: start.merge(rb.span),
        })
    }

    fn parse_if(&mut self) -> PResult<Expr> {
        let start = self.cur_span();
        self.expect(&TokenKind::KwIf)?;
        let cond = self.parse_cond_expr()?;
        let then_block = self.parse_block()?;
        let mut span = start.merge(then_block.span);

        // `else` may appear after a newline.
        let save = self.i;
        self.skip_newlines();
        let else_branch = if self.at(&TokenKind::KwElse) {
            self.bump();
            if self.at(&TokenKind::KwIf) {
                let nested = self.parse_if()?;
                span = span.merge(nested.span());
                Some(Box::new(ElseBranch::ElseIf(nested)))
            } else {
                let block = self.parse_block()?;
                span = span.merge(block.span);
                Some(Box::new(ElseBranch::Else(block)))
            }
        } else {
            self.i = save;
            None
        };

        Ok(Expr::If {
            cond: Box::new(cond),
            then_block,
            else_branch,
            span,
        })
    }

    fn parse_match(&mut self) -> PResult<Expr> {
        let start = self.cur_span();
        self.expect(&TokenKind::KwMatch)?;
        let scrutinee = self.parse_cond_expr()?;
        self.expect(&TokenKind::LBrace)?;
        let mut arms = Vec::new();
        loop {
            self.skip_newlines();
            if self.at(&TokenKind::RBrace) {
                break;
            }
            let mut pat = self.parse_pattern()?;
            // `A | B -> ...`. Only at arm level: a `|` inside a variant's
            // fields would be ambiguous with a union type.
            if self.at(&TokenKind::Pipe) {
                let mut alts = vec![pat];
                while self.eat(&TokenKind::Pipe) {
                    alts.push(self.parse_pattern()?);
                }
                let span = alts[0].span().merge(alts[alts.len() - 1].span());
                pat = Pattern::Or(alts, span);
            }
            let guard = if self.eat(&TokenKind::KwIf) {
                Some(self.parse_cond_expr()?)
            } else {
                None
            };
            self.expect(&TokenKind::Arrow)?;
            self.skip_newlines();
            // Control-flow statements are allowed directly as arm bodies:
            // `_ -> break` desugars to `_ -> { break }`.
            let empty_brace = self.at(&TokenKind::LBrace) && self.brace_is_empty();
            let body = match self.cur() {
                // `-> {}` is an arm that does nothing, which is what an
                // empty *block* means. Everywhere else `{}` is the empty map
                // ([G-MAP-BLOCK-AMBIG]); here the arm's own braces win,
                // because "do nothing" is what somebody writing it meant. A
                // non-empty `{k: v}` is still a map.
                TokenKind::LBrace if empty_brace => Expr::Block(self.parse_block()?),
                TokenKind::KwBreak | TokenKind::KwContinue | TokenKind::KwReturn => {
                    let stmt = self.parse_stmt()?;
                    let span = crate::parser::stmt_span_of(&stmt);
                    Expr::Block(Block {
                        stmts: vec![stmt],
                        span,
                    })
                }
                _ => {
                    let e = self.parse_expr()?;
                    // Assignments too: `Some(i) -> xs[i] = v`.
                    let op = match self.cur() {
                        TokenKind::Eq => Some(AssignOp::Assign),
                        TokenKind::PlusEq => Some(AssignOp::Add),
                        TokenKind::MinusEq => Some(AssignOp::Sub),
                        TokenKind::StarEq => Some(AssignOp::Mul),
                        TokenKind::SlashEq => Some(AssignOp::Div),
                        _ => None,
                    };
                    match op {
                        Some(op) => {
                            self.bump();
                            self.skip_newlines_if_continuation();
                            let value = self.parse_expr()?;
                            let span = e.span().merge(value.span());
                            if !is_place_expr(&e) {
                                self.diags.push(
                                    Diagnostic::error(e.span(), "invalid assignment target")
                                        .with_help("only variables, fields, and index expressions can be assigned to"),
                                );
                            }
                            Expr::Block(Block {
                                stmts: vec![Stmt::Assign {
                                    target: e,
                                    op,
                                    value,
                                    span,
                                }],
                                span,
                            })
                        }
                        None => e,
                    }
                }
            };
            let span = pat.span().merge(body.span());
            arms.push(Arm {
                pattern: pat,
                guard,
                body,
                span,
            });
        }
        let rb = self.expect(&TokenKind::RBrace)?;
        Ok(Expr::Match {
            scrutinee: Box::new(scrutinee),
            arms,
            span: start.merge(rb.span),
        })
    }

    /// `{` opens either a block or a map literal. Telling them apart by
    /// lookahead would mean re-implementing expression parsing to find the
    /// `:`, so this reads it as a map and rewinds if that does not fit. The
    /// two never both succeed: a map entry needs a `:` where a statement
    /// cannot have one.
    /// Whether the `{` at the cursor closes immediately, newlines aside.
    fn brace_is_empty(&self) -> bool {
        let mut j = self.i + 1;
        while matches!(self.toks.get(j).map(|t| &t.kind), Some(TokenKind::Newline)) {
            j += 1;
        }
        matches!(self.toks.get(j).map(|t| &t.kind), Some(TokenKind::RBrace))
    }

    fn parse_brace_expr(&mut self) -> PResult<Expr> {
        let mark = (self.i, self.modes.clone(), self.diags.len());
        if !self.not_a_map.contains(&mark.0) {
            self.map_committed = false;
            if let Some(e) = self.try_parse_map() {
                return Ok(e);
            }
            // A `key :` was read, so this *is* a map literal and the error is
            // inside it. Rewinding to the block reading instead reported
            // "expected end of line after statement" at the colon — a parse
            // error about the wrong thing, which is what
            // [G-STRUCT-NONEMPTY] records as fixed for struct literals.
            if self.map_committed {
                return Err(());
            }
            self.not_a_map.insert(mark.0);
        }
        self.i = mark.0;
        self.modes = mark.1;
        self.diags.truncate(mark.2);
        Ok(Expr::Block(self.parse_block()?))
    }

    /// The map reading of `{...}`, or None if it is not one. Rewinding is the
    /// caller's job, so this may leave the parser mid-brace.
    fn try_parse_map(&mut self) -> Option<Expr> {
        let start = self.cur_span();
        self.expect(&TokenKind::LBrace).ok()?;
        self.skip_newlines();
        let mut entries: Vec<(Expr, Expr)> = Vec::new();
        if self.at(&TokenKind::RBrace) {
            let end = self.cur_span();
            self.bump();
            return Some(Expr::Map(entries, start.merge(end)));
        }
        loop {
            let key = self.parse_expr().ok()?;
            self.skip_newlines();
            self.expect(&TokenKind::Colon).ok()?;
            self.map_committed = true;
            self.skip_newlines();
            let val = self.parse_expr().ok()?;
            entries.push((key, val));
            self.skip_newlines();
            if self.at(&TokenKind::Comma) {
                self.bump();
                self.skip_newlines();
                if self.at(&TokenKind::RBrace) {
                    let end = self.cur_span();
                    self.bump();
                    return Some(Expr::Map(entries, start.merge(end)));
                }
                continue;
            }
            if self.at(&TokenKind::RBrace) {
                let end = self.cur_span();
                self.bump();
                return Some(Expr::Map(entries, start.merge(end)));
            }
            return None;
        }
    }

    fn parse_closure(&mut self) -> PResult<Expr> {
        let start = self.cur_span();
        self.expect(&TokenKind::Pipe)?;
        let mut params = Vec::new();
        if !self.at(&TokenKind::Pipe) {
            loop {
                params.push(self.ident()?);
                if !self.eat(&TokenKind::Comma) {
                    break;
                }
            }
        }
        self.expect(&TokenKind::Pipe)?;
        let body = if self.at(&TokenKind::LBrace) {
            Expr::Block(self.parse_block()?)
        } else {
            self.parse_expr()?
        };
        let span = start.merge(body.span());
        Ok(Expr::Closure {
            params,
            body: Box::new(body),
            span,
        })
    }

    // ------------------------------------------------------------------
    // Patterns
    // ------------------------------------------------------------------

    fn parse_pattern(&mut self) -> PResult<Pattern> {
        // Patterns nest like everything else, and everything that walks one
        // does it by recursion. A 5000-deep constructor pattern checked in
        // 15ms and then took the native backend 114 seconds to compile, its
        // build time growing roughly cubically — the one deep shape the cap
        // did not count ([G-NEST-LIMIT]).
        self.enter_nested()?;
        let r = self.parse_pattern_inner();
        self.depth -= 1;
        r
    }

    fn parse_pattern_inner(&mut self) -> PResult<Pattern> {
        let start = self.cur_span();
        match self.cur().clone() {
            TokenKind::Underscore => {
                self.bump();
                Ok(Pattern::Wildcard(start))
            }
            TokenKind::Int(v) => {
                self.bump();
                Ok(Pattern::Int(v, start))
            }
            TokenKind::Minus => {
                self.bump();
                let span = self.cur_span();
                if let TokenKind::Int(v) = self.cur().clone() {
                    self.bump();
                    Ok(Pattern::Int(-v, start.merge(span)))
                } else {
                    self.error_here("expected an integer after `-` in pattern");
                    Err(())
                }
            }
            TokenKind::KwTrue => {
                self.bump();
                Ok(Pattern::Bool(true, start))
            }
            TokenKind::KwFalse => {
                self.bump();
                Ok(Pattern::Bool(false, start))
            }
            TokenKind::Str(parts) => {
                self.bump();
                match parts.as_slice() {
                    [StrPart::Text(t)] => Ok(Pattern::Str(t.clone(), start)),
                    [] => Ok(Pattern::Str(String::new(), start)),
                    _ => {
                        self.diags.push(Diagnostic::error(
                            start,
                            "string patterns cannot contain interpolation",
                        ));
                        Err(())
                    }
                }
            }
            TokenKind::Ident(_) => {
                let path = self.parse_path()?;
                if self.at(&TokenKind::LParen) {
                    self.bump();
                    let mut fields = Vec::new();
                    loop {
                        if self.at(&TokenKind::RParen) {
                            break;
                        }
                        fields.push(self.parse_pattern()?);
                        if !self.eat(&TokenKind::Comma) {
                            break;
                        }
                    }
                    let rp = self.expect(&TokenKind::RParen)?;
                    Ok(Pattern::Variant {
                        span: path.span.merge(rp.span),
                        path,
                        fields,
                    })
                } else if path.segments.len() > 1 {
                    Ok(Pattern::Variant {
                        span: path.span,
                        path,
                        fields: Vec::new(),
                    })
                } else {
                    Ok(Pattern::Bind(path.segments.into_iter().next().unwrap()))
                }
            }
            TokenKind::LParen => {
                self.bump();
                let mut items = Vec::new();
                loop {
                    if self.at(&TokenKind::RParen) {
                        break;
                    }
                    items.push(self.parse_pattern()?);
                    if !self.eat(&TokenKind::Comma) {
                        break;
                    }
                }
                let rp = self.expect(&TokenKind::RParen)?;
                Ok(Pattern::Tuple(items, start.merge(rp.span)))
            }
            other => {
                self.diags.push(Diagnostic::error(
                    start,
                    format!("expected a pattern but found {}", other.describe()),
                ));
                Err(())
            }
        }
    }
}

pub(crate) fn stmt_span_of(s: &Stmt) -> Span {
    match s {
        Stmt::Let { span, .. }
        | Stmt::LetTuple { span, .. }
        | Stmt::Assign { span, .. }
        | Stmt::Return(_, span)
        | Stmt::Break(span)
        | Stmt::Continue(span)
        | Stmt::While { span, .. }
        | Stmt::For { span, .. }
        | Stmt::With { span, .. }
        | Stmt::Parallel { span, .. }
        | Stmt::Spawn { span, .. } => *span,
        Stmt::Expr(e) => e.span(),
    }
}

fn is_place_expr(e: &Expr) -> bool {
    matches!(e, Expr::Ident(_) | Expr::Field { .. } | Expr::Index { .. })
}

/// The spelling of a binary operator token, for diagnostics.
///
/// Kept beside the parser rather than on `TokenKind`, because it answers a
/// narrower question than `describe()` does: not "what is this token" but
/// "is this a binary operator, and what does it look like".
fn binary_operator_name(kind: &TokenKind) -> Option<&'static str> {
    match kind {
        TokenKind::KwAnd => Some("and"),
        TokenKind::KwOr => Some("or"),
        TokenKind::Plus => Some("+"),
        TokenKind::Star => Some("*"),
        TokenKind::Slash => Some("/"),
        TokenKind::Percent => Some("%"),
        TokenKind::EqEq => Some("=="),
        TokenKind::NotEq => Some("!="),
        TokenKind::Lt => Some("<"),
        TokenKind::Gt => Some(">"),
        TokenKind::LtEq => Some("<="),
        TokenKind::GtEq => Some(">="),
        TokenKind::PipeGt => Some("|>"),
        // `-` is deliberately absent: it is also unary, so a line starting with
        // one is at least as likely to be a negation as a continuation.
        _ => None,
    }
}
