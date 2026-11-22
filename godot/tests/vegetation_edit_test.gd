# SPDX-License-Identifier: GPL-3.0-or-later

## Headless native vegetation edit, persistence, tool routing and preview contract.
## Uses explicit failures because headless GDScript assertions do not terminate a test.
extends SceneTree

const VegetationTool := preload("res://scripts/tools/vegetation_tool.gd")
const InputManager := preload("res://scripts/core/input_manager.gd")
const TerrainPanel := preload("res://scripts/ui/terrain_panel.gd")
const Vegetation := preload("res://scripts/renderers/vegetation.gd")
const VegetationPlacements := preload("res://scripts/renderers/vegetation_placements.gd")
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

func _arrow(key: Key) -> InputEventKey:
	var event := InputEventKey.new()
	event.keycode = key
	event.pressed = true
	return event

func _expect(condition: bool, message: String) -> void:
	if not condition:
		_failures += 1
		push_error(message)

func _expect_height_label(tool: VegetationTool, expected: String, step: int) -> void:
	_expect(tool._option_label.text == expected, "point label must show the held plant's mesh height in metres")
	_expect(tool._size_indicator.visible and tool._size_indicator.texture == tool._size_indicator_textures[step],
		"point label must highlight the visible size step")

func _check_canopy_model_heights(simulation: SimulationNode) -> void:
	var meshes := TreeSpecies.build_meshes()
	for species in [TreeSpecies.CONIFER, TreeSpecies.BROADLEAF]:
		for variant in range(TreeSpecies.VARIANT_COUNTS[species]):
			var mesh: ArrayMesh = meshes[species][variant][0]
			var height := mesh.get_aabb().size.y
			var mirrored := simulation.get_vegetation_canopy_model_height_m(species, variant)
			_expect(absf(height - mirrored) <= 0.05,
				"%s height is %.3f m, Rust mirrors %.3f m" % [
					TreeSpecies.canopy_model(species, variant), height, mirrored])

func _run() -> void:
	var host := Node3D.new()
	root.add_child(host)
	var simulation := SimulationNode.new()
	simulation.name = "SimulationNode"
	host.add_child(simulation)
	simulation.set_simulation_speed(0.0)
	_check_canopy_model_heights(simulation)
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
	var initial_pick := tool._point_pick
	tool.option_index = 2
	var held_pick := tool._point_pick
	_expect(held_pick != initial_pick, "choosing Birch must pick a new plant")
	_expect(tool.preset == VegetationTool.BRUSH_OPTIONS[2].preset, "the option must resolve its native preset ordinal")
	tool.radius = VegetationTool.MIN_RADIUS_M
	var point := Vector2(11.0, 13.0)
	var planned: Variant
	for _i in range(100):
		planned = simulation.preview_vegetation_pick_at(point, tool.preset, held_pick, tool._point_size_step)
		if planned is PackedFloat32Array:
			tool._update_point_preview(point)
			if tool._point_ghost != null:
				break
		await create_timer(0.01).timeout
	_expect(planned is PackedFloat32Array and planned.size() == 6, "clear point must return a packed plant")
	_expect(tool._point_ghost != null and tool._point_ghost.visible, "a clear point must show a ghost")
	if tool._point_ghost == null:
		quit(1)
		return
	_expect((int(planned[5]) >> Vegetation.SPECIES_BITS) & VegetationPlacements.FIXED_LOOK != 0,
		"point preview must carry the fixed-look flag")
	_expect_height_label(tool, "Birch 14 m", 2)
	var mesh := tool._point_ghost.multimesh.mesh
	var transform := tool._point_ghost.multimesh.get_instance_transform(0)
	var moved := point + Vector2(0.25, 0.0)
	for _i in range(100):
		tool._update_point_preview(moved, 1.0 / 60.0)
		if tool._point_pos == moved:
			break
		await create_timer(0.01).timeout
	_expect(tool._point_pos == moved and tool._point_pick == held_pick
		and tool._point_ghost.multimesh.mesh == mesh
		and tool._point_ghost.multimesh.get_instance_transform(0) == transform,
		"moving hover must keep the held plant's model and proportions")
	_expect(tool._point_tilt.x < 0.0 and tool._point_ghost.global_position.x == moved.x,
		"the ghost top must lag the cursor with the base at the new spot")
	for _i in range(90):
		tool._update_point_preview(moved, 1.0 / 60.0)
	_expect(tool._point_tilt == Vector2.ZERO, "the spring must settle at rest")
	var up := _arrow(KEY_UP)
	var down := _arrow(KEY_DOWN)
	tool._unhandled_input(up)
	_expect(tool._point_size_step == 3, "up must step one size")
	var upper: Variant
	for _i in range(100):
		upper = simulation.preview_vegetation_pick_at(moved, tool.preset, held_pick, 3)
		tool._update_point_preview(moved)
		if upper is PackedFloat32Array and tool._point_preview_step == 3:
			break
		await create_timer(0.01).timeout
	_expect(upper is PackedFloat32Array and tool._point_preview_step == 3, "the larger ghost must refresh")
	if upper is PackedFloat32Array:
		_expect_height_label(tool, "Birch 19 m", 3)
	tool._unhandled_input(down)
	for _i in range(100):
		tool._update_point_preview(moved)
		if tool._point_preview_step == 2:
			break
		await create_timer(0.01).timeout
	_expect_height_label(tool, "Birch 14 m", 2)
	for _i in range(8):
		tool._unhandled_input(up)
	_expect(tool._point_size_step == 4, "up must clamp at the largest size")
	for _i in range(100):
		tool._update_point_preview(moved)
		if tool._point_preview_step == 4:
			break
		await create_timer(0.01).timeout
	_expect_height_label(tool, "Birch 24 m", 4)
	for _i in range(8):
		tool._unhandled_input(down)
	_expect(tool._point_size_step == 0, "down must clamp at the smallest size")
	for _i in range(100):
		tool._update_point_preview(moved)
		if tool._point_preview_step == 0:
			break
		await create_timer(0.01).timeout
	_expect_height_label(tool, "Birch 4.0 m", 0)
	tool._unhandled_input(up)
	tool._unhandled_input(up)
	_expect(tool._point_pick == held_pick, "size keys must keep the held pick")
	for _i in range(100):
		tool._update_point_preview(point)
		if tool._point_pos == point:
			break
		await create_timer(0.01).timeout
	var ghost_mm := tool._point_ghost.multimesh
	var ghost_basis := ghost_mm.get_instance_transform(0).basis
	var ghost_tint := ghost_mm.get_instance_color(0)
	_expect(simulation.debug_hold_core(1000), "the fixture must hold the native core")
	var busy = simulation.preview_vegetation_pick_at(point, tool.preset, held_pick, tool._point_size_step)
	_expect(busy is bool and not busy, "hover must return busy while the core is held")
	tool._update_point_preview(point + Vector2(10.0, 0.0))
	_expect(tool._point_pos == point and tool._point_ghost.visible, "busy hover must keep the last ghost")
	tool._unhandled_input(up)
	var camera := Camera3D.new()
	camera.projection = Camera3D.PROJECTION_ORTHOGONAL
	camera.size = 10.0
	camera.position = Vector3(point.x, 80.0, point.y)
	camera.rotation.x = -PI * 0.5
	host.add_child(camera)
	camera.make_current()
	_expect(tool._stamp(true), "a point click on the surface must be handled")
	camera.free()
	_expect(tool._point_pick != held_pick and tool._point_size_step == 3,
		"a successful plant must choose the next pick and keep the selected size step")
	_expect(tool._option_label.text.is_empty() and not tool._size_indicator.visible,
		"the old height and size bars must clear while the next pick is pending")
	tool._unhandled_input(down)
	var planted := simulation.get_decorative_tree_patch(right, span, true)
	var planted_record := PackedFloat32Array()
	for at in range(0, planted.size(), 6):
		if planted[at] == point.x and planted[at + 2] == point.y:
			planted_record = planted.slice(at, at + 6)
	_expect(planted_record == planned, "the click must match all six lanes of the last visible ghost")
	if not planted_record.is_empty():
		var drawn := VegetationPlacements.instances(planted_record, tool._point_placements._meshes)
		var plant_mm: MultiMesh = drawn.get_child(0).multimesh
		_expect(plant_mm.mesh == mesh and plant_mm.get_instance_transform(0).basis == ghost_basis
			and plant_mm.get_instance_color(0) == ghost_tint,
			"the planted record must draw the held ghost's exact mesh, basis and tint")
		drawn.free()
	for _i in range(100):
		tool._update_point_preview(point)
		if tool._point_pos == point:
			break
		await create_timer(0.01).timeout
	_expect(tool._point_ghost.visible and tool._option_label.text == "Too close to another plant"
		and tool._option_label.modulate == VegetationTool.UIStyle.TEXT_ALERT
		and not tool._size_indicator.visible,
		"a click must refresh the same spot to show its blocked ghost and reason")
	var next_pick := tool._point_pick
	_expect(tool.apply_at(point) == 0 and tool._point_pick == next_pick,
		"a refused point plant must keep the pick")
	# Hiding the preview (the pointer over a panel) clears the reason; the same pick shown again
	# on clear ground must drop the blocked tint with it. The headless renderer keeps no instance
	# colours, so the tool's record of the tint it applied is read.
	_expect(tool._point_tinted_blocked, "a refused ghost must carry the blocked tint")
	tool._hide_point_preview()
	var clear_spot := point + Vector2(0.0, 40.0)
	for _i in range(100):
		tool._update_point_preview(clear_spot)
		if tool._point_pos == clear_spot:
			break
		await create_timer(0.01).timeout
	_expect(tool._point_pos == clear_spot and tool._point_reason.is_empty()
		and not tool._point_tinted_blocked,
		"a ghost shown again on clear ground after a refusal must drop the blocked tint")
	_expect(simulation.undo_action(), "point undo must queue")
	for _i in range(100):
		await create_timer(0.01).timeout
		tool._update_point_preview(point)
		if tool._point_reason.is_empty():
			break
	_expect(tool._point_ghost.visible and tool._point_reason.is_empty(), "undo must refresh a stationary ghost")
	_expect(tool.apply_at(point) == 1, "the point must be plantable again after undo")
	tool.mode = VegetationTool.Mode.REMOVE
	tool._unhandled_input(up)
	_expect(tool._point_size_step == 2, "arrows must leave the size alone while removing")
	_expect(not tool._point_ghost.visible and tool._point_reason.is_empty(), "remove mode must hide the point preview")
	tool.radius = 0.01
	_expect(tool.apply_at(Vector2(11.0, 13.0)) == 1, "remove mode must forward its radius")
	tool.mode = VegetationTool.Mode.PLANT
	tool.option_index = 0
	var pine_pick := tool._point_pick
	tool._unhandled_input(down)
	for _i in range(100):
		tool._update_point_preview(point)
		if tool._point_model_pick == pine_pick and tool._point_model_step == 1:
			break
		await create_timer(0.01).timeout
	_expect_height_label(tool, "Pine 10 m", 1)
	var young_mesh := tool._point_ghost.multimesh.mesh
	_expect(young_mesh.get_aabb().size.y < 10.0, "the 10 m step must use the young pine")
	tool._unhandled_input(up)
	for _i in range(100):
		tool._update_point_preview(point)
		if tool._point_model_step == 2:
			break
		await create_timer(0.01).timeout
	_expect_height_label(tool, "Pine 16 m", 2)
	var mature_mesh := tool._point_ghost.multimesh.mesh
	_expect(mature_mesh.get_aabb().size.y > 22.0 and mature_mesh != young_mesh,
		"the 16 m step must swap the ghost to the mature pine")
	tool._unhandled_input(down)
	for _i in range(100):
		tool._update_point_preview(point)
		if tool._point_model_step == 1:
			break
		await create_timer(0.01).timeout
	_expect_height_label(tool, "Pine 10 m", 1)
	_expect(tool._point_pick == pine_pick and tool._point_ghost.multimesh.mesh == young_mesh,
		"returning to 10 m must restore the same young pine and keep the held pick")
	tool._unhandled_input(up)
	tool.radius = 32.0
	tool._unhandled_input(up)
	_expect(tool._point_size_step == 2, "arrows must leave size alone while brush painting")
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
	var brush_radius := tool.radius
	tool.radius = VegetationTool.MIN_RADIUS_M
	tool._unhandled_input(down)
	_expect(tool._point_size_step == 2, "arrows must leave size alone while laying hedges")
	tool.radius = brush_radius
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
	tool.radius = 32.0
	_expect(tool._option_label.text == VegetationTool.BRUSH_OPTIONS[tool.option_index].label
		and not tool._size_indicator.visible, "the brush label must show only the active option")
	tool.radius = VegetationTool.MIN_RADIUS_M
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

	tool.radius = VegetationTool.MIN_RADIUS_M
	tool._show_brush_preview(Vector3(10.0, 0.0, 10.0), tool.radius)
	var flower: Variant
	for _i in range(100):
		flower = simulation.preview_vegetation_pick_at(point, tool.preset, tool._point_pick, 2)
		tool._update_point_preview(point)
		if flower is PackedFloat32Array and flower.size() == 6 and tool._point_pos == point:
			break
		await create_timer(0.01).timeout
	_expect(flower is PackedFloat32Array and flower.size() == 6, "the flower preview must fit")
	if flower is PackedFloat32Array and flower.size() == 6:
		var height := tool._point_ghost.multimesh.mesh.get_aabb().size.y * float(flower[4])
		_expect_height_label(tool, "Lupine %.1f m" % height, 2)
		_expect(height < 10.0, "the flower fixture must exercise decimal heights")
	_expect(tool._preview_visible, "a shown preview paints the ground disc")
	manager._activate_tool_logic(InputManager.Tool.VEGETATION, false)
	tool._process(0.0)
	# The disc is painted by the ground shaders from a global flag, so a tool left without
	# clearing it would keep a brush painted on the world after it is gone. The headless
	# renderer does not keep global parameters, so the tool's own record of the flag is read.
	_expect(not tool.active and not tool._preview_visible, "deactivation must hide the ground preview")
	_expect(not tool._point_ghost.visible and tool._point_reason.is_empty(), "deactivation must clear the point preview")
	tool._unhandled_input(down)
	_expect(tool._point_size_step == 2, "arrows must leave size alone while the tool is inactive")
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
			var pin := VegetationPlacements.variant_pin(packed)
			_expect((packed >> Vegetation.SPECIES_BITS) & VegetationPlacements.FIXED_LOOK == 0,
				"a brush stroke must keep the position-seeded look")
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
