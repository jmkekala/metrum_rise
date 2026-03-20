shader_type spatial;
render_mode cull_disabled, depth_draw_always;

void fragment() {
	ALBEDO = vec3(0.3, 0.3, 0.3); // Solid Dark Grey Asphalt
	ROUGHNESS = 0.8;
	METALLIC = 0.0;
}
