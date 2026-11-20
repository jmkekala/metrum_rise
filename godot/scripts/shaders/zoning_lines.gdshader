// SPDX-License-Identifier: GPL-3.0-or-later

shader_type spatial;
render_mode unshaded, blend_mix, cull_disabled, depth_draw_never, fog_disabled;

uniform vec4 line_color : source_color = vec4(1.0, 0.5, 0.0, 0.9);

void fragment() {
    ALBEDO = line_color.rgb;
    ALPHA = line_color.a;
}
