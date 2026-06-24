# SPDX-License-Identifier: Apache-2.0
cmake_minimum_required(VERSION 3.23)

project({{Name}}Plugin
    VERSION 0.1.0
    DESCRIPTION "{{name}} — an OpenUSD plugin managed by OpenStrata"
    LANGUAGES CXX)

if(NOT CMAKE_CXX_STANDARD)
    set(CMAKE_CXX_STANDARD 17)
endif()
set(CMAKE_CXX_STANDARD_REQUIRED ON)
set(CMAKE_CXX_EXTENSIONS OFF)

# Resolved from the runtime OpenStrata activates (toolchain sets the USD roots).
find_package(pxr REQUIRED CONFIG)

set(PLUGIN_NAME {{name}})

add_library(${PLUGIN_NAME} SHARED
    src/{{name}}.cpp)

target_compile_features(${PLUGIN_NAME} PRIVATE cxx_std_17)
target_include_directories(${PLUGIN_NAME} PRIVATE
    "${CMAKE_CURRENT_SOURCE_DIR}/src"
    ${PXR_INCLUDE_DIRS})

# Link the USD monolith if present, else the discrete libraries.
if(TARGET usd_ms)
    target_link_libraries(${PLUGIN_NAME} PRIVATE usd_ms)
else()
    set(_pxr_targets)
    foreach(_t IN ITEMS arch tf plug)
        if(TARGET ${_t})
            list(APPEND _pxr_targets ${_t})
        elseif(TARGET pxr::${_t})
            list(APPEND _pxr_targets pxr::${_t})
        endif()
    endforeach()
    if(_pxr_targets)
        target_link_libraries(${PLUGIN_NAME} PRIVATE ${_pxr_targets})
    elseif(PXR_LIBRARIES)
        target_link_libraries(${PLUGIN_NAME} PRIVATE ${PXR_LIBRARIES})
    else()
        message(FATAL_ERROR "Could not locate OpenUSD CMake targets or PXR_LIBRARIES.")
    endif()
endif()

# USD discovers plugins by a directory containing plugInfo.json; the shared
# library sits next to it.
set_target_properties(${PLUGIN_NAME} PROPERTIES PREFIX "")

# Install rules so `ost package` produces a non-empty, loadable plugin.
install(TARGETS ${PLUGIN_NAME}
    LIBRARY DESTINATION plugin
    RUNTIME DESTINATION plugin)
install(FILES plugin/resources/plugInfo.json
    DESTINATION plugin/resources)
