use crate::utils::truncate_for_log;
use log::error;
use reqwest::multipart::Part;
use std::env;
use std::path::PathBuf;
use std::process::Stdio;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;
use tokio::process::Command;

// 16 kHz mono 16-bit samples, which is what whisper.cpp, Phonon and RE-USE get.
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
    // -fs counts output bytes, so timestamps in the file can't fool it.
    let max_bytes = (max_seconds + 1) * WAV_BYTES_PER_SECOND;
    let max_bytes = max_bytes.to_string();
    let wav = ffmpeg(
        audio,
        INPUT_FORMATS,
        &[
            "-vn", "-ac", "1", "-ar", "16000", "-fs", &max_bytes, "-f", "wav",
        ],
    )
    .await?;
    Some(Wav {
        seconds: wav_seconds(wav.len()),
        bytes: wav,
    })
}

/// Encodes WAV from the RE-USE server as an Ogg Opus voice message, the only
/// format Telegram shows as one.
pub async fn to_voice(wav: Vec<u8>) -> Option<Vec<u8>> {
    ffmpeg(
        wav,
        "wav",
        &["-ac", "1", "-c:a", "libopus", "-b:a", "48k", "-f", "ogg"],
    )
    .await
}

/// Runs ffmpeg on `input`, which must be in one of `input_formats`, and returns
/// what it writes with `output_args`. Returns `None` after logging why it
/// couldn't.
async fn ffmpeg(input: Vec<u8>, input_formats: &str, output_args: &[&str]) -> Option<Vec<u8>> {
    // ffmpeg reads a file rather than stdin because mp4 files often keep their
    // index at the end, which a pipe can't seek back to.
    let file = TempFile::new();
    if let Err(err) = tokio::fs::write(&file.0, input).await {
        error!("Failed to write audio for ffmpeg: {err}");
        return None;
    }

    let output = Command::new("ffmpeg")
        .args(["-nostdin", "-loglevel", "error"])
        .args(["-format_whitelist", input_formats, "-i"])
        .arg(&file.0)
        .args(output_args)
        .arg("pipe:1")
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true)
        .output();

    match tokio::time::timeout(FFMPEG_TIMEOUT, output).await {
        Ok(Ok(output)) if output.status.success() => Some(output.stdout),
        Ok(Ok(output)) => {
            error!(
                "ffmpeg failed ({}): {}",
                output.status,
                truncate_for_log(&String::from_utf8_lossy(&output.stderr))
            );
            None
        }
        Ok(Err(err)) => {
            error!("Failed to run ffmpeg: {err}");
            None
        }
        Err(_) => {
            error!("ffmpeg timed out");
            None
        }
    }
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

        // What RE-USE sends back goes out as an Ogg Opus voice message.
        let wav = to_wav(
            encode(&["-i", "sine=duration=3", "-f", "wav", "pipe:1"]),
            60,
        )
        .await
        .unwrap();
        let voice = to_voice(wav.bytes).await.unwrap();
        assert!(voice.starts_with(b"OggS"));
        assert!(voice.windows(8).any(|w| w == b"OpusHead"));
        assert!(to_voice(b"not audio".to_vec()).await.is_none());
        // A playlist pointing at another file in the temp directory.
        let playlist = b"ffconcat version 1.0\nfile duck-audio-1-0\n".to_vec();
        assert!(to_wav(playlist, 1800).await.is_none());
    }
}
