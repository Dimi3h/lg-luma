uniform sampler2D u_atlas;
uniform vec4 u_edge_fade;
varying PAINT_PRECISION vec2 v_uv;
varying PAINT_PRECISION vec4 v_color;
varying PAINT_PRECISION float v_exponent;
varying PAINT_PRECISION float v_logical_y;
void main() {
    float edge = 1.0;
    if (u_edge_fade.y > u_edge_fade.x) {
        edge = smoothstep(u_edge_fade.x, u_edge_fade.y, v_logical_y) *
            (1.0 - smoothstep(u_edge_fade.z, u_edge_fade.w, v_logical_y));
    }
    // The atlas is single-channel (LUMINANCE on GLES 2, RED on desktop): coverage is in .r.
    float coverage = texture2D(u_atlas, v_uv).r;
    coverage = pow(coverage, v_exponent);
    gl_FragColor = vec4(v_color.rgb, v_color.a * coverage * edge);
}
