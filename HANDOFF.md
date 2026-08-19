# Handoff — Heart

**Last updated:** 2026-08-13
**Status:** Phase 0.1 complete. Next task is **0.4 / Phase 1 — URDF**.

---

## Where this is right now

**Phase 0.1 is done.** Nav2 and slam_toolbox build from source against ROS 2 Lyrical inside this
workspace, and the commits that work are pinned in `nav2-lyrical.repos`. That file, not the build
output, was the deliverable.

```
46 packages finished [17min 43s], exit 0, zero failures
ros2 pkg list | grep -x nav2_bringup   → nav2_bringup
ros2 pkg list | grep -x slam_toolbox   → slam_toolbox
```

36 `nav2_*` packages plus `slam_toolbox` and the 8 `mio_*` packages are installed. The 37 packages
reporting stderr are all CMake `Manually-specified variables were not used: PYTHON_EXECUTABLE`
warnings and upstream deprecation notices — expected, not failures.

**The one-week time-box was not needed.** It was written for an API-drift failure class that turned
out not to exist, because both repos maintain `lyrical` branches. See finding 2 below.

## Set this up on a new machine

```bash
cd repos/miobots-heart
vcs import src < nav2-lyrical.repos
rosdep install -r -y --from-paths src --ignore-src
./build.sh --symlink-install --parallel-workers 4
source install/setup.bash
```

`--parallel-workers` is a **memory** cap, not a preference. Check `free -g` first: 4 above ~5 GB
available, 2 below it. Nav2's C++ will OOM at default parallelism across 12 cores.

Same procedure on the Pi 5 in Phase 5.2. **Budget hours, not minutes**, and watch memory — add swap
or cross-compile if it struggles.

---

## Findings that contradicted the written plan

All four are now recorded as **HEART_DECISIONS 32**, and `HEART_TASKS.md` 0.1 and
`.claude/skills/nav2-source-build/SKILL.md` have been corrected. Kept here because they cost real
time to discover.

1. **The Nav2 clone URL in HEART_TASKS was dead.** Nav2 moved from the `ros-planning` org to
   `ros-navigation` in the OSRA reorganisation.

2. **A maintained `lyrical` branch exists on both repos.** HEART_TASKS said to use `main` "since no
   `lyrical`-specific branch exists yet". Nav2's `lyrical` branch is protected with live backport
   PRs; slam_toolbox's tip is literally "Bump for lyrical release". Using them removes the API-drift
   failure class the time-box was written for.

3. **`build.sh` never worked as written.** `set -euo pipefail` was active when it sourced
   `/opt/ros/lyrical/setup.bash`, which reads `AMENT_TRACE_SETUP_FILES` without defaulting it; under
   `set -u` that aborted the script before colcon was ever reached. Decision 30 records build.sh as
   "verified working" — that verification was evidently done against the raw colcon command. Fixed
   by enabling `-u` only after the source.

4. **Never pass `--cmake-args` to `build.sh`.** colcon declares it `nargs='*'`, so a second
   `--cmake-args` on the command line *replaces* the list inside build.sh rather than appending to
   it. You would silently lose the `-DPython3_EXECUTABLE` pinning and get decision 30's
   `catkin_pkg` failure back, while believing you had only added a flag. Put new flags in build.sh.

**Also corrected:** the 2026-08-11 handoff called `sudo apt-get install ros-lyrical-ompl` a hard
blocker. It was not. Only `nav2_route` and `nav2_smac_planner` need OMPL, system `libompl-dev` 1.6.0
was already installed and ships `omplConfig.cmake`, and `find_package(ompl)` resolves against it.
Both packages compiled with no sudo involved. `rosdep` asks for the ROS-keyed package; CMake does
not care.

---

## Things you will trip over

- **Do not run `colcon build` directly.** Use `./build.sh`. A PreToolUse hook blocks the bare form;
  the reason is decision 30.
- **Do not create `~/miobots_ws`.** This repo *is* the workspace.
- `src/navigation2` and `src/slam_toolbox` are gitignored on purpose — `nav2-lyrical.repos` tracks
  them instead. Do not `git add -f` them.
- The two Rust crates carry `COLCON_IGNORE` (decision 31). `cargo build` for those; colcon skips
  them and that is correct until `ros2_rust` is set up.
- There is a second workspace at `~/ros2_ws` with an older nav2 + slam_toolbox on different
  branches. It is not this build and is not authoritative.

## Constraints on this machine

| Constraint | Value | Why it matters |
|---|---|---|
| Repo partition | ntfs3, ~24 GB free after the build | `-DCMAKE_BUILD_TYPE=Release` is baked into build.sh; Debug does not fit |
| RAM | ~10 GB available of 14 GB, 17 GB swap | Sets `--parallel-workers`; see above |
| Cores | 12 | Deliberately not used in full |

Symlinks work on this mount and it is case-sensitive, so `--symlink-install` is safe.

## Agent tooling in this repo

Committed in `.claude/`, so it travels with a clone. See README for the full table. The
`nav2-source-build` skill covers the build procedure and is now accurate; `ros2-build-debugger` is
the subagent to hand a failed build log to.
