#[compute]
#version 450

// SPDX-License-Identifier: GPL-2.0-only

// Bakes the tiling 3D cloud noise once at startup (weather_cloud_effect.gd). Every lattice wraps
// at the texture edge, so the clouds repeat seamlessly.
//   r: Perlin-Worley, billowy Perlin fbm remapped by Worley fbm, the cloud body shape.
//   g, b, a: Worley fbm at 4, 8 and 16 cells per tile, the erosion of its edges.
layout(local_size_x = 4, local_size_y = 4, local_size_z = 4) in;

layout(rgba8, set = 0, binding = 0) uniform restrict writeonly image3D noise_image;

vec3 hash33(vec3 cell) {
	uvec3 v = uvec3(ivec3(cell)) * uvec3(1597334673u, 3812015801u, 2798796415u);
	v = (v.x ^ v.y ^ v.z) * uvec3(1597334673u, 3812015801u, 2798796415u);
	return vec3(v) / float(0xffffffffu);
}

// Gradient noise with `period` cells per tile, in about [-1, 1].
float gradient_noise(vec3 x, float period) {
	vec3 p = floor(x);
	vec3 w = fract(x);
	vec3 u = w * w * w * (w * (w * 6.0 - 15.0) + 10.0);
	float result[8];
	for (int i = 0; i < 8; i++) {
		vec3 corner = vec3(float(i & 1), float((i >> 1) & 1), float((i >> 2) & 1));
		vec3 g = hash33(mod(p + corner, period)) * 2.0 - 1.0;
		result[i] = dot(g, w - corner);
	}
	float x0 = mix(mix(result[0], result[1], u.x), mix(result[2], result[3], u.x), u.y);
	float x1 = mix(mix(result[4], result[5], u.x), mix(result[6], result[7], u.x), u.y);
	return mix(x0, x1, u.z) * 1.6;
}

// Inverted Worley noise: 1 at a feature point, falling to 0 a cell away.
float worley(vec3 uvw, float period) {
	vec3 id = floor(uvw * period);
	vec3 p = fract(uvw * period);
	float best = 1e4;
	for (int z = -1; z <= 1; z++) {
		for (int y = -1; y <= 1; y++) {
			for (int x = -1; x <= 1; x++) {
				vec3 offset = vec3(float(x), float(y), float(z));
				vec3 d = p - (hash33(mod(id + offset, period)) + offset);
				best = min(best, dot(d, d));
			}
		}
	}
	return 1.0 - best;
}

float worley_fbm(vec3 uvw, float period) {
	return worley(uvw, period) * 0.625
		+ worley(uvw, period * 2.0) * 0.25
		+ worley(uvw, period * 4.0) * 0.125;
}

float perlin_fbm(vec3 uvw, float period) {
	float total = 0.0;
	float amplitude = 1.0;
	for (int i = 0; i < 7; i++) {
		total += amplitude * gradient_noise(uvw * period, period);
		period *= 2.0;
		amplitude *= 0.55;
	}
	return total;
}

void main() {
	ivec3 size = imageSize(noise_image);
	ivec3 id = ivec3(gl_GlobalInvocationID);
	if (any(greaterThanEqual(id, size))) {
		return;
	}
	vec3 uvw = (vec3(id) + 0.5) / vec3(size);
	float g = worley_fbm(uvw, 4.0);
	float b = worley_fbm(uvw, 8.0);
	float a = worley_fbm(uvw, 16.0);
	float perlin = abs(mix(1.0, perlin_fbm(uvw, 4.0), 0.5) * 2.0 - 1.0);
	// Remap the billowy Perlin by the Worley fbm, so cells round off into cauliflower lobes.
	float r = g + perlin * (1.0 - g);
	imageStore(noise_image, id, clamp(vec4(r, g, b, a), 0.0, 1.0));
}
