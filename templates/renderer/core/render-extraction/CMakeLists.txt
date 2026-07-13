add_library({{name}}-render-extraction STATIC extraction.cpp)
add_library({{Name}}::RenderExtraction ALIAS {{name}}-render-extraction)
set_target_properties({{name}}-render-extraction PROPERTIES EXPORT_NAME RenderExtraction)
target_include_directories({{name}}-render-extraction PUBLIC
  "$<BUILD_INTERFACE:${PROJECT_SOURCE_DIR}/include>"
  "$<INSTALL_INTERFACE:${CMAKE_INSTALL_INCLUDEDIR}>")
target_link_libraries({{name}}-render-extraction PUBLIC {{Name}}::RenderWorld)
target_compile_features({{name}}-render-extraction PUBLIC cxx_std_20)
