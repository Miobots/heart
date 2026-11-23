#include "core.h"

#include <cstdio>
#include <ctime>
#include <fstream>
#include <stdexcept>

#include <nlohmann/json.hpp>

namespace mio {

// ---- text -------------------------------------------------------------------------------------

std::u32string to_utf32(std::string_view s) {
  std::u32string out;
  for (size_t i = 0; i < s.size();) {
    const unsigned char c = s[i];
    const int n = c < 0x80 ? 1 : (c >> 5) == 0x6 ? 2 : (c >> 4) == 0xE ? 3 : (c >> 3) == 0x1E ? 4 : 0;
    if (n == 0 || i + n > s.size()) {  // a stray byte: skip it rather than fail on a bad transcript
      i++;
      continue;
    }
    char32_t cp = n == 1 ? c : c & (0xFF >> (n + 1));
    for (int k = 1; k < n; k++) cp = (cp << 6) | (static_cast<unsigned char>(s[i + k]) & 0x3F);
    out.push_back(cp);
    i += n;
  }
  return out;
}

static void append_utf8(std::string& out, char32_t cp) {
  if (cp < 0x80) {
    out += static_cast<char>(cp);
  } else if (cp < 0x800) {
    out += static_cast<char>(0xC0 | (cp >> 6));
    out += static_cast<char>(0x80 | (cp & 0x3F));
  } else if (cp < 0x10000) {
    out += static_cast<char>(0xE0 | (cp >> 12));
    out += static_cast<char>(0x80 | ((cp >> 6) & 0x3F));
    out += static_cast<char>(0x80 | (cp & 0x3F));
  } else {
    out += static_cast<char>(0xF0 | (cp >> 18));
    out += static_cast<char>(0x80 | ((cp >> 12) & 0x3F));
    out += static_cast<char>(0x80 | ((cp >> 6) & 0x3F));
    out += static_cast<char>(0x80 | (cp & 0x3F));
  }
}

namespace {

constexpr char32_t kDrop = 0, kSpace = U' ';

// Code points are written as numbers on purpose: several of these pairs are visually identical.
char32_t fold(char32_t c) {
  if (c < 0x80) {
    if (c >= 'A' && c <= 'Z') return c + ('a' - 'A');
    if ((c >= 'a' && c <= 'z') || (c >= '0' && c <= '9')) return c;
    return c == '\'' ? kDrop : kSpace;  // "what's" -> "whats"
  }
  switch (c) {
    case 0x064A:  // ARABIC YEH
    case 0x0649:  // ALEF MAKSURA
    case 0x0626:  // YEH WITH HAMZA
      return 0x06CC;  // FARSI YEH, the Urdu one
    case 0x0643:  // ARABIC KAF
      return 0x06A9;  // KEHEH
    case 0x0647:  // ARABIC HEH
    case 0x0629:  // TEH MARBUTA
    case 0x06C2:  // HEH GOAL WITH HAMZA
    case 0x06C3:  // TEH MARBUTA GOAL
    case 0x06D5:  // AE
      return 0x06C1;  // HEH GOAL
    case 0x0624:  // WAW WITH HAMZA
      return 0x0648;  // WAW
    case 0x0623:  // ALEF WITH HAMZA ABOVE
    case 0x0625:  // ALEF WITH HAMZA BELOW
    case 0x0671:  // ALEF WASLA
      return 0x0627;  // ALEF
    case 0x06D3:  // YEH BARREE WITH HAMZA
      return 0x06D2;  // YEH BARREE
    case 0x2018:
    case 0x2019:  // curly apostrophes
      return kDrop;
    case 0x060C:  // Arabic comma
    case 0x061B:  // Arabic semicolon
    case 0x061F:  // Arabic question mark
    case 0x06D4:  // Urdu full stop
    case 0x00A0:  // no-break space
    case 0x00AB:
    case 0x00BB:
    case 0x201C:
    case 0x201D:
    case 0x2026:
      return kSpace;
  }
  if (c >= 0x06F0 && c <= 0x06F9) return U'0' + (c - 0x06F0);  // Urdu digits
  if (c >= 0x0660 && c <= 0x0669) return U'0' + (c - 0x0660);  // Arabic-Indic digits
  if (c >= 0x066A && c <= 0x066D) return kSpace;               // Arabic percent, separators, star
  if (c >= 0x2010 && c <= 0x2015) return kSpace;               // dashes
  const bool mark = (c >= 0x064B && c <= 0x065F) || c == 0x0670 || (c >= 0x06D6 && c <= 0x06ED) ||
                    c == 0x0640 /* tatweel */;
  const bool invisible = (c >= 0x200B && c <= 0x200F) || (c >= 0x202A && c <= 0x202E) ||
                         c == 0x2060 || c == 0xFEFF;
  return mark || invisible ? kDrop : c;
}

std::vector<std::string> split(std::string_view s, char separator) {
  std::vector<std::string> parts;
  size_t start = 0;
  while (true) {
    const size_t end = s.find(separator, start);
    parts.emplace_back(s.substr(start, end - start));
    if (end == std::string_view::npos) return parts;
    start = end + 1;
  }
}

std::string join(const std::vector<std::string>& words, size_t from = 0) {
  std::string out;
  for (size_t i = from; i < words.size(); i++) {
    if (!out.empty()) out += ' ';
    out += words[i];
  }
  return out;
}

bool same_word(const std::string& heard, const std::string& wanted, bool fuzzy) {
  if (heard == wanted) return true;
  if (!fuzzy) return false;
  // One slip is forgiven on words long enough that a slip does not turn them into another word:
  // "glases" is glasses, but "bat" must not become "bag".
  const std::u32string a = to_utf32(heard), b = to_utf32(wanted);
  return b.size() >= 5 && edit_distance(a, b) <= 1;
}

/// Whether `phrase` (one or more words) occurs in `words` as a run.
bool contains(const std::vector<std::string>& words, const std::string& phrase, bool fuzzy) {
  const std::vector<std::string> wanted = split_words(phrase);
  if (wanted.empty() || wanted.size() > words.size()) return false;
  for (size_t at = 0; at + wanted.size() <= words.size(); at++) {
    bool all = true;
    for (size_t k = 0; k < wanted.size() && all; k++) all = same_word(words[at + k], wanted[k], fuzzy);
    if (all) return true;
  }
  return false;
}

/// The intent of the first keyword row of `stage` that the utterance contains, or "".
std::string keyword(const Tables& tables, const std::vector<std::string>& words,
                    const std::string& stage) {
  for (const Phrase& p : tables.phrases) {
    if (p.stage == stage && contains(words, p.text, false)) return p.intent;
  }
  return "";
}

nlohmann::json read_json(const std::string& path) {
  std::ifstream in(path);
  if (!in) throw std::runtime_error("cannot open " + path);
  return nlohmann::json::parse(in);
}

/// `{hour}`, `{minute}` (two digits), `{min}` (as a number), `{ampm}` and `{period}` for a moment.
Tables::Vars clock_vars(const Tables& tables, int64_t ms, const std::string& lang) {
  const time_t seconds = ms / 1000;
  tm local{};
  localtime_r(&seconds, &local);
  const int h = local.tm_hour;
  char padded[8];
  std::snprintf(padded, sizeof padded, "%02d", local.tm_min);
  const char* period = h >= 5 && h < 12    ? "clock.morning"
                       : h >= 12 && h < 16 ? "clock.afternoon"
                       : h >= 16 && h < 20 ? "clock.evening"
                                           : "clock.night";
  return {{"hour", std::to_string(h % 12 == 0 ? 12 : h % 12)},
          {"minute", padded},
          {"min", std::to_string(local.tm_min)},
          {"ampm", tables.reply(h < 12 ? "clock.am" : "clock.pm", lang)},
          {"period", tables.reply(period, lang)}};
}

std::string ago(const Tables& tables, int64_t then_ms, int64_t now_ms, const std::string& lang) {
  const int64_t minutes = std::max<int64_t>(1, (now_ms - then_ms) / 60000);
  const char* unit = minutes < 60 ? "minute" : minutes < 48 * 60 ? "hour" : "day";
  const int64_t n = minutes < 60 ? minutes : minutes < 48 * 60 ? minutes / 60 : minutes / (24 * 60);
  return tables.reply(std::string("ago.") + unit + (n == 1 ? "" : "s"), lang, {{"n", std::to_string(n)}});
}

std::string list(const Tables& tables, const std::vector<std::string>& items, const std::string& lang) {
  std::string out;
  for (size_t i = 0; i < items.size(); i++) {
    if (i > 0) {
      out += i + 1 == items.size() ? " " + tables.reply("list.and", lang) + " "
                                    : tables.reply("list.comma", lang) + " ";
    }
    out += items[i];
  }
  return out;
}

std::string room_name(const Tables& tables, const std::string& region_id, const std::string& lang) {
  return tables.reply("room." + region_id, lang);
}

}  // namespace

std::string normalize(std::string_view utf8) {
  std::string out;
  bool pending_space = false;
  for (char32_t c : to_utf32(utf8)) {
    c = fold(c);
    if (c == kDrop) continue;
    if (c == kSpace) {
      pending_space = !out.empty();
      continue;
    }
    if (pending_space) out += ' ';
    pending_space = false;
    append_utf8(out, c);
  }
  return out;
}

std::vector<std::string> split_words(std::string_view normalized) {
  if (normalized.empty()) return {};
  return split(normalized, ' ');
}

bool has_arabic_script(std::string_view utf8) {
  for (char32_t c : to_utf32(utf8)) {
    if (c >= 0x0600 && c <= 0x06FF) return true;
  }
  return false;
}

// ---- tables -----------------------------------------------------------------------------------

std::vector<std::vector<std::string>> read_tsv(const std::string& path, size_t columns) {
  std::ifstream in(path);
  if (!in) throw std::runtime_error("cannot open " + path);
  std::vector<std::vector<std::string>> rows;
  std::string line;
  for (int number = 1; std::getline(in, line); number++) {
    if (!line.empty() && line.back() == '\r') line.pop_back();
    if (line.empty() || line[0] == '#') continue;
    rows.push_back(split(line, '\t'));
    if (rows.back().size() != columns) {
      throw std::runtime_error(path + ":" + std::to_string(number) + ": expected " +
                               std::to_string(columns) + " tab-separated columns, found " +
                               std::to_string(rows.back().size()));
    }
  }
  return rows;
}

Tables Tables::load(const std::string& config_dir) {
  Tables tables;
  for (const auto& row : read_tsv(config_dir + "/phrases.tsv", 5)) {
    Phrase p{row[0], row[1], row[2], row[3], normalize(row[4]), {}};
    p.chars = to_utf32(p.text);
    tables.phrases.push_back(std::move(p));
  }
  for (const auto& row : read_tsv(config_dir + "/replies.tsv", 3)) {
    tables.replies[row[0] + '\t' + row[1]] = row[2];
  }
  return tables;
}

std::string Tables::reply(const std::string& key, const std::string& lang, const Vars& vars) const {
  auto found = replies.find(key + '\t' + lang);
  if (found == replies.end()) found = replies.find(key + "\ten");
  if (found == replies.end()) throw std::runtime_error("no reply template for '" + key + "'");
  std::string text = found->second;
  for (const auto& [name, value] : vars) {
    const std::string slot = "{" + name + "}";
    for (size_t at = text.find(slot); at != std::string::npos; at = text.find(slot, at + value.size())) {
      text.replace(at, slot.size(), value);
    }
  }
  return text;
}

// ---- local records ----------------------------------------------------------------------------

World World::load(const std::string& dir, int64_t now_ms) {
  World world;

  const nlohmann::json state = read_json(dir + "/state.json");
  world.battery_pct = state.at("battery").at("percentage").get<int>();
  world.charging = state.at("battery").at("is_charging").get<bool>();
  world.room = state.value("room", "");
  for (const auto& [id, status] : state.at("capabilities").items()) {
    world.capabilities.emplace_back(id, status.at("state").get<std::string>());
  }

  for (const auto& item : read_json(dir + "/sightings.json")) {
    Sighting s;
    s.label = item.at("label").get<std::string>();
    s.region_id = item.at("region_id").get<std::string>();
    s.names = item.at("names").get<std::map<std::string, std::string>>();
    s.confidence = item.value("confidence", 0.0);
    s.seen_ms = now_ms - item.at("seen_min_ago").get<int64_t>() * 60000;
    for (const auto& alias : item.at("aliases")) s.aliases.push_back(normalize(alias.get<std::string>()));
    world.sightings.push_back(std::move(s));
  }

  for (const auto& item : read_json(dir + "/reminders.json")) {
    Reminder r;
    r.occurrence_id = item.at("occurrence_id").get<std::string>();
    r.text = item.at("text").get<std::map<std::string, std::string>>();
    r.due_ms = now_ms + item.at("due_in_min").get<int64_t>() * 60000;
    world.reminders.push_back(std::move(r));
  }
  return world;
}

// ---- routing ----------------------------------------------------------------------------------

size_t wake_length(const Tables& tables, const std::vector<std::string>& words, size_t max_edits) {
  size_t longest = 0;
  for (const Phrase& p : tables.phrases) {
    if (p.stage != "wake") continue;
    std::u32string wanted;
    for (char32_t c : p.chars) {
      if (c != U' ') wanted.push_back(c);
    }
    // A loose match is only safe against a phrase too long to swallow an ordinary word: "mio"
    // within two edits is also "my".
    const size_t slack = wanted.size() >= 5 ? max_edits : 0;
    std::u32string joined;
    for (size_t k = 1; k <= std::min<size_t>(words.size(), 3); k++) {
      joined += to_utf32(words[k - 1]);
      if (k > longest && edit_distance(joined, wanted) <= slack) longest = k;
    }
  }
  return longest;
}

std::string strip_wake(const Tables& tables, const std::string& normalized, size_t max_edits) {
  const std::vector<std::string> words = split_words(normalized);
  return join(words, wake_length(tables, words, max_edits));
}

std::vector<Sighting> search(const World& world, const std::string& normalized) {
  const std::vector<std::string> words = split_words(normalized);
  std::vector<Sighting> rows;
  for (const Sighting& s : world.sightings) {
    size_t matched = 0;
    for (const std::string& alias : s.aliases) {
      if (contains(words, alias, true)) matched = std::max(matched, to_utf32(alias).size());
    }
    if (matched == 0) continue;
    rows.push_back(s);
    rows.back().matched = matched;
  }
  std::stable_sort(rows.begin(), rows.end(),
                   [](const Sighting& a, const Sighting& b) { return a.seen_ms > b.seen_ms; });
  return rows;
}

Pick baseline_pick(const Tables& tables, const std::string& normalized,
                   const std::vector<Sighting>& rows) {
  Pick pick;
  size_t longest = 0;
  for (size_t i = 0; i < rows.size(); i++) {
    if (rows[i].matched > longest) {
      longest = rows[i].matched;
      pick.row = static_cast<int>(i);
    }
  }
  pick.ask = keyword(tables, split_words(normalized), "ask") == "when" ? "when" : "where";
  return pick;
}

Reply route(const Tables& tables, const World& world, std::string_view transcript,
            const std::string& stt_lang, int64_t now_ms, const ModelFn& model,
            const RouteOptions& options) {
  const std::string said = strip_wake(tables, normalize(transcript));
  const std::vector<std::string> words = split_words(said);

  Reply reply;
  reply.lang = has_arabic_script(said) || stt_lang == "ur" ? "ur" : "en";
  auto refuse = [&](const std::string& intent) {
    reply.stage = "refusal";
    reply.intent = intent;
    reply.text = tables.reply(intent, reply.lang);
    return reply;
  };

  if (said.empty()) return refuse("not_understood");

  // Deterministic and grammar: the whole utterance is (nearly) one phrase from the closed list.
  const std::u32string chars = to_utf32(said);
  const Phrase* match = nullptr;
  double closest = options.match_threshold;
  for (const Phrase& p : tables.phrases) {
    if (p.stage != "deterministic" && p.stage != "grammar") continue;
    const double distance = static_cast<double>(edit_distance(chars, p.chars)) /
                            static_cast<double>(std::max(chars.size(), p.chars.size()));
    if (distance <= closest && (!match || distance < closest)) {
      match = &p;
      closest = distance;
    }
  }

  if (match) {
    const std::string lang = match->lang == "en" ? "en" : "ur";
    reply.stage = match->stage;
    reply.intent = match->intent;
    reply.slot = match->slot == "-" ? "" : match->slot;
    reply.lang = lang;

    if (match->stage == "grammar") {
      reply.command = reply.slot.empty() ? reply.intent : reply.intent + " " + reply.slot;
      if (reply.intent == "repeat") {
        if (options.last) {
          reply.text = options.last->text;
          reply.lang = options.last->lang;
        } else {
          reply.text = tables.reply("cmd.repeat_none", lang);
        }
      } else if (reply.intent == "goto") {
        reply.text = tables.reply("cmd.goto", lang, {{"room", room_name(tables, reply.slot, lang)}});
      } else {
        reply.text = tables.reply("cmd." + reply.intent, lang);
      }
      return reply;
    }

    if (reply.intent == "battery") {
      reply.text = tables.reply(world.charging ? "battery_charging" : "battery", lang,
                                {{"pct", std::to_string(world.battery_pct)}});
    } else if (reply.intent == "time") {
      reply.text = tables.reply("time", lang, clock_vars(tables, now_ms, lang));
    } else if (reply.intent == "next_reminder") {
      const Reminder* next = nullptr;
      for (const Reminder& r : world.reminders) {
        if (r.due_ms > now_ms && (!next || r.due_ms < next->due_ms)) next = &r;
      }
      if (next) {
        Tables::Vars vars = clock_vars(tables, next->due_ms, lang);
        const auto text = next->text.find(lang);
        vars["text"] = text != next->text.end() ? text->second : next->text.begin()->second;
        reply.text = tables.reply("next_reminder", lang, vars);
      } else {
        reply.text = tables.reply("no_reminder", lang);
      }
    } else if (reply.intent == "room") {
      reply.text = world.room.empty()
                       ? tables.reply("room_unknown", lang)
                       : tables.reply("room", lang, {{"room", room_name(tables, world.room, lang)}});
    } else if (reply.intent == "capabilities") {
      std::vector<std::string> can, cannot;
      for (const auto& [id, state] : world.capabilities) {
        (state == "unavailable" ? cannot : can).push_back(tables.reply("cap." + id, lang));
      }
      reply.text = cannot.empty()
                       ? tables.reply("capabilities_all", lang, {{"can", list(tables, can, lang)}})
                       : tables.reply("capabilities", lang,
                                      {{"can", list(tables, can, lang)},
                                       {"cannot", list(tables, cannot, lang)}});
    } else {
      throw std::runtime_error("phrases.tsv has a deterministic intent with no handler: " + reply.intent);
    }
    return reply;
  }

  // Grounded model. The rule that cannot bend: nothing retrieved, no model. It is never asked to
  // answer from its own knowledge.
  const std::vector<Sighting> rows = search(world, said);
  if (rows.empty()) {
    if (!keyword(tables, words, "ask").empty()) return refuse("no_record");
    if (!keyword(tables, words, "brain").empty()) return refuse("needs_brain");
    return refuse("refusal");
  }

  reply.model_called = true;
  const std::optional<Pick> pick = model(said, rows);
  if (!pick) return refuse("not_understood");  // no valid structure: do not guess (SRS UC-11 alt 2)
  const bool usable = pick->row >= 0 && pick->row < static_cast<int>(rows.size()) &&
                      (pick->ask == "where" || pick->ask == "when");
  if (!usable) {
    // The rows shared a word with the request, but the model says none of them is the thing asked
    // after. For a where/when question that is the same answer as finding nothing.
    const bool none_of_them = pick->row < 0 || pick->row >= static_cast<int>(rows.size());
    if (none_of_them && !keyword(tables, words, "ask").empty()) return refuse("no_record");
    return refuse(keyword(tables, words, "brain").empty() ? "refusal" : "needs_brain");
  }

  const Sighting& row = rows[pick->row];
  const auto name = row.names.find(reply.lang);
  reply.stage = "grounded-model";
  reply.intent = "found";
  reply.slot = row.label + ":" + pick->ask;
  reply.text = tables.reply("found_" + pick->ask, reply.lang,
                            {{"label", name != row.names.end() ? name->second : row.label},
                             {"room", room_name(tables, row.region_id, reply.lang)},
                             {"ago", ago(tables, row.seen_ms, now_ms, reply.lang)}});
  return reply;
}

}  // namespace mio
