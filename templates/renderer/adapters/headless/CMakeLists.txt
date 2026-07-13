add_executable({{name}}-headless main.cpp)
target_link_libraries({{name}}-headless PRIVATE
  {{Name}}::RenderWorld
  {{Name}}::RenderExtraction
  {{Name}}::Vulkan)
target_compile_features({{name}}-headless PRIVATE cxx_std_20)

if(NOT CMAKE_CROSSCOMPILING)
  add_custom_command(TARGET {{name}}-headless POST_BUILD
    COMMAND "$<TARGET_FILE:{{name}}-headless>"
      --report "${CMAKE_BINARY_DIR}/renderer-report.json"
    COMMENT "Writing renderer-report.json"
    VERBATIM)
endif()
