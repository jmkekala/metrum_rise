shader_type spatial;
render_mode cull_disabled;
// The graded variant for plain tiles (TERRAIN-07-D, T35 (b)): the graded cells of tiles whose
// samples are neither covered, named by a seam nor painted (Rust's TilePayload::plain), with the
// graded height and normal and no seam mask, discard or material class, so they keep early depth.
// See terrain_surface.gdshaderinc and terrain_cells.gdshaderinc.
#define TERRAIN_GRADED
#define TERRAIN_GRADED_PLAIN
#include "res://scripts/shaders/terrain_surface.gdshaderinc"
