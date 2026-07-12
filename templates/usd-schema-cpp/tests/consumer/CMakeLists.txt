cmake_minimum_required(VERSION 3.22)
project({{Name}}InstalledConsumer LANGUAGES CXX)

find_package({{Name}} REQUIRED CONFIG)

add_executable({{name}}-consumer main.cpp)
target_link_libraries({{name}}-consumer PRIVATE {{Name}}::Schema)
target_compile_features({{name}}-consumer PRIVATE cxx_std_17)
