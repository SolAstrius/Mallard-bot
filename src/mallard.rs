use rand::Rng;

use crate::dictionaries::{replies_for, Keyword, CREATURES, EXCEPTIONS, RANDOM_RESPONSES, TEXT_KEYWORDS};
use crate::responses::ResponseType;

pub struct Mallard {
    pub random_answer_rate: u32,
}

/// One keyword that matched somewhere in a message. Byte offsets are into
/// the uppercased form of the text; since every alphabet used in
/// [`TEXT_KEYWORDS`] is length-preserving under `to_uppercase` (Cyrillic,
/// Latin), the same offsets slice the original-case string too.
#[derive(Debug, Clone, Copy)]
pub struct KeywordMatch {
    pub keyword: Keyword,
    pub byte_start: usize,
    pub byte_end: usize,
}

/// Match mode for keyword scanning. Per-group, set via
/// `ambient.keywords.<group>.mode`. Default `contains` preserves the
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

/// Scan `saying` for every keyword that matches in *contains* mode, applying
/// the existing [`EXCEPTIONS`] table. The caller is responsible for filtering
/// these candidates further by per-keyword feature flag and per-group
/// match-mode — `scan_keywords` itself is mode-agnostic so a single pass
/// gives the call site everything it needs to make policy decisions.
pub fn scan_keywords(saying: &str) -> Vec<KeywordMatch> {
    if saying.is_empty() {
        return Vec::new();
    }
    let upper = saying.to_uppercase();
    let mut out: Vec<KeywordMatch> = Vec::new();
    for (kw_str, kw_enum) in TEXT_KEYWORDS.iter() {
        if let Some((start, end)) = upper.find(kw_str).map(|i| (i, i + kw_str.len())) {
            let exception_hit = EXCEPTIONS
                .iter()
                .find(|(k, _)| k == kw_str)
                .map(|(_, exc)| exc.iter().any(|e| upper.contains(e)))
                .unwrap_or(false);
            if exception_hit {
                continue;
            }
            if replies_for(*kw_enum)
                .map(|v| v.is_empty())
                .unwrap_or(true)
            {
                continue;
            }
            out.push(KeywordMatch {
                keyword: *kw_enum,
                byte_start: start,
                byte_end: end,
            });
        }
    }
    out
}

/// Pick a random reply text for `kw` from its registered pool.
pub fn pick_reply(kw: Keyword) -> Option<(String, ResponseType)> {
    let replies = replies_for(kw)?;
    if replies.is_empty() {
        return None;
    }
    let mut rng = rand::thread_rng();
    let r = &replies[rng.gen_range(0..replies.len())];
    Some((r.text.clone(), r.response_type))
}

impl Mallard {
    pub fn new(random_answer_rate: u32) -> Self {
        Self { random_answer_rate }
    }

    pub fn get_creature(&self) -> &'static str {
        let idx = rand::thread_rng().gen_range(0..CREATURES.len());
        CREATURES[idx]
    }

    /// Default-rules pipeline: scan for a keyword match in `contains` mode,
    /// then fall back to the random roulette. Uses no chat config — this is
    /// the legacy "no feature flags" behaviour, exposed for tests and as a
    /// safe fallback when DB lookup fails. Production callers use
    /// `pick_ambient_response` in `bot.rs` which threads chat rules through.
    pub fn process(&self, saying: &str) -> Option<(String, ResponseType)> {
        if saying.is_empty() {
            return None;
        }
        // Mirror the old behaviour: don't react when the message is exactly
        // one of our own creature replies.
        if CREATURES.contains(&saying) {
            return None;
        }
        let candidates = scan_keywords(saying);
        let from_keywords = if !candidates.is_empty() {
            use rand::Rng;
            let mut rng = rand::thread_rng();
            let cand = candidates[rng.gen_range(0..candidates.len())];
            pick_reply(cand.keyword)
        } else {
            None
        };
        let reply = from_keywords.or_else(|| self.generate_random_answer())?;
        Some(maybe_scream(reply))
    }

    /// 1-in-`random_answer_rate` shot at an unprompted random reply. Returns
    /// `None` when the roulette doesn't fire or the rate is zero.
    pub fn generate_random_answer(&self) -> Option<(String, ResponseType)> {
        if self.random_answer_rate == 0 {
            return None;
        }
        let mut rng = rand::thread_rng();
        if rng.gen_range(0..self.random_answer_rate) != 0 {
            return None;
        }
        let r = &RANDOM_RESPONSES[rng.gen_range(0..RANDOM_RESPONSES.len())];
        Some((r.text.clone(), r.response_type))
    }
}

/// 1-in-300: the duck briefly loses it. Text replies only — uppercases the
/// whole thing, stretches the final letter, slaps on "!!!".
pub fn maybe_scream((text, ty): (String, ResponseType)) -> (String, ResponseType) {
    if ty != ResponseType::Text {
        return (text, ty);
    }
    let mut rng = rand::thread_rng();
    if rng.gen_range(0..300) != 0 {
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
    fn mode_startswith_requires_leading_position() {
        assert_eq!(
            match_keyword(MatchMode::StartsWith, "  ДА ЛАДНО ТЕБЕ", "ДА ЛАДНО"),
            Some((2, 2 + "ДА ЛАДНО".len()))
        );
        assert_eq!(
            match_keyword(MatchMode::StartsWith, "НУ ДА ЛАДНО", "ДА ЛАДНО"),
            None
        );
    }

    #[test]
    fn mode_endswith_anchors_trailing() {
        let s = "ОН СКАЗАЛ ДА ЛАДНО";
        let m = match_keyword(MatchMode::EndsWith, s, "ДА ЛАДНО").unwrap();
        assert_eq!(&s[m.0..m.1], "ДА ЛАДНО");
        assert_eq!(
            match_keyword(MatchMode::EndsWith, "ДА ЛАДНО ТЕБЕ", "ДА ЛАДНО"),
            None
        );
    }

    #[test]
    fn mode_equals_whole_message() {
        assert!(match_keyword(MatchMode::Equals, "ДА ЛАДНО", "ДА ЛАДНО").is_some());
        assert!(match_keyword(MatchMode::Equals, "  ДА ЛАДНО  ", "ДА ЛАДНО").is_some());
        assert!(match_keyword(MatchMode::Equals, "НУ ДА ЛАДНО", "ДА ЛАДНО").is_none());
    }

    #[test]
    fn mode_word_respects_boundaries() {
        // ПЕЛЬМЕН substring inside ПЕЛЬМЕНЬ should still match: trailing
        // letter is alphanumeric — word mode rejects it.
        assert!(match_keyword(MatchMode::Word, "ПЕЛЬМЕНЬ", "ПЕЛЬМЕН").is_none());
        // Standalone with surrounding spaces matches.
        assert!(match_keyword(MatchMode::Word, "А ПЕЛЬМЕН?", "ПЕЛЬМЕН").is_some());
    }
}
