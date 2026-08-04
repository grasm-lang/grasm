//! Grasm-Core AST. Every node carries a span for diagnostics.

use crate::span::Span;

#[derive(Debug, Clone, PartialEq)]
pub struct Ident {
    pub name: String,
    pub span: Span,
}

/// Dotted path: `io.net`, `math`, `Shape.Circle`.
#[derive(Debug, Clone, PartialEq)]
pub struct Path {
    pub segments: Vec<Ident>,
    pub span: Span,
}

impl Path {
    pub fn dotted(&self) -> String {
        self.segments
            .iter()
            .map(|s| s.name.as_str())
            .collect::<Vec<_>>()
            .join(".")
    }
}

// ---------------------------------------------------------------------------
// Module & declarations
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq, Default)]
pub struct Module {
    pub decls: Vec<Decl>,
    /// `///` blocks keyed by the span start of the declaration they precede.
    /// Kept beside the tree rather than on every node so adding documentation
    /// never changes how anything compiles.
    pub docs: std::collections::BTreeMap<u32, String>,
}

impl Module {
    /// Documentation attached to a declaration, if any.
    pub fn doc_for(&self, decl: &Decl) -> Option<&str> {
        self.docs.get(&decl_span(decl).start).map(String::as_str)
    }
}

/// Where a declaration begins — the key documentation is filed under.
pub fn decl_span(d: &Decl) -> Span {
    match d {
        Decl::Fn(f) => f.span,
        Decl::Type(t) => t.span,
        Decl::Effect(e) => e.span,
        Decl::Test(t) => t.span,
        Decl::Bench(b) => b.span,
        Decl::Import(i) => i.span,
    }
}

#[derive(Debug, Clone, PartialEq)]
pub enum Decl {
    Fn(FnDecl),
    Type(TypeDecl),
    Effect(EffectDecl),
    Test(TestDecl),
    Bench(BenchDecl),
    Import(ImportDecl),
}

#[derive(Debug, Clone, PartialEq)]
pub struct FnDecl {
    pub is_pub: bool,
    pub name: Ident,
    pub generics: Vec<Ident>,
    pub params: Vec<Param>,
    pub ret: Option<TypeExpr>,
    /// `! E` — error type of the `T ! E` union.
    pub err: Option<TypeExpr>,
    /// `/ {io.net, async}` — declared effect row.
    pub effects: Option<Vec<Path>>,
    pub body: Block,
    pub span: Span,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ParamMode {
    /// Default: caller keeps the value, callee reads it (pass-by-immutable-ref).
    Let,
    /// Exclusive in-place mutation.
    Inout,
    /// Ownership transfer (move).
    Own,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Param {
    pub mode: ParamMode,
    pub name: Ident,
    pub ty: TypeExpr,
    pub span: Span,
}

#[derive(Debug, Clone, PartialEq)]
pub struct TypeDecl {
    pub is_pub: bool,
    pub name: Ident,
    pub generics: Vec<Ident>,
    pub body: TypeBody,
    pub span: Span,
}

#[derive(Debug, Clone, PartialEq)]
pub enum TypeBody {
    /// `type User = { id: u64, name: str }`
    Struct(Vec<FieldDef>),
    /// `type Shape = | Circle(r: f64) | Rect(w: f64, h: f64)`
    Sum(Vec<VariantDef>),
    /// `type Port = i32`
    Alias(TypeExpr),
}

#[derive(Debug, Clone, PartialEq)]
pub struct FieldDef {
    pub name: Ident,
    pub ty: TypeExpr,
    pub span: Span,
}

#[derive(Debug, Clone, PartialEq)]
pub struct VariantDef {
    pub name: Ident,
    pub fields: Vec<FieldDef>,
    pub span: Span,
}

/// `effect Clock { fn now() -> Instant }`
#[derive(Debug, Clone, PartialEq)]
pub struct EffectDecl {
    pub name: Ident,
    pub ops: Vec<FnSig>,
    pub span: Span,
}

#[derive(Debug, Clone, PartialEq)]
pub struct FnSig {
    pub name: Ident,
    pub params: Vec<Param>,
    pub ret: Option<TypeExpr>,
    pub err: Option<TypeExpr>,
    pub span: Span,
}

/// `test "name" { ... }`, or `test "name" (x: i64) { ... }` — a *property*
/// test, whose parameters are filled with generated values and whose body must
/// hold for all of them. One declaration form covers unit, property, and fuzz
/// testing; the parameter list is the only difference.
#[derive(Debug, Clone, PartialEq)]
pub struct TestDecl {
    pub name: String,
    /// Empty for a unit test; non-empty makes it a property test.
    pub params: Vec<Param>,
    pub body: Block,
    pub span: Span,
}

/// `bench "name" { ... }` — timed repeatedly, reported in ns/op.
#[derive(Debug, Clone, PartialEq)]
pub struct BenchDecl {
    pub name: String,
    pub body: Block,
    pub span: Span,
}

#[derive(Debug, Clone, PartialEq)]
pub struct ImportDecl {
    pub path: Path,
    pub span: Span,
}

// ---------------------------------------------------------------------------
// Types (syntax level)
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq)]
pub enum TypeExpr {
    /// `i64`, `str`, `Option[T]`, `map.HashMap[K, V]`
    Name {
        path: Path,
        args: Vec<TypeExpr>,
        span: Span,
    },
    /// `[T]`
    List(Box<TypeExpr>, Span),
    /// `(A, B)`; empty = unit `()`
    Tuple(Vec<TypeExpr>, Span),
    /// `(E1 | E2)` — used for error unions.
    Union(Vec<TypeExpr>, Span),
}

impl TypeExpr {
    pub fn span(&self) -> Span {
        match self {
            TypeExpr::Name { span, .. }
            | TypeExpr::List(_, span)
            | TypeExpr::Tuple(_, span)
            | TypeExpr::Union(_, span) => *span,
        }
    }
}

// ---------------------------------------------------------------------------
// Statements & blocks
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq)]
pub struct Block {
    pub stmts: Vec<Stmt>,
    pub span: Span,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AssignOp {
    Assign,
    Add,
    Sub,
    Mul,
    Div,
}

#[derive(Debug, Clone, PartialEq)]
pub enum Stmt {
    Let {
        mutable: bool,
        name: Ident,
        ty: Option<TypeExpr>,
        init: Expr,
        span: Span,
    },
    Assign {
        target: Expr,
        op: AssignOp,
        value: Expr,
        span: Span,
    },
    Expr(Expr),
    Return(Option<Expr>, Span),
    Break(Span),
    Continue(Span),
    While {
        cond: Expr,
        body: Block,
        span: Span,
    },
    For {
        var: Ident,
        iter: Expr,
        body: Block,
        span: Span,
    },
    /// `parallel { spawn { .. } spawn { .. } }` — a structured concurrency
    /// scope. Every task spawned inside completes before the scope exits.
    Parallel { body: Block, span: Span },
    /// `spawn { ... }` — schedule a task in the enclosing `parallel` scope.
    Spawn { body: Block, span: Span },
    /// `with Clock handled_by ClockHandler{ now: || 0 } { ... }`
    With {
        effect: Path,
        handler: Expr,
        body: Block,
        span: Span,
    },
}

// ---------------------------------------------------------------------------
// Expressions
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UnaryOp {
    Neg,
    Not,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BinaryOp {
    Add,
    Sub,
    Mul,
    Div,
    Rem,
    Pow,
    Eq,
    NotEq,
    Lt,
    LtEq,
    Gt,
    GtEq,
    And,
    Or,
    /// `??` — none-coalescing.
    Coalesce,
}

/// One piece of an interpolated string after parsing.
#[derive(Debug, Clone, PartialEq)]
pub enum StrPiece {
    Text(String),
    Expr(Expr),
}

#[derive(Debug, Clone, PartialEq)]
pub enum Expr {
    Int(i64, Span),
    Float(f64, Span),
    Bool(bool, Span),
    Str(Vec<StrPiece>, Span),
    Ident(Ident),
    List(Vec<Expr>, Span),
    Tuple(Vec<Expr>, Span),
    /// `User{ id: 1, name: "x" }`
    StructLit {
        name: Path,
        fields: Vec<(Ident, Expr)>,
        span: Span,
    },
    Call {
        callee: Box<Expr>,
        args: Vec<Expr>,
        span: Span,
    },
    /// `recv.name(args)` — UFCS-resolved later.
    MethodCall {
        recv: Box<Expr>,
        name: Ident,
        args: Vec<Expr>,
        span: Span,
    },
    Field {
        recv: Box<Expr>,
        name: Ident,
        span: Span,
    },
    Index {
        recv: Box<Expr>,
        index: Box<Expr>,
        span: Span,
    },
    Unary {
        op: UnaryOp,
        expr: Box<Expr>,
        span: Span,
    },
    Binary {
        op: BinaryOp,
        lhs: Box<Expr>,
        rhs: Box<Expr>,
        span: Span,
    },
    Range {
        start: Box<Expr>,
        end: Box<Expr>,
        inclusive: bool,
        span: Span,
    },
    If {
        cond: Box<Expr>,
        then_block: Block,
        else_branch: Option<Box<ElseBranch>>,
        span: Span,
    },
    Match {
        scrutinee: Box<Expr>,
        arms: Vec<Arm>,
        span: Span,
    },
    Block(Block),
    /// `try expr` — propagate the error branch.
    Try {
        expr: Box<Expr>,
        span: Span,
    },
    /// `|x, y| expr` or `|x| { ... }`
    Closure {
        params: Vec<Ident>,
        body: Box<Expr>,
        span: Span,
    },
}

#[derive(Debug, Clone, PartialEq)]
pub enum ElseBranch {
    Else(Block),
    ElseIf(Expr), // always Expr::If
}

#[derive(Debug, Clone, PartialEq)]
pub struct Arm {
    pub pattern: Pattern,
    pub guard: Option<Expr>,
    pub body: Expr,
    pub span: Span,
}

#[derive(Debug, Clone, PartialEq)]
pub enum Pattern {
    Wildcard(Span),
    Int(i64, Span),
    Bool(bool, Span),
    Str(String, Span),
    /// Binds a name — or a nullary variant, disambiguated during typecheck.
    Bind(Ident),
    /// `Circle(r)` / `Shape.Circle(r)` / `Some(x)`
    Variant {
        path: Path,
        fields: Vec<Pattern>,
        span: Span,
    },
    Tuple(Vec<Pattern>, Span),
}

impl Expr {
    pub fn span(&self) -> Span {
        match self {
            Expr::Int(_, s)
            | Expr::Float(_, s)
            | Expr::Bool(_, s)
            | Expr::Str(_, s)
            | Expr::List(_, s)
            | Expr::Tuple(_, s) => *s,
            Expr::Ident(id) => id.span,
            Expr::StructLit { span, .. }
            | Expr::Call { span, .. }
            | Expr::MethodCall { span, .. }
            | Expr::Field { span, .. }
            | Expr::Index { span, .. }
            | Expr::Unary { span, .. }
            | Expr::Binary { span, .. }
            | Expr::Range { span, .. }
            | Expr::If { span, .. }
            | Expr::Match { span, .. }
            | Expr::Try { span, .. }
            | Expr::Closure { span, .. } => *span,
            Expr::Block(b) => b.span,
        }
    }
}

impl Pattern {
    pub fn span(&self) -> Span {
        match self {
            Pattern::Wildcard(s)
            | Pattern::Int(_, s)
            | Pattern::Bool(_, s)
            | Pattern::Str(_, s) => *s,
            Pattern::Bind(id) => id.span,
            Pattern::Variant { span, .. } => *span,
            Pattern::Tuple(_, s) => *s,
        }
    }
}
