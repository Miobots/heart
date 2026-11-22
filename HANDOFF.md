# Handoff — Heart

**Last updated:** 2026-11-23 (the `H3.3` and `H3.2` sections under "Next step"; `H2.3`, `H2.4` and `H3.1` as of 2026-11-18; the rest as of 2026-10-01)
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

**`H3.3` — the outbox proven in Gazebo** on 2026-11-23, with a script anyone can re-run:

```bash
src/mio_gateway/scripts/outbox_proof.sh [output dir]   # needs miobots-brain and miobots-protocol beside
                                                        # this repo, ./build.sh, and the gateway built
                                                        # with `cargo build --features ros`
```

It runs headless Gazebo, Nav2, the supervisor, this gateway and the real Brain on their own ROS
domain and Gazebo partition, with `scripts/wiretap.ts` between gateway and Brain writing down every
message. An app (`scripts/app_goal.ts`) asks for the living room; the Brain is killed the moment
progress flows; the robot finishes the drive (`success: true`, 7.8 s, at x = −1.36) and is then
tipped over — both events on disk, nothing on the wire; the gateway is `kill -9`ed and restarted
("2 event(s) still waiting", and its own fresh `TIP_OVER` makes three); 8 s more and nothing has
left the disk. The Brain comes back: the three crossed once each, in the order they happened, each
with the time it happened (12, 11 and 8 s before it crossed), the Brain confirmed all three, and
the outbox emptied. Every check prints PASS or FAIL; the script exits non-zero on any FAIL. It
passed on two runs in a row; the figures here are from the first.

- **It needs a quiet machine.** The first run failed while another process held four cores:
  Gazebo fell behind real time, the supervisor (rightly) called the IMU silent, and Nav2's answer
  missed the 5 s the gateway gives it. A loaded machine fails it; that is the simulator, not the
  outbox.
- **Found (Brain):** a goal started from the app — a map tap — is not one the Brain tracks, so it
  logs that goal's progress and result as "unmatched" and drops them. Only goals its own agent
  starts are followed. Not a Heart problem; the app does not see map-tap progress until it is fixed.
- **Not done here:** on hardware. The gateway and outbox are the same code there; the script is not
  — it launches Gazebo and tips the robot through it. The real-robot run is part of `B4.4` (I4).

**`H3.2` — the durable outbox — works, checked with the real node and the real Brain** on
2026-11-23. Every event (`nav.result`, `diag.fault`; whatever `queues_in_outbox` names) is a file in
`outbox_dir` before it is sent, and the file goes only when the Brain's `ACK sys.receipt` names it.
Unconfirmed events go out again, oldest first, after every welcome.

- **Where:** the `outbox_dir` parameter, default `$XDG_STATE_HOME/mio_gateway/outbox`, else
  `~/.local/state/mio_gateway/outbox`. One `00000000000000000007.json` per event, numbered in the
  order they happened; `ls` it to see what the Brain has not confirmed. A file that is not an event
  is renamed `*.bad` and logged, never deleted. If the directory cannot be opened, the gateway
  says so and keeps events in memory only.
- **Checked:** no Brain running and no supervisor, so the gateway raised `SAFETY_SUPERVISOR_NO_DATA`
  after 3 s and put it on disk; killed with `kill -9`; the file was still there. Then the Brain
  (with receipts, brain `feat/event-receipts`) started and the gateway restarted: it logged "1
  event(s) still waiting", was let in, sent it, and the file was deleted on the receipt — as was
  the new run's own fault.
- **Never queued:** progress, heartbeats and the manifest — they are stale a moment later
  (`miobots-protocol/conformance/outbox.json`, which the conformance test holds this side to).
- **Cap:** 10 000 events. At the cap the oldest is dropped and logged as an error.
- **The node does not stop on Ctrl-C (SIGINT); SIGTERM stops it.** `ros2 launch` sends SIGINT,
  then SIGTERM after its timeout. With the outbox that costs time, not events.

**`H2.3` works in simulation, verified headless** on 2026-11-18. `mio_safety_supervisor` (Rust)
sits between everything that drives and the wheels: Nav2 and teleop publish `/cmd_vel`, the
supervisor clamps it into `/cmd_vel_safe`, and the Gazebo bridge forwards `/cmd_vel_safe` and
nothing else. `sim.launch.py` starts it, so nothing about launching changes.

**`H2.4` works in simulation, verified headless** on 2026-11-18, in the same node. It reads tilt
off `/imu`. Past 30° for 100 ms it stops the wheels, then reports `TIP_OVER` on `/diagnostics`, and
stays locked — with drive commands still arriving — until `~/clear` is called with the robot
upright for a second.

**`H3.1` works in simulation, verified headless** on 2026-11-18. `mio_gateway` (Rust) dials the
Brain and is the only node that does. Start it with the navigation launch:

```bash
DEV_TOKEN=<the Brain's DEV_TOKEN> BRAIN_URL=ws://127.0.0.1:8080/ws \
  ros2 launch mio_nav navigation.launch.py gateway:=true
```

`DEVICE_ID` defaults to `heart-01`. Without `DEV_TOKEN` it refuses to start.

- **What it does.** `nav.goto` becomes a Nav2 `NavigateToPose` goal, by coordinates or by a name
  looked up in `mio_nav/maps/home_arena.regions.json`. The ACK says what Nav2 said. Progress goes
  back once a second, and the result always goes back. `nav.cancel` cancels the goal it names. A
  retried command gets its first answer and the robot moves once. It sends a heartbeat every 5 s,
  the capability manifest on connecting, on any change and every 10 s, and the supervisor's faults
  as `diag.fault`. While a fault stands it refuses goals with `active_fault`.
- **Measured against the real Brain** (`ce527da`, on a spare port): an app-role client asked the
  Brain for `(1.0, 0)` and then for `"Living Room"`; the Brain relayed both, the gateway's ACKs came
  back through it, `/odom` went to x = 0.79 and then to x = −1.42, and the Brain logged 13 valid
  `nav.feedback` and 2 valid `nav.result`, none invalid. `"attic"` came back refused,
  `unknown_region`.
- **Measured against a scripted Brain,** 35 checks, all passing, twice: a goal by coordinates and
  its retry, a goal by name cancelled on the way, five kinds of refusal, a 20 s outage with a goal
  in flight (the result arrived after reconnecting, stamped with when it happened), and a tip-over
  (reported, goals refused, cleared, driving back). Every message the gateway sent was parsed by
  the TypeScript codec the Brain uses: 36–38 messages a run, none invalid.
- **Not done.** No pose, battery or map goes to the app. `voice.speak` is
  refused, `local_voice_unavailable`. `ws://` only — no TLS is compiled in. "Go to the kitchen"
  through the Brain's language model was not run; that is `B2.6`, and it needs a model key.
- **The named places are a stand-in.** Two points typed into a file, inside the 2 m patch the saved
  map knows. Real labels come from people, through `region.upsert`. **There is no "bedroom":** in
  `home_arena.sdf` that room has walls on all four sides and no door.
- **Nav2 failed to start twice in about two dozen bring-ups tonight.** Both times it stopped at
  `Activating controller_server`, after the local costmap's `KeepoutFilter: Subscribing` line, and
  `bt_navigator` then rejected every goal as "inactive" — which the gateway passed on correctly as
  `navigation_rejected`. Not explained. It happened once with no gateway running, and four
  bring-ups in a row passed with stale `/dev/shm/fastdds_*` files present, so it is neither of
  those. If goals are all rejected, check `ros2 lifecycle get /bt_navigator` before anything else.
- **Tests:** `cd src/mio_gateway && cargo test` needs no ROS and is what CI runs. The node itself
  is `cargo build --features ros`, which `./build.sh` does.

- **Measured**, three times, with a 20 Hz publisher never stopping: robot dropped on its side →
  stop published 80–120 ms after the tilt passed 30° (message stamps, so simulation time; the hold
  is 100 ms of wall time), `TIP_OVER` reported 13–15 ms after the stop. Set
  upright again: 0 of 210 messages on `/cmd_vel_safe` non-zero, `/odom` moved 0 mm. `~/clear` on
  its side: refused, "still tipped". `~/clear` upright: "unlocked", and it drove on at 0.30 m/s.
- **No IMU, no motion.** A silent IMU is a fault of its own (`IMU_NO_DATA`): pausing the
  simulation stopped the wheels and unpausing freed them, with no one clearing anything. So is an
  IMU that says it does not know its orientation — covariance `-1`, which is what a bare MPU6050
  driver sends. **On hardware the supervisor will not move the robot until something publishes a
  fused orientation** (`H5.2`).
- **To tip the simulated robot:** `gz service -s /world/home_arena/set_pose --reqtype gz.msgs.Pose
  --reptype gz.msgs.Boolean --timeout 3000 --req 'name: "mio", position: {x: 0.5, z: 0.3},
  orientation: {x: 0.5646, w: 0.8253}'`. To clear: `ros2 service call /mio_safety_supervisor/clear
  std_srvs/srv/Trigger`.
- **The lock is in memory.** A supervisor that restarts comes back unlocked unless the robot is
  still tilted. Announcing aloud and reporting to the Brain are `H4.x` and `H3.1`; this only puts
  the fault where they can hear it.

- **Measured:** a 20 Hz publisher killed with `kill -9` mid-drive, ten runs across ±0.5 and
  ±0.3 m/s, ±2.0 rad/s, and 0.5 m/s with −1.0 rad/s. The robot was standing still **283–357 ms**
  after the last command: the stop went out at 222–240 ms and braking took 61–118 ms. Read
  standstill off the *position* on `/odom`; its speed is a 20 ms average and reads zero a sample late.
- **400 ms is split, not waited.** The supervisor waits 240 ms of silence (`stop_within_ms` 400
  less `brake_ms` 160) and the drive brakes in the rest. For that the simulated drive's
  acceleration limits went from 1.5 m/s² and 3.0 rad/s² to 4.5 and 20, and `max_linear` is
  0.5 m/s. **Those four numbers are one measurement** — on hardware, measure braking first (`H5.5`),
  then set `brake_ms`. If braking leaves under 240 ms of silence the supervisor will not start.
- **The price of braking that hard:** a step command or a stop at full speed *while turning* can
  slip the wheels. Heading from `/odom` was off by 0.0–1.8° against Gazebo over six runs at
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
- Both Rust packages are built by `./build.sh` through cargo (decisions 47 and 49). Neither carries
  `COLCON_IGNORE` any more, and `ros2_rust` is not needed.
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
