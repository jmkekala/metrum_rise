// SPDX-License-Identifier: GPL-2.0-only

// The sun's shadow of a tunnel's lining (`ROAD-61`): drawn only into the shadow map, on a road
// chunk's concrete layer. A lining vertex has colour alpha 0, a bridge's concrete alpha 1, so
// only the lining's walls, ceiling and headwalls cast; road chunks otherwise only receive.
shader_type spatial;
render_mode unshaded, cull_disabled;

void fragment() {
	if (COLOR.a > 0.5) {
		discard;
	}
	ALBEDO = vec3(0.0);
}
