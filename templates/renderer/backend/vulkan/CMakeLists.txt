add_library({{name}}-render-vulkan STATIC
  vulkan_backend.cpp
  vulkan_present.cpp)
add_library({{Name}}::Vulkan ALIAS {{name}}-render-vulkan)
set_target_properties({{name}}-render-vulkan PROPERTIES EXPORT_NAME Vulkan)
target_include_directories({{name}}-render-vulkan PUBLIC
  "$<BUILD_INTERFACE:${PROJECT_SOURCE_DIR}/include>"
  "$<INSTALL_INTERFACE:${CMAKE_INSTALL_INCLUDEDIR}>")
target_link_libraries({{name}}-render-vulkan PUBLIC {{Name}}::RenderExtraction)
target_compile_features({{name}}-render-vulkan PUBLIC cxx_std_20)
if({{NAME}}_WITH_VULKAN)
  target_link_libraries({{name}}-render-vulkan PRIVATE Vulkan::Vulkan)
  target_compile_definitions({{name}}-render-vulkan PRIVATE {{NAME}}_HAS_VULKAN=1)

  # Slang is the single shader source. Each entry point compiles to its own
  # SPIR-V artifact; slangc names the SPIR-V entry point `main`, which is what
  # the pipeline stage creation expects. The explicit matrix layout keeps
  # host-visible data layouts deterministic once shaders grow parameters.
  set(_shader_source "${CMAKE_CURRENT_SOURCE_DIR}/shaders/triangle.slang")
  set(_shader_dir "${CMAKE_CURRENT_BINARY_DIR}/shaders")
  set({{NAME}}_VERTEX_SPV "${_shader_dir}/triangle.vert.spv")
  set({{NAME}}_FRAGMENT_SPV "${_shader_dir}/triangle.frag.spv")
  add_custom_command(
    OUTPUT "${{{NAME}}_VERTEX_SPV}"
    COMMAND "${CMAKE_COMMAND}" -E make_directory "${_shader_dir}"
    COMMAND "${{{NAME}}_SLANGC_EXECUTABLE}" "${_shader_source}"
      -entry vertex_main -stage vertex -target spirv
      -matrix-layout-column-major -warnings-as-errors all
      -o "${{{NAME}}_VERTEX_SPV}"
    DEPENDS shaders/triangle.slang
    VERBATIM)
  add_custom_command(
    OUTPUT "${{{NAME}}_FRAGMENT_SPV}"
    COMMAND "${CMAKE_COMMAND}" -E make_directory "${_shader_dir}"
    COMMAND "${{{NAME}}_SLANGC_EXECUTABLE}" "${_shader_source}"
      -entry fragment_main -stage fragment -target spirv
      -matrix-layout-column-major -warnings-as-errors all
      -o "${{{NAME}}_FRAGMENT_SPV}"
    DEPENDS shaders/triangle.slang
    VERBATIM)
  add_custom_target({{name}}-renderer-shaders
    DEPENDS "${{{NAME}}_VERTEX_SPV}" "${{{NAME}}_FRAGMENT_SPV}")
  set({{NAME}}_VERTEX_SPV "${{{NAME}}_VERTEX_SPV}" PARENT_SCOPE)
  set({{NAME}}_FRAGMENT_SPV "${{{NAME}}_FRAGMENT_SPV}" PARENT_SCOPE)
endif()
