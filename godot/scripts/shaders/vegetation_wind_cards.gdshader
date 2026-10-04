shader_type spatial;
render_mode world_vertex_coords, cull_disabled;

#include "res://scripts/shaders/vegetation_wind.gdshaderinc"

uniform sampler2D foliage_mask : source_color, filter_linear_mipmap, repeat_enable;
// Mean linear shade the atlas bake scales each cell's opaque texels to; see
// tools/bake_foliage_atlas.py, which reads it from here.
const float LEAF_SHADE_MEAN = 0.6;
// Mean linear colour of the opaque texels of `foliage_mask`, and the mean of its mip-4 alpha under
// them. The shared understory atlas is a grey shade at LEAF_SHADE_MEAN; a canopy tree's own atlas
// carries its leaf colour, and tools/prepare_tree_models.py measures both means for it. Dividing
// by the mean keeps vertex colour the mean albedo, which the distant levels read.
uniform vec3 leaf_mean = vec3(0.6);
uniform float leaf_depth_mean = 0.675;
// Leaf albedo share of this form; see LEAF_ALBEDO_SCALE in tree_species.gd.
uniform float leaf_albedo_scale = 1.0;

varying float canopy_light;
varying float shadow_keep;
varying float crown_height;
// Stand closure of a canopy tree, from its instance data; understory has none.
varying flat float stand_closure;
varying flat float crossfade;
// Dither thresholds this level keeps, as [x, y); see vegetation_level_keeps.
varying flat vec2 dither;
// One for canopy trees, which have impostors to hand over to. Understory shares these materials
// and keeps its own ranges, so it stays at zero. Set per instance by vegetation.gd.
instance uniform float hands_over = 0.0;

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
	crossfade = hands_over > 0.5
		? vegetation_crossfade(MODEL_MATRIX[3].xyz, MAIN_CAM_INV_VIEW_MATRIX[3].xyz) : 0.0;
	// A canopy tree casts from its impostor at every distance, so only the colour pass hands over.
	bool shadow_pass = vegetation_shadow_pass(CAMERA_POSITION_WORLD, MAIN_CAM_INV_VIEW_MATRIX[3].xyz);
	float reduced = vegetation_reduced_share(MODEL_MATRIX[3].xyz, MAIN_CAM_INV_VIEW_MATRIX[3].xyz,
		hands_over > 0.5);
	dither = shadow_pass ? vec2(0.0, 2.0) : vegetation_level_keeps(crossfade, reduced);
	// A tree this level keeps no pixel of collapses to a point and is never rasterised.
	if (!shadow_pass && dither.x >= min(dither.y, 1.0)) {
		VERTEX = MODEL_MATRIX[3].xyz;
	}
}

void fragment() {
	// Godot flips normals on back faces of a two-sided material. These normals describe
	// the foliage volume, not the card plane: both sides must keep the authored direction.
	NORMAL = FRONT_FACING ? NORMAL : -NORMAL;
	// A coarse mip of the mask measures how deep a texel sits inside its cluster: the leaves
	// at the ragged rim catch light, and the packed middle is shaded by the leaves around it.
	// leaf_depth_mean is the mean depth over the opaque texels, so the term averages one.
	float depth = textureLod(foliage_mask, UV, 4.0).a;
	vec4 leaf = texture(foliage_mask, UV);
	// Dividing the atlas colour by its mean varies the colour inside a cluster without moving it.
	vec3 leaf_colour = COLOR.rgb * leaf_albedo_scale;
	ALBEDO = leaf_colour * canopy_light * (1.0 + 0.55 * (leaf_depth_mean - depth)) * leaf.rgb / leaf_mean;
	ROUGHNESS = 1.0;
	BACKLIGHT = vegetation_backlight(leaf_colour) * canopy_light;
	float threshold = vegetation_dither_threshold(FRAGCOORD.xy);
	ALPHA = threshold >= dither.x && threshold < dither.y ? leaf.a : 0.0;
	ALPHA_SCISSOR_THRESHOLD = 0.4;
}

void light() {
	float shadow = vegetation_shadow(ATTENUATION, LIGHT_IS_DIRECTIONAL, shadow_keep);
	if (LIGHT_IS_DIRECTIONAL) {
		shadow = min(shadow, vegetation_stand_sun(crown_height, stand_closure));
	}
	// No sun highlight. A card stands for a cluster of leaves under one normal, so the highlight
	// lit a whole cluster at once and turned crowns facing the sun white. In a real crown the
	// leaves around a leaf hide most of its grazing glint. The sky reflection stays.
	DIFFUSE_LIGHT += vegetation_diffuse_light(NORMAL, LIGHT, LIGHT_COLOR, shadow, BACKLIGHT, 0.0);
}
