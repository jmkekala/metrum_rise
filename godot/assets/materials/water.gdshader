shader_type spatial;
render_mode depth_prepass_alpha, shadows_disabled, cull_disabled;

uniform sampler2D heightmap : repeat_disable, filter_linear;
uniform sampler2D watermap : repeat_disable, filter_linear;
uniform sampler2D screen_texture : hint_screen_texture, repeat_disable, filter_linear_mipmap;
uniform float height_scale = 10.0;
uniform vec4 shallow_water_color : source_color = vec4(0.10, 0.28, 0.40, 0.62);
uniform vec4 deep_water_color : source_color = vec4(0.018, 0.09, 0.18, 0.90);
uniform vec4 foam_color : source_color = vec4(0.76, 0.91, 0.96, 0.82);
uniform vec4 sky_reflection_color : source_color = vec4(0.38, 0.56, 0.72, 1.0);
uniform float shore_softness_m = 0.26;
uniform float shore_foam_band_m = 0.18;
uniform float water_deep_color_depth_m = 3.5;
uniform float water_fresnel_strength = 0.46;
uniform float water_fresnel_power = 2.8;
uniform float water_wave_color_strength = 0.052;
uniform float water_wave_roughness_strength = 0.024;
uniform float water_wave_normal_strength = 0.42;
uniform float water_sun_glitter_strength = 0.38;
uniform float water_refraction_strength = 0.010;
uniform float water_refraction_mix = 0.13;
uniform float water_surface_smoothing = 0.94;
uniform float water_surface_blend_radius_texels = 1.0;
uniform int water_visual_debug_mode = 0;
uniform vec3 scene_sun_direction = vec3(-0.46, 0.74, -0.49);
uniform vec3 scene_sun_color : source_color = vec3(1.00, 0.92, 0.80);
uniform vec3 scene_sky_color : source_color = vec3(0.58, 0.73, 0.86);
uniform float scene_ambient_strength = 0.44;
uniform float scene_shadow_max_distance_m = 1800.0;
uniform vec3 scene_shadow_split_distances_m = vec3(144.0, 396.0, 900.0);
uniform vec2 water_debug_patch_key = vec2(0.0);
uniform float water_debug_lod_step = 1.0;
uniform vec2 watermap_texture_size = vec2(1.0, 1.0);
uniform vec2 inner_sample_offset_texels = vec2(1.0, 1.0);
uniform vec2 inner_sample_size_texels = vec2(2.0, 2.0);

varying vec2 var_centered_uv;
varying vec3 var_world_pos;
varying vec3 var_sun_direction_view;
varying vec3 var_view_direction;
varying float var_view_distance_m;

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
	var_sun_direction_view = normalize((MODELVIEW_MATRIX * vec4(normalize(scene_sun_direction), 0.0)).xyz);
	vec3 view_position = (MODELVIEW_MATRIX * vec4(VERTEX, 1.0)).xyz;
	var_view_direction = normalize(-view_position);
	var_view_distance_m = length(view_position);
}

float hash12(vec2 p) {
	return fract(sin(dot(p, vec2(127.1, 311.7))) * 43758.5453123);
}

vec3 value_noise_field(vec2 p) {
	vec2 cell = floor(p);
	vec2 local = fract(p);
	vec2 smooth_local = local * local * (3.0 - 2.0 * local);
	vec2 smooth_derivative = 6.0 * local * (1.0 - local);
	float n00 = hash12(cell);
	float n10 = hash12(cell + vec2(1.0, 0.0));
	float n01 = hash12(cell + vec2(0.0, 1.0));
	float n11 = hash12(cell + vec2(1.0, 1.0));
	float row_0 = mix(n00, n10, smooth_local.x);
	float row_1 = mix(n01, n11, smooth_local.x);
	return vec3(
		mix(row_0, row_1, smooth_local.y),
		mix(n10 - n00, n11 - n01, smooth_local.y) * smooth_derivative.x,
		(row_1 - row_0) * smooth_derivative.y
	);
}

vec3 water_noise_octave(
	vec2 world_xz,
	float scale,
	vec2 axis,
	vec2 drift,
	vec2 offset
) {
	vec2 perpendicular = vec2(-axis.y, axis.x);
	vec2 rotated_world = vec2(dot(world_xz, axis), dot(world_xz, perpendicular));
	vec3 field = value_noise_field(rotated_world * scale + drift * TIME + offset);
	vec2 world_gradient = (axis * field.y + perpendicular * field.z) * scale;
	return vec3(field.x, world_gradient);
}

vec3 water_surface_field(vec2 world_xz) {
	// Rotated, independently drifting octaves avoid exposing either the source
	// value-noise cells or an infinite periodic wave train in high camera views.
	vec3 octave_1 = water_noise_octave(
		world_xz,
		0.0055,
		vec2(0.939693, 0.342020),
		vec2(0.026, -0.018),
		vec2(0.0)
	);
	vec3 octave_2 = water_noise_octave(
		world_xz,
		0.0117,
		vec2(0.573576, -0.819152),
		vec2(-0.021, 0.029),
		vec2(17.4, -9.1)
	);
	vec3 octave_3 = water_noise_octave(
		world_xz,
		0.0240,
		vec2(0.258819, 0.965926),
		vec2(0.017, 0.013),
		vec2(-31.7, 22.8)
	);
	vec3 combined = octave_1 * 0.58 + octave_2 * 0.29 + octave_3 * 0.13;
	return vec3(combined.x, combined.yz * 7.5);
}

vec3 perturb_water_normal(vec3 base_normal, vec2 wave_slope, float strength) {
	vec3 axis = abs(base_normal.y) < 0.98 ? vec3(0.0, 1.0, 0.0) : vec3(1.0, 0.0, 0.0);
	vec3 tangent = normalize(cross(axis, base_normal));
	vec3 bitangent = normalize(cross(base_normal, tangent));
	return normalize(base_normal + (tangent * wave_slope.x + bitangent * wave_slope.y) * strength);
}

vec3 water_world_normal(vec2 wave_slope) {
	return normalize(vec3(
		-wave_slope.x * water_wave_normal_strength * 2.6,
		1.0,
		-wave_slope.y * water_wave_normal_strength * 2.6
	));
}

float water_sun_glitter(vec3 view_normal, float breakup) {
	vec3 sun_dir = normalize(var_sun_direction_view);
	vec3 normal = normalize(view_normal);
	vec3 view_dir = normalize(var_view_direction);
	vec3 reflected_sun = reflect(-sun_dir, normal);
	float alignment = clamp(dot(reflected_sun, view_dir), 0.0, 1.0);
	float broad_path = pow(alignment, 30.0);
	float bright_core = pow(alignment, 72.0);
	float broken_surface = mix(0.72, 1.0, smoothstep(0.38, 0.78, breakup));
	return clamp(
		broad_path * broken_surface * 0.55
			+ bright_core * mix(0.55, 0.85, broken_surface),
		0.0,
		1.0
	);
}

float water_debug_patch_border(vec2 patch_uv) {
	vec2 edge_distance = min(patch_uv, vec2(1.0) - patch_uv);
	float nearest_edge = min(edge_distance.x, edge_distance.y);
	return 1.0 - smoothstep(0.006, 0.018, nearest_edge);
}

vec3 water_debug_patch_color(vec2 key) {
	return vec3(
		0.25 + hash12(key + vec2(17.3, 3.9)) * 0.70,
		0.25 + hash12(key + vec2(61.1, 29.4)) * 0.70,
		0.25 + hash12(key + vec2(8.8, 83.7)) * 0.70
	);
}

vec3 water_debug_lod_color(float lod_step) {
	if (lod_step <= 1.5) {
		return vec3(0.10, 0.85, 0.22);
	}
	if (lod_step <= 2.5) {
		return vec3(0.05, 0.70, 1.00);
	}
	if (lod_step <= 4.5) {
		return vec3(1.00, 0.78, 0.08);
	}
	return vec3(1.00, 0.12, 0.08);
}

vec3 water_material_debug_color(
	vec2 patch_uv,
	vec3 depth_tint,
	float alpha_value,
	float fresnel_value,
	float foam_value,
	float normal_value
) {
	float column = floor(clamp(patch_uv.x, 0.0, 0.999) * 5.0);
	vec3 color = depth_tint;
	if (column < 1.0) {
		color = depth_tint;
	} else if (column < 2.0) {
		color = vec3(alpha_value);
	} else if (column < 3.0) {
		color = mix(vec3(0.03, 0.07, 0.12), sky_reflection_color.rgb, fresnel_value);
	} else if (column < 4.0) {
		color = mix(vec3(0.02, 0.05, 0.08), foam_color.rgb, foam_value);
	} else {
		color = vec3(normal_value, normal_value * 0.65, 1.0 - normal_value * 0.55);
	}
	float separator = step(fract(patch_uv.x * 5.0), 0.018);
	return mix(color, vec3(1.0), separator * 0.65);
}

vec3 water_lighting_debug_color(
	vec2 patch_uv,
	vec3 world_normal,
	float fresnel_value,
	float specular_value,
	float normal_value
) {
	float column = floor(clamp(patch_uv.x, 0.0, 0.999) * 5.0);
	vec3 sun_dir = normalize(scene_sun_direction);
	float sun_facing = clamp(dot(world_normal, sun_dir), 0.0, 1.0);
	float camera_distance_m = var_view_distance_m;
	vec3 cascade_color = vec3(1.00, 0.20, 0.14);
	if (camera_distance_m < scene_shadow_split_distances_m.x) {
		cascade_color = vec3(0.20, 0.82, 0.28);
	} else if (camera_distance_m < scene_shadow_split_distances_m.y) {
		cascade_color = vec3(0.12, 0.58, 1.00);
	} else if (camera_distance_m < scene_shadow_split_distances_m.z) {
		cascade_color = vec3(1.00, 0.78, 0.16);
	}
	vec3 color = mix(vec3(0.02, 0.04, 0.08), scene_sun_color, sun_facing);
	if (column < 1.0) {
		color = mix(vec3(0.02, 0.04, 0.08), scene_sun_color, sun_facing);
	} else if (column < 2.0) {
		color = mix(vec3(0.02, 0.03, 0.05), vec3(1.0, 0.94, 0.72), specular_value);
	} else if (column < 3.0) {
		color = mix(vec3(0.03, 0.07, 0.12), scene_sky_color, fresnel_value);
	} else if (column < 4.0) {
		color = cascade_color;
	} else {
		color = vec3(normal_value, normal_value * 0.65, 1.0 - normal_value * 0.55);
	}
	float separator = step(fract(patch_uv.x * 5.0), 0.018);
	return mix(color, vec3(1.0), separator * 0.65);
}

vec3 water_visual_debug_color(vec2 patch_uv, float depth_m, float shore_mask) {
	vec3 debug_color = vec3(0.0);
	if (water_visual_debug_mode == 1 || water_visual_debug_mode == 8) {
		debug_color = water_debug_patch_color(water_debug_patch_key);
	} else if (water_visual_debug_mode == 2 || water_visual_debug_mode == 7) {
		debug_color = water_debug_lod_color(water_debug_lod_step);
	} else if (water_visual_debug_mode == 3) {
		float terrain_height = sample_terrain_world_height(var_centered_uv);
		debug_color = vec3(clamp(terrain_height / max(height_scale, 0.001), 0.0, 1.0));
	} else if (water_visual_debug_mode == 5) {
		debug_color = vec3(shore_mask, depth_m > 0.001 ? 0.45 : 0.0, 1.0 - shore_mask);
	} else {
		float depth_visibility = smoothstep(0.0, 6.0, depth_m);
		debug_color = mix(vec3(0.02, 0.04, 0.08), vec3(0.02, 0.60, 1.00), depth_visibility);
	}
	float border = water_debug_patch_border(patch_uv);
	return mix(debug_color, vec3(1.0), border);
}

void fragment() {
	float contour_depth_m = sample_contour_water_depth(var_centered_uv);
	float depth_m = contour_depth_m;
	float shore_mask = smoothstep(0.0, max(shore_softness_m, 0.001), contour_depth_m);
	float depth_factor = smoothstep(0.12, max(water_deep_color_depth_m, 0.121), depth_m);
	vec3 surface_field = water_surface_field(var_world_pos.xz);
	float wave = surface_field.x;
	float wave_signed = wave * 2.0 - 1.0;
	vec2 wave_slope = surface_field.yz;
	vec3 world_normal = water_world_normal(wave_slope);
	float normal_debug_strength = clamp(length(wave_slope) * water_wave_normal_strength * 18.0, 0.0, 1.0);
	vec3 depth_tint = mix(shallow_water_color.rgb, deep_water_color.rgb, depth_factor);
	depth_tint *= 1.0 + wave_signed * water_wave_color_strength;
	float shore_foam = (1.0 - smoothstep(0.0, max(shore_foam_band_m, 0.001), contour_depth_m))
		* shore_mask;
	float foam_amount = clamp(shore_foam * 0.16, 0.0, 0.34);
	float final_alpha = mix(shallow_water_color.a, deep_water_color.a, depth_factor) * shore_mask;
	final_alpha = clamp(final_alpha, 0.0, 0.92);

	vec3 base_view_normal = normalize(NORMAL);
	NORMAL = perturb_water_normal(base_view_normal, wave_slope, water_wave_normal_strength);
	float fresnel = pow(
		1.0 - clamp(dot(base_view_normal, normalize(var_view_direction)), 0.0, 1.0),
		water_fresnel_power
	);
	float fresnel_mix = clamp(fresnel * water_fresnel_strength, 0.0, 1.0);
	float sun_specular = water_sun_glitter(NORMAL, wave)
		* shore_mask
		* (0.40 + depth_factor * 0.60);
	vec3 sky_reflection = mix(sky_reflection_color.rgb, scene_sky_color, 0.65);

	if (water_visual_debug_mode > 0) {
		if (water_visual_debug_mode == 9) {
			ALBEDO = water_material_debug_color(
				UV,
				depth_tint,
				final_alpha,
				fresnel_mix,
				foam_amount,
				normal_debug_strength
			);
		} else if (water_visual_debug_mode == 10) {
			ALBEDO = water_lighting_debug_color(
				UV,
				world_normal,
				fresnel_mix,
				sun_specular,
				normal_debug_strength
			);
		} else {
			ALBEDO = water_visual_debug_color(UV, depth_m, shore_mask);
		}
		ALPHA = 0.92;
		ROUGHNESS = 1.0;
		SPECULAR = 0.0;
	} else {
		if (shore_mask < 0.001 && contour_depth_m <= 0.001) {
			discard;
		}

		vec3 final_color = mix(depth_tint, foam_color.rgb, foam_amount);
		float shallow_refraction = (1.0 - depth_factor) * shore_mask;
		vec2 refraction_offset = wave_slope * water_refraction_strength * shallow_refraction;
		vec3 refracted_scene = texture(screen_texture, clamp(SCREEN_UV + refraction_offset, vec2(0.001), vec2(0.999))).rgb;
		final_color = mix(
			final_color,
			refracted_scene,
			clamp(shallow_refraction * water_refraction_mix, 0.0, 0.35)
		);
		final_color = mix(final_color, sky_reflection, fresnel_mix);
		final_color += scene_sun_color * sun_specular * water_sun_glitter_strength;
		float topdown_ripple = smoothstep(0.48, 0.86, wave)
			* normal_debug_strength
			* (1.0 - fresnel_mix * 0.35)
			* 0.07;
		final_color = mix(final_color, sky_reflection, topdown_ripple);
		ALBEDO = final_color;
		ALPHA = final_alpha;
		ROUGHNESS = clamp(0.08 + normal_debug_strength * water_wave_roughness_strength, 0.05, 0.20);
		SPECULAR = 0.72;
	}
}
