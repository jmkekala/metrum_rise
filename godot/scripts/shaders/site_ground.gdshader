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

#include "res://scripts/shaders/ground_grass.gdshaderinc"

void vertex() {
	world_pos = (MODEL_MATRIX * vec4(VERTEX, 1.0)).xyz;
	world_normal = normalize((MODEL_MATRIX * vec4(NORMAL, 0.0)).xyz);
}

void fragment() {
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
	ALBEDO = color;
	EMISSION = color * clamp(ground_shadow_ambient, 0.0, 1.0) * scene_ambient_light.rgb;
	ROUGHNESS = 0.96;
	SPECULAR = 0.08;
}

void light() {
	float shadow_visibility = mix(
		clamp(ground_shadow_min_visibility, 0.0, 1.0),
		1.0,
		clamp(ATTENUATION, 0.0, 1.0)
	);
	float facing = clamp(dot(NORMAL, LIGHT), 0.0, 1.0);
	float soft_facing = mix(0.82, 1.0, facing);
	DIFFUSE_LIGHT += ALBEDO
		* LIGHT_COLOR
		* ground_shadow_sun_strength
		* shadow_visibility
		* soft_facing;
}
