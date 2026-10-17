// SPDX-License-Identifier: GPL-2.0-only

// Bakes one tile of the cloud shape into RG and the regional placement fields into BA. Drawn once at startup by weather_cloud_shape.gd.
shader_type canvas_item;
render_mode unshaded, blend_disabled;

#include "res://scripts/shaders/weather_cloud_field.gdshaderinc"

void fragment() {
	// UV spans exactly one period, sampled at texel centres.
	vec2 position_m = UV * WEATHER_CLOUD_PERIOD_M;
	COLOR = vec4(weather_cloud_shape(position_m), weather_cloud_region_fields(position_m));
}
