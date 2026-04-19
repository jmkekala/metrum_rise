## Blank-world authoring shell — launched via `--world-editor`.
## Shares the same SimulationNode runtime as gameplay, but keeps the editor
## paused and exposes terrain and authored-water world-definition actions.
extends Node3D

const TopMenu = preload("res://scripts/ui/top_menu.gd")
const UIStyle = preload("res://scripts/ui/ui_style.gd")

const DEFAULT_WIDTH_KM := 20.0
const DEFAULT_HEIGHT_KM := 20.0
const DEFAULT_TERRAIN_CELL_M := 10.0
const DEFAULT_TERRAIN_CHUNK_M := 512.0
const DEFAULT_BASE_ELEVATION_M := 0.0
const DEFAULT_WATER_RATE := 0.5
const DEFAULT_LAKE_SURFACE_OFFSET_M := 5.0
const WORLDS_DIR := "user://worlds"
const SCULPT_RADIUS_M := 15.0
const SCULPT_STRENGTH_PER_SEC := 2.0
const WATER_REMOVE_RADIUS_M := 40.0

enum Tool {
	NONE,
	RAISE,
	LOWER,
	WATER_SOURCE,
	WATER_SINK,
	WATER_LAKE_FILL,
	WATER_OPEN_WATER,
}

@onready var sim: SimulationNode = $SimulationNode
@onready var terrain: MeshInstance3D = $Terrain
@onready var water: MeshInstance3D = $Water
@onready var editor_camera_input: Node = $EditorCameraInput

var _active_tool: Tool = Tool.RAISE
var _current_world_name := "Untitled World"
var _current_world_path := ""

var _toolbar_status: Label
var _raise_btn: Button
var _lower_btn: Button
var _water_group_btn: Button
var _water_tool_panel: PanelContainer
var _water_source_btn: Button
var _water_sink_btn: Button
var _water_lake_fill_btn: Button
var _water_open_water_btn: Button
var _water_rate_spin: SpinBox
var _lake_offset_spin: SpinBox

var _new_world_window: Window
var _new_world_name_edit: LineEdit
var _new_world_width_spin: SpinBox
var _new_world_height_spin: SpinBox
var _new_world_cell_spin: SpinBox
var _new_world_chunk_spin: SpinBox
var _new_world_base_spin: SpinBox
var _debug_enabled := false
var _lake_preview_active := false
var _lake_preview_seed_world_pos := Vector2.ZERO
var _lake_preview_seed_height_m := 0.0
var _lake_preview_surface_m := 0.0
var _lake_preview_valid := false
var _lake_preview_status := "inactive"
var _lake_preview_kind := "inactive"

func _ready() -> void:
	if not sim.is_world_editor_mode():
		push_error("WorldEditor scene loaded without --world-editor flag")

	_debug_enabled = _world_editor_debug_enabled()
	_attach_top_menu()
	_configure_editor_camera()
	_build_ui()
	_refresh_after_world_change(true)
	_update_tool_buttons()
	_set_status("Terrain world ready.")
	_debug_log("ready world=%s path=%s" % [_current_world_name, _world_path_label()])

func _process(delta: float) -> void:
	if not _is_sculpt_tool(_active_tool):
		return
	if not Input.is_mouse_button_pressed(MOUSE_BUTTON_LEFT):
		return
	if _is_pointer_over_ui():
		return
	_apply_sculpt(delta)

func _unhandled_input(event: InputEvent) -> void:
	if event is InputEventKey and event.pressed and not event.echo:
		if event.keycode == KEY_ESCAPE and _lake_preview_active:
			_cancel_lake_fill_preview()
			get_viewport().set_input_as_handled()
			return
		match event.keycode:
			KEY_1:
				_set_active_tool(Tool.RAISE)
			KEY_2:
				_set_active_tool(Tool.LOWER)
			KEY_3:
				_set_active_tool(Tool.WATER_SOURCE)
			KEY_4:
				_set_active_tool(Tool.WATER_SINK)
			KEY_5:
				_set_active_tool(Tool.WATER_LAKE_FILL)
			KEY_6:
				_set_active_tool(Tool.WATER_OPEN_WATER)
			KEY_ESCAPE:
				_set_active_tool(Tool.NONE)
			KEY_N:
				if event.ctrl_pressed:
					menu_new_world()
			KEY_O:
				if event.ctrl_pressed:
					menu_open_world()
			KEY_S:
				if event.ctrl_pressed:
					if event.shift_pressed:
						menu_save_as()
					else:
						menu_save()
	elif event is InputEventMouseButton:
		if event.button_index == MOUSE_BUTTON_LEFT and event.pressed and _is_water_tool(_active_tool):
			if _is_pointer_over_ui():
				return
			_apply_water_tool(event.shift_pressed)
			get_viewport().set_input_as_handled()

func menu_new_world() -> void:
	_ensure_new_world_window()
	_new_world_name_edit.text = _current_world_name
	_new_world_window.popup_centered()
	_new_world_name_edit.grab_focus()
	_new_world_name_edit.select_all()
	_debug_log("open new-world dialog current_name=%s" % _current_world_name)

func menu_open_world() -> void:
	_ensure_worlds_dir()
	_debug_log("open world file dialog dir=%s" % ProjectSettings.globalize_path(WORLDS_DIR))
	var dialog := FileDialog.new()
	dialog.access = FileDialog.ACCESS_FILESYSTEM
	dialog.file_mode = FileDialog.FILE_MODE_OPEN_FILE
	dialog.filters = PackedStringArray(["*.sqlite ; WorldDefinition Files"])
	dialog.current_dir = ProjectSettings.globalize_path(WORLDS_DIR)
	dialog.file_selected.connect(func(path: String): _on_open_world_selected(path, dialog))
	dialog.canceled.connect(dialog.queue_free)
	add_child(dialog)
	dialog.popup_centered(Vector2i(880, 620))

func menu_save() -> void:
	if _current_world_path.is_empty():
		_debug_log("save requested without existing path -> save_as")
		menu_save_as()
		return
	_cancel_lake_fill_preview("", false)
	_debug_log("save world path=%s name=%s" % [_current_world_path, _current_world_name])
	if sim.save_world_definition(_current_world_path, _current_world_name):
		_set_status("Saved world: %s" % _current_world_name)
	else:
		_set_status("Save failed.", true)

func menu_save_as() -> void:
	_ensure_worlds_dir()
	_debug_log("open save-as dialog dir=%s suggested=%s.sqlite" % [
		ProjectSettings.globalize_path(WORLDS_DIR),
		_sanitize_filename(_current_world_name)
	])
	var dialog := FileDialog.new()
	dialog.access = FileDialog.ACCESS_FILESYSTEM
	dialog.file_mode = FileDialog.FILE_MODE_SAVE_FILE
	dialog.filters = PackedStringArray(["*.sqlite ; WorldDefinition Files"])
	dialog.current_dir = ProjectSettings.globalize_path(WORLDS_DIR)
	dialog.current_file = "%s.sqlite" % _sanitize_filename(_current_world_name)
	dialog.file_selected.connect(func(path: String): _on_save_world_selected(path, dialog))
	dialog.canceled.connect(dialog.queue_free)
	add_child(dialog)
	dialog.popup_centered(Vector2i(880, 620))

func _attach_top_menu() -> void:
	if has_node("TopMenu"):
		return
	var top_menu := TopMenu.new()
	top_menu.name = "TopMenu"
	top_menu.scene_kind = TopMenu.SCENE_WORLD_EDITOR
	add_child(top_menu)

func _configure_editor_camera() -> void:
	editor_camera_input.panel_left_w = 0.0
	editor_camera_input.panel_right_w = 0.0
	editor_camera_input.panel_top_h = float(TopMenu.BAR_HEIGHT)
	editor_camera_input.panel_bot_h = UIStyle.HUD_STRIP_HEIGHT + UIStyle.HUD_BOTTOM_MARGIN + 24.0
	_debug_log(
		"camera bounds top_h=%.1f bottom_h=%.1f" % [
			editor_camera_input.panel_top_h,
			editor_camera_input.panel_bot_h
		]
	)

func _build_ui() -> void:
	var canvas := CanvasLayer.new()
	add_child(canvas)

	var root := Control.new()
	root.set_anchors_and_offsets_preset(Control.PRESET_FULL_RECT)
	root.mouse_filter = Control.MOUSE_FILTER_IGNORE
	canvas.add_child(root)

	var bottom_margin := MarginContainer.new()
	bottom_margin.set_anchors_and_offsets_preset(Control.PRESET_FULL_RECT)
	bottom_margin.mouse_filter = Control.MOUSE_FILTER_IGNORE
	bottom_margin.add_theme_constant_override("margin_bottom", int(UIStyle.HUD_BOTTOM_MARGIN))
	root.add_child(bottom_margin)

	var stack := VBoxContainer.new()
	stack.alignment = BoxContainer.ALIGNMENT_END
	stack.mouse_filter = Control.MOUSE_FILTER_IGNORE
	bottom_margin.add_child(stack)

	_water_tool_panel = PanelContainer.new()
	_water_tool_panel.visible = false
	_water_tool_panel.mouse_filter = Control.MOUSE_FILTER_STOP
	_water_tool_panel.add_theme_stylebox_override("panel", UIStyle.hud_group_style())
	stack.add_child(_water_tool_panel)

	var water_margin := MarginContainer.new()
	water_margin.add_theme_constant_override("margin_left", int(UIStyle.HUD_SHELL_PAD_X))
	water_margin.add_theme_constant_override("margin_right", int(UIStyle.HUD_SHELL_PAD_X))
	water_margin.add_theme_constant_override("margin_top", int(UIStyle.HUD_SHELL_PAD_Y))
	water_margin.add_theme_constant_override("margin_bottom", int(UIStyle.HUD_SHELL_PAD_Y))
	_water_tool_panel.add_child(water_margin)

	var water_row := HBoxContainer.new()
	water_row.alignment = BoxContainer.ALIGNMENT_CENTER
	water_row.add_theme_constant_override("separation", int(UIStyle.HUD_PANEL_GAP))
	water_margin.add_child(water_row)

	_water_source_btn = _make_tool_button("Source", Tool.WATER_SOURCE)
	water_row.add_child(_water_source_btn)

	_water_sink_btn = _make_tool_button("Sink", Tool.WATER_SINK)
	water_row.add_child(_water_sink_btn)

	_water_lake_fill_btn = _make_tool_button("Lake Fill", Tool.WATER_LAKE_FILL)
	water_row.add_child(_water_lake_fill_btn)

	_water_open_water_btn = _make_tool_button("Open Water", Tool.WATER_OPEN_WATER)
	water_row.add_child(_water_open_water_btn)

	var water_separator := VSeparator.new()
	water_row.add_child(water_separator)

	var rate_label := Label.new()
	rate_label.text = "Rate"
	rate_label.add_theme_color_override("font_color", UIStyle.TEXT_PRIMARY)
	water_row.add_child(rate_label)

	_water_rate_spin = _make_spin_box(0.1, 20.0, 0.1, DEFAULT_WATER_RATE)
	_water_rate_spin.custom_minimum_size = Vector2(110.0, UIStyle.HUD_BUTTON_HEIGHT)
	water_row.add_child(_water_rate_spin)

	var surface_label := Label.new()
	surface_label.text = "Surface +m"
	surface_label.add_theme_color_override("font_color", UIStyle.TEXT_PRIMARY)
	water_row.add_child(surface_label)

	_lake_offset_spin = _make_spin_box(0.5, 200.0, 0.5, DEFAULT_LAKE_SURFACE_OFFSET_M)
	_lake_offset_spin.custom_minimum_size = Vector2(110.0, UIStyle.HUD_BUTTON_HEIGHT)
	_lake_offset_spin.value_changed.connect(_on_lake_offset_changed)
	water_row.add_child(_lake_offset_spin)

	var center := CenterContainer.new()
	center.mouse_filter = Control.MOUSE_FILTER_IGNORE
	stack.add_child(center)

	var shell := PanelContainer.new()
	shell.add_theme_stylebox_override("panel", UIStyle.hud_group_style())
	shell.mouse_filter = Control.MOUSE_FILTER_STOP
	center.add_child(shell)

	var shell_margin := MarginContainer.new()
	shell_margin.add_theme_constant_override("margin_left", int(UIStyle.HUD_SHELL_PAD_X))
	shell_margin.add_theme_constant_override("margin_right", int(UIStyle.HUD_SHELL_PAD_X))
	shell_margin.add_theme_constant_override("margin_top", int(UIStyle.HUD_SHELL_PAD_Y))
	shell_margin.add_theme_constant_override("margin_bottom", int(UIStyle.HUD_SHELL_PAD_Y))
	shell.add_child(shell_margin)

	var row := HBoxContainer.new()
	row.alignment = BoxContainer.ALIGNMENT_CENTER
	row.add_theme_constant_override("separation", int(UIStyle.HUD_PANEL_GAP))
	shell_margin.add_child(row)

	_raise_btn = _make_tool_button("Raise", Tool.RAISE)
	row.add_child(_raise_btn)

	_lower_btn = _make_tool_button("Lower", Tool.LOWER)
	row.add_child(_lower_btn)

	_water_group_btn = Button.new()
	_water_group_btn.text = "Water"
	_water_group_btn.toggle_mode = true
	_water_group_btn.custom_minimum_size = Vector2(120.0, UIStyle.HUD_BUTTON_HEIGHT)
	_water_group_btn.pressed.connect(_toggle_water_group)
	row.add_child(_water_group_btn)

	var separator := VSeparator.new()
	row.add_child(separator)

	_toolbar_status = Label.new()
	_toolbar_status.add_theme_color_override("font_color", UIStyle.TEXT_PRIMARY)
	_toolbar_status.vertical_alignment = VERTICAL_ALIGNMENT_CENTER
	_toolbar_status.text = "Terrain world ready."
	row.add_child(_toolbar_status)

func _make_tool_button(label: String, tool: Tool) -> Button:
	var button := Button.new()
	button.text = label
	button.toggle_mode = true
	button.custom_minimum_size = Vector2(120.0, UIStyle.HUD_BUTTON_HEIGHT)
	button.pressed.connect(func(): _toggle_tool(tool))
	return button

func _toggle_tool(tool: Tool) -> void:
	if _active_tool == tool:
		_set_active_tool(Tool.NONE)
	else:
		_set_active_tool(tool)

func _toggle_water_group() -> void:
	if _is_water_tool(_active_tool):
		_set_active_tool(Tool.NONE)
	else:
		_set_active_tool(Tool.WATER_SOURCE)

func _set_active_tool(tool: Tool) -> void:
	var previous_tool := _active_tool
	if _is_surface_fill_tool(previous_tool) and tool != previous_tool:
		_cancel_lake_fill_preview("", false)
	_active_tool = tool
	_update_tool_buttons()
	match _active_tool:
		Tool.RAISE:
			_set_status("Raise terrain tool active.")
		Tool.LOWER:
			_set_status("Lower terrain tool active.")
		Tool.WATER_SOURCE:
			_set_status("Water source tool active. Shift+Click removes nearest source.")
		Tool.WATER_SINK:
			_set_status("Water sink tool active. Shift+Click removes nearest sink.")
		Tool.WATER_LAKE_FILL:
			if _lake_preview_active:
				_set_status(
					"Lake preview active. Adjust Surface +m, click again to confirm, Esc to cancel.",
					not _lake_preview_valid
				)
			else:
				_set_status("Lake fill tool active. Click to preview, click again to confirm, Shift+Click removes nearest lake fill.")
		Tool.WATER_OPEN_WATER:
			if _lake_preview_active:
				_set_status(
					"Open water preview active. Adjust Surface +m, click again to confirm, Esc to cancel.",
					not _lake_preview_valid
				)
			else:
				_set_status("Open water tool active. Click to preview, click again to confirm, Shift+Click removes nearest open water fill.")
		_:
			_set_status("No active world-authoring tool.")
	_debug_log("active_tool=%s" % Tool.keys()[_active_tool])

func _update_tool_buttons() -> void:
	if _raise_btn:
		_raise_btn.button_pressed = _active_tool == Tool.RAISE
	if _lower_btn:
		_lower_btn.button_pressed = _active_tool == Tool.LOWER
	if _water_group_btn:
		_water_group_btn.button_pressed = _is_water_tool(_active_tool)
	if _water_source_btn:
		_water_source_btn.button_pressed = _active_tool == Tool.WATER_SOURCE
	if _water_sink_btn:
		_water_sink_btn.button_pressed = _active_tool == Tool.WATER_SINK
	if _water_lake_fill_btn:
		_water_lake_fill_btn.button_pressed = _active_tool == Tool.WATER_LAKE_FILL
	if _water_open_water_btn:
		_water_open_water_btn.button_pressed = _active_tool == Tool.WATER_OPEN_WATER
	if _water_tool_panel:
		_water_tool_panel.visible = _is_water_tool(_active_tool)

func _apply_sculpt(delta: float) -> void:
	var intersection = _terrain_intersection_under_cursor()
	if intersection == null:
		return

	var strength := SCULPT_STRENGTH_PER_SEC * delta
	if _active_tool == Tool.LOWER:
		strength *= -1.0
	sim.sculpt_terrain(Vector2(intersection.x, intersection.z), SCULPT_RADIUS_M, strength)

func _apply_water_tool(remove_mode: bool) -> void:
	var intersection = _terrain_intersection_under_cursor()
	if intersection == null:
		return

	var world_pos := Vector2(intersection.x, intersection.z)
	match _active_tool:
		Tool.WATER_SOURCE:
			if remove_mode:
				if sim.remove_world_water_source_near(world_pos, WATER_REMOVE_RADIUS_M):
					_set_status("Removed nearest water source.")
				else:
					_set_status("No water source found nearby.", true)
			elif sim.add_world_water_source(world_pos, float(_water_rate_spin.value)):
				_set_status("Added water source.")
			else:
				_set_status("Add water source failed.", true)
		Tool.WATER_SINK:
			if remove_mode:
				if sim.remove_world_water_sink_near(world_pos, WATER_REMOVE_RADIUS_M):
					_set_status("Removed nearest water sink.")
				else:
					_set_status("No water sink found nearby.", true)
			elif sim.add_world_water_sink(world_pos, float(_water_rate_spin.value)):
				_set_status("Added water sink.")
			else:
				_set_status("Add water sink failed.", true)
		Tool.WATER_LAKE_FILL:
			if remove_mode:
				if _lake_preview_active:
					_cancel_lake_fill_preview("", false)
				if sim.remove_world_lake_fill_near(world_pos, WATER_REMOVE_RADIUS_M):
					_set_status("Removed nearest lake fill.")
				else:
					_set_status("No lake fill found nearby.", true)
			else:
				if _lake_preview_active:
					if sim.commit_world_lake_fill_preview():
						_clear_lake_fill_preview_state()
						_set_status("Added lake fill.")
					else:
						var preview_state: Dictionary = sim.get_world_lake_fill_preview()
						_consume_lake_fill_preview_state(preview_state, false)
				else:
					var surface_elevation: float = intersection.y + float(_lake_offset_spin.value)
					var preview_state: Dictionary = sim.begin_world_lake_fill_preview(world_pos, surface_elevation)
					_consume_lake_fill_preview_state(preview_state, true)
		Tool.WATER_OPEN_WATER:
			if remove_mode:
				if _lake_preview_active:
					_cancel_lake_fill_preview("", false)
				if sim.remove_world_open_water_fill_near(world_pos, WATER_REMOVE_RADIUS_M):
					_set_status("Removed nearest open water fill.")
				else:
					_set_status("No open water fill found nearby.", true)
			else:
				if _lake_preview_active:
					if sim.commit_world_open_water_fill_preview():
						_clear_lake_fill_preview_state()
						_set_status("Added open water fill.")
					else:
						var preview_state: Dictionary = sim.get_world_open_water_fill_preview()
						_consume_lake_fill_preview_state(preview_state, false)
				else:
					var surface_elevation: float = intersection.y + float(_lake_offset_spin.value)
					var preview_state: Dictionary = sim.begin_world_open_water_fill_preview(world_pos, surface_elevation)
					_consume_lake_fill_preview_state(preview_state, true)

func _terrain_intersection_under_cursor():
	var camera := get_viewport().get_camera_3d()
	if not camera:
		return null

	var mouse_pos := get_viewport().get_mouse_position()
	var ray_origin := camera.project_ray_origin(mouse_pos)
	var ray_dir := camera.project_ray_normal(mouse_pos)
	return sim.intersect_terrain(ray_origin, ray_dir)

func _is_sculpt_tool(tool: Tool) -> bool:
	return tool == Tool.RAISE or tool == Tool.LOWER

func _is_water_tool(tool: Tool) -> bool:
	return (
		tool == Tool.WATER_SOURCE
		or tool == Tool.WATER_SINK
		or tool == Tool.WATER_LAKE_FILL
		or tool == Tool.WATER_OPEN_WATER
	)

func _is_surface_fill_tool(tool: Tool) -> bool:
	return tool == Tool.WATER_LAKE_FILL or tool == Tool.WATER_OPEN_WATER

func _refresh_after_world_change(focus_camera: bool) -> void:
	terrain.rebuild_from_simulation_state()
	water.rebuild_from_simulation_state()
	if focus_camera:
		_focus_camera_on_world()
	_update_toolbar_summary()
	var world_size := sim.get_terrain_world_size()
	var dims := sim.get_heightmap_size()
	_debug_log(
		"refresh_world world=%s size_km=(%.2f, %.2f) samples=(%d, %d) focus_camera=%s path=%s" % [
			_current_world_name,
			world_size.x / 1000.0,
			world_size.y / 1000.0,
			int(dims.x),
			int(dims.y),
			str(focus_camera),
			_world_path_label()
		]
	)

func _focus_camera_on_world() -> void:
	var world_size := sim.get_terrain_world_size()
	var center := Vector3(0.0, sim.get_height_at(Vector2.ZERO), 0.0)
	var radius := maxf(world_size.x, world_size.y) * 0.5
	editor_camera_input.focus_on(center, radius)

func _update_toolbar_summary() -> void:
	var world_size := sim.get_terrain_world_size()
	var dims := sim.get_heightmap_size()
	var path_label := _current_world_path.get_file() if not _current_world_path.is_empty() else "unsaved"
	_toolbar_status.text = "%s  |  %.1f km × %.1f km  |  %d × %d samples  |  %s" % [
		_current_world_name,
		world_size.x / 1000.0,
		world_size.y / 1000.0,
		int(dims.x),
		int(dims.y),
		path_label,
	]

func _set_status(message: String, is_error: bool = false) -> void:
	if not _toolbar_status:
		return
	_update_toolbar_summary()
	_toolbar_status.text += "  |  %s" % message
	_toolbar_status.add_theme_color_override(
		"font_color",
		UIStyle.TEXT_ALERT if is_error else UIStyle.TEXT_PRIMARY
	)

func _on_lake_offset_changed(_value: float) -> void:
	if not _lake_preview_active:
		return
	var surface_elevation := _lake_preview_seed_height_m + float(_lake_offset_spin.value)
	var preview_state: Dictionary
	if _lake_preview_kind == "open_water":
		preview_state = sim.update_world_open_water_fill_preview(surface_elevation)
	else:
		preview_state = sim.update_world_lake_fill_preview(surface_elevation)
	_consume_lake_fill_preview_state(preview_state, false)

func _consume_lake_fill_preview_state(preview_state: Dictionary, started_preview: bool) -> void:
	var ok := bool(preview_state.get("ok", false))
	var active := bool(preview_state.get("active", false))
	if not active:
		_clear_lake_fill_preview_state()
		_set_status(str(preview_state.get("message", "Lake fill preview unavailable.")), true)
		return

	_lake_preview_active = true
	_lake_preview_seed_world_pos = Vector2(
		float(preview_state.get("seed_world_x", 0.0)),
		float(preview_state.get("seed_world_z", 0.0))
	)
	_lake_preview_seed_height_m = float(preview_state.get("seed_height_m", 0.0))
	_lake_preview_surface_m = float(preview_state.get("surface_elevation_m", 0.0))
	_lake_preview_valid = bool(preview_state.get("valid", false))
	_lake_preview_status = str(preview_state.get("status", "inactive"))
	_lake_preview_kind = str(preview_state.get("kind", "lake"))

	if not ok:
		_set_status(str(preview_state.get("message", "Lake fill preview failed.")), true)
		return

	if _lake_preview_valid:
		var prefix := (
			"Open water preview ready."
			if _lake_preview_kind == "open_water" and started_preview
			else "Open water preview updated."
			if _lake_preview_kind == "open_water"
			else "Lake preview ready."
			if started_preview
			else "Lake preview updated."
		)
		_set_status(
			"%s Surface %.1f m over %d cells. Adjust Surface +m or click again to confirm." % [
				prefix,
				_lake_preview_surface_m,
				int(preview_state.get("filled_cells", 0))
			]
		)
		return

	match _lake_preview_status:
		"below_seed":
			if _lake_preview_kind == "open_water":
				_set_status("Open water preview is below the seed terrain. Raise Surface +m or press Esc.", true)
			else:
				_set_status("Lake preview is below the seed terrain. Raise Surface +m or press Esc.", true)
		"edge_escape":
			if _lake_preview_kind == "open_water":
				_set_status(
					"Open water preview unexpectedly lost edge connection at %.1f m. Adjust Surface +m or press Esc." % _lake_preview_surface_m,
					true
				)
			else:
				_set_status(
					"Lake preview escapes the basin at %.1f m. Lower Surface +m or press Esc." % _lake_preview_surface_m,
					true
				)
		"not_edge_connected":
			_set_status(
				"Open water preview does not reach the world edge at %.1f m. Raise Surface +m or press Esc." % _lake_preview_surface_m,
				true
			)
		_:
			_set_status(str(preview_state.get("message", "Lake fill preview is not valid.")), true)

func _cancel_lake_fill_preview(status_message: String = "Cancelled surface-fill preview.", update_status: bool = true) -> void:
	if _lake_preview_active:
		if _lake_preview_kind == "open_water":
			sim.cancel_world_open_water_fill_preview()
		else:
			sim.cancel_world_lake_fill_preview()
	_clear_lake_fill_preview_state()
	if update_status:
		_set_status(status_message)

func _clear_lake_fill_preview_state() -> void:
	_lake_preview_active = false
	_lake_preview_seed_world_pos = Vector2.ZERO
	_lake_preview_seed_height_m = 0.0
	_lake_preview_surface_m = 0.0
	_lake_preview_valid = false
	_lake_preview_status = "inactive"
	_lake_preview_kind = "inactive"

func _is_pointer_over_ui() -> bool:
	return get_viewport().gui_get_hovered_control() != null

func _ensure_worlds_dir() -> void:
	DirAccess.make_dir_recursive_absolute(ProjectSettings.globalize_path(WORLDS_DIR))

func _ensure_new_world_window() -> void:
	if _new_world_window:
		return

	_new_world_window = Window.new()
	_new_world_window.title = "New World"
	_new_world_window.size = Vector2i(420, 360)
	_new_world_window.unresizable = false
	_new_world_window.exclusive = false
	_new_world_window.close_requested.connect(_new_world_window.hide)
	add_child(_new_world_window)

	var body := PanelContainer.new()
	body.set_anchors_and_offsets_preset(Control.PRESET_FULL_RECT)
	body.add_theme_stylebox_override("panel", UIStyle.window_body_style())
	_new_world_window.add_child(body)

	var margin := MarginContainer.new()
	margin.set_anchors_and_offsets_preset(Control.PRESET_FULL_RECT)
	margin.add_theme_constant_override("margin_left", UIStyle.PAD_WINDOW)
	margin.add_theme_constant_override("margin_right", UIStyle.PAD_WINDOW)
	margin.add_theme_constant_override("margin_top", UIStyle.PAD_WINDOW)
	margin.add_theme_constant_override("margin_bottom", UIStyle.PAD_WINDOW)
	body.add_child(margin)

	var layout := VBoxContainer.new()
	layout.add_theme_constant_override("separation", 10)
	margin.add_child(layout)

	var intro := Label.new()
	intro.text = "Create a blank world for terrain and water authoring."
	intro.add_theme_color_override("font_color", UIStyle.TEXT_DIM)
	layout.add_child(intro)

	var fields := GridContainer.new()
	fields.columns = 2
	fields.add_theme_constant_override("h_separation", 12)
	fields.add_theme_constant_override("v_separation", 8)
	layout.add_child(fields)

	_new_world_name_edit = LineEdit.new()
	_new_world_name_edit.text = _current_world_name
	_add_labeled_field(fields, "Name", _new_world_name_edit)

	_new_world_width_spin = _make_spin_box(1.0, 500.0, 0.5, DEFAULT_WIDTH_KM)
	_add_labeled_field(fields, "Width (km)", _new_world_width_spin)

	_new_world_height_spin = _make_spin_box(1.0, 500.0, 0.5, DEFAULT_HEIGHT_KM)
	_add_labeled_field(fields, "Height (km)", _new_world_height_spin)

	_new_world_cell_spin = _make_spin_box(1.0, 200.0, 1.0, DEFAULT_TERRAIN_CELL_M)
	_add_labeled_field(fields, "Terrain cell (m)", _new_world_cell_spin)

	_new_world_chunk_spin = _make_spin_box(64.0, 4096.0, 64.0, DEFAULT_TERRAIN_CHUNK_M)
	_add_labeled_field(fields, "Terrain chunk (m)", _new_world_chunk_spin)

	_new_world_base_spin = _make_spin_box(-2000.0, 4000.0, 1.0, DEFAULT_BASE_ELEVATION_M)
	_add_labeled_field(fields, "Base elevation (m)", _new_world_base_spin)

	var buttons := HBoxContainer.new()
	buttons.alignment = BoxContainer.ALIGNMENT_END
	buttons.add_theme_constant_override("separation", 10)
	layout.add_child(buttons)

	var cancel_btn := Button.new()
	cancel_btn.text = "Cancel"
	cancel_btn.pressed.connect(_new_world_window.hide)
	buttons.add_child(cancel_btn)

	var create_btn := Button.new()
	create_btn.text = "Create World"
	create_btn.pressed.connect(_on_new_world_confirmed)
	buttons.add_child(create_btn)

func _add_labeled_field(parent: GridContainer, label_text: String, field: Control) -> void:
	var label := Label.new()
	label.text = label_text
	label.add_theme_color_override("font_color", UIStyle.TEXT_PRIMARY)
	parent.add_child(label)
	parent.add_child(field)

func _make_spin_box(min_value: float, max_value: float, step: float, value: float) -> SpinBox:
	var spin := SpinBox.new()
	spin.min_value = min_value
	spin.max_value = max_value
	spin.step = step
	spin.value = value
	spin.size_flags_horizontal = Control.SIZE_EXPAND_FILL
	return spin

func _on_new_world_confirmed() -> void:
	var world_name := _new_world_name_edit.text.strip_edges()
	if world_name.is_empty():
		world_name = "Untitled World"
	_debug_log(
		"confirm new world name=%s width_km=%.1f height_km=%.1f terrain_cell_m=%.1f terrain_chunk_m=%.1f base_elevation_m=%.1f" % [
			world_name,
			float(_new_world_width_spin.value),
			float(_new_world_height_spin.value),
			float(_new_world_cell_spin.value),
			float(_new_world_chunk_spin.value),
			float(_new_world_base_spin.value)
		]
	)
	_cancel_lake_fill_preview("", false)
	if sim.create_blank_world(
		float(_new_world_width_spin.value) * 1000.0,
		float(_new_world_height_spin.value) * 1000.0,
		float(_new_world_cell_spin.value),
		float(_new_world_chunk_spin.value),
		float(_new_world_base_spin.value)
	):
		_current_world_name = world_name
		_current_world_path = ""
		_new_world_window.hide()
		_refresh_after_world_change(true)
		_set_active_tool(Tool.RAISE)
		_set_status("Created blank world.")
		_debug_log("new world created name=%s" % _current_world_name)
	else:
		_set_status("Create blank world failed.", true)
		_debug_log("new world creation failed name=%s" % world_name)

func _on_open_world_selected(path: String, dialog: FileDialog) -> void:
	dialog.hide()
	dialog.call_deferred("queue_free")
	call_deferred("_finish_open_world_selection", path)

func _on_save_world_selected(path: String, dialog: FileDialog) -> void:
	dialog.hide()
	dialog.call_deferred("queue_free")
	call_deferred("_finish_save_world_selection", path)

func _finish_open_world_selection(path: String) -> void:
	_debug_log("selected world file path=%s" % path)
	_cancel_lake_fill_preview("", false)
	if sim.load_world_definition(path):
		_current_world_path = path
		_current_world_name = path.get_file().get_basename()
		_refresh_after_world_change(true)
		_set_status("Loaded world: %s" % _current_world_name)
		_debug_log("world loaded name=%s path=%s" % [_current_world_name, _current_world_path])
	else:
		_set_status("Load failed.", true)
		_debug_log("world load failed path=%s" % path)

func _finish_save_world_selection(path: String) -> void:
	var world_name := _current_world_name.strip_edges()
	if world_name.is_empty():
		world_name = path.get_file().get_basename()
	_debug_log("selected save path=%s name=%s" % [path, world_name])
	_cancel_lake_fill_preview("", false)
	if sim.save_world_definition(path, world_name):
		_current_world_path = path
		_current_world_name = world_name
		_update_toolbar_summary()
		_set_status("Saved world: %s" % _current_world_name)
		_debug_log("world saved name=%s path=%s" % [_current_world_name, _current_world_path])
	else:
		_set_status("Save failed.", true)
		_debug_log("world save failed path=%s" % path)

func _sanitize_filename(text: String) -> String:
	var lower := text.strip_edges().to_lower().replace(" ", "_")
	var cleaned := ""
	for ch in lower:
		var code := ch.unicode_at(0)
		if (code >= 97 and code <= 122) or (code >= 48 and code <= 57) or ch == "_" or ch == "-":
			cleaned += ch
	if cleaned.is_empty():
		return "world"
	return cleaned

func _world_editor_debug_enabled() -> bool:
	var debug_value := OS.get_environment("METRUM_DEBUG").strip_edges()
	if debug_value.is_empty() or debug_value == "0":
		return false
	var filter := OS.get_environment("METRUM_DEBUG_FILTER").strip_edges().to_lower()
	if filter.is_empty():
		return true
	for entry in filter.split(","):
		if entry.strip_edges() == "world-editor":
			return true
	return false

func _debug_log(message: String) -> void:
	if _debug_enabled:
		print("[DEBUG:world-editor] %s" % message)

func _world_path_label() -> String:
	if _current_world_path.is_empty():
		return "unsaved"
	return _current_world_path
