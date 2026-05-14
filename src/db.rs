//! Thin async wrapper over a single SQLite connection.
//!
//! Schema is created idempotently on open. All access goes through
//! `spawn_blocking` so the Tokio runtime never holds a blocking sqlite call.
//! The connection is wrapped in a `Mutex` — fine for this workload (chat-rate
//! writes, no read fan-out).

use std::path::Path;
use std::sync::Arc;

use rusqlite::Connection;
use tokio::sync::Mutex;

#[derive(Clone)]
pub struct Db {
    conn: Arc<Mutex<Connection>>,
}

/// Bring pre-typed-features databases up to the current `chat_features`
/// shape: ensure a `value TEXT` column exists, backfilled from the legacy
/// `enabled INTEGER` if present. Idempotent — safe to call on every open.
fn migrate_chat_features_to_typed(conn: &Connection) -> rusqlite::Result<()> {
    let mut stmt = conn.prepare("PRAGMA table_info(chat_features)")?;
    let cols: Vec<String> = stmt
        .query_map([], |r| r.get::<_, String>(1))?
        .collect::<Result<_, _>>()?;
    drop(stmt);

    let has_value = cols.iter().any(|c| c == "value");
    let has_enabled = cols.iter().any(|c| c == "enabled");

    if !has_value {
        // Pre-typed schema: add the column, then backfill from `enabled`.
        conn.execute(
            "ALTER TABLE chat_features ADD COLUMN value TEXT NOT NULL DEFAULT 'on'",
            [],
        )?;
        if has_enabled {
            conn.execute(
                "UPDATE chat_features SET value = CASE enabled WHEN 1 THEN 'on' ELSE 'off' END",
                [],
            )?;
        }
    }
    Ok(())
}

const SCHEMA: &str = "
CREATE TABLE IF NOT EXISTS cha_sessions (
    id           INTEGER PRIMARY KEY AUTOINCREMENT,
    chat_id      INTEGER NOT NULL,
    user_id      INTEGER NOT NULL,
    user_name    TEXT    NOT NULL,
    tea          TEXT    NOT NULL,
    started_at   INTEGER NOT NULL,
    ended_at     INTEGER NOT NULL,
    duration_s   INTEGER NOT NULL,
    steeps       INTEGER NOT NULL,
    auto_closed  INTEGER NOT NULL,
    notes        TEXT    NOT NULL
);

CREATE INDEX IF NOT EXISTS idx_cha_chat_ended
    ON cha_sessions(chat_id, ended_at DESC);

-- Nixpkgs / NixOS options local catalog. Populated by a background task
-- that downloads channels.nixos.org snapshots. FTS5 for query-time search.
CREATE VIRTUAL TABLE IF NOT EXISTS nix_pkg USING fts5(
    attr_name, pname, version, description, long_description, main_program,
    tokenize = 'unicode61 remove_diacritics 2'
);

-- Sibling table for metadata we don't search on: homepage, license, source
-- position, platforms, maintainers, health flags. Joined to nix_pkg by
-- attr_name. Refilled in the same transaction as nix_pkg.
CREATE TABLE IF NOT EXISTS nix_pkg_extra (
    attr_name   TEXT PRIMARY KEY,
    homepage    TEXT NOT NULL,
    license     TEXT NOT NULL,
    position    TEXT NOT NULL,
    platforms   TEXT NOT NULL,
    maintainers TEXT NOT NULL,
    broken      INTEGER NOT NULL,
    insecure    INTEGER NOT NULL,
    unfree      INTEGER NOT NULL
);

CREATE VIRTUAL TABLE IF NOT EXISTS nix_opt USING fts5(
    name, type_, default_, description,
    tokenize = 'unicode61 remove_diacritics 2'
);

-- Single-row key/value table for catalog metadata.
CREATE TABLE IF NOT EXISTS nix_meta (
    key   TEXT PRIMARY KEY,
    value TEXT NOT NULL
);

-- Maps render-input hashes to Telegram `file_id`s so we don't re-render
-- and re-upload the same source. Used by inline-mode photo answers, which
-- *only* accept already-uploaded files (no public URL host on our side).
-- `file_ids_json` is a JSON array — multi-page renders cache every page.
CREATE TABLE IF NOT EXISTS render_cache (
    doc_hash      TEXT PRIMARY KEY,
    file_ids_json TEXT NOT NULL,
    created       INTEGER NOT NULL,
    hits          INTEGER NOT NULL DEFAULT 0
);

-- Per-user opt-in for the rich inline-query handler. Missing row → user
-- gets the legacy creature response on inline queries; presence → query
-- string is parsed and dispatched. Per-user (not per-chat) because inline
-- mode follows the user across chats.
CREATE TABLE IF NOT EXISTS inline_opt_in (
    user_id INTEGER PRIMARY KEY,
    since   INTEGER NOT NULL
);

-- Per-chat per-feature toggle. Missing row → use the in-code default for that
-- feature name. `value` is the stored value as a string; the in-code registry
-- (`crate::features::FEATURES`) declares each flag's type (bool / int / enum)
-- and the resolver parses on read. The legacy `enabled INTEGER` column may
-- exist on pre-typed-features databases; `Db::open` runs a migration that
-- backfills `value` from it if needed.
CREATE TABLE IF NOT EXISTS chat_features (
    chat_id INTEGER NOT NULL,
    feature TEXT    NOT NULL,
    value   TEXT    NOT NULL DEFAULT 'on',
    PRIMARY KEY (chat_id, feature)
);

-- (user, chat) presence map populated from any message we see. Powers
-- /cha gossip's 'we share a chat' lookup. `last_seen` lets future code
-- threshold out long-dormant memberships.
CREATE TABLE IF NOT EXISTS user_membership (
    user_id   INTEGER NOT NULL,
    chat_id   INTEGER NOT NULL,
    last_seen INTEGER NOT NULL,
    user_name TEXT    NOT NULL,
    PRIMARY KEY (user_id, chat_id)
);

-- Per-user opt-in for cross-chat tea presence. Mutual: you only see opted-in
-- users; you only appear in others' lists if you're opted in yourself.
CREATE TABLE IF NOT EXISTS cha_gossip_optin (
    user_id INTEGER PRIMARY KEY,
    enabled INTEGER NOT NULL
);
";

#[derive(Debug, Clone)]
pub struct ChaLogRow {
    pub user_name: String,
    pub tea: String,
    pub started_at: i64,
    pub duration_s: i64,
    pub steeps: i64,
    pub auto_closed: bool,
    pub notes_json: String,
}

impl Db {
    pub fn open(path: &Path) -> rusqlite::Result<Self> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).ok();
        }
        let conn = Connection::open(path)?;
        conn.pragma_update(None, "journal_mode", "WAL")?;
        conn.pragma_update(None, "synchronous", "NORMAL")?;
        conn.execute_batch(SCHEMA)?;
        migrate_chat_features_to_typed(&conn)?;
        Ok(Self {
            conn: Arc::new(Mutex::new(conn)),
        })
    }

    /// Insert a closed cha session.
    #[allow(clippy::too_many_arguments)]
    pub async fn insert_cha_session(
        &self,
        chat_id: i64,
        user_id: i64,
        user_name: String,
        tea: String,
        started_at: i64,
        ended_at: i64,
        duration_s: i64,
        steeps: i64,
        auto_closed: bool,
        notes_json: String,
    ) -> rusqlite::Result<()> {
        let conn = self.conn.clone();
        tokio::task::spawn_blocking(move || {
            let conn = conn.blocking_lock();
            conn.execute(
                "INSERT INTO cha_sessions
                 (chat_id, user_id, user_name, tea, started_at, ended_at,
                  duration_s, steeps, auto_closed, notes)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)",
                rusqlite::params![
                    chat_id,
                    user_id,
                    user_name,
                    tea,
                    started_at,
                    ended_at,
                    duration_s,
                    steeps,
                    if auto_closed { 1 } else { 0 },
                    notes_json,
                ],
            )?;
            Ok::<_, rusqlite::Error>(())
        })
        .await
        .map_err(|e| rusqlite::Error::ToSqlConversionFailure(Box::new(e)))?
    }

    /// Fetch the most recent `limit` closed sessions for a chat, newest first.
    pub async fn tail_cha_log(&self, chat_id: i64, limit: i64) -> rusqlite::Result<Vec<ChaLogRow>> {
        let conn = self.conn.clone();
        tokio::task::spawn_blocking(move || {
            let conn = conn.blocking_lock();
            let mut stmt = conn.prepare(
                "SELECT user_name, tea, started_at, duration_s, steeps, auto_closed, notes
                 FROM cha_sessions
                 WHERE chat_id = ?1
                 ORDER BY ended_at DESC
                 LIMIT ?2",
            )?;
            let rows = stmt
                .query_map(rusqlite::params![chat_id, limit], |row| {
                    Ok(ChaLogRow {
                        user_name: row.get(0)?,
                        tea: row.get(1)?,
                        started_at: row.get(2)?,
                        duration_s: row.get(3)?,
                        steeps: row.get(4)?,
                        auto_closed: row.get::<_, i64>(5)? != 0,
                        notes_json: row.get(6)?,
                    })
                })?
                .collect::<Result<Vec<_>, _>>()?;
            Ok::<_, rusqlite::Error>(rows)
        })
        .await
        .map_err(|e| rusqlite::Error::ToSqlConversionFailure(Box::new(e)))?
    }

    // ---------- nix catalog ----------

    pub async fn nix_meta_get(&self, key: &str) -> rusqlite::Result<Option<String>> {
        let conn = self.conn.clone();
        let key = key.to_string();
        tokio::task::spawn_blocking(move || {
            let conn = conn.blocking_lock();
            let mut stmt = conn.prepare("SELECT value FROM nix_meta WHERE key = ?1")?;
            let mut rows = stmt.query(rusqlite::params![key])?;
            if let Some(row) = rows.next()? {
                Ok::<_, rusqlite::Error>(Some(row.get(0)?))
            } else {
                Ok(None)
            }
        })
        .await
        .map_err(|e| rusqlite::Error::ToSqlConversionFailure(Box::new(e)))?
    }

    /// True when `nix_pkg` has data but `nix_pkg_extra` doesn't. Indicates a
    /// schema bump landed and the catalog needs a full refresh to repopulate
    /// the extras side.
    pub async fn nix_extras_missing(&self) -> rusqlite::Result<bool> {
        let conn = self.conn.clone();
        tokio::task::spawn_blocking(move || {
            let conn = conn.blocking_lock();
            let any_pkg: bool =
                conn.query_row("SELECT EXISTS(SELECT 1 FROM nix_pkg)", [], |r| r.get(0))?;
            let any_extra: bool =
                conn.query_row("SELECT EXISTS(SELECT 1 FROM nix_pkg_extra)", [], |r| {
                    r.get(0)
                })?;
            Ok::<_, rusqlite::Error>(any_pkg && !any_extra)
        })
        .await
        .map_err(|e| rusqlite::Error::ToSqlConversionFailure(Box::new(e)))?
    }

    pub async fn nix_meta_set(&self, key: &str, value: &str) -> rusqlite::Result<()> {
        let conn = self.conn.clone();
        let key = key.to_string();
        let value = value.to_string();
        tokio::task::spawn_blocking(move || {
            let conn = conn.blocking_lock();
            conn.execute(
                "INSERT INTO nix_meta(key, value) VALUES(?1, ?2)
                 ON CONFLICT(key) DO UPDATE SET value = excluded.value",
                rusqlite::params![key, value],
            )?;
            Ok::<_, rusqlite::Error>(())
        })
        .await
        .map_err(|e| rusqlite::Error::ToSqlConversionFailure(Box::new(e)))?
    }

    /// Replace the entire nix_pkg table contents in one transaction.
    pub async fn replace_nix_packages(&self, rows: Vec<NixPkgRow>) -> rusqlite::Result<usize> {
        let conn = self.conn.clone();
        tokio::task::spawn_blocking(move || {
            let mut conn = conn.blocking_lock();
            let tx = conn.transaction()?;
            tx.execute("DELETE FROM nix_pkg", [])?;
            tx.execute("DELETE FROM nix_pkg_extra", [])?;
            {
                let mut fts = tx.prepare(
                    "INSERT INTO nix_pkg
                     (attr_name, pname, version, description, long_description, main_program)
                     VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
                )?;
                let mut extra = tx.prepare(
                    "INSERT INTO nix_pkg_extra
                     (attr_name, homepage, license, position, platforms, maintainers,
                      broken, insecure, unfree)
                     VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
                )?;
                for r in &rows {
                    fts.execute(rusqlite::params![
                        r.attr_name,
                        r.pname,
                        r.version,
                        r.description,
                        r.long_description,
                        r.main_program,
                    ])?;
                    extra.execute(rusqlite::params![
                        r.attr_name,
                        r.homepage,
                        r.license,
                        r.position,
                        r.platforms,
                        r.maintainers,
                        if r.broken { 1 } else { 0 },
                        if r.insecure { 1 } else { 0 },
                        if r.unfree { 1 } else { 0 },
                    ])?;
                }
            }
            tx.commit()?;
            Ok::<_, rusqlite::Error>(rows.len())
        })
        .await
        .map_err(|e| rusqlite::Error::ToSqlConversionFailure(Box::new(e)))?
    }

    /// Replace the entire nix_opt table contents in one transaction.
    pub async fn replace_nix_options(&self, rows: Vec<NixOptRow>) -> rusqlite::Result<usize> {
        let conn = self.conn.clone();
        tokio::task::spawn_blocking(move || {
            let mut conn = conn.blocking_lock();
            let tx = conn.transaction()?;
            tx.execute("DELETE FROM nix_opt", [])?;
            {
                let mut stmt = tx.prepare(
                    "INSERT INTO nix_opt (name, type_, default_, description)
                     VALUES (?1, ?2, ?3, ?4)",
                )?;
                for r in &rows {
                    stmt.execute(rusqlite::params![
                        r.name,
                        r.type_,
                        r.default_,
                        r.description,
                    ])?;
                }
            }
            tx.commit()?;
            Ok::<_, rusqlite::Error>(rows.len())
        })
        .await
        .map_err(|e| rusqlite::Error::ToSqlConversionFailure(Box::new(e)))?
    }

    pub async fn search_nix_packages(
        &self,
        query: &str,
        limit: i64,
    ) -> rusqlite::Result<Vec<NixPkgRow>> {
        let conn = self.conn.clone();
        let q = fts_phrase(query);
        let raw = query.trim().to_string();
        tokio::task::spawn_blocking(move || {
            let conn = conn.blocking_lock();
            // Rank: exact attr_name / pname / main_program matches always win,
            // then BM25 with strong attr_name weight. lower BM25 = better.
            let mut stmt = conn.prepare(
                "SELECT p.attr_name, p.pname, p.version, p.description,
                        p.long_description, p.main_program,
                        e.homepage, e.license, e.position, e.platforms,
                        e.maintainers, e.broken, e.insecure, e.unfree
                 FROM nix_pkg p
                 LEFT JOIN nix_pkg_extra e ON e.attr_name = p.attr_name
                 WHERE nix_pkg MATCH ?1
                 ORDER BY
                   (p.attr_name = ?2) DESC,
                   (p.pname = ?2) DESC,
                   (p.main_program = ?2) DESC,
                   bm25(nix_pkg, 12.0, 8.0, 1.0, 1.5, 0.5, 4.0)
                 LIMIT ?3",
            )?;
            let rows = stmt
                .query_map(rusqlite::params![q, raw, limit], pkg_row_with_extra)?
                .collect::<Result<Vec<_>, _>>()?;
            Ok::<_, rusqlite::Error>(rows)
        })
        .await
        .map_err(|e| rusqlite::Error::ToSqlConversionFailure(Box::new(e)))?
    }

    pub async fn search_nix_options(
        &self,
        query: &str,
        limit: i64,
    ) -> rusqlite::Result<Vec<NixOptRow>> {
        let conn = self.conn.clone();
        let q = fts_phrase(query);
        tokio::task::spawn_blocking(move || {
            let conn = conn.blocking_lock();
            let mut stmt = conn.prepare(
                "SELECT name, type_, default_, description
                 FROM nix_opt
                 WHERE nix_opt MATCH ?1
                 ORDER BY bm25(nix_opt, 8.0, 1.0, 1.0, 2.0)
                 LIMIT ?2",
            )?;
            let rows = stmt
                .query_map(rusqlite::params![q, limit], |row| {
                    Ok(NixOptRow {
                        name: row.get(0)?,
                        type_: row.get(1)?,
                        default_: row.get(2)?,
                        description: row.get(3)?,
                    })
                })?
                .collect::<Result<Vec<_>, _>>()?;
            Ok::<_, rusqlite::Error>(rows)
        })
        .await
        .map_err(|e| rusqlite::Error::ToSqlConversionFailure(Box::new(e)))?
    }

    /// Lookup packages that provide `binary` as their mainProgram, falling
    /// back to attr_name / pname matches (covers the common "binary == package
    /// name" case).
    pub async fn search_nix_programs(
        &self,
        binary: &str,
        limit: i64,
    ) -> rusqlite::Result<Vec<NixPkgRow>> {
        let conn = self.conn.clone();
        let b = binary.to_string();
        tokio::task::spawn_blocking(move || {
            let conn = conn.blocking_lock();
            // First exact mainProgram, then attr_name, then pname. Union via
            // a temporary CTE so we keep the order.
            let mut stmt = conn.prepare(
                "SELECT p.attr_name, p.pname, p.version, p.description,
                        p.long_description, p.main_program,
                        e.homepage, e.license, e.position, e.platforms,
                        e.maintainers, e.broken, e.insecure, e.unfree
                 FROM nix_pkg p
                 LEFT JOIN nix_pkg_extra e ON e.attr_name = p.attr_name
                 WHERE p.main_program = ?1 OR p.attr_name = ?1 OR p.pname = ?1
                 ORDER BY (p.main_program = ?1) DESC, (p.attr_name = ?1) DESC
                 LIMIT ?2",
            )?;
            let rows = stmt
                .query_map(rusqlite::params![b, limit], pkg_row_with_extra)?
                .collect::<Result<Vec<_>, _>>()?;
            Ok::<_, rusqlite::Error>(rows)
        })
        .await
        .map_err(|e| rusqlite::Error::ToSqlConversionFailure(Box::new(e)))?
    }

    // ---------- render cache (typst/mitex → file_id) ----------

    pub async fn render_cache_get(&self, doc_hash: &str) -> rusqlite::Result<Option<Vec<String>>> {
        let conn = self.conn.clone();
        let doc_hash = doc_hash.to_string();
        tokio::task::spawn_blocking(move || {
            let conn = conn.blocking_lock();
            let mut stmt = conn
                .prepare("SELECT file_ids_json FROM render_cache WHERE doc_hash = ?1")?;
            let mut rows = stmt.query(rusqlite::params![doc_hash])?;
            let Some(row) = rows.next()? else {
                return Ok(None);
            };
            let json: String = row.get(0)?;
            drop(rows);
            drop(stmt);
            // Best-effort hit-counter bump.
            let _ = conn.execute(
                "UPDATE render_cache SET hits = hits + 1 WHERE doc_hash = ?1",
                rusqlite::params![doc_hash],
            );
            match serde_json::from_str::<Vec<String>>(&json) {
                Ok(v) => Ok::<_, rusqlite::Error>(Some(v)),
                Err(_) => Ok(None),
            }
        })
        .await
        .map_err(|e| rusqlite::Error::ToSqlConversionFailure(Box::new(e)))?
    }

    pub async fn render_cache_put(
        &self,
        doc_hash: &str,
        file_ids: &[String],
    ) -> rusqlite::Result<()> {
        let conn = self.conn.clone();
        let doc_hash = doc_hash.to_string();
        let json = serde_json::to_string(file_ids).unwrap_or_else(|_| "[]".to_string());
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_secs() as i64;
        tokio::task::spawn_blocking(move || {
            let conn = conn.blocking_lock();
            conn.execute(
                "INSERT INTO render_cache(doc_hash, file_ids_json, created, hits) \
                 VALUES(?1, ?2, ?3, 0) \
                 ON CONFLICT(doc_hash) DO UPDATE SET \
                   file_ids_json = excluded.file_ids_json, created = excluded.created",
                rusqlite::params![doc_hash, json, now],
            )?;
            Ok::<_, rusqlite::Error>(())
        })
        .await
        .map_err(|e| rusqlite::Error::ToSqlConversionFailure(Box::new(e)))?
    }

    // ---------- per-user inline opt-in ----------

    pub async fn inline_opt_in_get(&self, user_id: i64) -> rusqlite::Result<bool> {
        let conn = self.conn.clone();
        tokio::task::spawn_blocking(move || {
            let conn = conn.blocking_lock();
            let mut stmt = conn.prepare("SELECT 1 FROM inline_opt_in WHERE user_id = ?1")?;
            let mut rows = stmt.query(rusqlite::params![user_id])?;
            Ok::<_, rusqlite::Error>(rows.next()?.is_some())
        })
        .await
        .map_err(|e| rusqlite::Error::ToSqlConversionFailure(Box::new(e)))?
    }

    /// Flip the user's opt-in state. Returns the new state.
    pub async fn inline_opt_in_toggle(&self, user_id: i64) -> rusqlite::Result<bool> {
        let conn = self.conn.clone();
        tokio::task::spawn_blocking(move || {
            let conn = conn.blocking_lock();
            let mut stmt = conn.prepare("SELECT 1 FROM inline_opt_in WHERE user_id = ?1")?;
            let already = stmt.query(rusqlite::params![user_id])?.next()?.is_some();
            drop(stmt);
            if already {
                conn.execute(
                    "DELETE FROM inline_opt_in WHERE user_id = ?1",
                    rusqlite::params![user_id],
                )?;
                Ok::<_, rusqlite::Error>(false)
            } else {
                let now = std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .unwrap()
                    .as_secs() as i64;
                conn.execute(
                    "INSERT INTO inline_opt_in(user_id, since) VALUES(?1, ?2)",
                    rusqlite::params![user_id, now],
                )?;
                Ok(true)
            }
        })
        .await
        .map_err(|e| rusqlite::Error::ToSqlConversionFailure(Box::new(e)))?
    }

    // ---------- per-chat feature toggles ----------

    pub async fn feature_set(
        &self,
        chat_id: i64,
        feature: &str,
        value: &str,
    ) -> rusqlite::Result<()> {
        let conn = self.conn.clone();
        let feature = feature.to_string();
        let value = value.to_string();
        tokio::task::spawn_blocking(move || {
            let conn = conn.blocking_lock();
            conn.execute(
                "INSERT INTO chat_features(chat_id, feature, value) VALUES(?1, ?2, ?3)
                 ON CONFLICT(chat_id, feature) DO UPDATE SET value = excluded.value",
                rusqlite::params![chat_id, feature, value],
            )?;
            Ok::<_, rusqlite::Error>(())
        })
        .await
        .map_err(|e| rusqlite::Error::ToSqlConversionFailure(Box::new(e)))?
    }

    /// All rules (pattern, value) for a chat. Used by the hierarchical
    /// resolver in [`crate::features`]. Values are strings; the registry
    /// declares the type and the resolver parses on read.
    pub async fn feature_rules_for_chat(
        &self,
        chat_id: i64,
    ) -> rusqlite::Result<Vec<(String, String)>> {
        let conn = self.conn.clone();
        tokio::task::spawn_blocking(move || {
            let conn = conn.blocking_lock();
            let mut stmt = conn.prepare(
                "SELECT feature, value FROM chat_features WHERE chat_id = ?1 ORDER BY feature",
            )?;
            let rows = stmt.query_map(rusqlite::params![chat_id], |row| {
                let pat: String = row.get(0)?;
                let val: String = row.get(1)?;
                Ok((pat, val))
            })?;
            let mut out = Vec::new();
            for r in rows {
                out.push(r?);
            }
            Ok::<_, rusqlite::Error>(out)
        })
        .await
        .map_err(|e| rusqlite::Error::ToSqlConversionFailure(Box::new(e)))?
    }

    /// Delete every rule whose pattern starts with `<prefix>.`, plus an
    /// exact match on `<prefix>` itself. Backs `/feature <prefix>.** reset`.
    /// Returns the number of rows removed.
    pub async fn feature_clear_prefix(
        &self,
        chat_id: i64,
        prefix: &str,
    ) -> rusqlite::Result<usize> {
        let conn = self.conn.clone();
        let exact = prefix.to_string();
        let like = format!("{prefix}.%");
        tokio::task::spawn_blocking(move || {
            let conn = conn.blocking_lock();
            let n = conn.execute(
                "DELETE FROM chat_features WHERE chat_id = ?1 \
                 AND (feature = ?2 OR feature LIKE ?3)",
                rusqlite::params![chat_id, exact, like],
            )?;
            Ok::<_, rusqlite::Error>(n)
        })
        .await
        .map_err(|e| rusqlite::Error::ToSqlConversionFailure(Box::new(e)))?
    }

    pub async fn feature_clear(&self, chat_id: i64, feature: &str) -> rusqlite::Result<()> {
        let conn = self.conn.clone();
        let feature = feature.to_string();
        tokio::task::spawn_blocking(move || {
            let conn = conn.blocking_lock();
            conn.execute(
                "DELETE FROM chat_features WHERE chat_id = ?1 AND feature = ?2",
                rusqlite::params![chat_id, feature],
            )?;
            Ok::<_, rusqlite::Error>(())
        })
        .await
        .map_err(|e| rusqlite::Error::ToSqlConversionFailure(Box::new(e)))?
    }

    // ---------- gossip (cross-chat tea presence) ----------

    /// Upsert a row in `user_membership` — bumped on every incoming message.
    pub async fn bump_membership(
        &self,
        user_id: i64,
        chat_id: i64,
        user_name: String,
        now_unix: i64,
    ) -> rusqlite::Result<()> {
        let conn = self.conn.clone();
        tokio::task::spawn_blocking(move || {
            let conn = conn.blocking_lock();
            conn.execute(
                "INSERT INTO user_membership(user_id, chat_id, last_seen, user_name)
                 VALUES (?1, ?2, ?3, ?4)
                 ON CONFLICT(user_id, chat_id) DO UPDATE SET
                     last_seen = excluded.last_seen,
                     user_name = excluded.user_name",
                rusqlite::params![user_id, chat_id, now_unix, user_name],
            )?;
            Ok::<_, rusqlite::Error>(())
        })
        .await
        .map_err(|e| rusqlite::Error::ToSqlConversionFailure(Box::new(e)))?
    }

    pub async fn gossip_get(&self, user_id: i64) -> rusqlite::Result<bool> {
        let conn = self.conn.clone();
        tokio::task::spawn_blocking(move || {
            let conn = conn.blocking_lock();
            let mut stmt =
                conn.prepare("SELECT enabled FROM cha_gossip_optin WHERE user_id = ?1")?;
            let mut rows = stmt.query(rusqlite::params![user_id])?;
            if let Some(row) = rows.next()? {
                let v: i64 = row.get(0)?;
                Ok::<_, rusqlite::Error>(v != 0)
            } else {
                Ok(false)
            }
        })
        .await
        .map_err(|e| rusqlite::Error::ToSqlConversionFailure(Box::new(e)))?
    }

    pub async fn gossip_set(&self, user_id: i64, enabled: bool) -> rusqlite::Result<()> {
        let conn = self.conn.clone();
        tokio::task::spawn_blocking(move || {
            let conn = conn.blocking_lock();
            conn.execute(
                "INSERT INTO cha_gossip_optin(user_id, enabled) VALUES(?1, ?2)
                 ON CONFLICT(user_id) DO UPDATE SET enabled = excluded.enabled",
                rusqlite::params![user_id, if enabled { 1 } else { 0 }],
            )?;
            Ok::<_, rusqlite::Error>(())
        })
        .await
        .map_err(|e| rusqlite::Error::ToSqlConversionFailure(Box::new(e)))?
    }

    /// User IDs of opted-in users who share at least one chat with `asker`.
    pub async fn gossip_targets(&self, asker: i64) -> rusqlite::Result<Vec<i64>> {
        let conn = self.conn.clone();
        tokio::task::spawn_blocking(move || {
            let conn = conn.blocking_lock();
            let mut stmt = conn.prepare(
                "SELECT DISTINCT m_other.user_id
                 FROM   user_membership m_self
                 JOIN   user_membership m_other
                        ON m_self.chat_id = m_other.chat_id
                       AND m_other.user_id <> m_self.user_id
                 WHERE  m_self.user_id = ?1
                   AND  m_other.user_id IN
                        (SELECT user_id FROM cha_gossip_optin WHERE enabled = 1)",
            )?;
            let rows = stmt
                .query_map(rusqlite::params![asker], |row| row.get::<_, i64>(0))?
                .collect::<Result<Vec<_>, _>>()?;
            Ok::<_, rusqlite::Error>(rows)
        })
        .await
        .map_err(|e| rusqlite::Error::ToSqlConversionFailure(Box::new(e)))?
    }
}

#[derive(Debug, Clone, Default)]
pub struct NixPkgRow {
    pub attr_name: String,
    pub pname: String,
    pub version: String,
    pub description: String,
    pub long_description: String,
    pub main_program: String,
    // From nix_pkg_extra. Empty strings when absent.
    pub homepage: String,
    pub license: String,
    /// `pkgs/by-name/.../foo.nix:62` — relative path inside nixpkgs, with
    /// trailing `:line`. Empty when absent.
    pub position: String,
    /// CSV of `<arch>-<os>` triples. Empty when absent.
    pub platforms: String,
    /// CSV of github handles. Empty when absent.
    pub maintainers: String,
    pub broken: bool,
    pub insecure: bool,
    pub unfree: bool,
}

#[derive(Debug, Clone)]
pub struct NixOptRow {
    pub name: String,
    pub type_: String,
    pub default_: String,
    pub description: String,
}

/// Row mapper for `nix_pkg LEFT JOIN nix_pkg_extra` — columns 0..=5 come from
/// the FTS table, 6..=13 from the extras table (NULLs become empty strings /
/// false booleans).
fn pkg_row_with_extra(row: &rusqlite::Row<'_>) -> rusqlite::Result<NixPkgRow> {
    Ok(NixPkgRow {
        attr_name: row.get(0)?,
        pname: row.get(1)?,
        version: row.get(2)?,
        description: row.get(3)?,
        long_description: row.get(4)?,
        main_program: row.get(5)?,
        homepage: row.get::<_, Option<String>>(6)?.unwrap_or_default(),
        license: row.get::<_, Option<String>>(7)?.unwrap_or_default(),
        position: row.get::<_, Option<String>>(8)?.unwrap_or_default(),
        platforms: row.get::<_, Option<String>>(9)?.unwrap_or_default(),
        maintainers: row.get::<_, Option<String>>(10)?.unwrap_or_default(),
        broken: row.get::<_, Option<i64>>(11)?.unwrap_or(0) != 0,
        insecure: row.get::<_, Option<i64>>(12)?.unwrap_or(0) != 0,
        unfree: row.get::<_, Option<i64>>(13)?.unwrap_or(0) != 0,
    })
}

/// Escape a user query for FTS5 by wrapping each whitespace-split token in
/// double quotes and joining with AND-implicit space. Suffix wildcard lets
/// `tail` match `tailscale`.
fn fts_phrase(q: &str) -> String {
    q.split_whitespace()
        .map(|tok| {
            let cleaned: String = tok
                .chars()
                .filter(|c| c.is_alphanumeric() || matches!(c, '-' | '_' | '.' | '/'))
                .collect();
            if cleaned.is_empty() {
                String::new()
            } else {
                format!("\"{cleaned}\"*")
            }
        })
        .filter(|s| !s.is_empty())
        .collect::<Vec<_>>()
        .join(" ")
}
