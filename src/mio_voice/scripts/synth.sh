#!/usr/bin/env bash
# Builds a synthetic copy of the corpus with the pipeline's own voices, as a regression smoke test:
# does a change break the audio path end to end?
#
# NOT a measurement. A synthetic voice is one clean speaker with no room, no accent and no
# hesitation, so numbers from these clips say nothing about how the pipeline does on people.
# The reported numbers come from scripts/record.sh recordings only.
#
#   ./synth.sh [out_dir]        default ~/.cache/mio_voice/synthetic
set -euo pipefail

here=$(cd "$(dirname "$0")/.." && pwd)
bin=${MIO_VOICE_BIN:-$HOME/.cache/mio_voice/build/mio_voice}
out=${1:-$HOME/.cache/mio_voice/synthetic}
mkdir -p "$out"
tmp=$(mktemp -d)
trap 'rm -rf "$tmp"' EXIT

grep -v '^#' "$here/corpus/manifest.tsv" | while IFS=$'\t' read -r id _ _ lang _ _ _ text; do
  [ -n "$id" ] || continue
  if [ "$lang" = en ]; then
    "$bin" say --lang en --text "Hey Mio, $text." --out "$tmp/clip.wav" </dev/null
    ffmpeg -loglevel error -y -i "$tmp/clip.wav" -ar 16000 -ac 1 -c:a pcm_s16le "$out/$id.wav" </dev/null
  else
    # A bilingual speaker says the wake phrase the same way whichever language follows. A fresh
    # rendition per clip: the voice varies between runs, and one reused take would make every
    # Urdu clip succeed or fail at waking together.
    "$bin" say --lang en --text "Hey Mio." --out "$tmp/wake.wav" </dev/null
    "$bin" say --lang ur --text "$text" --out "$tmp/said.wav" </dev/null
    ffmpeg -loglevel error -y -i "$tmp/wake.wav" -i "$tmp/said.wav" -filter_complex \
      "[0]aresample=16000[w];[1]aresample=16000,adelay=150[s];[w][s]concat=n=2:v=0:a=1" \
      -ac 1 -c:a pcm_s16le "$out/$id.wav" </dev/null
  fi
done
echo "synthetic corpus: $(ls "$out" | wc -l) clips in $out"
