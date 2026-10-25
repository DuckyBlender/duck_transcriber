use crate::media::TempFile;
use crate::transcribe::truncate_for_log;
use crate::types::TranscriptionError;
use log::{error, info};
use serde::Deserialize;
use std::env;
use std::process::Stdio;
use std::time::Duration;
use tokio::process::Command;

pub const PHONON_MODEL: &str = "Phonon-2.5 (Parakeet TDT 0.6B v3)";
const DEFAULT_PHONON_URL: &str = "http://host.docker.internal:8082/v1/audio/transcriptions";
// The Phonon server decodes one request at a time on the CPU, at roughly 35x
// realtime, so a 30 minute recording takes about a minute.
const PHONON_TIMEOUT: Duration = Duration::from_secs(300);
const FFMPEG_TIMEOUT: Duration = Duration::from_secs(60);

/// Transcribes audio with the local Phonon-2.5 server. Returns `None` when the
/// audio has no speech.
pub async fn transcribe(buffer: Vec<u8>) -> Result<Option<String>, TranscriptionError> {
    let url = env::var("PHONON_URL").unwrap_or_else(|_| DEFAULT_PHONON_URL.to_string());
    info!("Transcribing with {PHONON_MODEL} at {url}");

    // The server only reads wav, flac, ogg and mp3, so video notes and m4a
    // files are converted first. Everything goes through ffmpeg so every
    // upload is the 16 kHz mono audio the model wants.
    let wav = to_wav(buffer).await?;

    let part = reqwest::multipart::Part::bytes(wav)
        .file_name("audio.wav")
        .mime_str("audio/wav")
        .map_err(|e| {
            error!("Failed to build Phonon upload: {e}");
            TranscriptionError::Parse("Invalid MIME type".to_string())
        })?;
    let form = reqwest::multipart::Form::new().part("file", part);

    let res = reqwest::Client::new()
        .post(url)
        .timeout(PHONON_TIMEOUT)
        .multipart(form)
        .send()
        .await
        .map_err(|err| {
            error!("Failed to send request to Phonon: {err}");
            TranscriptionError::Network(format!("Failed to send request to Phonon: {err}"))
        })?;

    let status = res.status();
    let body = res.text().await.map_err(|err| {
        error!("Failed to read Phonon response body ({status}): {err}");
        TranscriptionError::Network(format!("Failed to read Phonon response body: {err}"))
    })?;

    if !status.is_success() {
        error!("Phonon returned {status}: {}", truncate_for_log(&body));
        return Err(TranscriptionError::Api(format!("Phonon error: {status}")));
    }

    let text = parse_response(&body)?;
    if text.trim().is_empty() {
        return Ok(None);
    }

    Ok(Some(text))
}

fn parse_response(body: &str) -> Result<String, TranscriptionError> {
    serde_json::from_str::<PhononResponse>(body)
        .map(|res| res.text)
        .map_err(|err| {
            error!(
                "Failed to parse Phonon response: {err}; body: {}",
                truncate_for_log(body)
            );
            TranscriptionError::Parse("Failed to parse Phonon response".to_string())
        })
}

/// Converts any audio or video ffmpeg can read into 16 kHz mono WAV.
async fn to_wav(buffer: Vec<u8>) -> Result<Vec<u8>, TranscriptionError> {
    // ffmpeg reads the input from a file rather than stdin because mp4 files
    // often keep their index at the end, which a pipe can't seek back to.
    let input = TempFile::new();
    tokio::fs::write(&input.0, buffer).await.map_err(|err| {
        error!("Failed to write audio for ffmpeg: {err}");
        TranscriptionError::Parse("Failed to prepare audio".to_string())
    })?;

    let output = Command::new("ffmpeg")
        .args(["-nostdin", "-loglevel", "error", "-i"])
        .arg(&input.0)
        .args(["-vn", "-ac", "1", "-ar", "16000", "-f", "wav", "pipe:1"])
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true)
        .output();

    let output = tokio::time::timeout(FFMPEG_TIMEOUT, output)
        .await
        .map_err(|_| {
            error!("ffmpeg timed out converting audio");
            TranscriptionError::Parse("Audio conversion timed out".to_string())
        })?
        .map_err(|err| {
            error!("Failed to run ffmpeg: {err}");
            TranscriptionError::Parse("Failed to run ffmpeg".to_string())
        })?;

    if !output.status.success() {
        error!(
            "ffmpeg failed ({}): {}",
            output.status,
            truncate_for_log(&String::from_utf8_lossy(&output.stderr))
        );
        return Err(TranscriptionError::Parse(
            "Failed to convert audio".to_string(),
        ));
    }

    Ok(output.stdout)
}

/// The `json` response of the OpenAI-compatible transcription endpoint.
#[derive(Debug, Deserialize)]
struct PhononResponse {
    #[serde(default)]
    text: String,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_text() {
        assert_eq!(parse_response(r#"{"text": "Hello."}"#).unwrap(), "Hello.");
        assert_eq!(parse_response("{}").unwrap(), "");
    }

    #[test]
    fn rejects_error_bodies() {
        assert!(parse_response("<html>502</html>").is_err());
    }

    /// Needs ffmpeg and a running Phonon server:
    /// `PHONON_URL=... PHONON_TEST_AUDIO=clip.ogg cargo test -- --ignored`
    #[tokio::test]
    #[ignore]
    async fn transcribes_with_a_running_server() {
        let path = env::var("PHONON_TEST_AUDIO").expect("PHONON_TEST_AUDIO not set");
        let audio = std::fs::read(path).unwrap();
        let text = transcribe(audio).await.unwrap().unwrap();
        println!("{text}");
        assert!(!text.trim().is_empty());
    }
}
