shader_type spatial;
render_mode cull_disabled, depth_draw_opaque;

uniform sampler2D albedo_tex : source_color, filter_linear_mipmap_anisotropic, repeat_enable;
uniform sampler2D normal_tex : hint_normal, filter_linear_mipmap_anisotropic, repeat_enable;
uniform sampler2D roughness_tex : hint_roughness_r, filter_linear_mipmap_anisotropic, repeat_enable;
uniform sampler2D displacement_tex : hint_default_black, filter_linear_mipmap_anisotropic, repeat_enable;

uniform vec4 tint : source_color = vec4(1.0);
uniform float uv_scale = 0.20;
uniform float macro_uv_scale = 0.035;
uniform float macro_influence = 0.15;
uniform float brightness = 1.0;
uniform vec3 albedo_floor = vec3(0.0);
uniform float floor_influence = 0.0;

varying vec3 world_pos;

void vertex() {
	world_pos = (MODEL_MATRIX * vec4(VERTEX, 1.0)).xyz;
}

void fragment() {
	vec2 detail_uv = world_pos.xz * uv_scale;
	vec2 macro_uv = world_pos.xz * macro_uv_scale;

	vec3 detail_albedo = texture(albedo_tex, detail_uv).rgb;
	vec3 macro_albedo = texture(albedo_tex, macro_uv).rgb;
	vec3 albedo = detail_albedo * mix(vec3(1.0), macro_albedo * 1.2, macro_influence) * brightness;
	albedo = mix(albedo, max(albedo, albedo_floor), floor_influence);
	vec3 normal = texture(normal_tex, detail_uv).rgb;
	float roughness_micro = texture(roughness_tex, detail_uv).r;
	float roughness_macro = texture(roughness_tex, macro_uv).r;
	float roughness = mix(roughness_micro, roughness_macro, macro_influence * 0.5);

	ALBEDO = albedo * tint.rgb;
	NORMAL_MAP = normal;
	ROUGHNESS = roughness;
	METALLIC = 0.0;
}
