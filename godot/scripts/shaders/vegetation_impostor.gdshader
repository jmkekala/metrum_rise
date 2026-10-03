shader_type spatial;
render_mode cull_disabled;

#include "res://scripts/shaders/vegetation_wind.gdshaderinc"
#include "res://scripts/shaders/vegetation_impostor.gdshaderinc"

uniform sampler2DArray albedo_atlas : source_color, filter_linear_mipmap, repeat_disable;
uniform sampler2DArray normal_atlas : filter_linear_mipmap, repeat_disable;
// One baked form per near variant; the layer is the variant.
uniform vec3 bounds_centre[12];
uniform float bounds_size[12];
// Leaf albedo share of each layer's form; see LEAF_ALBEDO_SCALE in tree_species.gd. The bake
// holds the unscaled leaves, and the few trunk texels in a frame take the same share.
uniform float leaf_albedo_scale[12];
// Per view (x) and form (y), the frame UV rectangle that holds its coverage; see
// vegetation_impostor.gdshaderinc.
uniform sampler2D frame_bounds : filter_nearest, repeat_disable;
uniform float impostor_radiance_match = 1.0;
// Distances over which each tree dissolves before no tree is drawn, while the ground takes the
// stand over; see TREE_FAR_M in tree_species.gd. Set from tree_species.gd.
uniform float tree_far_begin_m = 4000.0;
uniform float tree_far_end_m = 4500.0;
// Scale a far crown reaches, and the distances it grows over; see CROWN_GROWTH in
// tree_species.gd. The defaults leave every crown at its baked size.
uniform float crown_growth = 1.0;
uniform float crown_growth_begin_m = 1.0e9;
uniform float crown_growth_end_m = 1.0e9;
// A mip texel averages the normals under it, and the bake keeps that mean unnormalized: its
// shortfall from unit length measures how far those normals disagreed. Renormalizing it would
// light a whole distant crown by the one normal facing the camera, which is a closed shell,
// and a shell halves in brightness when the camera faces the sun. The near crown is seen
// through, so its response hardly depends on the view. Scattered texels therefore lift toward
// up and wrap their diffuse, which is the volume response the near crown has. The wrap also
// takes a floor that does not depend on the mip: a crown of cards scatters light through itself
// at the handover distance, where the frames are sampled near their full resolution and their
// normals barely disagree. All three are per species and fitted; see IMPOSTOR_VOLUME in
// tree_species.gd. Roughness stays at the near crown's one, so the impostor reflects as much sky
// as the cards it replaces: a specular-free impostor lacked that blue and read as a warmer
// forest than the near band.
uniform float volume_lift_gain = 2.0;
uniform float volume_wrap_floor = 0.0;
uniform float volume_wrap_gain = 1.5;

// Varyings are packed: each one takes a slot however few components it has, and some drivers
// allow 31 slots with the built-ins included. Metal on macOS is one, and an unpacked shader
// that used 32 failed there and drew no far trees.
// Frame UVs 0 (xy) and 1 (zw).
varying vec4 frame_uv01;
// Frame UV 2 (xy), the shadow keep (z) and the tree's stand closure (w).
varying vec4 frame_uv2_keep;
// Atlas cells 0 (xy) and 1 (zw).
varying flat vec4 cells01;
// Atlas cell 2 (xy), the form layer (z) and the handover crossfade (w).
varying flat vec4 cell2_layer_fade;
// Instance tint (xyz) and the far fade (w).
varying flat vec4 tint_far;
varying flat vec3 weights;
// The canopy shade and its distance ramp, the height of the card above the instance origin at
// unit scale, which stands in for the height of the surface drawn on it, and the base ramp. The height is
// interpolated and the base term taken per fragment: the card reaches below the ground, and a
// base term blended between its corners darkened the whole crown. One varying for the three.
varying vec4 canopy_card;
varying float volume_wrap;
// Written by fragment() for light() only, so it takes no interpolant slot.
// Baked object-space normals to view space, for this instance. The fragment stage of a
// MultiMesh sees the node's MODEL_NORMAL_MATRIX, not the instance's, so a normal transformed
// there ignored each tree's own yaw: every impostor was lit as if it faced one way, and over a
// stand of random yaws its response to the sun's direction averaged out to nothing.
varying flat mat3 normal_to_view;

void vertex() {
	int layer = int(INSTANCE_CUSTOM.x);
	vec3 centre = bounds_centre[layer];
	vec3 camera_object = (inverse(MODEL_MATRIX) * vec4(CAMERA_POSITION_WORLD, 1.0)).xyz;
	vec3 view = normalize(camera_object - centre);
	vec2 f = (hemi_encode(view) * 0.5 + 0.5) * 8.0 - 0.5;
	vec2 base = floor(f);
	vec2 part = fract(f);
	vec2 cell0;
	if (part.x + part.y < 1.0) {
		cell0 = base;
		weights = vec3(1.0 - part.x - part.y, part.x, part.y);
	} else {
		cell0 = base + vec2(1.0);
		weights = vec3(part.x + part.y - 1.0, 1.0 - part.y, 1.0 - part.x);
	}
	cell0 = clamp(cell0, vec2(0.0), vec2(7.0));
	vec2 cell1 = clamp(base + vec2(1.0, 0.0), vec2(0.0), vec2(7.0));
	vec2 cell2 = clamp(base + vec2(0.0, 1.0), vec2(0.0), vec2(7.0));
	vec3 right = cross(vec3(0.0, 1.0, 0.0), view);
	// Exactly overhead has no preferred azimuth; keep a finite camera-facing basis.
	right = dot(right, right) > 0.000001 ? normalize(right) : vec3(1.0, 0.0, 0.0);
	vec3 up = cross(view, right);
	// About the bounds centre: the base sinks into the ground it already stands in.
	float growth = mix(1.0, crown_growth, smoothstep(crown_growth_begin_m, crown_growth_end_m,
		distance(MODEL_MATRIX[3].xyz, MAIN_CAM_INV_VIEW_MATRIX[3].xyz)));
	float quad_m = bounds_size[layer] * growth * length(MODEL_MATRIX[0].xyz);
	float pixel_m = 2.0 * distance((MODEL_MATRIX * vec4(centre, 1.0)).xyz, CAMERA_POSITION_WORLD)
		/ (PROJECTION_MATRIX[1][1] * VIEWPORT_SIZE.y);
	float pad = impostor_crop_pad(pixel_m / quad_m);
	vec4 crop = vec4(1.0e9, 1.0e9, -1.0e9, -1.0e9);
	for (int i = 0; i < 3; i++) {
		vec2 cell = i == 0 ? cell0 : (i == 1 ? cell1 : cell2);
		vec4 rect = texelFetch(frame_bounds, ivec2(int(cell.x) + 8 * int(cell.y), layer), 0);
		crop = impostor_crop(crop, right, up, cell, rect, pad);
	}
	vec3 offset = impostor_crop_offset(crop, right, up, UV);
	VERTEX = centre + offset * bounds_size[layer] * growth;
	NORMAL = view;
	normal_to_view = mat3(VIEW_MATRIX) * MODEL_NORMAL_MATRIX;
	frame_uv01 = vec4(project_frame(offset, cell0), project_frame(offset, cell1));
	cells01 = vec4(cell0, cell1);
	vec3 world = (MODEL_MATRIX * vec4(VERTEX, 1.0)).xyz;
	float view_distance_m = distance(world, CAMERA_POSITION_WORLD);
	canopy_card = vec4(vegetation_canopy_shade_parts(MODEL_MATRIX[3].xz, view_distance_m), VERTEX.y,
		vegetation_canopy_base_ramp(view_distance_m));
	// The layer lane carries the stand closure below one half; see vegetation.gd.
	frame_uv2_keep = vec4(project_frame(offset, cell2), vegetation_shadow_keep(view_distance_m),
		2.0 * fract(INSTANCE_CUSTOM.x));
	float crossfade = vegetation_crossfade(MODEL_MATRIX[3].xyz, MAIN_CAM_INV_VIEW_MATRIX[3].xyz);
	float far_fade = smoothstep(tree_far_begin_m, tree_far_end_m,
		distance(MODEL_MATRIX[3].xyz, MAIN_CAM_INV_VIEW_MATRIX[3].xyz));
	cell2_layer_fade = vec4(cell2, float(layer), crossfade);
	tint_far = vec4(INSTANCE_CUSTOM.yzw, far_fade);
	// A tree still wholly branched, or wholly given to the ground, draws no impostor.
	if (crossfade <= 0.0 || far_fade >= 1.0) {
		VERTEX = vec3(0.0);
	}
}

// Limit sampling to the last independent frame mip. Its coarser trilinear tap
// determines the inset, so even a minified frame cannot sample an adjacent view.
float frame_lod(vec2 uv) {
	float footprint = max(length(dFdx(uv)), length(dFdy(uv))) * FRAME_PX;
	return clamp(log2(max(footprint, 1.0)), 0.0, log2(FRAME_PX));
}

vec3 atlas_uv(vec2 uv, vec2 cell, float lod) {
	float inset = 0.5 * exp2(ceil(lod)) / FRAME_PX;
	return vec3((cell + clamp(uv, vec2(inset), vec2(1.0 - inset))) / 8.0, cell2_layer_fade.z);
}

void fragment() {
	float lod0 = frame_lod(frame_uv01.xy);
	float lod1 = frame_lod(frame_uv01.zw);
	float lod2 = frame_lod(frame_uv2_keep.xy);
	vec3 uv0 = atlas_uv(frame_uv01.xy, cells01.xy, lod0);
	vec3 uv1 = atlas_uv(frame_uv01.zw, cells01.zw, lod1);
	vec3 uv2 = atlas_uv(frame_uv2_keep.xy, cell2_layer_fade.xy, lod2);
	vec4 albedo = textureLod(albedo_atlas, uv0, lod0) * weights.x
		+ textureLod(albedo_atlas, uv1, lod1) * weights.y
		+ textureLod(albedo_atlas, uv2, lod2) * weights.z;
	vec3 n = (textureLod(normal_atlas, uv0, lod0).xyz * weights.x
		+ textureLod(normal_atlas, uv1, lod1).xyz * weights.y
		+ textureLod(normal_atlas, uv2, lod2).xyz * weights.z) * 2.0 - 1.0;
	float spread = 1.0 - clamp(length(n), 0.0, 1.0);
	vec3 object_normal = normalize(mix(n, vec3(0.0, 1.0, 0.0), clamp(volume_lift_gain * spread, 0.0, 1.0)));
	NORMAL = normalize(normal_to_view * object_normal);
	// The near tree folds its instance tint into COLOR; the impostor carries the same tint.
	vec3 tinted = albedo.rgb * tint_far.rgb * leaf_albedo_scale[int(cell2_layer_fade.z)];
	float canopy_light = vegetation_canopy_combine(canopy_card.xy, vegetation_canopy_base(canopy_card.z, canopy_card.w));
	ALBEDO = tinted * canopy_light * impostor_radiance_match;
	BACKLIGHT = vegetation_backlight(tinted) * canopy_light * impostor_radiance_match;
	ROUGHNESS = 1.0;
	volume_wrap = clamp(volume_wrap_floor + volume_wrap_gain * spread, 0.0, 1.0);
	// The far fade drops the pixels whose threshold is under its share, the same threshold the
	// handover keeps below its own, so the two never compete for one pixel.
	ALPHA = vegetation_crossfade_keeps(FRAGCOORD.xy, cell2_layer_fade.w, true)
		&& !vegetation_crossfade_keeps(FRAGCOORD.xy, tint_far.w, true) ? albedo.a : 0.0;
	ALPHA_SCISSOR_THRESHOLD = 0.4;
}

void light() {
	float shadow = vegetation_shadow(ATTENUATION, LIGHT_IS_DIRECTIONAL, frame_uv2_keep.z);
	if (LIGHT_IS_DIRECTIONAL) {
		shadow = min(shadow, vegetation_stand_sun(canopy_card.z, frame_uv2_keep.w));
	}
	// No sun highlight, like the cards it replaces; see vegetation_wind_cards.gdshader.
	DIFFUSE_LIGHT += vegetation_diffuse_light(NORMAL, LIGHT, LIGHT_COLOR, shadow, BACKLIGHT,
		volume_wrap);
}
