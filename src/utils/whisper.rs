use tracing::{error, info};
use whisper_rs::{FullParams, SamplingStrategy, WhisperContext, WhisperContextParameters};

use super::transcode::transcode_mp3_to_wav;

const MINUTE_LIMIT: usize = 5;
const PATH_TO_MODEL: &str = "/opt/ggml-base-q5_1.bin";

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum TranscribeType {
    Transcribe,
    Translate,
}

// dont warn about unused variants
#[allow(dead_code)]
#[derive(Debug, Clone, Copy)]
pub enum Voice {
    Alloy,
    Echo,
    Fable,
    Onyx,
    Nova,
    Shimmer,
}

impl std::fmt::Display for Voice {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Voice::Alloy => write!(f, "Alloy"),
            Voice::Echo => write!(f, "Echo"),
            Voice::Fable => write!(f, "Fable"),
            Voice::Onyx => write!(f, "Onyx"),
            Voice::Nova => write!(f, "Nova"),
            Voice::Shimmer => write!(f, "Shimmer"),
        }
    }
}

// https://platform.openai.com/docs/guides/error-codes/api-errors
#[derive(Debug, PartialEq)]
#[allow(dead_code)]
pub enum TranscribeError {
    TooLong,
    Unknown(String),
}

impl std::fmt::Display for TranscribeError {
    fn fmt(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
        match self {
            TranscribeError::TooLong => {
                write!(f, "Audio length is more than {} minutes", MINUTE_LIMIT)
            }
            TranscribeError::Unknown(e) => write!(f, "Unknown error: {}", e),
        }
    }
}

pub fn convert_integer_to_float_audio(samples: &[u8]) -> Vec<f32> {
    let mut floats = Vec::with_capacity(samples.len());
    for sample in samples {
        floats.push(*sample as f32 / i16::MAX as f32);
    }
    floats
}

pub async fn transcribe_audio(
    buffer: Vec<u8>,
    transcribe_type: TranscribeType,
    seconds: u32,
) -> Result<String, TranscribeError> {
    // Check if minute limit is exceeded
    if seconds > MINUTE_LIMIT as u32 * 60 {
        info!("Audio length is more than {} minutes", MINUTE_LIMIT);
        return Err(TranscribeError::Unknown(format!(
            "Audio length is more than {} minutes",
            MINUTE_LIMIT
        )));
    }

    // Convert (ffmpeg -i input.mp3 -ar 16000 -ac 1 -c:a pcm_s16le output.wav)
    let buffer = transcode_mp3_to_wav(&buffer).await.map_err(|e| {
        error!("Failed to transcode audio: {}", e);
        TranscribeError::Unknown(format!("Failed to transcode audio: {}", e))
    })?;

    // load a context and model
    let ctx = WhisperContext::new_with_params(PATH_TO_MODEL, WhisperContextParameters::default()) // no gpu on aws lambda
        .map_err(|e| {
            error!("Failed to load model: {}", e);
            TranscribeError::Unknown(format!("Failed to load model: {}", e))
        })?;

    // create a params object
    let mut params = FullParams::new(SamplingStrategy::Greedy { best_of: 1 });

    // edit things as needed
    // we also enable translation
    if transcribe_type == TranscribeType::Translate {
        params.set_translate(true);
        params.set_language(Some("en"))
    }

    // we also explicitly disable anything that prints to stdout
    params.set_print_special(false);
    params.set_print_progress(false);
    params.set_print_realtime(false);
    params.set_print_timestamps(false);

    // assume we have a buffer of audio data
    let audio_data = buffer;

    // Convert integer to float audio
    let audio_data = &convert_integer_to_float_audio(&audio_data);

    // now we can run the model
    let mut state = ctx.create_state().map_err(|e| {
        error!("Failed to create state: {}", e);
        TranscribeError::Unknown(format!("Failed to create state: {}", e))
    })?;
    state.full(params, &audio_data[..]).map_err(|e| {
        error!("Failed to run model: {}", e);
        TranscribeError::Unknown(format!("Failed to run model: {}", e))
    })?;

    let mut output = String::new();

    // fetch the results
    let num_segments = state.full_n_segments().map_err(|e| {
        error!("Failed to get number of segments: {}", e);
        TranscribeError::Unknown(format!("Failed to get number of segments: {}", e))
    })?;

    for i in 0..num_segments {
        let segment = state
            .full_get_segment_text(i)
            .expect("failed to get segment");
        let start_timestamp = state
            .full_get_segment_t0(i)
            .expect("failed to get segment start timestamp");
        let end_timestamp = state
            .full_get_segment_t1(i)
            .expect("failed to get segment end timestamp");
        output.push_str(&format!(
            "{} - {}: {}\n",
            start_timestamp, end_timestamp, segment
        ));
    }

    // return the output
    Ok(output)
}
