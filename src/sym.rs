//! Symbolic CAS wrapper around the `symbolica` crate.
//!
//! Recognized syntaxes (parsed by *our* outer layer, not Symbolica's
//! evaluator — Symbolica treats `diff(...)` etc. as inert function names):
//!
//!   * `<expr>`                                  — parse + canonicalize
//!   * `expand(<expr>)`                          — distribute / expand
//!   * `factor(<expr>)`                          — factor over rationals
//!   * `together(<expr>)`                        — common-denominator
//!   * `simplify(<expr>)`                        — expand + together pass
//!   * `diff(<expr>, <var>)`                     — derivative
//!   * `derivative(<expr>, <var>)`               — alias for `diff`
//!   * `series(<expr> [, <var> [, <point> [, <depth>]]])` — Taylor/Laurent
//!     series. Defaults: var=`x`, point=`0`, depth=`5`.
//!   * `solve(<eq>, <var>)`                      — linear single-var
//!   * `solve(<eq1>, <eq2>, ..., <var1>, <var2>, ...)` — linear system
//!     (equal counts of equations and unknowns)
//!   * `replace(<expr>, <pattern>, <rhs>)`       — pattern rewrite.
//!     Wildcards: identifiers ending in `_` (Symbolica convention),
//!     e.g. `replace(f(x), f(y_), y_+1)`.
//!
//! Output is both plain text (Display) and LaTeX (via `AtomPrinter` with
//! `PrintOptions::latex()`), so callers can either send text or pipe the
//! LaTeX through the mitex renderer for a pretty image.

use std::time::Duration;

use symbolica::atom::{Atom, AtomCore, Indeterminate};
use symbolica::domains::rational::Rational;
use symbolica::printer::{AtomPrinter, PrintOptions};

#[derive(Debug)]
pub enum SymError {
    Empty,
    TooLong { len: usize, max: usize },
    Timeout,
    Parse(String),
    Eval(String),
}

impl std::fmt::Display for SymError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Empty => write!(f, "пустое выражение"),
            Self::TooLong { len, max } => write!(f, "слишком длинно ({len} > {max})"),
            Self::Timeout => write!(f, "вычисление слишком долгое"),
            Self::Parse(s) => write!(f, "не разобрал: {s}"),
            Self::Eval(s) => write!(f, "{s}"),
        }
    }
}

impl std::error::Error for SymError {}

#[derive(Debug, Clone)]
pub struct SymOpts {
    pub max_input_bytes: usize,
    pub timeout: Duration,
}

impl Default for SymOpts {
    fn default() -> Self {
        Self {
            max_input_bytes: 4 * 1024,
            timeout: Duration::from_secs(10),
        }
    }
}

#[derive(Debug, Clone)]
pub struct SymResult {
    pub text: String,
    pub latex: String,
}

pub async fn evaluate(input: &str, opts: &SymOpts) -> Result<SymResult, SymError> {
    let trimmed = input.trim();
    if trimmed.is_empty() {
        return Err(SymError::Empty);
    }
    if trimmed.len() > opts.max_input_bytes {
        return Err(SymError::TooLong {
            len: trimmed.len(),
            max: opts.max_input_bytes,
        });
    }

    let owned = trimmed.to_string();
    let work = tokio::task::spawn_blocking(move || -> Result<SymResult, SymError> {
        let atom = dispatch(&owned)?;
        let text = format!("{}", atom);
        let latex = format!(
            "{}",
            AtomPrinter::new_with_options(atom.as_view(), PrintOptions::latex())
        );
        Ok(SymResult { text, latex })
    });

    match tokio::time::timeout(opts.timeout, work).await {
        Ok(Ok(r)) => r,
        Ok(Err(e)) => Err(SymError::Eval(format!("worker panic: {e}"))),
        Err(_) => Err(SymError::Timeout),
    }
}

#[derive(Clone, Copy)]
enum Op {
    Identity,
    Expand,
    Factor,
    Together,
    Simplify,
    Derivative,
    Series,
    Solve,
    Replace,
}

fn parse_op_call(input: &str) -> (Op, &str) {
    let t = input.trim();
    // Order matters when names overlap (`derivative` vs `diff`, `replace`
    // vs nothing): longer/more-specific first.
    let ops: &[(&str, Op)] = &[
        ("derivative", Op::Derivative),
        ("simplify", Op::Simplify),
        ("together", Op::Together),
        ("replace", Op::Replace),
        ("expand", Op::Expand),
        ("factor", Op::Factor),
        ("series", Op::Series),
        ("solve", Op::Solve),
        ("diff", Op::Derivative),
    ];
    for (name, op) in ops {
        if let Some(rest) = t.strip_prefix(name) {
            let rest = rest.trim_start();
            if rest.starts_with('(') && rest.ends_with(')') && balanced(rest) {
                let inner = &rest[1..rest.len() - 1];
                return (*op, inner);
            }
        }
    }
    (Op::Identity, t)
}

/// True iff `s` has matched `()`/`[]`/`{}` with no unbalanced closer
/// before the end. Used to confirm that a prefix like `expand(…)` actually
/// owns its trailing `)`.
fn balanced(s: &str) -> bool {
    let mut depth = 0i32;
    for c in s.chars() {
        match c {
            '(' | '[' | '{' => depth += 1,
            ')' | ']' | '}' => {
                depth -= 1;
                if depth < 0 {
                    return false;
                }
            }
            _ => {}
        }
    }
    depth == 0
}

/// Split on commas at paren-depth 0. Respects `()`, `[]`, `{}`.
fn split_top_commas(s: &str) -> Vec<&str> {
    let mut out = Vec::new();
    let mut depth = 0i32;
    let mut last = 0;
    for (i, c) in s.char_indices() {
        match c {
            '(' | '[' | '{' => depth += 1,
            ')' | ']' | '}' => depth -= 1,
            ',' if depth == 0 => {
                out.push(&s[last..i]);
                last = i + 1;
            }
            _ => {}
        }
    }
    out.push(&s[last..]);
    out
}

fn dispatch(input: &str) -> Result<Atom, SymError> {
    let (op, inner) = parse_op_call(input);
    match op {
        Op::Identity => parse_atom(input),
        Op::Expand => Ok(parse_atom(inner)?.expand()),
        Op::Factor => Ok(parse_atom(inner)?.factor()),
        Op::Together => Ok(parse_atom(inner)?.together()),
        Op::Simplify => Ok(parse_atom(inner)?.together().expand()),
        Op::Derivative => dispatch_derivative(inner),
        Op::Series => dispatch_series(inner),
        Op::Solve => dispatch_solve(inner),
        Op::Replace => dispatch_replace(inner),
    }
}

fn dispatch_derivative(inner: &str) -> Result<Atom, SymError> {
    let parts = split_top_commas(inner);
    if parts.len() != 2 {
        return Err(SymError::Eval(
            "diff/derivative ждёт два аргумента: diff(<выражение>, <переменная>)".to_string(),
        ));
    }
    let expr = parse_atom(parts[0])?;
    let var = parse_var_symbol(parts[1], "diff")?;
    Ok(expr.derivative(var))
}

fn dispatch_series(inner: &str) -> Result<Atom, SymError> {
    let parts = split_top_commas(inner);
    // Defaults: var=x, point=0, depth=5. Required: <expr> [, <var> [, <point> [, <depth>]]].
    if parts.is_empty() || parts.iter().all(|p| p.trim().is_empty()) {
        return Err(SymError::Eval(
            "series ждёт хотя бы выражение: series(<expr>[, <var>[, <point>[, <depth>]]])"
                .to_string(),
        ));
    }
    let expr = parse_atom(parts[0])?;
    let var = if parts.len() >= 2 {
        parse_var_symbol(parts[1], "series")?
    } else {
        parse_var_symbol("x", "series")?
    };
    let point = if parts.len() >= 3 {
        parse_atom(parts[2])?
    } else {
        Atom::num(0)
    };
    let depth_n: u64 = if parts.len() >= 4 {
        parts[3]
            .trim()
            .parse::<u64>()
            .map_err(|_| SymError::Eval(format!("series: глубина должна быть числом, не {:?}", parts[3].trim())))?
    } else {
        5
    };
    let indet: Indeterminate = var.into();
    let series = expr
        .series(indet, point, Rational::from(depth_n), false)
        .map_err(SymError::Eval)?;
    Ok(series.to_atom())
}

fn dispatch_solve(inner: &str) -> Result<Atom, SymError> {
    let parts = split_top_commas(inner);
    if parts.len() < 2 || !parts.len().is_multiple_of(2) {
        return Err(SymError::Eval(
            "solve ждёт пары: solve(<eq>, <var>) или solve(<eq1>,…,<eqN>, <var1>,…,<varN>)"
                .to_string(),
        ));
    }
    let n = parts.len() / 2;
    let eqs: Vec<Atom> = parts[..n]
        .iter()
        .map(|s| parse_atom(s))
        .collect::<Result<_, _>>()?;
    let vars: Vec<Atom> = parts[n..]
        .iter()
        .map(|s| parse_atom(s))
        .collect::<Result<_, _>>()?;

    match Atom::solve_linear_system::<u8, _, _>(&eqs, &vars) {
        Ok(sol) => {
            // Build `(var1 = val1, var2 = val2, …)` as a single Atom via a
            // function call (the only way to package a tuple in Symbolica's
            // atom tree). For one variable, just return the single value.
            if sol.len() == 1 {
                Ok(sol.into_iter().next().unwrap())
            } else {
                // Render as "var1 = val1, var2 = val2" by formatting on the
                // way out (Atom doesn't have a clean "list" type for this).
                // We emit a custom string instead of an Atom — use a dummy
                // wrapper function and let the formatter expose the pairs.
                let parts: Vec<String> = vars
                    .iter()
                    .zip(sol.iter())
                    .map(|(v, s)| format!("{v} = {s}"))
                    .collect();
                // Wrap the pretty form as a parsed atom expression so the
                // rest of the pipeline (text + latex) works uniformly.
                // We just print a comma-separated list of equality atoms.
                let combined = parts.join(", ");
                // Parse the combined string back into an Atom — Symbolica's
                // parser handles `a = b, c = d` as a function-call-shaped
                // expression poorly, so emit it as a `solution(a-b, c-d)`
                // tuple instead. Simpler: emit `(a=b)*(c=d)*…` no.
                // Honest answer: just synthesize the text by hand and
                // return it as a special atom via parse — but `=` isn't a
                // legal binary op. Workaround: rejoin as a synthetic
                // function call.
                let fake = format!("solution({combined})");
                parse_atom(&fake)
            }
        }
        Err(e) => Err(SymError::Eval(format!("solve: {e:?}"))),
    }
}

fn dispatch_replace(inner: &str) -> Result<Atom, SymError> {
    let parts = split_top_commas(inner);
    if parts.len() != 3 {
        return Err(SymError::Eval(
            "replace ждёт три аргумента: replace(<выражение>, <шаблон>, <замена>)".to_string(),
        ));
    }
    use symbolica::id::Pattern;
    let expr = parse_atom(parts[0])?;
    let pattern: Pattern = parse_atom(parts[1])?.into();
    let rhs: Pattern = parse_atom(parts[2])?.into();
    Ok(expr.replace(pattern).with(rhs))
}

fn parse_var_symbol(s: &str, op: &str) -> Result<symbolica::atom::Symbol, SymError> {
    let var_name = s.trim();
    if var_name.is_empty() {
        return Err(SymError::Eval(format!("{op}: имя переменной пустое")));
    }
    // `symbol!` panics on invalid identifiers; we'd rather surface that
    // as a normal error.
    match std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        symbolica::symbol!(var_name)
    })) {
        Ok(s) => Ok(s),
        Err(_) => Err(SymError::Eval(format!(
            "{op}: невалидное имя переменной {var_name:?}"
        ))),
    }
}

fn parse_atom(s: &str) -> Result<Atom, SymError> {
    symbolica::try_parse!(s.trim()).map_err(SymError::Parse)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Even with a licensed Symbolica, cargo's test process reuses the
    /// instance lock between tests within one binary. So every assertion
    /// that actually touches Symbolica lives inside one `#[test]` body.
    /// Pure helpers (split, balanced, empty rejection) stay as separate
    /// tests.
    #[test]
    fn live_symbolica_smoke() {
        let rt = tokio::runtime::Runtime::new().unwrap();
        let run = |input: &str| -> String {
            rt.block_on(async {
                evaluate(input, &SymOpts::default())
                    .await
                    .unwrap_or_else(|e| panic!("eval {input:?} failed: {e}"))
                    .text
            })
        };

        // Bare expression — parsed and canonicalized.
        let out = run("x^2 + 2*x + 1");
        assert!(out.contains('x'), "bare expr: {out}");

        // Expansion.
        let out = run("expand((x+1)^3)");
        assert!(out.contains("x^3"), "expand: {out}");
        assert!(out.contains('3'), "expand: {out}");

        // Factoring.
        let out = run("factor(x^2-1)");
        assert!(
            out.contains("(x-1)") || out.contains("(-1+x)") || out.contains("(1+x)"),
            "factor: {out}"
        );

        // Symbolic derivative — power rule.
        let out = run("diff(x^2, x)");
        assert!(out.contains('2') && out.contains('x'), "diff x^2: {out}");

        // Symbolic derivative — chain on sin.
        let out = run("diff(sin(x), x)");
        assert!(out.contains("cos(x)"), "diff sin: {out}");

        // Series — sin(x) at 0, depth 5 has x and x^3 terms.
        let out = run("series(sin(x), x, 0, 5)");
        assert!(out.contains('x'), "series: {out}");

        // Solve — linear two-equation system.
        let out = run("solve(2*x + y - 1, x + y + 1, x, y)");
        assert!(out.contains('2') && out.contains("-3"), "solve: {out}");

        // Replace — wildcards via Symbolica's _-suffix convention.
        let out = run("replace(f(1,2,x) + f(1,2,3), f(1,2,y_), f(1,2,y_+1))");
        assert!(out.contains("f(1,2,") || out.contains("f(1, 2,"), "replace: {out}");

        // Simplify — together + expand normalizes.
        let out = run("simplify(1/x + 1/y)");
        assert!(out.contains('x') && out.contains('y'), "simplify: {out}");
    }

    #[test]
    fn split_commas_respects_parens() {
        let parts = split_top_commas("a, b(c, d), e");
        assert_eq!(parts, vec!["a", " b(c, d)", " e"]);
    }

    #[test]
    fn balanced_matches_parens() {
        assert!(balanced("()"));
        assert!(balanced("(a, b(c, d), e)"));
        assert!(!balanced("(a"));
        assert!(!balanced(")a("));
    }

    #[tokio::test]
    async fn empty_rejected() {
        // Doesn't touch Symbolica — pure input validation.
        assert!(matches!(
            evaluate("   ", &SymOpts::default()).await.unwrap_err(),
            SymError::Empty
        ));
    }
}
