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
wheels" a demonstrable property rather than an argument.

**Two safety layers, in a hierarchy, not peers.** The MCU cuts motors in hardware for cliff, bump
and E-stop, and survives the Pi freezing entirely. The Rust supervisor clamps and vetoes what
reaches the MCU but **cannot override it**. The MCU is final authority.

**Tip-over ordering never changes:** halt motors → announce aloud locally → report upward. In that
order, every time.

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

**The two Rust packages carry `COLCON_IGNORE` on purpose.** They are plain cargo crates for now;
building them as ROS 2 packages needs `ros2_rust` (colcon-cargo + cargo-ament-build), which is not
installed and is its own task. Delete the ignore file once that is set up. Until then `cargo build`
works and colcon skips them, so the workspace still builds.

**The architectural bet: sim and real are the same code.** Only the launch file and the driver
packages differ — Gazebo plugins publish `/scan`, `/imu`, `/odom` in simulation; real drivers
publish the same topics on hardware. Same topic names, same message types. If you find yourself
writing an `if simulation` branch anywhere outside `mio_sim`, you have broken the bet.

---

## Current state

**Scaffold only. Nothing is implemented.**

The first task is **not** in this repo — it is building Nav2 and slam_toolbox from source against
Lyrical, pinned in a `.repos` file committed here. Everything else waits on it, and its duration is
genuinely unknown. Time-boxed to one week of real attempts, after which fall back to a
containerised Jazzy environment.

Then, strictly in order: URDF → Gazebo spawn → manual driving → odometry → LiDAR → SLAM mapping →
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
