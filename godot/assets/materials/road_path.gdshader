shader_type spatial;
render_mode cull_back, depth_draw_opaque, specular_disabled;

// Sidewalks and footpaths (`ROAD-56`). Roadside sidewalks keep the shared paving look
// (paving_surface.gdshaderinc's uniforms and sampling). Footpath stretches carry road-space
// UVs and COLOR.g = 1 (footpath_color in rust/src/simulation/network/render/road/mod.rs) and
// draw as a Finnish light-traffic path instead.

varying vec3 world_pos;
// Share of direct sunlight the clouds let through, from fragment() to light(), as the yards'
// paving receives it (paving_surface.gdshaderinc).
varying float cloud_sun;

#include "res://scripts/shaders/ground_paving.gdshaderinc"
#include "res://scripts/shaders/weather_clouds.gdshaderinc"
#include "res://scripts/shaders/road_surface_finnish.gdshaderinc"
#include "res://scripts/shaders/path_surface_finnish.gdshaderinc"

// After the includes: their function parameters may not shadow a uniform.
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
uniform sampler2D detail_texture : filter_linear_mipmap_anisotropic, repeat_enable;
uniform float wetness : hint_range(0.0, 1.0) = 0.0;

void vertex() {
	world_pos = (MODEL_MATRIX * vec4(VERTEX, 1.0)).xyz;
}

#include "res://scripts/shaders/street_lamp_light.gdshaderinc"

void fragment() {
	cloud_sun = weather_cloud_sun(world_pos);
	// 8-bit channels: R half-width in decimetres, G footpath marker, B surface kind, A 128 + age.
	vec4 path = COLOR * 255.0;
	bool footpath = path.g > 127.5;
	// A zero half-width marks footpath junction and end pieces, which have no road space.
	bool road_space = footpath && path.r > 0.5;
	vec2 road_m = road_space ? UV : world_pos.xz;
	float half_width_m = road_space ? max(round(path.r) * 0.1, 0.5) : 1.0e4;
	float age = clamp((path.a - 128.0) / 127.0, 0.0, 1.0);
	float kind = round(path.b);

	// Both looks are evaluated in uniform control flow: the path functions take derivatives.
	vec3 path_albedo;
	float path_roughness;
	float height_m;
	vec2 micro_slope;
	float water;
	fp_surface(detail_texture, world_pos.xz, road_m, half_width_m, age, wetness,
		length(fwidth(road_m)), kind, path_albedo, path_roughness, height_m, micro_slope, water);
	vec3 relief = fr_relief_normal(road_m, height_m, micro_slope, water);
	vec3 dp1 = dFdx(VERTEX);
	vec3 dp2 = dFdy(VERTEX);
	vec2 duv1 = dFdx(road_m);
	vec2 duv2 = dFdy(road_m);
	vec3 dp2perp = cross(dp2, NORMAL);
	vec3 dp1perp = cross(NORMAL, dp1);
	vec3 t = dp2perp * duv1.x + dp1perp * duv2.x;
	vec3 b = dp2perp * duv1.y + dp1perp * duv2.y;
	float frame_scale = inversesqrt(max(max(dot(t, t), dot(b, b)), 1e-20));

	PavingSample paving = sample_paving(
		albedo_tex, normal_tex, roughness_tex, world_pos.xz,
		uv_scale, macro_uv_scale, macro_influence, brightness, albedo_floor, floor_influence
	);
	vec3 paving_t = paving_tangent(NORMAL, VIEW_MATRIX);

	TANGENT = footpath ? t * frame_scale : paving_t;
	BINORMAL = footpath ? b * frame_scale : normalize(cross(paving_t, NORMAL));
	NORMAL_MAP = footpath ? relief * 0.5 + 0.5 : paving.normal_map;
	ALBEDO = footpath ? path_albedo : paving.albedo * tint.rgb;
	ROUGHNESS = footpath ? path_roughness : paving.roughness;
	METALLIC = 0.0;
	SPECULAR = PAVING_SPECULAR;
	EMISSION += ALBEDO * (1.0 - METALLIC) * street_lamp_surface(VERTEX, NORMAL, TANGENT, BINORMAL,
		NORMAL_MAP, NORMAL_MAP_DEPTH, INV_VIEW_MATRIX, CAMERA_POSITION_WORLD);
}

void light() {
	DIFFUSE_LIGHT += paving_diffuse(NORMAL, LIGHT, LIGHT_COLOR, ATTENUATION)
		* (LIGHT_IS_DIRECTIONAL ? cloud_sun : 1.0);
}
