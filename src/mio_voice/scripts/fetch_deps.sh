#!/usr/bin/env bash
# Fetches every engine and model the offline voice pipeline needs.
#
# This script is the ONLY part of mio_voice that touches the network. The binary has no network
# code and CMake downloads nothing, which is what lets "no audio leaves the device" be checked by
# running the whole thing inside a network-less sandbox (see README).
#
# Everything lands under $MIO_VOICE_HOME (default ~/.cache/mio_voice), deliberately outside the
# repo: the workspace lives on an NTFS volume with a space in its path, and model files do not
# belong in git.
#
#   ./fetch_deps.sh        engines, speech models and the default language model (~2.2 GB)
#   ./fetch_deps.sh all    plus the other language-model candidates compared in VOICE_EVAL (~4 GB)
#
# Checksums are the ones the publisher states (GitHub asset digest, Hugging Face LFS oid). The two
# marked "first download" have no published digest, so theirs were taken on 2026-10-12.
set -euo pipefail

ROOT="${MIO_VOICE_HOME:-$HOME/.cache/mio_voice}"
DL="$ROOT/dl"
mkdir -p "$DL" "$ROOT/models"

GH=https://github.com
HF=https://huggingface.co
SHERPA=$GH/k2-fsa/sherpa-onnx/releases/download

# fetch <sha256> <url> — download into $DL (resuming a partial file), verify, print the local path.
fetch() {
  local sha=$1 url=$2 out="$DL/${2##*/}"
  # sha256sum reports a missing file on stdout even under --status, and stdout is the return value.
  if ! echo "$sha  $out" | sha256sum -c --status >/dev/null 2>&1; then
    echo "fetching ${url##*/}" >&2
    curl -fL --retry 3 --retry-delay 5 -C - -o "$out" "$url" >&2
    if ! echo "$sha  $out" | sha256sum -c --status >/dev/null 2>&1; then
      rm -f "$out"
      echo "checksum mismatch, deleted: ${url##*/}" >&2
      exit 1
    fi
  fi
  echo "$out"
}

# unpack <archive> <dir> — extract with the archive's top-level directory stripped.
unpack() {
  [ -e "$2/.unpacked" ] && return 0
  mkdir -p "$2"
  tar -xf "$1" -C "$2" --strip-components=1
  touch "$2/.unpacked"
}

# model <sha256> <url> — a single file that is used as downloaded.
model() {
  local path
  path=$(fetch "$1" "$2")
  ln -sf "$path" "$ROOT/models/${2##*/}"
}

# --- engines -----------------------------------------------------------------------------------

# Wake word, voice activity detection and speech synthesis: one prebuilt library, C API.
unpack "$(fetch c0bdb7907d3a74bba1d55d22bf4d9fa75586cf1530614ebe88a27b9118e015c4 \
  "$SHERPA/v1.13.8/sherpa-onnx-v1.13.8-linux-x64-shared.tar.bz2")" "$ROOT/sherpa-onnx"

# Speech to text. Source, because it is linked into the binary (first download).
unpack "$(fetch ff1a9053feb509ff9d7729703355541ae9690073a6b1c40eb692c962e0dc1720 \
  "$GH/ggml-org/whisper.cpp/archive/refs/tags/v1.9.5.tar.gz")" "$ROOT/whisper.cpp"

# Prebuilt whisper-cli, only for experiments that are not worth linking in (grammar decoding).
unpack "$(fetch a72becf15d7917f990f6313867a52638b82b7f9ef237fb0c980dac56a135781c \
  "$GH/ggml-org/whisper.cpp/releases/download/b5454/whisper-bin-ubuntu-x64.tar.gz")" "$ROOT/whisper-bin"

# The small language model runs as a child process, so "unload" is the process exiting.
unpack "$(fetch f6d25dde8f51133143d1453da4fd5f73b145127177612a283bf7995957af3392 \
  "$GH/ggml-org/llama.cpp/releases/download/b11429/llama-b11429-bin-ubuntu-x64.tar.gz")" "$ROOT/llama"

# --- speech models -----------------------------------------------------------------------------

# English keyword-spotting model; "Hey Mio" is spelled in its tokens (first download).
unpack "$(fetch f170013b4716e41b62b9bfd809687c207cef798ef9bc6534d524e17af9b6561a \
  "$SHERPA/kws-models/sherpa-onnx-kws-zipformer-gigaspeech-3.3M-2024-01-01.tar.bz2")" "$ROOT/models/kws"

unpack "$(fetch de64e03109f3c896b7cf910db6d9ffd2703e3d665580d9a19bedbf3e0760d0e4 \
  "$SHERPA/tts-models/vits-piper-ur_PK-fasih-medium.tar.bz2")" "$ROOT/models/tts-ur"
unpack "$(fetch 9e3febfacf0abf4270172d2958bcec246032b7e88efc2720840cc80c93de334e \
  "$SHERPA/tts-models/vits-piper-en_US-lessac-medium.tar.bz2")" "$ROOT/models/tts-en"

model 9e2449e1087496d8d4caba907f23e0bd3f78d91fa552479bb9c23ac09cbb1fd6 \
  "$SHERPA/asr-models/silero_vad.onnx"

model 60ed5bc3dd14eea856493d334349b405782ddcaf0028d4b5df4088345fba2efe \
  "$HF/ggerganov/whisper.cpp/resolve/main/ggml-base.bin"
model 1be3a9b2063867b937e64e2ec7483364a79917e157fa98c5d94b5c1fffea987b \
  "$HF/ggerganov/whisper.cpp/resolve/main/ggml-small.bin"

# --- language models ---------------------------------------------------------------------------

model 6a1a2eb6d15622bf3c96857206351ba97e1af16c30d7a74ee38970e434e9407e \
  "$HF/Qwen/Qwen2.5-1.5B-Instruct-GGUF/resolve/main/qwen2.5-1.5b-instruct-q4_k_m.gguf"

if [ "${1:-}" = all ]; then
  model bd258782e35f7f458f8aced1adc053e6e92e89bc735ba3be89d38a06121dc517 \
    "$HF/unsloth/Qwen3.5-0.8B-GGUF/resolve/main/Qwen3.5-0.8B-Q4_K_M.gguf"
  model aaf42c8b7c3cab2bf3d69c355048d4a0ee9973d48f16c731c0520ee914699223 \
    "$HF/unsloth/Qwen3.5-2B-GGUF/resolve/main/Qwen3.5-2B-Q4_K_M.gguf"
fi

echo "ready: $ROOT"
