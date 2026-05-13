//! Static content tables used by the bot.

use std::sync::LazyLock;

use rand::seq::SliceRandom;

/// Full Unicode CLDR emoji list, sourced at compile-time from the `emojis`
/// crate. Used to tag freshly-added stickers via `addStickerToSet`'s
/// `emoji_list` field.
pub static EMOJI_LIST: LazyLock<Vec<&'static str>> =
    LazyLock::new(|| emojis::iter().map(|e| e.as_str()).collect());

pub fn random_emoji() -> &'static str {
    let mut rng = rand::thread_rng();
    EMOJI_LIST.choose(&mut rng).copied().unwrap_or("\u{1F60C}")
}
