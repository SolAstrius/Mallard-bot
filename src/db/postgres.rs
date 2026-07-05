use std::collections::HashSet;

use deadpool_postgres::{Config, Pool, Runtime};
use tokio_postgres::NoTls;

use super::{ChabaniRow, DbError, NixOptRow, NixPkgRow};

type Result<T> = std::result::Result<T, DbError>;

#[derive(Clone)]
pub struct PgDb {
    pool: Pool,
}

const SCHEMA: &str = "
CREATE TABLE IF NOT EXISTS cha_chabani (
    id              TEXT    PRIMARY KEY,
    label           TEXT    NOT NULL,
    origin_chat_id  BIGINT  NOT NULL,
    started_at      BIGINT  NOT NULL,
    ended_at        BIGINT  NOT NULL,
    duration_s      BIGINT  NOT NULL,
    sips            BIGINT  NOT NULL,
    auto_closed     BOOLEAN NOT NULL,
    notes           TEXT    NOT NULL,
    participants    TEXT    NOT NULL
);

CREATE INDEX IF NOT EXISTS idx_cha_chabani_chat_ended
    ON cha_chabani(origin_chat_id, ended_at DESC);

CREATE TABLE IF NOT EXISTS cha_chat_settings (
    chat_id   BIGINT  PRIMARY KEY,
    tea_aware BOOLEAN NOT NULL DEFAULT FALSE,
    set_at    BIGINT  NOT NULL
);

CREATE TABLE IF NOT EXISTS nix_pkg (
    attr_name        TEXT PRIMARY KEY,
    pname            TEXT NOT NULL,
    version          TEXT NOT NULL,
    description      TEXT NOT NULL,
    long_description TEXT NOT NULL,
    main_program     TEXT NOT NULL,
    homepage         TEXT NOT NULL DEFAULT '',
    license          TEXT NOT NULL DEFAULT '',
    position         TEXT NOT NULL DEFAULT '',
    platforms        TEXT NOT NULL DEFAULT '',
    maintainers      TEXT NOT NULL DEFAULT '',
    broken           BOOLEAN NOT NULL DEFAULT FALSE,
    insecure         BOOLEAN NOT NULL DEFAULT FALSE,
    unfree           BOOLEAN NOT NULL DEFAULT FALSE,
    search_vec       tsvector
);

CREATE INDEX IF NOT EXISTS idx_nix_pkg_search ON nix_pkg USING GIN(search_vec);
CREATE INDEX IF NOT EXISTS idx_nix_pkg_main_program ON nix_pkg(main_program);
CREATE INDEX IF NOT EXISTS idx_nix_pkg_pname ON nix_pkg(pname);

CREATE TABLE IF NOT EXISTS nix_opt (
    name        TEXT PRIMARY KEY,
    type_       TEXT NOT NULL,
    default_    TEXT NOT NULL,
    description TEXT NOT NULL,
    search_vec  tsvector
);

CREATE INDEX IF NOT EXISTS idx_nix_opt_search ON nix_opt USING GIN(search_vec);

CREATE TABLE IF NOT EXISTS nix_meta (
    key   TEXT PRIMARY KEY,
    value TEXT NOT NULL
);

CREATE TABLE IF NOT EXISTS render_cache (
    doc_hash      TEXT PRIMARY KEY,
    file_ids_json TEXT    NOT NULL,
    created       BIGINT NOT NULL,
    hits          BIGINT NOT NULL DEFAULT 0
);

CREATE TABLE IF NOT EXISTS inline_opt_in (
    user_id BIGINT PRIMARY KEY,
    since   BIGINT NOT NULL
);

CREATE TABLE IF NOT EXISTS chat_features (
    chat_id BIGINT NOT NULL,
    feature TEXT   NOT NULL,
    value   TEXT   NOT NULL DEFAULT 'on',
    PRIMARY KEY (chat_id, feature)
);

CREATE TABLE IF NOT EXISTS user_membership (
    user_id   BIGINT NOT NULL,
    chat_id   BIGINT NOT NULL,
    last_seen BIGINT NOT NULL,
    user_name TEXT   NOT NULL,
    PRIMARY KEY (user_id, chat_id)
);

CREATE INDEX IF NOT EXISTS idx_user_membership_user
    ON user_membership(user_id);
";

impl PgDb {
    pub async fn open(database_url: &str) -> Result<Self> {
        let mut cfg = Config::new();
        cfg.url = Some(database_url.to_string());
        cfg.pool = Some(deadpool_postgres::PoolConfig::new(10));

        let pool = cfg
            .create_pool(Some(Runtime::Tokio1), NoTls)
            .map_err(|e| DbError::Other(e.to_string()))?;

        let client = pool.get().await.map_err(|e| DbError::Other(e.to_string()))?;
        client.batch_execute(SCHEMA).await?;

        Ok(Self { pool })
    }

    async fn conn(
        &self,
    ) -> Result<deadpool_postgres::Object> {
        self.pool
            .get()
            .await
            .map_err(|e| DbError::Other(e.to_string()))
    }

    // ---------- chabani ----------

    #[allow(clippy::too_many_arguments)]
    pub async fn insert_chabani(
        &self,
        id: String,
        label: String,
        origin_chat_id: i64,
        started_at: i64,
        ended_at: i64,
        duration_s: i64,
        sips: i64,
        auto_closed: bool,
        notes_json: String,
        participants_json: String,
    ) -> Result<()> {
        let c = self.conn().await?;
        c.execute(
            "INSERT INTO cha_chabani
             (id, label, origin_chat_id, started_at, ended_at,
              duration_s, sips, auto_closed, notes, participants)
             VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10)",
            &[
                &id,
                &label,
                &origin_chat_id,
                &started_at,
                &ended_at,
                &duration_s,
                &sips,
                &auto_closed,
                &notes_json,
                &participants_json,
            ],
        )
        .await?;
        Ok(())
    }

    pub async fn tail_chabani_by_origin(
        &self,
        chat_id: i64,
        limit: i64,
    ) -> Result<Vec<ChabaniRow>> {
        let c = self.conn().await?;
        let rows = c
            .query(
                "SELECT id, label, origin_chat_id, started_at, duration_s,
                        sips, auto_closed, notes, participants
                 FROM cha_chabani
                 WHERE origin_chat_id = $1
                 ORDER BY ended_at DESC
                 LIMIT $2",
                &[&chat_id, &limit],
            )
            .await?;
        Ok(rows
            .iter()
            .map(|r| ChabaniRow {
                id: r.get(0),
                label: r.get(1),
                origin_chat_id: r.get(2),
                started_at: r.get(3),
                duration_s: r.get(4),
                sips: r.get(5),
                auto_closed: r.get(6),
                notes_json: r.get(7),
                participants_json: r.get(8),
            })
            .collect())
    }

    // ---------- nix catalog ----------

    pub async fn nix_meta_get(&self, key: &str) -> Result<Option<String>> {
        let c = self.conn().await?;
        let rows = c
            .query("SELECT value FROM nix_meta WHERE key = $1", &[&key])
            .await?;
        Ok(rows.first().map(|r| r.get(0)))
    }

    pub async fn nix_extras_missing(&self) -> Result<bool> {
        let c = self.conn().await?;
        let row = c
            .query_one(
                "SELECT
                   EXISTS(SELECT 1 FROM nix_pkg) AS has_pkg,
                   NOT EXISTS(SELECT 1 FROM nix_pkg WHERE homepage != '') AS no_extras",
                &[],
            )
            .await?;
        let has_pkg: bool = row.get(0);
        let no_extras: bool = row.get(1);
        Ok(has_pkg && no_extras)
    }

    pub async fn nix_meta_set(&self, key: &str, value: &str) -> Result<()> {
        let c = self.conn().await?;
        c.execute(
            "INSERT INTO nix_meta(key, value) VALUES($1, $2)
             ON CONFLICT(key) DO UPDATE SET value = EXCLUDED.value",
            &[&key, &value],
        )
        .await?;
        Ok(())
    }

    pub async fn replace_nix_packages(&self, rows: Vec<NixPkgRow>) -> Result<usize> {
        let mut c = self.conn().await?;
        let tx = c.transaction().await?;

        tx.execute("TRUNCATE nix_pkg", &[]).await?;

        // Batch in chunks to avoid oversized statements.
        const BATCH: usize = 500;
        for chunk in rows.chunks(BATCH) {
            let mut sql = String::from(
                "INSERT INTO nix_pkg (attr_name, pname, version, description,
                 long_description, main_program, homepage, license, position,
                 platforms, maintainers, broken, insecure, unfree, search_vec) VALUES ",
            );
            let mut params: Vec<&(dyn tokio_postgres::types::ToSql + Sync)> = Vec::new();
            let mut param_idx = 1u32;

            // We need owned storage for bools since we borrow references into params.
            struct RowVals {
                attr_name: String,
                pname: String,
                version: String,
                description: String,
                long_description: String,
                main_program: String,
                homepage: String,
                license: String,
                position: String,
                platforms: String,
                maintainers: String,
                broken: bool,
                insecure: bool,
                unfree: bool,
            }
            let vals: Vec<RowVals> = chunk
                .iter()
                .map(|r| RowVals {
                    attr_name: r.attr_name.clone(),
                    pname: r.pname.clone(),
                    version: r.version.clone(),
                    description: r.description.clone(),
                    long_description: r.long_description.clone(),
                    main_program: r.main_program.clone(),
                    homepage: r.homepage.clone(),
                    license: r.license.clone(),
                    position: r.position.clone(),
                    platforms: r.platforms.clone(),
                    maintainers: r.maintainers.clone(),
                    broken: r.broken,
                    insecure: r.insecure,
                    unfree: r.unfree,
                })
                .collect();

            for (i, v) in vals.iter().enumerate() {
                if i > 0 {
                    sql.push(',');
                }
                let p = param_idx;
                sql.push_str(&format!(
                    "(${}, ${}, ${}, ${}, ${}, ${}, ${}, ${}, ${}, ${}, ${}, ${}, ${}, ${}, \
                     setweight(to_tsvector('simple', ${}), 'A') || \
                     setweight(to_tsvector('simple', ${}), 'B') || \
                     setweight(to_tsvector('simple', ${}), 'C') || \
                     setweight(to_tsvector('simple', ${}), 'D'))",
                    p, p+1, p+2, p+3, p+4, p+5, p+6, p+7, p+8, p+9, p+10, p+11, p+12, p+13,
                    // tsvector weights: A=attr_name, B=pname, C=main_program, D=description
                    p, p+1, p+5, p+3,
                ));
                param_idx += 14;

                params.push(&v.attr_name);
                params.push(&v.pname);
                params.push(&v.version);
                params.push(&v.description);
                params.push(&v.long_description);
                params.push(&v.main_program);
                params.push(&v.homepage);
                params.push(&v.license);
                params.push(&v.position);
                params.push(&v.platforms);
                params.push(&v.maintainers);
                params.push(&v.broken);
                params.push(&v.insecure);
                params.push(&v.unfree);
            }

            tx.execute(&sql, &params).await?;
        }

        tx.commit().await?;
        Ok(rows.len())
    }

    pub async fn replace_nix_options(&self, rows: Vec<NixOptRow>) -> Result<usize> {
        let mut c = self.conn().await?;
        let tx = c.transaction().await?;

        tx.execute("TRUNCATE nix_opt", &[]).await?;

        const BATCH: usize = 500;
        for chunk in rows.chunks(BATCH) {
            let mut sql = String::from(
                "INSERT INTO nix_opt (name, type_, default_, description, search_vec) VALUES ",
            );
            let mut params: Vec<&(dyn tokio_postgres::types::ToSql + Sync)> = Vec::new();
            let mut param_idx = 1u32;

            struct RowVals {
                name: String,
                type_: String,
                default_: String,
                description: String,
            }
            let vals: Vec<RowVals> = chunk
                .iter()
                .map(|r| RowVals {
                    name: r.name.clone(),
                    type_: r.type_.clone(),
                    default_: r.default_.clone(),
                    description: r.description.clone(),
                })
                .collect();

            for (i, v) in vals.iter().enumerate() {
                if i > 0 {
                    sql.push(',');
                }
                let p = param_idx;
                sql.push_str(&format!(
                    "(${}, ${}, ${}, ${}, \
                     setweight(to_tsvector('simple', ${}), 'A') || \
                     setweight(to_tsvector('simple', ${}), 'D'))",
                    p, p+1, p+2, p+3,
                    // A=name, D=description
                    p, p+3,
                ));
                param_idx += 4;

                params.push(&v.name);
                params.push(&v.type_);
                params.push(&v.default_);
                params.push(&v.description);
            }

            tx.execute(&sql, &params).await?;
        }

        tx.commit().await?;
        Ok(rows.len())
    }

    pub async fn search_nix_packages(
        &self,
        query: &str,
        limit: i64,
    ) -> Result<Vec<NixPkgRow>> {
        let c = self.conn().await?;
        let raw = query.trim().to_string();
        let rows = c
            .query(
                "SELECT attr_name, pname, version, description, long_description,
                        main_program, homepage, license, position, platforms,
                        maintainers, broken, insecure, unfree
                 FROM nix_pkg
                 WHERE search_vec @@ plainto_tsquery('simple', $1)
                    OR attr_name = $2 OR pname = $2 OR main_program = $2
                 ORDER BY
                   (attr_name = $2) DESC,
                   (pname = $2) DESC,
                   (main_program = $2) DESC,
                   ts_rank_cd(search_vec, plainto_tsquery('simple', $1)) DESC
                 LIMIT $3",
                &[&raw, &raw, &limit],
            )
            .await?;
        Ok(rows.iter().map(pkg_from_row).collect())
    }

    pub async fn search_nix_options(
        &self,
        query: &str,
        limit: i64,
    ) -> Result<Vec<NixOptRow>> {
        let c = self.conn().await?;
        let raw = query.trim().to_string();
        let prefix = format!("{}%", raw);
        let rows = c
            .query(
                "SELECT name, type_, default_, description
                 FROM nix_opt
                 WHERE name LIKE $1
                    OR search_vec @@ plainto_tsquery('simple', $2)
                 ORDER BY
                   (name = $2) DESC,
                   (name LIKE $1) DESC,
                   ts_rank_cd(search_vec, plainto_tsquery('simple', $2)) DESC
                 LIMIT $3",
                &[&prefix, &raw, &limit],
            )
            .await?;
        Ok(rows
            .iter()
            .map(|r| NixOptRow {
                name: r.get(0),
                type_: r.get(1),
                default_: r.get(2),
                description: r.get(3),
            })
            .collect())
    }

    pub async fn search_nix_programs(
        &self,
        binary: &str,
        limit: i64,
    ) -> Result<Vec<NixPkgRow>> {
        let c = self.conn().await?;
        let rows = c
            .query(
                "SELECT attr_name, pname, version, description, long_description,
                        main_program, homepage, license, position, platforms,
                        maintainers, broken, insecure, unfree
                 FROM nix_pkg
                 WHERE main_program = $1 OR attr_name = $1 OR pname = $1
                 ORDER BY (main_program = $1) DESC, (attr_name = $1) DESC
                 LIMIT $2",
                &[&binary, &limit],
            )
            .await?;
        Ok(rows.iter().map(pkg_from_row).collect())
    }

    // ---------- render cache ----------

    pub async fn render_cache_get(&self, doc_hash: &str) -> Result<Option<Vec<String>>> {
        let c = self.conn().await?;
        let rows = c
            .query(
                "SELECT file_ids_json FROM render_cache WHERE doc_hash = $1",
                &[&doc_hash],
            )
            .await?;
        let Some(row) = rows.first() else {
            return Ok(None);
        };
        let json: String = row.get(0);
        // Best-effort hit counter bump.
        let _ = c
            .execute(
                "UPDATE render_cache SET hits = hits + 1 WHERE doc_hash = $1",
                &[&doc_hash],
            )
            .await;
        match serde_json::from_str::<Vec<String>>(&json) {
            Ok(v) => Ok(Some(v)),
            Err(_) => Ok(None),
        }
    }

    pub async fn render_cache_put(&self, doc_hash: &str, file_ids: &[String]) -> Result<()> {
        let c = self.conn().await?;
        let json = serde_json::to_string(file_ids).unwrap_or_else(|_| "[]".to_string());
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_secs() as i64;
        c.execute(
            "INSERT INTO render_cache(doc_hash, file_ids_json, created, hits)
             VALUES($1, $2, $3, 0)
             ON CONFLICT(doc_hash) DO UPDATE SET
               file_ids_json = EXCLUDED.file_ids_json, created = EXCLUDED.created",
            &[&doc_hash, &json, &now],
        )
        .await?;
        Ok(())
    }

    // ---------- inline opt-in ----------

    pub async fn inline_opt_in_get(&self, user_id: i64) -> Result<bool> {
        let c = self.conn().await?;
        let rows = c
            .query(
                "SELECT 1 FROM inline_opt_in WHERE user_id = $1",
                &[&user_id],
            )
            .await?;
        Ok(!rows.is_empty())
    }

    pub async fn inline_opt_in_toggle(&self, user_id: i64) -> Result<bool> {
        let c = self.conn().await?;
        let rows = c
            .query(
                "SELECT 1 FROM inline_opt_in WHERE user_id = $1",
                &[&user_id],
            )
            .await?;
        if !rows.is_empty() {
            c.execute("DELETE FROM inline_opt_in WHERE user_id = $1", &[&user_id])
                .await?;
            Ok(false)
        } else {
            let now = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_secs() as i64;
            c.execute(
                "INSERT INTO inline_opt_in(user_id, since) VALUES($1, $2)",
                &[&user_id, &now],
            )
            .await?;
            Ok(true)
        }
    }

    // ---------- chat features ----------

    pub async fn feature_set(&self, chat_id: i64, feature: &str, value: &str) -> Result<()> {
        let c = self.conn().await?;
        c.execute(
            "INSERT INTO chat_features(chat_id, feature, value) VALUES($1, $2, $3)
             ON CONFLICT(chat_id, feature) DO UPDATE SET value = EXCLUDED.value",
            &[&chat_id, &feature, &value],
        )
        .await?;
        Ok(())
    }

    pub async fn feature_rules_for_chat(&self, chat_id: i64) -> Result<Vec<(String, String)>> {
        let c = self.conn().await?;
        let rows = c
            .query(
                "SELECT feature, value FROM chat_features
                 WHERE chat_id = $1 ORDER BY feature",
                &[&chat_id],
            )
            .await?;
        Ok(rows.iter().map(|r| (r.get(0), r.get(1))).collect())
    }

    pub async fn feature_clear_prefix(&self, chat_id: i64, prefix: &str) -> Result<usize> {
        let c = self.conn().await?;
        let like = format!("{prefix}.%");
        let n = c
            .execute(
                "DELETE FROM chat_features
                 WHERE chat_id = $1 AND (feature = $2 OR feature LIKE $3)",
                &[&chat_id, &prefix, &like],
            )
            .await?;
        Ok(n as usize)
    }

    pub async fn feature_clear(&self, chat_id: i64, feature: &str) -> Result<()> {
        let c = self.conn().await?;
        c.execute(
            "DELETE FROM chat_features WHERE chat_id = $1 AND feature = $2",
            &[&chat_id, &feature],
        )
        .await?;
        Ok(())
    }

    // ---------- chat membership + tea visibility ----------

    pub async fn touch_chat_member(
        &self,
        user_id: i64,
        chat_id: i64,
        user_name: String,
        now_unix: i64,
    ) -> Result<()> {
        let c = self.conn().await?;
        c.execute(
            "INSERT INTO user_membership(user_id, chat_id, last_seen, user_name)
             VALUES ($1, $2, $3, $4)
             ON CONFLICT(user_id, chat_id) DO UPDATE SET
                 last_seen = EXCLUDED.last_seen,
                 user_name = EXCLUDED.user_name",
            &[&user_id, &chat_id, &now_unix, &user_name],
        )
        .await?;
        Ok(())
    }

    pub async fn users_in_chat(&self, chat_id: i64) -> Result<Vec<i64>> {
        let c = self.conn().await?;
        let rows = c
            .query(
                "SELECT user_id FROM user_membership WHERE chat_id = $1",
                &[&chat_id],
            )
            .await?;
        Ok(rows.iter().map(|r| r.get(0)).collect())
    }

    pub async fn chats_for_user(&self, user_id: i64) -> Result<Vec<i64>> {
        let c = self.conn().await?;
        let rows = c
            .query(
                "SELECT chat_id FROM user_membership WHERE user_id = $1",
                &[&user_id],
            )
            .await?;
        Ok(rows.iter().map(|r| r.get(0)).collect())
    }

    pub async fn set_chat_tea_aware(&self, chat_id: i64, on: bool) -> Result<()> {
        let c = self.conn().await?;
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs() as i64)
            .unwrap_or(0);
        c.execute(
            "INSERT INTO cha_chat_settings(chat_id, tea_aware, set_at)
             VALUES ($1, $2, $3)
             ON CONFLICT(chat_id) DO UPDATE SET
                 tea_aware = EXCLUDED.tea_aware,
                 set_at    = EXCLUDED.set_at",
            &[&chat_id, &on, &now],
        )
        .await?;
        Ok(())
    }

    pub async fn is_chat_tea_aware(&self, chat_id: i64) -> Result<bool> {
        let c = self.conn().await?;
        let rows = c
            .query(
                "SELECT tea_aware FROM cha_chat_settings WHERE chat_id = $1",
                &[&chat_id],
            )
            .await?;
        Ok(rows.first().map(|r| r.get::<_, bool>(0)).unwrap_or(false))
    }

    pub async fn tea_aware_chats(&self) -> Result<HashSet<i64>> {
        let c = self.conn().await?;
        let rows = c
            .query(
                "SELECT chat_id FROM cha_chat_settings WHERE tea_aware = TRUE",
                &[],
            )
            .await?;
        Ok(rows.iter().map(|r| r.get(0)).collect())
    }
}

fn pkg_from_row(r: &tokio_postgres::Row) -> NixPkgRow {
    NixPkgRow {
        attr_name: r.get(0),
        pname: r.get(1),
        version: r.get(2),
        description: r.get(3),
        long_description: r.get(4),
        main_program: r.get(5),
        homepage: r.get(6),
        license: r.get(7),
        position: r.get(8),
        platforms: r.get(9),
        maintainers: r.get(10),
        broken: r.get(11),
        insecure: r.get(12),
        unfree: r.get(13),
    }
}
