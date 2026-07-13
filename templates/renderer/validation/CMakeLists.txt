configure_file(
  run-install-smoke.cmake.in
  "${CMAKE_CURRENT_BINARY_DIR}/run-install-smoke.cmake"
  @ONLY)

add_test(NAME {{name}}-renderer-evidence
  COMMAND "$<TARGET_FILE:{{name}}-headless>"
    --report "${CMAKE_BINARY_DIR}/renderer-ctest-report.json")

add_test(NAME {{name}}-renderer-install-tree
  COMMAND "${CMAKE_COMMAND}"
    "-DBUILD_DIR=${CMAKE_BINARY_DIR}"
    "-DCONFIG=$<CONFIG>"
    "-DPREFIX=${CMAKE_BINARY_DIR}/renderer-install"
    -P "${CMAKE_CURRENT_BINARY_DIR}/run-install-smoke.cmake")
set_tests_properties({{name}}-renderer-install-tree PROPERTIES RUN_SERIAL TRUE)
