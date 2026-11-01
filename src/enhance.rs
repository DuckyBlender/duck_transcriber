use crate::utils::truncate_for_log;
use log::{error, warn};
use std::env;
use std::time::Duration;
use tokio::sync::{Mutex, Semaphore, SemaphorePermit};

pub const REUSE_MODEL: &str = "NVIDIA RE-USE";
const DEFAULT_REUSE_URL: &str = "http://host.docker.internal:8083/v1/audio/enhance";
// Covers one recording, since the bot sends them one at a time: a 2 minute
// recording takes about 76 seconds on the shared GPU.
const REUSE_TIMEOUT: Duration = Duration::from_secs(300);
// Recordings that may wait while another one is being enhanced. More would
// wait minutes, so they are turned away instead.
const MAX_WAITING: usize = 2;

// The RE-USE server enhances one recording at a time, so the bot sends it one
// at a time, and holds the rest here, where they don't count against the
// timeout.
static RUNNING: Mutex<()> = Mutex::const_new(());
static QUEUE: Semaphore = Semaphore::const_new(1 + MAX_WAITING);

/// A place in the queue for RE-USE, held until the recording is enhanced.
pub struct Slot(#[allow(dead_code)] SemaphorePermit<'static>);

/// Takes a place in the queue, or returns `None` when it is full.
pub fn queue() -> Option<Slot> {
    let slot = QUEUE.try_acquire().ok().map(Slot);
    if slot.is_none() {
        warn!("The RE-USE queue is full");
    }
    slot
}

/// Cleans up 16 kHz mono WAV (noise, echo, bad microphones) with the local
/// RE-USE server, and returns the result as WAV at the same sample rate. Waits
/// for the recordings ahead of it in the queue first.
pub async fn enhance(_slot: &Slot, wav: Vec<u8>) -> Result<Vec<u8>, String> {
    let _running = RUNNING.lock().await;
    let url = env::var("REUSE_URL").unwrap_or_else(|_| DEFAULT_REUSE_URL.to_string());
    let res = reqwest::Client::new()
        .post(url)
        .timeout(REUSE_TIMEOUT)
        .header(reqwest::header::CONTENT_TYPE, "audio/wav")
        .body(wav)
        .send()
        .await
        .map_err(|err| {
            error!("Failed to send request to RE-USE: {err}");
            format!("Failed to send request to RE-USE: {err}")
        })?;

    let status = res.status();
    let body = res.bytes().await.map_err(|err| {
        error!("Failed to read RE-USE response body ({status}): {err}");
        format!("Failed to read RE-USE response body: {err}")
    })?;
    if !status.is_success() {
        error!(
            "RE-USE returned {status}: {}",
            truncate_for_log(&String::from_utf8_lossy(&body))
        );
        return Err(format!("RE-USE error: {status}"));
    }
    Ok(body.to_vec())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn turns_requests_away_when_the_queue_is_full() {
        let slots: Vec<_> = (0..1 + MAX_WAITING).map(|_| queue().unwrap()).collect();
        assert!(queue().is_none());
        drop(slots);
        assert!(queue().is_some());
    }

    /// Needs ffmpeg and a running RE-USE server:
    /// `REUSE_URL=... REUSE_TEST_AUDIO=clip.ogg cargo test -- --ignored`
    #[tokio::test]
    #[ignore]
    async fn enhances_with_a_running_server() {
        let path = env::var("REUSE_TEST_AUDIO").expect("REUSE_TEST_AUDIO not set");
        let audio = std::fs::read(path).unwrap();
        let wav = crate::media::to_wav(audio, 1800).await.unwrap();
        let slot = queue().unwrap();
        let enhanced = enhance(&slot, wav.bytes).await.unwrap();
        let voice = crate::media::to_voice(enhanced).await.unwrap();
        assert!(voice.starts_with(b"OggS"));
    }
}
