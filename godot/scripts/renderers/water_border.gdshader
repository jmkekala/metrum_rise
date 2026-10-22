shader_type spatial;
render_mode blend_mix, depth_prepass_alpha, cull_disabled;

uniform vec4 shallow_water_color : source_color = vec4(0.10, 0.28, 0.40, 0.62);
uniform vec4 deep_water_color : source_color = vec4(0.018, 0.09, 0.18, 0.90);

varying float local_depth_factor;
varying float local_alpha_factor;
varying float vertical_t;
uniform float specular = 0.42;

// Godot's lighting with the cloud shadow on the sun; reads `specular`.
#include "res://scripts/shaders/standard_light.gdshaderinc"

void vertex() {
	local_depth_factor = clamp(COLOR.r, 0.0, 1.0);
	local_alpha_factor = clamp(COLOR.a, 0.0, 1.0);
	vertical_t = clamp(UV.y, 0.0, 1.0);
}

#include "res://scripts/shaders/street_lamp_light.gdshaderinc"

void fragment() {
	vec4 base_color = mix(shallow_water_color, deep_water_color, local_depth_factor);
	vec3 color = mix(base_color.rgb * 1.08, base_color.rgb * 0.86, vertical_t);
	float alpha = mix(0.78, 0.96, local_alpha_factor);
	alpha *= mix(0.98, 0.82, vertical_t);
	ALBEDO = color;
	ALPHA = alpha;
	ROUGHNESS = 0.14;
	standard_light_cloud_sun = standard_light_cloud(VERTEX, INV_VIEW_MATRIX);
	EMISSION += ALBEDO * (1.0 - METALLIC) * street_lamp_surface(VERTEX, NORMAL, TANGENT, BINORMAL,
		NORMAL_MAP, NORMAL_MAP_DEPTH, INV_VIEW_MATRIX, CAMERA_POSITION_WORLD);
}
