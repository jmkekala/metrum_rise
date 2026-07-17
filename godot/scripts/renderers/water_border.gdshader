shader_type spatial;
render_mode blend_mix, depth_prepass_alpha, cull_disabled;

uniform vec4 shallow_water_color : source_color = vec4(0.10, 0.28, 0.40, 0.62);
uniform vec4 deep_water_color : source_color = vec4(0.018, 0.09, 0.18, 0.90);

varying float local_depth_factor;
varying float local_alpha_factor;
varying float vertical_t;

void vertex() {
	local_depth_factor = clamp(COLOR.r, 0.0, 1.0);
	local_alpha_factor = clamp(COLOR.a, 0.0, 1.0);
	vertical_t = clamp(UV.y, 0.0, 1.0);
}

void fragment() {
	vec4 base_color = mix(shallow_water_color, deep_water_color, local_depth_factor);
	vec3 color = mix(base_color.rgb * 1.08, base_color.rgb * 0.86, vertical_t);
	float alpha = mix(0.78, 0.96, local_alpha_factor);
	alpha *= mix(0.98, 0.82, vertical_t);
	ALBEDO = color;
	ALPHA = alpha;
	ROUGHNESS = 0.14;
	SPECULAR = 0.42;
}
