shader_type spatial;
render_mode cull_disabled, unshaded;

#include "res://scripts/shaders/vegetation_impostor.gdshaderinc"

// The shadow caster for trees whose visible level is the impostor. It draws the same baked
// forms as vegetation_impostor.gdshader, but chooses and faces the views toward the light
// rather than the camera, so each tree casts its own silhouette seen from the sun: its species,
// its variant, and the gaps in its crown. Drawn only into the shadow maps, for every canopy tree
// at every distance: the branched level never casts; see vegetation.gd.
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

// One baked view per tree, the nearest to the light: a shadow needs no blend between views,
// and one fetch per texel keeps the pass cheap across every tree in the sun's range.
varying vec2 frame_uv;
varying flat vec3 cell_layer;

void vertex() {
	int layer = int(INSTANCE_CUSTOM.x);
	vec3 centre = bounds_centre[layer];
	// The shadow camera looks along the light; its back axis points at the sun. Object space,
	// so each tree's own yaw selects its views.
	vec3 view = normalize((inverse(MODEL_MATRIX) * vec4(INV_VIEW_MATRIX[2].xyz, 0.0)).xyz);
	vec2 cell = clamp(floor((hemi_encode(view) * 0.5 + 0.5) * 8.0), vec2(0.0), vec2(7.0));
	vec3 right = cross(vec3(0.0, 1.0, 0.0), view);
	right = dot(right, right) > 0.000001 ? normalize(right) : vec3(1.0, 0.0, 0.0);
	vec3 up = cross(view, right);
	// A shadow map is orthographic, so one of its texels spans the same width at any depth.
	float pixel_m = 2.0 / (PROJECTION_MATRIX[1][1] * VIEWPORT_SIZE.y);
	float pad = impostor_crop_pad(pixel_m / (bounds_size[layer] * length(MODEL_MATRIX[0].xyz)));
	vec4 rect = texelFetch(frame_bounds, ivec2(int(cell.x) + 8 * int(cell.y), layer), 0);
	vec4 crop = impostor_crop(vec4(1.0e9, 1.0e9, -1.0e9, -1.0e9), right, up, cell, rect, pad);
	vec3 offset = impostor_crop_offset(crop, right, up, UV);
	VERTEX = centre + (offset - view * LIGHT_SETBACK) * bounds_size[layer];
	frame_uv = project_frame(offset, cell);
	cell_layer = vec3(cell, float(layer));
}

float frame_lod(vec2 uv) {
	float footprint = max(length(dFdx(uv)), length(dFdy(uv))) * FRAME_PX;
	return clamp(log2(max(footprint, 1.0)), 0.0, log2(FRAME_PX));
}

vec3 atlas_uv(vec2 uv, vec2 cell, float lod) {
	float inset = 0.5 * exp2(ceil(lod)) / FRAME_PX;
	return vec3((cell + clamp(uv, vec2(inset), vec2(1.0 - inset))) / 8.0, cell_layer.z);
}

void fragment() {
	float lod = frame_lod(frame_uv);
	ALBEDO = vec3(0.0);
	ALPHA = textureLod(albedo_atlas, atlas_uv(frame_uv, cell_layer.xy, lod), lod).a;
	ALPHA_SCISSOR_THRESHOLD = 0.4;
}
