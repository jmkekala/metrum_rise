shader_type spatial;
render_mode unshaded, cull_disabled, depth_test_disabled, depth_draw_never, shadows_disabled, fog_disabled;

// The road tool's lines (`ROAD-112` round V): quads of a constant on-screen width, as
// `route_ribbon.gdshader` widens them, from `road_tool_frame`'s `gizmo_*_vertices` and
// `gizmo_*_attributes`. Each corner carries its segment's other end in UV (x, y) and UV2.x, and in
// UV2.y the side it moves to as the sign and its station along the strip in metres plus one as
// the magnitude (0 on a padding quad, which collapses). Drawn over the terrain and the roads.

// Line width in pixels.
uniform float width_px = 4.0;
uniform vec4 color : source_color = vec4(1.0);
// Dash and gap in pixels at the depth of `dash_focus` (0 dash: solid).
uniform float dash_px = 0.0;
uniform float gap_px = 0.0;
// The world point whose depth sets the dashes' scale (the cursor), so a dash keeps its length on
// screen at every zoom without its pattern sliding along the line.
uniform vec3 dash_focus = vec3(0.0);

varying float station_px;
varying float across;

void vertex() {
	vec4 here = PROJECTION_MATRIX * (MODELVIEW_MATRIX * vec4(VERTEX, 1.0));
	vec4 there = PROJECTION_MATRIX * (MODELVIEW_MATRIX * vec4(UV.x, UV.y, UV2.x, 1.0));
	// Screen direction in pixels; a point behind the camera is held just in front of it.
	vec2 along = (there.xy / max(there.w, 1e-4) - here.xy / max(here.w, 1e-4)) * VIEWPORT_SIZE;
	vec2 dir = length(along) > 1e-4 ? normalize(along) : vec2(1.0, 0.0);
	float side = sign(UV2.y);
	// One NDC unit is half the viewport, so this offset is half the width on each side (plus a
	// pixel each side for the antialiased edge).
	here.xy += vec2(-dir.y, dir.x) * side * (width_px + 2.0) / VIEWPORT_SIZE * here.w;
	POSITION = here;
	float focus_depth = max(-(VIEW_MATRIX * vec4(dash_focus, 1.0)).z, 0.05);
	float px_per_m = PROJECTION_MATRIX[1][1] * VIEWPORT_SIZE.y * 0.5 / focus_depth;
	station_px = max(abs(UV2.y) - 1.0, 0.0) * px_per_m;
	// The far end's corners see the segment reversed, so their side flips with it: `orient`
	// flips exactly then (the same two clip points, subtracted the other way), which makes
	// `across` the corner's geometric side for the antialiased edge.
	float orient = dot(dir, vec2(1.0, 0.000123)) >= 0.0 ? 1.0 : -1.0;
	across = side * orient * (width_px + 2.0) * 0.5;
}

void fragment() {
	if (dash_px > 0.0 && mod(station_px, dash_px + gap_px) > dash_px) {
		discard;
	}
	ALBEDO = color.rgb;
	// A one-pixel soft edge.
	ALPHA = color.a * clamp(width_px * 0.5 + 0.5 - abs(across), 0.0, 1.0);
}
