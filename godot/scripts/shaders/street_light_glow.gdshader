shader_type spatial;
render_mode unshaded, blend_add, depth_draw_never, cull_disabled, shadows_disabled;

// Glow of a street-light head (`ROAD-59`), one camera-facing quad per lamp from street_lights.gd.
// Near the camera it is a soft halo of `world_radius`; further away the quad keeps at least
// `min_pixel_radius` pixels, so every lit lamp of a town stays visible across the map at night.
// COLOR is the lamp's colour (MultiMesh instance colour).

global uniform vec2 scene_window_clock;

uniform float world_radius = 0.3;
uniform float min_pixel_radius = 2.2;
uniform float glow_energy = 4.0;
// Sun elevations in degrees over which the lamps come on (matches street_light.gdshader).
uniform vec2 lamps_on_elevation_deg = vec2(4.0, -2.0);

varying float intensity;

void vertex() {
	vec3 center = (MODELVIEW_MATRIX * vec4(0.0, 0.0, 0.0, 1.0)).xyz;
	float distance_m = max(-center.z, 0.01);
	// World size of one pixel at this depth.
	float pixel_m = distance_m * 2.0 / (PROJECTION_MATRIX[1][1] * VIEWPORT_SIZE.y);
	float radius = max(world_radius, min_pixel_radius * pixel_m);
	// Near, a faint halo round a lens that already glows; far, where the lens is gone, the point
	// carries the lamp alone.
	intensity = mix(0.35, 1.0, smoothstep(world_radius, world_radius * 6.0, radius));
	POSITION = PROJECTION_MATRIX * vec4(center + vec3(VERTEX.xy * radius, 0.0), 1.0);
}

void fragment() {
	float night = smoothstep(lamps_on_elevation_deg.x, lamps_on_elevation_deg.y, scene_window_clock.y);
	vec2 q = UV * 2.0 - 1.0;
	float halo = exp(-dot(q, q) * 4.0);
	ALBEDO = COLOR.rgb * halo * intensity * night * glow_energy;
}
