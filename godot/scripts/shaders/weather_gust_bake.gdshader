// SPDX-License-Identifier: GPL-2.0-only

// Bakes the gust lattice, one weather_gust_corner() value per texel, drawn once by
// weather_gust_map.gd. weather_gust_layer() interpolates it in one filtered fetch.
shader_type canvas_item;
render_mode unshaded, blend_disabled;

#include "res://scripts/shaders/weather_wind.gdshaderinc"

void fragment() {
	// UV addresses texel centres, so floor() gives the lattice cell exactly.
	uvec2 cell = uvec2(floor(UV * float(WEATHER_GUST_CELLS)));
	COLOR = vec4(vec3(weather_gust_corner(cell)), 1.0);
}
