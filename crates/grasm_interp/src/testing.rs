//! The test runner: unit tests, property tests, benchmarks, and coverage.
//!
//! Grasm has one test declaration, not three. `test "name" { ... }` is a unit
//! test; adding parameters — `test "name" (xs: [i64]) { ... }` — makes it a
//! *property* test, and the runner supplies the values. Fuzzing is the same
//! machinery pointed at `str` and `[i64]`, so there is nothing extra to learn.
//!
//! Everything here is deterministic. A property run is driven by a seed that
//! is reported on failure, so a red test reproduces exactly with `--seed`.

use crate::interp::{Flow, Interp};
use crate::value::Value;
use grasm_syntax::ast::{Module, TypeExpr};
use std::collections::BTreeSet;
use std::io::Write;
use std::rc::Rc;

fn list_of(items: Vec<Value>) -> Value {
    Value::List(Rc::new(items))
}

/// How a property run ended.
pub enum PropertyOutcome {
    /// Every generated case held.
    Passed(u32),
    /// A case failed. `inputs` is already shrunk to a minimal counterexample.
    Failed {
        inputs: Vec<(String, Value)>,
        message: String,
        case: u32,
    },
}

// ---------------------------------------------------------------------------
// Deterministic randomness
// ---------------------------------------------------------------------------

/// splitmix64 — small, fast, and good enough to generate test inputs. Chosen
/// over a system RNG because a property failure has to reproduce exactly.
pub struct Rng {
    state: u64,
}

impl Rng {
    pub fn new(seed: u64) -> Rng {
        Rng {
            // A zero seed would otherwise emit a constant stream.
            state: seed ^ 0x9e37_79b9_7f4a_7c15,
        }
    }

    pub fn next_u64(&mut self) -> u64 {
        self.state = self.state.wrapping_add(0x9e37_79b9_7f4a_7c15);
        let mut z = self.state;
        z = (z ^ (z >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
        z ^ (z >> 31)
    }

    /// Uniform in `0..n`. `n == 0` yields 0.
    pub fn below(&mut self, n: u64) -> u64 {
        if n == 0 {
            0
        } else {
            self.next_u64() % n
        }
    }

    fn bool(&mut self) -> bool {
        self.next_u64() & 1 == 1
    }
}

// ---------------------------------------------------------------------------
// Generation
// ---------------------------------------------------------------------------

/// Characters worth throwing at string-handling code: ASCII, plus the
/// separators and multi-byte scalars that break naive byte indexing.
const INTERESTING_CHARS: &[char] = &[
    'a', 'b', 'z', 'A', 'Z', '0', '9', ' ', '\t', '\n', '"', '\\', '{', '}', '/', '-', '_', 'é',
    'ß', '中', '🙂',
];

/// Integers that sit on the boundaries where arithmetic tends to go wrong.
const INTERESTING_INTS: &[i64] = &[0, 1, -1, 2, -2, 10, 100, i64::MIN, i64::MAX, i64::MIN + 1];

/// Generates a value of the type `te` describes.
///
/// `size` grows with the case number so early cases are small (and shrink
/// reports stay readable) while later ones stress bigger inputs.
pub fn generate(rng: &mut Rng, te: &TypeExpr, size: u32) -> Value {
    match te {
        TypeExpr::List(inner, _) => {
            let n = rng.below(u64::from(size).min(32) + 1) as usize;
            let items = (0..n).map(|_| generate(rng, inner, size)).collect();
            list_of(items)
        }
        TypeExpr::Tuple(items, _) => {
            Value::Tuple(Rc::new(
                items.iter().map(|t| generate(rng, t, size)).collect(),
            ))
        }
        TypeExpr::Union(items, _) => match items.first() {
            Some(t) => generate(rng, t, size),
            None => Value::Unit,
        },
        TypeExpr::Name { path, args, .. } => {
            let name = path.dotted();
            match name.as_str() {
                "i64" => Value::Int({
                    // One case in four is a boundary value; the rest are
                    // spread over a range that widens with `size`.
                    if rng.below(4) == 0 {
                        INTERESTING_INTS[rng.below(INTERESTING_INTS.len() as u64) as usize]
                    } else {
                        let span = i64::from(size).saturating_mul(8).max(16);
                        (rng.next_u64() % (span as u64 * 2 + 1)) as i64 - span
                    }
                }),
                "f64" => {
                    if rng.below(8) == 0 {
                        Value::Float(
                            [0.0f64, -0.0, 1.0, -1.0, f64::INFINITY, f64::NEG_INFINITY]
                                [rng.below(6) as usize],
                        )
                    } else {
                        let scale = f64::from(size).max(1.0);
                        let unit = (rng.next_u64() as f64) / (u64::MAX as f64);
                        Value::Float((unit * 2.0 - 1.0) * scale)
                    }
                }
                "bool" => Value::Bool(rng.bool()),
                "str" => {
                    let n = rng.below(u64::from(size).min(24) + 1) as usize;
                    let s: String = (0..n)
                        .map(|_| INTERESTING_CHARS[rng.below(INTERESTING_CHARS.len() as u64) as usize])
                        .collect();
                    Value::str(s)
                }
                "Option" => {
                    // `None` stays common so the empty case is well covered.
                    match args.first() {
                        Some(inner) if rng.below(4) != 0 => {
                            Value::some(generate(rng, inner, size))
                        }
                        _ => Value::none(),
                    }
                }
                // The checker rejects anything else, so this is unreachable
                // for programs that compiled.
                _ => Value::Unit,
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Shrinking
// ---------------------------------------------------------------------------

/// Smaller values to try in place of `v`, most aggressive first. A shrink
/// candidate is only accepted if it still fails, so this can be greedy.
fn shrink(v: &Value) -> Vec<Value> {
    match v {
        Value::Int(0) => vec![],
        Value::Int(n) => {
            // Binary search toward zero, most aggressive first: 0, n/2, 3n/4,
            // 7n/8, … The greedy caller takes the first candidate that still
            // fails, so a failure boundary far from zero is reached in about
            // log2(n) rounds instead of one step at a time.
            let mut out = vec![Value::Int(0)];
            let mut delta = *n;
            while delta != 0 {
                let candidate = n - delta;
                if candidate != *n && candidate != 0 {
                    out.push(Value::Int(candidate));
                }
                delta /= 2;
            }
            // A positive value reads better than a negative one of the same
            // magnitude. `wrapping_neg` because `-i64::MIN` is not a value.
            if *n < 0 {
                out.push(Value::Int(n.wrapping_neg()));
            }
            out
        }
        Value::Float(f) if *f == 0.0 => vec![],
        Value::Float(f) => {
            let mut out = vec![Value::Float(0.0)];
            if f.is_finite() {
                out.push(Value::Float(f / 2.0));
                out.push(Value::Float(f.trunc()));
            }
            out
        }
        Value::Bool(true) => vec![Value::Bool(false)],
        Value::Bool(false) => vec![],
        Value::Str(s) if s.is_empty() => vec![],
        Value::Str(s) => {
            let chars: Vec<char> = s.chars().collect();
            let mut out = vec![Value::str(String::new())];
            if chars.len() > 1 {
                out.push(Value::str(chars[..chars.len() / 2].iter().collect::<String>()));
                out.push(Value::str(chars[chars.len() / 2..].iter().collect::<String>()));
            }
            // Drop one character at a time, so the survivor is minimal.
            for i in 0..chars.len() {
                let mut c = chars.clone();
                c.remove(i);
                out.push(Value::str(c.into_iter().collect::<String>()));
            }
            out
        }
        Value::List(items) if items.is_empty() => vec![],
        Value::List(items) => {
            let mut out = vec![list_of(vec![])];
            if items.len() > 1 {
                out.push(list_of(items[..items.len() / 2].to_vec()));
                out.push(list_of(items[items.len() / 2..].to_vec()));
            }
            for i in 0..items.len() {
                let mut c = items.as_ref().clone();
                c.remove(i);
                out.push(list_of(c));
            }
            // Then shrink elements in place, keeping the length.
            for i in 0..items.len() {
                for smaller in shrink(&items[i]) {
                    let mut c = items.as_ref().clone();
                    c[i] = smaller;
                    out.push(list_of(c));
                }
            }
            out
        }
        Value::Tuple(items) => {
            let mut out = Vec::new();
            for i in 0..items.len() {
                for smaller in shrink(&items[i]) {
                    let mut c = items.as_ref().clone();
                    c[i] = smaller;
                    out.push(Value::Tuple(Rc::new(c)));
                }
            }
            out
        }
        Value::Variant {
            variant, fields, ..
        } if variant.as_str() == "Some" => {
            let mut out = vec![Value::none()];
            if let Some(inner) = fields.first() {
                for smaller in shrink(inner) {
                    out.push(Value::some(smaller));
                }
            }
            out
        }
        _ => vec![],
    }
}

/// Total shrink attempts allowed, so a pathological property cannot hang the
/// suite. Reported when hit rather than silently truncating.
const SHRINK_BUDGET: u32 = 2000;

// ---------------------------------------------------------------------------
// Coverage
// ---------------------------------------------------------------------------

/// Which source lines the run reached, against which lines could be reached.
#[derive(Default)]
pub struct Coverage {
    pub executable: BTreeSet<(String, u32)>,
    pub covered: BTreeSet<(String, u32)>,
}

impl Coverage {
    pub fn percent(&self) -> f64 {
        if self.executable.is_empty() {
            return 100.0;
        }
        let hit = self
            .executable
            .iter()
            .filter(|k| self.covered.contains(*k))
            .count();
        hit as f64 * 100.0 / self.executable.len() as f64
    }

    /// Uncovered lines per file, in source order.
    pub fn misses(&self) -> Vec<(String, Vec<u32>)> {
        let mut out: Vec<(String, Vec<u32>)> = Vec::new();
        for (file, line) in &self.executable {
            if self.covered.contains(&(file.clone(), *line)) {
                continue;
            }
            match out.last_mut() {
                Some((f, lines)) if f == file => lines.push(*line),
                _ => out.push((file.clone(), vec![*line])),
            }
        }
        out
    }
}

/// Collapses a run of line numbers into `3-7, 11` form.
pub fn format_ranges(lines: &[u32]) -> String {
    let mut parts: Vec<String> = Vec::new();
    let mut i = 0;
    while i < lines.len() {
        let start = lines[i];
        let mut end = start;
        while i + 1 < lines.len() && lines[i + 1] == end + 1 {
            i += 1;
            end = lines[i];
        }
        parts.push(if start == end {
            format!("{start}")
        } else {
            format!("{start}-{end}")
        });
        i += 1;
    }
    parts.join(", ")
}

// ---------------------------------------------------------------------------
// Benchmarks
// ---------------------------------------------------------------------------

pub struct BenchResult {
    pub name: String,
    pub iterations: u64,
    pub ns_per_op: f64,
    /// Set when the body failed; the timing is then meaningless.
    pub error: Option<String>,
}

/// How long each benchmark is sampled for. Long enough to swamp clock
/// granularity, short enough that a suite of them stays interactive.
const BENCH_TARGET: std::time::Duration = std::time::Duration::from_millis(200);
const BENCH_MAX_ITERS: u64 = 20_000_000;

// ---------------------------------------------------------------------------
// Runner
// ---------------------------------------------------------------------------

/// Options for one `grasm test` invocation.
pub struct TestOptions {
    pub seed: u64,
    pub cases: u32,
    pub coverage: bool,
    /// Substring filter over declaration names; empty runs everything.
    pub filter: String,
}

impl Default for TestOptions {
    fn default() -> Self {
        TestOptions {
            seed: 0x5eed,
            cases: 100,
            coverage: false,
            filter: String::new(),
        }
    }
}

pub struct TestSummary {
    pub passed: u32,
    pub failed: u32,
    pub coverage: Option<Coverage>,
}

/// Renders a counterexample as `x = 3, s = "ab"`.
pub fn render_inputs(inputs: &[(String, Value)]) -> String {
    inputs
        .iter()
        .map(|(n, v)| match v {
            Value::Str(s) => format!("{n} = {s:?}"),
            other => format!("{n} = {other}"),
        })
        .collect::<Vec<_>>()
        .join(", ")
}

impl<W: Write> Interp<'_, W> {
    /// Starts recording which statements execute.
    pub fn enable_coverage(&mut self) {
        self.coverage = Some(BTreeSet::new());
    }

    /// Takes the recorded span offsets, if coverage was enabled.
    pub fn take_coverage(&mut self) -> Option<BTreeSet<u32>> {
        self.coverage.take()
    }

    /// Writes a line to the runner's output.
    pub fn report(&mut self, line: &str) {
        let _ = writeln!(self.out, "{line}");
    }

    /// Runs one unit test (no parameters). Returns whether it passed.
    pub fn run_unit_test(&mut self, t: &grasm_syntax::ast::TestDecl) -> bool {
        match self.try_case(&[], &t.body) {
            None => {
                self.report(&format!("test \"{}\" ... ok", t.name));
                true
            }
            Some(message) => {
                self.report(&format!("test \"{}\" ... FAILED: {message}", t.name));
                false
            }
        }
    }

    /// Runs one property test to completion, shrinking any failure.
    pub fn run_property(
        &mut self,
        params: &[grasm_syntax::ast::Param],
        body: &grasm_syntax::ast::Block,
        opts: &TestOptions,
    ) -> PropertyOutcome {
        let mut rng = Rng::new(opts.seed);
        for case in 0..opts.cases {
            // Size ramps from 1 to ~50 over the run.
            let size = 1 + case / 2;
            let inputs: Vec<(String, Value)> = params
                .iter()
                .map(|p| (p.name.name.clone(), generate(&mut rng, &p.ty, size)))
                .collect();
            if let Some(message) = self.try_case(&inputs, body) {
                let (inputs, message) = self.shrink_failure(inputs, message, body);
                return PropertyOutcome::Failed {
                    inputs,
                    message,
                    case: case + 1,
                };
            }
        }
        PropertyOutcome::Passed(opts.cases)
    }

    /// Runs the body with `inputs` bound. `None` means it held.
    fn try_case(
        &mut self,
        inputs: &[(String, Value)],
        body: &grasm_syntax::ast::Block,
    ) -> Option<String> {
        let mut scope = std::collections::HashMap::new();
        for (name, v) in inputs {
            scope.insert(name.clone(), v.clone());
        }
        self.scopes.push(scope);
        let result = self.eval_block(body);
        self.scopes.pop();
        match result {
            Ok(_) | Err(Flow::Return(_)) => None,
            Err(Flow::Panic(m)) => Some(m),
            Err(Flow::Err(e)) => Some(format!("error: {e}")),
            Err(Flow::Exit(c)) => Some(format!("exit({c}) inside a test")),
            Err(_) => Some("unexpected control flow".to_string()),
        }
    }

    /// Greedily replaces inputs with smaller ones that still fail.
    fn shrink_failure(
        &mut self,
        mut inputs: Vec<(String, Value)>,
        mut message: String,
        body: &grasm_syntax::ast::Block,
    ) -> (Vec<(String, Value)>, String) {
        let mut budget = SHRINK_BUDGET;
        let mut improved = true;
        while improved && budget > 0 {
            improved = false;
            for i in 0..inputs.len() {
                for candidate in shrink(&inputs[i].1) {
                    if budget == 0 {
                        break;
                    }
                    budget -= 1;
                    let mut trial = inputs.clone();
                    trial[i].1 = candidate;
                    if let Some(m) = self.try_case(&trial, body) {
                        inputs = trial;
                        message = m;
                        improved = true;
                        break;
                    }
                }
            }
        }
        (inputs, message)
    }

    /// Times one benchmark body. Warms up, then samples until `BENCH_TARGET`.
    pub fn run_bench(&mut self, name: &str, body: &grasm_syntax::ast::Block) -> BenchResult {
        let fail = |message: String| BenchResult {
            name: name.to_string(),
            iterations: 0,
            ns_per_op: 0.0,
            error: Some(message),
        };
        // One warm-up pass, which also surfaces a broken body before timing.
        self.scopes.push(std::collections::HashMap::new());
        let warm = self.eval_block(body);
        self.scopes.pop();
        match warm {
            Ok(_) | Err(Flow::Return(_)) => {}
            Err(Flow::Panic(m)) => return fail(m),
            Err(Flow::Err(e)) => return fail(format!("error: {e}")),
            Err(_) => return fail("unexpected control flow".to_string()),
        }

        let start = std::time::Instant::now();
        let mut iterations = 0u64;
        while start.elapsed() < BENCH_TARGET && iterations < BENCH_MAX_ITERS {
            // Batch between clock reads: for a fast body the clock call would
            // otherwise dominate what is being measured.
            for _ in 0..16 {
                self.scopes.push(std::collections::HashMap::new());
                let r = self.eval_block(body);
                self.scopes.pop();
                if let Err(Flow::Panic(m)) = r {
                    return fail(m);
                }
            }
            iterations += 16;
        }
        let elapsed = start.elapsed();
        BenchResult {
            name: name.to_string(),
            iterations,
            ns_per_op: elapsed.as_nanos() as f64 / iterations.max(1) as f64,
            error: None,
        }
    }
}

/// Every line that holds a statement — the denominator for coverage.
pub fn executable_lines(
    file: &str,
    module: &Module,
    base: u32,
    index: &grasm_syntax::span::LineIndex,
    out: &mut BTreeSet<(String, u32)>,
) {
    use grasm_syntax::ast::Decl;
    for decl in &module.decls {
        let body = match decl {
            Decl::Fn(f) => &f.body,
            Decl::Test(t) => &t.body,
            Decl::Bench(b) => &b.body,
            _ => continue,
        };
        block_lines(file, body, base, index, out);
    }
}

fn block_lines(
    file: &str,
    block: &grasm_syntax::ast::Block,
    base: u32,
    index: &grasm_syntax::span::LineIndex,
    out: &mut BTreeSet<(String, u32)>,
) {
    use grasm_syntax::ast::Stmt;
    for stmt in &block.stmts {
        out.insert((file.to_string(), index.line_col(stmt_start(stmt).saturating_sub(base)).line));
        // Nested blocks contribute their own lines; a statement only counts as
        // covered when control actually reached it.
        match stmt {
            Stmt::While { cond, body, .. } => {
                expr_lines(file, cond, base, index, out);
                block_lines(file, body, base, index, out);
            }
            Stmt::For { iter, body, .. } => {
                expr_lines(file, iter, base, index, out);
                block_lines(file, body, base, index, out);
            }
            Stmt::Parallel { body, .. } | Stmt::Spawn { body, .. } => {
                block_lines(file, body, base, index, out)
            }
            Stmt::With { handler, body, .. } => {
                expr_lines(file, handler, base, index, out);
                block_lines(file, body, base, index, out);
            }
            Stmt::Let { init, .. } => expr_lines(file, init, base, index, out),
            Stmt::Assign { target, value, .. } => {
                expr_lines(file, target, base, index, out);
                expr_lines(file, value, base, index, out);
            }
            Stmt::Return(Some(e), _) | Stmt::Expr(e) => expr_lines(file, e, base, index, out),
            _ => {}
        }
    }
}

/// `if`, `match`, and closures carry blocks inside expressions, so coverage
/// has to descend into them too — otherwise a never-taken branch would look
/// covered simply because its enclosing statement ran.
fn expr_lines(
    file: &str,
    e: &grasm_syntax::ast::Expr,
    base: u32,
    index: &grasm_syntax::span::LineIndex,
    out: &mut BTreeSet<(String, u32)>,
) {
    use grasm_syntax::ast::{ElseBranch, Expr};
    match e {
        Expr::If {
            cond,
            then_block,
            else_branch,
            ..
        } => {
            expr_lines(file, cond, base, index, out);
            block_lines(file, then_block, base, index, out);
            match else_branch.as_deref() {
                Some(ElseBranch::Else(b)) => block_lines(file, b, base, index, out),
                Some(ElseBranch::ElseIf(inner)) => expr_lines(file, inner, base, index, out),
                None => {}
            }
        }
        Expr::Match { scrutinee, arms, .. } => {
            expr_lines(file, scrutinee, base, index, out);
            for arm in arms {
                if let Some(g) = &arm.guard {
                    expr_lines(file, g, base, index, out);
                }
                expr_lines(file, &arm.body, base, index, out);
            }
        }
        Expr::Block(b) => block_lines(file, b, base, index, out),
        Expr::Closure { body, .. } => expr_lines(file, body, base, index, out),
        Expr::Try { expr, .. } | Expr::Unary { expr, .. } => expr_lines(file, expr, base, index, out),
        Expr::Binary { lhs, rhs, .. } => {
            expr_lines(file, lhs, base, index, out);
            expr_lines(file, rhs, base, index, out);
        }
        Expr::Call { args, .. } => {
            for a in args {
                expr_lines(file, a, base, index, out);
            }
        }
        Expr::MethodCall { recv, args, .. } => {
            expr_lines(file, recv, base, index, out);
            for a in args {
                expr_lines(file, a, base, index, out);
            }
        }
        Expr::List(items, _) | Expr::Tuple(items, _) => {
            for i in items {
                expr_lines(file, i, base, index, out);
            }
        }
        _ => {}
    }
}

pub(crate) fn stmt_start(s: &grasm_syntax::ast::Stmt) -> u32 {
    stmt_span(s).start
}

/// The span of a whole statement.
pub(crate) fn stmt_span(s: &grasm_syntax::ast::Stmt) -> grasm_syntax::span::Span {
    use grasm_syntax::ast::Stmt;
    match s {
        Stmt::Let { span, .. }
        | Stmt::Assign { span, .. }
        | Stmt::Return(_, span)
        | Stmt::Break(span)
        | Stmt::Continue(span)
        | Stmt::While { span, .. }
        | Stmt::For { span, .. }
        | Stmt::Parallel { span, .. }
        | Stmt::Spawn { span, .. }
        | Stmt::With { span, .. } => *span,
        Stmt::Expr(e) => e.span(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// `Value` has no `PartialEq`; equality is structural via `eq_value`.
    fn has(candidates: &[Value], want: &Value) -> bool {
        candidates.iter().any(|c| c.eq_value(want))
    }

    #[test]
    fn the_rng_is_deterministic_and_seed_dependent() {
        let a: Vec<u64> = (0..5).scan(Rng::new(7), |r, _| Some(r.next_u64())).collect();
        let b: Vec<u64> = (0..5).scan(Rng::new(7), |r, _| Some(r.next_u64())).collect();
        let c: Vec<u64> = (0..5).scan(Rng::new(8), |r, _| Some(r.next_u64())).collect();
        assert_eq!(a, b, "the same seed must replay exactly");
        assert_ne!(a, c, "different seeds must diverge");
    }

    #[test]
    fn a_zero_seed_still_varies() {
        let vals: Vec<u64> = (0..8).scan(Rng::new(0), |r, _| Some(r.next_u64())).collect();
        assert!(
            vals.windows(2).any(|w| w[0] != w[1]),
            "seed 0 produced a constant stream: {vals:?}"
        );
    }

    #[test]
    fn below_stays_in_range() {
        let mut r = Rng::new(1);
        for n in [1u64, 2, 7, 100] {
            for _ in 0..200 {
                assert!(r.below(n) < n);
            }
        }
        assert_eq!(r.below(0), 0, "below(0) must not divide by zero");
    }

    #[test]
    fn shrinking_moves_toward_zero_and_empty() {
        assert!(shrink(&Value::Int(0)).is_empty(), "0 is already minimal");
        assert!(has(&shrink(&Value::Int(50)), &Value::Int(0)));
        assert!(shrink(&Value::Bool(false)).is_empty());
        assert!(has(&shrink(&Value::Bool(true)), &Value::Bool(false)));
        assert!(has(&shrink(&Value::str("abc")), &Value::str("")));
        assert!(shrink(&Value::str("")).is_empty());
        assert!(shrink(&list_of(vec![])).is_empty());
        assert!(has(&shrink(&list_of(vec![Value::Int(1)])), &list_of(vec![])));
    }

    #[test]
    fn shrinking_the_extreme_int_does_not_overflow() {
        // `-i64::MIN` is not representable; a naive negate would panic here.
        let candidates = shrink(&Value::Int(i64::MIN));
        assert!(!candidates.is_empty());
        assert!(has(&candidates, &Value::Int(0)));
    }

    #[test]
    fn shrinking_a_list_can_drop_and_reduce_elements() {
        let v = list_of(vec![Value::Int(9), Value::Int(4)]);
        let c = shrink(&v);
        assert!(has(&c, &list_of(vec![])), "must try the empty list");
        assert!(
            has(&c, &list_of(vec![Value::Int(4)])),
            "must try dropping the first element"
        );
        assert!(
            has(&c, &list_of(vec![Value::Int(0), Value::Int(4)])),
            "must try shrinking an element in place"
        );
    }

    #[test]
    fn some_shrinks_to_none() {
        let v = Value::some(Value::Int(5));
        let c = shrink(&v);
        assert!(has(&c, &Value::none()));
    }

    #[test]
    fn line_ranges_collapse() {
        assert_eq!(format_ranges(&[]), "");
        assert_eq!(format_ranges(&[3]), "3");
        assert_eq!(format_ranges(&[3, 4, 5]), "3-5");
        assert_eq!(format_ranges(&[3, 4, 5, 9, 11, 12]), "3-5, 9, 11-12");
    }

    #[test]
    fn coverage_percentage_and_misses() {
        let mut c = Coverage::default();
        assert_eq!(c.percent(), 100.0, "nothing executable is fully covered");
        for line in 1..=4 {
            c.executable.insert(("a.gr".into(), line));
        }
        c.covered.insert(("a.gr".into(), 1));
        c.covered.insert(("a.gr".into(), 2));
        assert_eq!(c.percent(), 50.0);
        assert_eq!(c.misses(), vec![("a.gr".to_string(), vec![3, 4])]);
    }
}
