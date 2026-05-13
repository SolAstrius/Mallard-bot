//! Tea session tracking.
//!
//! Per-(chat, user) live session that remembers what's being brewed and on
//! which steep. Designed for *presence + counter*, not timing.
//!
//! Live sessions live in memory. Closed sessions get written to SQLite
//! (`cha_sessions` table) so `/cha log` survives pod restarts. Idle sessions
//! are auto-closed by a background reaper after `IDLE_TIMEOUT`.

use std::collections::HashMap;
use std::sync::Arc;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use teloxide::types::{ChatId, UserId};
use tokio::sync::Mutex;

use crate::db::Db;

const IDLE_TIMEOUT: Duration = Duration::from_secs(90 * 60);
const REAPER_INTERVAL: Duration = Duration::from_secs(5 * 60);

#[derive(Debug, Clone)]
pub struct TeaSession {
    pub chat: ChatId,
    pub user: UserId,
    pub user_name: String,
    /// Free-text label (e.g. "sheng pu-erh 2017 menghai").
    pub tea: String,
    pub started_at: Instant,
    /// Wall-clock start, captured at session creation so we can persist a
    /// real unix timestamp when the session closes.
    pub started_unix: i64,
    pub last_activity: Instant,
    pub steeps: u32,
    pub notes: Vec<String>,
}

impl TeaSession {
    pub fn new(chat: ChatId, user: UserId, user_name: String, tea: String) -> Self {
        let now = Instant::now();
        let started_unix = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_secs() as i64)
            .unwrap_or(0);
        Self {
            chat,
            user,
            user_name,
            tea,
            started_at: now,
            started_unix,
            last_activity: now,
            steeps: 0,
            notes: Vec::new(),
        }
    }

    pub fn sip(&mut self) {
        self.steeps += 1;
        self.last_activity = Instant::now();
    }

    pub fn add_note(&mut self, text: String) {
        self.notes.push(text);
        self.last_activity = Instant::now();
    }

    pub fn elapsed(&self) -> Duration {
        Instant::now().saturating_duration_since(self.started_at)
    }

    pub fn idle_for(&self) -> Duration {
        Instant::now().saturating_duration_since(self.last_activity)
    }
}

pub type SessionStore = Arc<Mutex<HashMap<(ChatId, UserId), TeaSession>>>;

pub fn new_store() -> SessionStore {
    Arc::new(Mutex::new(HashMap::new()))
}

/// Format a duration as a cozy "1ч 12м" / "12 мин".
pub fn fmt_dur(d: Duration) -> String {
    let total = d.as_secs();
    let h = total / 3600;
    let m = (total % 3600) / 60;
    if h > 0 {
        format!("{h}ч {m}м")
    } else if m > 0 {
        format!("{m} мин")
    } else {
        format!("{}с", total)
    }
}

/// Persist a closed session to SQLite. Best-effort: errors are logged.
pub async fn persist(db: &Db, session: &TeaSession, auto_closed: bool) {
    let ended_unix = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0);
    let duration_s = session.elapsed().as_secs() as i64;
    let notes_json = serialize_notes(&session.notes);
    let result = db
        .insert_cha_session(
            session.chat.0,
            session.user.0 as i64,
            session.user_name.clone(),
            session.tea.clone(),
            session.started_unix,
            ended_unix,
            duration_s,
            session.steeps as i64,
            auto_closed,
            notes_json,
        )
        .await;
    if let Err(e) = result {
        log::warn!("cha session persist failed: {e}");
    }
}

/// Spawn the background reaper. Closes sessions idle past `IDLE_TIMEOUT` and
/// writes them to the DB.
pub fn spawn_reaper(store: SessionStore, db: Db) {
    tokio::spawn(async move {
        let mut tick = tokio::time::interval(REAPER_INTERVAL);
        tick.tick().await; // skip immediate fire
        loop {
            tick.tick().await;
            let expired = {
                let store = store.lock().await;
                store
                    .iter()
                    .filter(|(_, s)| s.idle_for() >= IDLE_TIMEOUT)
                    .map(|(k, _)| *k)
                    .collect::<Vec<_>>()
            };
            for key in expired {
                let session = { store.lock().await.remove(&key) };
                if let Some(s) = session {
                    log::info!(
                        "auto-closing idle tea session: {} · {} · {} steeps",
                        s.user_name,
                        s.tea,
                        s.steeps
                    );
                    persist(&db, &s, true).await;
                }
            }
        }
    });
}

fn serialize_notes(notes: &[String]) -> String {
    // Tiny hand-rolled JSON — keeps the dep surface small.
    let parts: Vec<String> = notes
        .iter()
        .map(|n| format!("\"{}\"", n.replace('\\', "\\\\").replace('"', "\\\"")))
        .collect();
    format!("[{}]", parts.join(","))
}

/// Parse the JSON array of strings written by `serialize_notes`. Best-effort:
/// returns empty on parse trouble. Only used for `/cha log` rendering, so
/// a missed escape just means a missed note.
pub fn parse_notes(json: &str) -> Vec<String> {
    let s = json.trim();
    if !s.starts_with('[') || !s.ends_with(']') || s.len() < 2 {
        return Vec::new();
    }
    let inner = &s[1..s.len() - 1];
    if inner.trim().is_empty() {
        return Vec::new();
    }
    let mut out = Vec::new();
    let mut buf = String::new();
    let mut in_str = false;
    let mut escape_next = false;
    for ch in inner.chars() {
        if escape_next {
            buf.push(ch);
            escape_next = false;
            continue;
        }
        if ch == '\\' {
            escape_next = true;
            continue;
        }
        if ch == '"' {
            if in_str {
                out.push(std::mem::take(&mut buf));
            }
            in_str = !in_str;
            continue;
        }
        if in_str {
            buf.push(ch);
        }
    }
    out
}
