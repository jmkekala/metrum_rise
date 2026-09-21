shader_type spatial;
render_mode world_vertex_coords, cull_disabled;

#include "res://scripts/shaders/vegetation_wind.gdshaderinc"

uniform sampler2D foliage_mask : source_color, filter_linear_mipmap, repeat_enable;

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
	// Godot flips normals on back faces of a two-sided material. These normals describe
	// the foliage volume, not the card plane: both sides must keep the authored direction.
	NORMAL = FRONT_FACING ? NORMAL : -NORMAL;
	ALBEDO = COLOR.rgb * canopy_light;
	ROUGHNESS = 1.0;
	BACKLIGHT = vegetation_backlight(COLOR.rgb) * canopy_light;
	ALPHA = texture(foliage_mask, UV).a;
	ALPHA_SCISSOR_THRESHOLD = 0.4;
}
