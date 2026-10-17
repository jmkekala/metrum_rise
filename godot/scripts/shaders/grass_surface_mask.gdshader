// SPDX-License-Identifier: GPL-2.0-only

shader_type spatial;
render_mode unshaded, cull_disabled, world_vertex_coords, fog_disabled;

uniform bool height_is_baked = false;
uniform sampler2D heightmap : repeat_disable, filter_linear;
uniform float height_scale;
uniform vec2 heightmap_texture_size;
uniform vec2 inner_sample_offset_texels;
uniform vec2 inner_sample_size_texels;
uniform vec2 patch_origin;
uniform vec2 patch_world_size_m;
varying vec3 surface_position;

void vertex() {
	// Reuse the terrain's triangles/road holes; flatten only this private mask draw.
	surface_position = VERTEX;
	VERTEX.y = 0.0;
}

void fragment() {
	// CDT blue distinguishes natural ground from graded frontage. Sites/roads are holes.
	ALBEDO = vec3(float(!height_is_baked || COLOR.b > 0.5));
	if (height_is_baked) {
		vec2 patch_uv = (surface_position.xz - patch_origin) / patch_world_size_m;
		vec2 uv = (inner_sample_offset_texels + patch_uv * (inner_sample_size_texels - 1.0) + 0.5)
			/ heightmap_texture_size;
		// Natural CDT tie-ins may also be graded. Only retain ground the heightmap supports;
		// do not grow floating blades over an arbitrary engineered triangle.
		float height_error = abs(surface_position.y - textureLod(heightmap, uv, 0.0).r * height_scale);
		ALBEDO *= float(height_error < 0.025);
	}
}
