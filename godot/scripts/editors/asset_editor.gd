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
const SelectionRectOverlay = preload("res://scripts/editors/selection_rect_overlay.gd")
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
const SITE_SURFACE_CONTEXT_ADD_VERTEX := 1
const SITE_SURFACE_CONTEXT_DELETE_VERTEX := 2
const PACK_MENU_CREATE_NEW := 1000000
const PACK_MENU_NO_PACKS := 1000001
const PACK_SCHEMA_VERSION := 1
const MESH_ROTATION_DRAG_DEG_PER_PX := 0.35
const MESH_ROTATION_CARDINAL_SNAP_DEG := 4.0
const SELECTION_DRAG_THRESHOLD_PX := 6.0
const SITE_ANCHOR_DRAG_RADIUS_M := 1.25
const SITE_SURFACE_VERTEX_PICK_RADIUS_M := 1.25
const SITE_SURFACE_EDGE_PICK_RADIUS_M := 1.10
const SITE_ANCHOR_DEFAULT_WIDTH_M := {
	"driveway": 3.0,
	"parking": 2.5,
	"loading_bay": 3.5,
}
const SITE_ANCHOR_DEFAULT_LENGTH_M := {
	"parking": 5.0,
	"loading_bay": 8.0,
}
const SITE_SURFACE_MATERIALS := [
	{"id": "asphalt", "label": "Asphalt"},
	{"id": "concrete", "label": "Concrete"},
	{"id": "gravel", "label": "Gravel"},
	{"id": "paving", "label": "Paving"},
]
const SITE_SURFACE_DEFAULT_SIZE_M := {
	"asphalt": Vector2(5.0, 7.0),
	"concrete": Vector2(1.4, 6.0),
	"gravel": Vector2(5.0, 7.0),
	"paving": Vector2(2.0, 4.0),
}
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
var _selection_rect_overlay: Control
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
var _selected_part_indices: Array[int] = []
var _updating_mesh_part_selection: bool = false
var _frontage_lbl: Label  # shows current frontage forward vector
var _site_anchor_list: ItemList
var _site_anchor_name_edit: LineEdit
var _site_anchor_vehicle_class_btn: OptionButton
var _site_anchor_x_spin: SpinBox
var _site_anchor_y_spin: SpinBox
var _site_anchor_z_spin: SpinBox
var _site_anchor_yaw_spin: SpinBox
var _site_anchor_width_spin: SpinBox
var _site_anchor_length_spin: SpinBox
var _site_surface_list: ItemList
var _site_surface_name_edit: LineEdit
var _site_surface_material_btn: OptionButton
var _site_surface_y_spin: SpinBox
var _site_surface_context_menu: PopupMenu
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
var _site_anchors_data: Array[Dictionary] = []
var _site_surfaces_data: Array[Dictionary] = []
var _selected_site_anchor_index: int = -1
var _selected_site_anchor_indices: Array[int] = []
var _selected_site_surface_index: int = -1
var _updating_site_anchor_controls: bool = false
var _updating_site_anchor_list: bool = false
var _updating_site_surface_controls: bool = false
var _updating_site_surface_list: bool = false
var _dragging_site_anchor: bool = false
var _dragging_site_surface: bool = false
var _dragging_site_surface_vertex: bool = false
var _dragging_mesh_part: bool = false
var _selecting_mesh_parts: bool = false
var _rotating_site_anchor: bool = false
var _rotating_mesh_part: bool = false
var _site_anchor_drag_offset: Vector3 = Vector3.ZERO
var _site_anchor_drag_start_hit: Vector3 = Vector3.ZERO
var _site_anchor_drag_start_positions: Array[Vector3] = []
var _site_anchor_rotate_start_x: float = 0.0
var _site_anchor_rotate_start_yaw: float = 0.0
var _site_surface_drag_start_hit: Vector3 = Vector3.ZERO
var _site_surface_drag_start_vertices: Array = []
var _site_surface_drag_index: int = -1
var _site_surface_vertex_drag_index: int = -1
var _site_surface_context_index: int = -1
var _site_surface_context_vertex_index: int = -1
var _site_surface_context_edge_index: int = -1
var _site_surface_context_insert_point: Vector2 = Vector2.ZERO
var _mesh_part_drag_start_hit: Vector3 = Vector3.ZERO
var _mesh_part_drag_start_positions: Array[Vector3] = []
var _mesh_part_rotate_start_x: float = 0.0
var _mesh_part_rotate_start_yaw: float = 0.0
var _selection_start_screen: Vector2 = Vector2.ZERO
var _selection_end_screen: Vector2 = Vector2.ZERO
var _selection_additive: bool = false
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
	_set_main_entrance_forward(_frontage_fwd)
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
	if _preview.has_method("set_theme_mode"):
		_preview.set_theme_mode(_theme_mode)
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
	_selection_rect_overlay = SelectionRectOverlay.new()
	_selection_rect_overlay.set_anchors_preset(Control.PRESET_FULL_RECT)
	_selection_rect_overlay.mouse_filter = Control.MOUSE_FILTER_IGNORE
	center.add_child(_selection_rect_overlay)
	if _cam_input:
		_cam_input.viewport_rect_control = center

	_build_right_panel(_right_split)
	_sync_preview_lot_size_from_fields()
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
	if _preview and _preview.has_method("set_theme_mode"):
		_preview.set_theme_mode(_theme_mode)
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
	_lod_list.select_mode = ItemList.SELECT_MULTI
	_lod_list.item_selected.connect(_on_mesh_part_selected)
	asset_box.add_child(_lod_list)
	var add_lod_btn := Button.new()
	add_lod_btn.text = "Add Mesh Part..."
	add_lod_btn.pressed.connect(_on_add_lod_pressed)
	asset_box.add_child(add_lod_btn)
	var remove_lod_btn := Button.new()
	remove_lod_btn.text = "Remove Mesh Part"
	remove_lod_btn.pressed.connect(_on_remove_mesh_parts_pressed)
	asset_box.add_child(remove_lod_btn)

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

	var yard_box := _add_inspector_tab(tabs, tab_header, tab_buttons, "Yard")
	_add_label(yard_box, "Site Surfaces", _font_size_section)
	_site_surface_list = ItemList.new()
	_site_surface_list.custom_minimum_size.y = 140
	_site_surface_list.size_flags_vertical = Control.SIZE_EXPAND_FILL
	_site_surface_list.select_mode = ItemList.SELECT_SINGLE
	_site_surface_list.item_selected.connect(_on_site_surface_selected)
	yard_box.add_child(_site_surface_list)
	_site_surface_context_menu = PopupMenu.new()
	_site_surface_context_menu.id_pressed.connect(_on_site_surface_context_menu_id_pressed)
	yard_box.add_child(_site_surface_context_menu)
	EditorTheme.style_popup_menu(_site_surface_context_menu, _theme_mode)

	var surface_button_row := HBoxContainer.new()
	surface_button_row.size_flags_horizontal = Control.SIZE_EXPAND_FILL
	surface_button_row.add_theme_constant_override("separation", PANEL_GAP)
	yard_box.add_child(surface_button_row)
	for material in SITE_SURFACE_MATERIALS:
		var add_surface_btn := Button.new()
		add_surface_btn.text = str(material["label"])
		add_surface_btn.size_flags_horizontal = Control.SIZE_EXPAND_FILL
		add_surface_btn.pressed.connect(Callable(self, "_add_site_surface").bind(str(material["id"])))
		surface_button_row.add_child(add_surface_btn)
	var remove_surface_btn := Button.new()
	remove_surface_btn.text = "Remove Surface"
	remove_surface_btn.pressed.connect(_remove_selected_site_surface)
	yard_box.add_child(remove_surface_btn)

	_site_surface_name_edit = _add_line_edit(yard_box, "Name", "")
	_site_surface_name_edit.text_changed.connect(_on_site_surface_text_changed)
	_add_label(yard_box, "Material", _font_size_label)
	_site_surface_material_btn = OptionButton.new()
	for material in SITE_SURFACE_MATERIALS:
		_site_surface_material_btn.add_item(str(material["label"]))
		_site_surface_material_btn.set_item_metadata(
			_site_surface_material_btn.get_item_count() - 1,
			str(material["id"])
		)
	_site_surface_material_btn.item_selected.connect(_on_site_surface_material_selected)
	yard_box.add_child(_site_surface_material_btn)
	_site_surface_y_spin = _add_spinbox(yard_box, "Surface Y (m)", -5.0, 5.0, 0.01)
	_site_surface_y_spin.step = 0.01
	_site_surface_y_spin.value_changed.connect(_on_site_surface_spin_changed)
	_refresh_site_surface_list()

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
	var reset_entrance_btn := Button.new()
	reset_entrance_btn.text = "Reset Entrance To Frontage"
	reset_entrance_btn.pressed.connect(_on_reset_main_entrance_pressed)
	anchors_box.add_child(reset_entrance_btn)

	_add_label(anchors_box, "Anchors", _font_size_section)
	_site_anchor_list = ItemList.new()
	_site_anchor_list.custom_minimum_size.y = 130
	_site_anchor_list.size_flags_vertical = Control.SIZE_EXPAND_FILL
	_site_anchor_list.select_mode = ItemList.SELECT_MULTI
	_site_anchor_list.item_selected.connect(_on_site_anchor_selected)
	_site_anchor_list.multi_selected.connect(_on_site_anchor_multi_selected)
	anchors_box.add_child(_site_anchor_list)

	var site_button_row := HBoxContainer.new()
	site_button_row.size_flags_horizontal = Control.SIZE_EXPAND_FILL
	anchors_box.add_child(site_button_row)
	for anchor_type in ["driveway", "parking", "loading_bay"]:
		var add_anchor_btn := Button.new()
		add_anchor_btn.text = _site_anchor_type_label(anchor_type)
		add_anchor_btn.size_flags_horizontal = Control.SIZE_EXPAND_FILL
		add_anchor_btn.pressed.connect(Callable(self, "_add_site_anchor").bind(anchor_type))
		site_button_row.add_child(add_anchor_btn)
	var remove_anchor_btn := Button.new()
	remove_anchor_btn.text = "Remove Anchor"
	remove_anchor_btn.pressed.connect(_remove_selected_site_anchor)
	anchors_box.add_child(remove_anchor_btn)

	_site_anchor_name_edit = _add_line_edit(anchors_box, "Name", "")
	_site_anchor_name_edit.text_changed.connect(_on_site_anchor_text_changed)

	_add_label(anchors_box, "Vehicle Class", _font_size_label)
	_site_anchor_vehicle_class_btn = OptionButton.new()
	for class_id in ["car", "freight", "service"]:
		_site_anchor_vehicle_class_btn.add_item(class_id.capitalize())
		_site_anchor_vehicle_class_btn.set_item_metadata(
			_site_anchor_vehicle_class_btn.get_item_count() - 1,
			class_id
		)
	_site_anchor_vehicle_class_btn.item_selected.connect(_on_site_anchor_vehicle_class_selected)
	anchors_box.add_child(_site_anchor_vehicle_class_btn)

	_site_anchor_x_spin = _add_spinbox(anchors_box, "Anchor X (m)", -500.0, 500.0, 0.0)
	_site_anchor_y_spin = _add_spinbox(anchors_box, "Anchor Y (m)", -500.0, 500.0, 0.0)
	_site_anchor_z_spin = _add_spinbox(anchors_box, "Anchor Z (m)", -500.0, 500.0, 0.0)
	_site_anchor_yaw_spin = _add_spinbox(anchors_box, "Anchor Rotation Y", -180.0, 180.0, 0.0)
	_site_anchor_width_spin = _add_spinbox(anchors_box, "Width (m)", 0.1, 50.0, 3.0)
	_site_anchor_length_spin = _add_spinbox(anchors_box, "Length (m)", 0.0, 100.0, 0.0)
	for spin in [
		_site_anchor_x_spin,
		_site_anchor_y_spin,
		_site_anchor_z_spin,
		_site_anchor_yaw_spin,
		_site_anchor_width_spin,
		_site_anchor_length_spin,
	]:
		spin.step = 0.1
		spin.value_changed.connect(_on_site_anchor_spin_changed)
	_refresh_site_anchor_list()

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
	_site_anchors_data.clear()
	_site_surfaces_data.clear()
	_selected_site_anchor_index = -1
	_selected_site_anchor_indices.clear()
	_selected_site_surface_index = -1
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
	_set_main_entrance_forward(Vector3.FORWARD)
	_set_main_entrance_position(_default_main_entrance_position(), true)
	_refresh_site_anchor_list()
	_refresh_site_surface_list()
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
	_site_anchors_data.clear()
	_site_surfaces_data.clear()

	var main_anchor_pos := _default_main_entrance_position()
	var main_anchor_fwd := Vector3.FORWARD
	var has_main_anchor := false
	var loaded_anchors: Array[Dictionary] = []
	for anchor in data.get("anchors", []):
		if not (anchor is Dictionary):
			continue
		var anchor_dict: Dictionary = anchor
		var anchor_type := str(anchor_dict.get("anchor_type", "")).strip_edges()
		var anchor_name := str(anchor_dict.get("name", "")).strip_edges()
		if anchor_type == "entrance":
			if anchor_name == "main" and not has_main_anchor:
				var pos = anchor_dict.get("position", [])
				if pos is Array and pos.size() == 3:
					main_anchor_pos = Vector3(float(pos[0]), float(pos[1]), float(pos[2]))
				var fwd = anchor_dict.get("forward", [])
				if fwd is Array and fwd.size() == 3:
					main_anchor_fwd = Vector3(float(fwd[0]), float(fwd[1]), float(fwd[2]))
				has_main_anchor = true
			continue
		loaded_anchors.append(_sanitize_site_anchor_dict(anchor_dict))

	var entrance_anchor := _make_main_entrance_anchor(
		main_anchor_pos,
		main_anchor_fwd if has_main_anchor else Vector3.FORWARD
	)
	_site_anchors_data.append(entrance_anchor)
	for anchor in loaded_anchors:
		_site_anchors_data.append(anchor)
	_selected_site_anchor_index = -1
	_selected_site_anchor_indices.clear()
	_selected_site_surface_index = -1

	_set_frontage_forward(_anchor_forward(entrance_anchor))
	if not has_main_anchor:
		_set_main_entrance_position(_default_main_entrance_position(), true)
		_log("[color=yellow]Loaded asset has no 'entrance/main' anchor; using frontage default.[/color]")
	else:
		_main_entrance_auto = false
		_update_site_anchor_preview()
	_refresh_site_anchor_list()

	for surface in data.get("site_surfaces", []):
		if surface is Dictionary:
			_site_surfaces_data.append(_sanitize_site_surface_dict(surface))
	_refresh_site_surface_list()

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
	_sync_preview_lot_size_from_fields()
	if _main_entrance_auto:
		_set_main_entrance_position(_default_main_entrance_position(), true)
	else:
		_update_main_entrance_preview()
	_clamp_mesh_parts_to_lot()
	_clamp_site_anchors_to_lot()
	_clamp_site_surfaces_to_lot()
	_update_economy_profile_status()

func _sync_preview_lot_size_from_fields() -> void:
	if not _preview or not _width_spin or not _depth_spin:
		return
	_preview.set_lot_size(int(_width_spin.value), int(_depth_spin.value))

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
	var clean := _asset_id_slug_from_display_name(_display_name_edit.text)
	# Set text without triggering the manual-edit flag.
	_asset_id_edit.text_changed.disconnect(_on_asset_id_text_changed)
	_asset_id_edit.text = "building.%s.%s" % [prefix, clean]
	_asset_id_edit.text_changed.connect(_on_asset_id_text_changed)

func _asset_id_slug_from_display_name(display_name: String) -> String:
	var clean := ""
	var pending_separator := false
	for ch in display_name.strip_edges().to_lower():
		var code := ch.unicode_at(0)
		var valid := (code >= 97 and code <= 122) or (code >= 48 and code <= 57)
		if valid:
			if pending_separator and not clean.is_empty():
				clean += "_"
			clean += ch
			pending_separator = false
		elif not clean.is_empty():
			pending_separator = true
	if clean.is_empty():
		return "unnamed"
	return clean

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

func _on_remove_mesh_parts_pressed() -> void:
	_remove_selected_mesh_parts()

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
	_part_positions[idx] = _clamp_mesh_part_position_to_lot(idx, position)
	_preview.set_mesh_part_transform(
		idx,
		_part_positions[idx],
		rotation_y,
		_part_scales[idx],
		stored_pivot
	)
	return idx

func _select_mesh_part(index: int, clear_site_anchors: bool = true) -> void:
	if index < 0 or index >= _lod_source_paths.size():
		_set_selected_mesh_parts([], -1)
		if clear_site_anchors:
			_set_selected_site_anchors([], -1)
		_set_selected_site_surface(-1)
		return
	_set_selected_mesh_parts([index], index)
	if clear_site_anchors:
		_set_selected_site_anchors([], -1)
	_set_selected_site_surface(-1)

func _on_mesh_part_selected(index: int) -> void:
	if _updating_mesh_part_selection:
		return
	_select_mesh_part(index)

func _set_selected_mesh_parts(indices: Array, primary_index: int = -1) -> void:
	var seen := {}
	var resolved: Array[int] = []
	for raw_index in indices:
		var index := int(raw_index)
		if index < 0 or index >= _lod_source_paths.size() or seen.has(index):
			continue
		seen[index] = true
		resolved.append(index)
	resolved.sort()
	if resolved.is_empty():
		_selected_part_index = -1
	else:
		_selected_part_index = primary_index if resolved.has(primary_index) else int(resolved[0])
	_selected_part_indices = resolved
	_updating_mesh_part_selection = true
	if _lod_list:
		_lod_list.deselect_all()
		for index in _selected_part_indices:
			if index >= 0 and index < _lod_list.item_count:
				_lod_list.select(index, false)
	_updating_mesh_part_selection = false
	if _selected_part_index >= 0:
		_mesh_aabb = _part_aabbs[_selected_part_index]
		_pivot_offset = _part_pivot_offsets[_selected_part_index]
	else:
		_mesh_aabb = AABB()
		_pivot_offset = Vector3.ZERO
	if _preview and _preview.has_method("set_selected_mesh_parts"):
		_preview.set_selected_mesh_parts(_selected_part_indices, _selected_part_index)
	_refresh_selected_part_controls()
	_update_dim_label()

func _remove_selected_mesh_parts() -> bool:
	var remove_indices := _selected_part_indices.duplicate()
	if remove_indices.is_empty() and _has_selected_mesh_part():
		remove_indices.append(_selected_part_index)
	if remove_indices.is_empty():
		_log("[color=yellow]No mesh part selected to remove.[/color]")
		return false

	remove_indices.sort()
	var first_removed := int(remove_indices[0])
	for i in range(remove_indices.size() - 1, -1, -1):
		var index := int(remove_indices[i])
		if index < 0 or index >= _lod_source_paths.size():
			continue
		_lod_source_paths.remove_at(index)
		_part_positions.remove_at(index)
		_part_rotation_y.remove_at(index)
		_part_scales.remove_at(index)
		_part_pivot_offsets.remove_at(index)
		_part_aabbs.remove_at(index)
		if _lod_list and index < _lod_list.item_count:
			_lod_list.remove_item(index)

	if _preview and _preview.has_method("remove_mesh_parts"):
		_preview.remove_mesh_parts(remove_indices)

	var next_index := -1
	if not _lod_source_paths.is_empty():
		next_index = mini(first_removed, _lod_source_paths.size() - 1)
	_set_selected_mesh_parts([next_index] if next_index >= 0 else [], next_index)
	_log("Removed %d mesh part(s)." % remove_indices.size())
	return true

func _site_surface_material_label(material: String) -> String:
	match material:
		"asphalt":
			return "Asphalt"
		"concrete":
			return "Concrete"
		"gravel":
			return "Gravel"
		"paving":
			return "Paving"
		_:
			return material.capitalize()

func _add_site_surface(material: String) -> void:
	var size: Vector2 = SITE_SURFACE_DEFAULT_SIZE_M.get(material, Vector2(3.0, 5.0))
	var center := _default_site_surface_position(material)
	var half := size * 0.5
	var surface := {
		"material": material,
		"name": "",
		"y_m": 0.01,
		"vertices": [
			[snappedf(center.x - half.x, 0.01), snappedf(center.z - half.y, 0.01)],
			[snappedf(center.x + half.x, 0.01), snappedf(center.z - half.y, 0.01)],
			[snappedf(center.x + half.x, 0.01), snappedf(center.z + half.y, 0.01)],
			[snappedf(center.x - half.x, 0.01), snappedf(center.z + half.y, 0.01)],
		],
	}
	_clamp_site_surface_vertices_to_lot(surface)
	_site_surfaces_data.append(surface)
	_set_selected_site_surface(_site_surfaces_data.size() - 1)
	_log("Added %s site surface." % _site_surface_material_label(material))

func _default_site_surface_position(material: String) -> Vector3:
	var count := _site_surface_count(material)
	var side_offset := float((count % 5) - 2) * 2.0
	var depth_offset := float(count / 5) * 2.0
	var fwd := _frontage_fwd.normalized()
	if fwd.length_squared() < 0.001:
		fwd = Vector3.FORWARD
	var side := Vector3(-fwd.z, 0.0, fwd.x)
	return _clamp_anchor_position_to_lot(side * side_offset - fwd * depth_offset)

func _site_surface_count(material: String) -> int:
	var count := 0
	for surface in _site_surfaces_data:
		if str(surface.get("material", "")).strip_edges() == material:
			count += 1
	return count

func _refresh_site_surface_list() -> void:
	if not _site_surface_list:
		return
	_updating_site_surface_list = true
	_site_surface_list.clear()
	var material_counts := {}
	for i in _site_surfaces_data.size():
		var surface := _site_surfaces_data[i]
		var material := str(surface.get("material", "")).strip_edges()
		material_counts[material] = int(material_counts.get(material, 0)) + 1
		_site_surface_list.add_item(_site_surface_display_label(i, material_counts[material]))
	if _selected_site_surface_index >= 0 and _selected_site_surface_index < _site_surface_list.item_count:
		_site_surface_list.select(_selected_site_surface_index, false)
	_updating_site_surface_list = false
	_update_site_surface_controls()
	_update_site_surface_preview()

func _site_surface_display_label(index: int, material_index: int = -1) -> String:
	if index < 0 or index >= _site_surfaces_data.size():
		return "Surface"
	var surface := _site_surfaces_data[index]
	var material := str(surface.get("material", "")).strip_edges()
	var name := str(surface.get("name", "")).strip_edges()
	if not name.is_empty():
		return "%s - %s" % [_site_surface_material_label(material), name]
	if material_index < 0:
		material_index = _site_surface_index_among_material(index)
	return "%s %d" % [_site_surface_material_label(material), material_index]

func _site_surface_index_among_material(index: int) -> int:
	if index < 0 or index >= _site_surfaces_data.size():
		return 0
	var material := str(_site_surfaces_data[index].get("material", "")).strip_edges()
	var count := 0
	for i in index + 1:
		if str(_site_surfaces_data[i].get("material", "")).strip_edges() == material:
			count += 1
	return count

func _set_selected_site_surface(index: int) -> void:
	_selected_site_surface_index = index if index >= 0 and index < _site_surfaces_data.size() else -1
	_refresh_site_surface_list()
	if _selected_site_surface_index >= 0:
		_set_selected_mesh_parts([], -1)
		_set_selected_site_anchors([], -1)

func _on_site_surface_selected(index: int) -> void:
	if _updating_site_surface_list:
		return
	_set_selected_site_surface(index)

func _remove_selected_site_surface() -> bool:
	if _selected_site_surface_index < 0 or _selected_site_surface_index >= _site_surfaces_data.size():
		_log("[color=yellow]No site surface selected to remove.[/color]")
		return false
	var material := str(_site_surfaces_data[_selected_site_surface_index].get("material", ""))
	_site_surfaces_data.remove_at(_selected_site_surface_index)
	var next_index := -1
	if not _site_surfaces_data.is_empty():
		next_index = mini(_selected_site_surface_index, _site_surfaces_data.size() - 1)
	_selected_site_surface_index = next_index
	_refresh_site_surface_list()
	_log("Removed %s site surface." % _site_surface_material_label(material))
	return true

func _update_site_surface_controls() -> void:
	_updating_site_surface_controls = true
	var has_surface := _selected_site_surface_index >= 0 and _selected_site_surface_index < _site_surfaces_data.size()
	var surface := _site_surfaces_data[_selected_site_surface_index] if has_surface else {}
	_site_surface_name_edit.text = str(surface.get("name", "")) if has_surface else ""
	_set_option_by_metadata(_site_surface_material_btn, str(surface.get("material", "asphalt")))
	_site_surface_y_spin.value = _site_surface_number(surface, "y_m", 0.01) if has_surface else 0.0
	_site_surface_name_edit.editable = has_surface
	_site_surface_material_btn.disabled = not has_surface
	_site_surface_y_spin.editable = has_surface
	_updating_site_surface_controls = false

func _on_site_surface_text_changed(_value: String) -> void:
	if _updating_site_surface_controls:
		return
	_apply_site_surface_controls()

func _on_site_surface_material_selected(_index: int) -> void:
	if _updating_site_surface_controls:
		return
	_apply_site_surface_controls()

func _on_site_surface_spin_changed(_value: float) -> void:
	if _updating_site_surface_controls:
		return
	_apply_site_surface_controls()

func _apply_site_surface_controls() -> void:
	if _selected_site_surface_index < 0 or _selected_site_surface_index >= _site_surfaces_data.size():
		return
	var surface := _site_surfaces_data[_selected_site_surface_index]
	surface["name"] = _site_surface_name_edit.text.strip_edges()
	surface["material"] = _selected_option_metadata(_site_surface_material_btn, "asphalt")
	surface["y_m"] = snappedf(float(_site_surface_y_spin.value), 0.01)
	_refresh_site_surface_list()

func _update_site_surface_preview() -> void:
	if _preview and _preview.has_method("set_site_surfaces"):
		_preview.set_site_surfaces(_site_surfaces_data, _selected_site_surface_index)

func _site_surface_number(surface: Dictionary, key: String, fallback: float) -> float:
	var value = surface.get(key, null)
	if value == null:
		return fallback
	if _anchor_value_is_number(value):
		return float(value)
	if value is String:
		var text := (value as String).strip_edges()
		if text.is_valid_float():
			return text.to_float()
	return fallback

func _site_surface_vertices(surface: Dictionary) -> Array[Vector2]:
	var vertices: Array[Vector2] = []
	var raw_vertices = surface.get("vertices", [])
	if raw_vertices is Array:
		for raw_vertex in raw_vertices:
			if raw_vertex is Array and raw_vertex.size() >= 2:
				vertices.append(Vector2(float(raw_vertex[0]), float(raw_vertex[1])))
	return vertices

func _site_surface_vertices_to_arrays(vertices: Array[Vector2]) -> Array:
	var result := []
	for vertex in vertices:
		result.append([snappedf(vertex.x, 0.01), snappedf(vertex.y, 0.01)])
	return result

func _site_surface_polygon_area(vertices: Array[Vector2]) -> float:
	if vertices.size() < 3:
		return 0.0
	var twice_area := 0.0
	for i in vertices.size():
		var a := vertices[i]
		var b := vertices[(i + 1) % vertices.size()]
		twice_area += a.x * b.y - b.x * a.y
	return twice_area * 0.5

func _site_surface_polygon_is_valid(vertices: Array[Vector2]) -> bool:
	if vertices.size() < 3:
		return false
	if absf(_site_surface_polygon_area(vertices)) <= 0.001:
		return false
	for i in vertices.size():
		for j in range(i + 1, vertices.size()):
			var i_next := (i + 1) % vertices.size()
			var j_next := (j + 1) % vertices.size()
			if i == j or i == j_next or i_next == j or i_next == j_next:
				continue
			if _segments_intersect_2d(vertices[i], vertices[i_next], vertices[j], vertices[j_next]):
				return false
	return true

func _segments_intersect_2d(a: Vector2, b: Vector2, c: Vector2, d: Vector2) -> bool:
	var eps := 0.0001
	var ab_c := _orientation_2d(a, b, c)
	var ab_d := _orientation_2d(a, b, d)
	var cd_a := _orientation_2d(c, d, a)
	var cd_b := _orientation_2d(c, d, b)
	if absf(ab_c) <= eps and _point_on_segment_2d(a, b, c):
		return true
	if absf(ab_d) <= eps and _point_on_segment_2d(a, b, d):
		return true
	if absf(cd_a) <= eps and _point_on_segment_2d(c, d, a):
		return true
	if absf(cd_b) <= eps and _point_on_segment_2d(c, d, b):
		return true
	return (ab_c > eps) != (ab_d > eps) and (cd_a > eps) != (cd_b > eps)

func _orientation_2d(a: Vector2, b: Vector2, c: Vector2) -> float:
	return (b.x - a.x) * (c.y - a.y) - (b.y - a.y) * (c.x - a.x)

func _point_on_segment_2d(a: Vector2, b: Vector2, p: Vector2) -> bool:
	var eps := 0.0001
	return (
		p.x >= minf(a.x, b.x) - eps
		and p.x <= maxf(a.x, b.x) + eps
		and p.y >= minf(a.y, b.y) - eps
		and p.y <= maxf(a.y, b.y) + eps
	)

func _sanitize_site_surface_dict(surface: Dictionary) -> Dictionary:
	var clean := surface.duplicate(true)
	var material := str(clean.get("material", "asphalt")).strip_edges()
	if not _site_surface_material_is_valid(material):
		material = "asphalt"
	clean["material"] = material
	if clean.get("name", null) == null:
		clean["name"] = ""
	clean["y_m"] = snappedf(_site_surface_number(clean, "y_m", 0.01), 0.01)
	var vertices := _site_surface_vertices(clean)
	clean["vertices"] = _site_surface_vertices_to_arrays(vertices)
	_clamp_site_surface_vertices_to_lot(clean)
	return clean

func _site_surface_material_is_valid(material: String) -> bool:
	for entry in SITE_SURFACE_MATERIALS:
		if str(entry["id"]) == material:
			return true
	return false

func _export_site_surface(surface: Dictionary) -> Dictionary:
	var clean := _sanitize_site_surface_dict(surface)
	return {
		"material": str(clean.get("material", "asphalt")),
		"name": str(clean.get("name", "")).strip_edges(),
		"y_m": float(clean.get("y_m", 0.01)),
		"vertices": clean.get("vertices", []),
	}

func _site_anchor_type_label(anchor_type: String) -> String:
	match anchor_type:
		"entrance":
			return "Entrance"
		"driveway":
			return "Driveway"
		"parking":
			return "Parking"
		"loading_bay":
			return "Loading Bay"
		_:
			return anchor_type.capitalize()

func _make_main_entrance_anchor(position: Vector3, forward: Vector3) -> Dictionary:
	var resolved := forward
	if resolved.length_squared() < 0.001:
		resolved = _frontage_fwd
	if resolved.length_squared() < 0.001:
		resolved = Vector3.FORWARD
	return {
		"anchor_type": "entrance",
		"name": "main",
		"position": _vector3_to_array(_clamp_anchor_position_to_lot(position), 0.01),
		"forward": _vector3_to_array(resolved.normalized(), 0.001),
	}

func _is_main_entrance_anchor(anchor: Dictionary) -> bool:
	return (
		str(anchor.get("anchor_type", "")).strip_edges() == "entrance"
		and str(anchor.get("name", "")).strip_edges() == "main"
	)

func _main_entrance_index() -> int:
	for i in _site_anchors_data.size():
		if _is_main_entrance_anchor(_site_anchors_data[i]):
			return i
	return -1

func _ensure_main_entrance_anchor() -> int:
	var index := _main_entrance_index()
	if index >= 0:
		return index
	_site_anchors_data.insert(
		0,
		_make_main_entrance_anchor(_default_main_entrance_position(), _frontage_fwd)
	)
	return 0

func _add_site_anchor(anchor_type: String) -> void:
	if anchor_type == "entrance":
		_select_site_anchor(_ensure_main_entrance_anchor())
		return
	var anchor := {
		"anchor_type": anchor_type,
		"name": "",
		"position": _vector3_to_array(_default_site_anchor_position(anchor_type), 0.01),
		"forward": _vector3_to_array(_frontage_fwd, 0.001),
		"width_m": float(SITE_ANCHOR_DEFAULT_WIDTH_M.get(anchor_type, 2.0)),
		"vehicle_class": _default_site_anchor_vehicle_class(anchor_type),
	}
	if SITE_ANCHOR_DEFAULT_LENGTH_M.has(anchor_type):
		anchor["length_m"] = float(SITE_ANCHOR_DEFAULT_LENGTH_M[anchor_type])
	anchor["position"] = _vector3_to_array(
		_clamp_site_anchor_position_to_lot(anchor, _anchor_position(anchor)),
		0.01
	)
	_site_anchors_data.append(anchor)
	_select_site_anchor(_site_anchors_data.size() - 1)
	_log("Added %s anchor." % _site_anchor_type_label(anchor_type))

func _default_site_anchor_position(anchor_type: String) -> Vector3:
	var lot_half_w := _width_spin.value * 10.0 * 0.5
	var lot_half_d := _depth_spin.value * 10.0 * 0.5
	var count := _site_anchor_count(anchor_type)
	var side_offset := float((count % 5) - 2) * 2.5
	var depth_offset := float(count / 5) * 2.0
	var fwd := _frontage_fwd.normalized()
	if fwd.length_squared() < 0.001:
		fwd = Vector3.FORWARD
	var side := Vector3(-fwd.z, 0.0, fwd.x)
	var edge_distance := lot_half_d if absf(fwd.z) >= absf(fwd.x) else lot_half_w
	var inward := -fwd
	var base := fwd * maxf(0.0, edge_distance - 2.0 - depth_offset) + side * side_offset
	if anchor_type == "parking":
		base += inward * 2.5
	elif anchor_type == "loading_bay":
		base += inward * 4.0
	return _clamp_anchor_position_to_lot(Vector3(base.x, 0.0, base.z))

func _site_anchor_count(anchor_type: String) -> int:
	var count := 0
	for anchor in _site_anchors_data:
		if str(anchor.get("anchor_type", "")).strip_edges() == anchor_type:
			count += 1
	return count

func _default_site_anchor_vehicle_class(anchor_type: String) -> String:
	return "freight" if anchor_type == "loading_bay" else "car"

func _refresh_site_anchor_list() -> void:
	if not _site_anchor_list:
		return
	_ensure_main_entrance_anchor()
	_updating_site_anchor_list = true
	_site_anchor_list.clear()
	var type_counts := {}
	for i in _site_anchors_data.size():
		var anchor := _site_anchors_data[i]
		var anchor_type := str(anchor.get("anchor_type", "")).strip_edges()
		type_counts[anchor_type] = int(type_counts.get(anchor_type, 0)) + 1
		_site_anchor_list.add_item(_site_anchor_display_label(i, type_counts[anchor_type]))
	for index in _selected_site_anchor_indices:
		if index >= 0 and index < _site_anchor_list.item_count:
			_site_anchor_list.select(index, false)
	_updating_site_anchor_list = false
	_update_site_anchor_controls()
	_update_site_anchor_preview()

func _site_anchor_display_label(index: int, type_index: int = -1) -> String:
	if index < 0 or index >= _site_anchors_data.size():
		return "Anchor"
	var anchor := _site_anchors_data[index]
	var anchor_type := str(anchor.get("anchor_type", "")).strip_edges()
	var name := str(anchor.get("name", "")).strip_edges()
	if _is_main_entrance_anchor(anchor):
		return "Entrance - main"
	if not name.is_empty():
		return "%s - %s" % [_site_anchor_type_label(anchor_type), name]
	if type_index < 0:
		type_index = _site_anchor_index_among_type(index)
	return "%s %d" % [_site_anchor_type_label(anchor_type), type_index]

func _site_anchor_index_among_type(index: int) -> int:
	if index < 0 or index >= _site_anchors_data.size():
		return 0
	var anchor_type := str(_site_anchors_data[index].get("anchor_type", "")).strip_edges()
	var count := 0
	for i in index + 1:
		if str(_site_anchors_data[i].get("anchor_type", "")).strip_edges() == anchor_type:
			count += 1
	return count

func _select_site_anchor(index: int, clear_mesh_parts: bool = true) -> void:
	if index < 0 or index >= _site_anchors_data.size():
		_set_selected_site_anchors([], -1)
	else:
		_set_selected_site_anchors([index], index)
		if clear_mesh_parts:
			_set_selected_mesh_parts([], -1)
		_set_selected_site_surface(-1)

func _set_selected_site_anchors(indices: Array, primary_index: int = -1) -> void:
	var seen := {}
	var resolved: Array[int] = []
	for raw_index in indices:
		var index := int(raw_index)
		if index < 0 or index >= _site_anchors_data.size() or seen.has(index):
			continue
		seen[index] = true
		resolved.append(index)
	resolved.sort()
	if resolved.is_empty():
		_selected_site_anchor_index = -1
	else:
		_selected_site_anchor_index = primary_index if resolved.has(primary_index) else int(resolved[0])
	_selected_site_anchor_indices = resolved
	_refresh_site_anchor_list()

func _toggle_site_anchor_selection(index: int) -> void:
	if index < 0 or index >= _site_anchors_data.size():
		return
	var selected := _selected_site_anchor_indices.duplicate()
	if selected.has(index):
		selected.erase(index)
		_set_selected_site_anchors(selected, -1 if selected.is_empty() else int(selected[0]))
	else:
		selected.append(index)
		_set_selected_site_anchors(selected, index)

func _on_site_anchor_selected(index: int) -> void:
	if _updating_site_anchor_list:
		return
	var selected := _site_anchor_list.get_selected_items()
	_set_selected_site_anchors(selected, index)
	if not Input.is_key_pressed(KEY_CTRL):
		_set_selected_mesh_parts([], -1)

func _on_site_anchor_multi_selected(index: int, _selected: bool) -> void:
	if _updating_site_anchor_list:
		return
	var selected := _site_anchor_list.get_selected_items()
	_set_selected_site_anchors(selected, index)

func _remove_selected_site_anchor() -> bool:
	var remove_indices := _selected_site_anchor_indices.duplicate()
	if remove_indices.is_empty() and _selected_site_anchor_index >= 0:
		remove_indices.append(_selected_site_anchor_index)
	if remove_indices.is_empty():
		_log("[color=yellow]No site anchor selected to remove.[/color]")
		return false
	remove_indices.sort()
	var first_removed := int(remove_indices[0])
	var removed_labels: Array[String] = []
	var skipped_required := false
	for i in range(remove_indices.size() - 1, -1, -1):
		var index := int(remove_indices[i])
		if index < 0 or index >= _site_anchors_data.size():
			continue
		if _is_main_entrance_anchor(_site_anchors_data[index]):
			skipped_required = true
			continue
		var removed_type := str(_site_anchors_data[index].get("anchor_type", ""))
		removed_labels.append(_site_anchor_type_label(removed_type))
		_site_anchors_data.remove_at(index)
	if removed_labels.is_empty():
		if skipped_required:
			_log("[color=yellow]Entrance is required and cannot be removed.[/color]")
		return false
	var next_index := -1
	if not _site_anchors_data.is_empty():
		next_index = mini(first_removed, _site_anchors_data.size() - 1)
	_set_selected_site_anchors([next_index] if next_index >= 0 else [], next_index)
	if removed_labels.size() == 1:
		_log("Removed %s anchor." % removed_labels[0])
	else:
		_log("Removed %d site anchors." % removed_labels.size())
	return true

func _update_site_anchor_controls() -> void:
	_updating_site_anchor_controls = true
	var has_anchor := _selected_site_anchor_index >= 0 and _selected_site_anchor_index < _site_anchors_data.size()
	var anchor := _site_anchors_data[_selected_site_anchor_index] if has_anchor else {}
	var anchor_type := str(anchor.get("anchor_type", "")).strip_edges()
	var is_entrance := has_anchor and _is_main_entrance_anchor(anchor)
	var has_size := has_anchor and anchor_type != "entrance"
	var has_length := has_anchor and (anchor_type == "parking" or anchor_type == "loading_bay")
	var pos := _anchor_position(anchor)
	var yaw := _yaw_from_forward(_anchor_forward(anchor))
	_site_anchor_name_edit.text = str(anchor.get("name", "")) if has_anchor else ""
	_set_option_by_metadata(_site_anchor_vehicle_class_btn, _anchor_text(anchor, "vehicle_class", "car"))
	_site_anchor_x_spin.value = pos.x
	_site_anchor_y_spin.value = pos.y
	_site_anchor_z_spin.value = pos.z
	_site_anchor_yaw_spin.value = yaw
	_site_anchor_width_spin.value = _anchor_number(anchor, "width_m", 3.0) if has_size else 0.0
	_site_anchor_length_spin.value = _anchor_number(anchor, "length_m", 0.0) if has_length else 0.0
	_site_anchor_name_edit.editable = has_anchor and not is_entrance
	_site_anchor_vehicle_class_btn.disabled = not has_size
	for control in [_site_anchor_x_spin, _site_anchor_y_spin, _site_anchor_z_spin, _site_anchor_yaw_spin]:
		(control as SpinBox).editable = has_anchor
	_site_anchor_width_spin.editable = has_size
	_site_anchor_length_spin.editable = has_length
	_updating_site_anchor_controls = false

func _on_site_anchor_text_changed(_value: String) -> void:
	if _updating_site_anchor_controls:
		return
	_apply_site_anchor_controls()

func _on_site_anchor_vehicle_class_selected(_index: int) -> void:
	if _updating_site_anchor_controls:
		return
	_apply_site_anchor_controls()

func _on_site_anchor_spin_changed(_value: float) -> void:
	if _updating_site_anchor_controls:
		return
	_apply_site_anchor_controls()

func _apply_site_anchor_controls() -> void:
	if _selected_site_anchor_index < 0 or _selected_site_anchor_index >= _site_anchors_data.size():
		return
	var anchor := _site_anchors_data[_selected_site_anchor_index]
	var anchor_type := str(anchor.get("anchor_type", "")).strip_edges()
	var is_entrance := _is_main_entrance_anchor(anchor)
	if is_entrance:
		_main_entrance_auto = false
	if is_entrance:
		anchor["name"] = "main"
		anchor.erase("vehicle_class")
		anchor.erase("width_m")
		anchor.erase("length_m")
	else:
		anchor["name"] = _site_anchor_name_edit.text.strip_edges()
		anchor["vehicle_class"] = _selected_option_metadata(_site_anchor_vehicle_class_btn, "car")
		anchor["width_m"] = snappedf(maxf(0.1, float(_site_anchor_width_spin.value)), 0.01)
	anchor["forward"] = _vector3_to_array(
		_forward_from_yaw(_snap_rotation_y_to_cardinal_if_close(_site_anchor_yaw_spin.value)),
		0.001
	)
	if not is_entrance and (anchor_type == "parking" or anchor_type == "loading_bay"):
		anchor["length_m"] = snappedf(maxf(0.1, float(_site_anchor_length_spin.value)), 0.01)
	elif anchor.has("length_m"):
		anchor.erase("length_m")
	anchor["position"] = _vector3_to_array(
		_clamp_site_anchor_position_to_lot(anchor, Vector3(
			_site_anchor_x_spin.value,
			_site_anchor_y_spin.value,
			_site_anchor_z_spin.value
		)),
		0.01
	)
	_refresh_site_anchor_list()

func _update_site_anchor_preview() -> void:
	if _preview and _preview.has_method("set_site_anchors"):
		_preview.set_site_anchors(
			_site_anchors_data,
			_selected_site_anchor_indices,
			_selected_site_anchor_index
		)

func _anchor_position(anchor: Dictionary) -> Vector3:
	var pos = anchor.get("position", [])
	if pos is Array and pos.size() == 3:
		return Vector3(float(pos[0]), float(pos[1]), float(pos[2]))
	return Vector3.ZERO

func _anchor_forward(anchor: Dictionary) -> Vector3:
	var fwd = anchor.get("forward", [])
	if fwd is Array and fwd.size() == 3:
		var resolved := Vector3(float(fwd[0]), float(fwd[1]), float(fwd[2]))
		if resolved.length_squared() > 0.001:
			return resolved.normalized()
	return Vector3.FORWARD

func _sanitize_site_anchor_dict(anchor: Dictionary) -> Dictionary:
	var clean := anchor.duplicate(true)
	var anchor_type := str(clean.get("anchor_type", "")).strip_edges()
	if anchor_type == "entrance":
		clean["name"] = "main"
		clean.erase("vehicle_class")
		clean.erase("width_m")
		clean.erase("length_m")
		return clean
	for key in ["name", "vehicle_class"]:
		if clean.get(key, null) == null:
			clean.erase(key)
	for key in ["width_m", "length_m"]:
		var value = clean.get(key, null)
		if _anchor_value_is_number(value):
			clean[key] = float(value)
		elif value is String and (value as String).strip_edges().is_valid_float():
			clean[key] = (value as String).strip_edges().to_float()
		else:
			clean.erase(key)
	return clean

func _anchor_number(anchor: Dictionary, key: String, fallback: float) -> float:
	var value = anchor.get(key, null)
	if value == null:
		return fallback
	if _anchor_value_is_number(value):
		return float(value)
	if value is String:
		var text := (value as String).strip_edges()
		if text.is_valid_float():
			return text.to_float()
	return fallback

func _anchor_value_is_number(value) -> bool:
	var value_type := typeof(value)
	return value_type == TYPE_FLOAT or value_type == TYPE_INT

func _anchor_text(anchor: Dictionary, key: String, fallback: String) -> String:
	var value = anchor.get(key, null)
	if value == null:
		return fallback
	return str(value).strip_edges()

func _set_site_anchor_position(index: int, pos: Vector3) -> void:
	if index < 0 or index >= _site_anchors_data.size():
		return
	_site_anchors_data[index]["position"] = _vector3_to_array(
		_clamp_site_anchor_position_to_lot(_site_anchors_data[index], pos),
		0.01
	)
	_update_site_anchor_controls()
	_update_site_anchor_preview()

func _set_site_anchor_yaw(index: int, yaw_degrees: float) -> void:
	if index < 0 or index >= _site_anchors_data.size():
		return
	_site_anchors_data[index]["forward"] = _vector3_to_array(
		_forward_from_yaw(_snap_rotation_y_to_cardinal_if_close(yaw_degrees)),
		0.001
	)
	_site_anchors_data[index]["position"] = _vector3_to_array(
		_clamp_site_anchor_position_to_lot(_site_anchors_data[index], _anchor_position(_site_anchors_data[index])),
		0.01
	)
	_update_site_anchor_controls()
	_update_site_anchor_preview()

func _vector3_to_array(value: Vector3, snap: float) -> Array:
	return [snappedf(value.x, snap), snappedf(value.y, snap), snappedf(value.z, snap)]

func _forward_from_yaw(yaw_degrees: float) -> Vector3:
	var yaw := deg_to_rad(yaw_degrees)
	return Vector3(sin(yaw), 0.0, cos(yaw)).normalized()

func _yaw_from_forward(forward: Vector3) -> float:
	var flat := Vector3(forward.x, 0.0, forward.z)
	if flat.length_squared() < 0.001:
		return 0.0
	flat = flat.normalized()
	return _normalize_degrees(rad_to_deg(atan2(flat.x, flat.z)))

func _set_option_by_metadata(button: OptionButton, metadata_value: String) -> void:
	if not button:
		return
	for i in button.item_count:
		if str(button.get_item_metadata(i)) == metadata_value:
			button.select(i)
			return
	if button.item_count > 0:
		button.select(0)

func _selected_option_metadata(button: OptionButton, fallback: String) -> String:
	if not button or button.selected < 0:
		return fallback
	var value = button.get_item_metadata(button.selected)
	if value == null:
		return fallback
	return str(value)

func _export_site_anchor(anchor: Dictionary) -> Dictionary:
	var anchor_type := str(anchor.get("anchor_type", "")).strip_edges()
	var pos := _anchor_position(anchor)
	var fwd := _anchor_forward(anchor)
	var exported := {
		"anchor_type": anchor_type,
		"name": str(anchor.get("name", "")).strip_edges(),
		"position": _vector3_to_array(pos, 0.01),
		"forward": _vector3_to_array(fwd, 0.001),
	}
	if anchor_type == "entrance":
		exported["name"] = "main"
		return exported
	exported["width_m"] = snappedf(maxf(0.1, _anchor_number(anchor, "width_m", 2.0)), 0.01)
	var vehicle_class := _anchor_text(anchor, "vehicle_class", "")
	if not vehicle_class.is_empty():
		exported["vehicle_class"] = vehicle_class
	if anchor_type == "parking" or anchor_type == "loading_bay":
		exported["length_m"] = snappedf(maxf(0.1, _anchor_number(anchor, "length_m", 5.0)), 0.01)
	return exported

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
	_part_positions[_selected_part_index] = _clamp_mesh_part_position_to_lot(
		_selected_part_index,
		_part_positions[_selected_part_index]
	)
	_sync_selected_mesh_part_controls()
	_preview.set_mesh_part_transform(
		_selected_part_index,
		_part_positions[_selected_part_index],
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
	_selected_part_indices.clear()
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
		_set_main_entrance_forward(_frontage_fwd)
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
	_clamp_mesh_parts_to_lot()
	var mesh_fit_error := _mesh_parts_lot_fit_error()
	if not mesh_fit_error.is_empty():
		_log("[color=red]%s[/color]" % mesh_fit_error)
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

	_ensure_main_entrance_anchor()
	var anchors := []
	for anchor in _site_anchors_data:
		anchors.append(_export_site_anchor(anchor))
	_clamp_site_surfaces_to_lot()
	var site_surfaces := []
	for surface in _site_surfaces_data:
		site_surfaces.append(_export_site_surface(surface))

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
		"site_surfaces": site_surfaces,
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
	if _site_surface_context_menu:
		EditorTheme.style_popup_menu(_site_surface_context_menu, _theme_mode)
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

func _on_reset_main_entrance_pressed() -> void:
	_set_main_entrance_forward(_frontage_fwd)
	_set_main_entrance_position(_default_main_entrance_position(), true)
	_log("Main entrance reset to the current frontage edge.")

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
	_set_site_anchor_position(_ensure_main_entrance_anchor(), pos)

func _set_main_entrance_forward(fwd: Vector3) -> void:
	var resolved := fwd
	if resolved.length_squared() < 0.001:
		resolved = _frontage_fwd
	if resolved.length_squared() < 0.001:
		resolved = Vector3.FORWARD
	var index := _ensure_main_entrance_anchor()
	_site_anchors_data[index]["forward"] = _vector3_to_array(resolved.normalized(), 0.001)
	_update_site_anchor_controls()
	_update_site_anchor_preview()

func _update_main_entrance_preview() -> void:
	_ensure_main_entrance_anchor()
	_update_site_anchor_preview()

func _clamp_anchor_position_to_lot(pos: Vector3) -> Vector3:
	if not _width_spin or not _depth_spin:
		return pos
	var lot_half_w := float(_width_spin.value) * 10.0 * 0.5
	var lot_half_d := float(_depth_spin.value) * 10.0 * 0.5
	return Vector3(
		clampf(pos.x, -lot_half_w, lot_half_w),
		pos.y,
		clampf(pos.z, -lot_half_d, lot_half_d)
	)

func _clamp_site_anchor_position_to_lot(anchor: Dictionary, pos: Vector3) -> Vector3:
	if not _width_spin or not _depth_spin:
		return pos
	var offsets := _site_anchor_footprint_offsets(anchor)
	if offsets.is_empty():
		return _clamp_anchor_position_to_lot(pos)
	var lot_half_w := float(_width_spin.value) * 10.0 * 0.5
	var lot_half_d := float(_depth_spin.value) * 10.0 * 0.5
	var min_offset_x := 0.0
	var max_offset_x := 0.0
	var min_offset_z := 0.0
	var max_offset_z := 0.0
	var first := true
	for raw_offset in offsets:
		if not (raw_offset is Vector3):
			continue
		var offset := raw_offset as Vector3
		if first:
			min_offset_x = offset.x
			max_offset_x = offset.x
			min_offset_z = offset.z
			max_offset_z = offset.z
			first = false
		else:
			min_offset_x = minf(min_offset_x, offset.x)
			max_offset_x = maxf(max_offset_x, offset.x)
			min_offset_z = minf(min_offset_z, offset.z)
			max_offset_z = maxf(max_offset_z, offset.z)
	if first:
		return _clamp_anchor_position_to_lot(pos)
	return Vector3(
		_clamp_to_possible_interval(pos.x, -lot_half_w - min_offset_x, lot_half_w - max_offset_x),
		pos.y,
		_clamp_to_possible_interval(pos.z, -lot_half_d - min_offset_z, lot_half_d - max_offset_z)
	)

func _site_anchor_footprint_offsets(anchor: Dictionary) -> Array:
	var anchor_type := str(anchor.get("anchor_type", "")).strip_edges()
	var forward := _anchor_forward(anchor)
	var side := Vector3(-forward.z, 0.0, forward.x)
	if side.length_squared() < 0.001:
		return []
	side = side.normalized()
	forward = Vector3(forward.x, 0.0, forward.z).normalized()
	var width := maxf(0.1, _anchor_number(anchor, "width_m", SITE_ANCHOR_DRAG_RADIUS_M))
	var half_w := width * 0.5
	var length := 0.0
	if anchor_type == "parking" or anchor_type == "loading_bay":
		length = maxf(0.1, _anchor_number(anchor, "length_m", width))
	elif anchor_type == "driveway":
		length = maxf(1.5, width * 1.4)
	else:
		return []
	return [
		-side * half_w,
		side * half_w,
		side * half_w + forward * length,
		-side * half_w + forward * length,
	]

func _clamp_site_surface_vertices_to_lot(surface: Dictionary) -> void:
	var vertices := _site_surface_vertices(surface)
	if vertices.is_empty():
		return
	for i in vertices.size():
		vertices[i] = _clamp_site_surface_vertex_to_lot(vertices[i])
	surface["vertices"] = _site_surface_vertices_to_arrays(vertices)

func _clamp_site_surface_vertex_to_lot(vertex: Vector2) -> Vector2:
	if not _width_spin or not _depth_spin:
		return vertex
	var lot_half_w := float(_width_spin.value) * 10.0 * 0.5
	var lot_half_d := float(_depth_spin.value) * 10.0 * 0.5
	return Vector2(
		clampf(vertex.x, -lot_half_w, lot_half_w),
		clampf(vertex.y, -lot_half_d, lot_half_d)
	)

func _clamp_site_surface_delta_to_lot(vertices: Array, delta: Vector2) -> Vector2:
	if not _width_spin or not _depth_spin or vertices.is_empty():
		return delta
	var lot_half_w := float(_width_spin.value) * 10.0 * 0.5
	var lot_half_d := float(_depth_spin.value) * 10.0 * 0.5
	var min_x := 0.0
	var max_x := 0.0
	var min_z := 0.0
	var max_z := 0.0
	var first := true
	for raw_vertex in vertices:
		if not (raw_vertex is Vector2):
			continue
		var vertex := raw_vertex as Vector2
		if first:
			min_x = vertex.x
			max_x = vertex.x
			min_z = vertex.y
			max_z = vertex.y
			first = false
		else:
			min_x = minf(min_x, vertex.x)
			max_x = maxf(max_x, vertex.x)
			min_z = minf(min_z, vertex.y)
			max_z = maxf(max_z, vertex.y)
	if first:
		return delta
	return Vector2(
		_clamp_to_possible_interval(delta.x, -lot_half_w - min_x, lot_half_w - max_x),
		_clamp_to_possible_interval(delta.y, -lot_half_d - min_z, lot_half_d - max_z)
	)

func _clamp_to_possible_interval(value: float, min_value: float, max_value: float) -> float:
	if min_value <= max_value:
		return clampf(value, min_value, max_value)
	return (min_value + max_value) * 0.5

func _clamp_site_anchors_to_lot() -> void:
	var changed := false
	for i in _site_anchors_data.size():
		var pos := _anchor_position(_site_anchors_data[i])
		var clamped := _clamp_site_anchor_position_to_lot(_site_anchors_data[i], pos)
		if clamped.distance_squared_to(pos) > 0.0001:
			_site_anchors_data[i]["position"] = _vector3_to_array(clamped, 0.01)
			changed = true
	if changed:
		_refresh_site_anchor_list()
	else:
		_update_site_anchor_preview()

func _clamp_site_surfaces_to_lot() -> void:
	var changed := false
	for i in _site_surfaces_data.size():
		var before: Array = _site_surfaces_data[i].get("vertices", []).duplicate(true)
		_clamp_site_surface_vertices_to_lot(_site_surfaces_data[i])
		if before != _site_surfaces_data[i].get("vertices", []):
			changed = true
	if changed:
		_refresh_site_surface_list()
	else:
		_update_site_surface_preview()

func _clamp_mesh_part_position_to_lot(part_index: int, pos: Vector3) -> Vector3:
	if not _width_spin or not _depth_spin:
		return pos
	var bounds := _mesh_part_footprint_bounds(part_index, pos)
	if bounds.is_empty():
		return _clamp_anchor_position_to_lot(pos)
	var lot_half_w := float(_width_spin.value) * 10.0 * 0.5
	var lot_half_d := float(_depth_spin.value) * 10.0 * 0.5
	return Vector3(
		_clamp_to_possible_interval(
			pos.x,
			pos.x - lot_half_w - float(bounds["min_x"]),
			pos.x + lot_half_w - float(bounds["max_x"])
		),
		pos.y,
		_clamp_to_possible_interval(
			pos.z,
			pos.z - lot_half_d - float(bounds["min_z"]),
			pos.z + lot_half_d - float(bounds["max_z"])
		)
	)

func _clamp_mesh_parts_to_lot() -> void:
	var changed := false
	for i in _part_positions.size():
		var pos := _part_positions[i]
		var clamped := _clamp_mesh_part_position_to_lot(i, pos)
		if clamped.distance_squared_to(pos) > 0.0001:
			_part_positions[i] = clamped
			_apply_mesh_part_transform_from_state(i)
			changed = true
	if changed:
		_sync_selected_mesh_part_controls()
		_update_dim_label()

func _mesh_parts_lot_fit_error() -> String:
	for i in _part_positions.size():
		if not _mesh_part_fits_in_lot(i):
			return (
				"Mesh part '%s' footprint crosses the lot/plot bounds. "
				+ "Move, rotate, scale it down, or enlarge the lot before exporting."
			) % _part_name_for_index(i)
	return ""

func _mesh_part_fits_in_lot(part_index: int) -> bool:
	var bounds := _mesh_part_footprint_bounds(part_index, _part_positions[part_index])
	if bounds.is_empty() or not _width_spin or not _depth_spin:
		return true
	var lot_half_w := float(_width_spin.value) * 10.0 * 0.5
	var lot_half_d := float(_depth_spin.value) * 10.0 * 0.5
	var eps := 0.01
	return (
		float(bounds["min_x"]) >= -lot_half_w - eps
		and float(bounds["max_x"]) <= lot_half_w + eps
		and float(bounds["min_z"]) >= -lot_half_d - eps
		and float(bounds["max_z"]) <= lot_half_d + eps
	)

func _mesh_part_footprint_bounds(part_index: int, pos: Vector3) -> Dictionary:
	var result := {}
	if (
		part_index < 0
		or part_index >= _part_aabbs.size()
		or part_index >= _part_scales.size()
		or part_index >= _part_rotation_y.size()
		or part_index >= _part_pivot_offsets.size()
	):
		return result
	var aabb := _part_aabbs[part_index]
	if aabb.size.x < 0.001 or aabb.size.z < 0.001:
		return result
	var yaw_basis := Basis(Vector3.UP, deg_to_rad(_part_rotation_y[part_index]))
	var scale := maxf(0.001, _part_scales[part_index])
	var pivot := _part_pivot_offsets[part_index]
	var first := true
	var min_x := 0.0
	var max_x := 0.0
	var min_z := 0.0
	var max_z := 0.0
	for local_corner in _aabb_corners(aabb):
		var corner := local_corner as Vector3
		var world_offset: Vector3 = yaw_basis * ((pivot + corner) * scale)
		var x := pos.x + world_offset.x
		var z := pos.z + world_offset.z
		if first:
			min_x = x
			max_x = x
			min_z = z
			max_z = z
			first = false
		else:
			min_x = minf(min_x, x)
			max_x = maxf(max_x, x)
			min_z = minf(min_z, z)
			max_z = maxf(max_z, z)
	if first:
		return {}
	return {
		"min_x": min_x,
		"max_x": max_x,
		"min_z": min_z,
		"max_z": max_z,
	}

func _aabb_corners(aabb: AABB) -> Array:
	var p := aabb.position
	var s := aabb.size
	return [
		Vector3(p.x, p.y, p.z),
		Vector3(p.x + s.x, p.y, p.z),
		Vector3(p.x, p.y + s.y, p.z),
		Vector3(p.x + s.x, p.y + s.y, p.z),
		Vector3(p.x, p.y, p.z + s.z),
		Vector3(p.x + s.x, p.y, p.z + s.z),
		Vector3(p.x, p.y + s.y, p.z + s.z),
		Vector3(p.x + s.x, p.y + s.y, p.z + s.z),
	]

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
		_part_positions[_selected_part_index] = _clamp_mesh_part_position_to_lot(
			_selected_part_index,
			_part_positions[_selected_part_index]
		)
		_sync_selected_mesh_part_controls()
		_apply_selected_part_transform_from_state()
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
	if event is InputEventKey:
		var key := event as InputEventKey
		if (
			key.pressed
			and not key.echo
			and key.keycode == KEY_DELETE
			and not _ui_captures_editor_text_input()
		):
			var removed := false
			if _selected_site_surface_index >= 0:
				removed = _remove_selected_site_surface()
			if not _selected_site_anchor_indices.is_empty():
				removed = _remove_selected_site_anchor()
			if not _selected_part_indices.is_empty():
				removed = _remove_selected_mesh_parts() or removed
			if removed:
				get_viewport().set_input_as_handled()
		return

	if event is InputEventMouseButton:
		var mb := event as InputEventMouseButton
		if mb.button_index != MOUSE_BUTTON_LEFT and mb.button_index != MOUSE_BUTTON_RIGHT:
			return
		if mb.pressed:
			if not _is_mouse_in_3d_area():
				return
			var mouse_pos := get_viewport().get_mouse_position()
			if mb.button_index == MOUSE_BUTTON_LEFT:
				if Input.is_key_pressed(KEY_SHIFT):
					_begin_box_selection(mouse_pos, Input.is_key_pressed(KEY_CTRL))
					get_viewport().set_input_as_handled()
					return
				if Input.is_key_pressed(KEY_CTRL):
					if _toggle_selection_at_mouse(mouse_pos):
						get_viewport().set_input_as_handled()
						return
					_begin_box_selection(mouse_pos, true)
					get_viewport().set_input_as_handled()
					return
				if _try_begin_site_surface_vertex_drag(mouse_pos):
					get_viewport().set_input_as_handled()
					return
				if _try_begin_site_surface_drag(mouse_pos):
					get_viewport().set_input_as_handled()
					return
				if _try_begin_site_anchor_drag(mouse_pos):
					get_viewport().set_input_as_handled()
					return
				if _try_begin_mesh_part_drag(mouse_pos):
					get_viewport().set_input_as_handled()
					return
				if _try_begin_ghost_drag(mouse_pos):
					get_viewport().set_input_as_handled()
					return
				_begin_box_selection(mouse_pos, false)
				get_viewport().set_input_as_handled()
				return
			if mb.button_index == MOUSE_BUTTON_RIGHT:
				if _try_open_site_surface_context_menu(mouse_pos):
					get_viewport().set_input_as_handled()
					return
				if _try_begin_site_anchor_rotation(mouse_pos):
					get_viewport().set_input_as_handled()
					return
				if _try_begin_mesh_part_rotation(mouse_pos):
					get_viewport().set_input_as_handled()
			return
		if _dragging_site_anchor:
			_dragging_site_anchor = false
			_site_anchor_drag_start_positions.clear()
			_mesh_part_drag_start_positions.clear()
			get_viewport().set_input_as_handled()
		if _dragging_site_surface:
			_dragging_site_surface = false
			_site_surface_drag_start_vertices.clear()
			get_viewport().set_input_as_handled()
		if _dragging_site_surface_vertex:
			_dragging_site_surface_vertex = false
			_site_surface_drag_start_vertices.clear()
			get_viewport().set_input_as_handled()
		if _dragging_mesh_part:
			_dragging_mesh_part = false
			_mesh_part_drag_start_positions.clear()
			_site_anchor_drag_start_positions.clear()
			get_viewport().set_input_as_handled()
		if _selecting_mesh_parts:
			_finish_mesh_part_box_selection(get_viewport().get_mouse_position())
			get_viewport().set_input_as_handled()
		if _rotating_mesh_part:
			_rotating_mesh_part = false
			get_viewport().set_input_as_handled()
		if _rotating_site_anchor:
			_rotating_site_anchor = false
			get_viewport().set_input_as_handled()
		if _dragging_ghost:
			_dragging_ghost = false
			get_viewport().set_input_as_handled()
		return

	if event is InputEventMouseMotion and _dragging_site_anchor:
		if _drag_site_anchor_from_mouse(get_viewport().get_mouse_position()):
			get_viewport().set_input_as_handled()
	elif event is InputEventMouseMotion and _dragging_site_surface:
		if _drag_site_surface_from_mouse(get_viewport().get_mouse_position()):
			get_viewport().set_input_as_handled()
	elif event is InputEventMouseMotion and _dragging_site_surface_vertex:
		if _drag_site_surface_vertex_from_mouse(get_viewport().get_mouse_position()):
			get_viewport().set_input_as_handled()
	elif event is InputEventMouseMotion and _dragging_mesh_part:
		if _drag_mesh_part_from_mouse(get_viewport().get_mouse_position()):
			get_viewport().set_input_as_handled()
	elif event is InputEventMouseMotion and _selecting_mesh_parts:
		_update_mesh_part_box_selection(get_viewport().get_mouse_position())
		get_viewport().set_input_as_handled()
	elif event is InputEventMouseMotion and _rotating_mesh_part:
		_rotate_mesh_part_from_mouse(get_viewport().get_mouse_position())
		get_viewport().set_input_as_handled()
	elif event is InputEventMouseMotion and _rotating_site_anchor:
		_rotate_site_anchor_from_mouse(get_viewport().get_mouse_position())
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

func _ui_captures_editor_text_input() -> bool:
	var focus_owner := get_viewport().gui_get_focus_owner()
	return (
		focus_owner is LineEdit
		or focus_owner is TextEdit
		or focus_owner is CodeEdit
		or focus_owner is SpinBox
	)

func _try_begin_site_surface_vertex_drag(mouse_pos: Vector2) -> bool:
	var hit = _project_mouse_to_horizontal_plane(mouse_pos, 0.0)
	if hit == null:
		return false
	var vertex_hit := _site_surface_vertex_hit_at_world_xz(hit)
	if vertex_hit.is_empty():
		return false
	_site_surface_drag_index = int(vertex_hit["surface"])
	_site_surface_vertex_drag_index = int(vertex_hit["vertex"])
	_set_selected_site_surface(_site_surface_drag_index)
	_site_surface_drag_start_hit = Vector3(hit.x, 0.0, hit.z)
	_site_surface_drag_start_vertices = _site_surface_vertices(_site_surfaces_data[_site_surface_drag_index])
	_dragging_site_surface_vertex = true
	return true

func _try_begin_site_surface_drag(mouse_pos: Vector2) -> bool:
	var hit = _project_mouse_to_horizontal_plane(mouse_pos, 0.0)
	if hit == null:
		return false
	var surface_index := _site_surface_index_at_world_xz(hit)
	if surface_index < 0:
		return false
	_site_surface_drag_index = surface_index
	_set_selected_site_surface(surface_index)
	_site_surface_drag_start_hit = Vector3(hit.x, 0.0, hit.z)
	_site_surface_drag_start_vertices = _site_surface_vertices(_site_surfaces_data[surface_index])
	_dragging_site_surface = true
	return true

func _drag_site_surface_from_mouse(mouse_pos: Vector2) -> bool:
	if _site_surface_drag_index < 0 or _site_surface_drag_index >= _site_surfaces_data.size():
		return false
	if _site_surface_drag_start_vertices.is_empty():
		return false
	var hit = _project_mouse_to_horizontal_plane(mouse_pos, 0.0)
	if hit == null:
		return false
	var delta := Vector2(hit.x - _site_surface_drag_start_hit.x, hit.z - _site_surface_drag_start_hit.z)
	delta = _clamp_site_surface_delta_to_lot(_site_surface_drag_start_vertices, delta)
	var vertices: Array[Vector2] = []
	for raw_vertex in _site_surface_drag_start_vertices:
		if raw_vertex is Vector2:
			vertices.append((raw_vertex as Vector2) + delta)
	if not _site_surface_polygon_is_valid(vertices):
		return false
	_site_surfaces_data[_site_surface_drag_index]["vertices"] = _site_surface_vertices_to_arrays(vertices)
	_update_site_surface_controls()
	_update_site_surface_preview()
	return true

func _drag_site_surface_vertex_from_mouse(mouse_pos: Vector2) -> bool:
	if _site_surface_drag_index < 0 or _site_surface_drag_index >= _site_surfaces_data.size():
		return false
	if _site_surface_vertex_drag_index < 0 or _site_surface_vertex_drag_index >= _site_surface_drag_start_vertices.size():
		return false
	var hit = _project_mouse_to_horizontal_plane(mouse_pos, 0.0)
	if hit == null:
		return false
	var vertices: Array[Vector2] = []
	for raw_vertex in _site_surface_drag_start_vertices:
		if raw_vertex is Vector2:
			vertices.append(raw_vertex as Vector2)
	if _site_surface_vertex_drag_index >= vertices.size():
		return false
	vertices[_site_surface_vertex_drag_index] = _clamp_site_surface_vertex_to_lot(Vector2(hit.x, hit.z))
	if not _site_surface_polygon_is_valid(vertices):
		return false
	_site_surfaces_data[_site_surface_drag_index]["vertices"] = _site_surface_vertices_to_arrays(vertices)
	_update_site_surface_controls()
	_update_site_surface_preview()
	return true

func _try_open_site_surface_context_menu(mouse_pos: Vector2) -> bool:
	if not _site_surface_context_menu:
		return false
	var hit = _project_mouse_to_horizontal_plane(mouse_pos, 0.0)
	if hit == null:
		return false
	var vertex_hit := _site_surface_vertex_hit_at_world_xz(hit)
	var edge_hit := {}
	if vertex_hit.is_empty():
		edge_hit = _site_surface_edge_hit_at_world_xz(hit)
	if vertex_hit.is_empty() and edge_hit.is_empty():
		return false

	_site_surface_context_menu.clear()
	_site_surface_context_index = -1
	_site_surface_context_vertex_index = -1
	_site_surface_context_edge_index = -1
	_site_surface_context_insert_point = Vector2(hit.x, hit.z)
	if not vertex_hit.is_empty():
		_site_surface_context_index = int(vertex_hit["surface"])
		_site_surface_context_vertex_index = int(vertex_hit["vertex"])
		_set_selected_site_surface(_site_surface_context_index)
		var vertices := _site_surface_vertices(_site_surfaces_data[_site_surface_context_index])
		_site_surface_context_menu.add_item("Delete Vertex", SITE_SURFACE_CONTEXT_DELETE_VERTEX)
		_site_surface_context_menu.set_item_disabled(0, vertices.size() <= 3)
	else:
		_site_surface_context_index = int(edge_hit["surface"])
		_site_surface_context_edge_index = int(edge_hit["edge"])
		_site_surface_context_insert_point = edge_hit["point"] as Vector2
		_set_selected_site_surface(_site_surface_context_index)
		_site_surface_context_menu.add_item("Add Vertex", SITE_SURFACE_CONTEXT_ADD_VERTEX)
	var popup_pos := get_viewport().get_mouse_position()
	_site_surface_context_menu.position = Vector2i(int(round(popup_pos.x)), int(round(popup_pos.y)))
	_site_surface_context_menu.popup()
	return true

func _on_site_surface_context_menu_id_pressed(id: int) -> void:
	match id:
		SITE_SURFACE_CONTEXT_ADD_VERTEX:
			_add_site_surface_vertex_from_context()
		SITE_SURFACE_CONTEXT_DELETE_VERTEX:
			_delete_site_surface_vertex_from_context()

func _add_site_surface_vertex_from_context() -> void:
	var surface_index := _site_surface_context_index
	if surface_index < 0 or surface_index >= _site_surfaces_data.size():
		return
	var edge_index := _site_surface_context_edge_index
	var vertices := _site_surface_vertices(_site_surfaces_data[surface_index])
	if edge_index < 0 or edge_index >= vertices.size():
		return
	vertices.insert(edge_index + 1, _clamp_site_surface_vertex_to_lot(_site_surface_context_insert_point))
	if not _site_surface_polygon_is_valid(vertices):
		_log("[color=yellow]Cannot add vertex there; it would create an invalid yard polygon.[/color]")
		return
	_site_surfaces_data[surface_index]["vertices"] = _site_surface_vertices_to_arrays(vertices)
	_set_selected_site_surface(surface_index)
	_log("Added yard vertex.")

func _delete_site_surface_vertex_from_context() -> void:
	var surface_index := _site_surface_context_index
	if surface_index < 0 or surface_index >= _site_surfaces_data.size():
		return
	var vertex_index := _site_surface_context_vertex_index
	var vertices := _site_surface_vertices(_site_surfaces_data[surface_index])
	if vertices.size() <= 3:
		_log("[color=yellow]Yard polygons need at least three vertices.[/color]")
		return
	if vertex_index < 0 or vertex_index >= vertices.size():
		return
	vertices.remove_at(vertex_index)
	if not _site_surface_polygon_is_valid(vertices):
		_log("[color=yellow]Cannot delete that vertex; it would create an invalid yard polygon.[/color]")
		return
	_site_surfaces_data[surface_index]["vertices"] = _site_surface_vertices_to_arrays(vertices)
	_set_selected_site_surface(surface_index)
	_log("Deleted yard vertex.")

func _try_begin_site_anchor_drag(mouse_pos: Vector2) -> bool:
	var hit = _project_mouse_to_horizontal_plane(mouse_pos, 0.0)
	if hit == null:
		return false
	var anchor_index := _site_anchor_index_at_world_xz(hit)
	if anchor_index < 0:
		return false
	if not _selected_site_anchor_indices.has(anchor_index):
		_select_site_anchor(anchor_index)
	else:
		_set_selected_site_anchors(_selected_site_anchor_indices, anchor_index)
	if _is_main_entrance_anchor(_site_anchors_data[anchor_index]):
		_main_entrance_auto = false
	var pos := _anchor_position(_site_anchors_data[anchor_index])
	_site_anchor_drag_offset = pos - Vector3(hit.x, pos.y, hit.z)
	_site_anchor_drag_start_hit = Vector3(hit.x, 0.0, hit.z)
	_site_anchor_drag_start_positions.clear()
	for index in _site_anchor_drag_indices():
		_site_anchor_drag_start_positions.append(_anchor_position(_site_anchors_data[index]))
	_mesh_part_drag_start_hit = _site_anchor_drag_start_hit
	_mesh_part_drag_start_positions.clear()
	for index in _mesh_part_drag_indices():
		_mesh_part_drag_start_positions.append(_part_positions[index])
	_dragging_site_anchor = true
	return true

func _drag_site_anchor_from_mouse(mouse_pos: Vector2) -> bool:
	var anchor_indices := _site_anchor_drag_indices()
	var mesh_indices := _mesh_part_drag_indices()
	if anchor_indices.is_empty() and mesh_indices.is_empty():
		return false
	var plane_y := 0.0
	if not anchor_indices.is_empty():
		plane_y = _anchor_position(_site_anchors_data[anchor_indices[0]]).y
	var hit = _project_mouse_to_horizontal_plane(mouse_pos, plane_y)
	if hit == null:
		return false
	var delta := Vector3(hit.x, 0.0, hit.z) - _site_anchor_drag_start_hit
	for i in anchor_indices.size():
		var index := anchor_indices[i]
		if index < 0 or index >= _site_anchors_data.size() or i >= _site_anchor_drag_start_positions.size():
			continue
		var start_pos := _site_anchor_drag_start_positions[i]
		_site_anchors_data[index]["position"] = _vector3_to_array(
			_clamp_site_anchor_position_to_lot(_site_anchors_data[index], start_pos + delta),
			0.01
		)
	for i in mesh_indices.size():
		var index := mesh_indices[i]
		if index < 0 or index >= _part_positions.size() or i >= _mesh_part_drag_start_positions.size():
			continue
		_part_positions[index] = _clamp_mesh_part_position_to_lot(
			index,
			_mesh_part_drag_start_positions[i] + delta
		)
		_apply_mesh_part_transform_from_state(index)
	_update_site_anchor_controls()
	_update_site_anchor_preview()
	_sync_selected_mesh_part_controls()
	_update_dim_label()
	return true

func _try_begin_site_anchor_rotation(mouse_pos: Vector2) -> bool:
	var hit = _project_mouse_to_horizontal_plane(mouse_pos, 0.0)
	if hit == null:
		return false
	var anchor_index := _site_anchor_index_at_world_xz(hit)
	if anchor_index < 0:
		return false
	_select_site_anchor(anchor_index)
	if _is_main_entrance_anchor(_site_anchors_data[anchor_index]):
		_main_entrance_auto = false
	_site_anchor_rotate_start_x = mouse_pos.x
	_site_anchor_rotate_start_yaw = _yaw_from_forward(_anchor_forward(_site_anchors_data[anchor_index]))
	_rotating_site_anchor = true
	return true

func _rotate_site_anchor_from_mouse(mouse_pos: Vector2) -> void:
	if _selected_site_anchor_index < 0 or _selected_site_anchor_index >= _site_anchors_data.size():
		return
	var delta_px := mouse_pos.x - _site_anchor_rotate_start_x
	var raw_rotation := _site_anchor_rotate_start_yaw + delta_px * MESH_ROTATION_DRAG_DEG_PER_PX
	_set_site_anchor_yaw(_selected_site_anchor_index, raw_rotation)

func _site_anchor_index_at_world_xz(world_pos: Vector3) -> int:
	for i in range(_site_anchors_data.size() - 1, -1, -1):
		if _site_anchor_contains_world_xz(i, world_pos):
			return i
	return -1

func _site_anchor_contains_world_xz(index: int, world_pos: Vector3) -> bool:
	if index < 0 or index >= _site_anchors_data.size():
		return false
	var anchor := _site_anchors_data[index]
	var anchor_type := str(anchor.get("anchor_type", "")).strip_edges()
	var pos := _anchor_position(anchor)
	var forward := _anchor_forward(anchor)
	var side := Vector3(-forward.z, 0.0, forward.x)
	var rel := Vector3(world_pos.x - pos.x, 0.0, world_pos.z - pos.z)
	var local_x := rel.dot(side)
	var local_z := rel.dot(forward)
	var width := maxf(0.1, _anchor_number(anchor, "width_m", SITE_ANCHOR_DRAG_RADIUS_M))
	if anchor_type == "parking" or anchor_type == "loading_bay":
		var length := maxf(0.1, _anchor_number(anchor, "length_m", width))
		return (
			absf(local_x) <= width * 0.5 + 0.5
			and local_z >= -0.5
			and local_z <= length + 0.5
		)
	if anchor_type == "driveway":
		return (
			absf(local_x) <= width * 0.5 + 0.5
			and local_z >= -0.75
			and local_z <= width + 0.75
		)
	return rel.length() <= SITE_ANCHOR_DRAG_RADIUS_M

func _site_surface_index_at_world_xz(world_pos: Vector3) -> int:
	if (
		_selected_site_surface_index >= 0
		and _selected_site_surface_index < _site_surfaces_data.size()
		and _site_surface_contains_world_xz(_selected_site_surface_index, world_pos)
	):
		return _selected_site_surface_index
	for i in range(_site_surfaces_data.size() - 1, -1, -1):
		if _site_surface_contains_world_xz(i, world_pos):
			return i
	return -1

func _site_surface_contains_world_xz(index: int, world_pos: Vector3) -> bool:
	if index < 0 or index >= _site_surfaces_data.size():
		return false
	return _point_in_site_surface_polygon(Vector2(world_pos.x, world_pos.z), _site_surface_vertices(_site_surfaces_data[index]))

func _point_in_site_surface_polygon(point: Vector2, vertices: Array[Vector2]) -> bool:
	if vertices.size() < 3:
		return false
	var inside := false
	var j := vertices.size() - 1
	for i in vertices.size():
		var vi := vertices[i]
		var vj := vertices[j]
		var crosses := (vi.y > point.y) != (vj.y > point.y)
		if crosses:
			var denom := vj.y - vi.y
			if absf(denom) < 0.000001:
				denom = 0.000001 if denom >= 0.0 else -0.000001
			var x_at_y := (vj.x - vi.x) * (point.y - vi.y) / denom + vi.x
			if point.x < x_at_y:
				inside = not inside
		j = i
	return inside

func _site_surface_vertex_hit_at_world_xz(world_pos: Vector3) -> Dictionary:
	var point := Vector2(world_pos.x, world_pos.z)
	if _selected_site_surface_index >= 0 and _selected_site_surface_index < _site_surfaces_data.size():
		var selected_vertices := _site_surface_vertices(_site_surfaces_data[_selected_site_surface_index])
		for vertex_index in selected_vertices.size():
			if point.distance_to(selected_vertices[vertex_index]) <= SITE_SURFACE_VERTEX_PICK_RADIUS_M:
				return {"surface": _selected_site_surface_index, "vertex": vertex_index}
	for surface_index in range(_site_surfaces_data.size() - 1, -1, -1):
		if surface_index == _selected_site_surface_index:
			continue
		var vertices := _site_surface_vertices(_site_surfaces_data[surface_index])
		for vertex_index in vertices.size():
			if point.distance_to(vertices[vertex_index]) <= SITE_SURFACE_VERTEX_PICK_RADIUS_M:
				return {"surface": surface_index, "vertex": vertex_index}
	return {}

func _site_surface_edge_hit_at_world_xz(world_pos: Vector3) -> Dictionary:
	var point := Vector2(world_pos.x, world_pos.z)
	var best_surface := -1
	var best_edge := -1
	var best_point := Vector2.ZERO
	var best_dist_sq := SITE_SURFACE_EDGE_PICK_RADIUS_M * SITE_SURFACE_EDGE_PICK_RADIUS_M
	if _selected_site_surface_index >= 0 and _selected_site_surface_index < _site_surfaces_data.size():
		var selected_hit := _site_surface_edge_hit_for_surface(_selected_site_surface_index, point, best_dist_sq)
		if not selected_hit.is_empty():
			return selected_hit
	for surface_index in range(_site_surfaces_data.size() - 1, -1, -1):
		if surface_index == _selected_site_surface_index:
			continue
		var vertices := _site_surface_vertices(_site_surfaces_data[surface_index])
		for edge_index in vertices.size():
			var a := vertices[edge_index]
			var b := vertices[(edge_index + 1) % vertices.size()]
			var nearest := _nearest_point_on_segment_2d(a, b, point)
			var dist_sq := nearest.distance_squared_to(point)
			if dist_sq <= best_dist_sq:
				best_dist_sq = dist_sq
				best_surface = surface_index
				best_edge = edge_index
				best_point = nearest
	if best_surface < 0:
		return {}
	return {"surface": best_surface, "edge": best_edge, "point": best_point}

func _site_surface_edge_hit_for_surface(surface_index: int, point: Vector2, max_dist_sq: float) -> Dictionary:
	if surface_index < 0 or surface_index >= _site_surfaces_data.size():
		return {}
	var vertices := _site_surface_vertices(_site_surfaces_data[surface_index])
	var best_edge := -1
	var best_point := Vector2.ZERO
	var best_dist_sq := max_dist_sq
	for edge_index in vertices.size():
		var a := vertices[edge_index]
		var b := vertices[(edge_index + 1) % vertices.size()]
		var nearest := _nearest_point_on_segment_2d(a, b, point)
		var dist_sq := nearest.distance_squared_to(point)
		if dist_sq <= best_dist_sq:
			best_dist_sq = dist_sq
			best_edge = edge_index
			best_point = nearest
	if best_edge < 0:
		return {}
	return {"surface": surface_index, "edge": best_edge, "point": best_point}

func _nearest_point_on_segment_2d(a: Vector2, b: Vector2, point: Vector2) -> Vector2:
	var ab := b - a
	var len_sq := ab.length_squared()
	if len_sq <= 0.000001:
		return a
	var t := clampf((point - a).dot(ab) / len_sq, 0.0, 1.0)
	return a + ab * t

func _try_begin_mesh_part_drag(mouse_pos: Vector2) -> bool:
	var hit = _project_mouse_to_horizontal_plane(mouse_pos, 0.0)
	if hit == null:
		return false
	var part_index := _mesh_part_index_at_world_xz(hit)
	if part_index < 0:
		return false
	if not _selected_part_indices.has(part_index):
		_select_mesh_part(part_index)
	else:
		_set_selected_mesh_parts(_selected_part_indices, part_index)
	var selected_indices := _mesh_part_drag_indices()
	if selected_indices.is_empty():
		return false
	_mesh_part_drag_start_hit = Vector3(hit.x, 0.0, hit.z)
	_mesh_part_drag_start_positions.clear()
	for index in selected_indices:
		_mesh_part_drag_start_positions.append(_part_positions[index])
	_site_anchor_drag_start_hit = _mesh_part_drag_start_hit
	_site_anchor_drag_start_positions.clear()
	for index in _site_anchor_drag_indices():
		_site_anchor_drag_start_positions.append(_anchor_position(_site_anchors_data[index]))
	_dragging_mesh_part = true
	return true

func _drag_mesh_part_from_mouse(mouse_pos: Vector2) -> bool:
	var selected_indices := _mesh_part_drag_indices()
	var anchor_indices := _site_anchor_drag_indices()
	if selected_indices.is_empty() and anchor_indices.is_empty():
		return false
	var hit = _project_mouse_to_horizontal_plane(mouse_pos, 0.0)
	if hit == null:
		return false
	var delta := Vector3(hit.x, 0.0, hit.z) - _mesh_part_drag_start_hit
	for i in selected_indices.size():
		var index := selected_indices[i]
		if index < 0 or index >= _part_positions.size() or i >= _mesh_part_drag_start_positions.size():
			continue
		_part_positions[index] = _clamp_mesh_part_position_to_lot(
			index,
			_mesh_part_drag_start_positions[i] + delta
		)
		_apply_mesh_part_transform_from_state(index)
	for i in anchor_indices.size():
		var index := anchor_indices[i]
		if index < 0 or index >= _site_anchors_data.size() or i >= _site_anchor_drag_start_positions.size():
			continue
		var start_pos := _site_anchor_drag_start_positions[i]
		_site_anchors_data[index]["position"] = _vector3_to_array(
			_clamp_site_anchor_position_to_lot(_site_anchors_data[index], start_pos + delta),
			0.01
		)
	_update_site_anchor_controls()
	_update_site_anchor_preview()
	_sync_selected_mesh_part_controls()
	_update_dim_label()
	return true

func _mesh_part_drag_indices() -> Array[int]:
	if not _selected_part_indices.is_empty():
		return _selected_part_indices
	if _has_selected_mesh_part():
		return [_selected_part_index]
	return []

func _site_anchor_drag_indices() -> Array[int]:
	if not _selected_site_anchor_indices.is_empty():
		return _selected_site_anchor_indices
	if _selected_site_anchor_index >= 0 and _selected_site_anchor_index < _site_anchors_data.size():
		return [_selected_site_anchor_index]
	return []

func _begin_box_selection(mouse_pos: Vector2, additive: bool) -> void:
	_selecting_mesh_parts = true
	_selection_additive = additive
	_selection_start_screen = mouse_pos
	_selection_end_screen = mouse_pos
	if _selection_rect_overlay and _selection_rect_overlay.has_method("clear"):
		_selection_rect_overlay.clear()

func _update_mesh_part_box_selection(mouse_pos: Vector2) -> void:
	_selection_end_screen = mouse_pos
	if _selection_start_screen.distance_to(_selection_end_screen) < SELECTION_DRAG_THRESHOLD_PX:
		if _selection_rect_overlay and _selection_rect_overlay.has_method("clear"):
			_selection_rect_overlay.clear()
		return
	if _selection_rect_overlay and _selection_rect_overlay.has_method("set_rect_global"):
		_selection_rect_overlay.set_rect_global(_selection_start_screen, _selection_end_screen, true)

func _finish_mesh_part_box_selection(mouse_pos: Vector2) -> void:
	_selecting_mesh_parts = false
	_selection_end_screen = mouse_pos
	if _selection_rect_overlay and _selection_rect_overlay.has_method("clear"):
		_selection_rect_overlay.clear()
	if _selection_start_screen.distance_to(_selection_end_screen) < SELECTION_DRAG_THRESHOLD_PX:
		if not _selection_additive and _human_visible and _place_human_from_mouse(mouse_pos):
			_selection_additive = false
			return
		if not _selection_additive:
			_set_selected_site_anchors([], -1)
			_set_selected_mesh_parts([], -1)
			_set_selected_site_surface(-1)
		_selection_additive = false
		return
	var selection_rect := _selection_screen_rect()
	var selected_meshes := _mesh_parts_in_screen_rect(selection_rect)
	var selected_anchors := _site_anchors_in_screen_rect(selection_rect)
	var selected_surfaces := _site_surfaces_in_screen_rect(selection_rect)
	if _selection_additive:
		selected_meshes = _merged_indices(_selected_part_indices, selected_meshes)
		selected_anchors = _merged_indices(_selected_site_anchor_indices, selected_anchors)
	if selected_meshes.is_empty() and selected_anchors.is_empty() and not selected_surfaces.is_empty():
		_set_selected_site_surface(int(selected_surfaces[0]))
	else:
		_set_selected_mesh_parts(
			selected_meshes,
			int(selected_meshes[0]) if not selected_meshes.is_empty() else -1
		)
		_set_selected_site_anchors(
			selected_anchors,
			int(selected_anchors[0]) if not selected_anchors.is_empty() else -1
		)
		if not _selection_additive:
			_set_selected_site_surface(-1)
	_selection_additive = false

func _selection_screen_rect() -> Rect2:
	var top_left := Vector2(
		minf(_selection_start_screen.x, _selection_end_screen.x),
		minf(_selection_start_screen.y, _selection_end_screen.y)
	)
	var size := Vector2(
		absf(_selection_start_screen.x - _selection_end_screen.x),
		absf(_selection_start_screen.y - _selection_end_screen.y)
	)
	return Rect2(top_left, size)

func _mesh_parts_in_screen_rect(selection_rect: Rect2) -> Array[int]:
	var cam := get_viewport().get_camera_3d()
	if not cam or not _preview or not _preview.has_method("mesh_part_world_corners"):
		return []
	var selected: Array[int] = []
	for index in _lod_source_paths.size():
		var part_rect = _mesh_part_screen_rect(index, cam)
		if part_rect != null and selection_rect.intersects(part_rect, true):
			selected.append(index)
	return selected

func _site_anchors_in_screen_rect(selection_rect: Rect2) -> Array[int]:
	var cam := get_viewport().get_camera_3d()
	if not cam:
		return []
	var selected: Array[int] = []
	for index in _site_anchors_data.size():
		var anchor_rect = _site_anchor_screen_rect(index, cam)
		if anchor_rect != null and selection_rect.intersects(anchor_rect, true):
			selected.append(index)
	return selected

func _site_surfaces_in_screen_rect(selection_rect: Rect2) -> Array[int]:
	var cam := get_viewport().get_camera_3d()
	if not cam:
		return []
	var selected: Array[int] = []
	for index in _site_surfaces_data.size():
		var surface_rect = _site_surface_screen_rect(index, cam)
		if surface_rect != null and selection_rect.intersects(surface_rect, true):
			selected.append(index)
	return selected

func _site_anchor_screen_rect(index: int, cam: Camera3D):
	if index < 0 or index >= _site_anchors_data.size():
		return null
	var anchor := _site_anchors_data[index]
	var anchor_type := str(anchor.get("anchor_type", "")).strip_edges()
	var pos := _anchor_position(anchor)
	var forward := _anchor_forward(anchor)
	var side := Vector3(-forward.z, 0.0, forward.x)
	var width := maxf(0.1, _anchor_number(anchor, "width_m", SITE_ANCHOR_DRAG_RADIUS_M))
	var points: Array = []
	if anchor_type == "parking" or anchor_type == "loading_bay":
		var half_w := width * 0.5
		var length := maxf(0.1, _anchor_number(anchor, "length_m", width))
		points = [
			pos - side * half_w,
			pos + side * half_w,
			pos + side * half_w + forward * length,
			pos - side * half_w + forward * length,
		]
	elif anchor_type == "driveway":
		var half_w := width * 0.5
		var length_m := maxf(1.5, width * 1.4)
		points = [
			pos - side * half_w,
			pos + side * half_w,
			pos + side * half_w + forward * length_m,
			pos - side * half_w + forward * length_m,
		]
	else:
		if cam.is_position_behind(pos):
			return null
		var screen_pos := cam.unproject_position(pos)
		var radius := MAIN_ENTRANCE_PICK_RADIUS_PX
		return Rect2(screen_pos - Vector2(radius, radius), Vector2(radius * 2.0, radius * 2.0))
	return _screen_rect_for_world_points(points, cam)

func _mesh_part_screen_rect(part_index: int, cam: Camera3D):
	var corners: Array = _preview.mesh_part_world_corners(part_index)
	return _screen_rect_for_world_points(corners, cam)

func _site_surface_screen_rect(index: int, cam: Camera3D):
	if index < 0 or index >= _site_surfaces_data.size():
		return null
	var points: Array = []
	var y := _site_surface_number(_site_surfaces_data[index], "y_m", 0.01)
	for vertex in _site_surface_vertices(_site_surfaces_data[index]):
		points.append(Vector3(vertex.x, y, vertex.y))
	return _screen_rect_for_world_points(points, cam)

func _screen_rect_for_world_points(points: Array, cam: Camera3D):
	var has_point := false
	var min_pos := Vector2.ZERO
	var max_pos := Vector2.ZERO
	for point in points:
		if not (point is Vector3):
			continue
		var world_pos := point as Vector3
		if cam.is_position_behind(world_pos):
			continue
		var screen_pos := cam.unproject_position(world_pos)
		if not has_point:
			min_pos = screen_pos
			max_pos = screen_pos
			has_point = true
		else:
			min_pos.x = minf(min_pos.x, screen_pos.x)
			min_pos.y = minf(min_pos.y, screen_pos.y)
			max_pos.x = maxf(max_pos.x, screen_pos.x)
			max_pos.y = maxf(max_pos.y, screen_pos.y)
	if not has_point:
		return null
	return Rect2(min_pos, max_pos - min_pos)

func _merged_indices(existing: Array, incoming: Array) -> Array[int]:
	var seen := {}
	var result: Array[int] = []
	for source in [existing, incoming]:
		for raw_index in source:
			var index := int(raw_index)
			if seen.has(index):
				continue
			seen[index] = true
			result.append(index)
	result.sort()
	return result

func _toggle_selection_at_mouse(mouse_pos: Vector2) -> bool:
	var hit = _project_mouse_to_horizontal_plane(mouse_pos, 0.0)
	if hit == null:
		return false
	var surface_vertex := _site_surface_vertex_hit_at_world_xz(hit)
	if not surface_vertex.is_empty():
		_toggle_site_surface_selection(int(surface_vertex["surface"]))
		return true
	var surface_index := _site_surface_index_at_world_xz(hit)
	if surface_index >= 0:
		_toggle_site_surface_selection(surface_index)
		return true
	var anchor_index := _site_anchor_index_at_world_xz(hit)
	if anchor_index >= 0:
		_toggle_site_anchor_selection(anchor_index)
		return true
	var part_index := _mesh_part_index_at_world_xz(hit)
	if part_index >= 0:
		_toggle_mesh_part_selection(part_index)
		return true
	return false

func _toggle_site_surface_selection(index: int) -> void:
	if index < 0 or index >= _site_surfaces_data.size():
		return
	if _selected_site_surface_index == index:
		_set_selected_site_surface(-1)
	else:
		_set_selected_site_surface(index)

func _toggle_mesh_part_selection(index: int) -> void:
	if index < 0 or index >= _lod_source_paths.size():
		return
	var selected := _selected_part_indices.duplicate()
	if selected.has(index):
		selected.erase(index)
		_set_selected_mesh_parts(selected, -1 if selected.is_empty() else int(selected[0]))
	else:
		selected.append(index)
		_set_selected_mesh_parts(selected, index)

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
	_part_positions[_selected_part_index] = _clamp_mesh_part_position_to_lot(
		_selected_part_index,
		position
	)
	_sync_selected_mesh_part_controls()
	_apply_selected_part_transform_from_state()

func _set_selected_mesh_part_rotation_y(rotation_y: float) -> void:
	if not _has_selected_mesh_part():
		return
	_part_rotation_y[_selected_part_index] = rotation_y
	_part_positions[_selected_part_index] = _clamp_mesh_part_position_to_lot(
		_selected_part_index,
		_part_positions[_selected_part_index]
	)
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
	_apply_mesh_part_transform_from_state(_selected_part_index)

func _apply_mesh_part_transform_from_state(index: int) -> void:
	if index < 0 or index >= _lod_source_paths.size():
		return
	_preview.set_mesh_part_transform(
		index,
		_part_positions[index],
		_part_rotation_y[index],
		_part_scales[index],
		_part_pivot_offsets[index]
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
