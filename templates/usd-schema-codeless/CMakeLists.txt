# Codeless USD schema: no compiled code. usdGenSchema turns schema.usda into the
# bundle's resources (plugInfo.json `Types` + generatedSchema.usda) that USD
# loads at runtime. `ost plugin build` composes the resolved runtime so
# usdGenSchema and its Python are on PATH.
cmake_minimum_required(VERSION 3.20)
project({{name}}_schema NONE)

find_program(USD_GEN_SCHEMA usdGenSchema)
if(NOT USD_GEN_SCHEMA)
    message(FATAL_ERROR
        "usdGenSchema not found on PATH — build via `ost plugin build {{name}}`, "
        "which puts the runtime's usdGenSchema + Python on PATH.")
endif()

set(_schema_src "${CMAKE_CURRENT_SOURCE_DIR}/schema.usda")
set(_resources  "${CMAKE_CURRENT_SOURCE_DIR}/plugin/resources/{{name}}")

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
