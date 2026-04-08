## Asset editor shell — launched via `--asset-editor` command-line argument.
## Shares the same SimulationNode and compiled .so as the game, but runs a
## 500 m sandbox with no agents, no demand simulation, and no background tick thread.
## Calls: sim.is_asset_editor_mode(), sim.load_asset_packs(dir, filter),
##        sim.get_registered_asset_ids(), sim.validate_and_export_asset(),
##        sim.get_asset_manifest_json(), sim.get_pack_manifest_json(),
##        sim.load_economy_project()
extends Node3D

const PANEL_LEFT_W  := 270
const PANEL_RIGHT_W := 300
const PANEL_BOT_H   := 140
const MAIN_ENTRANCE_PICK_RADIUS_PX := 18.0

const TEMPLATES := [
	"Flat Studio",
	"Zoned Roadside",
	"Lane Reference",
	"Traffic Comparison",
	"Night Lighting",
]

const ZONE_TYPES := ["residential", "commercial", "industrial", "office", "mixed"]
const DENSITY_TYPES := ["low", "medium", "high"]

@onready var sim: SimulationNode = $SimulationNode

# ── UI refs ───────────────────────────────────────────────────────────────────
var _log_label: RichTextLabel
var _asset_list: ItemList
var _template_btn: OptionButton

# Inspector – pack
var _pack_id_edit: LineEdit
var _pack_name_edit: LineEdit
var _pack_author_edit: LineEdit

# Inspector – asset
var _asset_id_edit: LineEdit
var _display_name_edit: LineEdit
var _asset_set_edit: LineEdit
var _tags_edit: LineEdit
var _zone_type_btn: OptionButton
var _density_btn: OptionButton
var _width_spin: SpinBox
var _depth_spin: SpinBox
var _level_spin: SpinBox
var _residents_spin: SpinBox
var _workers_spin: SpinBox
var _economy_profile_btn: OptionButton
var _economy_profile_status_lbl: Label
var _lod_list: ItemList
var _lod_source_paths: Array[String] = []  # parallel to _lod_list items
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
var _dim_label: Label          # live "→ W × D × H m" display
var _scale_preset_btn: OptionButton

# Registered asset IDs, refreshed after pack load.
var _asset_ids: PackedStringArray = []
var _asset_id_auto: bool = true  # false once the user manually edits the ID field
var _autofit_on_load: bool = false   # true when loading from browser with scale=1 (never fitted)
var _keep_camera: bool = false       # true when loading from browser — skip focus_on
var _human_visible: bool = false     # mirrors the human figure toggle state
var _last_glb_dir: String = ""     # last directory used in GLB file dialogs
var _config: ConfigFile            # persistent editor preferences
var _font_size_header:  int = 14   # section title labels ("Asset Browser", "Building Importer")
var _font_size_section: int = 12   # sub-section labels ("Pack", "Asset", "Building", etc.)
var _font_size_label:   int = 11   # spinbox labels and small info text
var _economy_profile_ids: Array[String] = []
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

# ──────────────────────────────────────────────────────────────────────────────

const CONFIG_PATH := "user://asset_editor.cfg"

func _ready() -> void:
	if not sim.is_asset_editor_mode():
		push_error("AssetEditor scene loaded without --asset-editor flag")

	_config = ConfigFile.new()
	_config.load(CONFIG_PATH)  # silently no-ops if file doesn't exist yet
	_last_glb_dir = _config.get_value("import", "last_glb_dir", "")

	_font_size_header  = _config.get_value("ui", "font_size_header",  14)
	_font_size_section = _config.get_value("ui", "font_size_section", 12)
	_font_size_label   = _config.get_value("ui", "font_size_label",   11)
	_save_config()  # write defaults if keys are missing

	_build_preview_node()
	_build_ui()
	_bbcode_strip_regex = RegEx.new()
	_bbcode_strip_regex.compile("\\[/?[^\\]]+\\]")
	_set_frontage_forward(_frontage_fwd)
	_set_main_entrance_position(_default_main_entrance_position(), true)
	_load_economy_profiles()
	_load_packs()
	_apply_template(0)

# ──────────────────────────────────────────────────────────────────────────────
# 3D preview
# ──────────────────────────────────────────────────────────────────────────────

func _build_preview_node() -> void:
	var script := load("res://scripts/building_preview.gd")
	_preview = Node3D.new()
	_preview.set_script(script)
	add_child(_preview)
	_preview.mesh_loaded.connect(_on_mesh_loaded)

	var cam_script := load("res://scripts/editor_camera_input.gd")
	_cam_input = Node.new()
	_cam_input.set_script(cam_script)
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
	canvas.add_child(anchor)

	# Root VBox fills the anchor.
	var root := VBoxContainer.new()
	root.set_anchors_preset(Control.PRESET_FULL_RECT)
	root.add_theme_constant_override("separation", 0)
	anchor.add_child(root)

	# Top row: left panel | transparent center | right panel — expands vertically.
	var h_row := HBoxContainer.new()
	h_row.size_flags_vertical = Control.SIZE_EXPAND_FILL
	h_row.add_theme_constant_override("separation", 0)
	root.add_child(h_row)

	_build_left_panel(h_row)

	# Center spacer — 3D viewport shows through.
	# MOUSE_FILTER_IGNORE so the GUI system never claims mouse events here,
	# which lets gui_get_hover_control() return null in the 3D area.
	var center := Control.new()
	center.size_flags_horizontal = Control.SIZE_EXPAND_FILL
	center.mouse_filter = Control.MOUSE_FILTER_IGNORE
	h_row.add_child(center)

	_build_right_panel(h_row)
	_build_bottom_panel(root)

func _build_left_panel(parent: Control) -> void:
	var panel := PanelContainer.new()
	panel.custom_minimum_size.x = PANEL_LEFT_W
	panel.size_flags_vertical = Control.SIZE_EXPAND_FILL
	parent.add_child(panel)

	var scroll := ScrollContainer.new()
	scroll.size_flags_vertical = Control.SIZE_EXPAND_FILL
	panel.add_child(scroll)

	var vbox := VBoxContainer.new()
	vbox.size_flags_horizontal = Control.SIZE_EXPAND_FILL
	scroll.add_child(vbox)

	_add_label(vbox, "Asset Browser", _font_size_header)
	_asset_list = ItemList.new()
	_asset_list.custom_minimum_size.y = 160
	_asset_list.item_activated.connect(_on_asset_selected)
	vbox.add_child(_asset_list)

	vbox.add_child(HSeparator.new())
	_add_label(vbox, "Scene Template", _font_size_section)
	_template_btn = OptionButton.new()
	for t in TEMPLATES:
		_template_btn.add_item(t)
	_template_btn.item_selected.connect(_on_template_selected)
	vbox.add_child(_template_btn)

func _build_right_panel(parent: Control) -> void:
	var panel := PanelContainer.new()
	panel.custom_minimum_size.x = PANEL_RIGHT_W
	panel.size_flags_vertical = Control.SIZE_EXPAND_FILL
	parent.add_child(panel)

	var scroll := ScrollContainer.new()
	scroll.size_flags_vertical = Control.SIZE_EXPAND_FILL
	panel.add_child(scroll)

	var vbox := VBoxContainer.new()
	vbox.size_flags_horizontal = Control.SIZE_EXPAND_FILL
	scroll.add_child(vbox)

	_add_label(vbox, "Building Importer", _font_size_header)

	# Import GLB
	var glb_btn := Button.new()
	glb_btn.text = "Import mesh..."
	glb_btn.pressed.connect(_on_import_glb_pressed)
	vbox.add_child(glb_btn)

	vbox.add_child(HSeparator.new())
	_add_label(vbox, "Pack", _font_size_section)
	_pack_id_edit     = _add_line_edit(vbox, "Pack ID (kebab-case)", "my-pack")
	_pack_name_edit   = _add_line_edit(vbox, "Pack Name", "My Pack")
	_pack_author_edit = _add_line_edit(vbox, "Author", "")

	vbox.add_child(HSeparator.new())
	_add_label(vbox, "Asset", _font_size_section)
	_asset_id_edit    = _add_line_edit(vbox, "Asset ID (dot.separated)", "building.residential.house")
	_asset_id_edit.text_changed.connect(_on_asset_id_text_changed)
	_display_name_edit = _add_line_edit(vbox, "Display Name", "House")
	_display_name_edit.text_changed.connect(func(_t): _auto_suggest_asset_id())
	_asset_set_edit   = _add_line_edit(vbox, "Asset Set (optional)", "")
	_tags_edit        = _add_line_edit(vbox, "Tags (comma-separated)", "")

	vbox.add_child(HSeparator.new())
	_add_label(vbox, "Building", _font_size_section)

	_zone_type_btn = OptionButton.new()
	for z in ZONE_TYPES:
		_zone_type_btn.add_item(z.capitalize())
	_zone_type_btn.item_selected.connect(_on_zone_or_lot_changed)
	_zone_type_btn.item_selected.connect(func(_i): _auto_suggest_asset_id())
	vbox.add_child(_zone_type_btn)

	_add_label(vbox, "Density", _font_size_label)
	_density_btn = OptionButton.new()
	for d in DENSITY_TYPES:
		_density_btn.add_item(d.capitalize())
	vbox.add_child(_density_btn)

	_width_spin    = _add_spinbox(vbox, "Lot Width (cells)", 1, 20, 2)
	_depth_spin    = _add_spinbox(vbox, "Lot Depth (cells)", 1, 20, 2)
	_level_spin    = _add_spinbox(vbox, "Level", 1, 255, 1)
	_residents_spin = _add_spinbox(vbox, "Residents Capacity", 0, 9999, 0)
	_workers_spin  = _add_spinbox(vbox, "Worker Capacity", 0, 9999, 0)
	_add_label(vbox, "Economy Profile", _font_size_label)
	var economy_row := HBoxContainer.new()
	_economy_profile_btn = OptionButton.new()
	_economy_profile_btn.size_flags_horizontal = Control.SIZE_EXPAND_FILL
	_economy_profile_btn.item_selected.connect(_on_economy_profile_selected)
	economy_row.add_child(_economy_profile_btn)
	var refresh_profiles_btn := Button.new()
	refresh_profiles_btn.text = "Refresh"
	refresh_profiles_btn.pressed.connect(_load_economy_profiles)
	economy_row.add_child(refresh_profiles_btn)
	vbox.add_child(economy_row)
	_economy_profile_status_lbl = Label.new()
	_economy_profile_status_lbl.autowrap_mode = TextServer.AUTOWRAP_WORD_SMART
	_economy_profile_status_lbl.add_theme_font_size_override("font_size", _font_size_label)
	vbox.add_child(_economy_profile_status_lbl)
	var suggest_btn := Button.new()
	suggest_btn.text = "Suggest Capacity"
	suggest_btn.pressed.connect(_suggest_capacity)
	vbox.add_child(suggest_btn)

	_width_spin.value_changed.connect(func(_v): _on_zone_or_lot_changed(0))
	_depth_spin.value_changed.connect(func(_v): _on_zone_or_lot_changed(0))

	vbox.add_child(HSeparator.new())
	_add_label(vbox, "Preview Scale", _font_size_section)

	# Preset dropdown.
	_scale_preset_btn = OptionButton.new()
	_scale_preset_btn.add_item("Custom")
	_scale_preset_btn.add_item("Fit to Lot")
	_scale_preset_btn.add_item("½ Lot")
	_scale_preset_btn.add_item("¼ Lot")
	_scale_preset_btn.item_selected.connect(_on_scale_preset_selected)
	vbox.add_child(_scale_preset_btn)

	_preview_scale_spin = _add_spinbox(vbox, "Scale multiplier", 0.01, 1000.0, 1.0)
	_preview_scale_spin.step = 0.01
	_preview_scale_spin.value_changed.connect(_on_preview_scale_changed)

	# Live dimension label.
	_dim_label = Label.new()
	_dim_label.text = "→ —"
	_dim_label.add_theme_font_size_override("font_size", _font_size_label)
	_dim_label.add_theme_color_override("font_color", Color(0.7, 0.9, 0.7))
	vbox.add_child(_dim_label)

	var autofit_btn := Button.new()
	autofit_btn.text = "Auto-fit to Lot"
	autofit_btn.pressed.connect(_on_autofit_pressed)
	vbox.add_child(autofit_btn)

	var human_btn := CheckButton.new()
	human_btn.text = "Show Human (1.8 m)"
	human_btn.toggled.connect(_on_human_toggled)
	vbox.add_child(human_btn)

	var clear_ghost_btn := Button.new()
	clear_ghost_btn.text = "Clear Ghost"
	clear_ghost_btn.pressed.connect(_on_clear_ghost_pressed)
	vbox.add_child(clear_ghost_btn)

	vbox.add_child(HSeparator.new())
	_add_label(vbox, "LOD Files", _font_size_section)
	_lod_list = ItemList.new()
	_lod_list.custom_minimum_size.y = 60
	vbox.add_child(_lod_list)
	var add_lod_btn := Button.new()
	add_lod_btn.text = "Add LOD..."
	add_lod_btn.pressed.connect(_on_add_lod_pressed)
	vbox.add_child(add_lod_btn)

	vbox.add_child(HSeparator.new())
	_add_label(vbox, "Frontage", _font_size_section)
	_frontage_lbl = Label.new()
	_frontage_lbl.text = "Forward: (0, 0, 1)"
	_frontage_lbl.add_theme_font_size_override("font_size", _font_size_label)
	vbox.add_child(_frontage_lbl)
	var set_front_btn := Button.new()
	set_front_btn.text = "Set Front From View"
	set_front_btn.pressed.connect(_on_set_front_from_view)
	vbox.add_child(set_front_btn)
	_add_label(vbox, "Main Entrance (local)", _font_size_label)
	_entrance_x_spin = _add_spinbox(vbox, "X (m)", -500.0, 500.0, 0.0)
	_entrance_y_spin = _add_spinbox(vbox, "Y (m)", -500.0, 500.0, 0.0)
	_entrance_z_spin = _add_spinbox(vbox, "Z (m)", -500.0, 500.0, 10.0)
	_entrance_x_spin.step = 0.1
	_entrance_y_spin.step = 0.1
	_entrance_z_spin.step = 0.1
	_entrance_x_spin.value_changed.connect(_on_main_entrance_changed)
	_entrance_y_spin.value_changed.connect(_on_main_entrance_changed)
	_entrance_z_spin.value_changed.connect(_on_main_entrance_changed)
	var reset_entrance_btn := Button.new()
	reset_entrance_btn.text = "Reset Entrance To Frontage"
	reset_entrance_btn.pressed.connect(_on_reset_main_entrance_pressed)
	vbox.add_child(reset_entrance_btn)
	var entrance_hint := Label.new()
	entrance_hint.text = "Drag the yellow sphere in the viewport to move X/Z. Use Y for height."
	entrance_hint.autowrap_mode = TextServer.AUTOWRAP_WORD_SMART
	entrance_hint.add_theme_font_size_override("font_size", _font_size_label)
	vbox.add_child(entrance_hint)

	vbox.add_child(HSeparator.new())
	var export_btn := Button.new()
	export_btn.text = "Export Asset"
	export_btn.pressed.connect(_on_export_pressed)
	vbox.add_child(export_btn)

func _build_bottom_panel(parent: Control) -> void:
	var panel := PanelContainer.new()
	panel.custom_minimum_size.y = PANEL_BOT_H
	panel.size_flags_horizontal = Control.SIZE_EXPAND_FILL
	parent.add_child(panel)

	var vbox := VBoxContainer.new()
	vbox.size_flags_vertical = Control.SIZE_EXPAND_FILL
	panel.add_child(vbox)

	var header := HBoxContainer.new()
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
	_asset_ids = sim.get_registered_asset_ids()
	_asset_list.clear()
	for aid in _asset_ids:
		_asset_list.add_item(aid)
	_log("Registry: %d asset(s) loaded." % _asset_ids.size())

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

func _on_asset_selected(idx: int) -> void:
	if idx < 0 or idx >= _asset_ids.size():
		return
	var aid: String = _asset_ids[idx]
	var json_str: String = sim.get_asset_manifest_json(aid)
	if json_str.is_empty():
		return
	var data = JSON.parse_string(json_str)
	if data == null:
		return
	_populate_inspector_from(data)
	_log("Loaded manifest for '%s'." % aid)

func _populate_inspector_from(data: Dictionary) -> void:
	# Pack fields — read from pack.toml on disk via Rust.
	var pack_id: String = data.get("pack_id", "")
	if not pack_id.is_empty():
		_pack_id_edit.text = pack_id
		var pack_dir: String = ProjectSettings.globalize_path("user://mods/" + pack_id + "/")
		var pack_json: String = sim.get_pack_manifest_json(pack_dir)
		if not pack_json.is_empty():
			var pack_data = JSON.parse_string(pack_json)
			if pack_data is Dictionary:
				_pack_name_edit.text   = pack_data.get("display_name", "")
				_pack_author_edit.text = pack_data.get("author", "")

	# Prevent auto-suggest from overwriting the loaded asset ID.
	_asset_id_auto = false
	_asset_id_edit.text     = data.get("asset_id", "")
	_display_name_edit.text = data.get("display_name", "")
	_asset_set_edit.text    = data.get("asset_set", "") if data.get("asset_set") != null else ""
	_tags_edit.text         = ", ".join(data.get("tags", []))
	_width_spin.value       = data.get("lot_width_cells", 1)
	_depth_spin.value       = data.get("lot_depth_cells", 1)
	_level_spin.value       = data.get("level", 1)
	_residents_spin.value   = data.get("residents_capacity", 0) if data.get("residents_capacity") != null else 0
	_workers_spin.value     = data.get("worker_capacity", 0) if data.get("worker_capacity") != null else 0
	_preview_scale_spin.value = data.get("preview_scale", 1.0) if data.get("preview_scale") != null else 1.0
	_preview.set_preview_scale(_preview_scale_spin.value)
	var po = data.get("pivot_offset", null)
	if po is Array and po.size() == 3:
		_pivot_offset = Vector3(po[0], po[1], po[2])
	else:
		_pivot_offset = Vector3.ZERO

	var zt: String = data.get("zone_type", "residential")
	var zi := ZONE_TYPES.find(zt)
	if zi >= 0:
		_zone_type_btn.selected = zi

	var dt: String = data.get("density", "low")
	var di := DENSITY_TYPES.find(dt)
	_density_btn.selected = maxi(0, di)
	_set_economy_profile_selection(data.get("economy_profile", "") if data.get("economy_profile") != null else "")
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

	_lod_list.clear()
	_lod_source_paths.clear()
	var lods: Array = data.get("lods", [])
	for lod in lods:
		_lod_list.add_item("%s  (%.0f–%s m)" % [
			lod.get("file", "?"),
			lod.get("distance_min_m", 0.0),
			str(lod.get("distance_max_m", "∞")),
		])
		# Resolve the source path from the mods directory.
		var asset_id: String = data.get("asset_id", "")
		var fname: String = lod.get("file", "")
		var native: String = ProjectSettings.globalize_path(
			"user://mods/%s/assets/%s/%s" % [pack_id, asset_id, fname])
		_lod_source_paths.append(native)

	_preview.set_lot_size(int(_width_spin.value), int(_depth_spin.value))

	# Load LOD0 mesh into the preview if the file exists on disk.
	if lods.size() > 0 and not pack_id.is_empty():
		var lod0_path: String = _lod_source_paths[0]
		if FileAccess.file_exists(lod0_path):
			_glb_path = lod0_path
			# Auto-fit when the asset has never been scaled (scale == 1.0).
			_autofit_on_load = absf(_preview_scale_spin.value - 1.0) < 0.01
			_keep_camera = true
			_preview.load_glb(lod0_path)
		else:
			_log("[color=yellow]LOD0 file not found on disk: %s[/color]" % lod0_path)
			_preview.clear()

# ──────────────────────────────────────────────────────────────────────────────
# Lot / zone change
# ──────────────────────────────────────────────────────────────────────────────

func _on_zone_or_lot_changed(_idx) -> void:
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

	var validation: Array = payload.get("validation", [])
	for message in validation:
		if message is Dictionary and str(message.get("severity", "")) != "error":
			_economy_catalog_warning_count += 1

	_economy_catalog_loaded = true
	_update_economy_profile_status()
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
	if not _economy_profile_status_lbl:
		return

	var selected_id := _selected_economy_profile_id()
	var zone_type: String = ""
	if _zone_type_btn:
		zone_type = ZONE_TYPES[_zone_type_btn.selected]
	if not _economy_catalog_loaded:
		var msg := "Economy catalog unavailable."
		if not _economy_catalog_error.is_empty():
			msg = "Economy catalog unavailable: %s" % _economy_catalog_error
		if not selected_id.is_empty():
			msg += " Existing selection will be preserved on export."
		_set_economy_profile_status(msg, Color(0.95, 0.78, 0.38))
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

	if not _unresolved_economy_profile_id.is_empty() and selected_id == _unresolved_economy_profile_id:
		_set_economy_profile_status(
			"Selected profile is missing from the current economy catalog and will be exported unchanged.",
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
	var zone: String = ZONE_TYPES[_zone_type_btn.selected]
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
	_asset_id_edit.text = "building.%s.%s" % [zone, clean]
	_asset_id_edit.text_changed.connect(_on_asset_id_text_changed)

# ──────────────────────────────────────────────────────────────────────────────
# Import GLB
# ──────────────────────────────────────────────────────────────────────────────

func _on_import_glb_pressed() -> void:
	var dialog := FileDialog.new()
	dialog.access = FileDialog.ACCESS_FILESYSTEM
	dialog.file_mode = FileDialog.FILE_MODE_OPEN_FILE
	dialog.filters = PackedStringArray(["*.glb ; GLB Files", "*.gltf ; GLTF Files", "*.fbx ; FBX Files"])
	if not _last_glb_dir.is_empty():
		dialog.current_dir = _last_glb_dir
	dialog.file_selected.connect(_on_glb_file_selected)
	dialog.canceled.connect(dialog.queue_free)
	add_child(dialog)
	dialog.popup_centered(Vector2i(800, 600))

func _on_glb_file_selected(path: String) -> void:
	_last_glb_dir = path.get_base_dir()
	_save_config()
	_glb_path = path
	_preview.load_glb(path)

	var fname := path.get_file()
	if _lod_list.item_count == 0:
		# First import — add LOD0 entry.
		_lod_list.add_item("%s  (0–150 m)" % fname)
		_lod_source_paths.append(path)
	else:
		# Replace LOD0 with the new file, preserving the distance band text.
		var old_text: String = _lod_list.get_item_text(0)
		var band := "0–150 m"
		var parts := old_text.split("  ")
		if parts.size() >= 2:
			band = parts[1].trim_prefix("(").trim_suffix(")")
		_lod_list.set_item_text(0, "%s  (%s)" % [fname, band])
		if _lod_source_paths.size() > 0:
			_lod_source_paths[0] = path
		else:
			_lod_source_paths.append(path)
	_log("LOD0 set to '%s'." % fname)

# ──────────────────────────────────────────────────────────────────────────────
# LOD management
# ──────────────────────────────────────────────────────────────────────────────

func _on_add_lod_pressed() -> void:
	var dialog := FileDialog.new()
	dialog.access = FileDialog.ACCESS_FILESYSTEM
	dialog.file_mode = FileDialog.FILE_MODE_OPEN_FILE
	dialog.filters = PackedStringArray(["*.glb ; GLB Files", "*.gltf ; GLTF Files", "*.fbx ; FBX Files"])
	if not _last_glb_dir.is_empty():
		dialog.current_dir = _last_glb_dir
	dialog.file_selected.connect(_on_lod_file_selected)
	dialog.canceled.connect(dialog.queue_free)
	add_child(dialog)
	dialog.popup_centered(Vector2i(800, 600))

func _on_lod_file_selected(path: String) -> void:
	_last_glb_dir = path.get_base_dir()
	_save_config()
	var fname := path.get_file()
	var idx := _lod_list.item_count
	# Default distance bands per LOD tier.
	var min_m := [0.0, 150.0, 600.0, 2000.0]
	var max_m := ["150", "600", "2000", "∞"]
	var i := mini(idx, min_m.size() - 1)
	_lod_list.add_item("%s  (%.0f–%s m)" % [fname, min_m[i], max_m[i]])
	_lod_source_paths.append(path)
	_log("LOD%d set to '%s'." % [idx, fname])

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
	_set_frontage_forward(horizontal)
	if _main_entrance_auto:
		_set_main_entrance_position(_default_main_entrance_position(), true)
	_log("Frontage set: front face points toward camera.")

# ──────────────────────────────────────────────────────────────────────────────
# Export
# ──────────────────────────────────────────────────────────────────────────────

func _on_export_pressed() -> void:
	var pack_id: String = _pack_id_edit.text.strip_edges()
	if pack_id.is_empty():
		_log("[color=red]Pack ID is required.[/color]")
		return

	var asset_id: String = _asset_id_edit.text.strip_edges()
	if asset_id.is_empty():
		_log("[color=red]Asset ID is required.[/color]")
		return

	if _lod_list.item_count == 0:
		_log("[color=yellow]Warning: no LOD files registered. Add at least LOD0.[/color]")

	# Build LOD array from the list items. The file name is the first token before spaces.
	var lods := []
	var default_min := [0.0, 150.0, 600.0, 2000.0]
	var default_max := [150.0, 600.0, 2000.0, null]
	for i in _lod_list.item_count:
		var fname := _lod_list.get_item_text(i).split(" ")[0]
		lods.append({
			"file": fname,
			"distance_min_m": default_min[mini(i, default_min.size() - 1)],
			"distance_max_m": default_max[mini(i, default_max.size() - 1)],
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

	var params := {
		"pack_id":          pack_id,
		"pack_name":        _pack_name_edit.text.strip_edges(),
		"pack_author":      _pack_author_edit.text.strip_edges(),
		"asset_class":      "building",
		"asset_id":         asset_id,
		"display_name":     _display_name_edit.text.strip_edges(),
		"asset_set":        asset_set_val if not asset_set_val.is_empty() else null,
		"tags":             tags,
		"zone_type":        ZONE_TYPES[_zone_type_btn.selected],
		"density":          DENSITY_TYPES[_density_btn.selected],
		"lot_width_cells":   int(_width_spin.value),
		"lot_depth_cells":   int(_depth_spin.value),
		"level":             int(_level_spin.value),
		"economy_profile":   economy_profile_id if not economy_profile_id.is_empty() else null,
		"preview_scale":     _preview_scale_spin.value,
		"pivot_offset":      [_pivot_offset.x, _pivot_offset.y, _pivot_offset.z],
		"residents_capacity": int(_residents_spin.value) if _residents_spin.value > 0 else null,
		"worker_capacity":    int(_workers_spin.value)   if _workers_spin.value > 0 else null,
		"lods":    lods,
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
		for lod_entry in lods:
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
				_log("[color=yellow]LOD%d source not found on disk — skipped: %s[/color]" % [i, src])
				continue
			var dst: String = asset_dir + src.get_file()
			if src != dst:
				var copy_err := DirAccess.copy_absolute(src, dst)
				if copy_err != OK:
					_log("[color=red]Failed to copy LOD%d '%s' (error %d)[/color]" % [i, src.get_file(), copy_err])
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
				DirAccess.make_dir_recursive_absolute(ref_dst.get_base_dir())
				var ref_err := DirAccess.copy_absolute(ref_src, ref_dst)
				if ref_err != OK:
					_log("[color=yellow]Could not copy external ref '%s' (error %d)[/color]" % [rel_path, ref_err])
				else:
					_log("Copied external ref: %s" % rel_path)
		_log("[color=green]Exported '%s:%s' → %s (%d mesh file(s) copied)[/color]" % [pack_id, asset_id, output_dir, copied])
		if copy_errors > 0:
			_log("[color=yellow]%d file(s) failed to copy — check paths.[/color]" % copy_errors)
		_refresh_asset_browser()
		if _lod_list.item_count == 1:
			_log("[color=yellow]Warning: only LOD0 exported — consider adding LOD1/LOD2.[/color]")
	else:
		_log("[color=red]Export failed:[/color]\n" + err)

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

# ──────────────────────────────────────────────────────────────────────────────
# UI helpers
# ──────────────────────────────────────────────────────────────────────────────

func _add_label(parent: Control, text: String, size: int) -> void:
	var lbl := Label.new()
	lbl.text = text
	lbl.add_theme_font_size_override("font_size", size)
	parent.add_child(lbl)

func _add_line_edit(parent: Control, placeholder: String, default_val: String) -> LineEdit:
	var edit := LineEdit.new()
	edit.placeholder_text = placeholder
	edit.text = default_val
	parent.add_child(edit)
	return edit

func _add_spinbox(parent: Control, label: String, min_val: float, max_val: float, default_val: float) -> SpinBox:
	var lbl := Label.new()
	lbl.text = label
	lbl.add_theme_font_size_override("font_size", _font_size_label)
	parent.add_child(lbl)
	var sb := SpinBox.new()
	sb.min_value = min_val
	sb.max_value = max_val
	sb.value = default_val
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
	# Compute pivot offset: centre XZ, ground Y (bottom face → Y=0).
	_pivot_offset = Vector3(
		-(aabb.position.x + aabb.size.x * 0.5),
		-aabb.position.y,
		-(aabb.position.z + aabb.size.z * 0.5)
	)
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
	_preview.set_preview_scale(value)
	_update_dim_label()
	# Revert preset display to "Custom" when spinner is edited directly.
	if _scale_preset_btn.selected != 0:
		_scale_preset_btn.selected = 0

func _on_scale_preset_selected(idx: int) -> void:
	if idx == 0:  # Custom — do nothing
		return
	if _mesh_aabb.size.length() < 0.001:
		_log("[color=yellow]No mesh loaded — cannot apply preset.[/color]")
		_scale_preset_btn.selected = 0
		return
	match idx:
		1: _on_autofit_pressed()           # Fit to Lot
		2: _apply_scale_fraction(0.5)      # ½ Lot
		3: _apply_scale_fraction(0.25)     # ¼ Lot

func _apply_scale_fraction(fraction: float) -> void:
	var lot_w := _width_spin.value * 10.0
	var lot_d := _depth_spin.value * 10.0
	var mesh_w := _mesh_aabb.size.x
	var mesh_d := _mesh_aabb.size.z
	if mesh_w < 0.001 or mesh_d < 0.001:
		return
	var fit_scale := minf(lot_w / mesh_w, lot_d / mesh_d)
	var scale := snappedf(fit_scale * fraction, 0.01)
	_preview_scale_spin.value = scale
	_preview.set_preview_scale(scale)
	_update_dim_label()

func _suggest_capacity() -> void:
	if _mesh_aabb.size.length() < 0.001:
		return
	var s         := _preview_scale_spin.value
	var sw        := _mesh_aabb.size.x * s
	var sd        := _mesh_aabb.size.z * s
	var sh        := _mesh_aabb.size.y * s
	# Residential roofs inflate height — discount to habitable portion before estimating floors.
	var res_h  := sh * 0.65
	var floors    := maxi(1, roundi(sh / 3.5))
	var res_floors := maxi(1, roundi(res_h / 3.5))
	var footprint := sw * sd
	var zone: String    = ZONE_TYPES[_zone_type_btn.selected]
	var density: String = DENSITY_TYPES[_density_btn.selected]
	# m² per person/worker by zone and density. Level does not affect capacity yet
	# (deferred until wealth/money system is implemented).
	var sqm_res := 30.0
	var sqm_wrk := 20.0
	match density:
		"medium": sqm_res = 20.0; sqm_wrk = 15.0
		"high":   sqm_res = 12.0; sqm_wrk = 10.0
	match zone:
		"residential":
			var lot_cells := int(_width_spin.value) * int(_depth_spin.value)
			var cap_per_floor := 4 * lot_cells  # hard cap: 4 residents per lot cell per floor
			var suggested := mini(
				maxi(1, roundi(footprint * res_floors / sqm_res)),
				cap_per_floor * res_floors)
			_residents_spin.value = suggested
			_workers_spin.value = 0
		"commercial", "office":
			_residents_spin.value = 0
			_workers_spin.value = maxi(1, roundi(footprint * floors / sqm_wrk))
		"industrial":
			_residents_spin.value = 0
			# Industrial is slightly less dense than commercial
			_workers_spin.value = maxi(1, roundi(footprint * floors / (sqm_wrk * 1.25)))
		"mixed":
			_residents_spin.value = maxi(1, roundi(footprint * floors / (sqm_res * 1.33)))
			_workers_spin.value   = maxi(1, roundi(footprint * floors / (sqm_wrk * 1.33)))

func _update_dim_label() -> void:
	if _mesh_aabb.size.length() < 0.001 or not _dim_label:
		return
	var s := _preview_scale_spin.value
	var w := snappedf(_mesh_aabb.size.x * s, 0.1)
	var d := snappedf(_mesh_aabb.size.z * s, 0.1)
	var h := snappedf(_mesh_aabb.size.y * s, 0.1)
	_dim_label.text = "→ %.1fm × %.1fm × %.1fm" % [w, d, h]

func _on_human_toggled(pressed: bool) -> void:
	_human_visible = pressed
	_preview.set_show_human(pressed)

func _on_clear_ghost_pressed() -> void:
	_preview.clear_ghost()

func _input(event: InputEvent) -> void:
	if event is InputEventMouseButton:
		var mb := event as InputEventMouseButton
		if mb.button_index != MOUSE_BUTTON_LEFT:
			return
		if mb.pressed:
			if not _is_mouse_in_3d_area():
				return
			var mouse_pos := get_viewport().get_mouse_position()
			if _try_begin_main_entrance_drag(mouse_pos):
				get_viewport().set_input_as_handled()
				return
			if _human_visible and _place_human_from_mouse(mouse_pos):
				get_viewport().set_input_as_handled()
			return
		if _dragging_main_entrance:
			_dragging_main_entrance = false
			get_viewport().set_input_as_handled()
		return

	if event is InputEventMouseMotion and _dragging_main_entrance:
		if _drag_main_entrance_from_mouse(get_viewport().get_mouse_position()):
			get_viewport().set_input_as_handled()

func _is_mouse_in_3d_area() -> bool:
	var mouse_pos := get_viewport().get_mouse_position()
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
	if _mesh_aabb.size.length() < 0.001:
		_log("[color=yellow]No mesh loaded yet — import a .glb first.[/color]")
		return
	var lot_w: float = _width_spin.value * 10.0   # CELL_M = 10
	var lot_d: float = _depth_spin.value * 10.0
	var mesh_w: float = _mesh_aabb.size.x
	var mesh_d: float = _mesh_aabb.size.z
	if mesh_w < 0.001 or mesh_d < 0.001:
		_log("[color=yellow]Mesh has zero XZ extent — cannot auto-fit.[/color]")
		return
	# Scale so the larger mesh dimension fills the corresponding lot dimension.
	var scale_x := lot_w / mesh_w
	var scale_z := lot_d / mesh_d
	var fit_scale := minf(scale_x, scale_z)
	_preview_scale_spin.value = snappedf(fit_scale, 0.01)
	_preview.set_preview_scale(fit_scale)
	_update_dim_label()
	if _scale_preset_btn:
		_scale_preset_btn.selected = 1  # Fit to Lot
	var scaled_w := snappedf(mesh_w * fit_scale, 0.1)
	var scaled_d := snappedf(mesh_d * fit_scale, 0.1)
	var scaled_h := snappedf(_mesh_aabb.size.y * fit_scale, 0.1)
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
