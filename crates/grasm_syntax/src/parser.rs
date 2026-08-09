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

/// Parse a single expression (used by the REPL and interpolation).
pub fn parse_expr_src(src: &str, offset: u32) -> (Option<Expr>, Vec<Diagnostic>) {
    let mut tokens = lex(src);
    for t in &mut tokens {
        t.span = Span::new(t.span.start + offset, t.span.end + offset);
    }
    let mut p = Parser::new(tokens);
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
            .with_help(
                "close the interpolation before this, or join the parts with an operator",
            ),
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
/// frames). 64 levels is far beyond any reasonable program. Applied to both
/// expressions and blocks.
const MAX_NESTING: u32 = 64;

struct Parser {
    toks: Vec<Token>,
    i: usize,
    modes: Vec<NlMode>,
    diags: Vec<Diagnostic>,
    /// When true, `Ident {` is NOT a struct literal (condition positions).
    no_struct_lit: bool,
    depth: u32,
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
        }
    }

    fn enter_nested(&mut self) -> PResult<()> {
        if self.depth >= MAX_NESTING {
            self.error_here(format!(
                "code is nested too deeply (more than {MAX_NESTING} levels)"
            ));
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
            TokenKind::RParen | TokenKind::RBracket | TokenKind::RBrace => {
                if self.modes.len() > 1 {
                    self.modes.pop();
                }
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
            self.diags.push(Diagnostic::error(
                span,
                format!("expected {} but found {}", kind.describe(), found),
            ));
            Err(())
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
        self.diags.push(Diagnostic::error(span, msg));
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
                    if n == "bench"
                        && matches!(&self.toks[self.i + 1].kind, TokenKind::Str(_)) =>
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
                Ok(d) => module.decls.push(d),
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
            _ => {
                let found = self.cur().describe();
                self.error_here(format!(
                    "expected a declaration (`fn`, `let`, `type`, `effect`, `test`, `bench`, `import`) but found {found}"
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
        let start = self.cur_span();
        self.expect(&TokenKind::KwFn)?;
        let name = self.ident()?;
        let generics = self.parse_generics()?;
        let params = self.parse_params()?;
        let (ret, err, effects) = self.parse_fn_tail()?;
        let body = self.parse_block()?;
        let span = start.merge(body.span);
        Ok(FnDecl {
            is_pub,
            name,
            generics,
            params,
            ret,
            err,
            effects,
            body,
            span,
        })
    }

    fn parse_generics(&mut self) -> PResult<Vec<Ident>> {
        let mut generics = Vec::new();
        if self.eat(&TokenKind::LBracket) {
            loop {
                if self.at(&TokenKind::RBracket) {
                    break;
                }
                generics.push(self.ident()?);
                if !self.eat(&TokenKind::Comma) {
                    break;
                }
            }
            self.expect(&TokenKind::RBracket)?;
        }
        Ok(generics)
    }

    fn parse_params(&mut self) -> PResult<Vec<Param>> {
        self.expect(&TokenKind::LParen)?;
        let mut params = Vec::new();
        loop {
            if self.at(&TokenKind::RParen) {
                break;
            }
            let start = self.cur_span();
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

    /// `-> Ret ! Err / {effects}` — every piece optional.
    #[allow(clippy::type_complexity)]
    fn parse_fn_tail(&mut self) -> PResult<(Option<TypeExpr>, Option<TypeExpr>, Option<Vec<Path>>)> {
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
                paths.push(self.parse_path()?);
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
        let generics = self.parse_generics()?;
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
        Ok(TypeExpr::Fallible(Box::new(inner), Box::new(err), span))
    }

    fn parse_type(&mut self) -> PResult<TypeExpr> {
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
                        effects.push(self.parse_path()?);
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
                        names.push(self.ident()?);
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
                        names.push(self.ident()?);
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
                    self.ident()?
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
                            Diagnostic::error(expr.span(), "invalid assignment target")
                                .with_help("only variables, fields, and index expressions can be assigned to"),
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
        let r = self.parse_or();
        self.depth -= 1;
        r
    }

    fn parse_or(&mut self) -> PResult<Expr> {
        let mut lhs = self.parse_and()?;
        while self.at(&TokenKind::KwOr) {
            self.bump();
            self.skip_newlines_if_continuation();
            let rhs = self.parse_and()?;
            let span = lhs.span().merge(rhs.span());
            lhs = Expr::Binary {
                op: BinaryOp::Or,
                lhs: Box::new(lhs),
                rhs: Box::new(rhs),
                span,
            };
        }
        Ok(lhs)
    }

    fn parse_and(&mut self) -> PResult<Expr> {
        let mut lhs = self.parse_comparison()?;
        while self.at(&TokenKind::KwAnd) {
            self.bump();
            self.skip_newlines_if_continuation();
            let rhs = self.parse_comparison()?;
            let span = lhs.span().merge(rhs.span());
            lhs = Expr::Binary {
                op: BinaryOp::And,
                lhs: Box::new(lhs),
                rhs: Box::new(rhs),
                span,
            };
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
            lhs = Expr::Binary {
                op,
                lhs: Box::new(lhs),
                rhs: Box::new(rhs),
                span,
            };
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

    fn parse_coalesce(&mut self) -> PResult<Expr> {
        let mut lhs = self.parse_pipeline()?;
        while self.at(&TokenKind::QuestionQuestion) {
            self.bump();
            self.skip_newlines_if_continuation();
            let rhs = self.parse_pipeline()?;
            let span = lhs.span().merge(rhs.span());
            lhs = Expr::Binary {
                op: BinaryOp::Coalesce,
                lhs: Box::new(lhs),
                rhs: Box::new(rhs),
                span,
            };
        }
        Ok(lhs)
    }

    /// `x |> f(a)` desugars to `f(x, a)`; `x |> obj.m(a)` to `obj.m(x, a)`.
    fn parse_pipeline(&mut self) -> PResult<Expr> {
        let mut lhs = self.parse_additive()?;
        while self.at(&TokenKind::PipeGt) {
            self.bump();
            self.skip_newlines_if_continuation();
            let rhs = self.parse_additive()?;
            let span = lhs.span().merge(rhs.span());
            lhs = match rhs {
                Expr::Call { callee, mut args, .. } => {
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
                        Diagnostic::error(other.span(), "right side of `|>` must be a function or call")
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
            lhs = Expr::Binary {
                op,
                lhs: Box::new(lhs),
                rhs: Box::new(rhs),
                span,
            };
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
            lhs = Expr::Binary {
                op,
                lhs: Box::new(lhs),
                rhs: Box::new(rhs),
                span,
            };
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
                            let (expr, mut diags) = parse_expr_src(&src, offset);
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
            TokenKind::Ident(name) => {
                self.bump();
                let ident = Ident {
                    name,
                    span: start,
                };
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
            TokenKind::LBrace => self.parse_brace_expr(),
            TokenKind::Pipe => self.parse_closure(),
            other => {
                // Operators other languages spell differently are the ones a
                // newcomer types by reflex, and "expected an expression" says
                // nothing about which word to reach for instead.
                let hint = match other {
                    TokenKind::Bang => Some("negation is `not`, as in `not done`"),
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
                TokenKind::LBrace if empty_brace => {
                    Expr::Block(self.parse_block()?)
                }
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
        if let Some(e) = self.try_parse_map() {
            return Ok(e);
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
    matches!(
        e,
        Expr::Ident(_) | Expr::Field { .. } | Expr::Index { .. }
    )
}
