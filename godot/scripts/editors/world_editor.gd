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
const TERRAIN_RENDER_HEIGHT_SCALE := 20.0
const DEFAULT_LAKE_SURFACE_OFFSET_M := 5.0
const MIN_SCULPT_DIAMETER_CELLS := 6.0
const DEFAULT_SCULPT_DIAMETER_CELLS := 8.0
const DEFAULT_SCULPT_DIAMETER_M := 80.0
const DEFAULT_SCULPT_STRENGTH_PER_SEC := 2.0
const MIN_SCULPT_DIAMETER_M := 5.0
const MAX_SCULPT_DIAMETER_M := 200.0
const MIN_SCULPT_STRENGTH_PER_SEC := 0.1
const MAX_SCULPT_STRENGTH_PER_SEC := 20.0
const LIVE_SCULPT_REFRESH_INTERVAL_SEC := 1.0 / 12.0
const SURFACE_NUDGE_STEP_M := 0.5
const SURFACE_NUDGE_FAST_STEP_M := 5.0
const WORLDS_DIR := "user://worlds"
const WORLD_KEYBOARD_PASSTHROUGH_META := "world_editor_keyboard_passthrough"
const WATER_REMOVE_RADIUS_M := 40.0
const WATER_MARKER_GROUND_OFFSET_M := 0.8
const WATER_FILL_STEM_RADIUS_M := 0.24
const WATER_FILL_DISC_RADIUS_M := 5.0
const WATER_FILL_DISC_THICKNESS_M := 0.42
const WATER_FILL_MIN_STEM_M := 2.0
const WATER_FILL_SEED_RADIUS_M := 0.83
const WATER_LAKE_MARKER_COLOR := Color(0.34, 0.80, 0.92, 0.88)
const WATER_OPEN_WATER_MARKER_COLOR := Color(0.18, 0.48, 0.92, 0.88)
const WATER_PREVIEW_MARKER_COLOR := Color(0.92, 0.97, 1.0, 0.92)
const COAL_OVERLAY_MODE := 4
const DEFAULT_RESOURCE_DIAMETER_M := 120.0
const MIN_RESOURCE_DIAMETER_M := 10.0
const MAX_RESOURCE_DIAMETER_M := 500.0
const DEFAULT_COAL_RICHNESS_PERCENT := 70.0
const SLOPE_GUIDE_START_COLOR := Color(0.98, 0.82, 0.34, 0.96)
const SLOPE_GUIDE_END_COLOR := Color(0.34, 0.90, 1.0, 0.96)
const SLOPE_GUIDE_LINE_COLOR := Color(0.90, 0.95, 1.0, 0.82)
const SLOPE_GUIDE_MARKER_RADIUS_M := 1.6
const SLOPE_GUIDE_STEM_HEIGHT_M := 6.0
const SLOPE_GUIDE_STEM_RADIUS_M := 0.20
const SLOPE_GUIDE_LINE_THICKNESS_M := 0.30
const SLOPE_MIN_ANCHOR_DISTANCE_M := 1.0
const BRUSH_PREVIEW_SHADER := """
shader_type spatial;
render_mode unshaded, cull_disabled;

uniform vec4 fill_color : source_color = vec4(0.20, 0.62, 0.28, 0.10);
uniform vec4 ring_color : source_color = vec4(0.76, 1.0, 0.82, 0.85);

void fragment() {
	vec2 p = UV * 2.0 - 1.0;
	float dist = length(p);
	if (dist > 1.0) {
		discard;
	}
	float fill = 1.0 - smoothstep(0.78, 1.0, dist);
	float ring = smoothstep(0.74, 0.80, dist) * (1.0 - smoothstep(0.94, 1.0, dist));
	float cross_x = (1.0 - smoothstep(0.0, 0.018, abs(p.x))) * (1.0 - smoothstep(0.0, 0.22, abs(p.y)));
	float cross_y = (1.0 - smoothstep(0.0, 0.018, abs(p.y))) * (1.0 - smoothstep(0.0, 0.22, abs(p.x)));
	float accent = max(ring, max(cross_x, cross_y));
	ALBEDO = mix(fill_color.rgb, ring_color.rgb, accent);
	ALPHA = fill * fill_color.a + accent * ring_color.a;
}
"""

enum Tool {
	NONE,
	RAISE,
	LOWER,
	LEVEL,
	SMOOTH,
	SLOPE,
	WATER_LAKE_FILL,
	WATER_OPEN_WATER,
	RESOURCE_COAL,
	RESOURCE_ERASE,
}

@onready var sim: SimulationNode = $SimulationNode
@onready var terrain = $Terrain
@onready var water = $Water
@onready var editor_camera_input = $EditorCameraInput

var _active_tool: Tool = Tool.RAISE
var _current_world_name := "Untitled World"
var _current_world_path := ""

var _toolbar_status: Label
var _raise_btn: Button
var _lower_btn: Button
var _level_btn: Button
var _smooth_btn: Button
var _slope_btn: Button
var _terrain_tool_panel: PanelContainer
var _terrain_tool_title: Label
var _brush_diameter_spin: SpinBox
var _brush_strength_spin: SpinBox
var _water_group_btn: Button
var _water_tool_panel: PanelContainer
var _water_lake_fill_btn: Button
var _water_open_water_btn: Button
var _lake_offset_spin: SpinBox
var _preview_confirm_btn: Button
var _preview_cancel_btn: Button
var _resource_group_btn: Button
var _resource_tool_panel: PanelContainer
var _resource_coal_btn: Button
var _resource_erase_btn: Button
var _resource_diameter_spin: SpinBox
var _resource_richness_spin: SpinBox

var _new_world_window: Window
var _new_world_name_edit: LineEdit
var _new_world_width_spin: SpinBox
var _new_world_height_spin: SpinBox
var _new_world_cell_spin: SpinBox
var _new_world_chunk_spin: SpinBox
var _new_world_base_spin: SpinBox
var _debug_enabled := false
var _brush_preview: MeshInstance3D
var _brush_preview_material: ShaderMaterial
var _water_marker_root: Node3D
var _slope_guide_root: Node3D
var _sculpt_stroke_active := false
var _live_sculpt_refresh_timer := 0.0
var _live_sculpt_visual_pending := false
var _resource_stroke_active := false
var _live_resource_overlay_timer := 0.0
var _live_resource_overlay_pending := false
var _level_target_height_m := 0.0
var _slope_start_world_pos := Vector3.ZERO
var _slope_end_world_pos := Vector3.ZERO
var _slope_has_start := false
var _slope_has_end := false
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
	_ensure_brush_preview()
	_ensure_water_marker_root()
	_ensure_slope_guide_root()
	_refresh_after_world_change(true)
	_update_tool_buttons()
	_set_status("Terrain world ready.")
	_debug_log("ready world=%s path=%s" % [_current_world_name, _world_path_label()])

func _process(delta: float) -> void:
	_update_brush_preview()
	if _resource_stroke_active:
		_process_resource_stroke(delta)
	if not _sculpt_stroke_active:
		_live_sculpt_visual_pending = false
		_live_sculpt_refresh_timer = 0.0
		if not _resource_stroke_active:
			_live_resource_overlay_pending = false
			_live_resource_overlay_timer = 0.0
		return
	_live_sculpt_refresh_timer = max(_live_sculpt_refresh_timer - delta, 0.0)
	if not Input.is_mouse_button_pressed(MOUSE_BUTTON_LEFT):
		_end_sculpt_stroke()
		return
	if _ui_captures_world_pointer_input():
		_flush_live_sculpt_visuals(false)
		return
	if not _is_sculpt_tool(_active_tool):
		_flush_live_sculpt_visuals(false)
		return
	if _is_pointer_over_ui():
		_flush_live_sculpt_visuals(false)
		return
	_apply_sculpt(delta)
	_flush_live_sculpt_visuals(false)

func _unhandled_input(event: InputEvent) -> void:
	if event is InputEventKey and event.pressed and not event.echo:
		if event.keycode == KEY_ESCAPE and _numeric_field_has_focus():
			_clear_numeric_field_focus()
			get_viewport().set_input_as_handled()
			return
		if _ui_captures_world_keyboard_input():
			return
		if event.keycode == KEY_ESCAPE and _lake_preview_active:
			_cancel_lake_fill_preview()
			get_viewport().set_input_as_handled()
			return
		if _lake_preview_active:
			var surface_step := SURFACE_NUDGE_FAST_STEP_M if event.shift_pressed else SURFACE_NUDGE_STEP_M
			match event.keycode:
				KEY_BRACKETLEFT, KEY_MINUS:
					_nudge_surface_offset(-surface_step)
					get_viewport().set_input_as_handled()
					return
				KEY_BRACKETRIGHT, KEY_EQUAL, KEY_PLUS:
					_nudge_surface_offset(surface_step)
					get_viewport().set_input_as_handled()
					return
		match event.keycode:
			KEY_1:
				_set_active_tool(Tool.RAISE)
			KEY_2:
				_set_active_tool(Tool.LOWER)
			KEY_3:
				_set_active_tool(Tool.LEVEL)
			KEY_4:
				_set_active_tool(Tool.SMOOTH)
			KEY_5:
				_set_active_tool(Tool.SLOPE)
			KEY_6:
				_set_active_tool(Tool.WATER_LAKE_FILL)
			KEY_7:
				_set_active_tool(Tool.WATER_OPEN_WATER)
			KEY_8:
				_set_active_tool(Tool.RESOURCE_COAL)
			KEY_9:
				_set_active_tool(Tool.RESOURCE_ERASE)
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
		if event.button_index == MOUSE_BUTTON_LEFT:
			if _is_sculpt_tool(_active_tool):
				if event.pressed:
					if _ui_captures_world_pointer_input():
						return
					if _active_tool == Tool.SLOPE and not _slope_profile_ready():
						_capture_slope_anchor()
						get_viewport().set_input_as_handled()
						return
					_begin_sculpt_stroke()
					if _sculpt_stroke_active:
						get_viewport().set_input_as_handled()
				else:
					_end_sculpt_stroke()
				return
			if event.pressed and _is_water_tool(_active_tool):
				if _ui_captures_world_pointer_input():
					return
				_apply_water_tool(event.shift_pressed)
				get_viewport().set_input_as_handled()
			if _is_resource_tool(_active_tool):
				if event.pressed:
					if _ui_captures_world_pointer_input():
						return
					_begin_resource_stroke(event.shift_pressed)
					if _resource_stroke_active:
						get_viewport().set_input_as_handled()
				else:
					_end_resource_stroke()
				return

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
	dialog.exclusive = true
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
	dialog.exclusive = true
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

	_terrain_tool_panel = PanelContainer.new()
	_terrain_tool_panel.visible = false
	_terrain_tool_panel.mouse_filter = Control.MOUSE_FILTER_STOP
	_terrain_tool_panel.size_flags_horizontal = Control.SIZE_SHRINK_CENTER
	_terrain_tool_panel.add_theme_stylebox_override("panel", UIStyle.hud_group_style())
	stack.add_child(_terrain_tool_panel)

	var terrain_margin := MarginContainer.new()
	terrain_margin.add_theme_constant_override("margin_left", int(UIStyle.HUD_SHELL_PAD_X))
	terrain_margin.add_theme_constant_override("margin_right", int(UIStyle.HUD_SHELL_PAD_X))
	terrain_margin.add_theme_constant_override("margin_top", int(UIStyle.HUD_SHELL_PAD_Y))
	terrain_margin.add_theme_constant_override("margin_bottom", int(UIStyle.HUD_SHELL_PAD_Y))
	_terrain_tool_panel.add_child(terrain_margin)

	var terrain_row := HBoxContainer.new()
	terrain_row.alignment = BoxContainer.ALIGNMENT_CENTER
	terrain_row.add_theme_constant_override("separation", int(UIStyle.HUD_PANEL_GAP))
	terrain_margin.add_child(terrain_row)

	_terrain_tool_title = Label.new()
	_terrain_tool_title.text = "Raise Brush"
	_terrain_tool_title.add_theme_color_override("font_color", UIStyle.TEXT_PRIMARY)
	terrain_row.add_child(_terrain_tool_title)

	var terrain_separator := VSeparator.new()
	terrain_row.add_child(terrain_separator)

	var diameter_label := Label.new()
	diameter_label.text = "Diameter m"
	diameter_label.add_theme_color_override("font_color", UIStyle.TEXT_PRIMARY)
	terrain_row.add_child(diameter_label)

	_brush_diameter_spin = _make_hud_spin_box(
		MIN_SCULPT_DIAMETER_M,
		MAX_SCULPT_DIAMETER_M,
		1.0,
		DEFAULT_SCULPT_DIAMETER_M
	)
	_brush_diameter_spin.value_changed.connect(_on_brush_control_changed)
	_brush_diameter_spin.focus_exited.connect(_release_brush_field_focus)
	terrain_row.add_child(_brush_diameter_spin)

	var strength_label := Label.new()
	strength_label.text = "Strength"
	strength_label.add_theme_color_override("font_color", UIStyle.TEXT_PRIMARY)
	terrain_row.add_child(strength_label)

	_brush_strength_spin = _make_hud_spin_box(
		MIN_SCULPT_STRENGTH_PER_SEC,
		MAX_SCULPT_STRENGTH_PER_SEC,
		0.1,
		DEFAULT_SCULPT_STRENGTH_PER_SEC
	)
	_brush_strength_spin.value_changed.connect(_on_brush_control_changed)
	_brush_strength_spin.focus_exited.connect(_release_brush_field_focus)
	terrain_row.add_child(_brush_strength_spin)

	_water_tool_panel = PanelContainer.new()
	_water_tool_panel.visible = false
	_water_tool_panel.mouse_filter = Control.MOUSE_FILTER_STOP
	_water_tool_panel.size_flags_horizontal = Control.SIZE_SHRINK_CENTER
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

	_water_lake_fill_btn = _make_tool_button("Lake Fill", Tool.WATER_LAKE_FILL)
	water_row.add_child(_water_lake_fill_btn)

	_water_open_water_btn = _make_tool_button("Open Water", Tool.WATER_OPEN_WATER)
	water_row.add_child(_water_open_water_btn)

	var water_separator := VSeparator.new()
	water_row.add_child(water_separator)

	var surface_label := Label.new()
	surface_label.text = "Surface +m"
	surface_label.add_theme_color_override("font_color", UIStyle.TEXT_PRIMARY)
	water_row.add_child(surface_label)

	_lake_offset_spin = _make_hud_spin_box(0.5, 200.0, 0.5, DEFAULT_LAKE_SURFACE_OFFSET_M)
	_lake_offset_spin.value_changed.connect(_on_lake_offset_changed)
	_lake_offset_spin.focus_exited.connect(_release_surface_field_focus)
	water_row.add_child(_lake_offset_spin)

	var preview_separator := VSeparator.new()
	water_row.add_child(preview_separator)

	_preview_confirm_btn = Button.new()
	_preview_confirm_btn.text = "OK"
	_preview_confirm_btn.focus_mode = Control.FOCUS_NONE
	_preview_confirm_btn.custom_minimum_size = Vector2(88.0, UIStyle.HUD_BUTTON_HEIGHT)
	_preview_confirm_btn.pressed.connect(_confirm_surface_fill_preview)
	water_row.add_child(_preview_confirm_btn)

	_preview_cancel_btn = Button.new()
	_preview_cancel_btn.text = "Cancel"
	_preview_cancel_btn.focus_mode = Control.FOCUS_NONE
	_preview_cancel_btn.custom_minimum_size = Vector2(110.0, UIStyle.HUD_BUTTON_HEIGHT)
	_preview_cancel_btn.pressed.connect(func(): _cancel_lake_fill_preview())
	water_row.add_child(_preview_cancel_btn)

	_resource_tool_panel = PanelContainer.new()
	_resource_tool_panel.visible = false
	_resource_tool_panel.mouse_filter = Control.MOUSE_FILTER_STOP
	_resource_tool_panel.size_flags_horizontal = Control.SIZE_SHRINK_CENTER
	_resource_tool_panel.add_theme_stylebox_override("panel", UIStyle.hud_group_style())
	stack.add_child(_resource_tool_panel)

	var resource_margin := MarginContainer.new()
	resource_margin.add_theme_constant_override("margin_left", int(UIStyle.HUD_SHELL_PAD_X))
	resource_margin.add_theme_constant_override("margin_right", int(UIStyle.HUD_SHELL_PAD_X))
	resource_margin.add_theme_constant_override("margin_top", int(UIStyle.HUD_SHELL_PAD_Y))
	resource_margin.add_theme_constant_override("margin_bottom", int(UIStyle.HUD_SHELL_PAD_Y))
	_resource_tool_panel.add_child(resource_margin)

	var resource_row := HBoxContainer.new()
	resource_row.alignment = BoxContainer.ALIGNMENT_CENTER
	resource_row.add_theme_constant_override("separation", int(UIStyle.HUD_PANEL_GAP))
	resource_margin.add_child(resource_row)

	_resource_coal_btn = _make_tool_button("Coal", Tool.RESOURCE_COAL)
	resource_row.add_child(_resource_coal_btn)

	_resource_erase_btn = _make_tool_button("Erase", Tool.RESOURCE_ERASE)
	resource_row.add_child(_resource_erase_btn)

	var resource_separator := VSeparator.new()
	resource_row.add_child(resource_separator)

	var resource_diameter_label := Label.new()
	resource_diameter_label.text = "Diameter m"
	resource_diameter_label.add_theme_color_override("font_color", UIStyle.TEXT_PRIMARY)
	resource_row.add_child(resource_diameter_label)

	_resource_diameter_spin = _make_hud_spin_box(
		MIN_RESOURCE_DIAMETER_M,
		MAX_RESOURCE_DIAMETER_M,
		1.0,
		DEFAULT_RESOURCE_DIAMETER_M
	)
	_resource_diameter_spin.value_changed.connect(_on_resource_control_changed)
	_resource_diameter_spin.focus_exited.connect(_release_resource_field_focus)
	resource_row.add_child(_resource_diameter_spin)

	var richness_label := Label.new()
	richness_label.text = "Richness %"
	richness_label.add_theme_color_override("font_color", UIStyle.TEXT_PRIMARY)
	resource_row.add_child(richness_label)

	_resource_richness_spin = _make_hud_spin_box(1.0, 100.0, 1.0, DEFAULT_COAL_RICHNESS_PERCENT)
	_resource_richness_spin.value_changed.connect(_on_resource_control_changed)
	_resource_richness_spin.focus_exited.connect(_release_resource_field_focus)
	resource_row.add_child(_resource_richness_spin)

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

	_level_btn = _make_tool_button("Level", Tool.LEVEL)
	row.add_child(_level_btn)

	_smooth_btn = _make_tool_button("Smooth", Tool.SMOOTH)
	row.add_child(_smooth_btn)

	_slope_btn = _make_tool_button("Slope", Tool.SLOPE)
	row.add_child(_slope_btn)

	var water_group_separator := VSeparator.new()
	row.add_child(water_group_separator)

	_water_group_btn = Button.new()
	_water_group_btn.text = "Water"
	_water_group_btn.toggle_mode = true
	_water_group_btn.focus_mode = Control.FOCUS_NONE
	_water_group_btn.custom_minimum_size = Vector2(120.0, UIStyle.HUD_BUTTON_HEIGHT)
	_water_group_btn.pressed.connect(_toggle_water_group)
	row.add_child(_water_group_btn)

	var resource_group_separator := VSeparator.new()
	row.add_child(resource_group_separator)

	_resource_group_btn = Button.new()
	_resource_group_btn.text = "Resources"
	_resource_group_btn.toggle_mode = true
	_resource_group_btn.focus_mode = Control.FOCUS_NONE
	_resource_group_btn.custom_minimum_size = Vector2(132.0, UIStyle.HUD_BUTTON_HEIGHT)
	_resource_group_btn.pressed.connect(_toggle_resource_group)
	row.add_child(_resource_group_btn)

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
	button.focus_mode = Control.FOCUS_NONE
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
		_set_active_tool(Tool.WATER_LAKE_FILL)

func _toggle_resource_group() -> void:
	if _is_resource_tool(_active_tool):
		_set_active_tool(Tool.NONE)
	else:
		_set_active_tool(Tool.RESOURCE_COAL)

func _set_active_tool(tool: Tool) -> void:
	var previous_tool := _active_tool
	if _is_sculpt_tool(previous_tool) and tool != previous_tool:
		_end_sculpt_stroke()
	if _is_resource_tool(previous_tool) and tool != previous_tool:
		_end_resource_stroke()
	if previous_tool == Tool.SLOPE and tool != previous_tool:
		_clear_slope_profile(false)
	if _is_surface_fill_tool(previous_tool) and tool != previous_tool:
		_cancel_lake_fill_preview("", false)
	_active_tool = tool
	_sync_resource_overlay_for_active_tool()
	_update_tool_buttons()
	match _active_tool:
		Tool.RAISE:
			_set_status("Raise terrain tool active.")
		Tool.LOWER:
			_set_status("Lower terrain tool active.")
		Tool.LEVEL:
			_set_status("Level terrain tool active. Click-drag to level toward the clicked height.")
		Tool.SMOOTH:
			_set_status("Smooth terrain tool active. Click-drag to relax terrain toward the local average.")
		Tool.SLOPE:
			if _slope_profile_ready():
				_set_status("Slope tool active. Brush to apply the captured grade.")
			elif _slope_has_start:
				_set_status("Slope start captured. Click second point to define the slope.")
			else:
				_set_status("Slope tool active. Click first point, then second point, then brush the slope.")
		Tool.WATER_LAKE_FILL:
			if _lake_preview_active:
				_set_status(
					"Lake preview active. Adjust Surface +m, press OK to confirm, Esc to cancel.",
					not _lake_preview_valid
				)
			else:
				_set_status("Lake fill tool active. Click to preview, press OK to confirm, Shift+Click removes nearest lake fill.")
		Tool.WATER_OPEN_WATER:
			if _lake_preview_active:
				_set_status(
					"Open water preview active. Adjust Surface +m, press OK to confirm, Esc to cancel.",
					not _lake_preview_valid
				)
			else:
				_set_status("Open water tool active. Click to preview, press OK to confirm, Shift+Click removes nearest open water fill.")
		Tool.RESOURCE_COAL:
			_set_status("Coal deposit brush active. Paint authored coal richness onto the world.")
		Tool.RESOURCE_ERASE:
			_set_status("Coal deposit eraser active. Paint to clear authored coal.")
		_:
			_set_status("No active world-authoring tool.")
	_debug_log("active_tool=%s" % Tool.keys()[_active_tool])

func _update_tool_buttons() -> void:
	if _raise_btn:
		_raise_btn.button_pressed = _active_tool == Tool.RAISE
	if _lower_btn:
		_lower_btn.button_pressed = _active_tool == Tool.LOWER
	if _level_btn:
		_level_btn.button_pressed = _active_tool == Tool.LEVEL
	if _smooth_btn:
		_smooth_btn.button_pressed = _active_tool == Tool.SMOOTH
	if _slope_btn:
		_slope_btn.button_pressed = _active_tool == Tool.SLOPE
	if _terrain_tool_panel:
		_terrain_tool_panel.visible = _is_sculpt_tool(_active_tool)
	if _terrain_tool_title:
		match _active_tool:
			Tool.RAISE:
				_terrain_tool_title.text = "Raise Brush"
			Tool.LOWER:
				_terrain_tool_title.text = "Lower Brush"
			Tool.LEVEL:
				_terrain_tool_title.text = "Level Brush"
			Tool.SMOOTH:
				_terrain_tool_title.text = "Smooth Brush"
			Tool.SLOPE:
				_terrain_tool_title.text = "Slope Brush"
			_:
				_terrain_tool_title.text = "Terrain Brush"
	if _water_group_btn:
		_water_group_btn.button_pressed = _is_water_tool(_active_tool)
	if _water_lake_fill_btn:
		_water_lake_fill_btn.button_pressed = _active_tool == Tool.WATER_LAKE_FILL
	if _water_open_water_btn:
		_water_open_water_btn.button_pressed = _active_tool == Tool.WATER_OPEN_WATER
	if _water_tool_panel:
		_water_tool_panel.visible = _is_water_tool(_active_tool)
	if _resource_group_btn:
		_resource_group_btn.button_pressed = _is_resource_tool(_active_tool)
	if _resource_coal_btn:
		_resource_coal_btn.button_pressed = _active_tool == Tool.RESOURCE_COAL
	if _resource_erase_btn:
		_resource_erase_btn.button_pressed = _active_tool == Tool.RESOURCE_ERASE
	if _resource_tool_panel:
		_resource_tool_panel.visible = _is_resource_tool(_active_tool)
	if _resource_richness_spin:
		_resource_richness_spin.editable = _active_tool != Tool.RESOURCE_ERASE
	if _slope_guide_root:
		_slope_guide_root.visible = _active_tool == Tool.SLOPE and (_slope_has_start or _slope_has_end)
	_update_preview_action_buttons()

func _apply_sculpt(delta: float) -> void:
	var intersection = _terrain_intersection_under_cursor()
	if intersection == null:
		return

	if _active_tool == Tool.SLOPE and not _slope_profile_ready():
		return

	var radius := _brush_radius_m()
	var world_pos := Vector2(intersection.x, intersection.z)
	var strength := _brush_strength_per_sec() * delta
	var heightmap_strength := strength / TERRAIN_RENDER_HEIGHT_SCALE
	match _active_tool:
		Tool.RAISE:
			sim.sculpt_terrain_stroke_step(world_pos, radius, heightmap_strength)
		Tool.LOWER:
			sim.sculpt_terrain_stroke_step(world_pos, radius, -heightmap_strength)
		Tool.LEVEL:
			sim.level_terrain_stroke_step(
				world_pos,
				radius,
				_level_target_height_m,
				heightmap_strength
			)
		Tool.SMOOTH:
			sim.smooth_terrain_stroke_step(world_pos, radius, heightmap_strength)
		Tool.SLOPE:
			sim.slope_terrain_stroke_step(
				world_pos,
				radius,
				Vector2(_slope_start_world_pos.x, _slope_start_world_pos.z),
				_slope_start_world_pos.y,
				Vector2(_slope_end_world_pos.x, _slope_end_world_pos.z),
				_slope_end_world_pos.y,
				heightmap_strength
			)
	_live_sculpt_visual_pending = true

func _begin_sculpt_stroke() -> void:
	var intersection = _terrain_intersection_under_cursor()
	if intersection == null:
		_end_sculpt_stroke()
		return
	sim.begin_terrain_stroke()
	_sculpt_stroke_active = true
	if _active_tool == Tool.LEVEL:
		_level_target_height_m = intersection.y
		_set_status("Level target captured at %.1f m." % _level_target_height_m)

func _end_sculpt_stroke() -> void:
	_flush_live_sculpt_visuals(true)
	if _sculpt_stroke_active:
		sim.end_terrain_stroke()
	_sculpt_stroke_active = false

func _flush_live_sculpt_visuals(force: bool) -> void:
	if not _live_sculpt_visual_pending:
		return
	if not force and _live_sculpt_refresh_timer > 0.0:
		return
	if not terrain.update_terrain_visuals():
		return
	_refresh_water_markers()
	_live_sculpt_visual_pending = false
	_live_sculpt_refresh_timer = LIVE_SCULPT_REFRESH_INTERVAL_SEC

func _brush_diameter_m() -> float:
	return float(_brush_diameter_spin.value) if _brush_diameter_spin else DEFAULT_SCULPT_DIAMETER_M

func _brush_radius_m() -> float:
	return _brush_diameter_m() * 0.5

func _brush_strength_per_sec() -> float:
	return float(_brush_strength_spin.value) if _brush_strength_spin else DEFAULT_SCULPT_STRENGTH_PER_SEC

func _sync_brush_diameter_limits() -> void:
	if not _brush_diameter_spin:
		return
	var min_diameter := _minimum_sculpt_diameter_m()
	var max_diameter := maxf(MAX_SCULPT_DIAMETER_M, min_diameter)
	_brush_diameter_spin.max_value = max_diameter
	_brush_diameter_spin.min_value = min_diameter
	if _brush_diameter_spin.value < min_diameter:
		_brush_diameter_spin.value = _default_sculpt_diameter_m()
	elif _brush_diameter_spin.value > max_diameter:
		_brush_diameter_spin.value = max_diameter

func _minimum_sculpt_diameter_m() -> float:
	return maxf(MIN_SCULPT_DIAMETER_M, _current_terrain_cell_m() * MIN_SCULPT_DIAMETER_CELLS)

func _default_sculpt_diameter_m() -> float:
	var min_diameter := _minimum_sculpt_diameter_m()
	return clampf(
		_current_terrain_cell_m() * DEFAULT_SCULPT_DIAMETER_CELLS,
		min_diameter,
		maxf(MAX_SCULPT_DIAMETER_M, min_diameter)
	)

func _current_terrain_cell_m() -> float:
	if terrain:
		var cell_m := float(terrain.terrain_cell_m)
		if cell_m > 0.0:
			return cell_m
	return DEFAULT_TERRAIN_CELL_M

func _on_brush_control_changed(_value: float) -> void:
	_release_brush_field_focus()

func _process_resource_stroke(delta: float) -> void:
	_live_resource_overlay_timer = max(_live_resource_overlay_timer - delta, 0.0)
	if not Input.is_mouse_button_pressed(MOUSE_BUTTON_LEFT):
		_end_resource_stroke()
		return
	if _ui_captures_world_pointer_input():
		_flush_resource_overlay(false)
		return
	if not _is_resource_tool(_active_tool):
		_flush_resource_overlay(false)
		return
	if _is_pointer_over_ui():
		_flush_resource_overlay(false)
		return
	_apply_resource_tool(Input.is_key_pressed(KEY_SHIFT))
	_flush_resource_overlay(false)

func _begin_resource_stroke(erase_override: bool) -> void:
	var intersection = _terrain_intersection_under_cursor()
	if intersection == null:
		_end_resource_stroke()
		return
	_resource_stroke_active = true
	_apply_resource_tool(erase_override)
	_flush_resource_overlay(false)

func _end_resource_stroke() -> void:
	_flush_resource_overlay(true)
	_resource_stroke_active = false

func _apply_resource_tool(erase_override: bool) -> void:
	var intersection = _terrain_intersection_under_cursor()
	if intersection == null:
		return

	var world_pos := Vector2(intersection.x, intersection.z)
	var changed := false
	if _active_tool == Tool.RESOURCE_ERASE or erase_override:
		changed = sim.erase_world_coal_deposit(world_pos, _resource_radius_m())
	else:
		changed = sim.paint_world_coal_deposit(
			world_pos,
			_resource_radius_m(),
			float(_resource_richness_spin.value)
		)
	if changed:
		_live_resource_overlay_pending = true

func _flush_resource_overlay(force: bool) -> void:
	if not _live_resource_overlay_pending:
		return
	if not force and _live_resource_overlay_timer > 0.0:
		return
	_mark_resource_overlay_dirty()
	_live_resource_overlay_pending = false
	_live_resource_overlay_timer = LIVE_SCULPT_REFRESH_INTERVAL_SEC

func _resource_diameter_m() -> float:
	return float(_resource_diameter_spin.value) if _resource_diameter_spin else DEFAULT_RESOURCE_DIAMETER_M

func _resource_radius_m() -> float:
	return _resource_diameter_m() * 0.5

func _on_resource_control_changed(_value: float) -> void:
	_release_resource_field_focus()

func _apply_water_tool(remove_mode: bool) -> void:
	var intersection = _terrain_intersection_under_cursor()
	if intersection == null:
		return

	var world_pos := Vector2(intersection.x, intersection.z)
	match _active_tool:
		Tool.WATER_LAKE_FILL:
			if remove_mode:
				if _lake_preview_active:
					_cancel_lake_fill_preview("", false)
				if sim.remove_world_lake_fill_near(world_pos, WATER_REMOVE_RADIUS_M):
					_refresh_water_markers()
					_set_status("Removed nearest lake fill.")
				else:
					_set_status("No lake fill found nearby.", true)
			else:
				var surface_elevation: float = intersection.y + float(_lake_offset_spin.value)
				var preview_state: Dictionary = sim.begin_world_lake_fill_preview(world_pos, surface_elevation)
				_consume_lake_fill_preview_state(preview_state, true)
		Tool.WATER_OPEN_WATER:
			if remove_mode:
				if _lake_preview_active:
					_cancel_lake_fill_preview("", false)
				if sim.remove_world_open_water_fill_near(world_pos, WATER_REMOVE_RADIUS_M):
					_refresh_water_markers()
					_set_status("Removed nearest open water fill.")
				else:
					_set_status("No open water fill found nearby.", true)
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
	return (
		tool == Tool.RAISE
		or tool == Tool.LOWER
		or tool == Tool.LEVEL
		or tool == Tool.SMOOTH
		or tool == Tool.SLOPE
	)

func _is_water_tool(tool: Tool) -> bool:
	return (
		tool == Tool.WATER_LAKE_FILL
		or tool == Tool.WATER_OPEN_WATER
	)

func _is_resource_tool(tool: Tool) -> bool:
	return tool == Tool.RESOURCE_COAL or tool == Tool.RESOURCE_ERASE

func _is_surface_fill_tool(tool: Tool) -> bool:
	return tool == Tool.WATER_LAKE_FILL or tool == Tool.WATER_OPEN_WATER

func _sync_resource_overlay_for_active_tool() -> void:
	if not terrain:
		return
	if _is_resource_tool(_active_tool):
		terrain.overlay_mode = COAL_OVERLAY_MODE
		_mark_resource_overlay_dirty()
	elif terrain.overlay_mode == COAL_OVERLAY_MODE:
		terrain.overlay_mode = 0
		_mark_resource_overlay_dirty()

func _mark_resource_overlay_dirty() -> void:
	if not terrain:
		return
	if terrain.has_method("mark_overlay_dirty"):
		terrain.mark_overlay_dirty()

func _refresh_after_world_change(focus_camera: bool) -> void:
	_clear_slope_profile(false)
	terrain.rebuild_from_simulation_state()
	water.rebuild_from_simulation_state()
	_refresh_water_markers()
	_sync_resource_overlay_for_active_tool()
	if focus_camera:
		_focus_camera_on_world()
	_sync_brush_diameter_limits()
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
	_release_surface_field_focus()

func _nudge_surface_offset(delta_m: float) -> void:
	if not _lake_preview_active:
		return
	var next_value := clampf(
		float(_lake_offset_spin.value) + delta_m,
		_lake_offset_spin.min_value,
		_lake_offset_spin.max_value
	)
	_lake_offset_spin.set_value_no_signal(next_value)
	_on_lake_offset_changed(next_value)

func _release_surface_field_focus() -> void:
	if _lake_offset_spin and _lake_offset_spin.has_focus():
		_lake_offset_spin.release_focus()

func _release_brush_field_focus() -> void:
	if _brush_diameter_spin and _brush_diameter_spin.has_focus():
		_brush_diameter_spin.release_focus()
	if _brush_strength_spin and _brush_strength_spin.has_focus():
		_brush_strength_spin.release_focus()

func _release_resource_field_focus() -> void:
	if _resource_diameter_spin and _resource_diameter_spin.has_focus():
		_resource_diameter_spin.release_focus()
	if _resource_richness_spin and _resource_richness_spin.has_focus():
		_resource_richness_spin.release_focus()

func _clear_numeric_field_focus() -> void:
	var focus_owner := get_viewport().gui_get_focus_owner()
	if focus_owner != null and _control_is_numeric_field(focus_owner):
		focus_owner.release_focus()
	_release_brush_field_focus()
	_release_resource_field_focus()
	_release_surface_field_focus()

func _clear_editor_focus() -> void:
	_clear_numeric_field_focus()
	_release_brush_field_focus()
	_release_resource_field_focus()
	_release_surface_field_focus()
	var focus_owner := get_viewport().gui_get_focus_owner()
	if focus_owner != null:
		focus_owner.release_focus()

func _confirm_surface_fill_preview() -> void:
	if not _lake_preview_active:
		return
	if not _lake_preview_valid:
		_set_status("Surface-fill preview is not valid yet.", true)
		return
	var committed := false
	if _lake_preview_kind == "open_water":
		committed = sim.commit_world_open_water_fill_preview()
	else:
		committed = sim.commit_world_lake_fill_preview()
	if committed:
		var kind_label := "open water fill" if _lake_preview_kind == "open_water" else "lake fill"
		_clear_lake_fill_preview_state()
		_clear_editor_focus()
		_set_status("Added %s." % kind_label)
	else:
		var preview_state: Dictionary = (
			sim.get_world_open_water_fill_preview()
			if _lake_preview_kind == "open_water"
			else sim.get_world_lake_fill_preview()
		)
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
	_refresh_water_markers()

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
			"%s Surface %.1f m over %d cells. Adjust Surface +m and press OK to confirm." % [
				prefix,
				_lake_preview_surface_m,
				int(preview_state.get("filled_cells", 0))
			]
		)
		_update_preview_action_buttons()
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
	_update_preview_action_buttons()

func _cancel_lake_fill_preview(status_message: String = "Cancelled surface-fill preview.", update_status: bool = true) -> void:
	if _lake_preview_active:
		if _lake_preview_kind == "open_water":
			sim.cancel_world_open_water_fill_preview()
		else:
			sim.cancel_world_lake_fill_preview()
	_clear_lake_fill_preview_state()
	_clear_editor_focus()
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
	_refresh_water_markers()
	_update_preview_action_buttons()

func _update_preview_action_buttons() -> void:
	var surface_tool_active := _is_surface_fill_tool(_active_tool)
	if _preview_confirm_btn:
		_preview_confirm_btn.visible = surface_tool_active
		_preview_confirm_btn.disabled = not (_lake_preview_active and _lake_preview_valid)
	if _preview_cancel_btn:
		_preview_cancel_btn.visible = surface_tool_active
		_preview_cancel_btn.disabled = not _lake_preview_active

func _ensure_brush_preview() -> void:
	if _brush_preview:
		return
	_brush_preview = MeshInstance3D.new()
	_brush_preview.name = "BrushPreview"
	_brush_preview.cast_shadow = GeometryInstance3D.SHADOW_CASTING_SETTING_OFF
	_brush_preview.visible = false
	_brush_preview.rotation.x = -PI * 0.5

	var mesh := QuadMesh.new()
	mesh.size = Vector2.ONE
	_brush_preview.mesh = mesh

	_brush_preview_material = ShaderMaterial.new()
	var shader := Shader.new()
	shader.code = BRUSH_PREVIEW_SHADER
	_brush_preview_material.shader = shader
	_brush_preview.material_override = _brush_preview_material
	add_child(_brush_preview)

func _ensure_water_marker_root() -> void:
	if _water_marker_root:
		return
	_water_marker_root = Node3D.new()
	_water_marker_root.name = "WorldWaterMarkers"
	add_child(_water_marker_root)

func _refresh_water_markers() -> void:
	_ensure_water_marker_root()
	for child in _water_marker_root.get_children():
		child.free()

	var committed_markers: Array = sim.get_world_water_authoring_markers()
	for marker_variant in committed_markers:
		var marker: Dictionary = marker_variant
		_add_committed_water_marker(marker)

	if _lake_preview_active:
		_add_preview_water_marker()

func _add_committed_water_marker(marker: Dictionary) -> void:
	var kind := str(marker.get("kind", ""))
	var world_x := float(marker.get("world_x", 0.0))
	var world_z := float(marker.get("world_z", 0.0))
	var terrain_height_m := float(marker.get("terrain_height_m", 0.0))
	match kind:
		"lake_fill":
			_add_water_fill_marker(
				world_x,
				terrain_height_m,
				world_z,
				float(marker.get("surface_elevation_m", terrain_height_m)),
				WATER_LAKE_MARKER_COLOR,
				false
			)
		"open_water_fill":
			_add_water_fill_marker(
				world_x,
				terrain_height_m,
				world_z,
				float(marker.get("surface_elevation_m", terrain_height_m)),
				WATER_OPEN_WATER_MARKER_COLOR,
				false
			)

func _add_preview_water_marker() -> void:
	var color := WATER_PREVIEW_MARKER_COLOR if _lake_preview_valid else UIStyle.TEXT_ALERT
	_add_water_fill_marker(
		_lake_preview_seed_world_pos.x,
		_lake_preview_seed_height_m,
		_lake_preview_seed_world_pos.y,
		_lake_preview_surface_m,
		color,
		true
	)

func _add_water_fill_marker(
	world_x: float,
	terrain_height_m: float,
	world_z: float,
	surface_elevation_m: float,
	color: Color,
	preview: bool
) -> void:
	var root := Node3D.new()
	root.position = Vector3(world_x, 0.0, world_z)
	_water_marker_root.add_child(root)

	var terrain_y := terrain_height_m + WATER_MARKER_GROUND_OFFSET_M
	var surface_y := surface_elevation_m + WATER_MARKER_GROUND_OFFSET_M
	var stem_height := maxf(absf(surface_y - terrain_y), WATER_FILL_MIN_STEM_M)
	var stem_center_y := (terrain_y + surface_y) * 0.5
	var stem := MeshInstance3D.new()
	stem.cast_shadow = GeometryInstance3D.SHADOW_CASTING_SETTING_OFF
	var stem_mesh := CylinderMesh.new()
	stem_mesh.top_radius = WATER_FILL_STEM_RADIUS_M
	stem_mesh.bottom_radius = WATER_FILL_STEM_RADIUS_M
	stem_mesh.height = stem_height
	stem.mesh = stem_mesh
	stem.position = Vector3(0.0, stem_center_y, 0.0)
	stem.material_override = _make_water_marker_material(
		color.darkened(0.12),
		0.88 if preview else 0.78,
		2.2 if preview else 1.6
	)
	root.add_child(stem)

	var cap := MeshInstance3D.new()
	cap.cast_shadow = GeometryInstance3D.SHADOW_CASTING_SETTING_OFF
	var cap_mesh := CylinderMesh.new()
	cap_mesh.top_radius = WATER_FILL_DISC_RADIUS_M
	cap_mesh.bottom_radius = WATER_FILL_DISC_RADIUS_M
	cap_mesh.height = WATER_FILL_DISC_THICKNESS_M
	cap.mesh = cap_mesh
	cap.position = Vector3(0.0, surface_y, 0.0)
	cap.material_override = _make_water_marker_material(
		color,
		0.58 if preview else 0.44,
		3.0 if preview else 2.2
	)
	root.add_child(cap)

	var seed := MeshInstance3D.new()
	seed.cast_shadow = GeometryInstance3D.SHADOW_CASTING_SETTING_OFF
	var seed_mesh := SphereMesh.new()
	seed_mesh.radius = WATER_FILL_SEED_RADIUS_M
	seed_mesh.height = WATER_FILL_SEED_RADIUS_M * 2.0
	seed.mesh = seed_mesh
	seed.position = Vector3(0.0, terrain_y, 0.0)
	seed.material_override = _make_water_marker_material(color.lightened(0.08), 1.0, 3.8)
	root.add_child(seed)

func _make_water_marker_material(color: Color, alpha: float, emission_energy: float) -> StandardMaterial3D:
	var material := StandardMaterial3D.new()
	var shaded_color := color
	shaded_color.a = alpha
	material.albedo_color = shaded_color
	material.transparency = StandardMaterial3D.TRANSPARENCY_ALPHA
	material.shading_mode = StandardMaterial3D.SHADING_MODE_UNSHADED
	material.cull_mode = StandardMaterial3D.CULL_DISABLED
	material.emission_enabled = true
	material.emission = color
	material.emission_energy_multiplier = emission_energy
	return material

func _slope_profile_ready() -> bool:
	return _slope_has_start and _slope_has_end

func _capture_slope_anchor() -> void:
	var intersection = _terrain_intersection_under_cursor()
	if intersection == null:
		return

	var world_pos := Vector3(intersection.x, intersection.y, intersection.z)
	if not _slope_has_start:
		_slope_start_world_pos = world_pos
		_slope_end_world_pos = Vector3.ZERO
		_slope_has_start = true
		_slope_has_end = false
		_refresh_slope_guide()
		_set_status("Slope start captured at %.1f m. Click the end point." % world_pos.y)
		return

	if world_pos.distance_to(_slope_start_world_pos) < SLOPE_MIN_ANCHOR_DISTANCE_M:
		_set_status("Slope end point is too close to the start point.", true)
		return

	_slope_end_world_pos = world_pos
	_slope_has_end = true
	_refresh_slope_guide()
	_set_status(
		"Slope ready from %.1f m to %.1f m. Brush to apply the grade." % [
			_slope_start_world_pos.y,
			_slope_end_world_pos.y
		]
	)

func _clear_slope_profile(update_status: bool = true) -> void:
	_slope_has_start = false
	_slope_has_end = false
	_slope_start_world_pos = Vector3.ZERO
	_slope_end_world_pos = Vector3.ZERO
	_refresh_slope_guide()
	if update_status and _active_tool == Tool.SLOPE:
		_set_status("Slope tool active. Click first point, then second point, then brush the slope.")

func _ensure_slope_guide_root() -> void:
	if _slope_guide_root:
		return
	_slope_guide_root = Node3D.new()
	_slope_guide_root.name = "SlopeGuide"
	add_child(_slope_guide_root)

func _refresh_slope_guide() -> void:
	_ensure_slope_guide_root()
	for child in _slope_guide_root.get_children():
		child.free()

	if not _slope_has_start:
		_slope_guide_root.visible = false
		return

	var start_tip := _add_slope_guide_marker(_slope_start_world_pos, SLOPE_GUIDE_START_COLOR)
	if _slope_has_end:
		var end_tip := _add_slope_guide_marker(_slope_end_world_pos, SLOPE_GUIDE_END_COLOR)
		_add_slope_guide_line(start_tip, end_tip)

	_slope_guide_root.visible = _active_tool == Tool.SLOPE

func _add_slope_guide_marker(world_pos: Vector3, color: Color) -> Vector3:
	var root := Node3D.new()
	root.position = world_pos
	_slope_guide_root.add_child(root)

	var stem := MeshInstance3D.new()
	stem.cast_shadow = GeometryInstance3D.SHADOW_CASTING_SETTING_OFF
	var stem_mesh := CylinderMesh.new()
	stem_mesh.top_radius = SLOPE_GUIDE_STEM_RADIUS_M
	stem_mesh.bottom_radius = SLOPE_GUIDE_STEM_RADIUS_M
	stem_mesh.height = SLOPE_GUIDE_STEM_HEIGHT_M
	stem.mesh = stem_mesh
	stem.position = Vector3(0.0, SLOPE_GUIDE_STEM_HEIGHT_M * 0.5, 0.0)
	stem.material_override = _make_water_marker_material(color.darkened(0.12), 0.98, 2.4)
	root.add_child(stem)

	var cap := MeshInstance3D.new()
	cap.cast_shadow = GeometryInstance3D.SHADOW_CASTING_SETTING_OFF
	var cap_mesh := SphereMesh.new()
	cap_mesh.radius = SLOPE_GUIDE_MARKER_RADIUS_M
	cap_mesh.height = SLOPE_GUIDE_MARKER_RADIUS_M * 2.0
	cap.mesh = cap_mesh
	cap.position = Vector3(0.0, SLOPE_GUIDE_STEM_HEIGHT_M, 0.0)
	cap.material_override = _make_water_marker_material(color, 1.0, 3.6)
	root.add_child(cap)

	return world_pos + Vector3.UP * SLOPE_GUIDE_STEM_HEIGHT_M

func _add_slope_guide_line(start_tip: Vector3, end_tip: Vector3) -> void:
	var delta := end_tip - start_tip
	var length := delta.length()
	if length <= 0.001:
		return

	var direction := delta / length
	var reference := Vector3.UP if absf(direction.dot(Vector3.UP)) < 0.98 else Vector3.FORWARD
	var x_axis := reference.cross(direction).normalized()
	var z_axis := direction.cross(x_axis).normalized()

	var line := MeshInstance3D.new()
	line.cast_shadow = GeometryInstance3D.SHADOW_CASTING_SETTING_OFF
	var line_mesh := BoxMesh.new()
	line_mesh.size = Vector3(
		SLOPE_GUIDE_LINE_THICKNESS_M,
		length,
		SLOPE_GUIDE_LINE_THICKNESS_M
	)
	line.mesh = line_mesh
	line.material_override = _make_water_marker_material(SLOPE_GUIDE_LINE_COLOR, 0.82, 2.0)
	line.transform = Transform3D(
		Basis(x_axis, direction, z_axis),
		start_tip.lerp(end_tip, 0.5)
	)
	_slope_guide_root.add_child(line)

func _update_brush_preview() -> void:
	if not _brush_preview:
		return
	if (not _is_sculpt_tool(_active_tool) and not _is_resource_tool(_active_tool)) or _ui_captures_world_pointer_input():
		_brush_preview.visible = false
		return
	if _active_tool == Tool.SLOPE and not _slope_profile_ready():
		_brush_preview.visible = false
		return
	var intersection = _terrain_intersection_under_cursor()
	if intersection == null:
		_brush_preview.visible = false
		return

	_brush_preview.visible = true
	_brush_preview.position = Vector3(intersection.x, intersection.y + 0.12, intersection.z)
	var preview_mesh := _brush_preview.mesh as QuadMesh
	if preview_mesh:
		var diameter := _resource_diameter_m() if _is_resource_tool(_active_tool) else _brush_diameter_m()
		preview_mesh.size = Vector2(diameter, diameter)
	_brush_preview.scale = Vector3.ONE

	var fill_color := Color(0.20, 0.62, 0.28, 0.10)
	var ring_color := Color(0.76, 1.0, 0.82, 0.85)
	match _active_tool:
		Tool.LOWER:
			fill_color = Color(0.62, 0.22, 0.18, 0.10)
			ring_color = Color(1.0, 0.80, 0.76, 0.85)
		Tool.LEVEL:
			fill_color = Color(0.60, 0.48, 0.18, 0.10)
			ring_color = Color(1.0, 0.94, 0.72, 0.88)
		Tool.SMOOTH:
			fill_color = Color(0.18, 0.44, 0.56, 0.10)
			ring_color = Color(0.72, 0.94, 1.0, 0.88)
		Tool.SLOPE:
			fill_color = Color(0.52, 0.42, 0.18, 0.12)
			ring_color = Color(1.0, 0.94, 0.70, 0.90)
		Tool.RESOURCE_COAL:
			fill_color = Color(0.08, 0.07, 0.06, 0.16)
			ring_color = Color(0.95, 0.79, 0.46, 0.90)
		Tool.RESOURCE_ERASE:
			fill_color = Color(0.42, 0.08, 0.06, 0.12)
			ring_color = Color(1.0, 0.66, 0.58, 0.88)

	_brush_preview_material.set_shader_parameter("fill_color", fill_color)
	_brush_preview_material.set_shader_parameter("ring_color", ring_color)

func _is_pointer_over_ui() -> bool:
	return _ui_captures_world_pointer_input()

func _ui_has_modal_popup() -> bool:
	var viewport := get_viewport()
	var window := viewport as Window
	return (
		window != null
		and window.has_method("has_visible_popup")
		and window.has_visible_popup()
	)

func _ui_captures_world_pointer_input() -> bool:
	var viewport := get_viewport()
	return _ui_has_modal_popup() or viewport.gui_get_hovered_control() != null

func _ui_captures_world_keyboard_input() -> bool:
	var viewport := get_viewport()
	var focus_owner := viewport.gui_get_focus_owner()
	var editing_focus := (
		focus_owner is SpinBox
		or focus_owner is LineEdit
		or focus_owner is TextEdit
		or focus_owner is CodeEdit
	)
	return _ui_has_modal_popup() or editing_focus

func _numeric_field_has_focus() -> bool:
	var focus_owner := get_viewport().gui_get_focus_owner()
	return focus_owner != null and _control_is_numeric_field(focus_owner)

func _control_is_numeric_field(control: Control) -> bool:
	var node: Node = control
	while node != null:
		if node is SpinBox:
			return true
		node = node.get_parent()
	return false

func _ensure_worlds_dir() -> void:
	DirAccess.make_dir_recursive_absolute(ProjectSettings.globalize_path(WORLDS_DIR))

func _ensure_new_world_window() -> void:
	if _new_world_window:
		return

	_new_world_window = Window.new()
	_new_world_window.title = "New World"
	_new_world_window.size = Vector2i(420, 360)
	_new_world_window.unresizable = false
	_new_world_window.exclusive = true
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
	_configure_numeric_spin_box(spin)
	return spin

func _make_hud_spin_box(min_value: float, max_value: float, step: float, value: float) -> SpinBox:
	var spin := _make_spin_box(min_value, max_value, step, value)
	spin.size_flags_horizontal = Control.SIZE_SHRINK_CENTER
	spin.custom_minimum_size = Vector2(96.0, UIStyle.HUD_BUTTON_HEIGHT)
	spin.set_meta(WORLD_KEYBOARD_PASSTHROUGH_META, true)
	var line_edit := spin.get_line_edit()
	if line_edit:
		line_edit.set_meta(WORLD_KEYBOARD_PASSTHROUGH_META, true)
	return spin

func _configure_numeric_spin_box(spin: SpinBox) -> void:
	var line_edit := spin.get_line_edit()
	if line_edit == null:
		return
	var allow_negative := spin.min_value < 0.0
	var allow_decimal := absf(spin.step - roundf(spin.step)) > 0.0001
	line_edit.text_changed.connect(func(new_text: String):
		var sanitized := _sanitize_numeric_text(new_text, allow_negative, allow_decimal)
		if sanitized != new_text:
			line_edit.text = sanitized
	)

func _sanitize_numeric_text(text: String, allow_negative: bool, allow_decimal: bool) -> String:
	var cleaned := ""
	var saw_decimal := false
	var saw_sign := false
	for i in text.length():
		var ch := text.substr(i, 1)
		var code := ch.unicode_at(0)
		if code >= 48 and code <= 57:
			cleaned += ch
			continue
		if allow_negative and not saw_sign and cleaned.is_empty() and ch == "-":
			cleaned += ch
			saw_sign = true
			continue
		if allow_decimal and not saw_decimal and ch == ".":
			if cleaned.is_empty() or cleaned == "-":
				cleaned += "0"
			cleaned += "."
			saw_decimal = true
			continue
	return cleaned

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
