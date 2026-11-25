cmake_minimum_required(VERSION 3.12)
project(mio_sim)

find_package(ament_cmake REQUIRED)

install(
  DIRECTORY config launch models worlds
  DESTINATION share/${PROJECT_NAME}
)

# The small house's furniture (worlds/small_house.sdf) comes from sim-assets.repos, not this repo.
# Installed next to our own models, so one GZ_SIM_RESOURCE_PATH entry resolves every model:// URI.
set(HOUSE_MODELS ${CMAKE_CURRENT_SOURCE_DIR}/../../sim_assets/aws-robomaker-small-house-world/models)
if(EXISTS ${HOUSE_MODELS})
  install(DIRECTORY ${HOUSE_MODELS}/ DESTINATION share/${PROJECT_NAME}/models PATTERN model.sdf EXCLUDE)
  # Upstream's inertial blocks don't load in gz-sim 10: 46 give a <mass> with no <inertia>, and the
  # shoe rack lists ixx twice and izz never. Gazebo Classic accepted them; gz-sim refuses the whole
  # world. Every house model is static, so mass is irrelevant: the block is dropped and sdformat's
  # defaults are used. The upstream files are left untouched.
  file(GLOB HOUSE_SDFS ${HOUSE_MODELS}/*/model.sdf)
  foreach(SDF ${HOUSE_SDFS})
    get_filename_component(DIR ${SDF} DIRECTORY)
    get_filename_component(NAME ${DIR} NAME)
    file(READ ${SDF} TEXT)
    string(FIND "${TEXT}" "<inertial>" START)
    while(NOT START EQUAL -1)
      string(FIND "${TEXT}" "</inertial>" END)
      math(EXPR END "${END} + 11")
      string(SUBSTRING "${TEXT}" 0 ${START} BEFORE)
      string(SUBSTRING "${TEXT}" ${END} -1 AFTER)
      set(TEXT "${BEFORE}${AFTER}")
      string(FIND "${TEXT}" "<inertial>" START)
    endwhile()
    file(WRITE ${CMAKE_CURRENT_BINARY_DIR}/house_models/${NAME}/model.sdf "${TEXT}")
  endforeach()
  install(DIRECTORY ${CMAKE_CURRENT_BINARY_DIR}/house_models/ DESTINATION share/${PROJECT_NAME}/models)
else()
  message(WARNING "small_house models missing — run: mkdir -p sim_assets && vcs import sim_assets < sim-assets.repos")
endif()

ament_package()
