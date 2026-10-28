// SPDX-License-Identifier: GPL-2.0-only

shader_type spatial;
render_mode cull_disabled;

uniform vec2 window_cell;
uniform int window_columns = 1;
// One clump per cell of this size. The near layer (grass.gd) draws a finer grid of its own
// clumps, with its own hash salt, between the far layer's, and fades out by `near_fade_m.y`.
uniform float cell_m = 0.8;
uniform uint layer_salt = 0u;
// Distances over which the near layer fades out; zero for the far layer, which fades by cover.
uniform vec2 near_fade_m = vec2(0.0);
uniform vec2 patch_origin;
uniform vec2 patch_world_size_m;
// PlaneMesh intervals.
uniform vec2 terrain_mesh_cells;
uniform sampler2D heightmap : repeat_disable, filter_linear;
uniform float height_scale = 20.0;
uniform vec2 heightmap_texture_size;
uniform vec2 inner_sample_offset_texels;
uniform vec2 inner_sample_size_texels;
uniform float terrain_cell_m = 5.0;
uniform sampler2D watermap : repeat_disable, filter_linear;
uniform vec2 watermap_texture_size;
uniform vec2 watermap_inner_sample_offset_texels;
uniform vec2 watermap_inner_sample_size_texels;
uniform sampler2D land_cover_texture : repeat_disable, filter_linear;
uniform vec4 land_cover_world_bounds;
uniform sampler2D field_overlay_texture : repeat_disable, filter_linear;
uniform vec4 field_overlay_world_bounds;
uniform sampler2D coal_pit_texture : repeat_disable, filter_linear;
uniform vec4 coal_pit_overlay_world_bounds;
uniform sampler2D surface_mask : repeat_disable, filter_linear;
uniform vec4 mask_world_bounds;
uniform sampler2D terrain_grain_albedo : source_color, repeat_enable, filter_linear_mipmap;
uniform float terrain_grain_detail_scale;
uniform vec3 terrain_grain_tint : source_color;
uniform float terrain_grain_brightness;
uniform float terrain_grain_mix_strength;
uniform float terrain_grain_macro_strength;
uniform float ground_shadow_ambient = 0.5;
uniform float canopy_floor_sky_transmission = 0.4;
uniform float specular = 0.0;
global uniform vec4 scene_ambient_light : source_color;
global uniform float scene_ambient_desaturation;

#define GRASS_BAKED_WORLD_NOISE
#include "res://scripts/shaders/ground_grass.gdshaderinc"
#include "res://scripts/shaders/terrain_grain.gdshaderinc"
#include "res://scripts/shaders/weather_wind.gdshaderinc"
#define STANDARD_LIGHT_GRASS_TRANSMISSION
#include "res://scripts/shaders/standard_light.gdshaderinc"
#include "res://scripts/shaders/weather_rain.gdshaderinc"

varying vec3 root_position;
varying float forest_cover;
varying float field_cover;
varying float blade_height;
varying float grass_sheen;

// Integer avalanche: world cells, including negative coordinates, have stable identities.
float cell_random(ivec2 cell, uint salt) {
	uint h = uint(cell.x) * 1597334677u ^ uint(cell.y) * 3812015801u ^ salt
		^ layer_salt * 2654435769u;
	h = (h ^ (h >> 16u)) * 2246822519u;
	h = (h ^ (h >> 13u)) * 3266489917u;
	return float((h ^ (h >> 16u)) >> 8u) / 16777216.0;
}

float overlay_at(sampler2D map, vec4 bounds, vec2 world_xz) {
	vec2 uv = (world_xz - bounds.xy) / max(bounds.zw, vec2(0.001));
	if (bounds.z <= 0.0 || bounds.w <= 0.0 || any(lessThan(uv, vec2(0.0)))
			|| any(greaterThan(uv, vec2(1.0)))) {
		return 0.0;
	}
	return textureLod(map, uv, 0.0).r;
}

float height_at(vec2 uv) {
	return textureLod(heightmap, uv, 0.0).r * height_scale;
}

vec3 surface_at(vec2 patch_uv, vec2 height_uv) {
	if (terrain_mesh_cells.x > 0.0) {
		vec2 grid = patch_uv * terrain_mesh_cells;
		vec2 f = fract(grid);
		vec2 uv_step = (inner_sample_size_texels - 1.0) / terrain_mesh_cells / heightmap_texture_size;
		vec2 uv00 = height_uv - f * uv_step;
		float a = height_at(uv00);
		float b = height_at(uv00 + vec2(uv_step.x, 0.0));
		float c = height_at(uv00 + vec2(0.0, uv_step.y));
		float d = height_at(uv00 + uv_step);
		// Godot PlaneMesh joins (1,0) to (0,1). Bilinear interpolation is NOT that surface.
		vec2 gradient = f.x + f.y <= 1.0 ? vec2(b - a, c - a) : vec2(d - c, d - b);
		float y = f.x + f.y <= 1.0 ? a + dot(f, gradient) : d - dot(1.0 - f, gradient);
		return vec3(y, gradient / (patch_world_size_m / terrain_mesh_cells));
	}
	vec2 texel = 1.0 / heightmap_texture_size;
	return vec3(height_at(height_uv),
		vec2(height_at(height_uv + vec2(texel.x, 0.0)) - height_at(height_uv - vec2(texel.x, 0.0)),
			height_at(height_uv + vec2(0.0, texel.y)) - height_at(height_uv - vec2(0.0, texel.y))) / (2.0 * terrain_cell_m));
}

void vertex() {
	ivec2 cell = ivec2(window_cell) + ivec2(INSTANCE_ID % window_columns, INSTANCE_ID / window_columns);
	vec2 jitter = vec2(cell_random(cell, 17u), cell_random(cell, 71u)) - 0.5;
	vec2 xz = (vec2(cell) + 0.5 + jitter * 0.72) * cell_m;
	field_cover = step(0.5, overlay_at(field_overlay_texture, field_overlay_world_bounds, xz));
	// Grain keeps straight world-Z rows; jitter only along them. No camera-relative pattern.
	xz.x = mix(xz.x, (float(cell.x) + 0.5) * cell_m, field_cover);
	vec2 patch_uv = (xz - patch_origin) / patch_world_size_m;
	vec2 height_uv = (inner_sample_offset_texels + patch_uv * (inner_sample_size_texels - 1.0) + 0.5)
		/ heightmap_texture_size;
	vec3 surface = surface_at(patch_uv, height_uv);
	float ground_y = surface.x;
	root_position = vec3(xz.x, ground_y - 0.025, xz.y);
	float distance_m = length(CAMERA_POSITION_WORLD - root_position);
	float density = 1.0 - smoothstep(mix(28.0, 45.0, field_cover), mix(60.0, 70.0, field_cover), distance_m);
	if (near_fade_m.y > 0.0) {
		// Grain keeps the far layer's rows; a finer grid between them would break them.
		density = (1.0 - smoothstep(near_fade_m.x, near_fade_m.y, distance_m)) * (1.0 - field_cover);
	}
	bool keep = density > cell_random(cell, 113u)
		&& all(greaterThanEqual(patch_uv, vec2(0.0))) && all(lessThan(patch_uv, vec2(1.0)));
	forest_cover = 0.0;
	grass_sheen = 1.0;
	blade_height = UV.y;
	if (keep) {
		vec2 water_uv = (watermap_inner_sample_offset_texels
			+ patch_uv * (watermap_inner_sample_size_texels - 1.0) + 0.5) / watermap_texture_size;
		vec2 mask_uv = (xz - mask_world_bounds.xy) / mask_world_bounds.zw;
		// A footprint guard also keeps blade roots off narrow paths and mesh-cut site edges.
		vec2 guard = vec2(0.4) / mask_world_bounds.zw;
		float natural = min(min(textureLod(surface_mask, mask_uv + guard, 0.0).r,
			textureLod(surface_mask, mask_uv - guard, 0.0).r),
			min(textureLod(surface_mask, mask_uv + vec2(guard.x, -guard.y), 0.0).r,
			textureLod(surface_mask, mask_uv + vec2(-guard.x, guard.y), 0.0).r));
		float cover = overlay_at(land_cover_texture, land_cover_world_bounds, xz);
		forest_cover = cover * smoothstep(0.35, 0.7, cover) * (1.0 - field_cover);
		keep = natural > 0.99 && textureLod(watermap, water_uv, 0.0).r < 0.001
			&& dot(surface.yz, surface.yz) < 0.36
			&& overlay_at(coal_pit_texture, coal_pit_overlay_world_bounds, xz) < 0.01
			&& cell_random(cell, 197u) < mix(1.0, 0.24, forest_cover);
	}
	if (keep) {
		float height = mix(0.25, 0.5, cell_random(cell, 229u));
		height = mix(height * mix(1.0, 0.55, forest_cover), mix(0.8, 1.0, cell_random(cell, 257u)), field_cover);
		float angle = cell_random(cell, 293u) * TAU;
		vec2 blade_xz = mat2(vec2(cos(angle), sin(angle)), vec2(-sin(angle), cos(angle))) * VERTEX.xz;
		float root_slope_y = dot(surface.yz, blade_xz);
		// The wave runs along a fixed direction (see weather_wind_direction); the blades bend
		// downwind.
		const vec2 wave_along = vec2(0.94385836, 0.33035042);
		vec2 downwind = weather_wind_direction / max(length(weather_wind_direction), 0.0001);
		float gust_factor = weather_gust_factor(weather_gust(xz));
		float storminess = weather_storminess();
		// Quantize the fundamental before multiples: all phases close at the clock wrap.
		float phase = dot(xz, wave_along) * TAU / 14.0 - weather_wind.x * weather_wind_omega(2.4);
		float wave = sin(phase) * 0.5 + 0.5;
		float lean = min(weather_wind.y, 3.5) * gust_factor
			* (0.10 + storminess * 0.12 + wave * mix(0.10, 0.24, storminess));
		lean *= mix(1.0, 0.35, forest_cover) * mix(1.0, 1.2, field_cover);
		lean = min(lean, 1.15);
		float bend = lean * UV.y;
		blade_xz += downwind * sin(bend) * height * UV.y;
		vec3 world = root_position + vec3(blade_xz.x, root_slope_y + cos(bend) * height * UV.y, blade_xz.y);
		VERTEX = world - MODEL_MATRIX[3].xyz;
		// Soft upright normals avoid three dark paper fins; tip tilt makes rolling light waves.
		NORMAL = normalize(vec3(-downwind.x * sin(bend) * 0.65, 1.0, -downwind.y * sin(bend) * 0.65));
		grass_sheen = 1.0 + storminess * (wave - 0.5) * 0.16;
	} else {
		// Degenerate triangles: no alpha blend, discarded fragments or screen-door stipple.
		VERTEX = root_position - MODEL_MATRIX[3].xyz;
	}
}

#include "res://scripts/shaders/street_lamp_light.gdshaderinc"

void fragment() {
	// cull_disabled flips back-face normals before fragment(); keep the authored soft-up basis.
	if (!FRONT_FACING) {
		NORMAL = -NORMAL;
	}
	float macro = macro_variation(root_position.xz);
	vec3 color = terrain_surface_color(root_position.y, macro, 0.0);
	color = apply_natural_land_variation(color, root_position.xz, 0.0, 0.0, 0.0, 0.0, 0.0);
	color = apply_grass_detail(color, root_position, 1.0);
	vec3 floor_color = mix(vec3(0.46, 0.40, 0.28), vec3(0.38, 0.45, 0.30), macro);
	floor_color *= dot(color, vec3(0.2126, 0.7152, 0.0722)) / dot(floor_color, vec3(0.2126, 0.7152, 0.0722));
	color = mix(color, floor_color, forest_cover);
	if (field_cover > 0.5) {
		color = terrain_grain_field_color(root_position.xz);
		color = mix(color, color * vec3(1.14, 1.04, 0.70), blade_height);
	}
	// The terrain under it is the clump's mean colour; the blades vary about it so they read as
	// grass, not as the ground raised: darker, shaded bases, lighter and drier tips, and a hue
	// that changes from clump to clump.
	float clump = fract(sin(dot(floor(root_position.xz / 0.8), vec2(12.9898, 78.233))) * 43758.5453);
	vec3 dry = color * vec3(1.18, 1.10, 0.72);
	color = mix(color, dry, (0.05 + 0.2 * clump) * blade_height * (1.0 - field_cover));
	color *= mix(0.72, 1.12, blade_height) * mix(0.9, 1.08, clump) * grass_sheen;
	color = mix(color, vec3(dot(color, vec3(0.2126, 0.7152, 0.0722))), scene_ambient_desaturation);
	// Darkened with the ground it grows from; blades carry no specular to turn glossy.
	float blade_roughness = 1.0;
	weather_wet_surface(weather_wetness(1.0), 1.0, color, blade_roughness);
	ALBEDO = color;
	ROUGHNESS = 1.0;
	SPECULAR = 0.0;
	IRRADIANCE = vec4(0.0, 0.0, 0.0, 1.0);
	RADIANCE = vec4(0.0, 0.0, 0.0, 1.0);
	EMISSION = color * ground_shadow_ambient * scene_ambient_light.rgb
		* mix(1.0, canopy_floor_sky_transmission, 1.0 - pow(1.0 - forest_cover, 2.0));
	standard_light_cloud_sun = standard_light_cloud(VERTEX, INV_VIEW_MATRIX);
	EMISSION += ALBEDO * (1.0 - METALLIC) * street_lamp_surface(VERTEX, NORMAL, TANGENT, BINORMAL,
		NORMAL_MAP, NORMAL_MAP_DEPTH, INV_VIEW_MATRIX, CAMERA_POSITION_WORLD);
}
