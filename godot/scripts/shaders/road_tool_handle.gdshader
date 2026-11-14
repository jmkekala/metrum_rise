shader_type spatial;
render_mode unshaded, cull_disabled, depth_test_disabled, depth_draw_never, shadows_disabled, fog_disabled;

// The road tool's handles (`ROAD-112` round V): filled round points of a constant on-screen size
// with a dark rim, drawn over the terrain and the roads.

uniform vec4 color : source_color = vec4(1.0);
uniform vec4 rim_color : source_color = vec4(0.07, 0.07, 0.07, 0.85);
uniform float size_px = 14.0;
// The rim's share of the radius.
uniform float rim = 0.22;

void vertex() {
	POINT_SIZE = size_px;
}

void fragment() {
	float r = length(POINT_COORD * 2.0 - 1.0);
	// One pixel of antialiasing at the edge.
	float edge = 2.0 / size_px;
	float coverage = clamp((1.0 - r) / edge, 0.0, 1.0);
	if (coverage <= 0.0) {
		discard;
	}
	vec4 fill = r > 1.0 - rim ? rim_color : color;
	ALBEDO = fill.rgb;
	ALPHA = fill.a * coverage;
}
