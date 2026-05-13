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
        Ok(Self {
            conn: Arc::new(Mutex::new(conn)),
        })
    }

    /// Insert a closed cha session.
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
}
