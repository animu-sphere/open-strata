# The viewport is an optional native host, not part of the renderer's
# validation contract: composition evidence stays owned by the headless
# adapter. GLFW builds C sources when fetched.
enable_language(C)

if(NOT {{NAME}}_WITH_VULKAN)
  message(FATAL_ERROR
    "{{NAME}}_ENABLE_VIEWPORT requires a Vulkan 1.3 SDK and slangc")
endif()

# GLFW is the native window boundary. Prefer an installed package; otherwise
# fetch the pinned 3.4 release so every build compiles the same sources.
find_package(glfw3 3.4 CONFIG QUIET)
if(NOT TARGET glfw)
  include(FetchContent)
  set(GLFW_BUILD_DOCS OFF CACHE BOOL "" FORCE)
  set(GLFW_BUILD_TESTS OFF CACHE BOOL "" FORCE)
  set(GLFW_BUILD_EXAMPLES OFF CACHE BOOL "" FORCE)
  set(GLFW_INSTALL OFF CACHE BOOL "" FORCE)
  FetchContent_Declare(glfw
    GIT_REPOSITORY https://github.com/glfw/glfw.git
    GIT_TAG 7b6aead9fb88b3623e3b3725ebb42670cbe4c579 # 3.4
    GIT_SHALLOW FALSE)
  FetchContent_MakeAvailable(glfw)
endif()

add_executable({{name}}-viewport
  main.cpp
  window_glfw.cpp)
target_link_libraries({{name}}-viewport PRIVATE
  {{Name}}::RenderWorld
  {{Name}}::RenderExtraction
  {{Name}}::Vulkan
  Vulkan::Vulkan
  glfw)
target_compile_features({{name}}-viewport PRIVATE cxx_std_20)

add_dependencies({{name}}-viewport {{name}}-renderer-shaders)
add_custom_command(TARGET {{name}}-viewport POST_BUILD
  COMMAND "${CMAKE_COMMAND}" -E make_directory
    "$<TARGET_FILE_DIR:{{name}}-viewport>/shaders"
  COMMAND "${CMAKE_COMMAND}" -E copy_if_different
    "${{{NAME}}_VERTEX_SPV}"
    "$<TARGET_FILE_DIR:{{name}}-viewport>/shaders/triangle.vert.spv"
  COMMAND "${CMAKE_COMMAND}" -E copy_if_different
    "${{{NAME}}_FRAGMENT_SPV}"
    "$<TARGET_FILE_DIR:{{name}}-viewport>/shaders/triangle.frag.spv"
  VERBATIM)

# The installed tree reuses bin/shaders written by the project install rules.
install(TARGETS {{name}}-viewport
  RUNTIME DESTINATION "${CMAKE_INSTALL_BINDIR}")

if({{NAME}}_BUILD_TESTS)
  add_test(NAME {{name}}-viewport-present
    COMMAND {{name}}-viewport --hidden --frames 8 --vsync off)
  set_tests_properties({{name}}-viewport-present PROPERTIES
    LABELS "gpu;viewport;presentation"
    SKIP_RETURN_CODE 77
    TIMEOUT 60)
endif()
