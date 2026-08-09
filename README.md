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
