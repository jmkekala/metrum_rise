## Shared top menu bar for gameplay and editor scenes.
##
## The menu is attached by each scene root and dispatches to that root's
## menu_* helpers so gameplay and editor flows can stay scene-specific.
extends CanvasLayer

const UIStyle = preload("res://scripts/ui/ui_style.gd")

const BAR_HEIGHT := 28

const SCENE_GAMEPLAY := "gameplay"
const SCENE_ASSET_EDITOR := "asset_editor"
const SCENE_ECONOMY_EDITOR := "economy_editor"
const SCENE_WORLD_EDITOR := "world_editor"

enum ActionId {
	FILE_NEW_GAME = 1,
	FILE_SAVE = 2,
	FILE_LOAD = 3,
	FILE_RETURN_TO_GAME = 4,
	FILE_QUIT = 5,
	FILE_NEW_WORLD = 6,
	FILE_OPEN_WORLD = 7,
	FILE_SAVE_AS = 8,
	VIEW_TOGGLE_ZONING = 10,
	VIEW_OVERLAY_NONE = 11,
	VIEW_OVERLAY_POLLUTION = 12,
	VIEW_OVERLAY_NOISE = 13,
	VIEW_OVERLAY_DESIRABILITY = 14,
	CITY_STATS = 20,
	CITY_ECONOMY = 21,
	CITY_DEMAND = 22,
	TOOLS_OPEN_ASSET_EDITOR = 30,
	TOOLS_OPEN_ECONOMY_EDITOR = 31,
	HELP_SHORTCUTS = 40,
	HELP_ABOUT = 41,
	ASSET_RELOAD_PACKS = 50,
	ASSET_IMPORT_MESH = 51,
	ECONOMY_RELOAD = 60,
	ECONOMY_RUN_SANDBOX = 61,
}

var scene_kind: String = SCENE_GAMEPLAY

var _scene_root: Node
var _windows: Dictionary = {}

func _ready() -> void:
	_scene_root = get_parent()
	if scene_kind.is_empty():
		scene_kind = _detect_scene_kind()
	_build_menu_bar()

func _build_menu_bar() -> void:
	var root := Control.new()
	add_child(root)
	root.set_anchors_and_offsets_preset(Control.PRESET_FULL_RECT)
	root.mouse_filter = Control.MOUSE_FILTER_IGNORE

	var shell := PanelContainer.new()
	root.add_child(shell)
	shell.set_anchors_and_offsets_preset(Control.PRESET_TOP_WIDE)
	shell.offset_bottom = BAR_HEIGHT
	shell.mouse_filter = Control.MOUSE_FILTER_STOP
	shell.add_theme_stylebox_override("panel", UIStyle.panel_style(UIStyle.BG_DARK, 0))

	var menu_bar := MenuBar.new()
	shell.add_child(menu_bar)
	menu_bar.set_anchors_and_offsets_preset(Control.PRESET_FULL_RECT)
	menu_bar.prefer_global_menu = false
	menu_bar.flat = true
	menu_bar.add_theme_color_override("font_color", UIStyle.TEXT_PRIMARY)
	menu_bar.add_theme_color_override("font_hover_color", UIStyle.TEXT_PRIMARY)
	menu_bar.add_theme_color_override("font_focus_color", UIStyle.TEXT_PRIMARY)
	menu_bar.add_theme_color_override("font_pressed_color", UIStyle.TEXT_PRIMARY)
	menu_bar.add_theme_color_override("font_disabled_color", UIStyle.TEXT_DIM)

	match scene_kind:
		SCENE_GAMEPLAY:
			_build_gameplay_menus(menu_bar)
		SCENE_ASSET_EDITOR:
			_build_asset_editor_menus(menu_bar)
		SCENE_ECONOMY_EDITOR:
			_build_economy_editor_menus(menu_bar)
		SCENE_WORLD_EDITOR:
			_build_world_editor_menus(menu_bar)

func _build_gameplay_menus(menu_bar: MenuBar) -> void:
	var file_popup := _add_menu_popup(menu_bar, "File")
	file_popup.add_item("New Game", ActionId.FILE_NEW_GAME)
	file_popup.add_item("Load [Ctrl+L]", ActionId.FILE_LOAD)
	file_popup.add_item("Save [Ctrl+S]", ActionId.FILE_SAVE)
	file_popup.add_separator()
	file_popup.add_item("Quit", ActionId.FILE_QUIT)
	file_popup.id_pressed.connect(_on_file_menu_pressed)

	var view_popup := _add_menu_popup(menu_bar, "View")
	var overlays_popup := _create_popup_menu("Overlays")
	view_popup.add_submenu_node_item("Overlays", overlays_popup)
	view_popup.add_separator()
	view_popup.add_item("Toggle Zoning Overlay", ActionId.VIEW_TOGGLE_ZONING)
	view_popup.id_pressed.connect(_on_view_menu_pressed)
	overlays_popup.add_item("None [7]", ActionId.VIEW_OVERLAY_NONE)
	overlays_popup.add_item("Pollution [8]", ActionId.VIEW_OVERLAY_POLLUTION)
	overlays_popup.add_item("Noise [9]", ActionId.VIEW_OVERLAY_NOISE)
	overlays_popup.add_item("Desirability [0]", ActionId.VIEW_OVERLAY_DESIRABILITY)
	overlays_popup.id_pressed.connect(_on_view_menu_pressed)

	var city_popup := _add_menu_popup(menu_bar, "City")
	city_popup.add_item("City Statistics", ActionId.CITY_STATS)
	city_popup.add_item("Economy Overview", ActionId.CITY_ECONOMY)
	city_popup.add_item("Demand Overview", ActionId.CITY_DEMAND)
	city_popup.id_pressed.connect(_on_city_menu_pressed)

	var tools_popup := _add_menu_popup(menu_bar, "Tools")
	tools_popup.add_item("Open Asset Editor", ActionId.TOOLS_OPEN_ASSET_EDITOR)
	tools_popup.add_item("Open Economy Editor", ActionId.TOOLS_OPEN_ECONOMY_EDITOR)
	tools_popup.id_pressed.connect(_on_tools_menu_pressed)

	var help_popup := _add_menu_popup(menu_bar, "Help")
	help_popup.add_item("Keyboard Shortcuts", ActionId.HELP_SHORTCUTS)
	help_popup.add_item("About", ActionId.HELP_ABOUT)
	help_popup.id_pressed.connect(_on_help_menu_pressed)

func _build_asset_editor_menus(menu_bar: MenuBar) -> void:
	var file_popup := _add_menu_popup(menu_bar, "File")
	file_popup.add_item("Save [Ctrl+S]", ActionId.FILE_SAVE)
	file_popup.add_separator()
	file_popup.add_item("Return To Game", ActionId.FILE_RETURN_TO_GAME)
	file_popup.add_item("Quit", ActionId.FILE_QUIT)
	file_popup.id_pressed.connect(_on_file_menu_pressed)

	var asset_popup := _add_menu_popup(menu_bar, "Asset")
	asset_popup.add_item("Reload Packs", ActionId.ASSET_RELOAD_PACKS)
	asset_popup.add_item("Import Mesh...", ActionId.ASSET_IMPORT_MESH)
	asset_popup.id_pressed.connect(_on_asset_menu_pressed)

func _build_economy_editor_menus(menu_bar: MenuBar) -> void:
	var file_popup := _add_menu_popup(menu_bar, "File")
	file_popup.add_item("Save [Ctrl+S]", ActionId.FILE_SAVE)
	file_popup.add_separator()
	file_popup.add_item("Return To Game", ActionId.FILE_RETURN_TO_GAME)
	file_popup.add_item("Quit", ActionId.FILE_QUIT)
	file_popup.id_pressed.connect(_on_file_menu_pressed)

	var economy_popup := _add_menu_popup(menu_bar, "Economy")
	economy_popup.add_item("Reload Project", ActionId.ECONOMY_RELOAD)
	economy_popup.add_item("Run Sandbox", ActionId.ECONOMY_RUN_SANDBOX)
	economy_popup.id_pressed.connect(_on_economy_menu_pressed)

func _build_world_editor_menus(menu_bar: MenuBar) -> void:
	var file_popup := _add_menu_popup(menu_bar, "File")
	file_popup.add_item("New World [Ctrl+N]", ActionId.FILE_NEW_WORLD)
	file_popup.add_item("Open World [Ctrl+O]", ActionId.FILE_OPEN_WORLD)
	file_popup.add_item("Save [Ctrl+S]", ActionId.FILE_SAVE)
	file_popup.add_item("Save As...", ActionId.FILE_SAVE_AS)
	file_popup.add_separator()
	file_popup.add_item("Quit", ActionId.FILE_QUIT)
	file_popup.id_pressed.connect(_on_file_menu_pressed)

	var help_popup := _add_menu_popup(menu_bar, "Help")
	help_popup.add_item("Keyboard Shortcuts", ActionId.HELP_SHORTCUTS)
	help_popup.add_item("About", ActionId.HELP_ABOUT)
	help_popup.id_pressed.connect(_on_help_menu_pressed)

func _add_menu_popup(menu_bar: MenuBar, label: String) -> PopupMenu:
	var popup := _create_popup_menu(label)
	menu_bar.add_child(popup)
	return popup

func _create_popup_menu(label: String) -> PopupMenu:
	var popup := PopupMenu.new()
	popup.title = label
	popup.name = label.replace(" ", "")
	popup.prefer_native_menu = false
	popup.add_theme_color_override("font_color", UIStyle.TEXT_PRIMARY)
	popup.add_theme_color_override("font_hover_color", UIStyle.TEXT_PRIMARY)
	popup.add_theme_color_override("font_disabled_color", UIStyle.TEXT_DIM)
	return popup

func _on_file_menu_pressed(id: int) -> void:
	match id:
		ActionId.FILE_NEW_GAME:
			if _scene_root and _scene_root.has_method("menu_new_game"):
				_scene_root.menu_new_game()
		ActionId.FILE_NEW_WORLD:
			if _scene_root and _scene_root.has_method("menu_new_world"):
				_scene_root.menu_new_world()
		ActionId.FILE_OPEN_WORLD:
			if _scene_root and _scene_root.has_method("menu_open_world"):
				_scene_root.menu_open_world()
		ActionId.FILE_SAVE:
			if _scene_root and _scene_root.has_method("menu_save"):
				_scene_root.menu_save()
		ActionId.FILE_SAVE_AS:
			if _scene_root and _scene_root.has_method("menu_save_as"):
				_scene_root.menu_save_as()
		ActionId.FILE_LOAD:
			if _scene_root and _scene_root.has_method("menu_load"):
				_scene_root.menu_load()
		ActionId.FILE_RETURN_TO_GAME:
			if _scene_root and _scene_root.has_method("menu_return_to_game"):
				_scene_root.menu_return_to_game()
		ActionId.FILE_QUIT:
			get_tree().quit()

func _on_view_menu_pressed(id: int) -> void:
	match id:
		ActionId.VIEW_TOGGLE_ZONING:
			if _scene_root and _scene_root.has_method("menu_toggle_zoning_overlay"):
				_scene_root.menu_toggle_zoning_overlay()
		ActionId.VIEW_OVERLAY_NONE:
			_set_overlay_mode(0)
		ActionId.VIEW_OVERLAY_POLLUTION:
			_set_overlay_mode(1)
		ActionId.VIEW_OVERLAY_NOISE:
			_set_overlay_mode(2)
		ActionId.VIEW_OVERLAY_DESIRABILITY:
			_set_overlay_mode(3)

func _on_city_menu_pressed(id: int) -> void:
	match id:
		ActionId.CITY_STATS:
			_open_window(_ensure_text_window(
				"city_stats",
				"City Statistics",
				[
					"Placeholder window",
					"",
					"Live population, housing, budget, and utility status",
					"will be wired to SimulationNode in a follow-up pass."
				],
				Vector2i(420, 220)
			))
		ActionId.CITY_ECONOMY:
			_open_window(_ensure_text_window(
				"economy_overview",
				"Economy Overview",
				[
					"Placeholder window",
					"",
					"Commercial and industrial operating budgets,",
					"OWA ratios, and freight-facing signals belong here."
				],
				Vector2i(440, 220)
			))
		ActionId.CITY_DEMAND:
			_open_window(_ensure_text_window(
				"demand_overview",
				"Demand Overview",
				[
					"Placeholder window",
					"",
					"Residential, commercial, and industrial demand",
					"signals will be surfaced here in a later pass."
				],
				Vector2i(440, 220)
			))

func _on_tools_menu_pressed(id: int) -> void:
	match id:
		ActionId.TOOLS_OPEN_ASSET_EDITOR:
			if _scene_root and _scene_root.has_method("menu_open_asset_editor"):
				_scene_root.menu_open_asset_editor()
		ActionId.TOOLS_OPEN_ECONOMY_EDITOR:
			if _scene_root and _scene_root.has_method("menu_open_economy_editor"):
				_scene_root.menu_open_economy_editor()

func _on_help_menu_pressed(id: int) -> void:
	match id:
		ActionId.HELP_SHORTCUTS:
			if scene_kind == SCENE_WORLD_EDITOR:
				_open_window(_ensure_text_window(
					"keyboard_shortcuts",
					"Keyboard Shortcuts",
					[
						"1  Raise terrain tool",
						"2  Lower terrain tool",
						"3  Water source tool",
						"4  Water sink tool",
						"5  Lake fill tool",
						"6  Open water tool",
						"Left Mouse  Sculpt terrain",
						"Lake / Open Water: click once to preview, click again to confirm",
						"Shift+Left Mouse  Remove nearest authored water feature",
						"Escape  Cancel surface-fill preview / clear active tool",
						"Middle Mouse  Orbit camera",
						"Right Mouse  Pan camera",
						"W / A / S / D  Pan camera",
						"Mouse Wheel  Zoom camera",
						"Ctrl+N  New world",
						"Ctrl+O  Open world",
						"Ctrl+S  Save world"
					],
					Vector2i(440, 360)
				))
				return
			_open_window(_ensure_text_window(
				"keyboard_shortcuts",
				"Keyboard Shortcuts",
				[
					"R  Road tool",
					"X  Walkway tool",
					"Z  Zoning tool",
					"M  Move tool",
					"V  Select tool",
					"C  Cul-de-sac tool",
					"Y  Terrain sculpt",
					"K  Water source tool",
					"Space  Pause / unpause",
					"Escape  Cancel active tool",
					"Ctrl+S  Save",
					"Ctrl+L  Load",
					"Ctrl+Z  Undo",
					"7 / 8 / 9 / 0  Overlay modes"
				],
				Vector2i(380, 340)
			))
		ActionId.HELP_ABOUT:
			_open_window(_ensure_text_window(
				"about",
				"About Metrum Rise",
				_about_lines(),
				Vector2i(420, 220)
			))

func _on_asset_menu_pressed(id: int) -> void:
	match id:
		ActionId.ASSET_RELOAD_PACKS:
			if _scene_root and _scene_root.has_method("menu_reload_packs"):
				_scene_root.menu_reload_packs()
		ActionId.ASSET_IMPORT_MESH:
			if _scene_root and _scene_root.has_method("menu_import_mesh"):
				_scene_root.menu_import_mesh()

func _on_economy_menu_pressed(id: int) -> void:
	match id:
		ActionId.ECONOMY_RELOAD:
			if _scene_root and _scene_root.has_method("menu_reload_project"):
				_scene_root.menu_reload_project()
		ActionId.ECONOMY_RUN_SANDBOX:
			if _scene_root and _scene_root.has_method("menu_run_sandbox"):
				_scene_root.menu_run_sandbox()

func _set_overlay_mode(mode: int) -> void:
	if _scene_root and _scene_root.has_method("menu_set_overlay_mode"):
		_scene_root.menu_set_overlay_mode(mode)

func _ensure_text_window(key: String, title: String, lines: Array[String], size: Vector2i) -> Window:
	if _windows.has(key):
		return _windows[key]

	var window := Window.new()
	window.title = title
	window.size = size
	window.unresizable = false
	window.exclusive = false
	window.close_requested.connect(window.hide)

	var body := PanelContainer.new()
	body.set_anchors_and_offsets_preset(Control.PRESET_FULL_RECT)
	body.add_theme_stylebox_override("panel", UIStyle.window_body_style())
	window.add_child(body)

	var margin := MarginContainer.new()
	margin.set_anchors_and_offsets_preset(Control.PRESET_FULL_RECT)
	margin.add_theme_constant_override("margin_left", UIStyle.PAD_WINDOW)
	margin.add_theme_constant_override("margin_right", UIStyle.PAD_WINDOW)
	margin.add_theme_constant_override("margin_top", UIStyle.PAD_WINDOW)
	margin.add_theme_constant_override("margin_bottom", UIStyle.PAD_WINDOW)
	body.add_child(margin)

	var text := RichTextLabel.new()
	text.bbcode_enabled = false
	text.scroll_active = true
	text.selection_enabled = true
	text.autowrap_mode = TextServer.AUTOWRAP_WORD_SMART
	text.fit_content = false
	text.size_flags_horizontal = Control.SIZE_EXPAND_FILL
	text.size_flags_vertical = Control.SIZE_EXPAND_FILL
	text.add_theme_color_override("default_color", UIStyle.TEXT_PRIMARY)
	text.text = "\n".join(lines)
	margin.add_child(text)

	add_child(window)
	_windows[key] = window
	return window

func _open_window(window: Window) -> void:
	if not window.has_meta("opened_once"):
		window.popup_centered()
		window.set_meta("opened_once", true)
	else:
		window.show()
		window.grab_focus()

func _detect_scene_kind() -> String:
	var simulation_node := _scene_root.get_node_or_null("SimulationNode")
	if simulation_node:
		if simulation_node.has_method("is_asset_editor_mode") and simulation_node.is_asset_editor_mode():
			return SCENE_ASSET_EDITOR
		if simulation_node.has_method("is_economy_editor_mode") and simulation_node.is_economy_editor_mode():
			return SCENE_ECONOMY_EDITOR
		if simulation_node.has_method("is_world_editor_mode") and simulation_node.is_world_editor_mode():
			return SCENE_WORLD_EDITOR
	return SCENE_GAMEPLAY

func _about_lines() -> Array[String]:
	if scene_kind == SCENE_WORLD_EDITOR:
		return [
			"Metrum Rise World Editor",
			"",
			"Terrain-first blank-world authoring shell backed by the",
			"shared Rust SimulationNode runtime and WorldDefinition assets.",
			"",
			"This top menu is shared across gameplay and editor shells."
		]
	return [
		"Metrum Rise",
		"",
		"Large-scale city simulation with a Rust simulation backend",
		"loaded into a Godot 4 frontend through GDExtension.",
		"",
		"This top menu is shared across gameplay and editor shells."
	]
