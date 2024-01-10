use aws_sdk_bedrockruntime as bedrockruntime;
use base64::{engine::general_purpose::STANDARD as BASE64, Engine};
use bedrockruntime::primitives::Blob;
use serde_json::json;

pub async fn generate_image(prompt: String) -> Result<Vec<u8>, String> {
    // Configure AWS SDK with environment variables and set the region to us-east-1
    let config = aws_config::from_env().region("us-east-1").load().await; // most models are in us-east-1

    // Create a new client using the configured AWS SDK
    let client = aws_sdk_bedrockruntime::Client::new(&config);

    // Prepare the input JSON payload
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

    // Convert the input JSON payload to bytes and create a Blob
    let input_string = input.to_string();
    let input_bytes = input_string.into_bytes();
    let input_blob = Blob::new(input_bytes);

    // Invoke the model using the client, passing the input Blob and content type
    let resp = client
        .invoke_model()
        .model_id("amazon.titan-image-generator-v1")
        .body(input_blob)
        .content_type("application/json")
        .send()
        .await
        .map_err(|err| format!("Failed to invoke model: {err}"))?;

    // Extract the output Blob from the response
    let output_blob: Blob = resp.body;

    // Convert the output Blob to bytes and then to a string
    let output_bytes: &[u8] = output_blob.as_ref();
    let output_str = core::str::from_utf8(output_bytes)
        .map_err(|err| format!("Failed to convert output bytes to string: {err}"))?;

    // Create a String from the output string
    let output_string: String = output_str.to_owned();

    // Parse the output string as JSON
    let output_json: serde_json::Value = serde_json::from_str(&output_string)
        .map_err(|err| format!("Failed to parse output string as JSON: {err}"))?;

    // Extract the base64 image string from the parsed JSON
    let base64_image: &str = output_json["images"][0].as_str().unwrap_or_default();

    // Decode the base64 image string into bytes
    let output_image_bytes = BASE64
        .decode(base64_image)
        .map_err(|err| format!("Failed to decode base64 image: {err}"))?;

    // Return the decoded image bytes
    Ok(output_image_bytes)
}
