use std::process::Stdio;
use tokio::io::AsyncWriteExt;

// equivalent to
// ffmpeg -i input.mp3 -ar 16000 -ac 1 -c:a pcm_s16le output.wav

pub async fn transcode_mp3_to_wav(input_data: &[u8]) -> Result<Vec<u8>, String> {
    let mut ffmpeg = tokio::process::Command::new("/opt/ffmpeg")
        .args([
            "-i",
            "pipe:0",
            "-ar",
            "16000",
            "-ac",
            "1",
            "-c:a",
            "pcm_s16le",
            "-f",
            "wav",
            "pipe:1",
        ])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .spawn()
        .map_err(|e| format!("Failed to spawn ffmpeg: {}", e))?;

    let stdin = ffmpeg.stdin.as_mut().ok_or("Failed to open stdin")?;
    stdin
        .write_all(input_data)
        .await
        .map_err(|e| format!("Failed to write to stdin: {}", e))?;

    let output = ffmpeg
        .wait_with_output()
        .await
        .map_err(|e| format!("Failed to wait for ffmpeg: {}", e))?;

    if !output.status.success() {
        return Err(format!("ffmpeg failed with status: {}", output.status));
    }

    Ok(output.stdout)
}
