find_package(pxr CONFIG REQUIRED)

if(NOT {{NAME}}_WITH_VULKAN)
  message(FATAL_ERROR
    "{{NAME}}_ENABLE_HYDRA2 requires a Vulkan 1.3 SDK and glslc")
endif()
if(NOT TARGET hd OR NOT TARGET plug)
  message(FATAL_ERROR
    "The OpenUSD package must provide the hd and plug CMake targets")
endif()

# The co-built runtime is implementation detail of the module. It is not an
# independently packaged OpenStrata plugin bundle or renderer SDK component.
add_library({{name}}-hydra2-runtime STATIC src/adapter.cpp)
target_include_directories({{name}}-hydra2-runtime PUBLIC
  "${CMAKE_CURRENT_SOURCE_DIR}/src")
target_link_libraries({{name}}-hydra2-runtime
  PUBLIC hd
  PRIVATE
    {{Name}}::RenderWorld
    {{Name}}::RenderExtraction
    {{Name}}::Vulkan
    ${CMAKE_DL_LIBS})
target_compile_features({{name}}-hydra2-runtime PUBLIC cxx_std_20)
target_compile_definitions({{name}}-hydra2-runtime PRIVATE
  NOMINMAX
  _CRT_SECURE_NO_WARNINGS
  _SILENCE_CXX20_OLD_SHARED_PTR_ATOMIC_SUPPORT_DEPRECATION_WARNING)

add_library(hd{{Name}} MODULE
  src/renderer_plugin.cpp
  resources/plugInfo.json.in)
target_link_libraries(hd{{Name}} PRIVATE {{name}}-hydra2-runtime hd)
target_compile_features(hd{{Name}} PRIVATE cxx_std_20)
target_compile_definitions(hd{{Name}} PRIVATE
  NOMINMAX
  _SILENCE_CXX20_OLD_SHARED_PTR_ATOMIC_SUPPORT_DEPRECATION_WARNING)
set_target_properties(hd{{Name}} PROPERTIES PREFIX "")

# Static renderer packs become part of the Hydra module on ELF/Mach-O hosts.
set_target_properties(
  {{name}}-render-world
  {{name}}-render-extraction
  {{name}}-render-vulkan
  {{name}}-hydra2-runtime
  PROPERTIES POSITION_INDEPENDENT_CODE ON)

set({{NAME}}_HYDRA_LIBRARY_NAME
    "hd{{Name}}${CMAKE_SHARED_MODULE_SUFFIX}")
configure_file(resources/plugInfo.json.in
  "${CMAKE_CURRENT_BINARY_DIR}/resources/plugInfo.json" @ONLY)

add_dependencies(hd{{Name}} {{name}}-renderer-shaders)
add_custom_command(TARGET hd{{Name}} POST_BUILD
  COMMAND "${CMAKE_COMMAND}" -E make_directory
    "$<TARGET_FILE_DIR:hd{{Name}}>/resources"
  COMMAND "${CMAKE_COMMAND}" -E copy_if_different
    "${CMAKE_CURRENT_BINARY_DIR}/resources/plugInfo.json"
    "$<TARGET_FILE_DIR:hd{{Name}}>/resources/plugInfo.json"
  COMMAND "${CMAKE_COMMAND}" -E make_directory
    "$<TARGET_FILE_DIR:hd{{Name}}>/shaders"
  COMMAND "${CMAKE_COMMAND}" -E copy_if_different
    "${{{NAME}}_VERTEX_SPV}"
    "$<TARGET_FILE_DIR:hd{{Name}}>/shaders/triangle.vert.spv"
  COMMAND "${CMAKE_COMMAND}" -E copy_if_different
    "${{{NAME}}_FRAGMENT_SPV}"
    "$<TARGET_FILE_DIR:hd{{Name}}>/shaders/triangle.frag.spv"
  VERBATIM)

set({{NAME}}_HYDRA_INSTALL_DIR
    "${CMAKE_INSTALL_LIBDIR}/usd/hd{{Name}}")
install(TARGETS hd{{Name}}
  RUNTIME DESTINATION "${{{NAME}}_HYDRA_INSTALL_DIR}"
  LIBRARY DESTINATION "${{{NAME}}_HYDRA_INSTALL_DIR}")
install(FILES "${CMAKE_CURRENT_BINARY_DIR}/resources/plugInfo.json"
  DESTINATION "${{{NAME}}_HYDRA_INSTALL_DIR}/resources")
install(FILES "${{{NAME}}_VERTEX_SPV}" "${{{NAME}}_FRAGMENT_SPV}"
  DESTINATION "${{{NAME}}_HYDRA_INSTALL_DIR}/shaders")
install(FILES tests/usdview-smoke.usda
  DESTINATION "${CMAKE_INSTALL_DATADIR}/{{name}}/tests")

if({{NAME}}_BUILD_TESTS)
  add_executable({{name}}-hydra2-probe tests/plugin_probe.cpp)
  target_link_libraries({{name}}-hydra2-probe PRIVATE hd plug)
  target_compile_features({{name}}-hydra2-probe PRIVATE cxx_std_20)
  target_compile_definitions({{name}}-hydra2-probe PRIVATE
    NOMINMAX
    _SILENCE_CXX20_OLD_SHARED_PTR_ATOMIC_SUPPORT_DEPRECATION_WARNING)
  add_dependencies({{name}}-hydra2-probe hd{{Name}})

  add_executable({{name}}-hydra2-render-buffer-test
    tests/render_buffer_test.cpp)
  target_link_libraries({{name}}-hydra2-render-buffer-test
    PRIVATE {{name}}-hydra2-runtime)
  target_compile_features({{name}}-hydra2-render-buffer-test PRIVATE cxx_std_20)
  target_compile_definitions({{name}}-hydra2-render-buffer-test PRIVATE
    NOMINMAX
    _SILENCE_CXX20_OLD_SHARED_PTR_ATOMIC_SUPPORT_DEPRECATION_WARNING)

  if(PXR_INCLUDE_DIRS)
    list(GET PXR_INCLUDE_DIRS 0 _{{name}}_pxr_include)
    get_filename_component({{NAME}}_PXR_ROOT
      "${_{{name}}_pxr_include}/.." ABSOLUTE)
  elseif(DEFINED pxr_DIR)
    set({{NAME}}_PXR_ROOT "${pxr_DIR}")
  else()
    message(FATAL_ERROR "Could not derive the OpenUSD runtime root")
  endif()

  set(_{{name}}_pxr_runtime_modifications
    "PATH=path_list_prepend:${{{NAME}}_PXR_ROOT}/bin"
    "PATH=path_list_prepend:${{{NAME}}_PXR_ROOT}/lib")

  add_test(NAME {{name}}-renderer-plugin-discovery
    COMMAND {{name}}-hydra2-probe --discovery
      "$<TARGET_FILE_DIR:hd{{Name}}>/resources")
  add_test(NAME {{name}}-renderer-delegate-creation
    COMMAND {{name}}-hydra2-probe --delegate
      "$<TARGET_FILE_DIR:hd{{Name}}>/resources")
  add_test(NAME {{name}}-renderer-hydra-render-buffer
    COMMAND {{name}}-hydra2-render-buffer-test)
  set_tests_properties(
    {{name}}-renderer-plugin-discovery
    {{name}}-renderer-delegate-creation
    {{name}}-renderer-hydra-render-buffer
    PROPERTIES ENVIRONMENT_MODIFICATION
      "${_{{name}}_pxr_runtime_modifications}")

  find_package(Python3 COMPONENTS Interpreter QUIET)
  find_program({{NAME}}_TESTUSDVIEW testusdview
    PATHS "${{{NAME}}_PXR_ROOT}/bin" NO_DEFAULT_PATH)
  set(_{{name}}_has_display TRUE)
  if(UNIX AND NOT APPLE AND NOT DEFINED ENV{DISPLAY} AND
     NOT DEFINED ENV{WAYLAND_DISPLAY})
    set(_{{name}}_has_display FALSE)
  endif()
  if(Python3_Interpreter_FOUND AND {{NAME}}_TESTUSDVIEW AND
     _{{name}}_has_display)
    add_test(NAME {{name}}-renderer-usdview-host
      COMMAND "${CMAKE_COMMAND}"
        "-DRENDERER_BUILD_DIR=${CMAKE_BINARY_DIR}"
        "-DRENDERER_CONFIG=$<CONFIG>"
        "-DRENDERER_STAGE_DIR=${CMAKE_CURRENT_BINARY_DIR}/usdview-install"
        "-DRENDERER_INSTALL_LIBDIR=${CMAKE_INSTALL_LIBDIR}"
        "-DRENDERER_INSTALL_DATADIR=${CMAKE_INSTALL_DATADIR}"
        "-DRENDERER_PXR_ROOT=${{{NAME}}_PXR_ROOT}"
        "-DRENDERER_PYTHON=${Python3_EXECUTABLE}"
        "-DRENDERER_TESTUSDVIEW=${{{NAME}}_TESTUSDVIEW}"
        "-DRENDERER_TEST_SCRIPT=${CMAKE_CURRENT_SOURCE_DIR}/tests/usdview_smoke_test.py"
        -P "${CMAKE_CURRENT_SOURCE_DIR}/tests/run_usdview_smoke.cmake")
    set_tests_properties({{name}}-renderer-usdview-host PROPERTIES
      LABELS "host-smoke;gpu"
      TIMEOUT 60
      RUN_SERIAL TRUE)
  else()
    message(STATUS
      "testusdview, matching Python, or a display is unavailable; usdview host evidence will be SKIP")
  endif()
endif()
