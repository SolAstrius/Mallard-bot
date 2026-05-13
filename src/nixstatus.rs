//! Live Nix-ecosystem lookups: channel revision and flake registry.
//!
//! These hit `channels.nixos.org` per call (no local cache) — they're tiny
//! responses and the data is "always-current" by nature.

use std::time::Duration;

use serde::Deserialize;

const DEFAULT_CHANNEL: &str = "nixos-unstable";
const FLAKE_REGISTRY_URL: &str = "https://channels.nixos.org/flake-registry.json";

#[derive(Debug, Clone)]
pub struct ChannelInfo {
    pub channel: String,
    pub revision: String,
    /// `nixos-26.05pre995699.da5ad661ba4e` — extracted from the redirect path
    /// on `packages.json.br`. Tells you the channel's snapshot label.
    pub version_label: Option<String>,
}

pub async fn fetch_channel(channel: Option<&str>) -> anyhow::Result<ChannelInfo> {
    let ch = channel.unwrap_or(DEFAULT_CHANNEL);

    let follow = reqwest::Client::builder()
        .timeout(Duration::from_secs(15))
        .redirect(reqwest::redirect::Policy::limited(5))
        .build()?;
    let rev_url = format!("https://channels.nixos.org/{ch}/git-revision");
    let revision = follow
        .get(&rev_url)
        .send()
        .await?
        .error_for_status()?
        .text()
        .await?
        .trim()
        .to_string();

    // Read the Location header on a HEAD request to extract the snapshot path.
    let no_redir = reqwest::Client::builder()
        .timeout(Duration::from_secs(15))
        .redirect(reqwest::redirect::Policy::none())
        .build()?;
    let probe_url = format!("https://channels.nixos.org/{ch}/packages.json.br");
    let version_label = no_redir
        .head(&probe_url)
        .send()
        .await
        .ok()
        .and_then(|r| r.headers().get("location").cloned())
        .and_then(|v| v.to_str().ok().map(String::from))
        .and_then(|loc| {
            // .../nixos/unstable/<label>/packages.json.br → second-to-last segment.
            loc.trim_end_matches('/')
                .rsplit('/')
                .nth(1)
                .map(String::from)
        });

    Ok(ChannelInfo {
        channel: ch.to_string(),
        revision,
        version_label,
    })
}

#[derive(Debug, Clone, Deserialize)]
pub struct FlakeRegistry {
    pub flakes: Vec<FlakeEntry>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct FlakeEntry {
    pub from: FlakeRef,
    pub to: FlakeRef,
}

#[derive(Debug, Clone, Deserialize)]
pub struct FlakeRef {
    #[serde(rename = "type")]
    pub kind: String,
    #[serde(default)]
    pub id: Option<String>,
    #[serde(default)]
    pub owner: Option<String>,
    #[serde(default)]
    pub repo: Option<String>,
    #[serde(default)]
    pub url: Option<String>,
    #[serde(rename = "ref", default)]
    pub git_ref: Option<String>,
    #[serde(default)]
    pub dir: Option<String>,
}

pub async fn fetch_flake_registry() -> anyhow::Result<FlakeRegistry> {
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(15))
        .build()?;
    let body = client
        .get(FLAKE_REGISTRY_URL)
        .send()
        .await?
        .error_for_status()?
        .text()
        .await?;
    Ok(serde_json::from_str(&body)?)
}

/// Case-insensitive lookup by `from.id`.
pub fn lookup_flake<'a>(registry: &'a FlakeRegistry, query: &str) -> Option<&'a FlakeEntry> {
    let q = query.trim().to_lowercase();
    if q.is_empty() {
        return None;
    }
    registry.flakes.iter().find(|e| {
        e.from
            .id
            .as_ref()
            .map(|id| id.to_lowercase() == q)
            .unwrap_or(false)
    })
}

/// Render a flake `to` reference as a human-friendly URL.
pub fn to_url(r: &FlakeRef) -> String {
    match r.kind.as_str() {
        "github" => {
            let owner = r.owner.as_deref().unwrap_or("?");
            let repo = r.repo.as_deref().unwrap_or("?");
            let mut s = format!("github:{owner}/{repo}");
            if let Some(g) = &r.git_ref {
                s.push('/');
                s.push_str(g);
            }
            if let Some(d) = &r.dir {
                s.push_str("?dir=");
                s.push_str(d);
            }
            s
        }
        "gitlab" => {
            let owner = r.owner.as_deref().unwrap_or("?");
            let repo = r.repo.as_deref().unwrap_or("?");
            format!("gitlab:{owner}/{repo}")
        }
        "sourcehut" => {
            let owner = r.owner.as_deref().unwrap_or("?");
            let repo = r.repo.as_deref().unwrap_or("?");
            format!("sourcehut:{owner}/{repo}")
        }
        "git" | "tarball" | "file" => r.url.clone().unwrap_or_else(|| "?".to_string()),
        other => format!("({other}) {}", r.url.as_deref().unwrap_or("?")),
    }
}
