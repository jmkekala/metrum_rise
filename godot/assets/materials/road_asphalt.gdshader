shader_type spatial;
render_mode cull_back, depth_draw_opaque;

// Carriageway asphalt (`ROAD-56`). Road stretches carry road-space UVs and a packed road
// description in COLOR (road_space_color in rust/src/simulation/network/render/road/mod.rs);
// junction asphalt carries world UVs and a zero COLOR.

#include "res://scripts/shaders/ground_paving.gdshaderinc"
#include "res://scripts/shaders/road_surface_finnish.gdshaderinc"

uniform sampler2D detail_texture : filter_linear_mipmap_anisotropic, repeat_enable;
uniform float wetness : hint_range(0.0, 1.0) = 0.0;
uniform float lane_width_m = 3.5;

varying vec3 world_pos;
// CUSTOM0 is zero on ordinary asphalt; crossings carry metres across/along, age+1, kerb distance.
varying vec4 crossing;

void vertex() {
	crossing = CUSTOM0;
	world_pos = (MODEL_MATRIX * vec4(VERTEX, 1.0)).xyz;
}

#include "res://scripts/shaders/street_lamp_light.gdshaderinc"

void fragment() {
	// 8-bit channels: lane counts and km/h arrive as exact integers; alpha >= 128 is road space.
	vec4 road = COLOR * 255.0;
	float lanes = round(road.r) + round(road.g);
	bool lane_space = road.a >= 127.5 && lanes > 0.5;
	float age = road.a >= 127.5 ? (road.a - 128.0) / 127.0 : 0.5;
	vec2 road_m = lane_space ? UV : world_pos.xz;
	// Studded tyres rut multi-lane and fast roads; a two-lane street at 50 km/h wears evenly.
	float traffic = max(clamp((lanes - 2.0) * 0.5, 0.0, 1.0), smoothstep(55.0, 85.0, round(road.b)));

	vec3 albedo;
	float roughness;
	float height_m;
	vec2 micro_slope;
	float water;
	fr_surface_painted(detail_texture, world_pos.xz, road_m, lane_space, max(lanes, 1.0), lane_width_m,
		traffic, age, wetness, false, 0.0, length(fwidth(road_m)), crossing,
		albedo, roughness, height_m, micro_slope, water);
	vec3 relief = fr_relief_normal(road_m, height_m, micro_slope, water);

	// Tangent frame along road_m from screen derivatives, so relief follows the road's direction.
	vec3 dp1 = dFdx(VERTEX);
	vec3 dp2 = dFdy(VERTEX);
	vec2 duv1 = dFdx(road_m);
	vec2 duv2 = dFdy(road_m);
	vec3 dp2perp = cross(dp2, NORMAL);
	vec3 dp1perp = cross(NORMAL, dp1);
	vec3 t = dp2perp * duv1.x + dp1perp * duv2.x;
	vec3 b = dp2perp * duv1.y + dp1perp * duv2.y;
	float frame_scale = inversesqrt(max(max(dot(t, t), dot(b, b)), 1e-20));
	TANGENT = t * frame_scale;
	BINORMAL = b * frame_scale;
	NORMAL_MAP = relief * 0.5 + 0.5;

	ALBEDO = albedo;
	ROUGHNESS = roughness;
	METALLIC = 0.0;
	SPECULAR = PAVING_SPECULAR;
	EMISSION += ALBEDO * (1.0 - METALLIC) * street_lamp_surface(VERTEX, NORMAL, TANGENT, BINORMAL,
		NORMAL_MAP, NORMAL_MAP_DEPTH, INV_VIEW_MATRIX, CAMERA_POSITION_WORLD);
}

void light() {
	DIFFUSE_LIGHT += paving_diffuse(NORMAL, LIGHT, LIGHT_COLOR, ATTENUATION);
	SPECULAR_LIGHT += fr_sun_specular(NORMAL, LIGHT, VIEW, ROUGHNESS, LIGHT_COLOR, ATTENUATION)
		* SPECULAR_AMOUNT;
}
