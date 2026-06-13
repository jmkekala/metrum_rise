## Asset editor shell — launched via `--asset-editor` command-line argument.
## Shares the same SimulationNode and compiled .so as the game, but runs a
## 500 m sandbox with no agents, no demand simulation, and no background tick thread.
## Calls: sim.is_asset_editor_mode(), sim.load_asset_packs(dir, filter),
##        sim.get_registered_asset_ids(), sim.validate_and_export_asset(),
##        sim.get_asset_manifest_json(), sim.get_pack_manifest_json(),
##        sim.load_economy_project()
extends Node3D

const TopMenu = preload("res://scripts/ui/top_menu.gd")
const MeshImportDialog = preload("res://scripts/editors/mesh_import_dialog.gd")
const EditorTheme = preload("res://scripts/ui/editor_theme.gd")

const PANEL_LEFT_W  := 360
const PANEL_RIGHT_W := 420
const PANEL_BOT_H   := 140
const PANEL_LEFT_MIN_W := 240
const PANEL_RIGHT_MIN_W := 340
const PANEL_BOTTOM_MIN_H := 96
const MAIN_ENTRANCE_PICK_RADIUS_PX := 18.0
const PANEL_PAD := 8
const PANEL_GAP := 6
const ASSET_CONTEXT_USE_AS_GHOST := 1
const PACK_MENU_CREATE_NEW := 1000000
const PACK_MENU_NO_PACKS := 1000001
const PACK_SCHEMA_VERSION := 1
const MESH_ROTATION_DRAG_DEG_PER_PX := 0.35
const MESH_ROTATION_CARDINAL_SNAP_DEG := 4.0
const PLACEMENT_MODES := [
	{"id": "zoned_private", "label": "Zoned Private"},
	{"id": "explicit", "label": "Explicit"},
]
const SERVICE_CLASSES := [
	{"id": "none", "label": "None"},
	{"id": "police", "label": "Police"},
	{"id": "fire", "label": "Fire"},
	{"id": "healthcare", "label": "Healthcare"},
	{"id": "education", "label": "Education"},
	{"id": "power", "label": "Power"},
	{"id": "water", "label": "Water"},
	{"id": "waste", "label": "Waste"},
	{"id": "transit", "label": "Transit"},
	{"id": "parks", "label": "Parks"},
	{"id": "government", "label": "Government"},
]
const UTILITY_PROFILE_BY_SERVICE := {
	"power": "power_plant_basic",
	"water": "water_plant_basic",
	"waste": "wastewater_treatment_basic",
}

const TEMPLATES := [
	"Flat Studio",
	"Zoned Roadside",
	"Lane Reference",
	"Traffic Comparison",
	"Night Lighting",
]

@onready var sim: SimulationNode = $SimulationNode

# ── UI refs ───────────────────────────────────────────────────────────────────
var _log_label: RichTextLabel
var _asset_tree: Tree
var _asset_search_edit: LineEdit
var _asset_count_lbl: Label
var _asset_context_menu: PopupMenu
var _asset_context_asset_id: String = ""
var _template_btn: OptionButton
var _main_vsplit: VSplitContainer
var _left_split: HSplitContainer
var _right_split: HSplitContainer
var _preview_view_rect: Control
var _theme_root: Control
var _top_menu: Node
var _layout_restoring: bool = false

# Inspector – pack
var _selected_pack_id: String = "my-pack"
var _selected_pack_name: String = "My Pack"
var _selected_pack_author: String = ""
var _pack_summary_lbl: Label
var _pack_set_btn: Button
var _pack_select_menu: PopupMenu
var _pack_create_window: Window
var _new_pack_id_edit: LineEdit
var _new_pack_name_edit: LineEdit
var _new_pack_author_edit: LineEdit
var _known_packs: Array[Dictionary] = []
var _loaded_asset_pack_id: String = ""
var _loaded_asset_id: String = ""
var _retarget_export_window: Window
var _retarget_export_message_lbl: Label

# Inspector – asset
var _asset_id_edit: LineEdit
var _display_name_edit: LineEdit
var _asset_set_edit: LineEdit
var _tags_edit: LineEdit
var _placement_mode_btn: OptionButton
var _zone_type_btn: OptionButton
var _density_btn: OptionButton
var _zoned_only_box: VBoxContainer
var _width_spin: SpinBox
var _depth_spin: SpinBox
var _min_zone_width_spin: SpinBox
var _min_zone_depth_spin: SpinBox
var _level_spin: SpinBox
var _residents_spin: SpinBox
var _flat_size_spin: SpinBox
var _workers_spin: SpinBox
var _service_class_btn: OptionButton
var _economy_profile_btn: OptionButton
var _economy_profile_status_lbl: Label
var _lod_list: ItemList
var _lod_source_paths: Array[String] = []  # one source mesh path per mesh part
var _part_positions: Array[Vector3] = []
var _part_rotation_y: Array[float] = []
var _part_scales: Array[float] = []
var _part_pivot_offsets: Array[Vector3] = []
var _part_aabbs: Array[AABB] = []
var _selected_part_index: int = -1
var _frontage_lbl: Label  # shows current frontage forward vector
var _entrance_x_spin: SpinBox
var _entrance_y_spin: SpinBox
var _entrance_z_spin: SpinBox
var _glb_path: String = ""

# 3D preview node
var _preview: Node  # BuildingPreview instance
var _cam_input: Node  # EditorCameraInput instance

# Current frontage forward (updated by Set Front From View)
var _frontage_fwd: Vector3 = Vector3.FORWARD

# Last loaded mesh AABB (used by auto-fit and pivot computation).
var _mesh_aabb: AABB = AABB()
# Pivot offset in model units: centres XZ, grounds Y. Written to asset.toml.
var _pivot_offset: Vector3 = Vector3.ZERO

# Inspector – preview scale
var _preview_scale_spin: SpinBox
var _part_x_spin: SpinBox
var _part_y_spin: SpinBox
var _part_z_spin: SpinBox
var _part_rotation_y_spin: SpinBox
var _dim_label: Label          # live "→ W × D × H m" display
var _scale_preset_btn: OptionButton
var _human_btn: CheckButton

# Registered asset IDs, refreshed after pack load.
var _asset_ids: Array[String] = []
var _asset_display_names: Dictionary = {}
var _asset_id_auto: bool = true  # false once the user manually edits the ID field
var _autofit_on_load: bool = false   # true when loading from browser with scale=1 (never fitted)
var _keep_camera: bool = false       # true when loading from browser — skip focus_on
var _human_visible: bool = false     # mirrors the human figure toggle state
var _suppress_preview_scale_changed: bool = false
var _suppress_part_transform_changed: bool = false
var _last_glb_dir: String = ""     # last directory used in GLB file dialogs
var _config: ConfigFile            # persistent editor preferences
var _theme_mode: String = EditorTheme.MODE_DARK
var _font_size_header:  int = 14   # section title labels ("Asset Browser", "Building Importer")
var _font_size_section: int = 12   # sub-section labels ("Pack", "Asset", "Building", etc.)
var _font_size_label:   int = 11   # spinbox labels and small info text
var _economy_profile_ids: Array[String] = []
var _economy_profiles_cache: Dictionary = {}
var _unresolved_economy_profile_id: String = ""
var _economy_catalog_loaded: bool = false
var _economy_catalog_warning_count: int = 0
var _economy_catalog_error: String = ""
var _log_plain_lines: Array[String] = []
var _bbcode_strip_regex: RegEx
var _main_entrance_auto: bool = true
var _updating_main_entrance_fields: bool = false
var _extra_anchors: Array[Dictionary] = []
var _dragging_main_entrance: bool = false
var _dragging_mesh_part: bool = false
var _rotating_mesh_part: bool = false
var _mesh_part_drag_offset: Vector3 = Vector3.ZERO
var _mesh_part_rotate_start_x: float = 0.0
var _mesh_part_rotate_start_yaw: float = 0.0
var _dragging_ghost: bool = false
var _ghost_drag_offset: Vector3 = Vector3.ZERO
var _zoning_profiles: Array[Dictionary] = []
var _zone_types: Array[String] = []
var _density_types_by_zone: Dictionary = {}
var _last_lot_width_cells: int = 1
var _last_lot_depth_cells: int = 1

# ──────────────────────────────────────────────────────────────────────────────

const CONFIG_PATH := "user://asset_editor.cfg"

func _ready() -> void:
	if not sim.is_asset_editor_mode():
		push_error("AssetEditor scene loaded without --asset-editor flag")

	_config = ConfigFile.new()
	_config.load(CONFIG_PATH)  # silently no-ops if file doesn't exist yet
	_theme_mode = EditorTheme.normalize_mode(str(_config.get_value("ui", "theme_mode", EditorTheme.MODE_DARK)))
	_last_glb_dir = _config.get_value("import", "last_glb_dir", "")

	_font_size_header  = _config.get_value("ui", "font_size_header",  14)
	_font_size_section = _config.get_value("ui", "font_size_section", 12)
	_font_size_label   = _config.get_value("ui", "font_size_label",   11)
	_save_config()  # write defaults if keys are missing
	_restore_window_geometry()

	_attach_top_menu()
	_configure_preview_environment()
	_load_zone_profiles()
	_build_preview_node()
	_build_ui()
	_bbcode_strip_regex = RegEx.new()
	_bbcode_strip_regex.compile("\\[/?[^\\]]+\\]")
	_set_frontage_forward(_frontage_fwd)
	_set_main_entrance_position(_default_main_entrance_position(), true)
	_load_economy_profiles()
	_load_packs()
	_apply_template(0)

func _configure_preview_environment() -> void:
	var world_environment := find_child("WorldEnvironment", true, false) as WorldEnvironment
	if not world_environment:
		return
	world_environment.environment = EditorTheme.preview_environment(_theme_mode)

func _load_zone_profiles() -> void:
	_zoning_profiles.clear()
	_zone_types.clear()
	_density_types_by_zone.clear()

	var payload = sim.get_zone_profiles()
	if payload is Array:
		for entry in payload:
			if not (entry is Dictionary):
				continue
			var profile: Dictionary = entry
			var zone_type := str(profile.get("zone_type", "")).strip_edges()
			var density := str(profile.get("density", "")).strip_edges()
			if zone_type.is_empty() or density.is_empty():
				continue
			_zoning_profiles.append(profile.duplicate(true))
			if not _zone_types.has(zone_type):
				_zone_types.append(zone_type)
			var densities: Array = _density_types_by_zone.get(zone_type, [])
			if not densities.has(density):
				densities.append(density)
				densities.sort()
			_density_types_by_zone[zone_type] = densities

	if _zone_types.is_empty():
		_zone_types = ["residential", "commercial", "industrial"]
		_density_types_by_zone = {
			"residential": ["low", "medium", "high"],
			"commercial": ["low", "medium", "high"],
			"industrial": ["low", "medium", "high"],
		}

# ──────────────────────────────────────────────────────────────────────────────
# 3D preview
# ──────────────────────────────────────────────────────────────────────────────

func _build_preview_node() -> void:
	var script := load("res://scripts/renderers/building_preview.gd")
	_preview = Node3D.new()
	_preview.set_script(script)
	add_child(_preview)
	_preview.mesh_loaded.connect(_on_mesh_loaded)

	var cam_script := load("res://scripts/core/editor_camera_input.gd")
	_cam_input = Node.new()
	_cam_input.set_script(cam_script)
	_cam_input.right_mouse_pan_enabled = false
	add_child(_cam_input)

# ──────────────────────────────────────────────────────────────────────────────
# UI construction
# ──────────────────────────────────────────────────────────────────────────────

func _build_ui() -> void:
	var canvas := CanvasLayer.new()
	add_child(canvas)

	# Anchor wrapper — fills the full viewport via PRESET_FULL_RECT so the VBox
	# gets a concrete rect to size against.
	var anchor := Control.new()
	anchor.set_anchors_preset(Control.PRESET_FULL_RECT)
	anchor.offset_top = TopMenu.BAR_HEIGHT
	canvas.add_child(anchor)
	_theme_root = anchor

	_main_vsplit = VSplitContainer.new()
	_main_vsplit.set_anchors_preset(Control.PRESET_FULL_RECT)
	anchor.add_child(_main_vsplit)

	_left_split = HSplitContainer.new()
	_left_split.size_flags_vertical = Control.SIZE_EXPAND_FILL
	_main_vsplit.add_child(_left_split)

	_build_left_panel(_left_split)

	_right_split = HSplitContainer.new()
	_right_split.size_flags_horizontal = Control.SIZE_EXPAND_FILL
	_right_split.size_flags_vertical = Control.SIZE_EXPAND_FILL
	_left_split.add_child(_right_split)

	# Center spacer — 3D viewport shows through. It also becomes the authoritative
	# hit rect for camera and preview-anchor mouse interaction.
	var center := Control.new()
	center.size_flags_horizontal = Control.SIZE_EXPAND_FILL
	center.size_flags_vertical = Control.SIZE_EXPAND_FILL
	center.mouse_filter = Control.MOUSE_FILTER_IGNORE
	_right_split.add_child(center)
	_preview_view_rect = center
	if _cam_input:
		_cam_input.viewport_rect_control = center

	_build_right_panel(_right_split)
	_build_bottom_panel(_main_vsplit)
	_apply_editor_theme(anchor)
	call_deferred("_restore_split_layout")
	_connect_layout_signals()

func _restore_window_geometry() -> void:
	var window := get_window()
	if not window:
		return
	var width := int(_config.get_value("layout", "window_width", window.size.x))
	var height := int(_config.get_value("layout", "window_height", window.size.y))
	width = maxi(width, 960)
	height = maxi(height, 640)
	if width != window.size.x or height != window.size.y:
		window.size = Vector2i(width, height)
	if _config.has_section_key("layout", "window_x") and _config.has_section_key("layout", "window_y"):
		window.position = Vector2i(
			int(_config.get_value("layout", "window_x", window.position.x)),
			int(_config.get_value("layout", "window_y", window.position.y))
		)

func _restore_split_layout() -> void:
	_layout_restoring = true
	var vp_size := get_viewport().get_visible_rect().size
	if _main_vsplit:
		var content_h := maxf(0.0, vp_size.y - TopMenu.BAR_HEIGHT)
		var bottom_h := float(_config.get_value("layout", "bottom_log_height", PANEL_BOT_H))
		bottom_h = clampf(bottom_h, float(PANEL_BOTTOM_MIN_H), maxf(float(PANEL_BOTTOM_MIN_H), content_h - 240.0))
		_main_vsplit.split_offset = int(maxf(240.0, content_h - bottom_h))
	if _left_split:
		var left_w := int(_config.get_value("layout", "left_panel_width", PANEL_LEFT_W))
		var max_left := int(maxf(float(PANEL_LEFT_MIN_W), vp_size.x - PANEL_RIGHT_MIN_W - 320.0))
		_left_split.split_offset = clampi(left_w, PANEL_LEFT_MIN_W, max_left)
	if _right_split:
		var available_w := _right_split.size.x
		if available_w <= 0.0:
			available_w = maxf(0.0, vp_size.x - PANEL_LEFT_W)
		var right_w := int(_config.get_value("layout", "right_panel_width", PANEL_RIGHT_W))
		var max_right := int(maxf(float(PANEL_RIGHT_MIN_W), available_w - 320.0))
		right_w = clampi(right_w, PANEL_RIGHT_MIN_W, max_right)
		_right_split.split_offset = int(maxf(320.0, available_w - right_w))
	_layout_restoring = false

func _connect_layout_signals() -> void:
	var window := get_window()
	if window and window.has_signal("size_changed"):
		window.connect("size_changed", Callable(self, "_on_layout_changed"))
	if _main_vsplit and _main_vsplit.has_signal("dragged"):
		_main_vsplit.connect("dragged", Callable(self, "_on_split_dragged"))
	if _left_split and _left_split.has_signal("dragged"):
		_left_split.connect("dragged", Callable(self, "_on_split_dragged"))
	if _right_split and _right_split.has_signal("dragged"):
		_right_split.connect("dragged", Callable(self, "_on_split_dragged"))

func _on_split_dragged(_offset: int) -> void:
	_save_layout_state()

func _on_layout_changed() -> void:
	if _layout_restoring:
		return
	_save_layout_state()

func _notification(what: int) -> void:
	if what == NOTIFICATION_WM_CLOSE_REQUEST or what == NOTIFICATION_PREDELETE:
		_save_layout_state()

func _save_layout_state() -> void:
	if _layout_restoring or not _config:
		return
	var window := get_window()
	if window:
		_config.set_value("layout", "window_width", window.size.x)
		_config.set_value("layout", "window_height", window.size.y)
		_config.set_value("layout", "window_x", window.position.x)
		_config.set_value("layout", "window_y", window.position.y)
	if _left_split:
		_config.set_value("layout", "left_panel_width", _left_split.split_offset)
	if _right_split and _right_split.size.x > 0.0:
		_config.set_value(
			"layout",
			"right_panel_width",
			maxi(PANEL_RIGHT_MIN_W, int(round(_right_split.size.x - _right_split.split_offset)))
		)
	if _main_vsplit:
		var content_h := _current_editor_content_height()
		_config.set_value(
			"layout",
			"bottom_log_height",
			maxi(PANEL_BOTTOM_MIN_H, int(round(content_h - _main_vsplit.split_offset)))
		)
	_save_config()

func _current_editor_content_height() -> float:
	var viewport := get_viewport()
	if viewport:
		return maxf(0.0, viewport.get_visible_rect().size.y - TopMenu.BAR_HEIGHT)
	var window := get_window()
	if window:
		return maxf(0.0, float(window.size.y) - TopMenu.BAR_HEIGHT)
	return maxf(
		float(PANEL_BOTTOM_MIN_H),
		float(_config.get_value("layout", "window_height", 720)) - TopMenu.BAR_HEIGHT
	)

func _attach_top_menu() -> void:
	if has_node("TopMenu"):
		return
	var top_menu := TopMenu.new()
	top_menu.name = "TopMenu"
	top_menu.scene_kind = TopMenu.SCENE_ASSET_EDITOR
	add_child(top_menu)
	_top_menu = top_menu
	if top_menu.has_method("set_editor_theme_mode"):
		top_menu.set_editor_theme_mode(_theme_mode)

func get_ui_theme_mode() -> String:
	return _theme_mode

func menu_toggle_ui_theme() -> String:
	set_ui_theme_mode(EditorTheme.next_mode(_theme_mode))
	return _theme_mode

func set_ui_theme_mode(mode: String) -> void:
	var next_mode := EditorTheme.normalize_mode(mode)
	if _theme_mode == next_mode:
		return
	_theme_mode = next_mode
	_save_config()
	_configure_preview_environment()
	if _theme_root:
		_apply_editor_theme(_theme_root)
	if _top_menu and _top_menu.has_method("set_editor_theme_mode"):
		_top_menu.set_editor_theme_mode(_theme_mode)

func menu_save() -> void:
	_on_export_pressed()

func menu_new_asset() -> void:
	_start_new_asset()

func menu_reload_packs() -> void:
	_load_packs()

func menu_import_mesh() -> void:
	_on_import_glb_pressed()

func _build_left_panel(parent: Control) -> void:
	var panel := PanelContainer.new()
	panel.custom_minimum_size.x = PANEL_LEFT_MIN_W
	panel.size_flags_vertical = Control.SIZE_EXPAND_FILL
	parent.add_child(panel)

	var margin := _add_panel_margin(panel)

	var vbox := VBoxContainer.new()
	vbox.size_flags_horizontal = Control.SIZE_EXPAND_FILL
	vbox.size_flags_vertical = Control.SIZE_EXPAND_FILL
	vbox.add_theme_constant_override("separation", PANEL_GAP)
	margin.add_child(vbox)

	_add_label(vbox, "Asset Browser", _font_size_header)
	_asset_search_edit = LineEdit.new()
	_asset_search_edit.placeholder_text = "Search assets"
	_asset_search_edit.size_flags_horizontal = Control.SIZE_EXPAND_FILL
	_asset_search_edit.text_changed.connect(_on_asset_search_changed)
	vbox.add_child(_asset_search_edit)

	_asset_count_lbl = Label.new()
	_asset_count_lbl.text = "0 assets"
	_asset_count_lbl.add_theme_font_size_override("font_size", _font_size_label)
	vbox.add_child(_asset_count_lbl)

	_asset_tree = Tree.new()
	_asset_tree.hide_root = true
	_asset_tree.columns = 1
	_asset_tree.size_flags_horizontal = Control.SIZE_EXPAND_FILL
	_asset_tree.size_flags_vertical = Control.SIZE_EXPAND_FILL
	_asset_tree.item_activated.connect(_on_asset_tree_activated)
	_asset_tree.gui_input.connect(_on_asset_tree_gui_input)
	vbox.add_child(_asset_tree)

	_asset_context_menu = PopupMenu.new()
	_asset_context_menu.add_item("Use as Ghost", ASSET_CONTEXT_USE_AS_GHOST)
	_asset_context_menu.id_pressed.connect(_on_asset_context_menu_id_pressed)
	panel.add_child(_asset_context_menu)
	EditorTheme.style_popup_menu(_asset_context_menu, _theme_mode)

	vbox.add_child(HSeparator.new())
	_add_label(vbox, "Scene Template", _font_size_section)
	_template_btn = OptionButton.new()
	for t in TEMPLATES:
		_template_btn.add_item(t)
	_template_btn.item_selected.connect(_on_template_selected)
	vbox.add_child(_template_btn)

func _build_right_panel(parent: Control) -> void:
	var panel := PanelContainer.new()
	panel.custom_minimum_size.x = PANEL_RIGHT_MIN_W
	panel.size_flags_vertical = Control.SIZE_EXPAND_FILL
	parent.add_child(panel)

	var margin := _add_panel_margin(panel)

	var root := VBoxContainer.new()
	root.size_flags_horizontal = Control.SIZE_EXPAND_FILL
	root.size_flags_vertical = Control.SIZE_EXPAND_FILL
	root.add_theme_constant_override("separation", PANEL_GAP)
	margin.add_child(root)

	_add_label(root, "Building Importer", _font_size_header)

	# Import GLB
	var glb_btn := Button.new()
	glb_btn.text = "Import mesh..."
	glb_btn.pressed.connect(_on_import_glb_pressed)
	root.add_child(glb_btn)

	var tab_shell := VBoxContainer.new()
	tab_shell.size_flags_horizontal = Control.SIZE_EXPAND_FILL
	tab_shell.size_flags_vertical = Control.SIZE_EXPAND_FILL
	tab_shell.add_theme_constant_override("separation", PANEL_GAP)
	root.add_child(tab_shell)

	var tab_header := HBoxContainer.new()
	tab_header.add_theme_constant_override("separation", PANEL_GAP)
	tab_shell.add_child(tab_header)

	var tab_buttons: Array[Button] = []

	var tabs := TabContainer.new()
	tabs.size_flags_horizontal = Control.SIZE_EXPAND_FILL
	tabs.size_flags_vertical = Control.SIZE_EXPAND_FILL
	tabs.tabs_visible = false
	tabs.use_hidden_tabs_for_min_size = true
	tabs.clip_tabs = true
	tabs.tab_changed.connect(func(tab: int): _sync_inspector_tab_buttons(tab_buttons, tab))
	tab_shell.add_child(tabs)

	var pack_box := _add_inspector_tab(tabs, tab_header, tab_buttons, "Pack")
	_pack_set_btn = Button.new()
	_pack_set_btn.text = "Set Pack..."
	_pack_set_btn.size_flags_horizontal = Control.SIZE_EXPAND_FILL
	_pack_set_btn.pressed.connect(func(): _open_pack_select_menu(_pack_set_btn))
	pack_box.add_child(_pack_set_btn)

	_pack_summary_lbl = Label.new()
	_pack_summary_lbl.autowrap_mode = TextServer.AUTOWRAP_WORD_SMART
	_pack_summary_lbl.size_flags_horizontal = Control.SIZE_EXPAND_FILL
	_pack_summary_lbl.add_theme_font_size_override("font_size", _font_size_label)
	pack_box.add_child(_pack_summary_lbl)
	_update_pack_summary()

	_pack_select_menu = PopupMenu.new()
	_pack_select_menu.id_pressed.connect(_on_pack_select_menu_id_pressed)
	pack_box.add_child(_pack_select_menu)
	EditorTheme.style_popup_menu(_pack_select_menu, _theme_mode)

	var asset_box := _add_inspector_tab(tabs, tab_header, tab_buttons, "Asset")
	_asset_id_edit    = _add_line_edit(asset_box, "Asset ID (dot.separated)", "building.residential.house")
	_asset_id_edit.text_changed.connect(_on_asset_id_text_changed)
	_display_name_edit = _add_line_edit(asset_box, "Display Name", "House")
	_display_name_edit.text_changed.connect(func(_t): _auto_suggest_asset_id())
	_asset_set_edit   = _add_line_edit(asset_box, "Upgrade Family / Asset Set (optional)", "")
	_tags_edit        = _add_line_edit(asset_box, "Tags (comma-separated)", "")

	_add_label(asset_box, "Mesh Parts", _font_size_section)
	_lod_list = ItemList.new()
	_lod_list.custom_minimum_size.y = 150
	_lod_list.size_flags_vertical = Control.SIZE_EXPAND_FILL
	_lod_list.item_selected.connect(_on_mesh_part_selected)
	asset_box.add_child(_lod_list)
	var add_lod_btn := Button.new()
	add_lod_btn.text = "Add Mesh Part..."
	add_lod_btn.pressed.connect(_on_add_lod_pressed)
	asset_box.add_child(add_lod_btn)

	var building_box := _add_inspector_tab(tabs, tab_header, tab_buttons, "Building")
	_add_label(building_box, "Placement Mode", _font_size_label)
	_placement_mode_btn = OptionButton.new()
	for mode in PLACEMENT_MODES:
		_placement_mode_btn.add_item(str(mode["label"]))
		_placement_mode_btn.set_item_metadata(
			_placement_mode_btn.get_item_count() - 1,
			str(mode["id"])
		)
	_placement_mode_btn.item_selected.connect(_on_placement_mode_selected)
	building_box.add_child(_placement_mode_btn)

	_zoned_only_box = VBoxContainer.new()
	_zoned_only_box.add_theme_constant_override("separation", PANEL_GAP)
	building_box.add_child(_zoned_only_box)

	_add_label(_zoned_only_box, "Zone Type", _font_size_label)
	_zone_type_btn = OptionButton.new()
	for z in _zone_types:
		_zone_type_btn.add_item(z.capitalize())
	_zone_type_btn.item_selected.connect(_on_zone_type_selected)
	_zoned_only_box.add_child(_zone_type_btn)

	_add_label(_zoned_only_box, "Density", _font_size_label)
	_density_btn = OptionButton.new()
	_density_btn.item_selected.connect(_on_zone_or_lot_changed)
	_zoned_only_box.add_child(_density_btn)
	_refresh_density_options()

	_width_spin    = _add_spinbox(building_box, "Lot Width (cells)", 1, 20, 2)
	_depth_spin    = _add_spinbox(building_box, "Lot Depth (cells)", 1, 20, 2)
	_min_zone_width_spin = _add_spinbox(_zoned_only_box, "Min Zoned Width (cells)", 1, 20, 2)
	_min_zone_depth_spin = _add_spinbox(_zoned_only_box, "Min Zoned Depth (cells)", 1, 20, 2)
	_level_spin    = _add_spinbox(building_box, "Level", 1, 255, 1)
	_residents_spin = _add_spinbox(building_box, "Household Capacity", 0, 9999, 0)
	_flat_size_spin = _add_spinbox(building_box, "Avg Flat Size (m²)", 0, 9999, 60.0)
	_flat_size_spin.step = 0.5
	_workers_spin  = _add_spinbox(building_box, "Worker Capacity", 0, 9999, 0)
	_add_label(building_box, "Service Class", _font_size_label)
	_service_class_btn = OptionButton.new()
	for service_class in SERVICE_CLASSES:
		_service_class_btn.add_item(str(service_class["label"]))
		_service_class_btn.set_item_metadata(
			_service_class_btn.get_item_count() - 1,
			str(service_class["id"])
		)
	_service_class_btn.item_selected.connect(_on_service_class_selected)
	building_box.add_child(_service_class_btn)
	_add_label(building_box, "Economy Profile", _font_size_label)
	var economy_row := HBoxContainer.new()
	economy_row.add_theme_constant_override("separation", PANEL_GAP)
	_economy_profile_btn = OptionButton.new()
	_economy_profile_btn.size_flags_horizontal = Control.SIZE_EXPAND_FILL
	_economy_profile_btn.item_selected.connect(_on_economy_profile_selected)
	economy_row.add_child(_economy_profile_btn)
	var refresh_profiles_btn := Button.new()
	refresh_profiles_btn.text = "Refresh"
	refresh_profiles_btn.pressed.connect(_load_economy_profiles)
	economy_row.add_child(refresh_profiles_btn)
	building_box.add_child(economy_row)
	_economy_profile_status_lbl = Label.new()
	_economy_profile_status_lbl.autowrap_mode = TextServer.AUTOWRAP_WORD_SMART
	_economy_profile_status_lbl.add_theme_font_size_override("font_size", _font_size_label)
	building_box.add_child(_economy_profile_status_lbl)
	var suggest_btn := Button.new()
	suggest_btn.text = "Suggest Flat Size"
	suggest_btn.pressed.connect(_suggest_capacity)
	building_box.add_child(suggest_btn)

	_width_spin.value_changed.connect(func(_v): _on_zone_or_lot_changed(0))
	_depth_spin.value_changed.connect(func(_v): _on_zone_or_lot_changed(0))
	_last_lot_width_cells = int(_width_spin.value)
	_last_lot_depth_cells = int(_depth_spin.value)
	_min_zone_width_spin.value = _width_spin.value
	_min_zone_depth_spin.value = _depth_spin.value
	_update_building_mode_visibility()

	var preview_box := _add_inspector_tab(tabs, tab_header, tab_buttons, "Preview")
	# Preset dropdown.
	_scale_preset_btn = OptionButton.new()
	_scale_preset_btn.add_item("Custom")
	_scale_preset_btn.add_item("Fit to Lot")
	_scale_preset_btn.add_item("½ Lot")
	_scale_preset_btn.add_item("¼ Lot")
	_scale_preset_btn.item_selected.connect(_on_scale_preset_selected)
	preview_box.add_child(_scale_preset_btn)

	_preview_scale_spin = _add_spinbox(preview_box, "Selected Part Scale", 0.01, 1000.0, 1.0)
	_preview_scale_spin.step = 0.01
	_preview_scale_spin.value_changed.connect(_on_preview_scale_changed)

	_part_x_spin = _add_spinbox(preview_box, "Selected Part X (m)", -500.0, 500.0, 0.0)
	_part_x_spin.step = 0.1
	_part_x_spin.value_changed.connect(_on_part_transform_changed)
	_part_y_spin = _add_spinbox(preview_box, "Selected Part Y (m)", -100.0, 100.0, 0.0)
	_part_y_spin.step = 0.1
	_part_y_spin.value_changed.connect(_on_part_transform_changed)
	_part_z_spin = _add_spinbox(preview_box, "Selected Part Z (m)", -500.0, 500.0, 0.0)
	_part_z_spin.step = 0.1
	_part_z_spin.value_changed.connect(_on_part_transform_changed)
	_part_rotation_y_spin = _add_spinbox(preview_box, "Selected Part Rotation Y", -180.0, 180.0, 0.0)
	_part_rotation_y_spin.step = 1.0
	_part_rotation_y_spin.value_changed.connect(_on_part_transform_changed)

	# Live dimension label.
	_dim_label = Label.new()
	_dim_label.text = "→ —"
	_dim_label.add_theme_font_size_override("font_size", _font_size_label)
	_dim_label.add_theme_color_override("font_color", Color(0.7, 0.9, 0.7))
	preview_box.add_child(_dim_label)

	var autofit_btn := Button.new()
	autofit_btn.text = "Auto-fit to Lot"
	autofit_btn.pressed.connect(_on_autofit_pressed)
	preview_box.add_child(autofit_btn)

	_human_btn = CheckButton.new()
	_human_btn.text = "Show Human (1.8 m)"
	_human_btn.toggled.connect(_on_human_toggled)
	preview_box.add_child(_human_btn)

	var clear_ghost_btn := Button.new()
	clear_ghost_btn.text = "Clear Ghost"
	clear_ghost_btn.pressed.connect(_on_clear_ghost_pressed)
	preview_box.add_child(clear_ghost_btn)

	var anchors_box := _add_inspector_tab(tabs, tab_header, tab_buttons, "Anchors")
	_add_label(anchors_box, "Frontage", _font_size_section)
	_frontage_lbl = Label.new()
	_frontage_lbl.text = "Forward: (0, 0, 1)"
	_frontage_lbl.add_theme_font_size_override("font_size", _font_size_label)
	anchors_box.add_child(_frontage_lbl)
	var set_front_btn := Button.new()
	set_front_btn.text = "Set Front From View"
	set_front_btn.pressed.connect(_on_set_front_from_view)
	anchors_box.add_child(set_front_btn)
	_add_label(anchors_box, "Main Entrance (local)", _font_size_label)
	_entrance_x_spin = _add_spinbox(anchors_box, "X (m)", -500.0, 500.0, 0.0)
	_entrance_y_spin = _add_spinbox(anchors_box, "Y (m)", -500.0, 500.0, 0.0)
	_entrance_z_spin = _add_spinbox(anchors_box, "Z (m)", -500.0, 500.0, 10.0)
	_entrance_x_spin.step = 0.1
	_entrance_y_spin.step = 0.1
	_entrance_z_spin.step = 0.1
	_entrance_x_spin.value_changed.connect(_on_main_entrance_changed)
	_entrance_y_spin.value_changed.connect(_on_main_entrance_changed)
	_entrance_z_spin.value_changed.connect(_on_main_entrance_changed)
	var reset_entrance_btn := Button.new()
	reset_entrance_btn.text = "Reset Entrance To Frontage"
	reset_entrance_btn.pressed.connect(_on_reset_main_entrance_pressed)
	anchors_box.add_child(reset_entrance_btn)
	var entrance_hint := Label.new()
	entrance_hint.text = "Drag the yellow sphere in the viewport to move X/Z. Use Y for height."
	entrance_hint.autowrap_mode = TextServer.AUTOWRAP_WORD_SMART
	entrance_hint.add_theme_font_size_override("font_size", _font_size_label)
	anchors_box.add_child(entrance_hint)

	var export_btn := Button.new()
	export_btn.text = "Export Asset"
	export_btn.pressed.connect(_on_export_pressed)
	root.add_child(export_btn)
	_sync_inspector_tab_buttons(tab_buttons, tabs.current_tab)

func _build_bottom_panel(parent: Control) -> void:
	var panel := PanelContainer.new()
	panel.custom_minimum_size.y = PANEL_BOTTOM_MIN_H
	panel.size_flags_horizontal = Control.SIZE_EXPAND_FILL
	parent.add_child(panel)

	var margin := _add_panel_margin(panel)

	var vbox := VBoxContainer.new()
	vbox.size_flags_vertical = Control.SIZE_EXPAND_FILL
	vbox.add_theme_constant_override("separation", PANEL_GAP)
	margin.add_child(vbox)

	var header := HBoxContainer.new()
	header.add_theme_constant_override("separation", PANEL_GAP)
	vbox.add_child(header)
	var title := Label.new()
	title.text = "Import Log"
	title.add_theme_font_size_override("font_size", _font_size_section)
	title.size_flags_horizontal = Control.SIZE_EXPAND_FILL
	header.add_child(title)
	var copy_btn := Button.new()
	copy_btn.text = "Copy All"
	copy_btn.pressed.connect(_on_copy_log_pressed)
	header.add_child(copy_btn)
	_log_label = RichTextLabel.new()
	_log_label.size_flags_vertical = Control.SIZE_EXPAND_FILL
	_log_label.scroll_following = true
	_log_label.bbcode_enabled = true
	_log_label.selection_enabled = true
	_log_label.context_menu_enabled = true
	_log_label.focus_mode = Control.FOCUS_CLICK
	vbox.add_child(_log_label)

# ──────────────────────────────────────────────────────────────────────────────
# Pack loading
# ──────────────────────────────────────────────────────────────────────────────

func _load_packs() -> void:
	var mods_path: String = ProjectSettings.globalize_path("user://mods/")
	if not DirAccess.dir_exists_absolute(mods_path):
		_log("No mods directory at %s — skipping pack scan." % mods_path)
		return

	var warnings: String = sim.load_asset_packs(mods_path, "")
	for line in warnings.split("\n"):
		if line != "":
			_log("[color=yellow]Warning:[/color] " + line)

	_refresh_asset_browser()

func _refresh_asset_browser() -> void:
	var mods_path: String = ProjectSettings.globalize_path("user://mods/")
	var warnings: String = sim.load_asset_packs(mods_path, "")
	if not warnings.is_empty():
		_log("[color=yellow]%s[/color]" % warnings)
	_asset_ids.clear()
	for aid in sim.get_registered_asset_ids():
		_asset_ids.append(str(aid))
	_asset_ids.sort()
	_refresh_known_packs(mods_path)
	_refresh_asset_display_name_cache()
	_rebuild_asset_tree()
	_log("Registry: %d asset(s) loaded." % _asset_ids.size())

func _refresh_known_packs(mods_path: String) -> void:
	var by_id := {}
	if DirAccess.dir_exists_absolute(mods_path):
		var dir := DirAccess.open(mods_path)
		if dir:
			dir.list_dir_begin()
			var entry := dir.get_next()
			while not entry.is_empty():
				if not entry.begins_with(".") and dir.current_is_dir():
					var pack := _pack_manifest_from_dir(mods_path.path_join(entry))
					if not pack.is_empty():
						by_id[str(pack.get("pack_id", ""))] = pack
				entry = dir.get_next()
			dir.list_dir_end()

	for aid in _asset_ids:
		var pack_id := _asset_pack_id(aid)
		if pack_id.is_empty() or by_id.has(pack_id):
			continue
		var pack := _pack_manifest_from_dir(mods_path.path_join(pack_id))
		if pack.is_empty():
			pack = {
				"pack_id": pack_id,
				"display_name": pack_id,
				"author": "",
			}
		by_id[pack_id] = pack

	_known_packs.clear()
	for pack in by_id.values():
		if pack is Dictionary:
			_known_packs.append(pack)
	_known_packs.sort_custom(func(a: Dictionary, b: Dictionary):
		return str(a.get("pack_id", "")) < str(b.get("pack_id", ""))
	)

func _pack_manifest_from_dir(pack_dir: String) -> Dictionary:
	if not FileAccess.file_exists(pack_dir.path_join("pack.toml")):
		return {}
	var json_str: String = sim.get_pack_manifest_json(pack_dir)
	if json_str.is_empty():
		return {}
	var parsed = JSON.parse_string(json_str)
	if parsed is Dictionary:
		var pack_id := str((parsed as Dictionary).get("pack_id", "")).strip_edges()
		if not pack_id.is_empty():
			return parsed
	return {}

func _open_pack_select_menu(field: Control) -> void:
	if not _pack_select_menu:
		return
	_rebuild_pack_select_menu()
	var pos := field.global_position + Vector2(0.0, field.size.y + 2.0)
	_pack_select_menu.position = Vector2i(int(round(pos.x)), int(round(pos.y)))
	_pack_select_menu.popup()

func _rebuild_pack_select_menu() -> void:
	_pack_select_menu.clear()
	if _known_packs.is_empty():
		_pack_select_menu.add_item("No installed packs", PACK_MENU_NO_PACKS)
		_pack_select_menu.set_item_disabled(_pack_select_menu.get_item_count() - 1, true)
	else:
		for i in _known_packs.size():
			var pack := _known_packs[i]
			var pack_id := str(pack.get("pack_id", "")).strip_edges()
			var display_name := str(pack.get("display_name", "")).strip_edges()
			var label := pack_id
			if not display_name.is_empty() and display_name != pack_id:
				label = "%s  (%s)" % [display_name, pack_id]
			_pack_select_menu.add_item(label, i)
	_pack_select_menu.add_separator()
	_pack_select_menu.add_item("Create New Pack...", PACK_MENU_CREATE_NEW)

func _on_pack_select_menu_id_pressed(id: int) -> void:
	if id == PACK_MENU_CREATE_NEW:
		_open_new_pack_dialog()
		return
	if id < 0 or id >= _known_packs.size():
		return
	_apply_pack_fields(_known_packs[id])

func _apply_pack_fields(pack: Dictionary) -> void:
	var pack_id := str(pack.get("pack_id", "")).strip_edges()
	var display_name := str(pack.get("display_name", "")).strip_edges()
	var author := str(pack.get("author", "")).strip_edges()
	if display_name.is_empty():
		display_name = pack_id
	_selected_pack_id = pack_id
	_selected_pack_name = display_name
	_selected_pack_author = author
	_update_pack_summary()

func _update_pack_summary() -> void:
	if not _pack_summary_lbl:
		return
	var pack_id := _selected_pack_id.strip_edges()
	var display_name := _selected_pack_name.strip_edges()
	var author := _selected_pack_author.strip_edges()
	if display_name.is_empty():
		display_name = pack_id
	var lines := PackedStringArray()
	lines.append("%s (%s)" % [display_name, pack_id])
	lines.append("Author: %s" % (author if not author.is_empty() else "Unspecified"))
	_pack_summary_lbl.text = "\n".join(lines)

func _open_new_pack_dialog() -> void:
	_ensure_pack_create_window()
	var current_id := _selected_pack_id.strip_edges()
	if current_id.is_empty() or _pack_id_exists(current_id):
		current_id = _suggest_new_pack_id()
	_new_pack_id_edit.text = current_id
	_new_pack_name_edit.text = _display_name_from_pack_id(current_id)
	_new_pack_author_edit.text = ""
	_pack_create_window.popup_centered(Vector2i(460, 250))
	_new_pack_id_edit.grab_focus()
	_new_pack_id_edit.select_all()

func _display_name_from_pack_id(pack_id: String) -> String:
	var words := PackedStringArray()
	for part in pack_id.replace("_", "-").split("-"):
		var word := str(part).strip_edges()
		if word.is_empty():
			continue
		words.append(word.substr(0, 1).to_upper() + word.substr(1))
	if words.is_empty():
		return "My Pack"
	return " ".join(words)

func _ensure_pack_create_window() -> void:
	if _pack_create_window and is_instance_valid(_pack_create_window):
		return
	_pack_create_window = Window.new()
	_pack_create_window.title = "Create Pack"
	_pack_create_window.min_size = Vector2i(420, 220)
	_pack_create_window.size = Vector2i(460, 250)
	_pack_create_window.close_requested.connect(func(): _pack_create_window.hide())
	add_child(_pack_create_window)

	var panel := PanelContainer.new()
	panel.set_anchors_preset(Control.PRESET_FULL_RECT)
	_pack_create_window.add_child(panel)

	var margin := _add_panel_margin(panel)
	var root := VBoxContainer.new()
	root.size_flags_horizontal = Control.SIZE_EXPAND_FILL
	root.size_flags_vertical = Control.SIZE_EXPAND_FILL
	root.add_theme_constant_override("separation", PANEL_GAP)
	margin.add_child(root)

	_add_label(root, "New Pack", _font_size_header)
	_new_pack_id_edit = _add_line_edit(root, "Pack ID (kebab-case)", "")
	_new_pack_name_edit = _add_line_edit(root, "Pack Name", "")
	_new_pack_author_edit = _add_line_edit(root, "Author", "")

	var hint := Label.new()
	hint.text = "Creates user://mods/<pack_id>/pack.toml now. Asset files are added on export."
	hint.autowrap_mode = TextServer.AUTOWRAP_WORD_SMART
	hint.add_theme_font_size_override("font_size", _font_size_label)
	root.add_child(hint)

	var footer := HBoxContainer.new()
	footer.add_theme_constant_override("separation", PANEL_GAP)
	root.add_child(footer)
	var spacer := Control.new()
	spacer.size_flags_horizontal = Control.SIZE_EXPAND_FILL
	footer.add_child(spacer)
	var cancel_btn := Button.new()
	cancel_btn.text = "Cancel"
	cancel_btn.pressed.connect(func(): _pack_create_window.hide())
	footer.add_child(cancel_btn)
	var create_btn := Button.new()
	create_btn.text = "Create"
	create_btn.pressed.connect(_on_create_pack_pressed)
	footer.add_child(create_btn)
	_apply_editor_theme(_pack_create_window)

func _on_create_pack_pressed() -> void:
	var pack_id := _new_pack_id_edit.text.strip_edges()
	var display_name := _new_pack_name_edit.text.strip_edges()
	var author := _new_pack_author_edit.text.strip_edges()
	if display_name.is_empty():
		display_name = pack_id
	if not _pack_id_is_valid(pack_id):
		_log("[color=red]Pack ID must match ^[a-z0-9][a-z0-9_-]*$.[/color]")
		return
	if _pack_id_exists(pack_id):
		_log("[color=red]Pack already exists: %s[/color]" % pack_id)
		return

	var mods_path := ProjectSettings.globalize_path("user://mods/")
	var err := DirAccess.make_dir_recursive_absolute(mods_path)
	if err != OK:
		_log("[color=red]Could not create mods directory: %s (error %d)[/color]" % [mods_path, err])
		return
	var pack_dir := mods_path.path_join(pack_id)
	err = DirAccess.make_dir_recursive_absolute(pack_dir)
	if err != OK:
		_log("[color=red]Could not create pack directory: %s (error %d)[/color]" % [pack_dir, err])
		return

	var pack_path := pack_dir.path_join("pack.toml")
	if FileAccess.file_exists(pack_path):
		_log("[color=red]Pack manifest already exists: %s[/color]" % pack_path)
		return
	var file := FileAccess.open(pack_path, FileAccess.WRITE)
	if file == null:
		_log("[color=red]Could not write pack manifest: %s (error %d)[/color]" % [pack_path, FileAccess.get_open_error()])
		return
	file.store_string(_build_pack_toml(pack_id, display_name, author))
	file.close()

	var pack := {
		"pack_id": pack_id,
		"display_name": display_name,
		"author": author,
	}
	_refresh_known_packs(mods_path)
	_apply_pack_fields(pack)
	_pack_create_window.hide()
	_log("[color=green]Created pack '%s' at %s[/color]" % [pack_id, pack_path])

func _build_pack_toml(pack_id: String, display_name: String, author: String) -> String:
	return (
		"pack_id = \"%s\"\n" % _toml_escape(pack_id)
		+ "schema_version = %d\n" % PACK_SCHEMA_VERSION
		+ "display_name = \"%s\"\n" % _toml_escape(display_name)
		+ "version = \"0.1.0\"\n"
		+ "author = \"%s\"\n" % _toml_escape(author)
		+ "license = \"CC0\"\n"
		+ "description = \"\"\n"
	)

func _toml_escape(value: String) -> String:
	return value.replace("\\", "\\\\").replace("\"", "\\\"").replace("\n", " ")

func _pack_id_is_valid(pack_id: String) -> bool:
	if pack_id.is_empty():
		return false
	if not _is_ascii_lower_or_digit(pack_id.substr(0, 1)):
		return false
	for i in pack_id.length():
		var ch := pack_id.substr(i, 1)
		if not _is_ascii_lower_or_digit(ch) and ch != "_" and ch != "-":
			return false
	return true

func _is_ascii_lower_or_digit(ch: String) -> bool:
	if ch.is_empty():
		return false
	var code := ch.unicode_at(0)
	return (code >= 97 and code <= 122) or (code >= 48 and code <= 57)

func _pack_id_exists(pack_id: String) -> bool:
	var target := pack_id.strip_edges()
	if target.is_empty():
		return false
	for pack in _known_packs:
		if str(pack.get("pack_id", "")).strip_edges() == target:
			return true
	var pack_path := ProjectSettings.globalize_path("user://mods/%s/pack.toml" % target)
	return FileAccess.file_exists(pack_path)

func _suggest_new_pack_id() -> String:
	var base := "my-pack"
	if not _pack_id_exists(base):
		return base
	var i := 2
	while i < 10000:
		var candidate := "%s-%d" % [base, i]
		if not _pack_id_exists(candidate):
			return candidate
		i += 1
	return "my-pack-%d" % int(Time.get_unix_time_from_system())

func _refresh_asset_display_name_cache() -> void:
	_asset_display_names.clear()
	for aid in _asset_ids:
		var json_str: String = sim.get_asset_manifest_json(aid)
		if json_str.is_empty():
			continue
		var parsed = JSON.parse_string(json_str)
		if not (parsed is Dictionary):
			continue
		var display_name := str((parsed as Dictionary).get("display_name", "")).strip_edges()
		if not display_name.is_empty():
			_asset_display_names[aid] = display_name

func _on_asset_search_changed(_text: String) -> void:
	_rebuild_asset_tree()

func _rebuild_asset_tree() -> void:
	if not _asset_tree:
		return

	var query := ""
	if _asset_search_edit:
		query = _asset_search_edit.text.strip_edges().to_lower()

	_asset_tree.clear()
	var root := _asset_tree.create_item()
	var visible_ids: Array[String] = []
	for aid in _asset_ids:
		if _asset_matches_query(aid, query):
			visible_ids.append(aid)

	if _asset_count_lbl:
		if query.is_empty():
			_asset_count_lbl.text = "%d assets" % _asset_ids.size()
		else:
			_asset_count_lbl.text = "%d / %d assets" % [visible_ids.size(), _asset_ids.size()]

	if visible_ids.is_empty():
		var empty_item := _asset_tree.create_item(root)
		empty_item.set_text(0, "No matching assets")
		empty_item.set_selectable(0, false)
		return

	var pack_counts := {}
	var category_counts := {}
	for aid in visible_ids:
		var pack := _asset_pack_id(aid)
		var category := _asset_category_id(aid)
		pack_counts[pack] = int(pack_counts.get(pack, 0)) + 1
		var category_key := "%s\n%s" % [pack, category]
		category_counts[category_key] = int(category_counts.get(category_key, 0)) + 1

	var pack_items := {}
	var category_items := {}
	for aid in visible_ids:
		var pack := _asset_pack_id(aid)
		var category := _asset_category_id(aid)
		var pack_item: TreeItem = pack_items.get(pack, null)
		if pack_item == null:
			pack_item = _asset_tree.create_item(root)
			pack_item.set_text(0, "%s (%d)" % [pack, int(pack_counts.get(pack, 0))])
			pack_item.set_selectable(0, false)
			pack_item.set_collapsed(false)
			pack_items[pack] = pack_item

		var category_key := "%s\n%s" % [pack, category]
		var category_item: TreeItem = category_items.get(category_key, null)
		if category_item == null:
			category_item = _asset_tree.create_item(pack_item)
			category_item.set_text(
				0,
				"%s (%d)" % [category, int(category_counts.get(category_key, 0))]
			)
			category_item.set_selectable(0, false)
			category_item.set_collapsed(false)
			category_items[category_key] = category_item

		var asset_item := _asset_tree.create_item(category_item)
		asset_item.set_text(0, _asset_browser_label(aid))
		asset_item.set_metadata(0, aid)
		asset_item.set_tooltip_text(0, aid)

func _asset_matches_query(aid: String, query: String) -> bool:
	if query.is_empty():
		return true
	return (
		aid.to_lower().contains(query)
		or _asset_browser_label(aid).to_lower().contains(query)
	)

func _asset_pack_id(aid: String) -> String:
	var sep := aid.find(":")
	if sep < 0:
		return "unpacked"
	return aid.substr(0, sep)

func _asset_local_id(aid: String) -> String:
	var sep := aid.find(":")
	if sep < 0:
		return aid
	return aid.substr(sep + 1)

func _asset_category_id(aid: String) -> String:
	var local_id := _asset_local_id(aid)
	var parts := local_id.split(".")
	if parts.size() >= 2:
		return "%s / %s" % [str(parts[0]), str(parts[1])]
	if parts.size() == 1 and not str(parts[0]).is_empty():
		return str(parts[0])
	return "uncategorized"

func _asset_browser_label(aid: String) -> String:
	return str(_asset_display_names.get(aid, _asset_local_id(aid))).strip_edges()

# ──────────────────────────────────────────────────────────────────────────────
# Scene templates
# ──────────────────────────────────────────────────────────────────────────────

func _on_template_selected(idx: int) -> void:
	_apply_template(idx)

func _apply_template(idx: int) -> void:
	match idx:
		0: _template_flat_studio()
		1: _template_zoned_roadside()
		2: _template_lane_reference()
		3: _template_traffic_comparison()
		4: _template_night_lighting()

func _template_flat_studio() -> void:
	_set_sun_angle(45.0)
	_log("Template: Flat Studio")

func _template_zoned_roadside() -> void:
	_set_sun_angle(55.0)
	_log("Template: Zoned Roadside (road setup in Step 5)")

func _template_lane_reference() -> void:
	_set_sun_angle(60.0)
	_log("Template: Lane + Sidewalk Reference (road setup in Step 5)")

func _template_traffic_comparison() -> void:
	_set_sun_angle(50.0)
	_log("Template: Traffic Comparison (vehicles in Step 6)")

func _template_night_lighting() -> void:
	_set_sun_angle(5.0)
	_log("Template: Night Lighting")

func _set_sun_angle(degrees: float) -> void:
	var light := find_child("DirectionalLight3D", true, false) as DirectionalLight3D
	if light:
		light.rotation_degrees.x = -degrees

# ──────────────────────────────────────────────────────────────────────────────
# Asset browser
# ──────────────────────────────────────────────────────────────────────────────

func _on_asset_tree_activated() -> void:
	_load_selected_asset_from_tree()

func _on_asset_tree_gui_input(event: InputEvent) -> void:
	if not (event is InputEventMouseButton):
		return
	var mb := event as InputEventMouseButton
	if mb.button_index != MOUSE_BUTTON_RIGHT or not mb.pressed:
		return
	if _open_asset_context_menu_at(mb.position):
		_asset_tree.accept_event()

func _open_asset_context_menu_at(mouse_position: Vector2) -> bool:
	if not _asset_tree or not _asset_context_menu:
		return false
	var item := _asset_tree.get_item_at_position(mouse_position)
	if item == null:
		return false
	var metadata = item.get_metadata(0)
	if metadata == null:
		return false
	_asset_context_asset_id = str(metadata).strip_edges()
	if _asset_context_asset_id.is_empty():
		return false
	item.select(0)
	var popup_pos := _asset_tree.get_global_mouse_position()
	_asset_context_menu.position = Vector2i(int(round(popup_pos.x)), int(round(popup_pos.y)))
	_asset_context_menu.popup()
	return true

func _on_asset_context_menu_id_pressed(id: int) -> void:
	if id != ASSET_CONTEXT_USE_AS_GHOST:
		return
	if _asset_context_asset_id.is_empty():
		return
	_use_asset_as_ghost(_asset_context_asset_id)

func _load_selected_asset_from_tree() -> void:
	if not _asset_tree:
		return
	var item := _asset_tree.get_selected()
	if item == null:
		return
	var metadata = item.get_metadata(0)
	if metadata == null:
		return
	var aid := str(metadata).strip_edges()
	if aid.is_empty():
		return
	_load_asset_manifest(aid)

func _load_asset_manifest(aid: String) -> void:
	var json_str: String = sim.get_asset_manifest_json(aid)
	if json_str.is_empty():
		return
	var data = JSON.parse_string(json_str)
	if data == null:
		return
	_populate_inspector_from(data)
	_loaded_asset_pack_id = str(data.get("pack_id", _asset_pack_id(aid))).strip_edges()
	_loaded_asset_id = str(data.get("asset_id", _asset_local_id(aid))).strip_edges()
	_log("Loaded manifest for '%s'." % aid)

func _use_asset_as_ghost(aid: String) -> void:
	var data := _asset_manifest(aid)
	if data.is_empty():
		_log("[color=yellow]Could not load asset manifest for ghost: %s[/color]" % aid)
		return
	var ghost_path := _asset_first_mesh_part_path(data)
	if ghost_path.is_empty() or not FileAccess.file_exists(ghost_path):
		_log("[color=yellow]Ghost LOD0 file not found for %s[/color]" % aid)
		return
	var scale := _asset_first_mesh_part_scale(data)
	var lot_width := int(data.get("lot_width_cells", 1))
	var lot_depth := int(data.get("lot_depth_cells", 1))
	if _preview.load_ghost(ghost_path, scale, lot_width, lot_depth):
		var label := str(data.get("display_name", aid)).strip_edges()
		if label.is_empty():
			label = aid
		_log("Using '%s' as ghost." % label)

func _asset_manifest(aid: String) -> Dictionary:
	var json_str: String = sim.get_asset_manifest_json(aid)
	if json_str.is_empty():
		return {}
	var parsed = JSON.parse_string(json_str)
	if parsed is Dictionary:
		return parsed
	return {}

func _asset_first_mesh_part_path(data: Dictionary) -> String:
	var pack_id := str(data.get("pack_id", "")).strip_edges()
	var asset_id := str(data.get("asset_id", "")).strip_edges()
	var parts: Array = data.get("mesh_parts", [])
	if pack_id.is_empty() or asset_id.is_empty() or parts.is_empty():
		return ""
	var first_part = parts[0]
	if not (first_part is Dictionary):
		return ""
	var lods: Array = (first_part as Dictionary).get("lods", [])
	if lods.is_empty() or not (lods[0] is Dictionary):
		return ""
	var lod0 := lods[0] as Dictionary
	var file_name := str((lod0 as Dictionary).get("file", "")).strip_edges()
	if file_name.is_empty():
		return ""
	return ProjectSettings.globalize_path("user://mods/%s/assets/%s/%s" % [pack_id, asset_id, file_name])

func _asset_first_mesh_part_scale(data: Dictionary) -> float:
	var parts: Array = data.get("mesh_parts", [])
	if parts.is_empty() or not (parts[0] is Dictionary):
		return 1.0
	return maxf(0.001, float((parts[0] as Dictionary).get("scale", 1.0)))

func _start_new_asset() -> void:
	if _asset_tree:
		_asset_tree.deselect_all()
	_asset_context_asset_id = ""

	_apply_pack_fields({
		"pack_id": "my-pack",
		"display_name": "My Pack",
		"author": "",
	})

	_asset_id_auto = true
	_display_name_edit.text = "House"
	_asset_set_edit.text = ""
	_tags_edit.text = ""
	_set_placement_mode_selection("zoned_private")
	_set_service_class_selection("none")
	var residential_idx := _zone_types.find("residential")
	if residential_idx >= 0:
		_zone_type_btn.select(residential_idx)
	elif _zone_type_btn.get_item_count() > 0:
		_zone_type_btn.select(0)
	_refresh_density_options("low")

	_width_spin.value = 2
	_depth_spin.value = 2
	_min_zone_width_spin.value = 2
	_min_zone_depth_spin.value = 2
	_last_lot_width_cells = 2
	_last_lot_depth_cells = 2
	_level_spin.value = 1
	_residents_spin.value = 0
	_flat_size_spin.value = 60.0
	_workers_spin.value = 0
	_suppress_preview_scale_changed = true
	_preview_scale_spin.value = 1.0
	_suppress_preview_scale_changed = false
	if _scale_preset_btn:
		_scale_preset_btn.select(0)
	_human_visible = false
	if _human_btn:
		_human_btn.button_pressed = false

	_auto_suggest_asset_id()
	_set_economy_profile_selection("")
	_extra_anchors.clear()
	_glb_path = ""
	_clear_mesh_parts()
	_loaded_asset_pack_id = ""
	_loaded_asset_id = ""
	_mesh_aabb = AABB()
	_pivot_offset = Vector3.ZERO
	_dim_label.text = "→ —"

	if _preview:
		_preview.set_lot_size(2, 2)
		_preview.set_show_human(false)
	_set_frontage_forward(Vector3.FORWARD)
	_set_main_entrance_position(_default_main_entrance_position(), true)
	_update_building_mode_visibility()
	_update_economy_profile_status()
	_log("Started a new asset.")

func _populate_inspector_from(data: Dictionary) -> void:
	# Pack fields — read from pack.toml on disk via Rust.
	var pack_id: String = data.get("pack_id", "")
	if not pack_id.is_empty():
		var pack_dir: String = ProjectSettings.globalize_path("user://mods/" + pack_id + "/")
		var pack_data := {
			"pack_id": pack_id,
			"display_name": pack_id,
			"author": "",
		}
		var pack_json: String = sim.get_pack_manifest_json(pack_dir)
		if not pack_json.is_empty():
			var parsed_pack_data = JSON.parse_string(pack_json)
			if parsed_pack_data is Dictionary:
				pack_data = parsed_pack_data
		_apply_pack_fields(pack_data)

	# Prevent auto-suggest from overwriting the loaded asset ID.
	_asset_id_auto = false
	_asset_id_edit.text     = data.get("asset_id", "")
	_display_name_edit.text = data.get("display_name", "")
	_asset_set_edit.text    = data.get("asset_set", "") if data.get("asset_set") != null else ""
	_tags_edit.text         = ", ".join(data.get("tags", []))
	var lot_width := int(data.get("lot_width_cells", 1))
	var lot_depth := int(data.get("lot_depth_cells", 1))
	_width_spin.value       = lot_width
	_depth_spin.value       = lot_depth
	_last_lot_width_cells = lot_width
	_last_lot_depth_cells = lot_depth
	_level_spin.value       = data.get("level", 1)
	_residents_spin.value   = data.get("household_capacity", 0) if data.get("household_capacity") != null else 0
	var fsm2 = data.get("flat_size_m2", null)
	if fsm2 != null:
		_flat_size_spin.value = float(fsm2)
	else:
		# Fallback to defaults based on density if missing in manifest
		var d = data.get("density", "low")
		var default_sqm := 60.0
		if d == "medium": default_sqm = 45.0
		elif d == "high": default_sqm = 30.0
		_flat_size_spin.value = default_sqm

	_workers_spin.value     = data.get("worker_capacity", 0) if data.get("worker_capacity") != null else 0
	_suppress_preview_scale_changed = true
	_preview_scale_spin.value = 1.0
	_suppress_preview_scale_changed = false
	_pivot_offset = Vector3.ZERO

	_set_placement_mode_selection(str(data.get("placement_mode", "zoned_private")))
	_set_service_class_selection(str(data.get("service_class", "none")))

	var zt_value = data.get("zone_type", null)
	var zt: String = str(zt_value if zt_value != null else "residential")
	var zi := _zone_types.find(zt)
	if zi >= 0:
		_zone_type_btn.selected = zi

	var dt_value = data.get("density", null)
	var dt: String = str(dt_value if dt_value != null else "low")
	_refresh_density_options(dt)
	_min_zone_width_spin.value = (
		int(data.get("min_zone_width_cells"))
		if data.get("min_zone_width_cells") != null
		else lot_width
	)
	_min_zone_depth_spin.value = (
		int(data.get("min_zone_depth_cells"))
		if data.get("min_zone_depth_cells") != null
		else lot_depth
	)
	_update_building_mode_visibility()
	var loaded_profile_id := str(data.get("economy_profile", "") if data.get("economy_profile") != null else "")
	_set_economy_profile_selection(loaded_profile_id)
	if loaded_profile_id.strip_edges().is_empty():
		_auto_select_profile_for_service(_selected_service_class())
	_extra_anchors.clear()

	var main_anchor_pos := _default_main_entrance_position()
	var main_anchor_fwd := Vector3.FORWARD
	var has_main_anchor := false
	for anchor in data.get("anchors", []):
		if not (anchor is Dictionary):
			continue
		var anchor_dict: Dictionary = anchor
		var anchor_type := str(anchor_dict.get("anchor_type", "")).strip_edges()
		var anchor_name := str(anchor_dict.get("name", "")).strip_edges()
		if anchor_type == "entrance" and anchor_name == "main" and not has_main_anchor:
			var pos = anchor_dict.get("position", [])
			if pos is Array and pos.size() == 3:
				main_anchor_pos = Vector3(float(pos[0]), float(pos[1]), float(pos[2]))
			var fwd = anchor_dict.get("forward", [])
			if fwd is Array and fwd.size() == 3:
				main_anchor_fwd = Vector3(float(fwd[0]), float(fwd[1]), float(fwd[2]))
			has_main_anchor = true
			continue
		_extra_anchors.append(anchor_dict.duplicate(true))

	_set_frontage_forward(main_anchor_fwd if has_main_anchor else Vector3.FORWARD)
	if has_main_anchor:
		_set_main_entrance_position(main_anchor_pos, false)
	else:
		_set_main_entrance_position(_default_main_entrance_position(), true)
		_log("[color=yellow]Loaded asset has no 'entrance/main' anchor; using frontage default.[/color]")

	_clear_mesh_parts()
	var mesh_parts: Array = data.get("mesh_parts", [])
	for part in mesh_parts:
		if not (part is Dictionary):
			continue
		var part_dict := part as Dictionary
		var lods: Array = part_dict.get("lods", [])
		if lods.is_empty() or not (lods[0] is Dictionary):
			continue
		var lod0 := lods[0] as Dictionary
		var fname := str(lod0.get("file", "")).strip_edges()
		if fname.is_empty():
			continue
		var asset_id: String = data.get("asset_id", "")
		var native: String = ProjectSettings.globalize_path(
			"user://mods/%s/assets/%s/%s" % [pack_id, asset_id, fname])
		if not FileAccess.file_exists(native):
			_log("[color=yellow]Mesh part file not found on disk: %s[/color]" % native)
			continue
		var pos := _array_to_vector3(part_dict.get("position", []), Vector3.ZERO)
		var rot := _array_to_vector3(part_dict.get("rotation_degrees", []), Vector3.ZERO)
		var pivot := _array_to_vector3(part_dict.get("pivot_offset", []), Vector3.ZERO)
		var scale := maxf(0.001, float(part_dict.get("scale", 1.0)))
		_add_mesh_part_from_path(native, str(part_dict.get("name", "")), pos, rot.y, scale, pivot, false)

	_preview.set_lot_size(int(_width_spin.value), int(_depth_spin.value))
	_update_economy_profile_status()
	if _lod_source_paths.size() > 0:
		_glb_path = _lod_source_paths[0]
		_keep_camera = true
		_select_mesh_part(0)
		_update_dim_label()
	else:
		_preview.clear_mesh_parts()

func _array_to_vector3(value, fallback: Vector3) -> Vector3:
	if value is Array and value.size() == 3:
		return Vector3(float(value[0]), float(value[1]), float(value[2]))
	return fallback

# ──────────────────────────────────────────────────────────────────────────────
# Lot / zone change
# ──────────────────────────────────────────────────────────────────────────────

func _selected_placement_mode() -> String:
	if not _placement_mode_btn or _placement_mode_btn.get_item_count() == 0:
		return "zoned_private"
	var idx := clampi(_placement_mode_btn.selected, 0, _placement_mode_btn.get_item_count() - 1)
	return str(_placement_mode_btn.get_item_metadata(idx)).strip_edges()

func _set_placement_mode_selection(placement_mode: String) -> void:
	var target := placement_mode.strip_edges()
	if target != "explicit":
		target = "zoned_private"
	for i in range(_placement_mode_btn.get_item_count()):
		if str(_placement_mode_btn.get_item_metadata(i)).strip_edges() == target:
			_placement_mode_btn.select(i)
			return
	_placement_mode_btn.select(0)

func _selected_zone_type() -> String:
	if not _zone_type_btn or _zone_type_btn.get_item_count() == 0:
		return "residential"
	var idx := clampi(_zone_type_btn.selected, 0, _zone_type_btn.get_item_count() - 1)
	return _zone_types[idx]

func _selected_density() -> String:
	if not _density_btn or _density_btn.get_item_count() == 0:
		return "low"
	return _density_btn.get_item_text(_density_btn.selected).to_lower()

func _selected_service_class() -> String:
	if not _service_class_btn or _service_class_btn.get_item_count() == 0:
		return "none"
	var idx := clampi(_service_class_btn.selected, 0, _service_class_btn.get_item_count() - 1)
	return str(_service_class_btn.get_item_metadata(idx)).strip_edges()

func _set_service_class_selection(service_class: String) -> void:
	var target := service_class.strip_edges()
	if target.is_empty():
		target = "none"
	for i in range(_service_class_btn.get_item_count()):
		if str(_service_class_btn.get_item_metadata(i)).strip_edges() == target:
			_service_class_btn.select(i)
			return
	_service_class_btn.select(0)

func _expected_utility_service_for_class(service_class: String) -> String:
	match service_class.strip_edges():
		"power":
			return "power"
		"water":
			return "water"
		"waste":
			return "sewage"
	return ""

func _is_utility_service_class(service_class: String) -> bool:
	return UTILITY_PROFILE_BY_SERVICE.has(service_class.strip_edges())

func _default_economy_profile_for_service(service_class: String) -> String:
	return str(UTILITY_PROFILE_BY_SERVICE.get(service_class.strip_edges(), "")).strip_edges()

func _auto_select_profile_for_service(service_class: String) -> void:
	if not _selected_economy_profile_id().is_empty():
		return
	var profile_id := _default_economy_profile_for_service(service_class)
	if profile_id.is_empty():
		return
	if _economy_profiles_cache.has(profile_id):
		_set_economy_profile_selection(profile_id)

func _utility_profile_matches_service(profile_id: String, service_class: String) -> bool:
	var selected_id := profile_id.strip_edges()
	if selected_id.is_empty():
		return false
	if not _economy_catalog_loaded:
		return false
	var expected_service := _expected_utility_service_for_class(service_class)
	if expected_service.is_empty():
		return true
	var profile = _economy_profiles_cache.get(selected_id)
	if not (profile is Dictionary):
		return false
	var kind := str(profile.get("kind", "")).strip_edges()
	if kind != "utility_producer" and kind != "utility_processor":
		return false
	return str(profile.get("utility_service", "")).strip_edges() == expected_service

func _update_building_mode_visibility() -> void:
	var is_zoned_private := _selected_placement_mode() == "zoned_private"
	if _zoned_only_box:
		_zoned_only_box.visible = is_zoned_private

func _refresh_density_options(preferred_density: String = "") -> void:
	if not _density_btn:
		return
	var zone_type := _selected_zone_type()
	var densities: Array = _density_types_by_zone.get(zone_type, [])
	if densities.is_empty():
		densities = ["low"]
	_density_btn.clear()
	for density in densities:
		_density_btn.add_item(str(density).capitalize())
	var selected_idx := 0
	if not preferred_density.is_empty():
		var preferred_idx := densities.find(preferred_density)
		if preferred_idx >= 0:
			selected_idx = preferred_idx
	_density_btn.selected = selected_idx

func _on_zone_type_selected(_idx: int) -> void:
	_refresh_density_options()
	_auto_suggest_asset_id()
	_on_zone_or_lot_changed(0)

func _on_placement_mode_selected(_idx: int) -> void:
	var previous_service := _selected_service_class()
	if _selected_placement_mode() == "zoned_private" and previous_service != "none":
		var selected_profile := _selected_economy_profile_id()
		_set_service_class_selection("none")
		if selected_profile == _default_economy_profile_for_service(previous_service):
			_set_economy_profile_selection("")
	_update_building_mode_visibility()
	_auto_suggest_asset_id()
	_on_zone_or_lot_changed(0)

func _on_service_class_selected(_idx: int) -> void:
	var service_class := _selected_service_class()
	if service_class != "none" and _selected_placement_mode() != "explicit":
		_set_placement_mode_selection("explicit")
		_update_building_mode_visibility()
	_auto_select_profile_for_service(service_class)
	_auto_suggest_asset_id()
	_update_economy_profile_status()

func _sync_min_zone_defaults_with_lot_change() -> void:
	if not _min_zone_width_spin or not _min_zone_depth_spin:
		return
	var current_width := int(_width_spin.value)
	var current_depth := int(_depth_spin.value)
	if int(_min_zone_width_spin.value) == _last_lot_width_cells:
		_min_zone_width_spin.value = current_width
	if int(_min_zone_depth_spin.value) == _last_lot_depth_cells:
		_min_zone_depth_spin.value = current_depth
	_last_lot_width_cells = current_width
	_last_lot_depth_cells = current_depth

func _on_zone_or_lot_changed(_idx) -> void:
	_sync_min_zone_defaults_with_lot_change()
	_preview.set_lot_size(int(_width_spin.value), int(_depth_spin.value))
	if _main_entrance_auto:
		_set_main_entrance_position(_default_main_entrance_position(), true)
	else:
		_update_main_entrance_preview()
	_update_economy_profile_status()

func _on_asset_id_text_changed(_t: String) -> void:
	_asset_id_auto = false

func _load_economy_profiles() -> void:
	if not _economy_profile_btn:
		return

	var current_id := _selected_economy_profile_id()
	_economy_profile_ids.clear()
	_economy_profiles_cache.clear()
	_economy_catalog_loaded = false
	_economy_catalog_warning_count = 0
	_economy_catalog_error = ""
	_unresolved_economy_profile_id = ""

	_economy_profile_btn.clear()
	_economy_profile_btn.add_item("Unassigned")
	_economy_profile_btn.set_item_metadata(0, "")
	_economy_profile_btn.select(0)

	var economy_dir := ProjectSettings.globalize_path("res://../economy")
	if not DirAccess.dir_exists_absolute(economy_dir):
		_economy_catalog_error = "catalog folder missing at %s" % economy_dir
		_log("[color=yellow]Economy profile catalog missing at %s[/color]" % economy_dir)
		_update_economy_profile_status()
		_set_economy_profile_selection(current_id)
		return

	var payload = JSON.parse_string(sim.load_economy_project(economy_dir))
	if not (payload is Dictionary):
		_economy_catalog_error = "could not parse economy project JSON"
		_log("[color=yellow]Economy profile catalog returned unreadable JSON.[/color]")
		_update_economy_profile_status()
		_set_economy_profile_selection(current_id)
		return

	if not payload.get("ok", false):
		_economy_catalog_error = str(payload.get("error", "catalog load failed"))
		_log("[color=yellow]Economy profile catalog unavailable: %s[/color]" % _economy_catalog_error)
		_update_economy_profile_status()
		_set_economy_profile_selection(current_id)
		return

	var project = payload.get("project", {})
	var profiles: Array = project.get("profiles", [])
	for profile in profiles:
		if not (profile is Dictionary):
			continue
		var profile_id := str(profile.get("id", "")).strip_edges()
		if profile_id.is_empty():
			continue
		var display_name := str(profile.get("display_name", "")).strip_edges()
		var label := profile_id if display_name.is_empty() else "%s — %s" % [profile_id, display_name]
		_economy_profile_btn.add_item(label)
		var idx := _economy_profile_btn.get_item_count() - 1
		_economy_profile_btn.set_item_metadata(idx, profile_id)
		_economy_profile_ids.append(profile_id)
		_economy_profiles_cache[profile_id] = profile

	var validation: Array = payload.get("validation", [])
	for message in validation:
		if message is Dictionary and str(message.get("severity", "")) != "error":
			_economy_catalog_warning_count += 1

	_economy_catalog_loaded = true
	if current_id.is_empty():
		_auto_select_profile_for_service(_selected_service_class())
		if _selected_economy_profile_id().is_empty():
			_update_economy_profile_status()
	else:
		_set_economy_profile_selection(current_id)

func _selected_economy_profile_id() -> String:
	if not _economy_profile_btn or _economy_profile_btn.get_item_count() == 0:
		return ""
	var idx := _economy_profile_btn.selected
	if idx < 0 or idx >= _economy_profile_btn.get_item_count():
		return ""
	return str(_economy_profile_btn.get_item_metadata(idx)).strip_edges()

func _set_economy_profile_selection(profile_id: String) -> void:
	if not _economy_profile_btn:
		return

	var target_id := profile_id.strip_edges()
	_remove_unresolved_economy_profile_item()
	if target_id.is_empty():
		_economy_profile_btn.select(0)
		_update_economy_profile_status()
		return

	for i in range(_economy_profile_btn.get_item_count()):
		if str(_economy_profile_btn.get_item_metadata(i)).strip_edges() == target_id:
			_economy_profile_btn.select(i)
			_update_economy_profile_status()
			return

	_unresolved_economy_profile_id = target_id
	_economy_profile_btn.add_item("[Missing] %s" % target_id)
	var missing_idx := _economy_profile_btn.get_item_count() - 1
	_economy_profile_btn.set_item_metadata(missing_idx, target_id)
	_economy_profile_btn.select(missing_idx)
	_update_economy_profile_status()

func _remove_unresolved_economy_profile_item() -> void:
	if not _economy_profile_btn or _unresolved_economy_profile_id.is_empty():
		return
	for i in range(_economy_profile_btn.get_item_count()):
		if str(_economy_profile_btn.get_item_metadata(i)).strip_edges() == _unresolved_economy_profile_id:
			_economy_profile_btn.remove_item(i)
			break
	_unresolved_economy_profile_id = ""

func _on_economy_profile_selected(_idx: int) -> void:
	var selected_id := _selected_economy_profile_id()
	if not _unresolved_economy_profile_id.is_empty() and selected_id != _unresolved_economy_profile_id:
		_remove_unresolved_economy_profile_item()
		for i in range(_economy_profile_btn.get_item_count()):
			if str(_economy_profile_btn.get_item_metadata(i)).strip_edges() == selected_id:
				_economy_profile_btn.select(i)
				break
	_update_economy_profile_status()

func _update_economy_profile_status() -> void:
	_sync_workers_to_profile()
	if not _economy_profile_status_lbl:
		return

	var selected_id := _selected_economy_profile_id()
	var placement_mode := _selected_placement_mode()
	var zone_type := _selected_zone_type()
	var service_class := _selected_service_class()
	if service_class != "none" and placement_mode != "explicit":
		_set_economy_profile_status(
			"Service assets must use explicit placement.",
			Color(1.0, 0.42, 0.36)
		)
		return

	if not _economy_catalog_loaded:
		var msg := "Economy catalog unavailable."
		if not _economy_catalog_error.is_empty():
			msg = "Economy catalog unavailable: %s" % _economy_catalog_error
		if _is_utility_service_class(service_class):
			if selected_id.is_empty():
				msg += " Utility service assets require a resolved matching profile."
			else:
				msg += " Cannot validate utility profile '%s'." % selected_id
			_set_economy_profile_status(msg, Color(1.0, 0.42, 0.36))
			return
		if not selected_id.is_empty():
			msg += " Existing selection will be preserved on export."
		_set_economy_profile_status(msg, Color(0.95, 0.78, 0.38))
		return

	if not _unresolved_economy_profile_id.is_empty() and selected_id == _unresolved_economy_profile_id:
		_set_economy_profile_status(
			"Selected profile is missing from the current economy catalog and will be exported unchanged.",
			Color(0.95, 0.78, 0.38)
		)
		return

	if placement_mode == "explicit":
		if _is_utility_service_class(service_class):
			if selected_id.is_empty():
				_set_economy_profile_status(
					"Utility service assets require an economy profile.",
					Color(1.0, 0.42, 0.36)
				)
				return
			if not _utility_profile_matches_service(selected_id, service_class):
				_set_economy_profile_status(
					"Selected profile does not provide the %s utility service." % _expected_utility_service_for_class(service_class),
					Color(1.0, 0.42, 0.36)
				)
				return
		if selected_id.is_empty():
			var explicit_msg := "Explicit buildings are outside zoned-private growth. Economy profile is optional."
			if _economy_catalog_warning_count > 0:
				explicit_msg += " Catalog has %d validation warning(s)." % _economy_catalog_warning_count
			_set_economy_profile_status(explicit_msg, Color(0.72, 0.82, 0.92))
			return
		var explicit_selected_msg := "Selected economy profile: %s" % selected_id
		if _economy_catalog_warning_count > 0:
			explicit_selected_msg += " (catalog has %d validation warning(s))" % _economy_catalog_warning_count
		_set_economy_profile_status(explicit_selected_msg, Color(0.72, 0.92, 0.72))
		return

	if zone_type == "residential":
		if selected_id.is_empty():
			var residential_msg := "Residential buildings do not require an economy profile."
			if _economy_catalog_warning_count > 0:
				residential_msg += " Catalog has %d validation warning(s)." % _economy_catalog_warning_count
			_set_economy_profile_status(residential_msg, Color(0.72, 0.92, 0.72))
			return
		_set_economy_profile_status(
			"Residential buildings usually do not require an economy profile. Leave this unassigned unless a later system explicitly needs one.",
			Color(0.95, 0.78, 0.38)
		)
		return

	if selected_id.is_empty():
		var unassigned_msg := "No economy profile assigned."
		if _economy_catalog_warning_count > 0:
			unassigned_msg += " Catalog has %d validation warning(s)." % _economy_catalog_warning_count
		_set_economy_profile_status(unassigned_msg, Color(0.72, 0.82, 0.92))
		return

	var selected_msg := "Selected economy profile: %s" % selected_id
	if _economy_catalog_warning_count > 0:
		selected_msg += " (catalog has %d validation warning(s))" % _economy_catalog_warning_count
	_set_economy_profile_status(selected_msg, Color(0.72, 0.92, 0.72))

func _set_economy_profile_status(message: String, color: Color) -> void:
	if not _economy_profile_status_lbl:
		return
	_economy_profile_status_lbl.text = message
	_economy_profile_status_lbl.add_theme_color_override("font_color", color)

# Auto-fills the Asset ID field from zone type + display name, but only while
# the user has not manually edited the field.
func _auto_suggest_asset_id() -> void:
	if not _asset_id_auto:
		return
	var prefix := "explicit"
	if _selected_placement_mode() == "zoned_private":
		prefix = _selected_zone_type()
	else:
		var service_class := _selected_service_class()
		if service_class != "none":
			prefix = service_class
	var name_slug: String = _display_name_edit.text.strip_edges().to_lower()
	name_slug = name_slug.replace(" ", "_")
	var clean := ""
	for ch in name_slug:
		var code := ch.unicode_at(0)
		if (code >= 97 and code <= 122) or (code >= 48 and code <= 57) or ch == "_":
			clean += ch
	if clean.is_empty():
		clean = "unnamed"
	# Set text without triggering the manual-edit flag.
	_asset_id_edit.text_changed.disconnect(_on_asset_id_text_changed)
	_asset_id_edit.text = "building.%s.%s" % [prefix, clean]
	_asset_id_edit.text_changed.connect(_on_asset_id_text_changed)

func _sync_workers_to_profile() -> void:
	if not _workers_spin:
		return
	var selected_id := _selected_economy_profile_id()
	if selected_id.is_empty():
		_workers_spin.editable = true
		return

	var profile = _economy_profiles_cache.get(selected_id)
	if profile is Dictionary:
		var cap = int(profile.get("worker_capacity", 0))
		_workers_spin.value = cap
		_workers_spin.editable = false
	else:
		# Unresolved profile (missing from catalog) — allow manual override
		# but default to current value.
		_workers_spin.editable = true

# ──────────────────────────────────────────────────────────────────────────────
# Import GLB
# ──────────────────────────────────────────────────────────────────────────────

func _on_import_glb_pressed() -> void:
	var dialog := MeshImportDialog.new()
	dialog.theme_mode = _theme_mode
	dialog.mesh_selected.connect(_on_glb_file_selected)
	add_child(dialog)
	dialog.open(_last_glb_dir)

func _on_glb_file_selected(path: String) -> void:
	_last_glb_dir = path.get_base_dir()
	_save_config()
	_glb_path = path
	var idx := _add_mesh_part_from_path(path, "")
	if idx >= 0:
		_select_mesh_part(idx)
	_log("Added mesh part '%s'." % path.get_file())

# ──────────────────────────────────────────────────────────────────────────────
# Mesh part management
# ──────────────────────────────────────────────────────────────────────────────

func _on_add_lod_pressed() -> void:
	var dialog := MeshImportDialog.new()
	dialog.theme_mode = _theme_mode
	dialog.mesh_selected.connect(_on_lod_file_selected)
	add_child(dialog)
	dialog.open(_last_glb_dir)

func _on_lod_file_selected(path: String) -> void:
	_last_glb_dir = path.get_base_dir()
	_save_config()
	var idx := _add_mesh_part_from_path(path, "")
	if idx >= 0:
		_select_mesh_part(idx)
	_log("Added mesh part '%s'." % path.get_file())

func _add_mesh_part_from_path(
	path: String,
	part_name: String,
	position: Vector3 = Vector3.ZERO,
	rotation_y: float = 0.0,
	scale: float = 1.0,
	pivot_offset: Vector3 = Vector3.ZERO,
	auto_pivot: bool = true
) -> int:
	if not _preview:
		return -1
	var idx := _lod_source_paths.size()
	var name := part_name.strip_edges()
	if name.is_empty():
		name = "part_%d" % [idx + 1]
	var aabb: AABB = _preview.add_mesh_part(path)
	var stored_pivot := pivot_offset
	if auto_pivot:
		stored_pivot = Vector3(
			-(aabb.position.x + aabb.size.x * 0.5),
			-aabb.position.y,
			-(aabb.position.z + aabb.size.z * 0.5)
		)
	_lod_source_paths.append(path)
	_part_positions.append(position)
	_part_rotation_y.append(rotation_y)
	_part_scales.append(maxf(0.001, scale))
	_part_pivot_offsets.append(stored_pivot)
	_part_aabbs.append(aabb)
	_lod_list.add_item("%s  —  %s" % [name, path.get_file()])
	_preview.set_mesh_part_transform(idx, position, rotation_y, scale, stored_pivot)
	return idx

func _select_mesh_part(index: int) -> void:
	if index < 0 or index >= _lod_source_paths.size():
		_selected_part_index = -1
	else:
		_selected_part_index = index
		if _lod_list and index < _lod_list.item_count:
			_lod_list.select(index)
		_mesh_aabb = _part_aabbs[index]
		_pivot_offset = _part_pivot_offsets[index]
	_refresh_selected_part_controls()
	_update_dim_label()

func _on_mesh_part_selected(index: int) -> void:
	_select_mesh_part(index)

func _refresh_selected_part_controls() -> void:
	_suppress_part_transform_changed = true
	_suppress_preview_scale_changed = true
	var has_part := _selected_part_index >= 0 and _selected_part_index < _lod_source_paths.size()
	var pos := _part_positions[_selected_part_index] if has_part else Vector3.ZERO
	_part_x_spin.value = pos.x
	_part_y_spin.value = pos.y
	_part_z_spin.value = pos.z
	_part_rotation_y_spin.value = _part_rotation_y[_selected_part_index] if has_part else 0.0
	_preview_scale_spin.value = _part_scales[_selected_part_index] if has_part else 1.0
	_suppress_preview_scale_changed = false
	_suppress_part_transform_changed = false

func _on_part_transform_changed(_value: float) -> void:
	if _suppress_part_transform_changed:
		return
	_apply_selected_part_transform()

func _apply_selected_part_transform() -> void:
	if _selected_part_index < 0 or _selected_part_index >= _lod_source_paths.size():
		return
	var pos := Vector3(_part_x_spin.value, _part_y_spin.value, _part_z_spin.value)
	var rot_y := float(_part_rotation_y_spin.value)
	var scale := maxf(0.001, float(_preview_scale_spin.value))
	_part_positions[_selected_part_index] = pos
	_part_rotation_y[_selected_part_index] = rot_y
	_part_scales[_selected_part_index] = scale
	_preview.set_mesh_part_transform(
		_selected_part_index,
		pos,
		rot_y,
		scale,
		_part_pivot_offsets[_selected_part_index]
	)
	_update_dim_label()

func _clear_mesh_parts() -> void:
	_lod_source_paths.clear()
	_part_positions.clear()
	_part_rotation_y.clear()
	_part_scales.clear()
	_part_pivot_offsets.clear()
	_part_aabbs.clear()
	_selected_part_index = -1
	if _lod_list:
		_lod_list.clear()
	if _preview and _preview.has_method("clear_mesh_parts"):
		_preview.clear_mesh_parts()
	_refresh_selected_part_controls()

func _selected_part_aabb() -> AABB:
	if _selected_part_index < 0 or _selected_part_index >= _part_aabbs.size():
		return AABB()
	return _part_aabbs[_selected_part_index]

# ──────────────────────────────────────────────────────────────────────────────
# Frontage
# ──────────────────────────────────────────────────────────────────────────────

func _on_set_front_from_view() -> void:
	var cam := get_viewport().get_camera_3d()
	if not cam:
		_log("[color=red]No active camera found.[/color]")
		return
	# The frontage is the face that looks toward the viewer.
	# Compute the direction from the building (at origin) toward the camera,
	# projected onto the XZ plane — this is the outward normal of the front face.
	var to_cam := cam.global_position
	var horizontal := Vector3(to_cam.x, 0.0, to_cam.z).normalized()
	if horizontal.length_squared() < 0.001:
		_log("[color=yellow]Camera is directly above — frontage unchanged.[/color]")
		return
	_set_frontage_forward(_snap_xz_to_cardinal(horizontal))
	if _main_entrance_auto:
		_set_main_entrance_position(_default_main_entrance_position(), true)
	_log("Frontage set: front face points toward camera.")

# ──────────────────────────────────────────────────────────────────────────────
# Export
# ──────────────────────────────────────────────────────────────────────────────

func _on_export_pressed() -> void:
	if _export_needs_pack_retarget_choice():
		_open_export_retarget_dialog()
		return
	_export_asset(false)

func _export_needs_pack_retarget_choice() -> bool:
	return (
		not _loaded_asset_pack_id.is_empty()
		and not _loaded_asset_id.is_empty()
		and _selected_pack_id.strip_edges() != _loaded_asset_pack_id
	)

func _open_export_retarget_dialog() -> void:
	_ensure_export_retarget_window()
	var target_pack := _selected_pack_id.strip_edges()
	var target_asset := _asset_id_edit.text.strip_edges()
	if _retarget_export_message_lbl:
		_retarget_export_message_lbl.text = (
			"This asset was loaded from:\n%s:%s\n\nExporting now targets:\n%s:%s\n\n"
			+ "Copy creates/updates the target asset and leaves the original untouched.\n"
			+ "Move creates/updates the target asset, then deletes the original asset folder after export succeeds."
		) % [_loaded_asset_pack_id, _loaded_asset_id, target_pack, target_asset]
	_retarget_export_window.popup_centered(Vector2i(560, 310))

func _ensure_export_retarget_window() -> void:
	if _retarget_export_window and is_instance_valid(_retarget_export_window):
		return
	_retarget_export_window = Window.new()
	_retarget_export_window.title = "Export To Different Pack"
	_retarget_export_window.min_size = Vector2i(520, 280)
	_retarget_export_window.size = Vector2i(560, 310)
	_retarget_export_window.close_requested.connect(func(): _retarget_export_window.hide())
	add_child(_retarget_export_window)

	var panel := PanelContainer.new()
	panel.set_anchors_preset(Control.PRESET_FULL_RECT)
	_retarget_export_window.add_child(panel)

	var margin := _add_panel_margin(panel)
	var root := VBoxContainer.new()
	root.name = "Root"
	root.size_flags_horizontal = Control.SIZE_EXPAND_FILL
	root.size_flags_vertical = Control.SIZE_EXPAND_FILL
	root.add_theme_constant_override("separation", PANEL_GAP)
	margin.add_child(root)

	var message := Label.new()
	message.name = "Message"
	message.autowrap_mode = TextServer.AUTOWRAP_WORD_SMART
	message.size_flags_horizontal = Control.SIZE_EXPAND_FILL
	message.size_flags_vertical = Control.SIZE_EXPAND_FILL
	message.add_theme_font_size_override("font_size", _font_size_label)
	root.add_child(message)
	_retarget_export_message_lbl = message

	var footer := HBoxContainer.new()
	footer.add_theme_constant_override("separation", PANEL_GAP)
	root.add_child(footer)
	var spacer := Control.new()
	spacer.size_flags_horizontal = Control.SIZE_EXPAND_FILL
	footer.add_child(spacer)
	var cancel_btn := Button.new()
	cancel_btn.text = "Cancel"
	cancel_btn.pressed.connect(func(): _retarget_export_window.hide())
	footer.add_child(cancel_btn)
	var copy_btn := Button.new()
	copy_btn.text = "Copy"
	copy_btn.pressed.connect(func():
		_retarget_export_window.hide()
		_export_asset(false)
	)
	footer.add_child(copy_btn)
	var move_btn := Button.new()
	move_btn.text = "Move"
	move_btn.pressed.connect(func():
		_retarget_export_window.hide()
		_export_asset(true)
	)
	footer.add_child(move_btn)
	_apply_editor_theme(_retarget_export_window)

func _export_asset(move_original_after_export: bool) -> void:
	var pack_id: String = _selected_pack_id.strip_edges()
	if pack_id.is_empty():
		_log("[color=red]Pack ID is required.[/color]")
		return

	var asset_id: String = _asset_id_edit.text.strip_edges()
	if asset_id.is_empty():
		_log("[color=red]Asset ID is required.[/color]")
		return
	var source_pack_id := _loaded_asset_pack_id
	var source_asset_id := _loaded_asset_id

	if _lod_source_paths.is_empty():
		_log("[color=red]At least one mesh part is required.[/color]")
		return

	var mesh_parts := []
	for i in _lod_source_paths.size():
		var src_path := _lod_source_paths[i]
		var part_name := _part_name_for_index(i)
		var pos := _part_positions[i]
		var pivot := _part_pivot_offsets[i]
		mesh_parts.append({
			"name": part_name,
			"position": [snappedf(pos.x, 0.01), snappedf(pos.y, 0.01), snappedf(pos.z, 0.01)],
			"rotation_degrees": [0.0, snappedf(_part_rotation_y[i], 0.01), 0.0],
			"scale": snappedf(_part_scales[i], 0.001),
			"pivot_offset": [snappedf(pivot.x, 0.001), snappedf(pivot.y, 0.001), snappedf(pivot.z, 0.001)],
			"lods": [{
				"file": src_path.get_file(),
				"distance_min_m": 0.0,
				"distance_max_m": null,
			}],
		})

	# Build entrance anchor from frontage forward.
	var fwd := _frontage_fwd
	var entrance_pos := _get_main_entrance_position()
	var anchors := [{
		"anchor_type": "entrance",
		"name": "main",
		"position": [
			snappedf(entrance_pos.x, 0.01),
			snappedf(entrance_pos.y, 0.01),
			snappedf(entrance_pos.z, 0.01),
		],
		"forward": [snappedf(fwd.x, 0.001), 0.0, snappedf(fwd.z, 0.001)],
	}]
	for anchor in _extra_anchors:
		anchors.append(anchor.duplicate(true))

	var tags_raw: String = _tags_edit.text.strip_edges()
	var tags: Array = []
	if not tags_raw.is_empty():
		for t in tags_raw.split(","):
			var trimmed := t.strip_edges()
			if not trimmed.is_empty():
				tags.append(trimmed)

	var asset_set_val = _asset_set_edit.text.strip_edges()
	var economy_profile_id := _selected_economy_profile_id()
	var placement_mode := _selected_placement_mode()
	var service_class := _selected_service_class()
	if service_class != "none" and placement_mode != "explicit":
		_log("[color=red]Service assets must use explicit placement.[/color]")
		return
	if _is_utility_service_class(service_class):
		if economy_profile_id.is_empty():
			_log("[color=red]Utility service assets require an economy profile.[/color]")
			return
		if not _economy_catalog_loaded:
			_log("[color=red]Economy catalog unavailable; utility service assets require a resolved matching profile.[/color]")
			return
		if not _utility_profile_matches_service(economy_profile_id, service_class):
			_log("[color=red]Selected economy profile does not match the utility service class.[/color]")
			return
	var lot_width := int(_width_spin.value)
	var lot_depth := int(_depth_spin.value)
	var min_zone_width := int(_min_zone_width_spin.value)
	var min_zone_depth := int(_min_zone_depth_spin.value)

	var params := {
		"pack_id":          pack_id,
		"pack_name":        _selected_pack_name.strip_edges(),
		"pack_author":      _selected_pack_author.strip_edges(),
		"asset_class":      "building",
		"asset_id":         asset_id,
		"display_name":     _display_name_edit.text.strip_edges(),
		"asset_set":        asset_set_val if not asset_set_val.is_empty() else null,
		"tags":             tags,
		"placement_mode":   placement_mode,
		"zone_type":        _selected_zone_type() if placement_mode == "zoned_private" else null,
		"density":          _selected_density() if placement_mode == "zoned_private" else null,
		"lot_width_cells":   lot_width,
		"lot_depth_cells":   lot_depth,
		"min_zone_width_cells": min_zone_width if placement_mode == "zoned_private" and min_zone_width != lot_width else null,
		"min_zone_depth_cells": min_zone_depth if placement_mode == "zoned_private" and min_zone_depth != lot_depth else null,
		"level":             int(_level_spin.value),
		"service_class":     service_class if service_class != "none" else null,
		"economy_profile":   economy_profile_id if not economy_profile_id.is_empty() else null,
		"household_capacity": int(_residents_spin.value) if _residents_spin.value > 0 else null,
		"flat_size_m2":      _flat_size_spin.value if _flat_size_spin.value > 0 else null,
		"worker_capacity":    int(_workers_spin.value)   if _workers_spin.value > 0 else null,
		"mesh_parts": mesh_parts,
		"anchors": anchors,
	}

	var output_dir: String = ProjectSettings.globalize_path("user://mods/" + pack_id + "/")
	var err: String = sim.validate_and_export_asset(JSON.stringify(params), output_dir)

	if err.is_empty():
		var asset_dir: String = output_dir + "assets/" + asset_id + "/"
		var copied := 0
		var copy_errors := 0
		var copied_dirs: Array[String] = []

		# Collect the set of filenames that should exist after this export.
		var expected_files: Array[String] = []
		for part in mesh_parts:
			var part_lods: Array = part["lods"]
			for lod_entry in part_lods:
				expected_files.append(lod_entry["file"])

		# Delete stale GLB files in the asset dir that are no longer referenced.
		var da_check := DirAccess.open(asset_dir)
		if da_check:
			da_check.list_dir_begin()
			var entry := da_check.get_next()
			while entry != "":
				if not da_check.current_is_dir() and (entry.ends_with(".glb") or entry.ends_with(".gltf") or entry.ends_with(".fbx")):
					if entry not in expected_files:
						DirAccess.remove_absolute(asset_dir + entry)
						_log("Removed stale file: %s" % entry)
				entry = da_check.get_next()
			da_check.list_dir_end()

		for i in _lod_source_paths.size():
			var src: String = _lod_source_paths[i]
			if src.is_empty() or not FileAccess.file_exists(src):
				_log("[color=yellow]Mesh part %d source not found on disk — skipped: %s[/color]" % [i, src])
				copy_errors += 1
				continue
			var dst: String = asset_dir + src.get_file()
			if src != dst:
				var copy_err := DirAccess.copy_absolute(src, dst)
				if copy_err != OK:
					_log("[color=red]Failed to copy mesh part %d '%s' (error %d)[/color]" % [i, src.get_file(), copy_err])
					copy_errors += 1
				else:
					copied += 1
			# Copy external texture/material files referenced by the GLB.
			var src_dir: String = src.get_base_dir()
			if src_dir in copied_dirs:
				continue
			copied_dirs.append(src_dir)
			var ext_refs := _glb_external_refs(src)
			for rel_path in ext_refs:
				var ref_src := src_dir + "/" + rel_path
				var ref_dst := asset_dir + rel_path
				if ref_src == ref_dst:
					continue
				var ref_dir_err := DirAccess.make_dir_recursive_absolute(ref_dst.get_base_dir())
				if ref_dir_err != OK:
					_log("[color=red]Could not create directory for external ref '%s' (error %d)[/color]" % [rel_path, ref_dir_err])
					copy_errors += 1
					continue
				var ref_err := DirAccess.copy_absolute(ref_src, ref_dst)
				if ref_err != OK:
					_log("[color=yellow]Could not copy external ref '%s' (error %d)[/color]" % [rel_path, ref_err])
					copy_errors += 1
				else:
					_log("Copied external ref: %s" % rel_path)
		_log("[color=green]Exported '%s:%s' → %s (%d mesh file(s) copied)[/color]" % [pack_id, asset_id, output_dir, copied])
		if copy_errors > 0:
			_log("[color=yellow]%d file(s) failed to copy — check paths.[/color]" % copy_errors)
		if move_original_after_export:
			_move_original_asset_after_export(
				source_pack_id,
				source_asset_id,
				pack_id,
				asset_id,
				copy_errors
			)
		_loaded_asset_pack_id = pack_id
		_loaded_asset_id = asset_id
		_refresh_asset_browser()
	else:
		_log("[color=red]Export failed:[/color]\n" + err)

func _part_name_for_index(index: int) -> String:
	if _lod_list and index >= 0 and index < _lod_list.item_count:
		var label := _lod_list.get_item_text(index)
		var split := label.split("  —  ")
		if split.size() > 0 and not str(split[0]).strip_edges().is_empty():
			return str(split[0]).strip_edges()
	return "part_%d" % [index + 1]

func _move_original_asset_after_export(
	source_pack_id: String,
	source_asset_id: String,
	target_pack_id: String,
	target_asset_id: String,
	copy_errors: int
) -> void:
	if source_pack_id.is_empty() or source_asset_id.is_empty():
		_log("[color=yellow]Move skipped: this editor state has no original asset path.[/color]")
		return
	if copy_errors > 0:
		_log("[color=yellow]Move skipped: export had file copy errors, so the original was left untouched.[/color]")
		return
	var source_dir := ProjectSettings.globalize_path(
		"user://mods/%s/assets/%s" % [source_pack_id, source_asset_id]
	)
	var target_dir := ProjectSettings.globalize_path(
		"user://mods/%s/assets/%s" % [target_pack_id, target_asset_id]
	)
	if source_dir == target_dir:
		return
	if not DirAccess.dir_exists_absolute(source_dir):
		_log("[color=yellow]Move skipped: original asset folder was not found: %s[/color]" % source_dir)
		return
	var err := _remove_dir_recursive(source_dir)
	if err != OK:
		_log("[color=red]Move exported the target, but failed to delete original asset folder '%s' (error %d).[/color]" % [source_dir, err])
		return
	_log("[color=green]Moved asset from '%s:%s' to '%s:%s'.[/color]" % [
		source_pack_id,
		source_asset_id,
		target_pack_id,
		target_asset_id,
	])

# Parses a GLB file's embedded JSON chunk and returns a list of external file
# URI references (images with a relative `uri`, not embedded buffer views or
# data: URIs). These are paths relative to the GLB's own directory.
func _glb_external_refs(glb_path: String) -> Array[String]:
	var result: Array[String] = []
	# FBX embeds or uses absolute texture paths — no relative URI refs to parse.
	if glb_path.get_extension().to_lower() == "fbx":
		return result
	var f := FileAccess.open(glb_path, FileAccess.READ)
	if not f:
		return result

	# GLB header: magic (4) + version (4) + total_length (4) = 12 bytes.
	var magic := f.get_32()
	if magic != 0x46546C67:  # "glTF" in little-endian
		# Not a GLB — treat as plain GLTF text.
		f.seek(0)
		var text := f.get_as_text()
		f.close()
		var gltf = JSON.parse_string(text)
		if gltf is Dictionary:
			result = _collect_uris(gltf)
		return result

	f.get_32()  # version
	f.get_32()  # total length

	# Chunk 0 header: chunk_length (4) + chunk_type (4).
	var chunk_len := f.get_32()
	var chunk_type := f.get_32()
	if chunk_type != 0x4E4F534A:  # "JSON"
		f.close()
		return result

	var json_bytes := f.get_buffer(chunk_len)
	f.close()

	var json_text := json_bytes.get_string_from_utf8()
	var gltf = JSON.parse_string(json_text)
	if gltf is Dictionary:
		result = _collect_uris(gltf)
	return result

# Extracts external URI references from a parsed GLTF JSON dictionary.
func _collect_uris(gltf: Dictionary) -> Array[String]:
	var refs: Array[String] = []
	for image in gltf.get("images", []):
		var uri: String = image.get("uri", "")
		# Skip empty URIs, data: URIs (embedded), and buffer-view references.
		if uri.is_empty() or uri.begins_with("data:"):
			continue
		refs.append(uri)
	return refs

# Recursively copies `src_dir` into `dst_dir`, creating directories as needed.
# Returns OK on success or the first non-OK error encountered.
func _save_config() -> void:
	_config.set_value("import", "last_glb_dir",    _last_glb_dir)
	_config.set_value("ui",     "theme_mode",      _theme_mode)
	_config.set_value("ui",     "font_size_header",  _font_size_header)
	_config.set_value("ui",     "font_size_section", _font_size_section)
	_config.set_value("ui",     "font_size_label",   _font_size_label)
	var err := _config.save(CONFIG_PATH)
	if err != OK:
		push_warning("AssetEditor: could not save config to %s (error %d)" % [CONFIG_PATH, err])

func _copy_dir_recursive(src_dir: String, dst_dir: String) -> Error:
	var err := DirAccess.make_dir_recursive_absolute(dst_dir)
	if err != OK:
		return err
	var da := DirAccess.open(src_dir)
	if not da:
		return FAILED
	da.list_dir_begin()
	var entry := da.get_next()
	while entry != "":
		var src_path := src_dir + "/" + entry
		var dst_path := dst_dir + "/" + entry
		if da.current_is_dir() and not entry.begins_with("."):
			err = _copy_dir_recursive(src_path, dst_path)
			if err != OK:
				da.list_dir_end()
				return err
		elif not da.current_is_dir():
			err = DirAccess.copy_absolute(src_path, dst_path)
			if err != OK:
				da.list_dir_end()
				return err
		entry = da.get_next()
	da.list_dir_end()
	return OK

func _remove_dir_recursive(path: String) -> Error:
	var da := DirAccess.open(path)
	if not da:
		return FAILED
	da.list_dir_begin()
	var entry := da.get_next()
	while entry != "":
		if entry == "." or entry == "..":
			entry = da.get_next()
			continue
		var child_path := path.path_join(entry)
		var err := OK
		if da.current_is_dir():
			err = _remove_dir_recursive(child_path)
		else:
			err = DirAccess.remove_absolute(child_path)
		if err != OK:
			da.list_dir_end()
			return err
		entry = da.get_next()
	da.list_dir_end()
	return DirAccess.remove_absolute(path)

# ──────────────────────────────────────────────────────────────────────────────
# UI helpers
# ──────────────────────────────────────────────────────────────────────────────

func _apply_editor_theme(root: Node) -> void:
	EditorTheme.apply_to_tree(root, _theme_mode)
	if _asset_context_menu:
		EditorTheme.style_popup_menu(_asset_context_menu, _theme_mode)
	if _pack_select_menu:
		EditorTheme.style_popup_menu(_pack_select_menu, _theme_mode)
	if _pack_create_window and is_instance_valid(_pack_create_window):
		EditorTheme.apply_to_tree(_pack_create_window, _theme_mode)
	if _retarget_export_window and is_instance_valid(_retarget_export_window):
		EditorTheme.apply_to_tree(_retarget_export_window, _theme_mode)

func _add_panel_margin(parent: Control) -> MarginContainer:
	var margin := MarginContainer.new()
	margin.size_flags_horizontal = Control.SIZE_EXPAND_FILL
	margin.size_flags_vertical = Control.SIZE_EXPAND_FILL
	margin.add_theme_constant_override("margin_left", PANEL_PAD)
	margin.add_theme_constant_override("margin_right", PANEL_PAD)
	margin.add_theme_constant_override("margin_top", PANEL_PAD)
	margin.add_theme_constant_override("margin_bottom", PANEL_PAD)
	parent.add_child(margin)
	return margin

func _add_inspector_tab(
	tabs: TabContainer,
	header: HBoxContainer,
	buttons: Array[Button],
	title: String
) -> VBoxContainer:
	var tab_index := buttons.size()
	var button := Button.new()
	button.text = title
	button.toggle_mode = true
	button.focus_mode = Control.FOCUS_NONE
	button.size_flags_horizontal = Control.SIZE_EXPAND_FILL
	button.pressed.connect(func():
		tabs.current_tab = tab_index
		_sync_inspector_tab_buttons(buttons, tab_index)
	)
	header.add_child(button)
	buttons.append(button)

	var scroll := ScrollContainer.new()
	scroll.name = title
	scroll.size_flags_horizontal = Control.SIZE_EXPAND_FILL
	scroll.size_flags_vertical = Control.SIZE_EXPAND_FILL
	scroll.horizontal_scroll_mode = ScrollContainer.SCROLL_MODE_DISABLED
	tabs.add_child(scroll)

	var box := VBoxContainer.new()
	box.size_flags_horizontal = Control.SIZE_EXPAND_FILL
	box.add_theme_constant_override("separation", PANEL_GAP)
	scroll.add_child(box)
	return box

func _sync_inspector_tab_buttons(buttons: Array[Button], active_index: int) -> void:
	for i in buttons.size():
		buttons[i].button_pressed = i == active_index

func _add_label(parent: Control, text: String, size: int) -> void:
	var lbl := Label.new()
	lbl.text = text
	lbl.autowrap_mode = TextServer.AUTOWRAP_WORD_SMART
	lbl.size_flags_horizontal = Control.SIZE_EXPAND_FILL
	lbl.add_theme_font_size_override("font_size", size)
	parent.add_child(lbl)

func _add_line_edit(parent: Control, placeholder: String, default_val: String) -> LineEdit:
	var edit := LineEdit.new()
	edit.placeholder_text = placeholder
	edit.text = default_val
	edit.size_flags_horizontal = Control.SIZE_EXPAND_FILL
	parent.add_child(edit)
	return edit

func _add_spinbox(parent: Control, label: String, min_val: float, max_val: float, default_val: float) -> SpinBox:
	var lbl := Label.new()
	lbl.text = label
	lbl.autowrap_mode = TextServer.AUTOWRAP_WORD_SMART
	lbl.size_flags_horizontal = Control.SIZE_EXPAND_FILL
	lbl.add_theme_font_size_override("font_size", _font_size_label)
	parent.add_child(lbl)
	var sb := SpinBox.new()
	sb.min_value = min_val
	sb.max_value = max_val
	sb.value = default_val
	sb.size_flags_horizontal = Control.SIZE_EXPAND_FILL
	parent.add_child(sb)
	return sb

func _set_frontage_forward(fwd: Vector3) -> void:
	var resolved := fwd
	if resolved.length_squared() < 0.001:
		resolved = Vector3.FORWARD
	_frontage_fwd = resolved.normalized()
	_frontage_lbl.text = "Forward: (%.2f, 0, %.2f)" % [_frontage_fwd.x, _frontage_fwd.z]
	_preview.set_frontage_forward(_frontage_fwd)
	_update_main_entrance_preview()

func _snap_xz_to_cardinal(dir: Vector3) -> Vector3:
	if absf(dir.x) >= absf(dir.z):
		return Vector3(1.0 if dir.x >= 0.0 else -1.0, 0.0, 0.0)
	return Vector3(0.0, 0.0, 1.0 if dir.z >= 0.0 else -1.0)

func _on_main_entrance_changed(_value: float) -> void:
	if _updating_main_entrance_fields:
		return
	_main_entrance_auto = false
	_update_main_entrance_preview()

func _on_reset_main_entrance_pressed() -> void:
	_set_main_entrance_position(_default_main_entrance_position(), true)
	_log("Main entrance reset to the current frontage edge.")

func _get_main_entrance_position() -> Vector3:
	return Vector3(_entrance_x_spin.value, _entrance_y_spin.value, _entrance_z_spin.value)

func _default_main_entrance_position() -> Vector3:
	var lot_half_w := _width_spin.value * 10.0 * 0.5
	var lot_half_d := _depth_spin.value * 10.0 * 0.5
	var fwd := _frontage_fwd
	if fwd.length_squared() < 0.001:
		fwd = Vector3.FORWARD
	if absf(fwd.x) >= absf(fwd.z):
		return Vector3((1.0 if fwd.x >= 0.0 else -1.0) * lot_half_w, 0.0, 0.0)
	return Vector3(0.0, 0.0, (1.0 if fwd.z >= 0.0 else -1.0) * lot_half_d)

func _set_main_entrance_position(pos: Vector3, auto_anchor: bool) -> void:
	_main_entrance_auto = auto_anchor
	_updating_main_entrance_fields = true
	_entrance_x_spin.value = pos.x
	_entrance_y_spin.value = pos.y
	_entrance_z_spin.value = pos.z
	_updating_main_entrance_fields = false
	_update_main_entrance_preview()

func _update_main_entrance_preview() -> void:
	if not _preview or not _entrance_x_spin or not _entrance_y_spin or not _entrance_z_spin:
		return
	_preview.set_entrance_anchor(_get_main_entrance_position(), _frontage_fwd)

func _on_mesh_loaded(aabb: AABB) -> void:
	_mesh_aabb = aabb
	# Compatibility path for one-shot preview loading: compute selected-part pivot data.
	_pivot_offset = Vector3(
		-(aabb.position.x + aabb.size.x * 0.5),
		-aabb.position.y,
		-(aabb.position.z + aabb.size.z * 0.5)
	)
	if _selected_part_index >= 0 and _selected_part_index < _part_aabbs.size():
		_part_aabbs[_selected_part_index] = aabb
		_part_pivot_offsets[_selected_part_index] = _pivot_offset
	if _autofit_on_load:
		_autofit_on_load = false
		_on_autofit_pressed()
	if _keep_camera:
		_keep_camera = false
	elif _cam_input:
		# Frame the camera on the lot footprint only on fresh imports.
		var lot_w := _width_spin.value * 10.0
		var lot_d := _depth_spin.value * 10.0
		_cam_input.focus_on(Vector3.ZERO, Vector2(lot_w, lot_d).length() * 0.6)
	_log("Mesh AABB: size=(%.2f, %.2f, %.2f)" % [aabb.size.x, aabb.size.y, aabb.size.z])

func _on_preview_scale_changed(value: float) -> void:
	if _suppress_preview_scale_changed:
		return
	_apply_selected_part_transform()
	# Revert preset display to "Custom" when spinner is edited directly.
	if _scale_preset_btn.selected != 0:
		_scale_preset_btn.selected = 0

func _on_scale_preset_selected(idx: int) -> void:
	if idx == 0:  # Custom — do nothing
		return
	if _selected_part_aabb().size.length() < 0.001:
		_log("[color=yellow]No mesh loaded — cannot apply preset.[/color]")
		_scale_preset_btn.selected = 0
		return
	match idx:
		1: _on_autofit_pressed()           # Fit to Lot
		2: _apply_scale_fraction(0.5)      # ½ Lot
		3: _apply_scale_fraction(0.25)     # ¼ Lot

func _apply_scale_fraction(fraction: float) -> void:
	var aabb := _selected_part_aabb()
	var lot_w := _width_spin.value * 10.0
	var lot_d := _depth_spin.value * 10.0
	var mesh_w := aabb.size.x
	var mesh_d := aabb.size.z
	if mesh_w < 0.001 or mesh_d < 0.001:
		return
	var fit_scale := minf(lot_w / mesh_w, lot_d / mesh_d)
	var scale := snappedf(fit_scale * fraction, 0.01)
	_preview_scale_spin.value = scale
	_apply_selected_part_transform()
	_update_dim_label()

func _suggest_capacity() -> void:
	var aabb := _selected_part_aabb()
	if aabb.size.length() < 0.001:
		return
	if _selected_placement_mode() != "zoned_private":
		_log("[color=yellow]Capacity suggestion currently supports zoned private buildings only.[/color]")
		return
	var s         := _preview_scale_spin.value
	var sw        := aabb.size.x * s
	var sd        := aabb.size.z * s
	var sh        := aabb.size.y * s
	# Residential roofs inflate height — discount to habitable portion before estimating floors.
	var res_h  := sh * 0.65
	var floors    := maxi(1, roundi(sh / 3.5))
	var res_floors := maxi(1, roundi(res_h / 3.5))
	var footprint := sw * sd
	var zone := _selected_zone_type()
	var density := _selected_density()
	# m² per person/worker by zone and density. Level does not affect capacity yet
	# (deferred until wealth/money system is implemented).
	var sqm_wrk := 20.0
	match density:
		"medium": sqm_wrk = 15.0
		"high":   sqm_wrk = 10.0
	match zone:
		"residential":
			var hh_count := int(_residents_spin.value)
			if hh_count <= 0:
				hh_count = 1
			
			var suggested_sqm := snappedf((footprint * res_floors) / float(hh_count), 0.1)
			_flat_size_spin.value = suggested_sqm
			_workers_spin.value = 0
		"commercial":
			_residents_spin.value = 0
			if _workers_spin.editable:
				_workers_spin.value = maxi(1, roundi(footprint * floors / sqm_wrk))
		"industrial":
			_residents_spin.value = 0
			# Industrial is slightly less dense than commercial
			if _workers_spin.editable:
				_workers_spin.value = maxi(1, roundi(footprint * floors / (sqm_wrk * 1.25)))

func _update_dim_label() -> void:
	var aabb := _selected_part_aabb()
	if aabb.size.length() < 0.001 or not _dim_label:
		return
	var s := _preview_scale_spin.value
	var w := snappedf(aabb.size.x * s, 0.1)
	var d := snappedf(aabb.size.z * s, 0.1)
	var h := snappedf(aabb.size.y * s, 0.1)
	_dim_label.text = "→ %.1fm × %.1fm × %.1fm" % [w, d, h]

func _on_human_toggled(pressed: bool) -> void:
	_human_visible = pressed
	_preview.set_show_human(pressed)

func _on_clear_ghost_pressed() -> void:
	_preview.clear_ghost()

func _input(event: InputEvent) -> void:
	if event is InputEventMouseButton:
		var mb := event as InputEventMouseButton
		if mb.button_index != MOUSE_BUTTON_LEFT and mb.button_index != MOUSE_BUTTON_RIGHT:
			return
		if mb.pressed:
			if not _is_mouse_in_3d_area():
				return
			var mouse_pos := get_viewport().get_mouse_position()
			if mb.button_index == MOUSE_BUTTON_LEFT:
				if _try_begin_main_entrance_drag(mouse_pos):
					get_viewport().set_input_as_handled()
					return
				if _try_begin_mesh_part_drag(mouse_pos):
					get_viewport().set_input_as_handled()
					return
				if _try_begin_ghost_drag(mouse_pos):
					get_viewport().set_input_as_handled()
					return
				if _human_visible and _place_human_from_mouse(mouse_pos):
					get_viewport().set_input_as_handled()
				return
			if mb.button_index == MOUSE_BUTTON_RIGHT and _try_begin_mesh_part_rotation(mouse_pos):
				get_viewport().set_input_as_handled()
			return
		if _dragging_main_entrance:
			_dragging_main_entrance = false
			get_viewport().set_input_as_handled()
		if _dragging_mesh_part:
			_dragging_mesh_part = false
			get_viewport().set_input_as_handled()
		if _rotating_mesh_part:
			_rotating_mesh_part = false
			get_viewport().set_input_as_handled()
		if _dragging_ghost:
			_dragging_ghost = false
			get_viewport().set_input_as_handled()
		return

	if event is InputEventMouseMotion and _dragging_main_entrance:
		if _drag_main_entrance_from_mouse(get_viewport().get_mouse_position()):
			get_viewport().set_input_as_handled()
	elif event is InputEventMouseMotion and _dragging_mesh_part:
		if _drag_mesh_part_from_mouse(get_viewport().get_mouse_position()):
			get_viewport().set_input_as_handled()
	elif event is InputEventMouseMotion and _rotating_mesh_part:
		_rotate_mesh_part_from_mouse(get_viewport().get_mouse_position())
		get_viewport().set_input_as_handled()
	elif event is InputEventMouseMotion and _dragging_ghost:
		if _drag_ghost_from_mouse(get_viewport().get_mouse_position()):
			get_viewport().set_input_as_handled()

func _is_mouse_in_3d_area() -> bool:
	var mouse_pos := get_viewport().get_mouse_position()
	if _preview_view_rect and is_instance_valid(_preview_view_rect):
		return _preview_view_rect.get_global_rect().has_point(mouse_pos)
	var vp_size   := get_viewport().get_visible_rect().size
	return (mouse_pos.x > PANEL_LEFT_W and
			mouse_pos.x < vp_size.x - PANEL_RIGHT_W and
			mouse_pos.y < vp_size.y - PANEL_BOT_H)

func _try_begin_main_entrance_drag(mouse_pos: Vector2) -> bool:
	if not _preview:
		return false
	var cam := get_viewport().get_camera_3d()
	if not cam:
		return false
	var anchor_world: Vector3 = _preview.get_entrance_anchor_world_position()
	if cam.is_position_behind(anchor_world):
		return false
	var anchor_screen: Vector2 = cam.unproject_position(anchor_world)
	if anchor_screen.distance_to(mouse_pos) > MAIN_ENTRANCE_PICK_RADIUS_PX:
		return false
	_dragging_main_entrance = true
	_main_entrance_auto = false
	return _drag_main_entrance_from_mouse(mouse_pos)

func _drag_main_entrance_from_mouse(mouse_pos: Vector2) -> bool:
	var anchor_pos := _get_main_entrance_position()
	var hit = _project_mouse_to_horizontal_plane(mouse_pos, anchor_pos.y)
	if hit == null:
		return false
	_set_main_entrance_position(Vector3(hit.x, anchor_pos.y, hit.z), false)
	return true

func _try_begin_mesh_part_drag(mouse_pos: Vector2) -> bool:
	var hit = _project_mouse_to_horizontal_plane(mouse_pos, 0.0)
	if hit == null:
		return false
	var part_index := _mesh_part_index_at_world_xz(hit)
	if part_index < 0:
		return false
	_select_mesh_part(part_index)
	var current_pos := _part_positions[_selected_part_index]
	_mesh_part_drag_offset = current_pos - Vector3(hit.x, current_pos.y, hit.z)
	_dragging_mesh_part = true
	return _drag_mesh_part_from_mouse(mouse_pos)

func _drag_mesh_part_from_mouse(mouse_pos: Vector2) -> bool:
	if not _has_selected_mesh_part():
		return false
	var hit = _project_mouse_to_horizontal_plane(mouse_pos, 0.0)
	if hit == null:
		return false
	var current_pos := _part_positions[_selected_part_index]
	var target := Vector3(hit.x, current_pos.y, hit.z) + _mesh_part_drag_offset
	_set_selected_mesh_part_position(target)
	return true

func _try_begin_mesh_part_rotation(mouse_pos: Vector2) -> bool:
	var hit = _project_mouse_to_horizontal_plane(mouse_pos, 0.0)
	if hit == null:
		return false
	var part_index := _mesh_part_index_at_world_xz(hit)
	if part_index < 0:
		return false
	_select_mesh_part(part_index)
	_mesh_part_rotate_start_x = mouse_pos.x
	_mesh_part_rotate_start_yaw = _part_rotation_y[_selected_part_index]
	_rotating_mesh_part = true
	return true

func _rotate_mesh_part_from_mouse(mouse_pos: Vector2) -> void:
	if not _has_selected_mesh_part():
		return
	var delta_px := mouse_pos.x - _mesh_part_rotate_start_x
	var raw_rotation := _mesh_part_rotate_start_yaw + delta_px * MESH_ROTATION_DRAG_DEG_PER_PX
	_set_selected_mesh_part_rotation_y(_snap_rotation_y_to_cardinal_if_close(raw_rotation))

func _has_selected_mesh_part() -> bool:
	return _selected_part_index >= 0 and _selected_part_index < _lod_source_paths.size()

func _mesh_part_index_at_world_xz(world_pos: Vector3) -> int:
	for i in range(_lod_source_paths.size() - 1, -1, -1):
		if _mesh_part_contains_world_xz(i, world_pos):
			return i
	return -1

func _mesh_part_contains_world_xz(part_index: int, world_pos: Vector3) -> bool:
	if part_index < 0 or part_index >= _lod_source_paths.size():
		return false
	var aabb := _part_aabbs[part_index]
	if aabb.size.x < 0.001 or aabb.size.z < 0.001:
		return false
	var scale := maxf(0.001, _part_scales[part_index])
	var yaw := deg_to_rad(_part_rotation_y[part_index])
	var pos := _part_positions[part_index]
	var rel := Vector3(world_pos.x - pos.x, 0.0, world_pos.z - pos.z)
	var local := (Basis(Vector3.UP, -yaw) * rel) * (1.0 / scale) - _part_pivot_offsets[part_index]
	var min_x := aabb.position.x - 0.75
	var max_x := aabb.position.x + aabb.size.x + 0.75
	var min_z := aabb.position.z - 0.75
	var max_z := aabb.position.z + aabb.size.z + 0.75
	return local.x >= min_x and local.x <= max_x and local.z >= min_z and local.z <= max_z

func _set_selected_mesh_part_position(position: Vector3) -> void:
	if not _has_selected_mesh_part():
		return
	_part_positions[_selected_part_index] = position
	_sync_selected_mesh_part_controls()
	_apply_selected_part_transform_from_state()

func _set_selected_mesh_part_rotation_y(rotation_y: float) -> void:
	if not _has_selected_mesh_part():
		return
	_part_rotation_y[_selected_part_index] = rotation_y
	_sync_selected_mesh_part_controls()
	_apply_selected_part_transform_from_state()

func _sync_selected_mesh_part_controls() -> void:
	if not _has_selected_mesh_part():
		return
	_suppress_part_transform_changed = true
	var pos := _part_positions[_selected_part_index]
	_part_x_spin.value = pos.x
	_part_y_spin.value = pos.y
	_part_z_spin.value = pos.z
	_part_rotation_y_spin.value = _part_rotation_y[_selected_part_index]
	_suppress_part_transform_changed = false

func _apply_selected_part_transform_from_state() -> void:
	if not _has_selected_mesh_part():
		return
	_preview.set_mesh_part_transform(
		_selected_part_index,
		_part_positions[_selected_part_index],
		_part_rotation_y[_selected_part_index],
		_part_scales[_selected_part_index],
		_part_pivot_offsets[_selected_part_index]
	)

func _normalize_degrees(value: float) -> float:
	var result := value
	while result > 180.0:
		result -= 360.0
	while result < -180.0:
		result += 360.0
	return snappedf(result, 0.1)

func _snap_rotation_y_to_cardinal_if_close(value: float) -> float:
	var normalized := _normalize_degrees(value)
	var nearest_cardinal := _normalize_degrees(roundf(normalized / 90.0) * 90.0)
	var distance := absf(_normalize_degrees(normalized - nearest_cardinal))
	if distance <= MESH_ROTATION_CARDINAL_SNAP_DEG:
		return nearest_cardinal
	return normalized

func _try_begin_ghost_drag(mouse_pos: Vector2) -> bool:
	if not _preview or not _preview.has_method("ghost_contains_world_xz"):
		return false
	var hit = _project_mouse_to_horizontal_plane(mouse_pos, 0.0)
	if hit == null:
		return false
	if not _preview.ghost_contains_world_xz(hit):
		return false
	_dragging_ghost = true
	_ghost_drag_offset = _preview.get_ghost_world_position() - Vector3(hit.x, 0.0, hit.z)
	return _drag_ghost_from_mouse(mouse_pos)

func _drag_ghost_from_mouse(mouse_pos: Vector2) -> bool:
	var hit = _project_mouse_to_horizontal_plane(mouse_pos, 0.0)
	if hit == null:
		return false
	var target := Vector3(hit.x, 0.0, hit.z) + _ghost_drag_offset
	_preview.set_ghost_world_position(target)
	return true

func _place_human_from_mouse(mouse_pos: Vector2) -> bool:
	var hit = _project_mouse_to_horizontal_plane(mouse_pos, 0.0)
	if hit == null:
		return false
	_preview.place_human_at(hit.x, hit.z)
	return true

func _project_mouse_to_horizontal_plane(mouse_pos: Vector2, plane_y: float):
	var cam := get_viewport().get_camera_3d()
	if not cam:
		return null
	var origin := cam.project_ray_origin(mouse_pos)
	var dir := cam.project_ray_normal(mouse_pos)
	if absf(dir.y) < 0.0001:
		return null
	var t := (plane_y - origin.y) / dir.y
	if t < 0.0:
		return null
	return origin + dir * t

func _on_copy_log_pressed() -> void:
	if _log_plain_lines.is_empty():
		DisplayServer.clipboard_set("")
		return
	DisplayServer.clipboard_set("\n".join(_log_plain_lines))

func _on_autofit_pressed() -> void:
	var aabb := _selected_part_aabb()
	if aabb.size.length() < 0.001:
		_log("[color=yellow]No mesh loaded yet — import a .glb first.[/color]")
		return
	var lot_w: float = _width_spin.value * 10.0   # CELL_M = 10
	var lot_d: float = _depth_spin.value * 10.0
	var mesh_w: float = aabb.size.x
	var mesh_d: float = aabb.size.z
	if mesh_w < 0.001 or mesh_d < 0.001:
		_log("[color=yellow]Mesh has zero XZ extent — cannot auto-fit.[/color]")
		return
	# Scale so the larger mesh dimension fills the corresponding lot dimension.
	var scale_x := lot_w / mesh_w
	var scale_z := lot_d / mesh_d
	var fit_scale := minf(scale_x, scale_z)
	_preview_scale_spin.value = snappedf(fit_scale, 0.01)
	_apply_selected_part_transform()
	_update_dim_label()
	if _scale_preset_btn:
		_scale_preset_btn.selected = 1  # Fit to Lot
	var scaled_w := snappedf(mesh_w * fit_scale, 0.1)
	var scaled_d := snappedf(mesh_d * fit_scale, 0.1)
	var scaled_h := snappedf(aabb.size.y * fit_scale, 0.1)
	_log("Building footprint: %.1fm × %.1fm × %.1fm scaled to fit %.0fm × %.0fm lot (scale %.2fx)" % [
		scaled_w, scaled_d, scaled_h, lot_w, lot_d, fit_scale])

func _log(msg: String) -> void:
	if _log_label:
		_log_label.append_text(msg + "\n")
	_log_plain_lines.append(_strip_bbcode(msg))

func _strip_bbcode(text: String) -> String:
	if _bbcode_strip_regex:
		return _bbcode_strip_regex.sub(text, "", true)
	return text
