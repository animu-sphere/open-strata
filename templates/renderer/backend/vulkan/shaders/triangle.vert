#version 450

const vec2 positions[3] = vec2[](
    vec2(-0.70, -0.65),
    vec2( 0.70, -0.65),
    vec2( 0.00,  0.70));

void main() {
  gl_Position = vec4(positions[gl_VertexIndex], 0.25, 1.0);
}
