// SPDX-License-Identifier: GPL-2.0-only
//
// The highlight band's uniforms and fragment, shared by the road selection
// (`road_highlight.gdshader`, drawn over everything) and the building inspector's ring
// (`footprint_halo.gdshader`, hidden behind the building). See `road_highlight.gdshader`.
uniform vec4 tint : source_color = vec4(0.38, 0.80, 0.88, 1.0);
uniform float fill_alpha = 0.10;
uniform float glow_alpha = 0.30;
uniform float edge_alpha = 0.95;
// Outline width in metres.
uniform float edge_m = 0.35;
// The ribbon's width in metres, for the outline size.
uniform float width_m = 10.0;
// A lane strip's width in metres.
uniform float lane_m = 3.5;
uniform float chevron_alpha = 0.22;
uniform float chevron_spacing_m = 7.0;
uniform float chevron_speed_mps = 2.5;
// 0 none, 1 the left half, 2 the right half.
uniform int side_glow = 0;
uniform float side_glow_alpha = 0.16;

void fragment() {
	if (COLOR.r > 0.5) {
		// Chevrons: lines where (along + across offset) is constant, apex ahead in the lane's
		// direction of travel, fading out before the lane's sides.
		float across = abs(UV.x - 0.5) * lane_m;
		float phase = fract((UV.y + across * 0.7 - TIME * chevron_speed_mps) / chevron_spacing_m);
		float band = smoothstep(0.0, 0.05, phase) * (1.0 - smoothstep(0.12, 0.17, phase));
		ALBEDO = tint.rgb;
		ALPHA = band * chevron_alpha * (1.0 - smoothstep(lane_m * 0.36, lane_m * 0.48, across));
	} else {
		// Distance in metres to the nearer edge.
		float d = min(UV.x, 1.0 - UV.x) * width_m;
		float aa = fwidth(d) * 1.2 + 0.001;
		float edge = 1.0 - smoothstep(edge_m - aa, edge_m + aa, d);
		float glow = exp(-max(d - edge_m, 0.0) * 1.1) * glow_alpha;
		float side = 0.0;
		if (side_glow != 0) {
			float half_aa = fwidth(UV.x) * 1.5;
			float left = 1.0 - smoothstep(0.5 - half_aa, 0.5 + half_aa, UV.x);
			side = (side_glow == 1 ? left : 1.0 - left) * side_glow_alpha;
		}
		ALBEDO = mix(tint.rgb, vec3(1.0), edge * 0.35);
		ALPHA = clamp(fill_alpha + glow + side + edge * edge_alpha, 0.0, 1.0);
	}
}
