//! Symbolic CAS wrapper around the `symbolica` crate.
//!
//! Recognized syntaxes (parsed by *our* outer layer, not Symbolica's
//! evaluator — Symbolica treats `diff(...)` etc. as inert function names):
//!
//!   * `<expr>`                       — parse + canonicalize
//!   * `expand(<expr>)`               — distribute / expand
//!   * `factor(<expr>)`               — factor over rationals
//!   * `together(<expr>)`             — common-denominator a sum of fractions
//!   * `diff(<expr>, <var>)`          — symbolic derivative
//!   * `derivative(<expr>, <var>)`    — alias
//!
//! Output is both plain text (Display) and LaTeX (via `AtomPrinter` with
//! `PrintOptions::latex()`), so callers can either send text or pipe the
//! LaTeX through the mitex renderer for a pretty image.

use std::time::Duration;

use symbolica::atom::{Atom, AtomCore};
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
    Derivative,
}

fn parse_op_call(input: &str) -> (Op, &str) {
    let t = input.trim();
    let ops: &[(&str, Op)] = &[
        ("expand", Op::Expand),
        ("factor", Op::Factor),
        ("together", Op::Together),
        ("derivative", Op::Derivative),
        ("diff", Op::Derivative),
    ];
    for (name, op) in ops {
        if let Some(rest) = t.strip_prefix(name) {
            let rest = rest.trim_start();
            // Only treat as an op call when the *whole* tail is `(...)` —
            // `expand(x) + 1` falls through to Identity so the user gets
            // a parse error from Symbolica instead of us silently dropping
            // the `+ 1` part.
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
        Op::Derivative => {
            let parts = split_top_commas(inner);
            if parts.len() != 2 {
                return Err(SymError::Eval(
                    "diff/derivative ждёт два аргумента: diff(<выражение>, <переменная>)"
                        .to_string(),
                ));
            }
            let expr = parse_atom(parts[0])?;
            let var_name = parts[1].trim();
            if var_name.is_empty() {
                return Err(SymError::Eval(
                    "diff/derivative: имя переменной пустое".to_string(),
                ));
            }
            // `symbol!` panics on invalid identifiers; we'd rather surface
            // that as a normal error, so wrap in `catch_unwind`.
            let var = match std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                symbolica::symbol!(var_name)
            })) {
                Ok(s) => s,
                Err(_) => {
                    return Err(SymError::Eval(format!(
                        "diff: невалидное имя переменной {var_name:?}"
                    )));
                }
            };
            Ok(expr.derivative(var))
        }
    }
}

fn parse_atom(s: &str) -> Result<Atom, SymError> {
    symbolica::try_parse!(s.trim()).map_err(SymError::Parse)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Symbolica's free tier allows only one instance per machine, and the
    /// process-wide "I'm running" flag isn't released between cargo test
    /// functions in the same binary. So every assertion that actually
    /// touches Symbolica has to live inside one `#[test]` body. Pure helpers
    /// (split, balanced, empty rejection) stay as separate tests.
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
