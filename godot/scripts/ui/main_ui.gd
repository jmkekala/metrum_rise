## Procedurally built HUD — all UI panels, buttons, and overlays constructed at runtime.
##
## Rust methods called: set_simulation_speed(), undo_action(),
##   get_pollution_image_data(), get_noise_image_data(), get_desirability_image_data(),
##   get_demand_pressures(), get_treasury_balance(), get_agent_count(),
##   get_service_building_assets()
##   get_industry_building_assets()
## No scene file for the UI; every control is created in _ready() and helper functions.
extends CanvasLayer

const CityStatusPanel = preload("res://scripts/ui/city_status_panel.gd")
const PackManager = preload("res://scripts/ui/pack_manager.gd")
const DemandMeter = preload("res://scripts/ui/demand_meter.gd")
const RoadPropertiesWindow = preload("res://scripts/ui/road_properties_window.gd")
const UIStyle = preload("res://scripts/ui/ui_style.gd")

const InputManager = preload("res://scripts/core/input_manager.gd")

@onready var input_manager = $"../InputManager"
@onready var road_tool = $"../RoadTool"
@onready var simulation_node = $"../SimulationNode"

var bottom_panel: Control
var main_toolbar: HBoxContainer

var road_main_btn: Button
var road_sub_menu: HBoxContainer
var road_options_menu: VBoxContainer
var road_combined_hbox: HBoxContainer
var zoning_main_btn: Button

var terrain_main_btn: Button
var terrain_sub_menu: HBoxContainer

# Road items
var road_2l_btn: Button
var road_4l_btn: Button
var walkway_btn: Button

# Road options
var straight_btn: Button
var spline_btn: Button
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
var clock_panel: PanelContainer
var clock_label: Label
var speed_label: Label
var speed_down_btn: Button
var speed_up_btn: Button
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

const CLOCK_PANEL_WIDTH := 220.0
const CITY_STATUS_PANEL_WIDTH := 170.0
const DEMAND_METER_WIDTH := 92.0

func _toolbar_button_height() -> float:
	return maxf(1.0, minf(UIStyle.HUD_BUTTON_HEIGHT, UIStyle.HUD_STRIP_HEIGHT - float(UIStyle.HUD_SHELL_PAD_Y * 2)))

func _ready():
	_build_ui()
	_build_auxiliary_windows()
	_connect_signals()
	_refresh_clock_display(true)
	_refresh_city_status_display()
	set_sim_speed_display(0.0)

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

	var left_strip_margin := MarginContainer.new()
	left_strip_margin.set_anchors_and_offsets_preset(Control.PRESET_FULL_RECT)
	left_strip_margin.mouse_filter = Control.MOUSE_FILTER_IGNORE
	left_strip_margin.add_theme_constant_override("margin_left", int(UIStyle.HUD_LEFT_MARGIN))
	left_strip_margin.add_theme_constant_override("margin_bottom", int(UIStyle.HUD_BOTTOM_MARGIN))
	bottom_panel.add_child(left_strip_margin)

	var left_strip_stack := VBoxContainer.new()
	left_strip_stack.alignment = BoxContainer.ALIGNMENT_END
	left_strip_stack.mouse_filter = Control.MOUSE_FILTER_IGNORE
	left_strip_margin.add_child(left_strip_stack)

	var left_bottom_strip := HBoxContainer.new()
	left_bottom_strip.add_theme_constant_override("separation", int(UIStyle.HUD_PANEL_GAP))
	left_bottom_strip.mouse_filter = Control.MOUSE_FILTER_IGNORE
	left_strip_stack.add_child(left_bottom_strip)

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
	main_vbox.mouse_filter = Control.MOUSE_FILTER_STOP
	main_vbox.alignment = BoxContainer.ALIGNMENT_END
	
	# VBox to hold the stack of menus (options on top, then sub-menu, then main toolbar)
	var vbox = VBoxContainer.new()
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
	
	# 3. Road Options Menu (Left side)
	road_options_menu = VBoxContainer.new()
	var options_panel = PanelContainer.new()
	var op_style = StyleBoxFlat.new()
	op_style.bg_color = Color(0.15, 0.15, 0.15, 0.7)
	op_style.set_corner_radius_all(10)
	options_panel.add_theme_stylebox_override("panel", op_style)
	
	var options_padding = MarginContainer.new()
	options_padding.add_theme_constant_override("margin_left", 8)
	options_padding.add_theme_constant_override("margin_right", 8)
	options_padding.add_theme_constant_override("margin_top", 5)
	options_padding.add_theme_constant_override("margin_bottom", 5)
	options_panel.add_child(options_padding)
	options_padding.add_child(road_options_menu)
	
	straight_btn = Button.new()
	straight_btn.text = "Straight"
	straight_btn.toggle_mode = true
	straight_btn.focus_mode = Control.FOCUS_NONE
	straight_btn.button_pressed = true 
	road_options_menu.add_child(straight_btn)

	spline_btn = Button.new()
	spline_btn.text = "Spline"
	spline_btn.toggle_mode = true
	spline_btn.focus_mode = Control.FOCUS_NONE
	road_options_menu.add_child(spline_btn)
	
	road_combined_hbox.add_child(options_panel)
	# Initially hide the options panel until a tool is actually selected? 
	# No, let's keep it visible if the submenu is open or just hide it like before.
	options_panel.visible = false 
	# Store reference to toggle it
	self.set_meta("options_panel", options_panel)

	var sep = VSeparator.new()
	road_combined_hbox.add_child(sep)
	sep.visible = false
	self.set_meta("road_sep", sep)
	
	# 2. Road Sub Menu (Right side)
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
	
	walkway_btn = Button.new()
	walkway_btn.text = "Walkway"
	walkway_btn.focus_mode = Control.FOCUS_NONE
	road_sub_menu.add_child(walkway_btn)
	
	road_2l_btn = Button.new()
	road_2l_btn.text = "2-Lane Road"
	road_2l_btn.focus_mode = Control.FOCUS_NONE
	road_sub_menu.add_child(road_2l_btn)

	road_4l_btn = Button.new()
	road_4l_btn.text = "4-Lane Road"
	road_4l_btn.focus_mode = Control.FOCUS_NONE
	road_sub_menu.add_child(road_4l_btn)

	var ow1_btn = Button.new()
	ow1_btn.text = "One-Way 1L"
	ow1_btn.focus_mode = Control.FOCUS_NONE
	road_sub_menu.add_child(ow1_btn)
	ow1_btn.pressed.connect(func(): _select_road_type(1, 0))

	var ow2_btn = Button.new()
	ow2_btn.text = "One-Way 2L"
	ow2_btn.focus_mode = Control.FOCUS_NONE
	road_sub_menu.add_child(ow2_btn)
	ow2_btn.pressed.connect(func(): _select_road_type(2, 0))

	var cul_de_sac_btn = Button.new()
	cul_de_sac_btn.text = "Cul-De-Sac"
	cul_de_sac_btn.custom_minimum_size = Vector2(100, 0)
	cul_de_sac_btn.focus_mode = Control.FOCUS_NONE
	road_sub_menu.add_child(cul_de_sac_btn)
	cul_de_sac_btn.pressed.connect(func(): input_manager._toggle_tool(InputManager.Tool.CUL_DE_SAC))
	
	road_combined_hbox.add_child(sub_panel)
	
	# --- Terrain Sub Menu ---
	terrain_sub_menu = HBoxContainer.new()
	terrain_sub_menu.add_theme_constant_override("separation", 10)
	terrain_sub_menu.visible = false
	var terrain_sub_panel = PanelContainer.new()
	terrain_sub_panel.add_child(terrain_sub_menu)
	
	var sculpt_btn = Button.new()
	sculpt_btn.text = "Raise/Lower Terrain"
	sculpt_btn.focus_mode = Control.FOCUS_NONE
	terrain_sub_menu.add_child(sculpt_btn)
	sculpt_btn.pressed.connect(func(): input_manager._toggle_tool(InputManager.Tool.SCULPT))
	
	var terrain_sub_margin = MarginContainer.new()
	terrain_sub_margin.add_theme_constant_override("margin_bottom", 5)
	terrain_sub_margin.add_child(terrain_sub_panel)
	
	var hbox_terrain_sub_center = HBoxContainer.new()
	hbox_terrain_sub_center.alignment = BoxContainer.ALIGNMENT_CENTER
	hbox_terrain_sub_center.add_child(terrain_sub_margin)
	vbox.add_child(hbox_terrain_sub_center)

	# --- Combined Zoning Sub-Menu Row ---
	zoning_combined_hbox = VBoxContainer.new()
	zoning_combined_hbox.alignment = BoxContainer.ALIGNMENT_CENTER
	zoning_combined_hbox.size_flags_horizontal = Control.SIZE_SHRINK_CENTER
	zoning_combined_hbox.add_theme_constant_override("separation", 8)
	zoning_combined_hbox.visible = false
	vbox.add_child(zoning_combined_hbox)
	vbox.move_child(zoning_combined_hbox, vbox.get_child_count() - 2)

	zoning_profile_panel = PanelContainer.new()
	var z_sub_style = StyleBoxFlat.new()
	z_sub_style.bg_color = Color(0.1, 0.1, 0.1, 0.8)
	z_sub_style.set_corner_radius_all(15)
	zoning_profile_panel.add_theme_stylebox_override("panel", z_sub_style)
	zoning_profile_panel.visible = false
	
	var z_profile_padding = MarginContainer.new()
	z_profile_padding.add_theme_constant_override("margin_left", 12)
	z_profile_padding.add_theme_constant_override("margin_right", 12)
	z_profile_padding.add_theme_constant_override("margin_top", 8)
	z_profile_padding.add_theme_constant_override("margin_bottom", 8)
	zoning_profile_panel.add_child(z_profile_padding)
	
	zoning_profile_menu = HBoxContainer.new()
	zoning_profile_menu.alignment = BoxContainer.ALIGNMENT_CENTER
	zoning_profile_menu.add_theme_constant_override("separation", 10)
	z_profile_padding.add_child(zoning_profile_menu)
	
	zoning_combined_hbox.add_child(zoning_profile_panel)

	var zoning_controls_row = HBoxContainer.new()
	zoning_controls_row.alignment = BoxContainer.ALIGNMENT_CENTER
	zoning_controls_row.add_theme_constant_override("separation", 15)
	zoning_combined_hbox.add_child(zoning_controls_row)

	var zoning_mode_panel = PanelContainer.new()
	zoning_mode_panel.add_theme_stylebox_override("panel", z_sub_style.duplicate())

	var zoning_mode_padding = MarginContainer.new()
	zoning_mode_padding.add_theme_constant_override("margin_left", 12)
	zoning_mode_padding.add_theme_constant_override("margin_right", 12)
	zoning_mode_padding.add_theme_constant_override("margin_top", 8)
	zoning_mode_padding.add_theme_constant_override("margin_bottom", 8)
	zoning_mode_panel.add_child(zoning_mode_padding)

	zoning_mode_menu = HBoxContainer.new()
	zoning_mode_menu.add_theme_constant_override("separation", 10)
	zoning_mode_padding.add_child(zoning_mode_menu)

	zoning_controls_row.add_child(zoning_mode_panel)

	var zoning_type_panel = PanelContainer.new()
	zoning_type_panel.add_theme_stylebox_override("panel", z_sub_style.duplicate())

	var zoning_type_padding = MarginContainer.new()
	zoning_type_padding.add_theme_constant_override("margin_left", 12)
	zoning_type_padding.add_theme_constant_override("margin_right", 12)
	zoning_type_padding.add_theme_constant_override("margin_top", 8)
	zoning_type_padding.add_theme_constant_override("margin_bottom", 8)
	zoning_type_panel.add_child(zoning_type_padding)

	zoning_type_menu = HBoxContainer.new()
	zoning_type_menu.add_theme_constant_override("separation", 10)
	zoning_type_padding.add_child(zoning_type_menu)

	zoning_controls_row.add_child(zoning_type_panel)

	zoning_options_btn = Button.new()
	zoning_options_btn.text = "⚙"
	zoning_options_btn.tooltip_text = "Parcel options"
	zoning_options_btn.custom_minimum_size = Vector2(70, 50)
	zoning_options_btn.focus_mode = Control.FOCUS_NONE
	zoning_options_btn.add_theme_font_size_override("font_size", 24)
	zoning_options_btn.button_down.connect(_remember_zoning_options_button_down_state)
	zoning_options_btn.pressed.connect(_toggle_zoning_options_popup)
	zoning_mode_menu.add_child(zoning_options_btn)
	_build_zoning_options_popup()

	for entry in ZONING_MAIN_TYPES:
		var zone_type := str(entry.get("id", ""))
		var button := Button.new()
		button.text = str(entry.get("label", zone_type.capitalize()))
		button.custom_minimum_size = Vector2(150, 50)
		button.toggle_mode = true
		_apply_colored_button_style(button, _zone_type_color(zone_type))
		button.pressed.connect(func(): _toggle_zoning_zone_type(zone_type))
		zoning_type_menu.add_child(button)
		_zoning_type_buttons[zone_type] = button

	_rebuild_zoning_profiles_index()
	_collapse_zoning_profiles()

	# --- Combined Services Sub-Menu Row ---
	services_combined_hbox = VBoxContainer.new()
	services_combined_hbox.alignment = BoxContainer.ALIGNMENT_CENTER
	services_combined_hbox.size_flags_horizontal = Control.SIZE_SHRINK_CENTER
	services_combined_hbox.add_theme_constant_override("separation", 8)
	services_combined_hbox.visible = false
	vbox.add_child(services_combined_hbox)

	var service_panel_style = StyleBoxFlat.new()
	service_panel_style.bg_color = Color(0.1, 0.1, 0.1, 0.8)
	service_panel_style.set_corner_radius_all(15)

	service_asset_panel = PanelContainer.new()
	service_asset_panel.add_theme_stylebox_override("panel", service_panel_style.duplicate())
	service_asset_panel.visible = false

	var service_asset_padding = MarginContainer.new()
	service_asset_padding.add_theme_constant_override("margin_left", 12)
	service_asset_padding.add_theme_constant_override("margin_right", 12)
	service_asset_padding.add_theme_constant_override("margin_top", 8)
	service_asset_padding.add_theme_constant_override("margin_bottom", 8)
	service_asset_panel.add_child(service_asset_padding)

	service_asset_menu = HBoxContainer.new()
	service_asset_menu.alignment = BoxContainer.ALIGNMENT_CENTER
	service_asset_menu.add_theme_constant_override("separation", 10)
	service_asset_padding.add_child(service_asset_menu)
	services_combined_hbox.add_child(service_asset_panel)

	var service_category_panel = PanelContainer.new()
	service_category_panel.add_theme_stylebox_override("panel", service_panel_style.duplicate())

	var service_category_padding = MarginContainer.new()
	service_category_padding.add_theme_constant_override("margin_left", 12)
	service_category_padding.add_theme_constant_override("margin_right", 12)
	service_category_padding.add_theme_constant_override("margin_top", 8)
	service_category_padding.add_theme_constant_override("margin_bottom", 8)
	service_category_panel.add_child(service_category_padding)

	service_category_menu = HBoxContainer.new()
	service_category_menu.alignment = BoxContainer.ALIGNMENT_CENTER
	service_category_menu.add_theme_constant_override("separation", 10)
	service_category_padding.add_child(service_category_menu)
	services_combined_hbox.add_child(service_category_panel)

	# --- Industry Sub-Menu Row ---
	industry_combined_hbox = VBoxContainer.new()
	industry_combined_hbox.alignment = BoxContainer.ALIGNMENT_CENTER
	industry_combined_hbox.size_flags_horizontal = Control.SIZE_SHRINK_CENTER
	industry_combined_hbox.add_theme_constant_override("separation", 8)
	industry_combined_hbox.visible = false
	vbox.add_child(industry_combined_hbox)

	industry_asset_panel = PanelContainer.new()
	industry_asset_panel.add_theme_stylebox_override("panel", service_panel_style.duplicate())
	industry_asset_panel.visible = true

	var industry_asset_padding = MarginContainer.new()
	industry_asset_padding.add_theme_constant_override("margin_left", 12)
	industry_asset_padding.add_theme_constant_override("margin_right", 12)
	industry_asset_padding.add_theme_constant_override("margin_top", 8)
	industry_asset_padding.add_theme_constant_override("margin_bottom", 8)
	industry_asset_panel.add_child(industry_asset_padding)

	industry_asset_menu = HBoxContainer.new()
	industry_asset_menu.alignment = BoxContainer.ALIGNMENT_CENTER
	industry_asset_menu.add_theme_constant_override("separation", 10)
	industry_asset_padding.add_child(industry_asset_menu)
	industry_combined_hbox.add_child(industry_asset_panel)
	
	# 1. Main Toolbar (Bottom stack layer)
	main_toolbar = HBoxContainer.new()
	main_toolbar.add_theme_constant_override("separation", 15)
	main_toolbar.size_flags_vertical = Control.SIZE_SHRINK_CENTER
	var toolbar_button_height := _toolbar_button_height()
	
	road_main_btn = Button.new()
	road_main_btn.text = "Roads"
	road_main_btn.custom_minimum_size = Vector2(100, toolbar_button_height)
	road_main_btn.size_flags_vertical = Control.SIZE_SHRINK_CENTER
	_apply_hud_toolbar_text_style(road_main_btn)
	var style = StyleBoxFlat.new()
	style.bg_color = Color(0.2, 0.2, 0.2, 0.8)
	style.corner_radius_top_left = 25
	style.corner_radius_top_right = 25
	style.corner_radius_bottom_left = 25
	style.corner_radius_bottom_right = 25
	road_main_btn.add_theme_stylebox_override("normal", style)
	main_toolbar.add_child(road_main_btn)

	zoning_main_btn = Button.new()
	zoning_main_btn.text = "Zoning"
	zoning_main_btn.custom_minimum_size = Vector2(100, toolbar_button_height)
	zoning_main_btn.size_flags_vertical = Control.SIZE_SHRINK_CENTER
	_apply_hud_toolbar_text_style(zoning_main_btn)
	zoning_main_btn.add_theme_stylebox_override("normal", style.duplicate())
	main_toolbar.add_child(zoning_main_btn)

	services_main_btn = Button.new()
	services_main_btn.text = "Services"
	services_main_btn.custom_minimum_size = Vector2(110, toolbar_button_height)
	services_main_btn.size_flags_vertical = Control.SIZE_SHRINK_CENTER
	_apply_hud_toolbar_text_style(services_main_btn)
	services_main_btn.add_theme_stylebox_override("normal", style.duplicate())
	main_toolbar.add_child(services_main_btn)

	industry_main_btn = Button.new()
	industry_main_btn.text = "Industry"
	industry_main_btn.custom_minimum_size = Vector2(110, toolbar_button_height)
	industry_main_btn.size_flags_vertical = Control.SIZE_SHRINK_CENTER
	_apply_hud_toolbar_text_style(industry_main_btn)
	industry_main_btn.add_theme_stylebox_override("normal", style.duplicate())
	main_toolbar.add_child(industry_main_btn)

	terrain_main_btn = Button.new()
	terrain_main_btn.text = "Terrain"
	terrain_main_btn.custom_minimum_size = Vector2(100, toolbar_button_height)
	terrain_main_btn.size_flags_vertical = Control.SIZE_SHRINK_CENTER
	_apply_hud_toolbar_text_style(terrain_main_btn)
	terrain_main_btn.add_theme_stylebox_override("normal", style.duplicate())
	main_toolbar.add_child(terrain_main_btn)
	
	select_main_btn = Button.new()
	select_main_btn.text = "Inspect"
	select_main_btn.custom_minimum_size = Vector2(100, toolbar_button_height)
	select_main_btn.size_flags_vertical = Control.SIZE_SHRINK_CENTER
	_apply_hud_toolbar_text_style(select_main_btn)
	select_main_btn.add_theme_stylebox_override("normal", style.duplicate())
	main_toolbar.add_child(select_main_btn)

	var mods_btn := Button.new()
	mods_btn.text = "Mods"
	mods_btn.custom_minimum_size = Vector2(100, toolbar_button_height)
	mods_btn.size_flags_vertical = Control.SIZE_SHRINK_CENTER
	_apply_hud_toolbar_text_style(mods_btn)
	mods_btn.add_theme_stylebox_override("normal", style.duplicate())
	mods_btn.pressed.connect(_on_mods_btn_pressed)
	main_toolbar.add_child(mods_btn)
	
	# Wrapper to center main toolbar
	var hbox_main_center = HBoxContainer.new()
	hbox_main_center.alignment = BoxContainer.ALIGNMENT_CENTER
	hbox_main_center.size_flags_horizontal = Control.SIZE_EXPAND_FILL
	hbox_main_center.add_child(main_toolbar)

	vbox.add_child(_create_bottom_strip_shell(hbox_main_center, 0.0, UIStyle.hud_clear_style()))
	toolbar_center.add_child(_create_bottom_group_shell(main_vbox))

	# --- Bottom-left Strip ---
	clock_panel = _create_bottom_strip_shell(_build_clock_content(), CLOCK_PANEL_WIDTH)
	left_bottom_strip.add_child(clock_panel)

	city_status_panel = CityStatusPanel.new()
	left_bottom_strip.add_child(_create_bottom_strip_shell(city_status_panel, CITY_STATUS_PANEL_WIDTH))

	demand_meter = DemandMeter.new()
	left_bottom_strip.add_child(_create_bottom_strip_shell(demand_meter, DEMAND_METER_WIDTH))

	# --- Bottom-right Tool Strip ---
	var right_strip_margin := MarginContainer.new()
	right_strip_margin.set_anchors_and_offsets_preset(Control.PRESET_FULL_RECT)
	right_strip_margin.mouse_filter = Control.MOUSE_FILTER_IGNORE
	right_strip_margin.add_theme_constant_override("margin_right", int(UIStyle.HUD_LEFT_MARGIN))
	right_strip_margin.add_theme_constant_override("margin_bottom", int(UIStyle.HUD_BOTTOM_MARGIN))
	bottom_panel.add_child(right_strip_margin)

	var right_strip_stack := VBoxContainer.new()
	right_strip_stack.alignment = BoxContainer.ALIGNMENT_END
	right_strip_stack.size_flags_horizontal = Control.SIZE_EXPAND_FILL
	right_strip_stack.mouse_filter = Control.MOUSE_FILTER_IGNORE
	right_strip_margin.add_child(right_strip_stack)

	var right_bottom_strip := HBoxContainer.new()
	right_bottom_strip.alignment = BoxContainer.ALIGNMENT_END
	right_bottom_strip.size_flags_horizontal = Control.SIZE_EXPAND_FILL
	right_bottom_strip.mouse_filter = Control.MOUSE_FILTER_IGNORE
	right_strip_stack.add_child(right_bottom_strip)

	bulldoze_btn = Button.new()
	bulldoze_btn.text = "⌫"
	bulldoze_btn.tooltip_text = "Bulldoze"
	bulldoze_btn.custom_minimum_size = Vector2(52, toolbar_button_height)
	bulldoze_btn.size_flags_vertical = Control.SIZE_SHRINK_CENTER
	_apply_hud_toolbar_text_style(bulldoze_btn)
	var bulldoze_style := StyleBoxFlat.new()
	bulldoze_style.bg_color = Color(0.45, 0.08, 0.06, 0.86)
	bulldoze_style.set_corner_radius_all(8)
	bulldoze_btn.add_theme_stylebox_override("normal", bulldoze_style)
	right_bottom_strip.add_child(_create_bottom_strip_shell(bulldoze_btn, 82.0))

func _build_auxiliary_windows() -> void:
	road_properties_panel = RoadPropertiesWindow.new()
	add_child(road_properties_panel)
	if road_properties_panel.has_method("setup"):
		road_properties_panel.setup(simulation_node, input_manager.select_tool)

func _build_clock_content() -> VBoxContainer:
	var clock_vbox := VBoxContainer.new()
	clock_vbox.add_theme_constant_override("separation", 8)

	clock_label = Label.new()
	clock_label.text = "Day 1 00:00"
	clock_label.horizontal_alignment = HORIZONTAL_ALIGNMENT_LEFT
	clock_label.add_theme_font_size_override("font_size", UIStyle.HUD_TEXT_SIZE)
	clock_label.add_theme_color_override("font_color", UIStyle.TEXT_PRIMARY)
	clock_vbox.add_child(clock_label)

	var speed_hbox := HBoxContainer.new()
	speed_hbox.add_theme_constant_override("separation", 8)
	clock_vbox.add_child(speed_hbox)

	speed_down_btn = Button.new()
	speed_down_btn.text = "-"
	speed_down_btn.custom_minimum_size = Vector2(36, 32)
	speed_down_btn.focus_mode = Control.FOCUS_NONE
	speed_hbox.add_child(speed_down_btn)

	speed_label = Label.new()
	speed_label.text = "Paused"
	speed_label.custom_minimum_size = Vector2(90, 0)
	speed_label.vertical_alignment = VERTICAL_ALIGNMENT_CENTER
	speed_label.add_theme_font_size_override("font_size", UIStyle.HUD_TEXT_SIZE)
	speed_label.add_theme_color_override("font_color", UIStyle.TEXT_PRIMARY)
	speed_hbox.add_child(speed_label)

	speed_up_btn = Button.new()
	speed_up_btn.text = "+"
	speed_up_btn.custom_minimum_size = Vector2(36, 32)
	speed_up_btn.focus_mode = Control.FOCUS_NONE
	speed_hbox.add_child(speed_up_btn)

	return clock_vbox

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
	shell.add_theme_stylebox_override("panel", UIStyle.hud_group_style())

	var padding := MarginContainer.new()
	padding.add_theme_constant_override("margin_left", UIStyle.HUD_SHELL_PAD_X)
	padding.add_theme_constant_override("margin_right", UIStyle.HUD_SHELL_PAD_X)
	padding.add_theme_constant_override("margin_top", UIStyle.HUD_SHELL_PAD_Y)
	padding.add_theme_constant_override("margin_bottom", UIStyle.HUD_SHELL_PAD_Y)
	shell.add_child(padding)

	content.size_flags_horizontal = Control.SIZE_SHRINK_CENTER
	padding.add_child(content)
	return shell

func _apply_hud_toolbar_text_style(button: Button) -> void:
	button.focus_mode = Control.FOCUS_NONE
	button.add_theme_font_size_override("font_size", UIStyle.HUD_TEXT_SIZE)
	button.add_theme_color_override("font_color", UIStyle.TEXT_PRIMARY)
	button.add_theme_color_override("font_hover_color", UIStyle.TEXT_PRIMARY)
	button.add_theme_color_override("font_pressed_color", UIStyle.TEXT_PRIMARY)
	button.add_theme_color_override("font_focus_color", UIStyle.TEXT_PRIMARY)
	button.add_theme_color_override("font_disabled_color", UIStyle.TEXT_DIM)

func _connect_signals():
	road_main_btn.pressed.connect(_on_road_main_pressed)
	terrain_main_btn.pressed.connect(_on_terrain_main_pressed)
	zoning_main_btn.pressed.connect(_on_zoning_main_pressed)
	services_main_btn.pressed.connect(_on_services_main_pressed)
	industry_main_btn.pressed.connect(_on_industry_main_pressed)
	select_main_btn.pressed.connect(_on_select_main_pressed)
	bulldoze_btn.pressed.connect(_on_bulldoze_pressed)
	
	road_2l_btn.pressed.connect(func(): _select_road_type(1, 1))
	road_4l_btn.pressed.connect(func(): _select_road_type(2, 2))
	walkway_btn.pressed.connect(func(): _select_road_type(0, 0))
	
	straight_btn.pressed.connect(func(): _set_draw_mode(0))
	spline_btn.pressed.connect(func(): _set_draw_mode(1))
	speed_down_btn.pressed.connect(func(): input_manager.step_simulation_speed(-1))
	speed_up_btn.pressed.connect(func(): input_manager.step_simulation_speed(1))

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

func _select_road_type(fwd: int, bkw: int):
	# Show options panel and separator
	get_meta("options_panel").visible = true
	get_meta("road_sep").visible = true
	
	# Activate tool via input manager
	# Walkway vs Road tool distinction from InputManager:
	var required_tool = InputManager.Tool.WALKWAY if fwd == 0 and bkw == 0 else InputManager.Tool.ROAD
	
	if input_manager.current_tool != required_tool:
		input_manager._toggle_tool(required_tool)
		
	# InputManager resets lanes, we override here
	if road_tool:
		road_tool.fwd_lanes = fwd
		road_tool.bkw_lanes = bkw
		if road_tool.has_method("_update_lanes_label"):
			road_tool._update_lanes_label()

func _set_draw_mode(mode: int):
	straight_btn.button_pressed = (mode == 0)
	spline_btn.button_pressed = (mode == 1)
	
	if road_tool:
		if mode == 0:
			# If road tool has a straight mode flag/variable we set it.
			# Let's assume road_tool.draw_mode exists or we can add it later.
			if "draw_mode" in road_tool:
				road_tool.draw_mode = 0
		else:
			if "draw_mode" in road_tool:
				road_tool.draw_mode = 1

func _on_select_main_pressed():
	road_combined_hbox.visible = false
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

func _color_from_hex(hex: String, alpha: float) -> Color:
	if hex.length() == 7 and hex.begins_with("#"):
		var r := hex.substr(1, 2).hex_to_int()
		var g := hex.substr(3, 2).hex_to_int()
		var b := hex.substr(5, 2).hex_to_int()
		return Color8(r, g, b, int(clampf(alpha, 0.0, 1.0) * 255.0))
	return Color(0.5, 0.5, 0.5, alpha)

func _apply_colored_button_style(button: Button, base_color: Color) -> void:
	button.focus_mode = Control.FOCUS_NONE
	var normal := StyleBoxFlat.new()
	normal.bg_color = base_color
	normal.set_corner_radius_all(10)

	var hover := normal.duplicate()
	hover.bg_color = base_color.lightened(0.08)

	var pressed := normal.duplicate()
	pressed.bg_color = base_color.lightened(0.16)

	var disabled := normal.duplicate()
	disabled.bg_color = Color(base_color.r, base_color.g, base_color.b, base_color.a * 0.5)

	button.add_theme_stylebox_override("normal", normal)
	button.add_theme_stylebox_override("hover", hover)
	button.add_theme_stylebox_override("pressed", pressed)
	button.add_theme_stylebox_override("focus", pressed)
	button.add_theme_stylebox_override("disabled", disabled)

func _zone_type_color(zone_type: String) -> Color:
	match zone_type:
		"residential":
			return UIStyle.ZONE_RESIDENTIAL
		"commercial":
			return UIStyle.ZONE_COMMERCIAL
		"industrial":
			return UIStyle.ZONE_INDUSTRIAL
		_:
			return Color(0.35, 0.35, 0.35, 0.75)

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
		var button := Button.new()
		button.text = str(profile.get("display_name", zone_type.capitalize()))
		button.tooltip_text = str(profile.get("ui_description", ""))
		button.custom_minimum_size = Vector2(165, 50)
		button.toggle_mode = true
		_apply_colored_button_style(button, _color_from_hex(str(profile.get("ui_color", "#777777")), 0.4))
		var runtime_id := int(profile.get("runtime_id", 0))
		button.pressed.connect(func(): _select_zoning_profile(zone_type, runtime_id))
		zoning_profile_menu.add_child(button)
		_zoning_profile_buttons[runtime_id] = button

	var clear_btn := Button.new()
	clear_btn.text = "Free"
	clear_btn.custom_minimum_size = Vector2(90, 50)
	clear_btn.toggle_mode = true
	_apply_colored_button_style(clear_btn, Color(0.45, 0.45, 0.45, 0.60))
	clear_btn.pressed.connect(func(): _select_zoning_profile(zone_type, 0))
	zoning_profile_menu.add_child(clear_btn)
	_zoning_profile_buttons[0] = clear_btn

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
		var button := Button.new()
		button.text = _service_class_label(class_id)
		button.custom_minimum_size = Vector2(145, 50)
		button.toggle_mode = true
		_apply_colored_button_style(button, _service_class_color(class_id))
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
		var button := Button.new()
		button.text = str(asset.get("display_name", asset_id))
		button.tooltip_text = asset_id
		button.custom_minimum_size = Vector2(190, 50)
		button.toggle_mode = true
		_apply_colored_button_style(button, _service_class_color(service_class).lightened(0.04))
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
		var button := Button.new()
		button.text = str(asset.get("display_name", asset_id))
		button.tooltip_text = "%s / %s" % [asset_id, str(asset.get("resource_id", ""))]
		button.custom_minimum_size = Vector2(190, 50)
		button.toggle_mode = true
		_apply_colored_button_style(button, Color(0.40, 0.39, 0.34, 0.76))
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

func _service_class_color(service_class: String) -> Color:
	match service_class:
		"power":
			return Color(0.78, 0.55, 0.18, 0.72)
		"water":
			return Color(0.16, 0.50, 0.70, 0.72)
		"waste", "sewage":
			return Color(0.28, 0.52, 0.34, 0.72)
		_:
			return Color(0.42, 0.42, 0.46, 0.72)

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
	var demand: Vector3 = simulation_node.get_demand_pressures()
	if demand_meter.has_method("set_pressures"):
		demand_meter.set_pressures(demand.x, demand.y, demand.z)

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

func set_sim_speed_display(speed: float):
	_display_speed = speed
	if not speed_label:
		return
	if speed <= 0.001:
		speed_label.text = "Paused"
	else:
		speed_label.text = "%.1fx" % speed

var _pack_manager: Window = null

func _on_mods_btn_pressed() -> void:
	if not _pack_manager:
		_pack_manager = PackManager.new()
		add_child(_pack_manager)
	_pack_manager.popup_centered()
