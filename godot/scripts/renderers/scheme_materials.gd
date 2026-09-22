# SPDX-License-Identifier: GPL-2.0-only

## Shared colour-scheme material construction. The asset editor preview and the gameplay
## building renderer bind the same channels through this one implementation, so a scheme
## cannot look different between them. Source materials and imported meshes are untouched.
extends RefCounted

## Duplicate `source` with a scheme's replacement textures bound. `textures` maps the channel
## names Rust publishes — `albedo`, `orm`, `normal`, `emission` — to loaded textures; an
## absent channel keeps the source value.
static func variant(source: BaseMaterial3D, textures: Dictionary) -> BaseMaterial3D:
	var material: BaseMaterial3D = source.duplicate()
	if textures.has("albedo"):
		material.albedo_texture = textures["albedo"]
	if textures.has("normal"):
		material.normal_enabled = true
		material.normal_texture = textures["normal"]
	if textures.has("emission"):
		material.emission_texture = textures["emission"]
	if textures.has("orm"):
		if material is ORMMaterial3D:
			material.orm_texture = textures["orm"]
		else:
			# StandardMaterial3D has no packed slot, so the three channels bind separately.
			material.ao_enabled = true
			material.ao_texture = textures["orm"]
			material.ao_texture_channel = BaseMaterial3D.TEXTURE_CHANNEL_RED
			material.roughness_texture = textures["orm"]
			material.roughness_texture_channel = BaseMaterial3D.TEXTURE_CHANNEL_GREEN
			material.metallic_texture = textures["orm"]
			material.metallic_texture_channel = BaseMaterial3D.TEXTURE_CHANNEL_BLUE
	return material
