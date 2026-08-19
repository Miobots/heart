#!/usr/bin/env bash
# PreToolUse guard for miobots-heart.
#
# Enforces the three rules in CLAUDE.md that are mechanically checkable. CLAUDE.md is advisory —
# Claude reads it and usually complies. These three have a cost high enough that "usually" isn't
# good enough, so they are enforced here instead.
#
# Exit 2 blocks the tool call and sends stderr back to Claude as the reason.
# Self-test: .claude/hooks/guard.test.sh
set -uo pipefail

payload=$(cat)
tool=$(jq -r '.tool_name // empty' <<<"$payload")

block() { printf '%s\n' "$1" >&2; exit 2; }

case "$tool" in
  Bash)
    cmd=$(jq -r '.tool_input.command // empty' <<<"$payload")
    # Decision 30: ~/.local/bin/python3.11 shadows the system python3.14 ROS Lyrical was built
    # against, so a bare `colcon build` fails every ament package with a missing catkin_pkg that
    # looks like a broken ROS install. build.sh pins the interpreter.
    if grep -q 'colcon build' <<<"$cmd" \
      && ! grep -q 'Python3_EXECUTABLE' <<<"$cmd" \
      && ! grep -q 'build\.sh' <<<"$cmd"; then
      block "Blocked: bare 'colcon build'. On this workspace it fails every ament package with
'ModuleNotFoundError: No module named catkin_pkg' — the wrong Python interpreter, not a broken
ROS install (HEART_DECISIONS #30).

Use:  ./build.sh $(sed 's/.*colcon build//' <<<"$cmd")

Or pass -DPython3_EXECUTABLE=/usr/bin/python3 yourself if you need colcon directly."
    fi
    ;;

  Edit | Write | MultiEdit)
    path=$(jq -r '.tool_input.file_path // empty' <<<"$payload")
    content=$(jq -r '[.tool_input.content, .tool_input.new_string]
                     | map(select(. != null)) | join("\n")' <<<"$payload")

    # REQUIREMENT §6: no garbage-collected runtime in any control path. Not even temporarily.
    case "$path" in
      */src/mio_safety_supervisor/*.py | */src/mio_gateway/*.py | */src/mio_voice/*.py)
        block "Blocked: Python in a control-path package ($(basename "$(dirname "$path")")).

HEART_DECISIONS §6 is a REQUIREMENT, not a preference: an unpredictable GC pause landing between
a velocity command and the wheels cannot be tested away. Rust/C++ from day one, no prototype.

mio_perception is the one documented exception (decision 18) — it is advisory and never issues a
velocity. If this genuinely belongs there, put it there. If you are changing the rule, amend
HEART_DECISIONS first."
        ;;
    esac

    # Decision 18: mio_perception is allowed to be Python *because* it publishes detections and
    # nothing else. A velocity publisher here deletes the reason the exception is safe.
    case "$path" in
      */src/mio_perception/*)
        if grep -Eq 'create_publisher[^)]*(Twist|cmd_vel)' <<<"$content"; then
          block "Blocked: velocity publisher in mio_perception.

Python is permitted here only because perception publishes detections and nothing else (decision
18). A cmd_vel publisher makes it a control path, and the no-GC requirement applies again.

Publish a detection message and let the supervisor decide. CLAUDE.md: 'If you find yourself adding
a velocity publisher to it, stop.'"
        fi
        ;;
    esac
    ;;
esac

exit 0
