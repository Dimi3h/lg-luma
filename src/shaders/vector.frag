varying PAINT_PRECISION vec4 v_color;
varying PAINT_PRECISION float v_coverage;
varying PAINT_PRECISION float v_logical_y;
uniform vec4 u_edge_fade;
// Text only (analytic path): coverage is raised to this power; 0 (unset) and 1 leave it linear.
uniform float u_coverage_exponent;
void main() {
    float edge = 1.0;
    if (u_edge_fade.y > u_edge_fade.x) {
        edge = smoothstep(u_edge_fade.x, u_edge_fade.y, v_logical_y) *
            (1.0 - smoothstep(u_edge_fade.z, u_edge_fade.w, v_logical_y));
    }
    float coverage = clamp(v_coverage, 0.0, 1.0);
    if (u_coverage_exponent > 0.0 && u_coverage_exponent != 1.0)
        coverage = pow(coverage, u_coverage_exponent);
    gl_FragColor = vec4(v_color.rgb, v_color.a * coverage * edge);
}
