// Graded terrain cells (TERRAIN-07-D, docs/terrain.md "Godot tile renderer"): the instanced
// 64 m cells of a patch with graded tiles, their lattices, stitching and clamping to the patch,
// the exact base and the graded texel reads, the exact seam mask and the material class.
//
// Positions are integer half-metres (hm) in the tile's frame, 0..128, so every vertex is exact
// and a border vertex two cells share gets the same bits on both. rust/src/simulation/terrain/
// graded/render_tests.rs ports this arithmetic and checks it against the bake: change both.
//
// The lattice codes, 3 bits each: the cell's own (bits 0-2), then the west (x = 0), east
// (x = 128), south (z = 0) and north (z = 128) sides' coarser neighbour. A code below 3 is
// GL0-GL2 (1, 2, 4 hm); from 3 the base lattice at 2^(code - 3) cells (base cells, GL3, a
// neighbouring PlaneMesh patch).
// INSTANCE_CUSTOM, every channel an integer below 2048, which the Compatibility renderer's
// half-float instance data holds exactly: x the tile's column plus 32 times its row among the
// patch's tiles (from the tile holding the patch rectangle's lower corner); y the slot layer
// (-1 for a base cell); z the codes' bits 0-8; w their bits 9-14.

// Per node (instance uniforms, so the base cells share the patch's material): 0 a PlaneMesh
// patch, 1 a PlaneMesh whose cells draw it (collapsed), 2 a node of cells. The patch rectangle
// and the base lattice's world origin (the negative half extents), world hm.
instance uniform float cell_mode = 0.0;
instance uniform ivec4 cell_patch_rect_hm = ivec4(0);
instance uniform ivec2 cell_lattice_anchor_hm = ivec2(0);
#ifdef TERRAIN_GRADED
// The slot pool of the material's level (one graded material per level and patch, so a draw
// binds two samplers: the GLES3 link of the base set plus eight failed, TERRAIN-07-D round 2):
// texels with apron, and the rows (seams from 0, material edges from the level's seam capacity),
// one layer per slot.
uniform sampler2DArray graded_texels : filter_nearest, repeat_disable;
uniform sampler2DArray graded_rows : filter_nearest, repeat_disable;
uniform int graded_seam_capacity = 512;
#endif

const int CELL_SIDE_HM = 128;

// Tile frame, slot and lattice of the fragment's cell, from vertex() to fragment(): var_cell is
// (slot, code, phase x, phase z), slot -1 off graded cells.
varying vec2 var_cell_local_m;
varying flat ivec4 var_cell;
varying float var_cell_delta;
varying vec3 var_cell_normal;

// floor(a / b) for b > 0, exact: GLSL leaves % and / undefined or truncating for negative
// operands, so the float quotient is corrected by the exact integer remainder.
int cell_floor_div(int a, int b) {
	int q = int(floor(float(a) / float(b)));
	if (a - q * b >= b) {
		q += 1;
	} else if (a - q * b < 0) {
		q -= 1;
	}
	return q;
}

int cell_mod(int a, int b) {
	return a - b * cell_floor_div(a, b);
}

int cell_code_spacing_hm(int code, int cell_hm) {
	return code < 3 ? (1 << code) : (cell_hm << (code - 3));
}

// Where a code's lattice lines fall in a tile starting at origin_hm: GL0-GL2 at the tile's own
// multiples, the base lattice at its world anchor.
int cell_code_phase_hm(int code, int spacing, int anchor_hm, int origin_hm) {
	return code < 3 ? 0 : cell_mod(anchor_hm - origin_hm, spacing);
}

// Mesh vertex k on one axis: GL0-GL2 at k spacings; base cells and GL3 at
// clamp(phase + s (k - 1), 0, 128), the tile's sides and the step lines between them.
int cell_vertex_hm(int code, int k, int spacing, int phase) {
	if (code < 3) {
		return k * spacing;
	}
	return clamp(phase + spacing * (k - 1), 0, CELL_SIDE_HM);
}

// A border vertex moved onto the coarser side's vertex at or before it: the side's ends and the
// coarse lattice's lines.
int cell_snap_hm(int p, int spacing, int phase) {
	if (p >= CELL_SIDE_HM) {
		return CELL_SIDE_HM;
	}
	return max(phase + spacing * cell_floor_div(p - phase, spacing), 0);
}

// The first lattice line strictly inside the tile, and how many lie strictly inside.
int cell_first_line_hm(int spacing, int phase) {
	return phase == 0 ? spacing : phase;
}

int cell_inner_lines(int spacing, int phase) {
	int first = cell_first_line_hm(spacing, phase);
	return first < CELL_SIDE_HM ? (CELL_SIDE_HM - 1 - first) / spacing + 1 : 0;
}

// The texel index (apron at 0) of the lattice sample at or before p in [0, 128]: the tile's
// lattice as the bake samples it (tile.rs `lattice_axis`).
int cell_index_at(int code, int p, int spacing, int phase) {
	if (code < 3) {
		return 1 + p / spacing;
	}
	int first = cell_first_line_hm(spacing, phase);
	if (p >= CELL_SIDE_HM) {
		return 2 + cell_inner_lines(spacing, phase);
	}
	if (p < first) {
		return 1;
	}
	return 2 + (p - first) / spacing;
}

// The position of texel index idx, hm: the inverse of cell_index_at, aprons included.
int cell_index_hm(int code, int idx, int spacing, int phase) {
	if (code < 3) {
		return (idx - 1) * spacing;
	}
	int first = cell_first_line_hm(spacing, phase);
	int end_index = 2 + cell_inner_lines(spacing, phase);
	if (idx <= 0) {
		return phase == 0 ? -spacing : phase - spacing;
	}
	if (idx == 1) {
		return 0;
	}
	if (idx < end_index) {
		return first + (idx - 2) * spacing;
	}
	if (idx == end_index) {
		return CELL_SIDE_HM;
	}
	return cell_mod(CELL_SIDE_HM - phase, spacing) == 0
		? CELL_SIDE_HM + spacing
		: first + cell_inner_lines(spacing, phase) * spacing;
}

// The graded variant's slot reads, from the material's level.
#ifdef TERRAIN_GRADED
vec4 cell_texel(ivec2 index, int slot) {
	return texelFetch(graded_texels, ivec3(index, slot), 0);
}

vec4 cell_row(int index, int slot) {
	return texelFetch(graded_rows, ivec3(index, 0, slot), 0);
}
#endif

// The base b(p) at a world point in hm, bilinear over the patch heightmap's texels in f32: the
// exact sample on a base lattice point, the same expression for every cell of every variant.
float cell_base_height_m(ivec2 world_hm, int cell_hm) {
	ivec2 from_min = world_hm - cell_patch_rect_hm.xy;
	ivec2 g = ivec2(cell_floor_div(from_min.x, cell_hm), cell_floor_div(from_min.y, cell_hm));
	vec2 f = vec2(from_min - g * cell_hm) / float(cell_hm);
	ivec2 last = ivec2(heightmap_texture_size) - 1;
	ivec2 t0 = clamp(ivec2(inner_sample_offset_texels) + g, ivec2(0), last);
	ivec2 t1 = min(t0 + 1, last);
	float h00 = texelFetch(heightmap, t0, 0).r;
	float h10 = texelFetch(heightmap, ivec2(t1.x, t0.y), 0).r;
	float h01 = texelFetch(heightmap, ivec2(t0.x, t1.y), 0).r;
	float h11 = texelFetch(heightmap, t1, 0).r;
	return mix(mix(h00, h10, f.x), mix(h01, h11, f.x), f.y) * height_scale;
}

struct CellFrame {
	int code;
	int slot;
	ivec2 origin_hm;
	ivec2 spacing;
	ivec2 phase;
};

// The lattice codes from their two INSTANCE_CUSTOM channels.
int cell_codes(vec4 custom) {
	return int(round(custom.z)) | (int(round(custom.w)) << 9);
}

CellFrame cell_frame(vec4 custom, int cell_hm) {
	CellFrame frame;
	frame.code = cell_codes(custom) & 7;
	frame.slot = int(round(custom.y));
	int place = int(round(custom.x));
	ivec2 first = ivec2(
		cell_floor_div(cell_patch_rect_hm.x, CELL_SIDE_HM),
		cell_floor_div(cell_patch_rect_hm.y, CELL_SIDE_HM)
	);
	frame.origin_hm = (first + ivec2(place % 32, place / 32)) * CELL_SIDE_HM;
	int spacing = cell_code_spacing_hm(frame.code, cell_hm);
	frame.spacing = ivec2(spacing);
	frame.phase = ivec2(
		cell_code_phase_hm(frame.code, spacing, cell_lattice_anchor_hm.x, frame.origin_hm.x),
		cell_code_phase_hm(frame.code, spacing, cell_lattice_anchor_hm.y, frame.origin_hm.y)
	);
	return frame;
}

// The frame of a fragment's cell from the flat varying var_cell (slot, code, phase x, phase z).
CellFrame cell_frame_from(ivec4 packed, int cell_hm) {
	CellFrame frame;
	frame.slot = packed.x;
	frame.code = packed.y;
	frame.origin_hm = ivec2(0);
	frame.spacing = ivec2(cell_code_spacing_hm(frame.code, cell_hm));
	frame.phase = packed.zw;
	return frame;
}

// The tile-frame position of mesh vertex k (hm): the lattice vertex, stitched on each side to a
// coarser neighbour, clamped to the patch.
ivec2 cell_vertex_position_hm(CellFrame frame, ivec2 k, int codes, int cell_hm) {
	ivec2 p = ivec2(
		cell_vertex_hm(frame.code, k.x, frame.spacing.x, frame.phase.x),
		cell_vertex_hm(frame.code, k.y, frame.spacing.y, frame.phase.y)
	);
	int west = (codes >> 3) & 7;
	int east = (codes >> 6) & 7;
	int south = (codes >> 9) & 7;
	int north = (codes >> 12) & 7;
	if (p.x == 0 || p.x == CELL_SIDE_HM) {
		int side = p.x == 0 ? west : east;
		int s = cell_code_spacing_hm(side, cell_hm);
		p.y = cell_snap_hm(p.y, s, cell_code_phase_hm(side, s, cell_lattice_anchor_hm.y, frame.origin_hm.y));
	}
	if (p.y == 0 || p.y == CELL_SIDE_HM) {
		int side = p.y == 0 ? south : north;
		int s = cell_code_spacing_hm(side, cell_hm);
		p.x = cell_snap_hm(p.x, s, cell_code_phase_hm(side, s, cell_lattice_anchor_hm.x, frame.origin_hm.x));
	}
	return clamp(p, cell_patch_rect_hm.xy - frame.origin_hm, cell_patch_rect_hm.zw - frame.origin_hm);
}

#ifdef TERRAIN_GRADED
// The graded delta at a tile-frame position (hm): the texel itself on a lattice point (every
// vertex but a GL3 one clamped to a patch side), else bilinear between the samples around it.
float cell_delta_m(CellFrame frame, ivec2 p) {
	ivec2 i0 = ivec2(
		cell_index_at(frame.code, p.x, frame.spacing.x, frame.phase.x),
		cell_index_at(frame.code, p.y, frame.spacing.y, frame.phase.y)
	);
	ivec2 p0 = ivec2(
		cell_index_hm(frame.code, i0.x, frame.spacing.x, frame.phase.x),
		cell_index_hm(frame.code, i0.y, frame.spacing.y, frame.phase.y)
	);
	ivec2 p1 = ivec2(
		cell_index_hm(frame.code, i0.x + 1, frame.spacing.x, frame.phase.x),
		cell_index_hm(frame.code, i0.y + 1, frame.spacing.y, frame.phase.y)
	);
	vec2 f = vec2(p - p0) / vec2(max(p1 - p0, ivec2(1)));
	float d00 = cell_texel(i0, frame.slot).r;
	if (f == vec2(0.0)) {
		return d00;
	}
	float d10 = cell_texel(i0 + ivec2(1, 0), frame.slot).r;
	float d01 = cell_texel(i0 + ivec2(0, 1), frame.slot).r;
	float d11 = cell_texel(i0 + ivec2(1, 1), frame.slot).r;
	return mix(mix(d00, d10, f.x), mix(d01, d11, f.x), f.y);
}

// Central difference of delta at a lattice vertex over its neighbours, per metre.
vec2 cell_delta_gradient(CellFrame frame, ivec2 p) {
	ivec2 i = ivec2(
		cell_index_at(frame.code, p.x, frame.spacing.x, frame.phase.x),
		cell_index_at(frame.code, p.y, frame.spacing.y, frame.phase.y)
	);
	float dx = cell_texel(i + ivec2(1, 0), frame.slot).r
		- cell_texel(i - ivec2(1, 0), frame.slot).r;
	float dz = cell_texel(i + ivec2(0, 1), frame.slot).r
		- cell_texel(i - ivec2(0, 1), frame.slot).r;
	int wx = cell_index_hm(frame.code, i.x + 1, frame.spacing.x, frame.phase.x)
		- cell_index_hm(frame.code, i.x - 1, frame.spacing.x, frame.phase.x);
	int wz = cell_index_hm(frame.code, i.y + 1, frame.spacing.y, frame.phase.y)
		- cell_index_hm(frame.code, i.y - 1, frame.spacing.y, frame.phase.y);
	return vec2(dx, dz) * 2.0 / vec2(max(ivec2(wx, wz), ivec2(1)));
}

// The fragment's lattice cell on one axis: the index at or before x (tile-frame metres) and the
// fraction towards the next sample.
vec2 cell_axis_stop(CellFrame frame, float x_m, int axis) {
	int spacing = axis == 0 ? frame.spacing.x : frame.spacing.y;
	int phase = axis == 0 ? frame.phase.x : frame.phase.y;
	int p = clamp(int(floor(x_m * 2.0)), 0, CELL_SIDE_HM - 1);
	int index = cell_index_at(frame.code, p, spacing, phase);
	float p0 = float(cell_index_hm(frame.code, index, spacing, phase)) * 0.5;
	float p1 = float(cell_index_hm(frame.code, index + 1, spacing, phase)) * 0.5;
	return vec2(float(index), clamp((x_m - p0) / max(p1 - p0, 1e-6), 0.0, 1.0));
}

struct CellSeamHit {
	float distance;
	float line_distance;
	float signed_distance;
	int index;
};

// One row segment (tile-frame [ax, az, bx, bz]) at a point: the nearest point with the clamps
// of seams.rs `segment_at`, the distance to the segment's line for ties, and the side: the road
// (or the surface) lies on the segment's left, cross > 0.
CellSeamHit cell_segment_at(vec4 seg, vec2 p, int index) {
	vec2 a = seg.xy;
	vec2 ab = seg.zw - seg.xy;
	float len2 = dot(ab, ab);
	vec2 q = a;
	float cross_z = 0.0;
	if (len2 > 0.0) {
		float t = dot(p - a, ab) / len2;
		cross_z = ab.x * (p.y - a.y) - ab.y * (p.x - a.x);
		q = t <= 0.0 ? a : (t >= 1.0 ? seg.zw : a + ab * t);
	}
	float d = length(p - q);
	float line = len2 > 0.0 ? abs(cross_z) / sqrt(len2) : 0.0;
	return CellSeamHit(d, line, cross_z > 0.0 ? -d : d, index);
}

// The nearest-seam total order of seams.rs `nearer_than`: smaller distance, then the larger
// distance to the line, then the smaller index.
bool cell_nearer(CellSeamHit a, CellSeamHit b) {
	if (a.distance != b.distance) {
		return a.distance < b.distance;
	}
	if (a.line_distance != b.line_distance) {
		return a.line_distance > b.line_distance;
	}
	return a.index < b.index;
}

struct CellMask {
	float sdf;
	int class_id;
};

// The exact seam mask and the material class at a fragment of a graded cell (docs/terrain.md
// "The exact seam mask"): bilinear sdf of the four samples around it; within two texels of zero
// the exact signed distance to the nearest of their seams. The class: the nearest sample's away
// from a material edge; near one, the nearest sample on the fragment's side of the nearest edge.
CellMask cell_mask(CellFrame frame, vec2 p) {
	vec2 sx = cell_axis_stop(frame, p.x, 0);
	vec2 sz = cell_axis_stop(frame, p.y, 1);
	ivec2 i0 = ivec2(int(sx.x), int(sz.x));
	vec4 t[4];
	for (int c = 0; c < 4; c++) {
		t[c] = cell_texel(i0 + ivec2(c & 1, c >> 1), frame.slot);
	}
	CellMask mask;
	mask.sdf = mix(mix(t[0].g, t[1].g, sx.y), mix(t[2].g, t[3].g, sx.y), sz.y);
	float spacing_m = float(min(frame.spacing.x, frame.spacing.y)) * 0.5;
	if (abs(mask.sdf) < 2.0 * spacing_m) {
		CellSeamHit best = CellSeamHit(1e30, 0.0, 1e30, 1 << 30);
		for (int c = 0; c < 4; c++) {
			int index = int(t[c].b);
			if (index < 0) {
				continue;
			}
			CellSeamHit hit = cell_segment_at(cell_row(index, frame.slot), p, index);
			if (cell_nearer(hit, best)) {
				best = hit;
			}
		}
		if (best.index < (1 << 30)) {
			mask.sdf = best.signed_distance;
		}
	}
	// The material word: 16 (edge + 1) + class.
	int nearest = (sx.y < 0.5 ? 0 : 1) + (sz.y < 0.5 ? 0 : 2);
	mask.class_id = int(t[nearest].a) & 15;
	CellSeamHit edge = CellSeamHit(1e30, 0.0, 1e30, 1 << 30);
	for (int c = 0; c < 4; c++) {
		int index = int(t[c].a) / 16 - 1;
		if (index < 0) {
			continue;
		}
		vec4 seg = cell_row(graded_seam_capacity + index, frame.slot);
		CellSeamHit hit = cell_segment_at(seg, p, index);
		if (cell_nearer(hit, edge)) {
			edge = hit;
		}
	}
	if (edge.index < (1 << 30)) {
		vec4 seg = cell_row(graded_seam_capacity + edge.index, frame.slot);
		bool inside = edge.signed_distance <= 0.0;
		float best_d = 1e30;
		for (int c = 0; c < 4; c++) {
			vec2 corner = vec2(
				float(cell_index_hm(frame.code, i0.x + (c & 1), frame.spacing.x, frame.phase.x)),
				float(cell_index_hm(frame.code, i0.y + (c >> 1), frame.spacing.y, frame.phase.y))
			) * 0.5;
			bool corner_inside = cell_segment_at(seg, corner, edge.index).signed_distance <= 0.0;
			float d = distance(corner, p);
			if (corner_inside == inside && d < best_d) {
				best_d = d;
				mask.class_id = int(t[c].a) & 15;
			}
		}
	}
	return mask;
}
#endif
