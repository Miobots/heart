// mio_voice — the offline voice pipeline, standalone (H4.0). No robot, no ROS, no network.
//
//   mio_voice run    --text "..." | --wav clip.wav     one utterance through the four stages
//   mio_voice say    --lang ur --text "..." --out a.wav  synthesise a reply to a file
//   mio_voice eval   --manifest corpus/manifest.tsv ...  the numbers (see README)
//   mio_voice listen                                     live: wake phrase -> answer aloud
#include <chrono>
#include <cstdio>
#include <cstdlib>
#include <ctime>
#include <filesystem>
#include <map>
#include <memory>
#include <signal.h>

#include "core.h"
#include "engines.h"

using namespace mio;

namespace {

// ---- options ----------------------------------------------------------------------------------

struct Options {
  std::map<std::string, std::string> values;

  bool has(const std::string& key) const { return values.count(key) > 0; }
  std::string get(const std::string& key, const std::string& fallback = "") const {
    const auto found = values.find(key);
    return found == values.end() ? fallback : found->second;
  }
  double number(const std::string& key, double fallback) const {
    return has(key) ? std::atof(get(key).c_str()) : fallback;
  }
};

Options parse(int argc, char** argv) {
  Options options;
  for (int i = 2; i < argc; i++) {
    const std::string key = argv[i];
    if (key.rfind("--", 0) != 0) throw std::runtime_error("unexpected argument: " + key);
    const bool has_value = i + 1 < argc && std::string(argv[i + 1]).rfind("--", 0) != 0;
    options.values[key.substr(2)] = has_value ? argv[++i] : "1";
  }
  return options;
}

int64_t wall_ms() {
  using namespace std::chrono;
  return duration_cast<milliseconds>(system_clock::now().time_since_epoch()).count();
}

/// Everything a subcommand needs, with the engines loaded only when first asked for.
struct Pipeline {
  Options options;
  std::string home, config;
  int threads;
  int64_t now;
  Tables tables;
  World world;

  explicit Pipeline(Options o) : options(std::move(o)) {
    const char* env_home = std::getenv("MIO_VOICE_HOME");
    home = options.get("home", env_home ? env_home : std::string(std::getenv("HOME")) + "/.cache/mio_voice");
    config = options.get("config", MIO_VOICE_CONFIG_DIR);
    threads = static_cast<int>(options.number("threads", 4));
    now = wall_ms();
    tables = Tables::load(config);
    world = World::load(config + "/fixtures", now);
  }

  std::string model_path(const std::string& name) const {
    return name.find('/') != std::string::npos ? name : home + "/models/" + name;
  }

  Stt& stt() {
    if (!stt_) {
      const std::string name = options.get("whisper", "small");
      const bool is_path = name.find('/') != std::string::npos || name.find(".bin") != std::string::npos;
      stt_ = std::make_unique<Stt>(model_path(is_path ? name : "ggml-" + name + ".bin"), threads,
                                   static_cast<int>(options.number("audio-ctx", 0)));
    }
    return *stt_;
  }

  /// Null when run with `--llm none`: the grounded-model stage then uses the no-model baseline.
  LanguageModel* llm() {
    const std::string name = options.get("llm", "Qwen3.5-0.8B-Q4_K_M.gguf");
    if (name == "none") return nullptr;
    if (!llm_) llm_ = std::make_unique<LanguageModel>(home + "/llama", model_path(name), threads);
    return llm_.get();
  }

  ModelFn model() {
    if (LanguageModel* m = llm()) {
      return [m](const std::string& said, const std::vector<Sighting>& rows) { return pick_row(*m, said, rows); };
    }
    return [this](const std::string& said, const std::vector<Sighting>& rows) -> std::optional<Pick> {
      return baseline_pick(tables, said, rows);
    };
  }

  const Tts& voice(const std::string& lang) {
    std::unique_ptr<Tts>& slot = lang == "ur" ? tts_ur_ : tts_en_;
    if (!slot) slot = std::make_unique<Tts>(home + "/models/tts-" + (lang == "ur" ? "ur" : "en"), 2);
    return *slot;
  }

  WakeWord& wake() {
    if (!wake_) {
      wake_ = std::make_unique<WakeWord>(home + "/models/kws", config + "/keywords.txt",
                                         static_cast<int>(options.number("wake-paths", 8)));
    }
    return *wake_;
  }

  Vad& vad() {
    if (!vad_) vad_ = std::make_unique<Vad>(home + "/models/silero_vad.onnx",
                                            static_cast<float>(options.number("silence", 0.6)));
    return *vad_;
  }

  RouteOptions route_options(const Reply* last = nullptr) const {
    RouteOptions r;
    r.match_threshold = options.number("match-threshold", r.match_threshold);
    r.last = last;
    return r;
  }

 private:
  std::unique_ptr<Stt> stt_;
  std::unique_ptr<LanguageModel> llm_;
  std::unique_ptr<Tts> tts_en_, tts_ur_;
  std::unique_ptr<WakeWord> wake_;
  std::unique_ptr<Vad> vad_;
};

void print_reply(const Reply& reply) {
  std::printf("stage:   %s\nintent:  %s%s%s\nreply:   [%s] %s\n", reply.stage.c_str(), reply.intent.c_str(),
              reply.slot.empty() ? "" : " ", reply.slot.c_str(), reply.lang.c_str(), reply.text.c_str());
  if (!reply.command.empty()) std::printf("command: %s   (printed, not dispatched: no robot here)\n", reply.command.c_str());
}

void print_model(Pipeline& p, const Reply& reply, int spawns_before) {
  LanguageModel* m = p.llm();
  if (!reply.model_called) {
    std::printf("model:   not called\n");
  } else if (!m) {
    std::printf("model:   baseline rule (--llm none)\n");
  } else {
    std::printf("model:   %s, %d process, %.0f ms, peak %.0f MB, said %s\n", m->name().c_str(),
                m->spawns - spawns_before, m->last.ms, m->last.peak_rss_kb / 1024.0,
                m->last.json ? m->last.json->dump().c_str() : "(no valid structure)");
  }
}

// ---- the wake phrase ---------------------------------------------------------------------------

/// Whether `speech` opens with the wake phrase, judged on its first couple of seconds.
bool opens_with_wake(Pipeline& p, const std::vector<float>& speech) {
  return wake_length(p.tables, split_words(normalize(p.stt().opening(speech)))) > 0;
}

/// What was asked: the transcript, and its text normalised with any wake phrase removed.
struct Request {
  Heard heard;
  std::string text;
};

/// Transcribes speech that opens with the wake phrase.
///
/// The phrase is English whatever follows it, and the language detector takes its cue from the
/// first second — so "Hey Mio" followed by Urdu is called English and transcribed as nonsense. The
/// language is therefore judged on the audio after the first 0.9 s, and the phrase is then removed
/// from the transcript: loosely, because it is already known to be there.
Request after_wake_phrase(Pipeline& p, const std::vector<float>& speech) {
  constexpr size_t kSkip = kSampleRate * 9 / 10;
  Heard guess;
  if (speech.size() >= kSkip + kSampleRate / 2) {
    guess = p.stt().detect({speech.begin() + static_cast<std::ptrdiff_t>(kSkip), speech.end()});
  }
  const Heard heard = p.stt().transcribe(speech, guess);
  return {heard, strip_wake(p.tables, normalize(heard.text), 2)};
}

/// Transcribes speech already known to be a request: the spotter fired and the phrase was cut
/// off, or it follows a wake phrase that was said alone.
Request plain(Pipeline& p, const std::vector<float>& speech) {
  const Heard heard = p.stt().transcribe(speech);
  return {heard, strip_wake(p.tables, normalize(heard.text))};
}

/// Drops everything up to `seconds` from the front of `audio`.
void cut_front(std::vector<float>& audio, double seconds) {
  const size_t n = std::min(audio.size(), static_cast<size_t>(std::max(0.0, seconds) * kSampleRate));
  audio.erase(audio.begin(), audio.begin() + static_cast<std::ptrdiff_t>(n));
}

void print_heard(const Heard& heard) {
  std::printf("heard:   %s\n         (%s; en %.2f, ur %.2f, hi %.2f; detect %.0f ms, decode %.0f ms)\n",
              heard.text.c_str(), heard.lang.c_str(), heard.p_en, heard.p_ur, heard.p_hi,
              heard.detect_ms, heard.decode_ms);
}

// ---- run, say ---------------------------------------------------------------------------------

int run(Pipeline& p) {
  std::string text = p.options.get("text");
  std::string lang = p.options.get("lang", has_arabic_script(text) ? "ur" : "en");
  if (p.options.has("wav")) {
    const std::vector<float> clip = read_wav(p.options.get("wav"));
    const bool opened = opens_with_wake(p, clip);
    std::printf("wake:    %s\n", opened ? "the clip opens with the wake phrase" : "no wake phrase at the start");
    const Request request = opened ? after_wake_phrase(p, clip) : plain(p, clip);
    print_heard(request.heard);
    text = request.text;
    lang = request.heard.lang;
  } else if (!p.options.has("text")) {
    throw std::runtime_error("run needs --text \"...\" or --wav clip.wav");
  }

  const int spawns = p.llm() ? p.llm()->spawns : 0;
  const Reply reply = route(p.tables, p.world, text, lang, p.now, p.model(), p.route_options());
  print_reply(reply);
  print_model(p, reply, spawns);

  if (p.options.has("say") || p.options.has("play")) {
    const Tts::Audio audio = p.voice(reply.lang).say(reply.text);
    std::printf("speech:  %.1f s of audio in %.0f ms\n", audio.samples.size() / double(audio.sample_rate), audio.ms);
    if (p.options.has("say")) write_wav(p.options.get("say"), audio.samples, audio.sample_rate);
    if (p.options.has("play")) play(p.options.get("play-cmd", "aplay -q -t raw -f S16_LE -c 1 -r {rate} -"), audio.samples, audio.sample_rate);
  }
  return 0;
}

int say(Pipeline& p) {
  const std::string text = p.options.get("text");
  if (text.empty() || !p.options.has("out")) throw std::runtime_error("say needs --text \"...\" --out file.wav");
  const Tts::Audio audio = p.voice(p.options.get("lang", has_arabic_script(text) ? "ur" : "en")).say(text);
  write_wav(p.options.get("out"), audio.samples, audio.sample_rate);
  return 0;
}

// ---- eval -------------------------------------------------------------------------------------

struct Tally {
  int n = 0, ok = 0;
  void add(bool good) { n++, ok += good; }
  std::string str() const {
    char buffer[48];
    std::snprintf(buffer, sizeof buffer, "%d/%d", ok, n);
    return buffer;
  }
};

double percentile(std::vector<double> values, double fraction) {
  if (values.empty()) return 0;
  std::sort(values.begin(), values.end());
  return values[std::min(values.size() - 1, static_cast<size_t>(values.size() * fraction))];
}

/// Feeds a whole clip to the wake-word spotter, with a tail of silence so the last word is decoded.
std::optional<WakeWord::Hit> spot(WakeWord& wake, const std::vector<float>& clip) {
  wake.reset();
  std::vector<float> padded = clip;
  padded.resize(padded.size() + kSampleRate * 6 / 10, 0.0f);
  std::optional<WakeWord::Hit> hit;
  for (size_t at = 0; at < padded.size() && !hit; at += 1600) {
    hit = wake.feed(padded.data() + at, static_cast<int>(std::min<size_t>(1600, padded.size() - at)));
  }
  return hit;
}

/// How often the wake phrase fires on audio that does not contain it: a long recording of ordinary
/// household speech or television, streamed through both detectors.
int eval_background(Pipeline& p) {
  const std::vector<float> audio = read_wav(p.options.get("background"));
  const double hours = audio.size() / double(kSampleRate) / 3600.0;

  int spotter_fired = 0;
  p.wake().reset();
  for (size_t at = 0; at + 1600 <= audio.size(); at += 1600) {
    if (const auto hit = p.wake().feed(audio.data() + at, 1600)) {
      std::printf("  spotter fired at %.1f s: %s\n", at / double(kSampleRate), hit->keyword.c_str());
      spotter_fired++;
      p.wake().reset();
    }
  }

  int transcript_fired = 0, segments = 0;
  p.vad().reset();
  for (size_t at = 0; at + 1600 <= audio.size(); at += 1600) {
    p.vad().feed(audio.data() + at, 1600);
    while (const auto segment = p.vad().pop()) {
      segments++;
      if (opens_with_wake(p, *segment)) {
        std::printf("  transcript fired at %.1f s\n", at / double(kSampleRate));
        transcript_fired++;
      }
    }
  }

  std::printf("\n-- false triggers on %.2f hours with no wake phrase in it --\n", hours);
  std::printf("  keyword spotter:          %d  (%.1f per hour)\n", spotter_fired, spotter_fired / hours);
  std::printf("  speech opens with it:     %d  (%.1f per hour), over %d stretches of speech\n",
              transcript_fired, transcript_fired / hours, segments);
  return 0;
}

/// Offline reminder creation, measured only: sentence in, {text, hour, minute, day} out.
int eval_reminders(Pipeline& p) {
  LanguageModel* m = p.llm();
  if (!m) throw std::runtime_error("--reminders needs a language model");
  Tally valid, hour, minute, day, all;
  std::vector<double> ms;
  long peak = 0;
  // id | lang | now HH:MM | hour | minute | day | sentence
  for (const auto& row : read_tsv(p.options.get("reminders"), 7)) {
    const int now_h = std::atoi(row[2].substr(0, 2).c_str()), now_m = std::atoi(row[2].substr(3, 2).c_str());
    const auto slots = extract_reminder(*m, row[6], now_h, now_m);
    ms.push_back(m->last.ms);
    peak = std::max(peak, m->last.peak_rss_kb);
    const bool h = slots && slots->hour == std::atoi(row[3].c_str());
    const bool mi = slots && slots->minute == std::atoi(row[4].c_str());
    const bool d = slots && slots->day == row[5];
    valid.add(slots.has_value()), hour.add(h), minute.add(mi), day.add(d), all.add(h && mi && d);
    std::printf("  %-8s %s %s -> %s\n", row[0].c_str(), h && mi && d ? "ok  " : "MISS", row[6].c_str(),
                m->last.json ? m->last.json->dump().c_str() : "(no valid structure)");
  }
  std::printf("\n-- reminder slots from text (%s) --\n", m->name().c_str());
  std::printf("  valid structure %s   hour %s   minute %s   day %s   all three %s\n", valid.str().c_str(),
              hour.str().c_str(), minute.str().c_str(), day.str().c_str(), all.str().c_str());
  std::printf("  load+answer+unload: median %.0f ms, p90 %.0f ms; peak memory %.0f MB\n",
              percentile(ms, 0.5), percentile(ms, 0.9), peak / 1024.0);
  return 0;
}

int eval(Pipeline& p) {
  if (p.options.has("background")) return eval_background(p);
  if (p.options.has("reminders")) return eval_reminders(p);

  // id | split | bucket | lang | stage | intent | slot | text
  const auto manifest = read_tsv(p.options.get("manifest", p.config + "/../corpus/manifest.tsv"), 8);
  const std::string audio_dir = p.options.get("audio"), split = p.options.get("split", "all");
  const bool from_audio = !audio_dir.empty();
  // As in `listen`: the wake phrase is found by one of two detectors, cut out, and only what
  // follows it is transcribed. Both detectors are counted on every clip; `--wake` picks which
  // one's verdict the rest of the run follows.
  const bool by_spotter = p.options.get("wake", "stt") == "kws";
  LanguageModel* llm = p.llm();
  std::FILE* out = p.options.has("out") ? std::fopen(p.options.get("out").c_str(), "w") : nullptr;
  if (out) std::fprintf(out, "id\tbucket\tlang\twant\tgot\tok\tspotter\ttranscript_wake\theard\treply\n");

  // What the model was asked and what it said, for scoring it separately from the routing.
  struct Asked {
    bool called = false;
    std::optional<Pick> pick;
    std::vector<Sighting> rows;
    std::string said;
  } asked;
  const ModelFn inner = p.model();
  const ModelFn model = [&](const std::string& said, const std::vector<Sighting>& rows) {
    asked = {true, inner(said, rows), rows, said};
    return asked.pick;
  };

  std::map<std::string, Tally> stage_ok, intent_ok;  // by "bucket lang"
  std::map<std::string, std::pair<size_t, size_t>> wer;  // lang -> (errors, reference words)
  std::map<std::string, std::vector<double>> latency;    // stage -> ms from end of speech to reply
  std::vector<double> detect_ms, decode_ms, model_ms, tts_ms;
  Tally spotter, transcript_wake, end_to_end, in_list, false_accept;
  Tally model_valid, model_right, baseline_right;
  int nothing_retrieved = 0, started_with_nothing = 0, recorded = 0, wanted = 0;
  long model_peak = 0;

  for (const auto& row : manifest) {
    const std::string &id = row[0], &bucket = row[2], &lang = row[3], &text = row[7];
    if (split != "all" && row[1] != split) continue;
    wanted++;

    std::string heard_text = text, heard_lang = lang == "en" ? "en" : "ur";
    bool spotted = false, transcript_woke = false;
    double stt_ms = 0;
    if (from_audio) {
      const std::string path = audio_dir + "/" + id + ".wav";
      if (!std::filesystem::exists(path)) continue;
      std::vector<float> clip = read_wav(path);
      const auto hit = spot(p.wake(), clip);  // live, the spotter runs as the audio arrives: untimed
      spotted = hit.has_value();
      const double hearing_start = now_ms();
      transcript_woke = opens_with_wake(p, clip);
      // A clip the chosen detector missed would go unanswered live. It is transcribed anyway, as
      // if it had been caught, so that recognition and routing are scored on every clip.
      Request request;
      if (by_spotter && hit) {
        cut_front(clip, hit->end_s);
        request = plain(p, clip);
      } else {
        request = after_wake_phrase(p, clip);
      }
      const Heard& heard = request.heard;
      heard_lang = heard.lang;
      stt_ms = now_ms() - hearing_start;  // the wake check, language detection and decoding
      detect_ms.push_back(heard.detect_ms);
      decode_ms.push_back(heard.decode_ms);

      heard_text = request.text;
      const auto reference = split_words(normalize(text)), hypothesis = split_words(request.text);
      wer[lang].first += edit_distance(reference, hypothesis);
      wer[lang].second += reference.size();
      spotter.add(spotted);
      transcript_wake.add(transcript_woke);
    }
    recorded++;

    asked = {};
    const int spawns = llm ? llm->spawns : 0;
    const double route_start = now_ms();
    const Reply reply = route(p.tables, p.world, heard_text, heard_lang, p.now, model, p.route_options());
    double answer_ms = stt_ms + (now_ms() - route_start);

    if (from_audio && !p.options.has("no-tts")) {
      const double synthesis = p.voice(reply.lang).say(reply.text).ms;
      tts_ms.push_back(synthesis);
      answer_ms += synthesis;
    }
    latency[reply.stage].push_back(answer_ms);

    const bool stage_right = reply.stage == row[4];
    const bool right = stage_right && reply.intent == row[5] && (row[6] == "-" || reply.slot == row[6]);
    stage_ok[bucket + " " + lang].add(stage_right);
    intent_ok[bucket + " " + lang].add(right);
    if (from_audio) end_to_end.add(right && (by_spotter ? spotted : transcript_woke));

    const bool closed = row[4] == "deterministic" || row[4] == "grammar";
    if (closed) in_list.add(right);
    else false_accept.add(reply.stage == "deterministic" || reply.stage == "grammar");

    // The rule that cannot bend, counted from real process starts rather than trusted.
    if (search(p.world, strip_wake(p.tables, normalize(heard_text))).empty()) {
      nothing_retrieved++;
      started_with_nothing += (llm ? llm->spawns - spawns : 0) + (asked.called ? 1 : 0);
    }

    if (asked.called) {
      // Right means: the wanted row and question, or — when no row answers it — declining.
      const size_t colon = row[6].find(':');
      const bool want_found = row[5] == "found" && colon != std::string::npos;
      auto judge = [&](const std::optional<Pick>& pick) {
        if (!pick) return false;
        const bool usable = pick->row >= 0 && pick->row < static_cast<int>(asked.rows.size()) &&
                            (pick->ask == "where" || pick->ask == "when");
        if (!want_found) return !usable;
        return usable && asked.rows[pick->row].label == row[6].substr(0, colon) &&
               pick->ask == row[6].substr(colon + 1);
      };
      model_valid.add(asked.pick.has_value());
      model_right.add(judge(asked.pick));
      baseline_right.add(judge(baseline_pick(p.tables, asked.said, asked.rows)));
      if (llm) {
        model_ms.push_back(llm->last.ms);
        model_peak = std::max(model_peak, llm->last.peak_rss_kb);
      }
    }

    if (!right || p.options.has("verbose")) {
      std::printf("  %-10s %s want %s/%s/%s got %s/%s/%s | %s\n", id.c_str(), right ? "ok  " : "MISS",
                  row[4].c_str(), row[5].c_str(), row[6].c_str(), reply.stage.c_str(), reply.intent.c_str(),
                  reply.slot.empty() ? "-" : reply.slot.c_str(), heard_text.c_str());
    }
    if (out) {
      std::fprintf(out, "%s\t%s\t%s\t%s/%s/%s\t%s/%s/%s\t%d\t%d\t%d\t%s\t%s\n", id.c_str(), bucket.c_str(),
                   lang.c_str(), row[4].c_str(), row[5].c_str(), row[6].c_str(), reply.stage.c_str(),
                   reply.intent.c_str(), reply.slot.c_str(), right, spotted, transcript_woke,
                   heard_text.c_str(), reply.text.c_str());
    }
  }
  if (out) std::fclose(out);

  std::printf("\n== %s: %d of %d utterances (split %s) ==\n",
              from_audio ? ("recordings in " + audio_dir).c_str() : "reference text, no audio", recorded,
              wanted, split.c_str());
  std::printf("speech model %s (audio-ctx %s)   language model %s   match threshold %.2f\n",
              from_audio ? p.options.get("whisper", "small").c_str() : "-", p.options.get("audio-ctx", "full").c_str(),
              llm ? llm->name().c_str() : "none (baseline rule)", p.route_options().match_threshold);

  if (from_audio) {
    std::printf("\n-- wake phrase (every clip starts with it); this run follows the %s --\n",
                by_spotter ? "keyword spotter" : "transcript detector");
    std::printf("  keyword spotter fired                 %s\n", spotter.str().c_str());
    std::printf("  opening words are the wake phrase     %s\n", transcript_wake.str().c_str());
    std::printf("\n-- word error rate, both sides normalised --\n  %-6s %10s %8s %7s\n", "lang", "ref words", "errors", "WER");
    for (const auto& [lang, counts] : wer) {
      std::printf("  %-6s %10zu %8zu %6.1f%%\n", lang.c_str(), counts.second, counts.first,
                  counts.second ? 100.0 * counts.first / counts.second : 0.0);
    }
  }

  std::printf("\n-- routing --\n  %-22s %10s %16s\n", "bucket lang", "stage", "stage+intent+slot");
  for (const auto& [key, tally] : stage_ok) {
    std::printf("  %-22s %10s %16s\n", key.c_str(), tally.str().c_str(), intent_ok[key].str().c_str());
  }
  std::printf("\n-- closed list --\n");
  std::printf("  in-list utterances given the right answer        %s\n", in_list.str().c_str());
  std::printf("  out-of-list utterances wrongly taken as in-list  %s\n", false_accept.str().c_str());
  if (from_audio) std::printf("  wake heard AND right answer (what a person gets) %s\n", end_to_end.str().c_str());

  std::printf("\n-- grounded-model stage --\n");
  std::printf("  utterances with nothing retrieved: %d; model started for them: %d  (must be 0)\n",
              nothing_retrieved, started_with_nothing);
  std::printf("  asked %d times: valid structure %s, right %s; no-model baseline right %s\n", model_valid.n,
              model_valid.str().c_str(), model_right.str().c_str(), baseline_right.str().c_str());
  if (llm && !model_ms.empty()) {
    std::printf("  load+answer+unload: median %.0f ms, p90 %.0f ms; peak memory %.0f MB; processes started %d\n",
                percentile(model_ms, 0.5), percentile(model_ms, 0.9), model_peak / 1024.0, llm->spawns);
  }

  if (from_audio) {
    std::printf("\n-- time, this laptop, %d threads --\n", p.threads);
    std::printf("  language detection median %.0f ms; decoding median %.0f ms; synthesis median %.0f ms\n",
                percentile(detect_ms, 0.5), percentile(decode_ms, 0.5), percentile(tts_ms, 0.5));
    std::printf("  end of speech -> reply ready, by stage (median / p90 ms):\n");
    for (const auto& [stage, values] : latency) {
      std::printf("    %-16s %6.0f / %6.0f   (n=%zu)\n", stage.c_str(), percentile(values, 0.5),
                  percentile(values, 0.9), values.size());
    }
  }
  return 0;
}

// ---- listen -----------------------------------------------------------------------------------

constexpr size_t kChunk = kSampleRate / 10;

/// Waits for the keyword spotter, then returns the speech that follows the wake phrase — empty if
/// the phrase was said alone. Nothing if the microphone ended.
std::optional<std::vector<float>> after_wake(Capture& mic, WakeWord& wake, Vad& vad) {
  wake.reset();
  std::vector<float> recent, chunk;
  size_t dropped = 0;  // samples already trimmed off the front of `recent`
  while (mic.read(chunk, kChunk)) {
    recent.insert(recent.end(), chunk.begin(), chunk.end());
    const auto hit = wake.feed(chunk.data(), static_cast<int>(chunk.size()));
    if (!hit) {
      if (recent.size() > kSampleRate * 4) {  // only the last few seconds can hold a wake phrase
        const size_t extra = recent.size() - kSampleRate * 4;
        recent.erase(recent.begin(), recent.begin() + static_cast<std::ptrdiff_t>(extra));
        dropped += extra;
      }
      continue;
    }

    std::printf("\n[wake] %s\n", hit->keyword.c_str());
    std::fflush(stdout);
    const double start = std::max(0.0, hit->end_s * kSampleRate - static_cast<double>(dropped));
    std::vector<float> utterance(recent.begin() + std::min(recent.size(), static_cast<size_t>(start)), recent.end());
    vad.reset();
    vad.feed(utterance.data(), static_cast<int>(utterance.size()));
    bool spoke = false;
    double quiet_s = 0;
    while (utterance.size() < kSampleRate * 12 && mic.read(chunk, kChunk)) {
      utterance.insert(utterance.end(), chunk.begin(), chunk.end());
      vad.feed(chunk.data(), static_cast<int>(chunk.size()));
      bool finished = false;
      while (vad.pop()) finished = true;
      spoke = spoke || finished || vad.speaking();
      if (finished && !vad.speaking()) break;
      quiet_s = vad.speaking() ? 0 : quiet_s + 0.1;
      if (!spoke && quiet_s > 2.5) break;
    }
    // Silence must not reach the recogniser: asked to transcribe nothing, it invents something.
    if (!spoke) utterance.clear();
    return utterance;
  }
  return std::nullopt;
}

/// The next stretch of speech, or nothing if none begins within `timeout_s` (0 waits forever).
std::optional<std::vector<float>> next_speech(Capture& mic, Vad& vad, double timeout_s) {
  std::vector<float> chunk;
  double waited_s = 0;
  while (mic.read(chunk, kChunk)) {
    vad.feed(chunk.data(), static_cast<int>(chunk.size()));
    while (auto segment = vad.pop()) {
      if (segment->size() >= kSampleRate * 3 / 10) return segment;  // shorter is a click or a cough
    }
    waited_s += 0.1;
    if (timeout_s > 0 && waited_s > timeout_s && !vad.speaking()) return std::nullopt;
  }
  return std::nullopt;
}

int listen(Pipeline& p) {
  // --wake stt: the first seconds of every stretch of speech are checked for the wake phrase.
  // --wake kws: a small always-on keyword model decides instead.
  // Either way the phrase is cut out, only what follows is transcribed, the audio never leaves
  // this process, and speech that does not open with the phrase is dropped unprinted.
  const bool use_spotter = p.options.get("wake", "stt") == "kws";
  const std::string play_command = p.options.get("play-cmd", "aplay -q -t raw -f S16_LE -c 1 -r {rate} -");

  p.stt();
  p.voice("en");
  p.voice("ur");
  Vad& vad = p.vad();
  if (use_spotter) p.wake();
  Capture mic(p.options.get("capture-cmd", "arecord -q -t raw -f S16_LE -r 16000 -c 1 -"));
  std::printf("listening (%s). Say \"Hey Mio\", then a request. Ctrl-C to stop.\n",
              use_spotter ? "keyword spotter" : "wake phrase by transcript");
  std::fflush(stdout);

  bool armed = false, muted = false;  // armed: the wake phrase was said alone, the request follows
  float volume = 1.0f;
  std::optional<Reply> last;

  while (true) {
    const bool need_spotter = use_spotter && !armed;
    auto speech = need_spotter ? after_wake(mic, p.wake(), vad) : next_speech(mic, vad, armed ? 6.0 : 0.0);
    if (!speech) {
      if (armed) {  // the request never came
        armed = false;
        vad.reset();
        continue;
      }
      break;  // the microphone command ended
    }

    const double heard_at = now_ms();
    bool woke = need_spotter;
    Request request;
    if (need_spotter) {
      if (!speech->empty()) request = plain(p, *speech);
    } else {
      woke = opens_with_wake(p, *speech);
      if (!woke && !armed) continue;  // ordinary speech with no wake phrase: dropped, never printed
      if (woke) std::printf("\n[wake]\n");
      request = woke ? after_wake_phrase(p, *speech) : plain(p, *speech);
    }
    if (!request.heard.text.empty()) print_heard(request.heard);
    if (request.text.empty()) {
      armed = woke;  // "Hey Mio" on its own: the request follows. Otherwise there was nothing in it.
      if (armed) std::printf("         (wake phrase alone — waiting for the request)\n");
      std::fflush(stdout);
      continue;
    }
    armed = false;
    const Heard& heard = request.heard;
    const std::string& rest = request.text;

    const int spawns = p.llm() ? p.llm()->spawns : 0;
    const Reply reply = route(p.tables, p.world, rest, heard.lang, wall_ms(), p.model(),
                              p.route_options(last ? &*last : nullptr));
    print_reply(reply);
    print_model(p, reply, spawns);

    if (reply.intent == "unmute") muted = false;
    if (reply.intent == "louder") volume = std::min(volume * 1.5f, 3.0f);
    if (reply.intent == "quieter") volume = std::max(volume / 1.5f, 0.2f);
    if (!muted && !p.options.has("no-speak")) {
      Tts::Audio audio = p.voice(reply.lang).say(reply.text);
      std::printf("timing:  %.0f ms from end of speech to first sound\n", now_ms() - heard_at);
      std::fflush(stdout);
      for (float& sample : audio.samples) sample *= volume;
      play(play_command, audio.samples, audio.sample_rate);
    }
    if (reply.intent == "mute") muted = true;
    if (reply.intent != "repeat") last = reply;

    mic.flush();  // whatever the microphone picked up while speaking was the robot itself
    vad.reset();
    std::fflush(stdout);
  }
  return 0;
}

}  // namespace

int main(int argc, char** argv) {
  signal(SIGPIPE, SIG_IGN);  // a player that exits early must not take the pipeline with it
  const std::string command = argc > 1 ? argv[1] : "";
  try {
    if (command == "run" || command == "say" || command == "eval" || command == "listen") {
      Pipeline pipeline(parse(argc, argv));
      return command == "run" ? run(pipeline) : command == "say" ? say(pipeline)
             : command == "eval" ? eval(pipeline) : listen(pipeline);
    }
  } catch (const std::exception& error) {
    std::fprintf(stderr, "mio_voice: %s\n", error.what());
    return 1;
  }
  std::fprintf(stderr,
               "usage: mio_voice run    --text \"...\" | --wav clip.wav [--say out.wav] [--play]\n"
               "       mio_voice say    --text \"...\" --out file.wav [--lang en|ur]\n"
               "       mio_voice eval   [--manifest file] [--audio dir] [--split dev|test] [--out results.tsv]\n"
               "                        | --background long.wav | --reminders corpus/reminders.tsv\n"
               "       mio_voice listen [--wake kws|stt] [--no-speak]\n"
               "common: --whisper base|small  --audio-ctx N  --llm file.gguf|none  --threads N  --match-threshold X\n"
               "        --home DIR  --config DIR      (see README.md)\n");
  return command.empty() ? 1 : 2;
}
