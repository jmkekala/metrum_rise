shader_type spatial;
render_mode cull_disabled;

uniform sampler2D albedo_tex : source_color, filter_linear_mipmap_anisotropic, repeat_enable;
uniform sampler2D normal_tex : hint_normal, filter_linear_mipmap_anisotropic, repeat_enable;
uniform sampler2D roughness_tex : hint_roughness_r, filter_linear_mipmap_anisotropic, repeat_enable;
uniform sampler2D displacement_tex : hint_default_black, filter_linear_mipmap_anisotropic, repeat_enable;

uniform float uv_scale = 0.1;

varying vec3 world_pos;
varying vec3 world_normal;

void vertex() {
	world_pos = (MODEL_MATRIX * vec4(VERTEX, 1.0)).xyz;
	world_normal = normalize((MODEL_MATRIX * vec4(NORMAL, 0.0)).xyz);
}

vec3 triplanar_texture(sampler2D p_sampler, vec3 p_weights, vec3 p_triplanar_pos) {
	vec3 samp = vec3(0.0);
	samp += texture(p_sampler, p_triplanar_pos.xy).rgb * p_weights.z;
	samp += texture(p_sampler, p_triplanar_pos.xz).rgb * p_weights.y;
	samp += texture(p_sampler, p_triplanar_pos.zy).rgb * p_weights.x;
	return samp;
}

void fragment() {
	vec3 blending = abs(world_normal);
	blending /= (blending.x + blending.y + blending.z);
	
	vec3 tri_pos = world_pos * uv_scale;
	
	vec3 albedo = triplanar_texture(albedo_tex, blending, tri_pos);
	vec3 normal = triplanar_texture(normal_tex, blending, tri_pos);
	float roughness = triplanar_texture(roughness_tex, blending, tri_pos).r;
	
	ALBEDO = albedo * COLOR.rgb;
	NORMAL = normalize(NORMAL);
	// Note: triplanar normal mapping is complex, using simplified approach here
	ROUGHNESS = roughness;
	METALLIC = 0.0;
	
	// Fix B_BRIDGE4: Add a small depth bias to prevent Z-fighting with deck edges
	DEPTH = FRAGCOORD.z - 0.0001;
}
