use crate::types::TaskType;
use log::{info, warn};
use sqlx::sqlite::{SqliteConnectOptions, SqlitePoolOptions};
use sqlx::{Row, SqlitePool};
use std::env;
use std::str::FromStr;
use std::time::{SystemTime, UNIX_EPOCH};
use teloxide::types::UserId;

const CACHE_EXPIRATION_DAYS: i64 = 7;
const CACHE_EXPIRATION_SUMMARIZE_DAYS: i64 = 1;
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

        let task_type_str = match task_type {
            TaskType::Transcribe => "transcribe",
            TaskType::Translate => "translate",
            TaskType::Summarize | TaskType::Caveman => "summarize",
        };

        let row = sqlx::query(&format!(
            r#"
                SELECT {} FROM cache 
                WHERE unique_file_id = ? AND expires_at > ?
                "#,
            task_type_str
        ))
        .bind(unique_file_id)
        .bind(now)
        .fetch_optional(&self.pool)
        .await?;

        Ok(row.and_then(|r| r.get(0)))
    }

    pub async fn set_cached(
        &self,
        unique_file_id: &str,
        task_type: &TaskType,
        text: &str,
    ) -> Result<(), sqlx::Error> {
        let expires_days = match task_type {
            TaskType::Summarize | TaskType::Caveman => CACHE_EXPIRATION_SUMMARIZE_DAYS,
            _ => CACHE_EXPIRATION_DAYS,
        };

        let expires_at = current_timestamp() + (expires_days * 24 * 3600);

        let task_type_str = match task_type {
            TaskType::Transcribe => "transcribe",
            TaskType::Translate => "translate",
            TaskType::Summarize | TaskType::Caveman => "summarize",
        };

        let result = sqlx::query(&format!(
            r#"
                UPDATE cache 
                SET {} = ?, expires_at = ?
                WHERE unique_file_id = ?
                "#,
            task_type_str
        ))
        .bind(text)
        .bind(expires_at)
        .bind(unique_file_id)
        .execute(&self.pool)
        .await?;

        if result.rows_affected() == 0 {
            sqlx::query(
                r#"
                INSERT INTO cache (unique_file_id, expires_at)
                VALUES (?, ?)
                "#,
            )
            .bind(unique_file_id)
            .bind(expires_at)
            .execute(&self.pool)
            .await?;

            sqlx::query(&format!(
                r#"
                    UPDATE cache 
                    SET {} = ?
                    WHERE unique_file_id = ?
                    "#,
                task_type_str
            ))
            .bind(text)
            .bind(unique_file_id)
            .execute(&self.pool)
            .await?;
        }

        Ok(())
    }

    pub async fn check_rate_limit(
        &self,
        user_id: UserId,
        rate_limit_per_minute: u32,
        rate_limit_per_hour: u32,
    ) -> Result<bool, sqlx::Error> {
        let now = current_timestamp();
        let one_minute_ago = now - 60;
        let one_hour_ago = now - 3600;
        let user_id_i64 = user_id.0 as i64;

        let minute_count: i64 = sqlx::query_scalar!(
            "SELECT COUNT(*) as count FROM rate_limits WHERE user_id = ? AND timestamp > ?",
            user_id_i64,
            one_minute_ago
        )
        .fetch_one(&self.pool)
        .await?;

        if minute_count >= rate_limit_per_minute as i64 {
            warn!(
                "Rate limit (per minute) exceeded for user {}: {} messages in last minute",
                user_id, minute_count
            );
            return Ok(false);
        }

        let hour_count: i64 = sqlx::query_scalar!(
            "SELECT COUNT(*) as count FROM rate_limits WHERE user_id = ? AND timestamp > ?",
            user_id_i64,
            one_hour_ago
        )
        .fetch_one(&self.pool)
        .await?;

        if hour_count >= rate_limit_per_hour as i64 {
            warn!(
                "Rate limit (per hour) exceeded for user {}: {} messages in last hour",
                user_id, hour_count
            );
            return Ok(false);
        }

        Ok(true)
    }

    pub async fn record_usage(&self, user_id: UserId) -> Result<(), sqlx::Error> {
        let now = current_timestamp();
        let user_id_i64 = user_id.0 as i64;

        sqlx::query!(
            "INSERT INTO rate_limits (user_id, timestamp) VALUES (?, ?)",
            user_id_i64,
            now
        )
        .execute(&self.pool)
        .await?;

        Ok(())
    }

    pub async fn cleanup_expired(&self) -> Result<(), sqlx::Error> {
        let now = current_timestamp();

        let deleted_cache = sqlx::query!("DELETE FROM cache WHERE expires_at <= ?", now)
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

fn current_timestamp() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("Time went backwards")
        .as_secs() as i64
}
