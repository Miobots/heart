// Thin wrappers around the engines the pipeline is built from. Each one wraps an existing library
// or binary and adds no behaviour of its own; the decisions live in core.cpp.
//
//   speech to text     whisper.cpp, linked in
//   wake word, VAD,    sherpa-onnx (one prebuilt library, C API); the voices are Piper voices
//   speech synthesis
//   language model     llama.cpp's `llama-completion`, one child process per request, so that
//                      "loads, answers, unloads" (HEART_DECISIONS §5) is simply a process lifetime
//   microphone,        `arecord` and `aplay` child processes, so this binary links no audio
//   speaker            library and contains no network code at all
#pragma once

#include <condition_variable>
#include <cstdint>
#include <deque>
#include <mutex>
#include <optional>
#include <string>
#include <thread>
#include <vector>

#include <nlohmann/json.hpp>

#include "core.h"

struct whisper_context;
struct whisper_state;
struct SherpaOnnxKeywordSpotter;
struct SherpaOnnxOnlineStream;
struct SherpaOnnxVoiceActivityDetector;
struct SherpaOnnxOfflineTts;

namespace mio {

constexpr int kSampleRate = 16000;  // what every recogniser here expects

double now_ms();

// ---- audio files ------------------------------------------------------------------------------

/// Reads a 16 kHz mono 16-bit PCM WAV, which is what scripts/record.sh writes. Anything else is an
/// error that says how to convert it, rather than a silent resample.
std::vector<float> read_wav(const std::string& path);
void write_wav(const std::string& path, const std::vector<float>& samples, int sample_rate);

// ---- speech to text ---------------------------------------------------------------------------

struct Heard {
  std::string text;
  std::string lang;                    // "en" or "ur", never anything else
  float p_en = 0, p_ur = 0, p_hi = 0;  // what the language detector said
  double detect_ms = 0, decode_ms = 0;
};

class Stt {
 public:
  /// `audio_ctx` shortens the encoder's window from 30 s (1500, or 0 for the default) to
  /// `audio_ctx / 50` seconds. The encoder is most of the cost and a command is a few seconds long,
  /// so this is the main speed lever — and it costs accuracy, so it is measured, not assumed.
  Stt(const std::string& model_path, int threads, int audio_ctx = 0);
  ~Stt();
  Stt(const Stt&) = delete;

  /// Which of the two languages this is. Only `lang`, the probabilities and `detect_ms` are set.
  Heard detect(std::vector<float> pcm);

  /// Pass a `guess` from detect() to decode in that language instead of detecting on `pcm` itself.
  Heard transcribe(std::vector<float> pcm, Heard guess = {});

  /// The first couple of seconds only, transcribed as English. This is how a wake phrase is found
  /// in speech: it is an English phrase whatever language follows it. Cheap — a 2.5 s encoder
  /// window instead of 30 s — so it can run on every stretch of speech.
  std::string opening(const std::vector<float>& pcm);

 private:
  whisper_context* ctx_;
  whisper_state* opening_state_;  // its own state, so its short window does not leak into transcribe()
  int threads_, audio_ctx_;
};

// ---- wake word --------------------------------------------------------------------------------

class WakeWord {
 public:
  WakeWord(const std::string& model_dir, const std::string& keywords_file, int max_active_paths);
  ~WakeWord();
  WakeWord(const WakeWord&) = delete;

  struct Hit {
    std::string keyword;  // the @NAME from keywords.txt
    double end_s;         // seconds since the last reset() at which the phrase ended, estimated
  };
  std::optional<Hit> feed(const float* samples, int n);
  void reset();

 private:
  const SherpaOnnxKeywordSpotter* spotter_;
  const SherpaOnnxOnlineStream* stream_ = nullptr;
};

// ---- voice activity ---------------------------------------------------------------------------

class Vad {
 public:
  Vad(const std::string& model_path, float min_silence_s);
  ~Vad();
  Vad(const Vad&) = delete;

  void feed(const float* samples, int n);
  bool speaking() const;
  /// A finished stretch of speech, oldest first, or nothing if none has finished yet.
  std::optional<std::vector<float>> pop();
  void reset();

 private:
  const SherpaOnnxVoiceActivityDetector* vad_;
  std::vector<float> pending_;  // the model takes fixed windows; this holds the remainder
};

// ---- speech synthesis -------------------------------------------------------------------------

class Tts {
 public:
  /// `voice_dir` is an unpacked sherpa-onnx Piper voice: one .onnx, tokens.txt, espeak-ng-data.
  Tts(const std::string& voice_dir, int threads);
  ~Tts();
  Tts(const Tts&) = delete;

  struct Audio {
    std::vector<float> samples;
    int sample_rate = 0;
    double ms = 0;  // how long synthesis took
  };
  Audio say(const std::string& text) const;

 private:
  const SherpaOnnxOfflineTts* tts_;
};

// ---- child processes --------------------------------------------------------------------------

/// The microphone: a shell command that writes 16 kHz mono 16-bit PCM to stdout. A thread drains
/// it into memory, so the command never blocks while the pipeline is busy answering.
class Capture {
 public:
  explicit Capture(const std::string& shell_command);
  ~Capture();
  Capture(const Capture&) = delete;

  /// Blocks until `n` samples are available. False once the command has ended and they are not.
  bool read(std::vector<float>& out, size_t n);
  /// Drops everything captured so far — used after speaking, so the robot does not hear itself.
  void flush();

 private:
  pid_t pid_ = -1;
  int fd_ = -1;
  std::thread reader_;
  std::mutex mutex_;
  std::condition_variable ready_;
  std::deque<float> queue_;
  bool ended_ = false;
};

/// The speaker: a shell command that reads mono 16-bit PCM from stdin; `{rate}` in it is replaced
/// by the sample rate. Returns when playback has finished.
void play(const std::string& shell_command, const std::vector<float>& samples, int sample_rate);

// ---- language model ---------------------------------------------------------------------------

class LanguageModel {
 public:
  LanguageModel(std::string llama_dir, std::string gguf_path, int threads);

  struct Run {
    std::optional<nlohmann::json> json;  // empty if the output was not valid JSON
    std::string raw;
    double ms = 0;         // spawn to exit: load + answer + unload
    long peak_rss_kb = 0;  // of the child
    int exit_code = 0;
  };
  /// One child process, constrained to `schema`, gone when this returns.
  Run complete(const std::string& prompt, const nlohmann::json& schema, int max_tokens = 48);

  /// Every child ever started. "No records, no model" is checked by reading this.
  int spawns = 0;
  Run last;

  const std::string& name() const { return name_; }

 private:
  std::string llama_dir_, gguf_path_, name_;
  int threads_;
};

/// The grounded-model stage's one question: which retrieved row, and what about it.
std::optional<Pick> pick_row(LanguageModel& model, const std::string& utterance,
                             const std::vector<Sighting>& rows);

/// Offline reminder creation, measured only (SRS UC-11 step 3): a sentence in, slots out.
struct ReminderSlots {
  std::string text, day;
  int hour = -1, minute = -1;
};
std::optional<ReminderSlots> extract_reminder(LanguageModel& model, const std::string& sentence,
                                              int now_hour, int now_minute);

}  // namespace mio
