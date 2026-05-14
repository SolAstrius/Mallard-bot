//! Hierarchical, pattern-matched per-chat feature flags — now typed.
//!
//! Each flag has a dotted path (`nix.npkg`, `util.theme.dark`,
//! `ambient.keywords.memes.mode`) and a registered [`TypeSpec`]: `Bool`,
//! `Int { min, max }`, or `Enum(&[..])`. Storage is a single TEXT column per
//! rule; the registry tells callers how to interpret it.
//!
//! Rules are stored per-chat as either an exact path or a trailing-wildcard
//! pattern (`util.*`, `util.time.*`, `*`). Resolution picks the rule with the
//! longest literal prefix; if the stored value doesn't parse as the leaf's
//! type, that rule is treated as missing and resolution falls back to the
//! in-code default. (This lets a wildcard like `ambient.keywords.memes.* off`
//! cleanly skip a sibling `…mode` enum leaf without erroring.)
//!
//! Writes via `/feature` accept the same exact/wildcard forms plus two
//! shorthands: a category name desugars to `<name>.*` at write time, and
//! `<prefix>.**` (only with `reset`) recursively deletes every rule beneath
//! that prefix.

use crate::db::Db;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TypeSpec {
    Bool,
    Int { min: i64, max: i64 },
    Enum(&'static [&'static str]),
}

#[derive(Clone, Copy, Debug)]
pub struct FeatureDef {
    pub path: &'static str,
    pub default: &'static str,
    pub ty: TypeSpec,
}

impl FeatureDef {
    /// True if `s` is a syntactically-valid value for this flag's type.
    pub fn validates(&self, s: &str) -> bool {
        match self.ty {
            TypeSpec::Bool => parse_bool(s).is_some(),
            TypeSpec::Int { min, max } => s
                .parse::<i64>()
                .ok()
                .map(|n| n >= min && n <= max)
                .unwrap_or(false),
            TypeSpec::Enum(variants) => variants.contains(&s),
        }
    }
}

pub fn parse_bool(s: &str) -> Option<bool> {
    match s.to_ascii_lowercase().as_str() {
        "on" | "true" | "yes" | "1" | "enable" => Some(true),
        "off" | "false" | "no" | "0" | "disable" => Some(false),
        _ => None,
    }
}

pub const KEYWORD_MODES: &[&str] = &["contains", "startswith", "endswith", "equals", "word"];

/// Registered leaves. Adding a new command means adding a row here and a
/// matching `is_enabled` / `get_*` check at the call site.
#[rustfmt::skip]
pub const FEATURES: &[FeatureDef] = &[
    // ---- commands ----
    FeatureDef { path: "fun.roll",       default: "on",  ty: TypeSpec::Bool },
    FeatureDef { path: "fun.pick",       default: "on",  ty: TypeSpec::Bool },
    FeatureDef { path: "fun.horoscope",  default: "on",  ty: TypeSpec::Bool },
    FeatureDef { path: "tea.cha",        default: "on",  ty: TypeSpec::Bool },
    FeatureDef { path: "tea.sip",        default: "on",  ty: TypeSpec::Bool },
    FeatureDef { path: "nix.npkg",       default: "off", ty: TypeSpec::Bool },
    FeatureDef { path: "nix.nopt",       default: "off", ty: TypeSpec::Bool },
    FeatureDef { path: "nix.nixwhere",   default: "off", ty: TypeSpec::Bool },
    FeatureDef { path: "nix.nchan",      default: "off", ty: TypeSpec::Bool },
    FeatureDef { path: "nix.nflake",     default: "off", ty: TypeSpec::Bool },
    FeatureDef { path: "util.typst",     default: "on",  ty: TypeSpec::Bool },
    FeatureDef { path: "util.latex",     default: "on",  ty: TypeSpec::Bool },
    FeatureDef { path: "util.math",      default: "on",  ty: TypeSpec::Bool },
    FeatureDef { path: "util.calc",      default: "on",  ty: TypeSpec::Bool },
    FeatureDef { path: "util.sym",       default: "on",  ty: TypeSpec::Bool },
    FeatureDef { path: "util.plot",      default: "on",  ty: TypeSpec::Bool },
    FeatureDef { path: "util.theme.dark", default: "off", ty: TypeSpec::Bool },

    // ---- ambient: typst/latex/math auto-render ----
    FeatureDef { path: "ambient.typst.fenced", default: "off", ty: TypeSpec::Bool },
    FeatureDef { path: "ambient.latex.fenced", default: "off", ty: TypeSpec::Bool },
    FeatureDef { path: "ambient.math.dollar",  default: "off", ty: TypeSpec::Bool },

    // ---- ambient: text-keyword triggers ----
    // Per-keyword bools. To turn off the whole group at once:
    //   /feature ambient.keywords.<group>.* off
    // Per-group mode picks how a keyword is matched against incoming text.
    FeatureDef { path: "ambient.keywords.creatures.kva",   default: "on", ty: TypeSpec::Bool },
    FeatureDef { path: "ambient.keywords.creatures.kar",   default: "on", ty: TypeSpec::Bool },
    FeatureDef { path: "ambient.keywords.creatures.krya",  default: "on", ty: TypeSpec::Bool },
    FeatureDef { path: "ambient.keywords.creatures.hryu",  default: "on", ty: TypeSpec::Bool },
    FeatureDef { path: "ambient.keywords.creatures.miu",   default: "on", ty: TypeSpec::Bool },
    FeatureDef { path: "ambient.keywords.creatures.mav",   default: "on", ty: TypeSpec::Bool },
    FeatureDef { path: "ambient.keywords.creatures.gaing", default: "on", ty: TypeSpec::Bool },
    FeatureDef { path: "ambient.keywords.creatures.woof",  default: "on", ty: TypeSpec::Bool },
    FeatureDef { path: "ambient.keywords.creatures.mode",  default: "contains", ty: TypeSpec::Enum(KEYWORD_MODES) },

    // memes is the noisy group — defaults to a tighter match mode, and the
    // catchphrase-spammiest leaf (DaNu = "да ладно" / "да ну") ships off.
    FeatureDef { path: "ambient.keywords.memes.danu",  default: "off", ty: TypeSpec::Bool },
    FeatureDef { path: "ambient.keywords.memes.oyvse", default: "on",  ty: TypeSpec::Bool },
    FeatureDef { path: "ambient.keywords.memes.goyda", default: "on",  ty: TypeSpec::Bool },
    FeatureDef { path: "ambient.keywords.memes.what",  default: "on",  ty: TypeSpec::Bool },
    FeatureDef { path: "ambient.keywords.memes.us",    default: "on",  ty: TypeSpec::Bool },
    FeatureDef { path: "ambient.keywords.memes.blin",  default: "on",  ty: TypeSpec::Bool },
    FeatureDef { path: "ambient.keywords.memes.mode",  default: "equals", ty: TypeSpec::Enum(KEYWORD_MODES) },

    FeatureDef { path: "ambient.keywords.food.pelmen", default: "on", ty: TypeSpec::Bool },
    FeatureDef { path: "ambient.keywords.food.borsch", default: "on", ty: TypeSpec::Bool },
    FeatureDef { path: "ambient.keywords.food.chai",   default: "on", ty: TypeSpec::Bool },
    FeatureDef { path: "ambient.keywords.food.bread",  default: "on", ty: TypeSpec::Bool },
    FeatureDef { path: "ambient.keywords.food.mode",   default: "contains", ty: TypeSpec::Enum(KEYWORD_MODES) },

    FeatureDef { path: "ambient.keywords.cozy.kiss",          default: "on", ty: TypeSpec::Bool },
    FeatureDef { path: "ambient.keywords.cozy.sad",           default: "on", ty: TypeSpec::Bool },
    FeatureDef { path: "ambient.keywords.cozy.tired",         default: "on", ty: TypeSpec::Bool },
    FeatureDef { path: "ambient.keywords.cozy.hungry",        default: "on", ty: TypeSpec::Bool },
    FeatureDef { path: "ambient.keywords.cozy.cold",          default: "on", ty: TypeSpec::Bool },
    FeatureDef { path: "ambient.keywords.cozy.sleepy",        default: "on", ty: TypeSpec::Bool },
    FeatureDef { path: "ambient.keywords.cozy.hug",           default: "on", ty: TypeSpec::Bool },
    FeatureDef { path: "ambient.keywords.cozy.morning_cozy",  default: "on", ty: TypeSpec::Bool },
    FeatureDef { path: "ambient.keywords.cozy.brat",          default: "on", ty: TypeSpec::Bool },
    FeatureDef { path: "ambient.keywords.cozy.cozy",          default: "on", ty: TypeSpec::Bool },
    FeatureDef { path: "ambient.keywords.cozy.bunny",         default: "on", ty: TypeSpec::Bool },
    FeatureDef { path: "ambient.keywords.cozy.mode",          default: "contains", ty: TypeSpec::Enum(KEYWORD_MODES) },

    FeatureDef { path: "ambient.keywords.misc.arch", default: "on", ty: TypeSpec::Bool },
    FeatureDef { path: "ambient.keywords.misc.mode", default: "contains", ty: TypeSpec::Enum(KEYWORD_MODES) },

    // ---- ambient: random + scream ----
    FeatureDef { path: "ambient.random", default: "on", ty: TypeSpec::Bool },
    FeatureDef { path: "ambient.scream", default: "on", ty: TypeSpec::Bool },
];

pub fn def_of(flag: &str) -> Option<&'static FeatureDef> {
    FEATURES.iter().find(|f| f.path == flag)
}

pub fn default_of(flag: &str) -> &'static str {
    def_of(flag).map(|f| f.default).unwrap_or("on")
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

/// Resolve `flag` against `rules`. Returns the (string) value and the
/// winning rule pattern (or `None` if the default applied). Rules whose
/// value doesn't validate against `flag`'s declared type are skipped — a
/// wildcard rule with a bool value won't shadow a sibling enum leaf.
pub fn resolve<'a>(rules: &'a [(String, String)], flag: &str) -> (String, Option<&'a str>) {
    let def = def_of(flag);
    let mut best: Option<(&'a str, &'a str)> = None;
    for (pat, val) in rules {
        if !pattern_matches(pat, flag) {
            continue;
        }
        if let Some(d) = def {
            if !d.validates(val) {
                continue;
            }
        }
        let s = specificity(pat);
        match best {
            None => best = Some((pat.as_str(), val.as_str())),
            Some((cur, _)) if s > specificity(cur) => best = Some((pat.as_str(), val.as_str())),
            _ => {}
        }
    }
    match best {
        Some((pat, val)) => (val.to_string(), Some(pat)),
        None => (default_of(flag).to_string(), None),
    }
}

/// Load the chat's rules from the DB and resolve a single flag as bool.
/// On DB error or type-mismatch, log and fall back to the in-code default
/// — never block a command on a flag-lookup failure.
pub async fn is_enabled(db: &Db, chat: i64, flag: &str) -> bool {
    match db.feature_rules_for_chat(chat).await {
        Ok(rules) => parse_bool(&resolve(&rules, flag).0).unwrap_or_else(|| {
            parse_bool(default_of(flag)).unwrap_or(true)
        }),
        Err(e) => {
            log::warn!("feature_rules_for_chat({chat}): {e} — defaulting {flag}");
            parse_bool(default_of(flag)).unwrap_or(true)
        }
    }
}

/// Load and resolve a flag as a string (used for enum-typed flags).
pub async fn get_str(db: &Db, chat: i64, flag: &str) -> String {
    match db.feature_rules_for_chat(chat).await {
        Ok(rules) => resolve(&rules, flag).0,
        Err(e) => {
            log::warn!("feature_rules_for_chat({chat}): {e} — defaulting {flag}");
            default_of(flag).to_string()
        }
    }
}

#[derive(Clone, Copy, Debug)]
pub enum Action {
    Set(&'static str), // canonical "on"/"off" for bools; for typed values use SetValue.
    Reset,
}

/// Outcome of trying to parse the trailing token(s) of `/feature` as a write.
/// `None` means "query", `Some(write)` means we have an intent.
#[derive(Debug)]
pub enum WriteIntent {
    Bool(bool),
    /// For non-bool flags: pass the raw token through; the resolver
    /// validates against each target leaf's TypeSpec.
    Literal(String),
    Reset,
}

impl WriteIntent {
    /// Recognize reset/clear keywords; bool tokens; or anything else as
    /// a literal value to be type-checked per-leaf.
    pub fn parse(s: &str) -> Self {
        let lower = s.to_ascii_lowercase();
        match lower.as_str() {
            "reset" | "default" | "clear" | "unset" => Self::Reset,
            _ => match parse_bool(s) {
                Some(b) => Self::Bool(b),
                None => Self::Literal(s.to_string()),
            },
        }
    }

    pub fn verb(&self) -> &'static str {
        match self {
            Self::Bool(true) => "включён",
            Self::Bool(false) => "выключен",
            Self::Literal(_) => "установлен",
            Self::Reset => "сброшен",
        }
    }

    /// Stored value if this intent applies to `def`. Returns `None` when
    /// the intent doesn't match the leaf's type.
    pub fn stored_for(&self, def: &FeatureDef) -> Option<String> {
        match (self, def.ty) {
            (Self::Bool(b), TypeSpec::Bool) => Some(if *b { "on" } else { "off" }.to_string()),
            (Self::Bool(_), _) => None,
            (Self::Literal(s), _) if def.validates(s) => Some(s.clone()),
            (Self::Literal(_), _) => None,
            (Self::Reset, _) => None, // handled by clear path
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

/// Validate + normalize a user-supplied pattern.
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

/// Render a stored value with a hint of its type — for the overview UI.
/// `value` may be either the raw stored string (when there's an explicit
/// rule) or the resolved default.
pub fn pretty_value(def: &FeatureDef, value: &str) -> String {
    match def.ty {
        TypeSpec::Bool => match parse_bool(value) {
            Some(true) => "\u{2705}".to_string(),
            Some(false) => "\u{274C}".to_string(),
            None => format!("?{value}"),
        },
        TypeSpec::Int { .. } | TypeSpec::Enum(_) => value.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pattern_match_basics() {
        assert!(pattern_matches("nix.npkg", "nix.npkg"));
        assert!(!pattern_matches("nix.npkg", "nix.nopt"));
        assert!(pattern_matches("nix.*", "nix.npkg"));
        assert!(pattern_matches("nix.*", "nix"));
        assert!(!pattern_matches("nix.*", "tea.cha"));
        assert!(pattern_matches("*", "anything.you.want"));
    }

    #[test]
    fn resolve_picks_most_specific_typed() {
        let rules = vec![
            ("nix.*".to_string(), "on".to_string()),
            ("nix.npkg".to_string(), "off".to_string()),
        ];
        assert_eq!(resolve(&rules, "nix.npkg").0, "off");
        assert_eq!(resolve(&rules, "nix.nopt").0, "on");
    }

    #[test]
    fn resolve_skips_type_mismatched_wildcard() {
        // bool-y wildcard rule shadows a sibling enum leaf? It must not.
        let rules = vec![("ambient.keywords.memes.*".to_string(), "off".to_string())];
        // memes.mode is enum; "off" isn't a valid mode → falls back to default.
        let (v, by) = resolve(&rules, "ambient.keywords.memes.mode");
        assert_eq!(v, "equals");
        assert!(by.is_none());
        // memes.danu is bool; "off" is fine.
        assert_eq!(resolve(&rules, "ambient.keywords.memes.danu").0, "off");
    }

    #[test]
    fn resolve_falls_back_to_default() {
        assert_eq!(resolve(&[], "fun.roll").0, "on");
        assert_eq!(resolve(&[], "nix.npkg").0, "off");
        assert_eq!(resolve(&[], "ambient.keywords.memes.mode").0, "equals");
    }

    #[test]
    fn write_intent_parses() {
        assert!(matches!(WriteIntent::parse("on"), WriteIntent::Bool(true)));
        assert!(matches!(WriteIntent::parse("off"), WriteIntent::Bool(false)));
        assert!(matches!(WriteIntent::parse("reset"), WriteIntent::Reset));
        match WriteIntent::parse("endswith") {
            WriteIntent::Literal(s) => assert_eq!(s, "endswith"),
            _ => panic!("expected Literal"),
        }
    }

    #[test]
    fn stored_for_validates_type() {
        let bool_def = def_of("fun.roll").unwrap();
        assert_eq!(
            WriteIntent::Bool(true).stored_for(bool_def).as_deref(),
            Some("on")
        );
        assert!(WriteIntent::Literal("endswith".into()).stored_for(bool_def).is_none());

        let mode_def = def_of("ambient.keywords.memes.mode").unwrap();
        assert_eq!(
            WriteIntent::Literal("endswith".into())
                .stored_for(mode_def)
                .as_deref(),
            Some("endswith")
        );
        assert!(WriteIntent::Literal("nonsense".into())
            .stored_for(mode_def)
            .is_none());
        // bool intent on enum leaf → rejected
        assert!(WriteIntent::Bool(true).stored_for(mode_def).is_none());
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
        assert!(normalize_pattern("Nix.NPKG").is_err());
        assert!(normalize_pattern("nix..npkg").is_err());
        assert!(normalize_pattern("").is_err());
    }
}
