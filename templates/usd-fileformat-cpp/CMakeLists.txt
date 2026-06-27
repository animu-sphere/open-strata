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

# Link the USD monolith if present, else the discrete libraries this plugin uses.
if(TARGET usd_ms)
    target_link_libraries(${PLUGIN_NAME} PRIVATE usd_ms)
else()
    set(_pxr_targets)
    foreach(_t IN ITEMS arch tf gf vt sdf usd)
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

# Stage the shared library into the bundle's lib/ so `ost plugin doctor` and the
# session env find it (lib/ is placed on the dynamic-library path). Use
# CMAKE_CURRENT_SOURCE_DIR (this bundle), not CMAKE_SOURCE_DIR — the latter points
# at the repo root when this bundle is consumed via add_subdirectory(), which
# would stage the lib away from the committed plugInfo.json LibraryPath.
set(OPENSTRATA_PLUGIN_LIBRARY_PREFIX "lib")
set_target_properties(${PLUGIN_NAME} PROPERTIES
    PREFIX "${OPENSTRATA_PLUGIN_LIBRARY_PREFIX}"
    LIBRARY_OUTPUT_DIRECTORY "${CMAKE_CURRENT_SOURCE_DIR}/lib"
    RUNTIME_OUTPUT_DIRECTORY "${CMAKE_CURRENT_SOURCE_DIR}/lib")

# Generate plugInfo.json from the .in so its LibraryPath carries this target's
# shared-library prefix/suffix. `ost plugin new` writes a host-correct
# plugInfo.json up front (so `doctor`/`test` can inspect it before the first
# build); this regenerates it for the target actually being built. USD resolves
# LibraryPath relative to plugInfo's Root and dlopens the absolutized path (no
# PATH fallback), so the name and the ../../../lib/ location must both be exact.
configure_file(
    "${CMAKE_CURRENT_SOURCE_DIR}/plugin/resources/{{name}}/plugInfo.json.in"
    "${CMAKE_CURRENT_SOURCE_DIR}/plugin/resources/{{name}}/plugInfo.json"
    @ONLY)
