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
// LLM usage has a daily limit, so it is kept for a full day.
const LLM_USAGE_CLEANUP_HOURS: i64 = 24;

type DatabaseResult<T> = Result<T, Infallible>;

pub struct Database {
    state: Mutex<DatabaseState>,
}

#[derive(Default)]
struct DatabaseState {
    cache: HashMap<String, CacheEntry>,
    rate_limits: Vec<RateLimitEntry>,
    llm_usage: Vec<RateLimitEntry>,
}

#[derive(Default)]
struct CacheEntry {
    transcribe: Option<CachedValue>,
    phonon: Option<CachedValue>,
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

/// Limits on llama.cpp requests (translations and summaries), which are far
/// slower than whisper.cpp and share one GPU.
pub struct LlmLimits {
    pub per_user_hour: usize,
    pub per_user_day: usize,
    pub global_hour: usize,
}

#[derive(Debug, PartialEq)]
pub enum LlmUsage {
    Allowed,
    UserLimited,
    GlobalLimited,
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
            TaskType::Phonon => entry.phonon.as_ref(),
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
            TaskType::Phonon => entry.phonon = Some(cached_value),
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

    /// Records one llama.cpp request for the user if it fits within `limits`.
    pub async fn check_and_record_llm_usage(
        &self,
        user_id: UserId,
        limits: &LlmLimits,
    ) -> DatabaseResult<LlmUsage> {
        let now = current_timestamp();
        let one_hour_ago = now - 3600;
        let one_day_ago = now - 24 * 3600;
        let mut state = self.state.lock().expect("database mutex poisoned");

        let user_entries = || {
            state
                .llm_usage
                .iter()
                .filter(|entry| entry.user_id == user_id)
        };
        let user_hour = user_entries()
            .filter(|entry| entry.timestamp > one_hour_ago)
            .count();
        let user_day = user_entries()
            .filter(|entry| entry.timestamp > one_day_ago)
            .count();
        if user_hour >= limits.per_user_hour || user_day >= limits.per_user_day {
            warn!(
                "LLM limit exceeded for user {user_id}: {user_hour} requests in the last hour, {user_day} in the last day"
            );
            return Ok(LlmUsage::UserLimited);
        }

        let global_hour = state
            .llm_usage
            .iter()
            .filter(|entry| entry.timestamp > one_hour_ago)
            .count();
        if global_hour >= limits.global_hour {
            warn!("Global LLM limit reached: {global_hour} requests in the last hour");
            return Ok(LlmUsage::GlobalLimited);
        }

        state.llm_usage.push(RateLimitEntry {
            user_id,
            timestamp: now,
        });
        Ok(LlmUsage::Allowed)
    }

    pub async fn cleanup_expired(&self) -> DatabaseResult<()> {
        let now = current_timestamp();
        let mut state = self.state.lock().expect("database mutex poisoned");

        let cache_before = state.cache.len();
        for entry in state.cache.values_mut() {
            clear_expired(&mut entry.transcribe, now);
            clear_expired(&mut entry.phonon, now);
            clear_expired(&mut entry.translate, now);
            clear_expired(&mut entry.summarize, now);
            clear_expired(&mut entry.caveman, now);
        }
        state.cache.retain(|_, entry| {
            entry.transcribe.is_some()
                || entry.phonon.is_some()
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

        let llm_threshold = now - (LLM_USAGE_CLEANUP_HOURS * 3600);
        state
            .llm_usage
            .retain(|entry| entry.timestamp > llm_threshold);

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
        TaskType::Transcribe | TaskType::Phonon | TaskType::Translate => {
            CACHE_EXPIRATION_DAYS * 24 * 3600
        }
    }
}

fn current_timestamp() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("Time went backwards")
        .as_secs() as i64
}

#[cfg(test)]
mod tests {
    use super::*;

    const LIMITS: LlmLimits = LlmLimits {
        per_user_hour: 2,
        per_user_day: 3,
        global_hour: 4,
    };

    async fn request(db: &Database, user: u64) -> LlmUsage {
        db.check_and_record_llm_usage(UserId(user), &LIMITS)
            .await
            .unwrap()
    }

    #[tokio::test]
    async fn limits_each_user_per_hour() {
        let db = Database::connect().await.unwrap();
        assert_eq!(request(&db, 1).await, LlmUsage::Allowed);
        assert_eq!(request(&db, 1).await, LlmUsage::Allowed);
        assert_eq!(request(&db, 1).await, LlmUsage::UserLimited);
        assert_eq!(request(&db, 2).await, LlmUsage::Allowed);
    }

    #[tokio::test]
    async fn limits_each_user_per_day() {
        let db = Database::connect().await.unwrap();
        let two_hours_ago = current_timestamp() - 2 * 3600;
        db.state
            .lock()
            .unwrap()
            .llm_usage
            .extend((0..3).map(|_| RateLimitEntry {
                user_id: UserId(1),
                timestamp: two_hours_ago,
            }));
        assert_eq!(request(&db, 1).await, LlmUsage::UserLimited);
        assert_eq!(request(&db, 2).await, LlmUsage::Allowed);
    }

    #[tokio::test]
    async fn limits_everyone_together_per_hour() {
        let db = Database::connect().await.unwrap();
        for user in 1..=4 {
            assert_eq!(request(&db, user).await, LlmUsage::Allowed);
        }
        assert_eq!(request(&db, 5).await, LlmUsage::GlobalLimited);
    }

    #[tokio::test]
    async fn refused_requests_are_not_recorded() {
        let db = Database::connect().await.unwrap();
        request(&db, 1).await;
        request(&db, 1).await;
        assert_eq!(request(&db, 1).await, LlmUsage::UserLimited);
        assert_eq!(db.state.lock().unwrap().llm_usage.len(), 2);
    }
}
