# Handoff — Heart

**Last updated:** 2026-11-18 (the `H2.3` section under "Next step"; the rest is as of 2026-10-01)
**Status:** Phase 0.1 and Phase 1 are on `main` — the source build, the URDF, the Gazebo Jetty
world, the slam_toolbox pipeline and the saved `home_arena` map. The 2026-08-14 note below said
this work was "committed on `stb/lyrical_source_build`, not pushed"; it merged as PR #1.

`H1.3` is measured rather than asserted: 1.00 m commanded reads **1.0002 m** on `/odom`
(see `CHANGELOG.md`, 2026-09-20). For the project's state, read `STATUS.md` — not this file.

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

Neither produces a map, and that was expected then — there was no `/scan` and no TF tree, because
there was no robot yet. There is now: `ros2 launch mio_sim sim.launch.py` publishes both, and
`ros2 launch mio_nav mapping.launch.py` builds a map from them.

**`tb3_loopback_simulation_launch.py` will not run as-is.** It needs `nav2_minimal_tb3_sim` for the
turtlebot URDF, which is a separate repo (`ros-navigation/nav2_minimal_turtlebot_simulation`) that
we deliberately did not build.

---

## Next step

**`H2.3` works in simulation, verified headless** on 2026-11-18. `mio_safety_supervisor` (Rust)
sits between everything that drives and the wheels: Nav2 and teleop publish `/cmd_vel`, the
supervisor clamps it into `/cmd_vel_safe`, and the Gazebo bridge forwards `/cmd_vel_safe` and
nothing else. `sim.launch.py` starts it, so nothing about launching changes. Next is `H2.4`
(tip-over) in the same node.

- **Measured:** a 20 Hz publisher killed with `kill -9` mid-drive, ten runs across ±0.5 and
  ±0.3 m/s, ±2.0 rad/s, and 0.5 m/s with −1.0 rad/s. The robot was standing still **308–382 ms**
  after the last command: the stop went out at 232–249 ms and braking took 74–134 ms.
- **400 ms is split, not waited.** The supervisor waits 250 ms of silence (`stop_within_ms` 400
  less `brake_ms` 150) and the drive brakes in the rest. For that the simulated drive's
  acceleration limits went from 1.5 m/s² and 3.0 rad/s² to 4.5 and 20, and `max_linear` is
  0.5 m/s. **Those four numbers are one measurement** — on hardware, measure braking first (`H5.5`),
  then set `brake_ms`. If braking leaves under 240 ms of silence the supervisor will not start.
- **The price of braking that hard:** a step command or a stop at full speed *while turning* can
  slip the wheels. Heading from `/odom` was off by 0.0–1.8° against Gazebo over four runs at
  0.5 m/s with −1.0 rad/s; straight lines and spins on the spot stayed exact (0 mm, 0.0°), and the
  old limits never slipped. Nav2 ramps its own commands and is not affected; a keyboard is.
- **Teleop is hold-to-drive now.** `teleop_twist_keyboard` publishes once per key press, so a tap
  moves the robot for a quarter of a second. Hold the key. Key-repeat takes 500 ms to start on
  this desktop, so there is one hitch at the start of every hold.
- **Do not reverse blind in a test.** Two "odometry errors" of 118 mm and 1 m while measuring this
  were the robot backing into the living-room table at x = −2.
- **Tests:** `cd src/mio_safety_supervisor && cargo test`, with ROS sourced. The node itself is
  built by `./build.sh` (which now needs `cargo`) into `build/`, not into `target/`.
- **Stop a launch with Ctrl-C, not `kill`.** `ros2 launch` killed with SIGTERM exits and leaves
  every child running — Gazebo, the bridge and the supervisor (checked: 4 of 4 survived). A
  supervisor left over from an earlier run is a second publisher on `/cmd_vel_safe`, possibly from
  an older build. Check `pgrep -fa mio_safety_supervisor` is empty before a run, as for `gz-sim-main`.
- **Not covered:** two things publishing `/cmd_vel` at once (Nav2 and teleop) simply interleave.
  Choosing between sources is a later job; the supervisor only clamps and times out.

**`H2.1` works in simulation, verified headless** on 2026-10-01: `navigation.launch.py`, then two
`NavigateToPose` goals — the same action RViz's "Nav2 Goal" sends — both `SUCCEEDED` and the robot
stopped within the 0.25 m goal tolerance. The RViz click itself still needs doing on a machine with
a display.

**The saved map is not the house.** `home_arena.pgm` is essentially one scan from the spawn point:
about 2 m around the origin is known, the rest is unknown. AMCL and goals work inside that patch
and nowhere else. Re-map the whole house (H1.5's exit check) before demoing H2.1 or H2.2.

**`H2.2` works in simulation, verified headless** on 2026-10-01. Drive `(-1.3, 0)` → `(1.0, 0)`
and drop `mio_sim/models/obstacle_box.sdf` at the origin mid-run (command in the file's header).
Three drops, the box appearing 1.1, 0.9 and 0.8 m ahead of the moving robot: each time it paused,
replanned, went round and `SUCCEEDED`, with a closest **ground-truth** approach of 0.31, 0.23 and
0.20 m. Measure clearance from Gazebo's `/world/home_arena/dynamic_pose/info`, not `/odom`.

**Found on the way — odometry was wrong in every turn.** The wheels' cylinder collisions touched
the ground at their rims, so the physics track was ~0.176 m while odometry used 0.20 m: spins
overshot `/odom` by ~12% and a 3 m Nav2 run ended 0.7 m from where `/odom` said. H1.3 only drove
straight, so it never showed. The wheel collisions are spheres now; spin, arc and zigzag match
ground truth to the centimetre. **On hardware this is a calibration job** — `wheel_separation` in
`mio_core.xacro` is the knob.

**Test hygiene:** stopping a launch can leave `gz-sim-main` running, and every server on the
machine shares Gazebo's topics, so a second sim silently mixes two robots' odometry. Check
`pgrep -fa gz-sim-main` is empty before a run.

Phase 1 is done: option A below was taken, and
the robot now drives, maps and saves a map in simulation. The options are kept because B is still a
useful reference to diff against if Nav2 misbehaves on our URDF.

*As decided on 2026-08-14:*

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
- `mio_safety_supervisor` is built by `./build.sh` through cargo (decision 47). Only `mio_gateway`
  still carries `COLCON_IGNORE` (decision 31) — the rest of this bullet is about it. `cargo build` for those; colcon skips
  them and that is correct until `ros2_rust` is set up.
- There is a second workspace at `~/ros2_ws` with an older nav2 + slam_toolbox on different
  branches. It is not this build and is not authoritative.

## Constraints on this machine

| Constraint | Value | Why it matters |
|---|---|---|
| Gazebo simulator | gz-sim 10.5.0 (Jetty) | ROS 2 Lyrical pairs with Jetty; Harmonic is retired |
| Repo partition | ntfs3, ~24 GB free after the build | `-DCMAKE_BUILD_TYPE=Release` is baked into build.sh; Debug does not fit |
| RAM | ~10 GB available of 14 GB, 17 GB swap | Sets `--parallel-workers`; see above |
| Cores | 12 | Deliberately not used in full |

Symlinks work on this mount and it is case-sensitive, so `--symlink-install` is safe.

## Agent tooling in this repo

Committed in `.claude/`, so it travels with a clone. See README for the full table. The
`nav2-source-build` skill covers the build procedure and is now accurate; `ros2-build-debugger` is
the subagent to hand a failed build log to.
