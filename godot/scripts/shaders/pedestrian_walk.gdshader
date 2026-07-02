shader_type spatial;
render_mode cull_back;

// VAT: width=num_verts, height=num_frames
// Each pixel = (delta_x, delta_y, delta_z, 1.0) in Godot Y-up model space.
uniform sampler2D vat_texture : hint_default_black, filter_nearest;
uniform sampler2D albedo_texture : source_color, filter_nearest;
uniform float     num_frames  = 30.0;
uniform vec3      albedo_tint = vec3(1.0, 1.0, 1.0);
uniform int       debug_mode  = 0;
uniform bool      reverse_phase = false;

varying float vat_id_debug;
varying float vat_offset_debug;

void vertex() {
	// walk_phase [0..1] passed from Rust via INSTANCE_CUSTOM.x
	float phase = reverse_phase ? fract(1.0 - INSTANCE_CUSTOM.x) : INSTANCE_CUSTOM.x;

	// UV2.x encodes the geometric vertex index as (vi + 0.5) / num_verts.
	// This is baked into the GLTF by bake_vat_blend.py via a "UVMap_VertexID" layer.
	float vat_u = UV2.x;

	// Map phase [0,1] → texture row, centred within each texel.
	float vat_v = (phase * (num_frames - 1.0) + 0.5) / num_frames;

	vec3 offset = texture(vat_texture, vec2(vat_u, vat_v)).rgb;
	vat_id_debug = vat_u;
	vat_offset_debug = length(offset);
	if (debug_mode != 1 && debug_mode != 2) {
		VERTEX += offset;
	}
}

void fragment() {
	if (debug_mode == 2) {
		ALBEDO = vec3(vat_id_debug, fract(vat_id_debug * 32.0), 1.0 - vat_id_debug);
	} else if (debug_mode == 3) {
		float intensity = clamp(vat_offset_debug * 4.0, 0.0, 1.0);
		ALBEDO = mix(vec3(0.05, 0.08, 0.12), vec3(1.0, 0.2, 0.05), intensity);
	} else {
		ALBEDO = texture(albedo_texture, UV).rgb * albedo_tint;
	}
	ROUGHNESS = 0.85;
	METALLIC  = 0.0;
}
