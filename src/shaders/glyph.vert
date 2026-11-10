attribute vec2 a_position;
attribute vec2 a_uv;
attribute vec4 a_color;
// Coverage exponent for light text (below 1): blending in sRGB thins light-on-dark edges, and
// raising coverage restores their weight. Dark text keeps linear coverage.
uniform float u_light_exponent;
varying PAINT_PRECISION vec2 v_uv;
varying PAINT_PRECISION vec4 v_color;
varying PAINT_PRECISION float v_exponent;
varying PAINT_PRECISION float v_logical_y;
void main() {
    // Quads arrive in logical units, already placed on the physical pixel grid.
    gl_Position = vec4(a_position.x / 960.0 - 1.0, 1.0 - a_position.y / 540.0, 0.0, 1.0);
    v_uv = a_uv;
    v_color = a_color;
    float luminance = dot(a_color.rgb, vec3(0.2126, 0.7152, 0.0722));
    v_exponent = mix(1.0, u_light_exponent, smoothstep(0.35, 0.65, luminance));
    v_logical_y = a_position.y;
}
