// SPDX-License-Identifier: GPL-3.0-or-later

// Trial (`RENDER-78`): a stronger relief term in the ground's sun and moon light, on the normal
// light() already reads. Today's term, mix(0.82, 1, N.L), moves a slope by at most 18 % of the
// key light; this adds to it, relative to flat ground, so level ground keeps today's light
// exactly. One global for the terrain and the site ground, so the two stay matched at their seams:
// x is the strength k, y the form. 0: today's term + k (N.L - flat N.L), floored at 0 (the
// "add k" form). 1: today's term times mix(1, clamp(N.L / max(flat N.L, 0.05), 0, 2), k) (the
// "ratio" form). Adding 2 to y lights the sun's relief only: while the sun is below the horizon
// (the moon is the key light) the term is today's. k = 0 is today's picture in every form, and
// the global's default is (0, 0). Set by the console's `relief` command or METRUM_TERRAIN_RELIEF
// at launch (debug_console.gd).
#ifndef TERRAIN_RELIEF_LIGHT_INCLUDED
#define TERRAIN_RELIEF_LIGHT_INCLUDED

global uniform vec2 terrain_relief_light;
#ifndef SCENE_WINDOW_CLOCK_DECLARED
#define SCENE_WINDOW_CLOCK_DECLARED
global uniform vec2 scene_window_clock; // hour, true solar elevation
#endif

// `facing` is clamp(N.L, 0, 1) and `soft_facing` today's term from it; `light_view` the light
// direction and `inv_view` the inverse view matrix of light() (flat ground's N.L is the light's
// world height).
float terrain_relief_facing(float soft_facing, float facing, vec3 light_view, mat4 inv_view) {
	float strength = terrain_relief_light.x * float(terrain_relief_light.y < 1.5 || scene_window_clock.y >= 0.0);
	float flat_facing = clamp((inv_view * vec4(light_view, 0.0)).y, 0.0, 1.0);
	if (mod(terrain_relief_light.y, 2.0) > 0.5) {
		return soft_facing * mix(1.0, clamp(facing / max(flat_facing, 0.05), 0.0, 2.0), strength);
	}
	return max(soft_facing + strength * (facing - flat_facing), 0.0);
}

#endif
