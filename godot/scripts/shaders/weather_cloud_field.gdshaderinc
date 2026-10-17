// SPDX-License-Identifier: GPL-2.0-only

// Procedural cloud shape, evaluated once per texel by weather_cloud_shape_bake.gdshader and never
// per frame. Every noise lattice divides WEATHER_CLOUD_PERIOD_M, so the baked texture tiles.
#ifndef WEATHER_CLOUD_FIELD_INCLUDED
#define WEATHER_CLOUD_FIELD_INCLUDED

// Side of one tile of the cloud field, in metres. Must divide the Rust offset wrap (65536 m).
const float WEATHER_CLOUD_PERIOD_M = 8192.0;

float weather_hash(uvec2 cell) {
	uint hashed = cell.x * 0x9e3779b9u ^ cell.y * 0x85ebca6bu;
	hashed ^= hashed >> 16u;
	hashed *= 0x7feb352du;
	hashed ^= hashed >> 15u;
	hashed *= 0x846ca68bu;
	return float((hashed ^ (hashed >> 16u)) >> 8u) / 16777216.0;
}

// Periodic value noise with `cells` lattice cells per period.
float weather_noise(vec2 position_m, float cells) {
	vec2 p = mod(position_m, vec2(WEATHER_CLOUD_PERIOD_M)) * cells / WEATHER_CLOUD_PERIOD_M;
	vec2 cell = floor(p);
	vec2 f = fract(p);
	f = f * f * (3.0 - 2.0 * f);
	float a = weather_hash(uvec2(mod(cell, vec2(cells))));
	float b = weather_hash(uvec2(mod(cell + vec2(1, 0), vec2(cells))));
	float c = weather_hash(uvec2(mod(cell + vec2(0, 1), vec2(cells))));
	float d = weather_hash(uvec2(mod(cell + vec2(1, 1), vec2(cells))));
	return mix(mix(a, b, f.x), mix(c, d, f.x), f.y);
}

// Periodic inverted Worley noise: 1 at a feature point, falling to 0 a cell away. Gives the
// rounded, cauliflower lobes that value noise alone smears into soft blobs.
float weather_worley(vec2 position_m, float cells) {
	vec2 q = mod(position_m, vec2(WEATHER_CLOUD_PERIOD_M)) * cells / WEATHER_CLOUD_PERIOD_M;
	vec2 cell = floor(q);
	float best = 4.0;
	for (int y = -1; y <= 1; y++) {
		for (int x = -1; x <= 1; x++) {
			vec2 neighbor = cell + vec2(float(x), float(y));
			vec2 wrapped = mod(neighbor, vec2(cells));
			vec2 jitter = vec2(weather_hash(uvec2(wrapped)),
				weather_hash(uvec2(mod(wrapped + vec2(37.0, 17.0), vec2(cells)))));
			vec2 delta = neighbor + vec2(0.15) + 0.7 * jitter - q;
			best = min(best, dot(delta, delta));
		}
	}
	return 1.0 - clamp(sqrt(best), 0.0, 1.0);
}

// Coverage-independent cloud shape at one position: x is the domain-warped macro field that
// coverage thresholds into cloud bodies, y the Worley lobes that erode their edges.
vec2 weather_cloud_shape(vec2 position_m) {
	vec2 warp = vec2(weather_noise(position_m, 4.0),
		weather_noise(position_m + vec2(1931.0, 719.0), 4.0)) - vec2(0.5);
	vec2 p = position_m + warp * 600.0;
	float macro_shape = weather_noise(p, 8.0) * 0.52
		+ weather_noise(p, 16.0) * 0.26
		+ weather_noise(p, 32.0) * 0.13
		+ weather_noise(p, 64.0) * 0.06
		+ weather_noise(p, 128.0) * 0.03;
	macro_shape = 0.8 * macro_shape + 0.2 * weather_worley(p, 16.0);
	float lobes = 0.7 * weather_worley(p, 32.0) + 0.3 * weather_worley(p, 64.0);
	return vec2(macro_shape, lobes);
}

// Two smooth fields independent of the shape, read at the regional scale to pick which
// placement of the map a region uses.
vec2 weather_cloud_region_fields(vec2 position_m) {
	return vec2(
		weather_noise(position_m + vec2(3001.0, 1337.0), 8.0) * 0.65
			+ weather_noise(position_m + vec2(3001.0, 1337.0), 16.0) * 0.35,
		weather_noise(position_m + vec2(523.0, 4111.0), 8.0) * 0.65
			+ weather_noise(position_m + vec2(523.0, 4111.0), 16.0) * 0.35);
}
#endif
