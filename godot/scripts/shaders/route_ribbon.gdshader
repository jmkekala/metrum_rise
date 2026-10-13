shader_type spatial;
render_mode unshaded, cull_disabled, depth_test_disabled, depth_draw_never, shadows_disabled, fog_disabled;

// Lines of a constant on-screen width, from the ribbon `SimulationNode.get_agent_route_lines`
// builds: two triangles per segment, each corner carrying the segment's other end in UV (x, y)
// and UV2.x, and in UV2.y the side (+1 or -1) it moves to. The corners move apart across the
// segment's screen direction, so the line stays `width_px` wide at any zoom.

// Line width in pixels.
uniform float width_px = 5.0;
// Outline pass: every segment in `tint` instead of its vertex color.
uniform bool use_tint = false;
uniform vec4 tint : source_color = vec4(0.0, 0.0, 0.0, 0.9);

void vertex() {
	vec4 here = PROJECTION_MATRIX * (MODELVIEW_MATRIX * vec4(VERTEX, 1.0));
	vec4 there = PROJECTION_MATRIX * (MODELVIEW_MATRIX * vec4(UV.x, UV.y, UV2.x, 1.0));
	// Screen direction in pixels; a point behind the camera is held just in front of it.
	vec2 along = (there.xy / max(there.w, 1e-4) - here.xy / max(here.w, 1e-4)) * VIEWPORT_SIZE;
	vec2 dir = length(along) > 1e-4 ? normalize(along) : vec2(1.0, 0.0);
	// One NDC unit is half the viewport, so this offset is half the width on each side.
	here.xy += vec2(-dir.y, dir.x) * UV2.y * width_px / VIEWPORT_SIZE * here.w;
	POSITION = here;
}

void fragment() {
	ALBEDO = use_tint ? tint.rgb : COLOR.rgb;
	ALPHA = use_tint ? tint.a : 1.0;
}
