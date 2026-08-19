#!/usr/bin/env bash
# Build the workspace. Use this rather than calling colcon directly.
#
# The --cmake-args are not optional on this machine. ~/.local/bin/python3.11 appears in PATH
# ahead of the system python3.14 that ROS 2 Lyrical was built against, and CMake picks it up.
# That python has no catkin_pkg, so every ament package fails with:
#     ModuleNotFoundError: No module named 'catkin_pkg'
# which looks like a broken ROS install and is not one.
#
# NEVER pass --cmake-args to this script. colcon declares it as nargs='*', so a second
# --cmake-args on the command line REPLACES the list below instead of appending to it — you would
# silently lose the interpreter pinning and get the catkin_pkg failure back. Add flags here instead.
#
# Release is deliberate: debug symbols are most of the footprint of a Nav2 source build, and the
# partition this workspace lives on has ~13 GB free. Building Debug does not fit.
set -eo pipefail
# ROS's setup.bash reads AMENT_TRACE_SETUP_FILES and friends without defaulting them, so `set -u`
# must not be active while it is sourced or the build dies before colcon is ever reached.
source /opt/ros/lyrical/setup.bash
set -u
exec colcon build \
  --cmake-args -DPython3_EXECUTABLE=/usr/bin/python3 -DPYTHON_EXECUTABLE=/usr/bin/python3 \
               -DCMAKE_BUILD_TYPE=Release \
  "$@"
