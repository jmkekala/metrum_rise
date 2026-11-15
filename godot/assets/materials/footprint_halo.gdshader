// SPDX-License-Identifier: GPL-2.0-only
//
// The building inspector's ring (`footprint_halo.gd`): the road highlight's band
// (`road_highlight.gdshaderinc`, the ribbon kind), depth-tested so the building hides the far
// side of its own ring instead of showing it through the walls.
shader_type spatial;
render_mode unshaded, cull_disabled, blend_mix, shadows_disabled, fog_disabled;

#include "res://scripts/shaders/road_highlight.gdshaderinc"
