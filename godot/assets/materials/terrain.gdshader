shader_type spatial;
render_mode cull_disabled, unshaded;

uniform sampler2D heightmap;
uniform sampler2D overlay_texture;
uniform sampler2D watermap : repeat_disable, filter_linear;
uniform sampler2D terrain_grass_albedo : source_color, filter_linear_mipmap_anisotropic, repeat_enable;
uniform sampler2D terrain_grass_height : hint_default_black, filter_linear_mipmap_anisotropic, repeat_enable;
uniform int overlay_mode = 0;
uniform float height_scale = 10.0;
uniform bool height_is_baked = false;
uniform vec2 world_size = vec2(256.0, 256.0);
uniform vec2 heightmap_texture_size = vec2(256.0, 256.0);
uniform vec2 inner_sample_offset_texels = vec2(1.0, 1.0);
uniform vec2 inner_sample_size_texels = vec2(2.0, 2.0);
uniform float terrain_cell_m = 10.0;
uniform float hillshade_azimuth_deg = 315.0;
uniform float hillshade_altitude_deg = 42.0;
uniform float hillshade_strength = 0.42;
uniform float hillshade_ambient = 0.36;
uniform float hillshade_contrast = 1.15;
uniform vec3 hillshade_shadow_tint : source_color = vec3(0.62, 0.71, 0.77);
uniform vec3 hillshade_light_tint : source_color = vec3(0.97, 0.99, 0.95);
uniform float terrain_macro_variation_strength = 0.10;
uniform vec3 terrain_grass_tint : source_color = vec3(0.22, 0.42, 0.16);
uniform float terrain_grass_tint_strength = 0.0;
uniform float terrain_natural_variation_strength = 0.18;
uniform float terrain_meadow_mottle_strength = 0.08;
uniform float terrain_baked_normal_blend = 0.75;
uniform float terrain_baked_readability_strength = 0.12;
uniform float terrain_grass_detail_scale = 0.12;
uniform float terrain_grass_detail_strength = 0.26;
uniform float terrain_grass_height_detail_strength = 0.10;
uniform float terrain_grass_detail_fade_start = 0.035;
uniform float terrain_grass_detail_fade_end = 0.18;
uniform float terrain_rock_slope_start = 0.15;
uniform float terrain_rock_slope_end = 0.34;
uniform float terrain_relief_sample_radius_texels = 3.0;
uniform float terrain_relief_start_m = 2.0;
uniform float terrain_relief_end_m = 16.0;
uniform float terrain_shore_blend_strength = 0.52;
uniform float terrain_shore_lookup_radius_texels = 2.0;
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
uniform float contour_zero_elevation_fade_m = 0.75;
uniform float contour_flat_relief_start_m = 0.10;
uniform float contour_flat_relief_end_m = 1.25;

varying float h;
varying vec2 var_centered_uv;
varying vec3 var_world_pos;
varying vec3 var_mesh_normal;

float hash12(vec2 p) {
	vec3 p3 = fract(vec3(p.xyx) * 0.1031);
	p3 += dot(p3, p3.yzx + 33.33);
	return fract((p3.x + p3.y) * p3.z);
}

float noise2d(vec2 p) {
	vec2 i = floor(p);
	vec2 f = fract(p);
	float a = hash12(i);
	float b = hash12(i + vec2(1.0, 0.0));
	float c = hash12(i + vec2(0.0, 1.0));
	float d = hash12(i + vec2(1.0, 1.0));
	vec2 u = f * f * (3.0 - 2.0 * f);
	return mix(mix(a, b, u.x), mix(c, d, u.x), u.y);
}

float macro_variation(vec2 world_xz) {
	float a = noise2d(world_xz * 0.0014 + vec2(17.0, 3.0));
	float b = noise2d(world_xz * 0.0045 + vec2(41.0, 29.0));
	return clamp(a * 0.7 + b * 0.3, 0.0, 1.0);
}

float warped_noise(vec2 world_xz, float scale, vec2 offset) {
	vec2 warp = vec2(
		noise2d(world_xz * 0.0007 + offset),
		noise2d(world_xz * 0.0007 + offset + vec2(37.0, 19.0))
	) - vec2(0.5);
	return noise2d((world_xz + warp * 180.0) * scale + offset * 0.37);
}

vec3 apply_natural_land_variation(
	vec3 color,
	vec2 world_xz,
	float slope,
	float ruggedness,
	float rock_mask,
	float cliff_face,
	float shore_mask
) {
	float broad = warped_noise(world_xz, 0.0010, vec2(5.0, 71.0));
	float mid = warped_noise(world_xz, 0.0036, vec2(23.0, 11.0));
	float mottle = noise2d(world_xz * 0.018 + vec2(59.0, 7.0));
	float land_mask = clamp((1.0 - rock_mask) * (1.0 - cliff_face) * (1.0 - shore_mask * 0.65), 0.0, 1.0);
	float open_mask = smoothstep(0.54, 0.86, broad) * (1.0 - ruggedness * 0.45);
	float cool_mask = smoothstep(0.18, 0.72, 1.0 - broad) * smoothstep(0.32, 0.78, mid);
	float scrub_mask = smoothstep(0.45, 0.90, mid) * smoothstep(0.02, 0.22, slope + ruggedness * 0.25);
	vec3 dry_open_tint = vec3(1.08, 1.05, 0.90);
	vec3 cool_meadow_tint = vec3(0.84, 1.04, 0.86);
	vec3 scrub_tint = vec3(0.88, 0.96, 0.78);
	float strength = clamp(terrain_natural_variation_strength, 0.0, 1.0) * land_mask;
	color = mix(color, color * dry_open_tint, open_mask * strength);
	color = mix(color, color * cool_meadow_tint, cool_mask * strength * 0.85);
	color = mix(color, color * scrub_tint, scrub_mask * strength * 0.70);
	color *= mix(1.0, mix(0.94, 1.06, mottle), clamp(terrain_meadow_mottle_strength, 0.0, 1.0) * land_mask);
	return color;
}

vec3 terrain_surface_color(float world_elevation, float macro_mask, float ruggedness) {
	vec3 dry_lowland = mix(vec3(0.36, 0.49, 0.29), vec3(0.28, 0.41, 0.24), macro_mask);
	vec3 inland_base = mix(vec3(0.23, 0.38, 0.18), vec3(0.18, 0.32, 0.15), macro_mask);
	vec3 dry_open = mix(vec3(0.38, 0.38, 0.25), vec3(0.32, 0.34, 0.22), macro_mask);
	vec3 conifer_dark = mix(vec3(0.11, 0.21, 0.10), vec3(0.15, 0.27, 0.13), macro_mask);
	vec3 upland = mix(vec3(0.30, 0.31, 0.23), vec3(0.36, 0.34, 0.25), macro_mask);
	vec3 snow = vec3(0.93, 0.94, 0.96);

	float lowland_bias = 1.0 - smoothstep(-60.0, 120.0, world_elevation);
	float upland_bias = smoothstep(260.0, 900.0, world_elevation);
	float alpine_bias = smoothstep(1500.0, 2200.0, world_elevation);

	// Dry terrain at elevation 0 is still land. Water color only comes from the
	// water renderer, so blank/test maps must not look like a flooded slab.
	vec3 color = mix(inland_base, dry_lowland, lowland_bias * 0.82);
	color = mix(color, dry_open, lowland_bias * 0.10);
	color = mix(color, conifer_dark, ruggedness * 0.34);
	color = mix(color, upland, upland_bias * 0.46);
	color = mix(color, snow, alpine_bias);
	vec3 macro_tint = mix(vec3(1.03, 1.01, 0.97), vec3(0.92, 1.01, 0.95), macro_mask);
	color *= mix(vec3(1.0), macro_tint, terrain_macro_variation_strength);
	float grass_tint_mask = 1.0 - alpine_bias;
	color = mix(
		color,
		terrain_grass_tint,
		clamp(terrain_grass_tint_strength, 0.0, 1.0) * grass_tint_mask
	);
	return color;
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

vec3 terrain_display_normal(vec2 uv) {
	vec3 height_normal = sample_world_normal(uv);
	if (!height_is_baked) {
		return height_normal;
	}
	return normalize(mix(
		height_normal,
		normalize(var_mesh_normal),
		clamp(terrain_baked_normal_blend, 0.0, 1.0)
	));
}

vec3 apply_grass_detail(vec3 color, vec3 world_pos, float grass_mask) {
	vec2 detail_uv = world_pos.xz * terrain_grass_detail_scale;
	vec2 cross_uv = mat2(vec2(0.80, 0.60), vec2(-0.60, 0.80))
		* world_pos.xz
		* (terrain_grass_detail_scale * 0.73)
		+ vec2(31.7, 11.3);
	vec3 grass_albedo = texture(terrain_grass_albedo, detail_uv).rgb;
	vec3 cross_albedo = texture(terrain_grass_albedo, cross_uv).rgb;
	float albedo_luma = mix(
		dot(grass_albedo, vec3(0.299, 0.587, 0.114)),
		dot(cross_albedo, vec3(0.299, 0.587, 0.114)),
		0.35
	);
	float albedo_detail = clamp((albedo_luma - 0.5) * 1.75 + 0.5, 0.0, 1.0);
	float height_detail = texture(terrain_grass_height, detail_uv).r;
	float footprint = max(length(fwidth(detail_uv)), length(fwidth(cross_uv)));
	float fade_start = max(terrain_grass_detail_fade_start, 0.0);
	float fade_end = max(terrain_grass_detail_fade_end, fade_start + 0.001);
	float screen_fade = 1.0 - smoothstep(fade_start, fade_end, footprint);
	float brightness = 1.0;
	brightness += (albedo_detail - 0.5) * terrain_grass_detail_strength * 0.68;
	brightness += (height_detail - 0.5) * terrain_grass_height_detail_strength * 2.0;
	brightness = max(brightness, 0.35);
	return color * mix(1.0, brightness, grass_mask * screen_fade);
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
	vec2 texel = terrain_shore_lookup_radius_texels / max(heightmap_texture_size, vec2(1.0));
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

float hillshade_mask(vec3 normal) {
	float azimuth = radians(hillshade_azimuth_deg);
	float altitude = radians(hillshade_altitude_deg);
	vec3 light_dir = normalize(vec3(
		sin(azimuth) * cos(altitude),
		sin(altitude),
		-cos(azimuth) * cos(altitude)
	));

	float diffuse = clamp(dot(normal, light_dir), 0.0, 1.0);
	float shaped = pow(diffuse, hillshade_contrast);
	return clamp(mix(hillshade_ambient, 1.0, shaped), 0.0, 1.0);
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

void vertex() {
	vec2 safe_inner_size = max(inner_sample_size_texels, vec2(1.0));
	vec2 centered_uv = (inner_sample_offset_texels + UV * (safe_inner_size - 1.0) + 0.5)
		/ max(heightmap_texture_size, vec2(1.0));
	var_centered_uv = centered_uv;
	h = texture(heightmap, centered_uv).r;
	if (height_is_baked) {
		h = VERTEX.y / max(height_scale, 0.001);
	} else {
		VERTEX.y += h * height_scale;
	}
	var_world_pos = (MODEL_MATRIX * vec4(VERTEX, 1.0)).xyz;
	var_mesh_normal = normalize(NORMAL);
}

void fragment() {
	float world_elevation = h * height_scale;
	float macro_mask = macro_variation(var_world_pos.xz);
	float local_relief = sample_local_relief(var_centered_uv);
	vec3 normal = terrain_display_normal(var_centered_uv);
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
	float shore_mask = shoreline_mask(var_centered_uv);
	float shore_flatness = 1.0 - smoothstep(terrain_rock_slope_start, terrain_rock_slope_end + 0.10, slope);
	vec3 shore_color = mix(vec3(0.34, 0.33, 0.24), vec3(0.18, 0.28, 0.16), ruggedness);
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
	color = apply_grass_detail(color, var_world_pos, clamp(grass_detail_mask, 0.0, 1.0));
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
	
	// Overlay Heatmaps (Pollution, Noise, Desirability)
	if (overlay_mode > 0) {
		vec2 world_uv = clamp(
			vec2(
				var_world_pos.x / max(world_size.x, 0.001),
				var_world_pos.z / max(world_size.y, 0.001)
			) + vec2(0.5),
			vec2(0.0),
			vec2(1.0)
		);
		vec4 heatmap_col = texture(overlay_texture, world_uv);
		if (heatmap_col.a > 0.0) {
			color = mix(color, heatmap_col.rgb, 0.6);
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
	float contour_relief_boost = max(
		smoothstep(0.30, 2.80, local_relief),
		smoothstep(0.010, 0.075, slope) * 0.70
	);
	float minor_strength = contour_minor_strength + contour_relief_minor_boost_strength * contour_relief_boost;
	color = mix(color, contour_minor_color, minor_contour * minor_strength * contour_visibility);
	color = mix(color, contour_major_color, major_contour * contour_major_strength * contour_visibility);
	color = mix(color, cliff_top_edge_color, cliff_top * cliff_edge_strength);
	color = mix(color, cliff_toe_edge_color, cliff_toe * cliff_edge_strength);
	ALBEDO = color;
	ROUGHNESS = mix(mix(0.96, 0.76, rock_mask), 0.90, cliff_face * 0.70);
	SPECULAR = 0.08;
}
