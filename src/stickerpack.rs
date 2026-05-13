//! Single-pack sticker management (Bot API 7.2+).
//!
//! With modern Telegram, one set can mix static / animated / video stickers,
//! so we keep a single pack `mallard_pack_by_<bot_username>` per bot. The
//! Python source kept two (image / animated) — the Rust port unifies them.
//!
//! Flow:
//!   1. Try `addStickerToSet`.
//!   2. If the set doesn't exist yet, `createNewStickerSet` with this sticker.
//!   3. Fetch the set, return the freshly-added sticker's `file_id` so the
//!      caller can reply with a pack-resident sticker (just like the Python).
//!   4. Best-effort prune older stickers down to a small ring buffer.

use teloxide::prelude::*;
use teloxide::types::{
    InputFile, InputSticker, Sticker, StickerFormat, StickerSet, StickerType, UserId,
};
use teloxide::ApiError;
use teloxide::RequestError;

use crate::exceptions::{ProcessingError, ProcessingErrorKind};

/// Cap the set so the bot doesn't accumulate forever; Telegram allows up to 120
/// regular stickers per set.
const KEEP_LAST: usize = 50;
const PACK_PREFIX: &str = "mallard_pack";

#[derive(Debug, Clone)]
pub struct StickerPack {
    pub admin_user_id: UserId,
    pub bot_username: String,
}

impl StickerPack {
    pub fn pack_name(&self) -> String {
        format!("{PACK_PREFIX}_by_{}", self.bot_username)
    }

    /// Upload `bytes` into the bot's pack and return the resulting pack-resident
    /// sticker. If the set doesn't exist, create it with this sticker as seed.
    pub async fn add(
        &self,
        bot: &Bot,
        bytes: Vec<u8>,
        format: StickerFormat,
        emoji: &str,
    ) -> Result<Sticker, ProcessingError> {
        let name = self.pack_name();
        let suffix = match format {
            StickerFormat::Static => "png",
            StickerFormat::Video => "webm",
            StickerFormat::Animated => "tgs",
        };
        let file = InputFile::memory(bytes).file_name(format!("sticker.{suffix}"));
        let sticker = InputSticker {
            sticker: file,
            format: format.clone(),
            emoji_list: vec![emoji.to_string()],
            mask_position: None,
            keywords: vec![],
        };

        let add_result = bot
            .add_sticker_to_set(self.admin_user_id, name.clone(), sticker.clone())
            .await;

        if let Err(RequestError::Api(ApiError::InvalidStickersSet)) = &add_result {
            log::info!("creating sticker pack {name}");
            bot.create_new_sticker_set(
                self.admin_user_id,
                name.clone(),
                "Mallard quotes".to_string(),
                [sticker],
            )
            .sticker_type(StickerType::Regular)
            .await
            .map_err(|e| {
                ProcessingError::new(
                    ProcessingErrorKind::Unexpected,
                    format!("createNewStickerSet: {e}"),
                )
            })?;
        } else if let Err(e) = add_result {
            return Err(ProcessingError::new(
                ProcessingErrorKind::Unexpected,
                format!("addStickerToSet: {e}"),
            ));
        }

        let set: StickerSet = bot.get_sticker_set(name.clone()).await.map_err(|e| {
            ProcessingError::new(
                ProcessingErrorKind::Unexpected,
                format!("getStickerSet: {e}"),
            )
        })?;
        let last = set
            .stickers
            .last()
            .cloned()
            .ok_or_else(|| ProcessingError::of(ProcessingErrorKind::Unexpected))?;

        // Best-effort prune: keep the most recent KEEP_LAST stickers; ignore
        // failures because pruning is non-fatal.
        if set.stickers.len() > KEEP_LAST {
            let drop = set.stickers.len() - KEEP_LAST;
            for s in &set.stickers[..drop] {
                let _ = bot.delete_sticker_from_set(s.file.id.0.clone()).await;
            }
        }

        Ok(last)
    }
}
