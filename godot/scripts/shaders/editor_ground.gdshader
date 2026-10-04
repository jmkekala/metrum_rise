shader_type spatial;

// The asset editor's ground: the game's grass palette and the site ground's lighting, from two
// plain texture reads. The terrain shader drew the same flat plane with stochastic multi-scale
// sampling and noise fields, which cost more than the rest of the editor view together; a
// preview ground only has to read as lawn behind the asset.

global uniform vec4 scene_ambient_light : source_color;
global uniform float scene_ambient_desaturation;
uniform float ground_shadow_ambient = 0.64;
uniform float ground_shadow_sun_strength = 0.36;
uniform float ground_shadow_min_visibility = 0.10;
// Stands in for the meadow variation this shader skips, which pulls the terrain's lawn greener
// and less blue. Fitted to the terrain shader's mean lawn colour in the editor view.
uniform vec3 editor_ground_tint = vec3(0.91, 0.93, 0.80);

varying vec3 world_pos;

#include "res://scripts/shaders/ground_grass.gdshaderinc"

// One grass layer without the meadow noise: the photo's chroma over the palette's luminance.
vec3 editor_grass_layer(vec3 base_color, vec3 grass_color, float contrast) {
	float target_luma = color_luminance(base_color);
	float grass_luma = max(color_luminance(grass_color), 0.001);
	float detail = clamp((grass_luma - 0.28) * contrast, -0.32, 0.30);
	vec3 grass_contrast = clamp(
		(grass_color - vec3(grass_luma)) * max(terrain_grass_chroma_gain, 0.0) + vec3(grass_luma),
		vec3(0.0),
		vec3(1.0)
	);
	vec3 textured = mix(base_color, grass_contrast, clamp(0.50 + contrast * 0.20, 0.58, 0.82));
	return match_luminance(textured * (1.0 + detail), target_luma);
}

void vertex() {
	world_pos = (MODEL_MATRIX * vec4(VERTEX, 1.0)).xyz;
}

void fragment() {
	IRRADIANCE = vec4(0.0, 0.0, 0.0, 1.0);
	RADIANCE = vec4(0.0, 0.0, 0.0, 1.0);
	vec3 color = terrain_surface_color(0.0, 0.5, 0.0);
	float strength = clamp(terrain_grass_albedo_strength, 0.0, 1.0);
	vec3 macro_grass = texture(terrain_grass_albedo, world_pos.xz * terrain_grass_macro_scale).rgb;
	vec3 mid_grass = texture(terrain_grass_albedo, world_pos.xz * terrain_grass_mid_scale).rgb;
	color = mix(color, editor_grass_layer(color, macro_grass, 0.90), strength * terrain_grass_macro_strength);
	color = mix(color, editor_grass_layer(color, mid_grass, 1.10), strength * terrain_grass_mid_strength);
	color *= editor_ground_tint;
	// Flat ground faces up, so the authored hillshade is one constant tint.
	vec3 hillshade_tint = mix(hillshade_shadow_tint, hillshade_light_tint, hillshade_mask(vec3(0.0, 1.0, 0.0)));
	color *= mix(vec3(1.0), hillshade_tint, hillshade_strength);
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
	float soft_facing = mix(0.82, 1.0, clamp(dot(NORMAL, LIGHT), 0.0, 1.0));
	DIFFUSE_LIGHT += ALBEDO * LIGHT_COLOR * ground_shadow_sun_strength * shadow_visibility * soft_facing;
}
