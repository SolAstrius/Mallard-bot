//! Thin wrapper that survives from the legacy Mallard struct.
//!
//! All keyword/random response logic now lives in `crate::triggers`. This
//! module keeps the `MatchMode` + `match_keyword` primitives (used by the
//! trigger engine) and the `Mallard` struct (used for `/iam` lookups and
//! the outgoing-reply scream mutator).

use rand::Rng;

use crate::responses::ResponseType;
use crate::triggers;

pub struct Mallard;

/// Match mode for keyword scanning. Per-group, set via
/// `ambient.triggers.<group>.mode`. Default `contains` preserves the
/// long-standing substring behaviour.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MatchMode {
    Contains,
    StartsWith,
    EndsWith,
    Equals,
    Word,
}

impl MatchMode {
    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "contains" => Some(Self::Contains),
            "startswith" => Some(Self::StartsWith),
            "endswith" => Some(Self::EndsWith),
            "equals" => Some(Self::Equals),
            "word" => Some(Self::Word),
            _ => None,
        }
    }
}

/// Test `kw` against `upper` under `mode`, returning the matched byte range
/// in `upper` (and equivalently in the original-case text, since byte
/// offsets are preserved for the alphabets we accept).
pub fn match_keyword(mode: MatchMode, upper: &str, kw: &str) -> Option<(usize, usize)> {
    let kw_len = kw.len();
    match mode {
        MatchMode::Contains => upper.find(kw).map(|i| (i, i + kw_len)),
        MatchMode::StartsWith => {
            let lead = upper.len() - upper.trim_start().len();
            let rest = &upper[lead..];
            if rest.starts_with(kw) {
                Some((lead, lead + kw_len))
            } else {
                None
            }
        }
        MatchMode::EndsWith => {
            let tail_end = upper.trim_end().len();
            let head = &upper[..tail_end];
            if head.ends_with(kw) && tail_end >= kw_len {
                Some((tail_end - kw_len, tail_end))
            } else {
                None
            }
        }
        MatchMode::Equals => {
            if upper.trim() == kw {
                let lead = upper.len() - upper.trim_start().len();
                Some((lead, lead + kw_len))
            } else {
                None
            }
        }
        MatchMode::Word => {
            let mut from = 0;
            while let Some(rel) = upper[from..].find(kw) {
                let abs = from + rel;
                let end = abs + kw_len;
                let left_ok = abs == 0
                    || upper[..abs]
                        .chars()
                        .next_back()
                        .is_none_or(|c| !c.is_alphanumeric());
                let right_ok = end == upper.len()
                    || upper[end..]
                        .chars()
                        .next()
                        .is_none_or(|c| !c.is_alphanumeric());
                if left_ok && right_ok {
                    return Some((abs, end));
                }
                from = abs + kw.chars().next().map(|c| c.len_utf8()).unwrap_or(1);
                if from >= upper.len() {
                    break;
                }
            }
            None
        }
    }
}

impl Mallard {
    pub fn new() -> Self {
        Self
    }

    /// Pick one of the "/iam" introduction phrases from the live trigger pack.
    /// Returns `None` if the pack has no `creatures` set (it must — `init` runs
    /// before this is called).
    pub fn get_creature(&self) -> String {
        let pack = triggers::current();
        if pack.creatures.is_empty() {
            return "Я уточка! Кря-кря!".to_string();
        }
        let i = rand::thread_rng().gen_range(0..pack.creatures.len());
        pack.creatures[i].clone()
    }
}

impl Default for Mallard {
    fn default() -> Self {
        Self::new()
    }
}

/// 1-in-`rate`: the duck briefly loses it. Text replies only — uppercases the
/// whole thing, stretches the final letter, slaps on "!!!".
pub fn maybe_scream(rate: u32, (text, ty): (String, ResponseType)) -> (String, ResponseType) {
    if ty != ResponseType::Text || rate == 0 {
        return (text, ty);
    }
    let mut rng = rand::thread_rng();
    if rate > 1 && rng.gen_range(0..rate) != 0 {
        return (text, ty);
    }
    (scream(&text, &mut rng), ty)
}

fn scream(text: &str, rng: &mut impl Rng) -> String {
    let upper = text.to_uppercase();
    let chars: Vec<char> = upper.chars().collect();
    let last_alpha = chars.iter().rposition(|c| c.is_alphabetic());
    let stretched = match last_alpha {
        Some(idx) => {
            let extra = rng.gen_range(6..12);
            let mut out: String = chars[..=idx].iter().collect();
            for _ in 0..extra {
                out.push(chars[idx]);
            }
            out.extend(chars[idx + 1..].iter());
            out
        }
        None => upper,
    };
    format!("{stretched}!!!")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mode_contains() {
        let s = "НУ ДА ЛАДНО";
        let (a, b) = match_keyword(MatchMode::Contains, s, "ДА ЛАДНО").unwrap();
        assert_eq!(&s[a..b], "ДА ЛАДНО");
    }

    #[test]
    fn mode_word_anchors() {
        let s = "КАР ТОЧКА";
        assert!(match_keyword(MatchMode::Word, s, "КАР").is_some());
        let s2 = "КАРТА";
        assert!(match_keyword(MatchMode::Word, s2, "КАР").is_none());
    }
}
