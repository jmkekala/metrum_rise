shader_type spatial;
render_mode cull_disabled;

uniform sampler2D albedo_tex : source_color, filter_linear_mipmap_anisotropic, repeat_enable;
uniform sampler2D roughness_tex : hint_roughness_r, filter_linear_mipmap_anisotropic, repeat_enable;

uniform float uv_scale = 0.1;

varying vec3 world_pos;
varying vec3 world_normal;
uniform float specular = 0.5;
// The share of the ambient light inside a tunnel's lining.
uniform float lining_ambient = 0.6;
// A tunnel's lamp strips (ROAD-61 round R8b): emissive bands across the ceiling, centred in each
// spacing of the lamp station the lining's UV.x carries (Rust's TUNNEL_LAMP_SPACING_M), so no
// Light3D is needed. Their light on the walls is emission too: strongest at the top of the wall
// and fading with the distance along the bore from the nearest lamp.
uniform float lamp_spacing_m = 10.0;
uniform float lamp_length_m = 1.2;
uniform vec3 lamp_color : source_color = vec3(1.0, 0.9, 0.72);
uniform float lamp_energy = 3.0;
uniform float lamp_wall_glow = 0.45;
uniform float lamp_glow_reach_m = 2.5;
// Rust's TUNNEL_LINING_HEIGHT_M: the lining's UV.y at the ceiling.
uniform float lining_height_m = 4.5;

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
	// A tunnel lining (colour alpha 0, ROAD-61) sees no sky: no sun reaches it (below), and the
	// sky's reflection and part of the ambient go with it. Bridges keep the defaults.
	float lining = 1.0 - COLOR.a;
	SPECULAR = mix(0.5, 0.0, lining);
	AO = mix(1.0, lining_ambient, lining);
	
	// Fix B_BRIDGE4: Add a small depth bias to prevent Z-fighting with deck edges
	DEPTH = FRAGCOORD.z - 0.0001;
	// A lining's sun share is none: its own shadow, exact where the shadow map leaks at its
	// corners (COLOR.a is 1 elsewhere).
	standard_light_cloud_sun = standard_light_cloud(VERTEX, INV_VIEW_MATRIX) * COLOR.a;
	// Lamp strips on a lined span's ceiling (UV.y is -1 on faces without lamps); a few ALU
	// operations per lining fragment, none for a bridge's.
	if (lining > 0.5 && UV.y >= 0.0) {
		// Metres along the bore from the nearest lamp's centre.
		float from_lamp = abs(fract(UV.x / lamp_spacing_m) - 0.5) * lamp_spacing_m;
		float edge = max(fwidth(UV.x), 1e-4);
		float ceiling = step(world_normal.y, -0.5);
		float strip = 1.0 - smoothstep(0.5 * lamp_length_m - edge, 0.5 * lamp_length_m + edge, from_lamp);
		float reach = from_lamp / lamp_glow_reach_m;
		float wall = (1.0 - ceiling) * exp(-reach * reach) * clamp(UV.y / lining_height_m, 0.0, 1.0);
		EMISSION += lamp_color * (lamp_energy * strip * ceiling + lamp_wall_glow * wall) * mix(vec3(1.0), ALBEDO, 1.0 - ceiling);
	}
	EMISSION += ALBEDO * (1.0 - METALLIC) * street_lamp_surface(VERTEX, NORMAL, TANGENT, BINORMAL,
		NORMAL_MAP, NORMAL_MAP_DEPTH, INV_VIEW_MATRIX, CAMERA_POSITION_WORLD);
}
