#[compute]
#version 450

// SPDX-License-Identifier: GPL-2.0-only

// Morning mist over the lakes, laid over the finished frame after the transparent pass, so the
// water is already drawn. Mist density falls off exponentially with height above the water it
// lies on and is zero below that water: a ray that ends on a lake bed seen through the water is
// integrated only down to the surface, which the opaque depth alone does not record.
//
// Each pixel's ray, from the camera to its depth (or a fixed distance for the sky), is cut to
// the part below the mist's ceiling and split into SEGMENTS. Each segment reads the mist map at
// its midpoint, at the mip its horizontal length spans, and integrates the height profile over
// its own rise in closed form, clipped at the water level.
layout(local_size_x = 8, local_size_y = 8, local_size_z = 1) in;

layout(rgba16f, set = 0, binding = 0) uniform restrict image2D color_image;
layout(set = 0, binding = 1) uniform sampler2D depth_texture;
layout(set = 0, binding = 2) uniform sampler2D mist_map;
// The clouds' tiling shape noise, reused for the mist banks.
layout(set = 0, binding = 4) uniform sampler2D bank_texture;

layout(set = 0, binding = 3, std140) uniform Params {
	mat4 inv_projection;
	// Camera-to-world transform.
	mat4 camera_transform;
	// xy: world X/Z of mist cell (0, 0)'s centre. z: metres per cell. w: mip levels in the map.
	vec4 map_origin;
	// xy: map size in cells. z: extinction per metre at the water surface over open water.
	// w: height in metres over which the density falls by e.
	vec4 map_size;
	// xyz: mist colour, lit. w: height of the mist ceiling in metres.
	vec4 mist_color;
	// xyz: towards the sun. w: forward-scatter strength.
	vec4 sun;
	// xyz: the sun's light colour. w: unused.
	vec4 sun_color;
	// xy: render size. zw: distance the wind has carried the air, metres (weather_wind.zw).
	vec4 target;
} params;

const int SEGMENTS = 6;
// Mist lies in banks a few hundred metres across with clear lanes between, carried by the wind:
// the shape noise's 8192 m tile shrunk to this, whose lobes are then 100-500 m.
const float BANK_TILE_M = 2048.0;
// Where a sky ray stops: past the far fade, which already hides the ground there.
const float SKY_DISTANCE_M = 12000.0;

// How thick the mist bank over a point is, in [0.35, 1]: one read of the clouds' baked shape
// noise (weather_cloud_shape.gd), stretched so a bank is a few hundred metres across.
float bank(vec2 world_xz) {
	vec2 uv = (world_xz - params.target.zw) / BANK_TILE_M;
	return mix(0.35, 1.0, smoothstep(0.45, 0.85, textureLod(bank_texture, uv, 1.0).r));
}

// Integral over one straight segment of exp(-(y - level) / scale) where y is at or above
// `level`, zero below: the segment runs `length_m` from height `y0` to `y1`.
float layer_depth(float y0, float y1, float length_m, float level, float scale) {
	float low = min(y0, y1);
	float high = max(y0, y1);
	if (high <= level) {
		return 0.0;
	}
	// The share of the segment that lies above the water.
	float rise = high - low;
	float above = rise > 1e-3 ? (high - max(low, level)) / rise : 1.0;
	float a = max(low, level) - level;
	float b = high - level;
	if (b - a < 1e-3) {
		return length_m * above * exp(-0.5 * (a + b) / scale);
	}
	// Mean of the exponential over [a, b], times the length that lies above the water.
	float mean = scale * (exp(-a / scale) - exp(-b / scale)) / (b - a);
	return length_m * above * mean;
}

// World position at a screen uv and a reversed-Z depth. The projection the compositor hands over
// already flips y for the render target.
vec3 world_point(vec2 uv, float depth) {
	vec4 view = params.inv_projection * vec4(uv * 2.0 - 1.0, depth, 1.0);
	return (params.camera_transform * vec4(view.xyz / view.w, 1.0)).xyz;
}

void main() {
	ivec2 pixel = ivec2(gl_GlobalInvocationID.xy);
	if (any(greaterThanEqual(pixel, ivec2(params.target.xy)))) {
		return;
	}
	vec2 uv = (vec2(pixel) + 0.5) / params.target.xy;
	float depth = texelFetch(depth_texture, pixel, 0).r;
	// The ray starts on the near plane (reversed Z: depth 1), so an orthographic camera's
	// parallel rays each start at their own pixel; a perspective one's all start at the camera.
	vec3 origin = world_point(uv, 1.0);
	vec3 point = world_point(uv, max(depth, 1e-7));
	vec3 ray = point - origin;
	float distance_m = depth > 0.0 ? length(ray) : SKY_DISTANCE_M;
	vec3 dir = normalize(ray);

	// The part of the ray below the mist ceiling.
	float ceiling = params.mist_color.w;
	float t0 = 0.0;
	float t1 = distance_m;
	if (abs(dir.y) < 1e-5) {
		if (origin.y > ceiling) {
			return;
		}
	} else {
		float t_ceiling = (ceiling - origin.y) / dir.y;
		if (dir.y < 0.0) {
			t0 = max(t0, t_ceiling);
		} else {
			t1 = min(t1, t_ceiling);
		}
	}
	if (t1 <= t0) {
		return;
	}

	float step_m = (t1 - t0) / float(SEGMENTS);
	float cell_m = params.map_origin.z;
	float lod = clamp(log2(max(step_m * length(dir.xz) / cell_m, 1.0)), 0.0, params.map_origin.w);
	float optical = 0.0;
	for (int i = 0; i < SEGMENTS; i++) {
		float ta = t0 + step_m * float(i);
		vec3 a = origin + dir * ta;
		vec3 b = a + dir * step_m;
		vec2 mid = 0.5 * (a.xz + b.xz);
		vec2 map_uv = ((mid - params.map_origin.xy) / cell_m + 0.5) / params.map_size.xy;
		vec2 mist = textureLod(mist_map, map_uv, lod).rg;
		if (mist.r <= 0.0) {
			continue;
		}
		// A blended level near a shore is weighted by presence; divide it back out.
		float level = mist.g / max(mist.r, 1e-3);
		optical += mist.r * bank(mid) * layer_depth(a.y, b.y, step_m, level, params.map_size.w);
	}
	float transmittance = exp(-params.map_size.z * optical);
	if (transmittance > 0.999) {
		return;
	}
	// Mist lit by the sky, brighter towards the sun.
	float towards_sun = max(dot(dir, normalize(params.sun.xyz)), 0.0);
	vec3 light = params.mist_color.rgb
		+ params.sun_color.rgb * params.sun.w * pow(towards_sun, 8.0);
	vec4 color = imageLoad(color_image, pixel);
	color.rgb = color.rgb * transmittance + light * (1.0 - transmittance);
	imageStore(color_image, pixel, color);
}
