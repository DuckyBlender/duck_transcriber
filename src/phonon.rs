use crate::media::wav_part;
use crate::utils::post_json;
use serde::Deserialize;
use std::env;
use std::time::Duration;

pub const PHONON_MODEL: &str = "Phonon-2.5 (Parakeet TDT 0.6B v3)";
const DEFAULT_PHONON_URL: &str = "http://host.docker.internal:8082/v1/audio/transcriptions";
// The Phonon server decodes one request at a time on the CPU, at roughly 35x
// realtime, so a 30 minute recording takes about a minute.
const PHONON_TIMEOUT: Duration = Duration::from_secs(300);

/// Transcribes 16 kHz mono WAV with the local Phonon-2.5 server. An empty
/// string means the audio has no speech.
pub async fn transcribe(wav: Vec<u8>) -> Result<String, String> {
    let url = env::var("PHONON_URL").unwrap_or_else(|_| DEFAULT_PHONON_URL.to_string());
    let form = reqwest::multipart::Form::new().part("file", wav_part(wav));
    let request = reqwest::Client::new()
        .post(url)
        .timeout(PHONON_TIMEOUT)
        .multipart(form);

    let res: PhononResponse = post_json("Phonon", request).await?;
    Ok(res.text)
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
        let res: PhononResponse = serde_json::from_str(r#"{"text": "Hello."}"#).unwrap();
        assert_eq!(res.text, "Hello.");
        let res: PhononResponse = serde_json::from_str("{}").unwrap();
        assert_eq!(res.text, "");
    }

    /// Needs ffmpeg and a running Phonon server:
    /// `PHONON_URL=... PHONON_TEST_AUDIO=clip.ogg cargo test -- --ignored`
    #[tokio::test]
    #[ignore]
    async fn transcribes_with_a_running_server() {
        let path = env::var("PHONON_TEST_AUDIO").expect("PHONON_TEST_AUDIO not set");
        let audio = std::fs::read(path).unwrap();
        let wav = crate::media::to_wav(audio, 1800).await.unwrap();
        let text = transcribe(wav.bytes).await.unwrap();
        println!("{text}");
        assert!(!text.trim().is_empty());
    }
}
