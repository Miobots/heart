# Handoff — Heart

**Last updated:** 2026-08-14
**Status:** Phase 0.1 complete and smoke-tested. **A decision is open on what to do next — see
"Next step" below.** Committed on branch `stb/lyrical_source_build` (`c4cacdd`), not pushed.

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

## Read this before you run anything

**`~/.bashrc` line 147 sources `~/ros2_ws/install/setup.bash`.** That is a *different* workspace with
its own older nav2 and slam_toolbox — slam_toolbox there is from 2026-07-17 on branch
`fix/slam-toolbox-map-serialization`, not the pinned `lyrical` commit. **In a fresh shell you are
running that build, not this one.** Always overlay this workspace on top before testing:

```bash
cd repos/miobots-heart && source install/setup.bash
```

Verify which one you actually got: `ros2 pkg prefix slam_toolbox` must print a path inside
`miobots-heart`, not `~/ros2_ws`.

**slam_toolbox and every Nav2 server are lifecycle nodes.** `ros2 run slam_toolbox
sync_slam_toolbox_node` starts the node in state `unconfigured` and it sits there doing nothing —
that is correct behaviour, not a failure. The launch files do the configure/activate transitions.
Use them.

## Smoke test — what is proven to run

No robot needed for either of these. Both were run on 2026-08-14 and both passed.

```bash
# Nav2: map server + AMCL against a bundled map
ros2 launch nav2_bringup localization_launch.py \
  map:=install/nav2_bringup/share/nav2_bringup/maps/warehouse.yaml \
  use_sim_time:=false use_rviz:=false
# in another shell:
ros2 lifecycle get /map_server     # -> active [3]
ros2 lifecycle get /amcl           # -> active [3]
ros2 topic echo /map --once        # -> 1006 x 1674 @ 0.03 m/px
```

```bash
# slam_toolbox, lifecycle-managed properly
ros2 launch slam_toolbox online_async_launch.py use_sim_time:=false
ros2 lifecycle get /slam_toolbox   # -> active [3]
ros2 service list | grep slam_toolbox   # -> save_map, serialize_map
```

Neither produces a map, and that is expected — there is no `/scan` and no TF tree, because there is
no robot yet. This proves the binaries run; it does not prove navigation works.

**`tb3_loopback_simulation_launch.py` will not run as-is.** It needs `nav2_minimal_tb3_sim` for the
turtlebot URDF, which is a separate repo (`ros-navigation/nav2_minimal_turtlebot_simulation`) that
we deliberately did not build.

---

## Next step — an open decision

Real mapping or navigation needs a robot. Three ways forward; **option A is the recommendation.**

**A. Start Phase 1 — build our own robot.** Write `mio_description`'s URDF, spawn it in Gazebo,
teleop it, add a simulated LiDAR, then point slam_toolbox at it. This is the next task on
HEART_TASKS and the one W2-01 on the task board is waiting for. Slower to first pixel, but nothing
is thrown away and it ends with a MioBots robot rather than a turtlebot.

**B. Build the turtlebot sim first, as a reference.** Clone and build
`ros-navigation/nav2_minimal_turtlebot_simulation` (~15 min), then
`tb3_loopback_simulation_launch.py` gives the full Nav2 demo — drive a robot, build a map, send
goals. Throwaway work that is not MioBots, but it is a known-good reference to diff against when our
own URDF misbehaves, and Phase 1 debugging is where you will want one.

**C. Neither — just fix the workspace shadowing.** Resolve the `~/ros2_ws` vs `miobots-heart`
sourcing conflict described above so nobody keeps testing the wrong build, and stop there.

Whichever is chosen, **do not run two of Phase 1's steps in parallel.** URDF → Gazebo spawn →
manual driving → odometry → LiDAR → SLAM, strictly in order — see CLAUDE.md.

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
