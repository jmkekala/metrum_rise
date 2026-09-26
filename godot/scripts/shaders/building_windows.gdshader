shader_type spatial;
render_mode blend_mix, depth_draw_opaque, cull_back, diffuse_burley, specular_schlick_ggx;

#include "res://scripts/shaders/window_schedule.gdshaderinc"
global uniform vec2 scene_window_clock; // hour, true solar elevation (including moonlit hours)
uniform bool window_preview = false;
uniform vec2 window_preview_clock = vec2(12.0, 45.0);
uniform vec4 window_preview_schedule = vec4(4.0, 24.5, 6.0, 1.0);
uniform float window_strength = 1.0;
uniform vec3 window_tint = vec3(1.0, 0.55, 0.23);

uniform vec4 albedo : source_color = vec4(1.0);
uniform sampler2D texture_albedo : source_color, hint_default_white, filter_linear_mipmap;
uniform sampler2D texture_emission : source_color, hint_default_black, filter_linear_mipmap;
uniform sampler2D texture_normal : hint_normal, filter_linear_mipmap;
uniform sampler2D texture_roughness : hint_default_white, filter_linear_mipmap;
uniform sampler2D texture_metallic : hint_default_white, filter_linear_mipmap;
uniform sampler2D texture_ao : hint_default_white, filter_linear_mipmap;
uniform vec4 roughness_channel = vec4(1.0, 0.0, 0.0, 0.0);
uniform vec4 metallic_channel = vec4(1.0, 0.0, 0.0, 0.0);
uniform vec4 ao_channel = vec4(1.0, 0.0, 0.0, 0.0);
uniform float roughness = 1.0;
uniform float metallic = 0.0;
uniform float specular = 0.5;
uniform float normal_scale = 1.0;
uniform float ao_light_affect = 0.0;
uniform float alpha_scissor = 0.5;
uniform vec3 uv1_scale = vec3(1.0);
uniform vec3 uv1_offset = vec3(0.0);
uniform vec3 uv2_scale = vec3(1.0);
uniform vec3 uv2_offset = vec3(0.0);
uniform bool emission_uv2 = false;
uniform bool ao_uv2 = false;
varying float window_level;

void vertex() {
	UV = UV * uv1_scale.xy + uv1_offset.xy;
	UV2 = UV2 * uv2_scale.xy + uv2_offset.xy;
	vec4 schedule = window_preview ? window_preview_schedule : INSTANCE_CUSTOM;
	vec2 clock = window_preview ? window_preview_clock : scene_window_clock;
	window_level = window_activity(schedule, clock.x, clock.y);
}

void fragment() {
	vec4 base = albedo * texture(texture_albedo, UV);
#ifdef WINDOW_VERTEX_COLOR
	base *= COLOR;
#endif
	ALBEDO = base.rgb;
	METALLIC = metallic * dot(texture(texture_metallic, UV), metallic_channel);
	ROUGHNESS = roughness * dot(texture(texture_roughness, UV), roughness_channel);
	SPECULAR = specular;
#ifdef WINDOW_NORMAL
	NORMAL_MAP = texture(texture_normal, UV).rgb;
	NORMAL_MAP_DEPTH = normal_scale;
#endif
#ifdef WINDOW_AO
	AO = dot(texture(texture_ao, ao_uv2 ? UV2 : UV), ao_channel);
	AO_LIGHT_AFFECT = ao_light_affect;
#endif
#ifdef WINDOW_TWO_SIDED
	if (!FRONT_FACING) { NORMAL = -NORMAL; }
#endif
#ifdef WINDOW_ALPHA
	ALPHA = base.a;
#endif
#ifdef WINDOW_CUTOUT
	ALPHA_SCISSOR_THRESHOLD = alpha_scissor;
#endif
#ifdef WINDOW_HASH
	ALPHA_HASH_SCALE = 1.0;
#endif
	EMISSION = texture(texture_emission, emission_uv2 ? UV2 : UV).rgb * window_tint * window_strength * window_level;
}
