# SPDX-License-Identifier: Apache-2.0
cmake_minimum_required(VERSION 3.23)

project({{Name}}
    VERSION 0.1.0
    DESCRIPTION "{{name}} — a C++ library managed by OpenStrata"
    LANGUAGES CXX)

# The C++ standard is supplied by OpenStrata's toolchain; default if CMake is
# invoked directly (e.g. without `--preset`).
if(NOT CMAKE_CXX_STANDARD)
    set(CMAKE_CXX_STANDARD 17)
endif()
set(CMAKE_CXX_STANDARD_REQUIRED ON)
set(CMAKE_CXX_EXTENSIONS OFF)

add_library({{name}}
    src/{{name}}.cpp)
add_library({{Name}}::{{name}} ALIAS {{name}})

target_include_directories({{name}}
    PUBLIC
        "$<BUILD_INTERFACE:${CMAKE_CURRENT_SOURCE_DIR}/include>"
        "$<INSTALL_INTERFACE:include>")
target_compile_features({{name}} PUBLIC cxx_std_17)

# Install rules so `ost package` produces a non-empty, usable artifact.
include(GNUInstallDirs)
install(TARGETS {{name}}
    EXPORT {{Name}}Targets
    ARCHIVE DESTINATION "${CMAKE_INSTALL_LIBDIR}"
    LIBRARY DESTINATION "${CMAKE_INSTALL_LIBDIR}"
    RUNTIME DESTINATION "${CMAKE_INSTALL_BINDIR}")
install(DIRECTORY include/
    DESTINATION "${CMAKE_INSTALL_INCLUDEDIR}")
