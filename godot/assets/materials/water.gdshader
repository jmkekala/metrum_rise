shader_type spatial;

uniform sampler2D heightmap;
uniform sampler2D watermap;
uniform sampler2D velocity_map;
uniform float height_scale = 10.0;
uniform vec4 water_color : source_color = vec4(0.0, 0.2, 0.5, 0.7);
uniform vec4 foam_color : source_color = vec4(0.8, 0.9, 1.0, 0.9);

varying float h;
varying float w;
varying float v;

void vertex() {
	h = texture(heightmap, UV).r;
	w = texture(watermap, UV).r;
	v = texture(velocity_map, UV).r;
	
	// Displacement: Offset vertically by terrain height + small bias
	VERTEX.y += (h + w) * height_scale + 0.05;
}

void fragment() {
	if (w < 0.001) {
		discard;
	}
	
	// Speed-based color mixing
	float speed = clamp(v * 0.5, 0.0, 1.0);
	vec3 final_color = mix(water_color.rgb, foam_color.rgb, speed);
	float final_alpha = mix(water_color.a, foam_color.a, speed);
	
	ALBEDO = final_color;
	ALPHA = final_alpha;
	ROUGHNESS = 0.1 * (1.0 - speed); // Faster water is "rougher" (more turbulent)
	SPECULAR = 0.5;
}
