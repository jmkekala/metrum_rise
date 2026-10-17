shader_type spatial;
render_mode unshaded, blend_add, depth_draw_never, cull_disabled, shadows_disabled, fog_disabled;

// Halo of a car lamp at night, one camera-facing quad per lamp (vehicle_lamps.gd). All four
// vertices sit at the lamp centre with the corner in UV; NORMAL is the way the lamp faces, so a
// headlight shows from ahead and a tail light from behind. Near, a halo of COLOR.a metres; far, at
// least `min_pixel_radius` pixels, so traffic stays visible across the map. By day the quad
// collapses in the vertex shader and draws nothing.

global uniform vec2 scene_window_clock;

uniform float min_pixel_radius = 1.6;
uniform float glow_energy = 3.0;
uniform vec2 lamps_on_elevation_deg = vec2(4.0, -2.0);

varying float intensity;

void vertex() {
	float night = smoothstep(lamps_on_elevation_deg.x, lamps_on_elevation_deg.y, scene_window_clock.y);
	vec3 center = (MODELVIEW_MATRIX * vec4(VERTEX, 1.0)).xyz;
	vec3 to_camera = normalize(-center);
	float facing = dot(normalize(mat3(MODELVIEW_MATRIX) * NORMAL), to_camera);
	float distance_m = max(-center.z, 0.01);
	// World size of one pixel at this depth.
	float pixel_m = distance_m * 2.0 / (PROJECTION_MATRIX[1][1] * VIEWPORT_SIZE.y);
	float radius = max(COLOR.a, min_pixel_radius * pixel_m);
	// Near, a faint halo round a lens that already glows; far, the point carries the lamp alone.
	intensity = night * smoothstep(-0.15, 0.5, facing)
		* mix(0.4, 1.0, smoothstep(COLOR.a, COLOR.a * 6.0, radius));
	// Drawn just in front of the lens so the car body does not cut the halo.
	center += to_camera * 0.2;
	POSITION = intensity > 0.0
		? PROJECTION_MATRIX * vec4(center + vec3(UV * radius, 0.0), 1.0)
		: vec4(2.0, 2.0, 2.0, 1.0);
}

void fragment() {
	float halo = exp(-dot(UV, UV) * 4.0);
	ALBEDO = COLOR.rgb * halo * intensity * glow_energy;
}
