## Procedurally built HUD — all UI panels, buttons, and overlays constructed at runtime.
##
## Rust methods called: set_simulation_speed(), undo_action(),
##   get_pollution_image_data(), get_noise_image_data(), get_desirability_image_data()
## No scene file for the UI; every control is created in _ready() and helper functions.
extends CanvasLayer

const PackManager = preload("res://scripts/pack_manager.gd")

const InputManager = preload("res://scripts/input_manager.gd")

@onready var input_manager = $"../InputManager"
@onready var road_tool = $"../RoadTool"
@onready var simulation_node = $"../SimulationNode"

var bottom_panel: MarginContainer
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
var zoning_combined_hbox: HBoxContainer
var zoning_sub_menu: HBoxContainer
var select_main_btn: Button
var road_properties_panel: PanelContainer

func _ready():
	_build_ui()
	_connect_signals()

func _build_ui():
	# Foolproof method for Godot 4 programmatic UI: Full screen root, push to bottom
	var root = Control.new()
	add_child(root)
	root.set_anchors_and_offsets_preset(Control.PRESET_FULL_RECT)
	root.mouse_filter = Control.MOUSE_FILTER_IGNORE
	
	var margin = MarginContainer.new()
	root.add_child(margin)
	margin.set_anchors_and_offsets_preset(Control.PRESET_FULL_RECT)
	margin.mouse_filter = Control.MOUSE_FILTER_IGNORE
	margin.add_theme_constant_override("margin_bottom", 20)
	
	var shell_panel = PanelContainer.new()
	shell_panel.mouse_filter = Control.MOUSE_FILTER_STOP
	shell_panel.size_flags_horizontal = Control.SIZE_SHRINK_CENTER
	shell_panel.size_flags_vertical = Control.SIZE_SHRINK_END
	
	var shell_style = StyleBoxFlat.new()
	shell_style.bg_color = Color(0.1, 0.1, 0.1, 0.5)
	shell_style.set_corner_radius_all(15)
	shell_panel.add_theme_stylebox_override("panel", shell_style)
	margin.add_child(shell_panel)
	
	var shell_padding = MarginContainer.new()
	shell_padding.add_theme_constant_override("margin_left", 15)
	shell_padding.add_theme_constant_override("margin_right", 15)
	shell_padding.add_theme_constant_override("margin_top", 10)
	shell_padding.add_theme_constant_override("margin_bottom", 10)
	shell_panel.add_child(shell_padding)
	
	var main_vbox = VBoxContainer.new()
	shell_padding.add_child(main_vbox)
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
	straight_btn.button_pressed = true 
	road_options_menu.add_child(straight_btn)

	spline_btn = Button.new()
	spline_btn.text = "Spline"
	spline_btn.toggle_mode = true
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
	road_sub_menu.add_child(walkway_btn)
	
	road_2l_btn = Button.new()
	road_2l_btn.text = "2-Lane Road"
	road_sub_menu.add_child(road_2l_btn)

	road_4l_btn = Button.new()
	road_4l_btn.text = "4-Lane Road"
	road_sub_menu.add_child(road_4l_btn)

	var ow1_btn = Button.new()
	ow1_btn.text = "One-Way 1L"
	road_sub_menu.add_child(ow1_btn)
	ow1_btn.pressed.connect(func(): _select_road_type(1, 0))

	var ow2_btn = Button.new()
	ow2_btn.text = "One-Way 2L"
	road_sub_menu.add_child(ow2_btn)
	ow2_btn.pressed.connect(func(): _select_road_type(2, 0))

	var cul_de_sac_btn = Button.new()
	cul_de_sac_btn.text = "Cul-De-Sac"
	cul_de_sac_btn.custom_minimum_size = Vector2(100, 0)
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
	terrain_sub_menu.add_child(sculpt_btn)
	sculpt_btn.pressed.connect(func(): input_manager._toggle_tool(InputManager.Tool.SCULPT))
	
	var water_btn = Button.new()
	water_btn.text = "Add Water Source"
	terrain_sub_menu.add_child(water_btn)
	water_btn.pressed.connect(func(): input_manager._toggle_tool(InputManager.Tool.WATER))
	
	var terrain_sub_margin = MarginContainer.new()
	terrain_sub_margin.add_theme_constant_override("margin_bottom", 5)
	terrain_sub_margin.add_child(terrain_sub_panel)
	
	var hbox_terrain_sub_center = HBoxContainer.new()
	hbox_terrain_sub_center.alignment = BoxContainer.ALIGNMENT_CENTER
	hbox_terrain_sub_center.add_child(terrain_sub_margin)
	vbox.add_child(hbox_terrain_sub_center)

	# --- Combined Zoning Sub-Menu Row ---
	zoning_combined_hbox = HBoxContainer.new()
	zoning_combined_hbox.alignment = BoxContainer.ALIGNMENT_CENTER
	zoning_combined_hbox.add_theme_constant_override("separation", 15)
	zoning_combined_hbox.visible = false
	vbox.add_child(zoning_combined_hbox)
	vbox.move_child(zoning_combined_hbox, vbox.get_child_count() - 2)

	# Zoning Types Panel
	var zoning_sub_panel = PanelContainer.new()
	var z_sub_style = StyleBoxFlat.new()
	z_sub_style.bg_color = Color(0.1, 0.1, 0.1, 0.8)
	z_sub_style.set_corner_radius_all(15)
	zoning_sub_panel.add_theme_stylebox_override("panel", z_sub_style)
	
	var z_sub_padding = MarginContainer.new()
	z_sub_padding.add_theme_constant_override("margin_left", 12)
	z_sub_padding.add_theme_constant_override("margin_right", 12)
	z_sub_padding.add_theme_constant_override("margin_top", 8)
	z_sub_padding.add_theme_constant_override("margin_bottom", 8)
	zoning_sub_panel.add_child(z_sub_padding)
	
	zoning_sub_menu = HBoxContainer.new()
	zoning_sub_menu.add_theme_constant_override("separation", 10)
	z_sub_padding.add_child(zoning_sub_menu)
	
	zoning_combined_hbox.add_child(zoning_sub_panel)

	var zone_info = [
		{"name": "Residential", "type": 1, "color": Color(0.1, 0.8, 0.1)},
		{"name": "Commercial", "type": 2, "color": Color(0.1, 0.1, 0.8)},
		{"name": "Industrial", "type": 3, "color": Color(0.8, 0.8, 0.1)},
		{"name": "Office", "type": 4, "color": Color(0.1, 0.8, 0.8)},
		{"name": "Mixed", "type": 5, "color": Color(0.8, 0.1, 0.8)},
		{"name": "Clear", "type": 0, "color": Color(0.5, 0.5, 0.5)}
	]
	
	for zi in zone_info:
		var b = Button.new()
		b.text = zi.name
		b.custom_minimum_size = Vector2(80, 50)
		var bs = StyleBoxFlat.new()
		bs.bg_color = zi.color
		bs.bg_color.a = 0.4
		bs.set_corner_radius_all(10)
		b.add_theme_stylebox_override("normal", bs)
		b.pressed.connect(func():
			input_manager._handle_zoning_selection(KEY_0 + zi.type)
		)
		zoning_sub_menu.add_child(b)
	
	# 1. Main Toolbar (Bottom stack layer)
	main_toolbar = HBoxContainer.new()
	main_toolbar.add_theme_constant_override("separation", 15)
	
	road_main_btn = Button.new()
	road_main_btn.text = "Roads"
	road_main_btn.custom_minimum_size = Vector2(100, 50)
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
	zoning_main_btn.custom_minimum_size = Vector2(100, 50)
	zoning_main_btn.add_theme_stylebox_override("normal", style.duplicate())
	main_toolbar.add_child(zoning_main_btn)

	terrain_main_btn = Button.new()
	terrain_main_btn.text = "Terrain"
	terrain_main_btn.custom_minimum_size = Vector2(100, 50)
	terrain_main_btn.add_theme_stylebox_override("normal", style.duplicate())
	main_toolbar.add_child(terrain_main_btn)
	
	select_main_btn = Button.new()
	select_main_btn.text = "Inspect"
	select_main_btn.custom_minimum_size = Vector2(100, 50)
	select_main_btn.add_theme_stylebox_override("normal", style.duplicate())
	main_toolbar.add_child(select_main_btn)

	var mods_btn := Button.new()
	mods_btn.text = "Mods"
	mods_btn.custom_minimum_size = Vector2(100, 50)
	mods_btn.add_theme_stylebox_override("normal", style.duplicate())
	mods_btn.pressed.connect(_on_mods_btn_pressed)
	main_toolbar.add_child(mods_btn)
	
	# --- Road Properties Panel (Side) ---
	road_properties_panel = PanelContainer.new()
	root.add_child(road_properties_panel)
	road_properties_panel.set_anchors_and_offsets_preset(Control.PRESET_CENTER_RIGHT)
	road_properties_panel.custom_minimum_size = Vector2(250, 0)
	road_properties_panel.offset_left = -270 # Position it inside the screen from the right
	road_properties_panel.offset_right = -20
	road_properties_panel.visible = false
	
	var prop_style = StyleBoxFlat.new()
	prop_style.bg_color = Color(0.1, 0.1, 0.1, 0.8)
	prop_style.set_corner_radius_all(10)
	road_properties_panel.add_theme_stylebox_override("panel", prop_style)
	
	var prop_vbox = VBoxContainer.new()
	prop_vbox.add_theme_constant_override("separation", 10)
	var prop_padding = MarginContainer.new()
	prop_padding.add_theme_constant_override("margin_left", 15)
	prop_padding.add_theme_constant_override("margin_right", 15)
	prop_padding.add_theme_constant_override("margin_top", 15)
	prop_padding.add_theme_constant_override("margin_bottom", 15)
	road_properties_panel.add_child(prop_padding)
	prop_padding.add_child(prop_vbox)
	
	var title_hbox = HBoxContainer.new()
	prop_vbox.add_child(title_hbox)
	
	var title = Label.new()
	title.text = "Road Properties"
	title.size_flags_horizontal = Control.SIZE_EXPAND_FILL
	title_hbox.add_child(title)
	
	var close_btn = Button.new()
	close_btn.text = "X"
	close_btn.custom_minimum_size = Vector2(30, 30)
	title_hbox.add_child(close_btn)
	close_btn.pressed.connect(hide_road_properties)
	
	var warning_label = Label.new()
	warning_label.name = "WarningLabel"
	warning_label.autowrap_mode = TextServer.AUTOWRAP_WORD
	warning_label.add_theme_color_override("font_color", Color.YELLOW)
	prop_vbox.add_child(warning_label)
	road_properties_panel.set_meta("warning_label", warning_label)
	
	var class_btns: Array[Button] = []
	var classes = ["Standard", "Bridge", "Tunnel"]
	for i in range(classes.size()):
		var btn = Button.new()
		btn.text = classes[i]
		btn.toggle_mode = true
		prop_vbox.add_child(btn)
		class_btns.append(btn)
		btn.pressed.connect(func():
			if input_manager.select_tool:
				input_manager.select_tool.set_selected_edge_class(i)
			# Visually depress only this button
			for j in range(class_btns.size()):
				class_btns[j].set_pressed_no_signal(j == i)
		)
	road_properties_panel.set_meta("class_btns", class_btns)

	var prop_sep = HSeparator.new()
	prop_vbox.add_child(prop_sep)

	var no_build_check = CheckBox.new()
	no_build_check.text = "No buildings"
	prop_vbox.add_child(no_build_check)
	no_build_check.toggled.connect(func(pressed: bool):
		if input_manager.select_tool:
			input_manager.select_tool.set_selected_edge_no_building_spawn(pressed)
	)
	road_properties_panel.set_meta("no_build_check", no_build_check)

	# Wrapper to center main toolbar
	var hbox_main_center = HBoxContainer.new()
	hbox_main_center.alignment = BoxContainer.ALIGNMENT_CENTER
	hbox_main_center.add_child(main_toolbar)
	
	vbox.add_child(hbox_main_center)

func _connect_signals():
	road_main_btn.pressed.connect(_on_road_main_pressed)
	terrain_main_btn.pressed.connect(_on_terrain_main_pressed)
	zoning_main_btn.pressed.connect(_on_zoning_main_pressed)
	select_main_btn.pressed.connect(_on_select_main_pressed)
	
	road_2l_btn.pressed.connect(func(): _select_road_type(1, 1))
	road_4l_btn.pressed.connect(func(): _select_road_type(2, 2))
	walkway_btn.pressed.connect(func(): _select_road_type(0, 0))
	
	straight_btn.pressed.connect(func(): _set_draw_mode(0))
	spline_btn.pressed.connect(func(): _set_draw_mode(1))

func _on_road_main_pressed():
	terrain_sub_menu.visible = false
	zoning_combined_hbox.visible = false
	road_combined_hbox.visible = !road_combined_hbox.visible
	if not road_combined_hbox.visible:
		get_meta("options_panel").visible = false
		get_meta("road_sep").visible = false
		input_manager._cancel_active_tool()

func _on_terrain_main_pressed():
	road_combined_hbox.visible = false
	zoning_combined_hbox.visible = false
	terrain_sub_menu.visible = !terrain_sub_menu.visible
	if not terrain_sub_menu.visible:
		input_manager._cancel_active_tool()

func _on_zoning_main_pressed():
	road_combined_hbox.visible = false
	terrain_sub_menu.visible = false
	zoning_combined_hbox.visible = !zoning_combined_hbox.visible
	input_manager._toggle_zoning_overlay()

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
	input_manager._toggle_tool(InputManager.Tool.SELECT)

## Shows the road properties panel for one or more selected edges.
## When multiple edges are selected the warning is suppressed and the
## "No buildings" checkbox reflects whether ALL selected edges have the flag set.
func show_road_properties_multi(edge_indices: Array):
	if edge_indices.is_empty(): return
	road_properties_panel.visible = true

	var warning = road_properties_panel.get_meta("warning_label") as Label
	var no_build_check = road_properties_panel.get_meta("no_build_check") as CheckBox
	var class_btns: Array = road_properties_panel.get_meta("class_btns")

	if edge_indices.size() == 1:
		var edge_idx: int = edge_indices[0]
		warning.text = ""
		no_build_check.set_pressed_no_signal(simulation_node.get_no_building_spawn(edge_idx))
		# Reflect current class
		var cur_class: int = simulation_node.get_edge_class(edge_idx)
		for j in range(class_btns.size()):
			class_btns[j].set_pressed_no_signal(j == cur_class)

		var geo = simulation_node.get_edge_geometry_3d(edge_idx)
		if geo.size() >= 2:
			var p0 = geo[0]; var p1 = geo[-1]
			var h0 = simulation_node.get_height_at(Vector2(p0.x, p0.z))
			var h1 = simulation_node.get_height_at(Vector2(p1.x, p1.z))
			var t_mid = geo[geo.size() / 2]
			var h_mid = simulation_node.get_height_at(Vector2(t_mid.x, t_mid.z))
			if p0.y > h0 + 1.0 and p1.y > h1 + 1.0:
				if t_mid.y < h_mid + 2.0:
					warning.text = "Warning: Bridge may clash with terrain!"
			elif p0.y < h0 - 1.0 and p1.y < h1 - 1.0:
				if t_mid.y > h_mid - 2.0:
					warning.text = "Warning: Tunnel might be above surface!"
	else:
		warning.text = "%d edges selected" % edge_indices.size()
		var all_no_build: bool = edge_indices.all(
			func(idx): return simulation_node.get_no_building_spawn(idx))
		no_build_check.set_pressed_no_signal(all_no_build)
		# Class buttons: depress the shared class if all edges have the same one.
		var first_class: int = simulation_node.get_edge_class(edge_indices[0])
		var shared: bool = edge_indices.all(
			func(idx): return simulation_node.get_edge_class(idx) == first_class)
		for j in range(class_btns.size()):
			class_btns[j].set_pressed_no_signal(shared and j == first_class)

func hide_road_properties():
	road_properties_panel.visible = false

var _pack_manager: Window = null

func _on_mods_btn_pressed() -> void:
	if not _pack_manager:
		_pack_manager = PackManager.new()
		add_child(_pack_manager)
	_pack_manager.popup_centered()
