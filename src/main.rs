use std::env;
use std::sync::Arc;

use teloxide::prelude::*;
use teloxide::types::UserId;
use tokio::sync::Mutex;

use teloxide::types::{BotCommandScope, Recipient};
use teloxide::utils::command::BotCommands as _;

use mallard_bot::bot::{build_dispatcher, BotConfig, Command};
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

    let mallard = Arc::new(Mutex::new(Mallard::new(150)));
    let bot = Bot::new(token);

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

    let config = BotConfig { admin_id, pack };

    build_dispatcher(bot, mallard, config).dispatch().await;
    Ok(())
}
