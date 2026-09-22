# SPDX-License-Identifier: GPL-2.0-only

## Non-destructive colour and emission inspection. Only preview surface overrides are changed;
## source materials, shared meshes and exported files retain their authored values.
extends RefCounted

const SchemeMaterials := preload("res://scripts/renderers/scheme_materials.gd")

const REFERENCE_COLOR := Color(1.0, 0.55, 0.23)
enum Mode { AUTHORED, OFF, ON }

var _surfaces: Array[Dictionary] = []
var _mode := -1
var _strength := -1.0
var _scheme := ""
var _overrides: Dictionary = {}

func capture(root: Node) -> void:
	if root is MeshInstance3D:
		var instance := root as MeshInstance3D
		if instance.mesh:
			for surface in instance.mesh.get_surface_count():
				var material := instance.get_active_material(surface) as BaseMaterial3D
				if material == null:
					continue
				_surfaces.append({"instance": instance, "surface": surface,
					"original": instance.get_surface_override_material(surface), "source": material,
					"variants": {}, "preview": material.duplicate() if material.emission_texture != null else null})
	for child in root.get_children():
		capture(child)

func apply(mode: int, strength: float) -> void:
	if _mode == mode and _strength == strength:
		return
	_mode = mode
	_strength = strength
	for entry in _surfaces:
		var instance: MeshInstance3D = entry["instance"]
		if not is_instance_valid(instance):
			continue
		var surface: int = entry["surface"]
		var source: BaseMaterial3D = entry["source"]
		var base: BaseMaterial3D = source
		if entry["variants"].has(_scheme):
			base = entry["variants"][_scheme]["base"]
		if mode == Mode.AUTHORED or base.emission_texture == null:
			if base != source:
				instance.set_surface_override_material(surface, base)
				continue
			instance.set_surface_override_material(surface, entry["original"])
			continue
		var material: BaseMaterial3D = entry["preview"]
		if entry["variants"].has(_scheme):
			material = entry["variants"][_scheme]["preview"]
		material.emission_enabled = mode == Mode.ON
		# Godot material colours are sRGB; the authoring reference factor is linear RGB.
		material.emission = REFERENCE_COLOR.linear_to_srgb()
		material.emission_energy_multiplier = strength
		material.emission_operator = BaseMaterial3D.EMISSION_OP_MULTIPLY
		instance.set_surface_override_material(surface, material)

func surface_count() -> int:
	var count := 0
	for entry in _surfaces:
		if entry["source"].emission_texture != null:
			count += 1
	return count

## Bind already-resolved source material names; retain mesh resources and cached variants.
func set_scheme(key: String, overrides: Dictionary) -> void:
	if _scheme == key and _overrides == overrides:
		return
	_scheme = key
	_overrides = overrides
	for entry in _surfaces:
		var source: BaseMaterial3D = entry["source"]
		if not overrides.has(source.resource_name) or entry["variants"].has(key):
			continue
		var material := SchemeMaterials.variant(source, overrides[source.resource_name])
		entry["variants"][key] = {"base": material, "preview": material.duplicate() if material.emission_texture != null else null}
	var previous_mode := _mode
	_mode = -1
	apply(maxi(previous_mode, Mode.AUTHORED), maxf(_strength, 0.0))

## Document edits discard obsolete variants; ordinary scheme selection retains them.
func clear_schemes() -> void:
	for entry in _surfaces:
		entry["variants"].clear()
	_scheme = ""
	_overrides = {}
	var previous_mode := _mode
	_mode = -1
	apply(maxi(previous_mode, Mode.AUTHORED), maxf(_strength, 0.0))
