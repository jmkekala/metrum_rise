shader_type spatial;
render_mode cull_disabled, depth_draw_opaque;

uniform sampler2D albedo_tex : source_color, filter_linear_mipmap_anisotropic, repeat_enable;
uniform sampler2D normal_tex : hint_normal, filter_linear_mipmap_anisotropic, repeat_enable;
uniform sampler2D roughness_tex : hint_roughness_r, filter_linear_mipmap_anisotropic, repeat_enable;
uniform sampler2D displacement_tex : hint_default_black, filter_linear_mipmap_anisotropic, repeat_enable;

uniform vec2 uv_scale = vec2(0.12, 0.12);
uniform vec2 vertical_uv_scale = vec2(0.12, 0.12);
uniform vec4 tint : source_color = vec4(1.0);
uniform float vertical_normal_strength = 0.25;

varying vec3 world_pos;
varying vec3 world_normal;

void vertex() {
	world_pos = (MODEL_MATRIX * vec4(VERTEX, 1.0)).xyz;
	world_normal = normalize((MODEL_MATRIX * vec4(NORMAL, 0.0)).xyz);
}

vec2 vertical_face_uv(vec3 pos, vec3 normal) {
	vec2 tangent = vec2(-normal.z, normal.x);
	float tangent_len = length(tangent);
	if (tangent_len <= 0.001) {
		tangent = vec2(1.0, 0.0);
	} else {
		tangent /= tangent_len;
	}
	return vec2(dot(pos.xz, tangent), pos.y) * vertical_uv_scale;
}

void fragment() {
	vec3 face_normal = normalize(world_normal);
	float horizontal_mask = smoothstep(0.45, 0.80, abs(face_normal.y));
	vec2 top_uv = world_pos.xz * uv_scale;
	vec2 wall_uv = vertical_face_uv(world_pos, face_normal);
	vec2 detail_uv = mix(wall_uv, top_uv, horizontal_mask);

	vec3 albedo = texture(albedo_tex, detail_uv).rgb * tint.rgb;
	vec3 normal = texture(normal_tex, detail_uv).rgb;
	float normal_strength = mix(vertical_normal_strength, 1.0, horizontal_mask);
	normal = mix(vec3(0.5, 0.5, 1.0), normal, normal_strength);
	float roughness = texture(roughness_tex, detail_uv).r;

	ALBEDO = albedo;
	NORMAL_MAP = normal;
	ROUGHNESS = roughness;
	METALLIC = 0.0;
}
