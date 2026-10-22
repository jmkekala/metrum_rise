// SPDX-License-Identifier: GPL-2.0-only

// Shared field colour; callers supply the terrain grain uniforms and noise2d.
vec2 rotate2d(vec2 uv, float angle) {
	float s = sin(angle);
	float c = cos(angle);
	return vec2(c * uv.x - s * uv.y, s * uv.x + c * uv.y);
}

vec3 terrain_grain_field_color(vec2 world_xz) {
	vec2 world_uv = world_xz * terrain_grain_detail_scale;
	vec3 grain_a = texture(terrain_grain_albedo, world_uv).rgb;
	vec3 grain_b = texture(
		terrain_grain_albedo,
		rotate2d(world_uv + vec2(19.17, 7.31), 1.5707963268)
	).rgb;
	vec3 grain_c = texture(
		terrain_grain_albedo,
		rotate2d(world_uv * 0.87 + vec2(-11.43, 23.61), -0.7853981634)
	).rgb;

	float mid_mask = smoothstep(0.25, 0.75, noise2d(world_xz * 0.050));
	float wide_mask = smoothstep(0.18, 0.82, noise2d(world_xz * 0.016 + vec2(7.0, 2.0)));
	vec3 grain = mix(grain_a, grain_b, mid_mask * terrain_grain_mix_strength);
	grain = mix(grain, grain_c, wide_mask * terrain_grain_mix_strength * 0.55);

	float macro = noise2d(world_xz * 0.010 + vec2(2.1, 6.4));
	grain *= mix(1.0 - terrain_grain_macro_strength, 1.0 + terrain_grain_macro_strength, macro);
	return grain * terrain_grain_tint * terrain_grain_brightness;
}
