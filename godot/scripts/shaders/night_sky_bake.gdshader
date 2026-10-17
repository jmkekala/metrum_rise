// SPDX-License-Identifier: GPL-2.0-only

// One celestial-frame octahedral map, drawn once by night_sky_bake.gd.
shader_type canvas_item;
render_mode unshaded, blend_disabled;

// The shared include also defines world-space lookups; the bake uses only its celestial sum.
global uniform vec4 weather_sun;
#include "res://scripts/shaders/night_sky.gdshaderinc"

void fragment() {
	// UV addresses texel centres. Unfold the lower octahedron, then project onto the sphere.
	vec2 uv = UV * 2.0 - 1.0;
	vec3 c = vec3(uv, 1.0 - abs(uv.x) - abs(uv.y));
	float fold = max(-c.z, 0.0);
	c.xy -= mix(vec2(-fold), vec2(fold), step(vec2(0.0), c.xy));
	c = normalize(c);
	// Godot 4.7.2 RD canvas.glsl writes custom COLOR directly (lines 701, 859).
	// With use_hdr_2d the target is RGBA16F: no sRGB encoding or peak clamping is needed.
	COLOR = vec4(night_sky_celestial(c, night_sky_texel_angle(c)), 1.0);
}
