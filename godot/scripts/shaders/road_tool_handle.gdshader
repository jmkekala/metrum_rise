shader_type spatial;
render_mode unshaded, cull_disabled, depth_test_disabled, depth_draw_never, shadows_disabled, fog_disabled;

// The road tool's handles (`ROAD-112` rounds V and V2): filled round points of a constant
// on-screen size with a dark rim, drawn over the terrain and the roads. One surface holds every
// handle: each point's kind is its UV.x (`road_tool_frame`'s `gizmo_handle_attributes`), which
// picks its colour and size.

const int KINDS = 6;
uniform vec4 colors[KINDS] : source_color;
uniform float sizes_px[KINDS];
uniform vec4 rim_color : source_color = vec4(0.07, 0.07, 0.07, 0.85);
// The rim's share of the radius.
uniform float rim = 0.22;

varying vec4 fill_color;
varying float size_px;

void vertex() {
	int kind = clamp(int(UV.x + 0.5), 0, KINDS - 1);
	fill_color = colors[kind];
	size_px = sizes_px[kind];
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
	vec4 fill = r > 1.0 - rim ? rim_color : fill_color;
	ALBEDO = fill.rgb;
	ALPHA = fill.a * coverage;
}
