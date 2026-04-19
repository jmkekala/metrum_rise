shader_type spatial;
render_mode cull_front;

uniform float skirt_depth_m = 120.0;
uniform vec3 top_color : source_color = vec3(0.42, 0.40, 0.34);
uniform vec3 mid_color : source_color = vec3(0.33, 0.31, 0.27);
uniform vec3 deep_color : source_color = vec3(0.24, 0.22, 0.20);
uniform vec3 rim_color : source_color = vec3(0.65, 0.63, 0.54);
uniform float band_interval_m = 12.0;
uniform float band_strength = 0.08;
uniform float contour_minor_interval_m = 5.0;
uniform float contour_major_interval_m = 25.0;
uniform vec3 contour_minor_color : source_color = vec3(0.13, 0.19, 0.16);
uniform vec3 contour_major_color : source_color = vec3(0.10, 0.16, 0.14);
uniform float contour_minor_strength = 0.14;
uniform float contour_major_strength = 0.28;

varying float local_height_m;
varying float depth_t;
varying float perimeter_u;

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

	float band_count = max(skirt_depth_m / max(band_interval_m, 0.001), 1.0);
	float band_phase = depth_t * band_count * TAU + perimeter_u * 0.015;
	float sediment = 0.5 + 0.5 * sin(band_phase);
	color *= mix(1.0 - band_strength, 1.0 + band_strength, sediment);

	float rim = pow(1.0 - depth_t, 2.0);
	color = mix(color, rim_color, rim * 0.34);

	float major_contour = contour_mask(local_height_m, contour_major_interval_m, 1.2);
	float minor_contour = contour_mask(local_height_m, contour_minor_interval_m, 0.9) * (1.0 - major_contour);
	color = mix(color, contour_minor_color, minor_contour * contour_minor_strength);
	color = mix(color, contour_major_color, major_contour * contour_major_strength);

	ALBEDO = color;
	ROUGHNESS = 0.98;
	SPECULAR = 0.04;
}
