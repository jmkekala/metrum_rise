// SPDX-License-Identifier: GPL-2.0-only

// Rendering-only, exact cell/parcel rectangles on the receiving ground. Coarse texels list
// conservative candidates; coverage and antialiased edges come from world position, not depth.
#ifndef ZONING_GROUND_INCLUDED
#define ZONING_GROUND_INCLUDED
global uniform bool zoning_mask_enabled;
global uniform vec4 zoning_mask_bounds;
global uniform sampler2DArray zoning_masks : filter_nearest, repeat_disable;
global uniform sampler2DArray zoning_rows : filter_nearest, repeat_disable;

vec4 zoning_row(int index, int layer) {
	return texelFetch(zoning_rows, ivec3(index & 255, index >> 8, layer), 0);
}

vec4 zoning_rectangle(int row, int layer, vec2 point, float pixel_m, float distance_m) {
	vec4 frame = zoning_row(row, layer);
	vec4 extent = zoning_row(row + 1, layer);
	vec2 delta = point - frame.xy;
	vec2 local = vec2(dot(delta, frame.zw), dot(delta, vec2(-frame.w, frame.z)) * extent.w);
	vec2 border = min(local, extent.xy - local);
	if (min(border.x, border.y) < 0.0) {
		return vec4(0.0);
	}
	float edge_m = min(border.x, border.y);
	float aa = max(pixel_m, 0.0001);
	float edge = (1.0 - smoothstep(0.12 - aa, 0.12 + aa, edge_m))
		* min(1.0, 0.24 / aa) * (1.0 - smoothstep(400.0, 1800.0, distance_m));
	// Style: occupied=0 (quieter fill keeps the zone readable), unpainted=0.5, painted=1, preview=2.
	float fill = extent.z > 1.5 ? 0.50 : (extent.z > 0.75 ? 0.30 : (extent.z > 0.25 ? 0.015 : 0.15));
	vec3 color = zoning_row(row + 2, layer).rgb;
	color = mix(color, min(color * 1.4 + vec3(0.12), vec3(1.0)), edge);
	return vec4(color, mix(fill, extent.z < 0.25 ? 0.35 : 0.75, edge));
}

vec4 zoning_layer(int layer, vec2 point, float pixel_m, float distance_m) {
	if (layer == 0) { return vec4(0.0); }
	uvec4 bytes = uvec4(round(texelFetch(zoning_masks, ivec3(ivec2(floor(point * 0.5)), layer), 0) * 255.0));
	uint index = bytes.x | (bytes.y << 8u) | (bytes.z << 16u) | (bytes.w << 24u);
	if (index == 0u) { return vec4(0.0); }
	int row = int(index >> 1u);
	if ((index & 1u) != 0u) {
		return zoning_rectangle(row, layer, point, pixel_m, distance_m);
	}
	int count = int(zoning_row(row, layer).x);
	vec4 candidates = vec4(0.0);
	for (int i = 0; i < count; i++) {
		if ((i & 3) == 0) { candidates = zoning_row(row + 1 + (i >> 2), layer); }
		vec4 result = zoning_rectangle(int(candidates[i & 3]), layer, point, pixel_m, distance_m);
		if (result.a > 0.0) { return result; }
	}
	return vec4(0.0);
}

vec4 zoning_ground(vec3 world, vec3 eye) {
	// Coherent global branch: hidden zoning performs no texture reads or derivatives.
	if (!zoning_mask_enabled) { return vec4(0.0); }
	vec2 chunk = floor(world.xz / 512.0);
	ivec2 address = ivec2(chunk - zoning_mask_bounds.xy);
	if (any(lessThan(address, ivec2(0))) || any(greaterThanEqual(address, ivec2(zoning_mask_bounds.zw)))) {
		return vec4(0.0);
	}
	int index = address.y * int(zoning_mask_bounds.z) + address.x;
	ivec2 layers = ivec2(zoning_row(index, 0).rg);
	if (layers == ivec2(0)) { return vec4(0.0); }
	vec2 point = world.xz - chunk * 512.0;
	// Derivatives of world coordinates, never of discontinuous cell-local coordinates.
	float pixel_m = max(length(dFdx(world.xz)), length(dFdy(world.xz)));
	float distance_m = distance(world, eye);
	vec4 preview = zoning_layer(layers.y, point, pixel_m, distance_m);
	if (preview.a > 0.0) { return preview; }
	return zoning_layer(layers.x, point, pixel_m, distance_m);
}

void apply_zoning_ground(vec3 world, vec3 eye, inout vec3 albedo, inout vec3 emission) {
	vec4 zone = zoning_ground(world, eye);
	albedo = mix(albedo, zone.rgb, zone.a);
	// Map annotation remains readable at night; physical objects still occlude it normally.
	emission = mix(emission, zone.rgb * 0.6, zone.a);
}
#endif
