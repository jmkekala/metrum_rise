shader_type spatial;
render_mode cull_disabled;
// The backdrop variant of the terrain surface (`TERRAIN-41` D2): the coarse whole-map stand-in a
// patch draws while it is not drawn as a patch (a hole, or outside the wanted square). One cell a
// patch on the 64 m lattice (the patches' detail step 8) with a skirt, instanced by block; its
// heights, water surface and land cover from the block's textures; no mask layer, flat water.
// See terrain_surface.gdshaderinc and docs/terrain.md "The backdrop drawn".
#define TERRAIN_BACKDROP
#include "res://scripts/shaders/terrain_surface.gdshaderinc"
