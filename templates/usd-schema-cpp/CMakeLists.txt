# SPDX-License-Identifier: Apache-2.0
cmake_minimum_required(VERSION 3.22)

project({{Name}}
    VERSION 0.1.0
    DESCRIPTION "Compiled OpenUSD API schema"
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

set(OPENSTRATA_SCHEMA_GENERATION_MODE "AUTO" CACHE STRING
    "Schema generation mode: AUTO, GENERATE, PREGENERATED, or VERIFY")
set_property(CACHE OPENSTRATA_SCHEMA_GENERATION_MODE PROPERTY STRINGS
    AUTO GENERATE PREGENERATED VERIFY)
string(TOUPPER "${OPENSTRATA_SCHEMA_GENERATION_MODE}" _schema_mode)
set(_schema_modes AUTO GENERATE PREGENERATED VERIFY)
if(NOT _schema_mode IN_LIST _schema_modes)
    message(FATAL_ERROR
        "OPENSTRATA_SCHEMA_GENERATION_MODE='${OPENSTRATA_SCHEMA_GENERATION_MODE}' "
        "is invalid; choose AUTO, GENERATE, PREGENERATED, or VERIFY")
endif()

set(_schema_source "${CMAKE_CURRENT_SOURCE_DIR}/schema.usda")
set(_committed_cpp "${CMAKE_CURRENT_SOURCE_DIR}/generated")
set(_committed_plug_info
    "${CMAKE_CURRENT_SOURCE_DIR}/plugin/resources/{{name}}/plugInfo.json.in")
set(_committed_generated_schema
    "${CMAKE_CURRENT_SOURCE_DIR}/plugin/resources/{{name}}/generatedSchema.usda")

find_program(USD_GEN_SCHEMA usdGenSchema)
find_program(USD_SCHEMA_PYTHON NAMES python python3)
set(_can_generate FALSE)
if(USD_GEN_SCHEMA AND USD_SCHEMA_PYTHON)
    set(_can_generate TRUE)
endif()

set(_use_generated FALSE)
if(_schema_mode STREQUAL "GENERATE" OR _schema_mode STREQUAL "VERIFY")
    if(NOT _can_generate)
        message(FATAL_ERROR
            "${_schema_mode} requires usdGenSchema and Python on PATH. "
            "Use PREGENERATED or activate a compatible OpenUSD runtime.")
    endif()
    set(_use_generated TRUE)
elseif(_schema_mode STREQUAL "AUTO" AND _can_generate)
    set(_use_generated TRUE)
elseif(_schema_mode STREQUAL "AUTO")
    message(STATUS
        "OpenStrata: usdGenSchema/Python unavailable; using committed generated sources")
endif()

if(_use_generated)
    set(_fresh "${CMAKE_CURRENT_BINARY_DIR}/schema-generated")
    file(REMOVE_RECURSE "${_fresh}")
    file(MAKE_DIRECTORY "${_fresh}")
    execute_process(
        COMMAND "${CMAKE_COMMAND}" -E env
            "PYTHONUTF8=1"
            "PYTHONIOENCODING=utf-8"
            "${USD_SCHEMA_PYTHON}" "${USD_GEN_SCHEMA}" "${_schema_source}" "${_fresh}"
        WORKING_DIRECTORY "${CMAKE_CURRENT_SOURCE_DIR}"
        RESULT_VARIABLE _generate_result
        COMMAND_ECHO STDOUT)
    if(NOT _generate_result EQUAL 0)
        message(FATAL_ERROR "usdGenSchema failed with exit code ${_generate_result}")
    endif()
endif()

if(_schema_mode STREQUAL "VERIFY")
    foreach(_file api.h contractAPI.h contractAPI.cpp tokens.h tokens.cpp)
        execute_process(
            COMMAND "${CMAKE_COMMAND}" -E compare_files
                "${_fresh}/${_file}" "${_committed_cpp}/${_file}"
            RESULT_VARIABLE _different)
        if(_different)
            message(FATAL_ERROR
                "usdGenSchema VERIFY mismatch: generated/${_file}; "
                "run GENERATE with the supported generator and commit the result")
        endif()
    endforeach()
    execute_process(
        COMMAND "${CMAKE_COMMAND}" -E compare_files
            "${_fresh}/plugInfo.json" "${_committed_plug_info}"
        RESULT_VARIABLE _plug_info_different)
    execute_process(
        COMMAND "${CMAKE_COMMAND}" -E compare_files
            "${_fresh}/generatedSchema.usda" "${_committed_generated_schema}"
        RESULT_VARIABLE _schema_different)
    if(_plug_info_different OR _schema_different)
        message(FATAL_ERROR
            "usdGenSchema VERIFY mismatch in plugInfo.json or generatedSchema.usda")
    endif()
    message(STATUS "OpenStrata: committed schema outputs match usdGenSchema")
endif()

if(_use_generated)
    set(_schema_cpp "${_fresh}")
    set(_plug_info_input "${_fresh}/plugInfo.json")
    set(_generated_schema_input "${_fresh}/generatedSchema.usda")
else()
    set(_schema_cpp "${_committed_cpp}")
    set(_plug_info_input "${_committed_plug_info}")
    set(_generated_schema_input "${_committed_generated_schema}")
endif()

set(PLUGIN_NAME {{Name}})
add_library(${PLUGIN_NAME} SHARED
    "${_schema_cpp}/contractAPI.cpp"
    "${_schema_cpp}/tokens.cpp")
add_library({{Name}}::Schema ALIAS ${PLUGIN_NAME})
set_target_properties(${PLUGIN_NAME} PROPERTIES EXPORT_NAME Schema)

target_compile_features(${PLUGIN_NAME} PUBLIC cxx_std_17)
target_compile_definitions(${PLUGIN_NAME} PRIVATE {{NAME}}_EXPORTS)
if(MSVC)
    # OpenUSD headers include Windows headers through arch. Export NOMINMAX to
    # installed consumers too, otherwise std::min/std::max in generated schema
    # headers are macro-expanded by <windows.h>.
    target_compile_definitions(${PLUGIN_NAME} PUBLIC NOMINMAX)
endif()
target_include_directories(${PLUGIN_NAME}
    PUBLIC
        "$<BUILD_INTERFACE:${_schema_cpp}>"
        "$<INSTALL_INTERFACE:include>"
    PRIVATE ${PXR_INCLUDE_DIRS})

# Public because the installed generated headers include OpenUSD headers and
# downstream C++ consumers use the same ABI-bearing types.
openstrata_link_openusd(
    TARGET ${PLUGIN_NAME}
    VISIBILITY PUBLIC
    COMPONENTS arch tf gf vt sdf usd)

set(PLUG_INFO_LIBRARY_PATH
    "../../../lib/lib{{Name}}${CMAKE_SHARED_LIBRARY_SUFFIX}")
set(PLUG_INFO_RESOURCE_PATH ".")
set(PLUG_INFO_ROOT ".")
openstrata_configure_plugin(
    TARGET ${PLUGIN_NAME}
    PLUG_INFO_INPUT "${_plug_info_input}"
    PLUG_INFO_OUTPUT
        "${CMAKE_CURRENT_SOURCE_DIR}/plugin/resources/{{name}}/plugInfo.json")

if(NOT _generated_schema_input STREQUAL _committed_generated_schema)
    configure_file(
        "${_generated_schema_input}"
        "${_committed_generated_schema}"
        COPYONLY)
endif()

openstrata_install_plugin_bundle(
    TARGET ${PLUGIN_NAME}
    EXPORT_SET {{Name}}Targets
    RESOURCES
        "${CMAKE_CURRENT_SOURCE_DIR}/plugin/resources/{{name}}/plugInfo.json"
        "${_committed_generated_schema}"
    RESOURCE_DESTINATION "plugin/resources/{{name}}"
    MANIFESTS
        "${CMAKE_CURRENT_SOURCE_DIR}/openstrata.plugin.yaml"
        "${CMAKE_CURRENT_SOURCE_DIR}/openstrata.scaffold.yaml"
    FIXTURE_SOURCE "${CMAKE_CURRENT_SOURCE_DIR}/tests/fixtures")

install(FILES
    "${_schema_cpp}/api.h"
    "${_schema_cpp}/contractAPI.h"
    "${_schema_cpp}/tokens.h"
    DESTINATION "include/{{name}}")

include(CMakePackageConfigHelpers)
configure_package_config_file(
    "${CMAKE_CURRENT_SOURCE_DIR}/cmake/{{Name}}Config.cmake.in"
    "${CMAKE_CURRENT_BINARY_DIR}/{{Name}}Config.cmake"
    INSTALL_DESTINATION "lib/cmake/{{Name}}")
write_basic_package_version_file(
    "${CMAKE_CURRENT_BINARY_DIR}/{{Name}}ConfigVersion.cmake"
    VERSION ${PROJECT_VERSION}
    COMPATIBILITY SameMajorVersion)
install(EXPORT {{Name}}Targets
    FILE {{Name}}Targets.cmake
    NAMESPACE {{Name}}::
    DESTINATION "lib/cmake/{{Name}}")
install(FILES
    "${CMAKE_CURRENT_BINARY_DIR}/{{Name}}Config.cmake"
    "${CMAKE_CURRENT_BINARY_DIR}/{{Name}}ConfigVersion.cmake"
    DESTINATION "lib/cmake/{{Name}}")
