//! teloxide wiring for the Mallard bot.

use std::sync::Arc;

use rand::Rng;
use teloxide::net::Download;
use teloxide::prelude::*;
use teloxide::types::{
    ChatId, ChatKind, FileId, InlineQueryResult, InlineQueryResultArticle,
    InlineQueryResultCachedPhoto, InputFile, InputMedia, InputMediaPhoto, InputMessageContent,
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
    pub bot_username: String,
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
    #[command(description = "ревизия канала: /nchan [nixos-unstable]")]
    Nchan(String),
    #[command(description = "flake registry: /nflake nixpkgs")]
    Nflake(String),
    #[command(description = "флаги в чате: /feature, /feature <шаблон> on|off|reset (для админов)")]
    Feature(String),
    #[command(description = "отрендерить typst: /typst x^2 + 1 (или в ответ на сообщение)")]
    Typst(String),
    #[command(description = "отрендерить latex: /latex \\frac{1}{2} (или в ответ на сообщение)")]
    Latex(String),
    #[command(hide)]
    Tex(String),
    #[command(description = "/math <код> — кряква сама поймёт, typst это или latex")]
    Math(String),
    #[command(description = "включить/выключить расширенный inline-режим (только в личке)")]
    Inline,
    #[command(description = "посчитать: /calc 1/3 + 1/3 + 1/3, /calc 60 mph in m/s, /calc 2^256")]
    Calc(String),
    #[command(description = "символьные штуки: /sym diff(sin(x), x), /sym solve(x^2-4, x), /sym series(sin(x), x, 0, 7)")]
    Sym(String),
    #[command(description = "нарисовать график: /plot sin(x), 0, 2*pi")]
    Plot(String),
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

const HELP_NCHAN: &str = "/nchan [канал] — текущее состояние канала nixpkgs.\n\
Без аргументов — nixos-unstable. Возвращает SHA коммита, snapshot-метку и ссылку на GitHub.\n\
кря-кря.";

const HELP_NFLAKE: &str = "/nflake <имя> — разрешает имя через nixpkgs flake registry.\n\
Тянет channels.nixos.org/flake-registry.json и ищет указанный input.\n\
Например: /nflake nixpkgs, /nflake home-manager\n\
кря-кря.";

const HELP_TYPST: &str = "/typst — отрендерить typst-сниппет в PNG.\n\
Два способа задать источник:\n\
* /typst <код> — например /typst x^2 + 1\n\
* /typst в ответ на сообщение — возьмёт текст ответного сообщения как код\n\
Если в коде нет ни # ни $, он оборачивается в `$ ... $`, то есть односложные формулы пишутся без лишних символов.\n\
Доступны пакеты с @preview/ (например cetz). Лимит компиляции — 15 секунд, ввод до 16 КБ.\n\
кря-кря.";

const HELP_LATEX: &str = "/latex (и /tex) — отрендерить latex-математику через пакет mitex.\n\
* /latex \\frac{1}{2}\n\
* /latex в ответ на сообщение — возьмёт текст ответного сообщения\n\
Поддерживается основной набор математических команд: \\frac, \\sum, \\int, \\sqrt, \\alpha, \\begin{matrix}…\\end{matrix} и так далее. TikZ, кастомные \\newcommand и полноценные документы — это к /typst.\n\
кря-кря.";

const HELP_MATH: &str = "/math <код> — кряква сама смотрит на источник и решает, typst это или latex.\n\
Эвристика простая: если в коде есть \\<буквы> (например \\frac) — это latex, иначе typst.\n\
Если автоопределение промахнулось, используйте явные /typst или /latex.\n\
кря-кря.";

const HELP_CALC: &str = "/calc <выражение> — посчитать. Под капотом fend-core, точная арифметика и куча единиц.\n\
Что умеет:\n\
* арифметика без округлений: /calc 1/3 + 1/3 + 1/3 → 1\n\
* большие числа: /calc 100! или /calc 2^256\n\
* единицы и конверсии: /calc 60 mph in m/s, /calc 5 km + 3 mi in km, /calc 1 atm * 1 L in J\n\
* температуры: /calc 451 fahrenheit in celsius\n\
* основания: /calc 0xff & 0b1010, /calc 255 to hex\n\
* комплексные: /calc (3 + 4i) * (1 - 2i)\n\
* тригонометрия с явными углами: /calc sin(pi/4), /calc tan(45 deg)\n\
* даты: /calc today + 30 days, /calc 2026-12-31 - today\n\
* короткие функции прямо в выражении: /calc f: x -> x^2; f(7)\n\
Для символьных штук (производные, упрощение) будет отдельный /sym.\n\
кря-кря.";

const HELP_THEME: &str = "Тёмная тема для всех картинок (typst, latex, /sym, /plot).\n\
По умолчанию белый фон, чёрный текст. Включить тёмную:\n\
/feature util.theme.dark on\n\
Выключить обратно:\n\
/feature util.theme.dark off\n\
Кэш картинок отдельный для каждой темы, так что переключение не путает закэшированные старые рендеры.\n\
кря-кря.";

const HELP_PLOT: &str = "/plot <выражение>, <от>, <до> — нарисовать график. Через typst + cetz-plot.\n\
Примеры:\n\
* /plot sin(x), 0, 2*pi — одна функция\n\
* /plot sin(x), cos(x), -pi, pi — несколько функций на одной картинке\n\
* /plot exp(-x^2), -3, 3 — гауссиана\n\
* /plot exp(-x/3) * cos(x), 0, 10 — затухающие колебания\n\
Поддерживаемые функции: sin, cos, tan, asin, acos, atan, sinh, cosh, tanh, exp, ln, log, sqrt, abs, pow. Константы: pi, e. Степень: x^2 (то же что pow(x, 2)).\n\
Границы диапазона принимают число, pi, -pi, e, N*pi, pi/N.\n\
кря-кря.";

const HELP_SYM: &str = "/sym <выражение> — символьные операции через symbolica. Кряква пришлёт ответ текстом и красиво отрисованной картинкой.\n\
Что умеет:\n\
* expand(<выр>) — раскрыть скобки: /sym expand((x+1)^5)\n\
* factor(<выр>) — разложить на множители: /sym factor(x^2 - 1)\n\
* together(<выр>) — привести к общему знаменателю: /sym together(1/x + 1/y)\n\
* simplify(<выр>) — together + expand за один проход\n\
* diff(<выр>, <переменная>) — производная: /sym diff(sin(x)*x^2, x) (синоним: derivative)\n\
* series(<выр> [, <var> [, <point> [, <depth>]]]) — ряд Тейлора: /sym series(sin(x), x, 0, 7). Значения по умолчанию: var=x, point=0, depth=5.\n\
* solve(<уравнение>, <переменная>) — линейное уравнение: /sym solve(2*x - 4, x). Для системы: /sym solve(2*x + y - 1, x + y + 1, x, y) (сначала все уравнения, потом все переменные).\n\
* replace(<выр>, <шаблон>, <замена>) — переписывание по шаблону. Идентификаторы с подчёркиванием в конце — wildcard: /sym replace(f(1,2,x) + f(1,2,3), f(1,2,y_), f(1,2,y_+1)).\n\
* <выр> — просто разобрать и привести к канонической форме\n\
Для численных вычислений см. /calc — там точная арифметика, единицы измерения и даты.\n\
кря-кря.";

const HELP_INLINE: &str = "/inline — переключить расширенный inline-режим (только в личке у кряквы).\n\
По умолчанию инлайн (@<бот> ...) возвращает только зверушку дня — это легаси и оно не меняется.\n\
После включения в инлайне доступны:\n\
* @<бот> roll 2d6 — кубики\n\
* @<бот> pick чай, кофе, борщ — выбор из списка\n\
* @<бот> horoscope — гороскоп\n\
* @<бот> typst x^2 + 1 — рендер typst-математики\n\
* @<бот> latex \\frac{1}{2} — рендер latex-математики через mitex\n\
* @<бот> math <код> — кряква сама поймёт, typst это или latex\n\
* @<бот> $$<код>$$ — кряква отрендерит\n\
* @<бот> calc 60 mph in m/s — посчитать\n\
* @<бот> $2^256$ — кряква посчитает\n\
* @<бот> plot sin(x), 0, 2*pi — нарисовать график\n\
* @<бот> creature — зверушка дня\n\
/inline ещё раз — выключить обратно.\n\
кря-кря.";

const HELP_FEATURE: &str = "/feature — управление флагами команд в этом чате.\n\
Имена флагов — это пути с точками (`fun.roll`, `nix.npkg`).\n\
Просмотр:\n\
* /feature — все флаги и активные правила.\n\
* /feature nix — состояние всего поддерева nix.\n\
Изменение (только для админов чата):\n\
* /feature nix.* on — включить всё под nix.\n\
* /feature nix.npkg off — выключить конкретный флаг.\n\
* /feature fun.roll, nix.* reset — снять правила с перечисленных шаблонов.\n\
* /feature util.** reset — рекурсивно удалить все правила под util.*.\n\
Правило с более длинным буквенным префиксом побеждает: /feature nix.* on плюс /feature nix.npkg off оставляет npkg выключенным, а остальное nix-* включённым.\n\
кря-кря.";

const HELP_CHA: &str = "/cha — чайная сессия.\n\
Кряква считает заварки и помнит, кто сейчас пьёт чай.\n\
* /cha <название> — начать сессию (название — свободный текст)\n\
* /sip — следующая заварка в твоей активной сессии\n\
* /cha note <заметка> — добавить заметку\n\
* /cha end — закрыть сессию\n\
* /cha who — кто сейчас пьёт в этом чате\n\
* /cha log — последние 10 закрытых сессий в этом чате\n\
* /cha gossip — кто сейчас пьёт в любых ваших общих чатах (opt-in)\n\
* /cha gossip on|off — подключиться/отключиться от кросс-чатовой видимости\n\
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
        "nchan" => HELP_NCHAN.to_string(),
        "nflake" => HELP_NFLAKE.to_string(),
        "feature" => HELP_FEATURE.to_string(),
        "typst" => HELP_TYPST.to_string(),
        "latex" | "tex" => HELP_LATEX.to_string(),
        "math" => HELP_MATH.to_string(),
        "inline" => HELP_INLINE.to_string(),
        "calc" => HELP_CALC.to_string(),
        "sym" => HELP_SYM.to_string(),
        "plot" => HELP_PLOT.to_string(),
        "theme" | "dark" => HELP_THEME.to_string(),
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

async fn handle_text(
    bot: Bot,
    msg: Message,
    mallard: SharedMallard,
    config: BotConfig,
) -> anyhow::Result<()> {
    track_membership(&msg, &config).await;
    ambient_fenced(
        &bot,
        &msg,
        &config,
        "ambient.typst.fenced",
        &["typst"],
        crate::math::Dialect::Typst,
    )
    .await;
    ambient_fenced(
        &bot,
        &msg,
        &config,
        "ambient.latex.fenced",
        &["latex", "tex"],
        crate::math::Dialect::Latex,
    )
    .await;
    ambient_math_dollar(&bot, &msg, &config).await;
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
    track_membership(&msg, &config).await;
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
        Command::Nchan(r) => format!("nchan {r}").trim().to_string(),
        Command::Nflake(r) => format!("nflake {r}").trim().to_string(),
        Command::Feature(r) => format!("feature {r}").trim().to_string(),
        Command::Typst(r) => format!("typst {r}").trim().to_string(),
        Command::Latex(r) => format!("latex {r}").trim().to_string(),
        Command::Tex(r) => format!("tex {r}").trim().to_string(),
        Command::Math(r) => format!("math {r}").trim().to_string(),
        Command::Inline => "inline".to_string(),
        Command::Calc(r) => format!("calc {r}").trim().to_string(),
        Command::Sym(r) => format!("sym {r}").trim().to_string(),
        Command::Plot(r) => format!("plot {r}").trim().to_string(),
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
        Command::Roll(rest) => handle_roll(&bot, &msg, &rest, &config).await,
        Command::Pick(rest) => handle_pick(&bot, &msg, &rest, &config).await,
        Command::Horoscope => handle_horoscope(&bot, &msg, &config).await,
        Command::Cha(rest) => handle_cha(&bot, &msg, &rest, &config).await,
        Command::Sip => handle_sip(&bot, &msg, &config).await,
        Command::Npkg(rest) => handle_npkg(&bot, &msg, &rest, &config).await,
        Command::Nopt(rest) => handle_nopt(&bot, &msg, &rest, &config).await,
        Command::Nixwhere(rest) => handle_nixwhere(&bot, &msg, &rest, &config).await,
        Command::Nchan(rest) => handle_nchan(&bot, &msg, &rest, &config).await,
        Command::Nflake(rest) => handle_nflake(&bot, &msg, &rest, &config).await,
        Command::Feature(rest) => handle_feature(&bot, &msg, &rest, &config).await,
        Command::Typst(rest) => handle_math_cmd(&bot, &msg, &rest, &config, MathRoute::Typst).await,
        Command::Latex(rest) | Command::Tex(rest) => {
            handle_math_cmd(&bot, &msg, &rest, &config, MathRoute::Latex).await
        }
        Command::Math(rest) => handle_math_cmd(&bot, &msg, &rest, &config, MathRoute::Auto).await,
        Command::Inline => handle_inline_toggle(&bot, &msg, &config).await,
        Command::Calc(rest) => handle_calc(&bot, &msg, &rest, &config).await,
        Command::Sym(rest) => handle_sym(&bot, &msg, &rest, &config).await,
        Command::Plot(rest) => handle_plot(&bot, &msg, &rest, &config).await,
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

async fn handle_inline(
    bot: Bot,
    q: InlineQuery,
    mallard: SharedMallard,
    config: BotConfig,
) -> anyhow::Result<()> {
    let user_id = q.from.id;
    let query = q.query.trim();
    log::info!(
        "inline from @{}: {:?}",
        q.from.username.as_deref().unwrap_or("?"),
        q.query
    );

    // Empty query → legacy creature, no opt-in check (no command to dispatch
    // anyway, and the result describes itself).
    if query.is_empty() {
        let creature = { mallard.lock().await.get_creature().to_string() };
        let results = vec![inline_creature_result(&creature, None)];
        bot.answer_inline_query(q.id, results)
            .cache_time(60 * 60 * 3)
            .is_personal(true)
            .await?;
        return Ok(());
    }

    // Non-empty query: gated. Admin auto-opted-in.
    let opted_in = config.admin_id == Some(user_id)
        || config
            .db
            .inline_opt_in_get(user_id.0 as i64)
            .await
            .unwrap_or(false);

    if !opted_in {
        let creature = { mallard.lock().await.get_creature().to_string() };
        let hint = "напишите /inline крякве в личке, чтобы включить roll / pick / horoscope / calc / typst в инлайне.";
        let results = vec![inline_creature_result(&creature, Some(hint))];
        bot.answer_inline_query(q.id, results)
            .cache_time(0)
            .is_personal(true)
            .await?;
        return Ok(());
    }

    // `$$ ... $$` shortcut: whole query wrapped in double dollars →
    // auto-detect dialect and render as a photo. Matches the chat-side
    // `ambient.math.dollar` convention but only available inline (no
    // false-positive risk because the user typed it deliberately).
    if let Some(inner) = query.strip_prefix("$$").and_then(|s| s.strip_suffix("$$")) {
        let expr = inner.trim();
        if !expr.is_empty() {
            let results = inline_render(&bot, &config, expr, MathRoute::Auto).await;
            bot.answer_inline_query(q.id, results)
                .cache_time(0)
                .is_personal(true)
                .await?;
            return Ok(());
        }
    }

    // `$ ... $` shortcut: whole query wrapped in single dollars → calc.
    // Compute, not render. Checked after `$$ ... $$` so the longer form
    // wins. Whitespace tolerance: any single-$ on each side.
    if query.starts_with('$') && query.ends_with('$') && query.len() >= 3 {
        let inner = &query[1..query.len() - 1];
        let expr = inner.trim();
        if !expr.is_empty() {
            let results = inline_calc(expr).await;
            bot.answer_inline_query(q.id, results)
                .cache_time(0)
                .is_personal(true)
                .await?;
            return Ok(());
        }
    }

    // Opted-in dispatch on the first word.
    let mut parts = query.splitn(2, char::is_whitespace);
    let verb = parts.next().unwrap_or("").to_lowercase();
    let rest = parts.next().unwrap_or("").trim();

    let results: Vec<InlineQueryResult> = match verb.as_str() {
        "roll" | "r" | "/roll" => inline_roll(rest),
        "pick" | "p" | "/pick" => inline_pick(rest),
        "horoscope" | "h" | "/horoscope" => inline_horoscope(),
        "typst" | "/typst" => inline_render(&bot, &config, rest, MathRoute::Typst).await,
        "latex" | "tex" | "/latex" | "/tex" => {
            inline_render(&bot, &config, rest, MathRoute::Latex).await
        }
        "math" | "/math" => inline_render(&bot, &config, rest, MathRoute::Auto).await,
        "calc" | "c" | "/calc" => inline_calc(rest).await,
        "plot" | "/plot" => inline_plot(&bot, &config, rest).await,
        "creature" | "ква" | "/creature" => {
            let creature = { mallard.lock().await.get_creature().to_string() };
            vec![inline_creature_result(&creature, None)]
        }
        _ => {
            let creature = { mallard.lock().await.get_creature().to_string() };
            let hint = "не ква, такой команды у кряквы пока нет в инлайне. умеет: roll, pick, horoscope, typst, latex, math, calc, plot, creature.";
            vec![inline_creature_result(&creature, Some(hint))]
        }
    };

    bot.answer_inline_query(q.id, results)
        .cache_time(0)
        .is_personal(true)
        .await?;
    Ok(())
}

fn inline_creature_result(creature: &str, description: Option<&str>) -> InlineQueryResult {
    let mut article = InlineQueryResultArticle::new(
        uuid::Uuid::new_v4().to_string(),
        "Кто ты сегодня?",
        InputMessageContent::Text(
            InputMessageContentText::new(format!("<i>{creature}</i>")).parse_mode(ParseMode::Html),
        ),
    );
    if let Some(d) = description {
        article = article.description(d.to_string());
    }
    InlineQueryResult::Article(article)
}

fn inline_roll(rest: &str) -> Vec<InlineQueryResult> {
    let expr = if rest.is_empty() { "1d6" } else { rest };
    let (body, as_html) = match caith::Roller::new(expr) {
        Ok(roller) => match roller.roll() {
            Ok(result) => (
                format!("\u{1F3B2} {}", caith_md_to_html(&result.to_string())),
                true,
            ),
            Err(e) => (format!("не ква, не получилось бросить: {e}"), false),
        },
        Err(e) => (format!("не ква, не понял выражение: {e}"), false),
    };
    // Strip HTML tags for the article title (Telegram doesn't render
    // formatting in titles, just truncates).
    let title = strip_tags(&body);
    let content = if as_html {
        InputMessageContent::Text(InputMessageContentText::new(body).parse_mode(ParseMode::Html))
    } else {
        InputMessageContent::Text(InputMessageContentText::new(body))
    };
    vec![InlineQueryResult::Article(
        InlineQueryResultArticle::new(uuid::Uuid::new_v4().to_string(), title, content)
            .description(format!("/roll {expr}")),
    )]
}

fn inline_pick(rest: &str) -> Vec<InlineQueryResult> {
    use rand::seq::SliceRandom;
    let options: Vec<&str> = rest
        .split([',', '|', ';'])
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .collect();
    if options.len() < 2 {
        let body = "перечисли через запятую хотя бы два варианта: pick чай, кофе, борщ";
        return vec![InlineQueryResult::Article(InlineQueryResultArticle::new(
            uuid::Uuid::new_v4().to_string(),
            "не хватает вариантов",
            InputMessageContent::Text(InputMessageContentText::new(body.to_string())),
        ))];
    }
    let chosen: String = {
        let mut rng = rand::thread_rng();
        options.choose(&mut rng).copied().unwrap_or("").to_string()
    };
    let body = format!("\u{1F50D} {chosen}");
    vec![InlineQueryResult::Article(
        InlineQueryResultArticle::new(
            uuid::Uuid::new_v4().to_string(),
            body.clone(),
            InputMessageContent::Text(InputMessageContentText::new(body)),
        )
        .description(format!("из {} вариантов", options.len())),
    )]
}

async fn inline_render(
    bot: &Bot,
    config: &BotConfig,
    rest: &str,
    route: MathRoute,
) -> Vec<InlineQueryResult> {
    let rest = rest.trim();
    if rest.is_empty() {
        let body = match route {
            MathRoute::Typst => "пример: typst x^2 + 1",
            MathRoute::Latex => "пример: latex \\frac{1}{2}",
            MathRoute::Auto => "пример: math \\frac{1}{2}",
        };
        return vec![InlineQueryResult::Article(InlineQueryResultArticle::new(
            uuid::Uuid::new_v4().to_string(),
            body,
            InputMessageContent::Text(InputMessageContentText::new(body.to_string())),
        ))];
    }

    let dialect = match route {
        MathRoute::Typst => crate::math::Dialect::Typst,
        MathRoute::Latex => crate::math::Dialect::Latex,
        MathRoute::Auto => crate::math::detect(rest),
    };

    match render_to_file_ids(bot, config, rest, dialect).await {
        Ok(ids) if !ids.is_empty() => {
            let total = ids.len();
            let file_id = ids.into_iter().next().unwrap();
            let title = match dialect {
                crate::math::Dialect::Typst => format!("typst: {}", short_preview(rest)),
                crate::math::Dialect::Latex => format!("latex: {}", short_preview(rest)),
            };
            let description = if total > 1 {
                format!("страница 1/{total} — для всех страниц используйте /typst в чате")
            } else {
                dialect.name().to_string()
            };
            vec![InlineQueryResult::CachedPhoto(
                InlineQueryResultCachedPhoto::new(uuid::Uuid::new_v4().to_string(), file_id)
                    .title(title)
                    .description(description),
            )]
        }
        Ok(_) => {
            let body = "ничего не вышло — пустой результат.";
            vec![InlineQueryResult::Article(InlineQueryResultArticle::new(
                uuid::Uuid::new_v4().to_string(),
                "пусто",
                InputMessageContent::Text(InputMessageContentText::new(body.to_string())),
            ))]
        }
        Err(e) => {
            let text = e.to_string();
            let trimmed: String = if text.chars().count() > 200 {
                let mut s: String = text.chars().take(200).collect();
                s.push('…');
                s
            } else {
                text
            };
            let title = format!("\u{26A0}\u{FE0F} ошибка ({})", dialect.name());
            vec![InlineQueryResult::Article(
                InlineQueryResultArticle::new(
                    uuid::Uuid::new_v4().to_string(),
                    title,
                    InputMessageContent::Text(InputMessageContentText::new(trimmed.clone())),
                )
                .description(trimmed),
            )]
        }
    }
}

fn short_preview(s: &str) -> String {
    let stripped: String = s.split_whitespace().collect::<Vec<_>>().join(" ");
    if stripped.chars().count() > 50 {
        let mut t: String = stripped.chars().take(50).collect();
        t.push('…');
        t
    } else {
        stripped
    }
}

async fn inline_plot(bot: &Bot, config: &BotConfig, rest: &str) -> Vec<InlineQueryResult> {
    let args = rest.trim();
    if args.is_empty() {
        let body = "пример: plot sin(x), 0, 2*pi";
        return vec![InlineQueryResult::Article(InlineQueryResultArticle::new(
            uuid::Uuid::new_v4().to_string(),
            body,
            InputMessageContent::Text(InputMessageContentText::new(body.to_string())),
        ))];
    }
    match plot_to_file_ids(bot, config, args).await {
        Ok(ids) if !ids.is_empty() => {
            let file_id = ids.into_iter().next().unwrap();
            let title = format!("plot: {}", short_preview(args));
            vec![InlineQueryResult::CachedPhoto(
                InlineQueryResultCachedPhoto::new(uuid::Uuid::new_v4().to_string(), file_id)
                    .title(title)
                    .description(format!("/plot {}", short_preview(args))),
            )]
        }
        Ok(_) => vec![InlineQueryResult::Article(InlineQueryResultArticle::new(
            uuid::Uuid::new_v4().to_string(),
            "пусто",
            InputMessageContent::Text(InputMessageContentText::new("пусто".to_string())),
        ))],
        Err(e) => {
            let text = e.to_string();
            let trimmed: String = if text.chars().count() > 200 {
                let mut s: String = text.chars().take(200).collect();
                s.push('…');
                s
            } else {
                text
            };
            vec![InlineQueryResult::Article(
                InlineQueryResultArticle::new(
                    uuid::Uuid::new_v4().to_string(),
                    format!("\u{26A0}\u{FE0F} {trimmed}"),
                    InputMessageContent::Text(InputMessageContentText::new(trimmed.clone())),
                )
                .description(trimmed),
            )]
        }
    }
}

async fn inline_calc(rest: &str) -> Vec<InlineQueryResult> {
    let expr = rest.trim();
    if expr.is_empty() {
        let body = "пример: calc 60 mph in m/s";
        return vec![InlineQueryResult::Article(InlineQueryResultArticle::new(
            uuid::Uuid::new_v4().to_string(),
            body,
            InputMessageContent::Text(InputMessageContentText::new(body.to_string())),
        ))];
    }
    let opts = crate::calc::CalcOpts::default();
    match crate::calc::evaluate(expr, &opts).await {
        Ok(result) => {
            let trimmed = if result.chars().count() > 3500 {
                let mut t: String = result.chars().take(3500).collect();
                t.push('…');
                t
            } else {
                result.clone()
            };
            let body = format!("{} = {}", expr, trimmed);
            let title = short_preview(&format!("= {}", trimmed));
            vec![InlineQueryResult::Article(
                InlineQueryResultArticle::new(
                    uuid::Uuid::new_v4().to_string(),
                    title,
                    InputMessageContent::Text(InputMessageContentText::new(body)),
                )
                .description(format!("/calc {}", short_preview(expr))),
            )]
        }
        Err(e) => {
            let text = e.to_string();
            let trimmed: String = if text.chars().count() > 200 {
                let mut s: String = text.chars().take(200).collect();
                s.push('…');
                s
            } else {
                text
            };
            vec![InlineQueryResult::Article(
                InlineQueryResultArticle::new(
                    uuid::Uuid::new_v4().to_string(),
                    format!("\u{26A0}\u{FE0F} {trimmed}"),
                    InputMessageContent::Text(InputMessageContentText::new(trimmed.clone())),
                )
                .description(trimmed),
            )]
        }
    }
}

fn inline_horoscope() -> Vec<InlineQueryResult> {
    use crate::dictionaries::CREATURES;
    use rand::seq::SliceRandom;
    let mut rng = rand::thread_rng();
    let creature = CREATURES.choose(&mut rng).copied().unwrap_or("Я уточка!");
    let line = HOROSCOPE_LINES
        .choose(&mut rng)
        .copied()
        .unwrap_or("сегодня день будет.");
    let body = format!("\u{1F52E} <i>{creature}</i>\n{line}");
    let title = format!("\u{1F52E} {line}");
    vec![InlineQueryResult::Article(InlineQueryResultArticle::new(
        uuid::Uuid::new_v4().to_string(),
        title,
        InputMessageContent::Text(InputMessageContentText::new(body).parse_mode(ParseMode::Html)),
    ))]
}

/// Strip well-formed HTML tags (`<…>`) — used for inline article titles
/// since Telegram renders titles as plain text.
fn strip_tags(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut in_tag = false;
    for c in s.chars() {
        match c {
            '<' => in_tag = true,
            '>' => in_tag = false,
            _ if !in_tag => out.push(c),
            _ => {}
        }
    }
    out
}

async fn handle_inline_toggle(
    bot: &Bot,
    msg: &Message,
    config: &BotConfig,
) -> anyhow::Result<()> {
    if !matches!(msg.chat.kind, ChatKind::Private(_)) {
        bot.send_message(msg.chat.id, "только в личке: напишите /inline крякве в DM.")
            .reply_parameters(reply_params(msg))
            .await?;
        return Ok(());
    }
    let Some(from) = msg.from.as_ref() else {
        return Ok(());
    };
    let new_state = config.db.inline_opt_in_toggle(from.id.0 as i64).await?;
    let body = if new_state {
        format!(
            "inline-режим включён. в любом чате наберите:\n\
             @{u} roll 2d6\n\
             @{u} pick чай, кофе, борщ\n\
             @{u} horoscope\n\
             /inline ещё раз — выключить.",
            u = config.bot_username
        )
    } else {
        "inline-режим выключен. /inline ещё раз — обратно включить.".to_string()
    };
    bot.send_message(msg.chat.id, body)
        .reply_parameters(reply_params(msg))
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

async fn handle_roll(
    bot: &Bot,
    msg: &Message,
    rest: &str,
    config: &BotConfig,
) -> anyhow::Result<()> {
    if !is_feature_enabled(config, msg.chat.id, "fun.roll").await {
        return Ok(());
    }
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

async fn handle_pick(
    bot: &Bot,
    msg: &Message,
    rest: &str,
    config: &BotConfig,
) -> anyhow::Result<()> {
    if !is_feature_enabled(config, msg.chat.id, "fun.pick").await {
        return Ok(());
    }
    use rand::seq::SliceRandom;
    let options: Vec<&str> = rest
        .split([',', '|', ';'])
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

async fn handle_horoscope(bot: &Bot, msg: &Message, config: &BotConfig) -> anyhow::Result<()> {
    if !is_feature_enabled(config, msg.chat.id, "fun.horoscope").await {
        return Ok(());
    }
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
    if !is_feature_enabled(config, msg.chat.id, "tea.cha").await {
        return Ok(());
    }
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
        "gossip" => cha_gossip(bot, msg, config, sub_arg).await,
        "end" => cha_end(bot, msg, config, key).await,
        "note" if !sub_arg.is_empty() => cha_note(bot, msg, config, key, sub_arg).await,
        _ => cha_start(bot, msg, config, chat_id, user_id, user_name, trimmed).await,
    }
}

async fn handle_sip(bot: &Bot, msg: &Message, config: &BotConfig) -> anyhow::Result<()> {
    if !is_feature_enabled(config, msg.chat.id, "tea.sip").await {
        return Ok(());
    }
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
                "\u{1F375} {ord}-я заварка, {name}, {tea}",
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
            "\u{1F375} {}, {} заварок, {}",
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
        "\u{1FAD6} {} закрыл(а) сессию: {}, {} заварок, {}\nкряква уважает",
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
        "тихо :(\nможет ты начнёшь?".to_string()
    } else {
        let mut lines = vec!["\u{1F375} кто сейчас пьёт:".to_string()];
        for (name, tea, steeps, elapsed, idle) in active {
            lines.push(format!(
                "  {name}, {tea}, {steeps} заварок, {ago}",
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
        "журнал пуст :(\nначни через /cha <название>".to_string()
    } else {
        let mut lines = vec!["\u{1F4D6} последние сессии:".to_string()];
        for r in rows {
            let when = format_unix_short(r.started_at);
            let dur = fmt_dur(std::time::Duration::from_secs(r.duration_s as u64));
            let auto = if r.auto_closed { ", авто" } else { "" };
            let notes = parse_notes(&r.notes_json);
            let mut entry = format!(
                "  {when}, {}, {}, {} заварок, {dur}{auto}",
                r.user_name, r.tea, r.steeps
            );
            if !notes.is_empty() {
                entry.push_str(&format!("\n    заметки: {}", notes.join(", ")));
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
    if !is_feature_enabled(config, msg.chat.id, "nix.npkg").await {
        return Ok(());
    }
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
            "\u{1F4E6} <b>{}</b>, {}",
            html_escape(&top.attr_name),
            html_escape(&top.version)
        )];
        // Health flags first, prominently.
        let mut flags = Vec::new();
        if top.broken {
            flags.push("сломан");
        }
        if top.insecure {
            flags.push("небезопасен");
        }
        if top.unfree {
            flags.push("несвободная лицензия");
        }
        if !flags.is_empty() {
            lines.push(format!("\u{26A0}\u{FE0F} {}", flags.join(", ")));
        }
        lines.push(html_escape(descr));
        if !top.homepage.is_empty() {
            lines.push(format!(
                "🏠 <a href=\"{0}\">{0}</a>",
                html_escape(&top.homepage)
            ));
        }
        if !top.license.is_empty() {
            lines.push(format!("📜 {}", html_escape(&top.license)));
        }
        if !top.platforms.is_empty() {
            lines.push(format!("🖥 {}", format_platforms(&top.platforms)));
        }
        if !top.maintainers.is_empty() {
            lines.push(format!(
                "👥 {}",
                html_escape(&format_maintainers(&top.maintainers))
            ));
        }
        if let Some(link) = position_to_url(&top.position) {
            let display = top.position.split('/').next_back().unwrap_or(&top.position);
            lines.push(format!(
                "↳ <a href=\"{}\">{}</a>",
                html_escape(&link),
                html_escape(display)
            ));
        }
        lines.push(format!(
            "• <code>nix run nixpkgs#{}</code>",
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
    if !is_feature_enabled(config, msg.chat.id, "nix.nopt").await {
        return Ok(());
    }
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
    if !is_feature_enabled(config, msg.chat.id, "nix.nixwhere").await {
        return Ok(());
    }
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
        let mut lines = vec![format!("\u{1F50E} <code>{}</code> →", html_escape(q))];
        for h in &hits {
            let main_suffix = if !h.main_program.is_empty() && h.main_program != q {
                format!(" (main: <code>{}</code>)", html_escape(&h.main_program))
            } else {
                String::new()
            };
            let descr = if h.description.is_empty() {
                String::new()
            } else {
                format!(" — {}", html_escape(&h.description))
            };
            lines.push(format!(
                "• <code>{}</code>{}{}",
                html_escape(&h.attr_name),
                main_suffix,
                descr
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

/// Compact platform-list summary: group by OS, list architectures per OS,
/// elide when the list is suspiciously "all platforms".
fn format_platforms(csv: &str) -> String {
    use std::collections::BTreeMap;
    let entries: Vec<&str> = csv.split(',').filter(|s| !s.is_empty()).collect();
    if entries.is_empty() {
        return String::new();
    }
    if entries.len() > 40 {
        // Almost certainly `lib.platforms.all` — not useful to enumerate.
        return "почти любая платформа".to_string();
    }
    // Parse `<arch>-<os>` (split on the LAST hyphen since arches contain hyphens too).
    let mut by_os: BTreeMap<&str, Vec<&str>> = BTreeMap::new();
    for e in &entries {
        if let Some(idx) = e.rfind('-') {
            let arch = &e[..idx];
            let os = &e[idx + 1..];
            by_os.entry(os).or_default().push(arch);
        }
    }
    // Prioritise linux + darwin first; others alphabetical.
    let mut parts = Vec::new();
    let preferred = ["linux", "darwin"];
    for &os in &preferred {
        if let Some(arches) = by_os.remove(os) {
            parts.push(format!("{} ({})", os, arches.join(", ")));
        }
    }
    for (os, arches) in by_os {
        parts.push(format!("{} ({})", os, arches.join(", ")));
    }
    parts.join(", ")
}

/// Format the maintainer CSV — keep github handles with `@`, raw names as-is.
fn format_maintainers(csv: &str) -> String {
    csv.split(',')
        .filter_map(|s| {
            let s = s.trim();
            if s.is_empty() {
                None
            } else if s.contains(' ') {
                // Names with spaces: probably a real name (no github handle).
                Some(s.to_string())
            } else {
                Some(format!("@{s}"))
            }
        })
        .collect::<Vec<_>>()
        .join(", ")
}

/// Render a nixpkgs source position (`path/to/file.nix:62`) as a GitHub
/// permalink on the `nixos-unstable` branch.
fn position_to_url(position: &str) -> Option<String> {
    if position.is_empty() {
        return None;
    }
    let (path, line) = match position.rsplit_once(':') {
        Some((p, n)) => (p, n.parse::<u32>().ok()),
        None => (position, None),
    };
    let mut url = format!(
        "https://github.com/NixOS/nixpkgs/blob/nixos-unstable/{}",
        path
    );
    if let Some(l) = line {
        url.push_str(&format!("#L{l}"));
    }
    Some(url)
}

// ---------- per-chat feature toggles ----------

use crate::features::{self, Action as FeatureAction, Pattern as FeaturePattern};

/// Thin call-site wrapper around [`features::is_enabled`] that takes the
/// teloxide [`ChatId`] directly.
async fn is_feature_enabled(config: &BotConfig, chat: ChatId, name: &str) -> bool {
    features::is_enabled(&config.db, chat.0, name).await
}

/// Bot admin (TG_ADMIN_ID) always allowed; in groups, Telegram chat admins
/// are also allowed; in private chats, the user IS the chat — allow.
async fn can_manage_features(bot: &Bot, msg: &Message, config: &BotConfig) -> bool {
    let Some(from) = msg.from.as_ref() else {
        return false;
    };
    if let Some(admin) = config.admin_id {
        if from.id == admin {
            return true;
        }
    }
    if matches!(msg.chat.kind, ChatKind::Private(_)) {
        return true;
    }
    match bot.get_chat_administrators(msg.chat.id).await {
        Ok(admins) => admins.iter().any(|m| m.user.id == from.id),
        Err(e) => {
            log::warn!("get_chat_administrators({}): {e}", msg.chat.id);
            false
        }
    }
}

async fn handle_feature(
    bot: &Bot,
    msg: &Message,
    rest: &str,
    config: &BotConfig,
) -> anyhow::Result<()> {
    let chat = msg.chat.id;
    let tokens: Vec<&str> = rest.split_whitespace().collect();

    // /feature with no arguments — full listing of every leaf grouped by
    // category, plus the chat's explicit rules.
    if tokens.is_empty() {
        let rules = config
            .db
            .feature_rules_for_chat(chat.0)
            .await
            .unwrap_or_else(|e| {
                log::warn!("feature_rules_for_chat({chat}): {e}");
                Vec::new()
            });
        bot.send_message(chat, render_feature_overview(chat.0, &rules))
            .reply_parameters(reply_params(msg))
            .await?;
        return Ok(());
    }

    // Last token may be an action token — if so this is a write, else it's
    // a scoped query over the listed prefixes.
    let action = FeatureAction::from_token(tokens[tokens.len() - 1]);
    let pattern_slice: &[&str] = match action {
        Some(_) => &tokens[..tokens.len() - 1],
        None => &tokens[..],
    };

    // Split on commas as well as whitespace; dedupe in input order.
    let mut raw_patterns: Vec<String> = Vec::new();
    let mut seen = std::collections::HashSet::new();
    for tok in pattern_slice {
        for raw in tok.split(',') {
            let s = raw.trim();
            if !s.is_empty() && seen.insert(s.to_string()) {
                raw_patterns.push(s.to_string());
            }
        }
    }

    if raw_patterns.is_empty() {
        bot.send_message(
            chat,
            "формат: /feature <шаблон>[, <шаблон>] [on|off|reset]\n\
             пример: /feature nix.* on; /feature util.time.tz off; /feature util.** reset",
        )
        .reply_parameters(reply_params(msg))
        .await?;
        return Ok(());
    }

    // Normalize every pattern up front so query and write share the same
    // validation. Bad patterns are reported alongside results.
    let mut patterns: Vec<FeaturePattern> = Vec::new();
    let mut bad: Vec<String> = Vec::new();
    for raw in &raw_patterns {
        match features::normalize_pattern(raw) {
            Ok(p) => patterns.push(p),
            Err(e) => bad.push(format!("{raw}: {e}")),
        }
    }

    // ===== Query path =====
    if action.is_none() {
        let rules = config
            .db
            .feature_rules_for_chat(chat.0)
            .await
            .unwrap_or_else(|e| {
                log::warn!("feature_rules_for_chat({chat}): {e}");
                Vec::new()
            });
        let mut sections: Vec<String> = patterns
            .iter()
            .map(|p| render_feature_subtree(&rules, p))
            .collect();
        if !bad.is_empty() {
            sections.push(format!("пропущено: {}", bad.join("; ")));
        }
        bot.send_message(chat, sections.join("\n\n"))
            .reply_parameters(reply_params(msg))
            .await?;
        return Ok(());
    }

    // ===== Write path =====
    let action = action.unwrap();

    if !can_manage_features(bot, msg, config).await {
        bot.send_message(chat, "не ква, только админы чата могут это менять.")
            .reply_parameters(reply_params(msg))
            .await?;
        return Ok(());
    }

    // `<prefix>.**` is reset-only — refuse on/off early so the user sees the
    // mistake before any DB write happens.
    if matches!(action, FeatureAction::On | FeatureAction::Off)
        && patterns
            .iter()
            .any(|p| matches!(p, FeaturePattern::Recursive(_)))
    {
        bot.send_message(
            chat,
            "** работает только с reset. для on/off укажите конкретный шаблон, например util.* on",
        )
        .reply_parameters(reply_params(msg))
        .await?;
        return Ok(());
    }

    let mut applied: Vec<String> = Vec::new();
    let mut failed: Vec<String> = Vec::new();
    for p in &patterns {
        let outcome: Result<String, rusqlite::Error> = match (action, p) {
            (FeatureAction::On | FeatureAction::Off, _) => {
                let stored = p
                    .stored()
                    .expect("recursive patterns rejected above for on/off");
                let val = matches!(action, FeatureAction::On);
                config
                    .db
                    .feature_set(chat.0, stored, val)
                    .await
                    .map(|()| p.display())
            }
            (FeatureAction::Reset, FeaturePattern::Recursive(prefix)) => config
                .db
                .feature_clear_prefix(chat.0, prefix)
                .await
                .map(|n| format!("{} ({n})", p.display())),
            (FeatureAction::Reset, _) => {
                let stored = p.stored().expect("non-recursive has a stored form");
                config
                    .db
                    .feature_clear(chat.0, stored)
                    .await
                    .map(|()| p.display())
            }
        };
        match outcome {
            Ok(label) => applied.push(label),
            Err(e) => {
                log::warn!("feature {:?} {}: {e}", action, p.display());
                failed.push(p.display());
            }
        }
    }

    // Recompute effective state across leaves touched by the patterns we
    // applied, so the user sees the resolved ✅/❌.
    let rules = config
        .db
        .feature_rules_for_chat(chat.0)
        .await
        .unwrap_or_default();
    let mut effective: Vec<String> = Vec::new();
    for p in &patterns {
        for leaf in features::leaves_matching(p) {
            let (on, _) = features::resolve(&rules, leaf.path);
            let mark = if on { "\u{2705}" } else { "\u{274C}" };
            effective.push(format!("  {mark} {}", leaf.path));
        }
    }
    effective.sort();
    effective.dedup();

    let mut lines: Vec<String> = Vec::new();
    if !applied.is_empty() {
        lines.push(format!("{}: {}", action.verb(), applied.join(", ")));
    }
    if !effective.is_empty() {
        lines.push("сейчас:".to_string());
        lines.extend(effective);
    }
    if !bad.is_empty() {
        lines.push(format!("пропущено: {}", bad.join("; ")));
    }
    if !failed.is_empty() {
        lines.push(format!("ошибка: {}", failed.join(", ")));
    }
    if lines.is_empty() {
        lines.push("ничего не изменено.".to_string());
    }

    bot.send_message(chat, lines.join("\n"))
        .reply_parameters(reply_params(msg))
        .await?;
    Ok(())
}

/// Full per-chat state: declared rules + effective on/off for every
/// registered leaf, grouped by top-level category.
fn render_feature_overview(chat_id: i64, rules: &[(String, bool)]) -> String {
    let mut lines: Vec<String> = vec![format!("\u{2699}\u{FE0F} флаги в чате {chat_id}:")];

    if rules.is_empty() {
        lines.push("правил нет — всё по умолчанию.".to_string());
    } else {
        lines.push("правила:".to_string());
        for (pat, val) in rules {
            let mark = if *val { "on " } else { "off" };
            lines.push(format!("  {mark}  {pat}"));
        }
    }

    lines.push(String::new());
    lines.push("состояние:".to_string());
    for cat in features::categories() {
        let cat_prefix = format!("{cat}.");
        let mut header_pushed = false;
        for f in features::FEATURES
            .iter()
            .filter(|f| f.path.starts_with(&cat_prefix) || f.path == cat)
        {
            if !header_pushed {
                lines.push(format!("  [{cat}]"));
                header_pushed = true;
            }
            let (on, by) = features::resolve(rules, f.path);
            let mark = if on { "\u{2705}" } else { "\u{274C}" };
            let src = by
                .map(|r| format!("← {r}"))
                .unwrap_or_else(|| "← по умолчанию".to_string());
            lines.push(format!("    {mark} {} {src}", f.path));
        }
    }

    lines.push(String::new());
    lines.push(
        "управление (для админов): /feature <шаблон>[, <шаблон>] on|off|reset".to_string(),
    );
    lines.push(
        "примеры: /feature nix.* on; /feature util.time.tz off; /feature util.** reset".to_string(),
    );
    lines.join("\n")
}

/// Effective state for the leaves under one pattern. Used by the
/// `/feature <prefix>` query form.
fn render_feature_subtree(rules: &[(String, bool)], pat: &FeaturePattern) -> String {
    let mut lines = vec![format!("[{}]", pat.display())];
    let leaves = features::leaves_matching(pat);
    if leaves.is_empty() {
        lines.push("  (пусто)".to_string());
        return lines.join("\n");
    }
    for leaf in leaves {
        let (on, by) = features::resolve(rules, leaf.path);
        let mark = if on { "\u{2705}" } else { "\u{274C}" };
        let src = by
            .map(|r| format!("← {r}"))
            .unwrap_or_else(|| "← по умолчанию".to_string());
        lines.push(format!("  {mark} {} {src}", leaf.path));
    }
    lines.join("\n")
}

// ---------- /nchan + /nflake — live nix channel & flake registry ----------

async fn handle_nchan(
    bot: &Bot,
    msg: &Message,
    rest: &str,
    config: &BotConfig,
) -> anyhow::Result<()> {
    if !is_feature_enabled(config, msg.chat.id, "nix.nchan").await {
        return Ok(());
    }
    let arg = rest.trim();
    let channel = if arg.is_empty() { None } else { Some(arg) };
    let info = match crate::nixstatus::fetch_channel(channel).await {
        Ok(i) => i,
        Err(e) => {
            log::warn!("nchan fetch failed: {e:#}");
            bot.send_message(msg.chat.id, "не ква, channels.nixos.org не отвечает :(")
                .reply_parameters(reply_params(msg))
                .await?;
            return Ok(());
        }
    };
    let short = info.revision.chars().take(12).collect::<String>();
    let mut lines = vec![format!("\u{1F33F} <b>{}</b>", html_escape(&info.channel))];
    if let Some(label) = &info.version_label {
        lines.push(format!("snapshot: <code>{}</code>", html_escape(label)));
    }
    lines.push(format!(
        "коммит: <a href=\"https://github.com/NixOS/nixpkgs/commit/{}\"><code>{}</code></a>",
        html_escape(&info.revision),
        html_escape(&short)
    ));
    bot.send_message(msg.chat.id, lines.join("\n"))
        .parse_mode(ParseMode::Html)
        .reply_parameters(reply_params(msg))
        .await?;
    Ok(())
}

async fn handle_nflake(
    bot: &Bot,
    msg: &Message,
    rest: &str,
    config: &BotConfig,
) -> anyhow::Result<()> {
    if !is_feature_enabled(config, msg.chat.id, "nix.nflake").await {
        return Ok(());
    }
    let q = rest.trim();
    if q.is_empty() {
        bot.send_message(
            msg.chat.id,
            "формат: /nflake <имя>, например /nflake nixpkgs",
        )
        .reply_parameters(reply_params(msg))
        .await?;
        return Ok(());
    }
    let registry = match crate::nixstatus::fetch_flake_registry().await {
        Ok(r) => r,
        Err(e) => {
            log::warn!("nflake fetch failed: {e:#}");
            bot.send_message(msg.chat.id, "не ква, flake-registry.json не отвечает :(")
                .reply_parameters(reply_params(msg))
                .await?;
            return Ok(());
        }
    };
    let Some(entry) = crate::nixstatus::lookup_flake(&registry, q) else {
        bot.send_message(
            msg.chat.id,
            format!("«{}» нет в flake registry. может, добавьте?", q),
        )
        .reply_parameters(reply_params(msg))
        .await?;
        return Ok(());
    };
    let from_id = entry.from.id.as_deref().unwrap_or(q);
    let to_render = crate::nixstatus::to_url(&entry.to);
    let github_link = match (entry.to.kind.as_str(), &entry.to.owner, &entry.to.repo) {
        ("github", Some(owner), Some(repo)) => Some(format!("https://github.com/{owner}/{repo}")),
        _ => None,
    };
    let mut lines = vec![format!("\u{1F9A9} <b>{}</b>", html_escape(from_id))];
    lines.push(format!("→ <code>{}</code>", html_escape(&to_render)));
    if let Some(link) = github_link {
        lines.push(format!(
            "<a href=\"{}\">{}</a>",
            html_escape(&link),
            html_escape(&link)
        ));
    }
    bot.send_message(msg.chat.id, lines.join("\n"))
        .parse_mode(ParseMode::Html)
        .reply_parameters(reply_params(msg))
        .await?;
    Ok(())
}

// ---------- /cha gossip — cross-chat presence ----------

/// Bump (user, chat) membership in the DB. Cheap upsert per incoming msg.
/// Skipped for bots, for messages without a sender, and for private chats
/// (where membership is just user→bot — not useful for cross-chat lookup).
async fn track_membership(msg: &Message, config: &BotConfig) {
    let Some(from) = msg.from.as_ref() else {
        return;
    };
    if from.is_bot {
        return;
    }
    if matches!(msg.chat.kind, ChatKind::Private(_)) {
        return;
    }
    let user_name = from
        .username
        .as_ref()
        .map(|n| format!("@{n}"))
        .unwrap_or_else(|| from.full_name());
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0);
    if let Err(e) = config
        .db
        .bump_membership(from.id.0 as i64, msg.chat.id.0, user_name, now)
        .await
    {
        log::warn!("bump_membership failed: {e}");
    }
}

async fn cha_gossip(
    bot: &Bot,
    msg: &Message,
    config: &BotConfig,
    action: &str,
) -> anyhow::Result<()> {
    let Some(from) = msg.from.as_ref() else {
        return Ok(());
    };
    let user_id = from.id.0 as i64;

    match action {
        "on" | "true" | "enable" | "yes" | "1" => {
            if let Err(e) = config.db.gossip_set(user_id, true).await {
                log::warn!("gossip_set on failed: {e}");
            }
            bot.send_message(msg.chat.id, "🍃 /cha gossip включён. ты теперь видимый.")
                .reply_parameters(reply_params(msg))
                .await?;
            return Ok(());
        }
        "off" | "false" | "disable" | "no" | "0" => {
            if let Err(e) = config.db.gossip_set(user_id, false).await {
                log::warn!("gossip_set off failed: {e}");
            }
            bot.send_message(msg.chat.id, "🍃 /cha gossip выключен.")
                .reply_parameters(reply_params(msg))
                .await?;
            return Ok(());
        }
        "" => {}
        _ => {
            bot.send_message(
                msg.chat.id,
                "формат: /cha gossip on|off (или /cha gossip — список)",
            )
            .reply_parameters(reply_params(msg))
            .await?;
            return Ok(());
        }
    }

    // List mode — requires opt-in.
    let opted_in = config.db.gossip_get(user_id).await.unwrap_or(false);
    if !opted_in {
        bot.send_message(
            msg.chat.id,
            "/cha gossip у тебя выключен. включить: /cha gossip on",
        )
        .reply_parameters(reply_params(msg))
        .await?;
        return Ok(());
    }

    let target_ids: std::collections::HashSet<i64> = match config.db.gossip_targets(user_id).await {
        Ok(ids) => ids.into_iter().collect(),
        Err(e) => {
            log::warn!("gossip_targets failed: {e}");
            std::collections::HashSet::new()
        }
    };

    // Cross-reference against the in-memory session store.
    let snapshot: Vec<(String, String, u32, std::time::Duration)> = {
        let store = config.tea_sessions.lock().await;
        store
            .iter()
            .filter(|((_, user), _)| target_ids.contains(&(user.0 as i64)))
            .map(|(_, s)| (s.user_name.clone(), s.tea.clone(), s.steeps, s.idle_for()))
            .collect()
    };

    let body = if snapshot.is_empty() {
        "тихо :(".to_string()
    } else {
        let mut lines = vec!["\u{1F343} кто сейчас пьёт:".to_string()];
        for (name, tea, steeps, idle) in snapshot {
            lines.push(format!(
                "  {name}, {tea}, {steeps} заварок, {ago}",
                ago = crate::sessions::fmt_dur(idle)
            ));
        }
        lines.join("\n")
    };
    bot.send_message(msg.chat.id, body)
        .reply_parameters(reply_params(msg))
        .await?;
    Ok(())
}

// ---------- /typst, /latex, /math + ambient detectors ----------

#[derive(Clone, Copy)]
enum MathRoute {
    Typst, // /typst — always typst
    Latex, // /latex, /tex — always latex via mitex
    Auto,  // /math — detect from source
}

impl MathRoute {
    fn feature_flag(self) -> &'static str {
        match self {
            Self::Typst => "util.typst",
            Self::Latex => "util.latex",
            Self::Auto => "util.math",
        }
    }

    /// Code-block language tags to prefer when extracting source from a
    /// replied message. Order matters within each route.
    fn preferred_langs(self) -> &'static [&'static str] {
        match self {
            Self::Typst => &["typst"],
            Self::Latex => &["latex", "tex"],
            Self::Auto => &["typst", "latex", "tex"],
        }
    }

    fn example(self) -> &'static str {
        match self {
            Self::Typst => "/typst x^2 + 1",
            Self::Latex => "/latex \\frac{1}{2}",
            Self::Auto => "/math \\frac{1}{2}",
        }
    }
}

/// Pull source out of a replied message. Preference order: language-tagged
/// fenced block matching `preferred_langs` → any fenced block → inline
/// `code` entity → whole message text. Telegram delivers code blocks with
/// the fences already stripped, so we can use the entity text directly.
fn extract_source(msg: &Message, preferred_langs: &[&str]) -> Option<String> {
    use teloxide::types::MessageEntityKind;

    let entities = msg
        .parse_entities()
        .or_else(|| msg.parse_caption_entities());

    if let Some(refs) = entities {
        for e in &refs {
            if let MessageEntityKind::Pre {
                language: Some(lang),
            } = e.kind()
            {
                if preferred_langs
                    .iter()
                    .any(|p| lang.eq_ignore_ascii_case(p))
                {
                    return Some(e.text().to_string());
                }
            }
        }
        for e in &refs {
            if matches!(e.kind(), MessageEntityKind::Pre { .. }) {
                return Some(e.text().to_string());
            }
        }
        for e in &refs {
            if matches!(e.kind(), MessageEntityKind::Code) {
                return Some(e.text().to_string());
            }
        }
    }

    msg.text().or_else(|| msg.caption()).map(|s| s.to_string())
}

async fn handle_math_cmd(
    bot: &Bot,
    msg: &Message,
    rest: &str,
    config: &BotConfig,
    route: MathRoute,
) -> anyhow::Result<()> {
    if !is_feature_enabled(config, msg.chat.id, route.feature_flag()).await {
        return Ok(());
    }

    let inline = rest.trim();
    let source: String = if !inline.is_empty() {
        inline.to_string()
    } else if let Some(reply) = msg.reply_to_message() {
        match extract_source(reply, route.preferred_langs()) {
            Some(t) => t,
            None => {
                bot.send_message(
                    msg.chat.id,
                    "не ква, ответьте на сообщение с кодом или напишите код после команды.",
                )
                .reply_parameters(reply_params(msg))
                .await?;
                return Ok(());
            }
        }
    } else {
        bot.send_message(
            msg.chat.id,
            format!(
                "пример: {}\nили ответьте этой командой на сообщение с кодом.",
                route.example()
            ),
        )
        .reply_parameters(reply_params(msg))
        .await?;
        return Ok(());
    };

    let dialect = match route {
        MathRoute::Typst => crate::math::Dialect::Typst,
        MathRoute::Latex => crate::math::Dialect::Latex,
        MathRoute::Auto => crate::math::detect(&source),
    };

    let opts = typst_opts_for(config, msg.chat.id).await;
    match crate::math::render(&source, dialect, &opts).await {
        Ok(pages) => {
            send_typst_pages(bot, msg, pages).await?;
        }
        Err(e) => {
            let text = e.to_string();
            let trimmed: String = if text.chars().count() > 800 {
                let mut s: String = text.chars().take(800).collect();
                s.push('…');
                s
            } else {
                text
            };
            bot.send_message(msg.chat.id, format!("\u{26A0}\u{FE0F} {trimmed}"))
                .reply_parameters(reply_params(msg))
                .await?;
        }
    }
    Ok(())
}

// ---------- ambient renderers ----------

/// Shared helper for `ambient.<dialect>.fenced` flags. Looks for a Pre
/// entity tagged with any of `langs`. Errors are logged, never echoed —
/// the user didn't explicitly ask for a render.
async fn ambient_fenced(
    bot: &Bot,
    msg: &Message,
    config: &BotConfig,
    flag: &'static str,
    langs: &[&str],
    dialect: crate::math::Dialect,
) {
    if !is_feature_enabled(config, msg.chat.id, flag).await {
        return;
    }
    let Some(refs) = msg
        .parse_entities()
        .or_else(|| msg.parse_caption_entities())
    else {
        return;
    };
    use teloxide::types::MessageEntityKind;
    let source = refs.iter().find_map(|e| match e.kind() {
        MessageEntityKind::Pre {
            language: Some(lang),
        } if langs.iter().any(|l| lang.eq_ignore_ascii_case(l)) => Some(e.text().to_string()),
        _ => None,
    });
    let Some(source) = source else { return };

    let opts = typst_opts_for(config, msg.chat.id).await;
    match crate::math::render(&source, dialect, &opts).await {
        Ok(pages) => {
            if let Err(e) = send_typst_pages(bot, msg, pages).await {
                log::warn!("ambient {flag} send: {e}");
            }
        }
        Err(e) => {
            log::info!("ambient {flag} skipped: {e}");
        }
    }
}

/// `ambient.math.dollar`: render the first `$$ ... $$` block in the
/// message. Dialect auto-detected from the content. Plain single `$ $`
/// is intentionally ignored — too many false positives (prices, code, …).
async fn ambient_math_dollar(bot: &Bot, msg: &Message, config: &BotConfig) {
    if !is_feature_enabled(config, msg.chat.id, "ambient.math.dollar").await {
        return;
    }
    let Some(text) = msg.text().or_else(|| msg.caption()) else {
        return;
    };
    let Some(start) = text.find("$$") else { return };
    let after = &text[start + 2..];
    let Some(end_rel) = after.find("$$") else { return };
    let source = after[..end_rel].trim();
    if source.is_empty() {
        return;
    }

    let dialect = crate::math::detect(source);
    let opts = typst_opts_for(config, msg.chat.id).await;
    match crate::math::render(source, dialect, &opts).await {
        Ok(pages) => {
            if let Err(e) = send_typst_pages(bot, msg, pages).await {
                log::warn!("ambient math.dollar send: {e}");
            }
        }
        Err(e) => log::info!("ambient math.dollar skipped: {e}"),
    }
}

/// Build `RenderOpts` with env-driven defaults (package cache path from
/// env). Theme stays at the type default (Light); call sites that have
/// access to a chat layer use [`typst_opts_for`] to override per-chat.
fn typst_opts_from_env() -> crate::typst::RenderOpts {
    let mut opts = crate::typst::RenderOpts::default();
    if let Ok(path) = std::env::var("TYPST_PACKAGE_CACHE_PATH") {
        opts.package_cache_path = Some(std::path::PathBuf::from(path));
    }
    opts
}

/// Build `RenderOpts` with the chat's theme preference applied.
async fn typst_opts_for(config: &BotConfig, chat: ChatId) -> crate::typst::RenderOpts {
    let mut opts = typst_opts_from_env();
    if is_feature_enabled(config, chat, "util.theme.dark").await {
        opts.theme = crate::typst::Theme::Dark;
    }
    opts
}

/// Cache key bound to render-affecting options. Bump `SCHEMA` when the
/// preamble, padding, font picks, or mitex version change in a way that
/// should invalidate previously-stored file_ids.
fn render_cache_key(
    dialect: crate::math::Dialect,
    doc: &str,
    opts: &crate::typst::RenderOpts,
) -> String {
    use sha2::{Digest, Sha256};
    const SCHEMA: u32 = 1;
    let mut h = Sha256::new();
    h.update(b"mallard-render");
    h.update(SCHEMA.to_le_bytes());
    h.update(b"|");
    h.update(dialect.name().as_bytes());
    h.update(b"|");
    h.update(opts.ppi.to_le_bytes());
    h.update(opts.text_size_pt.to_le_bytes());
    h.update(opts.min_width_px.to_le_bytes());
    h.update(opts.min_height_px.to_le_bytes());
    h.update(b"|");
    h.update(opts.theme.name().as_bytes());
    h.update(b"|");
    h.update(doc.as_bytes());
    hex::encode(h.finalize())
}

/// Upload `bytes` as a silent photo to the admin's DM (the "stash chat"),
/// capture the photo's `file_id`, then delete the message. Telegram keeps
/// the underlying photo file around indefinitely — the `file_id` we
/// captured stays valid forever and can be referenced from any future
/// answer (inline result, send_photo by id, …) without re-uploading.
async fn stash_upload(bot: &Bot, stash_chat: ChatId, bytes: Vec<u8>) -> anyhow::Result<FileId> {
    let m = bot
        .send_photo(stash_chat, InputFile::memory(bytes))
        .disable_notification(true)
        .await?;
    let file_id = m
        .photo()
        .and_then(|sizes| sizes.last())
        .map(|s| s.file.id.clone())
        .ok_or_else(|| anyhow::anyhow!("stash response had no photo"))?;
    if let Err(e) = bot.delete_message(stash_chat, m.id).await {
        // Not fatal — the file_id is captured regardless. Log so we notice
        // if the admin's DM accumulates renders.
        log::warn!("stash delete_message failed: {e}");
    }
    Ok(file_id)
}

/// Compile a typst `doc` to pages, upload each to the stash, and cache
/// the resulting `file_id`s under `cache_key`. Repeat calls with the same
/// key are a single SQLite read. Shared between math and plot renders;
/// callers build their own document + cache key for namespace separation.
async fn doc_to_file_ids(
    bot: &Bot,
    config: &BotConfig,
    cache_key: &str,
    doc: &str,
) -> anyhow::Result<Vec<FileId>> {
    if let Ok(Some(ids)) = config.db.render_cache_get(cache_key).await {
        log::debug!("render cache hit: {cache_key}");
        return Ok(ids.into_iter().map(FileId).collect());
    }

    let admin = config
        .admin_id
        .ok_or_else(|| anyhow::anyhow!("инлайн-картинки требуют настроенного админа"))?;
    let stash_chat = ChatId(admin.0 as i64);

    let opts = typst_opts_from_env();
    let pages = crate::typst::compile_doc(doc, &opts)
        .await
        .map_err(|e| anyhow::anyhow!("{e}"))?;

    let mut ids = Vec::with_capacity(pages.len());
    for bytes in pages {
        let id = stash_upload(bot, stash_chat, bytes).await?;
        ids.push(id);
    }

    let raw_strings: Vec<String> = ids.iter().map(|f| f.0.clone()).collect();
    if let Err(e) = config.db.render_cache_put(cache_key, &raw_strings).await {
        log::warn!("render_cache_put({cache_key}): {e}");
    }

    Ok(ids)
}

/// Resolve `(source, dialect)` to file_ids via the shared cache+stash.
async fn render_to_file_ids(
    bot: &Bot,
    config: &BotConfig,
    source: &str,
    dialect: crate::math::Dialect,
) -> anyhow::Result<Vec<FileId>> {
    let opts = typst_opts_from_env();
    let doc = crate::math::assemble(source, dialect, &opts);
    let key = render_cache_key(dialect, &doc, &opts);
    doc_to_file_ids(bot, config, &key, &doc).await
}

/// Resolve a plot-args string to file_ids via the shared cache+stash.
async fn plot_to_file_ids(
    bot: &Bot,
    config: &BotConfig,
    args: &str,
) -> anyhow::Result<Vec<FileId>> {
    let req = crate::plot::parse_args(args).map_err(|e| anyhow::anyhow!("{e}"))?;
    let opts = typst_opts_from_env();
    let doc = crate::plot::assemble(&req, &opts);
    let key = plot_cache_key(&doc, &opts);
    doc_to_file_ids(bot, config, &key, &doc).await
}

fn plot_cache_key(doc: &str, opts: &crate::typst::RenderOpts) -> String {
    use sha2::{Digest, Sha256};
    const SCHEMA: u32 = 1;
    let mut h = Sha256::new();
    h.update(b"mallard-plot");
    h.update(SCHEMA.to_le_bytes());
    h.update(b"|");
    h.update(opts.ppi.to_le_bytes());
    h.update(opts.text_size_pt.to_le_bytes());
    h.update(b"|");
    h.update(opts.theme.name().as_bytes());
    h.update(b"|");
    h.update(doc.as_bytes());
    hex::encode(h.finalize())
}

/// Single page → `send_photo`. Two-to-ten pages → `send_media_group`
/// (Telegram's album cap). Both reply to the originating message.
async fn send_typst_pages(
    bot: &Bot,
    msg: &Message,
    pages: Vec<Vec<u8>>,
) -> anyhow::Result<()> {
    match pages.len() {
        0 => Ok(()), // render returns NoOutput before this, but guard anyway
        1 => {
            bot.send_photo(
                msg.chat.id,
                InputFile::memory(pages.into_iter().next().unwrap()).file_name("typst.png"),
            )
            .reply_parameters(reply_params(msg))
            .await?;
            Ok(())
        }
        _ => {
            let media: Vec<InputMedia> = pages
                .into_iter()
                .enumerate()
                .map(|(i, bytes)| {
                    InputMedia::Photo(InputMediaPhoto::new(
                        InputFile::memory(bytes).file_name(format!("typst-{}.png", i + 1)),
                    ))
                })
                .collect();
            bot.send_media_group(msg.chat.id, media)
                .reply_parameters(reply_params(msg))
                .await?;
            Ok(())
        }
    }
}

// ---------- /calc — fend-core numeric calculator ----------

async fn handle_calc(
    bot: &Bot,
    msg: &Message,
    rest: &str,
    config: &BotConfig,
) -> anyhow::Result<()> {
    if !is_feature_enabled(config, msg.chat.id, "util.calc").await {
        return Ok(());
    }
    let expr = rest.trim();
    if expr.is_empty() {
        bot.send_message(
            msg.chat.id,
            "пример: /calc 60 mph in m/s, /calc 1/3 + 1/3 + 1/3, /calc 2^256",
        )
        .reply_parameters(reply_params(msg))
        .await?;
        return Ok(());
    }

    let opts = crate::calc::CalcOpts::default();
    let body = match crate::calc::evaluate(expr, &opts).await {
        Ok(result) => format!("= {}", truncate_chars(&result, 3500)),
        Err(e) => {
            let text = e.to_string();
            format!("\u{26A0}\u{FE0F} {}", truncate_chars(&text, 600))
        }
    };
    bot.send_message(msg.chat.id, body)
        .reply_parameters(reply_params(msg))
        .await?;
    Ok(())
}

fn truncate_chars(s: &str, max_chars: usize) -> String {
    if s.chars().count() <= max_chars {
        return s.to_string();
    }
    let mut out: String = s.chars().take(max_chars).collect();
    out.push('…');
    out
}

// ---------- /plot — function plots via typst + cetz-plot ----------

async fn handle_plot(
    bot: &Bot,
    msg: &Message,
    rest: &str,
    config: &BotConfig,
) -> anyhow::Result<()> {
    if !is_feature_enabled(config, msg.chat.id, "util.plot").await {
        return Ok(());
    }
    let args = rest.trim();
    if args.is_empty() {
        bot.send_message(
            msg.chat.id,
            "примеры:\n\
             /plot sin(x), 0, 2*pi\n\
             /plot sin(x), cos(x), -pi, pi\n\
             /plot exp(-x^2), -3, 3",
        )
        .reply_parameters(reply_params(msg))
        .await?;
        return Ok(());
    }

    let render_opts = typst_opts_for(config, msg.chat.id).await;
    match crate::plot::render_args(args, &render_opts).await {
        Ok(pages) => {
            send_typst_pages(bot, msg, pages).await?;
        }
        Err(e) => {
            let text = e.to_string();
            bot.send_message(
                msg.chat.id,
                format!("\u{26A0}\u{FE0F} {}", truncate_chars(&text, 600)),
            )
            .reply_parameters(reply_params(msg))
            .await?;
        }
    }
    Ok(())
}

// ---------- /sym — symbolica CAS ----------

async fn handle_sym(
    bot: &Bot,
    msg: &Message,
    rest: &str,
    config: &BotConfig,
) -> anyhow::Result<()> {
    if !is_feature_enabled(config, msg.chat.id, "util.sym").await {
        return Ok(());
    }
    let expr = rest.trim();
    if expr.is_empty() {
        bot.send_message(
            msg.chat.id,
            "примеры: /sym diff(sin(x), x), /sym factor(x^2-1), /sym expand((x+1)^3), /sym together(1/x + 1/y)",
        )
        .reply_parameters(reply_params(msg))
        .await?;
        return Ok(());
    }

    let opts = crate::sym::SymOpts::default();
    let result = match crate::sym::evaluate(expr, &opts).await {
        Ok(r) => r,
        Err(e) => {
            let text = e.to_string();
            bot.send_message(
                msg.chat.id,
                format!("\u{26A0}\u{FE0F} {}", truncate_chars(&text, 600)),
            )
            .reply_parameters(reply_params(msg))
            .await?;
            return Ok(());
        }
    };

    // Always send the text answer. Then attempt a pretty rendered image
    // via mitex (Symbolica's LaTeX output is mitex-compatible). Render
    // failures are silent — text already conveyed the result.
    let text_body = format!("= {}", truncate_chars(&result.text, 3500));
    bot.send_message(msg.chat.id, text_body)
        .reply_parameters(reply_params(msg))
        .await?;

    let render_opts = typst_opts_for(config, msg.chat.id).await;
    if let Ok(pages) = crate::math::render(
        &result.latex,
        crate::math::Dialect::Latex,
        &render_opts,
    )
    .await
    {
        if let Err(e) = send_typst_pages(bot, msg, pages).await {
            log::warn!("sym render send: {e}");
        }
    }
    Ok(())
}
