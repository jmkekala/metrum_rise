shader_type spatial;
render_mode cull_disabled, depth_draw_opaque;

uniform sampler2D albedo_tex : source_color, filter_linear_mipmap_anisotropic, repeat_enable;
uniform sampler2D normal_tex : hint_normal, filter_linear_mipmap_anisotropic, repeat_enable;
uniform sampler2D roughness_tex : hint_roughness_r, filter_linear_mipmap_anisotropic, repeat_enable;

uniform vec2 uv_scale = vec2(0.12, 0.12);
uniform vec2 vertical_uv_scale = vec2(0.12, 0.12);
uniform vec4 tint : source_color = vec4(1.0);
uniform float vertical_normal_strength = 0.25;
// Granite kerbstones: light grey with dark mica and feldspar specks from the aggregate texture
// (R channel, linear), sampled per metre of kerb.
uniform sampler2D detail_texture : filter_linear_mipmap_anisotropic, repeat_enable;
uniform vec3 granite_albedo = vec3(0.40, 0.395, 0.38);

// Road skirts (TERRAIN-07-B, docs/terrain.md "Skirts"): the skirt material sets `skirt`, and its
// UV.y is 1 on a skirt's bottom vertex, 0.5 m below the outer chain, and 0 on the chain (in
// between at chunk-line clip points). Coarser terrain far away sags further below a road's edge,
// so the bottom deepens with the camera distance, measured as terrain.gd measures patch LOD:
// 0.5 m to 2 km, 2.5 m to 5 km, 5 m to 12 km and 10 m beyond (T5). A clip point moves in
// proportion, so within one distance band a deepened strip stays planar; a point two chunks
// share gets the same drop in both.
uniform bool skirt = false;
const float SKIRT_NEAR_DEPTH_M = 0.5;

varying vec3 world_pos;
varying vec3 world_normal;
uniform float specular = 0.5;

// Godot's lighting with the cloud shadow on the sun; reads `specular`.
#include "res://scripts/shaders/standard_light.gdshaderinc"
#include "res://scripts/shaders/weather_rain.gdshaderinc"

float skirt_depth_m(float distance_m) {
	if (distance_m <= 2000.0) {
		return SKIRT_NEAR_DEPTH_M;
	}
	if (distance_m <= 5000.0) {
		return 2.5;
	}
	if (distance_m <= 12000.0) {
		return 5.0;
	}
	return 10.0;
}

void vertex() {
	world_pos = (MODEL_MATRIX * vec4(VERTEX, 1.0)).xyz;
	if (skirt) {
		// Road chunks are translated, never rotated or scaled: a model-space drop is a world one.
		float camera_m = distance(world_pos, INV_VIEW_MATRIX[3].xyz);
		float drop_m = (skirt_depth_m(camera_m) - SKIRT_NEAR_DEPTH_M) * UV.y;
		VERTEX.y -= drop_m;
		world_pos.y -= drop_m;
	}
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

#include "res://scripts/shaders/street_lamp_light.gdshaderinc"

void fragment() {
	vec3 face_normal = normalize(world_normal);
	float horizontal_mask = smoothstep(0.45, 0.80, abs(face_normal.y));
	vec2 top_uv = world_pos.xz * uv_scale;
	vec2 wall_uv = vertical_face_uv(world_pos, face_normal);
	vec2 detail_uv = mix(wall_uv, top_uv, horizontal_mask);

	vec2 metres = detail_uv / max(uv_scale, vec2(1.0e-4));
	float speck = texture(detail_texture, metres / 0.35).r;
	vec3 albedo = granite_albedo * (0.72 + 0.85 * (speck - 0.34)) * tint.rgb;
	vec3 normal = texture(normal_tex, detail_uv).rgb;
	float normal_strength = mix(vertical_normal_strength, 1.0, horizontal_mask);
	normal = mix(vec3(0.5, 0.5, 1.0), normal, normal_strength);
	float roughness = texture(roughness_tex, detail_uv).r;

	weather_wet_surface(weather_wetness((INV_VIEW_MATRIX * vec4(NORMAL, 0.0)).y), 0.9, albedo,
		roughness);
	ALBEDO = albedo;
	NORMAL_MAP = normal;
	ROUGHNESS = roughness;
	METALLIC = 0.0;
	standard_light_cloud_sun = standard_light_cloud(VERTEX, INV_VIEW_MATRIX);
	EMISSION += ALBEDO * (1.0 - METALLIC) * street_lamp_surface(VERTEX, NORMAL, TANGENT, BINORMAL,
		NORMAL_MAP, NORMAL_MAP_DEPTH, INV_VIEW_MATRIX, CAMERA_POSITION_WORLD);
}
