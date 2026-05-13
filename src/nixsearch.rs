//! Local nixpkgs + NixOS-options catalog.
//!
//! Downloads `packages.json.br` and `options.json.br` from
//! `channels.nixos.org` (the brotli-compressed snapshots published with every
//! nixos-unstable channel bump), parses them, and bulk-loads two SQLite FTS5
//! tables. Commands then query SQLite — no upstream call per /npkg.
//!
//! Refresh runs in a background task: on boot if the local copy is missing or
//! older than `STALE_THRESHOLD`, then on a 7-day cadence. Failures are logged
//! and don't crash the bot; the previous catalog stays usable.

use std::io::Read;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use serde::Deserialize;

use crate::db::{Db, NixOptRow, NixPkgRow};

const PACKAGES_URL: &str = "https://channels.nixos.org/nixos-unstable/packages.json.br";
const OPTIONS_URL: &str = "https://channels.nixos.org/nixos-unstable/options.json.br";

const REFRESH_INTERVAL: Duration = Duration::from_secs(7 * 24 * 3600);
const STALE_THRESHOLD: Duration = Duration::from_secs(7 * 24 * 3600);
const RETRY_BACKOFF: Duration = Duration::from_secs(30 * 60);

const META_LAST_REFRESH: &str = "nix_last_refresh_unix";

// --- on-disk JSON shapes (only what we keep) ---

#[derive(Deserialize)]
struct PackagesFile {
    packages: std::collections::BTreeMap<String, PackageRecord>,
}

#[derive(Deserialize)]
struct PackageRecord {
    #[serde(default)]
    pname: Option<String>,
    #[serde(default)]
    version: Option<String>,
    #[serde(default)]
    meta: Option<PackageMeta>,
}

#[derive(Deserialize)]
struct PackageMeta {
    #[serde(default)]
    description: Option<String>,
    #[serde(rename = "longDescription", default)]
    long_description: Option<String>,
    #[serde(rename = "mainProgram", default)]
    main_program: Option<String>,
}

#[derive(Deserialize)]
struct OptionRecord {
    #[serde(rename = "type", default)]
    typ: Option<String>,
    #[serde(default)]
    default: Option<serde_json::Value>,
    #[serde(default)]
    description: Option<String>,
}

// --- public API ---

/// Spawn the long-running refresh task. Runs once at startup if needed, then
/// every `REFRESH_INTERVAL`.
pub fn spawn_refresher(db: Db) {
    tokio::spawn(async move {
        loop {
            let staleness = last_refresh_age(&db).await;
            let need = match staleness {
                None => true,
                Some(age) => age >= STALE_THRESHOLD,
            };
            if need {
                log::info!("nix catalog refresh starting (staleness: {:?})", staleness);
                match refresh(&db).await {
                    Ok((pkgs, opts)) => {
                        log::info!("nix catalog refreshed: {pkgs} packages, {opts} options");
                        let now = SystemTime::now()
                            .duration_since(UNIX_EPOCH)
                            .map(|d| d.as_secs() as i64)
                            .unwrap_or(0);
                        if let Err(e) = db.nix_meta_set(META_LAST_REFRESH, &now.to_string()).await {
                            log::warn!("nix_meta_set failed: {e}");
                        }
                        tokio::time::sleep(REFRESH_INTERVAL).await;
                    }
                    Err(e) => {
                        log::warn!("nix catalog refresh failed: {e:#}");
                        tokio::time::sleep(RETRY_BACKOFF).await;
                    }
                }
            } else {
                let due_in = STALE_THRESHOLD.saturating_sub(staleness.unwrap_or(Duration::ZERO));
                log::info!("nix catalog is fresh; next refresh in {due_in:?}");
                tokio::time::sleep(due_in).await;
            }
        }
    });
}

/// Time since the last successful refresh, or `None` if never.
async fn last_refresh_age(db: &Db) -> Option<Duration> {
    let val = db.nix_meta_get(META_LAST_REFRESH).await.ok().flatten()?;
    let ts: i64 = val.parse().ok()?;
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0);
    let delta = (now - ts).max(0) as u64;
    Some(Duration::from_secs(delta))
}

async fn refresh(db: &Db) -> anyhow::Result<(usize, usize)> {
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(120))
        .build()?;

    let pkgs_bytes = fetch(&client, PACKAGES_URL).await?;
    let opts_bytes = fetch(&client, OPTIONS_URL).await?;

    // Parsing + decompression in a blocking thread — the JSON is big and
    // serde_json is synchronous.
    let (pkg_rows, opt_rows) = tokio::task::spawn_blocking(move || -> anyhow::Result<_> {
        let pkg_rows = parse_packages(&pkgs_bytes)?;
        let opt_rows = parse_options(&opts_bytes)?;
        Ok((pkg_rows, opt_rows))
    })
    .await??;

    let pkg_count = db.replace_nix_packages(pkg_rows).await?;
    let opt_count = db.replace_nix_options(opt_rows).await?;

    Ok((pkg_count, opt_count))
}

async fn fetch(client: &reqwest::Client, url: &str) -> anyhow::Result<Vec<u8>> {
    let resp = client.get(url).send().await?.error_for_status()?;
    Ok(resp.bytes().await?.to_vec())
}

fn parse_packages(compressed: &[u8]) -> anyhow::Result<Vec<NixPkgRow>> {
    let mut decoder = brotli::Decompressor::new(compressed, 64 * 1024);
    let mut buf = Vec::with_capacity(compressed.len() * 8);
    decoder.read_to_end(&mut buf)?;
    let file: PackagesFile = serde_json::from_slice(&buf)?;
    let mut rows = Vec::with_capacity(file.packages.len());
    for (attr_name, rec) in file.packages {
        let meta = rec.meta.unwrap_or(PackageMeta {
            description: None,
            long_description: None,
            main_program: None,
        });
        rows.push(NixPkgRow {
            attr_name,
            pname: rec.pname.unwrap_or_default(),
            version: rec.version.unwrap_or_default(),
            description: meta.description.unwrap_or_default(),
            long_description: meta.long_description.unwrap_or_default(),
            main_program: meta.main_program.unwrap_or_default(),
        });
    }
    Ok(rows)
}

fn parse_options(compressed: &[u8]) -> anyhow::Result<Vec<NixOptRow>> {
    let mut decoder = brotli::Decompressor::new(compressed, 64 * 1024);
    let mut buf = Vec::with_capacity(compressed.len() * 8);
    decoder.read_to_end(&mut buf)?;
    // The options file is a flat object: { "<name>": {...}, ... }
    let map: std::collections::BTreeMap<String, OptionRecord> = serde_json::from_slice(&buf)?;
    let mut rows = Vec::with_capacity(map.len());
    for (name, rec) in map {
        let default_str = rec.default.as_ref().map(format_default).unwrap_or_default();
        rows.push(NixOptRow {
            name,
            type_: rec.typ.unwrap_or_default(),
            default_: default_str,
            description: rec.description.unwrap_or_default(),
        });
    }
    Ok(rows)
}

/// Render a JSON `default` field in a way that's useful in chat.
/// - Strings → as-is.
/// - Bools/numbers/null → their literal form.
/// - Objects/arrays → compact JSON, truncated.
fn format_default(v: &serde_json::Value) -> String {
    match v {
        serde_json::Value::Null => "null".to_string(),
        serde_json::Value::Bool(b) => b.to_string(),
        serde_json::Value::Number(n) => n.to_string(),
        serde_json::Value::String(s) => s.clone(),
        _ => {
            let s = v.to_string();
            if s.len() > 120 {
                format!("{}…", &s[..120])
            } else {
                s
            }
        }
    }
}
