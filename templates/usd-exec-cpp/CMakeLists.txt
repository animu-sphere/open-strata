# SPDX-License-Identifier: Apache-2.0
cmake_minimum_required(VERSION 3.22)

project({{Name}}ExecPlugin
    VERSION 0.1.0
    DESCRIPTION "OpenExec computations for {{SchemaType}}"
    LANGUAGES CXX)

if(NOT CMAKE_CXX_STANDARD)
    set(CMAKE_CXX_STANDARD 17)
endif()
set(CMAKE_CXX_STANDARD_REQUIRED ON)
set(CMAKE_CXX_EXTENSIONS OFF)

include("${CMAKE_CURRENT_SOURCE_DIR}/cmake/OpenStrataPlugin.cmake")
openstrata_default_build_type()

if(NOT pxr_FOUND)
    find_package(pxr REQUIRED CONFIG)
endif()

set(PLUGIN_NAME {{Name}}Exec)

add_library(${PLUGIN_NAME} SHARED
    src/{{Name}}Computation.cpp
    src/{{Name}}Plugin.cpp)

target_compile_features(${PLUGIN_NAME} PRIVATE cxx_std_17)
target_include_directories(${PLUGIN_NAME} PRIVATE
    "${CMAKE_CURRENT_SOURCE_DIR}/src"
    ${PXR_INCLUDE_DIRS})

openstrata_link_openusd(
    TARGET ${PLUGIN_NAME}
    COMPONENTS exec tf vdf)

openstrata_configure_plugin(
    TARGET ${PLUGIN_NAME}
    PLUG_INFO_INPUT "${CMAKE_CURRENT_SOURCE_DIR}/plugin/resources/{{name}}/plugInfo.json.in"
    PLUG_INFO_OUTPUT "${CMAKE_CURRENT_SOURCE_DIR}/plugin/resources/{{name}}/plugInfo.json")

openstrata_install_plugin_bundle(
    TARGET ${PLUGIN_NAME}
    RESOURCES "${CMAKE_CURRENT_SOURCE_DIR}/plugin/resources/{{name}}/plugInfo.json"
    RESOURCE_DESTINATION "plugin/resources/{{name}}"
    MANIFESTS
        "${CMAKE_CURRENT_SOURCE_DIR}/openstrata.plugin.yaml"
        "${CMAKE_CURRENT_SOURCE_DIR}/openstrata.scaffold.yaml"
    FIXTURE_SOURCE "${CMAKE_CURRENT_SOURCE_DIR}/tests/fixtures")
