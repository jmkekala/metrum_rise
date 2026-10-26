shader_type spatial;
render_mode cull_disabled;
// The graded variant of the terrain surface (TERRAIN-07-D): the graded cells of a patch, with the
// graded height and normal, the exact seam mask and the material class. See
// terrain_surface.gdshaderinc and terrain_cells.gdshaderinc.
#define TERRAIN_GRADED
#include "res://scripts/shaders/terrain_surface.gdshaderinc"
