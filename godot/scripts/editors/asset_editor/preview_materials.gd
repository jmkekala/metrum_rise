# SPDX-License-Identifier: GPL-2.0-only

## Non-destructive colour and emission inspection. Only preview surface overrides are changed;
## source materials, shared meshes and exported files retain their authored values.
extends RefCounted

const WindowMaterials := preload("res://scripts/renderers/window_materials.gd")
const SchemeMaterials := preload("res://scripts/renderers/scheme_materials.gd")

const REFERENCE_COLOR := WindowMaterials.REFERENCE_COLOR
enum Mode { AUTHORED, OFF, ON }

var _surfaces: Array[Dictionary] = []
var _mode := -2
var _clock := Vector2(12, 45)
var _schedule := Vector4(4, 24.5, 6, 1)
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
					"variants": {}, "automatic": WindowMaterials.create(material, true) if material.emission_texture != null else null,
					"preview": material.duplicate() if material.emission_texture != null else null})
	for child in root.get_children():
		capture(child)

func apply(mode: int, strength: float, clock: Vector2 = Vector2(12, 45), schedule: Vector4 = Vector4(4, 24.5, 6, 1)) -> void:
	if _mode == mode and _strength == strength and _clock == clock and _schedule == schedule:
		return
	_mode = mode
	_strength = strength
	_clock = clock
	_schedule = schedule
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
		if mode == -1:
			var automatic: ShaderMaterial = entry["variants"][_scheme]["automatic"] if entry["variants"].has(_scheme) else entry["automatic"]
			automatic.set_shader_parameter("window_preview_clock", clock)
			automatic.set_shader_parameter("window_preview_schedule", schedule)
			automatic.set_shader_parameter("window_strength", strength)
			instance.set_surface_override_material(surface, automatic)
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
		entry["variants"][key] = {"base": material, "automatic": WindowMaterials.create(material, true) if material.emission_texture != null else null, "preview": material.duplicate() if material.emission_texture != null else null}
	var previous_mode := _mode
	_mode = -2
	apply(previous_mode if previous_mode >= -1 else Mode.AUTHORED, maxf(_strength, 0.0), _clock, _schedule)

## Document edits discard obsolete variants; ordinary scheme selection retains them.
func clear_schemes() -> void:
	for entry in _surfaces:
		entry["variants"].clear()
	_scheme = ""
	_overrides = {}
	var previous_mode := _mode
	_mode = -2
	apply(previous_mode if previous_mode >= -1 else Mode.AUTHORED, maxf(_strength, 0.0), _clock, _schedule)
