# duck_transcriber

## Overview

A Telegram bot that transcribes, translates, and summarizes voice messages. Everything runs locally: whisper.cpp for speech-to-text and llama.cpp with Nemotron 3.5 Bolt E4B for translation and summaries. No audio or text is sent to a third-party service.

**Note on Serverless History:** The last commit of the bot being serverless is [`1ccb016ccabeb320b0c7637d3c15fc9bdedb2a48`](https://github.com/DuckyBlender/duck_transcriber/commit/1ccb016ccabeb320b0c7637d3c15fc9bdedb2a48).

## How It Works

1. Send a voice, audio, or video note to the bot
2. The bot transcribes it with a local whisper.cpp server
3. For `/translate`, `/summarize` and `/caveman`, the transcript goes to a local
   llama.cpp server, which translates it into English or summarizes it straight
   from the original language
4. Results are sent back and cached for future use

The bot can be added to groups to automatically transcribe voice messages. In groups, transcripts and translations are sent as an expandable quote, so long transcripts stay a few lines tall until someone taps them. You can manually transcribe videos and audio files by replying to them with the bot commands. In DMs, the bot will automatically transcribe most media sent to it, and replies in plain text.

## Supported Commands

- `/start`: Initializes the bot and provides a welcome message
- `/help`: Provides information on how to use the bot and its features
- `/transcribe`: Transcribes the voice, audio, or video note in the reply message
- `/parakeet`: Transcribes the voice, audio, or video note in the reply message with Parakeet (Phonon-2.5) instead of Whisper
- `/translate` (aliases: `english`, `en`): Translates (into English) the voice, audio, or video note in the reply message
- `/summarize`: Summarizes the voice, audio, or video note in the reply message
- `/caveman`: Summarizes the voice, audio, or video note in a "caveman" style
- `/privacy`: Shows the privacy policy
- `/limits` (aliases: `/ratelimit`, `/ratelimits`): Shows current rate limit information
- `/donate`: Shows cryptocurrency donation addresses to support the project

## Technical Details

- **Language**: Rust with async/await
- **Telegram API**: Built using the `teloxide` crate
- **Transcription**: whisper.cpp server (`large-v3-turbo`) over HTTP with `reqwest` and rustls
- **`/parakeet`**: [Phonon-2.5](https://huggingface.co/FermionResearch/Phonon-2.5), a 156 MB low-bit build of NVIDIA's Parakeet TDT 0.6B v3 for 25 languages, served by `fermion serve` on the CPU and kept in RAM (about 1.2 GB). The bot converts the audio to 16 kHz mono WAV with ffmpeg first, because the server only reads wav, flac, ogg and mp3. Its transcripts are cached separately from whisper.cpp's
- **Translation & summaries**: llama.cpp server (`NVIDIA-Nemotron-3.5-Bolt-E4B` `UD-Q4_K_XL` GGUF from Unsloth) through its OpenAI-compatible chat completions endpoint, with reasoning turned off
- **TLS**: Uses rustls everywhere (no OpenSSL dependencies)
- **Logging**: Dual output to stdout and `bot.log` file using fern
- **Caching**:
  - Transcriptions and translations cached for 7 days; `/translate` and the summaries reuse a cached transcript instead of running whisper.cpp again
  - Summaries (default and caveman) cached for 1 hour
  - In-memory cache uses per-result expiration timestamps with automatic cleanup
- **Rate Limiting**:
  - Per-user tracking: 25 messages per minute, 150 messages per hour
  - Temporary per-user rate-limit records are cleaned up after 2 hours
  - Reacts with 🙊 emoji when per-user limit is exceeded
  - Applies to all audio operations (transcribe, translate, summarize, caveman)
- **LLM Limits** (`/translate`, `/summarize`, `/caveman`): llama.cpp is much slower than whisper.cpp and shares its GPU, so these commands have their own limits on top of the ones above
  - Per user: 15 requests per hour and 50 per day; reacts with 🙊 when exceeded
  - Whole bot: 100 requests per hour; reacts with 😴 when reached
  - `/translate` accepts recordings up to 10 minutes (translating takes about 15 seconds per minute of audio); summaries accept the full 30 minutes
  - Only actual llama.cpp requests count: cached results and recordings without speech are free
- **Failures**: Reacts with 💔 emoji instead of replying when whisper.cpp or llama.cpp can't produce a result

## Environment Variables

- `TELEGRAM_BOT_TOKEN`: The token for your Telegram bot
- `DATABASE_URL`: In-memory SQLite database URL for SQLx compile-time query checking and SQLx CLI commands.
- `WHISPER_LOCAL_URL`: whisper.cpp inference endpoint (default: `http://host.docker.internal:8080/inference`)
- `LLM_URL`: llama.cpp chat completions endpoint (default: `http://host.docker.internal:8081/v1/chat/completions`)
- `PARAKEET_URL`: Phonon transcription endpoint for `/parakeet` (default: `http://host.docker.internal:8082/v1/audio/transcriptions`)

## Deployment

The bot needs three servers running on the host: whisper.cpp, llama.cpp and
Phonon. whisper.cpp and llama.cpp share one GPU, so start whisper.cpp first.
llama-server then fits as many model layers into the VRAM that is left as it
can and keeps the rest in RAM. Phonon runs on the CPU only.

```bash
scripts/start-whisper-tmux.sh   # whisper.cpp on :8080
scripts/start-llama-tmux.sh     # llama.cpp on :8081, waits for whisper.cpp
scripts/start-phonon-tmux.sh    # Phonon-2.5 on :8082, for /parakeet
```

`start-llama-tmux.sh` expects a CUDA build of llama.cpp in `~/llama.cpp`
(override with `LLAMA_CPP_DIR`) and downloads the model GGUF on first start.
`start-phonon-tmux.sh` installs Phonon into `~/phonon/venv` with
[uv](https://docs.astral.sh/uv/) on first start (override with `PHONON_DIR`);
the model downloads on the server's first start.
To start them after a reboot, add them to the crontab in this order:

```
@reboot /path/to/duck_transcriber/scripts/start-whisper-tmux.sh
@reboot /path/to/duck_transcriber/scripts/start-llama-tmux.sh
@reboot /path/to/duck_transcriber/scripts/start-phonon-tmux.sh
```

### Local Development

1. **Install Rust**: https://rustup.rs/
2. **Clone the repository**: `git clone https://github.com/DuckyBlender/duck_transcriber.git`
3. **Create `.env` file**:
   ```bash
   TELEGRAM_BOT_TOKEN=your_bot_token
   DATABASE_URL=sqlite::memory:
   WHISPER_LOCAL_URL=http://localhost:8080/inference
   LLM_URL=http://localhost:8081/v1/chat/completions
   PARAKEET_URL=http://localhost:8082/v1/audio/transcriptions
   ```
   `/parakeet` also needs `ffmpeg` on the `PATH`.
4. **Run the bot**:
   ```bash
   cargo run --release
   ```

### Docker

Build and run the bot in Docker with an optimized multi-stage build. whisper.cpp
llama.cpp and Phonon are not part of Docker or compose; start them on the host first
(see above), then start duck_transcriber:

```bash
docker compose up -d
```

The compose file maps `host.docker.internal` to the host gateway so the
container can call the host servers.

The Dockerfile uses `cargo-chef` for efficient dependency caching, resulting in faster rebuilds.

## Error Handling & Reliability

- **Robust Error Handling**: All errors are properly handled and logged
- **Quiet Failures**: Uses 🙊 for per-user limits, 😴 when the whole bot is at its LLM limit, and reacts with 💔 (no error message) when transcription, translation or summarization fails, so a server outage doesn't flood groups with error replies
- **Type-Safe Errors**: Uses a custom `TranscriptionError` enum for clean error categorization

## Support & Donations

If you find this bot useful and would like to help cover hosting costs, donations are greatly appreciated! You can donate using various cryptocurrencies:

- **Bitcoin**: `bc1qqvahwa3we2eymeku5489kpdk29pvddnn8yruzy`
- **Bitcoin Lightning**: `duckyblender@strike.me`
- **Ethereum (or any ERC20 token)**: `0xd399774D0a969FC2ddF7D0D0f4f165638541408c`
- **Nano**: `nano_3ociqkh6taqqu7q7h99oiyuasnkugm7bss87r1r4eph7dym3tmp3cebtosc5`
- **Monero**: `49afHkLvAEAXmdCqhWDCYvZiBD942ouwrhxbwto2HXzoGW877umX2aEFcVf8qzpawuFTBo28TK5Kg8sSzSQSXh7D7YoKEeK`

You can also use the `/donate` command in the bot to view these addresses directly.

## License

This project is licensed under the GNU General Public License v3.0. See the [LICENSE](LICENSE) file for details.

## Contributing

Contributions are welcome! If you'd like to help improve this bot, please open a pull request with your changes.
