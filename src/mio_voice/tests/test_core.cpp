// Checks for the routing logic, with no speech engine and no language model: the model is a
// lambda, which is how "it is never called when nothing was retrieved" becomes a counted fact.
//
// Run through ctest. MIO_VOICE_CONFIG_DIR is set by CMake to the repo's config directory, so these
// also fail if phrases.tsv or replies.tsv stops loading.
#include <cstdio>
#include <cstdlib>
#include <ctime>
#include <string>

#include "core.h"

using namespace mio;

static int failures = 0;

#define CHECK(condition)                                                    \
  do {                                                                      \
    if (!(condition)) {                                                     \
      std::printf("FAIL %s:%d  %s\n", __FILE__, __LINE__, #condition);      \
      failures++;                                                           \
    }                                                                       \
  } while (0)

#define CHECK_EQ(actual, expected)                                          \
  do {                                                                      \
    const std::string a = (actual), e = (expected);                         \
    if (a != e) {                                                           \
      std::printf("FAIL %s:%d  %s\n  expected: %s\n  actual:   %s\n",       \
                  __FILE__, __LINE__, #actual, e.c_str(), a.c_str());       \
      failures++;                                                           \
    }                                                                       \
  } while (0)

static bool contains(const std::string& text, const std::string& part) {
  return text.find(part) != std::string::npos;
}

int main() {
  // 2026-10-12 16:05:00 UTC. Pinned, with the zone, so the spoken time is the same everywhere.
  setenv("TZ", "UTC", 1);
  tzset();
  const int64_t now = 1791821100000;

  const std::string config = MIO_VOICE_CONFIG_DIR;
  const Tables tables = Tables::load(config);
  const World world = World::load(config + "/fixtures", now);

  int model_calls = 0;
  Pick next_pick{0, "where"};
  bool model_fails = false;
  const ModelFn model = [&](const std::string&, const std::vector<Sighting>&) -> std::optional<Pick> {
    model_calls++;
    if (model_fails) return std::nullopt;
    return next_pick;
  };
  auto say = [&](const std::string& text, const std::string& lang = "en") {
    return route(tables, world, text, lang, now, model);
  };

  // ---- text ----
  CHECK_EQ(normalize("  What's the TIME?! "), "whats the time");
  CHECK_EQ(normalize("کچن میں جاؤ۔"), normalize("کچن میں جاو"));    // hamza on waw is a spelling choice
  CHECK_EQ(normalize("كتني"), normalize("کتنی"));                  // Arabic kaf/yeh vs the Urdu ones
  CHECK_EQ(normalize("۷۶ فیصد"), "76 فیصد");                        // Urdu digits
  CHECK(has_arabic_script("بیٹری") && !has_arabic_script("battery"));
  CHECK(edit_distance(to_utf32("kitten"), to_utf32("sitting")) == 3);
  CHECK(edit_distance(split_words("go to the kitchen"), split_words("go to kitchen now")) == 2);
  CHECK(edit_distance(std::u32string(), to_utf32("abc")) == 3);
  CHECK_EQ(strip_wake(tables, "hey mio what time is it"), "what time is it");
  CHECK_EQ(strip_wake(tables, "mio stop"), "stop");
  CHECK_EQ(strip_wake(tables, "stop"), "stop");
  CHECK_EQ(strip_wake(tables, normalize("ہے میو، رک جاؤ")), normalize("رک جاؤ"));
  CHECK_EQ(strip_wake(tables, "he mio stop"), "stop");  // spaces inside the phrase do not matter
  // A spelling nobody listed is only forgiven when the phrase is already known to be there ...
  CHECK_EQ(strip_wake(tables, "haymio stop"), "haymio stop");
  CHECK_EQ(strip_wake(tables, "haymio stop", 2), "stop");
  // ... and never for the bare "mio", which within two edits is also "my".
  CHECK_EQ(strip_wake(tables, "my glasses", 2), "my glasses");
  CHECK(wake_length(tables, split_words("hey mio")) == 2);
  CHECK(wake_length(tables, split_words("hello there")) == 0);

  // ---- deterministic: a template, no model ----
  Reply r = say("Hey Mio, what is your battery level?");
  CHECK_EQ(r.stage, "deterministic");
  CHECK_EQ(r.intent, "battery");
  CHECK_EQ(r.text, "My battery is at 76 percent.");

  r = say("بیٹری کتنی ہے؟", "ur");
  CHECK_EQ(r.stage, "deterministic");
  CHECK_EQ(r.lang, "ur");
  CHECK(contains(r.text, "76"));

  r = say("what time is it");
  CHECK_EQ(r.text, "It is 4:05 PM.");
  r = say("کیا وقت ہوا ہے", "ur");
  CHECK(contains(r.text, "4") && contains(r.text, "شام"));

  r = say("what is my next reminder");
  CHECK_EQ(r.text, "Your next reminder is: take your medicine, at 4:40 PM.");

  r = say("which room are you in");
  CHECK_EQ(r.text, "I am in the kitchen.");

  r = say("what can you do now");
  CHECK_EQ(r.intent, "capabilities");
  CHECK(contains(r.text, "driving") && contains(r.text, "These do not: recording"));

  // ---- grammar: a fixed command, still no model ----
  r = say("Mio, stop.");
  CHECK_EQ(r.stage, "grammar");
  CHECK_EQ(r.command, "stop");

  r = say("go to the kitchen");
  CHECK_EQ(r.command, "goto kitchen");
  CHECK_EQ(r.text, "Going to the kitchen.");

  r = say("kitchen jao");  // code-switched, heard as English: answered in Urdu
  CHECK_EQ(r.command, "goto kitchen");
  CHECK_EQ(r.lang, "ur");

  r = say("لاؤنج میں جاو", "ur");
  CHECK_EQ(r.command, "goto living_room");

  r = say("go to the kichen");  // one slip is still the closed list
  CHECK_EQ(r.command, "goto kitchen");

  r = say("repeat that");
  CHECK_EQ(r.intent, "repeat");
  CHECK_EQ(r.text, "I have not said anything yet.");
  const Reply earlier = say("which room are you in");
  RouteOptions with_last;
  with_last.last = &earlier;
  r = route(tables, world, "say that again", "en", now, model, with_last);
  CHECK_EQ(r.text, earlier.text);

  CHECK(model_calls == 0);  // nothing above may have reached the model

  // ---- out-of-list speech is not forced onto the closed list ----
  r = say("i think we should stop buying so much sugar");
  CHECK_EQ(r.stage, "refusal");
  r = say("the kitchen was painted last year");
  CHECK_EQ(r.stage, "refusal");
  CHECK(model_calls == 0);

  // ---- the rule that cannot bend: nothing retrieved, no model ----
  r = say("where is my passport");
  CHECK_EQ(r.stage, "refusal");
  CHECK_EQ(r.intent, "no_record");
  CHECK(!r.model_called);
  r = say("میرا پاسپورٹ کہاں ہے", "ur");
  CHECK_EQ(r.intent, "no_record");
  CHECK_EQ(r.lang, "ur");
  r = say("turn on the lights");
  CHECK_EQ(r.intent, "needs_brain");
  r = say("tell me a joke about cats");
  CHECK_EQ(r.intent, "refusal");
  r = say("");
  CHECK_EQ(r.intent, "not_understood");
  r = say("hey mio");
  CHECK_EQ(r.intent, "not_understood");
  CHECK(model_calls == 0);

  // ---- grounded model: called only with rows, and it only picks ----
  CHECK(search(world, "where are my glasses").size() == 1);
  CHECK(search(world, "where is the remote").size() == 2);          // TV and AC remotes
  CHECK(search(world, "where is the remote")[0].label == "tv_remote");  // most recent first
  CHECK(search(world, normalize("میرا چشمہ کہاں ہے")).size() == 1);
  CHECK(search(world, "where are my glases").size() == 1);          // one slip on a long word
  CHECK(search(world, "where is my bat").empty());                  // but "bat" is not "bag"

  next_pick = {0, "where"};
  r = say("where are my glasses");
  CHECK(model_calls == 1 && r.model_called);
  CHECK_EQ(r.stage, "grounded-model");
  CHECK_EQ(r.slot, "glasses:where");
  CHECK_EQ(r.text, "I last saw the glasses in the living room, 42 minutes ago.");

  next_pick = {1, "when"};
  r = say("when did you last see the ac remote");
  CHECK_EQ(r.slot, "ac_remote:when");
  CHECK_EQ(r.text, "I last saw the AC remote 5 hours ago, in the bedroom.");

  next_pick = {0, "where"};
  r = say("میرا چشمہ کہاں ہے", "ur");
  CHECK_EQ(r.lang, "ur");
  CHECK(contains(r.text, "لاؤنج") && contains(r.text, "42"));

  next_pick = {-1, "none"};  // the model says none of the rows is meant: refuse, do not guess
  r = say("clean my glasses please");
  CHECK_EQ(r.stage, "refusal");
  CHECK(r.model_called);

  next_pick = {7, "where"};  // a row that does not exist
  r = say("where are my glasses");
  CHECK_EQ(r.stage, "refusal");

  next_pick = {-1, "where"};  // "gym bag" retrieves the school bag; the model says that is not it
  r = say("where is my gym bag");
  CHECK_EQ(r.intent, "no_record");
  next_pick = {0, "none"};
  r = say("call my phone");
  CHECK_EQ(r.intent, "needs_brain");

  model_fails = true;  // no valid structure came back
  r = say("where are my glasses");
  CHECK_EQ(r.intent, "not_understood");
  model_fails = false;

  // ---- the no-model baseline the language model has to beat ----
  {
    const std::string said = "where is the ac remote";
    const Pick pick = baseline_pick(tables, said, search(world, said));
    CHECK(pick.row == 1);  // longest alias wins over most recent
    CHECK_EQ(pick.ask, "where");
    const std::string asked = "when did you see my keys";
    CHECK_EQ(baseline_pick(tables, asked, search(world, asked)).ask, "when");
  }

  if (failures == 0) std::printf("ok\n");
  return failures == 0 ? 0 : 1;
}
