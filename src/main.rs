use std::env;
use std::sync::Arc;

use teloxide::prelude::*;
use teloxide::types::UserId;
use tokio::sync::Mutex;

use teloxide::types::{BotCommandScope, Recipient};
use teloxide::utils::command::BotCommands as _;

use mallard_bot::bot::{build_dispatcher, BotConfig, Command};
use mallard_bot::db::Db;
use mallard_bot::nixsearch;
use mallard_bot::sessions::{new_store, spawn_reaper};
use mallard_bot::stickerpack::StickerPack;
use mallard_bot::Mallard;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let _ = dotenvy::dotenv();
    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("info")).init();

    let token = env::var("TG_API_KEY").map_err(|_| anyhow::anyhow!("TG_API_KEY not set"))?;
    let admin_id: Option<UserId> = env::var("TG_ADMIN_ID")
        .ok()
        .and_then(|s| s.parse::<u64>().ok())
        .map(UserId);

    mallard_bot::triggers::init();
    let mallard = Arc::new(Mutex::new(Mallard::new()));
    let mut bot = Bot::new(token);
    // Optional: point at a self-hosted Bot API server (raises the 20 MB
    // getFile cap to 2 GB). Set TG_API_URL=http://host:8081 in env.
    if let Ok(api_url) = env::var("TG_API_URL") {
        match reqwest::Url::parse(api_url.trim_end_matches('/')) {
            Ok(url) => {
                log::info!("using custom Bot API at {url}");
                bot = bot.set_api_url(url);
            }
            Err(e) => log::warn!("TG_API_URL set but unparseable ({e}); using default api.telegram.org"),
        }
    }

    let me = bot.get_me().await?;
    let bot_username = me
        .username
        .clone()
        .ok_or_else(|| anyhow::anyhow!("bot has no username"))?;
    log::info!("STARTED as @{bot_username}");

    // Public command list shown in the / menu (hidden variants are skipped).
    let public_cmds = Command::bot_commands();
    if let Err(e) = bot.set_my_commands(public_cmds.clone()).await {
        log::warn!("set_my_commands (default scope) failed: {e}");
    }

    // Admin sees the full list (including /voice) in their private chat.
    if let Some(admin) = admin_id {
        let mut admin_cmds = public_cmds;
        admin_cmds.push(teloxide::types::BotCommand::new(
            "voice",
            "сохранить голосовое в voices/<name>.ogg (только для админа)",
        ));
        admin_cmds.push(teloxide::types::BotCommand::new(
            "import",
            "перетащить чужой стикер-пак к себе (только для админа)",
        ));
        if let Err(e) = bot
            .set_my_commands(admin_cmds)
            .scope(BotCommandScope::Chat {
                chat_id: Recipient::Id(ChatId(admin.0 as i64)),
            })
            .await
        {
            log::warn!("set_my_commands (admin scope) failed: {e}");
        }
    }

    let pack = admin_id.map(|admin| StickerPack {
        admin_user_id: admin,
        bot_username: bot_username.clone(),
    });

    if let Some(admin) = admin_id {
        let _ = bot
            .send_message(ChatId(admin.0 as i64), "Я снова здесь!")
            .await;
    }

    // SQLite lives on its own PVC, separate from the voices/ audio bucket.
    let db_path =
        env::var("MALLARD_DB_PATH").unwrap_or_else(|_| "/app/data/mallard.db".to_string());
    let db = Db::open(std::path::Path::new(&db_path))
        .map_err(|e| anyhow::anyhow!("open db {db_path}: {e}"))?;
    log::info!("db open at {db_path}");

    let chabani = new_store();
    spawn_reaper(chabani.clone(), db.clone());
    nixsearch::spawn_refresher(db.clone());
    mallard_bot::fx::spawn_refresher();

    // Hosted Bot API caps getFile at 20 MB; self-hosted goes up to 2 GB.
    // Configurable via env so the manifest can opt into the higher limit.
    let download_max_bytes: u32 = env::var("TG_DOWNLOAD_MAX_BYTES")
        .ok()
        .and_then(|s| s.parse::<u32>().ok())
        .unwrap_or(20 * 1024 * 1024);

    let config = BotConfig {
        admin_id,
        pack,
        db,
        chabani,
        bot_username: bot_username.clone(),
        download_max_bytes,
    };

    build_dispatcher(bot, mallard, config).dispatch().await;
    Ok(())
}
