// SPDX-License-Identifier: GPL-3.0-or-later

// The ground's relief light (`RENDER-78`): a stronger relief term in the sun and moon light, on
// the normal light() already reads. The plain term, mix(0.82, 1, N.L), moves a slope by at most
// 18 % of the key light; this works relative to flat ground, so level ground keeps the plain
// light (while flat N.L stays above the ratio's 0.05 floor, a sun above about 2.9 degrees). One
// global for the terrain and the site ground, so the two stay matched at their seams: x is the
// strength k, y the form. 0: plain term + k (N.L - flat N.L), floored at 0 (the "add" form).
// 1: plain term times max(mix(1, clamp(N.L / max(flat N.L, 0.05), 0, 2), k), 0) (the "ratio"
// form; at most 1 + k times the plain term). Adding 2 to y lights the sun's relief only (while the
// sun is below the horizon the moonlit ground keeps the plain term). k = 0 is the plain light in
// every form. The shipped default, (2, 1), is ratio at k = 2 for sun and moon (the user's pick,
// 2026-11-25). Set live by the console's `relief` command or METRUM_TERRAIN_RELIEF at launch
// (debug_console.gd).
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
		// Past k = 1 the mix extrapolates (2r - 1 at k = 2): floored at 0 so a slope facing away
		// takes no sun rather than losing light, and capped at 1 + k by the clamp on r.
		return soft_facing * max(mix(1.0, clamp(facing / max(flat_facing, 0.05), 0.0, 2.0), strength), 0.0);
	}
	return max(soft_facing + strength * (facing - flat_facing), 0.0);
}

#endif
