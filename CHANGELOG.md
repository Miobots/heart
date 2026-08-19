# Changelog — miobots-heart

Newest first. One entry per working session. Records what changed and, where it matters, what was
found to be wrong. See `HANDOFF.md` for the current state and the next step.

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
