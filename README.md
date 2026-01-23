# duck_transcriber

## Overview

A Telegram bot that transcribes, translates, and summarizes voice messages using the Groq Whisper API. It runs as a standalone application with SQLite caching and per-user rate limiting.

**Note on Serverless History:** The last commit of the bot being serverless is [`1ccb016ccabeb320b0c7637d3c15fc9bdedb2a48`](https://github.com/DuckyBlender/duck_transcriber/commit/1ccb016ccabeb320b0c7637d3c15fc9bdedb2a48).

## How It Works

1. The bot receives a voice, audio, or video note message from a user.
2. It downloads the file from Telegram and checks its duration. If the duration is above a specified limit, it sends a warning message to the user and exits.
3. The bot checks if the transcription already exists in SQLite cache. If it does, the cached result is sent back to the user.
4. If no cached result is found, the bot uploads the original media to the Groq Whisper API for transcription.
5. The result is sent back to the user as a text message and stored in SQLite for future reference (7 days for transcriptions/translations, 1 day for summaries).
6. Per-user rate limiting is enforced: 5 voice messages per minute, 30 per hour. When rate-limited, the bot reacts with 🙊 emoji.

## Supported Commands

- `/start`: Initializes the bot and provides a welcome message
- `/help`: Provides information on how to use the bot and its features
- `/transcribe`: Transcribes the voice, audio, or video note in the reply message
- `/translate`: Translates (into English) the voice, audio, or video note in the reply message
- `/summarize`: Summarizes the voice, audio, or video note in the reply message
- `/caveman`: Summarizes the voice, audio, or video note in a "caveman" style
- `/privacy`: Shows the privacy policy

## Technical Details

- **Language**: Rust with async/await
- **Telegram API**: Built using the `teloxide` crate
- **Transcription API**: Uses `reqwest` with rustls to communicate with GroqCloud's Whisper API
- **Database**: SQLite with sqlx for fast, type-safe queries
- **TLS**: Uses rustls everywhere (no OpenSSL dependencies)
- **Logging**: Dual output to stdout and `bot.log` file using fern
- **Model**: `whisper-large-v3-turbo` for transcription/translation
- **Caching**: 
  - Transcriptions and translations cached for 7 days
  - Summaries (default & caveman) cached for 1 day
  - File-based cache uses SQLite with automatic expiration cleanup
- **Rate Limiting**:
  - Per-user tracking: 5 messages/minute, 30 messages/hour
  - Reacts with 🙊 emoji when limit is exceeded
  - Applies to all audio operations (transcribe, translate, summarize, caveman)

### GroqCloud Privacy

**Uses GroqCloud with Global ZDR (Zero Day Retention) active. No data is stored on GroqCloud servers.** Audio files are processed instantly and discarded immediately—nothing is retained on their infrastructure.

## Environment Variables

- `TELEGRAM_BOT_TOKEN`: The token for your Telegram bot
- `GROQ_API_KEY`: Your GroqCloud API key(s). Supports multiple keys separated by commas for automatic failover (e.g., `key1,key2,key3`)
- `DATABASE_URL`: SQLite database path (default: `sqlite:duck_transcriber.db`)

## Deployment

### Local Development

1. **Install Rust**: https://rustup.rs/
2. **Clone the repository**: `git clone https://github.com/DuckyBlender/duck_transcriber.git`
3. **Create `.env` file**:
   ```bash
   TELEGRAM_BOT_TOKEN=your_bot_token
   GROQ_API_KEY=your_groq_key
   DATABASE_URL=sqlite:duck_transcriber.db
   ```
4. **Run the bot**:
   ```bash
   cargo run --release
   ```

### Docker

Build and run the bot in Docker with an optimized multi-stage build:

```bash
docker build -t duck_transcriber .
docker run -d \
  --name duck_transcriber \
  --env-file .env \
  -v ./data:/app/data \
  --restart unless-stopped \
  duck_transcriber
```

**Important:** The `-v ./data:/app/data` volume mount ensures the SQLite database persists between container restarts. Make sure you have a `.env` file with the required environment variables (see Local Development section).

The Dockerfile uses `cargo-chef` for efficient dependency caching, resulting in faster rebuilds.

To view logs:
```bash
docker logs -f duck_transcriber
```

## Error Handling & Reliability

- **Robust Error Handling**: All errors are properly handled and logged
- **Rate Limit Fallback**: When rate limited, reacts with 🙊 emoji instead of failing
- **Type-Safe Errors**: Uses a custom `TranscriptionError` enum for clean error categorization
- **Automatic Retry**: Configurable API key rotation for automatic failover (if multiple keys provided)
- **Logging**: All events logged to both stdout and `bot.log` file with timestamps

## License

This project is licensed under the GNU General Public License v3.0. See the [LICENSE](LICENSE) file for details.

## Contributing

Contributions are welcome! If you'd like to help improve this bot, please open a pull request with your changes.
