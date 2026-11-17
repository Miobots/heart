# miobots-heart

The robot. ROS 2 Lyrical Luth workspace — C++ for navigation, Rust for the gateway and safety
supervisor, Python for advisory perception only.

```bash
./build.sh                 # not `colcon build` — it pins the Python interpreter, see CLAUDE.md
source install/setup.bash
```

Design documentation is in the Obsidian vault two levels up, under
`03 Engineering/Components/Heart/`. Start with `HEART_SPEC.md`.

**Phase 0.1 and Phase 1 are done:** Nav2 and slam_toolbox build from source against Lyrical (pinned
in `nav2-lyrical.repos`), and the simulated robot drives, maps and saves a map:

```bash
ros2 launch mio_sim sim.launch.py          # robot in the home_arena world (headless:=true for no GUI)
ros2 launch mio_nav mapping.launch.py      # slam_toolbox against the simulated /scan
ros2 launch mio_nav navigation.launch.py   # AMCL on the saved map + Nav2; click "Nav2 Goal" in RViz
ros2 run teleop_twist_keyboard teleop_twist_keyboard --ros-args -p stamped:=true   # /cmd_vel is TwistStamped
```

**Hold the key to drive.** `sim.launch.py` also starts `mio_safety_supervisor`, which is the only
thing the wheels listen to: it has the robot standing still within 400 ms of the last command, so
a tap is a nudge. It also locks the wheels if the robot tips past 30°, until someone sets it upright
and runs `ros2 service call /mio_safety_supervisor/clear std_srvs/srv/Trigger`.

**To let the Brain drive it**, start the Brain and add the gateway to the navigation launch:

```bash
DEV_TOKEN=<the Brain's DEV_TOKEN> BRAIN_URL=ws://127.0.0.1:8080/ws \
  ros2 launch mio_nav navigation.launch.py gateway:=true
```

Phase 2 runs in simulation — `H2.1` (AMCL and Nav2 goals), `H2.2` (routing around a box dropped mid-run), `H2.3` (the safety supervisor), `H2.4` (the tip-over lock) — and so does the first of Phase 3, `H3.1` (the gateway). `HANDOFF.md` has the details;
the project's state is in the vault's `STATUS.md`.

## Agent tooling

`.claude/` is committed, so cloning this repo gets you the hooks, the skill and the subagent. Claude
Code will ask you to trust the workspace on first open — the hooks do not run until you accept.

| What | Where | Does |
|---|---|---|
| `guard.sh` | PreToolUse hook | Blocks bare `colcon build`, Python in the Rust control-path packages, and a `cmd_vel` publisher in `mio_perception` |
| `rust-fmt.sh` | PostToolUse hook | `cargo fmt` on the owning crate after a `.rs` edit |
| `nav2-source-build` | skill | The Phase 0.1 source build, and the same procedure for the Pi in Phase 5.2 |
| `ros2-build-debugger` | subagent | Reads a failed build log and returns the first real error plus a diagnosis |

After changing `guard.sh`, run its self-test:

```bash
.claude/hooks/guard.test.sh
```

Two tools are **per-machine** and cannot be committed — each person installs them once:

```bash
uv tool install graphifyy && graphify install --platform claude   # code knowledge graph
npx claude-mem install && npx claude-mem start                    # cross-session memory
```

Graphify writes to `graphify-out/` (gitignored — rebuild with `graphify update .`). claude-mem
stores everything in `~/.claude-mem` and runs a local worker on `127.0.0.1:37700`.
