shader_type spatial;
render_mode cull_disabled, depth_draw_always;

uniform sampler2D marking_atlas : source_color, filter_linear_mipmap;
uniform vec3 asphalt_color : source_color = vec3(0.0, 0.0, 0.0);
uniform vec3 line_color : source_color = vec3(0.9, 0.9, 0.9);
uniform float line_width = 0.05;
uniform float dash_length = 2.0;
uniform float dash_gap = 2.0;

varying vec3 world_pos;

varying vec2 v_uv;
void vertex() {
	v_uv = UV;
	world_pos = (MODEL_MATRIX * vec4(VERTEX, 1.0)).xyz;
}

void fragment() {
	// Pure Grey Roads
	ALBEDO = vec3(0.4, 0.4, 0.4); // Solid Grey
	ROUGHNESS = 0.85;
	METALLIC = 0.1;
}
