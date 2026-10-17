shader_type spatial;
render_mode blend_mix, depth_draw_opaque, cull_back, diffuse_burley, specular_schlick_ggx;

#include "res://scripts/shaders/window_schedule.gdshaderinc"
global uniform vec2 scene_window_clock; // hour, true solar elevation (including moonlit hours)
uniform bool window_preview = false;
uniform vec2 window_preview_clock = vec2(12.0, 45.0);
uniform vec4 window_preview_schedule = vec4(4.0, 24.5, 6.0, 1.0);
uniform float window_strength = 3.0;
uniform vec3 window_tint = vec3(1.0, 0.76, 0.52);

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
varying vec3 window_radiance;
// After the material uniforms: its light() reads `specular`.
#define STANDARD_LIGHT_DIFFUSE_BURLEY
#include "res://scripts/shaders/standard_light.gdshaderinc"
#include "res://scripts/shaders/weather_rain.gdshaderinc"
#include "res://scripts/shaders/water_reflection.gdshaderinc"
// Set only on the water reflection's instances (water_reflection_sources.gd), so the main view
// and previews share the material unchanged.
instance uniform bool water_reflection_proxy = false;

void vertex() {
	// A reflection instance lifts whatever lies below its mirror's water plane onto the plane,
	// so a shore building's foundation never stands in the mirror under the water. Per vertex
	// and only for those instances: a fragment discard cost the main view's buildings 0.3-0.4 ms.
	if (water_reflection_proxy) {
		vec3 world = (MODEL_MATRIX * vec4(VERTEX, 1.0)).xyz;
		float plane = water_reflection_mirror_plane(CAMERA_VISIBLE_LAYERS);
		if (world.y < plane) {
			world.y = plane;
			VERTEX = (inverse(MODEL_MATRIX) * vec4(world, 1.0)).xyz;
		}
	}
	UV = UV * uv1_scale.xy + uv1_offset.xy;
	UV2 = UV2 * uv2_scale.xy + uv2_offset.xy;
#ifdef WINDOW_EMISSION
	vec4 schedule = window_preview ? window_preview_schedule : INSTANCE_CUSTOM;
	vec2 clock = window_preview ? window_preview_clock : scene_window_clock;
	// Saved identity already determines these schedule values. Reuse them for cosmetic
	// variation: no time/position dependence, extra instance data or material variants.
	float brightness = mix(0.85, 1.15, fract(schedule.x * 13.37 + schedule.y * 7.13));
	float warmth = fract(schedule.y * 5.17 + schedule.z * 11.73);
	vec3 tint = window_tint * mix(vec3(1.0, 0.90, 0.82), vec3(1.0, 1.10, 1.18), warmth);
	window_radiance = tint * window_strength * brightness * window_activity(schedule, clock.x, clock.y);
#endif
}

#define SCENE_WINDOW_CLOCK_DECLARED
#include "res://scripts/shaders/street_lamp_light.gdshaderinc"

void fragment() {
	vec4 base = albedo * texture(texture_albedo, UV);
#ifdef WINDOW_VERTEX_COLOR
	base *= COLOR;
#endif
	ALBEDO = base.rgb;
	METALLIC = metallic * dot(texture(texture_metallic, UV), metallic_channel);
	ROUGHNESS = roughness * dot(texture(texture_roughness, UV), roughness_channel);
	SPECULAR = specular;
	// Rain wets roads, roofs and walls; metal sheds it rather than soaking it up.
	vec3 surface_albedo = ALBEDO;
	float surface_roughness = ROUGHNESS;
	weather_wet_surface(weather_wetness((INV_VIEW_MATRIX * vec4(NORMAL, 0.0)).y),
		0.8 * (1.0 - METALLIC), surface_albedo, surface_roughness);
	ALBEDO = surface_albedo;
	ROUGHNESS = surface_roughness;
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
#ifdef WINDOW_EMISSION
	EMISSION = texture(texture_emission, emission_uv2 ? UV2 : UV).rgb * window_radiance;
#endif
	standard_light_cloud_sun = standard_light_cloud(VERTEX, INV_VIEW_MATRIX);
	if (!window_preview) {
		EMISSION += ALBEDO * (1.0 - METALLIC) * mix(1.0, AO, AO_LIGHT_AFFECT) * street_lamp_surface(VERTEX, NORMAL, TANGENT, BINORMAL,
			NORMAL_MAP, NORMAL_MAP_DEPTH, INV_VIEW_MATRIX, CAMERA_POSITION_WORLD);
	}
}
