# mio_voice — the offline voice pipeline, standalone (H4.0).
#
# Plain CMake on purpose: no ament, no ROS. Build it out of the workspace tree:
#
#   ./scripts/fetch_deps.sh
#   cmake -S . -B ~/.cache/mio_voice/build && cmake --build ~/.cache/mio_voice/build -j4
#   ctest --test-dir ~/.cache/mio_voice/build
#
# colcon skips this directory (COLCON_IGNORE) until H4.1 wraps the pipeline as a ROS node; that is
# when ament comes back. Nothing here downloads anything — scripts/fetch_deps.sh is the only part
# of mio_voice that touches the network.
cmake_minimum_required(VERSION 3.16)
project(mio_voice CXX C)

set(CMAKE_CXX_STANDARD 17)
set(CMAKE_CXX_STANDARD_REQUIRED ON)
if(NOT CMAKE_BUILD_TYPE)
  set(CMAKE_BUILD_TYPE Release)  # an unoptimised recogniser is several times slower
endif()

set(MIO_VOICE_HOME "$ENV{MIO_VOICE_HOME}" CACHE PATH "Where scripts/fetch_deps.sh put the engines")
if(NOT MIO_VOICE_HOME)
  set(MIO_VOICE_HOME "$ENV{HOME}/.cache/mio_voice")
endif()
if(NOT EXISTS "${MIO_VOICE_HOME}/whisper.cpp/CMakeLists.txt" OR
   NOT EXISTS "${MIO_VOICE_HOME}/sherpa-onnx/lib/libsherpa-onnx-c-api.so")
  message(FATAL_ERROR "Engines not found in ${MIO_VOICE_HOME}. Run scripts/fetch_deps.sh first.")
endif()

# Speech to text: whisper.cpp, compiled in, CPU only.
set(BUILD_SHARED_LIBS OFF CACHE BOOL "" FORCE)
set(WHISPER_BUILD_EXAMPLES OFF CACHE BOOL "" FORCE)
set(WHISPER_BUILD_TESTS OFF CACHE BOOL "" FORCE)
set(WHISPER_BUILD_SERVER OFF CACHE BOOL "" FORCE)
add_subdirectory("${MIO_VOICE_HOME}/whisper.cpp" whisper.cpp EXCLUDE_FROM_ALL)

# Wake word, voice activity and speech synthesis: sherpa-onnx, prebuilt.
add_library(sherpa_onnx SHARED IMPORTED)
set_target_properties(sherpa_onnx PROPERTIES
  IMPORTED_LOCATION "${MIO_VOICE_HOME}/sherpa-onnx/lib/libsherpa-onnx-c-api.so"
  INTERFACE_INCLUDE_DIRECTORIES "${MIO_VOICE_HOME}/sherpa-onnx/include")

find_package(nlohmann_json REQUIRED)
find_package(Threads REQUIRED)

# The decisions, with no engine linked in — which is what the tests exercise.
add_library(mio_voice_core STATIC src/core.cpp)
target_include_directories(mio_voice_core PUBLIC src)
target_link_libraries(mio_voice_core PUBLIC nlohmann_json::nlohmann_json)
target_compile_options(mio_voice_core PRIVATE -Wall -Wextra)

add_executable(mio_voice src/main.cpp src/engines.cpp)
target_link_libraries(mio_voice PRIVATE mio_voice_core whisper sherpa_onnx Threads::Threads)
target_compile_options(mio_voice PRIVATE -Wall -Wextra)
set_target_properties(mio_voice PROPERTIES BUILD_RPATH "${MIO_VOICE_HOME}/sherpa-onnx/lib")

enable_testing()
add_executable(test_core tests/test_core.cpp)
target_link_libraries(test_core PRIVATE mio_voice_core)
add_test(NAME core COMMAND test_core)

# The tables live in the source tree and are read at run time (override with --config).
foreach(target mio_voice test_core)
  target_compile_definitions(${target} PRIVATE "MIO_VOICE_CONFIG_DIR=\"${CMAKE_CURRENT_SOURCE_DIR}/config\"")
endforeach()
