// The offline voice pipeline's decisions, with no engine in sight.
//
// Everything here is plain text in, plain text out: normalising a transcript, matching it against
// the closed phrase list, searching local records, and routing through the four named stages
// (HEART_DECISIONS 24): deterministic -> grammar -> grounded-model -> refusal. Speech recognition,
// synthesis and the language model live in engines.cpp and are handed in from outside, which is
// what lets the stage order and the "no records, no model" rule be tested without any of them.
#pragma once

#include <algorithm>
#include <cstdint>
#include <functional>
#include <map>
#include <optional>
#include <string>
#include <string_view>
#include <vector>

namespace mio {

// ---- text -------------------------------------------------------------------------------------

std::u32string to_utf32(std::string_view utf8);

/// Lower-cases, drops punctuation and diacritics, and folds the Arabic/Urdu letter variants that
/// are spelling choices rather than different words (ي/ی, ك/ک, ؤ/و ...). Applied to both sides of
/// every comparison in this file, and to both sides of a word-error-rate count.
std::string normalize(std::string_view utf8);

std::vector<std::string> split_words(std::string_view normalized);

bool has_arabic_script(std::string_view utf8);

/// Levenshtein distance. One function, used on code points to match phrases and on words to count
/// recognition errors.
template <class Seq>
size_t edit_distance(const Seq& a, const Seq& b) {
  std::vector<size_t> row(b.size() + 1);
  for (size_t j = 0; j <= b.size(); j++) row[j] = j;
  for (size_t i = 1; i <= a.size(); i++) {
    size_t diagonal = row[0];
    row[0] = i;
    for (size_t j = 1; j <= b.size(); j++) {
      size_t above = row[j];
      row[j] = std::min({above + 1, row[j - 1] + 1, diagonal + (a[i - 1] == b[j - 1] ? 0 : 1)});
      diagonal = above;
    }
  }
  return row[b.size()];
}

// ---- tables -----------------------------------------------------------------------------------

/// One row of config/phrases.tsv. `stage` says how the row is used:
///   wake                   the wake phrase as the recogniser spells it; found at the front of a
///                          transcript, and stripped
///   deterministic, grammar the closed list: the whole utterance must match the phrase
///   ask, brain             keywords: the utterance only has to contain the phrase
struct Phrase {
  std::string stage, intent, slot, lang;
  std::string text;      // normalised
  std::u32string chars;  // the same, as code points
};

/// Rows of a tab-separated file. Blank lines and lines starting with '#' are skipped; a row with
/// the wrong number of columns is an error naming the line, not a silent skip.
std::vector<std::vector<std::string>> read_tsv(const std::string& path, size_t columns);

struct Tables {
  std::vector<Phrase> phrases;
  std::map<std::string, std::string> replies;  // "key\tlang" -> template with {placeholders}

  static Tables load(const std::string& config_dir);

  using Vars = std::map<std::string, std::string>;
  std::string reply(const std::string& key, const std::string& lang, const Vars& vars = {}) const;
};

// ---- local records ----------------------------------------------------------------------------

/// A recent sighting, shaped like the P3.1 `obs.sighting` payload plus the spoken names.
struct Sighting {
  std::string label, region_id;
  std::map<std::string, std::string> names;  // lang -> what to call it aloud
  std::vector<std::string> aliases;          // normalised; what a person might call it
  int64_t seen_ms = 0;
  double confidence = 0;
  size_t matched = 0;  // set by search(): length of the longest alias found in the utterance
};

/// One `sched.window` occurrence.
struct Reminder {
  std::string occurrence_id;
  std::map<std::string, std::string> text;
  int64_t due_ms = 0;
};

/// What the robot knows locally. On the robot these come from telemetry, the capability manifest
/// and the local database; on a laptop they are fixture files (config/fixtures).
struct World {
  int battery_pct = 0;
  bool charging = false;
  std::string room;                                               // region id, empty if unknown
  std::vector<std::pair<std::string, std::string>> capabilities;  // id -> state
  std::vector<Sighting> sightings;
  std::vector<Reminder> reminders;

  /// Fixture times are relative ("seen 42 minutes ago") and are anchored to `now_ms` here, so a
  /// live demo always has a recent sighting and an upcoming reminder.
  static World load(const std::string& fixtures_dir, int64_t now_ms);
};

// ---- routing ----------------------------------------------------------------------------------

/// The only thing the language model is allowed to say: which retrieved row the person means
/// (-1 for none of them) and what they want to know about it. It never writes a sentence.
struct Pick {
  int row = -1;
  std::string ask;  // "where" | "when" | "none"
};

/// Called only when search() returned at least one row. An empty result means the model failed to
/// produce a valid structure.
using ModelFn =
    std::function<std::optional<Pick>(const std::string& utterance, const std::vector<Sighting>& rows)>;

struct Reply {
  std::string stage;    // deterministic | grammar | grounded-model | refusal
  std::string intent;   // battery, stop, goto, found, no_record, needs_brain, refusal, ...
  std::string slot;     // region id for goto; "label:ask" for found
  std::string lang;     // en | ur
  std::string text;     // what is spoken
  std::string command;  // what the grammar stage would dispatch on the robot; empty otherwise
  bool model_called = false;
};

/// How many of the leading `words` are a wake phrase (0 if the utterance does not open with one).
/// Spaces are ignored, so "he mio" and "hemio" are the same spelling. `max_edits` loosens the
/// match; use it only when the phrase is already known to be there.
size_t wake_length(const Tables& tables, const std::vector<std::string>& words, size_t max_edits = 0);

/// Removes a leading wake phrase ("hey mio, stop" -> "stop").
std::string strip_wake(const Tables& tables, const std::string& normalized, size_t max_edits = 0);

/// Lexical search over recent sightings: an alias of the row occurs in the utterance. Most recent
/// first. Deliberately no vectors — the same reasoning as BRAIN task B3.3.
std::vector<Sighting> search(const World& world, const std::string& normalized);

/// The no-model answer the language model has to beat: the row whose alias match is longest (then
/// most recent), and "when" only if the utterance contains a when-word.
Pick baseline_pick(const Tables& tables, const std::string& normalized,
                   const std::vector<Sighting>& rows);

struct RouteOptions {
  /// Largest edit distance, as a fraction of the longer string, at which an utterance still counts
  /// as a phrase from the closed list. Calibrated on recordings, not assumed.
  double match_threshold = 0.25;
  /// For the "repeat" command.
  const Reply* last = nullptr;
};

Reply route(const Tables& tables, const World& world, std::string_view transcript,
            const std::string& stt_lang, int64_t now_ms, const ModelFn& model,
            const RouteOptions& options = {});

}  // namespace mio
