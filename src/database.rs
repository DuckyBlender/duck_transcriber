use crate::types::TaskType;
use log::{info, warn};
use sqlx::sqlite::{SqliteConnectOptions, SqlitePoolOptions};
use sqlx::{Row, SqlitePool};
use std::env;
use std::str::FromStr;
use std::time::{SystemTime, UNIX_EPOCH};
use teloxide::types::UserId;

const CACHE_EXPIRATION_DAYS: i64 = 7;
const SUMMARY_CACHE_EXPIRATION_DAYS: i64 = 1;
const RATE_LIMIT_CLEANUP_MINUTES: i64 = 120;

pub struct Database {
    pool: SqlitePool,
}

impl Database {
    pub async fn connect() -> Result<Self, sqlx::Error> {
        let database_url =
            env::var("DATABASE_URL").unwrap_or_else(|_| "sqlite:duck_transcriber.db".to_string());

        let connect_options =
            SqliteConnectOptions::from_str(&database_url)?.create_if_missing(true);

        let pool = SqlitePoolOptions::new()
            .max_connections(5)
            .connect_with(connect_options)
            .await?;

        info!("Connected to SQLite database");

        Ok(Database { pool })
    }

    pub async fn init(&self) -> Result<(), sqlx::Error> {
        sqlx::query(
            r#"
            CREATE TABLE IF NOT EXISTS cache (
                unique_file_id TEXT PRIMARY KEY,
                transcribe TEXT,
                translate TEXT,
                summarize TEXT,
                caveman TEXT,
                transcribe_expires_at INTEGER,
                translate_expires_at INTEGER,
                summarize_expires_at INTEGER,
                caveman_expires_at INTEGER,
                expires_at INTEGER NOT NULL
            )
            "#,
        )
        .execute(&self.pool)
        .await?;

        sqlx::query(
            r#"
            CREATE TABLE IF NOT EXISTS rate_limits (
                user_id INTEGER NOT NULL,
                timestamp INTEGER NOT NULL
            )
            "#,
        )
        .execute(&self.pool)
        .await?;

        sqlx::query(
            r#"
            CREATE INDEX IF NOT EXISTS idx_rate_limits_user 
            ON rate_limits(user_id, timestamp)
            "#,
        )
        .execute(&self.pool)
        .await?;

        self.ensure_cache_expiration_columns().await?;
        self.migrate_cache_expiration_columns().await?;

        info!("Database tables initialized");
        self.cleanup_expired().await?;

        Ok(())
    }

    pub async fn get_cached(
        &self,
        unique_file_id: &str,
        task_type: &TaskType,
    ) -> Result<Option<String>, sqlx::Error> {
        let now = current_timestamp();

        let value: Option<Option<String>> = match task_type {
            TaskType::Transcribe => {
                sqlx::query_scalar(
                    r#"
                SELECT transcribe FROM cache
                WHERE unique_file_id = ? AND transcribe_expires_at > ?
                "#,
                )
                .bind(unique_file_id)
                .bind(now)
                .fetch_optional(&self.pool)
                .await?
            }
            TaskType::Translate => {
                sqlx::query_scalar(
                    r#"
                SELECT translate FROM cache
                WHERE unique_file_id = ? AND translate_expires_at > ?
                "#,
                )
                .bind(unique_file_id)
                .bind(now)
                .fetch_optional(&self.pool)
                .await?
            }
            TaskType::Summarize => {
                sqlx::query_scalar(
                    r#"
                SELECT summarize FROM cache
                WHERE unique_file_id = ? AND summarize_expires_at > ?
                "#,
                )
                .bind(unique_file_id)
                .bind(now)
                .fetch_optional(&self.pool)
                .await?
            }
            TaskType::Caveman => {
                sqlx::query_scalar(
                    r#"
                SELECT caveman FROM cache
                WHERE unique_file_id = ? AND caveman_expires_at > ?
                "#,
                )
                .bind(unique_file_id)
                .bind(now)
                .fetch_optional(&self.pool)
                .await?
            }
        };

        Ok(value.flatten())
    }

    pub async fn set_cached(
        &self,
        unique_file_id: &str,
        task_type: &TaskType,
        text: &str,
    ) -> Result<(), sqlx::Error> {
        let expires_at = current_timestamp() + (cache_expiration_days(task_type) * 24 * 3600);

        match task_type {
            TaskType::Transcribe => {
                sqlx::query(
                    r#"
                INSERT INTO cache (unique_file_id, transcribe, transcribe_expires_at, expires_at)
                VALUES (?, ?, ?, ?)
                ON CONFLICT(unique_file_id) DO UPDATE SET
                    transcribe = excluded.transcribe,
                    transcribe_expires_at = excluded.transcribe_expires_at,
                    expires_at = max(cache.expires_at, excluded.expires_at)
                "#,
                )
                .bind(unique_file_id)
                .bind(text)
                .bind(expires_at)
                .bind(expires_at)
                .execute(&self.pool)
                .await?;
            }
            TaskType::Translate => {
                sqlx::query(
                    r#"
                INSERT INTO cache (unique_file_id, translate, translate_expires_at, expires_at)
                VALUES (?, ?, ?, ?)
                ON CONFLICT(unique_file_id) DO UPDATE SET
                    translate = excluded.translate,
                    translate_expires_at = excluded.translate_expires_at,
                    expires_at = max(cache.expires_at, excluded.expires_at)
                "#,
                )
                .bind(unique_file_id)
                .bind(text)
                .bind(expires_at)
                .bind(expires_at)
                .execute(&self.pool)
                .await?;
            }
            TaskType::Summarize => {
                sqlx::query(
                    r#"
                INSERT INTO cache (unique_file_id, summarize, summarize_expires_at, expires_at)
                VALUES (?, ?, ?, ?)
                ON CONFLICT(unique_file_id) DO UPDATE SET
                    summarize = excluded.summarize,
                    summarize_expires_at = excluded.summarize_expires_at,
                    expires_at = max(cache.expires_at, excluded.expires_at)
                "#,
                )
                .bind(unique_file_id)
                .bind(text)
                .bind(expires_at)
                .bind(expires_at)
                .execute(&self.pool)
                .await?;
            }
            TaskType::Caveman => {
                sqlx::query(
                    r#"
                INSERT INTO cache (unique_file_id, caveman, caveman_expires_at, expires_at)
                VALUES (?, ?, ?, ?)
                ON CONFLICT(unique_file_id) DO UPDATE SET
                    caveman = excluded.caveman,
                    caveman_expires_at = excluded.caveman_expires_at,
                    expires_at = max(cache.expires_at, excluded.expires_at)
                "#,
                )
                .bind(unique_file_id)
                .bind(text)
                .bind(expires_at)
                .bind(expires_at)
                .execute(&self.pool)
                .await?;
            }
        }

        Ok(())
    }

    async fn ensure_cache_expiration_columns(&self) -> Result<(), sqlx::Error> {
        let columns = sqlx::query("PRAGMA table_info(cache)")
            .fetch_all(&self.pool)
            .await?;
        let existing_columns: Vec<String> = columns
            .iter()
            .filter_map(|row| row.try_get::<String, _>("name").ok())
            .collect();

        for column in [
            "transcribe_expires_at",
            "translate_expires_at",
            "summarize_expires_at",
            "caveman_expires_at",
        ] {
            if !existing_columns.iter().any(|existing| existing == column) {
                match column {
                    "transcribe_expires_at" => {
                        sqlx::query("ALTER TABLE cache ADD COLUMN transcribe_expires_at INTEGER")
                            .execute(&self.pool)
                            .await?;
                    }
                    "translate_expires_at" => {
                        sqlx::query("ALTER TABLE cache ADD COLUMN translate_expires_at INTEGER")
                            .execute(&self.pool)
                            .await?;
                    }
                    "summarize_expires_at" => {
                        sqlx::query("ALTER TABLE cache ADD COLUMN summarize_expires_at INTEGER")
                            .execute(&self.pool)
                            .await?;
                    }
                    "caveman_expires_at" => {
                        sqlx::query("ALTER TABLE cache ADD COLUMN caveman_expires_at INTEGER")
                            .execute(&self.pool)
                            .await?;
                    }
                    _ => unreachable!(),
                }
            }
        }

        Ok(())
    }

    async fn migrate_cache_expiration_columns(&self) -> Result<(), sqlx::Error> {
        sqlx::query(
            r#"
            UPDATE cache
            SET
                transcribe_expires_at = CASE
                    WHEN transcribe IS NOT NULL AND transcribe_expires_at IS NULL THEN expires_at
                    ELSE transcribe_expires_at
                END,
                translate_expires_at = CASE
                    WHEN translate IS NOT NULL AND translate_expires_at IS NULL THEN expires_at
                    ELSE translate_expires_at
                END,
                summarize_expires_at = CASE
                    WHEN summarize IS NOT NULL AND summarize_expires_at IS NULL THEN min(expires_at, ?)
                    ELSE summarize_expires_at
                END,
                caveman_expires_at = CASE
                    WHEN caveman IS NOT NULL AND caveman_expires_at IS NULL THEN min(expires_at, ?)
                    ELSE caveman_expires_at
                END
            "#,
        )
        .bind(current_timestamp() + (SUMMARY_CACHE_EXPIRATION_DAYS * 24 * 3600))
        .bind(current_timestamp() + (SUMMARY_CACHE_EXPIRATION_DAYS * 24 * 3600))
        .execute(&self.pool)
        .await?;

        Ok(())
    }

    pub async fn check_and_record_usage(
        &self,
        user_id: UserId,
        rate_limit_per_minute: u32,
        rate_limit_per_hour: u32,
    ) -> Result<bool, sqlx::Error> {
        let mut conn = self.pool.acquire().await?;
        sqlx::query("BEGIN IMMEDIATE").execute(&mut *conn).await?;

        let now = current_timestamp();
        let one_minute_ago = now - 60;
        let one_hour_ago = now - 3600;
        let user_id_i64 = user_id.0 as i64;

        let result = async {
            let minute_count: i64 = sqlx::query_scalar(
                "SELECT COUNT(*) as count FROM rate_limits WHERE user_id = ? AND timestamp > ?",
            )
            .bind(user_id_i64)
            .bind(one_minute_ago)
            .fetch_one(&mut *conn)
            .await?;

            if minute_count >= rate_limit_per_minute as i64 {
                warn!(
                    "Rate limit (per minute) exceeded for user {}: {} messages in last minute",
                    user_id, minute_count
                );
                return Ok(false);
            }

            let hour_count: i64 = sqlx::query_scalar(
                "SELECT COUNT(*) as count FROM rate_limits WHERE user_id = ? AND timestamp > ?",
            )
            .bind(user_id_i64)
            .bind(one_hour_ago)
            .fetch_one(&mut *conn)
            .await?;

            if hour_count >= rate_limit_per_hour as i64 {
                warn!(
                    "Rate limit (per hour) exceeded for user {}: {} messages in last hour",
                    user_id, hour_count
                );
                return Ok(false);
            }

            sqlx::query("INSERT INTO rate_limits (user_id, timestamp) VALUES (?, ?)")
                .bind(user_id_i64)
                .bind(now)
                .execute(&mut *conn)
                .await?;

            Ok(true)
        }
        .await;

        match result {
            Ok(is_allowed) => {
                sqlx::query("COMMIT").execute(&mut *conn).await?;
                Ok(is_allowed)
            }
            Err(err) => {
                let _ = sqlx::query("ROLLBACK").execute(&mut *conn).await;
                Err(err)
            }
        }
    }

    pub async fn cleanup_expired(&self) -> Result<(), sqlx::Error> {
        let now = current_timestamp();

        sqlx::query(
            r#"
            UPDATE cache
            SET
                transcribe = CASE WHEN transcribe_expires_at <= ? THEN NULL ELSE transcribe END,
                translate = CASE WHEN translate_expires_at <= ? THEN NULL ELSE translate END,
                summarize = CASE WHEN summarize_expires_at <= ? THEN NULL ELSE summarize END,
                caveman = CASE WHEN caveman_expires_at <= ? THEN NULL ELSE caveman END,
                transcribe_expires_at = CASE WHEN transcribe_expires_at <= ? THEN NULL ELSE transcribe_expires_at END,
                translate_expires_at = CASE WHEN translate_expires_at <= ? THEN NULL ELSE translate_expires_at END,
                summarize_expires_at = CASE WHEN summarize_expires_at <= ? THEN NULL ELSE summarize_expires_at END,
                caveman_expires_at = CASE WHEN caveman_expires_at <= ? THEN NULL ELSE caveman_expires_at END
            "#,
        )
        .bind(now)
        .bind(now)
        .bind(now)
        .bind(now)
        .bind(now)
        .bind(now)
        .bind(now)
        .bind(now)
        .execute(&self.pool)
        .await?;

        let deleted_cache = sqlx::query(
            r#"
            DELETE FROM cache
            WHERE transcribe IS NULL
                AND translate IS NULL
                AND summarize IS NULL
                AND caveman IS NULL
            "#,
        )
        .execute(&self.pool)
        .await?
        .rows_affected();

        if deleted_cache > 0 {
            info!("Deleted {} expired cache entries", deleted_cache);
        }

        let cleanup_threshold = now - (RATE_LIMIT_CLEANUP_MINUTES * 60);
        let deleted_rate_limits = sqlx::query!(
            "DELETE FROM rate_limits WHERE timestamp <= ?",
            cleanup_threshold
        )
        .execute(&self.pool)
        .await?
        .rows_affected();

        if deleted_rate_limits > 0 {
            info!("Deleted {} old rate limit entries", deleted_rate_limits);
        }

        Ok(())
    }
}

fn cache_expiration_days(task_type: &TaskType) -> i64 {
    match task_type {
        TaskType::Summarize | TaskType::Caveman => SUMMARY_CACHE_EXPIRATION_DAYS,
        TaskType::Transcribe | TaskType::Translate => CACHE_EXPIRATION_DAYS,
    }
}

fn current_timestamp() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("Time went backwards")
        .as_secs() as i64
}
