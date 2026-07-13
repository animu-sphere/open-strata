add_library({{name}}-render-vulkan STATIC vulkan_backend.cpp)
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
endif()
