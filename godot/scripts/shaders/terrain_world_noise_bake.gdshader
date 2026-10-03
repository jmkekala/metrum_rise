// Bakes the position-only noise fields of ground_grass.gdshaderinc into one world texture.
// Each texel evaluates the same *_live functions the terrain shader used per pixel, at the texel's
// world-space centre; terrain.gdshader then reads them with one bilinear fetch.
shader_type canvas_item;
render_mode unshaded, blend_disabled;

#include "res://scripts/shaders/ground_grass.gdshaderinc"

// World min XZ in .xy, world size in .zw; matches the grass_world_noise_bounds global.
uniform vec4 world_bounds;

void fragment() {
	vec2 world_xz = world_bounds.xy + UV * world_bounds.zw;
	COLOR = vec4(
		macro_variation_live(world_xz),
		land_variation_broad_live(world_xz),
		land_variation_mid_live(world_xz),
		meadow_broad_live(world_xz)
	);
}
