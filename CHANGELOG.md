# Changelog — miobots-heart

Newest first. One entry per working session. Records what changed and, where it matters, what was
found to be wrong. See `HANDOFF.md` for the current state and the next step.

---

## 2026-11-18 — H3.1: the gateway dials the Brain and drives the simulated robot

`src/mio_gateway` is no longer a scaffold. It dials the Brain over WebSocket, says hello, and turns
`nav.goto` into a Nav2 goal — by coordinates, or by a name looked up in
`mio_nav/maps/home_arena.regions.json` — then sends the ACK Nav2 gave, progress once a second and
the result. It also sends a heartbeat every 5 s, the robot's half of the capability manifest, and
the supervisor's faults as `diag.fault`. `navigation.launch.py gateway:=true` starts it.

`src/session.rs` holds every decision and has no socket and no ROS in it; `src/main.rs` moves
messages between the socket, Nav2 and the supervisor. The library needs no ROS, so CI keeps testing
it with plain `cargo test`; the node is behind the `ros` feature.

Verified headless against the real Brain on a spare port: an app-role client asked for `(1.0, 0)`
and for `"Living Room"`, the Brain relayed both, the robot drove to x = 0.79 and then x = −1.42,
and the Brain logged 13 valid `nav.feedback` and 2 valid `nav.result`. And against a scripted
Brain, 35 checks: retry, cancel, refusals, a 20 s outage, a tip-over.

**Found:** the Brain does not do what ENVELOPE.md says in several places, and the gateway follows
the Brain. It cancels on a new `corr_id` and names the goal in the payload. It sends a room's name
and no coordinates. It never retries a command, never sends `sys.error` or `sys.receipt`, and
ignores an `ERR` — so an unsupported command is refused with an ACK, which it hears at once. Its
check on `nav.result` needs a `final_pose`, always. The list is in HEART_DECISIONS 49.

**Found:** Nav2 reports 0 m remaining until it has planned a path. Passed on, the first progress
message said "arrived" a second after the robot was asked to leave. Not forwarded now.

**Found:** an event held while the link was still connecting used up a `seq`, leaving a gap on the
wire. The stand-in Brain's check for gaps caught it; a held event is numbered when it is sent.

**Found:** a goal stamped with the gateway's clock would be in Nav2's future in simulation. Goals
go out unstamped, which Nav2 reads as "now".

**Found:** the simulated house's bedroom has no door.

**Not explained:** Nav2 failed to finish starting twice in about two dozen bring-ups. See HANDOFF.

---

## 2026-11-18 — H2.4: the tip-over lock, and 10 ms more room for the 400 ms stop

`mio_safety_supervisor` now reads tilt off `/imu`. Past 30° for 100 ms it stops the wheels, **then**
reports `TIP_OVER` on `/diagnostics`, and stays locked until `~/clear` (`std_srvs/Trigger`) is
called with the robot upright for a second. Halt first, report second: the stop goes out in the
IMU callback, the report on the watchdog's next tick.

Verified headless in Gazebo, three times, with drive commands arriving the whole time: dropped on
its side → stop published 80–120 ms after the tilt passed 30°, report 13–15 ms after the stop; set upright
→ 0 of 210 messages on `/cmd_vel_safe` non-zero and 0 mm on `/odom`; `~/clear` on its side refused,
upright accepted, and it drove on.

**Decided:** no usable IMU data means no motion. A silent IMU is its own fault (`IMU_NO_DATA`) and
clears itself when the IMU has been back for 100 ms; so is an orientation marked unknown (REP 145,
covariance `-1`). A subsystem that stops reporting is failed, not quiet (HEART_SPEC R9).

**Decided:** faults go out as `diagnostic_msgs` on `/diagnostics`, once a second and at once on a
change — ROS's own mechanism, not a new message. `mio_msgs` stays empty.

**Found:** the 400 ms stop had less room than the first measurement said. "Still" was read off the
speed on `/odom`, which is a 20 ms average and reads zero one sample late; a later top-speed run
read 409 ms, 389 by position. `brake_ms` is now 160 and silence 240 ms — the floor — and ten runs
measured by position stood still at **283–357 ms**.

---

## 2026-11-18 — H2.3: the safety supervisor, and Rust nodes without ros2_rust

`src/mio_safety_supervisor` is no longer a scaffold. It subscribes `/cmd_vel`, clamps it, and is
the only publisher of `/cmd_vel_safe`; the Gazebo bridge now forwards `/cmd_vel_safe` and nothing
else. When commands go silent it publishes stop, on a monotonic clock, early enough that the robot
is **standing still within 400 ms of the last command**: 250 ms of silence, then up to 150 ms of
braking. `src/lib.rs` holds the decisions and has no ROS in it — that is what `cargo test` covers;
`src/main.rs` is the glue. `sim.launch.py` starts the node, with `respawn`.

Verified headless in Gazebo: a publisher killed mid-drive, ten runs across ±0.5 and ±0.3 m/s,
±2.0 rad/s, and 0.5 m/s with −1.0 rad/s — robot standing still **308–382 ms** after the last
command (stop out at 232–249 ms, braking 74–134 ms). A Nav2 goal driven through the supervisor
still `SUCCEEDED`, and a supervisor killed with `kill -9` came back publishing stop.

**Found:** `ros2_rust`'s colcon tooling (decision 31) is not needed. Lyrical ships its message
types as Rust, and `rclrs` 0.8.0 + `ros-env` 0.3.0 build against it with plain cargo. A small
`CMakeLists.txt` has cargo build the node inside `./build.sh`, so the package lost its
`COLCON_IGNORE`. `cargo` is now needed to build the workspace.

**Found:** waiting 400 ms and then stopping is not "stopped within 400 ms". The first version did
that and the robot stood still at 590–613 ms, because the simulated drive (1.5 m/s², 3.0 rad/s²)
needed 200 ms to brake from 0.30 m/s — and 333 and 667 ms from its limits, more than the whole
promise. The 400 ms is now split: `stop_within_ms` 400 less `brake_ms` 150 leaves 250 ms of
silence. The simulated drive brakes at 4.5 m/s² and 20 rad/s², and `max_linear` is 0.5 m/s. The
supervisor refuses to start if braking leaves under 240 ms — two 100 ms teleop frames, so one lost
frame is still never a stop.

**Found:** braking that hard has a price. The drive's acceleration limits are also its brake
limits, so a step command or a stop at full speed *while turning* can slip the wheels: heading
from `/odom` was off by 0.0–1.8° against Gazebo over four such runs. Straight lines and spins on
the spot stayed exact, and the old limits never slipped. Nav2 ramps its own commands; a keyboard
does not. Ramping speed *increases* — in the supervisor or the MCU — would remove it.

**Found:** `teleop_twist_keyboard` publishes once per key press, so with a deadman in the path
keyboard driving is hold-to-drive, and a tap is a quarter-second nudge.

**Found:** `ros2 launch` killed with SIGTERM exits and leaves every child running (4 of 4 here). A
supervisor left over from an earlier run is a second publisher on `/cmd_vel_safe`. Stop launches
with Ctrl-C; that was checked and leaves nothing behind.

---

## 2026-10-12 — H4.0: the offline voice pipeline, standalone on a laptop (built, not yet measured on a voice)

`src/mio_voice` is no longer a scaffold. It is a C++ program, `mio_voice`, that runs the whole
offline chain with no robot, no ROS and no network: wake phrase → whisper.cpp → the four named
stages → a Piper voice. Plain CMake with `COLCON_IGNORE`; engines and models live in
`~/.cache/mio_voice`, fetched by `scripts/fetch_deps.sh` with pinned checksums. `README.md` in the
package has the build, the commands and how to measure.

`src/core.cpp` holds every decision and links no engine, so the stage order and the rule that
cannot bend are tested without one: **if local search returns nothing, the model is never
started.** The model is a child process (`llama-completion`), so that is a process count, and it
may only emit `{"row": N, "ask": "where"|"when"|"none"}` — a template speaks the reply.

**Not done.** H4.0's exit check needs real recordings and there are none yet. Everything measured
so far is on text or on synthetic speech; the numbers and what is still missing are in the vault's
`VOICE_EVAL.md`. Record with `src/mio_voice/scripts/record.sh`.

**Found:** an English wake phrase in front of Urdu speech makes whisper's language detector call
the whole thing English and transcribe the Urdu as nonsense. The language is now judged on the
audio after the first 0.9 s, and the phrase is stripped from the transcript.

**Found:** left to choose among all its languages, the detector often hears Urdu as Hindi and
writes Devanagari. Only English or Urdu is allowed; Hindi counts as Urdu.

**Found:** a vocabulary prompt (a list of room and object names) made the recogniser echo the list
back instead of the speech. Removed.

**Found:** Qwen3.5's chat template aborts `llama-completion` on a lone system message ("No user
query found"), so the instructions go in the user turn. With the model's default context a 1.1 GB
model peaked at 2.7 GB; `-c 1024` brings a 0.5 GB model to under 0.9 GB.

**Found:** the keyword spotter misses a good share of synthetic "Hey Mio"s, fewer with the
full-precision model files and a wider search than with its defaults. A second detector — the
first 2.5 s transcribed as English — exists alongside it; which one ships is decided on recordings.

**Found:** `unshare -rn` is blocked by AppArmor on Ubuntu 26.04. `bwrap --unshare-net` is not, and
is how "runs with no network" is shown.

---

## 2026-10-01 — H2.2: routing around an unmapped box, and odometry that is honest in turns

`mio_sim/models/obstacle_box.sdf` is a static 0.3 × 0.3 × 0.5 m box to drop in the robot's path
mid-run. H2.2 needed **no Nav2 change**: the stock costmaps already mark `/scan` obstacles, and the
robot replans and goes round. Verified three times against Gazebo ground truth — closest approach
0.20–0.31 m, every goal `SUCCEEDED`.

**Found:** odometry was wrong whenever the robot turned. Cylinder wheel collisions contacted the
ground at their rims (effective track ~0.176 m against the 0.20 m odometry uses), so spins
overshot by ~12% and a 3 m run drifted 0.7 m. H1.3 drove straight only. Wheel collisions are now
spheres, and odometry matches ground truth in spins, arcs and zigzags.

---

## 2026-10-01 — H2.1: AMCL and Nav2 goals in simulation

`mio_nav/launch/navigation.launch.py` wraps `nav2_bringup`'s `bringup_launch.py` with the saved map
and `mio_nav/config/nav2_params.yaml` — the stock file with the robot radius (0.12 m) and AMCL's
initial pose changed, each marked `# mio:`. Keepout and speed zones are off; there are no masks.

**Found:** Nav2 on Lyrical publishes `TwistStamped` on `/cmd_vel` (`enable_stamped_cmd_vel`
defaults to true), and the bridge carried plain `Twist`, so Nav2 would plan and the wheels would
never turn. The bridge is now `TwistStamped`; teleop needs `-p stamped:=true`.

**Found:** the saved `home_arena` map covers ~2 m around the spawn point, not the house. See
`HANDOFF.md`.

---

## 2026-09-27 — Documentation caught up with Phase 1

Docs only. `README.md` and `CLAUDE.md` still said *scaffold only, nothing is implemented*, and
`HANDOFF.md`'s "next step" still debated whether to start Phase 1. They now record Phase 0.1 and
Phase 1 as done and name Phase 2 (`H2.1`, AMCL and Nav2 goals) as next. `ANTIGRAVITY.md` told you to
run `mio_bringup real.launch.py`, which does not exist yet; it now shows `mapping.launch.py` and
marks the hardware launch as Phase 5. `HANDOFF.md`'s simulator version is corrected to gz-sim 10.5.0.

---

## 2026-09-20 — Wheel geometry has one home, and H1.3 finally has a number

### Summary

`mio_gazebo.xacro` hard-coded `wheel_separation` as `0.20` and `wheel_radius` as `0.033` inside the
DiffDrive plugin, while `mio_core.xacro` declared the same two values as xacro properties and built
the wheels from them. Both files agreed, so nothing was broken — but these two numbers are what
converts wheel rotation into distance. A copy that drifts from the model does not fail loudly; it
makes `/odom` quietly wrong, which is precisely the failure `H1.3`'s *"1 m driven ≈ 1.0 m"* check
exists to catch. They are both included from `mio.urdf.xacro`, so the properties were already in
scope and this is a two-token fix.

While verifying it, ran `H1.3`'s exit check and **wrote the number down**. `HEART_TASKS.md` records
the check as *"Passed on 2026-08-19"* but no measured value appears anywhere in the repo or the
vault, so the claim could not be re-checked by anyone.

### Changed

- **`mio_description/urdf/mio_gazebo.xacro`**: the DiffDrive plugin's `<wheel_separation>` and
  `<wheel_radius>` now read `${wheel_separation}` and `${wheel_radius}` from `mio_core.xacro`.

### Verified

- `xacro` output is **semantically identical** before and after — same tree, same numbers, only the
  comment and `0.20` rendering as `0.2` differ. `check_urdf` parses the tree cleanly.
- **`H1.3` measured**, headless (`ros2 launch mio_sim sim.launch.py headless:=true`), driving
  `/cmd_vel` at 0.2 m/s for 5 s:

  | | |
  |---|---|
  | commanded | 1.00 m |
  | reported on `/odom` | **1.0002 m** |
  | error | +0.2 mm (100.0% of commanded) |

- Simulator on this machine is **gz-sim 10.5.0** (Jetty). `STATUS.md` still records 10.4.0.

---

## 2026-08-19 — slam_toolbox mapping pipeline and the saved home_arena map

*Recorded late: this session landed as `b624ea4` and was never written up, which broke this file's
own "one entry per working session" rule.*

### Added

- **`mio_nav/launch/mapping.launch.py`** — brings up `slam_toolbox` in async mapping mode against
  the simulated `/scan`.
- **`mio_nav/config/slam_toolbox.yaml`** — mapper parameters for the `home_arena` world.
- **`mio_nav/config/mapping.rviz`** — RViz profile showing the live occupancy grid while driving.
- **`mio_nav/maps/home_arena.pgm` / `.yaml`** — the saved 2D map. 5 cm resolution, origin
  `[-3.952, -2.949, 0]`. This is the artefact `H1.5` — the Gazebo SLAM deliverable — produces.

---

## 2026-08-16 06:55 PKT — Phase 1: URDF / Xacro Robot Model & Gazebo Jetty Simulation Environment

### Summary Description

Implemented the complete kinematic robot model and Gazebo Jetty (`gz-sim` 10) simulation environment across `mio_description` and `mio_sim`. This delivers the foundational differential-drive mobile base required for simulation-driven SLAM mapping, odometry tracking, and autonomous Nav2 path planning.

The URDF was designed modularly using Xacro macros in `mio_description/urdf/`: `common_properties.xacro` (colors and box/cylinder/sphere inertia macros), `mio_core.xacro` (240mm diameter cylindrical chassis, 66mm drive wheels on a 200mm track, and dual low-friction front/rear casters), `sensors/lidar.xacro` (2D LiDAR at `laser_frame` with 360 rays, 12m range, 10Hz), `sensors/imu.xacro` (`imu_frame` with 50Hz sensor plugin), and `mio_gazebo.xacro` (Gazebo Jetty `DiffDrive` and `JointStatePublisher` system plugins with odometry and transform publishing).

In `mio_sim`, built the multi-room simulation world `worlds/home_arena.sdf` ($8\text{m} \times 6\text{m}$ walled environment with living room, kitchen, bedroom, doorways, and furniture obstacles). Configured `config/ros_gz_bridge.yaml` to establish bidirectional communication between ROS 2 DDS and Gazebo Transport for `/clock`, `/cmd_vel`, `/odom`, `/tf`, `/scan`, `/imu`, and `/joint_states`. Created master simulation launch files (`sim.launch.py` and `sim_rviz.launch.py`) along with pre-configured RViz display profiles (`sim.rviz`).

### Added

- **`mio_description`:**
  - `urdf/common_properties.xacro`: Materials and mathematical inertia macros.
  - `urdf/mio_core.xacro`: Differential drive base footprint, chassis, drive wheels, and casters.
  - `urdf/sensors/lidar.xacro`: 2D LiDAR mount and GPU LiDAR Gazebo plugin.
  - `urdf/sensors/imu.xacro`: 6-DOF IMU sensor link and Gazebo plugin.
  - `urdf/mio_gazebo.xacro`: Gazebo Jetty DiffDrive system plugin and friction tags.
  - `urdf/mio.urdf.xacro`: Master assembled Xacro robot model.
  - `launch/rsp.launch.py`: Robot State Publisher node.
  - `launch/view_robot.launch.py`: Joint State Publisher GUI + RViz2 visualizer.
  - `config/view_robot.rviz`: RViz configuration for robot inspection.
- **`mio_sim`:**
  - `worlds/home_arena.sdf`: Multi-room house simulation arena.
  - `config/ros_gz_bridge.yaml`: Bidirectional ROS 2 $\leftrightarrow$ Gazebo Jetty topic bridge.
  - `launch/sim.launch.py`: Master simulation launcher (Gazebo + RSP + Spawner + Bridge).
  - `launch/sim_rviz.launch.py`: Simulation + RViz2 inspector.
  - `config/sim.rviz`: Simulation RViz configuration displaying LaserScan, Odometry, and TF.

---

## 2026-08-16 06:10 PKT — Phase 1 Initialization: Architecture Alignment & Robotics Masterclass Documentation

### Summary Description

Initiated Phase 1 (Simulated Robot Base) following the completion and verification of the Nav2 / `slam_toolbox` source build on ROS 2 Lyrical. Analyzed all 9 internal `mio_*` packages (`mio_description`, `mio_sim`, `mio_bringup`, `mio_nav`, `mio_safety_supervisor`, `mio_gateway`, `mio_msgs`, `mio_docking`, `mio_perception`, `mio_voice`) to establish the simulation and kinematic pipeline.

Authored a comprehensive pedagogical reference manual in `TEACHER.md` breaking down the entire Heart subsystem for beginner-to-advanced developers. The guide covers ROS 2 Lyrical source compilation rationale, CMake interpreter pinning in `build.sh`, TF2 coordinate frames (`base_footprint` $\to$ `base_link` $\to$ `laser_frame`), differential drive kinematics, center-of-mass inertia tensors, Gazebo Jetty simulation bridges (`ros_gz_bridge`), the 2-tier safety hierarchy (MCU hardware cutoff + Rust 400ms deadman supervisor), and durable outbox reconciliation on Wi-Fi recovery.

Configured `.gitignore` to keep local educational study notes isolated from Git commits, prepared `ANTIGRAVITY.md` safety invariants, and aligned the simulation roadmap to implement the URDF / Xacro differential drive model (`mio.urdf.xacro`) and Gazebo Jetty launch environments.

### Added

- **`TEACHER.md`:** Deep-dive robotics engineering masterclass document explaining robot kinematics, TF2 tree, Gazebo Jetty simulation, and safety watchdogs.
- Updated `.gitignore` to prevent tracking local study guides.

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
