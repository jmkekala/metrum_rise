shader_type spatial;
render_mode world_vertex_coords;

#include "res://scripts/shaders/vegetation_wind.gdshaderinc"

void vertex() {
	VERTEX += vegetation_wind_offset(MODEL_MATRIX[3].xz, COLOR.a, TIME);
}

void fragment() {
	// Godot folds MultiMesh instance tint into COLOR before vertex(). Alpha is weight only.
	ALBEDO = COLOR.rgb;
	ROUGHNESS = 1.0;
	BACKLIGHT = vegetation_backlight(COLOR.rgb);
}
