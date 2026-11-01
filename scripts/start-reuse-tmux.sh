#!/usr/bin/env bash
set -euo pipefail

SESSION="${REUSE_TMUX_SESSION:-reuse}"
REPO_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
# NVIDIA RE-USE, a 9.6M parameter speech enhancement model, for /enhance.
MODEL="${REUSE_MODEL:-nvidia/RE-USE}"
HOST="${REUSE_HOST:-0.0.0.0}"
PORT="${REUSE_PORT:-8083}"
# NVIDIA's chunks are 5 seconds, which need about 1.1 GB of VRAM while they run.
# whisper.cpp and llama.cpp leave about 850 MB of a 4 GB GPU, which fits 2
# second chunks (about 570 MB).
CHUNK_SECONDS="${REUSE_CHUNK_SECONDS:-2}"
UV_BIN="${UV_BIN:-"$(command -v uv || echo "$HOME/.local/bin/uv")"}"
# llama-server fits as many layers as the free VRAM allows when it starts, so
# RE-USE takes its share of the GPU only after llama-server has loaded.
LLAMA_PORT="${LLAMA_PORT:-8081}"
LLAMA_WAIT_SECONDS="${LLAMA_WAIT_SECONDS:-600}"

if ! command -v tmux >/dev/null 2>&1; then
    echo "tmux is required but was not found in PATH" >&2
    exit 1
fi

if tmux has-session -t "$SESSION" 2>/dev/null; then
    echo "RE-USE server is already running in tmux session: $SESSION"
    exit 0
fi

if [[ ! -x "$UV_BIN" ]]; then
    echo "uv is required to run the RE-USE server but was not found (https://docs.astral.sh/uv/)" >&2
    exit 1
fi

for ((i = 0; i < LLAMA_WAIT_SECONDS; i++)); do
    if curl -sf "http://127.0.0.1:$LLAMA_PORT/health" >/dev/null 2>&1; then
        break
    fi
    sleep 1
done

# uv installs the script's dependencies (torch with CUDA and mamba_ssm, a few
# GB) on the first start, and the model downloads on the server's first start.
cmd=(
    "$UV_BIN" run --script "$REPO_DIR/scripts/reuse_server.py"
    --model "$MODEL"
    --chunk-seconds "$CHUNK_SECONDS"
    --host "$HOST"
    --port "$PORT"
)

printf -v run_cmd "%q " "${cmd[@]}"
# expandable_segments keeps PyTorch's memory from fragmenting on the small,
# shared GPU.
tmux new-session -d -s "$SESSION" -c "$REPO_DIR" \
    "PYTORCH_ALLOC_CONF=expandable_segments:True exec $run_cmd"

echo "Started RE-USE server in tmux session: $SESSION"
echo "Endpoint: http://$HOST:$PORT/v1/audio/enhance"
echo "Attach with: tmux attach -t $SESSION"
