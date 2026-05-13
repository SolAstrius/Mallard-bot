use rand::Rng;

use crate::dictionaries::{replies_for, CREATURES, EXCEPTIONS, RANDOM_RESPONSES, TEXT_KEYWORDS};
use crate::responses::ResponseType;

pub struct Mallard {
    pub random_answer_rate: u32,
}

impl Mallard {
    pub fn new(random_answer_rate: u32) -> Self {
        Self { random_answer_rate }
    }

    pub fn get_creature(&self) -> &'static str {
        let idx = rand::thread_rng().gen_range(0..CREATURES.len());
        CREATURES[idx]
    }

    pub fn process(&self, saying: &str) -> Option<(String, ResponseType)> {
        if saying.is_empty() {
            return None;
        }
        let reply = self
            .check_basic_saying(saying)
            .or_else(|| self.generate_random_answer())?;
        Some(maybe_scream(reply))
    }

    fn check_basic_saying(&self, saying: &str) -> Option<(String, ResponseType)> {
        if CREATURES.contains(&saying) {
            return None;
        }

        let upper = saying.to_uppercase();

        let mut found: Vec<(&'static str, crate::dictionaries::Keyword)> = Vec::new();
        for (kw_str, kw_enum) in TEXT_KEYWORDS.iter() {
            if upper.contains(kw_str)
                && replies_for(*kw_enum)
                    .map(|v| !v.is_empty())
                    .unwrap_or(false)
            {
                found.push((*kw_str, *kw_enum));
            }
        }

        // Apply exception rules — same semantics as the Python reference.
        found.retain(|(kw_str, _)| {
            let Some((_, excs)) = EXCEPTIONS.iter().find(|(k, _)| k == kw_str) else {
                return true;
            };
            !excs.iter().any(|e| upper.contains(e))
        });

        if found.is_empty() {
            return None;
        }

        let mut rng = rand::thread_rng();
        let (_, chosen_keyword) = found[rng.gen_range(0..found.len())];
        let replies = replies_for(chosen_keyword)?;
        let resp = &replies[rng.gen_range(0..replies.len())];
        Some((resp.text.clone(), resp.response_type))
    }

    fn generate_random_answer(&self) -> Option<(String, ResponseType)> {
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
fn maybe_scream((text, ty): (String, ResponseType)) -> (String, ResponseType) {
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
