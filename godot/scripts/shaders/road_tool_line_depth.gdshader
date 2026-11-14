// SPDX-License-Identifier: GPL-2.0-only

shader_type spatial;
render_mode unshaded, cull_disabled, depth_draw_never, shadows_disabled, fog_disabled;

// The road tool's lines with the normal depth test (`ROAD-112` round HU: the footprint hue's
// tunnel outlines, which hills, trees and buildings in front hide).
#define LINE_DEPTH_PULL
#include "res://scripts/shaders/road_tool_line.gdshaderinc"
