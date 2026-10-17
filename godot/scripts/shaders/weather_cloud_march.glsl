#[compute]
#version 450

// SPDX-License-Identifier: GPL-2.0-only

// Volumetric cloud march, dispatched by weather_cloud_effect.gd in two modes.
//   View (mode 0): one texel per low-resolution screen pixel. Each frame marches with a fresh
//     jitter and blends into the reprojected history, so a few dozen steps converge to a smooth
//     cloud over a handful of frames.
//   Panorama (mode 1): the upper hemisphere around the camera in a Lambert equal-area disk,
//     which the water samples for its cloud reflections.
// Each texel stores the scattered light (rgb) and the transmittance (a).
//
// Where clouds stand comes from the same 2D weather map as their ground shadows
// (weather_clouds.gdshaderinc), so a shadow lies under its cloud. The 3D noise only shapes them.
layout(local_size_x = 8, local_size_y = 8, local_size_z = 1) in;

layout(set = 0, binding = 0, std140) uniform Params {
	mat4 inv_projection;
	// Camera-to-world rotation.
	mat4 camera_basis;
	// World direction to the previous frame's clip space, translation dropped: clouds are far
	// enough that the camera's own motion barely moves them on screen.
	mat4 previous_view_projection;
	// xyz: camera position. w: frame counter for the jitter sequence.
	vec4 camera_position;
	// xy: world X/Z offset (seeded origin minus drift). z: coverage. w: optical depth.
	vec4 clouds;
	// xyz: towards the sun. w: 1 by day, 0 by moonlight.
	vec4 sun;
	vec4 shade;
	vec4 light;
	vec4 horizon;
	// xy: target size. z: 1 if the history is usable. w: mode.
	vec4 target;
	// The high cloud layer. xy: world X/Z offset (its seeded origin minus its drift). z: cover.
	// w: rain intensity in [0, 1], which belongs to the main deck, packed here for room.
	vec4 high;
} params;

layout(set = 0, binding = 1) uniform sampler3D noise_texture;
layout(set = 0, binding = 2) uniform sampler2D shape_texture;
layout(set = 0, binding = 3) uniform sampler2D history_texture;
layout(rgba16f, set = 0, binding = 4) uniform restrict writeonly image2D output_image;

// Must match weather_clouds.gdshaderinc and weather_cloud_field.gdshaderinc.
const float CLOUD_BASE_M = 1500.0;
const float CLOUD_THICKNESS_M = 1400.0;
// One tile of the body noise: its coarsest Worley cells are 500 m across, so a cloud stacks
// several billows; kilometre cells made each cloud one blob extruded upwards. The detail tile
// erodes the edges into lobes of a few tens of metres. Both divide the 65536 m offset wrap.
const float BODY_TILE_M = 2048.0;
const float DETAIL_TILE_M = 512.0;
// Extinction per metre of a cloud at density 1 and optical depth 1. Real cumulus is around
// 0.05 per metre; this is lower so a cloud's edge reads soft rather than cut out.
const float EXTINCTION_PER_M = 0.03;
// Path through the layer that is marched at most, and the nearest base distance still marched.
// Further cloud is horizon haze only.
const float MAX_PATH_M = 14000.0;
const float MAX_BASE_DISTANCE_M = 45000.0;
const float HAZE_DISTANCE_M = 26000.0;
const float HISTORY_WEIGHT = 0.88;
// Mip of the weather map the march reads at least: 2^3.5 texels of 8 m, about 90 m.
const float PRESENCE_MIN_LOD = 3.5;

float remap(float value, float from_low, float from_high, float to_low, float to_high) {
	return to_low + (value - from_low) / (from_high - from_low) * (to_high - to_low);
}

#include "weather_cloud_map.glslinc"

// The 3D noise tiles every BODY_TILE_M along the world axes. Under a broken sky the weather map
// hides that, but a closed deck shows the noise everywhere, and its repeats lined up into rows
// that converge on the horizon. A smooth index field instead picks a random placement of the
// tile every few hundred metres, and each point blends the two placements it lies between
// (Quilez, "texture repetition", technique 3). The blend leans on the two reads' difference so
// the change follows the noise's own features instead of a straight seam.
const float SHUFFLE_CELL_M = 1100.0;
const float SHUFFLE_STEPS = 8.0;

float shuffle_hash(uint n) {
	n = (n ^ 61u) ^ (n >> 16u);
	n *= 9u;
	n ^= n >> 4u;
	n *= 0x27d4eb2du;
	n ^= n >> 15u;
	return float(n >> 8u) / 16777216.0;
}

// Smooth value noise in 0 to 1 with one lattice cell per SHUFFLE_CELL_M. Not periodic: its
// input is the drifting noise position, which the Rust drift wraps at 65536 m, and a jump
// there is one more placement change.
float shuffle_index(vec2 xz) {
	vec2 q = xz / SHUFFLE_CELL_M;
	uvec2 c = uvec2(ivec2(floor(q)) + ivec2(1 << 20));
	vec2 f = fract(q);
	f = f * f * (3.0 - 2.0 * f);
	float a = shuffle_hash(c.x * 0x8da6b343u ^ c.y * 0xd8163841u);
	float b = shuffle_hash((c.x + 1u) * 0x8da6b343u ^ c.y * 0xd8163841u);
	float d = shuffle_hash(c.x * 0x8da6b343u ^ (c.y + 1u) * 0xd8163841u);
	float e = shuffle_hash((c.x + 1u) * 0x8da6b343u ^ (c.y + 1u) * 0xd8163841u);
	return mix(mix(a, b, f.x), mix(d, e, f.x), f.y);
}

vec2 shuffle_offset(float index) {
	uint i = uint(index);
	return vec2(shuffle_hash(i * 2u + 1u), shuffle_hash(i * 2u + 2u));
}

// The 3D noise at `position / tile` with its xz placement shuffled. One read outside the blend
// bands. The difference term moves the blend by at most 0.1, so below 0.05 and above 0.95 the
// weight is exactly 0 or 1 and the other read is skipped.
vec4 shuffled_noise(vec3 position, float tile, float index) {
	float level = index * SHUFFLE_STEPS;
	float step_index = floor(level);
	float f = level - step_index;
	vec3 uvw = position / tile;
	if (f > 0.95) {
		return textureLod(noise_texture, uvw + vec3(shuffle_offset(step_index + 1.0), 0.0).xzy, 0.0);
	}
	vec4 a = textureLod(noise_texture, uvw + vec3(shuffle_offset(step_index), 0.0).xzy, 0.0);
	if (f < 0.05) {
		return a;
	}
	vec4 b = textureLod(noise_texture, uvw + vec3(shuffle_offset(step_index + 1.0), 0.0).xzy, 0.0);
	return mix(a, b, smoothstep(0.15, 0.85, f - 0.1 * (b.r - a.r)));
}

// Cloud density at a point, 0 to 1. `detail` adds the erosion read.
float cloud_density(vec3 p, float lod, bool detail, vec3 regional) {
	float height = (p.y - CLOUD_BASE_M) / CLOUD_THICKNESS_M;
	if (height <= 0.0 || height >= 1.0) {
		return 0.0;
	}
	// Blurred to a few hundred metres: the map's own fine lobes, extruded upwards, fluted every
	// flank into vertical streaks. The 3D noise supplies the detail instead.
	float presence = weather_presence(p.xz, max(lod, PRESENCE_MIN_LOD), regional);
	if (presence <= 0.01) {
		return 0.0;
	}
	// Denser columns tower higher. The column's presence shrinks towards its top as a dome, so
	// the noise is carved ever narrower with height: a flat base and rounded flanks, where
	// carving by the column's presence alone extruded every cloud into straight walls.
	float top = mix(0.2, 0.9, presence * presence);
	float dome = sqrt(clamp(1.0 - (height / top) * (height / top), 0.0, 1.0));
	float local = presence * dome * smoothstep(0.0, 0.06, height);
	if (local <= 0.01) {
		return 0.0;
	}
	vec3 noise_position = vec3(p.x + params.clouds.x, p.y, p.z + params.clouds.y);
	float index = shuffle_index(noise_position.xz);
	vec4 body = shuffled_noise(noise_position, BODY_TILE_M, index);
	float worley = body.g * 0.625 + body.b * 0.25 + body.a * 0.125;
	// Stretched to the full range: the raw remap spans only 0.70 to 0.96 (5th to 95th
	// percentile of the baked texture), which the coverage threshold never carved.
	float shape = clamp(remap(remap(body.r, worley - 1.0, 1.0, 0.0, 1.0), 0.70, 0.96, 0.0, 1.0),
		0.0, 1.0);
	// Coverage carves the noise: thin columns keep only its peaks.
	float density = clamp(remap(shape, 1.0 - local, 1.0, 0.0, 1.0), 0.0, 1.0) * local;
	if (density <= 0.0 || !detail) {
		return density;
	}
	vec4 fine = shuffled_noise(noise_position, DETAIL_TILE_M, index);
	float erosion = clamp(remap(fine.g * 0.625 + fine.b * 0.25 + fine.a * 0.125, 0.50, 0.88,
		0.0, 1.0), 0.0, 1.0);
	// Wispy at the base, billowy at the top.
	erosion = mix(erosion, 1.0 - erosion, clamp(height * 4.0, 0.0, 1.0));
	return clamp(remap(density, erosion * 0.5, 1.0, 0.0, 1.0), 0.0, 1.0);
}

float henyey_greenstein(float cos_angle, float g) {
	float g2 = g * g;
	return (1.0 - g2) / (4.0 * 3.14159265 * pow(1.0 + g2 - 2.0 * g * cos_angle, 1.5));
}

// Optical depth from `p` towards the sun, read from the body noise only.
float sun_optical_depth(vec3 p, vec3 sun, float lod, vec3 regional) {
	float depth = 0.0;
	float previous = 0.0;
	const float DISTANCES[6] = float[](40.0, 110.0, 220.0, 380.0, 620.0, 1000.0);
	for (int i = 0; i < 6; i++) {
		float d = DISTANCES[i];
		depth += cloud_density(p + sun * d, lod, false, regional) * (d - previous);
		previous = d;
	}
	return depth;
}

float interleaved_gradient_noise(vec2 pixel, float frame) {
	pixel += 5.588238 * mod(frame, 64.0);
	return fract(52.9829189 * fract(dot(pixel, vec2(0.06711056, 0.00583715))));
}

// Scattered light and transmittance along `eye` from `origin`.
vec4 march(vec3 origin, vec3 eye, int steps, float jitter, bool detail) {
	vec3 haze_colour = params.horizon.rgb;
	float coverage = params.clouds.z;
	float deck = smoothstep(0.8, 1.0, coverage);
	if (eye.y <= 0.002 || coverage <= 0.0) {
		return vec4(0.0, 0.0, 0.0, 1.0);
	}
	float enter_m = max((CLOUD_BASE_M - origin.y) / eye.y, 0.0);
	float exit_m = (CLOUD_BASE_M + CLOUD_THICKNESS_M - origin.y) / eye.y;
	// Below the horizon fade the layer is only a haze band, as dense as the deck. The
	// march fades into the band over the last fifth of its reach; a hard cut drew a stepped edge.
	float fade = smoothstep(0.004, 0.04, eye.y)
		* (1.0 - smoothstep(0.8 * MAX_BASE_DISTANCE_M, MAX_BASE_DISTANCE_M, enter_m));
	// The band is the haze the far march converges to, so it meets the sky below and the march
	// above without a seam; the deck's own shade drew a grey stripe at night and dusk.
	vec3 band = haze_colour * deck * 0.85;
	if (fade <= 0.0) {
		return vec4(band, 1.0 - deck * 0.85);
	}
	exit_m = min(exit_m, enter_m + MAX_PATH_M);
	float step_m = (exit_m - enter_m) / float(steps);
	float lod = log2(max(step_m / SHAPE_TEXEL_M, 1.0));
	bool fine = detail && enter_m < 18000.0;

	vec3 sun = normalize(params.sun.xyz);
	float cos_angle = dot(eye, sun);
	// Mostly isotropic, as multiple scattering makes a thick cloud, plus a forward lobe for the
	// silver lining around a cloud in front of the sun.
	float phase = 0.75 + 0.5 * henyey_greenstein(cos_angle, 0.6) * 4.0 * 3.14159265;
	float sigma_scale = EXTINCTION_PER_M * params.clouds.w;
	// A closed deck is lit from above, so it greys and a storm darkens.
	vec3 neutral = vec3(dot(params.shade.rgb, vec3(0.2126, 0.7152, 0.0722)));
	// Rain clouds are darker still: water-laden, they absorb as well as scatter.
	vec3 ambient = mix(params.shade.rgb, neutral, deck * 0.6)
		* mix(1.0, 0.62, smoothstep(2.2, 4.0, params.clouds.w))
		* mix(1.0, 0.65, params.high.w);
	vec3 sun_light = max(params.light.rgb - params.shade.rgb, vec3(0.0)) * 3.0
		* mix(1.0, 0.35, deck) * params.sun.w;

	float transmittance = 1.0;
	vec3 scattered = vec3(0.0);
	float weighted_distance = 0.0;
	float weight = 0.0;
	for (int i = 0; i < steps; i++) {
		float t = enter_m + (float(i) + jitter) * step_m;
		vec3 p = origin + eye * t;
		if (p.y <= CLOUD_BASE_M || p.y >= CLOUD_BASE_M + CLOUD_THICKNESS_M) {
			continue;
		}
		vec3 regional = weather_region(p.xz);
		float density = cloud_density(p, lod, fine, regional);
		if (density <= 0.0) {
			continue;
		}
		float sigma = density * sigma_scale;
		float height = (p.y - CLOUD_BASE_M) / CLOUD_THICKNESS_M;
		// A raining deck is dark underneath: its base is lit only through the whole deck above,
		// thicker with water than the optical depth alone accounts for.
		vec3 luminance = ambient * mix(mix(0.4, 0.18, params.high.w), 1.0, height);
		// By moonlight sun_light is zero, so its six density samples are skipped; the branch is
		// the same for every texel.
		if (params.sun.w > 0.0) {
			float light_depth = sun_optical_depth(p, sun, lod, regional) * sigma_scale;
			// Beer towards the sun, with the powder term that darkens the sunlit skin's interior.
			float beer = max(exp(-light_depth), exp(-light_depth * 0.25) * 0.7);
			float powder = 1.0 - exp(-light_depth * 2.0);
			luminance += sun_light * beer * mix(0.6, 1.0, powder) * phase;
		}
		float step_transmittance = exp(-sigma * step_m);
		// Energy-conserving integration of the luminance over the step.
		scattered += transmittance * luminance * (1.0 - step_transmittance);
		weighted_distance += transmittance * (1.0 - step_transmittance) * t;
		weight += transmittance * (1.0 - step_transmittance);
		transmittance *= step_transmittance;
		if (transmittance < 0.01) {
			break;
		}
	}
	float opacity = 1.0 - transmittance;
	float distance_m = weight > 0.0 ? weighted_distance / weight : enter_m;
	float haze = 1.0 - exp(-distance_m / HAZE_DISTANCE_M);
	scattered = mix(scattered, haze_colour * opacity, haze);
	return mix(vec4(band, 1.0 - deck * 0.85), vec4(scattered, transmittance), fade);
}

// Lambert equal-area disk of the upper hemisphere; the water inverts this in
// weather_cloud_reflected().
// Cirrus: a thin sheet of ice cloud far above the main layer, drawn as one translucent surface
// rather than marched. Its streaks are the weather map read stretched along the world diagonal
// (x + z), near the prevailing high wind; a diagonal keeps every lookup period a divisor of the
// 65536 m offset wrap, so the drift wraps seamlessly, where turning the streaks with the wind
// would sweep the far sky faster than any wind. Broad patches (a 2048 m by 46 km period along
// the streaks) carry fine fibres (512 m by 5.8 km).
const float HIGH_CLOUD_ALTITUDE_M = 8000.0;
const float HIGH_CLOUD_MAX_OPACITY = 0.6;
const vec2 HIGH_PATCH_PERIOD_M = vec2(65536.0, 16384.0);
const vec2 HIGH_FIBRE_PERIOD_M = vec2(8192.0, 1024.0);

// Light from the high sheet along `eye` in rgb and its opacity in a.
vec4 high_clouds(vec3 origin, vec3 eye, bool panorama) {
	float cover = params.high.z;
	float fade = smoothstep(0.01, 0.08, eye.y);
	if (cover <= 0.0 || fade <= 0.0) {
		return vec4(0.0);
	}
	float t = (HIGH_CLOUD_ALTITUDE_M - origin.y) / eye.y;
	vec2 p = origin.xz + eye.xz * t + params.high.xy;
	// Along and across the diagonal, unnormalized so both stay whole multiples of the wrap.
	vec2 q = vec2(p.x + p.y, p.y - p.x);
	// A quarter-resolution texel spans about 0.2 degrees, a panorama texel about 0.7; both widen
	// along a grazing ray.
	float footprint_m = t * (panorama ? 0.012 : 0.0035) / max(eye.y, 0.05);
	float patch_lod = log2(max(footprint_m / (HIGH_PATCH_PERIOD_M.y / 1024.0), 1.0));
	float fibre_lod = log2(max(footprint_m / (HIGH_FIBRE_PERIOD_M.y / 1024.0), 1.0));
	vec2 patches = textureLod(shape_texture, q / HIGH_PATCH_PERIOD_M + vec2(0.17, 0.43), patch_lod).rg;
	float fibre = textureLod(shape_texture, q / HIGH_FIBRE_PERIOD_M + vec2(0.61, 0.29), fibre_lod).r;
	// Fibres the pixel cannot resolve fade to their mean rather than to noise.
	fibre = mix(fibre, 0.5, smoothstep(4.0, 7.0, fibre_lod));
	float threshold = mix(0.70, 0.26, cover);
	float body = smoothstep(threshold, threshold + 0.22, patches.x * 0.8 + patches.y * 0.2);
	float density = body * smoothstep(0.25, 0.75, fibre + 0.25 * body);
	float opacity = density * HIGH_CLOUD_MAX_OPACITY * mix(0.6, 1.0, cover) * fade;
	if (opacity <= 0.0) {
		return vec4(0.0);
	}
	// Thin ice cloud is mostly sunlit, with a strong forward glow near the sun; by moonlight
	// it takes the dim night palette.
	vec3 sun = normalize(params.sun.xyz);
	float forward = henyey_greenstein(dot(eye, sun), 0.7) * 4.0 * 3.14159265;
	vec3 colour = mix(params.shade.rgb, params.light.rgb, 0.75)
		* (1.0 + 0.25 * min(forward, 6.0) * params.sun.w);
	float haze = 1.0 - exp(-t / (HAZE_DISTANCE_M * 2.0));
	colour = mix(colour, params.horizon.rgb, haze);
	return vec4(colour, opacity);
}

vec3 panorama_direction(vec2 uv) {
	vec2 d = uv * 2.0 - 1.0;
	float r2 = dot(d, d);
	if (r2 > 1.0) {
		return vec3(0.0, -1.0, 0.0);
	}
	vec2 xz = d * sqrt(2.0 - r2);
	return normalize(vec3(xz.x, 1.0 - r2, xz.y));
}

void main() {
	ivec2 pixel = ivec2(gl_GlobalInvocationID.xy);
	ivec2 size = ivec2(params.target.xy);
	if (any(greaterThanEqual(pixel, size))) {
		return;
	}
	vec2 uv = (vec2(pixel) + 0.5) / vec2(size);
	float jitter = interleaved_gradient_noise(vec2(pixel), params.camera_position.w);
	bool panorama = params.target.w > 0.5;
	// The reflected clouds change slowly and the water's ripples hide fine detail, so the panorama
	// marches one workgroup of each 2x2 a frame and carries the other three from its history.
	// Whole workgroups, not texels: a group with one marching texel costs as much as a full one.
	if (panorama && params.target.z > 0.5) {
		int phase = int(mod(params.camera_position.w, 4.0));
		ivec2 group = ivec2(gl_WorkGroupID.xy);
		if ((group.x & 1) + 2 * (group.y & 1) != phase) {
			imageStore(output_image, pixel, textureLod(history_texture, uv, 0.0));
			return;
		}
	}
	vec3 eye;
	vec3 origin = params.camera_position.xyz;
	// An orthographic projection is affine: its inverse leaves w alone.
	bool orthographic = abs(params.inv_projection[2][3]) < 1e-9;
	if (panorama) {
		eye = panorama_direction(uv);
	} else {
		// The camera projection the compositor hands over already flips y for the render target,
		// so image rows map straight onto clip-space y. The ray runs from the near plane
		// (reversed Z: 1) through a deeper point, so an orthographic camera's rays stay parallel,
		// each from its own pixel; a perspective camera's all start at the camera.
		vec4 near = params.inv_projection * vec4(uv * 2.0 - 1.0, 1.0, 1.0);
		vec4 deeper = params.inv_projection * vec4(uv * 2.0 - 1.0, 0.5, 1.0);
		vec3 near_view = near.xyz / near.w;
		eye = normalize(mat3(params.camera_basis) * (deeper.xyz / deeper.w - near_view));
		origin += mat3(params.camera_basis) * near_view;
	}
	vec4 current = march(origin, eye, panorama ? 24 : 48, jitter, !panorama);
	// The main layer lies in front of the high one.
	vec4 high = high_clouds(origin, eye, panorama);
	current.rgb += current.a * high.rgb * high.a;
	current.a *= 1.0 - high.a;

	vec2 history_uv = uv;
	bool history_valid = params.target.z > 0.5;
	// Rotation-only reprojection projects a direction, which an orthographic camera cannot: its
	// history is read at the same pixel, as the clouds are far enough to barely move under it.
	if (!panorama && history_valid && !orthographic) {
		vec4 clip = params.previous_view_projection * vec4(eye, 0.0);
		history_valid = clip.w > 0.0;
		history_uv = clip.xy / clip.w * 0.5 + 0.5;
		history_valid = history_valid && all(greaterThanEqual(history_uv, vec2(0.0)))
			&& all(lessThanEqual(history_uv, vec2(1.0)));
	}
	if (history_valid) {
		vec4 history = textureLod(history_texture, history_uv, 0.0);
		current = mix(current, history, HISTORY_WEIGHT);
	}
	imageStore(output_image, pixel, current);
}
