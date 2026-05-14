//! Numeric calculator wrapper around `fend-core`.
//!
//! Exposes a tiny `evaluate` that takes a user expression and returns the
//! formatted result (or a friendly error). Fend handles arithmetic, units,
//! bases, complex numbers, dates, transcendentals, and one-shot lambdas;
//! see [`fend_core`] for the language reference.
//!
//! We keep the engine stateless — a fresh `Context` per evaluation. Each
//! call is independent (no carry-over of `let` bindings across messages),
//! matching Mallard's stateless contract.

use std::time::Duration;

#[derive(Debug)]
pub enum CalcError {
    Empty,
    TooLong { len: usize, max: usize },
    Timeout,
    Eval(String),
}

impl std::fmt::Display for CalcError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Empty => write!(f, "пустое выражение"),
            Self::TooLong { len, max } => write!(f, "слишком длинно ({len} > {max})"),
            Self::Timeout => write!(f, "вычисление слишком долгое"),
            Self::Eval(s) => write!(f, "{s}"),
        }
    }
}

impl std::error::Error for CalcError {}

#[derive(Debug, Clone)]
pub struct CalcOpts {
    pub max_input_bytes: usize,
    pub timeout: Duration,
}

impl Default for CalcOpts {
    fn default() -> Self {
        Self {
            max_input_bytes: 4 * 1024,
            timeout: Duration::from_secs(2),
        }
    }
}

/// Evaluate `expr` and return the main result as a chat-ready string.
///
/// Fend's evaluation is synchronous and CPU-bound; we run it under
/// `spawn_blocking` so it doesn't stall the async runtime, with a soft
/// timeout via [`tokio::time::timeout`]. The timeout is best-effort —
/// if fend is mid-evaluation when it fires, the worker still finishes
/// (fend doesn't have a cancellation token), but the bot moves on.
pub async fn evaluate(expr: &str, opts: &CalcOpts) -> Result<String, CalcError> {
    let trimmed = expr.trim();
    if trimmed.is_empty() {
        return Err(CalcError::Empty);
    }
    if trimmed.len() > opts.max_input_bytes {
        return Err(CalcError::TooLong {
            len: trimmed.len(),
            max: opts.max_input_bytes,
        });
    }

    let owned = trimmed.to_string();
    let work = tokio::task::spawn_blocking(move || {
        let mut ctx = fend_core::Context::new();
        match fend_core::evaluate(&owned, &mut ctx) {
            Ok(r) => Ok(r.get_main_result().to_string()),
            Err(e) => Err(e.to_string()),
        }
    });

    match tokio::time::timeout(opts.timeout, work).await {
        Ok(Ok(Ok(s))) => Ok(s),
        Ok(Ok(Err(msg))) => Err(CalcError::Eval(msg)),
        Ok(Err(join_err)) => Err(CalcError::Eval(format!("worker panic: {join_err}"))),
        Err(_) => Err(CalcError::Timeout),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn arithmetic_exact() {
        let opts = CalcOpts::default();
        assert_eq!(evaluate("1/3 + 1/3 + 1/3", &opts).await.unwrap(), "1");
        assert_eq!(evaluate("2 + 2 * 3", &opts).await.unwrap(), "8");
    }

    #[tokio::test]
    async fn bigint_factorials() {
        let opts = CalcOpts::default();
        let out = evaluate("20!", &opts).await.unwrap();
        assert_eq!(out, "2432902008176640000");
    }

    #[tokio::test]
    async fn units_conversion() {
        let opts = CalcOpts::default();
        let out = evaluate("60 mph to m/s", &opts).await.unwrap();
        // Fend's exact rational keeps the answer precise; format varies by
        // version, so just check the key magnitude.
        assert!(out.contains("26.8224"), "got: {out}");
        assert!(out.contains("m/s") || out.contains("m / s"), "got: {out}");
    }

    #[tokio::test]
    async fn bases_and_bitwise() {
        let opts = CalcOpts::default();
        assert_eq!(evaluate("0xff to decimal", &opts).await.unwrap(), "255");
    }

    #[tokio::test]
    async fn errors_on_garbage() {
        let opts = CalcOpts::default();
        let err = evaluate("totally not math", &opts).await.unwrap_err();
        assert!(matches!(err, CalcError::Eval(_)), "got: {err:?}");
    }

    #[tokio::test]
    async fn empty_input_rejected() {
        let opts = CalcOpts::default();
        let err = evaluate("   ", &opts).await.unwrap_err();
        assert!(matches!(err, CalcError::Empty));
    }
}
