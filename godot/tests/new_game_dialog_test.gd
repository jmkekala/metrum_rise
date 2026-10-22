# SPDX-License-Identifier: GPL-2.0-only

## Headless contract test for the new-game vegetation dialog.
## The dialog must report exactly what Rust accepts: its defaults, presets and readouts all come
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
	var presets: Dictionary = options.get_presets()
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

	_expect(
		dialog._forest_buttons.size() == 2 and dialog._forest_buttons[0].button_pressed,
		"the dialog must offer Less and More forest, with Less pressed"
	)
	var opened: Dictionary = dialog.selected_vegetation()
	var less := {
		"enabled": true,
		"seed": int(defaults["seed"]),
		"coverage": float(presets["less"]["coverage"]),
		"canopy_stems_per_ha": float(presets["less"]["canopy_stems_per_ha"]),
	}
	_expect(opened == less, "an untouched dialog must offer the Less preset, got %s" % [opened])
	for key in ["less", "more"]:
		var density := float(presets[key]["canopy_stems_per_ha"])
		_expect(
			density >= min_density and density <= max_density,
			"the %s forest's density must be accepted by Rust" % key
		)
	# More is both wider and denser, up to the renderer ceiling.
	_expect(
		float(presets["more"]["coverage"]) > float(presets["less"]["coverage"])
			and float(presets["more"]["canopy_stems_per_ha"])
				> float(presets["less"]["canopy_stems_per_ha"])
			and is_equal_approx(float(presets["more"]["canopy_stems_per_ha"]), max_density),
		"More must cover more ground than Less, at the renderer's density ceiling"
	)

	dialog._forest_buttons[1].button_pressed = true
	dialog._on_start_pressed()
	_expect(_emitted.size() == 1, "confirming must report exactly once")
	if _emitted.size() == 1:
		_expect(
			str(_emitted[0][0]) == WORLD_PATH,
			"the chosen world must travel with the parameters"
		)
		var payload: Dictionary = _emitted[0][1]
		_expect(
			bool(payload["enabled"]) and int(payload["seed"]) == int(defaults["seed"])
				and float(payload["coverage"]) == float(presets["more"]["coverage"])
				and float(payload["canopy_stems_per_ha"])
					== float(presets["more"]["canopy_stems_per_ha"]),
			"More must emit the Rust preset with the default seed"
		)
		# These keys are the `VegetationConfig` fields `start_new_game` unpacks.
		for key in ["enabled", "seed", "coverage", "canopy_stems_per_ha"]:
			_expect(payload.has(key), "the emitted parameters must carry %s" % key)
	_expect(not dialog.visible, "confirming must close the dialog")

	# Reopening must not carry the previous game's choices into the next one.
	dialog.popup_for_world(WORLD_PATH)
	await process_frame
	_expect(
		dialog._forest_buttons[0].button_pressed and dialog.selected_vegetation() == less,
		"reopening must restore Less"
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
