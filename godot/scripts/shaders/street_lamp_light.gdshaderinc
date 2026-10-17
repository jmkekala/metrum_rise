// SPDX-License-Identifier: GPL-2.0-only

// Street lamps past the real SpotLight3D handoff, lit inside the receiving material (ROAD-59).
// Reproduces Godot's spot light diffuse term (range window, inverse distance, 80° cone with
// attenuation exponent 2, Lambert) per lamp listed in the camera cell of the grid that
// street_lamp_grid.gd packs; the caller adds the result times albedo to EMISSION. Each lamp is
// weighted by the complement of the real light's distance fade, so the two sum to one lamp.
// Cost: one fetch for an empty cell, else at most STREET_LAMP_CAPACITY lamps; zero by day.
// Past street_lamp_far_fade the exact lamps hand over to street_lamp_far: each lamp's ground light
// summed into 16 m cells and sampled once, so a town stays lit at any distance.

#ifndef STREET_LAMP_LIGHT_INCLUDED
#define STREET_LAMP_LIGHT_INCLUDED

#define STREET_LAMP_CAPACITY 16

global uniform sampler2D street_lamp_cells : filter_nearest, repeat_disable;
// One texel per lamp kind: linear colour * energy, and range.
global uniform sampler2D street_lamp_kinds : filter_nearest, repeat_disable;
// World XZ origin, cell metres, side in cells. A zero side disables an absent renderer.
global uniform vec4 street_lamp_window;
global uniform vec2 street_lamp_fade;
global uniform vec2 street_lamp_settings; // cone cosine, on elevation in degrees
// Mean ground irradiance per cell (linear RGB), its window (origin, cell metres, side) and the
// camera distances over which it takes over from the exact lamps.
global uniform sampler2D street_lamp_far : filter_linear, repeat_disable;
global uniform vec4 street_lamp_far_window;
global uniform vec2 street_lamp_far_fade;
#ifndef SCENE_WINDOW_CLOCK_DECLARED
#define SCENE_WINDOW_CLOCK_DECLARED
global uniform vec2 scene_window_clock;
#endif

vec3 street_lamp_light(vec3 lamp_world_position, vec3 lamp_world_normal, vec3 eye) {
	ivec2 cell = ivec2(floor((lamp_world_position.xz - street_lamp_window.xy) / street_lamp_window.z));
	if (any(lessThan(cell, ivec2(0))) || any(greaterThanEqual(cell, ivec2(int(street_lamp_window.w))))) {
		return vec3(0.0);
	}
	vec3 result = vec3(0.0);
	// One cell of at most STREET_LAMP_CAPACITY heads (street_lamp_grid.gd), one fetch each; the kind
	// table is a handful of texels that stay in cache. A zero kind ends a partial list.
	for (int slot = 0; slot < STREET_LAMP_CAPACITY; slot++) {
		vec4 head = texelFetch(street_lamp_cells, ivec2(cell.x * STREET_LAMP_CAPACITY + slot, cell.y), 0);
		if (head.w == 0.0) { return result; }
		// Godot fades the real lights by camera distance over the same band; the rest is ours.
		float weight = smoothstep(street_lamp_fade.x, street_lamp_fade.y, distance(head.xyz, eye));
		if (weight <= 0.0) { continue; }
		vec4 lamp = texelFetch(street_lamp_kinds, ivec2(int(head.w) - 1, 0), 0);
		vec3 delta = head.xyz - lamp_world_position;
		float d = length(delta);
		if (d >= lamp.w) { continue; }
		vec3 direction = delta / max(d, 0.0001);
		float nd = d / lamp.w;
		nd *= nd;
		nd *= nd;
		float window = max(1.0 - nd, 0.0);
		// Godot stores 1 / spot_angle_attenuation: 0.5 gives exponent 2.
		float scos = max(direction.y, street_lamp_settings.x);
		float rim = max(0.0001, (1.0 - scos) / (1.0 - street_lamp_settings.x));
		float cone = 1.0 - rim * rim;
		// Linear RGB * energy is packed on the CPU; nonphysical PI cancels Lambert / PI.
		result += lamp.rgb * (weight * window * window * cone
			* max(dot(lamp_world_normal, direction), 0.0) / max(d, 0.0001));
	}
	return result;
}

vec3 street_lamp_far_light(vec3 lamp_far_position, vec3 lamp_far_normal) {
	vec2 uv = (lamp_far_position.xz - street_lamp_far_window.xy) / (street_lamp_far_window.z * street_lamp_far_window.w);
	if (any(lessThan(uv, vec2(0.0))) || any(greaterThan(uv, vec2(1.0)))) {
		return vec3(0.0);
	}
	// The map holds light on level ground; a tilted surface takes its share, walls none.
	// Edits take chunks back out by subtraction; clamp the float residue.
	return max(texture(street_lamp_far, uv).rgb, vec3(0.0)) * max(lamp_far_normal.y, 0.0);
}

vec3 street_lamp_surface(vec3 view_pos, vec3 view_normal, vec3 tangent, vec3 binormal,
		vec3 normal_map, float normal_depth, mat4 inv_view, vec3 eye) {
	// Uniform branch before any texture access, normal reconstruction or world transform.
	if (scene_window_clock.y >= street_lamp_settings.y || street_lamp_window.w == 0.0) {
		return vec3(0.0);
	}
	vec3 world_position = (inv_view * vec4(view_pos, 1.0)).xyz;
	float far = smoothstep(street_lamp_far_fade.x, street_lamp_far_fade.y, distance(world_position, eye));
	vec3 result = vec3(0.0);
	if (far > 0.0) {
		result = far * street_lamp_far_light(world_position, normalize((inv_view * vec4(view_normal, 0.0)).xyz));
		if (far >= 1.0) {
			return result;
		}
	}
	// Most lit pixels are nowhere near a lamp: settle that with one fetch before the normal work.
	ivec2 cell = ivec2(floor((world_position.xz - street_lamp_window.xy) / street_lamp_window.z));
	if (any(lessThan(cell, ivec2(0))) || any(greaterThanEqual(cell, ivec2(int(street_lamp_window.w))))
			|| texelFetch(street_lamp_cells, ivec2(cell.x * STREET_LAMP_CAPACITY, cell.y), 0).w == 0.0) {
		return result;
	}
	// Same RG normal reconstruction as scene_forward_clustered.glsl, after fragment().
	vec2 xy = normal_map.xy * 2.0 - 1.0;
	float z = sqrt(max(0.0, 1.0 - dot(xy, xy)));
	vec3 normal = normalize(mix(view_normal, tangent * xy.x + binormal * xy.y + view_normal * z, normal_depth));
	return result + (1.0 - far) * street_lamp_light(world_position,
		normalize((inv_view * vec4(normal, 0.0)).xyz), eye);
}
#endif
