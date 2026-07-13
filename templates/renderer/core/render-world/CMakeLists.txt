add_library({{name}}-render-world STATIC render_world.cpp)
add_library({{Name}}::RenderWorld ALIAS {{name}}-render-world)
set_target_properties({{name}}-render-world PROPERTIES EXPORT_NAME RenderWorld)
target_include_directories({{name}}-render-world PUBLIC
  "$<BUILD_INTERFACE:${PROJECT_SOURCE_DIR}/include>"
  "$<INSTALL_INTERFACE:${CMAKE_INSTALL_INCLUDEDIR}>")
target_compile_features({{name}}-render-world PUBLIC cxx_std_20)
