shader_type spatial;
render_mode blend_mix, cull_disabled, depth_draw_opaque;

// Road paint. Lane lines carry their metres along the road in UV.x and a dash pattern in UV.y
// (markings.rs): 2 solid, 3 Finnish 1 m dash / 3 m gap, 4 Finnish 3 m dash / 9 m gap.
// Zebra crossings are part of road_asphalt.gdshader; this material remains for lane lines.

uniform float specular = 0.5;
#include "res://scripts/shaders/standard_light.gdshaderinc"

uniform float opacity : hint_range(0.0, 1.0) = 0.8;

#include "res://scripts/shaders/street_lamp_light.gdshaderinc"

void fragment() {
	float pattern = round(UV.y);
	if (UV.y > 1.5 && pattern > 2.5) {
		float dash_m = pattern > 3.5 ? 3.0 : 1.0;
		if (mod(UV.x, dash_m * 4.0) > dash_m) {
			discard;
		}
	}
	ALBEDO = COLOR.rgb;
	ALPHA = COLOR.a * opacity;
	ROUGHNESS = 1.0;
	standard_light_cloud_sun = standard_light_cloud(VERTEX, INV_VIEW_MATRIX);
	EMISSION += ALBEDO * (1.0 - METALLIC) * street_lamp_surface(VERTEX, NORMAL, TANGENT, BINORMAL,
		NORMAL_MAP, NORMAL_MAP_DEPTH, INV_VIEW_MATRIX, CAMERA_POSITION_WORLD);
}
