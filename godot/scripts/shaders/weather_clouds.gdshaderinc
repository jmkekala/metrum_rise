// SPDX-License-Identifier: GPL-2.0-only

// The cloud layer's 2D weather map, shared by the volumetric march (weather_cloud_march.glsl)
// and every shadow receiver, so a shadow always lies under the cloud that casts it. Its density
// comes from a baked, tiling shape texture thresholded by the current coverage.
//
// Motion arrives only through the numeric weather_* globals, never TIME. Godot does not mark a
// sky dirty for a numeric global write, so the clouds drift every frame while the radiance map
// stays cached (WTH-PROBE-SKY, M2 Pro, Godot 4.7.2).
#ifndef WEATHER_CLOUDS_INCLUDED
#define WEATHER_CLOUDS_INCLUDED

#include "res://scripts/shaders/weather_cloud_field.gdshaderinc"

// RG cloud shape and BA regional placement fields from weather_cloud_shape_bake.gdshader, one
// period per texture repeat.
global uniform sampler2D weather_cloud_shape_map : filter_linear_mipmap, repeat_enable;
// xy: world X/Z added before the lookup (seeded origin minus drift), metres.
// z: coverage in [0, 1]. w: vertical optical depth of a fully dense cloud.
global uniform vec4 weather_clouds;
// xyz: unit direction towards the true sun, even when the moon is the key light.
// w: share of the cloud shadow cast: 1 while the sun lights a simulated world, 0 by moonlight
// or in a scene with no simulation.
global uniform vec4 weather_sun;
// Hemisphere panorama of the clouds around the camera: scattered light and transmittance.
global uniform sampler2D weather_cloud_reflection : filter_linear, repeat_disable;
// Sun transmission baked each frame over a window of the deck by weather_cloud_shadow_effect.gd.
global uniform sampler2D weather_cloud_shadow_map : filter_linear, repeat_disable;
// xy: world X/Z of the window's corner on the deck. z: 1 / its side in metres. w: 1 while the
// bake is valid; 0 makes every receiver evaluate the weather map itself.
global uniform vec4 weather_cloud_shadow_window;

// Cloud base, above the gameplay camera's highest point (1000 m distance), so the camera never
// enters the layer. Both must match weather_cloud_march.glsl.
const float WEATHER_CLOUD_ALTITUDE_M = 1500.0;
// Tallest a cloud stands above the base. Fair-weather cumulus is a few hundred metres to over a
// kilometre deep.
const float WEATHER_CLOUD_THICKNESS_M = 1400.0;
const float WEATHER_CLOUD_TEXELS = 1024.0;
// The regional variation spans the whole offset wrap; its mip blurs the map's 64 m texels at
// that scale to about half a kilometre.
const float WEATHER_CLOUD_REGION_PERIOD_M = 65536.0;
const float WEATHER_CLOUD_REGION_LOD = 3.0;
// Optical depth of the shadow per unit of map density and of `weather_clouds.w`. At 1 a fair
// cloud's core still passed about a fifth of the sun and its body nearer half, so a cloud
// shadow read as a faint tint beside the cast shadow of a tree. At 3 a core takes the sun
// away as fully as a tree does, and only the thin edge lets some through.
const float WEATHER_CLOUD_SHADOW_EXTINCTION = 3.0;
// Sine of the sun elevation (0.3 degrees) below which clouds cast no shadow; it fades in up to
// about 1.1 degrees. The shadow used to fade out from 4.6 degrees down, so a sunset sun went
// behind a cloud and nothing darkened. Low shadows land tens of kilometres from their cloud,
// which the tiling map and the deck's own spread make no different from a near one.
const float WEATHER_CLOUD_SHADOW_MIN_SUN_Y = 0.005;
// Mean density of a closed deck; overcast shadows lean towards it so they read as dull light
// rather than as a pattern of dark patches.
const float WEATHER_CLOUD_DECK_MEAN_DENSITY = 0.73;

// Mip level for a lookup footprint in world metres.
float weather_cloud_lod(float footprint_m) {
	float texel_m = WEATHER_CLOUD_PERIOD_M / WEATHER_CLOUD_TEXELS;
	return clamp(log2(max(footprint_m / texel_m, 1.0)), 0.0, log2(WEATHER_CLOUD_TEXELS));
}

// Cloud density in [0, 1] at a point on the deck: one filtered texture read plus fixed ALU.
// weather_cloud_march.glsl repeats this formula as weather_presence().
//
// The baked map repeats every WEATHER_CLOUD_PERIOD_M, and from a high camera the horizon is
// 30-40 km away, so read plainly the same clouds stood in rows converging on it. A regional
// read of the same map, stretched to the 65536 m offset wrap, picks between four placements of
// it (shifted, transposed, mirrored, rotated) and shifts the coverage, so neighbouring tiles
// differ and the drift wrap stays seamless. Two texture reads away from the blend bands.
float weather_cloud_density(vec2 deck_xz, float lod) {
	vec2 world = deck_xz + weather_clouds.xy;
	vec2 uv = world / WEATHER_CLOUD_PERIOD_M;
	vec2 region_uv = world / WEATHER_CLOUD_REGION_PERIOD_M;
	vec4 fields = textureLod(weather_cloud_shape_map, region_uv + vec2(0.31, 0.67),
		WEATHER_CLOUD_REGION_LOD);
	float region = fields.r;
	float pick_a = smoothstep(0.45, 0.55, fields.b);
	float pick_b = smoothstep(0.45, 0.55, fields.a);
	// Four placements of the same map, each read only where its weight is not zero, which
	// away from the blend bands is one read.
	vec2 shape = vec2(0.0);
	float w0 = (1.0 - pick_a) * (1.0 - pick_b);
	float w1 = pick_a * (1.0 - pick_b);
	float w2 = (1.0 - pick_a) * pick_b;
	float w3 = pick_a * pick_b;
	if (w0 > 0.0) {
		shape += w0 * textureLod(weather_cloud_shape_map, uv, lod).rg;
	}
	if (w1 > 0.0) {
		shape += w1 * textureLod(weather_cloud_shape_map, uv.yx + vec2(0.5, 0.25), lod).rg;
	}
	if (w2 > 0.0) {
		shape += w2 * textureLod(weather_cloud_shape_map, vec2(-uv.x, uv.y) + vec2(0.25, 0.6), lod).rg;
	}
	if (w3 > 0.0) {
		shape += w3 * textureLod(weather_cloud_shape_map, vec2(uv.y, -uv.x) + vec2(0.7, 0.1), lod).rg;
	}
	float closed = smoothstep(0.8, 1.0, weather_clouds.z);
	// Up to about a tenth more or less cover by region; a closed deck stays closed.
	float coverage = clamp(weather_clouds.z + (region - 0.5) * 0.6 * (1.0 - closed), 0.0, 1.0);
	float threshold = 0.76 - 0.40 * coverage;
	float body = clamp((shape.x - threshold) / (0.26 + 0.12 * coverage), 0.0, 1.0);
	// Worley lobes eat into the thin edge of a body, rounding it, and leave its core alone.
	float eroded = clamp(body - (1.0 - shape.y) * 0.22 * (1.0 - body), 0.0, 1.0);
	return mix(smoothstep(0.0, 0.46, eroded), 0.46 + 0.54 * shape.x, closed);
}

// The clouds reflected along `direction` from near the camera: scattered light in rgb, opacity in
// a. Reads the hemisphere panorama weather_cloud_effect.gd marches each frame, which is centred
// on the camera; the parallax that leaves on water a few hundred metres away is lost in the
// ripples. One texture read.
vec4 weather_cloud_reflected(vec3 direction) {
	if (direction.y <= 0.0) {
		return vec4(0.0);
	}
	// Inverse of the march's Lambert equal-area disk.
	vec2 uv = direction.xz / sqrt(1.0 + direction.y) * 0.5 + 0.5;
	vec4 cloud = textureLod(weather_cloud_reflection, uv, 0.0);
	return vec4(cloud.rgb, 1.0 - cloud.a);
}

// Share of direct sunlight that reaches `world_position` through the deck. Multiply only direct
// solar light by it, never ambient, reflections or emission. One texture read. `footprint_m` is
// the world size of the receiving pixel, which softens far shadows instead of aliasing them.
float weather_cloud_shadow(vec3 world_position, float footprint_m) {
	vec3 sun = normalize(weather_sun.xyz);
	if (sun.y <= WEATHER_CLOUD_SHADOW_MIN_SUN_Y || weather_sun.w <= 0.0 || weather_clouds.z <= 0.0
			|| world_position.y >= WEATHER_CLOUD_ALTITUDE_M) {
		return 1.0;
	}
	// Cast from a third of the way up, where a typical cloud's mass sits.
	float separation = WEATHER_CLOUD_ALTITUDE_M + WEATHER_CLOUD_THICKNESS_M * 0.3
		- world_position.y;
	vec2 p = world_position.xz + sun.xz * separation / sun.y;
	float transmission;
	// The baked window, one bilinear texel; a texel's margin keeps the filter inside it.
	vec2 window_uv = (p - weather_cloud_shadow_window.xy) * weather_cloud_shadow_window.z;
	if (weather_cloud_shadow_window.w > 0.0 && all(greaterThan(window_uv, vec2(0.001)))
			&& all(lessThan(window_uv, vec2(0.999)))) {
		transmission = textureLod(weather_cloud_shadow_map, window_uv, 0.0).r;
	} else {
		// An artistic soft edge that grows with the distance to the deck, wider than the sun disk.
		float penumbra = separation * 0.04 / max(sun.y, 0.2);
		float density = weather_cloud_density(p, weather_cloud_lod(max(penumbra, footprint_m)));
		density = mix(density, WEATHER_CLOUD_DECK_MEAN_DENSITY,
			smoothstep(0.8, 1.0, weather_clouds.z) * 0.65);
		transmission = exp(-WEATHER_CLOUD_SHADOW_EXTINCTION * weather_clouds.w * density
			/ max(sun.y, 0.08));
	}
	return mix(1.0, transmission,
		smoothstep(WEATHER_CLOUD_SHADOW_MIN_SUN_Y, 0.02, sun.y) * weather_sun.w);
}
// Cloud transmission for the fragment at `world_position`, sized to its pixel. Call from
// fragment() and hand the result to light() through a varying; derivatives need fragment().
float weather_cloud_sun(vec3 world_position) {
	return weather_cloud_shadow(world_position, length(fwidth(world_position.xz)));
}
#endif
