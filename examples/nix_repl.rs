//! Stdio REPL for the local nix catalog. Opens the DB at $MALLARD_DB_PATH
//! (default: ./nix-cache.sqlite), runs the refresher once if needed, then
//! reads /npkg/nopt/nixwhere queries from stdin and prints results.
//!
//! Usage:
//!     cargo run --example nix_repl
//!     /npkg ripgrep
//!     /nopt services.tailscale.enable
//!     /nixwhere mtr
//!     :q

use std::io::{self, BufRead, Write};
use std::path::PathBuf;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use mallard_bot::db::Db;

#[tokio::main(flavor = "multi_thread", worker_threads = 4)]
async fn main() -> anyhow::Result<()> {
    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("info")).init();

    let db_path = std::env::var("MALLARD_DB_PATH")
        .map(PathBuf::from)
        .unwrap_or_else(|_| PathBuf::from("./nix-cache.sqlite"));
    eprintln!("db: {}", db_path.display());
    let db = Db::open_sqlite(&db_path)?;

    // One-shot refresh if catalog is empty or stale. We can't reuse the
    // private refresh fn directly, so trigger the public spawner and wait.
    let need = db
        .nix_meta_get("nix_last_refresh_unix")
        .await?
        .and_then(|v| v.parse::<i64>().ok())
        .map(|ts| {
            let now = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .map(|d| d.as_secs() as i64)
                .unwrap_or(0);
            (now - ts).max(0) >= (7 * 24 * 3600)
        })
        .unwrap_or(true);

    if need {
        eprintln!("catalog stale/empty — refreshing (10-30s)...");
        mallard_bot::nixsearch::spawn_refresher(db.clone());
        // Poll until populated.
        let deadline = std::time::Instant::now() + Duration::from_secs(180);
        loop {
            if db.nix_meta_get("nix_last_refresh_unix").await?.is_some() {
                break;
            }
            if std::time::Instant::now() > deadline {
                anyhow::bail!("refresh timed out");
            }
            tokio::time::sleep(Duration::from_secs(2)).await;
        }
        eprintln!("catalog ready");
    } else {
        eprintln!("catalog fresh");
    }

    eprintln!("commands: /npkg <q>, /nopt <q>, /nixwhere <bin>, :q");
    let stdin = io::stdin();
    let mut stdout = io::stdout();
    let mut line = String::new();
    loop {
        print!("> ");
        stdout.flush().ok();
        line.clear();
        if stdin.lock().read_line(&mut line)? == 0 {
            break;
        }
        let trimmed = line.trim();
        if trimmed == ":q" || trimmed == "exit" {
            break;
        }
        if trimmed.is_empty() {
            continue;
        }
        let (cmd, rest) = match trimmed.split_once(char::is_whitespace) {
            Some((c, r)) => (c, r.trim()),
            None => (trimmed, ""),
        };
        let result = match cmd {
            "/npkg" => npkg(&db, rest).await,
            "/nopt" => nopt(&db, rest).await,
            "/nixwhere" => nixwhere(&db, rest).await,
            other => {
                println!("unknown command: {other}");
                continue;
            }
        };
        if let Err(e) = result {
            println!("error: {e:#}");
        }
    }
    Ok(())
}

async fn npkg(db: &Db, q: &str) -> anyhow::Result<()> {
    if q.is_empty() {
        println!("usage: /npkg <query>");
        return Ok(());
    }
    let hits = db.search_nix_packages(q, 5).await?;
    if hits.is_empty() {
        println!("no hits for {q:?}");
        return Ok(());
    }
    for (i, h) in hits.iter().enumerate() {
        println!(
            "[{}] {} · {} — {}",
            i + 1,
            h.attr_name,
            h.version,
            h.description
        );
        let mut flags = Vec::new();
        if h.broken {
            flags.push("broken");
        }
        if h.insecure {
            flags.push("insecure");
        }
        if h.unfree {
            flags.push("unfree");
        }
        if !flags.is_empty() {
            println!("    flags: {}", flags.join(", "));
        }
        if !h.homepage.is_empty() {
            println!("    homepage: {}", h.homepage);
        }
        if !h.license.is_empty() {
            println!("    license: {}", h.license);
        }
        if !h.platforms.is_empty() {
            let count = h.platforms.split(',').count();
            let preview = h
                .platforms
                .split(',')
                .take(6)
                .collect::<Vec<_>>()
                .join(", ");
            println!(
                "    platforms ({count}): {preview}{}",
                if count > 6 { " ..." } else { "" }
            );
        }
        if !h.maintainers.is_empty() {
            println!("    maintainers: {}", h.maintainers);
        }
        if !h.position.is_empty() {
            println!("    position: {}", h.position);
        }
        if !h.main_program.is_empty() && h.main_program != h.attr_name {
            println!("    main: {}", h.main_program);
        }
    }
    Ok(())
}

async fn nopt(db: &Db, q: &str) -> anyhow::Result<()> {
    if q.is_empty() {
        println!("usage: /nopt <query>");
        return Ok(());
    }
    let hits = db.search_nix_options(q, 3).await?;
    if hits.is_empty() {
        println!("no hits for {q:?}");
        return Ok(());
    }
    for (i, h) in hits.iter().enumerate() {
        println!("[{}] {}", i + 1, h.name);
        if !h.type_.is_empty() {
            println!("    type: {}", h.type_);
        }
        if !h.default_.is_empty() {
            println!("    default: {}", h.default_);
        }
        if !h.description.is_empty() {
            let d = h.description.trim();
            let shown = if d.len() > 300 {
                format!("{}…", &d[..300])
            } else {
                d.to_string()
            };
            println!("    {}", shown);
        }
    }
    Ok(())
}

async fn nixwhere(db: &Db, q: &str) -> anyhow::Result<()> {
    if q.is_empty() {
        println!("usage: /nixwhere <binary>");
        return Ok(());
    }
    let hits = db.search_nix_programs(q, 5).await?;
    if hits.is_empty() {
        println!("no hits for {q:?}");
        return Ok(());
    }
    for h in &hits {
        let suffix = if !h.main_program.is_empty() && h.main_program != q {
            format!(" (main: {})", h.main_program)
        } else {
            String::new()
        };
        println!("- {}{}", h.attr_name, suffix);
    }
    Ok(())
}
