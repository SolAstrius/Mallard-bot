mod sqlite;
mod postgres;

use std::collections::HashSet;

pub use sqlite::SqliteDb;
pub use postgres::PgDb;

#[derive(Debug)]
pub enum DbError {
    Sqlite(rusqlite::Error),
    Postgres(tokio_postgres::Error),
    Other(String),
}

impl std::fmt::Display for DbError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Sqlite(e) => write!(f, "sqlite: {e}"),
            Self::Postgres(e) => write!(f, "postgres: {e}"),
            Self::Other(e) => write!(f, "{e}"),
        }
    }
}

impl std::error::Error for DbError {}

impl From<rusqlite::Error> for DbError {
    fn from(e: rusqlite::Error) -> Self { Self::Sqlite(e) }
}

impl From<tokio_postgres::Error> for DbError {
    fn from(e: tokio_postgres::Error) -> Self { Self::Postgres(e) }
}

#[derive(Debug, Clone)]
pub struct ChabaniRow {
    pub id: String,
    pub label: String,
    pub origin_chat_id: i64,
    pub started_at: i64,
    pub duration_s: i64,
    pub sips: i64,
    pub auto_closed: bool,
    pub notes_json: String,
    pub participants_json: String,
}

#[derive(Debug, Clone, Default)]
pub struct NixPkgRow {
    pub attr_name: String,
    pub pname: String,
    pub version: String,
    pub description: String,
    pub long_description: String,
    pub main_program: String,
    pub homepage: String,
    pub license: String,
    pub position: String,
    pub platforms: String,
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

#[derive(Clone)]
pub enum Db {
    Sqlite(SqliteDb),
    Postgres(PgDb),
}

impl Db {
    pub fn open_sqlite(path: &std::path::Path) -> Result<Self, DbError> {
        Ok(Self::Sqlite(SqliteDb::open(path)?))
    }

    pub async fn open_postgres(url: &str) -> Result<Self, DbError> {
        Ok(Self::Postgres(PgDb::open(url).await?))
    }

    #[allow(clippy::too_many_arguments)]
    pub async fn insert_chabani(
        &self, id: String, label: String, origin_chat_id: i64, started_at: i64,
        ended_at: i64, duration_s: i64, sips: i64, auto_closed: bool,
        notes_json: String, participants_json: String,
    ) -> Result<(), DbError> {
        match self {
            Self::Sqlite(s) => s.insert_chabani(id, label, origin_chat_id, started_at, ended_at, duration_s, sips, auto_closed, notes_json, participants_json).await,
            Self::Postgres(p) => p.insert_chabani(id, label, origin_chat_id, started_at, ended_at, duration_s, sips, auto_closed, notes_json, participants_json).await,
        }
    }

    pub async fn tail_chabani_by_origin(&self, chat_id: i64, limit: i64) -> Result<Vec<ChabaniRow>, DbError> {
        match self { Self::Sqlite(s) => s.tail_chabani_by_origin(chat_id, limit).await, Self::Postgres(p) => p.tail_chabani_by_origin(chat_id, limit).await }
    }

    pub async fn nix_meta_get(&self, key: &str) -> Result<Option<String>, DbError> {
        match self { Self::Sqlite(s) => s.nix_meta_get(key).await, Self::Postgres(p) => p.nix_meta_get(key).await }
    }

    pub async fn nix_extras_missing(&self) -> Result<bool, DbError> {
        match self { Self::Sqlite(s) => s.nix_extras_missing().await, Self::Postgres(p) => p.nix_extras_missing().await }
    }

    pub async fn nix_meta_set(&self, key: &str, value: &str) -> Result<(), DbError> {
        match self { Self::Sqlite(s) => s.nix_meta_set(key, value).await, Self::Postgres(p) => p.nix_meta_set(key, value).await }
    }

    pub async fn replace_nix_packages(&self, rows: Vec<NixPkgRow>) -> Result<usize, DbError> {
        match self { Self::Sqlite(s) => s.replace_nix_packages(rows).await, Self::Postgres(p) => p.replace_nix_packages(rows).await }
    }

    pub async fn replace_nix_options(&self, rows: Vec<NixOptRow>) -> Result<usize, DbError> {
        match self { Self::Sqlite(s) => s.replace_nix_options(rows).await, Self::Postgres(p) => p.replace_nix_options(rows).await }
    }

    pub async fn search_nix_packages(&self, query: &str, limit: i64) -> Result<Vec<NixPkgRow>, DbError> {
        match self { Self::Sqlite(s) => s.search_nix_packages(query, limit).await, Self::Postgres(p) => p.search_nix_packages(query, limit).await }
    }

    pub async fn search_nix_options(&self, query: &str, limit: i64) -> Result<Vec<NixOptRow>, DbError> {
        match self { Self::Sqlite(s) => s.search_nix_options(query, limit).await, Self::Postgres(p) => p.search_nix_options(query, limit).await }
    }

    pub async fn search_nix_programs(&self, binary: &str, limit: i64) -> Result<Vec<NixPkgRow>, DbError> {
        match self { Self::Sqlite(s) => s.search_nix_programs(binary, limit).await, Self::Postgres(p) => p.search_nix_programs(binary, limit).await }
    }

    pub async fn render_cache_get(&self, doc_hash: &str) -> Result<Option<Vec<String>>, DbError> {
        match self { Self::Sqlite(s) => s.render_cache_get(doc_hash).await, Self::Postgres(p) => p.render_cache_get(doc_hash).await }
    }

    pub async fn render_cache_put(&self, doc_hash: &str, file_ids: &[String]) -> Result<(), DbError> {
        match self { Self::Sqlite(s) => s.render_cache_put(doc_hash, file_ids).await, Self::Postgres(p) => p.render_cache_put(doc_hash, file_ids).await }
    }

    pub async fn inline_opt_in_get(&self, user_id: i64) -> Result<bool, DbError> {
        match self { Self::Sqlite(s) => s.inline_opt_in_get(user_id).await, Self::Postgres(p) => p.inline_opt_in_get(user_id).await }
    }

    pub async fn inline_opt_in_toggle(&self, user_id: i64) -> Result<bool, DbError> {
        match self { Self::Sqlite(s) => s.inline_opt_in_toggle(user_id).await, Self::Postgres(p) => p.inline_opt_in_toggle(user_id).await }
    }

    pub async fn feature_set(&self, chat_id: i64, feature: &str, value: &str) -> Result<(), DbError> {
        match self { Self::Sqlite(s) => s.feature_set(chat_id, feature, value).await, Self::Postgres(p) => p.feature_set(chat_id, feature, value).await }
    }

    pub async fn feature_rules_for_chat(&self, chat_id: i64) -> Result<Vec<(String, String)>, DbError> {
        match self { Self::Sqlite(s) => s.feature_rules_for_chat(chat_id).await, Self::Postgres(p) => p.feature_rules_for_chat(chat_id).await }
    }

    pub async fn feature_clear_prefix(&self, chat_id: i64, prefix: &str) -> Result<usize, DbError> {
        match self { Self::Sqlite(s) => s.feature_clear_prefix(chat_id, prefix).await, Self::Postgres(p) => p.feature_clear_prefix(chat_id, prefix).await }
    }

    pub async fn feature_clear(&self, chat_id: i64, feature: &str) -> Result<(), DbError> {
        match self { Self::Sqlite(s) => s.feature_clear(chat_id, feature).await, Self::Postgres(p) => p.feature_clear(chat_id, feature).await }
    }

    pub async fn touch_chat_member(&self, user_id: i64, chat_id: i64, user_name: String, now_unix: i64) -> Result<(), DbError> {
        match self { Self::Sqlite(s) => s.touch_chat_member(user_id, chat_id, user_name, now_unix).await, Self::Postgres(p) => p.touch_chat_member(user_id, chat_id, user_name, now_unix).await }
    }

    pub async fn users_in_chat(&self, chat_id: i64) -> Result<Vec<i64>, DbError> {
        match self { Self::Sqlite(s) => s.users_in_chat(chat_id).await, Self::Postgres(p) => p.users_in_chat(chat_id).await }
    }

    pub async fn chats_for_user(&self, user_id: i64) -> Result<Vec<i64>, DbError> {
        match self { Self::Sqlite(s) => s.chats_for_user(user_id).await, Self::Postgres(p) => p.chats_for_user(user_id).await }
    }

    pub async fn set_chat_tea_aware(&self, chat_id: i64, on: bool) -> Result<(), DbError> {
        match self { Self::Sqlite(s) => s.set_chat_tea_aware(chat_id, on).await, Self::Postgres(p) => p.set_chat_tea_aware(chat_id, on).await }
    }

    pub async fn is_chat_tea_aware(&self, chat_id: i64) -> Result<bool, DbError> {
        match self { Self::Sqlite(s) => s.is_chat_tea_aware(chat_id).await, Self::Postgres(p) => p.is_chat_tea_aware(chat_id).await }
    }

    pub async fn tea_aware_chats(&self) -> Result<HashSet<i64>, DbError> {
        match self { Self::Sqlite(s) => s.tea_aware_chats().await, Self::Postgres(p) => p.tea_aware_chats().await }
    }
}
