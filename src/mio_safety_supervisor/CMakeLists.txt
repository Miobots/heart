cmake_minimum_required(VERSION 3.12)
project(mio_safety_supervisor)

find_package(ament_cmake REQUIRED)
find_program(CARGO cargo REQUIRED)

# cargo does the building. CMake only asks it to and installs the result, so `./build.sh` builds
# the supervisor with everything else and `ros2 launch` can find it — nobody drives on a stale
# safety binary because they forgot a second build step. `--locked`: the versions in Cargo.lock, or
# nothing. `--quiet`: cargo reports progress on stderr, and colcon should only flag this package
# when something is actually wrong. Tests are still `cargo test` in this directory, with ROS sourced.
add_custom_target(mio_safety_supervisor_cargo ALL
  COMMAND ${CARGO} build --release --locked --quiet
          --manifest-path ${CMAKE_CURRENT_SOURCE_DIR}/Cargo.toml
          --target-dir ${CMAKE_CURRENT_BINARY_DIR}/cargo
  VERBATIM
)

install(
  PROGRAMS ${CMAKE_CURRENT_BINARY_DIR}/cargo/release/mio_safety_supervisor
  DESTINATION lib/${PROJECT_NAME}
)

ament_package()
