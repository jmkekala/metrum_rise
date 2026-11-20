# SPDX-License-Identifier: GPL-3.0-or-later

## Procedurally built HUD — all UI panels, buttons, and overlays constructed at runtime.
##
## Rust methods called: set_simulation_speed(), undo_action(),
##   get_pollution_image_data(), get_noise_image_data(), get_desirability_image_data(),
##   get_growth_overview(), get_treasury_balance(), get_agent_count(),
##   get_service_building_assets()
##   get_industry_building_assets(), get_road_types()
## No scene file for the UI; every control is created in _ready() and helper functions.
extends CanvasLayer

const TopStatusBar = preload("res://scripts/ui/top_status_bar.gd")
const CityPanel = preload("res://scripts/ui/city_panel.gd")
const ZoningPanel = preload("res://scripts/ui/zoning_panel.gd")
const AssetPanel = preload("res://scripts/ui/asset_panel.gd")
const RoadPropertiesWindow = preload("res://scripts/ui/road_properties_window.gd")
const JunctionWindow = preload("res://scripts/ui/junction_window.gd")
const RoadTypeMenu = preload("res://scripts/ui/road_type_menu.gd")
const RoadBuilderWindow = preload("res://scripts/ui/road_builder/road_builder_window.gd")
const UIStyle = preload("res://scripts/ui/ui_style.gd")
const CheckIcons = preload("res://scripts/ui/check_icons.gd")
const HudIcons = preload("res://scripts/ui/hud_icons.gd")
const RoadOptionsView = preload("res://scripts/ui/road_options_view.gd")

const InputManager = preload("res://scripts/core/input_manager.gd")
const TerrainPanel = preload("res://scripts/ui/terrain_panel.gd")
# The length step offered first: the zoning cell (`WorldConfig`'s zone_cell_m, 10 m).
const LENGTH_STEP_DEFAULT_M := 10.0

@onready var input_manager = $"../InputManager"
@onready var road_tool = $"../RoadTool"
@onready var simulation_node = $"../SimulationNode"

var bottom_panel: Control
var _top_status: TopStatusBar
var _city_panel: CityPanel
var main_toolbar: HBoxContainer

var road_main_btn: Button
var road_sub_menu: HBoxContainer
var road_options_menu: VBoxContainer
var road_combined_hbox: HBoxContainer
var zoning_main_btn: Button

var terrain_main_btn: Button
var terrain_sub_menu: TerrainPanel

# Road items
## The Roads menu's road-type row (`ROAD-101`); its "Road builder..." opens `road_builder`.
var road_type_menu: RoadTypeMenu
var roundabout_btn: Button
var roundabout_row: HBoxContainer
var roundabout_slider: HSlider
var roundabout_size_label: Label
## The road builder window (`ROAD-101`).
var road_builder: RoadBuilderWindow

# Road options
var straight_btn: Button
var curve_btn: Button
var s_curve_btn: Button
var freeform_btn: Button
var continuous_check: Button
var zoning_grid_snap_check: Button
var network_snap_check: Button
var guide_lines_check: Button
var angle_step_option: OptionButton
var length_step_check: Button
var length_step_spin: SpinBox
var straight_grade_check: Button
var zoning_combined_hbox: VBoxContainer
var zoning_mode_menu: HBoxContainer
var zoning_type_menu: HBoxContainer
var zoning_profile_panel: PanelContainer
var zoning_profile_menu: HBoxContainer
var services_main_btn: Button
var services_combined_hbox: VBoxContainer
var service_category_menu: HBoxContainer
var service_asset_panel: PanelContainer
var service_asset_menu: HBoxContainer
var industry_main_btn: Button
var industry_combined_hbox: VBoxContainer
var industry_asset_panel: PanelContainer
var industry_asset_menu: HBoxContainer
var zoning_options_btn: Button
var zoning_options_popup: PopupPanel
var zoning_width_spin: SpinBox
var zoning_depth_spin: SpinBox
var zoning_gap_spin: SpinBox
var _zoning_options_open_on_button_down := false
var _zoning_profiles_by_zone_type: Dictionary = {}
var _zoning_type_buttons: Dictionary = {}
var _zoning_profile_buttons: Dictionary = {}
var _active_zoning_zone_type := ""
var _service_assets_by_class: Dictionary = {}
var _service_class_buttons: Dictionary = {}
var _service_asset_buttons: Dictionary = {}
var _active_service_class := ""
var _industry_assets: Array[Dictionary] = []
var _industry_asset_buttons: Dictionary = {}
var select_main_btn: Button
var bulldoze_btn: Button
var road_properties_panel: Node
var junction_window: Node
var clock_label: Label
var city_status_panel: Control
var demand_meter: Control
var _display_day: int = -1
var _display_minute_of_day: int = -1
var _display_speed: float = -1.0
var _display_treasury: float = INF
var _display_agent_count: int = -1

const ZONING_MAIN_TYPES := [
	{"id": "residential", "label": "Residential"},
	{"id": "commercial", "label": "Commercial"},
	{"id": "industrial", "label": "Industrial"},
]

const ZONING_PARCEL_WIDTH_DEFAULT_CELLS := 2
const ZONING_PARCEL_DEPTH_DEFAULT_CELLS := 2
const ZONING_PARCEL_GAP_DEFAULT_M := 0.0


func _ready():
	CheckIcons.install()
	_build_ui()
	_build_auxiliary_windows()
	_connect_signals()
	_refresh_clock_display(true)
	_refresh_city_status_display()
	set_sim_speed_display(0.0)
	_apply_number_badges()

func _process(_delta):
	_refresh_clock_display(false)
	_refresh_city_status_display()

func _build_ui():
	# Foolproof method for Godot 4 programmatic UI: Full screen root, push to bottom
	var root = Control.new()
	add_child(root)
	root.set_anchors_and_offsets_preset(Control.PRESET_FULL_RECT)
	root.mouse_filter = Control.MOUSE_FILTER_IGNORE

	bottom_panel = Control.new()
	root.add_child(bottom_panel)
	bottom_panel.set_anchors_and_offsets_preset(Control.PRESET_FULL_RECT)
	bottom_panel.mouse_filter = Control.MOUSE_FILTER_IGNORE

	var toolbar_margin := MarginContainer.new()
	toolbar_margin.set_anchors_and_offsets_preset(Control.PRESET_FULL_RECT)
	toolbar_margin.mouse_filter = Control.MOUSE_FILTER_IGNORE
	toolbar_margin.add_theme_constant_override("margin_bottom", int(UIStyle.HUD_BOTTOM_MARGIN))
	bottom_panel.add_child(toolbar_margin)

	var toolbar_stack := VBoxContainer.new()
	toolbar_stack.alignment = BoxContainer.ALIGNMENT_END
	toolbar_stack.mouse_filter = Control.MOUSE_FILTER_IGNORE
	toolbar_margin.add_child(toolbar_stack)

	var toolbar_center := CenterContainer.new()
	toolbar_center.size_flags_horizontal = Control.SIZE_EXPAND_FILL
	toolbar_center.mouse_filter = Control.MOUSE_FILTER_IGNORE
	toolbar_stack.add_child(toolbar_center)

	var main_vbox = VBoxContainer.new()
	main_vbox.mouse_filter = Control.MOUSE_FILTER_IGNORE
	main_vbox.alignment = BoxContainer.ALIGNMENT_END
	
	# VBox to hold the stack of menus (options on top, then sub-menu, then main toolbar)
	var vbox = VBoxContainer.new()
	vbox.mouse_filter = Control.MOUSE_FILTER_IGNORE
	vbox.alignment = BoxContainer.ALIGNMENT_CENTER
	vbox.size_flags_horizontal = Control.SIZE_SHRINK_CENTER
	vbox.add_theme_constant_override("separation", 10)
	main_vbox.add_child(vbox)
	
	# --- Combined Road Sub-Menu Row ---
	road_combined_hbox = HBoxContainer.new()
	road_combined_hbox.alignment = BoxContainer.ALIGNMENT_CENTER
	road_combined_hbox.add_theme_constant_override("separation", 15)
	road_combined_hbox.visible = false
	vbox.add_child(road_combined_hbox)
	
	# Road options: implemented tool modes and snapping controls.
	var road_options := RoadOptionsView.new()
	road_options_menu = road_options
	var options_panel = PanelContainer.new()
	var op_style := UIStyle.submenu_style()
	options_panel.add_theme_stylebox_override("panel", op_style)
	
	var options_padding = MarginContainer.new()
	options_padding.add_theme_constant_override("margin_left", 8)
	options_padding.add_theme_constant_override("margin_right", 8)
	options_padding.add_theme_constant_override("margin_top", 5)
	options_padding.add_theme_constant_override("margin_bottom", 5)
	options_panel.add_child(options_padding)
	options_padding.add_child(road_options_menu)
	
	straight_btn = road_options.straight_btn
	curve_btn = road_options.curve_btn
	s_curve_btn = road_options.s_curve_btn
	freeform_btn = road_options.freeform_btn
	continuous_check = road_options.continuous_btn
	continuous_check.toggled.connect(func(enabled: bool): road_tool.set_option("continuous", enabled))
	_wire_snapping_options(road_options)
	straight_grade_check = road_options.straight_grade_btn
	straight_grade_check.toggled.connect(func(enabled: bool): road_tool.set_straight_grade(enabled))
	if road_tool:
		road_tool.straight_grade_toggled.connect(func(enabled: bool): straight_grade_check.set_pressed_no_signal(enabled))

	# The roundabout tool's size (`ROAD-62`, D12): only while that tool is on.
	roundabout_row = HBoxContainer.new()
	roundabout_row.visible = false
	var diameter_label := Label.new()
	diameter_label.text = "Diameter"
	roundabout_row.add_child(diameter_label)
	roundabout_slider = HSlider.new()
	roundabout_slider.step = 1.0
	roundabout_slider.custom_minimum_size.x = 140.0
	roundabout_slider.size_flags_vertical = Control.SIZE_SHRINK_CENTER
	roundabout_slider.focus_mode = Control.FOCUS_NONE
	roundabout_slider.tooltip_text = "Between the outside kerbs. [ and ] step by 2 m."
	roundabout_row.add_child(roundabout_slider)
	roundabout_size_label = Label.new()
	roundabout_row.add_child(roundabout_size_label)
	road_options_menu.add_child(roundabout_row)
	options_panel.visible = false 
	# Store reference to toggle it
	self.set_meta("options_panel", options_panel)

	var sep = VSeparator.new()
	sep.visible = false
	self.set_meta("road_sep", sep)
	
	# Road type selection
	road_sub_menu = HBoxContainer.new()
	road_sub_menu.add_theme_constant_override("separation", 10)
	var sub_panel = PanelContainer.new()
	var sp_style = op_style.duplicate()
	sub_panel.add_theme_stylebox_override("panel", sp_style)
	
	var sub_padding = MarginContainer.new()
	sub_padding.add_theme_constant_override("margin_left", 8)
	sub_padding.add_theme_constant_override("margin_right", 8)
	sub_padding.add_theme_constant_override("margin_top", 5)
	sub_padding.add_theme_constant_override("margin_bottom", 5)
	
	sub_panel.add_child(sub_padding)
	sub_padding.add_child(road_sub_menu)
	
	# The road types from Rust (`ROAD-101`), the tool's current one marked.
	road_type_menu = RoadTypeMenu.new()
	road_type_menu.type_picked.connect(_select_road_type)
	if road_tool:
		road_tool.road_type_changed.connect(road_type_menu.mark)
	road_type_menu.setup(simulation_node)
	road_sub_menu.add_child(road_type_menu)

	roundabout_btn = HudIcons.icon_button("roundabout", "Roundabout", "A ring on empty ground, or in place of a junction.", HudIcons.TILE_SIZE)
	HudIcons.set_shortcut(roundabout_btn, "O")
	roundabout_btn.toggle_mode = true
	road_sub_menu.add_child(roundabout_btn)
	roundabout_btn.pressed.connect(func(): input_manager._toggle_tool(InputManager.Tool.ROUNDABOUT))
	call_deferred("_connect_roundabout_tool")
	# The select tool edits built roads (type, lanes, junctions), so it lives with the roads.
	select_main_btn = HudIcons.icon_button("edit_road", "Edit roads", "", HudIcons.TILE_SIZE)
	HudIcons.set_shortcut(select_main_btn, "V")
	select_main_btn.toggle_mode = true
	road_sub_menu.add_child(select_main_btn)
	
	road_combined_hbox.add_child(options_panel)
	road_combined_hbox.add_child(sep)
	road_combined_hbox.add_child(sub_panel)
	# Each panel keeps its own height, bottom-aligned, rather than stretching to the taller one.
	options_panel.size_flags_vertical = Control.SIZE_SHRINK_END
	sub_panel.size_flags_vertical = Control.SIZE_SHRINK_END
	
	# --- Terrain palette: sculpting and planting, laid out like Services ---
	var terrain_view := TerrainPanel.new()
	terrain_view.build(input_manager)
	terrain_view.visible = false
	terrain_sub_menu = terrain_view
	vbox.add_child(terrain_view)

	# Palette views own widget construction; selection and tool handlers remain here.
	var zoning_view := ZoningPanel.new()
	zoning_view.build()
	zoning_combined_hbox = zoning_view
	zoning_profile_panel = zoning_view.profile_panel
	zoning_profile_menu = zoning_view.profile_menu
	zoning_mode_menu = zoning_view.mode_menu
	zoning_type_menu = zoning_view.type_menu
	zoning_options_btn = zoning_view.options_button
	zoning_options_btn.button_down.connect(_remember_zoning_options_button_down_state)
	zoning_options_btn.pressed.connect(_toggle_zoning_options_popup)
	_build_zoning_options_popup()
	var cell_controls := zoning_view.cell_controls
	cell_controls.zoning_tool = input_manager.zoning_tool
	cell_controls.workflow_changed.connect(func(cells: bool):
		zoning_options_btn.visible = not cells
		zoning_options_popup.hide()
		_refresh_zoning_profile_button_states()
		_apply_number_badges())
	# Right of the zone types, so the number keys (zones 1-3, then the tools) read left to right.
	zoning_view.row.add_child(HudIcons.shell(cell_controls))
	vbox.add_child(zoning_view)
	zoning_view.hide()
	for entry in ZONING_MAIN_TYPES:
		var zone_type := str(entry.id)
		var button := ZoningPanel.tile(zone_type, str(entry.label), "")
		button.pressed.connect(func(): _toggle_zoning_zone_type(zone_type))
		zoning_type_menu.add_child(button)
		_zoning_type_buttons[zone_type] = button
	_rebuild_zoning_profiles_index()
	_collapse_zoning_profiles()

	var services_view := AssetPanel.new()
	services_view.build(true)
	services_combined_hbox = services_view
	service_asset_panel = services_view.asset_panel
	service_asset_menu = services_view.asset_menu
	service_category_menu = services_view.category_menu
	vbox.add_child(services_view)
	services_view.hide()
	var industry_view := AssetPanel.new()
	industry_view.build(false)
	industry_combined_hbox = industry_view
	industry_asset_panel = industry_view.asset_panel
	industry_asset_menu = industry_view.asset_menu
	vbox.add_child(industry_view)
	industry_view.hide()

	# 1. Main Toolbar (Bottom stack layer)
	main_toolbar = HBoxContainer.new()
	main_toolbar.add_theme_constant_override("separation", 6)
	main_toolbar.size_flags_vertical = Control.SIZE_SHRINK_CENTER
	# Bindings mirror InputManager._unhandled_input; R/Y select tools, not the category panel.
	# Painted icons for the categories; the bulldozer stays the line drawing in the alert red the
	# road builder's remove button uses.
	road_main_btn = HudIcons.dock_button("roads", "Roads", true)
	zoning_main_btn = HudIcons.dock_button("zoning", "Zoning", true)
	services_main_btn = HudIcons.dock_button("services", "Services", true)
	industry_main_btn = HudIcons.dock_button("industry", "Industry", true)
	terrain_main_btn = HudIcons.dock_button("terrain", "Terrain", true)
	bulldoze_btn = HudIcons.dock_button("bulldoze", "Bulldoze", false)
	HudIcons.tint(bulldoze_btn, UIStyle.TEXT_ALERT)
	for entry in [[road_main_btn, "R"], [zoning_main_btn, "Z"], [terrain_main_btn, "Y"], [bulldoze_btn, "B"]]:
		HudIcons.set_shortcut(entry[0], entry[1])
	for button in [road_main_btn, zoning_main_btn, services_main_btn, industry_main_btn, terrain_main_btn]:
		button.toggle_mode = true
		main_toolbar.add_child(button)
	main_toolbar.add_child(VSeparator.new())
	bulldoze_btn.toggle_mode = true
	HudIcons.set_accent(bulldoze_btn, UIStyle.SIGN_RED)
	main_toolbar.add_child(bulldoze_btn)

	# Wrapper to center main toolbar
	var hbox_main_center = HBoxContainer.new()
	hbox_main_center.alignment = BoxContainer.ALIGNMENT_CENTER
	hbox_main_center.size_flags_horizontal = Control.SIZE_EXPAND_FILL
	hbox_main_center.add_child(main_toolbar)

	var category_shell := _create_bottom_strip_shell(hbox_main_center)
	category_shell.size_flags_horizontal = Control.SIZE_SHRINK_CENTER
	vbox.add_child(category_shell)
	var toolbar_shell := _create_bottom_group_shell(main_vbox)
	toolbar_shell.add_to_group("debug_console_keep_clear")
	toolbar_center.add_child(toolbar_shell)

	_top_status = TopStatusBar.new()
	root.add_child(_top_status)
	_top_status.build(input_manager)
	clock_label = _top_status.clock_label
	city_status_panel = _top_status.city_status_panel
	demand_meter = _top_status.demand_meter
	_city_panel = CityPanel.new()
	root.add_child(_city_panel)
	demand_meter.pressed.connect(toggle_city_panel)

func _build_auxiliary_windows() -> void:
	road_properties_panel = RoadPropertiesWindow.new()
	add_child(road_properties_panel)
	if road_properties_panel.has_method("setup"):
		road_properties_panel.setup(simulation_node, input_manager.select_tool)
	junction_window = JunctionWindow.new()
	junction_window.setup(simulation_node, input_manager.select_tool)
	add_child(junction_window)
	road_builder = RoadBuilderWindow.new()
	road_builder.setup(simulation_node)
	add_child(road_builder)
	road_type_menu.road_builder_requested.connect(road_builder.open)
	road_builder.road_types_changed.connect(_on_road_types_changed)
	road_builder.number_key.connect(func(digit: int): handle_number_key(digit))

func _create_bottom_strip_shell(
	content: Control,
	width: float = 0.0,
	shell_style: StyleBox = null
) -> PanelContainer:
	var shell := PanelContainer.new()
	shell.mouse_filter = Control.MOUSE_FILTER_STOP
	shell.add_theme_stylebox_override(
		"panel",
		shell_style if shell_style != null else UIStyle.hud_shell_style()
	)
	shell.custom_minimum_size = Vector2(width, UIStyle.HUD_STRIP_HEIGHT)

	var padding := MarginContainer.new()
	padding.set_anchors_and_offsets_preset(Control.PRESET_FULL_RECT)
	padding.add_theme_constant_override("margin_left", UIStyle.HUD_SHELL_PAD_X)
	padding.add_theme_constant_override("margin_right", UIStyle.HUD_SHELL_PAD_X)
	padding.add_theme_constant_override("margin_top", UIStyle.HUD_SHELL_PAD_Y)
	padding.add_theme_constant_override("margin_bottom", UIStyle.HUD_SHELL_PAD_Y)
	shell.add_child(padding)

	var content_wrapper := VBoxContainer.new()
	content_wrapper.alignment = BoxContainer.ALIGNMENT_CENTER
	content_wrapper.size_flags_horizontal = Control.SIZE_EXPAND_FILL
	content_wrapper.size_flags_vertical = Control.SIZE_EXPAND_FILL
	content_wrapper.mouse_filter = Control.MOUSE_FILTER_IGNORE
	padding.add_child(content_wrapper)

	content.size_flags_horizontal = Control.SIZE_EXPAND_FILL
	content_wrapper.add_child(content)
	return shell

func _create_bottom_group_shell(content: Control) -> PanelContainer:
	var shell := PanelContainer.new()
	shell.mouse_filter = Control.MOUSE_FILTER_IGNORE
	shell.add_theme_stylebox_override("panel", UIStyle.hud_clear_style())

	var padding := MarginContainer.new()
	padding.mouse_filter = Control.MOUSE_FILTER_IGNORE
	padding.add_theme_constant_override("margin_left", UIStyle.HUD_SHELL_PAD_X)
	padding.add_theme_constant_override("margin_right", UIStyle.HUD_SHELL_PAD_X)
	padding.add_theme_constant_override("margin_top", UIStyle.HUD_SHELL_PAD_Y)
	padding.add_theme_constant_override("margin_bottom", UIStyle.HUD_SHELL_PAD_Y)
	shell.add_child(padding)

	content.size_flags_horizontal = Control.SIZE_SHRINK_CENTER
	padding.add_child(content)
	return shell

func _connect_signals():
	input_manager.tool_changed.connect(_refresh_hud_marks)
	input_manager.tool_changed.connect(_follow_tool)
	for panel in [road_combined_hbox, zoning_combined_hbox, services_combined_hbox, industry_combined_hbox, terrain_sub_menu]:
		panel.visibility_changed.connect(_refresh_hud_marks)
	_refresh_hud_marks()
	road_main_btn.pressed.connect(_on_road_main_pressed)
	terrain_main_btn.pressed.connect(_on_terrain_main_pressed)
	zoning_main_btn.pressed.connect(_on_zoning_main_pressed)
	services_main_btn.pressed.connect(_on_services_main_pressed)
	industry_main_btn.pressed.connect(_on_industry_main_pressed)
	select_main_btn.pressed.connect(_on_select_main_pressed)
	bulldoze_btn.pressed.connect(_on_bulldoze_pressed)
	
	straight_btn.pressed.connect(func(): _set_draw_mode(0))
	curve_btn.pressed.connect(func(): _set_draw_mode(1))
	s_curve_btn.pressed.connect(func(): _set_draw_mode(2))
	freeform_btn.pressed.connect(func(): _set_draw_mode(3))

# Both menu visibility and input transitions are event-driven, including keyboard/Esc paths.
func _refresh_hud_marks(_tool: int = 0) -> void:
	var tool: int = input_manager.current_tool
	HudIcons.mark(roundabout_btn, tool == InputManager.Tool.ROUNDABOUT)
	HudIcons.mark(road_main_btn, road_combined_hbox.visible or tool in _ROAD_PANEL_TOOLS)
	HudIcons.mark(zoning_main_btn, zoning_combined_hbox.visible or tool == InputManager.Tool.ZONING)
	HudIcons.mark(services_main_btn, services_combined_hbox.visible or tool == InputManager.Tool.SERVICES)
	HudIcons.mark(industry_main_btn, industry_combined_hbox.visible or tool == InputManager.Tool.INDUSTRY)
	HudIcons.mark(terrain_main_btn, terrain_sub_menu.visible or tool in [InputManager.Tool.SCULPT, InputManager.Tool.VEGETATION])
	HudIcons.mark(select_main_btn, tool == InputManager.Tool.SELECT)
	HudIcons.mark(bulldoze_btn, tool == InputManager.Tool.BULLDOZE)

const _ROAD_PANEL_TOOLS := [InputManager.Tool.ROAD, InputManager.Tool.WALKWAY, InputManager.Tool.ROUNDABOUT, InputManager.Tool.SELECT]
const _TERRAIN_PANEL_TOOLS := [InputManager.Tool.SCULPT, InputManager.Tool.VEGETATION]

# Keyboard tools (R, X, O, V, Y) open their dock panel like a click on the category does,
# and switching the tool off closes the panel it belongs to.
var _last_tool := InputManager.Tool.NONE

func _follow_tool(tool: int) -> void:
	if tool == InputManager.Tool.NONE:
		# Switching tools passes through NONE; only a tool that stays off closes its panel.
		_close_idle_tool_panel.call_deferred(_last_tool)
		_last_tool = InputManager.Tool.NONE
		return
	_last_tool = tool
	if tool in _ROAD_PANEL_TOOLS:
		if not road_combined_hbox.visible:
			_show_only_panel(road_combined_hbox)
		# The drawing options belong to the tools that draw, however they were switched on
		# (R, a tile, a number key); Select draws nothing.
		var draws := tool != InputManager.Tool.SELECT
		get_meta("options_panel").visible = draws
		get_meta("road_sep").visible = draws
	elif tool in _TERRAIN_PANEL_TOOLS and not terrain_sub_menu.visible:
		_show_only_panel(terrain_sub_menu)

func _close_idle_tool_panel(previous: int) -> void:
	if input_manager.current_tool != InputManager.Tool.NONE:
		return
	if previous in _ROAD_PANEL_TOOLS:
		road_combined_hbox.visible = false
		get_meta("options_panel").visible = false
		get_meta("road_sep").visible = false
	elif previous in _TERRAIN_PANEL_TOOLS:
		terrain_sub_menu.visible = false

func _show_only_panel(panel: Control) -> void:
	for other in [road_combined_hbox, terrain_sub_menu, zoning_combined_hbox, services_combined_hbox, industry_combined_hbox]:
		if other != panel:
			other.visible = false
	panel.visible = true

func _on_road_main_pressed():
	terrain_sub_menu.visible = false
	zoning_combined_hbox.visible = false
	services_combined_hbox.visible = false
	industry_combined_hbox.visible = false
	_deactivate_services_if_active()
	_deactivate_industry_if_active()
	road_combined_hbox.visible = !road_combined_hbox.visible
	if not road_combined_hbox.visible:
		get_meta("options_panel").visible = false
		get_meta("road_sep").visible = false
		input_manager._cancel_active_tool()

func _on_terrain_main_pressed():
	road_combined_hbox.visible = false
	zoning_combined_hbox.visible = false
	services_combined_hbox.visible = false
	industry_combined_hbox.visible = false
	_deactivate_services_if_active()
	_deactivate_industry_if_active()
	terrain_sub_menu.visible = !terrain_sub_menu.visible
	if not terrain_sub_menu.visible:
		input_manager._cancel_active_tool()

func _on_zoning_main_pressed():
	road_combined_hbox.visible = false
	terrain_sub_menu.visible = false
	services_combined_hbox.visible = false
	industry_combined_hbox.visible = false
	_deactivate_services_if_active()
	_deactivate_industry_if_active()
	zoning_combined_hbox.visible = !zoning_combined_hbox.visible
	if zoning_combined_hbox.visible:
		_collapse_zoning_profiles()
	else:
		_collapse_zoning_profiles()
	input_manager._toggle_zoning_overlay()

func _on_services_main_pressed():
	road_combined_hbox.visible = false
	terrain_sub_menu.visible = false
	zoning_combined_hbox.visible = false
	industry_combined_hbox.visible = false
	_deactivate_industry_if_active()
	services_combined_hbox.visible = !services_combined_hbox.visible
	if services_combined_hbox.visible:
		_rebuild_service_assets_index()
		_rebuild_service_category_menu()
		var service_classes := _sorted_service_classes()
		if service_classes.is_empty():
			input_manager._cancel_active_tool()
		else:
			_open_service_class(str(service_classes[0]), true)
	else:
		_collapse_service_assets()
		input_manager._cancel_active_tool()

func _on_industry_main_pressed():
	road_combined_hbox.visible = false
	terrain_sub_menu.visible = false
	zoning_combined_hbox.visible = false
	services_combined_hbox.visible = false
	_deactivate_services_if_active()
	industry_combined_hbox.visible = !industry_combined_hbox.visible
	if industry_combined_hbox.visible:
		_rebuild_industry_assets()
		_rebuild_industry_asset_menu()
		if _industry_assets.is_empty():
			input_manager._cancel_active_tool()
		else:
			var first_asset: Dictionary = _industry_assets[0]
			_select_industry_asset(str(first_asset.get("asset_id", "")))
	else:
		input_manager._cancel_active_tool()

# The input manager adds the roundabout tool deferred, as it does the bulldoze tool.
func _connect_roundabout_tool() -> void:
	var tool: Node = input_manager.roundabout_tool
	tool.active_changed.connect(_on_roundabout_active)
	tool.size_changed.connect(_on_roundabout_size)
	roundabout_slider.value_changed.connect(func(value: float): tool.set_diameter(value))

# The tile and the Diameter row follow the tool; `_follow_tool` opens the Roads panel when O
# switches it on with the panel closed.
func _on_roundabout_active(enabled: bool) -> void:
	HudIcons.mark(roundabout_btn, enabled)
	roundabout_row.visible = enabled

func _on_roundabout_size(diameter_m: float, min_m: float, max_m: float, lanes: int) -> void:
	roundabout_slider.set_block_signals(true)
	roundabout_slider.min_value = min_m
	roundabout_slider.max_value = max_m
	roundabout_slider.value = diameter_m
	roundabout_slider.set_block_signals(false)
	roundabout_size_label.text = "%.0f m, %d %s" % [diameter_m, lanes, "lane" if lanes == 1 else "lanes"]

# A road type picked in the Roads menu: the Walkway tool for a walkway, else the Road tool
# (whose activation picks the last type), then the picked type.
func _select_road_type(id: String, walkway: bool) -> void:
	get_meta("options_panel").visible = true
	get_meta("road_sep").visible = true
	var required_tool = InputManager.Tool.WALKWAY if walkway else InputManager.Tool.ROAD
	if input_manager.current_tool != required_tool:
		input_manager._toggle_tool(required_tool)
	if road_tool:
		road_tool.set_road_type(id)

## Lists the road types again after a library edit (`ROAD-101`, the road builder calls it) and
## re-picks the tool's type, so an edited type draws its new layout from the next stroke on.
func refresh_road_types() -> void:
	road_type_menu.refresh()
	_apply_number_badges()
	if road_tool:
		road_tool.reload_road_type()
	if road_properties_panel:
		road_properties_panel.refresh_types()

# The road builder wrote the library: everything listing types refreshes, and a type it just
# created becomes the road tool's, ready to draw.
func _on_road_types_changed(created_id: String) -> void:
	refresh_road_types()
	if not created_id.is_empty():
		road_combined_hbox.visible = true
		_select_road_type(created_id, false)

# The Snapping and Steps controls (`ROAD-112` round E): the session's snapping toggles and steps,
# each sent to Rust by the road tool under its option name. The controls start at the tool's
# values (the session's defaults: network snap and guide lines on, grid, angle lock and length
# step off). Shift frees the cursor of all but the network while held.
func _wire_snapping_options(view: RoadOptionsView) -> void:
	var intent: Dictionary = road_tool.intent() if road_tool else {}
	network_snap_check = view.network_snap_btn
	network_snap_check.button_pressed = intent.get("network_snap", true)
	network_snap_check.toggled.connect(func(enabled: bool): road_tool.set_option("network_snap", enabled))
	guide_lines_check = view.guide_lines_btn
	guide_lines_check.button_pressed = intent.get("guide_lines", true)
	guide_lines_check.toggled.connect(func(enabled: bool): road_tool.set_option("guide_lines", enabled))
	zoning_grid_snap_check = view.snap_btn
	zoning_grid_snap_check.toggled.connect(func(enabled: bool): road_tool.set_option("zoning_grid", enabled))
	angle_step_option = view.angle_step_option
	var steps: Array = RoadOptionsView.ANGLE_STEPS_DEG
	var angle_step := int(intent.get("angle_step_deg", 0.0))
	angle_step_option.select(maxi(steps.find(angle_step), 0))
	angle_step_option.item_selected.connect(func(index: int): road_tool.set_option("angle_step_deg", float(steps[index])))
	length_step_check = view.length_step_btn
	length_step_spin = view.length_step_spin
	var length_step: float = intent.get("length_step_m", 0.0)
	length_step_spin.value = length_step if length_step > 0.0 else LENGTH_STEP_DEFAULT_M
	length_step_spin.editable = length_step > 0.0
	length_step_check.button_pressed = length_step > 0.0
	length_step_check.toggled.connect(func(_enabled: bool): _send_length_step())
	length_step_spin.value_changed.connect(func(_value: float): _send_length_step())

func _send_length_step() -> void:
	length_step_spin.editable = length_step_check.button_pressed
	road_tool.set_option("length_step_m", length_step_spin.value if length_step_check.button_pressed else 0.0)

func _set_draw_mode(mode: int):
	straight_btn.button_pressed = (mode == 0)
	curve_btn.button_pressed = (mode == 1)
	s_curve_btn.button_pressed = (mode == 2)
	freeform_btn.button_pressed = (mode == 3)
	if road_tool:
		road_tool.draw_mode = mode

func _on_select_main_pressed():
	terrain_sub_menu.visible = false
	zoning_combined_hbox.visible = false
	services_combined_hbox.visible = false
	industry_combined_hbox.visible = false
	_deactivate_services_if_active()
	_deactivate_industry_if_active()
	input_manager._toggle_tool(InputManager.Tool.SELECT)

func _on_bulldoze_pressed():
	road_combined_hbox.visible = false
	terrain_sub_menu.visible = false
	zoning_combined_hbox.visible = false
	services_combined_hbox.visible = false
	industry_combined_hbox.visible = false
	_deactivate_services_if_active()
	_deactivate_industry_if_active()
	input_manager._toggle_tool(InputManager.Tool.BULLDOZE)

## Shows the road properties panel for one or more selected edges.
## When multiple edges are selected the warning is suppressed and the
## "No buildings" checkbox reflects whether ALL selected edges have the flag set.
func show_road_properties_multi(edge_indices: Array, screen_pos: Vector2 = Vector2.ZERO):
	if edge_indices.is_empty():
		return
	if road_properties_panel and road_properties_panel.has_method("setup"):
		road_properties_panel.setup(simulation_node, input_manager.select_tool)
	if road_properties_panel and road_properties_panel.has_method("show_for_edges"):
		road_properties_panel.show_for_edges(edge_indices, screen_pos)

func hide_road_properties():
	if road_properties_panel and road_properties_panel.has_method("close_window"):
		road_properties_panel.close_window()

## Shows the Junction window (turn pockets, `ROAD-95`) for junction `node_id`, clicked at
## `screen_pos`.
func show_junction_window(node_id: int, screen_pos: Vector2 = Vector2.ZERO) -> void:
	if junction_window:
		junction_window.show_for_node(node_id, screen_pos)

## Hides the Junction window.
func hide_junction_window() -> void:
	if junction_window:
		junction_window.close_window()

func _build_zoning_options_popup() -> void:
	zoning_options_popup = PopupPanel.new()
	zoning_options_popup.name = "ZoningOptions"
	zoning_options_popup.exclusive = false
	zoning_options_popup.add_theme_stylebox_override(
		"panel",
		UIStyle.panel_style(Color(0.09, 0.09, 0.10, 0.95), UIStyle.CORNER_PANEL, UIStyle.BORDER_ACCENT, 1)
	)
	add_child(zoning_options_popup)

	var margin := MarginContainer.new()
	margin.add_theme_constant_override("margin_left", 12)
	margin.add_theme_constant_override("margin_right", 12)
	margin.add_theme_constant_override("margin_top", 10)
	margin.add_theme_constant_override("margin_bottom", 10)
	zoning_options_popup.add_child(margin)

	var root := VBoxContainer.new()
	root.add_theme_constant_override("separation", 8)
	margin.add_child(root)

	zoning_width_spin = _make_zoning_dimension_spin(ZONING_PARCEL_WIDTH_DEFAULT_CELLS, 1.0, 8.0)
	zoning_depth_spin = _make_zoning_dimension_spin(ZONING_PARCEL_DEPTH_DEFAULT_CELLS, 1.0, 12.0)
	zoning_gap_spin = _make_zoning_dimension_spin(ZONING_PARCEL_GAP_DEFAULT_M, 0.0, 20.0)
	root.add_child(_make_zoning_dimension_row("Width", zoning_width_spin, "cells"))
	root.add_child(_make_zoning_dimension_row("Depth", zoning_depth_spin, "cells"))
	root.add_child(_make_zoning_dimension_row("Gap", zoning_gap_spin, "m"))

	zoning_width_spin.value_changed.connect(func(_value): _on_zoning_parcel_dimensions_changed())
	zoning_depth_spin.value_changed.connect(func(_value): _on_zoning_parcel_dimensions_changed())
	zoning_gap_spin.value_changed.connect(func(_value): _on_zoning_parcel_dimensions_changed())

func _make_zoning_dimension_row(label_text: String, spin: SpinBox, unit_text: String) -> HBoxContainer:
	var row := HBoxContainer.new()
	row.add_theme_constant_override("separation", 8)

	var label := Label.new()
	label.text = label_text
	label.custom_minimum_size = Vector2(58, 0)
	label.vertical_alignment = VERTICAL_ALIGNMENT_CENTER
	row.add_child(label)

	row.add_child(spin)

	var unit := Label.new()
	unit.text = unit_text
	unit.vertical_alignment = VERTICAL_ALIGNMENT_CENTER
	unit.custom_minimum_size = Vector2(38, 0)
	row.add_child(unit)
	return row

func _make_zoning_dimension_spin(value: float, min_value: float, max_value: float) -> SpinBox:
	var spin := SpinBox.new()
	spin.min_value = min_value
	spin.max_value = max_value
	spin.step = 1.0
	spin.value = value
	spin.custom_minimum_size = Vector2(92, 34)
	return spin

func _toggle_zoning_options_popup() -> void:
	if zoning_options_popup == null or zoning_options_btn == null:
		return
	if _zoning_options_open_on_button_down or zoning_options_popup.visible:
		_zoning_options_open_on_button_down = false
		zoning_options_popup.hide()
		return
	_zoning_options_open_on_button_down = false

	var popup_size := Vector2i(250, 154)
	var viewport_size := get_viewport().get_visible_rect().size
	var button_pos := zoning_options_btn.global_position
	var x := clampf(button_pos.x, 12.0, maxf(12.0, viewport_size.x - popup_size.x - 12.0))
	var y := button_pos.y - float(popup_size.y) - 8.0
	y = clampf(y, 40.0, maxf(40.0, viewport_size.y - popup_size.y - 12.0))
	zoning_options_popup.position = Vector2i(int(round(x)), int(round(y)))
	zoning_options_popup.size = popup_size
	zoning_options_popup.popup()

func _remember_zoning_options_button_down_state() -> void:
	_zoning_options_open_on_button_down = zoning_options_popup != null and zoning_options_popup.visible

func _on_zoning_parcel_dimensions_changed() -> void:
	if input_manager == null or zoning_width_spin == null or zoning_depth_spin == null or zoning_gap_spin == null:
		return
	input_manager.set_zoning_parcel_options(
		int(round(zoning_width_spin.value)),
		int(round(zoning_depth_spin.value)),
		float(zoning_gap_spin.value)
	)

func _rebuild_zoning_profiles_index() -> void:
	_zoning_profiles_by_zone_type.clear()
	var payload = simulation_node.get_zone_profiles()
	if not (payload is Array):
		return
	for entry in payload:
		if not (entry is Dictionary):
			continue
		var profile: Dictionary = entry
		var zone_type := str(profile.get("zone_type", "")).strip_edges()
		if zone_type.is_empty():
			continue
		if not _zoning_profiles_by_zone_type.has(zone_type):
			_zoning_profiles_by_zone_type[zone_type] = []
		var zone_profiles: Array = _zoning_profiles_by_zone_type[zone_type]
		zone_profiles.append(profile)
		_zoning_profiles_by_zone_type[zone_type] = zone_profiles

func _toggle_zoning_zone_type(zone_type: String) -> void:
	if zoning_profile_panel.visible and _active_zoning_zone_type == zone_type:
		_collapse_zoning_profiles()
		return
	_open_zoning_zone_type(zone_type, true)

func _open_zoning_zone_type(zone_type: String, auto_select_first_profile: bool = false) -> void:
	if zone_type.is_empty() or not _zoning_profiles_by_zone_type.has(zone_type):
		return

	_active_zoning_zone_type = zone_type
	_rebuild_zoning_profile_menu(zone_type)
	zoning_profile_panel.visible = true
	if auto_select_first_profile:
		var profiles: Array = _zoning_profiles_by_zone_type.get(zone_type, [])
		if not profiles.is_empty():
			var first_profile: Dictionary = profiles[0]
			_select_zoning_profile(zone_type, int(first_profile.get("runtime_id", 0)))
			return
	_refresh_zoning_type_button_states()
	_refresh_zoning_profile_button_states()

func _collapse_zoning_profiles() -> void:
	_active_zoning_zone_type = ""
	if zoning_profile_panel:
		zoning_profile_panel.visible = false
	_refresh_zoning_type_button_states()

func _rebuild_zoning_profile_menu(zone_type: String) -> void:
	while zoning_profile_menu.get_child_count() > 0:
		var child := zoning_profile_menu.get_child(0)
		zoning_profile_menu.remove_child(child)
		child.queue_free()
	_zoning_profile_buttons.clear()

	var profiles: Array = _zoning_profiles_by_zone_type.get(zone_type, [])
	for entry in profiles:
		var profile: Dictionary = entry
		var button := ZoningPanel.tile(zone_type,
			str(profile.get("display_name", zone_type.capitalize())), str(profile.get("ui_description", "")),
			str(profile.get("ui_color", "")), str(profile.get("density", "")))
		var runtime_id := int(profile.get("runtime_id", 0))
		button.pressed.connect(func(): _select_zoning_profile(zone_type, runtime_id))
		zoning_profile_menu.add_child(button)
		_zoning_profile_buttons[runtime_id] = button

	var clear_btn := HudIcons.icon_button("erase", "Free", "", 64)
	clear_btn.toggle_mode = true
	HudIcons.set_accent(clear_btn, UIStyle.SIGN_RED)
	clear_btn.pressed.connect(func(): _select_zoning_profile(zone_type, 0))
	zoning_profile_menu.add_child(clear_btn)
	_zoning_profile_buttons[0] = clear_btn
	clear_btn.visible = not input_manager.zoning_tool.workflow_cells

func _select_zoning_profile(zone_type: String, runtime_id: int) -> void:
	_active_zoning_zone_type = zone_type
	if zoning_profile_panel:
		zoning_profile_panel.visible = true
	input_manager.select_zone_profile(runtime_id)
	_refresh_zoning_type_button_states()
	_refresh_zoning_profile_button_states()

func _refresh_zoning_type_button_states() -> void:
	for zone_type in _zoning_type_buttons.keys():
		var button: Button = _zoning_type_buttons[zone_type]
		button.set_pressed_no_signal(
			zoning_profile_panel.visible and str(zone_type) == _active_zoning_zone_type
		)

func _refresh_zoning_profile_button_states() -> void:
	var current_runtime_id := _current_zoning_profile_runtime_id()
	for runtime_id in _zoning_profile_buttons.keys():
		var button: Button = _zoning_profile_buttons[runtime_id]
		if int(runtime_id) == 0:
			button.visible = not input_manager.zoning_tool.workflow_cells
		button.set_pressed_no_signal(int(runtime_id) == current_runtime_id)

func _current_zoning_profile_runtime_id() -> int:
	if input_manager and input_manager.zoning_tool:
		return int(input_manager.zoning_tool.current_profile_runtime_id)
	return 0

func _rebuild_service_assets_index() -> void:
	_service_assets_by_class.clear()
	var payload = simulation_node.get_service_building_assets()
	if not (payload is Array):
		return
	for entry in payload:
		if not (entry is Dictionary):
			continue
		var asset: Dictionary = entry
		var service_class := str(asset.get("service_class", "")).strip_edges()
		if service_class.is_empty():
			continue
		if not _service_assets_by_class.has(service_class):
			_service_assets_by_class[service_class] = []
		var assets: Array = _service_assets_by_class[service_class]
		assets.append(asset.duplicate(true))
		_service_assets_by_class[service_class] = assets

	for service_class in _service_assets_by_class.keys():
		var assets: Array = _service_assets_by_class[service_class]
		assets.sort_custom(func(a: Dictionary, b: Dictionary) -> bool:
			var a_name := str(a.get("display_name", a.get("asset_id", "")))
			var b_name := str(b.get("display_name", b.get("asset_id", "")))
			if a_name == b_name:
				return str(a.get("asset_id", "")) < str(b.get("asset_id", ""))
			return a_name < b_name
		)
		_service_assets_by_class[service_class] = assets

func _sorted_service_classes() -> Array:
	var classes := _service_assets_by_class.keys()
	classes.sort()
	return classes

func _rebuild_service_category_menu() -> void:
	while service_category_menu.get_child_count() > 0:
		var child := service_category_menu.get_child(0)
		service_category_menu.remove_child(child)
		child.queue_free()
	_service_class_buttons.clear()

	var classes := _sorted_service_classes()
	if classes.is_empty():
		var empty_btn := Button.new()
		empty_btn.text = "No Services"
		empty_btn.custom_minimum_size = Vector2(145, 50)
		empty_btn.disabled = true
		empty_btn.focus_mode = Control.FOCUS_NONE
		service_category_menu.add_child(empty_btn)
		service_asset_panel.visible = false
		return

	for service_class in classes:
		var class_id := str(service_class)
		var button := AssetPanel.class_tile(class_id, _service_class_label(class_id))
		button.pressed.connect(func(): _toggle_service_class(class_id))
		service_category_menu.add_child(button)
		_service_class_buttons[class_id] = button

func _toggle_service_class(service_class: String) -> void:
	if service_asset_panel.visible and _active_service_class == service_class:
		_collapse_service_assets()
		return
	_open_service_class(service_class, true)

func _open_service_class(service_class: String, auto_select_first_asset: bool = false) -> void:
	if service_class.is_empty() or not _service_assets_by_class.has(service_class):
		return
	_active_service_class = service_class
	_rebuild_service_asset_menu(service_class)
	service_asset_panel.visible = true
	if auto_select_first_asset:
		var assets: Array = _service_assets_by_class.get(service_class, [])
		if not assets.is_empty():
			var first_asset: Dictionary = assets[0]
			_select_service_asset(service_class, str(first_asset.get("asset_id", "")))
			return
	_refresh_service_class_button_states()
	_refresh_service_asset_button_states()

func _collapse_service_assets() -> void:
	_active_service_class = ""
	if service_asset_panel:
		service_asset_panel.visible = false
	_refresh_service_class_button_states()

func _rebuild_service_asset_menu(service_class: String) -> void:
	while service_asset_menu.get_child_count() > 0:
		var child := service_asset_menu.get_child(0)
		service_asset_menu.remove_child(child)
		child.queue_free()
	_service_asset_buttons.clear()

	var assets: Array = _service_assets_by_class.get(service_class, [])
	for entry in assets:
		var asset: Dictionary = entry
		var asset_id := str(asset.get("asset_id", ""))
		if asset_id.is_empty():
			continue
		var button := AssetPanel.asset_tile(asset, service_class)
		button.pressed.connect(func(): _select_service_asset(service_class, asset_id))
		service_asset_menu.add_child(button)
		_service_asset_buttons[asset_id] = button

func _select_service_asset(service_class: String, asset_id: String) -> void:
	if asset_id.is_empty():
		return
	_active_service_class = service_class
	if service_asset_panel:
		service_asset_panel.visible = true
	input_manager.select_service_asset(asset_id)
	_refresh_service_class_button_states()
	_refresh_service_asset_button_states()

func _refresh_service_class_button_states() -> void:
	for service_class in _service_class_buttons.keys():
		var button: Button = _service_class_buttons[service_class]
		button.set_pressed_no_signal(
			service_asset_panel.visible and str(service_class) == _active_service_class
		)

func _refresh_service_asset_button_states() -> void:
	var current_asset_id := _current_service_asset_id()
	for asset_id in _service_asset_buttons.keys():
		var button: Button = _service_asset_buttons[asset_id]
		button.set_pressed_no_signal(str(asset_id) == current_asset_id)

func _current_service_asset_id() -> String:
	if input_manager and input_manager.service_building_tool:
		return str(input_manager.service_building_tool.selected_asset_id)
	return ""

func _deactivate_services_if_active() -> void:
	if input_manager and input_manager.current_tool == InputManager.Tool.SERVICES:
		input_manager._cancel_active_tool()

func _rebuild_industry_assets() -> void:
	_industry_assets.clear()
	var payload = simulation_node.get_industry_building_assets()
	if not (payload is Array):
		return
	for entry in payload:
		if entry is Dictionary:
			_industry_assets.append((entry as Dictionary).duplicate(true))
	_industry_assets.sort_custom(func(a: Dictionary, b: Dictionary) -> bool:
		var a_name := str(a.get("display_name", a.get("asset_id", "")))
		var b_name := str(b.get("display_name", b.get("asset_id", "")))
		if a_name == b_name:
			return str(a.get("asset_id", "")) < str(b.get("asset_id", ""))
		return a_name < b_name
	)

func _rebuild_industry_asset_menu() -> void:
	while industry_asset_menu.get_child_count() > 0:
		var child := industry_asset_menu.get_child(0)
		industry_asset_menu.remove_child(child)
		child.queue_free()
	_industry_asset_buttons.clear()

	if _industry_assets.is_empty():
		var empty_btn := Button.new()
		empty_btn.text = "No Industry"
		empty_btn.custom_minimum_size = Vector2(145, 50)
		empty_btn.disabled = true
		empty_btn.focus_mode = Control.FOCUS_NONE
		industry_asset_menu.add_child(empty_btn)
		return

	for asset in _industry_assets:
		var asset_id := str(asset.get("asset_id", ""))
		if asset_id.is_empty():
			continue
		var button := AssetPanel.asset_tile(asset, str(asset.get("area_kind", "")))
		button.pressed.connect(func(): _select_industry_asset(asset_id))
		industry_asset_menu.add_child(button)
		_industry_asset_buttons[asset_id] = button

func _select_industry_asset(asset_id: String) -> void:
	if asset_id.is_empty():
		return
	if industry_combined_hbox:
		industry_combined_hbox.visible = true
	input_manager.select_industry_asset(asset_id, _industry_resource_for_asset(asset_id))
	_refresh_industry_asset_button_states()

func _industry_resource_for_asset(asset_id: String) -> String:
	for asset in _industry_assets:
		if str(asset.get("asset_id", "")) == asset_id:
			return str(asset.get("resource_id", "")).strip_edges()
	return ""

func _refresh_industry_asset_button_states() -> void:
	var current_asset_id := _current_industry_asset_id()
	for asset_id in _industry_asset_buttons.keys():
		var button: Button = _industry_asset_buttons[asset_id]
		button.set_pressed_no_signal(str(asset_id) == current_asset_id)

func _current_industry_asset_id() -> String:
	if input_manager and input_manager.industry_building_tool:
		return str(input_manager.industry_building_tool.selected_asset_id)
	return ""

func _deactivate_industry_if_active() -> void:
	if input_manager and input_manager.current_tool == InputManager.Tool.INDUSTRY:
		input_manager._cancel_active_tool()

func _service_class_label(service_class: String) -> String:
	match service_class:
		"power":
			return "Power"
		"water":
			return "Water"
		"waste":
			return "Waste"
		"sewage":
			return "Sewage"
		_:
			return service_class.replace("_", " ").capitalize()

func _refresh_clock_display(force: bool):
	if not clock_label or not simulation_node:
		return
	var day := int(simulation_node.get_current_day())
	var minute_of_day := int(simulation_node.get_current_minute_of_day())
	if not force and day == _display_day and minute_of_day == _display_minute_of_day:
		return
	_display_day = day
	_display_minute_of_day = minute_of_day
	var hours := minute_of_day / 60
	var minutes := minute_of_day % 60
	clock_label.text = "Day %d %02d:%02d" % [day, hours, minutes]
	_refresh_demand_display()

func _refresh_demand_display() -> void:
	if not demand_meter or not simulation_node:
		return
	var growth: Dictionary = simulation_node.get_growth_overview()
	demand_meter.set_growth(growth)
	if _city_panel and _city_panel.is_open():
		_city_panel.set_growth(growth)

func _refresh_city_status_display() -> void:
	if not city_status_panel or not simulation_node:
		return

	var treasury := float(simulation_node.get_treasury_balance())
	var agent_count := int(simulation_node.get_agent_count())
	if is_equal_approx(treasury, _display_treasury) and agent_count == _display_agent_count:
		return

	_display_treasury = treasury
	_display_agent_count = agent_count
	if city_status_panel.has_method("set_stats"):
		city_status_panel.set_stats(treasury, agent_count)
	if _city_panel and _city_panel.is_open():
		_city_panel.set_stats(treasury, agent_count)

## Tab, or a click on the demand bars: the city status panel with each zone's growth reasons.
# Number keys act on the open palette's tiles, each showing its digit at its bottom edge. While
# Roads, Zoning or Terrain is open it claims 1-9 (unnumbered digits do nothing); with none open the
# digits keep their global bindings (zone types, terrain overlays).
var _numbered: Array[Button] = []

## Runs digit `digit`'s action in the open palette; false when no numbered palette is open.
func handle_number_key(digit: int) -> bool:
	var slots: Array = []
	if road_combined_hbox.visible:
		slots = _road_number_slots()
	elif zoning_combined_hbox.visible:
		slots = _zoning_number_slots()
	elif terrain_sub_menu.visible:
		slots = terrain_sub_menu.number_slots()
	else:
		return false
	if digit >= 1 and digit <= slots.size():
		var action: Callable = slots[digit - 1][1]
		action.call()
	return true

# [tile, action] per digit from 1: built-in types, new custom road (its digit closes the open
# builder again), the custom roads tile (which cycles the player's types), roundabout, edit roads.
func _road_number_slots() -> Array:
	var slots: Array = []
	for tile in road_type_menu.number_tiles():
		var action := HudIcons.activate.bind(tile)
		if tile == road_type_menu.custom_menu:
			action = road_type_menu.pick_next_custom
		elif tile == road_type_menu.builder_tile():
			action = _toggle_road_builder
		slots.append([tile, action])
	slots.append([roundabout_btn, HudIcons.activate.bind(roundabout_btn)])
	slots.append([select_main_btn, HudIcons.activate.bind(select_main_btn)])
	return slots

# Zone types 1-3 (again: the zone's next profile), then the cell tools: the workflow switch,
# the four shapes, erase.
func _zoning_number_slots() -> Array:
	var slots: Array = []
	for entry in ZONING_MAIN_TYPES:
		var zone_type := str(entry.id)
		slots.append([_zoning_type_buttons[zone_type], _number_zone_type.bind(zone_type)])
	for tile in zoning_combined_hbox.cell_controls.number_tiles():
		slots.append([tile, HudIcons.activate.bind(tile)])
	return slots

func _toggle_road_builder() -> void:
	if road_builder and road_builder.is_open():
		road_builder.close()
		HudIcons.pulse(road_type_menu.builder_tile())
	else:
		HudIcons.activate(road_type_menu.builder_tile())

func _number_zone_type(zone_type: String) -> void:
	var profiles: Array = _zoning_profiles_by_zone_type.get(zone_type, [])
	if not zoning_profile_panel.visible or _active_zoning_zone_type != zone_type or profiles.is_empty():
		_open_zoning_zone_type(zone_type, true)
		return
	var current := _current_zoning_profile_runtime_id()
	var next := 0
	for index in profiles.size():
		if int(Dictionary(profiles[index]).get("runtime_id", 0)) == current:
			next = (index + 1) % profiles.size()
	_select_zoning_profile(zone_type, int(Dictionary(profiles[next]).get("runtime_id", 0)))

# Redrawn when the tiles change: road types refresh, the zoning workflow (its digit sits on the
# workflow it switches to). The terrain row is fixed.
func _apply_number_badges() -> void:
	if not is_instance_valid(road_type_menu) or not is_instance_valid(zoning_combined_hbox) \
			or not is_instance_valid(terrain_sub_menu):
		return
	for tile in _numbered:
		if is_instance_valid(tile):
			HudIcons.set_number(tile, "")
	_numbered.clear()
	for slots in [_road_number_slots(), _zoning_number_slots(), terrain_sub_menu.number_slots()]:
		for index in slots.size():
			var tile: Button = slots[index][0]
			HudIcons.set_number(tile, str(index + 1) if index < 9 else "")
			_numbered.append(tile)

## Closes the City panel or the open dock palette, one per call; false when none was open, so
## Esc goes on to the pause menu.
func close_open_panel() -> bool:
	if _city_panel and _city_panel.is_open():
		_city_panel.toggle()
		return true
	for panel in [road_combined_hbox, terrain_sub_menu, zoning_combined_hbox, services_combined_hbox, industry_combined_hbox]:
		if panel.visible:
			panel.visible = false
			get_meta("options_panel").visible = false
			get_meta("road_sep").visible = false
			_collapse_zoning_profiles()
			_refresh_hud_marks()
			return true
	return false

func toggle_city_panel() -> void:
	_city_panel.toggle()
	if _city_panel.is_open() and simulation_node:
		_city_panel.set_stats(_display_treasury, _display_agent_count)
		_city_panel.set_growth(simulation_node.get_growth_overview())

func set_sim_speed_display(speed: float):
	_display_speed = speed
	if _top_status:
		_top_status.mark_speed(speed)
