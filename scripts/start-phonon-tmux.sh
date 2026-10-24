#!/usr/bin/env bash
set -euo pipefail

SESSION="${PHONON_TMUX_SESSION:-phonon}"
PHONON_DIR="${PHONON_DIR:-"$HOME/phonon"}"
VENV_DIR="${PHONON_VENV_DIR:-"$PHONON_DIR/venv"}"
# Phonon-2.5 is a 156 MB, 25-language build of NVIDIA's Parakeet TDT 0.6B v3.
# It runs on the CPU and stays loaded in RAM, so it doesn't touch the GPU that
# whisper.cpp and llama.cpp share.
MODEL="${PHONON_MODEL:-phonon-2.5}"
HOST="${PHONON_HOST:-0.0.0.0}"
PORT="${PHONON_PORT:-8082}"
THREADS="${PHONON_THREADS:-}"
UV_BIN="${UV_BIN:-"$(command -v uv || echo "$HOME/.local/bin/uv")"}"

if ! command -v tmux >/dev/null 2>&1; then
    echo "tmux is required but was not found in PATH" >&2
    exit 1
fi

if tmux has-session -t "$SESSION" 2>/dev/null; then
    echo "Phonon server is already running in tmux session: $SESSION"
    exit 0
fi

if [[ ! -x "$VENV_DIR/bin/fermion" ]]; then
    if [[ ! -x "$UV_BIN" ]]; then
        echo "uv is required to install Phonon but was not found (https://docs.astral.sh/uv/)" >&2
        exit 1
    fi
    echo "Phonon not found, installing it into $VENV_DIR..."
    mkdir -p "$PHONON_DIR"
    "$UV_BIN" venv --python 3.11 "$VENV_DIR"
    # The CPU build of torch; the CUDA one is several GB and unused here.
    "$UV_BIN" pip install --python "$VENV_DIR/bin/python" torch --index-url https://download.pytorch.org/whl/cpu
    "$UV_BIN" pip install --python "$VENV_DIR/bin/python" fermion-research safetensors soundfile scipy zstandard
fi

cmd=(
    "$VENV_DIR/bin/fermion" serve "$MODEL"
    --host "$HOST"
    --port "$PORT"
)

if [[ -n "$THREADS" ]]; then
    cmd+=(--threads "$THREADS")
fi

printf -v run_cmd "%q " "${cmd[@]}"
tmux new-session -d -s "$SESSION" -c "$PHONON_DIR" "exec $run_cmd"

echo "Started Phonon server in tmux session: $SESSION"
echo "Endpoint: http://$HOST:$PORT/v1/audio/transcriptions"
echo "Attach with: tmux attach -t $SESSION"
