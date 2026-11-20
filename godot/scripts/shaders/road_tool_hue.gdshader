// SPDX-License-Identifier: GPL-3.0-or-later

shader_type spatial;
render_mode unshaded, cull_disabled, depth_draw_never, shadows_disabled, fog_disabled;

// The road tool's footprint hue (`ROAD-112` round HU): a flat translucent colour by the draft's
// state, drawn once with the normal depth test, so hills, trees and buildings in front hide it.
// Rust lays it on the preview's drawn pieces, 5 cm over both them and the drawn ground at every
// point (round HU2), so the geometry keeps it on top; the vertex is pulled towards the camera by
// only 1 cm plus 0.05 % of its distance, a guard against depth-buffer z-fighting at range, while
// a hill in front still hides it. A refused draft is hatched with diagonal screen stripes as well, so
// the refusal does not rest on colour alone.

uniform vec4 hue_color : source_color = vec4(0.35, 0.65, 1.0, 1.0);
uniform float hue_alpha = 0.3;
uniform bool hatched = false;
// The pull towards the camera: a constant and a share of the distance.
const float PULL_M = 0.01;
const float PULL_SHARE = 0.0005;
// The hatching's period in pixels: stripes half as wide.
const float HATCH_PX = 12.0;

void vertex() {
	vec3 view = (MODELVIEW_MATRIX * vec4(VERTEX, 1.0)).xyz;
	float distance_m = length(view);
	view *= max(distance_m - PULL_M - PULL_SHARE * distance_m, 0.0) / max(distance_m, 1e-4);
	POSITION = PROJECTION_MATRIX * vec4(view, 1.0);
}

void fragment() {
	ALBEDO = hue_color.rgb;
	float alpha = hue_alpha;
	if (hatched) {
		float stripe = step(0.5, fract((FRAGCOORD.x + FRAGCOORD.y) / HATCH_PX));
		alpha *= mix(0.3, 1.7, stripe);
	}
	ALPHA = clamp(alpha, 0.0, 1.0);
}
