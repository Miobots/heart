# Heart — the robot

**Heart is the robot.** Formally it is the software on a Raspberry Pi 5 plus a microcontroller,
but for every decision you make here, "Heart = the robot" is the correct mental model.

Its job: don't crash, don't fall, go where told, dock when low on battery, and always be able to
hear and speak — including with the internet completely gone.

---

## Non-negotiable rules

**Heart is the only component allowed to move the wheels.** Brain and the app can only *request*
motion. **Heart can refuse**, and refusal is normal operation, not an error.

**No safety function may depend on another component or on the network.** Heart halts, docks and
diagnoses alone.

**No garbage-collected runtime in any control path.** Not Python, not Node, not even temporarily.
The reason is not speed — it is that an unpredictable collection pause landing between a velocity
command and the wheels cannot be tested away. You cannot prove a Python prototype safe by running
it a thousand times; the failure is a tail-latency spike you did not happen to hit.

**One documented exception: `mio_perception` may be Python.** It is advisory, publishes detections
and nothing else, never issues a velocity, and its worst failure is a missed detection on one
frame. If you find yourself adding a velocity publisher to it, stop.

**`mio_gateway` is the only node allowed to speak the external protocol.** Everything else stays
inside the ROS graph. This is what makes "a language-model-adjacent process cannot command the
wheels" a demonstrable property rather than an argument. It reaches the wheels the way everything
else does — a goal to Nav2, whose velocities go through the supervisor — and it has no velocity
publisher of its own. Its token comes from the environment (`DEV_TOKEN`), never from a ROS parameter
and never from a default in the code (HEART_DECISIONS 49).

**The wheels listen to `/cmd_vel_safe`, and only `mio_safety_supervisor` publishes it**
(HEART_DECISIONS 47). Everything that wants to drive — Nav2, teleop, later the gateway — publishes
`/cmd_vel`. Never bridge `/cmd_vel` to the motors, and never publish `/cmd_vel_safe` from anywhere
else: either one removes the veto without anything failing.

**400 ms is a promise about the wheels, not a timer.** The robot is standing still within 400 ms of
the last command, so the supervisor waits `400 - brake_ms` of silence and braking gets the rest.
`brake_ms`, `max_linear`, `max_angular` and the drive's own acceleration limits are **measured
together**: raise a speed limit or weaken the brake and `brake_ms` is wrong until it is measured
again. The supervisor refuses to start when braking leaves under 240 ms of silence.

**Two safety layers, in a hierarchy, not peers.** The MCU cuts motors in hardware for cliff, bump
and E-stop, and survives the Pi freezing entirely. The Rust supervisor clamps and vetoes what
reaches the MCU but **cannot override it**. The MCU is final authority.

**Tip-over ordering never changes:** halt motors → announce aloud locally → report upward. In that
order, every time. In code: the supervisor publishes the stop, and only then `TIP_OVER` on
`/diagnostics` — which is where the voice and the gateway hear of it, so neither can get ahead of
the halt. The lock is released only by `~/clear`, and only with the robot upright. **No usable IMU
data means no motion** (HEART_DECISIONS 48).

---

## Build

```bash
./build.sh                 # NOT `colcon build` — see the comment in build.sh
source install/setup.bash
```

**If you see `ModuleNotFoundError: No module named 'catkin_pkg'`:** you called `colcon build`
directly. `~/.local/bin/python3.11` shadows the system python3.14 that ROS 2 Lyrical was built
against. `build.sh` pins the interpreter. This is a machine quirk, not a broken ROS install.

**ROS 2 Lyrical Luth on Ubuntu 26.04.** Verified: Nav2 and slam_toolbox are **not packaged for
Lyrical** — `apt install ros-lyrical-navigation2` does not resolve. They are built from source
against pinned commits. Any document saying "Jazzy" is stale.

---

## Commits

**One-line commit messages only** — no body. **Never add a `Co-Authored-By: Claude` trailer** or a
"Generated with Claude Code" line, in any commit or PR.

---

## Package layout

| Package | What it is | You write code, or configure? |
|---|---|---|
| `mio_description` | URDF, the robot's physical description | configure |
| `mio_sim` | Gazebo world. **Sim only — never ships to the Pi** | mostly configure |
| `mio_nav` | Nav2 + slam_toolbox params | **configure** — this is Open Robotics' code, don't rewrite it |
| `mio_docking` | Docking config, fiducial detection | mostly configure |
| `mio_bringup` | `sim.launch.py` and `real.launch.py` | glue |
| `mio_safety_supervisor` | **Rust. Real code.** The veto layer | write |
| `mio_gateway` | **Rust. Real code.** Talks to Brain, owns the outbox | write |
| `mio_voice` | Wake word, STT, local model, TTS glue | write the glue, wrap existing libraries |
| `mio_perception` | Object detection. **Advisory only** | write (Python permitted) |
| `mio_msgs` | Interface definitions. No logic at all | definitions only |

**The Rust packages are plain cargo crates, and `./build.sh` builds them** (HEART_DECISIONS 47).
`mio_safety_supervisor` has a small `CMakeLists.txt` that asks cargo to build and installs the
binary, so there is no second build step to forget. It needs `cargo` on the PATH and uses `rclrs` +
`ros-env` from crates.io; `ros2_rust`'s colcon tooling is **not** needed. Its tests are `cargo test`
in the package directory, with ROS sourced. `mio_gateway` is built the same way, with one
difference: **its library needs no ROS.** The wire contract (`lib.rs`) and everything the gateway
decides (`session.rs`) are plain Rust, so `cargo test` there runs anywhere — that is what CI runs,
against the vectors in `miobots-protocol`. The ROS node is the binary, behind the `ros` feature,
which `./build.sh` turns on. Keep it that way: logic goes in `session.rs`, and `main.rs` only moves
messages between the socket, Nav2 and the supervisor.

**The architectural bet: sim and real are the same code.** Only the launch file and the driver
packages differ — Gazebo plugins publish `/scan`, `/imu`, `/odom` in simulation; real drivers
publish the same topics on hardware. Same topic names, same message types. If you find yourself
writing an `if simulation` branch anywhere outside `mio_sim`, you have broken the bet.

---

## Current state

See the vault's `STATUS.md` for the project's state and `HANDOFF.md` for the detail; this covers
only what is true inside this repo.

**Done:** Phase 0.1 — Nav2 and slam_toolbox built from source against Lyrical and pinned in
`nav2-lyrical.repos`; the one-week time-box was not needed. Phase 1 — the URDF, the Gazebo Jetty
world, manual driving, odometry (1.00 m commanded → 1.0002 m on `/odom`), the simulated LiDAR, and a
saved SLAM map (`mio_nav/maps/home_arena`). Phase 2 has started: `H2.1` — `mio_nav navigation.launch.py`
runs AMCL on the saved map plus Nav2 (stock `nav2_bringup`, our values in `config/nav2_params.yaml`
marked `# mio:`), and `/cmd_vel` is `TwistStamped`; `H2.2` — it routes around a box dropped mid-run
(`mio_sim/models/obstacle_box.sdf`), with no config change needed; `H2.3` — `mio_safety_supervisor`
clamps `/cmd_vel` into `/cmd_vel_safe` and has the robot standing still within 400 ms of the last
command (240 ms of silence, then up to 160 ms of braking), and `sim.launch.py` starts it; `H2.4` —
the same node locks the wheels when the robot tips past 30° and keeps them locked until
`~/clear`. Phase 3 has started: `H3.1` — `mio_gateway` dials the Brain, turns `nav.goto` into a
Nav2 goal and sends progress and the result back, reports the supervisor's faults, and says what
the robot can do (`navigation.launch.py gateway:=true`); it remembers command keys for ten minutes
and sends a heartbeat every 5 s, which is `H3.4`. **Not started:** the durable outbox (`H3.2` — what
happens during an outage is held in memory only), telemetry to the app (pose, battery, map), speech,
and everything after. `mio_bringup` has no launch files yet.

The order, strictly: URDF → Gazebo spawn → manual driving → odometry → LiDAR → SLAM mapping →
AMCL → Nav2 goals → obstacle avoidance → safety supervisor → exploration → gateway → outbox →
voice → hardware. **Never parallelize two of these.**

---

## Where the design lives

Relative to this repo, in the Obsidian vault:

- `../../03 Engineering/Components/Heart/HEART_SPEC.md` — what Heart does and **why**
- `../../03 Engineering/Components/Heart/HEART_DECISIONS.md` — the numbered record. **Wins over
  the spec on any conflict.**
- `../../03 Engineering/Components/Heart/HEART_TASKS.md` — the ordered work, with an exit check per
  phase
- `../../03 Engineering/Protocol/ENVELOPE.md` — the wire protocol `mio_gateway` speaks

**Never treat `../../07 Archive/Unverified drafts/` as authoritative.** Those are AI-generated
first drafts with known errors, including inverted component names and a stale ROS distro.

## Working style

Label claims rather than stating them flatly — verified fact, decision, assumption, open question,
or risk. This project has already lost time to an "obvious" fact that turned out false. When
uncertain about package availability or version compatibility, **check locally rather than
guessing**.
