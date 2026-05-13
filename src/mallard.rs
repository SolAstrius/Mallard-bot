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
        if let Some(reply) = self.check_basic_saying(saying) {
            return Some(reply);
        }
        self.generate_random_answer()
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
