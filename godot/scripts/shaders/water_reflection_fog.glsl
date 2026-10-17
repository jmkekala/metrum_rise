#[compute]
#version 450

// SPDX-License-Identifier: GPL-2.0-only

// The distance fade of the planar water reflection's pass (water_reflection.gd), as coverage
// rather than colour. The main view fades far ground into the fog colour, which matches the sky
// behind it; the water reflects the sky by its own means, brighter than the fog colour at the
// scene's reflection strength, so a hill the fog had hidden reflected as a dark silhouette. Here
// the fade takes the scene out of the map instead: the colour and its coverage both scale by
// what the fade leaves, so the water shows the sky through the share the fog would have
// covered. The map is premultiplied by that coverage, which also keeps its bilinear reads
// between scene and empty texels from darkening the scene's edges.
layout(local_size_x = 8, local_size_y = 8, local_size_z = 1) in;

layout(rgba16f, set = 0, binding = 0) uniform restrict image2D color_image;
layout(set = 0, binding = 1) uniform sampler2D depth_texture;

layout(set = 0, binding = 2, std140) uniform Params {
	mat4 inv_projection;
	// x: fade begin, metres. y: fade end, metres. z: curve. w: density.
	vec4 fade;
	// xy: render size.
	vec4 target;
} params;

void main() {
	ivec2 pixel = ivec2(gl_GlobalInvocationID.xy);
	if (any(greaterThanEqual(pixel, ivec2(params.target.xy)))) {
		return;
	}
	vec4 color = imageLoad(color_image, pixel);
	if (color.a <= 0.0) {
		return;
	}
	vec2 uv = (vec2(pixel) + 0.5) / params.target.xy;
	float depth = max(texelFetch(depth_texture, pixel, 0).r, 1e-7);
	vec4 view = params.inv_projection * vec4(uv * 2.0 - 1.0, depth, 1.0);
	// Godot's depth fog: smoothstep over the distance from the camera, raised to the curve.
	float fade = smoothstep(params.fade.x, params.fade.y, length(view.xyz / view.w));
	float keep = 1.0 - clamp(pow(fade, params.fade.z) * params.fade.w, 0.0, 1.0);
	imageStore(color_image, pixel, color * keep);
}
