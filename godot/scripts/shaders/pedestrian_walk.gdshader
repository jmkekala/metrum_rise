shader_type spatial;
render_mode cull_back;

// Per-variant skin texture
uniform sampler2D albedo_texture : source_color, filter_linear_mipmap;

// VAT: width=num_verts, height=num_frames
// Each pixel = (delta_x, delta_y, delta_z, 1.0) in Godot Y-up model space.
uniform sampler2D vat_texture : hint_default_black, filter_nearest;
uniform float     num_frames  = 30.0;

void vertex() {
	// walk_phase [0..1] passed from Rust via INSTANCE_CUSTOM.x
	float phase = INSTANCE_CUSTOM.x;

	// UV2.x encodes the geometric vertex index as (vi + 0.5) / num_verts.
	// This is baked into the GLTF by bake_vat_blend.py via a "UVMap_VertexID" layer.
	float vat_u = UV2.x;

	// Map phase [0,1] → texture row, centred within each texel.
	float vat_v = (phase * (num_frames - 1.0) + 0.5) / num_frames;

	vec3 offset = texture(vat_texture, vec2(vat_u, vat_v)).rgb;
	VERTEX += offset;
}

void fragment() {
	ALBEDO    = texture(albedo_texture, UV).rgb;
	ROUGHNESS = 0.85;
	METALLIC  = 0.0;
}
