shader_type spatial;
render_mode cull_disabled;

uniform sampler2D albedo_tex : source_color, filter_linear_mipmap_anisotropic, repeat_enable;
uniform sampler2D roughness_tex : hint_roughness_r, filter_linear_mipmap_anisotropic, repeat_enable;

uniform float uv_scale = 0.1;

varying vec3 world_pos;
varying vec3 world_normal;
uniform float specular = 0.5;

// Godot's lighting with the cloud shadow on the sun; reads `specular`.
#include "res://scripts/shaders/standard_light.gdshaderinc"
#include "res://scripts/shaders/weather_rain.gdshaderinc"

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

#include "res://scripts/shaders/street_lamp_light.gdshaderinc"

void fragment() {
	vec3 blending = abs(world_normal);
	blending /= (blending.x + blending.y + blending.z);
	
	vec3 tri_pos = world_pos * uv_scale;
	
	vec3 albedo = triplanar_texture(albedo_tex, blending, tri_pos);
	float roughness = triplanar_texture(roughness_tex, blending, tri_pos).r;
	
	vec3 surface_albedo = albedo * COLOR.rgb;
	weather_wet_surface(weather_wetness((INV_VIEW_MATRIX * vec4(NORMAL, 0.0)).y), 0.9,
		surface_albedo, roughness);
	ALBEDO = surface_albedo;
	NORMAL = normalize(NORMAL);
	ROUGHNESS = roughness;
	METALLIC = 0.0;
	
	// Fix B_BRIDGE4: Add a small depth bias to prevent Z-fighting with deck edges
	DEPTH = FRAGCOORD.z - 0.0001;
	standard_light_cloud_sun = standard_light_cloud(VERTEX, INV_VIEW_MATRIX);
	EMISSION += ALBEDO * (1.0 - METALLIC) * street_lamp_surface(VERTEX, NORMAL, TANGENT, BINORMAL,
		NORMAL_MAP, NORMAL_MAP_DEPTH, INV_VIEW_MATRIX, CAMERA_POSITION_WORLD);
}
