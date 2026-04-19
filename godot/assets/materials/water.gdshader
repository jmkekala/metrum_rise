shader_type spatial;

uniform sampler2D heightmap : repeat_disable, filter_linear;
uniform sampler2D watermap : repeat_disable, filter_linear;
uniform sampler2D velocity_map : repeat_disable, filter_linear;
uniform float height_scale = 10.0;
uniform vec4 water_color : source_color = vec4(0.0, 0.2, 0.5, 0.7);
uniform vec4 foam_color : source_color = vec4(0.8, 0.9, 1.0, 0.9);
uniform float shore_softness_m = 2.5;
uniform float shore_foam_band_m = 1.4;

varying float h;
varying float w;
varying float v;
varying vec2 var_uv;

void vertex() {
	var_uv = UV;
	h = texture(heightmap, UV).r;
	w = texture(watermap, UV).r;
	v = texture(velocity_map, UV).r;
	
	// Terrain samples are still in pre-scaled storage units, but water depth is
	// already world-space metres. Only the terrain sample should use height_scale.
	VERTEX.y += h * height_scale + w + 0.05;
}

void fragment() {
	float depth_m = texture(watermap, var_uv).r;
	float speed_sample = texture(velocity_map, var_uv).r;
	float shore_mask = smoothstep(0.0, max(shore_softness_m, 0.001), depth_m);
	if (shore_mask < 0.01) {
		discard;
	}
	
	// Speed-based color mixing
	float speed = clamp(speed_sample * 0.5, 0.0, 1.0);
	vec3 final_color = mix(water_color.rgb, foam_color.rgb, speed);
	float final_alpha = mix(water_color.a, foam_color.a, speed) * shore_mask;
	float shore_foam = 1.0 - smoothstep(0.0, max(shore_foam_band_m, 0.001), depth_m);
	final_color = mix(final_color, foam_color.rgb, shore_foam * 0.35);
	
	ALBEDO = final_color;
	ALPHA = final_alpha;
	ROUGHNESS = 0.1 * (1.0 - speed); // Faster water is "rougher" (more turbulent)
	SPECULAR = 0.5;
}
