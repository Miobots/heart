# Changelog — miobots-heart

Newest first. One entry per working session. Records what changed and, where it matters, what was
found to be wrong. See `HANDOFF.md` for the current state and the next step.

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
