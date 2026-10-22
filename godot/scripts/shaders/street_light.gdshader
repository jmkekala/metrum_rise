shader_type spatial;
render_mode cull_back, depth_draw_opaque;

// Street lights (`ROAD-59`), one merged surface per model built by street_lights.gd. COLOR.rgb is
// the part's sRGB base colour, or for the lamp lens its emission colour; COLOR.a is the part
// (0 pole, 0.5 housing, 1 lamp lens). UV holds (roughness, metallic).

global uniform vec2 scene_window_clock;

// Lens luminance at night, in the scene's emission units.
uniform float lamp_energy = 4.0;
// Sun elevations in degrees over which the lamps come on as it sets.
uniform vec2 lamps_on_elevation_deg = vec2(4.0, -2.0);
uniform float specular = 0.5;
#include "res://scripts/shaders/standard_light.gdshaderinc"

uniform vec3 lens_albedo : source_color = vec3(0.78, 0.78, 0.76);

vec3 srgb_to_linear(vec3 c) {
	return mix(c / 12.92, pow((c + 0.055) / 1.055, vec3(2.4)), step(vec3(0.04045), c));
}

#define SCENE_WINDOW_CLOCK_DECLARED
#include "res://scripts/shaders/street_lamp_light.gdshaderinc"

void fragment() {
	float lens = step(0.75, COLOR.a);
	vec3 part = srgb_to_linear(COLOR.rgb);
	float night = smoothstep(lamps_on_elevation_deg.x, lamps_on_elevation_deg.y, scene_window_clock.y);
	ALBEDO = mix(part, lens_albedo, lens);
	ROUGHNESS = UV.x;
	METALLIC = UV.y;
	EMISSION = part * lens * night * lamp_energy;
	standard_light_cloud_sun = standard_light_cloud(VERTEX, INV_VIEW_MATRIX);
	EMISSION += ALBEDO * (1.0 - METALLIC) * street_lamp_surface(VERTEX, NORMAL, TANGENT, BINORMAL,
		NORMAL_MAP, NORMAL_MAP_DEPTH, INV_VIEW_MATRIX, CAMERA_POSITION_WORLD);
}
