add_executable({{name}}-headless main.cpp)
target_link_libraries({{name}}-headless PRIVATE
  {{Name}}::RenderWorld
  {{Name}}::RenderExtraction
  {{Name}}::Vulkan)
target_compile_features({{name}}-headless PRIVATE cxx_std_20)

if({{NAME}}_WITH_VULKAN)
  add_dependencies({{name}}-headless {{name}}-renderer-shaders)
  add_custom_command(TARGET {{name}}-headless POST_BUILD
    COMMAND "${CMAKE_COMMAND}" -E make_directory
      "$<TARGET_FILE_DIR:{{name}}-headless>/shaders"
    COMMAND "${CMAKE_COMMAND}" -E copy_if_different
      "${{{NAME}}_VERTEX_SPV}"
      "$<TARGET_FILE_DIR:{{name}}-headless>/shaders/triangle.vert.spv"
    COMMAND "${CMAKE_COMMAND}" -E copy_if_different
      "${{{NAME}}_FRAGMENT_SPV}"
      "$<TARGET_FILE_DIR:{{name}}-headless>/shaders/triangle.frag.spv"
    VERBATIM)
endif()

if(NOT CMAKE_CROSSCOMPILING)
  add_custom_command(TARGET {{name}}-headless POST_BUILD
    COMMAND "$<TARGET_FILE:{{name}}-headless>"
      --report "${CMAKE_BINARY_DIR}/renderer-report.json"
    COMMENT "Writing renderer-report.json"
    VERBATIM)
endif()
