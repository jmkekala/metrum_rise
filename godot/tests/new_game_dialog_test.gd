# SPDX-License-Identifier: GPL-2.0-only

## Headless contract test for the new-game vegetation dialog.
## The dialog must report exactly what Rust accepts: its defaults, bounds and readouts all come
## from `VegetationOptions`, and the dictionary it emits is what `start_new_game` consumes.
extends SceneTree

const NewGameDialog := preload("res://scripts/ui/new_game_dialog.gd")
# Never opened: the dialog only carries the selection through to whoever starts the game.
const WORLD_PATH := "/tmp/kuopio_324km2_10m.sqlite"

var _failures := 0
var _emitted: Array = []

func _initialize() -> void:
	_run()

func _run() -> void:
	var options := VegetationOptions.new()
	var defaults: Dictionary = options.get_default_config()
	var bounds: Dictionary = options.get_density_bounds()
	var default_density: float = float(defaults["canopy_stems_per_ha"])
	var min_density: float = float(bounds["min_canopy_stems_per_ha"])
	var max_density: float = float(bounds["max_canopy_stems_per_ha"])

	_expect(bool(defaults["enabled"]), "a new game must generate a forest by default")
	_expect(
		float(defaults["coverage"]) > 0.0 and float(defaults["coverage"]) < 1.0,
		"default coverage must be an interior area fraction, got %s" % defaults["coverage"]
	)
	_expect(
		min_density < default_density and default_density < max_density,
		"the shipped density must sit inside the accepted range"
	)
	# The default grid is the one the shipped world was measured on.
	_expect(
		is_equal_approx(options.get_canopy_spacing_m(default_density), 16.0),
		"the default density must read back as the shipped 16 m grid, got %f"
			% options.get_canopy_spacing_m(default_density)
	)
	# A denser forest must mean a tighter grid, since that is how density is delivered.
	_expect(
		options.get_canopy_spacing_m(max_density) < options.get_canopy_spacing_m(min_density),
		"a higher density must produce a smaller canopy cell"
	)

	# The dictionary is unpacked into `SimulationNode.start_new_game`, and GDScript cannot catch
	# a rename or an arity change there at parse time, so the boundary is checked directly.
	var entry_point := {}
	for method in ClassDB.class_get_method_list("SimulationNode", true):
		if str(method["name"]) == "start_new_game":
			entry_point = method
	_expect(not entry_point.is_empty(), "SimulationNode must expose start_new_game")
	if not entry_point.is_empty():
		var argument_names := PackedStringArray()
		for argument in entry_point["args"]:
			argument_names.append(str(argument["name"]))
		_expect(
			argument_names == PackedStringArray([
				"path",
				"vegetation_enabled",
				"vegetation_seed",
				"vegetation_coverage",
				"vegetation_canopy_stems_per_ha",
			]),
			"start_new_game must take the world and the four parameters, got %s" % [argument_names]
		)

	var dialog: Window = NewGameDialog.new()
	root.add_child(dialog)
	await process_frame
	dialog.start_requested.connect(_on_start_requested)
	dialog.popup_for_world(WORLD_PATH)
	await process_frame

	var opened: Dictionary = dialog.selected_vegetation()
	_expect(
		bool(opened["enabled"]) == bool(defaults["enabled"])
			and int(opened["seed"]) == int(defaults["seed"])
			and is_equal_approx(float(opened["canopy_stems_per_ha"]), default_density)
			and is_equal_approx(float(opened["coverage"]), float(defaults["coverage"])),
		"an untouched dialog must offer the Rust defaults, got %s" % [opened]
	)

	# The density slider must not be able to request a grid the renderer rejects.
	dialog._density_slider.value = max_density * 4.0
	dialog._coverage_slider.value = 400.0
	var clamped: Dictionary = dialog.selected_vegetation()
	_expect(
		is_equal_approx(float(clamped["canopy_stems_per_ha"]), max_density),
		"density must clamp to the renderer ceiling, got %s" % clamped["canopy_stems_per_ha"]
	)
	_expect(
		is_equal_approx(float(clamped["coverage"]), 1.0),
		"coverage must clamp to a full map, got %s" % clamped["coverage"]
	)

	# Starting empty is the same parameter set with generation turned off.
	dialog._enabled_check.button_pressed = false
	_expect(
		not bool(dialog.selected_vegetation()["enabled"]),
		"clearing the checkbox must start the map bare"
	)
	_expect(
		not dialog._density_slider.editable and not dialog._coverage_slider.editable,
		"the parameters must be inert while no forest is generated"
	)

	dialog._on_start_pressed()
	_expect(_emitted.size() == 1, "confirming must report exactly once")
	if _emitted.size() == 1:
		_expect(
			str(_emitted[0][0]) == WORLD_PATH,
			"the chosen world must travel with the parameters"
		)
		var payload: Dictionary = _emitted[0][1]
		# These keys are the `VegetationConfig` fields `start_new_game` unpacks.
		for key in ["enabled", "seed", "coverage", "canopy_stems_per_ha"]:
			_expect(payload.has(key), "the emitted parameters must carry %s" % key)
	_expect(not dialog.visible, "confirming must close the dialog")

	# Reopening must not carry the previous game's choices into the next one.
	dialog.popup_for_world(WORLD_PATH)
	await process_frame
	_expect(
		bool(dialog.selected_vegetation()["enabled"]),
		"reopening must restore the defaults"
	)

	dialog.free()
	quit(1 if _failures > 0 else 0)

func _on_start_requested(world_path: String, vegetation: Dictionary) -> void:
	_emitted.append([world_path, vegetation])

func _expect(condition: bool, message: String) -> void:
	if condition:
		return
	_failures += 1
	push_error(message)
