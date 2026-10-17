// SPDX-License-Identifier: GPL-2.0-only

// Rain from the Rust weather state (weather.rs): derived from the cloud deck, so it arrives and
// leaves with it, and holds still while the game is paused like the rest of the weather.
#ifndef WEATHER_RAIN_INCLUDED
#define WEATHER_RAIN_INCLUDED

// x: rain intensity in [0, 1], from dry to downpour; yzw reserved.
global uniform vec4 weather_rain;

// How wet the rain has made a surface facing `world_up` (its world normal's y), in [0, 1]. Upward
// faces take the rain; walls only run with it, so they darken about a third as much.
float weather_wetness(float world_up) {
	return clamp(weather_rain.x, 0.0, 1.0) * mix(0.35, 1.0, smoothstep(0.2, 0.8, world_up));
}

// Darkens and smooths a lit surface by `wetness`. Water fills the pores, so a rough, porous
// surface (soil, grass, asphalt) darkens most and turns glossy, while one already smooth barely
// changes. Strength by porosity, 0 for glass or metal and 1 for soil.
void weather_wet_surface(float wetness, float porosity, inout vec3 wet_albedo, inout float wet_roughness) {
	if (wetness <= 0.0) {
		return;
	}
	float soaked = wetness * porosity;
	wet_albedo *= mix(1.0, 0.55, soaked);
	wet_roughness = mix(wet_roughness, min(wet_roughness, 0.3), soaked);
}
#endif
