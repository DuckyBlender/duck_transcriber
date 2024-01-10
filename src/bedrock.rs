use aws_sdk_bedrockruntime as bedrockruntime;
use base64::{engine::general_purpose::STANDARD as BASE64, Engine};
use bedrockruntime::primitives::Blob;
use serde_json::json;

pub async fn generate_image(prompt: String) -> Result<Vec<u8>, &'static str> {
    let config = aws_config::from_env().region("us-east-1").load().await; // most models are in us-east-1
    let client = aws_sdk_bedrockruntime::Client::new(&config);

    let input = json!({
        "taskType": "TEXT_IMAGE",
        "textToImageParams": {
            "text": prompt
        },
        "imageGenerationConfig": {
            "numberOfImages": 1,
            "quality": "standard",
            "height": 512,
            "width": 512,
            //"cfgScale": 8.0,
            "seed": rand::random::<i32>().abs(), // 0-2147483647
        }
    });

    let input_string = input.to_string();
    let input_bytes = input_string.into_bytes();
    let input_blob = Blob::new(input_bytes);

    let resp = client
        .invoke_model()
        .model_id("amazon.titan-image-generator-v1")
        .body(input_blob)
        .content_type("application/json")
        .send()
        .await
        .map_err(|_| "Failed to invoke model")?;

    let output_blob: Blob = resp.body;
    let output_bytes: &[u8] = output_blob.as_ref();
    let output_str = std::str::from_utf8(output_bytes)
        .map_err(|_| "Failed to convert output bytes to string")?;
    let output_string: String = output_str.to_owned();

    let output_json: serde_json::Value = serde_json::from_str(&output_string)
        .map_err(|_| "Failed to parse output string to JSON")?;
    let base64_image: &str = output_json["images"][0].as_str().unwrap_or_default();
    let output_image_bytes = BASE64
        .decode(base64_image)
        .map_err(|_| "Failed to decode base64 image")?;

    Ok(output_image_bytes)
}
