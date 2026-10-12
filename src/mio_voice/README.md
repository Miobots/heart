# mio_voice — the offline voice pipeline, standalone (H4.0)

The whole offline voice chain on a laptop, with **no robot, no ROS and no network**: it wakes on
"Hey Mio", transcribes, answers from four stages in order of cost, and speaks the reply — in Urdu
and English.

Standalone first is deliberate (TASKS H4.0): debugging speech recognition and robot software at the
same time is how a fortnight disappears. H4.1–H4.4 wrap this as a ROS node on the robot.

## The four stages

Named, not numbered (HEART_DECISIONS 24). An utterance stops at the first stage that can take it.

| Stage | Handles | How |
|---|---|---|
| **deterministic** | battery, time, next reminder, which room, "what can you do now" | the utterance matches a phrase in `config/phrases.tsv`; the reply is a filled template. No model. |
| **grammar** | 15 fixed commands: stop, cancel, dock, mute, unmute, patrol start/pause/resume, go to kitchen / living room / bedroom, repeat, louder, quieter, sleep | same closed list. The command is printed, not dispatched — there is no robot here. |
| **grounded-model** | "where are my glasses", "when did you last see the AC remote" | lexical search over local records; the small model picks **which row** and **where/when**; a template speaks it. |
| **refusal** | everything else | says what it cannot do, and what still works. |

**The rule that cannot bend:** if the search returns nothing, the model is never started. It is a
child process, so this is checked by counting processes (`eval` prints the count, and
`tests/test_core.cpp` asserts it).

**The model never writes a sentence.** It may only emit `{"row": N, "ask": "where"|"when"|"none"}`
under a JSON-schema grammar. A wrong pick therefore shows up as an answer about the wrong, *named*
thing — never as an invented fact.

**No audio leaves the device, before the wake phrase or after it.** The binary contains no network
code and links no network library; `scripts/fetch_deps.sh` is the only thing here that touches the
network. See "Proving it is offline" below.

## What it is built from

| Part | Engine | Where |
|---|---|---|
| Speech to text | whisper.cpp v1.9.5, compiled in, CPU only | `Stt` in `src/engines.cpp` |
| Wake phrase | either the first 2.5 s of a stretch of speech, transcribed as English, opens with it (`--wake stt`, the default), or sherpa-onnx's keyword spotter fires (`--wake kws`, `config/keywords.txt`) | `opens_with_wake`, `after_wake` in `src/main.cpp` |
| End of speech | Silero VAD, through sherpa-onnx | `Vad` |
| Speech out | Piper voices `ur_PK-fasih-medium` and `en_US-lessac-medium`, on the sherpa-onnx runtime | `Tts` |
| Language model | a ~1B GGUF model run by llama.cpp's `llama-completion`, **one child process per request** — loads, answers, unloads (HEART_DECISIONS §5) | `LanguageModel` |
| Microphone, speaker | `arecord` and `aplay` child processes | `Capture`, `play` |

`src/core.cpp` holds every decision and links none of the engines; that is the part the tests cover.

### How a request is heard

"Hey Mio" is English whatever follows it, and that matters more than it looks:

1. Speech that does not open with the wake phrase is dropped — never transcribed in full, never
   printed.
2. The request's language is judged on the audio **after the first 0.9 s**. Judged from the start,
   "Hey Mio" followed by Urdu is called English and transcribed as nonsense.
3. Only two answers are allowed, English or Urdu. Left alone the detector often hears Urdu as Hindi
   and writes Devanagari; Hindi counts as Urdu.
4. The wake phrase is stripped from the transcript, using the spellings in the `wake` rows of
   `config/phrases.tsv`.
5. "Hey Mio" on its own arms the pipeline: the next thing said within 6 s is the request.

## Build

```bash
./scripts/fetch_deps.sh        # engines + models into ~/.cache/mio_voice (about 2.2 GB; "all" for 4 GB)
cmake -S . -B ~/.cache/mio_voice/build
cmake --build ~/.cache/mio_voice/build -j4
ctest --test-dir ~/.cache/mio_voice/build
```

Needs `nlohmann-json3-dev`, `cmake`, a C++17 compiler, and `arecord`/`aplay` at run time.

**Plain CMake, not colcon.** This directory carries `COLCON_IGNORE`, so `./build.sh` skips it.
Everything large lives under `~/.cache/mio_voice` (override with `MIO_VOICE_HOME`), outside the
repo.

## Use

```bash
mio=~/.cache/mio_voice/build/mio_voice

$mio run --text "where are my glasses"          # one utterance, typed
$mio run --wav clip.wav --play                  # one utterance, from a 16 kHz mono WAV
$mio listen                                     # live: say "Hey Mio, ..." — Ctrl-C to stop
$mio say --lang ur --text "السلام علیکم" --out hello.wav
```

What the robot "knows" on a laptop is `config/fixtures/`: battery, room and capabilities
(`state.json`), recent sightings (`sightings.json`) and upcoming reminders (`reminders.json`).
Times in them are relative, so a demo always has a recent sighting and a reminder ahead.

## Measuring it

The numbers come from **real recordings**, never from synthetic speech.

```bash
./scripts/record.sh jalal                      # 163 short clips, each shown as "Hey Mio, <text>"
$mio eval --audio corpus/audio/jalal --split test --out results.tsv
$mio eval --audio corpus/audio/jalal --split test --wake kws    # the same, waking by the spotter
$mio eval --background household.wav           # false wake-ups per hour on speech with no wake phrase
$mio eval --split test                         # routing and the model on the reference text alone
$mio eval --reminders corpus/reminders.tsv     # reminder slots from text (measured only)
```

`corpus/manifest.tsv` says what each utterance should do. Half the rows are `dev` (tune the phrase
table, thresholds and wake variants on these) and half are `test` (report these, never tune on
them). `eval` prints word error rate by language, closed-list accuracy and false accepts, wake
detection, how the model did against a no-model baseline, how many model processes were started
when nothing was retrieved (must be 0), and time from end of speech to reply by stage.

Recordings are gitignored (`corpus/audio/`): they are people's voices.

`scripts/synth.sh` builds a synthetic copy of the corpus with the pipeline's own voices. It is a
smoke test for the audio path, **not a measurement** — one clean synthetic speaker says nothing
about how the pipeline does on people.

Useful switches: `--whisper base|small`, `--audio-ctx N` (shorter encoder window: faster, less
accurate), `--llm <file.gguf>|none` (`none` = the no-model baseline), `--match-threshold X`,
`--wake stt|kws`, `--threads N`.

## Proving it is offline

```bash
bwrap --dev-bind / / --unshare-net $mio eval --audio corpus/audio/jalal --split test
bwrap --dev-bind / / --unshare-net $mio listen
```

Inside that sandbox there is only a loopback interface and DNS does not resolve; the pipeline runs
unchanged. (`unshare -rn` is blocked on Ubuntu by AppArmor; `bwrap` is not.)

## Tuning it to a voice

Everything likely to need tuning is a table, not code:

- `config/phrases.tsv` — the closed list, the wake-phrase spellings the recogniser produces
  (`wake` rows: "hemio", "hey meo" ...), and the keyword lists. A command or a wake phrase the
  recogniser hears slightly differently needs a row, not a code change.
- `config/replies.tsv` — every sentence it can say, in both languages.
- `config/keywords.txt` — the spotter's spellings of "Hey Mio" and "Mio", each with its own boost
  (`:`) and threshold (`#`). "Mio" alone carries the higher threshold (HEART_DECISIONS 27).
- `--match-threshold` — how far from a listed phrase still counts as that phrase.

## Not here (H4.1–H4.4)

The ROS node, numbers on the robot, barge-in, ducking, echo cancellation, halt-then-run, and
dispatching commands to navigation. Reminder creation by voice is measured (`--reminders`) but not
wired into the live pipeline.
