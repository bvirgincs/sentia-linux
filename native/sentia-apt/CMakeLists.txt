cmake_minimum_required(VERSION 3.20)
project(sentia_apt_worker LANGUAGES CXX)

set(CMAKE_CXX_STANDARD 17)
set(CMAKE_CXX_STANDARD_REQUIRED ON)
set(CMAKE_CXX_EXTENSIONS OFF)

find_package(PkgConfig QUIET)
if(PkgConfig_FOUND)
  pkg_check_modules(APT_PKG QUIET IMPORTED_TARGET apt-pkg)
endif()

if(NOT TARGET PkgConfig::APT_PKG)
  find_path(APT_PKG_INCLUDE_DIR apt-pkg/init.h REQUIRED)
  find_library(APT_PKG_LIBRARY NAMES apt-pkg REQUIRED)
endif()

find_package(nlohmann_json QUIET)
if(NOT nlohmann_json_FOUND)
  find_path(NLOHMANN_JSON_INCLUDE_DIR nlohmann/json.hpp)
  if(NOT NLOHMANN_JSON_INCLUDE_DIR)
    message(FATAL_ERROR "nlohmann-json3-dev is required to build sentia-apt-worker")
  endif()
endif()

add_executable(
  sentia-apt-worker
  src/main.cpp
  src/apt_worker.cpp
  src/sha256.cpp
)

target_compile_options(sentia-apt-worker PRIVATE -Wall -Wextra -Wpedantic)

if(TARGET PkgConfig::APT_PKG)
  target_link_libraries(sentia-apt-worker PRIVATE PkgConfig::APT_PKG)
else()
  target_include_directories(sentia-apt-worker PRIVATE ${APT_PKG_INCLUDE_DIR})
  target_link_libraries(sentia-apt-worker PRIVATE ${APT_PKG_LIBRARY})
endif()

if(nlohmann_json_FOUND)
  target_link_libraries(sentia-apt-worker PRIVATE nlohmann_json::nlohmann_json)
else()
  target_include_directories(sentia-apt-worker PRIVATE ${NLOHMANN_JSON_INCLUDE_DIR})
endif()

install(TARGETS sentia-apt-worker RUNTIME DESTINATION libexec/sentia)
install(
  FILES
    protocol/request.schema.json
    protocol/response.schema.json
    protocol/broker-contract.json
    protocol/commands.json
  DESTINATION share/sentia-apt-worker/protocol
)
