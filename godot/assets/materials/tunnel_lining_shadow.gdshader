// SPDX-License-Identifier: GPL-2.0-only

// The sun's shadow of a tunnel's lining (`ROAD-61`): drawn only into the shadow map, from a road
// chunk's lining layer (its walls, ceiling, end walls, headwalls and shell); road chunks
// otherwise only receive. Both sides cast.
shader_type spatial;
render_mode unshaded, cull_disabled;

void fragment() {
	ALBEDO = vec3(0.0);
}
