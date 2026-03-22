extends CanvasLayer

const InputManager = preload("res://scripts/input_manager.gd")

@onready var input_manager = $"../InputManager"
@onready var road_tool = $"../RoadTool"

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
var zoning_options_menu: VBoxContainer
var zoning_single_btn: Button
var zoning_paint_btn: Button
var zoning_delete_btn: Button
var zoning_marquee_btn: Button
var zoning_fill_btn: Button
var zoning_combined_hbox: HBoxContainer
var zoning_sub_menu: HBoxContainer

# Road zoning options
var road_zoning_left_btn: Button
var road_zoning_right_btn: Button

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
	
	var road_sep = HSeparator.new()
	road_options_menu.add_child(road_sep)
	
	road_zoning_left_btn = Button.new()
	road_zoning_left_btn.text = "Zone L"
	road_zoning_left_btn.toggle_mode = true
	road_zoning_left_btn.button_pressed = true
	road_options_menu.add_child(road_zoning_left_btn)
	
	road_zoning_right_btn = Button.new()
	road_zoning_right_btn.text = "Zone R"
	road_zoning_right_btn.toggle_mode = true
	road_zoning_right_btn.button_pressed = true
	road_options_menu.add_child(road_zoning_right_btn)
	
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

	# 1. Zoning Options Panel
	var z_options_panel = PanelContainer.new()
	var z_op_style = StyleBoxFlat.new()
	z_op_style.bg_color = Color(0.15, 0.15, 0.15, 0.7)
	z_op_style.set_corner_radius_all(10)
	z_options_panel.add_theme_stylebox_override("panel", z_op_style)
	
	var z_options_padding = MarginContainer.new()
	z_options_padding.add_theme_constant_override("margin_left", 8)
	z_options_padding.add_theme_constant_override("margin_right", 8)
	z_options_padding.add_theme_constant_override("margin_top", 5)
	z_options_padding.add_theme_constant_override("margin_bottom", 5)
	z_options_panel.add_child(z_options_padding)
	
	zoning_options_menu = VBoxContainer.new()
	z_options_padding.add_child(zoning_options_menu)
	
	zoning_single_btn = Button.new()
	zoning_single_btn.text = "Single"
	zoning_single_btn.toggle_mode = true
	zoning_single_btn.button_pressed = true 
	zoning_options_menu.add_child(zoning_single_btn)

	zoning_paint_btn = Button.new()
	zoning_paint_btn.text = "Paint"
	zoning_paint_btn.toggle_mode = true
	zoning_options_menu.add_child(zoning_paint_btn)
	
	zoning_fill_btn = Button.new()
	zoning_fill_btn.text = "Fill"
	zoning_fill_btn.toggle_mode = true
	zoning_options_menu.add_child(zoning_fill_btn)

	zoning_delete_btn = Button.new()
	zoning_delete_btn.text = "Delete road zones"
	zoning_delete_btn.toggle_mode = true
	zoning_options_menu.add_child(zoning_delete_btn)
	
	zoning_combined_hbox.add_child(z_options_panel)

	var z_modes = [zoning_single_btn, zoning_paint_btn, zoning_fill_btn, zoning_delete_btn]
	for btn in z_modes:
		btn.pressed.connect(func():
			for b in z_modes: b.set_pressed_no_signal(false)
			btn.set_pressed_no_signal(true)
			if input_manager.zoning_tool:
				input_manager.zoning_tool.current_mode = z_modes.find(btn)
		)

	# 2. Zoning Types Panel
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
			if input_manager.zoning_tool:
				# If we are in DELETE mode, switch back to SINGLE mode when selecting a type
				if input_manager.zoning_tool.current_mode == 3: # DELETE is 3
					input_manager.zoning_tool.current_mode = 0 # SINGLE is 0
					# Update UI buttons
					for btn in z_modes: btn.set_pressed_no_signal(false)
					zoning_single_btn.set_pressed_no_signal(true)
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
	
	# Wrapper to center main toolbar
	var hbox_main_center = HBoxContainer.new()
	hbox_main_center.alignment = BoxContainer.ALIGNMENT_CENTER
	hbox_main_center.add_child(main_toolbar)
	
	vbox.add_child(hbox_main_center)

func _connect_signals():
	road_main_btn.pressed.connect(_on_road_main_pressed)
	terrain_main_btn.pressed.connect(_on_terrain_main_pressed)
	zoning_main_btn.pressed.connect(_on_zoning_main_pressed)
	
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
