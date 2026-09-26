# SPDX-License-Identifier: Apache-2.0
cmake_minimum_required(VERSION 3.22)
project(OpenStrataImagingProbe LANGUAGES CXX)
find_package(pxr REQUIRED CONFIG)
include("${CMAKE_CURRENT_SOURCE_DIR}/OpenStrataPlugin.cmake")
add_executable(ost-usd-imaging-probe main.cpp)
target_include_directories(ost-usd-imaging-probe PRIVATE ${PXR_INCLUDE_DIRS})
openstrata_link_openusd(TARGET ost-usd-imaging-probe COMPONENTS usdImaging usd plug tf)
set_target_properties(ost-usd-imaging-probe PROPERTIES
    RUNTIME_OUTPUT_DIRECTORY "${CMAKE_BINARY_DIR}/bin")
foreach(config DEBUG RELEASE RELWITHDEBINFO MINSIZEREL)
    set_target_properties(ost-usd-imaging-probe PROPERTIES
        RUNTIME_OUTPUT_DIRECTORY_${config} "${CMAKE_BINARY_DIR}/bin")
endforeach()
