# SPDX-License-Identifier: GPL-2.0-only

## Non-destructive emission inspection. Only preview surface overrides are changed;
## source materials, shared meshes and exported files retain their authored values.
extends RefCounted

const REFERENCE_COLOR := Color(1.0, 0.55, 0.23)
enum Mode { AUTHORED, OFF, ON }

var _surfaces: Array[Dictionary] = []
var _source_catalog: Array[Dictionary] = []
var _mode := -1
var _strength := -1.0

func capture(root: Node) -> void:
	if root is MeshInstance3D:
		var instance := root as MeshInstance3D
		if instance.mesh:
			for surface in instance.mesh.get_surface_count():
				var source := instance.get_active_material(surface)
				var material := source as BaseMaterial3D
				_source_catalog.append({"name": source.resource_name if source != null and not source.resource_name.is_empty() else "%s / surface %d" % [instance.name, surface],
					"type": source.get_class() if source != null else "Default material",
					"albedo_texture": material != null and material.albedo_texture != null,
					"emission_texture": material != null and material.emission_texture != null})
				if material == null or material.emission_texture == null:
					continue
				_surfaces.append({"instance": instance, "surface": surface,
					"original": instance.get_surface_override_material(surface), "source": material,
					"preview": material.duplicate()})
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
		if mode == Mode.AUTHORED:
			instance.set_surface_override_material(surface, entry["original"])
			continue
		var material: BaseMaterial3D = entry["preview"]
		material.emission_enabled = mode == Mode.ON
		# Godot material colours are sRGB; the authoring reference factor is linear RGB.
		material.emission = REFERENCE_COLOR.linear_to_srgb()
		material.emission_energy_multiplier = strength
		material.emission_operator = BaseMaterial3D.EMISSION_OP_MULTIPLY
		instance.set_surface_override_material(surface, material)

func surface_count() -> int:
	return _surfaces.size()

func source_catalog() -> Array[Dictionary]:
	return _source_catalog.duplicate(true)
