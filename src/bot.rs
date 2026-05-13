//! teloxide wiring for the Mallard bot.

use std::sync::Arc;

use teloxide::net::Download;
use teloxide::prelude::*;
use teloxide::types::{
    FileId, InlineQueryResult, InlineQueryResultArticle, InputFile, InputMessageContent,
    InputMessageContentText, MediaKind, MessageKind, ParseMode, ReplyParameters, StickerFormat,
    UserId,
};
use teloxide::utils::command::BotCommands;
use tokio::sync::Mutex;

use crate::arguments::{parse_photo_arguments, parse_video_arguments, PhotoQuoteArguments};
use crate::content::random_emoji;
use crate::exceptions::{ProcessingError, ProcessingErrorKind};
use crate::imaging::{image_to_emoji, image_to_sticker, FilePreprocessType};
use crate::mallard::Mallard;
use crate::quote::render_quote;
use crate::responses::ResponseType;
use crate::stickerpack::StickerPack;
use crate::video::{video_to_emoji, video_to_sticker, VideoPreprocess};

pub type SharedMallard = Arc<Mutex<Mallard>>;

#[derive(Clone)]
pub struct BotConfig {
    pub admin_id: Option<UserId>,
    pub pack: Option<StickerPack>,
}

#[derive(BotCommands, Clone)]
#[command(rename_rule = "lowercase")]
pub enum Command {
    Help,
    Snap(String),
    Qva(String),
    Qwa(String),
    Emoji,
    Id,
    Voice(String),
}

pub const HELP_TEXT: &str = "Кряква умеет превращать кружочки, гифки, видео и картинки в стикеры.\n\
Используйте /qva для анимированных стикеров и /snap для обычных.\n\
Используйте /emoji в ответ на стикер, чтобы преобразовать его в формат, подходящий для кастомных эмодзи (реакций).\n\
При использовании /qva вы также можете использовать параметры:\n\
* s<sec> обрежет видео начиная с sec, sec должно быть целым\n\
* e<sec> обрежет видео до sec, sec должно быть целым\n\
* x<speed> изменит скорость видео на speed, speed может иметь вид 123.123\n\
* r — если указан, видео будет инвертировано\n\
* b<id> — добавляет пузырёк, 1 — справа, 2 — сверху\n\
* j — если указан, преобразует видео/фото в формат, подходящий для кастомных эмодзи\n\
* например /qva s5 e7 x2.5 r обрежет видео с 5 по 7 секунды, инвертирует полученный фрагмент и ускорит его в два с половиной раза\n\
кря-кря.";

fn reply_params(msg: &Message) -> ReplyParameters {
    ReplyParameters::new(msg.id)
}

pub fn build_dispatcher(
    bot: Bot,
    mallard: SharedMallard,
    config: BotConfig,
) -> Dispatcher<Bot, anyhow::Error, teloxide::dispatching::DefaultKey> {
    let handler = dptree::entry()
        .branch(
            Update::filter_message()
                .filter_command::<Command>()
                .endpoint(handle_command),
        )
        .branch(Update::filter_message().endpoint(handle_text))
        .branch(Update::filter_inline_query().endpoint(handle_inline));

    Dispatcher::builder(bot, handler)
        .dependencies(dptree::deps![mallard, config])
        .enable_ctrlc_handler()
        .build()
}

async fn download_file(bot: &Bot, file_id: FileId) -> anyhow::Result<Vec<u8>> {
    let file = bot.get_file(file_id).await?;
    let mut buf = Vec::with_capacity(file.size as usize);
    bot.download_file(&file.path, &mut buf).await?;
    Ok(buf)
}

async fn handle_text(bot: Bot, msg: Message, mallard: SharedMallard) -> anyhow::Result<()> {
    let text = match msg.text().or_else(|| msg.caption()) {
        Some(t) => t.to_string(),
        None => return Ok(()),
    };

    let reply = { mallard.lock().await.process(&text) };
    let Some((reply_text, reply_type)) = reply else {
        return Ok(());
    };
    let target = msg.chat.id;
    let rp = reply_params(&msg);
    match reply_type {
        ResponseType::Text => {
            bot.send_message(target, reply_text)
                .reply_parameters(rp)
                .await?;
        }
        ResponseType::Sticker => {
            bot.send_sticker(target, InputFile::file_id(FileId(reply_text)))
                .reply_parameters(rp)
                .await?;
        }
        ResponseType::Voice => {
            let path = format!("voices/{reply_text}.ogg");
            bot.send_voice(target, InputFile::file(path))
                .reply_parameters(rp)
                .await?;
        }
    }
    Ok(())
}

async fn handle_command(
    bot: Bot,
    msg: Message,
    cmd: Command,
    config: BotConfig,
    _mallard: SharedMallard,
) -> anyhow::Result<()> {
    let result = match cmd {
        Command::Help => {
            bot.send_message(msg.chat.id, HELP_TEXT)
                .reply_parameters(reply_params(&msg))
                .await?;
            return Ok(());
        }
        Command::Id => handle_id(&bot, &msg).await,
        Command::Emoji => handle_emoji(&bot, &msg, &config).await,
        Command::Snap(rest) => handle_snap(&bot, &msg, &rest, &config).await,
        Command::Qva(rest) | Command::Qwa(rest) => handle_qva(&bot, &msg, &rest, &config).await,
        Command::Voice(rest) => handle_voice(&bot, &msg, &rest, &config).await,
    };
    if let Err(e) = result {
        let body = if let Some(pe) = e.downcast_ref::<ProcessingError>() {
            pe.to_string()
        } else {
            log::error!("command failed: {e:#}");
            ProcessingError::of(ProcessingErrorKind::Unexpected).to_string()
        };
        bot.send_message(msg.chat.id, body)
            .reply_parameters(reply_params(&msg))
            .await?;
    }
    Ok(())
}

async fn handle_id(bot: &Bot, msg: &Message) -> anyhow::Result<()> {
    let Some(reply) = msg.reply_to_message() else {
        bot.send_message(
            msg.chat.id,
            "Команда должна быть отправлена в ответ на стикер",
        )
        .reply_parameters(reply_params(msg))
        .await?;
        return Ok(());
    };
    match reply.sticker() {
        Some(s) => {
            bot.send_message(msg.chat.id, s.file.id.0.clone())
                .reply_parameters(reply_params(msg))
                .await?;
        }
        None => {
            bot.send_message(
                msg.chat.id,
                "Команда должна быть отправлена в ответ на стикер",
            )
            .reply_parameters(reply_params(msg))
            .await?;
        }
    }
    Ok(())
}

async fn handle_emoji(bot: &Bot, msg: &Message, _config: &BotConfig) -> anyhow::Result<()> {
    let Some(reply) = msg.reply_to_message() else {
        return Ok(());
    };
    let Some(sticker) = reply.sticker() else {
        return Ok(());
    };
    if sticker.is_animated() {
        return Ok(());
    }
    if sticker.is_video() {
        let bytes = download_file(bot, sticker.file.id.clone()).await?;
        let out = video_to_emoji(&bytes).await?;
        bot.send_document(
            msg.chat.id,
            InputFile::memory(out).file_name(format!("{}.webm", uuid::Uuid::new_v4())),
        )
        .reply_parameters(reply_params(msg))
        .await?;
        return Ok(());
    }
    let fid = sticker
        .thumbnail
        .as_ref()
        .map(|t| t.file.id.clone())
        .unwrap_or_else(|| sticker.file.id.clone());
    let bytes = download_file(bot, fid).await?;
    let out = image_to_emoji(&bytes)?;
    bot.send_document(
        msg.chat.id,
        InputFile::memory(out).file_name(format!("{}.png", uuid::Uuid::new_v4())),
    )
    .reply_parameters(reply_params(msg))
    .await?;
    Ok(())
}

async fn handle_snap(
    bot: &Bot,
    msg: &Message,
    rest: &str,
    config: &BotConfig,
) -> anyhow::Result<()> {
    let args = parse_photo_arguments(&format!("/snap {rest}"))?;
    let Some(reply) = msg.reply_to_message() else {
        return Err(ProcessingError::of(ProcessingErrorKind::WrongSourceType).into());
    };

    let png = render_from_reply(bot, reply, &args).await?;

    if args.is_emoji.unwrap_or(false) {
        bot.send_document(
            msg.chat.id,
            InputFile::memory(png).file_name(format!("{}.png", uuid::Uuid::new_v4())),
        )
        .reply_parameters(reply_params(msg))
        .await?;
    } else if let Some(pack) = config.pack.as_ref() {
        let sticker = pack
            .add(bot, png, StickerFormat::Static, random_emoji())
            .await?;
        bot.send_sticker(msg.chat.id, InputFile::file_id(sticker.file.id))
            .reply_parameters(reply_params(msg))
            .await?;
    } else {
        bot.send_sticker(msg.chat.id, InputFile::memory(png))
            .reply_parameters(reply_params(msg))
            .await?;
    }
    Ok(())
}

async fn render_from_reply(
    bot: &Bot,
    reply: &Message,
    args: &PhotoQuoteArguments,
) -> Result<Vec<u8>, ProcessingError> {
    let common = match &reply.kind {
        MessageKind::Common(c) => c,
        _ => return Err(ProcessingError::of(ProcessingErrorKind::WrongSourceType)),
    };

    match &common.media_kind {
        MediaKind::Text(t) => {
            let author = reply
                .forward_from_user()
                .map(|u| u.full_name())
                .or_else(|| reply.from.as_ref().map(|u| u.full_name()))
                .unwrap_or_else(|| "unknown".to_string());
            render_quote(&t.text, &author, 0)
        }
        MediaKind::VideoNote(vn) => {
            let thumb = vn
                .video_note
                .thumbnail
                .as_ref()
                .ok_or_else(|| ProcessingError::of(ProcessingErrorKind::WrongSourceType))?;
            let bytes = download_blocking(bot, thumb.file.id.clone()).await?;
            image_to_sticker(&bytes, FilePreprocessType::Circle, args)
        }
        MediaKind::Photo(p) => {
            let best = p
                .photo
                .last()
                .ok_or_else(|| ProcessingError::of(ProcessingErrorKind::WrongSourceType))?;
            let bytes = download_blocking(bot, best.file.id.clone()).await?;
            image_to_sticker(&bytes, FilePreprocessType::Default, args)
        }
        MediaKind::Document(d) => {
            if let Some(thumb) = &d.document.thumbnail {
                let bytes = download_blocking(bot, thumb.file.id.clone()).await?;
                return image_to_sticker(&bytes, FilePreprocessType::Default, args);
            }
            if d.document.file.size > 10 * 1024 * 1024 {
                return Err(ProcessingError::of(ProcessingErrorKind::FileTooLarge));
            }
            Err(ProcessingError::of(ProcessingErrorKind::WrongSourceType))
        }
        MediaKind::Video(v) => {
            if let Some(thumb) = &v.video.thumbnail {
                let bytes = download_blocking(bot, thumb.file.id.clone()).await?;
                return image_to_sticker(&bytes, FilePreprocessType::Default, args);
            }
            if v.video.file.size > 10 * 1024 * 1024 {
                return Err(ProcessingError::of(ProcessingErrorKind::FileTooLarge));
            }
            Err(ProcessingError::of(ProcessingErrorKind::WrongSourceType))
        }
        _ => Err(ProcessingError::of(ProcessingErrorKind::WrongSourceType)),
    }
}

async fn download_blocking(bot: &Bot, file_id: FileId) -> Result<Vec<u8>, ProcessingError> {
    download_file(bot, file_id)
        .await
        .map_err(|e| ProcessingError::new(ProcessingErrorKind::Unexpected, format!("{e}")))
}

async fn handle_qva(
    bot: &Bot,
    msg: &Message,
    rest: &str,
    config: &BotConfig,
) -> anyhow::Result<()> {
    let wait = bot
        .send_message(
            msg.chat.id,
            "Ваш запрос очень кважен для нас, оставайтесь на линии!",
        )
        .reply_parameters(reply_params(msg))
        .await?;

    let result = async {
        let args = parse_video_arguments(&format!("/qva {rest}"))?;
        let Some(reply) = msg.reply_to_message() else {
            return Err(ProcessingError::of(ProcessingErrorKind::WrongSourceType));
        };
        let common = match &reply.kind {
            MessageKind::Common(c) => c,
            _ => return Err(ProcessingError::of(ProcessingErrorKind::WrongSourceType)),
        };
        let (file_id, preprocess) = match &common.media_kind {
            MediaKind::VideoNote(v) => (v.video_note.file.id.clone(), VideoPreprocess::Circle),
            MediaKind::Video(v) => {
                if v.video.file.size > 10 * 1024 * 1024 {
                    return Err(ProcessingError::of(ProcessingErrorKind::FileTooLarge));
                }
                (v.video.file.id.clone(), VideoPreprocess::VideoThumb)
            }
            MediaKind::Document(d) => {
                if d.document.file.size > 10 * 1024 * 1024 {
                    return Err(ProcessingError::of(ProcessingErrorKind::FileTooLarge));
                }
                (d.document.file.id.clone(), VideoPreprocess::VideoThumb)
            }
            _ => return Err(ProcessingError::of(ProcessingErrorKind::WrongSourceType)),
        };

        let bytes = download_file(bot, file_id)
            .await
            .map_err(|e| ProcessingError::new(ProcessingErrorKind::Unexpected, e.to_string()))?;
        let is_emoji = args.is_emoji.unwrap_or(false);
        let webm = video_to_sticker(&bytes, args, preprocess).await?;
        Ok::<_, ProcessingError>((webm, is_emoji))
    }
    .await;

    match result {
        Ok((webm, is_emoji)) => {
            if is_emoji {
                bot.send_document(
                    msg.chat.id,
                    InputFile::memory(webm).file_name(format!("{}.webm", uuid::Uuid::new_v4())),
                )
                .reply_parameters(reply_params(msg))
                .await?;
            } else if let Some(pack) = config.pack.as_ref() {
                let sticker = pack
                    .add(bot, webm, StickerFormat::Video, random_emoji())
                    .await?;
                bot.send_sticker(msg.chat.id, InputFile::file_id(sticker.file.id))
                    .reply_parameters(reply_params(msg))
                    .await?;
            } else {
                bot.send_sticker(msg.chat.id, InputFile::memory(webm))
                    .reply_parameters(reply_params(msg))
                    .await?;
            }
            bot.delete_message(wait.chat.id, wait.id).await.ok();
        }
        Err(e) => {
            bot.edit_message_text(wait.chat.id, wait.id, e.to_string())
                .await?;
        }
    }
    Ok(())
}

async fn handle_voice(
    bot: &Bot,
    msg: &Message,
    rest: &str,
    config: &BotConfig,
) -> anyhow::Result<()> {
    // Admin-only and only meaningful in private chats — same as the Python.
    let Some(from) = msg.from.as_ref() else {
        return Ok(());
    };
    let Some(admin) = config.admin_id else {
        return Ok(());
    };
    if from.id != admin {
        return Ok(());
    }
    if !matches!(msg.chat.kind, teloxide::types::ChatKind::Private(_)) {
        return Ok(());
    }
    let name = rest.split_whitespace().next().unwrap_or("").trim();
    if name.is_empty() {
        bot.send_message(msg.chat.id, "Usage: /voice <name>")
            .reply_parameters(reply_params(msg))
            .await?;
        return Ok(());
    }

    let Some(reply) = msg.reply_to_message() else {
        return Ok(());
    };
    let voice = match &reply.kind {
        MessageKind::Common(c) => match &c.media_kind {
            MediaKind::Voice(v) => &v.voice,
            _ => return Ok(()),
        },
        _ => return Ok(()),
    };

    let bytes = download_file(bot, voice.file.id.clone()).await?;
    tokio::fs::create_dir_all("voices").await.ok();
    let path = format!("voices/{name}.ogg");
    tokio::fs::write(&path, bytes).await?;
    bot.send_message(msg.chat.id, format!("saved {path}"))
        .reply_parameters(reply_params(msg))
        .await?;
    Ok(())
}

async fn handle_inline(bot: Bot, q: InlineQuery, mallard: SharedMallard) -> anyhow::Result<()> {
    let creature = { mallard.lock().await.get_creature().to_string() };
    let result = InlineQueryResultArticle::new(
        uuid::Uuid::new_v4().to_string(),
        "Кто ты сегодня?",
        InputMessageContent::Text(
            InputMessageContentText::new(format!("<i>{creature}</i>")).parse_mode(ParseMode::Html),
        ),
    );
    bot.answer_inline_query(q.id, [InlineQueryResult::Article(result)])
        .cache_time(60 * 60 * 3)
        .is_personal(true)
        .await?;
    Ok(())
}
