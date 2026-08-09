#!/usr/bin/env bash
# Build the workspace. Use this rather than calling colcon directly.
#
# The --cmake-args are not optional on this machine. ~/.local/bin/python3.11 appears in PATH
# ahead of the system python3.14 that ROS 2 Lyrical was built against, and CMake picks it up.
# That python has no catkin_pkg, so every ament package fails with:
#     ModuleNotFoundError: No module named 'catkin_pkg'
# which looks like a broken ROS install and is not one.
set -euo pipefail
source /opt/ros/lyrical/setup.bash
exec colcon build \
  --cmake-args -DPython3_EXECUTABLE=/usr/bin/python3 -DPYTHON_EXECUTABLE=/usr/bin/python3 \
  "$@"
