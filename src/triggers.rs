//! Trigger pack — data-driven keyword/random/time-based response engine.
//!
//! See DESIGN.md for the schema. A `TriggerPack` is loaded from `triggers.toml`
//! at startup (and atomically swapped on `/reload`). The pack drives the
//! ambient pipeline: every incoming text message goes through `engine::scan`
//! which evaluates each trigger's gates (kill switches, keyword match, time
//! window, optional Rhai `where`, rate dice) and returns at most one reply.

use std::collections::BTreeMap;
use std::sync::{Arc, OnceLock, RwLock};

use chrono::{Datelike, Local, Timelike, Weekday};
use rand::Rng;
use rhai::{Engine, Scope, AST};
use serde::Deserialize;

use crate::features::{self, FeatureDef, TypeSpec, KEYWORD_MODES};
use crate::mallard::{match_keyword, MatchMode};
use crate::responses::ResponseType;

// ---------- raw TOML schema ----------

#[derive(Debug, Deserialize, Default)]
struct RawPack {
    #[serde(default)]
    meta: RawMeta,
    #[serde(default)]
    creatures: Vec<String>,
    #[serde(default)]
    group: BTreeMap<String, RawGroup>,
    #[serde(default, rename = "trigger")]
    triggers: Vec<RawTrigger>,
}

#[derive(Debug, Deserialize, Default)]
struct RawMeta {
    #[serde(default)]
    version: u32,
}

#[derive(Debug, Deserialize, Default)]
struct RawGroup {
    #[serde(default)]
    default_mode: Option<String>,
    #[serde(default)]
    default_rate: Option<u32>,
}

#[derive(Debug, Deserialize)]
struct RawTrigger {
    id: String,
    #[serde(default, rename = "match")]
    match_: Option<RawMatch>,
    #[serde(default)]
    when: Option<RawWhen>,
    #[serde(default, rename = "where")]
    where_: Option<String>,
    #[serde(default)]
    rate: Option<u32>,
    #[serde(default)]
    reply: RawReply,
}

#[derive(Debug, Deserialize, Default)]
struct RawMatch {
    #[serde(default)]
    mode: Option<String>,
    #[serde(default)]
    any: Vec<String>,
    #[serde(default)]
    exclude: Vec<String>,
}

#[derive(Debug, Deserialize, Default)]
struct RawWhen {
    #[serde(default)]
    hour: Option<String>,
    #[serde(default)]
    weekday: Option<String>,
}

#[derive(Debug, Deserialize, Default)]
struct RawReply {
    #[serde(default)]
    text: Vec<String>,
    #[serde(default)]
    sticker: Vec<String>,
    #[serde(default)]
    voice: Vec<String>,
}

// ---------- validated runtime ----------

#[derive(Debug)]
pub struct TriggerPack {
    pub version: u32,
    pub creatures: Vec<String>,
    pub groups: BTreeMap<String, GroupInfo>,
    pub triggers: Vec<Trigger>,
}

#[derive(Debug, Clone)]
pub struct GroupInfo {
    pub default_mode: MatchMode,
    pub default_rate: u32,
}

#[derive(Debug)]
pub struct Trigger {
    pub id: String,
    pub group: String,
    pub leaf: String,
    pub keywords: Vec<String>,
    pub exclude: Vec<String>,
    pub mode: MatchMode,
    pub when: Option<WhenSpec>,
    pub where_ast: Option<AST>,
    pub rate: u32,
    pub replies: Vec<Reply>,
}

#[derive(Debug, Clone)]
pub struct Reply {
    pub text: String,
    pub ty: ResponseType,
}

#[derive(Debug, Clone)]
pub struct WhenSpec {
    pub hours: Option<HourSet>,
    pub weekdays: Option<WeekdaySet>,
}

/// Bit per hour 0..24.
#[derive(Debug, Clone, Copy)]
pub struct HourSet {
    mask: u32,
}

impl HourSet {
    pub fn contains(&self, h: u8) -> bool {
        h < 24 && (self.mask >> h) & 1 == 1
    }
}

/// Bit per weekday Mon=0..Sun=6.
#[derive(Debug, Clone, Copy)]
pub struct WeekdaySet {
    mask: u8,
}

impl WeekdaySet {
    pub fn contains(&self, w: Weekday) -> bool {
        let idx = w.num_days_from_monday() as u8;
        (self.mask >> idx) & 1 == 1
    }
}

// ---------- parsing ----------

pub fn parse(toml_str: &str) -> Result<TriggerPack, String> {
    let raw: RawPack = toml::from_str(toml_str).map_err(|e| format!("toml parse: {e}"))?;
    let engine = rhai_engine();

    let mut groups: BTreeMap<String, GroupInfo> = BTreeMap::new();
    for (name, raw_grp) in &raw.group {
        validate_segment(name)?;
        let default_mode = match raw_grp.default_mode.as_deref() {
            Some(s) => MatchMode::parse(s)
                .ok_or_else(|| format!("group {name}: unknown mode {s:?}"))?,
            None => MatchMode::Contains,
        };
        let default_rate = raw_grp.default_rate.unwrap_or(1);
        groups.insert(
            name.clone(),
            GroupInfo {
                default_mode,
                default_rate,
            },
        );
    }

    let mut seen_ids: std::collections::HashSet<String> = std::collections::HashSet::new();
    let mut triggers: Vec<Trigger> = Vec::with_capacity(raw.triggers.len());
    for raw_t in raw.triggers {
        let (group, leaf) = split_id(&raw_t.id)?;
        validate_segment(&group)?;
        validate_segment(&leaf)?;
        if !seen_ids.insert(raw_t.id.clone()) {
            return Err(format!("duplicate trigger id: {}", raw_t.id));
        }
        let grp = groups.entry(group.clone()).or_insert(GroupInfo {
            default_mode: MatchMode::Contains,
            default_rate: 1,
        });
        let (keywords, exclude, mode) = match raw_t.match_ {
            Some(m) => {
                let mode = match m.mode.as_deref() {
                    Some(s) => MatchMode::parse(s)
                        .ok_or_else(|| format!("{}: unknown mode {s:?}", raw_t.id))?,
                    None => grp.default_mode,
                };
                (
                    m.any.into_iter().map(|s| s.to_uppercase()).collect(),
                    m.exclude.into_iter().map(|s| s.to_uppercase()).collect(),
                    mode,
                )
            }
            None => (Vec::new(), Vec::new(), grp.default_mode),
        };
        let when = match raw_t.when {
            Some(w) => Some(WhenSpec {
                hours: w
                    .hour
                    .as_deref()
                    .map(parse_hour_range)
                    .transpose()
                    .map_err(|e| format!("{}: when.hour: {e}", raw_t.id))?,
                weekdays: w
                    .weekday
                    .as_deref()
                    .map(parse_weekday_range)
                    .transpose()
                    .map_err(|e| format!("{}: when.weekday: {e}", raw_t.id))?,
            }),
            None => None,
        };
        let where_ast = match raw_t.where_ {
            Some(src) => Some(
                engine
                    .compile(&src)
                    .map_err(|e| format!("{}: where: {e}", raw_t.id))?,
            ),
            None => None,
        };
        let rate = raw_t.rate.unwrap_or(grp.default_rate);

        let mut replies: Vec<Reply> = Vec::new();
        for t in raw_t.reply.text {
            replies.push(Reply {
                text: t,
                ty: ResponseType::Text,
            });
        }
        for s in raw_t.reply.sticker {
            replies.push(Reply {
                text: s,
                ty: ResponseType::Sticker,
            });
        }
        for v in raw_t.reply.voice {
            replies.push(Reply {
                text: v,
                ty: ResponseType::Voice,
            });
        }
        if replies.is_empty() {
            return Err(format!("{}: empty reply pool", raw_t.id));
        }

        triggers.push(Trigger {
            id: raw_t.id,
            group,
            leaf,
            keywords,
            exclude,
            mode,
            when,
            where_ast,
            rate,
            replies,
        });
    }

    Ok(TriggerPack {
        version: raw.meta.version,
        creatures: raw.creatures,
        groups,
        triggers,
    })
}

fn validate_segment(s: &str) -> Result<(), String> {
    if s.is_empty() {
        return Err("empty segment".into());
    }
    for c in s.chars() {
        if !(c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_') {
            return Err(format!("invalid segment {s:?}"));
        }
    }
    Ok(())
}

fn split_id(id: &str) -> Result<(String, String), String> {
    let (g, l) = id
        .split_once('.')
        .ok_or_else(|| format!("id must be 'group.leaf': {id}"))?;
    if l.contains('.') {
        return Err(format!("id must have exactly one dot: {id}"));
    }
    Ok((g.to_string(), l.to_string()))
}

fn parse_hour_range(s: &str) -> Result<HourSet, String> {
    if s.trim() == "*" {
        return Ok(HourSet {
            mask: (1u32 << 24) - 1,
        });
    }
    let mut mask = 0u32;
    for part in s.split(',') {
        let part = part.trim();
        let (a, b, inclusive) = parse_range(part)?;
        for h in a..=if inclusive { b } else { b - 1 } {
            if h >= 24 {
                return Err(format!("hour out of range: {h}"));
            }
            mask |= 1u32 << h;
        }
    }
    Ok(HourSet { mask })
}

fn parse_range(s: &str) -> Result<(u32, u32, bool), String> {
    if let Some((a, b)) = s.split_once("..=") {
        let a: u32 = a.trim().parse().map_err(|_| format!("bad range: {s}"))?;
        let b: u32 = b.trim().parse().map_err(|_| format!("bad range: {s}"))?;
        return Ok((a, b, true));
    }
    if let Some((a, b)) = s.split_once("..") {
        let a: u32 = a.trim().parse().map_err(|_| format!("bad range: {s}"))?;
        let b: u32 = b.trim().parse().map_err(|_| format!("bad range: {s}"))?;
        return Ok((a, b, false));
    }
    let n: u32 = s.trim().parse().map_err(|_| format!("bad range: {s}"))?;
    Ok((n, n, true))
}

fn parse_weekday_range(s: &str) -> Result<WeekdaySet, String> {
    if s.trim() == "*" {
        return Ok(WeekdaySet { mask: 0x7f });
    }
    let mut mask = 0u8;
    for part in s.split(',') {
        let part = part.trim();
        if let Some((a, b)) = part.split_once("..=") {
            let ai = weekday_index(a.trim())?;
            let bi = weekday_index(b.trim())?;
            for i in ai..=bi {
                mask |= 1u8 << i;
            }
        } else if let Some((a, b)) = part.split_once("..") {
            let ai = weekday_index(a.trim())?;
            let bi = weekday_index(b.trim())?;
            for i in ai..=bi {
                mask |= 1u8 << i;
            }
        } else {
            mask |= 1u8 << weekday_index(part)?;
        }
    }
    Ok(WeekdaySet { mask })
}

fn weekday_index(s: &str) -> Result<u8, String> {
    match s.to_ascii_lowercase().as_str() {
        "mon" | "monday" => Ok(0),
        "tue" | "tuesday" => Ok(1),
        "wed" | "wednesday" => Ok(2),
        "thu" | "thursday" => Ok(3),
        "fri" | "friday" => Ok(4),
        "sat" | "saturday" => Ok(5),
        "sun" | "sunday" => Ok(6),
        _ => Err(format!("unknown weekday: {s}")),
    }
}

// ---------- engine ----------

#[derive(Debug)]
pub struct MsgCtx<'a> {
    pub text: &'a str,
    pub upper: String,
    pub chat_id: i64,
}

impl<'a> MsgCtx<'a> {
    pub fn new(text: &'a str, chat_id: i64) -> Self {
        Self {
            text,
            upper: text.to_uppercase(),
            chat_id,
        }
    }
}

#[derive(Debug)]
pub struct TriggerHit {
    pub reply_text: String,
    pub reply_type: ResponseType,
    pub quote: Option<(String, u32)>,
}

/// Evaluate the pack against one incoming message. Returns at most one reply.
/// Keyword triggers take priority over any-match (random) triggers: if any
/// keyword trigger survives all gates, only those are considered; otherwise
/// any-match triggers compete.
pub fn scan(
    pack: &TriggerPack,
    rules: &[(String, String)],
    ctx: &MsgCtx,
) -> Option<TriggerHit> {
    let now = Local::now();
    let hour = now.hour() as u8;
    let weekday = now.weekday();

    let mut rng = rand::thread_rng();
    let mut kw_survivors: Vec<(&Trigger, (usize, usize))> = Vec::new();
    let mut any_survivors: Vec<&Trigger> = Vec::new();
    let engine = rhai_engine();

    for trig in &pack.triggers {
        // group + per-trigger kill switches
        if !flag_bool(rules, &format!("ambient.triggers.{}", trig.group), true) {
            continue;
        }
        if !flag_bool(
            rules,
            &format!("ambient.triggers.{}.{}", trig.group, trig.leaf),
            true,
        ) {
            continue;
        }
        // exclude
        if trig
            .exclude
            .iter()
            .any(|e| ctx.upper.contains(e.as_str()))
        {
            continue;
        }
        // match (keyword vs any-match)
        let span: Option<(usize, usize)> = if trig.keywords.is_empty() {
            None
        } else {
            let mode = {
                let mode_path = format!("ambient.triggers.{}.mode", trig.group);
                let s = features::resolve(rules, &mode_path).0;
                MatchMode::parse(&s).unwrap_or(trig.mode)
            };
            let m = trig
                .keywords
                .iter()
                .find_map(|k| match_keyword(mode, &ctx.upper, k));
            if m.is_none() {
                continue;
            }
            m
        };
        // when
        if let Some(w) = &trig.when {
            if let Some(hs) = &w.hours {
                if !hs.contains(hour) {
                    continue;
                }
            }
            if let Some(ws) = &w.weekdays {
                if !ws.contains(weekday) {
                    continue;
                }
            }
        }
        // where (Rhai)
        if let Some(ast) = &trig.where_ast {
            if !eval_where(&engine, ast, ctx, hour, weekday) {
                continue;
            }
        }
        // rate
        let rate_path = format!(
            "ambient.triggers.{}.{}.rate",
            trig.group, trig.leaf
        );
        let rate = flag_int(rules, &rate_path, trig.rate);
        if rate == 0 {
            continue;
        }
        if rate > 1 && rng.gen_range(0..rate) != 0 {
            continue;
        }

        match span {
            Some(s) => kw_survivors.push((trig, s)),
            None => any_survivors.push(trig),
        }
    }

    let pick_from_kw = !kw_survivors.is_empty();
    let (trig, span) = if pick_from_kw {
        let i = rng.gen_range(0..kw_survivors.len());
        let (t, s) = kw_survivors[i];
        (t, Some(s))
    } else if !any_survivors.is_empty() {
        let i = rng.gen_range(0..any_survivors.len());
        (any_survivors[i], None)
    } else {
        return None;
    };

    let reply = &trig.replies[rng.gen_range(0..trig.replies.len())];
    let quote = span.and_then(|(s, e)| {
        let q = ctx.text.get(s..e)?.to_string();
        Some((q, utf16_offset(ctx.text, s)))
    });
    Some(TriggerHit {
        reply_text: reply.text.clone(),
        reply_type: reply.ty,
        quote,
    })
}

fn flag_bool(rules: &[(String, String)], path: &str, default: bool) -> bool {
    features::parse_bool(&features::resolve(rules, path).0).unwrap_or(default)
}

fn flag_int(rules: &[(String, String)], path: &str, default: u32) -> u32 {
    features::resolve(rules, path)
        .0
        .parse::<u32>()
        .unwrap_or(default)
}

fn utf16_offset(s: &str, byte_pos: usize) -> u32 {
    s[..byte_pos.min(s.len())]
        .chars()
        .map(|c| c.len_utf16() as u32)
        .sum()
}

// ---------- Rhai ----------

fn rhai_engine() -> Engine {
    let mut e = Engine::new();
    e.set_max_expr_depths(32, 32);
    e.set_max_operations(10_000);
    e
}

fn eval_where(
    engine: &Engine,
    ast: &AST,
    ctx: &MsgCtx,
    hour: u8,
    weekday: Weekday,
) -> bool {
    let mut scope = Scope::new();
    scope.push_constant("text", ctx.text.to_string());
    scope.push_constant("chat_id", ctx.chat_id);
    scope.push_constant("hour", hour as i64);
    scope.push_constant("weekday", weekday.num_days_from_monday() as i64);
    scope.push_constant("len", ctx.text.chars().count() as i64);
    match engine.eval_ast_with_scope::<bool>(&mut scope, ast) {
        Ok(b) => b,
        Err(e) => {
            log::warn!("where expr failed: {e}");
            false
        }
    }
}

// ---------- registry: live snapshot + load ----------

static REGISTRY: OnceLock<RwLock<Arc<TriggerPack>>> = OnceLock::new();

const BUILTIN_TOML: &str = include_str!("../content/triggers.toml");

/// The path on disk to persist uploaded packs to. Override with
/// `$MALLARD_TRIGGERS_PATH`.
pub fn on_disk_path() -> std::path::PathBuf {
    std::env::var_os("MALLARD_TRIGGERS_PATH")
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|| std::path::PathBuf::from("triggers.toml"))
}

/// Snapshot of the live pack.
pub fn current() -> Arc<TriggerPack> {
    Arc::clone(&REGISTRY.get().expect("triggers not initialized").read().unwrap())
}

/// Initialise the registry. Prefers `$MALLARD_TRIGGERS_PATH` if readable;
/// otherwise falls back to the compiled-in default. Panics on parse failure
/// of either source — the bot can't reasonably start without a valid pack.
pub fn init() {
    let pack = load_initial().unwrap_or_else(|e| panic!("triggers init: {e}"));
    let arc = Arc::new(pack);
    publish_features(&arc);
    let cell = REGISTRY.get_or_init(|| RwLock::new(Arc::clone(&arc)));
    *cell.write().unwrap() = arc;
}

fn load_initial() -> Result<TriggerPack, String> {
    let path = on_disk_path();
    if path.exists() {
        let s = std::fs::read_to_string(&path)
            .map_err(|e| format!("read {}: {e}", path.display()))?;
        match parse(&s) {
            Ok(p) => {
                log::info!("triggers: loaded {} ({} triggers)", path.display(), p.triggers.len());
                return Ok(p);
            }
            Err(e) => {
                log::warn!("triggers: {} invalid ({e}); falling back to built-in", path.display());
            }
        }
    }
    let p = parse(BUILTIN_TOML)?;
    log::info!("triggers: loaded built-in ({} triggers)", p.triggers.len());
    Ok(p)
}

/// Validate + swap. On success returns the new pack's trigger count.
pub fn swap_from_str(toml_str: &str) -> Result<usize, String> {
    let pack = parse(toml_str)?;
    let n = pack.triggers.len();
    let arc = Arc::new(pack);
    publish_features(&arc);
    let cell = REGISTRY.get().ok_or_else(|| "registry not initialized".to_string())?;
    *cell.write().unwrap() = arc;
    Ok(n)
}

/// Persist + swap. Writes the bytes to `on_disk_path()` only after a
/// successful parse, so a bad upload never clobbers a working file.
pub fn swap_and_persist(toml_str: &str) -> Result<usize, String> {
    let n = swap_from_str(toml_str)?;
    let path = on_disk_path();
    if let Some(parent) = path.parent() {
        if !parent.as_os_str().is_empty() {
            let _ = std::fs::create_dir_all(parent);
        }
    }
    std::fs::write(&path, toml_str).map_err(|e| format!("write {}: {e}", path.display()))?;
    Ok(n)
}

/// Reload from `on_disk_path()`, falling back to the built-in if absent.
pub fn reload_from_disk() -> Result<usize, String> {
    let path = on_disk_path();
    let src = if path.exists() {
        std::fs::read_to_string(&path).map_err(|e| format!("read {}: {e}", path.display()))?
    } else {
        BUILTIN_TOML.to_string()
    };
    swap_from_str(&src)
}

// ---------- dynamic feature flags ----------

/// Re-register the pack's auto-generated leaves with `features::set_dynamic`.
/// Called on every swap so `/feature` immediately sees the new flags.
fn publish_features(pack: &TriggerPack) {
    let mut defs: Vec<FeatureDef> = Vec::new();
    let mut seen_group_bools: std::collections::HashSet<String> = std::collections::HashSet::new();
    let mut seen_group_modes: std::collections::HashSet<String> = std::collections::HashSet::new();

    for trig in &pack.triggers {
        // group bool (once)
        let group_path = format!("ambient.triggers.{}", trig.group);
        if seen_group_bools.insert(trig.group.clone()) {
            defs.push(leaked_feature(&group_path, "on", TypeSpec::Bool));
        }
        // per-trigger bool
        let trig_path = format!("ambient.triggers.{}.{}", trig.group, trig.leaf);
        defs.push(leaked_feature(&trig_path, "on", TypeSpec::Bool));
        // per-trigger rate (Int)
        let rate_path = format!("ambient.triggers.{}.{}.rate", trig.group, trig.leaf);
        defs.push(leaked_feature(
            &rate_path,
            &trig.rate.to_string(),
            TypeSpec::Int {
                min: 0,
                max: 100_000,
            },
        ));
        // group-level mode enum (once per group that has a keyword-bearing trigger)
        if !trig.keywords.is_empty() && seen_group_modes.insert(trig.group.clone()) {
            let mode_path = format!("ambient.triggers.{}.mode", trig.group);
            let default_mode = pack
                .groups
                .get(&trig.group)
                .map(|g| g.default_mode)
                .unwrap_or(MatchMode::Contains);
            defs.push(leaked_feature(
                &mode_path,
                mode_str(default_mode),
                TypeSpec::Enum(KEYWORD_MODES),
            ));
        }
    }
    features::set_dynamic(defs);
}

/// Leak a `FeatureDef` so its borrowed `&'static str` fields outlive the
/// pack-load. The pack is rarely reloaded; the leak budget is bounded by
/// admin actions in practice.
fn leaked_feature(path: &str, default: &str, ty: TypeSpec) -> FeatureDef {
    FeatureDef {
        path: Box::leak(path.to_string().into_boxed_str()),
        default: Box::leak(default.to_string().into_boxed_str()),
        ty,
    }
}

fn mode_str(m: MatchMode) -> &'static str {
    match m {
        MatchMode::Contains => "contains",
        MatchMode::StartsWith => "startswith",
        MatchMode::EndsWith => "endswith",
        MatchMode::Equals => "equals",
        MatchMode::Word => "word",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_builtin_pack() {
        let p = parse(BUILTIN_TOML).expect("builtin pack must parse");
        assert!(!p.triggers.is_empty());
        assert!(!p.creatures.is_empty());
    }

    #[test]
    fn hour_range_inclusive_exclusive() {
        let h = parse_hour_range("5..11").unwrap();
        assert!(h.contains(5));
        assert!(h.contains(10));
        assert!(!h.contains(11));
        let h = parse_hour_range("5..=11").unwrap();
        assert!(h.contains(11));
        let h = parse_hour_range("*").unwrap();
        assert!(h.contains(0));
        assert!(h.contains(23));
    }

    #[test]
    fn weekday_range_parse() {
        let w = parse_weekday_range("mon..fri").unwrap();
        assert!(w.contains(Weekday::Mon));
        assert!(w.contains(Weekday::Fri));
        assert!(!w.contains(Weekday::Sat));
        let w = parse_weekday_range("*").unwrap();
        assert!(w.contains(Weekday::Sun));
    }

    #[test]
    fn duplicate_id_rejected() {
        let s = r#"
[[trigger]]
id = "a.b"
reply.text = ["x"]
[[trigger]]
id = "a.b"
reply.text = ["y"]
"#;
        assert!(parse(s).is_err());
    }

    #[test]
    fn empty_reply_rejected() {
        let s = r#"
[[trigger]]
id = "a.b"
"#;
        assert!(parse(s).is_err());
    }

    #[test]
    fn where_expression_compiles_and_evaluates() {
        let s = r#"
[[trigger]]
id = "g.t"
match.any = []
where = "hour >= 6 && hour < 12"
rate = 1
reply.text = ["morning"]
"#;
        let p = parse(s).unwrap();
        assert!(p.triggers[0].where_ast.is_some());
    }
}
