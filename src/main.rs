use std::env;
use std::io::{self, BufRead, Write};

use mallard_bot::Mallard;

fn main() {
    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("info")).init();

    let _token = env::var("TG_API_KEY").ok();
    let _admin_id = env::var("TG_ADMIN_ID").ok();

    let mallard = Mallard::new(150);

    log::info!("STARTED");
    println!("STARTED");

    // The Python implementation polls Telegram. The Rust port deliberately
    // keeps the bot framework wiring out of this rewrite (it would require
    // teloxide, tokio, ffmpeg + opencv bindings — a separate engineering
    // effort). Until that lands, run as a local REPL so the binary is still
    // useful: pipe a line in, get the bot's reply out.
    let stdin = io::stdin();
    let mut stdout = io::stdout().lock();
    for line in stdin.lock().lines() {
        let Ok(line) = line else { break };
        match mallard.process(&line) {
            Some((text, ty)) => {
                let _ = writeln!(stdout, "[{:?}] {}", ty, text);
            }
            None => {
                let _ = writeln!(stdout, "[silent]");
            }
        }
        let _ = stdout.flush();
    }
}
