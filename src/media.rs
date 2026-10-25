use crate::utils::truncate_for_log;
use log::error;
use reqwest::multipart::Part;
use std::env;
use std::path::PathBuf;
use std::process::Stdio;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;
use tokio::process::Command;

// 16 kHz mono 16-bit samples, which is what whisper.cpp and Phonon both want.
const WAV_BYTES_PER_SECOND: u32 = 16_000 * 2;
const FFMPEG_TIMEOUT: Duration = Duration::from_secs(60);
// The containers Telegram voice messages, video notes, videos and audio files
// come in. Anything else is refused, because playlist formats such as ffconcat
// can make ffmpeg read another user's recording from the temp directory.
const INPUT_FORMATS: &str = "ogg,mov,matroska,mp3,wav,flac,aac,aiff,caf,amr";

static NEXT_TEMP_ID: AtomicU64 = AtomicU64::new(0);

/// A recording decoded to 16 kHz mono WAV, and its length in seconds.
pub struct Wav {
    pub bytes: Vec<u8>,
    pub seconds: u32,
}

/// Decodes any audio or video ffmpeg can read into 16 kHz mono WAV and measures
/// it. Returns `None` after logging why it couldn't. The length is counted from
/// the decoded samples, because both Telegram's number and the file's own
/// header can be wrong. Decoding stops a second past `max_seconds`, so refusing
/// a longer recording costs no more than that and it still comes back too long.
pub async fn to_wav(audio: Vec<u8>, max_seconds: u32) -> Option<Wav> {
    // ffmpeg reads a file rather than stdin because mp4 files often keep their
    // index at the end, which a pipe can't seek back to.
    let input = TempFile::new();
    if let Err(err) = tokio::fs::write(&input.0, audio).await {
        error!("Failed to write audio for ffmpeg: {err}");
        return None;
    }

    // -fs counts output bytes, so timestamps in the file can't fool it.
    let max_bytes = (max_seconds + 1) * WAV_BYTES_PER_SECOND;
    let output = Command::new("ffmpeg")
        .args(["-nostdin", "-loglevel", "error"])
        .args(["-format_whitelist", INPUT_FORMATS, "-i"])
        .arg(&input.0)
        .args(["-vn", "-ac", "1", "-ar", "16000", "-fs"])
        .arg(max_bytes.to_string())
        .args(["-f", "wav", "pipe:1"])
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true)
        .output();

    let output = match tokio::time::timeout(FFMPEG_TIMEOUT, output).await {
        Ok(Ok(output)) if output.status.success() => output,
        Ok(Ok(output)) => {
            error!(
                "ffmpeg failed ({}): {}",
                output.status,
                truncate_for_log(&String::from_utf8_lossy(&output.stderr))
            );
            return None;
        }
        Ok(Err(err)) => {
            error!("Failed to run ffmpeg: {err}");
            return None;
        }
        Err(_) => {
            error!("ffmpeg timed out decoding audio");
            return None;
        }
    };

    Some(Wav {
        seconds: wav_seconds(output.stdout.len()),
        bytes: output.stdout,
    })
}

/// The length of a `to_wav` output. Its header is under 100 bytes, far less
/// than a second of samples.
fn wav_seconds(len: usize) -> u32 {
    (len / WAV_BYTES_PER_SECOND as usize) as u32
}

/// The decoded audio as the upload whisper.cpp and Phonon expect.
pub fn wav_part(wav: Vec<u8>) -> Part {
    Part::bytes(wav)
        .file_name("audio.wav")
        .mime_str("audio/wav")
        .expect("audio/wav is a valid MIME type")
}

/// A temporary file that is deleted when dropped, so a failed decode never
/// leaves user audio behind.
struct TempFile(PathBuf);

impl TempFile {
    fn new() -> Self {
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
    fn counts_seconds_from_samples() {
        assert_eq!(wav_seconds(78 + 95 * WAV_BYTES_PER_SECOND as usize), 95);
        assert_eq!(wav_seconds(78), 0);
    }

    #[test]
    fn temp_file_is_removed_on_drop() {
        let file = TempFile::new();
        std::fs::write(&file.0, b"audio").unwrap();
        let path = file.0.clone();
        drop(file);
        assert!(!path.exists());
    }

    /// Encodes test audio with ffmpeg into a pipe.
    fn encode(args: &[&str]) -> Vec<u8> {
        let output = std::process::Command::new("ffmpeg")
            .args(["-nostdin", "-loglevel", "error", "-f", "lavfi"])
            .args(args)
            .output()
            .unwrap();
        assert!(output.status.success());
        output.stdout
    }

    /// Needs ffmpeg on the PATH.
    #[tokio::test]
    #[ignore]
    async fn measures_decoded_audio() {
        let opus = encode(&[
            "-i",
            "sine=duration=95",
            "-c:a",
            "libopus",
            "-f",
            "ogg",
            "pipe:1",
        ]);
        assert_eq!(to_wav(opus.clone(), 1800).await.unwrap().seconds, 95);
        assert_eq!(to_wav(opus, 60).await.unwrap().seconds, 61);

        // An mp3 whose header claims 5 seconds of high bitrate audio, followed
        // by 11 minutes at a low bitrate: ffprobe would estimate the length from
        // the first bitrate and the file size, and get it badly wrong.
        let mut spoof = encode(&[
            "-i",
            "sine=duration=5",
            "-b:a",
            "320k",
            "-f",
            "mp3",
            "pipe:1",
        ]);
        spoof.extend(encode(&[
            "-i",
            "sine=duration=660",
            "-ar",
            "8000",
            "-b:a",
            "8k",
            "-write_xing",
            "0",
            "-f",
            "mp3",
            "pipe:1",
        ]));
        assert!(to_wav(spoof, 600).await.unwrap().seconds > 600);

        assert!(to_wav(b"not audio".to_vec(), 1800).await.is_none());
        // A playlist pointing at another file in the temp directory.
        let playlist = b"ffconcat version 1.0\nfile duck-audio-1-0\n".to_vec();
        assert!(to_wav(playlist, 1800).await.is_none());
    }
}
