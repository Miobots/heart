#!/usr/bin/env bash
# Records the evaluation corpus: one clip per row of corpus/manifest.tsv, each spoken as
# "Hey Mio, <text>" so the same clips measure the wake phrase.
#
#   ./record.sh <speaker> [id-prefix]
#
#   <speaker>     a short name for the person; clips go to corpus/audio/<speaker>/<id>.wav
#   [id-prefix]   record only ids starting with this (g- for commands, f-ur for Urdu questions ...)
#
# Sit where you would normally talk to the robot from, about an arm's length or more from the
# microphone, in an ordinary room. Say each line the way you would actually say it — if the wording
# on screen is not how you speak, say it your way and fix the text in the manifest afterwards.
# Already-recorded ids are skipped, so the session can be stopped and resumed.
set -euo pipefail

speaker=${1:?usage: record.sh <speaker> [id-prefix]}
only=${2:-}
here=$(cd "$(dirname "$0")/.." && pwd)
out="$here/corpus/audio/$speaker"
mkdir -p "$out"

total=$(grep -vc '^#' "$here/corpus/manifest.tsv")
done_count=0
# The manifest is read on fd 3 so that the prompts below can read the keyboard.
while IFS=$'\t' read -r id _ _ lang _ _ _ text <&3; do
  [ -n "$id" ] && [[ $id != \#* ]] || continue
  [[ $id == "$only"* ]] || continue
  done_count=$((done_count + 1))
  [ -e "$out/$id.wav" ] && continue

  while true; do
    printf '\n[%s  %s  %d of %d]\n    Hey Mio,  %s\n' "$id" "$lang" "$done_count" "$total" "$text"
    read -r -p "Enter to record, s to skip, q to quit: " key
    case "$key" in
      q) exit 0 ;;
      s) continue 2 ;;
    esac
    arecord -q -f S16_LE -r 16000 -c 1 "$out/$id.part.wav" &
    recorder=$!
    read -r -p "  recording ... press Enter when you have finished speaking " _
    sleep 0.3  # do not clip the last word
    kill -INT "$recorder" 2>/dev/null || true
    wait "$recorder" 2>/dev/null || true
    read -r -p "  Enter to keep, r to redo, p to play it back: " key
    if [ "$key" = p ]; then
      aplay -q "$out/$id.part.wav" || true
      read -r -p "  Enter to keep, r to redo: " key
    fi
    if [ "$key" != r ]; then
      mv "$out/$id.part.wav" "$out/$id.wav"
      break
    fi
  done
done 3< "$here/corpus/manifest.tsv"

rm -f "$out"/*.part.wav
echo "recorded: $(ls "$out" | wc -l) clips in $out"
