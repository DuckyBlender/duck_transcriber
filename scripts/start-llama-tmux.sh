#!/usr/bin/env bash
set -euo pipefail

SESSION="${LLAMA_TMUX_SESSION:-llama}"
LLAMA_DIR="${LLAMA_CPP_DIR:-"$HOME/llama.cpp"}"
MODEL_FILE="${LLAMA_MODEL_FILE:-NVIDIA-Nemotron-3.5-Bolt-E4B-UD-Q4_K_XL.gguf}"
MODEL_URL="${LLAMA_MODEL_URL:-"https://huggingface.co/unsloth/NVIDIA-Nemotron-3.5-Bolt-E4B-GGUF/resolve/main/$MODEL_FILE"}"
MODEL_PATH="${LLAMA_MODEL_PATH:-"$LLAMA_DIR/models/$MODEL_FILE"}"
SERVER_BIN="${LLAMA_SERVER_BIN:-"$LLAMA_DIR/build/bin/llama-server"}"
HOST="${LLAMA_HOST:-0.0.0.0}"
PORT="${LLAMA_PORT:-8081}"
# Two slots of 16k tokens each: enough to translate a 30 minute recording, and a
# long request in one slot doesn't hold up short ones in the other.
CTX_SIZE="${LLAMA_CTX_SIZE:-32768}"
PARALLEL="${LLAMA_PARALLEL:-2}"
# whisper.cpp must hold its VRAM before llama-server starts, because llama-server
# fits as many layers as the free VRAM allows at startup.
WHISPER_PORT="${WHISPER_PORT:-8080}"
WHISPER_WAIT_SECONDS="${WHISPER_WAIT_SECONDS:-180}"

if ! command -v tmux >/dev/null 2>&1; then
    echo "tmux is required but was not found in PATH" >&2
    exit 1
fi

if tmux has-session -t "$SESSION" 2>/dev/null; then
    echo "llama.cpp server is already running in tmux session: $SESSION"
    exit 0
fi

if [[ ! -x "$SERVER_BIN" ]]; then
    echo "llama-server not found: $SERVER_BIN" >&2
    echo "Build llama.cpp with CUDA first (see https://github.com/ggml-org/llama.cpp/blob/master/docs/build.md)" >&2
    exit 1
fi

if [[ ! -f "$MODEL_PATH" ]]; then
    echo "Model not found, downloading $MODEL_FILE..."
    mkdir -p "$(dirname "$MODEL_PATH")"
    curl -fL -C - -o "$MODEL_PATH.part" "$MODEL_URL"
    mv "$MODEL_PATH.part" "$MODEL_PATH"
fi

for ((i = 0; i < WHISPER_WAIT_SECONDS; i++)); do
    if (exec 3<>"/dev/tcp/127.0.0.1/$WHISPER_PORT") 2>/dev/null; then
        break
    fi
    sleep 1
done

cmd=(
    "$SERVER_BIN"
    --host "$HOST"
    --port "$PORT"
    --model "$MODEL_PATH"
    --alias bolt
    --ctx-size "$CTX_SIZE"
    --parallel "$PARALLEL"
    --flash-attn on
    --reasoning off
    # Prometheus metrics on /metrics, scraped by ~/services/monitoring.
    --metrics
)

printf -v run_cmd "%q " "${cmd[@]}"
tmux new-session -d -s "$SESSION" -c "$LLAMA_DIR" "exec $run_cmd"

echo "Started llama.cpp server in tmux session: $SESSION"
echo "Endpoint: http://$HOST:$PORT/v1/chat/completions"
echo "Attach with: tmux attach -t $SESSION"
