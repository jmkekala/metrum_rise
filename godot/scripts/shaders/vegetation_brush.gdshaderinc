// SPDX-License-Identifier: GPL-3.0-or-later

global uniform bool vegetation_brush_enabled;
// World XZ centre and radius in metres.
global uniform vec3 vegetation_brush_footprint;
global uniform vec4 vegetation_brush_color : source_color;

void apply_vegetation_brush(vec2 world_xz, inout vec3 albedo, inout vec3 emission) {
	float radius = vegetation_brush_footprint.z;
	float edge_distance = distance(world_xz, vegetation_brush_footprint.xy) - radius;
	float pixel_m = max(fwidth(edge_distance), 0.0001);
	float aa = 0.5 * pixel_m;
	// Inset the stroke so the outside boundary marks the actual edit radius. Leave room
	// for fill when a point edit or a hedge end projects to just a few pixels.
	float stroke = min(2.5 * pixel_m, 0.4 * radius);
	float disc = 1.0 - smoothstep(-aa, aa, edge_distance);
	float edge = smoothstep(-stroke - aa, -stroke + aa, edge_distance);
	float opacity = disc * mix(0.24, 0.85, edge);
	vec3 color = vegetation_brush_color.rgb;
	albedo = mix(albedo, color, opacity);
	emission = mix(emission, color * 0.6, opacity);
}
