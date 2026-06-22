shader_type spatial;
render_mode cull_disabled, ambient_light_disabled;

uniform sampler2D terrain_grass_albedo : source_color, filter_linear_mipmap_anisotropic, repeat_enable;
uniform sampler2D terrain_grass_height : hint_default_black, filter_linear_mipmap_anisotropic, repeat_enable;

uniform vec3 scene_sun_direction = vec3(-0.46, 0.74, -0.49);
uniform float ground_shadow_ambient = 0.64;
uniform float ground_shadow_sun_strength = 0.36;
uniform float ground_shadow_min_visibility = 0.10;
uniform float hillshade_azimuth_deg = 315.0;
uniform float hillshade_altitude_deg = 38.0;
uniform float hillshade_strength = 0.18;
uniform float hillshade_ambient = 0.70;
uniform float hillshade_contrast = 1.05;
uniform vec3 hillshade_shadow_tint : source_color = vec3(0.84, 0.90, 0.88);
uniform vec3 hillshade_light_tint : source_color = vec3(1.00, 0.99, 0.94);

uniform float terrain_macro_variation_strength = 0.10;
uniform vec3 terrain_grass_tint : source_color = vec3(0.22, 0.42, 0.16);
uniform float terrain_grass_tint_strength = 0.0;
uniform float terrain_grass_albedo_strength = 0.90;
uniform float terrain_grass_macro_scale = 0.018;
uniform float terrain_grass_mid_scale = 0.065;
uniform float terrain_grass_macro_strength = 0.58;
uniform float terrain_grass_mid_strength = 0.80;
uniform float terrain_grass_micro_strength = 0.50;
uniform float terrain_natural_variation_strength = 0.18;
uniform float terrain_meadow_mottle_strength = 0.08;
uniform float terrain_grass_detail_scale = 0.34;
uniform float terrain_grass_detail_strength = 0.58;
uniform float terrain_grass_height_detail_strength = 0.24;
uniform float terrain_grass_detail_fade_start = 0.08;
uniform float terrain_grass_detail_fade_end = 0.90;

varying vec3 world_pos;
varying vec3 world_normal;

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

vec2 hash22(vec2 p) {
	return vec2(
		hash12(p),
		hash12(p + vec2(19.19, 73.31))
	);
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

vec3 terrain_surface_color(float world_elevation, float macro_mask) {
	vec3 dry_lowland = mix(vec3(0.36, 0.49, 0.29), vec3(0.28, 0.41, 0.24), macro_mask);
	vec3 inland_base = mix(vec3(0.23, 0.38, 0.18), vec3(0.18, 0.32, 0.15), macro_mask);
	vec3 dry_open = mix(vec3(0.38, 0.38, 0.25), vec3(0.32, 0.34, 0.22), macro_mask);

	float lowland_bias = 1.0 - smoothstep(-60.0, 120.0, world_elevation);
	vec3 color = mix(inland_base, dry_lowland, lowland_bias * 0.82);
	color = mix(color, dry_open, lowland_bias * 0.10);
	vec3 macro_tint = mix(vec3(1.03, 1.01, 0.97), vec3(0.92, 1.01, 0.95), macro_mask);
	color *= mix(vec3(1.0), macro_tint, terrain_macro_variation_strength);
	return mix(
		color,
		terrain_grass_tint,
		clamp(terrain_grass_tint_strength, 0.0, 1.0)
	);
}

vec3 apply_natural_land_variation(vec3 color, vec2 world_xz) {
	float broad = warped_noise(world_xz, 0.0010, vec2(5.0, 71.0));
	float mid = warped_noise(world_xz, 0.0036, vec2(23.0, 11.0));
	float mottle = noise2d(world_xz * 0.018 + vec2(59.0, 7.0));
	float open_mask = smoothstep(0.54, 0.86, broad);
	float cool_mask = smoothstep(0.18, 0.72, 1.0 - broad) * smoothstep(0.32, 0.78, mid);
	float scrub_mask = smoothstep(0.45, 0.90, mid);
	vec3 dry_open_tint = vec3(1.08, 1.05, 0.90);
	vec3 cool_meadow_tint = vec3(0.84, 1.04, 0.86);
	vec3 scrub_tint = vec3(0.88, 0.96, 0.78);
	float strength = clamp(terrain_natural_variation_strength, 0.0, 1.0);
	color = mix(color, color * dry_open_tint, open_mask * strength);
	color = mix(color, color * cool_meadow_tint, cool_mask * strength * 0.85);
	color = mix(color, color * scrub_tint, scrub_mask * strength * 0.20);
	color *= mix(1.0, mix(0.94, 1.06, mottle), clamp(terrain_meadow_mottle_strength, 0.0, 1.0));
	return color;
}

vec2 stochastic_grass_offset(vec2 cell, float salt) {
	return hash22(cell + vec2(salt, salt * 1.61803398875)) * 43.0;
}

vec3 sample_grass_albedo_stochastic(vec2 uv, float salt) {
	vec2 cell = floor(uv);
	vec2 local = fract(uv);
	vec2 blend = local * local * (3.0 - 2.0 * local);
	vec2 offset_00 = stochastic_grass_offset(cell, salt);
	vec2 offset_10 = stochastic_grass_offset(cell + vec2(1.0, 0.0), salt);
	vec2 offset_01 = stochastic_grass_offset(cell + vec2(0.0, 1.0), salt);
	vec2 offset_11 = stochastic_grass_offset(cell + vec2(1.0, 1.0), salt);
	vec3 sample_00 = texture(terrain_grass_albedo, uv + offset_00).rgb;
	vec3 sample_10 = texture(terrain_grass_albedo, uv + offset_10).rgb;
	vec3 sample_01 = texture(terrain_grass_albedo, uv + offset_01).rgb;
	vec3 sample_11 = texture(terrain_grass_albedo, uv + offset_11).rgb;
	return mix(
		mix(sample_00, sample_10, blend.x),
		mix(sample_01, sample_11, blend.x),
		blend.y
	);
}

vec3 sample_grass_albedo_cross(vec2 world_xz, float scale) {
	vec2 detail_uv = world_xz * scale;
	vec2 cross_uv = mat2(vec2(0.80, 0.60), vec2(-0.60, 0.80))
		* world_xz
		* (scale * 0.71)
		+ vec2(31.7, 11.3);
	vec3 detail = texture(terrain_grass_albedo, detail_uv).rgb;
	vec3 cross = texture(terrain_grass_albedo, cross_uv).rgb;
	return mix(detail, cross, 0.35);
}

float sample_grass_height_cross(vec2 world_xz, float scale) {
	vec2 detail_uv = world_xz * scale;
	vec2 cross_uv = mat2(vec2(0.80, 0.60), vec2(-0.60, 0.80))
		* world_xz
		* (scale * 0.71)
		+ vec2(31.7, 11.3);
	float detail = texture(terrain_grass_height, detail_uv).r;
	float cross = texture(terrain_grass_height, cross_uv).r;
	return mix(detail, cross, 0.35);
}

vec3 grass_meadow_variation(vec2 world_xz) {
	float broad = warped_noise(world_xz, 0.006, vec2(83.0, 17.0));
	float mid = warped_noise(world_xz, 0.026, vec2(11.0, 131.0));
	float fine = noise2d(world_xz * 0.085 + vec2(47.0, 5.0));
	vec3 damp_green = vec3(0.15, 0.27, 0.10);
	vec3 lush_green = vec3(0.30, 0.47, 0.20);
	vec3 dry_grass = vec3(0.39, 0.43, 0.22);
	vec3 meadow = mix(damp_green, lush_green, smoothstep(0.20, 0.86, broad));
	meadow = mix(meadow, dry_grass, smoothstep(0.64, 0.94, mid) * 0.35);
	meadow *= mix(0.88, 1.16, mid) * mix(0.94, 1.06, fine);
	return clamp(meadow, vec3(0.035), vec3(0.52, 0.70, 0.42));
}

vec3 grass_material_layer(vec3 base_color, vec3 grass_color, vec2 world_xz, float contrast, float green_anchor) {
	vec3 luma_weights = vec3(0.299, 0.587, 0.114);
	float grass_luma = max(dot(grass_color, luma_weights), 0.001);
	float detail = clamp((grass_luma - 0.28) * contrast, -0.32, 0.30);
	vec3 grass_contrast = clamp(
		(grass_color - vec3(grass_luma)) * 3.00 + vec3(grass_luma),
		vec3(0.0),
		vec3(1.0)
	);
	vec3 meadow_color = grass_meadow_variation(world_xz);
	vec3 anchored_color = mix(base_color, meadow_color, clamp(green_anchor, 0.0, 1.0));
	float texture_weight = clamp(0.50 + contrast * 0.20, 0.58, 0.82);
	vec3 textured_color = mix(anchored_color, grass_contrast, texture_weight);
	return clamp(textured_color * (1.0 + detail), vec3(0.030), vec3(0.54, 0.72, 0.44));
}

float grass_detail_brightness(vec3 grass_color, float height_detail, vec2 world_xz) {
	float albedo_luma = dot(grass_color, vec3(0.299, 0.587, 0.114));
	float albedo_detail = clamp((albedo_luma - 0.27) * 4.25, -0.45, 0.45);
	float height_relief = clamp((height_detail - 0.34) * 2.20, -0.55, 0.65);
	float fiber = noise2d(world_xz * 0.92 + vec2(5.0, 97.0)) - 0.5;
	float tuft = warped_noise(world_xz, 0.22, vec2(191.0, 41.0)) - 0.5;
	float brightness = 1.0;
	brightness += albedo_detail * terrain_grass_detail_strength;
	brightness += height_relief * terrain_grass_height_detail_strength;
	brightness += fiber * 0.08 + tuft * 0.10;
	return clamp(brightness, 0.70, 1.34);
}

vec3 apply_grass_detail(vec3 color, vec3 pos, float grass_mask) {
	vec2 micro_uv = pos.xz * terrain_grass_detail_scale;
	vec2 micro_cross_uv = mat2(vec2(0.80, 0.60), vec2(-0.60, 0.80))
		* pos.xz
		* (terrain_grass_detail_scale * 0.71)
		+ vec2(31.7, 11.3);
	vec3 macro_grass = sample_grass_albedo_stochastic(pos.xz * terrain_grass_macro_scale, 11.0);
	vec3 mid_grass = sample_grass_albedo_stochastic(pos.xz * terrain_grass_mid_scale, 29.0);
	vec3 micro_grass = sample_grass_albedo_cross(pos.xz, terrain_grass_detail_scale);
	float height_detail = sample_grass_height_cross(pos.xz, terrain_grass_detail_scale);
	float footprint = max(length(fwidth(micro_uv)), length(fwidth(micro_cross_uv)));
	float fade_start = max(terrain_grass_detail_fade_start, 0.0);
	float fade_end = max(terrain_grass_detail_fade_end, fade_start + 0.001);
	float micro_visibility = 1.0 - smoothstep(fade_start, fade_end, footprint);
	float master_strength = grass_mask * clamp(terrain_grass_albedo_strength, 0.0, 1.0);
	color = mix(
		color,
		grass_material_layer(color, macro_grass, pos.xz, 0.90, 0.50),
		master_strength * clamp(terrain_grass_macro_strength, 0.0, 1.0)
	);
	color = mix(
		color,
		grass_material_layer(color, mid_grass, pos.xz, 1.10, 0.58),
		master_strength * clamp(terrain_grass_mid_strength, 0.0, 1.0)
	);
	color = mix(
		color,
		grass_material_layer(color, micro_grass, pos.xz, 1.35, 0.42),
		master_strength * clamp(terrain_grass_micro_strength, 0.0, 1.0) * micro_visibility
	);
	float detail_mask = grass_mask * clamp(0.16 + micro_visibility * 0.84, 0.0, 1.0);
	return color * mix(
		1.0,
		grass_detail_brightness(micro_grass, height_detail, pos.xz),
		detail_mask
	);
}

float hillshade_mask(vec3 normal) {
	vec3 light_dir = scene_sun_direction;
	if (length(light_dir) <= 0.001) {
		float azimuth = radians(hillshade_azimuth_deg);
		float altitude = radians(hillshade_altitude_deg);
		light_dir = vec3(
			sin(azimuth) * cos(altitude),
			sin(altitude),
			-cos(azimuth) * cos(altitude)
		);
	}
	light_dir = normalize(light_dir);

	float diffuse = clamp(dot(normal, light_dir), 0.0, 1.0);
	float shaped = pow(diffuse, hillshade_contrast);
	return clamp(mix(hillshade_ambient, 1.0, shaped), 0.0, 1.0);
}

void vertex() {
	world_pos = (MODEL_MATRIX * vec4(VERTEX, 1.0)).xyz;
	world_normal = normalize((MODEL_MATRIX * vec4(NORMAL, 0.0)).xyz);
}

void fragment() {
	float macro_mask = macro_variation(world_pos.xz);
	vec3 color = terrain_surface_color(world_pos.y, macro_mask);
	color = apply_natural_land_variation(color, world_pos.xz);
	color = apply_grass_detail(color, world_pos, 1.0);

	float shade = hillshade_mask(normalize(world_normal));
	vec3 hillshade_tint = mix(hillshade_shadow_tint, hillshade_light_tint, shade);
	color *= mix(vec3(1.0), hillshade_tint, hillshade_strength);

	ALBEDO = color;
	EMISSION = color * clamp(ground_shadow_ambient, 0.0, 1.0);
	ROUGHNESS = 0.92;
	SPECULAR = 0.06;
}

void light() {
	float shadow_visibility = mix(
		clamp(ground_shadow_min_visibility, 0.0, 1.0),
		1.0,
		clamp(ATTENUATION, 0.0, 1.0)
	);
	float facing = clamp(dot(NORMAL, LIGHT), 0.0, 1.0);
	float soft_facing = mix(0.82, 1.0, facing);
	DIFFUSE_LIGHT += ALBEDO
		* LIGHT_COLOR
		* ground_shadow_sun_strength
		* shadow_visibility
		* soft_facing;
}
