# Heart (`miobots-heart`) — Antigravity Rules

**Sole physical authority, sensor fusion, autonomous navigation, safety supervisor, and hardware controller.** Pi 5 + MCU, ROS 2 Lyrical Luth, C++/Rust, Python only for perception.

---

## Non-Negotiable Invariants

1. **Sole Physical Authority:** Only Heart moves mass or commands motors.
2. **Two-Tier Safety Hierarchy:**
   - Layer 1 (MCU Hardware): Cuts motor power on hardware bump or UART silence (survives Pi freeze).
   - Layer 2 (Rust Safety Supervisor): deadman — robot standing still within 400ms of the last command — and the IMU tip-over latch: past 30° of tilt it halts, *then* reports `TIP_OVER` on `/diagnostics`, and stays locked until `~/clear` is called with the robot upright. No usable IMU data means no motion (HEART_DECISIONS 48).
3. **Cliff Sensor Rule:** Cliff drop-off detection stops robot immediately and writes permanent no-go zone into costmap.
4. **Halt-Then-Run Local Voice:** If local 1B model runs during navigation, pause navigation, generate answer, unload model, and resume navigation.
5. **Durable Outbox (`mio_gateway`):** An event is written to disk (`outbox.rs`) before it is sent and deleted only on the Brain's `ACK sys.receipt`; unconfirmed events go out again, in order, after every welcome. Never trim on send. Which topics wait is `queues_in_outbox`, held to `miobots-protocol/conformance/outbox.json` (ENVELOPE §8, HEART_DECISIONS 50).
6. **Wheels Listen To `/cmd_vel_safe` Only:** Nav2, teleop and (later) the gateway publish `/cmd_vel`; only `mio_safety_supervisor` publishes `/cmd_vel_safe` — clamped, and stopped when commands go silent. Never bridge `/cmd_vel` to the motors or publish `/cmd_vel_safe` from anywhere else (HEART_DECISIONS 47).
7. **400 ms Is A Promise About The Wheels, Not A Timer:** Standing still within 400 ms of the last command, so the supervisor waits `400 - brake_ms` of silence and braking gets the rest. `brake_ms`, `max_linear`, `max_angular` and the drive's acceleration limits are measured together — change one and `brake_ms` must be measured again. The supervisor refuses to start when braking leaves under 240 ms of silence (HEART_DECISIONS 47).
8. **Rust Packages Build With `./build.sh`:** `mio_safety_supervisor` is a plain cargo crate that a small `CMakeLists.txt` builds and installs; it needs `cargo` on the PATH. `ros2_rust`'s colcon tooling is not needed (HEART_DECISIONS 47). `mio_gateway` is built the same way, and its library needs no ROS: the wire contract (`lib.rs`) and everything the gateway decides (`session.rs`) are plain Rust that CI tests against the shared vectors; the ROS node is the binary behind the `ros` feature. Logic goes in `session.rs`, never in `main.rs`. The gateway has no velocity publisher, and its token comes from the environment (`DEV_TOKEN`), never a ROS parameter or a default in code (HEART_DECISIONS 49).

---

## Commits

**One-line commit messages only** — no body. **Never add a `Co-Authored-By: Claude` trailer** or a
"Generated with Claude Code" line, in any commit or PR.

---

## Build & Run Directives

> [!CAUTION]
> **Build Command Rule:** Always use `./build.sh --symlink-install`. **NEVER** run bare `colcon build` directly (causes Python interpreter shadowing). Never pass `--cmake-args` on CLI.

```bash
# Build workspace
./build.sh --symlink-install

# Sourcing (ALWAYS overlay this repo's install over ~/.bashrc)
source install/setup.bash

# Run simulation
ros2 launch mio_sim sim.launch.py

# Build a map (slam_toolbox against the simulated /scan)
ros2 launch mio_nav mapping.launch.py

# Localise on the saved map and drive to goals (AMCL + Nav2; click "Nav2 Goal" in RViz)
ros2 launch mio_nav navigation.launch.py

# Drive by hand — /cmd_vel is TwistStamped, because Nav2 on Lyrical publishes stamped.
# HOLD the key: the supervisor stops the wheels after 240 ms of silence, so a tap is a nudge.
ros2 run teleop_twist_keyboard teleop_twist_keyboard --ros-args -p stamped:=true   # /cmd_vel is TwistStamped

# Safety supervisor tests (the node itself is built by ./build.sh and started by sim.launch.py)
cd src/mio_safety_supervisor && cargo test

# Let the Brain drive: the same launch, plus the gateway dialling out
DEV_TOKEN=... BRAIN_URL=ws://<brain>:8080/ws ros2 launch mio_nav navigation.launch.py gateway:=true

# Gateway tests — no ROS needed, this is what CI runs
cd src/mio_gateway && cargo test

# Run on hardware — not written yet; mio_bringup has no launch files until Phase 5
# ros2 launch mio_bringup real.launch.py
```
