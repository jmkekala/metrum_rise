# SPDX-License-Identifier: GPL-2.0-only

## The road builder window (`ROAD-101`) through Main.tscn and the player's gestures: opened from
## the Roads row's drop-down; sidewalk, backward lane, grass median, forward lane, sidewalk built
## by clicking a slot and then tiles, and again through the drop handlers (tiles in, parts along
## and off the strip); the part popup's median style and width, Move and Remove; Rust's widths
## and the "1 + 1" label; an invalid order shows its reason with Create disabled; create with the
## suggested name and with a typed one (Enter), a taken name refused; the new type in the Roads
## row draws a road with its layout; edit (the tool re-picks it), duplicate (a custom type and a
## built-in), delete after a confirmation; an unusable entry listed with its reason and fixed;
## an unparsable library shows its error and writes nothing; Esc and Cancel write nothing. The
## type's attributes (`ROAD-101`): Sidewalks off draws shoulders and sends no sidewalk parts, the
## car-only warning, the Speed stepper (zoning off once at 80 km/h, unless the player set it, and
## never for an edited type), Rust's speed refusal and cost line; the created type's fields, the
## road tool's attributes and the drawn edge; Save keeps an edited type's own speed and zoning.
## Writes the road-type library, so it runs only in the isolated profile.
extends SceneTree

const TIMEOUT_MS := 60000
const LIBRARY := "user://road_types.json"
const SW := {"kind": "sidewalk"}
const BACK := {"kind": "car_lane", "forward": false}
const AHEAD := {"kind": "car_lane", "forward": true}
const SH := {"kind": "shoulder"}
# The palette's tile order.
const TILE_BACK := 0
const TILE_AHEAD := 1
const TILE_PAINTED := 2
const TILE_GRASS := 3

var _failures := 0
var _main: Node
var _simulation: Node
var _input: Node
var _tool: Node
var _ui: Node
var _builder: Node
var _changes: Array[String] = []

func _initialize() -> void:
	call_deferred("_run")

func _expect(condition: bool, message: String) -> void:
	if not condition:
		_failures += 1
		push_error(message)

func _ground(x: float, z: float) -> Vector3:
	return Vector3(x, _simulation.get_world_surface_height(Vector2(x, z)), z)

func _await_generation(generation: int) -> bool:
	var deadline := Time.get_ticks_msec() + TIMEOUT_MS
	while Time.get_ticks_msec() < deadline:
		if _simulation.get_network_render_generation() > generation:
			return true
		await process_frame
	return false

# One straight stroke along z = `z` through the road tool; returns the drawn edge (-1 if none).
func _draw(z: float) -> int:
	var points := PackedVector3Array([_ground(-100.0, z), _ground(100.0, z)])
	var generation: int = _simulation.get_network_render_generation()
	_tool._begin_stroke(points[0], 0, Vector3.ZERO)
	_tool.set_scripted_pointer(true, points[-1])
	_tool._process(1.0 / 60.0)
	_expect(bool(_tool._preview.get("is_valid", false)), "the stroke at z %.0f previews: %s" % [z, _tool._preview.get("invalid_reason", "")])
	_expect(_tool._commit_segment(points[-1]), "the click at z %.0f queues the road" % z)
	var deadline := Time.get_ticks_msec() + TIMEOUT_MS
	while _tool._commit_request_id > 0 and Time.get_ticks_msec() < deadline:
		await process_frame
	_tool.cancel_road()
	_tool.set_scripted_pointer(false)
	_expect(await _await_generation(generation), "the stroke at z %.0f publishes" % z)
	return _simulation.get_hovered_edge(0.0, z)

func _layout(edge: int) -> Array:
	var section: Dictionary = _simulation.get_edge_cross_section(edge)
	return [int(section.get("fwd_lanes", -1)), int(section.get("bkw_lanes", -1)),
		int(section.get("median_style", -1)), float(section.get("median_width", -1.0)),
		float(section.get("width", -1.0))]

func _library_bytes() -> PackedByteArray:
	return FileAccess.get_file_as_bytes(LIBRARY) if FileAccess.file_exists(LIBRARY) else PackedByteArray()

func _type(id: String) -> Dictionary:
	for type: Dictionary in _simulation.get_road_types().get("types", []):
		if String(type.get("id", "")) == id:
			return type
	return {}

func _type_named(type_name: String) -> Dictionary:
	for type: Dictionary in _simulation.get_road_types().get("types", []):
		if String(type.get("name", "")) == type_name:
			return type
	return {}

func _open() -> void:
	_ui.road_type_menu.custom_menu.get_popup().id_pressed.emit(_ui.road_type_menu.BUILDER_ITEM)
	await process_frame
	await process_frame

func _click(at: Vector2) -> void:
	for pressed in [true, false]:
		var event := InputEventMouseButton.new()
		event.button_index = MOUSE_BUTTON_LEFT
		event.pressed = pressed
		event.position = at
		_builder.strip._gui_input(event)

func _click_part(index: int) -> void:
	_click(_builder.strip.part_rect(index).get_center())
	await process_frame

# Remove in the open popup. The focus comes back from the closed popup on the next frame,
# before a player's next click could land.
func _remove_in_popup() -> void:
	_builder.part_popup.remove_button.pressed.emit()
	await process_frame

func _click_slot(slot: int) -> void:
	_click(_builder.strip.slot_position(slot))

func _tile(index: int) -> void:
	_builder.tiles[index].pressed.emit()

# The strip point on slot `slot`'s boundary, in the surface row.
func _slot_point(slot: int) -> Vector2:
	return Vector2(_builder.strip._xs[slot], _builder.strip.SURFACE_TOP + 20.0)

func _drop_tile(index: int, slot: int) -> void:
	var data = _builder.tiles[index]._get_drag_data(Vector2(4.0, 4.0))
	_expect(_builder.strip._can_drop_data(_slot_point(slot), data), "the strip takes tile %d" % index)
	_builder.strip._drop_data(_slot_point(slot), data)
	_builder.strip.notification(Control.NOTIFICATION_DRAG_END)

func _drag_part(index: int, slot: int) -> void:
	var data = _builder.strip._get_drag_data(_builder.strip.part_rect(index).get_center())
	_expect(data is Dictionary and data.road_builder == "part", "part %d drags" % index)
	_expect(_builder.strip._can_drop_data(_slot_point(slot), data), "the strip takes part %d back" % index)
	_builder.strip._drop_data(_slot_point(slot), data)
	_builder.strip.notification(Control.NOTIFICATION_DRAG_END)

func _drag_part_off(index: int) -> void:
	var data = _builder.strip._get_drag_data(_builder.strip.part_rect(index).get_center())
	_expect(data is Dictionary, "part %d drags off" % index)
	_builder.strip.notification(Control.NOTIFICATION_DRAG_END)

func _key(keycode: Key) -> void:
	var event := InputEventKey.new()
	event.keycode = keycode
	event.pressed = true
	_builder.window.window_input.emit(event)

func _type_name(text: String) -> void:
	_builder.name_edit.text = text
	_builder.name_edit.text_changed.emit(text)

func _run() -> void:
	# The test writes user://road_types.json: never in a real profile.
	if not OS.get_user_data_dir().to_lower().contains("godot-tests"):
		push_error("road_builder_test must run in the isolated profile from ./run.sh --test")
		quit(1)
		return
	DirAccess.remove_absolute(ProjectSettings.globalize_path(LIBRARY))
	_main = load("res://scenes/Main.tscn").instantiate()
	_main.set_script(null)
	root.add_child(_main)
	current_scene = _main
	await process_frame
	await process_frame
	_simulation = _main.get_node("SimulationNode")
	_input = _main.get_node("InputManager")
	_tool = _main.get_node("RoadTool")
	_ui = _main.get_node("MainUI")
	_expect(_simulation.create_blank_world(512.0, 512.0, 8.0, 128.0, 0.0), "the blank world must load")
	_input._refresh_after_world_load()
	_input.set_simulation_speed(0.0)
	var ready_deadline := Time.get_ticks_msec() + TIMEOUT_MS
	while not _input._world_ready and Time.get_ticks_msec() < ready_deadline:
		await process_frame
	_expect(_input._world_ready, "the world must become ready for input")
	_builder = _ui.road_builder
	_builder.road_types_changed.connect(func(id: String): _changes.append(id))
	var styles: Dictionary = _simulation.get_median_styles()
	var grass_m := float(styles[2].default_m)
	var paving_m := float(styles[3].default_m)
	var grass := {"kind": "median", "style": 2, "width_m": grass_m}
	var built := [SW, BACK, grass, AHEAD, SW]

	# Opened from the Roads row: a movable, resizable, non-exclusive window on a 1 + 1 layout.
	await _open()
	var window: Window = _builder.window
	_expect(_builder.is_open() and not window.exclusive and not window.unresizable, "Road builder... opens a non-exclusive resizable window")
	_expect(window.title == "Road Builder" and window.get_meta("metrum_base_window_min_size") == Vector2i(760, 400)
		and window.get_meta("metrum_base_window_size") == Vector2i(960, 440), "the window: %s" % window.title)
	var content_y := int((window.get_child(0) as Control).get_combined_minimum_size().y)
	_expect(content_y <= _builder.MIN_SIZE.y, "the smallest window holds every row: %d" % content_y)
	_expect(_builder.parts == [SW, BACK, AHEAD, SW] and _builder.lanes_label.text == "1 + 1", "a new layout is 1 + 1: %s" % [_builder.parts])
	_expect(not _builder.create_button.disabled and _builder.create_button.text == "Create road", "a new 1 + 1 can be created")
	_expect(_builder.name_edit.text == String(_builder.check.suggested_name) and not _builder.name_edit.text.is_empty(),
		"the name is Rust's suggestion: %s" % _builder.name_edit.text)
	_expect(_builder.tiles.size() == 5, "five part tiles")
	_expect(_builder.sidewalks_check.button_pressed and _builder.zoning_check.button_pressed and int(_builder.speed_box.value) == 50
		and _builder.speed_row.visible, "a new type has sidewalks, 50 km/h and zoning")
	_expect(_builder.cost_label.text == "15 / m, upkeep 0.10 / m / day" and _builder.warning_label.text.is_empty(),
		"Rust's cost of the 2-Lane Road: %s" % _builder.cost_label.text)

	# Click-insert: the popup removes both lanes, then a slot and three tiles build the road.
	await _click_part(1)
	var popup = _builder.part_popup
	_expect(popup.visible and popup._title.text == "Backward lane" and not popup.median_controls.visible, "a lane's popup: %s" % popup._title.text)
	await _remove_in_popup()
	await _click_part(1)
	_expect(popup.visible and _builder._popup_index == 1 and popup._title.text == "Forward lane",
		"the popup opens again on the next part: %s, %d" % [popup.visible, _builder._popup_index])
	await _remove_in_popup()
	_expect(_builder.parts == [SW, SW], "the popup removed both lanes: %s" % [_builder.parts])
	_expect(not _builder.strip.valid and _builder.create_button.disabled and _builder.status_label.text == "A road needs at least one car lane",
		"bare sidewalks are refused: %s" % _builder.status_label.text)
	_click(_builder.strip.part_rect(0).get_center())
	_expect(not popup.visible, "a sidewalk has no popup")
	_click_slot(1)
	_tile(TILE_BACK)
	_tile(TILE_GRASS)
	_tile(TILE_AHEAD)
	_expect(_builder.parts == built, "click-insert builds sidewalk, lane <-, grass, lane ->, sidewalk: %s" % [_builder.parts])
	_expect(_builder.strip.valid and _builder.lanes_label.text == "1 + 1", "the built layout is 1 + 1: %s" % _builder.lanes_label.text)
	var widths: PackedFloat32Array = _builder.check.part_widths_m
	_expect(Array(widths) == [1.5, 3.5, grass_m, 3.5, 1.5], "Rust's part widths: %s" % widths)
	_expect(_builder.widths_label.text == "Road %.1f m\nTotal %.1f m" % [7.0 + grass_m, 10.0 + grass_m], "the widths: %s" % _builder.widths_label.text)
	for px in _builder.strip._ws:
		_expect(px >= _builder.strip.MIN_PART_PX, "every part is at least 24 px: %.1f" % px)
	_expect(_builder.strip._ws[1] > _builder.strip._ws[0], "the strip is to scale: a lane wider than a sidewalk")

	# The drop handlers: parts dragged off, tiles dropped in, a part dragged along.
	_builder.library_list.new_button.pressed.emit()
	_drag_part_off(1)
	_drag_part_off(1)
	_expect(_builder.parts == [SW, SW], "parts dragged off the strip are removed: %s" % [_builder.parts])
	_drop_tile(TILE_BACK, 1)
	_drop_tile(TILE_GRASS, 2)
	_drop_tile(TILE_AHEAD, 3)
	_expect(_builder.parts == built, "dropped tiles build the same road: %s" % [_builder.parts])
	_drag_part(1, 4)
	_expect(_builder.parts == [SW, grass, AHEAD, BACK, SW], "a dragged part moves: %s" % [_builder.parts])
	_expect(not _builder.strip.valid and _builder.create_button.disabled and _builder.status_label.text == "Lanes against traffic go on the left",
		"an invalid order shows its reason: %s" % _builder.status_label.text)
	_drag_part(3, 1)
	_expect(_builder.parts == built and not _builder.create_button.disabled, "dragged back: %s" % [_builder.parts])

	# The median popup: width and style, Move left / right.
	await _click_part(2)
	var median = popup.median_controls
	_expect(popup.visible and median.visible and not median._style_buttons[0].visible, "a median's popup offers its styles without None")
	median._width_buttons[1].pressed.emit()
	_expect(is_equal_approx(float(_builder.parts[2].width_m), grass_m + 0.5), "+ widens the median: %s" % [_builder.parts[2]])
	median._style_buttons[3].pressed.emit()
	_expect(_builder.parts[2] == {"kind": "median", "style": 3, "width_m": paving_m}, "Paving takes its default width: %s" % [_builder.parts[2]])
	popup.move_left_button.pressed.emit()
	_expect(String(_builder.parts[1].kind) == "median" and _builder.status_label.text == "A median sits between the two directions",
		"Move left: %s / %s" % [_builder.parts, _builder.status_label.text])
	popup.move_right_button.pressed.emit()
	median._style_buttons[2].pressed.emit()
	_expect(_builder.parts == built, "Move right and Grass restore it: %s" % [_builder.parts])
	popup.hide()
	await process_frame

	# Create with the suggested name: the type is written, listed and picked for the road tool.
	var suggested: String = _builder.name_edit.text
	_builder.create_button.pressed.emit()
	await process_frame
	var boulevard := String(_type_named(suggested).get("id", ""))
	_expect(not _builder.is_open() and not boulevard.is_empty(), "Create writes %s and closes" % suggested)
	_expect(_changes == [boulevard], "road_types_changed carries the new id: %s" % [_changes])
	_expect(_ui.road_type_menu.custom_ids.has(boulevard) and _ui.road_type_menu.custom_marked(), "the Roads row lists and marks it")
	_expect(_input.current_tool == _input.Tool.ROAD and _tool.road_type_id == boulevard, "the road tool draws it")
	var edge := await _draw(-150.0)
	_expect(_layout(edge) == [1, 1, 2, grass_m, 7.0 + grass_m], "the new type draws its layout: %s" % [_layout(edge)])

	# A typed name, built by click-insert, created with Enter in the name field.
	await _open()
	_click_slot(1)
	_tile(TILE_BACK)
	_click_slot(3)
	_tile(TILE_PAINTED)
	_tile(TILE_AHEAD)
	_expect(_builder.lanes_label.text == "2 + 2", "2 + 2 painted: %s" % [_builder.parts])
	_type_name("Builder Test Avenue")
	_builder.name_edit.text_submitted.emit(_builder.name_edit.text)
	await process_frame
	var avenue := String(_type_named("Builder Test Avenue").get("id", ""))
	_expect(not avenue.is_empty() and not _builder.is_open(), "Enter creates the typed name")

	# A taken name is refused, also through Enter; an empty one too.
	await _open()
	var before := _library_bytes()
	for taken in ["2-lane road", "builder test AVENUE"]:
		_type_name(taken)
		_expect(_builder.create_button.disabled and _builder.status_label.text == "Another road type has this name",
			"%s is taken: %s" % [taken, _builder.status_label.text])
	_key(KEY_ENTER)
	_type_name("  ")
	_expect(_builder.create_button.disabled and _builder.status_label.text == "Enter a name", "an empty name: %s" % _builder.status_label.text)
	_builder.create()
	_expect(_library_bytes() == before and _builder.is_open(), "a refused name writes nothing")

	# Esc and Cancel close without writing.
	_drag_part_off(1)
	_key(KEY_ESCAPE)
	_expect(not _builder.is_open() and _library_bytes() == before, "Esc closes without writing")
	await _open()
	_tile(TILE_GRASS)
	_builder.cancel_button.pressed.emit()
	_expect(not _builder.is_open() and _library_bytes() == before, "Cancel closes without writing")

	# Edit: the type the tool holds is saved with a new median; the tool re-picks it.
	_ui.road_type_menu.pick(avenue)
	_changes.clear()
	await _open()
	_builder.library_list.pick(avenue)
	_expect(_builder.editing_id == avenue and _builder.create_button.text == "Save" and _builder.name_edit.text == "Builder Test Avenue",
		"selecting a custom type edits it: %s" % _builder.header_label.text)
	_expect(_builder.parts == _type(avenue).parts, "its layout is loaded")
	await _click_part(3)
	popup.median_controls._width_buttons[1].pressed.emit()
	var wider := float(_builder.parts[3].width_m)
	_expect(is_equal_approx(wider, float(_type(avenue).median_width) + 0.5), "+ widens the painted median: %.1f" % wider)
	popup.hide()
	await process_frame
	_builder.create_button.pressed.emit()
	await process_frame
	_expect(is_equal_approx(float(_type(avenue).median_width), wider) and _type(avenue).name == "Builder Test Avenue", "Save writes the edit: %s" % _type(avenue).median_width)
	_expect(_changes == [""] and is_equal_approx(_tool.median_width, wider) and _tool.road_type_id == avenue, "the tool re-picks the edited type")

	# Duplicate: a custom type and a built-in start new types; the walkway cannot be duplicated.
	await _open()
	var list = _builder.library_list
	list.pick(avenue)
	list.duplicate_button.pressed.emit()
	_expect(_builder.editing_id.is_empty() and _builder.parts == _type(avenue).parts and _builder.create_button.text == "Create road",
		"Duplicate starts a new type from it")
	var copy_name: String = _builder.name_edit.text
	_expect(copy_name != "Builder Test Avenue" and not _builder.create_button.disabled, "the copy has a free name: %s" % copy_name)
	list.pick("builtin:road_4l")
	_expect(_builder.parts == [SW, BACK, BACK, AHEAD, AHEAD, SW] and _builder.editing_id.is_empty() and list.delete_button.disabled,
		"a built-in starts a new type and cannot be deleted")
	list.pick("builtin:walkway")
	_expect(list.duplicate_button.disabled, "the walkway cannot be duplicated")
	list.pick(avenue)
	list.duplicate_button.pressed.emit()
	_builder.create_button.pressed.emit()
	await process_frame
	var copy := String(_type_named(copy_name).get("id", ""))
	_expect(not copy.is_empty() and copy != avenue, "the duplicate is created: %s" % copy)

	# Delete asks first; cancelling keeps the type.
	await _open()
	list.pick(copy)
	list.delete_button.pressed.emit()
	await process_frame
	var dialog: ConfirmationDialog = _builder.delete_dialog
	_expect(dialog.visible and dialog.dialog_text.contains(copy_name), "Delete asks: %s" % dialog.dialog_text)
	dialog.canceled.emit()
	dialog.hide()
	_expect(not _type(copy).is_empty(), "a cancelled delete keeps the type")
	_changes.clear()
	list.delete_button.pressed.emit()
	dialog.confirmed.emit()
	dialog.hide()
	_expect(_type(copy).is_empty() and list.index_of(copy) < 0 and _changes == [""], "a confirmed delete removes it")
	_expect(_builder.editing_id.is_empty() and _builder.parts == [SW, BACK, AHEAD, SW], "deleting the edited type starts a new one")
	_expect(not _ui.road_type_menu.custom_ids.has(copy), "the Roads row drops it")
	_builder.close()

	# Attributes: Sidewalks off (both ends together, shoulders drawn), the car-only warning, the
	# Speed stepper with zoning off once at 80 km/h, Rust's speed refusal and cost line.
	await _open()
	_builder.sidewalks_check.button_pressed = false
	_expect(_builder.parts == [SH, BACK, AHEAD, SH] and _builder.rust_parts() == [BACK, AHEAD], "no sidewalks: shoulder ends, not parts: %s" % [_builder.parts])
	_expect(_builder.strip.valid and not bool(_builder.check.sidewalks) and Array(_builder.strip.widths_m) == [0.75, 3.5, 3.5, 0.75],
		"the strip draws 0.75 m shoulders: %s" % _builder.strip.widths_m)
	_expect(_builder.widths_label.text == "Road 7.0 m\nTotal 8.5 m", "the widths without sidewalks: %s" % _builder.widths_label.text)
	_expect(_builder.status_label.text == "1 + 1 lanes, 50 km/h, no sidewalks, 8.5 m wide", "the description: %s" % _builder.status_label.text)
	_expect(_builder.warning_label.text == _builder.CAR_ONLY_WARNING, "no sidewalks with zoning warns: %s" % _builder.warning_label.text)
	_expect(not _builder.strip.is_interior(0) and _builder.strip.is_interior(1), "the shoulders are fixed ends")
	_builder.speed_box.value = 70
	_expect(_builder.speed_kmh == 70 and _builder.zoning, "70 km/h keeps zoning")
	_builder.speed_box.value = 80
	_expect(not _builder.zoning and not _builder.zoning_check.button_pressed and _builder.warning_label.text.is_empty(),
		"a new type reaching 80 km/h turns zoning off")
	_builder.speed_box.value = 50
	_builder.speed_box.value = 90
	_expect(not _builder.zoning, "only once: zoning stays off")
	_builder.zoning_check.button_pressed = true
	_expect(_builder.zoning and _builder.warning_label.text == _builder.CAR_ONLY_WARNING, "zoning on again warns")
	_builder.zoning_check.button_pressed = false
	_builder.set_speed(105)
	_expect(_builder.create_button.disabled and _builder.status_label.text == "Speed between 20 and 120 km/h, in steps of 10",
		"Rust refuses a speed off the grid: %s" % _builder.status_label.text)
	_builder.speed_box.value = 100
	_expect(_builder.cost_label.text == "24 / m, upkeep 0.16 / m / day" and is_equal_approx(float(_builder.check.build_per_m), 24.0),
		"Rust's cost at 100 km/h without sidewalks: %s" % _builder.cost_label.text)
	_builder.sidewalks_check.button_pressed = true
	_expect(_builder.parts == [SW, BACK, AHEAD, SW], "sidewalks back on: %s" % [_builder.parts])
	_builder.sidewalks_check.button_pressed = false
	_type_name("Builder Test Highway")
	_builder.create_button.pressed.emit()
	await process_frame
	var highway := _type_named("Builder Test Highway")
	_expect(int(highway.get("speed_kmh", 0)) == 100 and not bool(highway.get("zoning", true)) and not bool(highway.get("sidewalks", true))
		and highway.get("parts", []) == [BACK, AHEAD], "the created type: %s" % highway)
	var highway_id := String(highway.get("id", ""))
	_expect(_tool.road_type_id == highway_id and _tool.speed_kmh == 100 and not _tool.sidewalks and not _tool.zoning,
		"the road tool draws its attributes: %d %s %s" % [_tool.speed_kmh, _tool.sidewalks, _tool.zoning])
	var rural := await _draw(-100.0)
	var rural_section: Dictionary = _simulation.get_edge_cross_section(rural)
	_expect(int(rural_section.get("speed_kmh", 0)) == 100 and not bool(rural_section.get("sidewalks", true))
		and not bool(rural_section.get("zoning", true)) and String(rural_section.get("road_type", "")) == highway_id,
		"the drawn road has the type's attributes: %s" % rural_section)
	# Edit: the type's own attributes load and Save keeps them; an edit suggests nothing.
	await _open()
	list.pick(highway_id)
	_expect(_builder.parts == [SH, BACK, AHEAD, SH] and not _builder.sidewalks_check.button_pressed and int(_builder.speed_box.value) == 100
		and not _builder.zoning_check.button_pressed, "an edited type loads its attributes")
	_builder.speed_box.value = 110
	_builder.create_button.pressed.emit()
	await process_frame
	var saved := _type(highway_id)
	_expect(int(saved.get("speed_kmh", 0)) == 110 and not bool(saved.get("zoning", true)) and not bool(saved.get("sidewalks", true)),
		"Save keeps the edited type's own attributes: %s" % saved)
	await _open()
	list.pick(avenue)
	_builder.speed_box.value = 90
	_expect(_builder.zoning and _builder.zoning_check.button_pressed, "an edited type keeps its zoning at 90 km/h")
	_builder.library_list.new_button.pressed.emit()
	_builder.zoning_check.button_pressed = false
	_builder.zoning_check.button_pressed = true
	_builder.speed_box.value = 80
	_expect(_builder.zoning, "zoning the player set stays on at 80 km/h")
	_builder.close()

	# An unusable entry, added by hand: listed with its reason, not in the Roads row, fixed.
	# Spliced into Rust's text: Godot's JSON would write the integers back as floats.
	var text := _library_bytes().get_string_from_utf8()
	var close := text.rfind("]")
	var entry := JSON.stringify({"id": "custom:90", "name": "Broken Road", "parts": [SW, AHEAD, BACK, SW]})
	var writer := FileAccess.open(LIBRARY, FileAccess.WRITE)
	writer.store_string(text.substr(0, close).strip_edges(false, true) + ",\n    " + entry + "\n  " + text.substr(close))
	writer.close()
	await _open()
	var row: int = list.index_of("custom:90")
	_expect(row >= 0 and list.list.get_item_text(row).contains("Broken Road: Lanes against traffic go on the left"),
		"the unusable entry is listed with its reason: %s" % (list.list.get_item_text(row) if row >= 0 else "missing"))
	_expect(not _ui.road_type_menu.custom_ids.has("custom:90"), "the Roads row leaves it out")
	list.pick("custom:90")
	_expect(_builder.editing_id == "custom:90" and _builder.create_button.disabled, "it opens for editing, not savable as it is")
	_drag_part(2, 1)
	_builder.create_button.pressed.emit()
	await process_frame
	_expect(bool(_type("custom:90").get("usable", false)) and _ui.road_type_menu.custom_ids.has("custom:90"), "the fixed entry is usable and listed")

	# An unparsable library: the error on top, nothing written, until the file is removed.
	var broken := "{ not a road library".to_utf8_buffer()
	writer = FileAccess.open(LIBRARY, FileAccess.WRITE)
	writer.store_buffer(broken)
	writer.close()
	await _open()
	_expect(list.error_label.visible and list.error_label.text.begins_with("The road library file could not be read"),
		"the library error shows: %s" % list.error_label.text)
	_expect(_builder.strip.valid and _builder.create_button.disabled and _builder.status_label.text.begins_with("The road library file could not be read"),
		"Create is refused: %s" % _builder.status_label.text)
	_builder.create()
	_key(KEY_ENTER)
	_expect(_library_bytes() == broken, "the unparsable file is left as it was")
	_key(KEY_ESCAPE)
	DirAccess.remove_absolute(ProjectSettings.globalize_path(LIBRARY))
	await _open()
	_expect(not list.error_label.visible and not _builder.create_button.disabled, "a removed file clears the error")
	_builder.close()

	_input._toggle_tool(_input.Tool.SELECT)
	_main.queue_free()
	await process_frame
	if _failures == 0:
		print("road_builder_test: PASS")
	quit(_failures)
