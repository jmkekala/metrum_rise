shader_type spatial;

uniform sampler2D heightmap;
uniform sampler2D overlay_texture;
uniform sampler2D parcel_texture;
uniform sampler2D watermap : repeat_disable, filter_linear;
uniform int overlay_mode = 0;
uniform float height_scale = 10.0;
uniform vec2 mesh_size = vec2(256.0, 256.0);
uniform vec2 heightmap_texture_size = vec2(256.0, 256.0);
uniform float terrain_cell_m = 10.0;
uniform float hillshade_azimuth_deg = 315.0;
uniform float hillshade_altitude_deg = 42.0;
uniform float hillshade_strength = 0.42;
uniform float hillshade_ambient = 0.36;
uniform float hillshade_contrast = 1.15;
uniform vec3 hillshade_shadow_tint : source_color = vec3(0.62, 0.71, 0.77);
uniform vec3 hillshade_light_tint : source_color = vec3(0.97, 0.99, 0.95);
uniform float terrain_macro_variation_strength = 0.10;
uniform float terrain_rock_slope_start = 0.15;
uniform float terrain_rock_slope_end = 0.34;
uniform float terrain_shore_blend_strength = 0.52;
uniform float terrain_shore_lookup_radius_texels = 2.0;
uniform float contour_minor_interval_m = 2.5;
uniform float contour_major_interval_m = 10.0;
uniform float contour_minor_thickness = 0.95;
uniform float contour_major_thickness = 1.25;
uniform vec3 contour_minor_color : source_color = vec3(0.13, 0.19, 0.16);
uniform vec3 contour_major_color : source_color = vec3(0.10, 0.16, 0.14);
uniform float contour_minor_strength = 0.14;
uniform float contour_major_strength = 0.34;

varying float h;
varying vec2 var_centered_uv;

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

vec3 terrain_color_from_height(float world_elevation, float macro_mask) {
	vec3 low_wet = vec3(0.10, 0.18, 0.10);
	vec3 shore = vec3(0.52, 0.50, 0.34);
	vec3 grass_low = vec3(0.19, 0.34, 0.14);
	vec3 grass_high = vec3(0.14, 0.25, 0.12);
	vec3 upland = vec3(0.34, 0.33, 0.24);
	vec3 rock = vec3(0.43, 0.44, 0.42);
	vec3 snow = vec3(0.93, 0.94, 0.96);

	vec3 color = low_wet;
	color = mix(color, grass_low, smoothstep(-30.0, 35.0, world_elevation));
	color = mix(color, shore, 1.0 - smoothstep(8.0, 42.0, abs(world_elevation)));
	color = mix(color, grass_high, smoothstep(80.0, 260.0, world_elevation));
	color = mix(color, upland, smoothstep(300.0, 700.0, world_elevation));
	color = mix(color, rock, smoothstep(800.0, 1400.0, world_elevation));
	color = mix(color, snow, smoothstep(1550.0, 2200.0, world_elevation));
	vec3 macro_tint = mix(vec3(1.03, 1.01, 0.97), vec3(0.92, 1.01, 0.95), macro_mask);
	color *= mix(vec3(1.0), macro_tint, terrain_macro_variation_strength);

	return color;
}

float sample_world_height(vec2 uv) {
	vec2 clamped_uv = clamp(uv, vec2(0.0), vec2(1.0));
	return texture(heightmap, clamped_uv).r * height_scale;
}

vec3 sample_world_normal(vec2 uv) {
	vec2 texel = 1.0 / max(heightmap_texture_size, vec2(1.0));
	float h_left = sample_world_height(uv - vec2(texel.x, 0.0));
	float h_right = sample_world_height(uv + vec2(texel.x, 0.0));
	float h_up = sample_world_height(uv - vec2(0.0, texel.y));
	float h_down = sample_world_height(uv + vec2(0.0, texel.y));
	float safe_cell = max(terrain_cell_m, 0.001);
	float dzdx = (h_right - h_left) / (2.0 * safe_cell);
	float dzdz = (h_down - h_up) / (2.0 * safe_cell);
	return normalize(vec3(-dzdx, 1.0, -dzdz));
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

float hillshade_mask(vec2 uv) {
	vec3 normal = sample_world_normal(uv);
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

float contour_mask(float world_elevation, float interval_m, float thickness) {
	float safe_interval = max(interval_m, 0.001);
	float scaled = world_elevation / safe_interval;
	float dist_to_line = abs(fract(scaled + 0.5) - 0.5);
	float aa_width = max(fwidth(scaled) * thickness, 0.0001);
	return 1.0 - smoothstep(0.0, aa_width, dist_to_line);
}

void vertex() {
// ...

	// TEXEL-CENTERED SAMPLING: Sample from the center of each float32 pixel
	// UV 0.0 should map to the center of the first pixel (0.5/256)
	vec2 centered_uv = (UV * (mesh_size - 1.0) + 0.5) / mesh_size;
	var_centered_uv = centered_uv;
	h = texture(heightmap, centered_uv).r;
	VERTEX.y += h * height_scale;
}

void fragment() {
	float world_elevation = h * height_scale;
	vec2 world_xz = (var_centered_uv - vec2(0.5)) * mesh_size;
	float macro_mask = macro_variation(world_xz);
	vec3 color = terrain_color_from_height(world_elevation, macro_mask);
	vec3 normal = sample_world_normal(var_centered_uv);
	float slope = 1.0 - clamp(normal.y, 0.0, 1.0);
	float rock_mask = smoothstep(terrain_rock_slope_start, terrain_rock_slope_end, slope);
	rock_mask *= smoothstep(20.0, 220.0, world_elevation + (macro_mask - 0.5) * 120.0);
	vec3 rock_color = mix(vec3(0.31, 0.29, 0.25), vec3(0.46, 0.45, 0.43), macro_mask);
	color = mix(color, rock_color, rock_mask);
	float shore_mask = shoreline_mask(var_centered_uv);
	float shore_flatness = 1.0 - smoothstep(terrain_rock_slope_start, terrain_rock_slope_end + 0.10, slope);
	vec3 shore_color = mix(vec3(0.38, 0.37, 0.25), vec3(0.20, 0.29, 0.18), smoothstep(60.0, 220.0, world_elevation));
	color = mix(color, shore_color, shore_mask * shore_flatness * terrain_shore_blend_strength);
	float hillshade = hillshade_mask(var_centered_uv);
	vec3 hillshade_tint = mix(hillshade_shadow_tint, hillshade_light_tint, hillshade);
	color *= mix(vec3(1.0), hillshade_tint, hillshade_strength);
	
	// Overlay Heatmaps (Pollution, Noise, Desirability)
	if (overlay_mode > 0) {
		vec4 heatmap_col = texture(overlay_texture, var_centered_uv);
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
	color = mix(color, contour_minor_color, minor_contour * contour_minor_strength);
	color = mix(color, contour_major_color, major_contour * contour_major_strength);
	
	ALBEDO = color;
	ROUGHNESS = mix(0.96, 0.76, rock_mask);
	SPECULAR = 0.08;
}
