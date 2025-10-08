# duck_transcriber

## Overview

This is a serverless Telegram bot that transcribes voice, audio, and video notes sent to it using the Groq Whisper API. It also stores and retrieves transcriptions using AWS DynamoDB.

## How It Works

1. The bot receives a voice, audio, or video note message from a user.
2. It downloads the file from Telegram and checks its duration. If the duration is above a specified limit, it sends a warning message to the user and exits.
3. The bot checks if the transcription already exists in AWS DynamoDB. If it does, the saved transcription is sent back to the user.
4. If no transcription is found, the bot uploads the original media to the Groq Whisper API for transcription.
5. The transcription is sent back to the user as a text message and stored in DynamoDB for future reference.

## Supported Commands

- `/start`: Initializes the bot and provides a welcome message
- `/help`: Provides information on how to use the bot and its features
- `/transcribe`: Transcribes the voice, audio, or video note in the reply message
- `/translate`: Translates (into English) the voice, audio, or video note in the reply message
- `/summarize`: Summarizes the voice, audio, or video note in the reply message
- `/caveman`: Transcribes the voice, audio, or video note in the reply message in a "caveman" style

### Developer Commands

Developer commands have been removed.

## Technical Details

- The bot is built using the `teloxide` crate for interacting with the Telegram API.
- The transcription is done using the `reqwest` crate to send a request to the Groq Whisper API.
- The original file from Telegram is uploaded directly to the Groq Whisper API. FFmpeg has been removed due to memory constraints in the serverless environment, so no conversion step is performed.
- The bot uses AWS DynamoDB to store and retrieve transcriptions, ensuring that repeated requests for the same audio do not require retranscription.
- The bot is deployed as a serverless function using AWS Lambda.

### Error Handling & Reliability

- **Robust Error Handling**: All errors are properly handled and return HTTP 200 to prevent Telegram webhook retry loops.
- **IP Validation**: Validates that all incoming requests come from official Telegram server IP ranges. Unauthorized requests are silently rejected.
- **Multi-API-Key Support**: Configure multiple Groq API keys for automatic failover. When a rate limit is hit, the bot automatically tries the next key.
- **Rate Limit Feedback**: When all API keys are rate limited, the bot reacts with a 😴 emoji on the message to provide visual feedback without spamming the user.
- **Type-Safe Errors**: Uses a custom `TranscriptionError` enum for clean error categorization (rate limits, network errors, parse errors, API errors).

## Environment Variables

- `TELEGRAM_BOT_TOKEN`: the token for the Telegram bot.
- `GROQ_API_KEY`: the API key(s) for the Groq Whisper API. Supports multiple keys separated by commas for automatic failover on rate limits (e.g., `key1,key2,key3`).
- `DYNAMODB_TABLE`: the name of the DynamoDB table where transcriptions are stored.
  

## Deployment

Before deploying this bot, ensure you have the following prerequisites installed:

- **AWS CLI**: Follow the instructions [here](https://aws.amazon.com/cli/) to install the AWS Command Line Interface.
- **cargo-lambda**: Install `cargo-lambda` (not with cargo, it doesn't support cross compilation) by following the instructions [here](https://www.cargo-lambda.info/guide/getting-started.html).

### Build and Deploy

To build:
```bash
cargo lambda build --release --arm64
```

To deploy:
```bash
cargo lambda deploy
```

### Setting Up the Telegram Webhook

After deploying your Lambda function, you need to configure the Telegram webhook to point to your Lambda function URL.

#### Basic Setup

Replace `<YOUR_BOT_TOKEN>` with your Telegram bot token and `<YOUR_LAMBDA_URL>` with your Lambda function URL:

```bash
curl -X POST "https://api.telegram.org/bot<YOUR_BOT_TOKEN>/setWebhook" \
  -H "Content-Type: application/json" \
  -d '{
    "url": "<YOUR_LAMBDA_URL>",
    "allowed_updates": ["message"]
  }'
```

**Important**: The `allowed_updates` parameter can be set to `["message"]` to ensure the bot only receives message updates and nothing else (like inline queries, polls, etc.). This will reduce Lambda costs.

#### Troubleshooting Setup

If the bot is stuck in a loop or something goes wrong, you can reset the webhook and drop all pending updates:

```bash
curl -X POST "https://api.telegram.org/bot<YOUR_BOT_TOKEN>/setWebhook" \
  -H "Content-Type: application/json" \
  -d '{
    "url": "<YOUR_LAMBDA_URL>",
    "allowed_updates": ["message"],
    "drop_pending_updates": true
  }'
```

### FFmpeg

FFmpeg has been removed due to memory constraints in the serverless environment and is no longer required.

### AWS Lambda Permissions

Ensure that your AWS Lambda function has the necessary permissions to access DynamoDB. You will need to attach a policy that grants the Lambda function read and write permissions to the DynamoDB table. This can be done by attaching the `AWSLambdaDynamoDBExecutionRole` managed policy or by creating a custom policy with the necessary permissions.

## License

Do literally whatever you want with this code. I don't care.

## Contributing

Contributions are welcome! If you'd like to help improve this bot, please open a pull request with your changes.
