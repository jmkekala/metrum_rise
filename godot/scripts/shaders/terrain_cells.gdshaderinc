// Graded terrain cells (TERRAIN-07-D, docs/terrain.md "Godot tile renderer"): the instanced
// 64 m cells of a patch with graded tiles, their lattices and stitching, the exact base and the
// graded texel reads, the exact seam mask and the material class.
//
// Positions are integer half-metres (hm) in the tile's frame, 0..128, so every vertex is exact
// and a border vertex two cells share gets the same bits on both. rust/src/simulation/terrain/
// graded/render_tests.rs ports this arithmetic and checks it against the bake: change both.
//
// The lattice codes, 3 bits each: the cell's own (bits 0-2), then the west (x = 0), east
// (x = 128), south (z = 0) and north (z = 128) sides' coarser neighbour. A code below 3 is
// GL0-GL2 (1, 2, 4 hm); from 3 the base lattice at 2^(code - 3) cells (base cells, GL3, a
// neighbouring PlaneMesh patch).
// The world rules (docs/terrain.md section 1: cell 2, 4 or 8 m, sides in 128 m steps, 512 m
// patches from the world edge) put every tile wholly inside one patch and every code's lattice
// in phase with the tile: each spacing divides 128 hm and the base lattice has a line on every
// tile side. So a lattice line k of any code lies at k spacings from the tile's origin, every
// mesh vertex lies on its own lattice, and no vertex leaves its patch (grid alignment A3).
// INSTANCE_CUSTOM, every channel an integer below 2048, which the Compatibility renderer's
// half-float instance data holds exactly: x the tile's column plus 32 times its row among the
// patch's tiles (from the tile holding the patch rectangle's lower corner); y the slot layer
// (-1 for a base cell); z the codes' bits 0-8; w their bits 9-14.

// Per node (instance uniforms, so the base cells share the patch's material): 0 a PlaneMesh
// patch, 1 a PlaneMesh whose cells draw it (collapsed), 2 a node of cells. The patch rectangle
// and the base lattice's world origin (the negative half extents), world hm; the renderer still
// sets the origin, which the in-phase lattice no longer reads.
instance uniform float cell_mode = 0.0;
instance uniform ivec4 cell_patch_rect_hm = ivec4(0);
instance uniform ivec2 cell_lattice_anchor_hm = ivec2(0);
#ifdef TERRAIN_GRADED
// The page of the material's level that the patch's block uses (docs/terrain.md "Pages and the
// row atlas"; one graded material per level and patch, so a draw binds two samplers: the GLES3
// link of the base set plus eight failed, TERRAIN-07-D round 2): the slot layers, one per tile
// (the texels with apron, then a header line), and the row atlas, whose layers of 256 texels
// hold each tile's rows in a run of whole layers (the seams from 0, then the material edges, the
// seams' end heights and the squares' extra candidate lists, at the header's offsets).
uniform sampler2DArray graded_texels : filter_nearest, repeat_disable;
uniform sampler2DArray graded_atlas : filter_nearest, repeat_disable;
#endif

const int CELL_SIDE_HM = 128;

// Tile frame, slot and lattice of the fragment's cell, from vertex() to fragment(): var_cell is
// (slot, code), slot -1 off graded cells.
varying vec2 var_cell_local_m;
varying flat ivec2 var_cell;
varying float var_cell_delta;
varying vec3 var_cell_normal;
#ifdef TERRAIN_GRADED_BEHIND
// 1 where the cell's tile draws its road cut as the backstop (T65 option (c), `ROAD-112` round
// G2): bit 15 of its codes, the fourth custom channel's bit 6, which no side code reads.
varying flat int var_cell_backstop;
#endif

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

int cell_code_spacing_hm(int code, int cell_hm) {
	return code < 3 ? (1 << code) : (cell_hm << (code - 3));
}

// Mesh vertex k on one axis: GL0-GL2 at k spacings; base cells and GL3 at clamp(s (k - 1), 0,
// 128): their generic mesh starts one step before the tile's side, which the clamp folds onto it.
int cell_vertex_hm(int code, int k, int spacing) {
	if (code < 3) {
		return k * spacing;
	}
	return clamp(spacing * (k - 1), 0, CELL_SIDE_HM);
}

// A border vertex (p in [0, 128]) moved onto the coarser side's vertex at or before it: the
// side's ends and the coarse lattice's lines.
int cell_snap_hm(int p, int spacing) {
	return p / spacing * spacing;
}

// The texel index (apron at 0) of the lattice sample at or before p in [0, 128]: the tile's
// lattice as the bake samples it (tile.rs `lattice_axis`).
int cell_index_at(int p, int spacing) {
	return 1 + p / spacing;
}

// The position of texel index idx >= 0, hm: the inverse of cell_index_at, aprons included.
int cell_index_hm(int idx, int spacing) {
	return (idx - 1) * spacing;
}

// The graded variant's slot reads, from the material's level.
#ifdef TERRAIN_GRADED
vec4 cell_texel(ivec2 index, int slot) {
	return texelFetch(graded_texels, ivec3(index, slot), 0);
}

// The slot's header texel (residency.rs `pack_slot`), first on its last line: the first atlas
// layer of the tile's row run (Godot writes it at the upload), then the first rows of its
// material edges, its seams' heights and its extra candidate lists.
ivec4 cell_header(int slot) {
	return ivec4(texelFetch(graded_texels, ivec3(0, textureSize(graded_texels, 0).y - 1, slot), 0));
}

// Row `index` of the tile's run: 256 texels a layer (residency.rs `ATLAS_WIDTH`).
vec4 cell_row(int index, ivec4 header) {
	return texelFetch(graded_atlas, ivec3(index & 255, 0, header.x + (index >> 8)), 0);
}

// The seam channel (residency.rs `seam_channel`): the texel's named row (-1 none) plus 2048 times
// one more than the first row texel of the extra candidate list of the square whose lower corner
// it is (0 none), an integer below 2^24.
int cell_seam_list(float channel) {
	return (int(channel) + 1) >> 11;
}

int cell_seam_row(float channel) {
	return int(channel) - (cell_seam_list(channel) << 11);
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
	return mix(mix(h00, h10, f.x), mix(h01, h11, f.x), f.y);
}

struct CellFrame {
	int code;
	int slot;
	ivec2 origin_hm;
	ivec2 spacing;
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
	frame.spacing = ivec2(cell_code_spacing_hm(frame.code, cell_hm));
	return frame;
}

// The frame of a fragment's cell from the flat varying var_cell (slot, code).
CellFrame cell_frame_from(ivec2 packed, int cell_hm) {
	CellFrame frame;
	frame.slot = packed.x;
	frame.code = packed.y;
	frame.origin_hm = ivec2(0);
	frame.spacing = ivec2(cell_code_spacing_hm(frame.code, cell_hm));
	return frame;
}

// The tile-frame position of mesh vertex k (hm): the lattice vertex, stitched on each side to a
// coarser neighbour. It stays in the tile, and so in the patch.
ivec2 cell_vertex_position_hm(CellFrame frame, ivec2 k, int codes, int cell_hm) {
	ivec2 p = ivec2(
		cell_vertex_hm(frame.code, k.x, frame.spacing.x),
		cell_vertex_hm(frame.code, k.y, frame.spacing.y)
	);
	int west = (codes >> 3) & 7;
	int east = (codes >> 6) & 7;
	int south = (codes >> 9) & 7;
	int north = (codes >> 12) & 7;
	if (p.x == 0 || p.x == CELL_SIDE_HM) {
		int side = p.x == 0 ? west : east;
		p.y = cell_snap_hm(p.y, cell_code_spacing_hm(side, cell_hm));
	}
	if (p.y == 0 || p.y == CELL_SIDE_HM) {
		int side = p.y == 0 ? south : north;
		p.x = cell_snap_hm(p.x, cell_code_spacing_hm(side, cell_hm));
	}
	return p;
}

#ifdef TERRAIN_GRADED
// The graded delta at a mesh vertex (hm): the texel itself, as every vertex is a lattice point
// of its cell's code (a side's coarser lattice is a sub-lattice of it).
float cell_delta_m(CellFrame frame, ivec2 p) {
	return cell_texel(ivec2(cell_index_at(p.x, frame.spacing.x), cell_index_at(p.y, frame.spacing.y)), frame.slot).r;
}

// Central difference of delta at a lattice vertex over its neighbours, per metre.
vec2 cell_delta_gradient(CellFrame frame, ivec2 p) {
	ivec2 i = ivec2(cell_index_at(p.x, frame.spacing.x), cell_index_at(p.y, frame.spacing.y));
	float dx = cell_texel(i + ivec2(1, 0), frame.slot).r
		- cell_texel(i - ivec2(1, 0), frame.slot).r;
	float dz = cell_texel(i + ivec2(0, 1), frame.slot).r
		- cell_texel(i - ivec2(0, 1), frame.slot).r;
	int wx = cell_index_hm(i.x + 1, frame.spacing.x) - cell_index_hm(i.x - 1, frame.spacing.x);
	int wz = cell_index_hm(i.y + 1, frame.spacing.y) - cell_index_hm(i.y - 1, frame.spacing.y);
	return vec2(dx, dz) * 2.0 / vec2(max(ivec2(wx, wz), ivec2(1)));
}

// The fragment's lattice cell on one axis: the index at or before x (tile-frame metres) and the
// fraction towards the next sample.
vec2 cell_axis_stop(CellFrame frame, float x_m, int axis) {
	int spacing = axis == 0 ? frame.spacing.x : frame.spacing.y;
	int p = clamp(int(floor(x_m * 2.0)), 0, CELL_SIDE_HM - 1);
	int index = cell_index_at(p, spacing);
	float p0 = float(cell_index_hm(index, spacing)) * 0.5;
	float p1 = float(cell_index_hm(index + 1, spacing)) * 0.5;
	return vec2(float(index), clamp((x_m - p0) / max(p1 - p0, 1e-6), 0.0, 1.0));
}

struct CellSeamHit {
	float distance;
	float line_distance;
	float signed_distance;
	int index;
	// The nearest point's parameter along the segment, 0 at a and 1 at b.
	float along;
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
	float t = 0.0;
	if (len2 > 0.0) {
		t = dot(p - a, ab) / len2;
		cross_z = ab.x * (p.y - a.y) - ab.y * (p.x - a.x);
		q = t <= 0.0 ? a : (t >= 1.0 ? seg.zw : a + ab * t);
	}
	float d = length(p - q);
	float line = len2 > 0.0 ? abs(cross_z) / sqrt(len2) : 0.0;
	return CellSeamHit(d, line, cross_z > 0.0 ? -d : d, index, clamp(t, 0.0, 1.0));
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
	// The road top at the nearest seam's nearest point (world metres) where the exact distance
	// was taken, else -1e30: what the outer-band overlap compares the terrain with (T24 (a)).
	float road_top;
};

// The exact seam mask and the material class at a fragment of a graded cell (docs/terrain.md
// "The exact seam mask"): bilinear sdf of the four samples around it; within two texels of zero
// the exact signed distance to the nearest of their seams and their square's extra candidates.
// The class: the nearest sample's away from a material edge; near one, the nearest sample on the
// fragment's side of the nearest edge.
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
	mask.road_top = -1e30;
	float spacing_m = float(min(frame.spacing.x, frame.spacing.y)) * 0.5;
	// The rows are read through the slot's header, fetched once, where a fragment needs them.
	ivec4 header = ivec4(0);
	bool headed = false;
	if (abs(mask.sdf) < 2.0 * spacing_m) {
		header = cell_header(frame.slot);
		headed = true;
		CellSeamHit best = CellSeamHit(1e30, 0.0, 1e30, 1 << 30, 0.0);
		for (int c = 0; c < 4; c++) {
			int index = cell_seam_row(t[c].b);
			if (index < 0) {
				continue;
			}
			CellSeamHit hit = cell_segment_at(cell_row(index, header), p, index);
			if (cell_nearer(hit, best)) {
				best = hit;
			}
		}
		// The square's extra candidates (T48 (a)): the seams no corner names that may be nearest
		// in it, from the header's lists: (count, row, row, row), then four rows a texel.
		int list = cell_seam_list(t[0].b);
		if (list > 0) {
			int at = header.w + list - 1;
			int count = int(cell_row(at, header).x);
			for (int k = 1; k <= count; k++) {
				vec4 rows = cell_row(at + k / 4, header);
				int c = k % 4;
				int index = int(c == 0 ? rows.x : (c == 1 ? rows.y : (c == 2 ? rows.z : rows.w)));
				CellSeamHit hit = cell_segment_at(cell_row(index, header), p, index);
				if (cell_nearer(hit, best)) {
					best = hit;
				}
			}
		}
		if (best.index < (1 << 30)) {
			mask.sdf = best.signed_distance;
			vec4 heights = cell_row(header.z + best.index, header);
			mask.road_top = mix(heights.x, heights.y, best.along);
		}
	}
	// The material word: 16 (edge + 1) + class.
	int nearest = (sx.y < 0.5 ? 0 : 1) + (sz.y < 0.5 ? 0 : 2);
	mask.class_id = int(t[nearest].a) & 15;
	CellSeamHit edge = CellSeamHit(1e30, 0.0, 1e30, 1 << 30, 0.0);
	for (int c = 0; c < 4; c++) {
		int index = int(t[c].a) / 16 - 1;
		if (index < 0) {
			continue;
		}
		if (!headed) {
			header = cell_header(frame.slot);
			headed = true;
		}
		vec4 seg = cell_row(header.y + index, header);
		CellSeamHit hit = cell_segment_at(seg, p, index);
		if (cell_nearer(hit, edge)) {
			edge = hit;
		}
	}
	if (edge.index < (1 << 30)) {
		vec4 seg = cell_row(header.y + edge.index, header);
		bool inside = edge.signed_distance <= 0.0;
		float best_d = 1e30;
		for (int c = 0; c < 4; c++) {
			vec2 corner = vec2(
				float(cell_index_hm(i0.x + (c & 1), frame.spacing.x)),
				float(cell_index_hm(i0.y + (c >> 1), frame.spacing.y))
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
