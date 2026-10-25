use crate::types::TaskType;
use log::{info, warn};
use std::collections::HashMap;
use std::sync::{Mutex, MutexGuard};
use std::time::{SystemTime, UNIX_EPOCH};
use teloxide::types::{FileUniqueId, UserId};

const CACHE_EXPIRATION_DAYS: i64 = 7;
const SUMMARY_CACHE_EXPIRATION_HOURS: i64 = 1;
const RATE_LIMIT_CLEANUP_MINUTES: i64 = 120;
// LLM usage has a daily limit, so it is kept for a full day.
const LLM_USAGE_CLEANUP_HOURS: i64 = 24;

/// The bot's in-memory state: cached results and rate limit records. All of it
/// is lost on restart.
#[derive(Default)]
pub struct Database {
    state: Mutex<DatabaseState>,
}

#[derive(Default)]
struct DatabaseState {
    // One result per file and task, so /phonon, translations and summaries never
    // replace the whisper.cpp transcript. The i64 is when it expires.
    cache: HashMap<(FileUniqueId, TaskType), (Cached, i64)>,
    rate_limits: Vec<RateLimitEntry>,
    llm_usage: Vec<RateLimitEntry>,
}

/// A cached result and the decoded length in seconds of the recording it came
/// from. /translate's limit reads it when it starts from a cached transcript.
#[derive(Clone)]
pub struct Cached {
    pub text: String,
    pub seconds: u32,
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
    pub fn get_cached(&self, file: &FileUniqueId, task: TaskType) -> Option<Cached> {
        let now = current_timestamp();
        self.lock()
            .cache
            .get(&(file.clone(), task))
            .filter(|(_, expires_at)| *expires_at > now)
            .map(|(cached, _)| cached.clone())
    }

    pub fn set_cached(&self, file: &FileUniqueId, task: TaskType, cached: Cached) {
        let expires_at = current_timestamp() + cache_expiration_seconds(task);
        self.lock()
            .cache
            .insert((file.clone(), task), (cached, expires_at));
    }

    /// Records one audio request for the user if it fits within the limits.
    pub fn check_and_record_usage(
        &self,
        user_id: UserId,
        per_minute: usize,
        per_hour: usize,
    ) -> bool {
        let now = current_timestamp();
        let mut state = self.lock();

        let minute = count(&state.rate_limits, Some(user_id), now - 60);
        let hour = count(&state.rate_limits, Some(user_id), now - 3600);
        if minute >= per_minute || hour >= per_hour {
            warn!(
                "Rate limit exceeded for user {user_id}: {minute} requests in the last minute, {hour} in the last hour"
            );
            return false;
        }

        state.rate_limits.push(RateLimitEntry {
            user_id,
            timestamp: now,
        });
        true
    }

    /// Records one llama.cpp request for the user if it fits within `limits`.
    pub fn check_and_record_llm_usage(&self, user_id: UserId, limits: &LlmLimits) -> LlmUsage {
        let now = current_timestamp();
        let mut state = self.lock();

        let user_hour = count(&state.llm_usage, Some(user_id), now - 3600);
        let user_day = count(&state.llm_usage, Some(user_id), now - 24 * 3600);
        if user_hour >= limits.per_user_hour || user_day >= limits.per_user_day {
            warn!(
                "LLM limit exceeded for user {user_id}: {user_hour} requests in the last hour, {user_day} in the last day"
            );
            return LlmUsage::UserLimited;
        }

        let global_hour = count(&state.llm_usage, None, now - 3600);
        if global_hour >= limits.global_hour {
            warn!("Global LLM limit reached: {global_hour} requests in the last hour");
            return LlmUsage::GlobalLimited;
        }

        state.llm_usage.push(RateLimitEntry {
            user_id,
            timestamp: now,
        });
        LlmUsage::Allowed
    }

    pub fn cleanup_expired(&self) {
        let now = current_timestamp();
        let mut state = self.lock();

        let cache_before = state.cache.len();
        state.cache.retain(|_, (_, expires_at)| *expires_at > now);
        let deleted_cache = cache_before - state.cache.len();
        if deleted_cache > 0 {
            info!("Deleted {deleted_cache} expired cache entries");
        }

        let rate_limits_before = state.rate_limits.len();
        let rate_limit_threshold = now - RATE_LIMIT_CLEANUP_MINUTES * 60;
        state
            .rate_limits
            .retain(|entry| entry.timestamp > rate_limit_threshold);
        let deleted_rate_limits = rate_limits_before - state.rate_limits.len();
        if deleted_rate_limits > 0 {
            info!("Deleted {deleted_rate_limits} old rate limit entries");
        }

        let llm_threshold = now - LLM_USAGE_CLEANUP_HOURS * 3600;
        state
            .llm_usage
            .retain(|entry| entry.timestamp > llm_threshold);
    }

    fn lock(&self) -> MutexGuard<'_, DatabaseState> {
        self.state.lock().expect("database mutex poisoned")
    }
}

/// How many entries since `since` came from `user_id`, or from anyone for `None`.
fn count(entries: &[RateLimitEntry], user_id: Option<UserId>, since: i64) -> usize {
    entries
        .iter()
        .filter(|entry| entry.timestamp > since && user_id.is_none_or(|id| entry.user_id == id))
        .count()
}

fn cache_expiration_seconds(task: TaskType) -> i64 {
    match task {
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

    fn request(db: &Database, user: u64) -> LlmUsage {
        db.check_and_record_llm_usage(UserId(user), &LIMITS)
    }

    #[test]
    fn limits_each_user_per_hour() {
        let db = Database::default();
        assert_eq!(request(&db, 1), LlmUsage::Allowed);
        assert_eq!(request(&db, 1), LlmUsage::Allowed);
        assert_eq!(request(&db, 1), LlmUsage::UserLimited);
        assert_eq!(request(&db, 2), LlmUsage::Allowed);
    }

    #[test]
    fn limits_each_user_per_day() {
        let db = Database::default();
        let two_hours_ago = current_timestamp() - 2 * 3600;
        db.lock().llm_usage.extend((0..3).map(|_| RateLimitEntry {
            user_id: UserId(1),
            timestamp: two_hours_ago,
        }));
        assert_eq!(request(&db, 1), LlmUsage::UserLimited);
        assert_eq!(request(&db, 2), LlmUsage::Allowed);
    }

    #[test]
    fn limits_everyone_together_per_hour() {
        let db = Database::default();
        for user in 1..=4 {
            assert_eq!(request(&db, user), LlmUsage::Allowed);
        }
        assert_eq!(request(&db, 5), LlmUsage::GlobalLimited);
    }

    #[test]
    fn refused_requests_are_not_recorded() {
        let db = Database::default();
        request(&db, 1);
        request(&db, 1);
        assert_eq!(request(&db, 1), LlmUsage::UserLimited);
        assert_eq!(db.lock().llm_usage.len(), 2);
    }

    #[test]
    fn limits_audio_requests_per_minute() {
        let db = Database::default();
        assert!(db.check_and_record_usage(UserId(1), 2, 10));
        assert!(db.check_and_record_usage(UserId(1), 2, 10));
        assert!(!db.check_and_record_usage(UserId(1), 2, 10));
        assert!(db.check_and_record_usage(UserId(2), 2, 10));
    }

    #[test]
    fn caches_each_task_separately_until_it_expires() {
        let db = Database::default();
        let file = FileUniqueId("file".to_string());
        let cached = Cached {
            text: "a".to_string(),
            seconds: 60,
        };
        db.set_cached(&file, TaskType::Transcribe, cached);

        assert!(db.get_cached(&file, TaskType::Phonon).is_none());
        let hit = db.get_cached(&file, TaskType::Transcribe).unwrap();
        assert_eq!((hit.text.as_str(), hit.seconds), ("a", 60));

        for (_, expires_at) in db.lock().cache.values_mut() {
            *expires_at = 0;
        }
        assert!(db.get_cached(&file, TaskType::Transcribe).is_none());
        db.cleanup_expired();
        assert!(db.lock().cache.is_empty());
    }
}
