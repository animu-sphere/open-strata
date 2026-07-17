// SPDX-License-Identifier: Apache-2.0
// Single-source bootstrap triangle. slangc compiles one entry point per SPIR-V
// artifact from this module, so the vertex/fragment interface lives in one
// file and the same source can later add a Metal target without a rewrite.

static const float2 kPositions[3] = {
    float2(-0.70, -0.65),
    float2( 0.70, -0.65),
    float2( 0.00,  0.70),
};

struct VertexStageOutput {
  float4 position : SV_Position;
};

[shader("vertex")]
VertexStageOutput vertex_main(uint vertex_index : SV_VertexID) {
  VertexStageOutput output;
  output.position = float4(kPositions[vertex_index], 0.25, 1.0);
  return output;
}

[shader("fragment")]
float4 fragment_main() : SV_Target {
  return float4(0.80, 0.20, 0.10, 1.0);
}
