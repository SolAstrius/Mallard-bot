//! Static content tables used by the bot.

use std::sync::LazyLock;

use rand::seq::SliceRandom;

/// Telegram-safe emoji list for the `emoji_list` field on `addStickerToSet`.
///
/// We start from the full Unicode CLDR (`emojis::iter()`) but filter to base
/// emoji only — no skin tones, no ZWJ sequences (multi-codepoint glyphs like
/// 👨‍👩‍👧 or 🏳️‍🌈), no keycap sequences. Telegram rejects anything fancier
/// with `Bad Request: invalid sticker emojis`. What's left is ~1k single-glyph
/// emoji that the API consistently accepts.
pub static EMOJI_LIST: LazyLock<Vec<&'static str>> = LazyLock::new(|| {
    emojis::iter()
        .filter(|e| e.skin_tone().is_none())
        .map(|e| e.as_str())
        .filter(|s| is_simple_emoji(s))
        .collect()
});

fn is_simple_emoji(s: &str) -> bool {
    // Reject ZWJ sequences and keycaps; allow base codepoint with optional
    // variation selector (VS-16, U+FE0F) for emoji presentation.
    if s.contains('\u{200D}') || s.contains('\u{20E3}') {
        return false;
    }
    let mut chars = s.chars().filter(|c| *c != '\u{FE0F}');
    let Some(first) = chars.next() else {
        return false;
    };
    if chars.next().is_some() {
        return false;
    }
    // Drop ASCII / digit / # / * leftovers from the keycap base (now without
    // their U+20E3 / VS-16 they'd be plain characters, not emoji).
    !(first.is_ascii() || first.is_ascii_digit())
}

pub fn random_emoji() -> &'static str {
    let mut rng = rand::thread_rng();
    EMOJI_LIST.choose(&mut rng).copied().unwrap_or("\u{1F60C}")
}
