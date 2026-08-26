cmake_minimum_required(VERSION 3.20)
project(TinyConsumer LANGUAGES CXX)
find_package(Tiny CONFIG REQUIRED)
add_executable(consumer ../main.cpp)
target_link_libraries(consumer PRIVATE Tiny::tiny)
# Force the consumer test to use the activated runtime's loader paths.
set_target_properties(consumer PROPERTIES SKIP_BUILD_RPATH TRUE)
