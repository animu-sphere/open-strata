foreach(required IN ITEMS
    RENDERER_BUILD_DIR RENDERER_CONFIG RENDERER_STAGE_DIR
    RENDERER_INSTALL_LIBDIR RENDERER_INSTALL_DATADIR RENDERER_PXR_ROOT
    RENDERER_PYTHON RENDERER_TESTUSDVIEW RENDERER_TEST_SCRIPT)
  if(NOT DEFINED ${required})
    message(FATAL_ERROR "${required} is required")
  endif()
endforeach()

file(REMOVE_RECURSE "${RENDERER_STAGE_DIR}")
execute_process(
  COMMAND "${CMAKE_COMMAND}" --install "${RENDERER_BUILD_DIR}"
          --config "${RENDERER_CONFIG}" --prefix "${RENDERER_STAGE_DIR}"
  RESULT_VARIABLE install_result
  OUTPUT_VARIABLE install_output
  ERROR_VARIABLE install_error)
if(NOT install_result EQUAL 0)
  message(FATAL_ERROR
    "renderer install staging failed:\n${install_output}\n${install_error}")
endif()

set(plugin_path
  "${RENDERER_STAGE_DIR}/${RENDERER_INSTALL_LIBDIR}/usd/hd{{Name}}/resources")
set(scene
  "${RENDERER_STAGE_DIR}/${RENDERER_INSTALL_DATADIR}/{{name}}/tests/usdview-smoke.usda")
set(image_root "${RENDERER_STAGE_DIR}/usdview")
set(evidence "${RENDERER_STAGE_DIR}/hydra-host-evidence.log")
cmake_path(CONVERT
  "${RENDERER_PXR_ROOT}/bin;${RENDERER_PXR_ROOT}/lib;$ENV{PATH}"
  TO_NATIVE_PATH_LIST runtime_path NORMALIZE)

execute_process(
  COMMAND "${CMAKE_COMMAND}" -E env
    "PXR_PLUGINPATH_NAME=${plugin_path}"
    "PYTHONPATH=${RENDERER_PXR_ROOT}/lib/python"
    "PATH=${runtime_path}"
    "{{NAME}}_HYDRA_EVIDENCE=${evidence}"
    "{{NAME}}_HYDRA_IMAGE=${image_root}"
    "${RENDERER_PYTHON}" "${RENDERER_TESTUSDVIEW}" "${scene}"
    --renderer {{Name}} --camera /Camera --testScript "${RENDERER_TEST_SCRIPT}"
  RESULT_VARIABLE usdview_result
  OUTPUT_VARIABLE usdview_output
  ERROR_VARIABLE usdview_error
  TIMEOUT 50)
if(NOT usdview_result EQUAL 0)
  message(FATAL_ERROR
    "usdview smoke failed (${usdview_result}):\n${usdview_output}\n${usdview_error}")
endif()

if(NOT EXISTS "${evidence}")
  message(FATAL_ERROR "usdview did not produce Hydra frame evidence")
endif()
file(STRINGS "${evidence}" evidence_lines)
list(LENGTH evidence_lines evidence_count)
if(evidence_count LESS 2)
  message(FATAL_ERROR
    "usdview did not complete first-frame and stable-update rendering")
endif()
foreach(phase IN ITEMS first-frame stable-update)
  set(image "${image_root}-${phase}.png")
  if(NOT EXISTS "${image}")
    message(FATAL_ERROR "usdview did not produce ${phase} image")
  endif()
  file(SIZE "${image}" image_size)
  if(image_size LESS 100)
    message(FATAL_ERROR "usdview ${phase} image is unexpectedly small")
  endif()
endforeach()

# Merge the independently exercised host checks into the normal renderer
# report. Before this install-tree test runs they remain honest SKIPs, so a
# core-only or display-less build never inherits host evidence it did not earn.
set(renderer_report "${RENDERER_BUILD_DIR}/renderer-report.json")
if(NOT EXISTS "${renderer_report}")
  message(FATAL_ERROR "base renderer report is missing: ${renderer_report}")
endif()
file(READ "${renderer_report}" report_json)
string(JSON check_count LENGTH "${report_json}" checks)
set(required_host_checks
  renderer.install_tree
  renderer.plugin.discovery
  renderer.delegate.creation
  renderer.render_buffer.cpu
  renderer.host.first_frame
  renderer.host.stable_update)
set(observed_host_checks)
math(EXPR check_last "${check_count} - 1")
foreach(index RANGE 0 ${check_last})
  string(JSON check_id GET "${report_json}" checks ${index} id)
  if(check_id IN_LIST required_host_checks)
    string(JSON report_json SET "${report_json}"
      checks ${index} status "\"pass\"")
    # A rerun reads an already-merged report whose SKIP detail is gone, so the
    # removal must stay idempotent instead of failing on the missing member.
    string(JSON check_detail ERROR_VARIABLE check_detail_error
      GET "${report_json}" checks ${index} detail)
    if(NOT check_detail_error)
      string(JSON report_json REMOVE "${report_json}"
        checks ${index} detail)
    endif()
    list(APPEND observed_host_checks "${check_id}")
  endif()
endforeach()
foreach(required IN LISTS required_host_checks)
  if(NOT required IN_LIST observed_host_checks)
    message(FATAL_ERROR
      "base renderer report is missing host assertion ${required}")
  endif()
endforeach()
file(WRITE "${renderer_report}" "${report_json}\n")
file(WRITE "${RENDERER_BUILD_DIR}/renderer-hydra-report.json"
  "${report_json}\n")
