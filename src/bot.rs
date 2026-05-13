//! teloxide wiring for the Mallard bot.

use std::sync::Arc;

use rand::Rng;
use teloxide::net::Download;
use teloxide::prelude::*;
use teloxide::types::{
    ChatKind, FileId, InlineQueryResult, InlineQueryResultArticle, InputFile, InputMessageContent,
    InputMessageContentText, MediaKind, MessageKind, MessageOrigin, ParseMode, ReplyParameters,
    StickerFormat, UserId,
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
use crate::stickerpack::{PackKind, StickerPack};
use crate::video::{video_to_emoji, video_to_sticker, VideoPreprocess};

pub type SharedMallard = Arc<Mutex<Mallard>>;

#[derive(Clone)]
pub struct BotConfig {
    pub admin_id: Option<UserId>,
    pub pack: Option<StickerPack>,
    pub db: crate::db::Db,
    pub tea_sessions: crate::sessions::SessionStore,
}

#[derive(BotCommands, Clone)]
#[command(rename_rule = "lowercase")]
pub enum Command {
    #[command(description = "/help [команда] — справка (общая или по конкретной команде)")]
    Help(String),
    #[command(description = "сделать стикер из ответа (картинка/гифка/видео/текст)")]
    Snap(String),
    #[command(description = "сделать анимированный стикер из ответа (видео/гифка/кружок)")]
    Qva(String),
    #[command(hide)]
    Qwa(String),
    #[command(description = "конвертировать стикер в формат кастомного эмодзи")]
    Emoji,
    #[command(description = "узнать file_id стикера в ответе")]
    Id,
    #[command(hide)]
    Voice(String),
    #[command(hide)]
    Import(String),
    #[command(description = "бросить кубик: /roll, /roll 20, /roll 2d6")]
    Roll(String),
    #[command(description = "выбрать одно из перечисленного через запятую")]
    Pick(String),
    #[command(description = "гороскоп на сегодня для одной из зверушек")]
    Horoscope,
    #[command(description = "чайная сессия: /cha <чай>, /cha who, /cha log, /cha end")]
    Cha(String),
    #[command(description = "следующая заварка в активной сессии")]
    Sip,
    #[command(description = "поиск пакета в nixpkgs: /npkg ripgrep")]
    Npkg(String),
    #[command(description = "поиск опции NixOS: /nopt services.tailscale")]
    Nopt(String),
    #[command(description = "какой пакет даёт эту команду: /nixwhere mtr")]
    Nixwhere(String),
}

const HELP_OVERVIEW: &str = "Кряква умеет превращать кружочки, гифки, видео и картинки в стикеры.\n\
Используйте /qva для анимированных стикеров и /snap для обычных.\n\
Используйте /emoji в ответ на стикер, чтобы преобразовать его в формат, подходящий для кастомных эмодзи (реакций).\n\
Используйте /id в ответ на стикер, чтобы узнать его file_id.\n\
Подробности по конкретной команде: /help <команда>, например /help qva.\n\
кря-кря.";

const HELP_SNAP: &str = "/snap превращает ответное сообщение в стикер.\n\
В ответ на картинку, гифку, видео, кружок или текстовую цитату.\n\
При использовании /snap вы также можете использовать параметры:\n\
* b<id> — добавляет пузырёк, 1 — справа, 2 — сверху, без числа — случайный\n\
* j — если указан, преобразует результат в формат, подходящий для кастомных эмодзи (100×100)\n\
* например /snap b1 добавит пузырёк справа.\n\
кря-кря.";

const HELP_QVA: &str = "/qva превращает ответное сообщение в анимированный стикер.\n\
В ответ на видео, гифку, кружок или видео-стикер (до 10 МБ). Если ответить на картинку, бот вернёт обычный (статичный) стикер — параметры s/e/x/r при этом ничего не делают.\n\
При использовании /qva вы также можете использовать параметры:\n\
* s<sec> обрежет видео начиная с sec, sec должно быть целым\n\
* e<sec> обрежет видео до sec, sec должно быть целым (используется только вместе с s<sec>)\n\
* x<speed> изменит скорость видео на speed, speed может иметь вид 123.123\n\
* r — если указан, видео будет инвертировано\n\
* c — если указан, результат будет круглым (как кружок); для кружка применяется автоматически\n\
* b<id> — добавляет пузырёк, 1 — справа, 2 — сверху, без числа — случайный\n\
* j — если указан, преобразует результат в формат, подходящий для кастомных эмодзи (100×100)\n\
* например /qva s5 e7 x2.5 r обрежет видео с 5 по 7 секунды, инвертирует полученный фрагмент и ускорит его в два с половиной раза.\n\
Каждый параметр можно указать не больше одного раза, длина результата всё равно режется до 2.9 секунд.\n\
кря-кря.";

const HELP_EMOJI: &str =
    "/emoji конвертирует стикер в формат, подходящий для кастомных эмодзи (реакций).\n\
Используется в ответ на статичный или видео-стикер, только в личке с ботом.\n\
Бот вернёт готовый файл (.png или .webm) — его можно скормить @fStikBot для своего эмодзи-пака.\n\
кря-кря.";

const HELP_ID: &str = "/id показывает file_id стикера.\n\
Используется в ответ на стикер. Бот пришлёт его внутренний идентификатор.\n\
кря-кря.";

const HELP_VOICE: &str = "/voice <name> сохраняет голосовое сообщение в voices/<name>.ogg.\n\
Только для админа, только в личке, в ответ на голосовое сообщение.\n\
Сохранённые файлы потом могут проигрываться кряквой в ответ на ключевые слова.\n\
кря-кря.";

const HELP_IMPORT: &str = "/import <название_пака> переносит чужой стикер-пак в наш.\n\
Только для админа. Аргумент — короткое имя из ссылки t.me/addstickers/<имя>.\n\
Скачивает каждый стикер из источника и кладёт его в mallard_pack_by_<бот>, \
сохраняя формат и эмодзи. Долгая операция; кряква отчитается, когда закончит.\n\
кря-кря.";

const HELP_ROLL: &str = "/roll бросает кубики. Использует полноценный d20-DSL (caith).\n\
Без аргументов — d6. Поддерживаются модификаторы, advantage, exploding и прочее:\n\
* /roll 1d20 — обычный бросок\n\
* /roll 3d6+2 — три шестигранника плюс модификатор\n\
* /roll 2d20kh1 — advantage (взять наибольший из двух)\n\
* /roll 2d20kl1 — disadvantage (наименьший)\n\
* /roll 4d6k3 — keep highest three of four (стандартный stat-rolling)\n\
* /roll 3d6! — exploding (на максимуме перебрасывает)\n\
* /roll 3d6r1 — перебросить единицы\n\
* /roll 4d6t4 — посчитать количество кубиков ≥4\n\
* /roll 1d20 # save vs death — комментарии после #\n\
Полная грамматика: https://docs.rs/caith\n\
кря-кря.";

const HELP_PICK: &str = "/pick выбирает один из перечисленных вариантов.\n\
Разделитель — запятая, точка с запятой или вертикальная черта.\n\
Например: /pick чай, кофе, борщ — кряква подумает и выберет один.\n\
кря-кря.";

const HELP_HOROSCOPE: &str = "/horoscope даёт прогноз дня для случайной зверушки.\n\
Без аргументов. Использует тот же зверинец, что и инлайн-режим «Кто ты сегодня?».\n\
Точность гарантируется в пределах разумного.\n\
кря-кря.";

const HELP_NPKG: &str = "/npkg <запрос> — поиск пакета в nixpkgs (nixos-unstable).\n\
Локальный кэш каталога channels.nixos.org, обновляется раз в неделю.\n\
Возвращает топ-1 совпадение: атрибут, версию, описание, как поставить.\n\
Например: /npkg ripgrep\n\
кря-кря.";

const HELP_NOPT: &str = "/nopt <запрос> — поиск опции NixOS.\n\
Тот же локальный кэш. Возвращает имя, тип, дефолт, описание.\n\
Например: /nopt services.tailscale.enable\n\
кря-кря.";

const HELP_NIXWHERE: &str = "/nixwhere <команда> — какой пакет даёт эту бинарь.\n\
Не идеально: индексирует только mainProgram + имя пакета. \
Для bash/coreutils и прочих многобинарных — лучше поискать руками.\n\
Например: /nixwhere mtr\n\
кря-кря.";

const HELP_CHA: &str = "/cha — чайная сессия.\n\
Кряква считает заварки и помнит, кто сейчас пьёт чай в этом чате.\n\
* /cha <название> — начать сессию (название — свободный текст)\n\
* /sip — следующая заварка в твоей активной сессии\n\
* /cha note <заметка> — добавить заметку к текущей сессии\n\
* /cha end — закрыть сессию\n\
* /cha who — кто сейчас пьёт чай в этом чате\n\
* /cha log — последние 10 закрытых сессий\n\
* /cha — без аргументов: статус твоей активной сессии или эта подсказка.\n\
Сессии автоматически закрываются после 90 минут без активности.\n\
кря-кря.";

fn help_for(query: &str) -> String {
    let q = query.trim().trim_start_matches('/').to_ascii_lowercase();
    match q.as_str() {
        "" | "help" => HELP_OVERVIEW.to_string(),
        "snap" => HELP_SNAP.to_string(),
        "qva" | "qwa" => HELP_QVA.to_string(),
        "emoji" => HELP_EMOJI.to_string(),
        "id" => HELP_ID.to_string(),
        "voice" => HELP_VOICE.to_string(),
        "import" => HELP_IMPORT.to_string(),
        "roll" => HELP_ROLL.to_string(),
        "pick" => HELP_PICK.to_string(),
        "horoscope" => HELP_HOROSCOPE.to_string(),
        "cha" | "sip" => HELP_CHA.to_string(),
        "npkg" => HELP_NPKG.to_string(),
        "nopt" => HELP_NOPT.to_string(),
        "nixwhere" => HELP_NIXWHERE.to_string(),
        other => format!(
            "Не ква, не знаю такой команды ({other:?}). \
             Кряква умеет: /snap, /qva, /emoji, /id, /roll, /pick, /horoscope."
        ),
    }
}

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
        .branch(Update::filter_inline_query().endpoint(handle_inline))
        // Swallow membership / chat-admin updates so they don't log as
        // "unhandled". Nothing to do for them.
        .branch(Update::filter_my_chat_member().endpoint(noop_chat_member))
        .branch(Update::filter_chat_member().endpoint(noop_chat_member));

    Dispatcher::builder(bot, handler)
        .dependencies(dptree::deps![mallard, config])
        .default_handler(|_| async {})
        .enable_ctrlc_handler()
        .build()
}

async fn noop_chat_member(_: teloxide::types::ChatMemberUpdated) -> anyhow::Result<()> {
    Ok(())
}

fn origin(msg: &Message) -> String {
    let user = msg
        .from
        .as_ref()
        .map(|u| {
            u.username
                .as_ref()
                .map(|n| format!("@{n}"))
                .unwrap_or_else(|| u.full_name())
        })
        .unwrap_or_else(|| "?".to_string());
    let chat = match &msg.chat.kind {
        teloxide::types::ChatKind::Private(_) => "private".to_string(),
        teloxide::types::ChatKind::Public(p) => p
            .title
            .clone()
            .unwrap_or_else(|| format!("chat:{}", msg.chat.id.0)),
    };
    format!("{user}@{chat}")
}

fn describe_media(reply: &Message) -> &'static str {
    match &reply.kind {
        MessageKind::Common(c) => match &c.media_kind {
            MediaKind::Text(_) => "text",
            MediaKind::Photo(_) => "photo",
            MediaKind::Video(_) => "video",
            MediaKind::VideoNote(_) => "video_note",
            MediaKind::Document(_) => "document",
            MediaKind::Animation(_) => "animation",
            MediaKind::Voice(_) => "voice",
            MediaKind::Sticker(_) => "sticker",
            _ => "other",
        },
        _ => "non-common",
    }
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
    log::info!(
        "echo from {}: {:?} → {:?} {:.40?}",
        origin(&msg),
        text,
        reply_type,
        reply_text
    );
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
    let cmd_label = match &cmd {
        Command::Help(r) => format!("help {r}").trim().to_string(),
        Command::Id => "id".to_string(),
        Command::Emoji => "emoji".to_string(),
        Command::Snap(r) => format!("snap {r}").trim().to_string(),
        Command::Qva(r) => format!("qva {r}").trim().to_string(),
        Command::Qwa(r) => format!("qwa {r}").trim().to_string(),
        Command::Voice(r) => format!("voice {r}").trim().to_string(),
        Command::Import(r) => format!("import {r}").trim().to_string(),
        Command::Roll(r) => format!("roll {r}").trim().to_string(),
        Command::Pick(r) => format!("pick {r}").trim().to_string(),
        Command::Horoscope => "horoscope".to_string(),
        Command::Cha(r) => format!("cha {r}").trim().to_string(),
        Command::Sip => "sip".to_string(),
        Command::Npkg(r) => format!("npkg {r}").trim().to_string(),
        Command::Nopt(r) => format!("nopt {r}").trim().to_string(),
        Command::Nixwhere(r) => format!("nixwhere {r}").trim().to_string(),
    };
    let reply_kind = msg.reply_to_message().map(describe_media).unwrap_or("none");
    log::info!(
        "cmd /{} from {} (reply_to: {})",
        cmd_label,
        origin(&msg),
        reply_kind
    );
    let result = match cmd {
        Command::Help(query) => {
            bot.send_message(msg.chat.id, help_for(&query))
                .reply_parameters(reply_params(&msg))
                .await?;
            return Ok(());
        }
        Command::Id => handle_id(&bot, &msg).await,
        Command::Emoji => handle_emoji(&bot, &msg, &config).await,
        Command::Snap(rest) => handle_snap(&bot, &msg, &rest, &config).await,
        Command::Qva(rest) | Command::Qwa(rest) => handle_qva(&bot, &msg, &rest, &config).await,
        Command::Voice(rest) => handle_voice(&bot, &msg, &rest, &config).await,
        Command::Import(rest) => handle_import(&bot, &msg, &rest, &config).await,
        Command::Roll(rest) => handle_roll(&bot, &msg, &rest).await,
        Command::Pick(rest) => handle_pick(&bot, &msg, &rest).await,
        Command::Horoscope => handle_horoscope(&bot, &msg).await,
        Command::Cha(rest) => handle_cha(&bot, &msg, &rest, &config).await,
        Command::Sip => handle_sip(&bot, &msg, &config).await,
        Command::Npkg(rest) => handle_npkg(&bot, &msg, &rest, &config).await,
        Command::Nopt(rest) => handle_nopt(&bot, &msg, &rest, &config).await,
        Command::Nixwhere(rest) => handle_nixwhere(&bot, &msg, &rest, &config).await,
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
    // Python only runs /emoji in private chats — keep the same restriction.
    if !matches!(msg.chat.kind, ChatKind::Private(_)) {
        return Ok(());
    }
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
    let is_emoji = args.is_emoji.unwrap_or(false);

    match (is_emoji, config.pack.as_ref()) {
        (true, Some(pack)) => {
            let sticker = pack
                .add_emoji(bot, png, StickerFormat::Static, random_emoji())
                .await?;
            bot.send_sticker(msg.chat.id, InputFile::file_id(sticker.file.id))
                .reply_parameters(reply_params(msg))
                .await?;
        }
        (true, None) => {
            bot.send_document(
                msg.chat.id,
                InputFile::memory(png).file_name(format!("{}.png", uuid::Uuid::new_v4())),
            )
            .reply_parameters(reply_params(msg))
            .await?;
        }
        (false, Some(pack)) => {
            let sticker = pack
                .add(bot, png, StickerFormat::Static, random_emoji())
                .await?;
            bot.send_sticker(msg.chat.id, InputFile::file_id(sticker.file.id))
                .reply_parameters(reply_params(msg))
                .await?;
        }
        (false, None) => {
            bot.send_sticker(msg.chat.id, InputFile::memory(png))
                .reply_parameters(reply_params(msg))
                .await?;
        }
    }
    Ok(())
}

/// Author cascade for /snap-on-text quotes — matches the Python behaviour:
/// hidden-user forwards win, then visible-user forwards, then the message
/// sender, then a hard-coded fallback.
fn quote_author(reply: &Message) -> String {
    if let Some(MessageOrigin::HiddenUser {
        sender_user_name, ..
    }) = reply.forward_origin()
    {
        return sender_user_name.clone();
    }
    if let Some(u) = reply.forward_from_user() {
        return u.full_name();
    }
    reply
        .from
        .as_ref()
        .map(|u| u.full_name())
        .unwrap_or_else(|| "unknown".to_string())
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
            let author = quote_author(reply);
            // Python picks one of 4 palette colours per quote at random.
            let color_index = rand::thread_rng().gen_range(0..4);
            render_quote(&t.text, &author, color_index)
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
        let force_circle = args.circle.unwrap_or(false);
        let (file_id, preprocess) = match &common.media_kind {
            MediaKind::VideoNote(v) => (v.video_note.file.id.clone(), VideoPreprocess::Circle),
            MediaKind::Video(v) => {
                if v.video.file.size > 10 * 1024 * 1024 {
                    return Err(ProcessingError::of(ProcessingErrorKind::FileTooLarge));
                }
                let pp = if force_circle {
                    VideoPreprocess::Circle
                } else {
                    VideoPreprocess::VideoThumb
                };
                (v.video.file.id.clone(), pp)
            }
            MediaKind::Animation(a) => {
                if a.animation.file.size > 10 * 1024 * 1024 {
                    return Err(ProcessingError::of(ProcessingErrorKind::FileTooLarge));
                }
                let pp = if force_circle {
                    VideoPreprocess::Circle
                } else {
                    VideoPreprocess::VideoThumb
                };
                (a.animation.file.id.clone(), pp)
            }
            MediaKind::Document(d) => {
                if d.document.file.size > 10 * 1024 * 1024 {
                    return Err(ProcessingError::of(ProcessingErrorKind::FileTooLarge));
                }
                let pp = if force_circle {
                    VideoPreprocess::Circle
                } else {
                    VideoPreprocess::VideoThumb
                };
                (d.document.file.id.clone(), pp)
            }
            // Modern Telegram has video stickers — treat them like a video
            // source so /qva on a video sticker round-trips correctly.
            MediaKind::Sticker(s) if s.sticker.is_video() => {
                let pp = if force_circle {
                    VideoPreprocess::Circle
                } else {
                    VideoPreprocess::VideoThumb
                };
                (s.sticker.file.id.clone(), pp)
            }
            // Still image source — route through the image pipeline and
            // return a static sticker. Animation-only flags (s/e/x/r) have
            // no meaning on a single frame and are silently ignored.
            MediaKind::Photo(p) => {
                let best = p
                    .photo
                    .last()
                    .ok_or_else(|| ProcessingError::of(ProcessingErrorKind::WrongSourceType))?;
                let bytes = download_blocking(bot, best.file.id.clone()).await?;
                let photo_args = PhotoQuoteArguments {
                    speech_bubble: args.speech_bubble,
                    is_emoji: args.is_emoji,
                };
                let preprocess = if force_circle {
                    FilePreprocessType::Circle
                } else {
                    FilePreprocessType::Default
                };
                let png = image_to_sticker(&bytes, preprocess, &photo_args)?;
                return Ok::<_, ProcessingError>(QvaOutput::Still {
                    bytes: png,
                    is_emoji: args.is_emoji.unwrap_or(false),
                });
            }
            _ => return Err(ProcessingError::of(ProcessingErrorKind::WrongSourceType)),
        };

        let bytes = download_file(bot, file_id)
            .await
            .map_err(|e| ProcessingError::new(ProcessingErrorKind::Unexpected, e.to_string()))?;
        let is_emoji = args.is_emoji.unwrap_or(false);
        let webm = video_to_sticker(&bytes, args, preprocess).await?;
        Ok::<_, ProcessingError>(QvaOutput::Video {
            bytes: webm,
            is_emoji,
        })
    }
    .await;

    match result {
        Ok(out) => {
            let (bytes, is_emoji, format, ext) = match out {
                QvaOutput::Video { bytes, is_emoji } => {
                    (bytes, is_emoji, StickerFormat::Video, "webm")
                }
                QvaOutput::Still { bytes, is_emoji } => {
                    (bytes, is_emoji, StickerFormat::Static, "png")
                }
            };
            match (is_emoji, config.pack.as_ref()) {
                (true, Some(pack)) => {
                    let sticker = pack.add_emoji(bot, bytes, format, random_emoji()).await?;
                    bot.send_sticker(msg.chat.id, InputFile::file_id(sticker.file.id))
                        .reply_parameters(reply_params(msg))
                        .await?;
                }
                (true, None) => {
                    bot.send_document(
                        msg.chat.id,
                        InputFile::memory(bytes)
                            .file_name(format!("{}.{ext}", uuid::Uuid::new_v4())),
                    )
                    .reply_parameters(reply_params(msg))
                    .await?;
                }
                (false, Some(pack)) => {
                    let sticker = pack.add(bot, bytes, format, random_emoji()).await?;
                    bot.send_sticker(msg.chat.id, InputFile::file_id(sticker.file.id))
                        .reply_parameters(reply_params(msg))
                        .await?;
                }
                (false, None) => {
                    bot.send_sticker(msg.chat.id, InputFile::memory(bytes))
                        .reply_parameters(reply_params(msg))
                        .await?;
                }
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

enum QvaOutput {
    Video { bytes: Vec<u8>, is_emoji: bool },
    Still { bytes: Vec<u8>, is_emoji: bool },
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

async fn handle_import(
    bot: &Bot,
    msg: &Message,
    rest: &str,
    config: &BotConfig,
) -> anyhow::Result<()> {
    // Admin-only.
    let Some(from) = msg.from.as_ref() else {
        return Ok(());
    };
    let Some(admin) = config.admin_id else {
        return Ok(());
    };
    if from.id != admin {
        return Ok(());
    }
    let Some(pack) = config.pack.as_ref() else {
        bot.send_message(msg.chat.id, "Не ква, стикер-пак не настроен.")
            .reply_parameters(reply_params(msg))
            .await?;
        return Ok(());
    };

    let source_name = rest.split_whitespace().next().unwrap_or("").trim();
    if source_name.is_empty() {
        bot.send_message(
            msg.chat.id,
            "Не ква, нужно имя пака. Например: /import gigachad_cryakwa_video_stickers_by_fStikBot",
        )
        .reply_parameters(reply_params(msg))
        .await?;
        return Ok(());
    }

    let progress = bot
        .send_message(msg.chat.id, format!("Кряква берётся за {source_name}…"))
        .reply_parameters(reply_params(msg))
        .await?;

    let set = match bot.get_sticker_set(source_name.to_string()).await {
        Ok(s) => s,
        Err(e) => {
            bot.edit_message_text(
                progress.chat.id,
                progress.id,
                format!("Не ква, не нашла пак {source_name:?}: {e}"),
            )
            .await?;
            return Ok(());
        }
    };

    let total = set.stickers.len();
    log::info!("import {source_name}: {total} stickers");
    let _ = bot
        .edit_message_text(
            progress.chat.id,
            progress.id,
            format!("Нашла {total} стикеров в {source_name}, тащу к себе…"),
        )
        .await;

    let mut imported = 0usize;
    let mut failed = 0usize;
    for (i, src) in set.stickers.iter().enumerate() {
        match import_one(bot, pack, src).await {
            Ok(()) => imported += 1,
            Err(e) => {
                failed += 1;
                log::warn!("import sticker {i}/{total} failed: {e}");
            }
        }
        // Periodic progress + light rate-limit padding.
        if (i + 1) % 10 == 0 {
            let _ = bot
                .edit_message_text(
                    progress.chat.id,
                    progress.id,
                    format!("Кряква тащит: {}/{total}…", i + 1),
                )
                .await;
        }
        tokio::time::sleep(std::time::Duration::from_millis(150)).await;
    }

    let url = pack.pack_url(PackKind::Regular);
    bot.edit_message_text(
        progress.chat.id,
        progress.id,
        format!("Готово: {imported}/{total} перетащено, {failed} провалилось.\n{url}"),
    )
    .await?;
    Ok(())
}

async fn import_one(
    bot: &Bot,
    pack: &StickerPack,
    src: &teloxide::types::Sticker,
) -> anyhow::Result<()> {
    use teloxide::types::StickerFormat;
    let bytes = download_file(bot, src.file.id.clone()).await?;
    let format = if src.is_video() {
        StickerFormat::Video
    } else if src.is_animated() {
        StickerFormat::Animated
    } else {
        StickerFormat::Static
    };
    let emojis = src
        .emoji
        .clone()
        .map(|e| vec![e])
        .unwrap_or_else(|| vec![random_emoji().to_string()]);
    pack.add_to_with_emojis(bot, PackKind::Regular, bytes, format, emojis)
        .await?;
    Ok(())
}

async fn handle_inline(bot: Bot, q: InlineQuery, mallard: SharedMallard) -> anyhow::Result<()> {
    let creature = { mallard.lock().await.get_creature().to_string() };
    log::info!(
        "inline from @{}: {:?} → {creature}",
        q.from.username.as_deref().unwrap_or("?"),
        q.query
    );
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

// ---------- group-chat utilities ----------
//
// Pure-text commands accumulated over the years. No external I/O, no
// permissions — anyone in any chat can use them.

const HOROSCOPE_LINES: &[&str] = &[
    "ваш день начнётся с открытия, что вы — это вы",
    "сегодня не доверяйте лисичкам",
    "ваша сила — в борще",
    "пельмени принесут удачу после 18:00",
    "встреча с хомячком изменит ваши планы",
    "не открывайте письма от мишки, он опять про мёд",
    "сегодня хороший день, чтобы лечь спать пораньше",
    "ваш цвет — мятный, ваш напиток — чай",
    "избегайте квадратов, треугольников и понедельников",
    "ёжик передаёт привет",
    "Валера хочет с вами поговорить",
    "посмотрите в окно — там кто-то есть",
    "сегодня день обнимашек, лимит — три",
    "пушистики на вашей стороне",
    "звёзды говорят: купи себе цветы",
    "сова видела вас вчера и не одобряет",
    "сегодня вы притянете к себе одну (1) хорошую вещь",
    "если что-то идёт не так — это просто пельмени остыли",
];

async fn handle_roll(bot: &Bot, msg: &Message, rest: &str) -> anyhow::Result<()> {
    // Full d20-style DSL via caith — supports advantage/disadvantage,
    // exploding (!), keep/drop (k/d), rerolls (r), targets (t), modifiers,
    // comments. Examples: "1d20", "2d20kh1+5", "4d6k3", "3d6!", "1d20 # save".
    let expr = rest.trim();
    let expr = if expr.is_empty() { "1d6" } else { expr };

    let (body, as_html) = match caith::Roller::new(expr) {
        Ok(roller) => match roller.roll() {
            // caith emits its own markdown (`...` for dice, **...** for the
            // highlighted ones). Convert to Telegram HTML so the formatting
            // survives the wire — HTML's escape rules are tame compared to
            // MarkdownV2's.
            Ok(result) => (
                format!("\u{1F3B2} {}", caith_md_to_html(&result.to_string())),
                true,
            ),
            Err(e) => (format!("не ква, не получилось бросить: {e}"), false),
        },
        Err(e) => (format!("не ква, не понял выражение: {e}"), false),
    };
    let mut send = bot.send_message(msg.chat.id, body);
    if as_html {
        send = send.parse_mode(ParseMode::Html);
    }
    send.reply_parameters(reply_params(msg)).await?;
    Ok(())
}

/// Convert caith's mini-markdown to Telegram HTML.
/// Supports: `` `code` `` → `<code>…</code>`, `**bold**` → `<b>…</b>`.
/// Escapes `<`, `>`, `&` so user-supplied substrings (e.g. trailing comments)
/// can't smuggle markup. Tolerates unclosed runs by closing on EOL/EOS.
fn caith_md_to_html(s: &str) -> String {
    let mut out = String::new();
    let mut chars = s.chars().peekable();
    let mut in_code = false;
    let mut in_bold = false;
    while let Some(c) = chars.next() {
        match c {
            '`' => {
                out.push_str(if in_code { "</code>" } else { "<code>" });
                in_code = !in_code;
            }
            '*' if chars.peek() == Some(&'*') && !in_code => {
                chars.next();
                out.push_str(if in_bold { "</b>" } else { "<b>" });
                in_bold = !in_bold;
            }
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '&' => out.push_str("&amp;"),
            _ => out.push(c),
        }
    }
    if in_code {
        out.push_str("</code>");
    }
    if in_bold {
        out.push_str("</b>");
    }
    out
}

async fn handle_pick(bot: &Bot, msg: &Message, rest: &str) -> anyhow::Result<()> {
    use rand::seq::SliceRandom;
    let options: Vec<&str> = rest
        .split(|c: char| c == ',' || c == '|' || c == ';')
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .collect();
    if options.len() < 2 {
        bot.send_message(
            msg.chat.id,
            "перечисли через запятую хотя бы два варианта, например: /pick чай, кофе, борщ",
        )
        .reply_parameters(reply_params(msg))
        .await?;
        return Ok(());
    }
    let chosen: String = {
        let mut rng = rand::thread_rng();
        options.choose(&mut rng).copied().unwrap_or("").to_string()
    };
    bot.send_message(msg.chat.id, format!("\u{1F50D} {chosen}"))
        .reply_parameters(reply_params(msg))
        .await?;
    Ok(())
}

async fn handle_horoscope(bot: &Bot, msg: &Message) -> anyhow::Result<()> {
    use crate::dictionaries::CREATURES;
    use rand::seq::SliceRandom;
    let body = {
        let mut rng = rand::thread_rng();
        let creature = CREATURES.choose(&mut rng).copied().unwrap_or("Я уточка!");
        let line = HOROSCOPE_LINES.choose(&mut rng).copied().unwrap_or("кря");
        let lead = creature
            .split_whitespace()
            .take(2)
            .collect::<Vec<_>>()
            .join(" ");
        format!("{lead}\n\n{line}\n\nкря-кря.")
    };
    bot.send_message(msg.chat.id, body)
        .reply_parameters(reply_params(msg))
        .await?;
    Ok(())
}

// ---------- /cha + /sip — gong fu cha session tracking ----------

use crate::sessions::{fmt_dur, parse_notes, persist, TeaSession};

const CHA_AUTO_CLOSE_NOTE: bool = false;

async fn handle_cha(
    bot: &Bot,
    msg: &Message,
    rest: &str,
    config: &BotConfig,
) -> anyhow::Result<()> {
    let Some(from) = msg.from.as_ref() else {
        return Ok(());
    };
    let user_id = from.id;
    let user_name = from
        .username
        .as_ref()
        .map(|n| format!("@{n}"))
        .unwrap_or_else(|| from.full_name());
    let chat_id = msg.chat.id;
    let key = (chat_id, user_id);

    let trimmed = rest.trim();
    let (sub, sub_arg) = match trimmed.split_once(char::is_whitespace) {
        Some((s, a)) => (s.to_lowercase(), a.trim()),
        None => (trimmed.to_lowercase(), ""),
    };

    match sub.as_str() {
        "" => cha_status_or_help(bot, msg, config, key).await,
        "who" => cha_who(bot, msg, config).await,
        "log" => cha_log(bot, msg, config, chat_id).await,
        "end" => cha_end(bot, msg, config, key).await,
        "note" if !sub_arg.is_empty() => cha_note(bot, msg, config, key, sub_arg).await,
        _ => cha_start(bot, msg, config, chat_id, user_id, user_name, trimmed).await,
    }
}

async fn handle_sip(bot: &Bot, msg: &Message, config: &BotConfig) -> anyhow::Result<()> {
    let Some(from) = msg.from.as_ref() else {
        return Ok(());
    };
    let key = (msg.chat.id, from.id);
    let summary = {
        let mut store = config.tea_sessions.lock().await;
        match store.get_mut(&key) {
            Some(s) => {
                s.sip();
                Some((s.user_name.clone(), s.tea.clone(), s.steeps))
            }
            None => None,
        }
    };
    let body = match summary {
        Some((name, tea, steeps)) => {
            format!(
                "\u{1F375} {ord}-я заварка · {name} · {tea}",
                ord = steeps,
                name = name,
                tea = tea
            )
        }
        None => "у тебя нет активной сессии. начни через /cha <название>".to_string(),
    };
    bot.send_message(msg.chat.id, body)
        .reply_parameters(reply_params(msg))
        .await?;
    Ok(())
}

async fn cha_start(
    bot: &Bot,
    msg: &Message,
    config: &BotConfig,
    chat_id: ChatId,
    user_id: UserId,
    user_name: String,
    tea: &str,
) -> anyhow::Result<()> {
    let tea = if tea.is_empty() { "?" } else { tea };
    let key = (chat_id, user_id);
    let previous = {
        let mut store = config.tea_sessions.lock().await;
        let prev = store.remove(&key);
        store.insert(
            key,
            TeaSession::new(chat_id, user_id, user_name.clone(), tea.to_string()),
        );
        prev
    };
    // If a session was already running, persist it as auto-closed-by-restart.
    if let Some(prev) = previous {
        persist(&config.db, &prev, true).await;
    }
    bot.send_message(
        msg.chat.id,
        format!("\u{1FAD6} {user_name} начал(а) сессию: {tea}\nкряква садится рядом \u{1F60C}"),
    )
    .reply_parameters(reply_params(msg))
    .await?;
    Ok(())
}

async fn cha_status_or_help(
    bot: &Bot,
    msg: &Message,
    config: &BotConfig,
    key: (ChatId, UserId),
) -> anyhow::Result<()> {
    let snapshot = {
        let store = config.tea_sessions.lock().await;
        store.get(&key).cloned()
    };
    let body = match snapshot {
        Some(s) => format!(
            "\u{1F375} {} · {} заварок · {}",
            s.tea,
            s.steeps,
            fmt_dur(s.elapsed())
        ),
        None => HELP_CHA.to_string(),
    };
    bot.send_message(msg.chat.id, body)
        .reply_parameters(reply_params(msg))
        .await?;
    Ok(())
}

async fn cha_note(
    bot: &Bot,
    msg: &Message,
    config: &BotConfig,
    key: (ChatId, UserId),
    text: &str,
) -> anyhow::Result<()> {
    let ok = {
        let mut store = config.tea_sessions.lock().await;
        match store.get_mut(&key) {
            Some(s) => {
                s.add_note(text.to_string());
                true
            }
            None => false,
        }
    };
    let body = if ok {
        "\u{2713} записано".to_string()
    } else {
        "у тебя нет активной сессии :(".to_string()
    };
    bot.send_message(msg.chat.id, body)
        .reply_parameters(reply_params(msg))
        .await?;
    Ok(())
}

async fn cha_end(
    bot: &Bot,
    msg: &Message,
    config: &BotConfig,
    key: (ChatId, UserId),
) -> anyhow::Result<()> {
    let closed = {
        let mut store = config.tea_sessions.lock().await;
        store.remove(&key)
    };
    let Some(session) = closed else {
        bot.send_message(msg.chat.id, "у тебя нет активной сессии :(")
            .reply_parameters(reply_params(msg))
            .await?;
        return Ok(());
    };
    let body = format!(
        "\u{1FAD6} {} закрыл(а) сессию: {} · {} заварок · {}\nкряква уважает",
        session.user_name,
        session.tea,
        session.steeps,
        fmt_dur(session.elapsed()),
    );
    persist(&config.db, &session, CHA_AUTO_CLOSE_NOTE).await;
    bot.send_message(msg.chat.id, body)
        .reply_parameters(reply_params(msg))
        .await?;
    Ok(())
}

async fn cha_who(bot: &Bot, msg: &Message, config: &BotConfig) -> anyhow::Result<()> {
    let active: Vec<(
        String,
        String,
        u32,
        std::time::Duration,
        std::time::Duration,
    )> = {
        let store = config.tea_sessions.lock().await;
        store
            .iter()
            .filter(|((c, _), _)| *c == msg.chat.id)
            .map(|(_, s)| {
                (
                    s.user_name.clone(),
                    s.tea.clone(),
                    s.steeps,
                    s.elapsed(),
                    s.idle_for(),
                )
            })
            .collect()
    };
    let body = if active.is_empty() {
        "никто сейчас не пьёт чай :(\nможет ты начнёшь?".to_string()
    } else {
        let mut lines = vec!["\u{1F375} кто сейчас пьёт чай:".to_string()];
        for (name, tea, steeps, elapsed, idle) in active {
            lines.push(format!(
                "  {name} · {tea} · {steeps} заварок · {ago} назад",
                ago = fmt_dur(idle.max(elapsed))
            ));
        }
        lines.join("\n")
    };
    bot.send_message(msg.chat.id, body)
        .reply_parameters(reply_params(msg))
        .await?;
    Ok(())
}

async fn cha_log(bot: &Bot, msg: &Message, config: &BotConfig, chat: ChatId) -> anyhow::Result<()> {
    let rows = match config.db.tail_cha_log(chat.0, 10).await {
        Ok(r) => r,
        Err(e) => {
            log::warn!("cha_log query failed: {e}");
            Vec::new()
        }
    };
    let body = if rows.is_empty() {
        "журнал пуст. начните сессию через /cha <название>".to_string()
    } else {
        let mut lines = vec!["\u{1F4D6} последние сессии:".to_string()];
        for r in rows {
            let when = format_unix_short(r.started_at);
            let dur = fmt_dur(std::time::Duration::from_secs(r.duration_s as u64));
            let auto = if r.auto_closed { " · авто" } else { "" };
            let notes = parse_notes(&r.notes_json);
            let mut entry = format!(
                "  {when} · {} · {} · {} заварок · {dur}{auto}",
                r.user_name, r.tea, r.steeps
            );
            if !notes.is_empty() {
                entry.push_str(&format!("\n    заметки: {}", notes.join(" · ")));
            }
            lines.push(entry);
        }
        lines.join("\n")
    };
    bot.send_message(msg.chat.id, body)
        .reply_parameters(reply_params(msg))
        .await?;
    Ok(())
}

fn format_unix_short(unix_secs: i64) -> String {
    // Minimal "x ago"-ish formatter: minutes / hours / days. Avoids pulling in
    // chrono for one timestamp render.
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(unix_secs);
    let delta = (now - unix_secs).max(0);
    if delta < 3600 {
        format!("{} мин назад", (delta / 60).max(1))
    } else if delta < 86_400 {
        format!("{} ч назад", delta / 3600)
    } else {
        format!("{} дн назад", delta / 86_400)
    }
}

// ---------- /npkg /nopt /nixwhere — local nixpkgs catalog ----------

async fn handle_npkg(
    bot: &Bot,
    msg: &Message,
    rest: &str,
    config: &BotConfig,
) -> anyhow::Result<()> {
    let q = rest.trim();
    if q.is_empty() {
        bot.send_message(msg.chat.id, "что искать-то? например: /npkg ripgrep")
            .reply_parameters(reply_params(msg))
            .await?;
        return Ok(());
    }
    let hits = match config.db.search_nix_packages(q, 3).await {
        Ok(h) => h,
        Err(e) => {
            log::warn!("npkg query failed: {e}");
            bot.send_message(msg.chat.id, "не ква, поиск отвалился :(")
                .reply_parameters(reply_params(msg))
                .await?;
            return Ok(());
        }
    };
    let body = if hits.is_empty() {
        format!(
            "ничего не нашлось по «{q}».\n\
             если каталог только что подгрузился, попробуй ещё раз через минуту."
        )
    } else {
        let top = &hits[0];
        let descr = if top.description.is_empty() {
            "(без описания)"
        } else {
            top.description.as_str()
        };
        let mut lines = vec![format!(
            "\u{1F4E6} <b>{}</b> · {}",
            html_escape(&top.attr_name),
            html_escape(&top.version)
        )];
        lines.push(format!("{}", html_escape(descr)));
        lines.push(format!(
            "• <code>nix run nixpkgs#{}</code>",
            html_escape(&top.attr_name)
        ));
        lines.push(format!(
            "• <code>environment.systemPackages = [ pkgs.{} ];</code>",
            html_escape(&top.attr_name)
        ));
        if !top.main_program.is_empty() && top.main_program != top.attr_name {
            lines.push(format!(
                "команда: <code>{}</code>",
                html_escape(&top.main_program)
            ));
        }
        if hits.len() > 1 {
            let alts: Vec<String> = hits
                .iter()
                .skip(1)
                .take(3)
                .map(|h| format!("<code>{}</code>", html_escape(&h.attr_name)))
                .collect();
            lines.push(format!("ещё: {}", alts.join(", ")));
        }
        lines.join("\n")
    };
    bot.send_message(msg.chat.id, body)
        .parse_mode(ParseMode::Html)
        .reply_parameters(reply_params(msg))
        .await?;
    Ok(())
}

async fn handle_nopt(
    bot: &Bot,
    msg: &Message,
    rest: &str,
    config: &BotConfig,
) -> anyhow::Result<()> {
    let q = rest.trim();
    if q.is_empty() {
        bot.send_message(
            msg.chat.id,
            "что искать? например: /nopt services.tailscale.enable",
        )
        .reply_parameters(reply_params(msg))
        .await?;
        return Ok(());
    }
    let hits = match config.db.search_nix_options(q, 3).await {
        Ok(h) => h,
        Err(e) => {
            log::warn!("nopt query failed: {e}");
            bot.send_message(msg.chat.id, "не ква, поиск отвалился :(")
                .reply_parameters(reply_params(msg))
                .await?;
            return Ok(());
        }
    };
    let body = if hits.is_empty() {
        format!("опции «{q}» нет. может, я ещё не подгрузил каталог?")
    } else {
        let top = &hits[0];
        let mut lines = vec![format!(
            "\u{2699}\u{FE0F} <b>{}</b>",
            html_escape(&top.name)
        )];
        if !top.type_.is_empty() {
            lines.push(format!("тип: <code>{}</code>", html_escape(&top.type_)));
        }
        if !top.default_.is_empty() {
            lines.push(format!(
                "по умолчанию: <code>{}</code>",
                html_escape(&top.default_)
            ));
        }
        if !top.description.is_empty() {
            let descr = top.description.trim();
            // Truncate by chars, not bytes — nixpkgs descriptions contain
            // multibyte glyphs (‹›, em-dashes, CJK).
            let shown = if descr.chars().count() > 600 {
                let t: String = descr.chars().take(600).collect();
                format!("{t}…")
            } else {
                descr.to_string()
            };
            lines.push(html_escape(&shown));
        }
        if hits.len() > 1 {
            let alts: Vec<String> = hits
                .iter()
                .skip(1)
                .take(3)
                .map(|h| format!("<code>{}</code>", html_escape(&h.name)))
                .collect();
            lines.push(format!("ещё: {}", alts.join(", ")));
        }
        lines.join("\n")
    };
    bot.send_message(msg.chat.id, body)
        .parse_mode(ParseMode::Html)
        .reply_parameters(reply_params(msg))
        .await?;
    Ok(())
}

async fn handle_nixwhere(
    bot: &Bot,
    msg: &Message,
    rest: &str,
    config: &BotConfig,
) -> anyhow::Result<()> {
    let q = rest.trim();
    if q.is_empty() {
        bot.send_message(msg.chat.id, "какую команду искать? например: /nixwhere mtr")
            .reply_parameters(reply_params(msg))
            .await?;
        return Ok(());
    }
    let hits = match config.db.search_nix_programs(q, 5).await {
        Ok(h) => h,
        Err(e) => {
            log::warn!("nixwhere query failed: {e}");
            bot.send_message(msg.chat.id, "не ква, поиск отвалился :(")
                .reply_parameters(reply_params(msg))
                .await?;
            return Ok(());
        }
    };
    let body = if hits.is_empty() {
        format!(
            "не знаю, какой пакет даёт <code>{}</code>. \n\
             индекс неполный — у пакета может быть main_program с другим именем.",
            html_escape(q)
        )
    } else {
        let mut lines = vec![format!("\u{1F50E} <code>{}</code> → ", html_escape(q))];
        for h in &hits {
            let suffix = if !h.main_program.is_empty() && h.main_program != q {
                format!(" (main: <code>{}</code>)", html_escape(&h.main_program))
            } else {
                String::new()
            };
            lines.push(format!(
                "• <code>{}</code>{}",
                html_escape(&h.attr_name),
                suffix
            ));
        }
        lines.join("\n")
    };
    bot.send_message(msg.chat.id, body)
        .parse_mode(ParseMode::Html)
        .reply_parameters(reply_params(msg))
        .await?;
    Ok(())
}

fn html_escape(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
}
