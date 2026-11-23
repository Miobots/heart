#!/usr/bin/env bash
# Plays recordings into `mio_voice listen` as if a microphone were hearing them: at real speed,
# with quiet in between. The way to check the live path — what wakes it, what it ignores — without
# a person in the room.
#
#   MIO_VOICE=~/.cache/mio_voice/build/mio_voice ./replay.sh [listen options --] clip.wav ...
#
#   ./replay.sh corpus/audio/jalal/g-can-en1.wav corpus/audio/jalal/d-bat-ur1.wav
#   ./replay.sh --wake stt -- a.wav b.wav
#
# Clips are 16 kHz mono 16-bit, as record.sh makes them. Replies are printed, not spoken.
#
# `eval` does not cover this: it scores one recording at a time, cut to its speech. Only here does
# audio arrive as a stream — which is how it was found that the keyword spotter heard nothing when
# handed speech from its first sample (kLeadIn in src/engines.h).
set -euo pipefail

bin=${MIO_VOICE:-$HOME/.cache/mio_voice/build/mio_voice}
options=()
if printf '%s\n' "$@" | grep -qx -- '--'; then
  while [ "$1" != "--" ]; do options+=("$1"); shift; done
  shift
fi
[ $# -gt 0 ] || { sed -n '2,11p' "$0"; exit 2; }

# 8 s of quiet after each clip: once it has answered, `listen` throws away whatever the microphone
# heard meanwhile (it would be hearing itself), and answering takes a few seconds.
"$bin" listen --no-speak "${options[@]}" --capture-cmd "python3 -c '
import sys, time, wave
out, start, sent = sys.stdout.buffer, time.monotonic(), 0.0
def play(pcm):
    global sent
    for at in range(0, len(pcm), 3200):  # 100 ms at a time, no faster than a microphone
        out.write(pcm[at:at + 3200]); out.flush()
        sent += 0.1
        time.sleep(max(0.0, start + sent - time.monotonic()))
play(bytes(32000))
for path in sys.argv[1:]:
    w = wave.open(path)
    assert (w.getframerate(), w.getnchannels(), w.getsampwidth()) == (16000, 1, 2), path
    play(w.readframes(w.getnframes())); play(bytes(32000 * 8))
' $(printf '%q ' "$@")"
