// SPDX-License-Identifier: GPL-2.0-only

// Stars and the Milky Way, analytic in the visible sky and baked once in the celestial frame
// for water through night_sky_baked(). The 2048-square octahedral RGB9E5 map holds all four star
// layers and the glow; one mip-filtered lookup widens their footprints in reflections.
// The cloud pass hides them with its transmittance. They stay out of the radiance
// cubemap: a few thousand points of light do not light a city, and keeping them out means the
// turning sky never dirties the radiance map.
//
// Stars are drawn as finite Gaussian footprints, never as a hashed cell that is on or off per
// pixel. A footprint is at least about one pixel wide and keeps its total light as it widens,
// so a star neither flickers as the camera turns nor changes brightness with the resolution.
//
// Needs weather_sun from weather_clouds.gdshaderinc.
#ifndef NIGHT_SKY_INCLUDED
#define NIGHT_SKY_INCLUDED

// World-to-celestial rotation from day_cycle.gd: its columns are the rows of the rotation.
global uniform mat3 scene_celestial_basis;
global uniform sampler2D night_sky_map : filter_linear_mipmap, repeat_disable;

// Must match NightSkyBake.TEXELS.
const float NIGHT_SKY_TEXELS = 2048.0;

// Square root of local texel solid angle: the octahedron's radial projection has Jacobian
// 1 / length(p)^3, with p = c / dot(abs(c), vec3(1.0)). `c` is a unit direction.
float night_sky_texel_angle(vec3 c) {
	float l1 = dot(abs(c), vec3(1.0));
	return (2.0 / NIGHT_SKY_TEXELS) * l1 * sqrt(l1);
}

vec2 night_sky_oct_encode(vec3 c) {
	c /= dot(abs(c), vec3(1.0));
	vec2 uv = c.xy;
	if (c.z < 0.0) {
		uv = (1.0 - abs(uv.yx)) * mix(vec2(-1.0), vec2(1.0), step(vec2(0.0), uv));
	}
	return uv * 0.5 + 0.5;
}

// Galactic north pole and centre in the celestial frame (right ascension from the sun's,
// declination). The real ones, so the band crosses the sky at a plausible angle; with the sun's
// declination fixed this is an art choice, not a season.
const vec3 NIGHT_GALACTIC_POLE = vec3(-0.8676, -0.1981, 0.4560);
const vec3 NIGHT_GALACTIC_CENTRE = vec3(-0.0549, -0.8734, -0.4838);
// Sine of the band's half-width in galactic latitude.
const float NIGHT_BAND_WIDTH = 0.20;
// Angular radius of the smallest star footprint, about half a pixel at 1080p.
const float NIGHT_STAR_SIGMA = 0.00035;

uint night_hash(uint x) {
	x ^= x >> 16u;
	x *= 0x7feb352du;
	x ^= x >> 15u;
	x *= 0x846ca68bu;
	x ^= x >> 16u;
	return x;
}

float night_unit(uint h) {
	return float(h >> 8u) / 16777216.0;
}

// One layer of stars on a cube-sphere grid of `cells` per face edge; a cell holds at most one,
// with probability `presence`, kept away from the cell's edge so its footprint is never cut.
vec3 night_star_layer(vec3 c, float cells, float presence, float pixel, uint salt) {
	vec3 a = abs(c);
	uint face;
	vec2 uv;
	if (a.x >= a.y && a.x >= a.z) {
		face = c.x > 0.0 ? 0u : 1u;
		uv = c.yz / a.x;
	} else if (a.y >= a.z) {
		face = c.y > 0.0 ? 2u : 3u;
		uv = c.xz / a.y;
	} else {
		face = c.z > 0.0 ? 4u : 5u;
		uv = c.xy / a.z;
	}
	vec2 cell = floor((uv * 0.5 + 0.5) * cells);
	uint id = night_hash(night_hash(uint(cell.x) + uint(cell.y) * 4099u + face * 16785407u) ^ salt);
	if (night_unit(id) > presence) {
		return vec3(0.0);
	}
	uint h1 = night_hash(id);
	uint h2 = night_hash(h1);
	uint h3 = night_hash(h2);
	uint h4 = night_hash(h3);
	vec2 star_uv = (cell + 0.2 + 0.6 * vec2(night_unit(h1), night_unit(h2))) / cells * 2.0 - 1.0;
	vec3 s;
	if (face < 2u) {
		s = vec3(face == 0u ? 1.0 : -1.0, star_uv);
	} else if (face < 4u) {
		s = vec3(star_uv.x, face == 2u ? 1.0 : -1.0, star_uv.y);
	} else {
		s = vec3(star_uv, face == 4u ? 1.0 : -1.0);
	}
	float r = length(c - normalize(s));
	float sigma = max(NIGHT_STAR_SIGMA, pixel * 0.6);
	// Few bright stars, many faint ones.
	float flux = pow(night_unit(h3), 7.0);
	float spread = NIGHT_STAR_SIGMA * NIGHT_STAR_SIGMA / (sigma * sigma);
	// Blue-white to warm, as stars of different temperature read to the eye.
	vec3 tint = mix(vec3(0.75, 0.85, 1.0), vec3(1.0, 0.82, 0.62), night_unit(h4));
	return tint * flux * spread * exp(-0.5 * r * r / (sigma * sigma));
}

float night_noise(vec3 p) {
	vec3 i = floor(p);
	vec3 f = fract(p);
	f = f * f * (3.0 - 2.0 * f);
	uvec3 u = uvec3(ivec3(i) + 1024);
	float n000 = night_unit(night_hash(u.x + night_hash(u.y + night_hash(u.z))));
	float n100 = night_unit(night_hash(u.x + 1u + night_hash(u.y + night_hash(u.z))));
	float n010 = night_unit(night_hash(u.x + night_hash(u.y + 1u + night_hash(u.z))));
	float n110 = night_unit(night_hash(u.x + 1u + night_hash(u.y + 1u + night_hash(u.z))));
	float n001 = night_unit(night_hash(u.x + night_hash(u.y + night_hash(u.z + 1u))));
	float n101 = night_unit(night_hash(u.x + 1u + night_hash(u.y + night_hash(u.z + 1u))));
	float n011 = night_unit(night_hash(u.x + night_hash(u.y + 1u + night_hash(u.z + 1u))));
	float n111 = night_unit(night_hash(u.x + 1u + night_hash(u.y + 1u + night_hash(u.z + 1u))));
	return mix(
		mix(mix(n000, n100, f.x), mix(n010, n110, f.x), f.y),
		mix(mix(n001, n101, f.x), mix(n011, n111, f.x), f.y),
		f.z
	);
}

// The Milky Way's diffuse glow: a band about the galactic equator, brighter towards the centre,
// mottled and split by a darker dust lane. Returns glow and band weight.
vec2 night_milky_way(vec3 c) {
	float latitude = dot(c, NIGHT_GALACTIC_POLE);
	float mottle = night_noise(c * 7.0) * 0.5 + night_noise(c * 17.0) * 0.3 + night_noise(c * 41.0) * 0.2;
	float band = exp(-pow((latitude + 0.05 * (mottle - 0.5)) / NIGHT_BAND_WIDTH, 2.0));
	float bulge = exp((dot(c, NIGHT_GALACTIC_CENTRE) - 1.0) * 2.5);
	float lane = exp(-pow((latitude - 0.01 + 0.04 * (mottle - 0.5)) / 0.035, 2.0)) * (0.3 + 0.5 * bulge);
	float glow = band * mottle * mottle * 1.8 * (0.35 + 0.65 * bulge) * (1.0 - lane);
	return vec2(glow, band);
}

// Shared celestial-frame radiance, before twilight and world-horizon visibility.
vec3 night_sky_celestial(vec3 c, float pixel) {
	vec2 milky = night_milky_way(c);
	vec3 light = vec3(0.55, 0.6, 0.75) * milky.x * 0.16;
	light += night_star_layer(c, 40.0, 0.55, pixel, 0x68e31da4u) * 3.0;
	light += night_star_layer(c, 110.0, 0.5, pixel, 0xb5297a4du) * 0.8;
	// The faint field crowds into the band, which is most of what makes it read as stars.
	light += night_star_layer(c, 220.0, 0.1 + 0.85 * milky.y, pixel, 0x1b56c4e9u) * 0.35;
	light += night_star_layer(c, 400.0, 0.9 * milky.y, pixel, 0x7f4a7c15u) * 0.2;
	return light;
}

// Night-sky radiance along world direction `eye`; `pixel` is the angle one pixel spans. Zero
// by day, so the caller can skip it.
vec3 night_sky(vec3 eye, float pixel) {
	// Stars come out between 4 and 12 degrees of solar depression (civil to nautical twilight)
	// and thin towards the horizon, where the air they shine through is thickest.
	float dark = 1.0 - smoothstep(-0.208, -0.07, weather_sun.y);
	float visible = dark * smoothstep(-0.01, 0.15, eye.y);
	if (visible <= 0.0) {
		return vec3(0.0);
	}
	mat3 m = scene_celestial_basis;
	vec3 c = normalize(vec3(dot(m[0], eye), dot(m[1], eye), dot(m[2], eye)));
	return night_sky_celestial(c, pixel) * visible;
}

// One texture read for all stars and the Milky Way. Ordinary 2D mips do not join the folded
// sphere edges: clamp to the coarser mip's texel centres; some blur near the fold remains.
vec3 night_sky_baked(vec3 eye, float pixel) {
	float dark = 1.0 - smoothstep(-0.208, -0.07, weather_sun.y);
	float visible = dark * smoothstep(-0.01, 0.15, eye.y);
	if (visible <= 0.0) {
		return vec3(0.0);
	}
	mat3 m = scene_celestial_basis;
	vec3 c = normalize(vec3(dot(m[0], eye), dot(m[1], eye), dot(m[2], eye)));
	float lod = min(log2(max(pixel / night_sky_texel_angle(c), 1.0)), log2(NIGHT_SKY_TEXELS));
	float inset = 0.5 * exp2(ceil(lod)) / NIGHT_SKY_TEXELS;
	vec2 uv = clamp(night_sky_oct_encode(c), vec2(inset), vec2(1.0 - inset));
	return textureLod(night_sky_map, uv, lod).rgb * visible;
}
#endif
