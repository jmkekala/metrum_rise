shader_type spatial;
render_mode world_vertex_coords;

#include "res://scripts/shaders/vegetation_wind.gdshaderinc"

varying float canopy_light;
varying float shadow_keep;
varying float crown_height;
// Stand closure of a canopy tree, from its instance data; understory has none.
varying flat float stand_closure;
// One for canopy trees, which have impostors to hand over to. Understory shares these materials
// and keeps its own ranges, so it stays at zero. Set per instance by vegetation.gd.
instance uniform float hands_over = 0.0;
// A canopy tree's bark tile and its mean linear colour; see tools/prepare_tree_models.py. The
// division keeps vertex colour the mean albedo. Procedural wood has neither and samples white.
uniform sampler2D bark_albedo : source_color, hint_default_white, filter_linear_mipmap, repeat_enable;
uniform vec3 bark_mean = vec3(1.0);

void vertex() {
	// Measured before the displacement, which is bounded by half a metre and cannot change
	// which side of the gate a tree falls on. One distance now serves both terms.
	float view_distance_m = distance(VERTEX, CAMERA_POSITION_WORLD);
	VERTEX += vegetation_wind_offset(MODEL_MATRIX[3].xz, COLOR.a, TIME,
		vegetation_wind_visibility(view_distance_m, VIEWPORT_SIZE));
	// Understory is ground cover rather than a trunk under a crown, so it takes no base shade.
	crown_height = (VERTEX.y - MODEL_MATRIX[3].y) / length(MODEL_MATRIX[1].xyz);
	stand_closure = hands_over > 0.5 ? INSTANCE_CUSTOM.x : 0.0;
	canopy_light = vegetation_canopy_shade(MODEL_MATRIX[3].xz, view_distance_m, hands_over > 0.5
		? (VERTEX.y - MODEL_MATRIX[3].y) / length(MODEL_MATRIX[1].xyz) : CANOPY_BASE_TOP_M);
	shadow_keep = vegetation_shadow_keep(view_distance_m);
	float crossfade = hands_over > 0.5
		? vegetation_crossfade(MODEL_MATRIX[3].xyz, MAIN_CAM_INV_VIEW_MATRIX[3].xyz) : 0.0;
	// Wood takes no dither: a discard would move every trunk off the opaque fast path, and a
	// trunk in the handover is about two pixels wide. It leaves at the middle of its tree's
	// handover, when the impostor has drawn half the pixels and its own baked trunk. A canopy
	// tree casts from its impostor at every distance, so only the colour pass hands over. The
	// same holds between the full and the reduced level: both carry the trunk, and the fine
	// branches of the full level leave at the middle of the tree's own detail handover.
	float reduced = vegetation_reduced_share(MODEL_MATRIX[3].xyz, MAIN_CAM_INV_VIEW_MATRIX[3].xyz,
		hands_over > 0.5);
	if (!vegetation_shadow_pass(CAMERA_POSITION_WORLD, MAIN_CAM_INV_VIEW_MATRIX[3].xyz)
			&& (crossfade >= 0.5 || (reduced >= 0.5) != reduced_level)) {
		VERTEX = MODEL_MATRIX[3].xyz;
	}
}

void fragment() {
	// Godot folds MultiMesh instance tint into COLOR before vertex(). Alpha is weight only.
	ALBEDO = COLOR.rgb * texture(bark_albedo, UV).rgb / bark_mean * canopy_light;
	ROUGHNESS = 1.0;
	BACKLIGHT = vegetation_backlight(COLOR.rgb) * canopy_light;
}

void light() {
	float shadow = vegetation_shadow(ATTENUATION, LIGHT_IS_DIRECTIONAL, shadow_keep);
	if (LIGHT_IS_DIRECTIONAL) {
		shadow = min(shadow, vegetation_stand_sun(crown_height, stand_closure));
	}
	DIFFUSE_LIGHT += vegetation_diffuse_light(NORMAL, LIGHT, LIGHT_COLOR, shadow, BACKLIGHT, 0.0);
	SPECULAR_LIGHT += vegetation_specular_light(NORMAL, LIGHT, VIEW, LIGHT_COLOR, shadow,
		SPECULAR_AMOUNT);
}
