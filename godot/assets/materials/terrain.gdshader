shader_type spatial;
render_mode cull_disabled;

uniform sampler2D heightmap;
uniform sampler2D overlay_texture : repeat_disable, filter_linear;
uniform sampler2D coal_pit_texture : repeat_disable, filter_linear;
uniform sampler2D field_overlay_texture : repeat_disable, filter_linear;
uniform sampler2D watermap : repeat_disable, filter_linear;
uniform sampler2D land_cover_texture : hint_default_black, repeat_disable, filter_linear;
uniform vec4 land_cover_world_bounds = vec4(0.0);
uniform sampler2D terrain_coal_albedo : source_color, filter_linear_mipmap_anisotropic, repeat_enable;
uniform sampler2D terrain_grain_albedo : source_color, filter_linear_mipmap_anisotropic, repeat_enable;
uniform sampler2D site_asphalt_albedo_tex : source_color, filter_linear_mipmap_anisotropic, repeat_enable;
uniform sampler2D site_asphalt_normal_tex : hint_normal, filter_linear_mipmap_anisotropic, repeat_enable;
uniform sampler2D site_asphalt_roughness_tex : hint_roughness_r, filter_linear_mipmap_anisotropic, repeat_enable;
uniform sampler2D site_concrete_albedo_tex : source_color, filter_linear_mipmap_anisotropic, repeat_enable;
uniform sampler2D site_concrete_normal_tex : hint_normal, filter_linear_mipmap_anisotropic, repeat_enable;
uniform sampler2D site_concrete_roughness_tex : hint_roughness_r, filter_linear_mipmap_anisotropic, repeat_enable;
uniform float site_asphalt_uv_scale = 0.12;
uniform float site_asphalt_macro_uv_scale = 0.018;
uniform float site_asphalt_macro_influence = 0.25;
uniform float site_asphalt_brightness = 1.0;
uniform vec3 site_asphalt_albedo_floor = vec3(0.0);
uniform float site_asphalt_floor_influence = 0.0;
uniform float site_concrete_uv_scale = 0.18;
uniform float site_concrete_macro_uv_scale = 0.030;
uniform float site_concrete_macro_influence = 0.12;
uniform float site_concrete_brightness = 1.0;
uniform vec3 site_concrete_albedo_floor = vec3(0.0);
uniform float site_concrete_floor_influence = 0.0;
uniform int overlay_mode = 0;
uniform float height_scale = 10.0;
uniform bool height_is_baked = false;
uniform vec2 world_size = vec2(256.0, 256.0);
uniform vec4 coal_pit_overlay_world_bounds = vec4(0.0, 0.0, 0.0, 0.0);
uniform vec4 field_overlay_world_bounds = vec4(0.0, 0.0, 0.0, 0.0);
uniform int terrain_visual_debug_mode = 0;
uniform vec2 terrain_debug_patch_key = vec2(0.0);
uniform float terrain_debug_lod_step = 1.0;
uniform int terrain_grass_visual_debug_mode = 0;
global uniform vec3 scene_sun_direction;
global uniform vec4 scene_sun_color : source_color;
global uniform float scene_ambient_strength;
// Multiplier for the authored ambient floor. IRRADIANCE suppresses the environment's
// ambient term while keeping screen-space indirect light available to the ground.
global uniform vec4 scene_ambient_light : source_color;
// Colour vision washes out at night. Dimming alone leaves a saturated green ground reading as
// daylight with the brightness turned down, because the albedo's green channel still dominates
// whatever the ambient tint is multiplied by.
global uniform float scene_ambient_desaturation;
uniform float scene_shadow_max_distance_m = 1800.0;
uniform vec3 scene_shadow_split_distances_m = vec3(144.0, 396.0, 900.0);
uniform float ground_shadow_ambient = 0.64;
uniform float ground_shadow_sun_strength = 0.36;
uniform float ground_shadow_min_visibility = 0.10;
// Forest-floor shading past the shadow cascades. See CANOPY_FLOOR_SHADE_FLOOR in
// scene_lighting.gd; the crown half of the same term lives in vegetation_wind.gdshaderinc.
uniform float canopy_floor_shade_begin_m = 327.6;
uniform float canopy_floor_shade_end_m = 420.0;
uniform float canopy_floor_shade_floor : hint_range(0.0, 1.0) = 0.51;
uniform float canopy_floor_shade : hint_range(0.0, 2.0) = 1.0;
uniform vec2 heightmap_texture_size = vec2(256.0, 256.0);
uniform vec2 inner_sample_offset_texels = vec2(1.0, 1.0);
uniform vec2 inner_sample_size_texels = vec2(2.0, 2.0);
uniform vec2 watermap_texture_size = vec2(2.0, 2.0);
uniform vec2 watermap_inner_sample_offset_texels = vec2(0.0, 0.0);
uniform vec2 watermap_inner_sample_size_texels = vec2(2.0, 2.0);
uniform vec2 patch_world_size_m = vec2(256.0, 256.0);
uniform float terrain_cell_m = 10.0;
uniform float terrain_baked_readability_strength = 0.12;
uniform float terrain_coal_detail_scale = 0.22;
uniform float terrain_coal_pit_strength = 0.72;
uniform vec3 terrain_coal_lift_color : source_color = vec3(0.18, 0.16, 0.13);
uniform float terrain_coal_lift_strength = 0.45;
uniform float terrain_grain_detail_scale = 0.0833333333;
uniform float terrain_field_strength = 0.88;
uniform vec3 terrain_grain_tint : source_color = vec3(1.08, 1.01, 0.78);
uniform float terrain_grain_brightness = 1.10;
uniform float terrain_grain_mix_strength = 0.38;
uniform float terrain_grain_macro_strength = 0.12;
uniform float terrain_rock_slope_start = 0.15;
uniform float terrain_rock_slope_end = 0.34;
uniform float terrain_relief_sample_radius_texels = 3.0;
uniform float terrain_relief_start_m = 2.0;
uniform float terrain_relief_end_m = 16.0;
uniform float terrain_shore_blend_strength = 0.15;
uniform float terrain_shore_lookup_radius_texels = 0.55;
uniform float cliff_slope_start = 0.26;
uniform float cliff_slope_end = 0.44;
uniform float cliff_relief_start_m = 4.0;
uniform float cliff_relief_end_m = 14.0;
uniform float cliff_sample_radius_texels = 2.25;
uniform float cliff_lateral_smoothing_texels = 1.2;
uniform float cliff_face_strength = 0.34;
uniform float cliff_edge_strength = 0.46;
uniform float cliff_contour_fade = 0.78;
uniform vec3 cliff_face_color : source_color = vec3(0.35, 0.35, 0.32);
uniform vec3 cliff_top_edge_color : source_color = vec3(0.27, 0.28, 0.22);
uniform vec3 cliff_toe_edge_color : source_color = vec3(0.19, 0.20, 0.18);
uniform float contour_minor_interval_m = 2.5;
uniform float contour_major_interval_m = 10.0;
uniform float contour_minor_thickness = 0.95;
uniform float contour_major_thickness = 1.25;
uniform vec3 contour_minor_color : source_color = vec3(0.13, 0.19, 0.16);
uniform vec3 contour_major_color : source_color = vec3(0.10, 0.16, 0.14);
uniform float contour_minor_strength = 0.14;
uniform float contour_major_strength = 0.34;
uniform float contour_relief_minor_boost_strength = 0.10;
// Contour lines belong to the map views. Clear this to draw them in the plain world view.
uniform bool contour_map_view_only = true;
uniform float contour_zero_elevation_fade_m = 0.75;
uniform float contour_flat_relief_start_m = 0.10;
uniform float contour_flat_relief_end_m = 1.25;

varying float h;
varying vec2 var_centered_uv;
varying vec2 var_water_uv;
varying vec3 var_world_pos;
varying float var_view_distance_m;

#include "res://scripts/shaders/ground_grass.gdshaderinc"

vec2 watermap_centered_uv(vec2 patch_uv) {
	vec2 safe_inner_size = max(watermap_inner_sample_size_texels, vec2(1.0));
	return (watermap_inner_sample_offset_texels + patch_uv * (safe_inner_size - 1.0) + 0.5)
		/ max(watermap_texture_size, vec2(1.0));
}

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

float sample_world_height(vec2 uv) {
	vec2 clamped_uv = clamp(uv, vec2(0.0), vec2(1.0));
	return texture(heightmap, clamped_uv).r * height_scale;
}

vec2 sample_height_gradient(vec2 uv) {
	vec2 texel = 1.0 / max(heightmap_texture_size, vec2(1.0));
	float h_left = sample_world_height(uv - vec2(texel.x, 0.0));
	float h_right = sample_world_height(uv + vec2(texel.x, 0.0));
	float h_up = sample_world_height(uv - vec2(0.0, texel.y));
	float h_down = sample_world_height(uv + vec2(0.0, texel.y));
	float safe_cell = max(terrain_cell_m, 0.001);
	return vec2(
		(h_right - h_left) / (2.0 * safe_cell),
		(h_down - h_up) / (2.0 * safe_cell)
	);
}

vec3 sample_world_normal(vec2 uv) {
	vec2 gradient = sample_height_gradient(uv);
	return normalize(vec3(-gradient.x, 1.0, -gradient.y));
}

vec3 terrain_grass_debug_color(vec3 base_color, vec3 world_pos, float grass_mask) {
	vec2 micro_uv = world_pos.xz * terrain_grass_detail_scale;
	vec2 micro_cross_uv = mat2(vec2(0.80, 0.60), vec2(-0.60, 0.80))
		* world_pos.xz
		* (terrain_grass_detail_scale * 0.71)
		+ vec2(31.7, 11.3);
	vec3 raw_albedo = texture(terrain_grass_albedo, micro_uv).rgb;
	vec3 macro_grass = sample_grass_albedo_stochastic(world_pos.xz * terrain_grass_macro_scale, 11.0);
	vec3 mid_grass = sample_grass_albedo_stochastic(world_pos.xz * terrain_grass_mid_scale, 29.0);
	vec3 micro_grass = sample_grass_albedo_cross(world_pos.xz, terrain_grass_detail_scale);
	float footprint = grass_texture_footprint(micro_uv, micro_cross_uv);
	float fade_end = max(terrain_grass_detail_fade_end, terrain_grass_detail_fade_start + 0.001);
	float micro_visibility = grass_micro_visibility_from_footprint(footprint);
	float height_detail = sample_grass_height_cross(world_pos.xz, terrain_grass_detail_scale);
	if (terrain_grass_visual_debug_mode == 1) {
		return raw_albedo;
	}
	if (terrain_grass_visual_debug_mode == 2) {
		return macro_grass;
	}
	if (terrain_grass_visual_debug_mode == 3) {
		return mid_grass;
	}
	if (terrain_grass_visual_debug_mode == 4) {
		return micro_grass;
	}
	if (terrain_grass_visual_debug_mode == 5) {
		return vec3(1.0, 1.0, micro_visibility) * grass_mask;
	}
	if (terrain_grass_visual_debug_mode == 6) {
		vec3 debug_color = base_color;
		float master_strength = grass_mask * clamp(terrain_grass_albedo_strength, 0.0, 1.0);
		debug_color = mix(
			debug_color,
			grass_material_layer(debug_color, macro_grass, world_pos.xz, 0.90, 0.50),
			master_strength * clamp(terrain_grass_macro_strength, 0.0, 1.0)
		);
		debug_color = mix(
			debug_color,
			grass_material_layer(debug_color, mid_grass, world_pos.xz, 1.10, 0.58),
			master_strength * clamp(terrain_grass_mid_strength, 0.0, 1.0)
		);
		debug_color = mix(
			debug_color,
			grass_material_layer(debug_color, micro_grass, world_pos.xz, 1.35, 0.42),
			master_strength * clamp(terrain_grass_micro_strength, 0.0, 1.0) * micro_visibility
		);
		float detail_debug_mask = grass_mask * micro_visibility;
		return debug_color * mix(
			1.0,
			grass_detail_brightness(micro_grass, mid_grass, height_detail, world_pos.xz),
			detail_debug_mask
		);
	}
	if (terrain_grass_visual_debug_mode == 7) {
		return vec3(height_detail);
	}
	if (terrain_grass_visual_debug_mode == 8) {
		return vec3(grass_mask);
	}
	if (terrain_grass_visual_debug_mode == 9 || terrain_grass_visual_debug_mode == 10) {
		float master_strength = grass_mask * clamp(terrain_grass_albedo_strength, 0.0, 1.0);
		vec3 macro_color = mix(
			base_color,
			grass_material_layer(base_color, macro_grass, world_pos.xz, 0.90, 0.50),
			master_strength * clamp(terrain_grass_macro_strength, 0.0, 1.0)
		);
		vec3 mid_color = mix(
			macro_color,
			grass_material_layer(macro_color, mid_grass, world_pos.xz, 1.10, 0.58),
			master_strength * clamp(terrain_grass_mid_strength, 0.0, 1.0)
		);
		vec3 micro_color = mix(
			mid_color,
			grass_material_layer(mid_color, micro_grass, world_pos.xz, 1.35, 0.42),
			master_strength
				* clamp(terrain_grass_micro_strength, 0.0, 1.0)
				* micro_visibility
		);
		float detail_strength = grass_mask * micro_visibility;
		vec3 final_color = micro_color * (
			1.0
			+ (grass_detail_brightness(micro_grass, mid_grass, height_detail, world_pos.xz) - 1.0)
				* detail_strength
		);
		if (terrain_grass_visual_debug_mode == 10) {
			return vec3(
				clamp(footprint / fade_end, 0.0, 1.0),
				micro_visibility,
				grass_mask
			);
		}
		float lane = floor(fract(world_pos.x * 0.018) * 5.0);
		float luma = color_luminance(base_color);
		vec3 tint = vec3(1.00, 0.28, 0.24);
		if (lane < 0.5) {
			luma = color_luminance(base_color);
			tint = vec3(1.00, 0.28, 0.24);
		} else if (lane < 1.5) {
			luma = color_luminance(macro_color);
			tint = vec3(1.00, 0.78, 0.18);
		} else if (lane < 2.5) {
			luma = color_luminance(mid_color);
			tint = vec3(0.26, 0.95, 0.30);
		} else if (lane < 3.5) {
			luma = color_luminance(micro_color);
			tint = vec3(0.24, 0.62, 1.00);
		} else {
			luma = color_luminance(final_color);
			tint = vec3(1.00);
		}
		return clamp(vec3(luma * 1.45) * tint, vec3(0.0), vec3(1.0));
	}
	return base_color;
}

float sample_local_relief(vec2 uv) {
	vec2 texel = terrain_relief_sample_radius_texels / max(heightmap_texture_size, vec2(1.0));
	float center = sample_world_height(uv);
	float h_x_pos = sample_world_height(uv + vec2(texel.x, 0.0));
	float h_x_neg = sample_world_height(uv - vec2(texel.x, 0.0));
	float h_y_pos = sample_world_height(uv + vec2(0.0, texel.y));
	float h_y_neg = sample_world_height(uv - vec2(0.0, texel.y));
	float h_d0 = sample_world_height(uv + texel);
	float h_d1 = sample_world_height(uv - texel);
	float h_d2 = sample_world_height(uv + vec2(texel.x, -texel.y));
	float h_d3 = sample_world_height(uv + vec2(-texel.x, texel.y));
	float max_h = max(center, max(max(h_x_pos, h_x_neg), max(max(h_y_pos, h_y_neg), max(max(h_d0, h_d1), max(h_d2, h_d3)))));
	float min_h = min(center, min(min(h_x_pos, h_x_neg), min(min(h_y_pos, h_y_neg), min(min(h_d0, h_d1), min(h_d2, h_d3)))));
	return max_h - min_h;
}

float shoreline_mask(vec2 uv) {
	vec2 texel = terrain_shore_lookup_radius_texels / max(watermap_texture_size, vec2(1.0));
	float center = texture(watermap, uv).r;
	float ring = 0.0;
	ring += texture(watermap, uv + vec2(texel.x, 0.0)).r;
	ring += texture(watermap, uv - vec2(texel.x, 0.0)).r;
	ring += texture(watermap, uv + vec2(0.0, texel.y)).r;
	ring += texture(watermap, uv - vec2(0.0, texel.y)).r;
	ring += texture(watermap, uv + texel).r;
	ring += texture(watermap, uv - texel).r;
	ring += texture(watermap, uv + vec2(texel.x, -texel.y)).r;
	ring += texture(watermap, uv + vec2(-texel.x, texel.y)).r;
	float nearby = max(center, ring / 8.0);
	return smoothstep(0.02, 0.22, nearby);
}

float terrain_debug_patch_border(vec2 patch_uv) {
	vec2 edge_distance = min(patch_uv, vec2(1.0) - patch_uv);
	float nearest_edge = min(edge_distance.x, edge_distance.y);
	return 1.0 - smoothstep(0.006, 0.018, nearest_edge);
}

vec3 terrain_debug_patch_color(vec2 key) {
	return vec3(
		0.25 + hash12(key + vec2(13.1, 7.7)) * 0.70,
		0.25 + hash12(key + vec2(41.9, 23.3)) * 0.70,
		0.25 + hash12(key + vec2(5.2, 97.4)) * 0.70
	);
}

vec3 terrain_debug_lod_color(float lod_step) {
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

vec3 terrain_visual_debug_color(
	vec2 patch_uv,
	float height_sample,
	float world_elevation,
	float local_relief,
	float slope,
	float shore_mask,
	float grass_detail_mask,
	vec3 normal
) {
	vec3 debug_color = vec3(0.0);
	float water_depth_m = texture(watermap, var_water_uv).r;
	if (terrain_visual_debug_mode == 1) {
		debug_color = terrain_debug_patch_color(terrain_debug_patch_key);
	} else if (terrain_visual_debug_mode == 2) {
		debug_color = terrain_debug_lod_color(terrain_debug_lod_step);
	} else if (terrain_visual_debug_mode == 3) {
		float elevation_band = fract(world_elevation / 25.0);
		debug_color = mix(vec3(height_sample), vec3(elevation_band), 0.20);
	} else if (terrain_visual_debug_mode == 4) {
		debug_color = vec3(smoothstep(0.0, 20.0, local_relief));
	} else if (terrain_visual_debug_mode == 5) {
		debug_color = vec3(shore_mask, water_depth_m > 0.001 ? 0.45 : 0.0, 1.0 - shore_mask);
	} else if (
		terrain_visual_debug_mode == 6
		|| terrain_visual_debug_mode == 7
		|| terrain_visual_debug_mode == 8
		|| terrain_visual_debug_mode == 9
	) {
		float depth_visibility = smoothstep(0.0, 6.0, water_depth_m);
		debug_color = mix(vec3(0.03, 0.07, 0.08), vec3(0.02, 0.55, 1.00), depth_visibility);
		debug_color = mix(debug_color, vec3(0.05, 0.40, 0.18), grass_detail_mask * 0.25);
	} else if (terrain_visual_debug_mode == 10) {
		vec3 sun_dir = normalize(scene_sun_direction);
		float sun_facing = clamp(dot(normalize(normal), sun_dir), 0.0, 1.0);
		float camera_distance_m = var_view_distance_m;
		float split0 = scene_shadow_split_distances_m.x;
		float split1 = scene_shadow_split_distances_m.y;
		float split2 = scene_shadow_split_distances_m.z;
		float cascade_index = 3.0;
		if (camera_distance_m < split0) {
			cascade_index = 0.0;
		} else if (camera_distance_m < split1) {
			cascade_index = 1.0;
		} else if (camera_distance_m < split2) {
			cascade_index = 2.0;
		}
		vec3 cascade_color = vec3(0.20, 0.82, 0.28);
		if (cascade_index < 0.5) {
			cascade_color = vec3(0.20, 0.82, 0.28);
		} else if (cascade_index < 1.5) {
			cascade_color = vec3(0.12, 0.58, 1.00);
		} else if (cascade_index < 2.5) {
			cascade_color = vec3(1.00, 0.78, 0.16);
		} else {
			cascade_color = vec3(1.00, 0.20, 0.14);
		}
		vec3 facing_color = mix(vec3(0.04, 0.05, 0.08), scene_sun_color.rgb, sun_facing);
		debug_color = mix(facing_color, cascade_color, 0.38);
		debug_color *= mix(0.68, 1.12, clamp(scene_ambient_strength, 0.0, 1.0));
	} else {
		debug_color = vec3(slope);
	}
	float border = terrain_debug_patch_border(patch_uv);
	return mix(debug_color, vec3(1.0), border);
}

vec3 apply_baked_terrain_readability(
	vec3 color,
	float hillshade,
	float slope,
	float local_relief,
	float cliff_face,
	float shore_mask
) {
	if (!height_is_baked) {
		return color;
	}
	float slope_mask = smoothstep(0.004, 0.055, slope);
	float relief_mask = smoothstep(0.25, 3.50, local_relief);
	float feature_mask = max(slope_mask, relief_mask * 0.55);
	feature_mask *= clamp(1.0 - cliff_face * 0.80 - shore_mask * 0.35, 0.0, 1.0);
	float signed_shade = hillshade * 2.0 - 1.0;
	float shade = 1.0 + signed_shade * clamp(terrain_baked_readability_strength, 0.0, 0.30) * feature_mask;
	return color * clamp(shade, 0.86, 1.12);
}

float contour_mask(float world_elevation, float interval_m, float thickness) {
	float safe_interval = max(interval_m, 0.001);
	float scaled = world_elevation / safe_interval;
	float dist_to_line = abs(fract(scaled + 0.5) - 0.5);
	float aa_width = max(fwidth(scaled) * thickness, 0.0001);
	return 1.0 - smoothstep(0.0, aa_width, dist_to_line);
}

float slope_metric(vec2 uv) {
	return 1.0 - clamp(sample_world_normal(uv).y, 0.0, 1.0);
}

float cliff_candidate(vec2 uv, vec2 along_uv) {
	float slope_mask = smoothstep(cliff_slope_start, cliff_slope_end, slope_metric(uv));
	float relief_m = abs(sample_world_height(uv + along_uv) - sample_world_height(uv - along_uv));
	float relief_mask = smoothstep(cliff_relief_start_m, cliff_relief_end_m, relief_m);
	return slope_mask * relief_mask;
}

vec3 cliff_masks(vec2 uv) {
	vec2 gradient = sample_height_gradient(uv);
	float gradient_length = length(gradient);
	if (gradient_length <= 0.0001) {
		return vec3(0.0);
	}

	vec2 texel = 1.0 / max(heightmap_texture_size, vec2(1.0));
	vec2 upslope_dir = gradient / gradient_length;
	vec2 along_uv = upslope_dir * texel * cliff_sample_radius_texels;
	vec2 lateral_dir = vec2(-upslope_dir.y, upslope_dir.x);
	vec2 lateral_uv = lateral_dir * texel * cliff_lateral_smoothing_texels;
	float center = cliff_candidate(uv, along_uv);
	float left = cliff_candidate(uv + lateral_uv, along_uv);
	float right = cliff_candidate(uv - lateral_uv, along_uv);
	float uphill = cliff_candidate(uv + along_uv, along_uv);
	float uphill_left = cliff_candidate(uv + along_uv + lateral_uv, along_uv);
	float uphill_right = cliff_candidate(uv + along_uv - lateral_uv, along_uv);
	float downhill = cliff_candidate(uv - along_uv, along_uv);
	float downhill_left = cliff_candidate(uv - along_uv + lateral_uv, along_uv);
	float downhill_right = cliff_candidate(uv - along_uv - lateral_uv, along_uv);
	float face = center * 0.5 + left * 0.25 + right * 0.25;
	face = smoothstep(0.10, 0.82, face);
	float uphill_avg = (uphill + uphill_left + uphill_right) / 3.0;
	float downhill_avg = (downhill + downhill_left + downhill_right) / 3.0;
	float top_edge = clamp(face - uphill_avg, 0.0, 1.0);
	float toe_edge = clamp(face - downhill_avg, 0.0, 1.0);
	return vec3(face, top_edge, toe_edge);
}

#include "res://scripts/shaders/ground_paving.gdshaderinc"

void vertex() {
	vec2 safe_inner_size = max(inner_sample_size_texels, vec2(1.0));
	vec2 patch_uv = clamp(
		vec2(
			VERTEX.x / max(patch_world_size_m.x, 0.001) + 0.5,
			VERTEX.z / max(patch_world_size_m.y, 0.001) + 0.5
		),
		vec2(0.0),
		vec2(1.0)
	);
	vec2 centered_uv = (inner_sample_offset_texels + patch_uv * (safe_inner_size - 1.0) + 0.5)
		/ max(heightmap_texture_size, vec2(1.0));
	var_centered_uv = centered_uv;
	var_water_uv = watermap_centered_uv(patch_uv);
	h = texture(heightmap, centered_uv).r;
	if (height_is_baked) {
		h = VERTEX.y / max(height_scale, 0.001);
	} else {
		VERTEX.y += h * height_scale;
	}
	vec3 terrain_normal = sample_world_normal(centered_uv);
	NORMAL = terrain_normal;
	var_world_pos = (MODEL_MATRIX * vec4(VERTEX, 1.0)).xyz;
	var_view_distance_m = length((MODELVIEW_MATRIX * vec4(VERTEX, 1.0)).xyz);
}

varying float paved_surface;

void fragment() {
	paved_surface = float(height_is_baked && COLOR.b < 0.5 && terrain_visual_debug_mode == 0 && terrain_grass_visual_debug_mode == 0);
	if (paved_surface > 0.5) {
		// Graded frontage is paving: use the same texture, normal basis and lighting as yards.
		IRRADIANCE = vec4(0.0);
		RADIANCE = vec4(0.0);
		TANGENT = paving_tangent(NORMAL, VIEW_MATRIX);
		BINORMAL = normalize(cross(TANGENT, NORMAL));

		PavingSample paving;
		if (COLOR.r > 0.5) {
			paving = sample_paving(
				site_asphalt_albedo_tex, site_asphalt_normal_tex, site_asphalt_roughness_tex,
				var_world_pos.xz, site_asphalt_uv_scale, site_asphalt_macro_uv_scale,
				site_asphalt_macro_influence, site_asphalt_brightness,
				site_asphalt_albedo_floor, site_asphalt_floor_influence
			);
		} else {
			paving = sample_paving(
				site_concrete_albedo_tex, site_concrete_normal_tex, site_concrete_roughness_tex,
				var_world_pos.xz, site_concrete_uv_scale, site_concrete_macro_uv_scale,
				site_concrete_macro_influence, site_concrete_brightness,
				site_concrete_albedo_floor, site_concrete_floor_influence
			);
		}
		ALBEDO = paving.albedo;
		NORMAL_MAP = paving.normal_map;
		ROUGHNESS = paving.roughness;
		METALLIC = 0.0;
		SPECULAR = PAVING_SPECULAR;
	} else {
		// Preserve the authored sky floor without disabling window light spill (SSIL).
		IRRADIANCE = vec4(0.0, 0.0, 0.0, 1.0);
		RADIANCE = vec4(0.0, 0.0, 0.0, 1.0);
		// Rust publishes accepted crown coverage. One bilinear fetch; no stand-field mirror.
		vec2 cover_uv = (var_world_pos.xz - land_cover_world_bounds.xy)
			/ max(land_cover_world_bounds.zw, vec2(0.001));
		float land_cover = texture(land_cover_texture, cover_uv).r;
		float world_elevation = h * height_scale;
		float macro_mask = macro_variation(var_world_pos.xz);
		float local_relief = sample_local_relief(var_centered_uv);
		vec3 normal = sample_world_normal(var_centered_uv);
		float slope = 1.0 - clamp(normal.y, 0.0, 1.0);
		float ruggedness = smoothstep(terrain_relief_start_m, terrain_relief_end_m, local_relief);
		ruggedness = clamp(max(ruggedness, slope * 0.72), 0.0, 1.0);
		vec3 color = terrain_surface_color(world_elevation, macro_mask, ruggedness);
		vec3 cliff = cliff_masks(var_centered_uv);
		float cliff_face = cliff.x;
		float cliff_top = cliff.y;
		float cliff_toe = cliff.z;
		float rock_mask = smoothstep(terrain_rock_slope_start, terrain_rock_slope_end, slope);
		rock_mask *= mix(0.75, 1.0, ruggedness);
		rock_mask = max(rock_mask, cliff_face * 0.92);
		vec3 rock_color = mix(vec3(0.31, 0.29, 0.25), vec3(0.46, 0.45, 0.43), macro_mask);
		color = mix(color, rock_color, rock_mask);
		float shore_mask = shoreline_mask(var_water_uv);
		float shore_flatness = 1.0 - smoothstep(terrain_rock_slope_start, terrain_rock_slope_end + 0.10, slope);
		vec3 shore_color = mix(vec3(0.28, 0.39, 0.23), vec3(0.18, 0.32, 0.18), ruggedness);
		color = mix(color, shore_color, shore_mask * shore_flatness * terrain_shore_blend_strength);
		color = apply_natural_land_variation(
			color,
			var_world_pos.xz,
			slope,
			ruggedness,
			rock_mask,
			cliff_face,
			shore_mask * shore_flatness
		);
		float grass_detail_mask = (1.0 - rock_mask) * (1.0 - cliff_face * 0.80);
		grass_detail_mask *= 1.0 - shore_mask * shore_flatness * terrain_shore_blend_strength;
		if (terrain_visual_debug_mode > 0) {
			vec3 debug_color = terrain_visual_debug_color(
				UV,
				h,
				world_elevation,
				local_relief,
				slope,
				shore_mask,
				clamp(grass_detail_mask, 0.0, 1.0),
				normal
			);
			if (terrain_visual_debug_mode == 10) {
				ALBEDO = debug_color;
			} else {
				ALBEDO = vec3(0.0);
				EMISSION = debug_color;
			}
			ROUGHNESS = 1.0;
			SPECULAR = PAVING_SPECULAR;
		} else if (terrain_grass_visual_debug_mode > 0) {
			ALBEDO = vec3(0.0);
			EMISSION = terrain_grass_debug_color(
				color,
				var_world_pos,
				clamp(grass_detail_mask, 0.0, 1.0)
			);
			ROUGHNESS = 1.0;
			SPECULAR = PAVING_SPECULAR;
		} else {
			color = apply_grass_detail(color, var_world_pos, clamp(grass_detail_mask, 0.0, 1.0));
			// Litter, moss and duff replace grass hue at the same albedo luminance. Inside shadow
			// range the trees themselves supply every bit of the darkness, so coverage changes
			// hue only and never adds any of its own.
			vec3 forest_floor = mix(vec3(0.46, 0.40, 0.28), vec3(0.38, 0.45, 0.30), macro_mask);
			vec3 luminance_weights = vec3(0.2126, 0.7152, 0.0722);
			forest_floor *= dot(color, luminance_weights) / dot(forest_floor, luminance_weights);
			color = mix(color, forest_floor, land_cover);
			// Past the cascades there are no tree shadows left to supply it, and the floor of a
			// closed stand returned to open-meadow brightness while the crowns standing on it kept
			// their own replacement term. That is the pale far field: lit field with dark specks,
			// where the near side of the same stand is closed and dark. The ramp fades in exactly
			// where the cascades fade out, so nothing inside shadow range changes, and it scales
			// with accepted crown coverage, so a lone tree on a meadow darkens nearly nothing and
			// only ground a crown actually stands over goes dark. Coverage comes from the accepted
			// population and never from the stand field.
			float floor_shade_ramp = smoothstep(
				canopy_floor_shade_begin_m,
				canopy_floor_shade_end_m,
				var_view_distance_m
			) * canopy_floor_shade;
			float floor_shade = mix(1.0, canopy_floor_shade_floor, land_cover);
			color *= clamp(mix(1.0, floor_shade, floor_shade_ramp), 0.0, 1.0);
			vec2 world_uv = clamp(
				vec2(
					var_world_pos.x / max(world_size.x, 0.001),
					var_world_pos.z / max(world_size.y, 0.001)
				) + vec2(0.5),
				vec2(0.0),
				vec2(1.0)
			);
			float coal_pit_mask = 0.0;
			if (coal_pit_overlay_world_bounds.z > 0.001 && coal_pit_overlay_world_bounds.w > 0.001) {
				vec2 coal_pit_uv = (var_world_pos.xz - coal_pit_overlay_world_bounds.xy) / coal_pit_overlay_world_bounds.zw;
				if (
					coal_pit_uv.x >= 0.0
					&& coal_pit_uv.x <= 1.0
					&& coal_pit_uv.y >= 0.0
					&& coal_pit_uv.y <= 1.0
				) {
					coal_pit_mask = texture(coal_pit_texture, coal_pit_uv).r;
				}
			}
			if (coal_pit_mask > 0.001) {
				vec3 coal_albedo = texture(terrain_coal_albedo, var_world_pos.xz * terrain_coal_detail_scale).rgb;
				coal_albedo = mix(coal_albedo, max(coal_albedo, terrain_coal_lift_color), terrain_coal_lift_strength);
				coal_albedo = mix(coal_albedo, coal_albedo * vec3(0.82, 0.80, 0.76), macro_mask * 0.18);
				color = mix(color, coal_albedo, coal_pit_mask * terrain_coal_pit_strength);
			}
			float field_mask = 0.0;
			if (field_overlay_world_bounds.z > 0.001 && field_overlay_world_bounds.w > 0.001) {
				vec2 field_uv = (var_world_pos.xz - field_overlay_world_bounds.xy) / field_overlay_world_bounds.zw;
				if (
					field_uv.x >= 0.0
					&& field_uv.x <= 1.0
					&& field_uv.y >= 0.0
					&& field_uv.y <= 1.0
				) {
					field_mask = texture(field_overlay_texture, field_uv).r;
				}
			}
			if (field_mask > 0.001) {
				vec3 grain_albedo = terrain_grain_field_color(var_world_pos.xz);
				color = mix(color, grain_albedo, field_mask * terrain_field_strength);
			}
			float hillshade = hillshade_mask(normal);
			color = apply_baked_terrain_readability(
				color,
				hillshade,
				slope,
				local_relief,
				cliff_face,
				shore_mask * shore_flatness
			);
			color = mix(color, cliff_face_color, cliff_face * cliff_face_strength);
			vec3 hillshade_tint = mix(hillshade_shadow_tint, hillshade_light_tint, hillshade);
			color *= mix(vec3(1.0), hillshade_tint, hillshade_strength);

			// Overlay Heatmaps (Pollution, Noise, Desirability) and authored resource deposits.
			if (overlay_mode > 0) {
				vec4 heatmap_col = texture(overlay_texture, world_uv);
				if (heatmap_col.a > 0.0) {
					if (overlay_mode == 4) {
						float richness = clamp(heatmap_col.a, 0.0, 1.0);
						vec3 coal_color = mix(vec3(0.16, 0.135, 0.105), vec3(0.025, 0.023, 0.021), richness);
						color = mix(color, coal_color, richness * 0.78);
					} else {
						color = mix(color, heatmap_col.rgb, heatmap_col.a * 0.6);
					}
				}
			}

			float major_contour = contour_mask(
				world_elevation,
				contour_major_interval_m,
				contour_major_thickness
			);
			float minor_contour = contour_mask(
				world_elevation,
				contour_minor_interval_m,
				contour_minor_thickness
			) * (1.0 - major_contour);
			float zero_elevation_mask = 1.0 - smoothstep(
				0.0,
				max(contour_zero_elevation_fade_m, 0.001),
				abs(world_elevation)
			);
			float flat_relief_mask = smoothstep(
				contour_flat_relief_start_m,
				max(contour_flat_relief_end_m, contour_flat_relief_start_m + 0.001),
				local_relief
			);
			float flat_zero_contour_visibility = mix(1.0, flat_relief_mask, zero_elevation_mask);
			float contour_visibility = (1.0 - (cliff_face * cliff_contour_fade)) * flat_zero_contour_visibility;
			// Contours are a map affordance for reading slope while planning, not a property of
			// ground. Drawn in the plain world view they put survey lines across every hillside,
			// which is the single strongest "this is a topographic map" cue in an aerial shot.
			// The analysis overlays are read as a map and keep them. Anything else that wants
			// them - terrain sculpting, say - clears contour_map_view_only rather than editing
			// this; every interval and colour uniform is unchanged.
			if (contour_map_view_only && overlay_mode <= 0) {
				contour_visibility = 0.0;
			}
			float contour_relief_boost = max(
				smoothstep(0.30, 2.80, local_relief),
				smoothstep(0.010, 0.075, slope) * 0.70
			);
			float minor_strength = contour_minor_strength + contour_relief_minor_boost_strength * contour_relief_boost;
			color = mix(color, contour_minor_color, minor_contour * minor_strength * contour_visibility);
			color = mix(color, contour_major_color, major_contour * contour_major_strength * contour_visibility);
			color = mix(color, cliff_top_edge_color, cliff_top * cliff_edge_strength);
			color = mix(color, cliff_toe_edge_color, cliff_toe * cliff_edge_strength);
			// Desaturated before it is either shown or lit, so the moonlit diffuse washes out
			// with the ambient floor rather than staying vivid against a grey ground.
			color = mix(color, vec3(dot(color, vec3(0.2126, 0.7152, 0.0722))), scene_ambient_desaturation);
			ALBEDO = color;
			EMISSION = color * clamp(ground_shadow_ambient, 0.0, 1.0) * scene_ambient_light.rgb;
			ROUGHNESS = mix(mix(0.96, 0.76, rock_mask), 0.90, cliff_face * 0.70);
			SPECULAR = 0.08;
		}
	}
}

void light() {
	if (paved_surface > 0.5) {
		DIFFUSE_LIGHT += paving_diffuse(NORMAL, LIGHT, LIGHT_COLOR, ATTENUATION);
	} else if (terrain_visual_debug_mode == 0 || terrain_visual_debug_mode == 10) {
		float sun_strength = ground_shadow_sun_strength;
		if (terrain_visual_debug_mode == 10) {
			sun_strength = 1.0;
		}
		float shadow_visibility = mix(
			clamp(ground_shadow_min_visibility, 0.0, 1.0),
			1.0,
			clamp(ATTENUATION, 0.0, 1.0)
		);
		float facing = clamp(dot(NORMAL, LIGHT), 0.0, 1.0);
		float soft_facing = mix(0.82, 1.0, facing);
		DIFFUSE_LIGHT += ALBEDO
			* LIGHT_COLOR
			* sun_strength
			* shadow_visibility
			* soft_facing;
	}
}
