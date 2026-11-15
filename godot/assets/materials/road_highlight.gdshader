// SPDX-License-Identifier: GPL-2.0-only
//
// Road selection and hover highlight (SelectTool). Two kinds of strip, told apart by the vertex
// colour's red channel:
// - the road ribbon (red 0): UV.x runs across it (0 at the left edge, 1 at the right, looking
//   along the road's drawn direction) and UV.y along it in metres. It draws a faint fill and a
//   crisp glowing outline at both edges; `side_glow` lights one half (1 left, 2 right), so the
//   Road Properties window's Left and Right read off the road.
// - one strip per vehicle lane (red 1): UV.x across the lane, UV.y in metres along the lane's
//   direction of travel. When chevron_alpha > 0 it draws chevrons drifting the way that lane's
//   traffic goes, so a two-way road shows both directions.
// Unshaded: it marks, it is not lit.
shader_type spatial;
render_mode unshaded, cull_disabled, depth_test_disabled, blend_mix, shadows_disabled, fog_disabled;

#include "res://scripts/shaders/road_highlight.gdshaderinc"
