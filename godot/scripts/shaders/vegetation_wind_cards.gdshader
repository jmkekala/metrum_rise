shader_type spatial;
render_mode world_vertex_coords, cull_disabled;

#include "res://scripts/shaders/vegetation_wind.gdshaderinc"

uniform sampler2D foliage_mask : source_color, filter_linear_mipmap, repeat_enable;

void vertex() {
	VERTEX += vegetation_wind_offset(MODEL_MATRIX[3].xz, COLOR.a, TIME);
}

void fragment() {
	ALBEDO = COLOR.rgb;
	ROUGHNESS = 1.0;
	BACKLIGHT = vegetation_backlight(COLOR.rgb);
	ALPHA = texture(foliage_mask, UV).a;
	ALPHA_SCISSOR_THRESHOLD = 0.4;
}
