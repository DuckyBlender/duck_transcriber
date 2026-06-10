#!/usr/bin/env bash
set -euo pipefail

SESSION="${WHISPER_TMUX_SESSION:-whisper}"
REPO_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
WHISPER_DIR="${WHISPER_CPP_DIR:-"$REPO_DIR/whisper.cpp"}"
MODEL_NAME="${WHISPER_MODEL_NAME:-large-v3-turbo}"
MODEL_PATH="${WHISPER_MODEL_PATH:-"$WHISPER_DIR/models/ggml-$MODEL_NAME.bin"}"
SERVER_BIN="${WHISPER_SERVER_BIN:-"$WHISPER_DIR/build/bin/whisper-server"}"
HOST="${WHISPER_HOST:-0.0.0.0}"
PORT="${WHISPER_PORT:-8080}"
THREADS="${WHISPER_THREADS:-}"

if ! command -v tmux >/dev/null 2>&1; then
    echo "tmux is required but was not found in PATH" >&2
    exit 1
fi

if tmux has-session -t "$SESSION" 2>/dev/null; then
    echo "whisper.cpp server is already running in tmux session: $SESSION"
    exit 0
fi

if [[ ! -d "$WHISPER_DIR" ]]; then
    echo "whisper.cpp directory not found: $WHISPER_DIR" >&2
    exit 1
fi

if [[ ! -f "$MODEL_PATH" ]]; then
    echo "Model not found, downloading $MODEL_NAME..."
    "$WHISPER_DIR/models/download-ggml-model.sh" "$MODEL_NAME"
fi

if [[ ! -x "$SERVER_BIN" ]]; then
    echo "whisper-server not found, building it..."
    cmake -S "$WHISPER_DIR" -B "$WHISPER_DIR/build" -DWHISPER_BUILD_SERVER=ON
    cmake --build "$WHISPER_DIR/build" --config Release -j --target whisper-server
fi

cmd=(
    "$SERVER_BIN"
    --host "$HOST"
    --port "$PORT"
    --model "$MODEL_PATH"
    --convert
    --language auto
)

if [[ -n "$THREADS" ]]; then
    cmd+=(--threads "$THREADS")
fi

printf -v run_cmd "%q " "${cmd[@]}"
tmux new-session -d -s "$SESSION" -c "$WHISPER_DIR" "exec $run_cmd"

echo "Started whisper.cpp server in tmux session: $SESSION"
echo "Endpoint: http://$HOST:$PORT/inference"
echo "Attach with: tmux attach -t $SESSION"
