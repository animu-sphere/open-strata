# Codeless USD schema: no compiled code. usdGenSchema turns schema.usda into the
# bundle's resources (plugInfo.json `Types` + generatedSchema.usda) that USD
# loads at runtime. `ost plugin build` composes the resolved runtime so
# usdGenSchema and its Python are on PATH.
cmake_minimum_required(VERSION 3.20)
project({{name}}_schema NONE)

set(_schema_src "${CMAKE_CURRENT_SOURCE_DIR}/schema.usda")
set(_resources  "${CMAKE_CURRENT_SOURCE_DIR}/plugin/resources/{{name}}")

# usdGenSchema regenerates the resources, but it is not required to *use* this
# bundle: the resource-only plugInfo.json is committed and is itself the build
# product. So a missing usdGenSchema is a warning, not a hard error — otherwise
# this bundle would break a plain `cmake -S .` of a plugin-workspace root for a
# non-`ost` user (whose CMAKE_PREFIX_PATH does not put the USD bin on PATH).
# `ost plugin build` composes the resolved runtime so usdGenSchema + Python are
# on PATH and the regenerate target runs.
find_program(USD_GEN_SCHEMA usdGenSchema)
if(NOT USD_GEN_SCHEMA)
    message(WARNING
        "usdGenSchema not found on PATH — skipping schema regeneration; the "
        "committed plugin/resources/{{name}}/plugInfo.json is used as-is. To "
        "regenerate, build via `ost plugin build {{name}}` (puts the runtime's "
        "usdGenSchema + Python on PATH).")
    return()
endif()

# Regenerate the resource-only schema output in place. The generated
# plugInfo.json overwrites the committed host-correct copy with the same content.
add_custom_target({{name}}_schema ALL
    COMMAND "${USD_GEN_SCHEMA}" "${_schema_src}" "${_resources}"
    BYPRODUCTS
        "${_resources}/plugInfo.json"
        "${_resources}/generatedSchema.usda"
    WORKING_DIRECTORY "${CMAKE_CURRENT_SOURCE_DIR}"
    COMMENT "Generating codeless USD schema resources with usdGenSchema"
    VERBATIM)
