#[compute]
#version 450

// SPDX-License-Identifier: GPL-2.0-only

// Lays the low-resolution cloud buffer over the sky, after the sky is drawn and before the
// transparent pass, so the water's screen reads see it. Clouds stand behind every surface, so
// only pixels where no geometry was drawn take them.
layout(local_size_x = 8, local_size_y = 8, local_size_z = 1) in;

layout(rgba16f, set = 0, binding = 0) uniform restrict image2D color_image;
layout(set = 0, binding = 1) uniform sampler2D depth_texture;
layout(set = 0, binding = 2) uniform sampler2D cloud_texture;

layout(push_constant, std430) uniform Params {
	vec2 size;
	vec2 padding;
} params;

void main() {
	ivec2 pixel = ivec2(gl_GlobalInvocationID.xy);
	if (any(greaterThanEqual(pixel, ivec2(params.size)))) {
		return;
	}
	// Reversed Z: the cleared far plane is 0, and the sky writes no depth.
	if (texelFetch(depth_texture, pixel, 0).r > 0.0) {
		return;
	}
	vec4 cloud = textureLod(cloud_texture, (vec2(pixel) + 0.5) / params.size, 0.0);
	vec4 color = imageLoad(color_image, pixel);
	color.rgb = color.rgb * cloud.a + cloud.rgb;
	imageStore(color_image, pixel, color);
}
