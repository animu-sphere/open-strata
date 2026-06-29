# Dual-mode root for a repository of OpenStrata plugin bundles.
#
# `ost` builds each bundle individually (`ost plugin build <bundle>`), composing
# the resolved runtime's environment and toolchain. This root exists so a plain
# CMake user can build the whole repo *without* `ost`:
#
#   cmake -S . -B build -DCMAKE_PREFIX_PATH=<your-openusd-install>
#   cmake --build build
#
# It resolves OpenUSD once here, then add_subdirectory()s every bundle — an
# immediate subdirectory containing both an openstrata.plugin.yaml and a
# CMakeLists.txt. Each bundle guards its own `find_package(pxr)` with
# `if(NOT pxr_FOUND)`, so it builds either standalone (via `ost`) or as a
# subdirectory of this root. New bundles created with `ost plugin new` are picked
# up automatically — no edit here required.
cmake_minimum_required(VERSION 3.23)
project({{name}} LANGUAGES CXX)

if(NOT pxr_FOUND)
    find_package(pxr CONFIG REQUIRED)
endif()

file(GLOB _entries LIST_DIRECTORIES true RELATIVE
     "${CMAKE_CURRENT_SOURCE_DIR}" "${CMAKE_CURRENT_SOURCE_DIR}/*")
foreach(_entry ${_entries})
    set(_bundle "${CMAKE_CURRENT_SOURCE_DIR}/${_entry}")
    if(IS_DIRECTORY "${_bundle}"
       AND EXISTS "${_bundle}/openstrata.plugin.yaml"
       AND EXISTS "${_bundle}/CMakeLists.txt")
        message(STATUS "OpenStrata: adding plugin bundle '${_entry}'")
        add_subdirectory("${_entry}")
    endif()
endforeach()
