use crate::types::TaskType;
use log::{info, warn};
use std::collections::HashMap;
use std::convert::Infallible;
use std::sync::Mutex;
use std::time::{SystemTime, UNIX_EPOCH};
use teloxide::types::UserId;

const CACHE_EXPIRATION_DAYS: i64 = 7;
const SUMMARY_CACHE_EXPIRATION_HOURS: i64 = 1;
const RATE_LIMIT_CLEANUP_MINUTES: i64 = 120;

type DatabaseResult<T> = Result<T, Infallible>;

pub struct Database {
    state: Mutex<DatabaseState>,
}

#[derive(Default)]
struct DatabaseState {
    cache: HashMap<String, CacheEntry>,
    rate_limits: Vec<RateLimitEntry>,
}

#[derive(Default)]
struct CacheEntry {
    transcribe: Option<CachedValue>,
    translate: Option<CachedValue>,
    summarize: Option<CachedValue>,
    caveman: Option<CachedValue>,
}

struct CachedValue {
    text: String,
    expires_at: i64,
}

struct RateLimitEntry {
    user_id: UserId,
    timestamp: i64,
}

impl Database {
    pub async fn connect() -> DatabaseResult<Self> {
        info!("Connected to in-memory database");

        Ok(Database {
            state: Mutex::new(DatabaseState::default()),
        })
    }

    pub async fn init(&self) -> DatabaseResult<()> {
        info!("In-memory database initialized");
        self.cleanup_expired().await?;

        Ok(())
    }

    pub async fn get_cached(
        &self,
        unique_file_id: &str,
        task_type: &TaskType,
    ) -> DatabaseResult<Option<String>> {
        let now = current_timestamp();
        let state = self.state.lock().expect("database mutex poisoned");

        let Some(entry) = state.cache.get(unique_file_id) else {
            return Ok(None);
        };

        let cached_value = match task_type {
            TaskType::Transcribe => entry.transcribe.as_ref(),
            TaskType::Translate => entry.translate.as_ref(),
            TaskType::Summarize => entry.summarize.as_ref(),
            TaskType::Caveman => entry.caveman.as_ref(),
        };

        Ok(cached_value
            .filter(|value| value.expires_at > now)
            .map(|value| value.text.clone()))
    }

    pub async fn set_cached(
        &self,
        unique_file_id: &str,
        task_type: &TaskType,
        text: &str,
    ) -> DatabaseResult<()> {
        let expires_at = current_timestamp() + cache_expiration_seconds(task_type);
        let mut state = self.state.lock().expect("database mutex poisoned");
        let entry = state.cache.entry(unique_file_id.to_string()).or_default();
        let cached_value = CachedValue {
            text: text.to_string(),
            expires_at,
        };

        match task_type {
            TaskType::Transcribe => entry.transcribe = Some(cached_value),
            TaskType::Translate => entry.translate = Some(cached_value),
            TaskType::Summarize => entry.summarize = Some(cached_value),
            TaskType::Caveman => entry.caveman = Some(cached_value),
        }

        Ok(())
    }

    pub async fn check_and_record_usage(
        &self,
        user_id: UserId,
        rate_limit_per_minute: u32,
        rate_limit_per_hour: u32,
    ) -> DatabaseResult<bool> {
        let now = current_timestamp();
        let one_minute_ago = now - 60;
        let one_hour_ago = now - 3600;
        let mut state = self.state.lock().expect("database mutex poisoned");

        let minute_count = state
            .rate_limits
            .iter()
            .filter(|entry| entry.user_id == user_id && entry.timestamp > one_minute_ago)
            .count();

        if minute_count >= rate_limit_per_minute as usize {
            warn!(
                "Rate limit (per minute) exceeded for user {}: {} messages in last minute",
                user_id, minute_count
            );
            return Ok(false);
        }

        let hour_count = state
            .rate_limits
            .iter()
            .filter(|entry| entry.user_id == user_id && entry.timestamp > one_hour_ago)
            .count();

        if hour_count >= rate_limit_per_hour as usize {
            warn!(
                "Rate limit (per hour) exceeded for user {}: {} messages in last hour",
                user_id, hour_count
            );
            return Ok(false);
        }

        state.rate_limits.push(RateLimitEntry {
            user_id,
            timestamp: now,
        });

        Ok(true)
    }

    pub async fn cleanup_expired(&self) -> DatabaseResult<()> {
        let now = current_timestamp();
        let mut state = self.state.lock().expect("database mutex poisoned");

        let cache_before = state.cache.len();
        for entry in state.cache.values_mut() {
            clear_expired(&mut entry.transcribe, now);
            clear_expired(&mut entry.translate, now);
            clear_expired(&mut entry.summarize, now);
            clear_expired(&mut entry.caveman, now);
        }
        state.cache.retain(|_, entry| {
            entry.transcribe.is_some()
                || entry.translate.is_some()
                || entry.summarize.is_some()
                || entry.caveman.is_some()
        });
        let deleted_cache = cache_before - state.cache.len();

        if deleted_cache > 0 {
            info!("Deleted {} expired cache entries", deleted_cache);
        }

        let cleanup_threshold = now - (RATE_LIMIT_CLEANUP_MINUTES * 60);
        let rate_limits_before = state.rate_limits.len();
        state
            .rate_limits
            .retain(|entry| entry.timestamp > cleanup_threshold);
        let deleted_rate_limits = rate_limits_before - state.rate_limits.len();

        if deleted_rate_limits > 0 {
            info!("Deleted {} old rate limit entries", deleted_rate_limits);
        }

        Ok(())
    }
}

fn clear_expired(value: &mut Option<CachedValue>, now: i64) {
    if value
        .as_ref()
        .is_some_and(|cached_value| cached_value.expires_at <= now)
    {
        *value = None;
    }
}

fn cache_expiration_seconds(task_type: &TaskType) -> i64 {
    match task_type {
        TaskType::Summarize | TaskType::Caveman => SUMMARY_CACHE_EXPIRATION_HOURS * 3600,
        TaskType::Transcribe | TaskType::Translate => CACHE_EXPIRATION_DAYS * 24 * 3600,
    }
}

fn current_timestamp() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("Time went backwards")
        .as_secs() as i64
}
