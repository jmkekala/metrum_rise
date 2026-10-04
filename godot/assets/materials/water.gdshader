shader_type spatial;
render_mode depth_prepass_alpha, shadows_disabled, cull_disabled;

uniform sampler2D heightmap : repeat_disable, filter_linear;
uniform sampler2D watermap : repeat_disable, filter_linear;
uniform sampler2D screen_texture : hint_screen_texture, repeat_disable, filter_linear_mipmap;
uniform sampler2D depth_texture : hint_depth_texture, repeat_disable, filter_nearest;
uniform float height_scale = 10.0;
uniform vec4 shallow_water_color : source_color = vec4(0.10, 0.28, 0.40, 0.62);
uniform vec4 deep_water_color : source_color = vec4(0.018, 0.09, 0.18, 0.90);
uniform vec4 foam_color : source_color = vec4(0.76, 0.91, 0.96, 0.82);
uniform vec4 sky_reflection_color : source_color = vec4(0.38, 0.56, 0.72, 1.0);
uniform float shore_softness_m = 0.26;
uniform float shore_foam_band_m = 0.18;
uniform float water_deep_color_depth_m = 3.5;
uniform float water_visibility_m = 1.2;
uniform float water_fresnel_strength = 0.46;
uniform float water_fresnel_power = 2.8;
uniform float water_wave_color_strength = 0.052;
uniform float water_wave_roughness_strength = 0.024;
uniform float water_wave_normal_strength = 0.42;
uniform float water_ripple_strength = 1.0;
uniform float water_sun_glitter_strength = 0.38;
uniform float water_refraction_strength = 0.010;
uniform float water_refraction_mix = 0.13;
uniform float water_surface_smoothing = 0.94;
uniform float water_surface_blend_radius_texels = 1.0;
uniform int water_visual_debug_mode = 0;
global uniform vec3 scene_sun_direction;
global uniform vec4 scene_sun_color : source_color;
global uniform vec4 scene_sky_color : source_color;
global uniform float scene_ambient_strength;
// Water is lit normally, so it darkens with the environment on its own. This is used only to
// scale the sun glitter, whose strength is otherwise a fixed constant: without it the moon lays
// down a noon-bright specular path across the lake.
global uniform vec4 scene_ambient_light : source_color;
uniform float scene_shadow_max_distance_m = 1800.0;
uniform vec3 scene_shadow_split_distances_m = vec3(144.0, 396.0, 900.0);
uniform vec2 water_debug_patch_key = vec2(0.0);
uniform float water_debug_lod_step = 1.0;
uniform vec2 watermap_texture_size = vec2(1.0, 1.0);
uniform vec2 inner_sample_offset_texels = vec2(1.0, 1.0);
uniform vec2 inner_sample_size_texels = vec2(2.0, 2.0);

// Keeps the surface just above coincident terrain so the two never z-fight.
const float WATER_SURFACE_LIFT_M = 0.05;
// Opacity ceiling below the depth prepass's 0.99 alpha cut-off, so the water never writes the
// opaque depth that its own reflection and absorption read back.
const float WATER_MAX_ALPHA = 0.98;

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

// `heightmap` is the terrain patch's own texture, which covers the same samples as the watermap
// but with the terrain's wider border ring. Map a watermap UV to the same sample there; reading
// it with the watermap layout shifts terrain height outward by up to three samples near patch
// edges, which drops the surface below shore terrain in a strip along the seam.
vec2 terrain_height_uv(vec2 water_uv) {
	vec2 sample_texel = water_uv * max(watermap_texture_size, vec2(1.0)) - inner_sample_offset_texels;
	vec2 height_size = vec2(textureSize(heightmap, 0));
	vec2 height_border = max((height_size - max(inner_sample_size_texels, vec2(1.0))) * 0.5, vec2(0.0));
	return (height_border + sample_texel) / height_size;
}

float sample_terrain_world_height(vec2 uv) {
	return texture(heightmap, terrain_height_uv(uv)).r * height_scale;
}

float sample_raw_water_depth(vec2 uv) {
	return texture(watermap, uv).r;
}

// Smoothed water level at a vertex. A dry vertex takes the level of the wet samples beside it
// rather than its own terrain height, so the surface stays flat where a shoreline cell meets
// rising ground and the terrain hides the part below it. Lifting dry vertices to the terrain
// instead tilted the surface up steep banks. Without a wet sample nearby the vertex stays on the
// terrain.
float sample_display_water_surface(vec2 uv) {
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
		// The corner of a shoreline cell opposite its only wet corner has no wet side neighbour;
		// its wet sample is diagonal.
		for (int corner = 0; corner < 4; corner++) {
			vec2 offset = vec2(corner % 2 == 0 ? -1.0 : 1.0, corner < 2 ? -1.0 : 1.0) * (dx + dz);
			float diagonal_depth = sample_raw_water_depth(uv + offset);
			if (diagonal_depth > 0.001) {
				accum_surface += sample_terrain_world_height(uv + offset) + diagonal_depth;
				accum_weight += 1.0;
			}
		}
		if (accum_weight <= 0.0) {
			return center_terrain;
		}
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
	return display_surface;
}

void vertex() {
	var_centered_uv = centered_uv(UV);
	float water_surface = sample_display_water_surface(var_centered_uv);
	VERTEX.y += water_surface + WATER_SURFACE_LIFT_M;
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

// Wind ripples: a few deep-water waves around one wind direction. Each moves at its own phase
// speed, sqrt(g / k), so the pattern changes rather than sliding as one sheet, and fades out once
// its wavelength nears the pixel footprint, where it would only alias into shimmer. A slow noise
// field gates them into gusts, leaving calm patches that mirror the shore. The broad noise field
// above is 40-180 m across and only shades the surface; it never read as waves.
const vec2 WATER_WIND_DIRECTION = vec2(0.8, 0.6);
// Wavelength (m), angle from the wind (rad), steepness (amplitude x wavenumber).
const vec3 WATER_RIPPLES[5] = {
	vec3(7.3, 0.0, 0.010),
	vec3(4.1, 0.62, 0.011),
	vec3(2.6, -0.55, 0.011),
	vec3(1.55, 1.05, 0.010),
	vec3(0.93, -0.9, 0.009)
};

vec2 water_ripple_slope(vec2 world_xz, float footprint_m) {
	vec2 slope = vec2(0.0);
	for (int i = 0; i < 5; i++) {
		vec3 ripple = WATER_RIPPLES[i];
		float c = cos(ripple.y);
		float s = sin(ripple.y);
		vec2 direction = vec2(
			WATER_WIND_DIRECTION.x * c - WATER_WIND_DIRECTION.y * s,
			WATER_WIND_DIRECTION.x * s + WATER_WIND_DIRECTION.y * c
		);
		float wavenumber = TAU / ripple.x;
		float phase = wavenumber * dot(direction, world_xz) - sqrt(9.81 * wavenumber) * TIME
			+ float(i) * 1.7;
		float resolved = 1.0 - smoothstep(ripple.x * 0.1, ripple.x * 0.35, footprint_m);
		slope += direction * (ripple.z * cos(phase) * resolved);
	}
	float gust = smoothstep(0.3, 0.75,
		value_noise_field(world_xz * 0.012 + TIME * vec2(0.02, 0.015)).x);
	return slope * mix(0.35, 1.0, gust) * water_ripple_strength;
}

// View-space normal of the surface tilted by a world-space slope (dh/dx, dh/dz).
vec3 water_view_normal(vec2 slope, mat4 view_matrix) {
	return normalize((view_matrix * vec4(normalize(vec3(-slope.x, 1.0, -slope.y)), 0.0)).xyz);
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

// Metres of water the view ray crosses between the surface and the opaque scene behind it.
// The straight-down depth leaves the bed, and the shadows cast onto it, showing through deep
// water at low view angles, where the ray actually crosses far more water than that.
float water_view_thickness(vec2 screen_uv, vec3 view_pos, mat4 inv_projection) {
	float scene_depth = texture(depth_texture, screen_uv).r;
	vec4 scene = inv_projection * vec4(screen_uv * 2.0 - 1.0, scene_depth, 1.0);
	return max(length(scene.xyz / scene.w) - length(view_pos), 0.0);
}

// Screen-space reflection of the opaque scene in the water. Godot's own SSR skips transparent
// materials, so the water reflected only a flat sky colour: no shore, no trees, no buildings.
// The reflected view ray is marched through the opaque depth buffer in geometrically growing
// steps. A step that ends behind a drawn surface is bisected to the crossing, which is a hit
// only if the ray there is just behind that surface; a ray that only passed behind a nearer
// object, such as a trunk in front of the shore, keeps marching. Accepting a step only when it
// landed within a fixed depth behind the surface lost every ray whose step overshot a hillside,
// which cut reflected slopes into bands and left a sky-coloured gap following each near trunk.
// O(REFLECTION_STEPS + crossings * REFLECTION_REFINE) texture reads per water pixel. `weight`
// is zero where the ray meets no scene on screen; the sky there stays the engine's own specular
// sky reflection. Reflecting the sky from the screen instead stretched its edge row into streaks
// off screen, and any fallback colour framed the screen edge where it met the on-screen sky.
// Twelve steps growing 1.8x reach as far as 24 growing 1.38x, for half the reads, with the
// same reflections on the benchmark views.
const int REFLECTION_STEPS = 12;
const float REFLECTION_STEP_GROWTH = 1.8;
const int REFLECTION_REFINE = 6;
// Depth behind the surface at the refined crossing that still counts as the ray meeting it, as
// a multiple of the refined bracket plus a share of the view depth for the depth buffer's own
// precision. A ray behind a nearer object stays tens of metres behind it however far it is
// refined.
const float REFLECTION_HIT_BRACKETS = 4.0;
const float REFLECTION_HIT_DEPTH_SHARE = 0.01;
// Blur of the reflection: its width grows with the reflected ray's length, as the ripples
// scatter it, and is read from the screen-texture mip of that width in pixels. Sizing it in
// metres alone blurred a far island's thin reflection into a mip that is mostly sky, so it read
// pale and white beneath an island that is not.
const float REFLECTION_BLUR_PER_M = 0.02;
const float REFLECTION_MAX_LOD = 4.0;

vec3 water_view_at(vec2 uv, mat4 inv_projection) {
	float depth = texture(depth_texture, uv).r;
	vec4 view = inv_projection * vec4(uv * 2.0 - 1.0, depth, 1.0);
	return view.xyz / view.w;
}

vec2 water_screen_at(vec3 view, mat4 projection) {
	vec4 clip = projection * vec4(view, 1.0);
	return clip.xy / clip.w * 0.5 + 0.5;
}

// Mip whose texel spans the blur of a ray `ray_m` long meeting the scene at `hit_view`.
float water_reflection_lod(float ray_m, vec3 hit_view, mat4 projection) {
	float pixels_per_m = projection[1][1] * 0.5 * float(textureSize(screen_texture, 0).y)
		/ max(-hit_view.z, 1.0);
	return clamp(log2(1.0 + ray_m * REFLECTION_BLUR_PER_M * pixels_per_m), 0.0, REFLECTION_MAX_LOD);
}

// True where the view-space point is behind the opaque scene drawn at its pixel. The sky is
// never in front of anything.
bool water_ray_behind_scene(vec3 sample_view, mat4 projection, mat4 inv_projection,
		out float gap_m) {
	vec2 uv = water_screen_at(sample_view, projection);
	float scene_depth = texture(depth_texture, uv).r;
	vec4 scene = inv_projection * vec4(uv * 2.0 - 1.0, scene_depth, 1.0);
	gap_m = scene.z / scene.w - sample_view.z;
	return scene_depth > 0.0 && scene_depth < 1.0 && gap_m > 0.0;
}

vec3 water_reflection(vec3 view_pos, vec3 view_normal, mat4 projection, mat4 inv_projection,
		out float weight) {
	weight = 0.0;
	vec3 direction = reflect(normalize(view_pos), view_normal);
	// A ray turning back toward the camera leaves the screen at once. Fading out toward that
	// direction, rather than cutting at it, keeps reflected shores from ending at an edge where
	// the view looks steeply down.
	float facing = 1.0 - smoothstep(-0.30, -0.02, direction.z);
	if (facing <= 0.0) {
		return vec3(0.0);
	}
	float near_t = 0.0;
	float far_t = max(length(view_pos) * 0.02, 0.5);
	float gap_m;
	for (int i = 0; i < REFLECTION_STEPS; i++) {
		vec3 sample_view = view_pos + direction * far_t;
		vec2 uv = water_screen_at(sample_view, projection);
		if (any(lessThan(uv, vec2(0.0))) || any(greaterThan(uv, vec2(1.0)))) {
			break;
		}
		if (water_ray_behind_scene(sample_view, projection, inv_projection, gap_m)) {
			float in_front_t = near_t;
			float behind_t = far_t;
			for (int r = 0; r < REFLECTION_REFINE; r++) {
				float mid_t = (in_front_t + behind_t) * 0.5;
				if (water_ray_behind_scene(view_pos + direction * mid_t, projection, inv_projection,
						gap_m)) {
					behind_t = mid_t;
				} else {
					in_front_t = mid_t;
				}
			}
			vec3 hit_view = view_pos + direction * behind_t;
			water_ray_behind_scene(hit_view, projection, inv_projection, gap_m);
			if (gap_m < (behind_t - in_front_t) * REFLECTION_HIT_BRACKETS
					- hit_view.z * REFLECTION_HIT_DEPTH_SHARE) {
				uv = water_screen_at(hit_view, projection);
				vec2 edge = min(uv, vec2(1.0) - uv);
				weight = smoothstep(0.0, 0.15, min(edge.x, edge.y)) * facing;
				return textureLod(screen_texture, uv,
					water_reflection_lod(behind_t, hit_view, projection)).rgb;
			}
		}
		near_t = far_t;
		far_t *= REFLECTION_STEP_GROWTH;
	}
	return vec3(0.0);
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
	vec3 color = mix(vec3(0.02, 0.04, 0.08), scene_sun_color.rgb, sun_facing);
	if (column < 1.0) {
		color = mix(vec3(0.02, 0.04, 0.08), scene_sun_color.rgb, sun_facing);
	} else if (column < 2.0) {
		color = mix(vec3(0.02, 0.03, 0.05), vec3(1.0, 0.94, 0.72), specular_value);
	} else if (column < 3.0) {
		color = mix(vec3(0.03, 0.07, 0.12), scene_sky_color.rgb, fresnel_value);
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
	// Water column under this pixel: the flat surface minus the terrain below it. Shoreline cells
	// are drawn whole at the water level, so this reaches zero exactly where the terrain meets the
	// surface, unlike the depth texture, which interpolates across the cell to its dry corner.
	float contour_depth_m = max(
		var_world_pos.y - WATER_SURFACE_LIFT_M - sample_terrain_world_height(var_centered_uv),
		0.0
	);
	// Never deeper than the interpolated water depth, which is zero along any cell edge between
	// two dry samples. A dry sample can lie below the water level, in a hollow the lake fill does
	// not reach; the surface still takes the level of its wet neighbours, so the two cells holding
	// a wet corner drew water over the hollow and the two without one did not, which left a dark
	// cross of water on dry land. On an ordinary shore the rising terrain meets the surface first.
	contour_depth_m = min(contour_depth_m, sample_raw_water_depth(var_centered_uv));
	float depth_m = contour_depth_m;
	float shore_mask = smoothstep(0.0, max(shore_softness_m, 0.001), contour_depth_m);
	float depth_factor = smoothstep(0.12, max(water_deep_color_depth_m, 0.121), depth_m);
	vec3 surface_field = water_surface_field(var_world_pos.xz);
	float wave = surface_field.x;
	float wave_signed = wave * 2.0 - 1.0;
	vec2 wave_slope = surface_field.yz;
	vec2 ripple_slope = water_ripple_slope(var_world_pos.xz, length(fwidth(var_world_pos.xz)));
	vec3 world_normal = water_world_normal(wave_slope);
	float normal_debug_strength = clamp(length(wave_slope) * water_wave_normal_strength * 18.0, 0.0, 1.0);
	vec3 depth_tint = mix(shallow_water_color.rgb, deep_water_color.rgb, depth_factor);
	depth_tint *= 1.0 + wave_signed * water_wave_color_strength;
	float shore_foam = (1.0 - smoothstep(0.0, max(shore_foam_band_m, 0.001), contour_depth_m))
		* shore_mask;
	float foam_amount = clamp(shore_foam * 0.16, 0.0, 0.34);
	// Light from the bed decays with the water it crosses; the colour ramp alone capped opacity
	// below one, so tree shadows on the bed showed through even deep water.
	float absorbed = 1.0 - exp(-water_view_thickness(SCREEN_UV, VERTEX, INV_PROJECTION_MATRIX)
		/ max(water_visibility_m, 0.01));
	float final_alpha = max(mix(shallow_water_color.a, deep_water_color.a, depth_factor), absorbed)
		* shore_mask;
	final_alpha = clamp(final_alpha, 0.0, WATER_MAX_ALPHA);

	// The level surface's normal is world up. NORMAL is not: the patch triangles face away from a
	// camera above them, and with culling disabled Godot flips NORMAL on back faces, so the lake
	// was lit as a surface facing down, with no sun and Fresnel pinned at its grazing maximum.
	vec3 base_view_normal = normalize((VIEW_MATRIX * vec4(0.0, 1.0, 0.0, 0.0)).xyz);
	NORMAL = water_view_normal(wave_slope * water_wave_normal_strength + ripple_slope, VIEW_MATRIX);
	// Schlick with water's 0.02 reflectance straight down.
	float fresnel = 0.02 + 0.98 * pow(
		1.0 - clamp(dot(base_view_normal, normalize(var_view_direction)), 0.0, 1.0),
		water_fresnel_power
	);
	float fresnel_mix = clamp(fresnel * water_fresnel_strength, 0.0, 1.0);
	float sun_specular = water_sun_glitter(NORMAL, wave)
		* shore_mask
		* (0.40 + depth_factor * 0.60);
	vec3 sky_reflection = mix(sky_reflection_color.rgb, scene_sky_color.rgb, 0.65);

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
		float shallow_refraction = (1.0 - depth_factor) * shore_mask * (1.0 - absorbed);
		vec2 refraction_offset = wave_slope * water_refraction_strength * shallow_refraction;
		vec3 refracted_scene = texture(screen_texture, clamp(SCREEN_UV + refraction_offset, vec2(0.001), vec2(0.999))).rgb;
		final_color = mix(
			final_color,
			refracted_scene,
			clamp(shallow_refraction * water_refraction_mix, 0.0, 0.35)
		);
		// Reflected scene where the ray finds one, the sky where it does not. The scene is
		// already lit, so it is emitted rather than lit again as albedo. The ripples, and a small
		// share of the broad wave slope, bend the reflected ray.
		float reflection_weight = 0.0;
		vec3 reflected_scene = vec3(0.0);
		// Below a few percent the reflection cannot be seen, and steep views skip the march.
		if (fresnel_mix > 0.05) {
			// Half the ripple slope: the full bend chopped reflected trees into columns.
			vec3 ripple_normal = water_view_normal(
				(wave_slope * water_wave_normal_strength * 0.25 + ripple_slope) * 0.5, VIEW_MATRIX);
			reflected_scene = water_reflection(VERTEX, ripple_normal, PROJECTION_MATRIX,
				INV_PROJECTION_MATRIX, reflection_weight);
		}
		// The sky itself is the engine's specular reflection below; mixing a sky tint into the
		// albedo as well counted it twice once the surface faced up.
		final_color *= 1.0 - fresnel_mix;
		EMISSION = reflected_scene * fresnel_mix * reflection_weight;
		final_color += scene_sun_color.rgb
			* scene_ambient_light.rgb
			* sun_specular
			* water_sun_glitter_strength;
		float topdown_ripple = smoothstep(0.48, 0.86, wave)
			* normal_debug_strength
			* (1.0 - fresnel_mix * 0.35)
			* 0.07;
		final_color = mix(final_color, sky_reflection, topdown_ripple);
		ALBEDO = final_color;
		// A reflection is light leaving the surface, not the lake bed seen through it.
		ALPHA = min(mix(final_alpha, 1.0, fresnel_mix * shore_mask), WATER_MAX_ALPHA);
		ROUGHNESS = clamp(0.08 + normal_debug_strength * water_wave_roughness_strength, 0.05, 0.20);
		// The engine's own specular reflects the sky; where the scene is reflected instead, it
		// would lay that sky over the reflected shore and wash it out.
		// 0.25 is Godot's encoding of water's 0.02 reflectance; the engine applies its own
		// Fresnel on top, matching the one above.
		SPECULAR = 0.25 * (1.0 - reflection_weight);
	}
}
