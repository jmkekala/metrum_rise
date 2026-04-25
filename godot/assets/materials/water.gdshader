shader_type spatial;
render_mode depth_prepass_alpha, shadows_disabled, cull_disabled;

uniform sampler2D heightmap : repeat_disable, filter_linear;
uniform sampler2D watermap : repeat_disable, filter_linear;
uniform sampler2D velocity_map : repeat_disable, filter_linear;
uniform float height_scale = 10.0;
uniform vec4 shallow_water_color : source_color = vec4(0.20, 0.37, 0.40, 0.58);
uniform vec4 deep_water_color : source_color = vec4(0.05, 0.16, 0.29, 0.86);
uniform vec4 foam_color : source_color = vec4(0.8, 0.9, 1.0, 0.9);
uniform float shore_softness_m = 2.5;
uniform float shore_foam_band_m = 1.4;
uniform float water_fresnel_strength = 0.24;
uniform float water_fresnel_power = 4.0;
uniform float water_wave_color_strength = 0.025;
uniform float water_wave_roughness_strength = 0.010;
uniform float water_surface_smoothing = 0.94;
uniform float water_surface_blend_radius_texels = 1.0;
uniform vec2 watermap_texture_size = vec2(1.0, 1.0);
uniform vec2 inner_sample_offset_texels = vec2(1.0, 1.0);
uniform vec2 inner_sample_size_texels = vec2(2.0, 2.0);

varying vec2 var_centered_uv;
varying vec3 var_world_pos;

vec2 centered_uv(vec2 uv) {
	vec2 safe_inner_size = max(inner_sample_size_texels, vec2(1.0));
	return (inner_sample_offset_texels + uv * (safe_inner_size - 1.0) + 0.5)
		/ max(watermap_texture_size, vec2(1.0));
}

float sample_terrain_world_height(vec2 uv) {
	return texture(heightmap, uv).r * height_scale;
}

float sample_raw_water_depth(vec2 uv) {
	return texture(watermap, uv).r;
}

float sample_contour_water_depth(vec2 uv) {
	// The watermap sampler uses linear filtering. Sampling it between texel
	// centers yields a contour-style shoreline field on top of the authored
	// grid instead of exposing whole-cell steps directly.
	return sample_raw_water_depth(uv);
}

float sample_display_water_depth(vec2 uv) {
	float center_terrain = sample_terrain_world_height(uv);
	float center_depth = sample_raw_water_depth(uv);
	float center_surface = center_terrain + center_depth;
	vec2 texel = vec2(1.0) / max(watermap_texture_size, vec2(1.0, 1.0));
	vec2 dx = vec2(texel.x * max(water_surface_blend_radius_texels, 0.5), 0.0);
	vec2 dz = vec2(0.0, texel.y * max(water_surface_blend_radius_texels, 0.5));
	float accum_surface = 0.0;
	float accum_weight = 0.0;

	if (center_depth > 0.001) {
		accum_surface += center_surface * 2.0;
		accum_weight += 2.0;
	}

	float left_depth = sample_raw_water_depth(uv - dx);
	if (left_depth > 0.001) {
		accum_surface += sample_terrain_world_height(uv - dx) + left_depth;
		accum_weight += 1.0;
	}

	float right_depth = sample_raw_water_depth(uv + dx);
	if (right_depth > 0.001) {
		accum_surface += sample_terrain_world_height(uv + dx) + right_depth;
		accum_weight += 1.0;
	}

	float top_depth = sample_raw_water_depth(uv - dz);
	if (top_depth > 0.001) {
		accum_surface += sample_terrain_world_height(uv - dz) + top_depth;
		accum_weight += 1.0;
	}

	float bottom_depth = sample_raw_water_depth(uv + dz);
	if (bottom_depth > 0.001) {
		accum_surface += sample_terrain_world_height(uv + dz) + bottom_depth;
		accum_weight += 1.0;
	}

	if (accum_weight <= 0.0) {
		return 0.0;
	}

	float average_surface = accum_surface / accum_weight;
	float display_surface = average_surface;
	if (center_depth > 0.001) {
		display_surface = mix(
			center_surface,
			average_surface,
			clamp(water_surface_smoothing, 0.0, 1.0)
		);
	}
	return max(display_surface - center_terrain, 0.0);
}

void vertex() {
	var_centered_uv = centered_uv(UV);
	float terrain_height = texture(heightmap, var_centered_uv).r;
	float water_depth = sample_display_water_depth(var_centered_uv);
	
	// Terrain samples are still in pre-scaled storage units, but water depth is
	// already world-space metres. Only the terrain sample should use height_scale.
	VERTEX.y += terrain_height * height_scale + water_depth + 0.05;
	var_world_pos = (MODEL_MATRIX * vec4(VERTEX, 1.0)).xyz;
}

float hash12(vec2 p) {
	return fract(sin(dot(p, vec2(127.1, 311.7))) * 43758.5453123);
}

float value_noise(vec2 p) {
	vec2 cell = floor(p);
	vec2 local = fract(p);
	vec2 smooth_local = local * local * (3.0 - 2.0 * local);
	float n00 = hash12(cell);
	float n10 = hash12(cell + vec2(1.0, 0.0));
	float n01 = hash12(cell + vec2(0.0, 1.0));
	float n11 = hash12(cell + vec2(1.0, 1.0));
	return mix(
		mix(n00, n10, smooth_local.x),
		mix(n01, n11, smooth_local.x),
		smooth_local.y
	);
}

float water_surface_breakup(vec2 world_xz) {
	// Use soft, non-periodic motion so the sun reflection does not reveal a
	// regular checker or sine interference pattern.
	vec2 p = world_xz * 0.006 + vec2(TIME * 0.028, -TIME * 0.019);
	float n1 = value_noise(p);
	float n2 = value_noise(p * 2.13 + vec2(17.4, -9.1));
	float n3 = value_noise(p * 4.37 + vec2(-31.7, 22.8));
	return n1 * 0.58 + n2 * 0.29 + n3 * 0.13;
}

void fragment() {
	float contour_depth_m = sample_contour_water_depth(var_centered_uv);
	float depth_m = contour_depth_m;
	float speed_sample = texture(velocity_map, var_centered_uv).r;
	float shore_mask = smoothstep(0.0, max(shore_softness_m, 0.001), contour_depth_m);
	if (shore_mask < 0.001 && contour_depth_m <= 0.001) {
		discard;
	}
	
	float depth_factor = smoothstep(0.0, 10.0, depth_m);
	float wave = water_surface_breakup(var_world_pos.xz);
	float wave_signed = wave * 2.0 - 1.0;
	vec3 base_color = mix(shallow_water_color.rgb, deep_water_color.rgb, depth_factor);
	base_color *= 1.0 + wave_signed * water_wave_color_strength;

	// Speed-based color mixing
	float speed = clamp(speed_sample * 0.5, 0.0, 1.0);
	vec3 final_color = mix(base_color, foam_color.rgb, speed * 0.35);
	float final_alpha = mix(shallow_water_color.a, deep_water_color.a, depth_factor) * shore_mask;
	float shore_foam = 1.0 - smoothstep(0.0, max(shore_foam_band_m, 0.001), contour_depth_m);
	final_color = mix(final_color, foam_color.rgb, shore_foam * 0.35 + speed * 0.10);
	float fresnel = pow(
		1.0 - clamp(dot(normalize(NORMAL), normalize(VIEW)), 0.0, 1.0),
		water_fresnel_power
	);
	final_color = mix(final_color, vec3(0.96, 0.98, 1.0), fresnel * water_fresnel_strength);
	ALBEDO = final_color;
	ALPHA = final_alpha;
	ROUGHNESS = clamp(0.10 + speed * 0.08 + abs(wave_signed) * water_wave_roughness_strength, 0.06, 0.22);
	SPECULAR = 0.62;
}
