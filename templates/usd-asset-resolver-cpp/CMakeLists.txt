# SPDX-License-Identifier: Apache-2.0
cmake_minimum_required(VERSION 3.22)

project({{Name}}ResolverPlugin
    VERSION 0.1.0
    DESCRIPTION "OpenUSD {{scheme}} URI resolver skeleton"
    LANGUAGES CXX)

if(NOT CMAKE_CXX_STANDARD)
    set(CMAKE_CXX_STANDARD 17)
endif()
set(CMAKE_CXX_STANDARD_REQUIRED ON)
set(CMAKE_CXX_EXTENSIONS OFF)

get_property(_is_multi_config GLOBAL PROPERTY GENERATOR_IS_MULTI_CONFIG)
if(NOT _is_multi_config AND NOT CMAKE_BUILD_TYPE)
    set(CMAKE_BUILD_TYPE Release CACHE STRING "Build type" FORCE)
endif()

if(NOT pxr_FOUND)
    find_package(pxr REQUIRED CONFIG)
endif()

set(PLUGIN_NAME {{Name}}Resolver)

add_library(${PLUGIN_NAME} SHARED
    src/{{Name}}Resolver.cpp)

target_compile_features(${PLUGIN_NAME} PRIVATE cxx_std_17)
target_include_directories(${PLUGIN_NAME} PRIVATE
    "${CMAKE_CURRENT_SOURCE_DIR}/src"
    ${PXR_INCLUDE_DIRS})

if(TARGET usd_ms)
    target_link_libraries(${PLUGIN_NAME} PRIVATE usd_ms)
else()
    set(_pxr_targets)
    foreach(_t IN ITEMS arch tf js plug vt ar)
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

set(OPENSTRATA_PLUGIN_LIBRARY_PREFIX "lib")
set_target_properties(${PLUGIN_NAME} PROPERTIES
    PREFIX "${OPENSTRATA_PLUGIN_LIBRARY_PREFIX}"
    ARCHIVE_OUTPUT_DIRECTORY "${CMAKE_CURRENT_SOURCE_DIR}/lib"
    LIBRARY_OUTPUT_DIRECTORY "${CMAKE_CURRENT_SOURCE_DIR}/lib"
    RUNTIME_OUTPUT_DIRECTORY "${CMAKE_CURRENT_SOURCE_DIR}/lib")
foreach(_config DEBUG RELEASE RELWITHDEBINFO MINSIZEREL)
    set_target_properties(${PLUGIN_NAME} PROPERTIES
        ARCHIVE_OUTPUT_DIRECTORY_${_config} "${CMAKE_CURRENT_SOURCE_DIR}/lib"
        LIBRARY_OUTPUT_DIRECTORY_${_config} "${CMAKE_CURRENT_SOURCE_DIR}/lib"
        RUNTIME_OUTPUT_DIRECTORY_${_config} "${CMAKE_CURRENT_SOURCE_DIR}/lib")
endforeach()

if(MSVC)
    target_compile_definitions(${PLUGIN_NAME} PRIVATE NOMINMAX)
    target_compile_options(${PLUGIN_NAME} PRIVATE /utf-8)
endif()

configure_file(
    "${CMAKE_CURRENT_SOURCE_DIR}/plugin/resources/{{name}}/plugInfo.json.in"
    "${CMAKE_CURRENT_SOURCE_DIR}/plugin/resources/{{name}}/plugInfo.json"
    @ONLY)

install(TARGETS ${PLUGIN_NAME}
    RUNTIME DESTINATION "lib"
    LIBRARY DESTINATION "lib"
    ARCHIVE DESTINATION "lib")
install(FILES
    "${CMAKE_CURRENT_SOURCE_DIR}/plugin/resources/{{name}}/plugInfo.json"
    DESTINATION "plugin/resources/{{name}}")
install(FILES
    "${CMAKE_CURRENT_SOURCE_DIR}/openstrata.plugin.yaml"
    "${CMAKE_CURRENT_SOURCE_DIR}/openstrata.scaffold.yaml"
    DESTINATION ".")
install(DIRECTORY "${CMAKE_CURRENT_SOURCE_DIR}/tests/fixtures"
    DESTINATION "tests")
