# duck_transcriber

## Overview

A Telegram bot that transcribes, translates, and summarizes voice messages. Everything runs locally: whisper.cpp for speech-to-text and llama.cpp with Nemotron 3.5 Bolt E4B for translation and summaries. No audio or text is sent to a third-party service.

**Note on Serverless History:** The last commit of the bot being serverless is [`1ccb016ccabeb320b0c7637d3c15fc9bdedb2a48`](https://github.com/DuckyBlender/duck_transcriber/commit/1ccb016ccabeb320b0c7637d3c15fc9bdedb2a48).

## How It Works

1. Send a voice, audio, or video note to the bot
2. The bot decodes it with ffmpeg, measures its real length from the decoded
   audio, and transcribes it with a local whisper.cpp server
3. For `/translate`, `/summarize` and `/caveman`, the transcript goes to a local
   llama.cpp server, which translates it into English or summarizes it straight
   from the original language
4. For `/enhance`, the decoded audio goes to a local NVIDIA RE-USE server
   instead, which removes noise and echo, and the cleaned up recording comes
   back as a voice message
5. Results are sent back and cached for future use

The bot can be added to groups to automatically transcribe voice messages. In groups, transcripts and translations are sent as an expandable quote, so long transcripts stay a few lines tall until someone taps them. You can manually transcribe videos and audio files by replying to them with the bot commands. In DMs, the bot will automatically transcribe most media sent to it, and replies in plain text.

## Supported Commands

- `/start`: Initializes the bot and provides a welcome message
- `/help`: Provides information on how to use the bot and its features
- `/transcribe`: Transcribes the voice, audio, or video note in the reply message
- `/phonon`: Transcribes the voice, audio, or video note in the reply message with Phonon-2.5 instead of Whisper
- `/translate` (aliases: `english`, `en`): Translates (into English) the voice, audio, or video note in the reply message
- `/summarize`: Summarizes the voice, audio, or video note in the reply message
- `/caveman`: Summarizes the voice, audio, or video note in a "caveman" style
- `/enhance` (alias: `/cleanup`): Cleans up the voice, audio, or video note in the reply message (noise, echo, bad microphones) and sends it back as a voice message
- `/privacy`: Shows the privacy policy
- `/limits` (aliases: `/ratelimit`, `/ratelimits`): Shows current rate limit information
- `/donate`: Shows cryptocurrency donation addresses to support the project

## Technical Details

- **Language**: Rust with async/await
- **Telegram API**: Built using the `teloxide` crate
- **Transcription**: whisper.cpp server (`large-v3-turbo`) over HTTP with `reqwest` and rustls
- **`/phonon`**: [Phonon-2.5](https://huggingface.co/FermionResearch/Phonon-2.5), a 156 MB low-bit build of NVIDIA's Parakeet TDT 0.6B v3 for 25 languages, served by `fermion serve` on the CPU and kept in RAM (about 1.2 GB). It gets the same decoded 16 kHz mono WAV as whisper.cpp. Its transcripts are cached separately from whisper.cpp's
- **`/enhance`**: [NVIDIA RE-USE](https://huggingface.co/nvidia/RE-USE), a 9.6M parameter speech enhancement model, served by `scripts/reuse_server.py` on the GPU. It runs NVIDIA's own model code in fp32 and, like NVIDIA's `inference_chunk.py`, cuts recordings into chunks that overlap by half, because the model was trained on clips under 10 seconds. The chunks are 2 seconds instead of NVIDIA's 5: the model's weights are only 38 MB, but a 5 second chunk needs about 1.1 GB of VRAM while it runs, more than whisper.cpp and llama.cpp leave on a 4 GB GPU. With 2 second chunks the server uses about 570 MB of VRAM while it works and 165 MB idle, and enhances at about 1.5x realtime It gets the same decoded 16 kHz mono WAV as whisper.cpp; the result is sent back as an Ogg Opus voice message, and its Telegram file id is cached for 7 days (until the bot restarts), so the same recording isn't enhanced again in that time. The bot sends RE-USE one recording at a time and lets two more wait; any more get 😴 right away
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
  - Applies to all audio operations (transcribe, translate, summarize, caveman, enhance)
- **LLM Limits** (`/translate`, `/summarize`, `/caveman`): llama.cpp is much slower than whisper.cpp and shares its GPU, so these commands have their own limits on top of the ones above
  - Per user: 15 requests per hour and 50 per day; reacts with 🙊 when exceeded
  - Whole bot: 100 requests per hour; reacts with 😴 when reached
  - `/translate` accepts recordings up to 10 minutes (translating takes about 15 seconds per minute of audio); summaries accept the full 30 minutes
  - Lengths are measured from the decoded audio, never taken from Telegram, because the sender's app sets that number and it can be 0 or wrong
  - Only actual llama.cpp requests count: cached results and recordings without speech are free
- **`/enhance` Limits**: RE-USE holds the shared GPU for about 40 seconds per minute of audio, during which transcriptions wait, so it has the strictest limits
  - Per user: 5 recordings per hour and 15 per day; reacts with 🙊 when exceeded
  - Whole bot: 12 recordings per hour; reacts with 😴 when reached
  - Recordings up to 2 minutes
  - Cached results and refused recordings don't count
- **Failures**: Reacts with 💔 emoji instead of replying when whisper.cpp, llama.cpp or RE-USE can't produce a result

## Environment Variables

- `TELEGRAM_BOT_TOKEN`: The token for your Telegram bot
- `WHISPER_LOCAL_URL`: whisper.cpp inference endpoint (default: `http://host.docker.internal:8080/inference`)
- `LLM_URL`: llama.cpp chat completions endpoint (default: `http://host.docker.internal:8081/v1/chat/completions`)
- `PHONON_URL`: Phonon transcription endpoint for `/phonon` (default: `http://host.docker.internal:8082/v1/audio/transcriptions`)
- `REUSE_URL`: RE-USE enhancement endpoint for `/enhance` (default: `http://host.docker.internal:8083/v1/audio/enhance`)

## Deployment

The bot needs four servers running on the host: whisper.cpp, llama.cpp,
Phonon and RE-USE. whisper.cpp, llama.cpp and RE-USE share one GPU, so start
whisper.cpp first. llama-server then fits as many model layers into the VRAM
that is left as it can and keeps the rest in RAM, and RE-USE starts once
llama-server has loaded. Phonon runs on the CPU only.

```bash
scripts/start-whisper-tmux.sh   # whisper.cpp on :8080
scripts/start-llama-tmux.sh     # llama.cpp on :8081, waits for whisper.cpp
scripts/start-phonon-tmux.sh    # Phonon-2.5 on :8082, for /phonon
scripts/start-reuse-tmux.sh     # RE-USE on :8083, for /enhance, waits for llama.cpp
```

`start-llama-tmux.sh` expects a CUDA build of llama.cpp in `~/llama.cpp`
(override with `LLAMA_CPP_DIR`) and downloads the model GGUF on first start.
`start-phonon-tmux.sh` installs Phonon into `~/phonon/venv` with
[uv](https://docs.astral.sh/uv/) on first start (override with `PHONON_DIR`);
the model downloads on the server's first start.
`start-reuse-tmux.sh` runs `scripts/reuse_server.py` with
[uv](https://docs.astral.sh/uv/), which installs its dependencies (PyTorch with
CUDA and `mamba_ssm`) on the first start; the model (`REUSE_MODEL`, default
`nvidia/RE-USE`) downloads on the server's first start. RE-USE needs an NVIDIA
GPU: `mamba_ssm` has no CPU version.
To start them after a reboot, add them to the crontab in this order:

```
@reboot /path/to/duck_transcriber/scripts/start-whisper-tmux.sh
@reboot /path/to/duck_transcriber/scripts/start-llama-tmux.sh
@reboot /path/to/duck_transcriber/scripts/start-phonon-tmux.sh
@reboot /path/to/duck_transcriber/scripts/start-reuse-tmux.sh
```

### Local Development

1. **Install Rust**: https://rustup.rs/
2. **Clone the repository**: `git clone https://github.com/DuckyBlender/duck_transcriber.git`
3. **Create `.env` file**:
   ```bash
   TELEGRAM_BOT_TOKEN=your_bot_token
   WHISPER_LOCAL_URL=http://localhost:8080/inference
   LLM_URL=http://localhost:8081/v1/chat/completions
   PHONON_URL=http://localhost:8082/v1/audio/transcriptions
   REUSE_URL=http://localhost:8083/v1/audio/enhance
   ```
   The bot also needs `ffmpeg` on the `PATH`: every recording is decoded with it
   before it is measured and transcribed.
4. **Run the bot**:
   ```bash
   cargo run --release
   ```

### Docker

Build and run the bot in Docker with an optimized multi-stage build. whisper.cpp,
llama.cpp, Phonon and RE-USE are not part of Docker or compose; start them on the host first
(see above), then start duck_transcriber:

```bash
docker compose up -d
```

The compose file maps `host.docker.internal` to the host gateway so the
container can call the host servers.

The Dockerfile uses `cargo-chef` for efficient dependency caching, resulting in faster rebuilds.

## Error Handling & Reliability

- **Robust Error Handling**: All errors are properly handled and logged
- **Quiet Failures**: Uses 🙊 for per-user limits, 😴 when the whole bot is at its LLM or `/enhance` limit, and reacts with 💔 (no error message) when transcription, translation, summarization or enhancement fails, so a server outage doesn't flood groups with error replies

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
