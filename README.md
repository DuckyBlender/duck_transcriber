# duck_transcriber

## Overview

This is a serverless Telegram bot that transcribes voice, audio, and video notes sent to it using the Groq Whisper API. It also stores and retrieves transcriptions using AWS DynamoDB.

## How It Works

1. The bot receives a voice, audio, or video note message from a user.
2. It downloads the file from Telegram and checks its duration. If the duration is above a specified limit, it sends a warning message to the user and exits.
3. The bot checks if the transcription already exists in AWS DynamoDB. If it does, the saved transcription is sent back to the user.
4. If no transcription is found, the bot transcribes the audio using the Groq Whisper API.
5. The transcription is sent back to the user as a text message and stored in DynamoDB for future reference.

## Supported Commands

- `/start`: Initializes the bot and provides a welcome message.
- `/help`: Provides information on how to use the bot and its features.
- `/transcribe`: Transcribes the voice, audio, or video note in the reply message.
- `/translate`: Translates (into English) the voice, audio, or video note in the reply message.

## Technical Details

- The bot is built using the `teloxide` crate for interacting with the Telegram API.
- The transcription is done using the `reqwest` crate to send a request to the Groq Whisper API.
- The file is first converted to a 16 kHz mono WAV file using FFmpeg before being sent to the Groq Whisper API.
- The bot uses AWS DynamoDB to store and retrieve transcriptions, ensuring that repeated requests for the same audio do not require retranscription.
- The bot is deployed as a serverless function using AWS Lambda.

## Environment Variables

- `TELEGRAM_BOT_TOKEN`: the token for the Telegram bot.
- `GROQ_API_KEY`: the API key for the Groq Whisper API.
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

### Create FFmpeg Lambda Layer

1. **Download FFmpeg Static Build**:
   Download the ARM64 static build of FFmpeg for Amazon Linux 2023:
   ```bash
   wget https://johnvansickle.com/ffmpeg/releases/ffmpeg-release-arm64-static.tar.xz
   tar -xf ffmpeg-release-arm64-static.tar.xz
   mv ffmpeg-*-arm64-static/ffmpeg ./ffmpeg
   chmod +x ffmpeg
   ```

2. **Create the Layer Directory**:
   Create a directory structure for the Lambda layer:
   ```bash
   mkdir -p ffmpeg-layer/bin
   mv ffmpeg ffmpeg-layer/bin/
   ```

3. **Zip the Layer**:
   Compress the `ffmpeg-layer` directory:
   ```bash
   zip -r ffmpeg-layer.zip ffmpeg-layer
   ```

4. **Publish the Layer**:
   Publish the layer to AWS Lambda using the AWS CLI:
   ```bash
   aws lambda publish-layer-version \
     --layer-name ffmpeg-arm64 \
     --description "FFmpeg static binary for ARM64 on Amazon Linux 2023" \
     --zip-file fileb://ffmpeg-layer.zip \
     --compatible-runtimes provided.al2023 \
     --compatible-architectures arm64
   ```

5. **Attach the Layer to Your Lambda Function**:
   Update the Cargo.toml file to include the ARN of the FFmpeg Lambda layer

### AWS Lambda Permissions

Ensure that your AWS Lambda function has the necessary permissions to access DynamoDB. You will need to attach a policy that grants the Lambda function read and write permissions to the DynamoDB table. This can be done by attaching the `AWSLambdaDynamoDBExecutionRole` managed policy or by creating a custom policy with the necessary permissions.

## License

Do literally whatever you want with this code. I don't care.

## Contributing

Contributions are welcome! If you'd like to help improve this bot, please open a pull request with your changes.
