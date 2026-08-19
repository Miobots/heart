#!/usr/bin/env bash
# Self-test for guard.sh. Run it after touching the guard: .claude/hooks/guard.test.sh
# Exit 0 = every rule fires where it should and nowhere else.
set -uo pipefail
G="$(cd "$(dirname "$0")" && pwd)/guard.sh"
fail=0

check() { # name want_exit json
  printf '%s' "$3" | "$G" >/dev/null 2>&1
  got=$?
  if [ "$got" != "$2" ]; then
    echo "FAIL  $1  (want exit $2, got $got)"
    fail=1
  else
    echo "ok    $1"
  fi
}

check "bare colcon build is blocked" 2 \
  '{"tool_name":"Bash","tool_input":{"command":"colcon build --symlink-install"}}'
check "build.sh is allowed" 0 \
  '{"tool_name":"Bash","tool_input":{"command":"./build.sh --symlink-install"}}'
check "colcon with pinned interpreter is allowed" 0 \
  '{"tool_name":"Bash","tool_input":{"command":"colcon build --cmake-args -DPython3_EXECUTABLE=/usr/bin/python3"}}'
check "unrelated bash is allowed" 0 \
  '{"tool_name":"Bash","tool_input":{"command":"ros2 pkg list"}}'

check "python in safety supervisor is blocked" 2 \
  '{"tool_name":"Write","tool_input":{"file_path":"/w/src/mio_safety_supervisor/x.py","content":"x=1"}}'
check "python in gateway is blocked" 2 \
  '{"tool_name":"Write","tool_input":{"file_path":"/w/src/mio_gateway/x.py","content":"x=1"}}'
check "rust in safety supervisor is allowed" 0 \
  '{"tool_name":"Write","tool_input":{"file_path":"/w/src/mio_safety_supervisor/main.rs","content":"fn main(){}"}}'
check "python in perception is allowed" 0 \
  '{"tool_name":"Write","tool_input":{"file_path":"/w/src/mio_perception/node.py","content":"x=1"}}'

check "cmd_vel publisher in perception is blocked" 2 \
  '{"tool_name":"Write","tool_input":{"file_path":"/w/src/mio_perception/node.py","content":"self.create_publisher(Twist, \"cmd_vel\", 10)"}}'
check "detection publisher in perception is allowed" 0 \
  '{"tool_name":"Write","tool_input":{"file_path":"/w/src/mio_perception/node.py","content":"self.create_publisher(Detection2DArray, \"detections\", 10)"}}'
check "prose mentioning cmd_vel is allowed" 0 \
  '{"tool_name":"Write","tool_input":{"file_path":"/w/src/mio_perception/node.py","content":"# advisory only, never publishes cmd_vel"}}'
check "edit payload is checked too" 2 \
  '{"tool_name":"Edit","tool_input":{"file_path":"/w/src/mio_perception/node.py","new_string":"self.create_publisher(Twist, \"cmd_vel\", 10)"}}'

[ $fail -eq 0 ] && echo && echo "all guard checks pass"
exit $fail
