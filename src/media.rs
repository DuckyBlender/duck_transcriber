use crate::transcribe::truncate_for_log;
use log::{error, warn};
use std::env;
use std::path::PathBuf;
use std::process::Stdio;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;
use tokio::process::Command;

const FFPROBE_TIMEOUT: Duration = Duration::from_secs(30);

static NEXT_TEMP_ID: AtomicU64 = AtomicU64::new(0);

/// Measures the length of the audio in seconds with ffprobe. Returns `None`
/// when ffprobe can't tell.
pub async fn probe_duration(buffer: &[u8]) -> Option<u32> {
    // Read from a file rather than stdin so ffprobe can seek to the end of the
    // file, where ogg keeps the final timestamp and mp4 often keeps its index.
    let input = TempFile::new();
    if let Err(err) = tokio::fs::write(&input.0, buffer).await {
        error!("Failed to write audio for ffprobe: {err}");
        return None;
    }

    let output = Command::new("ffprobe")
        .args([
            "-v",
            "error",
            "-show_entries",
            "format=duration",
            "-of",
            "csv=p=0",
        ])
        .arg(&input.0)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true)
        .output();

    let output = match tokio::time::timeout(FFPROBE_TIMEOUT, output).await {
        Ok(Ok(output)) => output,
        Ok(Err(err)) => {
            error!("Failed to run ffprobe: {err}");
            return None;
        }
        Err(_) => {
            error!("ffprobe timed out measuring audio");
            return None;
        }
    };

    let stdout = String::from_utf8_lossy(&output.stdout);
    let seconds = parse_duration(&stdout);
    if !output.status.success() || seconds.is_none() {
        warn!(
            "ffprobe couldn't measure the audio ({}): {} {}",
            output.status,
            stdout.trim(),
            truncate_for_log(&String::from_utf8_lossy(&output.stderr))
        );
    }
    seconds
}

/// Parses ffprobe's `format=duration` output, such as `3725.120000` or `N/A`.
fn parse_duration(stdout: &str) -> Option<u32> {
    let seconds: f64 = stdout.trim().parse().ok()?;
    (seconds.is_finite() && seconds >= 0.0).then(|| seconds.round() as u32)
}

/// A temporary file that is deleted when dropped, so a failed conversion never
/// leaves user audio behind.
pub struct TempFile(pub PathBuf);

impl TempFile {
    pub fn new() -> Self {
        let id = NEXT_TEMP_ID.fetch_add(1, Ordering::Relaxed);
        Self(env::temp_dir().join(format!("duck-audio-{}-{id}", std::process::id())))
    }
}

impl Drop for TempFile {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.0);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_ffprobe_durations() {
        assert_eq!(parse_duration("3725.120000\n"), Some(3725));
        assert_eq!(parse_duration("0.600000"), Some(1));
        assert_eq!(parse_duration("N/A\n"), None);
        assert_eq!(parse_duration(""), None);
    }

    #[test]
    fn temp_file_is_removed_on_drop() {
        let file = TempFile::new();
        std::fs::write(&file.0, b"audio").unwrap();
        let path = file.0.clone();
        drop(file);
        assert!(!path.exists());
    }

    /// Needs ffmpeg on the PATH.
    #[tokio::test]
    #[ignore]
    async fn measures_audio_with_ffprobe() {
        let path = env::temp_dir().join(format!("duck-probe-test-{}.ogg", std::process::id()));
        let status = std::process::Command::new("ffmpeg")
            .args(["-nostdin", "-loglevel", "error", "-y", "-f", "lavfi"])
            .args(["-i", "sine=frequency=440:duration=95", "-c:a", "libopus"])
            .arg(&path)
            .status()
            .unwrap();
        assert!(status.success());
        let audio = std::fs::read(&path).unwrap();
        std::fs::remove_file(&path).unwrap();

        assert_eq!(probe_duration(&audio).await, Some(95));
        assert_eq!(probe_duration(b"not audio").await, None);
    }
}
