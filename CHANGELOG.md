# Changelog — miobots-heart

Newest first. One entry per working session. Records what changed and, where it matters, what was
found to be wrong. See `HANDOFF.md` for the current state and the next step.

---

## 2026-08-13

**Phase 0.1 complete.** Nav2 and slam_toolbox build from source against ROS 2 Lyrical, and the
working commits are pinned. The task's one-week time-box was not needed.

### Added

- **`nav2-lyrical.repos`** — the deliverable. Pins `navigation2` at `1639b9f4` and `slam_toolbox`
  at `6953058`, both from their maintained `lyrical` branches, in vcstool format. Restore a machine
  with `vcs import src < nav2-lyrical.repos`.

### Verified

- `./build.sh --symlink-install --parallel-workers 4 --packages-up-to nav2_bringup slam_toolbox`
  → **46 packages, 17 min 43 s, exit 0, zero failures.** The 37 packages with stderr are CMake
  `PYTHON_EXECUTABLE was not used` warnings and upstream deprecation notices.
- Exit check passes: `ros2 pkg list` returns both `nav2_bringup` and `slam_toolbox`. 36 `nav2_*`
  packages installed alongside the 8 `mio_*`.
- `--parallel-workers 4` is safe at ~10 GB available RAM; peak usage left ~5 GB headroom. It is a
  memory cap, not a preference — drop to 2 below ~5 GB available.

### Found wrong

- **`ros-lyrical-ompl` was never a blocker.** The 2026-08-11 handoff stopped on it waiting for a
  sudo password. Only `nav2_route` and `nav2_smac_planner` need OMPL; system `libompl-dev` 1.6.0 has
  been installed since 2026-07-20 and ships `omplConfig.cmake`, so `find_package(ompl)` resolves
  without the ROS-keyed package. Both compiled with no sudo involved. `rosdep` asks for the ROS
  package; CMake does not care.

### Changed

- `HEART_DECISIONS.md` — decision 32 written, recording all four plan-vs-reality findings, the
  Release and `--parallel-workers` rationale, and the OMPL correction.
- `HEART_TASKS.md` 0.1 — dead `ros-planning` URL and the "no `lyrical` branch exists" instruction
  replaced with the correct clones; `colcon build` replaced with `./build.sh`.
- `.claude/skills/nav2-source-build/SKILL.md` — the same two stale facts corrected, the
  `--cmake-args` trap documented, API-drift downgraded as a likely failure class.
- `HANDOFF.md` rewritten for the post-0.1 state. Both it and this file were sitting under stray
  `.tmp.84611.*` filenames from an interrupted write; renamed.

---

## 2026-08-11

Phase 0.1 started: Nav2 and slam_toolbox from source against ROS 2 Lyrical. Agent tooling set up for
the repo beforehand.

### Fixed

- **`build.sh` never ran to completion.** `set -euo pipefail` was active while sourcing
  `/opt/ros/lyrical/setup.bash`, which reads `AMENT_TRACE_SETUP_FILES` without defaulting it; under
  `set -u` the script aborted before reaching colcon. `-u` is now enabled after the source. Decision
  30 records build.sh as verified working, so that verification was evidently done against the raw
  colcon command rather than the script.

### Changed

- `build.sh` now builds Release (`-DCMAKE_BUILD_TYPE=Release`), appended to its existing
  `--cmake-args` list. Debug symbols dominate a Nav2 source build and this partition has ~12 GB
  free. A comment records that callers must never pass `--cmake-args` themselves — colcon declares
  it `nargs='*'`, so a second occurrence replaces the interpreter pinning instead of appending, and
  silently reintroduces decision 30's `catkin_pkg` failure.
- `.gitignore` now excludes `src/navigation2/`, `src/slam_toolbox/` and `graphify-out/`.
- `README.md` gained an "Agent tooling" section covering what is committed and what each person
  installs themselves.

### Added

- `src/navigation2` at branch `lyrical`, HEAD `1639b9f4` (2026-08-06).
- `src/slam_toolbox` at branch `lyrical`, HEAD `6953058` (2026-07-21).
- `.claude/hooks/guard.sh` — PreToolUse. Blocks bare `colcon build` (decision 30), Python in the
  Rust control-path packages (§6 REQUIREMENT), and a `cmd_vel` publisher in `mio_perception`
  (decision 18's boundary). `.claude/hooks/guard.test.sh` covers it with 12 assertions.
- `.claude/hooks/rust-fmt.sh` — PostToolUse `cargo fmt` on the owning crate after a `.rs` edit.
- `.claude/settings.json` — the two hooks, a ROS/cargo permission allowlist, and
  `autoMemoryDirectory` pointing at a memory directory shared across all five MioBots repos.
- `.claude/skills/nav2-source-build/SKILL.md` — this build procedure, including the Pi repeat in
  Phase 5.2. **Currently repeats two stale facts, see below.**
- `.claude/agents/ros2-build-debugger.md` — subagent that isolates the first real error in a build
  log rather than the cascade.
- `HANDOFF.md` and this file.

### Found stale or wrong

- **Nav2's clone URL in HEART_TASKS 0.1 is dead.** The repo moved from `ros-planning` to
  `ros-navigation` in the OSRA reorganisation.
- **Both repos now have maintained `lyrical` branches.** HEART_TASKS says to use `main` because no
  Lyrical branch exists. Nav2's is protected with four live backport PRs; slam_toolbox's tip is
  "Bump for lyrical release". Decided to use `lyrical` on both, which removes the API-drift failure
  class the one-week time-box was written for.
- Both facts are still repeated in `.claude/skills/nav2-source-build/SKILL.md`, which was written
  from HEART_TASKS earlier the same day. Not yet corrected.

### Verified

- `rosdep` resolves every dependency across both repos except `ros-lyrical-ompl`.
- `./build.sh` builds all 8 `mio_*` packages, exit 0, no `catkin_pkg` error — confirms the Release
  change did not break the interpreter pinning.
- `.claude/hooks/guard.test.sh` passes 12/12.
- The ntfs3 mount supports symlinks and is case-sensitive, so `--symlink-install` is safe on it.
  Space (~12 GB) and small-file I/O speed are the real constraints, not correctness.

### Machine-local, not committed

Graphify (`uv tool install graphifyy`) and claude-mem (`npx claude-mem install`) were installed on
this machine. Neither can be committed — claude-mem stores in `~/.claude-mem` and auto memory is
machine-local by design. Each person installs them once; commands are in the README. Graphify's
first index of this repo: 59 nodes, 45 edges.

### Still open

- `ros-lyrical-ompl` not installed — needs a sudo password, blocks the Nav2 build.
- Nav2 not built; `nav2-lyrical.repos` not yet generated. That file is the task's deliverable.
- HEART_DECISIONS decision 32 not yet written.
