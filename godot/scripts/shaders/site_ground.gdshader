shader_type spatial;
render_mode cull_disabled;

// As in terrain, the day cycle scales the authored ambient floor. IRRADIANCE suppresses
// the environment's ambient term without preventing window light spill (SSIL).
global uniform vec4 scene_ambient_light : source_color;
// Colour vision washes out at night. Dimming alone leaves a saturated green ground reading as
// daylight with the brightness turned down, because the albedo's green channel still dominates
// whatever the ambient tint is multiplied by.
global uniform float scene_ambient_desaturation;
uniform float ground_shadow_ambient = 0.64;
uniform float ground_shadow_sun_strength = 0.36;
uniform float ground_shadow_min_visibility = 0.10;

varying vec3 world_pos;
varying vec3 world_normal;
// Share of direct sunlight the clouds let through, from fragment() to light().
varying float cloud_sun;

#include "res://scripts/shaders/ground_grass.gdshaderinc"
#include "res://scripts/shaders/weather_clouds.gdshaderinc"
#include "res://scripts/shaders/weather_rain.gdshaderinc"
#include "res://scripts/shaders/ground_paving.gdshaderinc"

void vertex() {
	world_pos = (MODEL_MATRIX * vec4(VERTEX, 1.0)).xyz;
	world_normal = normalize((MODEL_MATRIX * vec4(NORMAL, 0.0)).xyz);
}

#include "res://scripts/shaders/street_lamp_light.gdshaderinc"

void fragment() {
	cloud_sun = weather_cloud_sun(world_pos);
	IRRADIANCE = vec4(0.0, 0.0, 0.0, 1.0);
	RADIANCE = vec4(0.0, 0.0, 0.0, 1.0);
	float macro_mask = macro_variation(world_pos.xz);
	vec3 color = terrain_surface_color(world_pos.y, macro_mask, 0.0);
	color = apply_natural_land_variation(color, world_pos.xz, 0.0, 0.0, 0.0, 0.0, 0.0);
	color = apply_grass_detail(color, world_pos, 1.0);

	float shade = hillshade_mask(normalize(world_normal));
	vec3 hillshade_tint = mix(hillshade_shadow_tint, hillshade_light_tint, shade);
	color *= mix(vec3(1.0), hillshade_tint, hillshade_strength);

	// Desaturated before it is either shown or lit, so the moonlit diffuse washes out
	// with the ambient floor rather than staying vivid against a grey ground.
	color = mix(color, vec3(dot(color, vec3(0.2126, 0.7152, 0.0722))), scene_ambient_desaturation);
	// Wet as the terrain around it is, before the ambient floor is emitted from it.
	float site_roughness = 0.96;
	weather_wet_surface(weather_wetness(normalize(world_normal).y), 1.0, color, site_roughness);
	ALBEDO = color;
	EMISSION = color * clamp(ground_shadow_ambient, 0.0, 1.0) * scene_ambient_light.rgb;
	ROUGHNESS = site_roughness;
	SPECULAR = 0.08;
	EMISSION += ALBEDO * (1.0 - METALLIC) * street_lamp_surface(VERTEX, NORMAL, TANGENT, BINORMAL,
		NORMAL_MAP, NORMAL_MAP_DEPTH, INV_VIEW_MATRIX, CAMERA_POSITION_WORLD);
}

void light() {
	// The shadow visibility floor belongs to sun/moon light only. Local lights must retain
	// their distance/cone/projector attenuation, including zero outside a headlight beam.
	if (!LIGHT_IS_DIRECTIONAL) {
		DIFFUSE_LIGHT += paving_diffuse(NORMAL, LIGHT, LIGHT_COLOR, ATTENUATION);
	} else {
		float shadow_visibility = mix(
			clamp(ground_shadow_min_visibility, 0.0, 1.0),
			1.0,
			clamp(ATTENUATION, 0.0, 1.0)
		);
		float facing = clamp(dot(NORMAL, LIGHT), 0.0, 1.0);
		float soft_facing = mix(0.82, 1.0, facing);
		// Clouds dim the sun only.
		DIFFUSE_LIGHT += ALBEDO
			* LIGHT_COLOR
			* ground_shadow_sun_strength
			* shadow_visibility
			* soft_facing
			* cloud_sun;
	}
}
