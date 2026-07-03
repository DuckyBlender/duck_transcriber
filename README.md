# duck_transcriber

## Overview

A Telegram bot that transcribes, translates, and summarizes voice messages using GroqCloud Whisper and chat completions.

**Note on Serverless History:** The last commit of the bot being serverless is [`1ccb016ccabeb320b0c7637d3c15fc9bdedb2a48`](https://github.com/DuckyBlender/duck_transcriber/commit/1ccb016ccabeb320b0c7637d3c15fc9bdedb2a48).

## How It Works

1. Send a voice, audio, or video note to the bot
2. The bot transcribes or translates audio using GroqCloud Whisper, summarizes
   translated text using GroqCloud chat completions, and falls back to a local
   whisper.cpp server when GroqCloud transcription/translation keys are missing
   or rate limited
3. Results are sent back instantly and cached for future use

The bot can be added to groups to automatically transcribe voice messages. You can manually transcribe videos and audio files by replying to them with the bot commands. In DMs, the bot will automatically transcribe most media sent to it.

## Supported Commands

- `/start`: Initializes the bot and provides a welcome message
- `/help`: Provides information on how to use the bot and its features
- `/transcribe`: Transcribes the voice, audio, or video note in the reply message
- `/translate` (aliases: `english`, `en`): Translates (into English) the voice, audio, or video note in the reply message
- `/summarize`: Summarizes the voice, audio, or video note in the reply message
- `/caveman`: Summarizes the voice, audio, or video note in a "caveman" style
- `/privacy`: Shows the privacy policy
- `/limits` (aliases: `/ratelimit`, `/ratelimits`): Shows current rate limit information
- `/donate`: Shows cryptocurrency donation addresses to support the project

## Technical Details

- **Language**: Rust with async/await
- **Telegram API**: Built using the `teloxide` crate
- **Transcription API**: Uses `reqwest` with rustls to communicate with GroqCloud's Whisper API, with a local whisper.cpp fallback when GroqCloud transcription/translation keys are missing or rate limited
- **Summarization API**: Uses GroqCloud chat completions with model fallback across configured API keys
- **Database**: SQLite with sqlx for fast, type-safe queries
- **TLS**: Uses rustls everywhere (no OpenSSL dependencies)
- **Logging**: Dual output to stdout and `bot.log` file using fern
- **Models**:
  - GroqCloud transcription: `whisper-large-v3-turbo`
  - GroqCloud translation: `whisper-large-v3`
  - Local whisper.cpp fallback: `large-v3-turbo`
  - Summarization fallback order: `qwen/qwen3.6-27b`, then `openai/gpt-oss-120b` and finally `openai/gpt-oss-20b`
- **Caching**: 
  - Transcriptions and translations cached for 7 days
  - Summaries (default and caveman) cached for 1 hour
  - In-memory cache uses per-result expiration timestamps with automatic cleanup
- **Rate Limiting**:
  - Per-user tracking: 25 messages per minute, 150 messages per hour
  - Temporary per-user rate-limit records are cleaned up after 2 hours
  - Reacts with 🙊 emoji when per-user limit is exceeded
  - Falls back to local whisper.cpp when GroqCloud transcription/translation keys are missing or rate limited
  - Reacts with 😴 emoji when GroqCloud summarization rate limits are reached
  - Applies to all audio operations (transcribe, translate, summarize, caveman)

### GroqCloud Privacy

**This bot uses GroqCloud with Global ZDR (Zero Day Retention) enabled.** No data is stored on GroqCloud servers. Audio files are processed instantly and discarded immediately—nothing is retained on their infrastructure.

## Environment Variables

- `TELEGRAM_BOT_TOKEN`: The token for your Telegram bot
- `GROQ_API_KEY`: Optional for `/transcribe` and `/translate` when local whisper.cpp is available; required for `/summarize` and `/caveman`. Supports multiple keys separated by commas for automatic failover (e.g., `key1,key2,key3`)
- `DATABASE_URL`: In-memory SQLite database URL for SQLx compile-time query checking and SQLx CLI commands.
- `WHISPER_LOCAL_URL`: Local whisper.cpp fallback endpoint (default: `http://host.docker.internal:8080/inference`)

## Deployment

### Local Development

1. **Install Rust**: https://rustup.rs/
2. **Clone the repository**: `git clone https://github.com/DuckyBlender/duck_transcriber.git`
3. **Create `.env` file**:
   ```bash
   TELEGRAM_BOT_TOKEN=your_bot_token
   # Optional for /transcribe and /translate when WHISPER_LOCAL_URL is available.
   # Required for /summarize and /caveman.
   GROQ_API_KEY=your_groq_key
   DATABASE_URL=sqlite::memory:
   ```
4. **Run the bot**:
   ```bash
   cargo run --release
   ```

### Docker

Build and run the bot in Docker with an optimized multi-stage build. The
whisper.cpp fallback is not part of Docker or compose; run it separately on the
host before starting the bot:

```bash
cd ../whisper.cpp
make -j large-v3-turbo
./build/bin/whisper-server \
  --host 0.0.0.0 \
  --port 8080 \
  --model models/ggml-large-v3-turbo.bin \
  --convert \
  --language auto
```

Then start duck_transcriber:

```bash
docker compose up -d
```

The compose file maps `host.docker.internal` to the host gateway so the
container can call the host whisper.cpp server at
`http://host.docker.internal:8080/inference`. The local server is used when no
GroqCloud API keys are configured for transcription/translation, or after all
configured GroqCloud API keys return rate limits.

The Dockerfile uses `cargo-chef` for efficient dependency caching, resulting in faster rebuilds.

## Error Handling & Reliability

- **Robust Error Handling**: All errors are properly handled and logged
- **Rate Limit Fallback**: Uses 🙊 for per-user limits, falls back to local whisper.cpp when GroqCloud transcription/translation keys are missing or rate limited, and uses 😴 when summarization rate limits cannot be served locally
- **Type-Safe Errors**: Uses a custom `TranscriptionError` enum for clean error categorization
- **Automatic Retry**: Configurable API key rotation for transcription/translation failover, plus model and key fallback for summarization

## Support & Donations

If you find this bot useful and would like to help cover API costs, donations are greatly appreciated! You can donate using various cryptocurrencies:

- **Bitcoin**: `bc1q3dqnaygpaqkwm20hjq73g3kcc534cnt47wjlmu`
- **Bitcoin Lightning**: `duckyblender@strike.me`
- **Ethereum (or any ERC20 token)**: `0x87d03a9DADd7927c1f058725307a1645BC406195`
- **Nano**: `nano_3ociqkh6taqqu7q7h99oiyuasnkugm7bss87r1r4eph7dym3tmp3cebtosc5`
- **Monero**: `84SdAF7JmMfQS3P1sSKasJHo8sQPjR3Xp58Vp1QWG4vMYdW26iZw6XuCMqL5FbtSQnUSKsGu6WtvXNMDEkwBtrE2VgKtNSK`

You can also use the `/donate` command in the bot to view these addresses directly.

## License

This project is licensed under the GNU General Public License v3.0. See the [LICENSE](LICENSE) file for details.

## Contributing

Contributions are welcome! If you'd like to help improve this bot, please open a pull request with your changes.
