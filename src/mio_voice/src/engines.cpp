#include "engines.h"

#include <chrono>
#include <cmath>
#include <cstring>
#include <filesystem>
#include <fstream>
#include <stdexcept>

#include <fcntl.h>
#include <poll.h>
#include <signal.h>
#include <spawn.h>
#include <sys/resource.h>
#include <sys/wait.h>
#include <unistd.h>

#include <sherpa-onnx/c-api/c-api.h>
#include <whisper.h>

extern char** environ;

namespace mio {

double now_ms() {
  using namespace std::chrono;
  return duration<double, std::milli>(steady_clock::now().time_since_epoch()).count();
}

// ---- audio files ------------------------------------------------------------------------------

std::vector<float> read_wav(const std::string& path) {
  std::ifstream in(path, std::ios::binary);
  if (!in) throw std::runtime_error("cannot open " + path);
  const std::string bytes((std::istreambuf_iterator<char>(in)), std::istreambuf_iterator<char>());
  auto u16 = [&](size_t at) { return static_cast<uint16_t>(static_cast<unsigned char>(bytes[at]) |
                                                           static_cast<unsigned char>(bytes[at + 1]) << 8); };
  auto u32 = [&](size_t at) { return static_cast<uint32_t>(u16(at)) | static_cast<uint32_t>(u16(at + 2)) << 16; };

  const std::string wrong = path + ": expected a 16 kHz mono 16-bit PCM WAV. Convert with:  "
                                   "ffmpeg -i in.wav -ar 16000 -ac 1 -c:a pcm_s16le out.wav";
  if (bytes.size() < 12 || bytes.compare(0, 4, "RIFF") != 0 || bytes.compare(8, 4, "WAVE") != 0) {
    throw std::runtime_error(wrong);
  }
  bool format_ok = false;
  for (size_t at = 12; at + 8 <= bytes.size();) {
    const std::string id = bytes.substr(at, 4);
    const size_t size = u32(at + 4), body = at + 8;
    if (id == "fmt " && body + 16 <= bytes.size()) {
      format_ok = u16(body) == 1 && u16(body + 2) == 1 && u32(body + 4) == kSampleRate && u16(body + 14) == 16;
    } else if (id == "data") {
      if (!format_ok) throw std::runtime_error(wrong);
      const size_t n = std::min(size, bytes.size() - body) / 2;  // arecord writes size 0 when piped
      std::vector<float> samples(n);
      for (size_t i = 0; i < n; i++) samples[i] = static_cast<int16_t>(u16(body + 2 * i)) / 32768.0f;
      return samples;
    }
    at = body + size + (size & 1);
  }
  throw std::runtime_error(wrong);
}

static std::string to_pcm16(const std::vector<float>& samples) {
  std::string pcm(samples.size() * 2, '\0');
  for (size_t i = 0; i < samples.size(); i++) {
    const auto v = static_cast<int16_t>(std::lround(std::clamp(samples[i], -1.0f, 1.0f) * 32767.0f));
    pcm[2 * i] = static_cast<char>(v & 0xFF);
    pcm[2 * i + 1] = static_cast<char>((v >> 8) & 0xFF);
  }
  return pcm;
}

void write_wav(const std::string& path, const std::vector<float>& samples, int sample_rate) {
  const std::string pcm = to_pcm16(samples);
  std::ofstream out(path, std::ios::binary);
  if (!out) throw std::runtime_error("cannot write " + path);
  auto le = [&](uint32_t value, int bytes) {
    for (int i = 0; i < bytes; i++) out.put(static_cast<char>((value >> (8 * i)) & 0xFF));
  };
  out << "RIFF";
  le(36 + pcm.size(), 4);
  out << "WAVEfmt ";
  le(16, 4);
  le(1, 2);                // PCM
  le(1, 2);                // mono
  le(sample_rate, 4);
  le(sample_rate * 2, 4);  // bytes per second
  le(2, 2);                // bytes per frame
  le(16, 2);               // bits per sample
  out << "data";
  le(pcm.size(), 4);
  out << pcm;
}

// ---- speech to text ---------------------------------------------------------------------------

Stt::Stt(const std::string& model_path, int threads, int audio_ctx)
    : threads_(threads), audio_ctx_(audio_ctx) {
  whisper_log_set([](ggml_log_level, const char*, void*) {}, nullptr);  // results only
  whisper_context_params params = whisper_context_default_params();
  params.use_gpu = false;  // CPU, so laptop timings mean something for the Pi
  ctx_ = whisper_init_from_file_with_params(model_path.c_str(), params);
  if (!ctx_) throw std::runtime_error("cannot load speech model " + model_path);
  opening_state_ = whisper_init_state(ctx_);
  if (!opening_state_) throw std::runtime_error("speech model: out of memory");

  if (audio_ctx_ > 0) {
    // Language detection has no window setting of its own: it reuses whatever the last decode
    // used. One decode of silence makes the very first detection use the short window too.
    whisper_full_params params = whisper_full_default_params(WHISPER_SAMPLING_GREEDY);
    params.n_threads = threads_;
    params.audio_ctx = audio_ctx_;
    params.language = "en";
    params.print_progress = false;
    params.no_timestamps = true;
    const std::vector<float> silence(kSampleRate / 2, 0.0f);
    whisper_full(ctx_, params, silence.data(), static_cast<int>(silence.size()));
  }
}

Stt::~Stt() {
  whisper_free_state(opening_state_);
  whisper_free(ctx_);
}

std::string Stt::opening(const std::vector<float>& pcm) {
  constexpr double kSeconds = 2.5;
  std::vector<float> head(pcm.begin(), pcm.begin() + static_cast<std::ptrdiff_t>(
                                           std::min(pcm.size(), static_cast<size_t>(kSampleRate * kSeconds))));
  if (head.size() < static_cast<size_t>(kSampleRate)) head.resize(kSampleRate, 0.0f);

  whisper_full_params params = whisper_full_default_params(WHISPER_SAMPLING_GREEDY);
  params.n_threads = threads_;
  params.language = "en";
  params.audio_ctx = 128;  // 2.56 s: the encoder is the cost, and this is a twelfth of it
  params.no_context = true;
  params.single_segment = true;
  params.no_timestamps = true;
  params.suppress_nst = true;
  params.print_progress = false;
  params.print_realtime = false;
  params.print_timestamps = false;
  params.print_special = false;
  if (whisper_full_with_state(ctx_, opening_state_, params, head.data(), static_cast<int>(head.size())) != 0) {
    throw std::runtime_error("speech model: decoding failed");
  }
  std::string text;
  for (int i = 0; i < whisper_full_n_segments_from_state(opening_state_); i++) {
    text += whisper_full_get_segment_text_from_state(opening_state_, i);
  }
  return text;
}

Heard Stt::detect(std::vector<float> pcm) {
  if (pcm.size() < kSampleRate * 11 / 10) pcm.resize(kSampleRate * 11 / 10, 0.0f);
  Heard heard;
  const double start = now_ms();
  if (whisper_pcm_to_mel(ctx_, pcm.data(), static_cast<int>(pcm.size()), threads_) != 0) {
    throw std::runtime_error("speech model: could not compute the spectrogram");
  }
  std::vector<float> probabilities(whisper_lang_max_id() + 1, 0.0f);
  if (whisper_lang_auto_detect(ctx_, 0, threads_, probabilities.data()) < 0) {
    throw std::runtime_error("speech model: language detection failed");
  }
  heard.p_en = probabilities[whisper_lang_id("en")];
  heard.p_ur = probabilities[whisper_lang_id("ur")];
  heard.p_hi = probabilities[whisper_lang_id("hi")];
  // Only two answers are allowed. Left to itself the detector sometimes hears Urdu as Hindi and
  // writes Devanagari, which matches nothing in the phrase table; Hindi counts as Urdu here.
  heard.lang = heard.p_en > heard.p_ur + heard.p_hi ? "en" : "ur";
  heard.detect_ms = now_ms() - start;
  return heard;
}

Heard Stt::transcribe(std::vector<float> pcm, Heard heard) {
  // A second of padding costs nothing and "stop" is well under a second.
  if (pcm.size() < kSampleRate * 11 / 10) pcm.resize(kSampleRate * 11 / 10, 0.0f);
  if (heard.lang.empty()) heard = detect(pcm);

  const double start = now_ms();
  whisper_full_params params = whisper_full_default_params(WHISPER_SAMPLING_GREEDY);
  params.n_threads = threads_;
  params.language = heard.lang.c_str();
  params.no_context = true;
  params.single_segment = true;
  params.no_timestamps = true;
  params.print_progress = false;
  params.print_realtime = false;
  params.print_timestamps = false;
  params.print_special = false;
  params.suppress_nst = true;
  params.audio_ctx = audio_ctx_;
  if (whisper_full(ctx_, params, pcm.data(), static_cast<int>(pcm.size())) != 0) {
    throw std::runtime_error("speech model: decoding failed");
  }
  heard.text.clear();
  for (int i = 0; i < whisper_full_n_segments(ctx_); i++) heard.text += whisper_full_get_segment_text(ctx_, i);
  heard.decode_ms = now_ms() - start;

  const size_t first = heard.text.find_first_not_of(" \n\t");
  const size_t last = heard.text.find_last_not_of(" \n\t");
  heard.text = first == std::string::npos ? "" : heard.text.substr(first, last - first + 1);
  return heard;
}

// ---- wake word --------------------------------------------------------------------------------

WakeWord::WakeWord(const std::string& model_dir, const std::string& keywords_file, int max_active_paths) {
  // The full-precision files: on the same clips the quantised encoder missed noticeably more.
  const std::string encoder = model_dir + "/encoder-epoch-12-avg-2-chunk-16-left-64.onnx";
  const std::string decoder = model_dir + "/decoder-epoch-12-avg-2-chunk-16-left-64.onnx";
  const std::string joiner = model_dir + "/joiner-epoch-12-avg-2-chunk-16-left-64.onnx";
  const std::string tokens = model_dir + "/tokens.txt";

  SherpaOnnxKeywordSpotterConfig config;
  std::memset(&config, 0, sizeof config);
  config.feat_config.sample_rate = kSampleRate;
  config.feat_config.feature_dim = 80;
  config.model_config.transducer.encoder = encoder.c_str();
  config.model_config.transducer.decoder = decoder.c_str();
  config.model_config.transducer.joiner = joiner.c_str();
  config.model_config.tokens = tokens.c_str();
  config.model_config.provider = "cpu";
  config.model_config.num_threads = 1;
  config.max_active_paths = max_active_paths;
  config.num_trailing_blanks = 1;
  config.keywords_score = 1.0f;       // defaults; each line of keywords.txt sets its own
  config.keywords_threshold = 0.25f;
  config.keywords_file = keywords_file.c_str();
  spotter_ = SherpaOnnxCreateKeywordSpotter(&config);
  if (!spotter_) throw std::runtime_error("cannot load the wake-word model from " + model_dir);
  reset();
}

WakeWord::~WakeWord() {
  SherpaOnnxDestroyOnlineStream(stream_);
  SherpaOnnxDestroyKeywordSpotter(spotter_);
}

void WakeWord::reset() {
  // A fresh stream, not ResetKeywordStream: token timestamps then count from here.
  if (stream_) SherpaOnnxDestroyOnlineStream(stream_);
  stream_ = SherpaOnnxCreateKeywordStream(spotter_);
}

std::optional<WakeWord::Hit> WakeWord::feed(const float* samples, int n) {
  SherpaOnnxOnlineStreamAcceptWaveform(stream_, kSampleRate, samples, n);
  std::optional<Hit> hit;
  while (!hit && SherpaOnnxIsKeywordStreamReady(spotter_, stream_)) {
    SherpaOnnxDecodeKeywordStream(spotter_, stream_);
    const SherpaOnnxKeywordResult* result = SherpaOnnxGetKeywordResult(spotter_, stream_);
    if (result && result->keyword && result->keyword[0] != '\0') {
      // Timestamps mark where each token starts; the last one still has to be spoken.
      const double last_token = result->count > 0 ? result->timestamps[result->count - 1] : 0.0;
      hit = Hit{result->keyword, last_token + 0.25};
    }
    SherpaOnnxDestroyKeywordResult(result);
  }
  return hit;
}

// ---- voice activity ---------------------------------------------------------------------------

namespace {
constexpr int kVadWindow = 512;
}

Vad::Vad(const std::string& model_path, float min_silence_s) {
  SherpaOnnxVadModelConfig config;
  std::memset(&config, 0, sizeof config);
  config.silero_vad.model = model_path.c_str();
  config.silero_vad.threshold = 0.5f;
  config.silero_vad.min_silence_duration = min_silence_s;
  config.silero_vad.min_speech_duration = 0.25f;
  config.silero_vad.max_speech_duration = 12.0f;
  config.silero_vad.window_size = kVadWindow;
  config.sample_rate = kSampleRate;
  config.num_threads = 1;
  config.provider = "cpu";
  vad_ = SherpaOnnxCreateVoiceActivityDetector(&config, 30.0f);
  if (!vad_) throw std::runtime_error("cannot load the voice-activity model " + model_path);
}

Vad::~Vad() { SherpaOnnxDestroyVoiceActivityDetector(vad_); }

void Vad::feed(const float* samples, int n) {
  pending_.insert(pending_.end(), samples, samples + n);
  size_t used = 0;
  for (; used + kVadWindow <= pending_.size(); used += kVadWindow) {
    SherpaOnnxVoiceActivityDetectorAcceptWaveform(vad_, pending_.data() + used, kVadWindow);
  }
  pending_.erase(pending_.begin(), pending_.begin() + static_cast<std::ptrdiff_t>(used));
}

bool Vad::speaking() const { return SherpaOnnxVoiceActivityDetectorDetected(vad_) != 0; }

std::optional<std::vector<float>> Vad::pop() {
  if (SherpaOnnxVoiceActivityDetectorEmpty(vad_)) return std::nullopt;
  const SherpaOnnxSpeechSegment* segment = SherpaOnnxVoiceActivityDetectorFront(vad_);
  std::vector<float> samples(segment->samples, segment->samples + segment->n);
  SherpaOnnxDestroySpeechSegment(segment);
  SherpaOnnxVoiceActivityDetectorPop(vad_);
  return samples;
}

void Vad::reset() {
  SherpaOnnxVoiceActivityDetectorReset(vad_);
  pending_.clear();
}

// ---- speech synthesis -------------------------------------------------------------------------

Tts::Tts(const std::string& voice_dir, int threads) {
  std::string model;
  for (const auto& entry : std::filesystem::directory_iterator(voice_dir)) {
    if (entry.path().extension() == ".onnx") model = entry.path().string();
  }
  if (model.empty()) throw std::runtime_error("no .onnx voice in " + voice_dir);
  const std::string tokens = voice_dir + "/tokens.txt", data = voice_dir + "/espeak-ng-data";

  SherpaOnnxOfflineTtsConfig config;
  std::memset(&config, 0, sizeof config);
  config.model.vits.model = model.c_str();
  config.model.vits.tokens = tokens.c_str();
  config.model.vits.data_dir = data.c_str();
  config.model.vits.noise_scale = 0.667f;
  config.model.vits.noise_scale_w = 0.8f;
  config.model.vits.length_scale = 1.0f;
  config.model.num_threads = threads;
  config.model.provider = "cpu";
  config.max_num_sentences = 2;
  tts_ = SherpaOnnxCreateOfflineTts(&config);
  if (!tts_) throw std::runtime_error("cannot load the voice in " + voice_dir);
}

Tts::~Tts() { SherpaOnnxDestroyOfflineTts(tts_); }

Tts::Audio Tts::say(const std::string& text) const {
  const double start = now_ms();
  SherpaOnnxGenerationConfig config;
  std::memset(&config, 0, sizeof config);
  config.speed = 1.0f;
  config.silence_scale = 0.2f;
  const SherpaOnnxGeneratedAudio* generated =
      SherpaOnnxOfflineTtsGenerateWithConfig(tts_, text.c_str(), &config, nullptr, nullptr);
  if (!generated) throw std::runtime_error("speech synthesis failed");
  Audio audio;
  audio.samples.assign(generated->samples, generated->samples + generated->n);
  audio.sample_rate = generated->sample_rate;
  SherpaOnnxDestroyOfflineTtsGeneratedAudio(generated);
  audio.ms = now_ms() - start;
  return audio;
}

// ---- child processes --------------------------------------------------------------------------

namespace {

/// Starts `argv` with the given stdin/stdout (-1 leaves stdout alone, and gives stdin /dev/null).
/// `environment` replaces the child's environment when non-empty.
pid_t spawn(const std::vector<std::string>& argv, int stdin_fd, int stdout_fd,
            const std::string& stderr_path = "", const std::vector<std::string>& environment = {}) {
  posix_spawn_file_actions_t actions;
  posix_spawn_file_actions_init(&actions);
  if (stdin_fd >= 0) {
    posix_spawn_file_actions_adddup2(&actions, stdin_fd, 0);
  } else {
    posix_spawn_file_actions_addopen(&actions, 0, "/dev/null", O_RDONLY, 0);
  }
  if (stdout_fd >= 0) posix_spawn_file_actions_adddup2(&actions, stdout_fd, 1);
  if (!stderr_path.empty()) {
    posix_spawn_file_actions_addopen(&actions, 2, stderr_path.c_str(), O_WRONLY | O_CREAT | O_TRUNC, 0644);
  }

  std::vector<char*> args, env;
  for (const std::string& a : argv) args.push_back(const_cast<char*>(a.c_str()));
  args.push_back(nullptr);
  for (const std::string& e : environment) env.push_back(const_cast<char*>(e.c_str()));
  env.push_back(nullptr);

  pid_t pid = -1;
  const int error = posix_spawnp(&pid, argv[0].c_str(), &actions, nullptr, args.data(),
                                 environment.empty() ? environ : env.data());
  posix_spawn_file_actions_destroy(&actions);
  if (error != 0) throw std::runtime_error("cannot start " + argv[0] + ": " + std::strerror(error));
  return pid;
}

void make_pipe(int fds[2]) {
  if (pipe2(fds, O_CLOEXEC) != 0) throw std::runtime_error("pipe failed");
}

}  // namespace

Capture::Capture(const std::string& shell_command) {
  int fds[2];
  make_pipe(fds);
  pid_ = spawn({"/bin/sh", "-c", shell_command}, -1, fds[1]);
  close(fds[1]);
  fd_ = fds[0];

  reader_ = std::thread([this] {
    unsigned char buffer[4096];
    size_t held = 0;  // an odd byte carried over between reads
    while (true) {
      const ssize_t got = ::read(fd_, buffer + held, sizeof buffer - held);
      if (got <= 0) break;
      const size_t have = held + static_cast<size_t>(got);
      {
        std::lock_guard<std::mutex> lock(mutex_);
        for (size_t i = 0; i + 1 < have; i += 2) {
          queue_.push_back(static_cast<int16_t>(buffer[i] | buffer[i + 1] << 8) / 32768.0f);
        }
        // Nobody is reading while the pipeline answers; keep half a minute at most.
        while (queue_.size() > static_cast<size_t>(kSampleRate) * 30) queue_.pop_front();
      }
      ready_.notify_one();
      held = have % 2;
      if (held) buffer[0] = buffer[have - 1];
    }
    {
      std::lock_guard<std::mutex> lock(mutex_);
      ended_ = true;
    }
    ready_.notify_one();
  });
}

Capture::~Capture() {
  kill(pid_, SIGTERM);
  reader_.join();
  close(fd_);
  waitpid(pid_, nullptr, 0);
}

bool Capture::read(std::vector<float>& out, size_t n) {
  std::unique_lock<std::mutex> lock(mutex_);
  ready_.wait(lock, [&] { return queue_.size() >= n || ended_; });
  if (queue_.size() < n) return false;
  out.assign(queue_.begin(), queue_.begin() + static_cast<std::ptrdiff_t>(n));
  queue_.erase(queue_.begin(), queue_.begin() + static_cast<std::ptrdiff_t>(n));
  return true;
}

void Capture::flush() {
  std::lock_guard<std::mutex> lock(mutex_);
  queue_.clear();
}

void play(const std::string& shell_command, const std::vector<float>& samples, int sample_rate) {
  std::string command = shell_command;
  const std::string slot = "{rate}";
  if (const size_t at = command.find(slot); at != std::string::npos) {
    command.replace(at, slot.size(), std::to_string(sample_rate));
  }
  int fds[2];
  make_pipe(fds);
  const pid_t pid = spawn({"/bin/sh", "-c", command}, fds[0], -1);
  close(fds[0]);
  const std::string pcm = to_pcm16(samples);
  for (size_t sent = 0; sent < pcm.size();) {
    const ssize_t wrote = write(fds[1], pcm.data() + sent, pcm.size() - sent);
    if (wrote <= 0) break;  // the player went away; SIGPIPE is ignored in main
    sent += static_cast<size_t>(wrote);
  }
  close(fds[1]);
  waitpid(pid, nullptr, 0);
}

// ---- language model ---------------------------------------------------------------------------

LanguageModel::LanguageModel(std::string llama_dir, std::string gguf_path, int threads)
    : llama_dir_(std::move(llama_dir)), gguf_path_(std::move(gguf_path)),
      name_(std::filesystem::path(gguf_path_).stem().string()), threads_(threads) {
  if (!std::filesystem::exists(gguf_path_)) throw std::runtime_error("no language model at " + gguf_path_);
}

LanguageModel::Run LanguageModel::complete(const std::string& prompt, const nlohmann::json& schema,
                                           int max_tokens) {
  // The whole instruction goes in the user turn: some chat templates refuse a lone system message.
  // No shell is involved, so quotes and Urdu text in the prompt need no escaping.
  const std::vector<std::string> argv = {
      llama_dir_ + "/llama-completion", "-m", gguf_path_, "-st", "-p", prompt,
      "-j", schema.dump(), "--no-display-prompt", "--simple-io", "--no-warmup",
      "--temp", "0", "-n", std::to_string(max_tokens), "-c", "1024", "-ngl", "0",
      "-t", std::to_string(threads_), "--offline"};

  // The prebuilt binary finds its libraries next to itself; everything else the shell exported
  // (ROS puts a long list here) stays out.
  std::vector<std::string> environment;
  for (char** e = environ; *e; e++) {
    if (std::strncmp(*e, "LD_LIBRARY_PATH=", 16) != 0) environment.emplace_back(*e);
  }
  environment.push_back("LD_LIBRARY_PATH=" + llama_dir_);

  Run run;
  const double start = now_ms();
  int fds[2];
  make_pipe(fds);
  const pid_t pid = spawn(argv, -1, fds[1], llama_dir_ + "/../llama.log", environment);
  close(fds[1]);
  spawns++;

  const double deadline = start + 60000;
  char buffer[4096];
  while (true) {
    pollfd waiting{fds[0], POLLIN, 0};
    const int remaining = static_cast<int>(deadline - now_ms());
    if (remaining <= 0 || poll(&waiting, 1, remaining) <= 0) {
      kill(pid, SIGKILL);  // a model that hangs is treated like one that answered nonsense
      break;
    }
    const ssize_t got = ::read(fds[0], buffer, sizeof buffer);
    if (got <= 0) break;
    run.raw.append(buffer, static_cast<size_t>(got));
  }
  close(fds[0]);

  int status = 0;
  rusage usage{};
  wait4(pid, &status, 0, &usage);
  run.ms = now_ms() - start;
  run.peak_rss_kb = usage.ru_maxrss;
  run.exit_code = WIFEXITED(status) ? WEXITSTATUS(status) : -1;

  // The binary prints the completion and then an end-of-text marker; take the first JSON object.
  // ponytail: first '{' to the next '}' — right for the flat objects asked for here; a schema with
  // nested objects needs a real brace matcher.
  const size_t open = run.raw.find('{'), shut = run.raw.find('}', open == std::string::npos ? 0 : open);
  if (run.exit_code == 0 && open != std::string::npos && shut != std::string::npos) {
    nlohmann::json parsed = nlohmann::json::parse(run.raw.substr(open, shut - open + 1), nullptr, false);
    if (!parsed.is_discarded()) run.json = std::move(parsed);
  }
  last = run;
  return run;
}

std::optional<Pick> pick_row(LanguageModel& model, const std::string& utterance,
                             const std::vector<Sighting>& rows) {
  std::string prompt =
      "A home robot keeps a list of things it has seen around the house. A person just asked it "
      "something. Decide which row the person is asking about, and what they want to know.\n\n"
      "Answer with JSON only: {\"row\": N, \"ask\": \"where\" | \"when\" | \"none\"}\n"
      "- row: the number of the row the person means. Use -1 if none of the rows is what they mean.\n"
      "- ask: \"where\" if they want to know where the thing is, or whether the robot has seen it.\n"
      "  \"when\" if they want to know when or how long ago it was last seen.\n"
      "  \"none\" if they want anything else, such as bringing, cleaning, buying or switching it on.\n"
      "The person may speak English, Urdu, or both mixed. In Urdu, کہاں / کدھر / kahan means "
      "where, and کب / kab means when.\n\nRows:\n";
  nlohmann::json row_numbers = nlohmann::json::array({-1});
  for (size_t i = 0; i < rows.size(); i++) {
    const auto name = rows[i].names.find("en");
    prompt += std::to_string(i) + ": " + (name != rows[i].names.end() ? name->second : rows[i].label) + " (also called:";
    for (const std::string& alias : rows[i].aliases) prompt += " " + alias + ",";
    prompt.back() = ')';
    prompt += "\n";
    row_numbers.push_back(i);
  }
  prompt += "\nThe person said: \"" + utterance + "\"\n";

  const nlohmann::json schema = {
      {"type", "object"},
      {"properties", {{"row", {{"enum", row_numbers}}}, {"ask", {{"enum", {"where", "when", "none"}}}}}},
      {"required", {"row", "ask"}},
      {"additionalProperties", false}};

  const LanguageModel::Run run = model.complete(prompt, schema);
  if (!run.json || !run.json->contains("row") || !run.json->contains("ask") ||
      !(*run.json)["row"].is_number_integer() || !(*run.json)["ask"].is_string()) {
    return std::nullopt;
  }
  return Pick{(*run.json)["row"].get<int>(), (*run.json)["ask"].get<std::string>()};
}

std::optional<ReminderSlots> extract_reminder(LanguageModel& model, const std::string& sentence,
                                              int now_hour, int now_minute) {
  char clock[8];
  std::snprintf(clock, sizeof clock, "%02d:%02d", now_hour, now_minute);
  const std::string prompt =
      std::string("A person asked a home robot to set a reminder. Extract the reminder.\n\n") +
      "Answer with JSON only: {\"text\": \"...\", \"hour\": H, \"minute\": M, \"day\": \"today\" | \"tomorrow\"}\n"
      "- text: what to be reminded about, in the person's own words, without the time.\n"
      "- hour: 0 to 23, in 24-hour time. minute: 0 to 59.\n"
      "- day: \"tomorrow\" only if they said tomorrow (Urdu: کل / kal). Otherwise \"today\".\n"
      "The person may speak English, Urdu, or both mixed. Urdu time words: صبح / subah is morning, "
      "دوپہر / dopahar is afternoon, شام / shaam is evening, رات / raat is night, بجے / baje is "
      "o'clock, ساڑھے / saarhe is half past, سوا / sawa is quarter past, پونے / paune is quarter to.\n"
      "It is now " + clock + ".\n\nThe person said: \"" + sentence + "\"\n";

  const nlohmann::json schema = {
      {"type", "object"},
      {"properties",
       {{"text", {{"type", "string"}, {"maxLength", 80}}},
        {"hour", {{"type", "integer"}, {"minimum", 0}, {"maximum", 23}}},
        {"minute", {{"type", "integer"}, {"minimum", 0}, {"maximum", 59}}},
        {"day", {{"enum", {"today", "tomorrow"}}}}}},
      {"required", {"text", "hour", "minute", "day"}},
      {"additionalProperties", false}};

  const LanguageModel::Run run = model.complete(prompt, schema, 96);
  if (!run.json) return std::nullopt;
  const nlohmann::json& j = *run.json;
  if (!j.contains("text") || !j.contains("hour") || !j.contains("minute") || !j.contains("day") ||
      !j["hour"].is_number_integer() || !j["minute"].is_number_integer() || !j["text"].is_string() ||
      !j["day"].is_string()) {
    return std::nullopt;
  }
  ReminderSlots slots;
  slots.text = j["text"].get<std::string>();
  slots.day = j["day"].get<std::string>();
  slots.hour = j["hour"].get<int>();
  slots.minute = j["minute"].get<int>();
  return slots;
}

}  // namespace mio
