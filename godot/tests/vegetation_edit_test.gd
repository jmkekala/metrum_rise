# SPDX-License-Identifier: GPL-2.0-only

## Headless native vegetation edit, persistence, tool routing and preview contract.
## Uses explicit failures because headless GDScript assertions do not terminate a test.
extends SceneTree

const VegetationTool := preload("res://scripts/tools/vegetation_tool.gd")
const InputManager := preload("res://scripts/core/input_manager.gd")
const MainUI := preload("res://scripts/ui/main_ui.gd")
const Vegetation := preload("res://scripts/renderers/vegetation.gd")
const TreeSpecies := preload("res://scripts/renderers/tree_species.gd")
var _failures := 0

func _initialize() -> void:
	call_deferred("_run")

# One ctrl-held wheel notch, which is the tool's radius control.
func _wheel(button: int) -> InputEventMouseButton:
	var event := InputEventMouseButton.new()
	event.button_index = button
	event.pressed = true
	event.ctrl_pressed = true
	return event

# A bare left press, which is what arms a stroke.
func _click() -> InputEventMouseButton:
	var event := InputEventMouseButton.new()
	event.button_index = MOUSE_BUTTON_LEFT
	event.pressed = true
	return event

func _expect(condition: bool, message: String) -> void:
	if not condition:
		_failures += 1
		push_error(message)

func _run() -> void:
	var host := Node3D.new()
	root.add_child(host)
	var simulation := SimulationNode.new()
	simulation.name = "SimulationNode"
	host.add_child(simulation)
	simulation.set_simulation_speed(0.0)
	_expect(simulation.create_blank_world(1020.0, 1020.0, 10.0, 510.0, 20.0), "fixture world must be created")
	var layout: Dictionary = simulation.get_terrain_patch_layout()
	var span: float = float(layout["terrain_cell_m"]) * float(layout["patch_interval_cells"])
	_expect(span == 510.0, "native patch layout must match the fixture")
	var left := Vector2(-510.0, 0.0)
	var right := Vector2(0.0, 0.0)
	var before_left := simulation.get_decorative_tree_patch(left, span, true)
	var before_right := simulation.get_decorative_tree_patch(right, span, true)
	# Captured before any edit, and canopy-only, so every record is known to be a generated
	# candidate of a known layer. Reading a position back out of an edited patch cannot tell the
	# two apart: an authored plant is packed into the same payload, in the same cell order.
	var generated := simulation.get_decorative_tree_patch(right, span, false)
	var target := Vector2(0.0, 17.0)
	_expect(simulation.add_vegetation_at(target, 3), "clear ground must accept a plant")
	_expect(simulation.get_vegetation_patch_generation(Vector2i(1, 1)) == 1, "only the containing render patch advances")
	_expect(simulation.get_vegetation_patch_generation(Vector2i(0, 1)) == 0, "the adjacent patch stays unchanged")
	_expect(simulation.get_decorative_tree_patch(left, span, true) == before_left, "boundary addition must not leak into the left patch")
	var authored := simulation.get_decorative_tree_patch(right, span, true)
	_expect(authored.size() == before_right.size() + 6, "boundary addition must appear exactly once")
	_expect(not simulation.add_vegetation_at(Vector2(510.0, 0.0), 0), "world edge must refuse placement")
	_expect(not simulation.add_vegetation_at(target, 99), "unknown preset must be refused")
	_expect(simulation.paint_vegetation(target, 32.0, -1, 0) == 0, "brush must refuse unknown preset")
	var path := OS.get_temp_dir().path_join("metrum_vegetation_edit_%d.sqlite" % OS.get_process_id())
	_expect(simulation.save_game(path), "authored vegetation must save")
	_expect(simulation.remove_vegetation_at(target, 0.01, 0) == 1, "authored plant removal must report one change")
	_expect(simulation.remove_vegetation_at(target, 0.01, 0) == 0, "repeated removal must report no change")
	_expect(simulation.load_game(path), "authored vegetation must load")
	_expect(simulation.get_decorative_tree_patch(right, span, true) == authored, "load must reproduce every packed plant bit")
	_expect(generated.size() >= 12, "fixture must contain generated canopy")
	if generated.size() >= 12:
		var first := Vector2(generated[0], generated[2])
		_expect(simulation.remove_vegetation_at(first, 0.01, 0) == 1, "a generated plant must be addressable")
		var second := Vector2(generated[6], generated[8])
		_expect(simulation.remove_vegetation_at(second, 0.01, 0) == 1, "canopy removal must tombstone exactly one tree")
		_expect(simulation.paint_vegetation(second, 0.01, 0, 0) == 1, "repaint must restore that generated tree")
		# This tiny disc covers only the generator candidate: its standing tree must not
		# acquire a duplicate when repainted.
		_expect(simulation.paint_vegetation(second, 0.01, 0, 0) == 0, "a standing tree must not be repainted")
	_expect(simulation.paint_vegetation(Vector2(-80.0, -80.0), 64.0, 1, 0) > 750, "brush must fill its dense lattice across existing forest")
	_expect(simulation.paint_vegetation(Vector2(-80.0, -80.0), 64.0, 1, 0) == 0, "repeated brush must not stack plants")

	var tool := VegetationTool.new()
	tool.name = "VegetationTool"
	host.add_child(tool)
	tool.set_process(false)
	# Attach the script after entering the tree to skip unrelated gameplay construction in
	# _ready(), while still giving the real keyboard dispatcher a viewport.
	var manager := Node.new()
	host.add_child(manager)
	manager.set_script(InputManager)
	manager.vegetation_tool = tool
	manager.current_tool = InputManager.Tool.VEGETATION
	manager._activate_tool_logic(InputManager.Tool.VEGETATION, true)
	_expect(tool.active, "input routing must activate the vegetation tool")
	# The radius chooses the point mutator or the brush inside one plant mode, so both dispatches
	# are exercised without a third mode to select them.
	tool.mode = VegetationTool.Mode.PLANT
	tool.option_index = 2
	_expect(tool.preset == VegetationTool.BRUSH_OPTIONS[2].preset, "the option must resolve its native preset ordinal")
	tool.radius = VegetationTool.MIN_RADIUS_M
	_expect(tool.apply_at(Vector2(11.0, 13.0)) == 1, "the minimum radius must call the native point mutator")
	tool.mode = VegetationTool.Mode.REMOVE
	tool.radius = 0.01
	_expect(tool.apply_at(Vector2(11.0, 13.0)) == 1, "remove mode must forward its radius")
	tool.mode = VegetationTool.Mode.PLANT
	tool.radius = 32.0
	_expect(tool.apply_at(Vector2(96.0, -96.0)) > 0, "a raised radius must call the native planting brush")
	# Ctrl and the wheel are the only radius control, and stepping back down has to land on the
	# minimum exactly, because that value is what selects the single-plant dispatch above.
	tool.radius = VegetationTool.MIN_RADIUS_M
	tool._unhandled_input(_wheel(MOUSE_BUTTON_WHEEL_UP))
	_expect(tool.radius > VegetationTool.MIN_RADIUS_M, "ctrl and wheel up must widen the brush")
	for _i in range(8):
		tool._unhandled_input(_wheel(MOUSE_BUTTON_WHEEL_DOWN))
	_expect(tool.radius == VegetationTool.MIN_RADIUS_M, "wheel down must clamp back to the point radius")
	tool.radius = 1024.0
	_expect(tool.radius == 256.0, "plant radius must match the native stamp bound")
	tool.mode = VegetationTool.Mode.REMOVE
	tool.radius = 1024.0
	_expect(tool.radius == 1024.0, "removal keeps its existing radius bound")
	tool.mode = VegetationTool.Mode.PLANT
	_expect(tool.radius == 256.0, "switching to planting must clamp the preview radius")
	_expect(simulation.paint_vegetation(Vector2.ZERO, 256.01, 0, 0) == 0, "native brush must reject oversized stamps")
	# An open species dropdown is an embedded subwindow holding the input grab, and the click
	# that dismisses it also reaches the tool. That click must dismiss and nothing else, or
	# picking a species costs the player a tree wherever the cursor happened to rest.
	tool.mode = VegetationTool.Mode.PLANT
	tool.radius = 32.0
	var menu := PopupMenu.new()
	host.add_child(menu)
	menu.popup()
	_expect(not root.get_embedded_subwindows().is_empty(), "fixture popup must embed in the viewport")
	tool._process(0.0)
	tool._unhandled_input(_click())
	_expect(not tool._painting, "the click that dismisses a popup must not arm a stroke")
	menu.hide()
	menu.free()
	# The latch decays on its own, so the very next deliberate click still paints.
	for _i in range(VegetationTool.MENU_DISMISS_FRAMES + 1):
		tool._process(0.0)
	tool._unhandled_input(_click())
	_expect(tool._painting, "a click with no popup open must still arm a stroke")
	tool._unhandled_input(_click())
	# The dropdown forwards its own wheel events here, because the grab hides them from the tool.
	tool.radius = VegetationTool.MIN_RADIUS_M
	tool.step_radius(1)
	_expect(tool.radius > VegetationTool.MIN_RADIUS_M, "the forwarded wheel notch must widen the brush")
	tool.step_radius(-1)
	_expect(tool.radius == VegetationTool.MIN_RADIUS_M, "the forwarded notch must step back onto the point radius")
	tool._painting = false

	# Build only the vegetation controls, using the real dropdown and popup connections.
	var ui := MainUI.new()
	ui.input_manager = manager
	ui.terrain_sub_menu = HBoxContainer.new()
	host.add_child(ui.terrain_sub_menu)
	ui._build_vegetation_controls()
	var dropdown := ui.terrain_sub_menu.get_child(1) as OptionButton
	var remove_button := ui.terrain_sub_menu.get_child(2) as Button
	_expect(dropdown.item_count == VegetationTool.BRUSH_OPTIONS.size(), "dropdown must use the tool's option list")
	tool.option_index = 0
	var cycle := _wheel(MOUSE_BUTTON_WHEEL_DOWN)
	cycle.ctrl_pressed = false
	cycle.shift_pressed = true
	tool._unhandled_input(cycle)
	_expect(tool.option_index == VegetationTool.BRUSH_OPTIONS.size() - 1, "cycling backward must wrap")
	_expect(dropdown.selected == tool.option_index, "cycling must update the dropdown")
	cycle.button_index = MOUSE_BUTTON_WHEEL_UP
	tool._unhandled_input(cycle)
	_expect(tool.option_index == 0 and dropdown.selected == 0, "cycling forward must wrap")
	dropdown.select(1)
	dropdown.item_selected.emit(1)
	_expect(tool.option_index == 1 and tool.preset == VegetationTool.BRUSH_OPTIONS[1].preset, "dropdown must update the cycle position and native preset")
	dropdown.get_popup().window_input.emit(cycle)
	_expect(tool.option_index == 2 and dropdown.selected == 2, "popup wheel must continue from the dropdown's selection")
	cycle.ctrl_pressed = true
	dropdown.get_popup().window_input.emit(cycle)
	_expect(tool.radius > VegetationTool.MIN_RADIUS_M and tool.option_index == 2, "Ctrl must retain radius priority over Shift in the popup")
	var erase := InputEventKey.new()
	erase.keycode = KEY_E
	erase.pressed = true
	manager._unhandled_input(erase)
	_expect(tool.mode == VegetationTool.Mode.REMOVE and remove_button.button_pressed, "E must enable erase and synchronize Remove")
	_expect(tool._option_label.text == "Remove" and tool._option_label.modulate == VegetationTool.UIStyle.TEXT_ALERT, "erase label must match the destructive mode")
	# If programmatic selection re-fired item_selected, its handler would leave erase mode.
	tool.step_option(1)
	_expect(tool.mode == VegetationTool.Mode.REMOVE and dropdown.selected == tool.option_index, "cycle synchronization must not fire the dropdown handler")
	manager._unhandled_input(erase)
	_expect(tool.mode == VegetationTool.Mode.PLANT and not remove_button.button_pressed, "E must restore planting and synchronize Remove")
	_expect(tool._option_label.text == VegetationTool.BRUSH_OPTIONS[tool.option_index].label, "plant label must show the active option")
	# Neither depth-tests, so only the draw order keeps the text off the ring it names.
	_expect(tool._option_label.render_priority > tool._ring_material.render_priority
		and tool._option_label.outline_render_priority > tool._ring_material.render_priority,
		"the label and its outline must sort above the ground ring")
	remove_button.button_pressed = true
	_expect(tool.mode == VegetationTool.Mode.REMOVE, "Remove must update the same mode as E")
	remove_button.button_pressed = false
	_expect(tool.mode == VegetationTool.Mode.PLANT, "Remove must toggle back to planting")
	ui.terrain_sub_menu.free()
	ui.free()

	manager._activate_tool_logic(InputManager.Tool.VEGETATION, false)
	tool._process(0.0)
	_expect(not tool.active and not tool.preview.visible, "deactivation must hide the ground preview")
	_expect(tool.preview.mesh is TorusMesh, "tool must provide a ground-ring preview")
	# The preset table in vegetation_api/brush.rs names which meshes are pine and which are
	# spruce; tree_species.gd owns that rule for the meshes themselves. Nothing but this check
	# links the two, so a change on either side that the other does not follow fails here.
	# Only these strokes pin a variant, so every pinned record in the patch came from one.
	for named in [[4, true], [5, false]]:
		var origin := Vector2(-510.0, -510.0) if named[0] == 4 else Vector2(0.0, -510.0)
		var tree: String = "pine" if named[1] else "spruce"
		# Keyed on position and diffed, because earlier sections of this test plant their own
		# trees in these patches and only this stroke's own plants prove anything here.
		var before := {}
		var prior := simulation.get_decorative_tree_patch(origin, span, true)
		for i in range(0, prior.size(), 6):
			before[Vector2(prior[i], prior[i + 2])] = true
		_expect(simulation.paint_vegetation(origin + Vector2(255.0, 255.0), 64.0, named[0], 0) > 0,
			"a %s stroke must plant something" % tree)
		var records := simulation.get_decorative_tree_patch(origin, span, true)
		var pinned := 0
		for i in range(0, records.size(), 6):
			if before.has(Vector2(records[i], records[i + 2])):
				continue
			var packed := int(records[i + 5])
			var pin: int = packed >> Vegetation.SPECIES_BITS
			_expect(pin > 0, "a %s stroke must pin every mesh it plants" % tree)
			if pin == 0:
				continue
			pinned += 1
			_expect(packed & Vegetation.SPECIES_MASK == TreeSpecies.CONIFER,
				"a named %s must pack as a conifer" % tree)
			_expect(TreeSpecies._is_pine(pin - 1) == named[1],
				"a %s stroke planted a mesh that is not a %s" % [tree, tree])
		_expect(pinned > 0, "a %s stroke must pin the meshes it planted" % tree)

	manager.free()
	var scene := load("res://scenes/Main.tscn") as PackedScene
	_expect(scene != null, "main scene must load with the vegetation tool")
	if scene != null:
		var state := scene.get_state()
		var found := false
		for i in range(state.get_node_count()):
			if state.get_node_name(i) == &"VegetationTool":
				found = state.get_node_type(i) == &"Node3D"
		_expect(found, "main scene must own a VegetationTool Node3D")
	host.free()
	DirAccess.remove_absolute(path)
	if _failures == 0:
		print("vegetation_edit_test: PASS")
	quit(1 if _failures > 0 else 0)
