// Shared surface program; wrappers only choose which faces are visible.
uniform sampler2D albedo_tex : source_color, filter_linear_mipmap_anisotropic, repeat_enable;
uniform sampler2D normal_tex : hint_normal, filter_linear_mipmap_anisotropic, repeat_enable;
uniform sampler2D roughness_tex : hint_roughness_r, filter_linear_mipmap_anisotropic, repeat_enable;

uniform vec4 tint : source_color = vec4(1.0);
uniform float uv_scale = 0.20;
uniform float macro_uv_scale = 0.035;
uniform float macro_influence = 0.15;
uniform float brightness = 1.0;
uniform vec3 albedo_floor = vec3(0.0);
uniform float floor_influence = 0.0;

varying vec3 world_pos;

#include "res://scripts/shaders/ground_paving.gdshaderinc"

void vertex() {
	world_pos = (MODEL_MATRIX * vec4(VERTEX, 1.0)).xyz;
}

void fragment() {
	TANGENT = paving_tangent(NORMAL, VIEW_MATRIX);
	BINORMAL = normalize(cross(TANGENT, NORMAL));

	PavingSample paving = sample_paving(
		albedo_tex, normal_tex, roughness_tex, world_pos.xz,
		uv_scale, macro_uv_scale, macro_influence, brightness, albedo_floor, floor_influence
	);

	ALBEDO = paving.albedo * tint.rgb;
	NORMAL_MAP = paving.normal_map;
	ROUGHNESS = paving.roughness;
	METALLIC = 0.0;
	SPECULAR = PAVING_SPECULAR;
}

void light() {
	DIFFUSE_LIGHT += paving_diffuse(NORMAL, LIGHT, LIGHT_COLOR, ATTENUATION);
}
