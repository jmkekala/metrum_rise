shader_type spatial;
render_mode cull_disabled;
#ifdef WATER_REFLECTION_PROXY
render_mode alpha_to_coverage_and_one, depth_draw_always;
#endif
// The base variant of the terrain surface (no discard): PlaneMesh patches and the base cells of
// patches with graded tiles. See terrain_surface.gdshaderinc.
#include "res://scripts/shaders/terrain_surface.gdshaderinc"
