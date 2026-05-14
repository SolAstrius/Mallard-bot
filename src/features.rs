//! Hierarchical, pattern-matched per-chat feature flags.
//!
//! Names are dotted paths (`nix.npkg`, `util.time.tz`). Rules are stored
//! per-chat as either an exact path or a trailing-wildcard pattern
//! (`util.*`, `util.time.*`, `*`). Resolution picks the rule with the
//! longest literal prefix; if none matches, the in-code default wins.
//!
//! Writes via `/feature` accept the same exact/wildcard forms plus two
//! shorthands: a category name desugars to `<name>.*` at write time, and
//! `<prefix>.**` (only with `reset`) recursively deletes every rule
//! beneath that prefix.

use crate::db::Db;

#[derive(Clone, Copy, Debug)]
pub struct FeatureDef {
    pub path: &'static str,
    pub default: bool,
}

/// Registered leaves. Adding a new command means adding a row here and a
/// matching `is_enabled` check at the call site.
pub const FEATURES: &[FeatureDef] = &[
    FeatureDef { path: "fun.roll",      default: true  },
    FeatureDef { path: "fun.pick",      default: true  },
    FeatureDef { path: "fun.horoscope", default: true  },
    FeatureDef { path: "tea.cha",       default: true  },
    FeatureDef { path: "tea.sip",       default: true  },
    FeatureDef { path: "nix.npkg",      default: false },
    FeatureDef { path: "nix.nopt",      default: false },
    FeatureDef { path: "nix.nixwhere",  default: false },
    FeatureDef { path: "nix.nchan",     default: false },
    FeatureDef { path: "nix.nflake",    default: false },
    FeatureDef { path: "util.typst",    default: true  },
];

pub fn default_of(flag: &str) -> bool {
    FEATURES
        .iter()
        .find(|f| f.path == flag)
        .map(|f| f.default)
        .unwrap_or(true)
}

/// Top-level segments in registration order, deduped.
pub fn categories() -> Vec<&'static str> {
    let mut out: Vec<&'static str> = Vec::new();
    for f in FEATURES {
        let top = f.path.split('.').next().unwrap_or("");
        if !top.is_empty() && !out.contains(&top) {
            out.push(top);
        }
    }
    out
}

/// `pat` matches `flag` if it equals `flag` exactly, or if it is
/// `<prefix>.*` and `flag == prefix` or `flag` starts with `<prefix>.`.
/// Bare `*` matches everything.
pub fn pattern_matches(pat: &str, flag: &str) -> bool {
    if pat == "*" {
        return true;
    }
    if let Some(prefix) = pat.strip_suffix(".*") {
        flag == prefix || flag.starts_with(&format!("{prefix}."))
    } else {
        pat == flag
    }
}

/// Number of literal segments before any `*`. Higher = more specific.
pub fn specificity(pat: &str) -> usize {
    pat.split('.').take_while(|s| *s != "*").count()
}

/// Resolve `flag` against `rules`. Returns the value and the winning rule
/// pattern (or `None` if the default applied).
pub fn resolve<'a>(rules: &'a [(String, bool)], flag: &str) -> (bool, Option<&'a str>) {
    let mut best: Option<(&'a str, bool)> = None;
    for (pat, val) in rules {
        if !pattern_matches(pat, flag) {
            continue;
        }
        let s = specificity(pat);
        match best {
            None => best = Some((pat.as_str(), *val)),
            Some((cur, _)) if s > specificity(cur) => best = Some((pat.as_str(), *val)),
            _ => {}
        }
    }
    match best {
        Some((pat, val)) => (val, Some(pat)),
        None => (default_of(flag), None),
    }
}

/// Load the chat's rules from the DB and resolve a single flag. On DB error,
/// log and fall back to the in-code default — never block a command on a
/// flag-lookup failure.
pub async fn is_enabled(db: &Db, chat: i64, flag: &str) -> bool {
    match db.feature_rules_for_chat(chat).await {
        Ok(rules) => resolve(&rules, flag).0,
        Err(e) => {
            log::warn!("feature_rules_for_chat({chat}): {e} — defaulting {flag}");
            default_of(flag)
        }
    }
}

#[derive(Clone, Copy, Debug)]
pub enum Action {
    On,
    Off,
    Reset,
}

impl Action {
    pub fn from_token(s: &str) -> Option<Self> {
        match s.to_lowercase().as_str() {
            "on" | "true" | "yes" | "1" | "enable" => Some(Self::On),
            "off" | "false" | "no" | "0" | "disable" => Some(Self::Off),
            "reset" | "default" | "clear" | "unset" => Some(Self::Reset),
            _ => None,
        }
    }

    pub fn verb(&self) -> &'static str {
        match self {
            Self::On => "включён",
            Self::Off => "выключен",
            Self::Reset => "сброшен",
        }
    }
}

/// User-supplied pattern, normalized.
#[derive(Debug, Clone)]
pub enum Pattern {
    /// Exact leaf path, e.g. `nix.npkg`. Stored verbatim.
    Exact(String),
    /// Trailing-wildcard rule, e.g. `nix.*` or `*`. Stored verbatim.
    Wildcard(String),
    /// `<prefix>.**` — not a stored rule; means "delete every rule whose
    /// pattern starts with `<prefix>.`". Only valid with `reset`.
    Recursive(String),
}

impl Pattern {
    pub fn display(&self) -> String {
        match self {
            Self::Exact(s) | Self::Wildcard(s) => s.clone(),
            Self::Recursive(prefix) => format!("{prefix}.**"),
        }
    }

    /// The string form this pattern is stored as in `chat_features`. `None`
    /// for `Recursive` — that's a prefix-delete operation, not a rule.
    pub fn stored(&self) -> Option<&str> {
        match self {
            Self::Exact(s) | Self::Wildcard(s) => Some(s.as_str()),
            Self::Recursive(_) => None,
        }
    }
}

/// Validate + normalize a user-supplied pattern. The input may be:
///   - an exact registered leaf path           → `Exact`
///   - `<prefix>.*` covering registered leaves → `Wildcard`
///   - bare `*`                                → `Wildcard("*")`
///   - a category/prefix without trailing `.*` → desugars to `Wildcard(<prefix>.*)`
///   - `<prefix>.**`                           → `Recursive(<prefix>)`
pub fn normalize_pattern(raw: &str) -> Result<Pattern, String> {
    let input = raw.trim();
    if input.is_empty() {
        return Err("пустой шаблон".to_string());
    }

    if input == "*" {
        return Ok(Pattern::Wildcard("*".to_string()));
    }

    if let Some(prefix) = input.strip_suffix(".**") {
        validate_segments(prefix)?;
        ensure_prefix_covers(prefix)?;
        return Ok(Pattern::Recursive(prefix.to_string()));
    }

    if let Some(prefix) = input.strip_suffix(".*") {
        validate_segments(prefix)?;
        ensure_prefix_covers(prefix)?;
        return Ok(Pattern::Wildcard(format!("{prefix}.*")));
    }

    // No wildcard suffix. Either an exact known leaf, or a category prefix
    // that should desugar to <prefix>.*.
    if FEATURES.iter().any(|f| f.path == input) {
        return Ok(Pattern::Exact(input.to_string()));
    }
    if FEATURES
        .iter()
        .any(|f| f.path.starts_with(&format!("{input}.")))
    {
        validate_segments(input)?;
        return Ok(Pattern::Wildcard(format!("{input}.*")));
    }

    Err(format!("неизвестный флаг или префикс: {input}"))
}

fn validate_segments(s: &str) -> Result<(), String> {
    if s.is_empty() {
        return Err("пустой префикс".to_string());
    }
    for seg in s.split('.') {
        if seg.is_empty() {
            return Err(format!("пустой сегмент в {s}"));
        }
        if !seg
            .chars()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_')
        {
            return Err(format!("неверный сегмент: {seg}"));
        }
    }
    Ok(())
}

fn ensure_prefix_covers(prefix: &str) -> Result<(), String> {
    let dotted = format!("{prefix}.");
    if FEATURES
        .iter()
        .any(|f| f.path == prefix || f.path.starts_with(&dotted))
    {
        Ok(())
    } else {
        Err(format!("ничего не подходит под {prefix}.*"))
    }
}

/// Leaves whose path matches `pat` (exact or wildcard). Used to scope
/// `/feature <prefix>` queries to a subtree.
pub fn leaves_matching(pat: &Pattern) -> Vec<&'static FeatureDef> {
    FEATURES
        .iter()
        .filter(|f| match pat {
            Pattern::Exact(s) => f.path == s,
            Pattern::Wildcard(s) => pattern_matches(s, f.path),
            Pattern::Recursive(prefix) => {
                let dotted = format!("{prefix}.");
                f.path.starts_with(&dotted) || f.path == prefix
            }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pattern_match_basics() {
        assert!(pattern_matches("nix.npkg", "nix.npkg"));
        assert!(!pattern_matches("nix.npkg", "nix.nopt"));
        assert!(pattern_matches("nix.*", "nix.npkg"));
        assert!(pattern_matches("nix.*", "nix")); // edge: nix.* matches "nix" itself
        assert!(!pattern_matches("nix.*", "tea.cha"));
        assert!(pattern_matches("*", "anything.you.want"));
        assert!(pattern_matches("util.time.*", "util.time.tz"));
        assert!(!pattern_matches("util.time.*", "util.numbers.conv"));
    }

    #[test]
    fn specificity_ordering() {
        assert!(specificity("util.time.tz") > specificity("util.time.*"));
        assert!(specificity("util.time.*") > specificity("util.*"));
        assert!(specificity("util.*") > specificity("*"));
    }

    #[test]
    fn resolve_picks_most_specific() {
        let rules = vec![
            ("nix.*".to_string(), true),
            ("nix.npkg".to_string(), false),
        ];
        // exact beats wildcard
        assert!(!resolve(&rules, "nix.npkg").0);
        // wildcard wins when no exact rule exists
        assert!(resolve(&rules, "nix.nopt").0);
    }

    #[test]
    fn resolve_falls_back_to_default() {
        // fun.roll has default = true
        assert!(resolve(&[], "fun.roll").0);
        // nix.npkg has default = false
        assert!(!resolve(&[], "nix.npkg").0);
    }

    #[test]
    fn normalize_accepts_exact_and_glob() {
        assert!(matches!(
            normalize_pattern("nix.npkg").unwrap(),
            Pattern::Exact(s) if s == "nix.npkg"
        ));
        assert!(matches!(
            normalize_pattern("nix.*").unwrap(),
            Pattern::Wildcard(s) if s == "nix.*"
        ));
        // category shorthand desugars
        assert!(matches!(
            normalize_pattern("nix").unwrap(),
            Pattern::Wildcard(s) if s == "nix.*"
        ));
        assert!(matches!(
            normalize_pattern("nix.**").unwrap(),
            Pattern::Recursive(s) if s == "nix"
        ));
        assert!(matches!(
            normalize_pattern("*").unwrap(),
            Pattern::Wildcard(s) if s == "*"
        ));
    }

    #[test]
    fn normalize_rejects_unknown_and_bad_syntax() {
        assert!(normalize_pattern("util.bogus").is_err());
        assert!(normalize_pattern("util.time").is_err()); // no util.time.* registered yet
        assert!(normalize_pattern("Nix.NPKG").is_err()); // case-sensitive
        assert!(normalize_pattern("nix..npkg").is_err()); // empty segment
        assert!(normalize_pattern("").is_err());
    }
}
