shader_type spatial;
render_mode world_vertex_coords;

#include "res://scripts/shaders/vegetation_wind.gdshaderinc"

varying float canopy_light;

void vertex() {
	// Measured before the displacement, which is bounded by half a metre and cannot change
	// which side of the gate a tree falls on. One distance now serves both terms.
	float view_distance_m = distance(VERTEX, CAMERA_POSITION_WORLD);
	VERTEX += vegetation_wind_offset(MODEL_MATRIX[3].xz, COLOR.a, TIME,
		vegetation_wind_visibility(view_distance_m, VIEWPORT_SIZE));
	canopy_light = vegetation_canopy_shade(MODEL_MATRIX[3].xz, view_distance_m);
}

void fragment() {
	// Godot folds MultiMesh instance tint into COLOR before vertex(). Alpha is weight only.
	ALBEDO = COLOR.rgb * canopy_light;
	ROUGHNESS = 1.0;
	BACKLIGHT = vegetation_backlight(COLOR.rgb) * canopy_light;
}
