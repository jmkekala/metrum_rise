shader_type spatial;
render_mode cull_disabled;
// The behind-the-road variant of the graded terrain surface (TERRAIN-07-G, docs/terrain.md T49
// (c)): drawn only by the cells of tiles whose bake finds the drawn terrain above the road top
// along a seam. Within one pixel inside a road seam their terrain is kept whatever its height,
// its depth pushed behind the road (T24 (a2)); writing DEPTH costs these cells the early depth
// test, so every other cell draws terrain_graded.gdshader. See terrain_surface.gdshaderinc.
#define TERRAIN_GRADED
#define TERRAIN_GRADED_BEHIND
#include "res://scripts/shaders/terrain_surface.gdshaderinc"
#include "res://scripts/shaders/street_lamp_light.gdshaderinc"
