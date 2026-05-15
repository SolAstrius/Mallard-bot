//! Chabani tracking.
//!
//! A *chabani* is a single teapot shared by one or more participants. Each
//! user is in at most one chabani at a time. The chabani — not the user —
//! is the unit of state: the steep counter, label, and notes belong to the
//! teapot, and mutating commands (`/sip`, `/cha note`) advance the shared
//! state, not the caller's personal counter.
//!
//! Live chabani live in memory. Closed chabani get persisted to SQLite
//! (`cha_chabani` table) so `/cha last` survives pod restarts. Idle ones
//! are auto-closed by a background reaper after `IDLE_TIMEOUT`.
//!
//! See [`crate::db`] for the on-disk schema; this module owns the in-memory
//! shape, the random-id minter, and the reaper.

use std::collections::{HashMap, HashSet};
use std::sync::Arc;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use rand::Rng;
use teloxide::types::{ChatId, UserId};
use tokio::sync::Mutex;

use crate::db::Db;

const IDLE_TIMEOUT: Duration = Duration::from_secs(90 * 60);
const REAPER_INTERVAL: Duration = Duration::from_secs(5 * 60);

/// Short opaque id used to share a chabani out-of-band (`/cha join <id>`).
/// 7 base62 characters → ~57 bits of entropy; collisions on the live set
/// are checked at mint time so we never hand out a duplicate.
#[derive(Debug, Clone, Hash, PartialEq, Eq)]
pub struct SessionId(pub String);

impl SessionId {
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl std::fmt::Display for SessionId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

#[derive(Debug, Clone)]
pub struct Participant {
    pub user_name: String,
    /// Chat the user joined the chabani from. Persisted at close time so
    /// `/cha last` can show "joined from where" if we ever want it.
    pub origin_chat: ChatId,
    pub joined_at: Instant,
}

#[derive(Debug, Clone)]
pub struct Chabani {
    pub id: SessionId,
    pub label: String,
    /// Chat where this chabani was first opened. Used to scope `/cha last`
    /// and as the default destination for sip echoes.
    pub origin_chat: ChatId,
    pub started_at: Instant,
    pub started_unix: i64,
    pub last_activity: Instant,
    pub sips: u32,
    /// `(author, text)` — each note keeps its writer so `/cha last` can
    /// attribute them.
    pub notes: Vec<(UserId, String)>,
    pub participants: HashMap<UserId, Participant>,
}

impl Chabani {
    fn new(
        id: SessionId,
        label: String,
        opener: UserId,
        opener_name: String,
        origin_chat: ChatId,
    ) -> Self {
        let now = Instant::now();
        let started_unix = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_secs() as i64)
            .unwrap_or(0);
        let mut participants = HashMap::new();
        participants.insert(
            opener,
            Participant {
                user_name: opener_name,
                origin_chat,
                joined_at: now,
            },
        );
        Self {
            id,
            label,
            origin_chat,
            started_at: now,
            started_unix,
            last_activity: now,
            sips: 0,
            notes: Vec::new(),
            participants,
        }
    }

    pub fn sip(&mut self) {
        self.sips += 1;
        self.last_activity = Instant::now();
    }

    pub fn add_note(&mut self, author: UserId, text: String) {
        self.notes.push((author, text));
        self.last_activity = Instant::now();
    }

    pub fn join(&mut self, user: UserId, user_name: String, from_chat: ChatId) {
        self.participants.insert(
            user,
            Participant {
                user_name,
                origin_chat: from_chat,
                joined_at: Instant::now(),
            },
        );
        self.last_activity = Instant::now();
    }

    /// Remove a participant. Returns `true` if the chabani is now empty and
    /// should be closed by the caller.
    pub fn leave(&mut self, user: UserId) -> bool {
        self.participants.remove(&user);
        self.last_activity = Instant::now();
        self.participants.is_empty()
    }

    pub fn elapsed(&self) -> Duration {
        Instant::now().saturating_duration_since(self.started_at)
    }

    pub fn idle_for(&self) -> Duration {
        Instant::now().saturating_duration_since(self.last_activity)
    }
}

/// Two indexes over the live chabani set:
/// * `by_id` — primary, what `/cha join <id>` and the reaper iterate.
/// * `by_user` — reverse lookup for "what chabani is this user in?",
///   needed by `/sip`, `/cha note`, `/cha leave`, and `/cha join @user`.
///
/// The two must stay consistent — see [`ChabaniState::open`],
/// [`ChabaniState::add_participant`], [`ChabaniState::remove_participant`].
#[derive(Debug, Default)]
pub struct ChabaniState {
    pub by_id: HashMap<SessionId, Chabani>,
    pub by_user: HashMap<UserId, SessionId>,
}

impl ChabaniState {
    /// Open a new chabani. Mints a fresh id, registers the opener as the
    /// sole participant, and indexes both ways. Returns the new chabani's id.
    pub fn open(
        &mut self,
        label: String,
        opener: UserId,
        opener_name: String,
        origin_chat: ChatId,
    ) -> SessionId {
        let id = self.mint_id();
        let chabani = Chabani::new(id.clone(), label, opener, opener_name, origin_chat);
        self.by_user.insert(opener, id.clone());
        self.by_id.insert(id.clone(), chabani);
        id
    }

    /// Add `user` to the chabani keyed by `id`. The caller must have
    /// already removed them from any prior chabani via
    /// [`Self::remove_participant`]. No-op if `id` doesn't exist.
    pub fn add_participant(
        &mut self,
        id: &SessionId,
        user: UserId,
        user_name: String,
        from_chat: ChatId,
    ) {
        if let Some(c) = self.by_id.get_mut(id) {
            c.join(user, user_name, from_chat);
            self.by_user.insert(user, id.clone());
        }
    }

    /// Remove `user` from whatever chabani they're in. Returns the closed
    /// chabani when this leave emptied the teapot — the caller should
    /// persist it. Returns `None` if the user wasn't seated or the chabani
    /// still has other participants.
    pub fn remove_participant(&mut self, user: UserId) -> Option<Chabani> {
        let id = self.by_user.remove(&user)?;
        let now_empty = self.by_id.get_mut(&id).map(|c| c.leave(user)).unwrap_or(true);
        if now_empty {
            self.by_id.remove(&id)
        } else {
            None
        }
    }

    pub fn chabani_of(&self, user: UserId) -> Option<&Chabani> {
        self.by_user.get(&user).and_then(|id| self.by_id.get(id))
    }

    pub fn chabani_of_mut(&mut self, user: UserId) -> Option<&mut Chabani> {
        let id = self.by_user.get(&user)?.clone();
        self.by_id.get_mut(&id)
    }

    fn mint_id(&self) -> SessionId {
        // Collision-resistant enough at 7 chars; retry on the rare hit.
        loop {
            let id = SessionId(random_session_id());
            if !self.by_id.contains_key(&id) {
                return id;
            }
        }
    }
}

pub type ChabaniStore = Arc<Mutex<ChabaniState>>;

pub fn new_store() -> ChabaniStore {
    Arc::new(Mutex::new(ChabaniState::default()))
}

/// Chabani visible "from" `chat` — those with at least one participant who's
/// been seen in `chat` (per `user_membership`). `members_of_chat` is the
/// set of user IDs known to belong to `chat`; pass it in so the caller can
/// reuse it for filtering display participants.
pub fn chabani_visible_in<'a>(
    state: &'a ChabaniState,
    members_of_chat: &HashSet<UserId>,
) -> Vec<&'a Chabani> {
    state
        .by_id
        .values()
        .filter(|c| c.participants.keys().any(|u| members_of_chat.contains(u)))
        .collect()
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

/// Persist a closed chabani to SQLite. Best-effort: errors are logged.
pub async fn persist_closed(db: &Db, chabani: &Chabani, auto_closed: bool) {
    let ended_unix = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0);
    let duration_s = chabani.elapsed().as_secs() as i64;
    let notes_json = serialize_notes(&chabani.notes);
    let participants_json = serialize_participants(&chabani.participants);
    let result = db
        .insert_chabani(
            chabani.id.0.clone(),
            chabani.label.clone(),
            chabani.origin_chat.0,
            chabani.started_unix,
            ended_unix,
            duration_s,
            chabani.sips as i64,
            auto_closed,
            notes_json,
            participants_json,
        )
        .await;
    if let Err(e) = result {
        log::warn!("chabani persist failed: {e}");
    }
}

/// Spawn the background reaper. Every `REAPER_INTERVAL` it walks the store,
/// auto-closes chabani idle past `IDLE_TIMEOUT`, and persists them.
pub fn spawn_reaper(store: ChabaniStore, db: Db) {
    tokio::spawn(async move {
        let mut tick = tokio::time::interval(REAPER_INTERVAL);
        tick.tick().await; // skip immediate fire
        loop {
            tick.tick().await;
            let expired: Vec<SessionId> = {
                let state = store.lock().await;
                state
                    .by_id
                    .iter()
                    .filter(|(_, c)| c.idle_for() >= IDLE_TIMEOUT)
                    .map(|(id, _)| id.clone())
                    .collect()
            };
            for id in expired {
                let chabani = {
                    let mut state = store.lock().await;
                    let c = state.by_id.remove(&id);
                    if let Some(ref c) = c {
                        for u in c.participants.keys() {
                            state.by_user.remove(u);
                        }
                    }
                    c
                };
                if let Some(c) = chabani {
                    log::info!(
                        "auto-closing idle chabani {}: {} · {} sips · {} participants",
                        c.id,
                        c.label,
                        c.sips,
                        c.participants.len()
                    );
                    persist_closed(&db, &c, true).await;
                }
            }
        }
    });
}

fn random_session_id() -> String {
    const ALPHABET: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789";
    let mut rng = rand::thread_rng();
    (0..7)
        .map(|_| ALPHABET[rng.gen_range(0..ALPHABET.len())] as char)
        .collect()
}

fn serialize_notes(notes: &[(UserId, String)]) -> String {
    let parts: Vec<String> = notes
        .iter()
        .map(|(u, t)| {
            format!(
                "{{\"u\":{},\"t\":\"{}\"}}",
                u.0,
                t.replace('\\', "\\\\").replace('"', "\\\"")
            )
        })
        .collect();
    format!("[{}]", parts.join(","))
}

fn serialize_participants(participants: &HashMap<UserId, Participant>) -> String {
    let parts: Vec<String> = participants
        .iter()
        .map(|(u, p)| {
            format!(
                "{{\"u\":{},\"n\":\"{}\",\"c\":{}}}",
                u.0,
                p.user_name.replace('\\', "\\\\").replace('"', "\\\""),
                p.origin_chat.0
            )
        })
        .collect();
    format!("[{}]", parts.join(","))
}

/// Parse the JSON written by `serialize_notes`. Best-effort: returns empty
/// on parse trouble. Only used for `/cha last` rendering.
pub fn parse_notes(json: &str) -> Vec<(i64, String)> {
    parse_object_array(json, |obj| {
        let u = obj.get("u").and_then(|v| v.parse::<i64>().ok())?;
        let t = obj.get("t").cloned()?;
        Some((u, t))
    })
}

/// Parse the JSON written by `serialize_participants`. Returns
/// `(user_id, name, origin_chat_id)` tuples. Best-effort.
pub fn parse_participants(json: &str) -> Vec<(i64, String, i64)> {
    parse_object_array(json, |obj| {
        let u = obj.get("u").and_then(|v| v.parse::<i64>().ok())?;
        let n = obj.get("n").cloned()?;
        let c = obj.get("c").and_then(|v| v.parse::<i64>().ok())?;
        Some((u, n, c))
    })
}

/// Tiny JSON-array-of-flat-objects parser. We control both ends, so this
/// only handles the exact shape `serialize_*` emits: `[{"k":v,"k":"s",...}]`
/// with numeric or double-quoted string values. Drops anything weirder.
fn parse_object_array<T, F>(json: &str, mut f: F) -> Vec<T>
where
    F: FnMut(&HashMap<String, String>) -> Option<T>,
{
    let s = json.trim();
    if !s.starts_with('[') || !s.ends_with(']') || s.len() < 2 {
        return Vec::new();
    }
    let inner = &s[1..s.len() - 1];
    let mut out = Vec::new();
    let mut depth = 0usize;
    let mut buf = String::new();
    let mut in_str = false;
    let mut escape = false;
    for ch in inner.chars() {
        if escape {
            buf.push(ch);
            escape = false;
            continue;
        }
        if ch == '\\' && in_str {
            escape = true;
            buf.push(ch);
            continue;
        }
        if ch == '"' {
            in_str = !in_str;
            buf.push(ch);
            continue;
        }
        if !in_str {
            if ch == '{' {
                depth += 1;
            }
            if ch == '}' {
                depth = depth.saturating_sub(1);
                buf.push(ch);
                if depth == 0 {
                    if let Some(obj) = parse_flat_object(&buf) {
                        if let Some(v) = f(&obj) {
                            out.push(v);
                        }
                    }
                    buf.clear();
                }
                continue;
            }
            if depth == 0 && (ch == ',' || ch.is_whitespace()) {
                continue;
            }
        }
        buf.push(ch);
    }
    out
}

fn parse_flat_object(src: &str) -> Option<HashMap<String, String>> {
    let s = src.trim();
    let inner = s.strip_prefix('{')?.strip_suffix('}')?;
    let mut out = HashMap::new();
    let mut i = 0;
    let bytes = inner.as_bytes();
    while i < bytes.len() {
        while i < bytes.len() && (bytes[i] == b',' || bytes[i].is_ascii_whitespace()) {
            i += 1;
        }
        if i >= bytes.len() {
            break;
        }
        // Key — always quoted.
        if bytes[i] != b'"' {
            return None;
        }
        i += 1;
        let key_start = i;
        while i < bytes.len() && bytes[i] != b'"' {
            i += 1;
        }
        let key = std::str::from_utf8(&bytes[key_start..i]).ok()?.to_string();
        i += 1; // closing quote
        while i < bytes.len() && (bytes[i] == b':' || bytes[i].is_ascii_whitespace()) {
            i += 1;
        }
        // Value — quoted string or bare number.
        let value = if i < bytes.len() && bytes[i] == b'"' {
            i += 1;
            let mut v = String::new();
            while i < bytes.len() && bytes[i] != b'"' {
                if bytes[i] == b'\\' && i + 1 < bytes.len() {
                    v.push(bytes[i + 1] as char);
                    i += 2;
                } else {
                    v.push(bytes[i] as char);
                    i += 1;
                }
            }
            i += 1; // closing quote
            v
        } else {
            let start = i;
            while i < bytes.len() && bytes[i] != b',' && !bytes[i].is_ascii_whitespace() {
                i += 1;
            }
            std::str::from_utf8(&bytes[start..i]).ok()?.to_string()
        };
        out.insert(key, value);
    }
    Some(out)
}
