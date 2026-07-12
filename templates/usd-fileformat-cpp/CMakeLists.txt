# SPDX-License-Identifier: Apache-2.0
cmake_minimum_required(VERSION 3.22)

project({{Name}}Plugin
    VERSION 0.1.0
    DESCRIPTION "OpenUSD {{extension}} file-format plugin"
    LANGUAGES CXX)

if(NOT CMAKE_CXX_STANDARD)
    set(CMAKE_CXX_STANDARD 17)
endif()
set(CMAKE_CXX_STANDARD_REQUIRED ON)
set(CMAKE_CXX_EXTENSIONS OFF)

include("${CMAKE_CURRENT_SOURCE_DIR}/cmake/OpenStrataPlugin.cmake")
openstrata_default_build_type()

# Resolved from the runtime OpenStrata activates (toolchain sets CMAKE_PREFIX_PATH).
# Guarded so a parent project (a dual-mode root that `add_subdirectory()`s this
# bundle) can resolve OpenUSD once and have it flow down here.
if(NOT pxr_FOUND)
    find_package(pxr REQUIRED CONFIG)
endif()

set(PLUGIN_NAME {{Name}}FileFormat)

add_library(${PLUGIN_NAME} SHARED
    src/{{Name}}FileFormat.cpp)

target_compile_features(${PLUGIN_NAME} PRIVATE cxx_std_17)
target_include_directories(${PLUGIN_NAME} PRIVATE
    "${CMAKE_CURRENT_SOURCE_DIR}/src"
    ${PXR_INCLUDE_DIRS})

# `ost plugin build` writes this fragment when the bundle co-hosts a compiled
# schema (`schema.usda` + `provides: usd-schema:*`). It adds the generated typed
# API sources to this same shared library and defines the generated `*_EXPORTS`
# macro so the schema symbols are exported from the plugin binary. Plain
# file-format bundles have no fragment and stay unchanged.
if(DEFINED OPENSTRATA_SCHEMA_SOURCES_FILE AND EXISTS "${OPENSTRATA_SCHEMA_SOURCES_FILE}")
    include("${OPENSTRATA_SCHEMA_SOURCES_FILE}")
endif()

openstrata_link_openusd(
    TARGET ${PLUGIN_NAME}
    COMPONENTS arch tf gf vt sdf usd)

# The copied helper keeps output/install mechanics identical in standalone and
# add_subdirectory workspace builds. This file still owns the target, OpenUSD
# components, resource path, and fixtures as the bundle's readable policy.
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
