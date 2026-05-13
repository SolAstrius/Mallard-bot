//! Single-pack sticker management (Bot API 7.2+).
//!
//! With modern Telegram, one set can mix static / animated / video stickers,
//! so we keep one regular pack and one custom-emoji pack per bot:
//!
//!   * `mallard_pack_by_<bot_username>`        — sticker replies to /snap, /qva
//!   * `mallard_emoji_pack_by_<bot_username>`  — custom emoji from /snap j, /qva j
//!
//! The Python source kept two regular packs (image / animated) and never
//! built a custom-emoji set at all. The Rust port unifies the regular pack
//! and adds a custom-emoji pack so `j` results are usable as Premium custom
//! emoji instead of just being uploaded as documents the user has to feed
//! through fStikBot.
//!
//! Flow per sticker:
//!   1. Try `addStickerToSet`.
//!   2. If the set doesn't exist yet, `createNewStickerSet` with this sticker.
//!   3. Fetch the set, return the freshly-added sticker so the caller can
//!      reply with a pack-resident sticker (just like the Python).
//!   4. Best-effort prune older stickers down to a small ring buffer.

use teloxide::prelude::*;
use teloxide::types::{
    InputFile, InputSticker, Sticker, StickerFormat, StickerSet, StickerType, UserId,
};
use teloxide::ApiError;
use teloxide::RequestError;

use crate::exceptions::{ProcessingError, ProcessingErrorKind};

/// Cap the regular set so the bot doesn't accumulate forever; Telegram allows
/// up to 120 regular stickers and up to 200 custom emoji per set.
const KEEP_LAST_STICKERS: usize = 100;
const KEEP_LAST_EMOJI: usize = 100;
const REGULAR_PREFIX: &str = "mallard_pack";
const EMOJI_PREFIX: &str = "mallard_emoji_pack";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PackKind {
    Regular,
    CustomEmoji,
}

#[derive(Debug, Clone)]
pub struct StickerPack {
    pub admin_user_id: UserId,
    pub bot_username: String,
}

impl StickerPack {
    fn name_for(&self, kind: PackKind) -> String {
        let prefix = match kind {
            PackKind::Regular => REGULAR_PREFIX,
            PackKind::CustomEmoji => EMOJI_PREFIX,
        };
        format!("{prefix}_by_{}", self.bot_username)
    }

    fn title_for(kind: PackKind) -> &'static str {
        match kind {
            PackKind::Regular => "Mallard quotes",
            PackKind::CustomEmoji => "Mallard custom emoji",
        }
    }

    fn keep_last(kind: PackKind) -> usize {
        match kind {
            PackKind::Regular => KEEP_LAST_STICKERS,
            PackKind::CustomEmoji => KEEP_LAST_EMOJI,
        }
    }

    /// Upload `bytes` into the bot's regular pack and return the resulting
    /// pack-resident sticker.
    pub async fn add(
        &self,
        bot: &Bot,
        bytes: Vec<u8>,
        format: StickerFormat,
        emoji: &str,
    ) -> Result<Sticker, ProcessingError> {
        self.add_to(bot, PackKind::Regular, bytes, format, emoji)
            .await
    }

    /// Upload `bytes` into the bot's custom-emoji pack and return the resulting
    /// custom-emoji sticker (Premium users can use it as a custom emoji).
    pub async fn add_emoji(
        &self,
        bot: &Bot,
        bytes: Vec<u8>,
        format: StickerFormat,
        emoji: &str,
    ) -> Result<Sticker, ProcessingError> {
        self.add_to(bot, PackKind::CustomEmoji, bytes, format, emoji)
            .await
    }

    pub fn pack_url(&self, kind: PackKind) -> String {
        format!("https://t.me/addstickers/{}", self.name_for(kind))
    }

    async fn add_to(
        &self,
        bot: &Bot,
        kind: PackKind,
        bytes: Vec<u8>,
        format: StickerFormat,
        emoji: &str,
    ) -> Result<Sticker, ProcessingError> {
        self.add_to_with_emojis(bot, kind, bytes, format, vec![emoji.to_string()])
            .await
    }

    /// Same as `add_to` but lets the caller specify the full emoji list — used
    /// by `/import` to preserve the source sticker's emoji tags.
    pub async fn add_to_with_emojis(
        &self,
        bot: &Bot,
        kind: PackKind,
        bytes: Vec<u8>,
        format: StickerFormat,
        emojis: Vec<String>,
    ) -> Result<Sticker, ProcessingError> {
        let name = self.name_for(kind);
        let suffix = match format {
            StickerFormat::Static => "png",
            StickerFormat::Video => "webm",
            StickerFormat::Animated => "tgs",
        };
        let file = InputFile::memory(bytes).file_name(format!("sticker.{suffix}"));
        let emoji_list = if emojis.is_empty() {
            vec!["\u{1F60C}".to_string()]
        } else {
            emojis
        };
        let sticker = InputSticker {
            sticker: file,
            format: format.clone(),
            emoji_list,
            mask_position: None,
            keywords: vec![],
        };

        let add_result = bot
            .add_sticker_to_set(self.admin_user_id, name.clone(), sticker.clone())
            .await;

        if let Err(RequestError::Api(ApiError::InvalidStickersSet)) = &add_result {
            log::info!("creating sticker pack {name} ({:?})", kind);
            let sticker_type = match kind {
                PackKind::Regular => StickerType::Regular,
                PackKind::CustomEmoji => StickerType::CustomEmoji,
            };
            bot.create_new_sticker_set(
                self.admin_user_id,
                name.clone(),
                Self::title_for(kind).to_string(),
                [sticker],
            )
            .sticker_type(sticker_type)
            .await
            .map_err(|e| {
                ProcessingError::new(
                    ProcessingErrorKind::Unexpected,
                    format!("createNewStickerSet ({name}): {e}"),
                )
            })?;
        } else if let Err(e) = add_result {
            return Err(ProcessingError::new(
                ProcessingErrorKind::Unexpected,
                format!("addStickerToSet ({name}): {e}"),
            ));
        }

        let set: StickerSet = bot.get_sticker_set(name.clone()).await.map_err(|e| {
            ProcessingError::new(
                ProcessingErrorKind::Unexpected,
                format!("getStickerSet ({name}): {e}"),
            )
        })?;
        let last = set
            .stickers
            .last()
            .cloned()
            .ok_or_else(|| ProcessingError::of(ProcessingErrorKind::Unexpected))?;

        let keep = Self::keep_last(kind);
        if set.stickers.len() > keep {
            let drop = set.stickers.len() - keep;
            for s in &set.stickers[..drop] {
                let _ = bot.delete_sticker_from_set(s.file.id.0.clone()).await;
            }
        }

        Ok(last)
    }
}
