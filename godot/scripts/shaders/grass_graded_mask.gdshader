// SPDX-License-Identifier: GPL-2.0-only

shader_type spatial;
render_mode unshaded, cull_disabled, fog_disabled;

// The mask draws the graded renderer's cell MultiMeshes into grass.gd's private top-down
// viewport. A base cell has slot -1 and is natural; a graded cell keeps grass only outside the
// road/portal SDF and outside authored paving classes.
uniform float terrain_cell_m = 5.0;
uniform sampler2D heightmap : repeat_disable, filter_linear;
uniform float height_scale;
uniform vec2 heightmap_texture_size;
uniform vec2 inner_sample_offset_texels;

#define TERRAIN_GRADED
#include "res://scripts/shaders/terrain_cells.gdshaderinc"

varying vec2 mask_cell_local_m;
varying flat ivec4 mask_cell;

void vertex() {
	int cell_hm = int(round(terrain_cell_m * 2.0));
	CellFrame cell = cell_frame(INSTANCE_CUSTOM, cell_hm);
	ivec2 k = ivec2(round(VERTEX.xz));
	ivec2 p = cell_vertex_position_hm(cell, k, cell_codes(INSTANCE_CUSTOM), cell_hm);
	ivec2 patch_centre_hm = (cell_patch_rect_hm.xy + cell_patch_rect_hm.zw) / 2;
	VERTEX = vec3(vec2(cell.origin_hm + p - patch_centre_hm) * 0.5, 0.0).xzy;
	mask_cell_local_m = vec2(p) * 0.5;
	mask_cell = ivec4(cell.slot, cell.code, cell.phase);
}

void fragment() {
	if (mask_cell.x < 0) {
		ALBEDO = vec3(1.0);
	} else {
		CellMask mask = cell_mask(
			cell_frame_from(mask_cell, int(round(terrain_cell_m * 2.0))), mask_cell_local_m
		);
		ALBEDO = vec3(float(mask.sdf >= 0.0 && mask.class_id == 0));
	}
}
