#[compute]
#version 450

// SPDX-License-Identifier: GPL-2.0-only

// Bakes the clouds' sun transmission over a square window of the deck around the camera, once
// per frame, before the opaque pass. Every receiver then reads one bilinear texel instead of
// evaluating the weather map itself; the trees did that per vertex, thousands of times a crown.
// Same formula as weather_cloud_shadow() in weather_clouds.gdshaderinc, at the penumbra of a
// receiver on the camera's ground, which the receivers fall back to outside the window.
layout(local_size_x = 8, local_size_y = 8, local_size_z = 1) in;

layout(push_constant, std430) uniform Params {
	// xy: world X/Z of texel (0, 0)'s corner on the deck. z: metres per texel. w: separation
	// between the reference receiver and the shadow-casting height, for the penumbra.
	vec4 window;
	// As the weather_clouds global: xy offset, z coverage, w optical depth.
	vec4 clouds;
	// xyz: towards the sun.
	vec4 sun;
} params;

layout(set = 0, binding = 0) uniform sampler2D shape_texture;
layout(r16f, set = 0, binding = 1) uniform restrict writeonly image2D output_image;

#include "weather_cloud_map.glslinc"

// Must match weather_clouds.gdshaderinc.
const float SHADOW_EXTINCTION = 3.0;
const float DECK_MEAN_DENSITY = 0.73;
const float SHAPE_TEXELS = 1024.0;

void main() {
	ivec2 pixel = ivec2(gl_GlobalInvocationID.xy);
	if (any(greaterThanEqual(pixel, imageSize(output_image)))) {
		return;
	}
	vec2 deck = params.window.xy + (vec2(pixel) + 0.5) * params.window.z;
	vec3 sun = normalize(params.sun.xyz);
	float penumbra = params.window.w * 0.04 / max(sun.y, 0.2);
	float lod = clamp(log2(max(penumbra / SHAPE_TEXEL_M, 1.0)), 0.0, log2(SHAPE_TEXELS));
	float density = weather_presence(deck, lod, weather_region(deck));
	density = mix(density, DECK_MEAN_DENSITY, smoothstep(0.8, 1.0, params.clouds.z) * 0.65);
	float transmission = exp(-SHADOW_EXTINCTION * params.clouds.w * density / max(sun.y, 0.08));
	imageStore(output_image, pixel, vec4(transmission, 0.0, 0.0, 1.0));
}
