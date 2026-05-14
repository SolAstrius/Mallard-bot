//! High-level math/typesetting render layer.
//!
//! Two dialects: native Typst, and LaTeX-math via the `@preview/mitex`
//! Typst package. Both run through the same underlying [`crate::typst`]
//! compile pipeline — we just hand it different assembled documents.
//!
//! Dialect comes from either an explicit caller choice (`/typst`, `/latex`)
//! or [`detect`], which looks for `\<letter>+` patterns that only LaTeX
//! uses inside math.

use crate::typst::{compile_doc, RenderError, RenderOpts};

const MITEX_VERSION: &str = "0.2.5";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Dialect {
    Typst,
    Latex,
}

impl Dialect {
    pub fn name(self) -> &'static str {
        match self {
            Self::Typst => "typst",
            Self::Latex => "latex",
        }
    }
}

/// Pick a dialect from the shape of `src`. `\<letter>+` (e.g. `\frac`,
/// `\alpha`, `\begin`) is unambiguously LaTeX — Typst's math mode never
/// uses backslash for control sequences. Anything else defaults to Typst.
///
/// Edge cases like a Typst source containing a literal `"\\frac"` string
/// will mis-detect; callers wanting precision should use an explicit
/// command.
pub fn detect(src: &str) -> Dialect {
    let bytes = src.as_bytes();
    let mut i = 0;
    while i + 1 < bytes.len() {
        if bytes[i] == b'\\' && bytes[i + 1].is_ascii_alphabetic() {
            return Dialect::Latex;
        }
        i += 1;
    }
    Dialect::Typst
}

/// Build the Typst document that will be fed to the compiler. Pure — no
/// I/O — so unit-tested without spinning up the binary.
pub fn assemble(src: &str, dialect: Dialect, opts: &RenderOpts) -> String {
    match dialect {
        Dialect::Typst => assemble_typst(src, opts),
        Dialect::Latex => assemble_latex(src, opts),
    }
}

fn preamble(opts: &RenderOpts) -> String {
    format!(
        "#set page(width: auto, height: auto, margin: (x: 10pt, y: 8pt))\n\
         #set text(size: {size}pt)\n",
        size = opts.text_size_pt,
    )
}

/// Bare math (no `#`, no `$`) → wrap in `$ ... $`. Otherwise verbatim.
fn assemble_typst(src: &str, opts: &RenderOpts) -> String {
    let trimmed = src.trim();
    let body = if trimmed.contains('#') || trimmed.contains('$') {
        trimmed.to_string()
    } else {
        format!("$ {trimmed} $")
    };
    format!("{}{body}\n", preamble(opts))
}

/// Wrap LaTeX in a `mitex(```...```)` raw block. We swap any literal
/// triple-backtick in user input to a visually-similar Unicode trio so the
/// raw block isn't truncated. Math expressions virtually never contain
/// triple-backticks, so the swap is invisible in practice.
fn assemble_latex(src: &str, opts: &RenderOpts) -> String {
    let trimmed = src.trim();
    let escaped = trimmed.replace("```", "\u{2034}\u{2034}\u{2034}");
    format!(
        "{preamble}#import \"@preview/mitex:{ver}\": mitex\n\
         #mitex(```\n{body}\n```)\n",
        preamble = preamble(opts),
        ver = MITEX_VERSION,
        body = escaped,
    )
}

/// Assemble + compile in one call. The hot path for both `/typst`,
/// `/latex`, and the ambient detectors.
pub async fn render(
    src: &str,
    dialect: Dialect,
    opts: &RenderOpts,
) -> Result<Vec<Vec<u8>>, RenderError> {
    let doc = assemble(src, dialect, opts);
    compile_doc(&doc, opts).await
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn detect_latex_from_backslash_command() {
        assert_eq!(detect("\\frac{1}{2}"), Dialect::Latex);
        assert_eq!(detect("\\alpha + \\beta"), Dialect::Latex);
        assert_eq!(detect("\\begin{matrix} 1 & 2 \\end{matrix}"), Dialect::Latex);
    }

    #[test]
    fn detect_typst_for_bare_math() {
        assert_eq!(detect("x^2 + 1"), Dialect::Typst);
        assert_eq!(detect("sum_(k=1)^n k"), Dialect::Typst);
        assert_eq!(detect("mat(1, 2; 3, 4)"), Dialect::Typst);
    }

    #[test]
    fn detect_typst_for_explicit_typst_syntax() {
        assert_eq!(detect("#set page(width: 5cm)"), Dialect::Typst);
        assert_eq!(detect("$ frac(1, 2) $"), Dialect::Typst);
    }

    #[test]
    fn detect_ignores_backslash_followed_by_non_letter() {
        // `\\` (backslash-backslash) is the only escape combination we
        // robustly ignore. A Windows-style `path\to\file` would mis-
        // detect as LaTeX — acceptable: the bot's domain is math chat,
        // not filesystem paths, and users always have explicit /typst.
        assert_eq!(detect("a \\\\ b"), Dialect::Typst);
    }

    #[test]
    fn assemble_typst_wraps_bare_math() {
        let out = assemble("x^2", Dialect::Typst, &RenderOpts::default());
        assert!(out.contains("$ x^2 $"), "got: {out}");
    }

    #[test]
    fn assemble_typst_leaves_doc_alone() {
        let out = assemble(
            "#set page(width: 5cm)\nhi",
            Dialect::Typst,
            &RenderOpts::default(),
        );
        assert!(out.contains("#set page(width: 5cm)"));
        assert!(!out.contains("$ #set"));
    }

    #[test]
    fn assemble_latex_uses_mitex() {
        let out = assemble("\\frac{1}{2}", Dialect::Latex, &RenderOpts::default());
        assert!(out.contains("@preview/mitex"), "got: {out}");
        assert!(out.contains("#mitex(```"));
        assert!(out.contains("\\frac{1}{2}"));
    }

    async fn typst_on_path() -> bool {
        tokio::process::Command::new("typst")
            .arg("--version")
            .output()
            .await
            .map(|o| o.status.success())
            .unwrap_or(false)
    }

    #[tokio::test]
    async fn render_latex_via_mitex() {
        if !typst_on_path().await {
            eprintln!("skipping render test — typst not on PATH");
            return;
        }
        let pages = render(
            "\\frac{1}{2} + \\int_0^\\infty e^{-x^2} dx = \\frac{\\sqrt{\\pi}}{2}",
            Dialect::Latex,
            &RenderOpts::default(),
        )
        .await
        .expect("render");
        assert_eq!(pages.len(), 1);
        assert_eq!(&pages[0][0..8], b"\x89PNG\r\n\x1a\n");
    }

    #[tokio::test]
    async fn render_routes_detected_dialect_correctly() {
        if !typst_on_path().await {
            return;
        }
        // The same source string should render with mitex when dialect=latex,
        // and via raw typst (which would compile-error on \frac) when dialect=typst.
        let src = "\\frac{1}{2}";
        let opts = RenderOpts::default();
        assert!(render(src, Dialect::Latex, &opts).await.is_ok());
        assert!(render(src, Dialect::Typst, &opts).await.is_err());
    }
}
