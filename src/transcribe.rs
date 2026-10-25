use crate::media::wav_part;
use crate::utils::post_json;
use serde::Deserialize;
use std::env;

pub const WHISPER_MODEL: &str = "whisper.cpp large-v3-turbo";
const DEFAULT_WHISPER_URL: &str = "http://host.docker.internal:8080/inference";

/// Transcribes 16 kHz mono WAV with the local whisper.cpp server. An empty
/// string means the audio has no speech.
pub async fn transcribe(wav: Vec<u8>) -> Result<String, String> {
    let url = env::var("WHISPER_LOCAL_URL").unwrap_or_else(|_| DEFAULT_WHISPER_URL.to_string());
    let form = reqwest::multipart::Form::new()
        .text("response_format", "verbose_json")
        .text("temperature", "0.0")
        .text("temperature_inc", "0.2")
        .text("language", "auto")
        .text("no_language_probabilities", "true")
        .part("file", wav_part(wav));

    let res: WhisperResponse = post_json(
        "whisper.cpp",
        reqwest::Client::new().post(url).multipart(form),
    )
    .await?;
    Ok(whisper_output_text(res))
}

fn whisper_output_text(res: WhisperResponse) -> String {
    if res.segments.is_empty() {
        return res.text;
    }

    res.segments
        .into_iter()
        .filter(|segment| {
            !is_silent_segment(
                segment.no_speech_prob.unwrap_or(0.0),
                segment.avg_logprob.unwrap_or(0.0),
            )
        })
        .map(|segment| segment.text)
        .collect()
}

fn is_silent_segment(no_speech_prob: f64, avg_logprob: f64) -> bool {
    // Tuned from production testing to reduce hallucinated silence.
    no_speech_prob > 0.6 && avg_logprob < -0.4
}

/// whisper.cpp verbose_json response. Only the fields the bot uses are read,
/// and all of them are optional so a missing field doesn't throw away an
/// otherwise usable transcription.
#[derive(Debug, Deserialize)]
struct WhisperResponse {
    #[serde(default)]
    text: String,
    #[serde(default)]
    segments: Vec<WhisperSegment>,
}

#[derive(Debug, Deserialize)]
struct WhisperSegment {
    #[serde(default)]
    text: String,
    avg_logprob: Option<f64>,
    no_speech_prob: Option<f64>,
}

#[cfg(test)]
mod tests {
    use super::*;

    fn output_text(body: &str) -> String {
        whisper_output_text(serde_json::from_str(body).unwrap())
    }

    #[test]
    fn parses_response_with_missing_fields() {
        assert_eq!(
            output_text(
                r#"{"text":" whole","segments":[{"text":" Hello"},{"text":" world","avg_logprob":-0.1,"no_speech_prob":0.01}]}"#
            ),
            " Hello world"
        );
    }

    #[test]
    fn falls_back_to_text_without_segments() {
        assert_eq!(output_text(r#"{"text":" Hello"}"#), " Hello");
    }

    #[test]
    fn drops_silent_segments() {
        assert_eq!(
            output_text(
                r#"{"text":"","segments":[{"text":" Thanks for watching","avg_logprob":-0.9,"no_speech_prob":0.9},{"text":" Hi"}]}"#
            ),
            " Hi"
        );
    }
}
