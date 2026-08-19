# miobots-heart

The robot. ROS 2 Lyrical Luth workspace — C++ for navigation, Rust for the gateway and safety
supervisor, Python for advisory perception only.

```bash
./build.sh                 # not `colcon build` — it pins the Python interpreter, see CLAUDE.md
source install/setup.bash
```

Design documentation is in the Obsidian vault two levels up, under
`03 Engineering/Components/Heart/`. Start with `HEART_SPEC.md`.

**Status: scaffold only.** The first task is building Nav2 and slam_toolbox from source against
Lyrical — they are not packaged for it.

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
