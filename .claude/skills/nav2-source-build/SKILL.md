---
name: nav2-source-build
description: Build Nav2 and slam_toolbox from source against ROS 2 Lyrical Luth in this workspace, and pin the working commits to a .repos file. Use when starting Phase 0.1, when a Nav2/slam_toolbox build fails, when setting the workspace up on a new machine, or when re-running the build on the Pi 5 in Phase 5.2. Also use when someone asks why `apt install ros-lyrical-navigation2` does not resolve.
---

# Nav2 + slam_toolbox source build (Lyrical)

**Why this exists:** Nav2 and slam_toolbox are not packaged for Lyrical. Verified by direct `apt`
failure and by `index.ros.org` showing no Lyrical release for `slam_toolbox` (HEART_DECISIONS §10).
A source build is the only path, and it has to be repeated on **every** machine that needs it —
including the arm64 Pi 5 in Phase 5.2. That repetition is why the `.repos` file at the end is the
actual deliverable, not the build.

**Duration is genuinely unknown.** Time-boxed to one week of real attempts; after that, fall back
to a containerised Jazzy environment.

## Before you start

**Do not create `~/miobots_ws`.** The workspace already exists and already builds — it is this
repo. Clone into this repo's `src/`.

**Never call `colcon build` directly here.** `~/.local/bin/python3.11` shadows the system
`python3.14` that ROS Lyrical was built against; CMake picks it up, and every ament package fails
with `ModuleNotFoundError: No module named 'catkin_pkg'`. It looks exactly like a broken ROS
install and is not one (decision 30). `./build.sh` pins the interpreter and forwards its arguments.
A PreToolUse hook blocks the bare form.

## Steps

```bash
source /opt/ros/lyrical/setup.bash          # must be silent
sudo apt install python3-colcon-common-extensions python3-vcstool python3-rosdep
sudo rosdep init                            # skip if already initialised
rosdep update
```

Clone both, into this repo's `src/`. **Both repos have a maintained `lyrical` branch** — use it, not
`main` (decision 32). Nav2 also moved org, `ros-planning` → `ros-navigation`; the old URL is dead.

```bash
git clone https://github.com/ros-navigation/navigation2.git   --branch lyrical src/navigation2
git clone https://github.com/SteveMacenski/slam_toolbox.git   --branch lyrical src/slam_toolbox
```

```bash
rosdep install -r -y --from-paths src --ignore-src   # expect it to surface missing system deps
./build.sh --symlink-install --parallel-workers 4
```

**Never pass `--cmake-args` to `build.sh`** (decision 32). colcon declares it `nargs='*'`, so a
second occurrence *replaces* the interpreter pinning rather than appending to it, and you get
decision 30's `catkin_pkg` failure back while believing you only added a flag. Add flags inside
`build.sh`.

`--parallel-workers` is a memory cap. Nav2's C++ will OOM at default parallelism across 12 cores if
available RAM is low; check `free -g` and drop to 2 if under ~5 GB available.

**Expect the build to fail at least once.** Read the actual error; do not retry blindly. The three
failure classes at this stage, in the order they usually appear:

1. A missing system dependency `rosdep` did not resolve — install it and re-run.
2. An API that changed between the distro these packages target and Lyrical's actual API. This is
   the expensive one. Search the exact error text plus "ROS 2 Lyrical", then check open issues on
   the Nav2 and slam_toolbox repos for someone hitting the same thing. **Much less likely now** —
   the `lyrical` branches are maintained against this distro, which is most of why the one-week
   time-box was not needed.
3. The interpreter shadowing from decision 30, if `colcon` got called directly somewhere.

For a wall of CMake output, hand it to the `ros2-build-debugger` subagent rather than reading the
cascade — the first real error is usually hundreds of lines above the last one.

## Exit check

Both must return results:

```bash
source install/setup.bash
ros2 pkg list | grep nav2
ros2 pkg list | grep slam_toolbox
```

## Then pin it — this is the deliverable

Record the exact commits that worked, in vcstool format, and commit the file:

```bash
vcs export src --exact > nav2-lyrical.repos
git add nav2-lyrical.repos && git commit -m "Pin Nav2 + slam_toolbox commits that build on Lyrical"
```

Without this file the build is rediscovered rather than repeated, on every machine and again on the
Pi. Record in `HEART_DECISIONS.md` which branches you ended up on and any patch you had to apply.

## On the Pi (Phase 5.2)

Same procedure, `vcs import src < nav2-lyrical.repos` instead of the clones. **Budget hours, not
minutes.** Watch memory during compilation — add swap or cross-compile if it struggles.
