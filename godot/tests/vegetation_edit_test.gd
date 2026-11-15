# SPDX-License-Identifier: GPL-2.0-only

## Headless native vegetation edit, persistence, tool routing and preview contract.
## Uses explicit failures because headless GDScript assertions do not terminate a test.
extends SceneTree

const VegetationTool := preload("res://scripts/tools/vegetation_tool.gd")
const InputManager := preload("res://scripts/core/input_manager.gd")
const TerrainPanel := preload("res://scripts/ui/terrain_panel.gd")
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
	# One stamp adds 10 stems/ha of its 1.29 ha disc. Repeats thicken the stand to the preset's
	# density and then add nothing, so held stamps never stack plants.
	_expect(simulation.paint_vegetation(Vector2(-80.0, -80.0), 64.0, 1, 0) == 13, "one stamp must add a thin share")
	var stand := 13
	for i in range(200):
		var added := simulation.paint_vegetation(Vector2(-80.0, -80.0), 64.0, 1, 0)
		stand += added
		if added == 0:
			break
	_expect(stand > 400, "repeated stamps must add a spaced stand across existing forest")
	_expect(simulation.paint_vegetation(Vector2(-80.0, -80.0), 64.0, 1, 0) == 0, "a full stand must not stack plants")

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
	for index in range(VegetationTool.BRUSH_OPTIONS.size()):
		if VegetationTool.BRUSH_OPTIONS[index].get("ground", false):
			tool.option_index = index
			break
	_expect(tool.radius == 64.0, "bush selection must clamp to the native ground-cover budget")
	_expect(simulation.paint_vegetation(Vector2.ZERO, 64.01, 2, 0) == 0, "native ground-cover cap must reject oversized stamps")
	# A hedge option lays a row between two ends rather than stamping, and a redrawn row stacks nothing.
	for index in range(VegetationTool.BRUSH_OPTIONS.size()):
		if VegetationTool.BRUSH_OPTIONS[index].get("line", false):
			tool.option_index = index
			break
	_expect(tool.is_line_option(), "a hedge option must lay rows")
	var row := tool.apply_line(Vector2(-150.0, 150.0), Vector2(-138.0, 150.0), 41)
	_expect(row == 12, "a 12 m hedge must lay twelve modules, got %d" % row)
	_expect(tool.apply_line(Vector2(-150.0, 150.0), Vector2(-138.0, 150.0), 42) == 0, "a redrawn hedge must not stack")
	# An end drawn just off the row's end lands on it, and the hedge brush has no size to step.
	_expect(simulation.snap_vegetation_line_end(Vector2(-137.6, 150.5)) == Vector2(-138.0, 150.0), "a hedge end must join the row it is drawn onto")
	var held_radius := tool.radius
	tool.step_radius(1)
	_expect(tool.radius == held_radius, "a hedge brush must not change size")
	tool.option_index = 0
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

	# The terrain palette: every option is in a category, with Rust's sample and its baked picture.
	var panel := TerrainPanel.new()
	host.add_child(panel)
	panel.build(manager)
	var listed := 0
	for group in TerrainPanel.GROUPS:
		listed += TerrainPanel.group_options(group[0]).size()
	_expect(listed == VegetationTool.BRUSH_OPTIONS.size(), "every option must sit in one category")
	var labels := VegetationTool.BRUSH_OPTIONS.map(func(option): return option.label)
	for group in TerrainPanel.GROUPS:
		_expect(group[2] in labels, "%s's tile picture must be one of the options" % group[1])
	for index in VegetationTool.BRUSH_OPTIONS.size():
		var option: Dictionary = VegetationTool.BRUSH_OPTIONS[index]
		_expect(not simulation.get_vegetation_preset_sample(option.preset).is_empty(), "%s must have a picture sample" % option.label)
		_expect(ResourceLoader.exists(TerrainPanel.picture_path(index)), "%s must have its baked picture (tests/bake_vegetation_pictures.gd)" % option.label)
	var hedge_sample: PackedFloat32Array = simulation.get_vegetation_preset_sample(17)
	_expect(hedge_sample.size() == 4 * 6, "a hedge's picture is a row of four modules")
	# A category tile opens its options and plants its first; an option tile picks itself.
	panel._group_buttons["shrubs"].pressed.emit()
	var shrubs := TerrainPanel.group_options("shrubs")
	_expect(panel._options_panel.visible and panel._option_buttons.size() == shrubs.size(), "Shrubs opens its options")
	_expect(tool.option_index == shrubs[0] and tool.mode == VegetationTool.Mode.PLANT, "a category plants its first option")
	panel._option_buttons[shrubs[2]].pressed.emit()
	_expect(tool.option_index == shrubs[2] and panel._option_buttons[shrubs[2]].button_pressed, "an option tile picks and marks itself")
	# Shift + wheel steps over every option and wraps; the palette opens the category it lands in.
	tool.option_index = 0
	var cycle := _wheel(MOUSE_BUTTON_WHEEL_DOWN)
	cycle.ctrl_pressed = false
	cycle.shift_pressed = true
	tool._unhandled_input(cycle)
	_expect(tool.option_index == VegetationTool.BRUSH_OPTIONS.size() - 1, "cycling backward must wrap")
	_expect(panel._open_group == "rocks", "cycling opens the option's category: %s" % panel._open_group)
	cycle.button_index = MOUSE_BUTTON_WHEEL_UP
	tool._unhandled_input(cycle)
	_expect(tool.option_index == 0 and panel._open_group == "trees", "cycling forward must wrap")
	var remove_button: Button = panel._remove_button
	var erase := InputEventKey.new()
	erase.keycode = KEY_E
	erase.pressed = true
	manager._unhandled_input(erase)
	_expect(tool.mode == VegetationTool.Mode.REMOVE and remove_button.button_pressed, "E must enable erase and synchronize Remove")
	_expect(tool._option_label.text == "Remove" and tool._option_label.modulate == VegetationTool.UIStyle.TEXT_ALERT, "erase label must match the destructive mode")
	# If programmatic selection re-fired item_selected, its handler would leave erase mode.
	tool.step_option(1)
	_expect(tool.mode == VegetationTool.Mode.REMOVE and remove_button.button_pressed, "cycling keeps Remove")
	manager._unhandled_input(erase)
	_expect(tool.mode == VegetationTool.Mode.PLANT and not remove_button.button_pressed, "E must restore planting and synchronize Remove")
	_expect(tool._option_label.text == VegetationTool.BRUSH_OPTIONS[tool.option_index].label, "plant label must show the active option")
	# Neither depth-tests, so only the draw order keeps the text off the ring it names.
	_expect(tool._option_label.render_priority > tool._ring_material.render_priority
		and tool._option_label.outline_render_priority > tool._ring_material.render_priority,
		"the label and its outline must sort above the ground ring")
	remove_button.pressed.emit()
	_expect(tool.mode == VegetationTool.Mode.REMOVE and remove_button.button_pressed, "Remove must update the same mode as E")
	remove_button.pressed.emit()
	_expect(tool.mode == VegetationTool.Mode.PLANT and not remove_button.button_pressed, "Remove must toggle back to planting")
	# Number keys: 1 Raise and lower, 2-6 the categories, 7 Remove. A category's digit opens it,
	# then steps through its options.
	var slots := panel.number_slots()
	_expect(slots.size() == 7 and slots[6][0] == remove_button, "the palette numbers its seven tiles")
	slots[3][1].call()
	var flowers := TerrainPanel.group_options("flowers")
	_expect(panel._open_group == "flowers" and tool.option_index == flowers[0], "3 opens Flowers")
	slots[3][1].call()
	_expect(panel._open_group == "flowers" and tool.option_index == flowers[1], "3 again steps to the next flower")
	slots[6][1].call()
	_expect(tool.mode == VegetationTool.Mode.REMOVE, "7 is Remove")
	slots[3][1].call()
	_expect(tool.mode == VegetationTool.Mode.PLANT and tool.option_index == flowers[1], "the open category's digit leaves Remove first")
	panel.free()

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
			_expect(TreeSpecies.canopy_model(TreeSpecies.CONIFER, pin - 1).begins_with("pine_") == named[1],
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
