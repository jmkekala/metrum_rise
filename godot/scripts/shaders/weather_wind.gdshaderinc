// SPDX-License-Identifier: GPL-2.0-only

// The wind every tree and every water ripple shares: one clock, one strength and one gust field,
// all from the Rust weather state (weather.rs). They therefore move together, follow the weather
// scenario and hold still while the game is paused. Nothing here reads TIME.
#ifndef WEATHER_WIND_INCLUDED
#define WEATHER_WIND_INCLUDED

// x: wind clock in seconds, wrapped at WEATHER_WIND_PERIOD_S: it runs with the game, never faster
// than normal speed (a time-lapse would only shake every tree), and stops while paused. y: wind
// speed over the speed sway and ripples are authored at, 1 in fair weather. zw: distance the air
// has moved in metres, wrapped at WEATHER_WIND_DRIFT_PERIOD_M.
global uniform vec4 weather_wind;
// Unit X/Z direction the wind blows towards; it turns as the weather changes. Bends, leans and
// streaks follow it. A pattern laid across the world, such as a sway phase, keeps a fixed
// direction instead: turning it would sweep its distant parts faster than any wind.
global uniform vec2 weather_wind_direction;

// Must match WIND_PERIOD_S and half of WEATHER_PERIOD_M in weather.rs.
const float WEATHER_WIND_PERIOD_S = 16384.0;
const float WEATHER_WIND_DRIFT_PERIOD_M = 32768.0;
// A gust is a few hundred metres across; 256 cells span the drift period exactly, so the field is
// continuous where the drift wraps.
const float WEATHER_GUST_CELL_M = 128.0;
const uint WEATHER_GUST_CELLS = 256u;
// The most a storm may scale the authored sway and ripples: storm wind is three and a half times
// fair, but a crown swaying 1.75 m reads as rubber and outruns the culling bounds.
const float WEATHER_WIND_MAX_STRENGTH = 2.0;

// The angular frequency nearest `omega` that completes a whole number of cycles in one wrap of
// the wind clock, so a phase `clock * omega` is continuous through the wrap. The step is
// 0.00038 rad/s, a fraction of a percent of any sway or ripple frequency.
float weather_wind_omega(float omega) {
	float quantum = TAU / WEATHER_WIND_PERIOD_S;
	return round(omega / quantum) * quantum;
}

// Wind speed scale for sway and ripple amplitudes, capped.
float weather_wind_strength() {
	return clamp(weather_wind.y, 0.0, WEATHER_WIND_MAX_STRENGTH);
}

float weather_gust_corner(uvec2 cell) {
	// Not `h`: shaders that include this one (terrain) already use that name.
	uint mixed = (cell.x % WEATHER_GUST_CELLS) + (cell.y % WEATHER_GUST_CELLS) * 977u;
	mixed ^= mixed >> 16u;
	mixed *= 0x7feb352du;
	mixed ^= mixed >> 15u;
	mixed *= 0x846ca68bu;
	mixed ^= mixed >> 16u;
	return float(mixed >> 8u) / 16777216.0;
}

// The gust lattice, one weather_gust_corner() per texel, baked once (weather_gust_map.gd).
global uniform sampler2D weather_gust_map : filter_linear, repeat_enable;

// Value noise over the lattice in one fetch: the bilinear filter interpolates between texel
// centres, and moving the sample point by the smoothstep of the cell fraction gives the same
// smooth interpolation the hashes had. Repeats every WEATHER_GUST_CELLS cells.
float weather_gust_layer(vec2 p) {
	vec2 cell = floor(p);
	vec2 f = p - cell;
	f = f * f * (3.0 - 2.0 * f);
	return textureLod(weather_gust_map, (cell + f + 0.5) / float(WEATHER_GUST_CELLS), 0.0).r;
}

// 0 below a fresh breeze, 1 in a storm: how gusty the wind is, and a uniform for branching.
float weather_storminess() {
	return smoothstep(1.3, 3.5, weather_wind.y);
}

// Gustiness in [0, 1] at a world position: value noise carried downwind at the air's speed.
// A strong wind adds a second layer at half that speed, so its gusts grow and die as they
// travel instead of sliding across the land as a fixed pattern; a breeze, which only breathes,
// skips it as one uniform branch and keeps the single layer. The half-speed layer's cells are
// half as wide, so its offset still jumps by a whole period where the air drift wraps. One
// fetch, two when it is gusty.
float weather_gust(vec2 world_xz) {
	float fast = weather_gust_layer((world_xz - weather_wind.zw) / WEATHER_GUST_CELL_M);
	float storm = weather_storminess();
	if (storm <= 0.0) {
		return fast;
	}
	float slow = weather_gust_layer((world_xz - 0.5 * weather_wind.zw) / (0.5 * WEATHER_GUST_CELL_M)
		+ vec2(97.0, 41.0));
	return mix(fast, 0.65 * fast + 0.35 * slow, storm);
}

// The wind a point feels over the mean, from its gustiness. A light wind only breathes, 0.6 to
// 1.4. A strong one comes in gusts (puuska): most of the land sits in a lull at half to four
// fifths of the mean while gust patches, about a fifth of it at any moment, blow at up to 2.2
// times it, a storm's peak gust over its mean wind. The mean over the land stays near 1.
float weather_gust_factor(float gust) {
	float breeze = mix(0.6, 1.4, gust);
	float storminess = weather_storminess();
	if (storminess <= 0.0) {
		return breeze;
	}
	float event = smoothstep(0.56, 0.74, gust);
	float storm = mix(0.5, 0.8, smoothstep(0.3, 0.6, gust)) + 1.4 * event;
	return mix(breeze, storm, storminess);
}
#endif
