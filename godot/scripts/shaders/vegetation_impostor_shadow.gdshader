shader_type spatial;
render_mode cull_disabled, unshaded;

#include "res://scripts/shaders/vegetation_wind.gdshaderinc"
#include "res://scripts/shaders/vegetation_impostor.gdshaderinc"

// The shadow caster for trees whose visible level is the impostor. It draws the same baked
// forms as vegetation_impostor.gdshader, but chooses and faces the views toward the light
// rather than the camera, so each tree casts its own silhouette seen from the sun: its species,
// its variant, and the gaps in its crown. Drawn only into the shadow maps, for every canopy tree
// past tree_shadow_handover_m; nearer, the branched tree casts. See vegetation_branched_casts.
//
// It replaced a solid lathe per species, one shape for every variant, sized to their mean
// envelope. That cast broad opaque blobs under narrow open crowns, and a closed stand cast a
// dark halo past its edge that no crown above could account for.

uniform sampler2DArray albedo_atlas : source_color, filter_linear_mipmap, repeat_disable;
uniform vec3 bounds_centre[12];
uniform float bounds_size[12];
uniform sampler2D frame_bounds : filter_nearest, repeat_disable;
// How far the quad stands behind its crown centre along the light, as a share of the form's
// bounds diagonal. Sliding a caster along the light leaves its shadow on the ground where it
// was; it only moves the depth, so a crown stops shadowing the half of itself facing the sun.
const float LIGHT_SETBACK = 0.12;
// Distance from the main camera within which a tree blends its views; set from tree_species.gd.
// Past it the nearest view casts alone, with its own tighter crop and one fetch: in a dense stand
// the blend cost every caster three fetches over a larger quad, and the views' shapes differ least
// where a tree is a few texels wide in a coarse cascade.
uniform float tree_shadow_blend_m = 150.0;

// The three baked views around the light, blended by their barycentric weights as the visible
// impostor blends them around the camera. With one view, the nearest, the shadow held a view's
// shape and then jumped to the next as the sun moved, and that view's direction is up to half a
// cell off the light, which put a low sun's crown shadow metres from where the tree throws it.
varying vec4 frame_uv01;
varying vec2 frame_uv2;
varying flat vec4 cells01;
varying flat vec3 cell2_layer;
varying flat vec3 weights;

void vertex() {
	int layer = int(INSTANCE_CUSTOM.x);
	vec3 centre = bounds_centre[layer];
	// The shadow camera looks along the light; its back axis points at the sun. Object space,
	// so each tree's own yaw selects its views.
	vec3 view = normalize((inverse(MODEL_MATRIX) * vec4(INV_VIEW_MATRIX[2].xyz, 0.0)).xyz);
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
	int views = 3;
	if (distance(MODEL_MATRIX[3].xyz, MAIN_CAM_INV_VIEW_MATRIX[3].xyz) > tree_shadow_blend_m) {
		cell0 = weights.x >= max(weights.y, weights.z) ? cell0 : (weights.y >= weights.z ? cell1 : cell2);
		weights = vec3(1.0, 0.0, 0.0);
		views = 1;
	}
	vec3 right = cross(vec3(0.0, 1.0, 0.0), view);
	right = dot(right, right) > 0.000001 ? normalize(right) : vec3(1.0, 0.0, 0.0);
	vec3 up = cross(view, right);
	// A shadow map is orthographic, so one of its texels spans the same width at any depth.
	float pixel_m = 2.0 / (PROJECTION_MATRIX[1][1] * VIEWPORT_SIZE.y);
	float pad = impostor_crop_pad(pixel_m / (bounds_size[layer] * length(MODEL_MATRIX[0].xyz)));
	vec4 crop = vec4(1.0e9, 1.0e9, -1.0e9, -1.0e9);
	for (int i = 0; i < views; i++) {
		vec2 cell = i == 0 ? cell0 : (i == 1 ? cell1 : cell2);
		vec4 rect = texelFetch(frame_bounds, ivec2(int(cell.x) + 8 * int(cell.y), layer), 0);
		crop = impostor_crop(crop, right, up, cell, rect, pad);
	}
	vec3 offset = impostor_crop_offset(crop, right, up, UV);
	VERTEX = centre + (offset - view * LIGHT_SETBACK) * bounds_size[layer];
	frame_uv01 = vec4(project_frame(offset, cell0), project_frame(offset, cell1));
	frame_uv2 = project_frame(offset, cell2);
	cells01 = vec4(cell0, cell1);
	cell2_layer = vec3(cell2, float(layer));
	// A tree that still casts from its branches draws no caster.
	if (distance(MODEL_MATRIX[3].xyz, MAIN_CAM_INV_VIEW_MATRIX[3].xyz) < tree_shadow_handover_m) {
		VERTEX = vec3(0.0);
	}
}

float frame_lod(vec2 uv) {
	float footprint = max(length(dFdx(uv)), length(dFdy(uv))) * FRAME_PX;
	return clamp(log2(max(footprint, 1.0)), 0.0, log2(FRAME_PX));
}

float frame_alpha(vec2 uv, vec2 cell) {
	float lod = frame_lod(uv);
	float inset = 0.5 * exp2(ceil(lod)) / FRAME_PX;
	vec3 atlas = vec3((cell + clamp(uv, vec2(inset), vec2(1.0 - inset))) / 8.0, cell2_layer.z);
	return textureLod(albedo_atlas, atlas, lod).a;
}

void fragment() {
	ALBEDO = vec3(0.0);
	ALPHA = frame_alpha(frame_uv01.xy, cells01.xy) * weights.x;
	// Flat per tree, so a caster past the blend distance skips both fetches.
	if (weights.x < 1.0) {
		ALPHA += frame_alpha(frame_uv01.zw, cells01.zw) * weights.y
			+ frame_alpha(frame_uv2, cell2_layer.xy) * weights.z;
	}
	ALPHA_SCISSOR_THRESHOLD = 0.4;
}
