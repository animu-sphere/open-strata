# SPDX-License-Identifier: Apache-2.0
# OpenStrata plugin scaffold helper, copied into generated bundles.
#
# Keep this file self-contained: generated projects must build with plain CMake
# and an OpenUSD SDK, without an OpenStrata source checkout or an `ost` binary.
include_guard(DIRECTORY)

set(OPENSTRATA_PLUGIN_CMAKE_HELPER_VERSION "1.2.0")

function(openstrata_default_build_type)
    get_property(_is_multi_config GLOBAL PROPERTY GENERATOR_IS_MULTI_CONFIG)
    if(NOT _is_multi_config AND NOT CMAKE_BUILD_TYPE)
        set(CMAKE_BUILD_TYPE Release CACHE STRING "Build type" FORCE)
    endif()
endfunction()

function(openstrata_link_openusd)
    cmake_parse_arguments(ARG "" "TARGET;VISIBILITY" "COMPONENTS" ${ARGN})
    if(NOT ARG_TARGET)
        message(FATAL_ERROR "openstrata_link_openusd requires TARGET")
    elseif(NOT TARGET ${ARG_TARGET})
        message(FATAL_ERROR
            "openstrata_link_openusd TARGET '${ARG_TARGET}' does not exist")
    endif()
    if(NOT ARG_COMPONENTS)
        message(FATAL_ERROR "openstrata_link_openusd requires COMPONENTS")
    endif()
    if(NOT ARG_VISIBILITY)
        set(ARG_VISIBILITY PRIVATE)
    elseif(NOT ARG_VISIBILITY MATCHES "^(PRIVATE|PUBLIC|INTERFACE)$")
        message(FATAL_ERROR
            "openstrata_link_openusd VISIBILITY must be PRIVATE, PUBLIC, or INTERFACE")
    endif()

    if(TARGET usd_ms)
        target_link_libraries(${ARG_TARGET} ${ARG_VISIBILITY} usd_ms)
        return()
    endif()
    if(TARGET pxr::usd_ms)
        target_link_libraries(${ARG_TARGET} ${ARG_VISIBILITY} pxr::usd_ms)
        return()
    endif()

    set(_pxr_targets)
    foreach(_component IN LISTS ARG_COMPONENTS)
        if(TARGET ${_component})
            list(APPEND _pxr_targets ${_component})
        elseif(TARGET pxr::${_component})
            list(APPEND _pxr_targets pxr::${_component})
        endif()
    endforeach()
    if(_pxr_targets)
        target_link_libraries(${ARG_TARGET} ${ARG_VISIBILITY} ${_pxr_targets})
    elseif(PXR_LIBRARIES)
        target_link_libraries(${ARG_TARGET} ${ARG_VISIBILITY} ${PXR_LIBRARIES})
    else()
        message(FATAL_ERROR
            "Could not locate OpenUSD CMake targets or PXR_LIBRARIES for ${ARG_TARGET}.")
    endif()
endfunction()

function(openstrata_configure_plugin)
    cmake_parse_arguments(ARG "" "TARGET;PLUG_INFO_INPUT;PLUG_INFO_OUTPUT" "" ${ARGN})
    if(NOT ARG_TARGET)
        message(FATAL_ERROR "openstrata_configure_plugin requires TARGET")
    elseif(NOT TARGET ${ARG_TARGET})
        message(FATAL_ERROR
            "openstrata_configure_plugin TARGET '${ARG_TARGET}' does not exist")
    endif()
    if(NOT ARG_PLUG_INFO_INPUT OR NOT ARG_PLUG_INFO_OUTPUT)
        message(FATAL_ERROR
            "openstrata_configure_plugin requires PLUG_INFO_INPUT and PLUG_INFO_OUTPUT")
    endif()

    # USD resolves LibraryPath relative to plugInfo.json, so stage the shared
    # library into this bundle's lib/ for every generator configuration. Use
    # CMAKE_CURRENT_SOURCE_DIR (this bundle), not CMAKE_SOURCE_DIR — when the
    # bundle is consumed via add_subdirectory() the latter points at the parent
    # workspace root and would stage the lib away from the committed
    # plugInfo.json LibraryPath.
    set(OPENSTRATA_PLUGIN_LIBRARY_PREFIX "lib")
    set_target_properties(${ARG_TARGET} PROPERTIES
        PREFIX "${OPENSTRATA_PLUGIN_LIBRARY_PREFIX}"
        ARCHIVE_OUTPUT_DIRECTORY "${CMAKE_CURRENT_SOURCE_DIR}/lib"
        LIBRARY_OUTPUT_DIRECTORY "${CMAKE_CURRENT_SOURCE_DIR}/lib"
        RUNTIME_OUTPUT_DIRECTORY "${CMAKE_CURRENT_SOURCE_DIR}/lib")
    foreach(_config DEBUG RELEASE RELWITHDEBINFO MINSIZEREL)
        set_target_properties(${ARG_TARGET} PROPERTIES
            ARCHIVE_OUTPUT_DIRECTORY_${_config} "${CMAKE_CURRENT_SOURCE_DIR}/lib"
            LIBRARY_OUTPUT_DIRECTORY_${_config} "${CMAKE_CURRENT_SOURCE_DIR}/lib"
            RUNTIME_OUTPUT_DIRECTORY_${_config} "${CMAKE_CURRENT_SOURCE_DIR}/lib")
    endforeach()

    if(MSVC)
        target_compile_definitions(${ARG_TARGET} PRIVATE NOMINMAX)
        target_compile_options(${ARG_TARGET} PRIVATE /utf-8)
    endif()

    # plugInfo.json is produced in two evaluation stages:
    #
    #   1. configure: `configure_file(@ONLY)` resolves OpenStrata/CMake
    #      variables. `@OPENSTRATA_PLUGIN_LIBRARY_PATH@` expands to the
    #      bundle-relative lib/ directory plus `$<TARGET_FILE_NAME:target>`, so
    #      the library filename comes from the target itself (OUTPUT_NAME,
    #      PREFIX, SUFFIX and per-config postfixes included) instead of being
    #      re-assembled from a guessed prefix and suffix.
    #   2. generate: `file(GENERATE)` evaluates that generator expression.
    #
    # Every configuration gets its own build-tree copy under
    # openstrata/<target>/<config>/, so multi-config generators never write different
    # content to one path. PLUG_INFO_OUTPUT (the source-tree file the manifest's
    # `usd.plug_info` names) holds the primary configuration only. It stays the
    # file `ost plugin build` merges co-hosted schema Types into, and the one
    # installed for the primary configuration. Note that file(GENERATE) output
    # does not exist until CMake's generate step has run.
    #
    # Legacy inputs that spell `@OPENSTRATA_PLUGIN_LIBRARY_PREFIX@` and
    # `@CMAKE_SHARED_LIBRARY_SUFFIX@` keep resolving exactly as before.
    get_filename_component(_plug_info_output "${ARG_PLUG_INFO_OUTPUT}" ABSOLUTE)
    get_filename_component(_plug_info_dir "${_plug_info_output}" DIRECTORY)
    set(_library_dir "${CMAKE_CURRENT_SOURCE_DIR}/lib")
    file(RELATIVE_PATH _library_relative "${_plug_info_dir}" "${_library_dir}")
    if(IS_ABSOLUTE "${_library_relative}")
        message(FATAL_ERROR
            "openstrata_configure_plugin: PLUG_INFO_OUTPUT '${_plug_info_output}' "
            "has no relative path to '${_library_dir}'")
    endif()
    set(OPENSTRATA_PLUGIN_LIBRARY_PATH
        "${_library_relative}/$<TARGET_FILE_NAME:${ARG_TARGET}>")

    get_property(_is_multi_config GLOBAL PROPERTY GENERATOR_IS_MULTI_CONFIG)
    if(_is_multi_config)
        set(_primary_config "${OPENSTRATA_PLUGIN_PRIMARY_CONFIG}")
        if(NOT _primary_config)
            if("Release" IN_LIST CMAKE_CONFIGURATION_TYPES)
                set(_primary_config Release)
            else()
                list(GET CMAKE_CONFIGURATION_TYPES 0 _primary_config)
            endif()
        elseif(NOT _primary_config IN_LIST CMAKE_CONFIGURATION_TYPES)
            message(FATAL_ERROR
                "OPENSTRATA_PLUGIN_PRIMARY_CONFIG='${_primary_config}' is not one "
                "of CMAKE_CONFIGURATION_TYPES (${CMAKE_CONFIGURATION_TYPES})")
        endif()
        set(_primary_condition "$<CONFIG:${_primary_config}>")
    else()
        set(_primary_config "${CMAKE_BUILD_TYPE}")
        set(_primary_condition 1)
    endif()

    get_filename_component(_input_name "${ARG_PLUG_INFO_INPUT}" NAME)
    set(_metadata_dir "${CMAKE_CURRENT_BINARY_DIR}/openstrata/${ARG_TARGET}")
    set(_staged_input "${_metadata_dir}/${_input_name}.genex")
    configure_file("${ARG_PLUG_INFO_INPUT}" "${_staged_input}" @ONLY)
    set(_config_dir "$<IF:$<BOOL:$<CONFIG>>,$<CONFIG>,NoConfig>")
    file(GENERATE
        OUTPUT "${_metadata_dir}/${_config_dir}/plugInfo.json"
        INPUT "${_staged_input}"
        TARGET ${ARG_TARGET})
    file(GENERATE
        OUTPUT "${_plug_info_output}"
        INPUT "${_staged_input}"
        CONDITION "${_primary_condition}"
        TARGET ${ARG_TARGET})

    set_target_properties(${ARG_TARGET} PROPERTIES
        OPENSTRATA_PLUG_INFO_OUTPUT "${_plug_info_output}"
        OPENSTRATA_PLUG_INFO_CONFIG_DIR "${_metadata_dir}"
        OPENSTRATA_PLUG_INFO_PRIMARY_CONFIG "${_primary_config}"
        OPENSTRATA_PLUG_INFO_LIBRARY_RELATIVE "${_library_relative}")
endfunction()

function(openstrata_install_plugin_bundle)
    cmake_parse_arguments(
        ARG
        ""
        "TARGET;RESOURCE_DESTINATION;FIXTURE_SOURCE;EXPORT_SET"
        "MANIFESTS;RESOURCES"
        ${ARGN})
    if(NOT ARG_TARGET)
        message(FATAL_ERROR "openstrata_install_plugin_bundle requires TARGET")
    elseif(NOT TARGET ${ARG_TARGET})
        message(FATAL_ERROR
            "openstrata_install_plugin_bundle TARGET '${ARG_TARGET}' does not exist")
    endif()
    if(NOT ARG_RESOURCES OR NOT ARG_RESOURCE_DESTINATION)
        message(FATAL_ERROR
            "openstrata_install_plugin_bundle requires RESOURCES and RESOURCE_DESTINATION")
    endif()

    if(ARG_EXPORT_SET)
        install(TARGETS ${ARG_TARGET}
            EXPORT ${ARG_EXPORT_SET}
            RUNTIME DESTINATION "lib"
            LIBRARY DESTINATION "lib"
            ARCHIVE DESTINATION "lib")
    else()
        install(TARGETS ${ARG_TARGET}
            RUNTIME DESTINATION "lib"
            LIBRARY DESTINATION "lib"
            ARCHIVE DESTINATION "lib")
    endif()
    # The target's own plugInfo.json is configuration-specific (see
    # openstrata_configure_plugin). Install the primary configuration's
    # source-tree file (which may carry `ost`-merged schema Types), and the
    # matching build-tree copy for any other configuration, so the installed
    # LibraryPath always names the library installed from that configuration.
    get_target_property(_plug_info_output ${ARG_TARGET} OPENSTRATA_PLUG_INFO_OUTPUT)
    set(_resources)
    foreach(_resource IN LISTS ARG_RESOURCES)
        get_filename_component(_resource_abs "${_resource}" ABSOLUTE)
        if(_plug_info_output AND _resource_abs STREQUAL _plug_info_output)
            _openstrata_install_plug_info(${ARG_TARGET} "${ARG_RESOURCE_DESTINATION}")
        else()
            list(APPEND _resources "${_resource}")
        endif()
    endforeach()
    if(_resources)
        install(FILES ${_resources} DESTINATION "${ARG_RESOURCE_DESTINATION}")
    endif()
    if(ARG_MANIFESTS)
        install(FILES ${ARG_MANIFESTS} DESTINATION ".")
    endif()
    if(ARG_FIXTURE_SOURCE)
        install(DIRECTORY "${ARG_FIXTURE_SOURCE}/" DESTINATION "tests/fixtures")
    endif()
endfunction()

function(_openstrata_install_plug_info target destination)
    get_target_property(_plug_info_output ${target} OPENSTRATA_PLUG_INFO_OUTPUT)
    get_target_property(_metadata_dir ${target} OPENSTRATA_PLUG_INFO_CONFIG_DIR)
    get_target_property(_primary_config ${target} OPENSTRATA_PLUG_INFO_PRIMARY_CONFIG)
    get_target_property(_library_relative ${target} OPENSTRATA_PLUG_INFO_LIBRARY_RELATIVE)

    # The library installs to <prefix>/lib. A LibraryPath that is correct
    # beside the source-tree plugInfo.json must also be correct beside the
    # installed one, or the package cannot load without the source tree.
    set(_install_root "${CMAKE_CURRENT_BINARY_DIR}/openstrata/install-layout")
    file(RELATIVE_PATH _installed_relative
        "${_install_root}/${destination}" "${_install_root}/lib")
    if(NOT _installed_relative STREQUAL _library_relative)
        message(FATAL_ERROR
            "plugInfo.json for '${target}' reaches the library through "
            "'${_library_relative}' in the source tree but would need "
            "'${_installed_relative}' when installed to '${destination}'. "
            "Keep the RESOURCE_DESTINATION at the same depth as the source "
            "plugInfo.json directory.")
    endif()

    get_property(_is_multi_config GLOBAL PROPERTY GENERATOR_IS_MULTI_CONFIG)
    if(NOT _is_multi_config)
        install(FILES "${_plug_info_output}" DESTINATION "${destination}")
        return()
    endif()
    install(FILES "${_plug_info_output}"
        DESTINATION "${destination}"
        CONFIGURATIONS ${_primary_config})
    set(_other_configs ${CMAKE_CONFIGURATION_TYPES})
    list(REMOVE_ITEM _other_configs ${_primary_config})
    if(_other_configs)
        install(FILES "${_metadata_dir}/$<CONFIG>/plugInfo.json"
            DESTINATION "${destination}"
            CONFIGURATIONS ${_other_configs})
    endif()
endfunction()
