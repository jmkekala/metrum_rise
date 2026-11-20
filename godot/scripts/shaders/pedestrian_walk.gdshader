shader_type spatial;
render_mode cull_back;

// VAT: width=num_verts, height=num_frames
// Each pixel = (delta_x, delta_y, delta_z, 1.0) in Godot Y-up model space.
uniform sampler2D vat_texture : hint_default_black, filter_nearest;
uniform sampler2D albedo_texture : source_color, filter_nearest;
uniform float     num_frames  = 30.0;
uniform vec3      albedo_tint = vec3(1.0, 1.0, 1.0);
// Palette texels recoloured per walker from INSTANCE_CUSTOM.y: top, trousers, hair (-1: none).
uniform ivec3     garment_slots = ivec3(-1);
uniform int       debug_mode  = 0;
uniform bool      reverse_phase = false;
// The walkers' draw distance fade band from the camera, metres (agents.gd sets it from Rust).
uniform vec2      draw_fade_m = vec2(1.0e9, 2.0e9);

varying float vat_id_debug;
varying float vat_offset_debug;
varying flat float draw_fade;
varying flat float look;

// Everyday colours, sRGB; a walker's look picks one of each.
const int TOP_COUNT = 14;
const vec3 TOPS[TOP_COUNT] = vec3[](
	vec3(0.16, 0.22, 0.36), vec3(0.86, 0.86, 0.84), vec3(0.12, 0.12, 0.13),
	vec3(0.58, 0.16, 0.16), vec3(0.20, 0.36, 0.24), vec3(0.75, 0.60, 0.22),
	vec3(0.44, 0.60, 0.75), vec3(0.48, 0.48, 0.48), vec3(0.40, 0.12, 0.18),
	vec3(0.75, 0.67, 0.55), vec3(0.16, 0.44, 0.44), vec3(0.80, 0.44, 0.20),
	vec3(0.55, 0.42, 0.62), vec3(0.30, 0.30, 0.34));
const int BOTTOM_COUNT = 6;
const vec3 BOTTOMS[BOTTOM_COUNT] = vec3[](
	vec3(0.20, 0.28, 0.42), vec3(0.14, 0.18, 0.28), vec3(0.10, 0.10, 0.11),
	vec3(0.58, 0.53, 0.40), vec3(0.35, 0.35, 0.37), vec3(0.35, 0.26, 0.18));
const int HAIR_COUNT = 6;
const vec3 HAIRS[HAIR_COUNT] = vec3[](
	vec3(0.08, 0.07, 0.06), vec3(0.22, 0.14, 0.09), vec3(0.37, 0.25, 0.16),
	vec3(0.72, 0.59, 0.37), vec3(0.59, 0.57, 0.55), vec3(0.43, 0.20, 0.12));

// A look's digit `salt` picks an entry from a list of `count`.
int pick(float walker_look, float salt, int count) {
	return min(int(fract(walker_look * salt) * float(count)), count - 1);
}
uniform float specular = 0.5;

// Godot's lighting with the cloud shadow on the sun; reads `specular`.
#include "res://scripts/shaders/standard_light.gdshaderinc"
#include "res://scripts/shaders/agent_draw_fade.gdshaderinc"

void vertex() {
	// walk_phase [0..1] passed from Rust via INSTANCE_CUSTOM.x
	float phase = reverse_phase ? fract(1.0 - INSTANCE_CUSTOM.x) : INSTANCE_CUSTOM.x;

	// UV2.x encodes the geometric vertex index as (vi + 0.5) / num_verts.
	// This is baked into the GLTF by bake_vat_blend.py via a "UVMap_VertexID" layer.
	float vat_u = UV2.x;

	// Phase [0,1] → a position between two baked rows; blend the pair so the legs move
	// smoothly between the cycle's 30 baked poses instead of snapping from one to the next.
	float frame = phase * (num_frames - 1.0);
	float row0 = floor(frame);
	float row1 = min(row0 + 1.0, num_frames - 1.0);
	vec3 offset0 = texture(vat_texture, vec2(vat_u, (row0 + 0.5) / num_frames)).rgb;
	vec3 offset1 = texture(vat_texture, vec2(vat_u, (row1 + 0.5) / num_frames)).rgb;
	vec3 offset = mix(offset0, offset1, frame - row0);
	vat_id_debug = vat_u;
	vat_offset_debug = length(offset);
	if (debug_mode != 1 && debug_mode != 2) {
		VERTEX += offset;
	}
	look = INSTANCE_CUSTOM.y;
	draw_fade = agent_draw_fade(MODEL_MATRIX, MAIN_CAM_INV_VIEW_MATRIX, draw_fade_m);
	// Faded out: every triangle collapses to the origin and rasterizes nothing.
	if (draw_fade <= 0.0) {
		VERTEX = vec3(0.0);
	}
}

#include "res://scripts/shaders/street_lamp_light.gdshaderinc"

void fragment() {
	if (agent_draw_dropped(draw_fade, FRAGCOORD.xy)) {
		discard;
	}
	if (debug_mode == 2) {
		ALBEDO = vec3(vat_id_debug, fract(vat_id_debug * 32.0), 1.0 - vat_id_debug);
	} else if (debug_mode == 3) {
		float intensity = clamp(vat_offset_debug * 4.0, 0.0, 1.0);
		ALBEDO = mix(vec3(0.05, 0.08, 0.12), vec3(1.0, 0.2, 0.05), intensity);
	} else {
		ALBEDO = texture(albedo_texture, UV).rgb;
		// The palette is one texel per material: recolour this walker's clothes and hair.
		int slot = int(UV.x * float(textureSize(albedo_texture, 0).x));
		if (slot == garment_slots.x) {
			ALBEDO = pow(TOPS[pick(look, 1.0, TOP_COUNT)], vec3(2.2));
		} else if (slot == garment_slots.y) {
			ALBEDO = pow(BOTTOMS[pick(look, float(TOP_COUNT), BOTTOM_COUNT)], vec3(2.2));
		} else if (slot == garment_slots.z) {
			ALBEDO = pow(HAIRS[pick(look, float(TOP_COUNT * BOTTOM_COUNT), HAIR_COUNT)], vec3(2.2));
		}
		ALBEDO *= albedo_tint;
	}
	ROUGHNESS = 0.85;
	METALLIC  = 0.0;
	standard_light_cloud_sun = standard_light_cloud(VERTEX, INV_VIEW_MATRIX);
	EMISSION += ALBEDO * (1.0 - METALLIC) * street_lamp_surface(VERTEX, NORMAL, TANGENT, BINORMAL,
		NORMAL_MAP, NORMAL_MAP_DEPTH, INV_VIEW_MATRIX, CAMERA_POSITION_WORLD);
}
