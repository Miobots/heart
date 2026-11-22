cmake_minimum_required(VERSION 3.12)
project(mio_gateway)

find_package(ament_cmake REQUIRED)
find_program(CARGO cargo REQUIRED)

# cargo does the building, as for mio_safety_supervisor (HEART_DECISIONS 47): one `./build.sh`
# builds the node and `ros2 run` finds it. `--features ros` is the node itself; without it the
# crate is only the wire contract and the session logic, which is what CI tests with no ROS.
add_custom_target(mio_gateway_cargo ALL
  COMMAND ${CARGO} build --release --locked --quiet --features ros
          --manifest-path ${CMAKE_CURRENT_SOURCE_DIR}/Cargo.toml
          --target-dir ${CMAKE_CURRENT_BINARY_DIR}/cargo
  VERBATIM
)

install(
  PROGRAMS ${CMAKE_CURRENT_BINARY_DIR}/cargo/release/mio_gateway
  DESTINATION lib/${PROJECT_NAME}
)

ament_package()
