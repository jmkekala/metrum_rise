// SPDX-License-Identifier: GPL-2.0-only

// Wind waves over grass and grain (honami): as a gust bends a stretch of blades their paler
// sides turn up, so bands of light roll downwind across a meadow or a field. From a gameplay
// camera the blades themselves are a pixel or two, but the bands are tens of metres and read at
// any distance where they stay wider than a few pixels. The terrain brightens its grass and
// field albedo by grass_wind_sheen(); the 3D grass near the camera bends with the same field.
// Needs weather_wind.gdshaderinc.
#ifndef GRASS_WIND_INCLUDED
#define GRASS_WIND_INCLUDED

// Band cells, short along the wind and long across it, for a wind along the world x axis; a wind
// along z swaps them. Laid on world axes, not the wind's, so the air drift's wrap (32768 m) is a
// whole number of the gust lattice's periods on both axes and the bands are continuous through
// it, and a turning wind never swings a band pattern laid across the whole world.
const vec2 GRASS_WAVE_CELL_M = vec2(16.0, 64.0);
const vec2 GRASS_RIPPLE_CELL_M = vec2(4.0, 16.0);

float grass_wind_bands(vec2 drifted, vec2 wave_cell, vec2 ripple_cell) {
	return weather_gust_layer(drifted / wave_cell) * 0.7
		+ weather_gust_layer(drifted / ripple_cell + vec2(31.0, 7.0)) * 0.3;
}

// Signed wave in about [-1, 1] at a point, carried downwind at the air's speed, times the gust
// factor there, so lulls lie still and gusts roll. The bands lie across whichever world axis
// the wind is nearer, crossfading within about ten degrees of the diagonal: two fetches, four
// there, plus the gust lookup.
float grass_wind_wave(vec2 world_xz, out float gust_factor) {
	gust_factor = weather_gust_factor(weather_gust(world_xz));
	vec2 drifted = world_xz - weather_wind.zw;
	vec2 downwind = weather_wind_direction / max(length(weather_wind_direction), 0.0001);
	float along_x = smoothstep(0.35, 0.65, downwind.x * downwind.x);
	float wave = 0.0;
	if (along_x > 0.0) {
		wave += along_x * grass_wind_bands(drifted, GRASS_WAVE_CELL_M, GRASS_RIPPLE_CELL_M);
	}
	if (along_x < 1.0) {
		wave += (1.0 - along_x)
			* grass_wind_bands(drifted, GRASS_WAVE_CELL_M.yx, GRASS_RIPPLE_CELL_M.yx);
	}
	return (wave * 2.0 - 1.0) * 2.2;
}

// Albedo scale for grass at a point: 1 in still air. `field` raises it for grain, whose tall
// stalks show the waves best; `sheltered` (forest floor) lowers it. `footprint_m` is the pixel's
// width on the ground: the bands fade once they would be under about four pixels, so they never
// alias. A breeze moves a field's bands by a few percent, a storm's gusts any grass by up to a
// quarter.
float grass_wind_sheen(vec2 world_xz, float footprint_m, float field, float sheltered) {
	// A breeze waves only grain, whose tall stalks show it; meadow grass needs strong wind. Over
	// the terrain the four fetches below cost about 0.3 ms, which a breeze's 2% on a meadow did
	// not earn.
	if (weather_storminess() <= 0.0 && field <= 0.001) {
		return 1.0;
	}
	float resolved = 1.0 - smoothstep(GRASS_WAVE_CELL_M.x / 6.0, GRASS_WAVE_CELL_M.x / 3.0, footprint_m);
	if (resolved <= 0.0 || weather_wind.y <= 0.05) {
		return 1.0;
	}
	float gust_factor;
	float wave = grass_wind_wave(world_xz, gust_factor);
	float strength = (0.035 + 0.075 * weather_storminess()) * min(weather_wind.y, 3.5) / 3.5 * 2.0;
	strength *= gust_factor * mix(1.0, 1.6, field) * mix(1.0, 0.3, sheltered);
	return 1.0 + clamp(wave * strength, -0.25, 0.25) * resolved;
}
#endif
