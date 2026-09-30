# Heart (`miobots-heart`) — Antigravity Rules

**Sole physical authority, sensor fusion, autonomous navigation, safety supervisor, and hardware controller.** Pi 5 + MCU, ROS 2 Lyrical Luth, C++/Rust, Python only for perception.

---

## Non-Negotiable Invariants

1. **Sole Physical Authority:** Only Heart moves mass or commands motors.
2. **Two-Tier Safety Hierarchy:**
   - Layer 1 (MCU Hardware): Cuts motor power on hardware bump or UART silence (survives Pi freeze).
   - Layer 2 (Rust Safety Supervisor): 400ms deadman timeout and IMU tip-over veto.
3. **Cliff Sensor Rule:** Cliff drop-off detection stops robot immediately and writes permanent no-go zone into costmap.
4. **Halt-Then-Run Local Voice:** If local 1B model runs during navigation, pause navigation, generate answer, unload model, and resume navigation.
5. **Durable Outbox (`mio_gateway`):** Events queue to disk during network dropouts and drain in order upon reconnection.

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

# Drive by hand — /cmd_vel is TwistStamped, because Nav2 on Lyrical publishes stamped
ros2 run teleop_twist_keyboard teleop_twist_keyboard --ros-args -p stamped:=true   # /cmd_vel is TwistStamped

# Run on hardware — not written yet; mio_bringup has no launch files until Phase 5
# ros2 launch mio_bringup real.launch.py
```
