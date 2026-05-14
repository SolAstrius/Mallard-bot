//! Function-plot rendering via typst + the `@preview/cetz-plot` package.
//!
//! Intercepts `plot(<expr>[, <expr>...], <lo>, <hi>)` calls at the chat-bot
//! layer (above /calc and /sym), translates each expression from our
//! "math chat" grammar to a cetz-plot lambda body, and renders through the
//! existing typst compile pipeline.
//!
//! Supported expression grammar (subset of fend/Symbolica syntax,
//! translated to typst's `calc.*` functions):
//!
//!   * variables: any identifier (default plotting variable is `x`)
//!   * numbers: `1`, `2.5`, `-3` — no scientific notation
//!   * constants: `pi`, `e`
//!   * operators: `+ - * / ^` (right-assoc `^` → `calc.pow`)
//!   * unary minus
//!   * math functions: `sin cos tan asin acos atan sinh cosh tanh
//!                      exp ln log sqrt abs pow` (prefixed with `calc.`)
//!   * parentheses
//!
//! Range bounds (`lo`, `hi`) support: a number, `pi`/`-pi`/`e`/`-e`,
//! `N*pi`/`Npi`, `pi/N`/`-pi/N`. Everything else is rejected.

use crate::typst::{compile_doc, RenderError, RenderOpts};

const CETZ_VERSION: &str = "0.4.2";
const CETZ_PLOT_VERSION: &str = "0.1.2";

#[derive(Debug)]
pub enum PlotError {
    NotAPlot,
    Parse(String),
    Render(RenderError),
}

impl std::fmt::Display for PlotError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::NotAPlot => write!(f, "не plot-вызов"),
            Self::Parse(m) => write!(f, "{m}"),
            Self::Render(e) => write!(f, "{e}"),
        }
    }
}

impl std::error::Error for PlotError {}

#[derive(Debug, Clone)]
pub struct PlotRequest {
    pub exprs: Vec<String>, // already translated to typst lambda bodies
    pub lo: f64,
    pub hi: f64,
}

/// Returns true if the input is shaped like a `plot(...)` call (cheap
/// check; full parse only happens inside [`parse`]).
pub fn looks_like_plot(input: &str) -> bool {
    let t = input.trim();
    let Some(rest) = t.strip_prefix("plot") else {
        return false;
    };
    let rest = rest.trim_start();
    rest.starts_with('(') && rest.ends_with(')')
}

/// Parse the outer `plot(...)` shell and the inner arguments. The last two
/// top-level args are the range; everything before is a curve expression.
pub fn parse(input: &str) -> Result<PlotRequest, PlotError> {
    let t = input.trim();
    let rest = t
        .strip_prefix("plot")
        .ok_or(PlotError::NotAPlot)?
        .trim_start();
    let inner = rest
        .strip_prefix('(')
        .and_then(|s| s.strip_suffix(')'))
        .ok_or(PlotError::NotAPlot)?;
    parse_args(inner)
}

/// Parse the comma-separated arg list without the outer `plot(...)`
/// wrapper. Used by the `/plot` command where the verb itself implies
/// the call.
pub fn parse_args(inner: &str) -> Result<PlotRequest, PlotError> {
    if !balanced(inner) {
        return Err(PlotError::Parse("незакрытые скобки".to_string()));
    }

    let parts: Vec<&str> = split_top_commas(inner).into_iter().collect();
    if parts.len() < 3 {
        return Err(PlotError::Parse(
            "нужно как минимум 3 аргумента: <выражение>, <от>, <до>".to_string(),
        ));
    }

    let lo = parse_const(parts[parts.len() - 2])
        .ok_or_else(|| PlotError::Parse(format!("не понял начало: {:?}", parts[parts.len() - 2].trim())))?;
    let hi = parse_const(parts[parts.len() - 1])
        .ok_or_else(|| PlotError::Parse(format!("не понял конец: {:?}", parts[parts.len() - 1].trim())))?;
    if lo >= hi || !lo.is_finite() || !hi.is_finite() {
        return Err(PlotError::Parse(format!(
            "пустой или невалидный диапазон: {lo} … {hi}"
        )));
    }

    let exprs: Result<Vec<String>, PlotError> = parts[..parts.len() - 2]
        .iter()
        .map(|s| translate(s.trim()).map_err(PlotError::Parse))
        .collect();
    let exprs = exprs?;
    if exprs.is_empty() {
        return Err(PlotError::Parse("нет выражения для plot".to_string()));
    }

    Ok(PlotRequest { exprs, lo, hi })
}

pub async fn render_args(input: &str, opts: &RenderOpts) -> Result<Vec<Vec<u8>>, PlotError> {
    let req = parse_args(input)?;
    let doc = assemble(&req);
    compile_doc(&doc, opts).await.map_err(PlotError::Render)
}

/// Build the typst document for a parsed request.
pub fn assemble(req: &PlotRequest) -> String {
    let mut body = String::new();
    for expr in &req.exprs {
        body.push_str(&format!(
            "    plot.add(domain: ({lo}, {hi}), x => {expr})\n",
            lo = req.lo,
            hi = req.hi,
            expr = expr,
        ));
    }
    format!(
        "#set page(width: auto, height: auto, margin: 8pt)\n\
         #import \"@preview/cetz:{cetz}\"\n\
         #import \"@preview/cetz-plot:{cp}\": plot\n\
         \n\
         #cetz.canvas({{\n\
             plot.plot(size: (12, 8), {{\n\
         {body}\
             }})\n\
         }})\n",
        cetz = CETZ_VERSION,
        cp = CETZ_PLOT_VERSION,
        body = body,
    )
}

pub async fn render(input: &str, opts: &RenderOpts) -> Result<Vec<Vec<u8>>, PlotError> {
    let req = parse(input)?;
    let doc = assemble(&req);
    compile_doc(&doc, opts).await.map_err(PlotError::Render)
}

// ---------- constant parsing for range bounds ----------

fn parse_const(s: &str) -> Option<f64> {
    let t = s.trim();
    if let Ok(n) = t.parse::<f64>() {
        return Some(n);
    }
    let pi = std::f64::consts::PI;
    let e = std::f64::consts::E;
    match t {
        "pi" => return Some(pi),
        "-pi" => return Some(-pi),
        "e" => return Some(e),
        "-e" => return Some(-e),
        _ => {}
    }
    // N*pi or Npi
    if let Some(rest) = t.strip_suffix("*pi").or_else(|| t.strip_suffix("pi")) {
        let rest = rest.trim();
        if rest.is_empty() {
            return Some(pi);
        }
        if rest == "-" {
            return Some(-pi);
        }
        return rest.parse::<f64>().ok().map(|n| n * pi);
    }
    // pi/N or -pi/N
    if let Some(rest) = t.strip_prefix("pi/") {
        return rest.parse::<f64>().ok().map(|n| pi / n);
    }
    if let Some(rest) = t.strip_prefix("-pi/") {
        return rest.parse::<f64>().ok().map(|n| -pi / n);
    }
    None
}

// ---------- expression tokenizer + parser + emitter ----------

#[derive(Debug, Clone, PartialEq)]
enum Token {
    Number(f64),
    Ident(String),
    Op(char), // + - * / ^ ( ) ,
}

fn tokenize(s: &str) -> Result<Vec<Token>, String> {
    let mut out = Vec::new();
    let mut chars = s.chars().peekable();
    while let Some(&c) = chars.peek() {
        match c {
            ' ' | '\t' | '\n' => {
                chars.next();
            }
            '0'..='9' | '.' => {
                let mut num = String::new();
                while let Some(&c) = chars.peek() {
                    if c.is_ascii_digit() || c == '.' {
                        num.push(c);
                        chars.next();
                    } else {
                        break;
                    }
                }
                let n: f64 = num
                    .parse()
                    .map_err(|_| format!("плохое число: {num:?}"))?;
                out.push(Token::Number(n));
            }
            'a'..='z' | 'A'..='Z' | '_' => {
                let mut name = String::new();
                while let Some(&c) = chars.peek() {
                    if c.is_alphanumeric() || c == '_' {
                        name.push(c);
                        chars.next();
                    } else {
                        break;
                    }
                }
                out.push(Token::Ident(name));
            }
            '+' | '-' | '*' | '/' | '^' | '(' | ')' | ',' => {
                out.push(Token::Op(c));
                chars.next();
            }
            _ => return Err(format!("неожиданный символ: {c:?}")),
        }
    }
    Ok(out)
}

#[derive(Debug)]
enum Expr {
    Num(f64),
    Var(String),
    Call(String, Vec<Expr>),
    Bin(Box<Expr>, char, Box<Expr>),
    Pow(Box<Expr>, Box<Expr>),
    Neg(Box<Expr>),
}

struct ExprParser<'a> {
    tokens: &'a [Token],
    pos: usize,
}

impl<'a> ExprParser<'a> {
    fn peek(&self) -> Option<&Token> {
        self.tokens.get(self.pos)
    }
    fn advance(&mut self) -> Option<&Token> {
        let t = self.tokens.get(self.pos);
        self.pos += 1;
        t
    }
    fn expect_op(&mut self, c: char) -> Result<(), String> {
        match self.advance() {
            Some(Token::Op(c2)) if *c2 == c => Ok(()),
            other => Err(format!("ожидался {c:?}, нашли {other:?}")),
        }
    }

    fn parse_expr(&mut self) -> Result<Expr, String> {
        let mut left = self.parse_term()?;
        while let Some(Token::Op(c)) = self.peek() {
            if *c == '+' || *c == '-' {
                let op = *c;
                self.advance();
                let right = self.parse_term()?;
                left = Expr::Bin(Box::new(left), op, Box::new(right));
            } else {
                break;
            }
        }
        Ok(left)
    }
    fn parse_term(&mut self) -> Result<Expr, String> {
        let mut left = self.parse_unary()?;
        while let Some(Token::Op(c)) = self.peek() {
            if *c == '*' || *c == '/' {
                let op = *c;
                self.advance();
                let right = self.parse_unary()?;
                left = Expr::Bin(Box::new(left), op, Box::new(right));
            } else {
                break;
            }
        }
        Ok(left)
    }
    // Unary minus has *lower* precedence than `^` (math convention:
    // `-x^2 == -(x^2)`), so `parse_unary` delegates to `parse_factor`
    // which handles `^`.
    fn parse_unary(&mut self) -> Result<Expr, String> {
        if let Some(Token::Op('-')) = self.peek() {
            self.advance();
            let e = self.parse_unary()?;
            Ok(Expr::Neg(Box::new(e)))
        } else if let Some(Token::Op('+')) = self.peek() {
            self.advance();
            self.parse_unary()
        } else {
            self.parse_factor()
        }
    }
    fn parse_factor(&mut self) -> Result<Expr, String> {
        let base = self.parse_atom()?;
        if let Some(Token::Op('^')) = self.peek() {
            self.advance();
            // right-assoc: a^b^c = a^(b^c). Recurse into parse_factor
            // (not parse_unary) so the exponent itself can't start with
            // a unary minus without parens — matches math convention.
            let exp = self.parse_factor()?;
            Ok(Expr::Pow(Box::new(base), Box::new(exp)))
        } else {
            Ok(base)
        }
    }
    fn parse_atom(&mut self) -> Result<Expr, String> {
        match self.advance().cloned() {
            Some(Token::Number(n)) => Ok(Expr::Num(n)),
            Some(Token::Ident(name)) => {
                if let Some(Token::Op('(')) = self.peek() {
                    self.advance();
                    let mut args = Vec::new();
                    // Allow zero args via immediate )
                    if !matches!(self.peek(), Some(Token::Op(')'))) {
                        args.push(self.parse_expr()?);
                        while let Some(Token::Op(',')) = self.peek() {
                            self.advance();
                            args.push(self.parse_expr()?);
                        }
                    }
                    self.expect_op(')')?;
                    Ok(Expr::Call(name, args))
                } else {
                    Ok(Expr::Var(name))
                }
            }
            Some(Token::Op('(')) => {
                let e = self.parse_expr()?;
                self.expect_op(')')?;
                Ok(e)
            }
            other => Err(format!("неожиданный токен: {other:?}")),
        }
    }
}

const MATH_FNS: &[&str] = &[
    "sin", "cos", "tan", "asin", "acos", "atan", "sinh", "cosh", "tanh", "exp", "ln", "log",
    "sqrt", "abs", "pow", "floor", "ceil", "round",
];

fn emit(e: &Expr) -> String {
    match e {
        Expr::Num(n) => {
            if n.fract() == 0.0 && n.abs() < 1e15 {
                format!("{:.0}", n)
            } else {
                format!("{}", n)
            }
        }
        Expr::Var(name) => match name.as_str() {
            "pi" => "calc.pi".to_string(),
            "e" => "calc.e".to_string(),
            _ => name.clone(),
        },
        Expr::Call(name, args) => {
            let is_math = MATH_FNS.iter().any(|m| m == name);
            let prefix = if is_math { "calc." } else { "" };
            let args_str: Vec<String> = args.iter().map(emit).collect();
            format!("{prefix}{name}({})", args_str.join(", "))
        }
        Expr::Bin(l, op, r) => format!("({} {op} {})", emit(l), emit(r)),
        Expr::Pow(l, r) => format!("calc.pow({}, {})", emit(l), emit(r)),
        Expr::Neg(e) => format!("(-{})", emit(e)),
    }
}

fn translate(expr: &str) -> Result<String, String> {
    if expr.is_empty() {
        return Err("пустое выражение".to_string());
    }
    let tokens = tokenize(expr)?;
    let mut p = ExprParser {
        tokens: &tokens,
        pos: 0,
    };
    let e = p.parse_expr()?;
    if p.pos != tokens.len() {
        return Err(format!(
            "лишние токены после позиции {}: {:?}",
            p.pos,
            &tokens[p.pos..]
        ));
    }
    Ok(emit(&e))
}

// ---------- shared helpers ----------

fn balanced(s: &str) -> bool {
    let mut d = 0i32;
    for c in s.chars() {
        match c {
            '(' | '[' | '{' => d += 1,
            ')' | ']' | '}' => {
                d -= 1;
                if d < 0 {
                    return false;
                }
            }
            _ => {}
        }
    }
    d == 0
}

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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn translates_simple_calls() {
        assert_eq!(translate("sin(x)").unwrap(), "calc.sin(x)");
        assert_eq!(translate("cos(x) + sin(x)").unwrap(), "(calc.cos(x) + calc.sin(x))");
    }

    #[test]
    fn translates_powers() {
        assert_eq!(translate("x^2").unwrap(), "calc.pow(x, 2)");
        assert_eq!(translate("x^2 + 1").unwrap(), "(calc.pow(x, 2) + 1)");
        assert_eq!(translate("sin(x)^2").unwrap(), "calc.pow(calc.sin(x), 2)");
        // right-assoc: 2^3^2 = 2^(3^2)
        assert_eq!(
            translate("2^3^2").unwrap(),
            "calc.pow(2, calc.pow(3, 2))"
        );
    }

    #[test]
    fn translates_constants() {
        assert_eq!(translate("pi").unwrap(), "calc.pi");
        assert_eq!(translate("e").unwrap(), "calc.e");
        assert_eq!(translate("2*pi").unwrap(), "(2 * calc.pi)");
    }

    #[test]
    fn translates_unary_minus() {
        assert_eq!(translate("-x").unwrap(), "(-x)");
        assert_eq!(translate("exp(-x^2)").unwrap(), "calc.exp((-calc.pow(x, 2)))");
    }

    #[test]
    fn rejects_garbage() {
        assert!(translate("x @ 2").is_err());
        assert!(translate("(x + ").is_err());
    }

    #[test]
    fn range_const_parser() {
        assert_eq!(parse_const("0").unwrap(), 0.0);
        assert_eq!(parse_const("2.5").unwrap(), 2.5);
        assert!((parse_const("pi").unwrap() - std::f64::consts::PI).abs() < 1e-10);
        assert!((parse_const("2*pi").unwrap() - 2.0 * std::f64::consts::PI).abs() < 1e-10);
        assert!((parse_const("pi/2").unwrap() - std::f64::consts::PI / 2.0).abs() < 1e-10);
        assert!((parse_const("-pi").unwrap() + std::f64::consts::PI).abs() < 1e-10);
        assert_eq!(parse_const("garbage"), None);
    }

    #[test]
    fn parses_single_curve() {
        let req = parse("plot(sin(x), 0, 2*pi)").unwrap();
        assert_eq!(req.exprs, vec!["calc.sin(x)"]);
        assert_eq!(req.lo, 0.0);
        assert!((req.hi - 2.0 * std::f64::consts::PI).abs() < 1e-10);
    }

    #[test]
    fn parses_multi_curve() {
        let req = parse("plot(sin(x), cos(x), -pi, pi)").unwrap();
        assert_eq!(req.exprs, vec!["calc.sin(x)", "calc.cos(x)"]);
    }

    #[test]
    fn rejects_non_plot() {
        assert!(matches!(parse("calc 2+2"), Err(PlotError::NotAPlot)));
        assert!(matches!(parse("plotter(x)"), Err(PlotError::NotAPlot)));
    }

    #[test]
    fn rejects_too_few_args() {
        assert!(matches!(parse("plot(sin(x), 0)"), Err(PlotError::Parse(_))));
    }

    #[test]
    fn rejects_empty_range() {
        assert!(matches!(
            parse("plot(sin(x), 1, 0)"),
            Err(PlotError::Parse(_))
        ));
    }
}
