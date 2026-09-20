// SPDX-License-Identifier: GPL-2.0-only

// Grid overlay for the shared game grass material, aligned with the authored lot cells.
shader_type spatial;
render_mode unshaded, depth_draw_never;

uniform float cell_m = 10.0;
uniform vec2 grid_origin = vec2(0.0);

varying vec2 ground_position;

void vertex() {
	ground_position = VERTEX.xz;
}

void fragment() {
	vec2 cell = (ground_position - grid_origin) / cell_m;
	vec2 footprint = max(fwidth(cell), vec2(0.00001));
	vec2 distance_to_line = abs(fract(cell + 0.5) - 0.5);
	// Keep lines at least one pixel wide; fade unresolved cells at grazing angles.
	vec2 width = max(vec2(0.006), footprint);
	vec2 lines = (vec2(1.0) - smoothstep(width * 0.5, width * 1.5, distance_to_line))
		* (vec2(1.0) - smoothstep(vec2(0.1), vec2(0.5), footprint));
	// Darken the grass without adding light at night or covering it between lines.
	ALBEDO = vec3(0.0);
	ALPHA = 0.40 * max(lines.x, lines.y);
}
