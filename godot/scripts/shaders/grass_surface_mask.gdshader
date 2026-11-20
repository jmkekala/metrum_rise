// SPDX-License-Identifier: GPL-3.0-or-later

shader_type spatial;
render_mode unshaded, cull_disabled, world_vertex_coords, fog_disabled;

void vertex() {
	// Flatten this private mask draw; graded cell masks cover roads and paved sites separately.
	VERTEX.y = 0.0;
}

void fragment() {
	ALBEDO = vec3(1.0);
}
