# SPDX-License-Identifier: Apache-2.0
cmake_minimum_required(VERSION 3.24)

project({{Name}}
  VERSION 0.1.0
  DESCRIPTION "{{name}} — an OpenStrata renderer skeleton"
  LANGUAGES CXX)

include(GNUInstallDirs)
include(CMakePackageConfigHelpers)

option({{NAME}}_ENABLE_VULKAN "Probe and link the Vulkan backend capability" ON)

# OST-facing build intent. The renderer keeps its project-owned option while
# `ost renderer view` can request the co-built adapter without knowing that
# private option name. A non-empty OST intent synchronizes both adapter options
# so switching intents cannot retain a stale CMake cache value; an empty intent
# leaves direct CMake option overrides in control.
set(OST_RENDERER_ADAPTERS "" CACHE STRING
  "Semicolon-separated renderer adapters requested by OpenStrata")
set(_{{NAME}}_MANAGED_ADAPTER_INTENT OFF)
if(NOT OST_RENDERER_ADAPTERS STREQUAL "")
  set(_{{NAME}}_MANAGED_ADAPTER_INTENT ON)
endif()
set(_{{NAME}}_HYDRA2_DEFAULT OFF)
if("hydra2" IN_LIST OST_RENDERER_ADAPTERS)
  set(_{{NAME}}_HYDRA2_DEFAULT ON)
endif()
set(_{{NAME}}_VIEWPORT_DEFAULT OFF)
if("viewport" IN_LIST OST_RENDERER_ADAPTERS)
  set(_{{NAME}}_VIEWPORT_DEFAULT ON)
endif()
if(_{{NAME}}_MANAGED_ADAPTER_INTENT)
  set({{NAME}}_ENABLE_HYDRA2 ${_{{NAME}}_HYDRA2_DEFAULT} CACHE BOOL
    "Build the optional co-built OpenUSD Hydra 2 adapter" FORCE)
  set({{NAME}}_ENABLE_VIEWPORT ${_{{NAME}}_VIEWPORT_DEFAULT} CACHE BOOL
    "Build the optional GLFW/Vulkan viewport adapter" FORCE)
else()
  option({{NAME}}_ENABLE_HYDRA2 "Build the optional co-built OpenUSD Hydra 2 adapter"
    ${_{{NAME}}_HYDRA2_DEFAULT})
  option({{NAME}}_ENABLE_VIEWPORT "Build the optional GLFW/Vulkan viewport adapter"
    ${_{{NAME}}_VIEWPORT_DEFAULT})
endif()
option({{NAME}}_BUILD_TESTS "Build renderer contract and install-tree tests" ON)

set(CMAKE_CXX_STANDARD 20)
set(CMAKE_CXX_STANDARD_REQUIRED ON)
set(CMAKE_CXX_EXTENSIONS OFF)

if({{NAME}}_BUILD_TESTS)
  include(CTest)
endif()

if(MSVC)
  add_compile_options(/W4 /permissive- /EHsc)
else()
  add_compile_options(-Wall -Wextra -Wpedantic)
endif()

set({{NAME}}_WITH_VULKAN OFF)
if({{NAME}}_ENABLE_VULKAN)
  find_package(Vulkan 1.3 QUIET)
  # Slang is the shader source of truth. The Vulkan SDK bundles slangc since
  # 1.3.296, so one SDK install provides both the loader and the compiler.
  find_program({{NAME}}_SLANGC_EXECUTABLE
    NAMES slangc
    HINTS ENV VULKAN_SDK
    PATH_SUFFIXES bin Bin)
  if(Vulkan_FOUND AND {{NAME}}_SLANGC_EXECUTABLE)
    set({{NAME}}_WITH_VULKAN ON)
  elseif(NOT Vulkan_FOUND)
    message(STATUS "Vulkan 1.3 was not found; GPU assertions will report SKIP")
  else()
    message(STATUS
      "slangc was not found (the Vulkan SDK bundles it since 1.3.296); "
      "GPU assertions will report SKIP")
  endif()
endif()

add_subdirectory(core/render-world)
add_subdirectory(core/render-extraction)
add_subdirectory(backend/vulkan)
add_subdirectory(adapters/headless)
if({{NAME}}_ENABLE_HYDRA2)
  add_subdirectory(adapters/hydra2)
  target_compile_definitions({{name}}-headless PRIVATE {{NAME}}_HAS_HYDRA2=1)
endif()
if({{NAME}}_ENABLE_VIEWPORT)
  add_subdirectory(adapters/viewport)
endif()

if({{NAME}}_BUILD_TESTS)
  add_subdirectory(validation)
endif()

# One project-level install graph. Internal source packs are CMake targets, not
# independently versioned packages.
install(TARGETS
    {{name}}-render-world
    {{name}}-render-extraction
    {{name}}-render-vulkan
  EXPORT {{Name}}Targets
  ARCHIVE DESTINATION "${CMAKE_INSTALL_LIBDIR}"
  LIBRARY DESTINATION "${CMAKE_INSTALL_LIBDIR}"
  RUNTIME DESTINATION "${CMAKE_INSTALL_BINDIR}")
install(TARGETS {{name}}-headless
  RUNTIME DESTINATION "${CMAKE_INSTALL_BINDIR}")
if({{NAME}}_WITH_VULKAN)
  install(FILES "${{{NAME}}_VERTEX_SPV}" "${{{NAME}}_FRAGMENT_SPV}"
    DESTINATION "${CMAKE_INSTALL_BINDIR}/shaders")
endif()
install(DIRECTORY include/ DESTINATION "${CMAKE_INSTALL_INCLUDEDIR}")

set({{Name}}_CONFIG_INSTALL_DIR "${CMAKE_INSTALL_LIBDIR}/cmake/{{Name}}")
configure_package_config_file(
  cmake/{{Name}}Config.cmake.in
  "${CMAKE_CURRENT_BINARY_DIR}/{{Name}}Config.cmake"
  INSTALL_DESTINATION "${{{Name}}_CONFIG_INSTALL_DIR}")
write_basic_package_version_file(
  "${CMAKE_CURRENT_BINARY_DIR}/{{Name}}ConfigVersion.cmake"
  VERSION "${PROJECT_VERSION}"
  COMPATIBILITY SameMajorVersion)
install(EXPORT {{Name}}Targets
  FILE {{Name}}Targets.cmake
  NAMESPACE {{Name}}::
  DESTINATION "${{{Name}}_CONFIG_INSTALL_DIR}")
install(FILES
    "${CMAKE_CURRENT_BINARY_DIR}/{{Name}}Config.cmake"
    "${CMAKE_CURRENT_BINARY_DIR}/{{Name}}ConfigVersion.cmake"
  DESTINATION "${{{Name}}_CONFIG_INSTALL_DIR}")
install(FILES openstrata.renderer.yaml openstrata.scaffold.yaml
  DESTINATION "${CMAKE_INSTALL_DATADIR}/{{name}}")
