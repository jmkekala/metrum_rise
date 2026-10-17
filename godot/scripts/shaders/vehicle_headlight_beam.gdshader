shader_type spatial;
render_mode unshaded, blend_add, depth_draw_never, depth_test_disabled, cull_front, shadows_disabled, fog_disabled;

// Light a car's lamps cast at night (vehicle_lamps.gd): a box round the car whose back faces cover
// every pixel it could light. Each pixel rebuilds its surface from the depth buffer, moves it into
// the car's space and adds the red tail glow behind and, for cars beyond the real headlights of
// vehicle_headlights.gd, the same headlight cone those real lights cast: Godot's SpotLight3D
// falloff with their range, decay, angle and rim, so a car looks the same as it crosses the
// handoff. The light is added on top of the shaded scene, blind to the surface's colour, which is
// why near cars use real lights instead; far off that error is a few pixels. No shadows. Near the camera the box shrinks to the tail glow; by day, and past
// `fade_m`, it collapses in the vertex shader and draws nothing.

global uniform vec2 scene_window_clock;

uniform sampler2D depth_texture : hint_depth_texture, filter_nearest;
// Right-hand lamp centres in the car's mesh space, on the lens face (cars face -Z); the left lamps
// mirror them. Set per model by vehicle_lamps.gd.
uniform vec3 head_lamp = vec3(0.55, 0.5, -1.3);
uniform vec3 tail_lamp = vec3(0.55, 0.5, 1.3);
uniform vec3 head_color = vec3(1.0, 0.93, 0.8);
uniform vec3 tail_color = vec3(1.0, 0.05, 0.02);
// The real headlight (vehicle_headlights.gd): range, decay, cone cosine and rim attenuation.
uniform float head_range_m = 60.0;
uniform float head_decay = 2.0;
uniform float head_cos_cutoff = 0.81915;
uniform float head_angle_attenuation = 3.0;
// Added scene light per unit of the real light's attenuation × cos, and the ground's tint on it,
// fitted to the mean light the real headlights add to the same night views (light-road save,
// 2026-10-14, 60 and 110 m). The box is blind to surface colour, so the tint stands in for it.
uniform float head_energy = 62.0;
uniform vec3 head_ground_tint = vec3(1.07, 1.0, 0.85);
// Peak luminous intensity of one tail lamp, candela.
uniform float tail_intensity_cd = 6.0;
// How far behind the tail lamps the box reaches (vehicle_lamps.gd `TAIL_REACH_M`).
uniform float tail_reach_m = 4.0;
// Height of the box's top face above the car (vehicle_lamps.gd `BEAM_ABOVE_M`); the cone fades
// out below it so walls and trees never show the face as a line.
uniform float beam_above_m = 8.0;
// Added scene light per lux of the tail glow.
uniform float energy_per_lux = 0.012;
// Camera distance over which the low beams take over from the real headlights
// (vehicle_headlights.gd: pool radius and fade).
uniform vec2 head_fade_in_m = vec2(70.0, 100.0);
// Camera distance over which the cast light fades out; the lamp glows carry traffic beyond.
uniform vec2 fade_m = vec2(250.0, 450.0);
uniform vec2 lamps_on_elevation_deg = vec2(4.0, -2.0);

// Rows of the view-to-car transform, constant per instance.
varying flat vec4 view_to_car_x;
varying flat vec4 view_to_car_y;
varying flat vec4 view_to_car_z;
varying flat float strength;
varying flat float head_strength;

void vertex() {
	float night = smoothstep(lamps_on_elevation_deg.x, lamps_on_elevation_deg.y, scene_window_clock.y);
	float distance_m = length(MODELVIEW_MATRIX[3].xyz);
	strength = night * (1.0 - smoothstep(fade_m.x, fade_m.y, distance_m));
	head_strength = smoothstep(head_fade_in_m.x, head_fade_in_m.y, distance_m);
	vec3 vertex = VERTEX;
	if (head_strength <= 0.0) {
		// Only the tail glow is drawn: pull the box in to the few metres behind the car.
		vertex.z = max(vertex.z, tail_lamp.z - 0.5);
		vertex.x = clamp(vertex.x, -3.0, 3.0);
		vertex.y = clamp(vertex.y, -2.0, 2.0);
	}
	mat4 to_car = inverse(MODELVIEW_MATRIX);
	view_to_car_x = vec4(to_car[0][0], to_car[1][0], to_car[2][0], to_car[3][0]);
	view_to_car_y = vec4(to_car[0][1], to_car[1][1], to_car[2][1], to_car[3][1]);
	view_to_car_z = vec4(to_car[0][2], to_car[1][2], to_car[2][2], to_car[3][2]);
	POSITION = strength > 0.0
		? PROJECTION_MATRIX * MODELVIEW_MATRIX * vec4(vertex, 1.0)
		: vec4(2.0, 2.0, 2.0, 1.0);
}

// Illuminance at `p` with normal `n` from a lamp at `lamp` of intensity `cd` in direction `v`.
float lux(vec3 v, vec3 n, float cd) {
	float r2 = max(dot(v, v), 0.01);
	return cd * max(dot(n, -v * inversesqrt(r2)), 0.0) / r2;
}

void fragment() {
	float depth = texture(depth_texture, SCREEN_UV).r;
	vec4 view = INV_PROJECTION_MATRIX * vec4(SCREEN_UV * 2.0 - 1.0, depth, 1.0);
	vec3 view_pos = view.xyz / view.w;
	vec4 vp = vec4(view_pos, 1.0);
	vec3 p = vec3(dot(view_to_car_x, vp), dot(view_to_car_y, vp), dot(view_to_car_z, vp));
	// Lit as level ground: normals rebuilt from depth derivatives broke at every kerb and depth
	// step into bright dashes and straight-edged patches. Far off, the light falls on the road.
	vec3 n = vec3(0.0, 1.0, 0.0);

	vec3 light = vec3(0.0);
	if (head_strength > 0.0) {
		// One cone centred between the lamps, just ahead of the lens, along the car's -Z, as the
		// real SpotLight3D sits (vehicle_headlights.gd `register_model`).
		vec3 v = p - vec3(0.0, head_lamp.y, head_lamp.z - 0.15);
		float d = max(length(v), 0.01);
		vec3 to_surface = v / d;
		float nd = d / head_range_m;
		nd *= nd;
		nd *= nd;
		float window = max(1.0 - nd, 0.0);
		window *= window;
		float scos = max(-to_surface.z, head_cos_cutoff);
		float rim = max(0.0001, (1.0 - scos) / (1.0 - head_cos_cutoff));
		float cone = 1.0 - pow(rim, head_angle_attenuation);
		float lambert = max(dot(n, -to_surface), 0.0);
		light += head_color * head_ground_tint * head_energy * window * pow(d, -head_decay) * cone * lambert
			* head_strength * smoothstep(beam_above_m, 0.6 * beam_above_m, p.y);
	}
	vec3 tail_light = vec3(0.0);
	for (float side = -1.0; side <= 1.0; side += 2.0) {
		vec3 mirror = vec3(side, 1.0, 1.0);
		vec3 t = p - tail_lamp * mirror;
		if (t.z > 0.02) {
			// Faded out before the end of the box so its edge never shows.
			tail_light += tail_color * lux(t, n, tail_intensity_cd) * smoothstep(tail_reach_m, 0.4 * tail_reach_m, t.z);
		}
	}
	ALBEDO = (light + tail_light * energy_per_lux) * strength;
}
