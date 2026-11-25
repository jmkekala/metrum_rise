// SPDX-License-Identifier: GPL-3.0-or-later

// Trial (`RENDER-78`): a stronger relief term in the ground's sun and moon light, on the normal
// light() already reads. Today's term, mix(0.82, 1, N.L), moves a slope by at most 18 % of the
// key light; this adds to it, relative to flat ground, so level ground keeps today's light
// exactly. One global for the terrain and the site ground, so the two stay matched at their seams:
// x is the strength k, y the form. 0: today's term + k (N.L - flat N.L), floored at 0 (the
// "add k" form). 1: today's term times mix(1, clamp(N.L / max(flat N.L, 0.05), 0, 2), k) (the
// "ratio" form). k = 0 is today's picture in either form. Set by the console's `relief` command
// or METRUM_TERRAIN_RELIEF at launch (debug_console.gd).
#ifndef TERRAIN_RELIEF_LIGHT_INCLUDED
#define TERRAIN_RELIEF_LIGHT_INCLUDED

global uniform vec2 terrain_relief_light;

// `facing` is clamp(N.L, 0, 1) and `soft_facing` today's term from it; `light_view` the light
// direction and `inv_view` the inverse view matrix of light() (flat ground's N.L is the light's
// world height).
float terrain_relief_facing(float soft_facing, float facing, vec3 light_view, mat4 inv_view) {
	float strength = terrain_relief_light.x;
	float flat_facing = clamp((inv_view * vec4(light_view, 0.0)).y, 0.0, 1.0);
	if (terrain_relief_light.y > 0.5) {
		return soft_facing * mix(1.0, clamp(facing / max(flat_facing, 0.05), 0.0, 2.0), strength);
	}
	return max(soft_facing + strength * (facing - flat_facing), 0.0);
}

#endif
