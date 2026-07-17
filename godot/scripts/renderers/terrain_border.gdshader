shader_type spatial;
render_mode cull_front;

uniform float skirt_depth_m = 120.0;
uniform vec3 top_color : source_color = vec3(0.42, 0.40, 0.34);
uniform vec3 mid_color : source_color = vec3(0.33, 0.31, 0.27);
uniform vec3 deep_color : source_color = vec3(0.24, 0.22, 0.20);
uniform vec3 rim_color : source_color = vec3(0.65, 0.63, 0.54);
uniform vec3 topsoil_color : source_color = vec3(0.17, 0.15, 0.11);
uniform float band_interval_m = 12.0;
uniform float band_strength = 0.08;
uniform float strata_warp_m = 3.5;
uniform float topsoil_depth_m = 3.0;
uniform float topsoil_strength = 0.74;
uniform float normal_strength = 0.09;
uniform float contour_minor_interval_m = 5.0;
uniform float contour_major_interval_m = 25.0;
uniform vec3 contour_minor_color : source_color = vec3(0.13, 0.19, 0.16);
uniform vec3 contour_major_color : source_color = vec3(0.10, 0.16, 0.14);
uniform float contour_minor_strength = 0.14;
uniform float contour_major_strength = 0.28;

varying float local_height_m;
varying float depth_t;
varying float perimeter_u;

float hash_noise(vec2 point) {
	vec3 point3 = fract(vec3(point.xyx) * 0.1031);
	point3 += dot(point3, point3.yzx + 33.33);
	return fract((point3.x + point3.y) * point3.z);
}

float value_noise(vec2 point) {
	vec2 cell = floor(point);
	vec2 local = fract(point);
	vec2 blend = local * local * (3.0 - 2.0 * local);
	float bottom = mix(hash_noise(cell), hash_noise(cell + vec2(1.0, 0.0)), blend.x);
	float top = mix(hash_noise(cell + vec2(0.0, 1.0)), hash_noise(cell + vec2(1.0)), blend.x);
	return mix(bottom, top, blend.y);
}

float contour_mask(float world_elevation, float interval_m, float thickness) {
	float safe_interval = max(interval_m, 0.001);
	float scaled = world_elevation / safe_interval;
	float dist_to_line = abs(fract(scaled + 0.5) - 0.5);
	float aa_width = max(fwidth(scaled) * thickness, 0.0001);
	return 1.0 - smoothstep(0.0, aa_width, dist_to_line);
}

void vertex() {
	local_height_m = VERTEX.y;
	depth_t = clamp(UV.y, 0.0, 1.0);
	perimeter_u = UV.x;
}

void fragment() {
	float mid_mix = smoothstep(0.0, 0.48, depth_t);
	float deep_mix = smoothstep(0.38, 1.0, depth_t);
	vec3 color = mix(top_color, mid_color, mid_mix);
	color = mix(color, deep_color, deep_mix);

	float broad_warp = value_noise(vec2(perimeter_u * 0.008, 3.17)) - 0.5;
	float local_warp = value_noise(vec2(perimeter_u * 0.026, local_height_m * 0.022)) - 0.5;
	float strata_height = local_height_m + (broad_warp * 1.4 + local_warp * 0.6) * strata_warp_m;
	float band_phase = strata_height / max(band_interval_m, 0.001) * TAU;
	float sediment = 0.5 + 0.5 * sin(band_phase);
	color *= mix(1.0 - band_strength, 1.0 + band_strength, sediment);

	float depth_m = depth_t * skirt_depth_m;
	float lip_variation = (value_noise(vec2(perimeter_u * 0.045, 11.73)) - 0.5) * 0.9;
	float lip_depth = max(topsoil_depth_m + lip_variation, 0.25);
	float lip_blend_half_width = 0.25;
	float topsoil = 1.0 - smoothstep(
		max(lip_depth - lip_blend_half_width, 0.0),
		lip_depth + lip_blend_half_width,
		depth_m
	);
	color = mix(color, topsoil_color, topsoil * topsoil_strength);

	float surface_rim = 1.0 - smoothstep(0.0, 0.35, depth_m);
	color = mix(color, rim_color, surface_rim * 0.22);

	float major_contour = contour_mask(local_height_m, contour_major_interval_m, 1.2);
	float minor_contour = contour_mask(local_height_m, contour_minor_interval_m, 0.9) * (1.0 - major_contour);
	color = mix(color, contour_minor_color, minor_contour * contour_minor_strength);
	color = mix(color, contour_major_color, major_contour * contour_major_strength);

	vec2 detail_point = vec2(perimeter_u, local_height_m) * 0.55;
	float detail_step = 0.08;
	float detail_center = value_noise(detail_point);
	float detail_x = value_noise(detail_point + vec2(detail_step, 0.0));
	float detail_y = value_noise(detail_point + vec2(0.0, detail_step));
	vec2 detail_gradient = (vec2(detail_x, detail_y) - detail_center) / detail_step;
	float detail_footprint = length(fwidth(detail_point));
	float detail_fade = 1.0 - smoothstep(0.45, 1.4, detail_footprint);
	vec3 detail_normal = normalize(vec3(-detail_gradient.x, detail_gradient.y, 1.0));

	ALBEDO = color;
	ROUGHNESS = 0.98;
	SPECULAR = 0.04;
	NORMAL_MAP = detail_normal * 0.5 + 0.5;
	NORMAL_MAP_DEPTH = normal_strength * detail_fade;
}
