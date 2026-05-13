use std::env;
use std::sync::Arc;

use teloxide::prelude::*;
use tokio::sync::Mutex;

use mallard_bot::bot::build_dispatcher;
use mallard_bot::Mallard;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("info")).init();

    let token = env::var("TG_API_KEY").map_err(|_| anyhow::anyhow!("TG_API_KEY not set"))?;
    let admin = env::var("TG_ADMIN_ID").ok();

    let mallard = Arc::new(Mutex::new(Mallard::new(150)));
    let bot = Bot::new(token);

    log::info!("STARTED");
    if let Some(admin) = admin {
        if let Ok(chat_id) = admin.parse::<i64>() {
            let _ = bot.send_message(ChatId(chat_id), "Я снова здесь!").await;
        }
    }

    build_dispatcher(bot, mallard).await.dispatch().await;
    Ok(())
}
