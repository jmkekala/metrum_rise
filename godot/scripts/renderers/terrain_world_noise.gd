# SPDX-License-Identifier: GPL-2.0-only

## Bakes the terrain's position-only noise fields once per world and publishes them as the
## `grass_world_noise` / `grass_world_noise_bounds` shader globals.
##
## The fields (macro variation, land-variation broad/mid, meadow broad) depend on world XZ only,
## never on terrain edits, so one GPU pass at world load replaces 13 noise evaluations in every
## terrain pixel of every frame. The bake runs the shared shader include's own functions, so the
## baked texel centres equal the old per-pixel values; between them the fields are bilinear, which
## their 167 m-or-coarser lattices make visually indistinguishable at TEXEL_M.
extends Node

const BAKE_SHADER := preload("res://scripts/shaders/terrain_world_noise_bake.gdshader")
const TEXEL_M := 16.0
const MAX_TEXELS := 4096

var _viewport: SubViewport
var _rect: ColorRect

func bake(world_size: Vector2) -> void:
	if world_size.x <= 0.0 or world_size.y <= 0.0:
		return
	if _viewport == null:
		_create_viewport()
	var texels := Vector2i(
		clampi(ceili(world_size.x / TEXEL_M), 1, MAX_TEXELS),
		clampi(ceili(world_size.y / TEXEL_M), 1, MAX_TEXELS)
	)
	# Terrain world coordinates are centred on the origin.
	var bounds := Vector4(-world_size.x * 0.5, -world_size.y * 0.5, world_size.x, world_size.y)
	_viewport.size = texels
	_rect.size = Vector2(texels)
	(_rect.material as ShaderMaterial).set_shader_parameter("world_bounds", bounds)
	_viewport.render_target_update_mode = SubViewport.UPDATE_ONCE
	RenderingServer.global_shader_parameter_set("grass_world_noise", _viewport.get_texture())
	RenderingServer.global_shader_parameter_set("grass_world_noise_bounds", bounds)

func _create_viewport() -> void:
	_viewport = SubViewport.new()
	_viewport.name = "WorldNoiseBake"
	_viewport.disable_3d = true
	_viewport.transparent_bg = true
	_viewport.render_target_clear_mode = SubViewport.CLEAR_MODE_ALWAYS
	_rect = ColorRect.new()
	var material := ShaderMaterial.new()
	material.shader = BAKE_SHADER
	_rect.material = material
	_viewport.add_child(_rect)
	add_child(_viewport)
