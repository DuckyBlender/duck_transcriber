#!/usr/bin/env -S uv run --script
# /// script
# requires-python = "==3.11.*"
# dependencies = [
#     "einops",
#     "huggingface_hub",
#     "mamba-ssm @ https://github.com/state-spaces/mamba/releases/download/v2.3.2.post1/mamba_ssm-2.3.2.post1+cu13torch2.10cxx11abiTRUE-cp311-cp311-linux_x86_64.whl",
#     "numpy",
#     "safetensors",
#     "torch==2.10.*",
# ]
#
# [tool.uv.sources]
# torch = { index = "pytorch-cu130" }
#
# [[tool.uv.index]]
# name = "pytorch-cu130"
# url = "https://download.pytorch.org/whl/cu130"
# explicit = true
# ///
"""HTTP server for NVIDIA RE-USE speech enhancement, used by /enhance.

POST a WAV file (16-bit PCM, mono, 8 to 48 kHz) to /v1/audio/enhance and get
the cleaned up recording back as WAV at the same sample rate.

The model code and weights come from the Hugging Face repo given by --model,
and run as NVIDIA's inference_chunk.py runs them: on the GPU (mamba_ssm has no
CPU kernels), in overlapping chunks.
"""

import argparse
import importlib
import io
import json
import logging
import struct
import sys
import threading
import time
import wave
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer

import numpy as np
import torch
import torch.nn.functional as F

SAMPLE_RATES = {8000, 16000, 22050, 24000, 32000, 44100, 48000}

log = logging.getLogger("reuse")


class Enhancer:
    def __init__(self, repo, chunk_seconds, hop_portion):
        from huggingface_hub import snapshot_download

        path = snapshot_download(
            repo, allow_patterns=["config.json", "model.safetensors", "models/*.py"]
        )
        sys.path.insert(0, path)
        generator = importlib.import_module("models.generator_SEMamba_time_d4")
        self.stfts = importlib.import_module("models.stfts")
        with open(f"{path}/config.json") as f:
            # The laughter variant's config is nested under "cfg".
            config = json.load(f)
            self.cfg = config.get("cfg", config)
        self.model = generator.SEMamba.from_pretrained(path, cfg=self.cfg).cuda().eval()
        self.chunk_seconds = chunk_seconds
        self.hop_portion = hop_portion

    @torch.inference_mode()
    def enhance(self, audio, sample_rate):
        """Enhances mono float32 samples. Recordings longer than one chunk are
        cut into overlapping chunks and put back together with a Hann window,
        as in NVIDIA's inference_chunk.py, because the model was trained on
        clips under 10 seconds."""
        wav = torch.from_numpy(audio).cuda()
        n = wav.shape[0]
        chunk = int(self.chunk_seconds * sample_rate)
        if n <= chunk:
            return self._forward(wav, sample_rate).cpu().numpy()

        hop = int(self.hop_portion * chunk)
        # The last chunk ends at the end of the recording, like NVIDIA's.
        starts = list(range(0, n - chunk, hop)) + [n - chunk]
        window = torch.hann_window(chunk, device="cuda").clamp(min=1e-7)
        out = torch.zeros(n, device="cuda")
        weight = torch.zeros(n, device="cuda")
        for start in starts:
            out[start : start + chunk] += self._forward(wav[start : start + chunk], sample_rate) * window
            weight[start : start + chunk] += window
        return (out / weight).cpu().numpy()

    def _forward(self, wav, sample_rate):
        stft = self.cfg["stft_cfg"]
        compress = self.cfg["model_cfg"]["compress_factor"]
        # The STFT scales with the sample rate, so every rate gets 5 ms frames.
        n_fft, hop, win = (
            make_even(stft[key] * sample_rate // stft["sampling_rate"])
            for key in ("n_fft", "hop_size", "win_size")
        )
        mag, pha, _ = self.stfts.mag_phase_stft(
            wav[None], n_fft=n_fft, hop_size=hop, win_size=win, compress_factor=compress
        )
        amp, pha, _ = self.model(mag, pha)
        # NVIDIA's fix for a "strange sweep artifact": silence frames where the
        # model zeroed most of the spectrum.
        silent = (torch.expm1(F.relu(amp)) == 0).float().mean(dim=1) > 0.5
        amp[silent[:, None, :].expand_as(amp)] = 0
        out = self.stfts.mag_phase_istft(amp, pha, n_fft, hop, win, compress)[0]
        # The inverse STFT can come out a few samples short.
        return F.pad(out, (0, wav.shape[0] - out.shape[0]), value=1e-8)[: wav.shape[0]]


def make_even(value):
    value = int(round(value))
    return value if value % 2 == 0 else value + 1


def read_wav(data):
    """Reads 16-bit PCM mono WAV. A data chunk whose size is unset, as ffmpeg
    writes it when it outputs to a pipe, runs to the end of the file."""
    if len(data) < 12 or data[:4] != b"RIFF" or data[8:12] != b"WAVE":
        raise ValueError("not a WAV file")
    pos, fmt = 12, None
    while pos + 8 <= len(data):
        tag, size = data[pos : pos + 4], struct.unpack("<I", data[pos + 4 : pos + 8])[0]
        body = pos + 8
        if tag == b"fmt ":
            fmt = struct.unpack("<HHIIHH", data[body : body + 16])
        elif tag == b"data":
            if fmt is None:
                raise ValueError("data before fmt")
            audio_format, channels, sample_rate, _, _, bits = fmt
            if audio_format != 1 or channels != 1 or bits != 16:
                raise ValueError("expected 16-bit PCM mono")
            end = len(data) if size in (0, 0xFFFFFFFF) else min(body + size, len(data))
            end -= (end - body) % 2
            samples = np.frombuffer(data[body:end], dtype="<i2")
            return samples.astype(np.float32) / 32768.0, sample_rate
        pos = body + size + (size & 1)
    raise ValueError("no data chunk")


def write_wav(audio, sample_rate):
    pcm = (np.clip(audio, -1.0, 1.0) * 32767.0).astype("<i2")
    buffer = io.BytesIO()
    with wave.open(buffer, "wb") as out:
        out.setnchannels(1)
        out.setsampwidth(2)
        out.setframerate(sample_rate)
        out.writeframes(pcm.tobytes())
    return buffer.getvalue()


def make_handler(enhancer, max_seconds):
    # One recording at a time: a second one would only slow the first down.
    lock = threading.Lock()

    class Handler(BaseHTTPRequestHandler):
        def do_GET(self):
            if self.path == "/health":
                self.reply(200, "application/json", b'{"status":"ok"}')
            else:
                self.reply(404, "text/plain", b"not found")

        def do_POST(self):
            if self.path != "/v1/audio/enhance":
                return self.reply(404, "text/plain", b"not found")
            try:
                length = int(self.headers["Content-Length"])
            except (TypeError, ValueError):
                return self.reply(411, "text/plain", b"Content-Length required")
            if length <= 0:
                return self.reply(400, "text/plain", b"empty body")
            if length > max_seconds * 48000 * 2 + 1024:
                return self.reply(413, "text/plain", b"recording too long")
            try:
                audio, sample_rate = read_wav(self.rfile.read(length))
            except (ValueError, struct.error) as err:
                return self.reply(400, "text/plain", str(err).encode())
            if sample_rate not in SAMPLE_RATES:
                return self.reply(400, "text/plain", b"unsupported sample rate")
            seconds = len(audio) / sample_rate
            if seconds > max_seconds:
                return self.reply(413, "text/plain", b"recording too long")
            if len(audio) < sample_rate // 10:
                return self.reply(400, "text/plain", b"recording too short")

            with lock:
                started = time.monotonic()
                enhanced = None
                try:
                    enhanced = enhancer.enhance(audio, sample_rate)
                except Exception:
                    # Out of memory, or a CUDA library that couldn't get its
                    # workspace on the shared GPU.
                    log.exception("Failed to enhance %.1fs at %d Hz", seconds, sample_rate)
                # The GPU is shared with whisper.cpp and llama.cpp, so the memory
                # goes back between recordings. This runs after the exception is
                # gone, since its traceback holds on to the tensors.
                torch.cuda.empty_cache()
                elapsed = time.monotonic() - started
            if enhanced is None:
                return self.reply(500, "text/plain", b"enhancement failed")
            log.info(
                "Enhanced %.1fs at %d Hz in %.2fs (%.1fx realtime)",
                seconds,
                sample_rate,
                elapsed,
                seconds / elapsed,
            )
            self.reply(200, "audio/wav", write_wav(enhanced, sample_rate))

        def reply(self, status, content_type, body):
            self.send_response(status)
            self.send_header("Content-Type", content_type)
            self.send_header("Content-Length", str(len(body)))
            self.end_headers()
            self.wfile.write(body)

        def log_message(self, *args):
            pass

    return Handler


def main():
    parser = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    parser.add_argument("--model", default="nvidia/RE-USE")
    parser.add_argument("--host", default="0.0.0.0")
    parser.add_argument("--port", type=int, default=8083)
    # NVIDIA's inference_chunk.sh uses 5 second chunks that overlap by half.
    parser.add_argument("--chunk-seconds", type=float, default=5.0)
    parser.add_argument("--hop-portion", type=float, default=0.5)
    parser.add_argument("--max-seconds", type=int, default=600)
    args = parser.parse_args()

    logging.basicConfig(level=logging.INFO, format="[%(asctime)s][%(levelname)s] %(message)s")
    log.info("Loading %s", args.model)
    enhancer = Enhancer(args.model, args.chunk_seconds, args.hop_portion)
    # The first recording would otherwise also wait for cuFFT, cuBLAS and cuDNN
    # to load.
    noise = np.random.default_rng(0).standard_normal(16000).astype(np.float32) * 0.01
    enhancer.enhance(noise, 16000)
    torch.cuda.empty_cache()
    server = ThreadingHTTPServer((args.host, args.port), make_handler(enhancer, args.max_seconds))
    log.info("Listening on http://%s:%d/v1/audio/enhance", args.host, args.port)
    server.serve_forever()


if __name__ == "__main__":
    main()
