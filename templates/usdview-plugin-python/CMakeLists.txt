cmake_minimum_required(VERSION 3.20)
project({{name}}_usdview_plugin NONE)

# The starter is Python-only. Projects may enable CXX and place a native module
# in python/{{identCamel}}; the same build/package provenance will capture it.
add_custom_target({{name}}_usdview_plugin ALL
  SOURCES
    plugin/plugInfo.json
    python/{{identCamel}}/__init__.py)

install(FILES plugin/plugInfo.json
  DESTINATION share/{{name}}/plugin)
install(DIRECTORY python/{{identCamel}}
  DESTINATION python)
