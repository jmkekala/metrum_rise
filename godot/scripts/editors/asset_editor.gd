# SPDX-License-Identifier: GPL-2.0-only

## Asset editor shell — launched via `--asset-editor` command-line argument.
## Shares the same SimulationNode and compiled .so as the game, but runs a
## 500 m sandbox with no agents, no demand simulation, and no background tick thread.
## Calls: sim.is_asset_editor_mode(), sim.load_all_asset_packs(dir),
## Rust owns documents/history, draft/package I/O, capabilities and validation.
## This shell coordinates preview manipulation, the pack browser and editor widgets.
extends Node3D

const PreviewGeometry = preload("res://scripts/editors/asset_editor/preview_geometry.gd")

const TopMenu = preload("res://scripts/ui/top_menu.gd")
const MeshImportDialog = preload("res://scripts/editors/mesh_import_dialog.gd")
const EditorTheme = preload("res://scripts/ui/editor_theme.gd")
const EditorView = preload("res://scripts/editors/asset_editor/editor_view.gd")
const MeshPart = preload("res://scripts/editors/asset_editor/mesh_part.gd")
const SceneLightingConfig = preload("res://scripts/core/scene_lighting.gd")
const PreviewLod = preload("res://scripts/editors/asset_editor/preview_lod.gd")
const EditorSpacing = preload("res://scripts/editors/asset_editor/editor_spacing.gd")
const AuthoringSession = preload("res://scripts/editors/asset_editor/authoring_session.gd")
const WorkspaceLayout = preload("res://scripts/editors/asset_editor/workspace_layout.gd")
const AssetSelection = preload("res://scripts/editors/asset_editor/asset_selection.gd")
const ContextMenus = preload("res://scripts/editors/asset_editor/context_menus.gd")

const PANEL_GAP := EditorSpacing.CONTROL_GAP
const PACK_MENU_CREATE_NEW := 1000000
const PACK_MENU_NO_PACKS := 1000001
const MESH_ROTATION_DRAG_DEG_PER_PX := 0.35
const SELECTION_DRAG_THRESHOLD_PX := AssetSelection.DRAG_THRESHOLD
const SITE_ANCHOR_DRAG_RADIUS_M := 1.25
const SITE_ANCHOR_DEFAULT_WIDTH_M := {
	"driveway": 3.0,
	"parking": 2.5,
	"loading_bay": 3.5,
}
const SITE_SURFACE_MATERIALS := [
	{"id": "asphalt", "label": "Asphalt"},
	{"id": "concrete", "label": "Concrete"},
]

var _view := EditorView.new()
var _layout := WorkspaceLayout.new()
var _selection := AssetSelection.new()
var _menus := ContextMenus.new()
var _session: RefCounted
var _lod_preview := PreviewLod.new()

@onready var sim: SimulationNode = $SimulationNode

# ── UI refs ───────────────────────────────────────────────────────────────────
var _lighting: Node
var _top_menu: Node

# Inspector – pack
var _known_packs: Array[Dictionary] = []

# Inspector – asset
var _parts: Array[MeshPart] = []
var _selected_part_index: int = -1
var _selected_part_indices: Array[int] = []
var _updating_mesh_part_selection: bool = false

# 3D preview node
var _preview: Node  # BuildingPreview instance
var _cam_input: Node  # EditorCameraInput instance

# Current frontage forward (updated by Set Front From View)
var _frontage_fwd: Vector3 = Vector3.FORWARD

# Registered asset IDs, refreshed after pack load.
var _asset_ids: Array[String] = []
var _asset_display_names: Dictionary = {}
var _suppress_part_transform_changed: bool = false
var _last_glb_dir: String = ""     # last directory used in GLB file dialogs
var _config: ConfigFile            # persistent editor preferences
var _log_plain_lines: Array[String] = []
var _bbcode_strip_regex: RegEx
var _site_anchors_data: Array[Dictionary] = []
var _site_surfaces_data: Array[Dictionary] = []
var _selected_site_anchor_index: int = -1
var _selected_site_anchor_indices: Array[int] = []
var _selected_site_surface_index: int = -1
var _updating_site_anchor_controls: bool = false
var _updating_site_anchor_list: bool = false
var _updating_site_surface_controls: bool = false
var _updating_site_surface_list: bool = false
var _site_anchor_drag_start_positions: Array[Vector3] = []
var _site_surface_drag_start_hit: Vector3 = Vector3.ZERO
var _site_surface_drag_start_vertices: Array = []
var _site_surface_drag_index: int = -1
var _site_surface_vertex_drag_index: int = -1
var _mesh_part_drag_start_hit: Vector3 = Vector3.ZERO
var _mesh_part_drag_start_positions: Array[Vector3] = []
var _selection_start_screen: Vector2 = Vector2.ZERO
var _selection_end_screen: Vector2 = Vector2.ZERO
var _selection_additive: bool = false
var _drag_plane_y := 0.0
var _density_types_by_zone: Dictionary = {}

# ──────────────────────────────────────────────────────────────────────────────

const CONFIG_PATH := "user://asset_editor.cfg"

func _ready() -> void:
	_session = AuthoringSession.new(self)
	_view.configure(self)
	if not sim.is_asset_editor_mode():
		push_error("AssetEditor scene loaded without --asset-editor flag")

	_config = ConfigFile.new()
	_config.load(CONFIG_PATH)  # silently no-ops if file doesn't exist yet
	_view._theme_mode = EditorTheme.normalize_mode(str(_config.get_value("ui", "theme_mode", EditorTheme.MODE_DARK)))
	_last_glb_dir = _config.get_value("import", "last_glb_dir", "")

	_view._font_size_header  = _config.get_value("ui", "font_size_header",  14)
	_view._font_size_section = _config.get_value("ui", "font_size_section", 12)
	_view._font_size_label   = _config.get_value("ui", "font_size_label",   11)
	_save_config()  # write defaults if keys are missing
	_restore_window_geometry()

	_attach_top_menu()
	_configure_preview_environment()
	_load_zone_profiles()
	_build_preview_node()
	_view._build_ui()
	_selection.configure(self)
	_lod_preview.configure(_preview, _view._preview_panel)
	_bbcode_strip_regex = RegEx.new()
	_bbcode_strip_regex.compile("\\[/?[^\\]]+\\]")
	_refresh_asset_browser()
	_session.initialize()
	_menus.configure(self)
	_lighting.pin_hour_of_day(10.5)

func _process(_delta: float) -> void:
	_lod_preview.update($CameraNode)
	if _menus.actions != null and not _menus.actions.mode.is_empty() and _cam_input._ui_has_modal_popup():
		_menus.actions.cancel()
	if _menus.actions == null or _menus.actions.mode.is_empty():
		_selection.update()

func _sync_preview_lods() -> void:
	_lod_preview.sync(_parts, _selected_part_index)
	_lod_preview.update($CameraNode)

func _on_preview_quality_changed(index: int) -> void:
	_lod_preview.set_quality(index)
	_lod_preview.update($CameraNode)

func _configure_preview_environment() -> void:
	if _lighting != null:
		return
	_lighting = SceneLightingConfig.new()
	_lighting.name = "PreviewLighting"
	add_child(_lighting)
	# Editor lighting changes only on user input, not every idle frame.
	_lighting.set_process(false)
	_lighting.pin_hour_of_day(10.5)

func _load_zone_profiles() -> void:
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
			var densities: Array = _density_types_by_zone.get(zone_type, [])
			if not densities.has(density):
				densities.append(density)
				densities.sort()
			_density_types_by_zone[zone_type] = densities

# ──────────────────────────────────────────────────────────────────────────────
# 3D preview
# ──────────────────────────────────────────────────────────────────────────────

func _build_preview_node() -> void:
	var script := load("res://scripts/renderers/building_preview.gd")
	_preview = Node3D.new()
	_preview.set_script(script)
	add_child(_preview)
	if _preview.has_method("set_theme_mode"):
		_preview.set_theme_mode(_view._theme_mode)

	var cam_script := load("res://scripts/core/editor_camera_input.gd")
	_cam_input = Node.new()
	_cam_input.set_script(cam_script)
	_cam_input.right_mouse_pan_enabled = false
	add_child(_cam_input)

# ──────────────────────────────────────────────────────────────────────────────
# UI construction
# ──────────────────────────────────────────────────────────────────────────────

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

func menu_reset_layout() -> void:
	_layout.reset()

func _notification(what: int) -> void:
	if what == NOTIFICATION_WM_CLOSE_REQUEST and _session != null:
		menu_quit()

func _exit_tree() -> void:
	# PREDELETE can run after removal from the tree, when get_window() is already null.
	_save_layout_state()

func _save_layout_state() -> void:
	_layout.save()

func _attach_top_menu() -> void:
	if has_node("TopMenu"):
		return
	var top_menu := TopMenu.new()
	top_menu.name = "TopMenu"
	top_menu.scene_kind = TopMenu.SCENE_ASSET_EDITOR
	add_child(top_menu)
	_top_menu = top_menu
	if top_menu.has_method("set_editor_theme_mode"):
		top_menu.set_editor_theme_mode(_view._theme_mode)

func get_ui_theme_mode() -> String:
	return _view._theme_mode

func menu_toggle_ui_theme() -> String:
	set_ui_theme_mode(EditorTheme.next_mode(_view._theme_mode))
	return _view._theme_mode

func set_ui_theme_mode(mode: String) -> void:
	var next_mode := EditorTheme.normalize_mode(mode)
	if _view._theme_mode == next_mode:
		return
	_view._theme_mode = next_mode
	_save_config()
	_configure_preview_environment()
	if _preview and _preview.has_method("set_theme_mode"):
		_preview.set_theme_mode(_view._theme_mode)
	if _view._theme_root:
		_view._apply_editor_theme(_view._theme_root)
	if _top_menu and _top_menu.has_method("set_editor_theme_mode"):
		_top_menu.set_editor_theme_mode(_view._theme_mode)

func menu_save() -> void:
	_session.save_draft()

func menu_load() -> void:
	_session.open_draft_dialog()

func menu_new_asset() -> void:
	_session.new_asset_dialog()

func menu_export_asset() -> void:
	if not _session.has_document:
		return
	_session.validate()
	_view.show_task("validate")
	if not _view.inspector.visible:
		_view.toggle_inspector()

func menu_quit() -> void:
	_session.guard(func(): get_tree().quit())

func menu_reload_packs() -> void:
	_refresh_asset_browser()

func menu_import_mesh() -> void:
	if _session.has_document:
		_on_import_glb_pressed()

func _refresh_asset_browser() -> void:
	var mods_path: String = ProjectSettings.globalize_path("user://mods/")
	var warnings: String = sim.load_all_asset_packs(mods_path)
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
	if not _view._pack_select_menu:
		return
	_rebuild_pack_select_menu()
	var pos := field.global_position + Vector2(0.0, field.size.y + 2.0)
	_view._pack_select_menu.position = Vector2i(int(round(pos.x)), int(round(pos.y)))
	_view._pack_select_menu.popup()

func _rebuild_pack_select_menu() -> void:
	_view._pack_select_menu.clear()
	if _known_packs.is_empty():
		_view._pack_select_menu.add_item("No installed packs", PACK_MENU_NO_PACKS)
		_view._pack_select_menu.set_item_disabled(_view._pack_select_menu.get_item_count() - 1, true)
	else:
		for i in _known_packs.size():
			var pack := _known_packs[i]
			var pack_id := str(pack.get("pack_id", "")).strip_edges()
			var display_name := str(pack.get("display_name", "")).strip_edges()
			var label := pack_id
			if not display_name.is_empty() and display_name != pack_id:
				label = "%s  (%s)" % [display_name, pack_id]
			_view._pack_select_menu.add_item(label, i)
	_view._pack_select_menu.add_separator()
	_view._pack_select_menu.add_item("Create New Pack...", PACK_MENU_CREATE_NEW)

func _on_pack_select_menu_id_pressed(id: int) -> void:
	if id == PACK_MENU_CREATE_NEW:
		_open_new_pack_dialog()
		return
	if id < 0 or id >= _known_packs.size():
		return
	_apply_pack_fields(_known_packs[id])

func _apply_pack_fields(pack: Dictionary) -> void:
	_session.choose_pack(pack)

func _update_pack_summary() -> void:
	if not _view._pack_summary_lbl:
		return
	var pack_id := str(_session.params.get("pack_id", "")).strip_edges()
	var display_name := str(_session.params.get("pack_name", "")).strip_edges()
	var author := str(_session.params.get("pack_author", "")).strip_edges()
	if display_name.is_empty():
		display_name = pack_id
	var lines := PackedStringArray()
	lines.append("%s (%s)" % [display_name, pack_id])
	lines.append("Author: %s" % (author if not author.is_empty() else "Unspecified"))
	_view._pack_summary_lbl.text = "\n".join(lines)

func _open_new_pack_dialog() -> void:
	_ensure_pack_create_window()
	var parent: Node = _session._creation if is_instance_valid(_session._creation) and _session._creation.visible else self
	if _view._pack_create_window.get_parent() != parent:
		_view._pack_create_window.reparent(parent)
	var current_id := str(_session.params.get("pack_id", "")).strip_edges()
	if current_id.is_empty() or _pack_id_exists(current_id):
		current_id = _suggest_new_pack_id()
	_view._new_pack_id_edit.text = current_id
	_view._new_pack_name_edit.text = _display_name_from_pack_id(current_id)
	_view._new_pack_author_edit.text = ""
	_view._pack_create_window.popup_centered(Vector2i(540, 340))
	_view._new_pack_id_edit.grab_focus()
	_view._new_pack_id_edit.select_all()

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
	if _view._pack_create_window and is_instance_valid(_view._pack_create_window):
		return
	_view._pack_create_window = Window.new()
	_view._pack_create_window.visible = false
	_view._pack_create_window.title = "Create Pack"
	_view._pack_create_window.transient = true
	_view._pack_create_window.exclusive = true
	_view._pack_create_window.min_size = Vector2i(480, 320)
	_view._pack_create_window.size = Vector2i(540, 340)
	_view._pack_create_window.close_requested.connect(func(): _view._pack_create_window.hide())
	add_child(_view._pack_create_window)

	var panel := PanelContainer.new()
	panel.set_anchors_preset(Control.PRESET_FULL_RECT)
	_view._pack_create_window.add_child(panel)

	var margin := _view._add_panel_margin(panel)
	var root := VBoxContainer.new()
	root.size_flags_horizontal = Control.SIZE_EXPAND_FILL
	root.size_flags_vertical = Control.SIZE_EXPAND_FILL
	root.add_theme_constant_override("separation", PANEL_GAP)
	margin.add_child(root)

	_view._add_label(root, "New Pack", _view._font_size_header)
	_view._new_pack_id_edit = _view._add_line_edit(root, "Pack ID (kebab-case)", "")
	_view._new_pack_name_edit = _view._add_line_edit(root, "Pack Name", "")
	_view._new_pack_author_edit = _view._add_line_edit(root, "Author", "")

	var hint := Label.new()
	hint.text = "Creates user://mods/<pack_id>/pack.toml now. Asset files are added on export."
	hint.autowrap_mode = TextServer.AUTOWRAP_WORD_SMART
	hint.add_theme_font_size_override("font_size", _view._font_size_label)
	root.add_child(hint)

	var footer := HBoxContainer.new()
	footer.add_theme_constant_override("separation", PANEL_GAP)
	root.add_child(footer)
	var spacer := Control.new()
	spacer.size_flags_horizontal = Control.SIZE_EXPAND_FILL
	footer.add_child(spacer)
	var cancel_btn := Button.new()
	cancel_btn.text = "Cancel"
	cancel_btn.pressed.connect(func(): _view._pack_create_window.hide())
	footer.add_child(cancel_btn)
	var create_btn := Button.new()
	create_btn.text = "Create"
	create_btn.pressed.connect(_on_create_pack_pressed)
	footer.add_child(create_btn)
	_view._apply_editor_theme(_view._pack_create_window)

func _on_create_pack_pressed() -> void:
	var pack_id := _view._new_pack_id_edit.text.strip_edges()
	var display_name := _view._new_pack_name_edit.text.strip_edges()
	var author := _view._new_pack_author_edit.text.strip_edges()
	if display_name.is_empty():
		display_name = pack_id
	var mods_path := ProjectSettings.globalize_path("user://mods/")
	var err := AssetAuthoringFiles.create_pack(mods_path, pack_id, display_name, author)
	if not err.is_empty():
		_session.message("Could not create pack: " + err)
		return

	var pack := {
		"pack_id": pack_id,
		"display_name": display_name,
		"author": author,
	}
	_refresh_known_packs(mods_path)
	_apply_pack_fields(pack)
	_view._pack_create_window.hide()
	_log("[color=green]Created pack '%s' at %s[/color]" % [pack_id, mods_path.path_join(pack_id)])

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
	if not _view._asset_tree:
		return

	var query := ""
	if _view._asset_search_edit:
		query = _view._asset_search_edit.text.strip_edges().to_lower()

	_view._asset_tree.clear()
	var root := _view._asset_tree.create_item()
	var visible_ids: Array[String] = []
	for aid in _asset_ids:
		if _asset_matches_query(aid, query):
			visible_ids.append(aid)

	if _view._asset_count_lbl:
		if query.is_empty():
			_view._asset_count_lbl.text = "%d assets" % _asset_ids.size()
		else:
			_view._asset_count_lbl.text = "%d / %d assets" % [visible_ids.size(), _asset_ids.size()]

	if visible_ids.is_empty():
		var empty_item := _view._asset_tree.create_item(root)
		empty_item.set_text(0, "No matching assets")
		empty_item.set_selectable(0, false)

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
	if query.is_empty():
		for known: Dictionary in _known_packs:
			var pack_id := str(known.pack_id)
			var item := _view._asset_tree.create_item(root)
			item.set_text(0, "%s (%d)" % [pack_id, int(pack_counts.get(pack_id, 0))])
			item.set_selectable(0, false)
			item.set_metadata(0, {"pack": pack_id})
			pack_items[pack_id] = item
	for aid in visible_ids:
		var pack := _asset_pack_id(aid)
		var category := _asset_category_id(aid)
		var pack_item: TreeItem = pack_items.get(pack, null)
		if pack_item == null:
			pack_item = _view._asset_tree.create_item(root)
			pack_item.set_text(0, "%s (%d)" % [pack, int(pack_counts.get(pack, 0))])
			pack_item.set_selectable(0, false)
			pack_item.set_metadata(0, {"pack": pack})
			pack_item.set_collapsed(false)
			pack_items[pack] = pack_item

		var category_key := "%s\n%s" % [pack, category]
		var category_item: TreeItem = category_items.get(category_key, null)
		if category_item == null:
			category_item = _view._asset_tree.create_item(pack_item)
			category_item.set_text(
				0,
				"%s (%d)" % [category, int(category_counts.get(category_key, 0))]
			)
			category_item.set_selectable(0, false)
			category_item.set_metadata(0, {"pack": pack, "type": category.get_slice(" / ", 1)})
			category_item.set_collapsed(false)
			category_items[category_key] = category_item

		var asset_item := _view._asset_tree.create_item(category_item)
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

# Preview choices never mutate placement bounds, source geometry or material files.
func _frame_selected_part() -> void:
	if _menus.actions != null:
		_menus.actions.frame(_menus.actions.selection())

func _on_preview_lod_selected(index: int) -> void:
	if not _has_selected_mesh_part():
		return
	_lod_preview.set_mode(index, $CameraNode)
	_lod_preview.update($CameraNode)

func _on_add_part_lod_requested() -> void:
	if not _has_selected_mesh_part():
		return
	var part := _parts[_selected_part_index]
	var revision := _menus.generation
	var dialog := MeshImportDialog.new()
	dialog.theme_mode = _view._theme_mode
	dialog.mesh_selected.connect(func(path: String):
		if revision != _menus.generation or not _parts.has(part):
			return
		part.append_lod(path)
		_sync_preview_lods()
		_session.capture_geometry("Add LOD")
		_last_glb_dir = path.get_base_dir()
		_save_config()
		if _has_selected_mesh_part() and _parts[_selected_part_index] == part:
			_on_preview_lod_selected(part.lods.size() - 1)
	)
	add_child(dialog)
	dialog.title = "Add LOD%d" % part.lods.size()
	dialog.open(_last_glb_dir)

func _on_asset_tree_activated() -> void:
	_load_selected_asset_from_tree()

func _on_asset_tree_gui_input(event: InputEvent) -> void:
	if event is InputEventKey and event.pressed and not event.echo and event.keycode == KEY_DELETE:
		var item := _view._asset_tree.get_selected()
		if item != null and item.get_metadata(0) is String:
			_menus.library.execute("library_trash", {"id": item.get_metadata(0)})
			_view._asset_tree.accept_event()
		return
	if not (event is InputEventMouseButton):
		return
	var mb := event as InputEventMouseButton
	if mb.button_index != MOUSE_BUTTON_RIGHT or not mb.pressed:
		return
	_menus.open_library(mb.position)
	_view._asset_tree.accept_event()

func _load_selected_asset_from_tree() -> void:
	if not _view._asset_tree:
		return
	var item := _view._asset_tree.get_selected()
	if item == null:
		return
	var metadata = item.get_metadata(0)
	if not metadata is String:
		return
	var aid := str(metadata).strip_edges()
	if aid.is_empty():
		return
	_load_asset_manifest(aid)

func _load_asset_manifest(aid: String) -> void:
	var json_str: String = sim.get_asset_manifest_json(aid)
	var data = JSON.parse_string(json_str)
	if data is Dictionary:
		_session.guard(func(): _session.load_manifest(data))

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
		_view.update_context_actions()
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

func _array_to_vector3(value, fallback: Vector3) -> Vector3:
	return PreviewGeometry.vector3(value, fallback)

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

func _on_import_glb_pressed() -> void:
	if str(_session.params.get("asset_id", "")).is_empty():
		_session.new_asset_dialog()
		return
	var generation: int = _menus.generation
	_menus.actions.import_at(Vector3.ZERO, func(): return generation == _menus.generation)

func _on_glb_file_selected(path: String) -> void:
	_menus.actions.begin_placement("create", [], {"kind": "mesh", "path": path}, Vector3.ZERO)
	_menus.actions.confirm()

# ──────────────────────────────────────────────────────────────────────────────
# Mesh part management
# ──────────────────────────────────────────────────────────────────────────────


func _select_mesh_part(index: int, clear_site_anchors: bool = true) -> void:
	if index < 0 or index >= _parts.size():
		_set_selected_mesh_parts([], -1)
		if clear_site_anchors:
			_set_selected_site_anchors([], -1)
		_set_selected_site_surface(-1)
		return
	_set_selected_mesh_parts([index], index)
	if clear_site_anchors:
		_set_selected_site_anchors([], -1)
	_set_selected_site_surface(-1)

func _on_mesh_part_multi_selected(index: int, _selected: bool) -> void:
	if _updating_mesh_part_selection:
		return
	_set_selected_mesh_parts(_view._mesh_part_list.get_selected_items(), index)
	if not Input.is_key_pressed(KEY_CTRL):
		_set_selected_site_anchors([], -1)
		_set_selected_site_surface(-1)

func _set_selected_mesh_parts(indices: Array, primary_index: int = -1) -> void:
	var seen := {}
	var resolved: Array[int] = []
	for raw_index in indices:
		var index := int(raw_index)
		if index < 0 or index >= _parts.size() or seen.has(index):
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
	if _view._mesh_part_list:
		_view._mesh_part_list.deselect_all()
		for index in _selected_part_indices:
			if index >= 0 and index < _view._mesh_part_list.item_count:
				_view._mesh_part_list.select(index, false)
	_updating_mesh_part_selection = false
	if _preview and _preview.has_method("set_selected_mesh_parts"):
		_preview.set_selected_mesh_parts(_selected_part_indices, _selected_part_index)
	_refresh_selected_part_controls()
	_update_dim_label()
	_sync_preview_lods()
	if _session != null:
		_session.selection_changed()

func _site_surface_material_label(material: String) -> String:
	match material:
		"asphalt":
			return "Asphalt"
		"concrete":
			return "Concrete"
		_:
			return material.capitalize()

func _refresh_site_surface_list() -> void:
	if not _view._site_surface_list:
		return
	_updating_site_surface_list = true
	_view._site_surface_list.clear()
	var material_counts := {}
	for i in _site_surfaces_data.size():
		var surface := _site_surfaces_data[i]
		var material := str(surface.get("material", "")).strip_edges()
		material_counts[material] = int(material_counts.get(material, 0)) + 1
		_view._site_surface_list.add_item(_site_surface_display_label(i, material_counts[material]))
	if _selected_site_surface_index >= 0 and _selected_site_surface_index < _view._site_surface_list.item_count:
		_view._site_surface_list.select(_selected_site_surface_index, false)
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

func _set_selected_site_surface(index: int, clear_others: bool = true) -> void:
	_selected_site_surface_index = index if index >= 0 and index < _site_surfaces_data.size() else -1
	_refresh_site_surface_list()
	if _selected_site_surface_index >= 0 and clear_others:
		_set_selected_mesh_parts([], -1)
		_set_selected_site_anchors([], -1)

func _on_site_surface_selected(index: int) -> void:
	if _updating_site_surface_list:
		return
	_set_selected_site_surface(index)

func _update_site_surface_controls() -> void:
	_updating_site_surface_controls = true
	var has_surface := _selected_site_surface_index >= 0 and _selected_site_surface_index < _site_surfaces_data.size()
	var surface := _site_surfaces_data[_selected_site_surface_index] if has_surface else {}
	_view._site_surface_name_edit.text = str(surface.get("name", "")) if has_surface else ""
	_set_option_by_metadata(_view._site_surface_material_btn, str(surface.get("material", "asphalt")))
	_view._site_surface_y_spin.value = _anchor_number(surface, "y_m", 0.01) if has_surface else 0.0
	_view._site_surface_name_edit.editable = has_surface
	_view._site_surface_material_btn.disabled = not has_surface
	_view._site_surface_y_spin.editable = has_surface
	_updating_site_surface_controls = false
	if _session != null:
		_session.selection_changed()

func _on_site_surface_text_changed(_value: String) -> void:
	if _updating_site_surface_controls:
		return
	_apply_site_surface_controls("name")

func _on_site_surface_material_selected(_index: int) -> void:
	if _updating_site_surface_controls:
		return
	_apply_site_surface_controls("material")

func _on_site_surface_spin_changed(_value: float) -> void:
	if _updating_site_surface_controls:
		return
	_apply_site_surface_controls("y_m")

func _apply_site_surface_controls(field: String) -> void:
	if _selected_site_surface_index < 0 or _selected_site_surface_index >= _site_surfaces_data.size():
		return
	var surface := _site_surfaces_data[_selected_site_surface_index]
	match field:
		"name": surface["name"] = _view._site_surface_name_edit.text.strip_edges()
		"material": surface["material"] = _selected_option_metadata(_view._site_surface_material_btn, "asphalt")
		"y_m": surface["y_m"] = float(_view._site_surface_y_spin.value)
	_refresh_site_surface_list()

func _update_site_surface_preview() -> void:
	if _preview and _preview.has_method("set_site_surfaces"):
		_preview.set_site_surfaces(_site_surfaces_data, _selected_site_surface_index)

func _site_surface_vertices(surface: Dictionary) -> Array[Vector2]:
	return PreviewGeometry.surface_vertices(surface)

func _site_surface_vertices_to_arrays(vertices: Array[Vector2]) -> Array:
	var result := []
	for vertex in vertices:
		result.append([snappedf(vertex.x, 0.01), snappedf(vertex.y, 0.01)])
	return result

func _site_surface_polygon_is_valid(vertices: Array[Vector2]) -> bool:
	return _session.policy.is_valid_site_polygon(PackedVector2Array(vertices))

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

func _refresh_site_anchor_list() -> void:
	if not _view._site_anchor_list:
		return
	_updating_site_anchor_list = true
	_view._site_anchor_list.clear()
	var type_counts := {}
	for i in _site_anchors_data.size():
		var anchor := _site_anchors_data[i]
		var anchor_type := str(anchor.get("anchor_type", "")).strip_edges()
		type_counts[anchor_type] = int(type_counts.get(anchor_type, 0)) + 1
		_view._site_anchor_list.add_item(_site_anchor_display_label(i, type_counts[anchor_type]))
	for index in _selected_site_anchor_indices:
		if index >= 0 and index < _view._site_anchor_list.item_count:
			_view._site_anchor_list.select(index, false)
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

func _on_site_anchor_multi_selected(index: int, _selected: bool) -> void:
	if _updating_site_anchor_list:
		return
	var selected := _view._site_anchor_list.get_selected_items()
	_set_selected_site_anchors(selected, index)
	if not Input.is_key_pressed(KEY_CTRL):
		_set_selected_mesh_parts([], -1)
		_set_selected_site_surface(-1)

func _update_site_anchor_controls() -> void:
	_updating_site_anchor_controls = true
	var has_anchor := _selected_site_anchor_index >= 0 and _selected_site_anchor_index < _site_anchors_data.size()
	var anchor := _site_anchors_data[_selected_site_anchor_index] if has_anchor else {}
	var anchor_type := str(anchor.get("anchor_type", "")).strip_edges()
	var is_entrance := has_anchor and _is_main_entrance_anchor(anchor)
	var is_driveway := has_anchor and anchor_type == "driveway"
	var has_size := has_anchor and anchor_type != "entrance"
	var has_length := has_anchor and (anchor_type == "parking" or anchor_type == "loading_bay")
	var pos := _anchor_position(anchor)
	var yaw := _yaw_from_forward(_anchor_forward(anchor))
	_view._site_anchor_name_edit.text = str(anchor.get("name", "")) if has_anchor else ""
	_set_option_by_metadata(_view._site_anchor_vehicle_class_btn, _anchor_text(anchor, "vehicle_class", "car"))
	_view._site_anchor_x_spin.value = pos.x
	_view._site_anchor_y_spin.value = pos.y
	_view._site_anchor_z_spin.value = pos.z
	_view._site_anchor_yaw_spin.value = yaw
	_view._site_anchor_width_spin.value = _anchor_number(anchor, "width_m", 3.0) if has_size else 0.0
	_view._site_anchor_length_spin.value = _anchor_number(anchor, "length_m", 0.0) if has_length else 0.0
	_view._site_anchor_name_edit.editable = has_anchor and not is_entrance
	_view._site_anchor_vehicle_class_btn.disabled = not has_size
	var frontage_outward := _frontage_edge_outward()
	_view._site_anchor_x_spin.editable = has_anchor and not (is_driveway and absf(frontage_outward.x) > 0.5)
	_view._site_anchor_y_spin.editable = has_anchor
	_view._site_anchor_z_spin.editable = has_anchor and not (is_driveway and absf(frontage_outward.z) > 0.5)
	_view._site_anchor_yaw_spin.editable = has_anchor and not is_driveway
	_view._site_anchor_width_spin.editable = has_size
	_view._site_anchor_length_spin.editable = has_length
	_updating_site_anchor_controls = false
	if _session != null:
		_session.selection_changed()

func _on_site_anchor_text_changed(_value: String) -> void:
	if _updating_site_anchor_controls:
		return
	_apply_site_anchor_controls("name")

func _on_site_anchor_vehicle_class_selected(_index: int) -> void:
	if _updating_site_anchor_controls:
		return
	_apply_site_anchor_controls("vehicle_class")

func _on_site_anchor_spin_changed(_value: float, field: String) -> void:
	if _updating_site_anchor_controls:
		return
	_apply_site_anchor_controls(field)

func _apply_site_anchor_controls(field: String) -> void:
	if _selected_site_anchor_index < 0 or _selected_site_anchor_index >= _site_anchors_data.size():
		return
	var anchor := _site_anchors_data[_selected_site_anchor_index]
	match field:
		"name": anchor["name"] = _view._site_anchor_name_edit.text.strip_edges()
		"vehicle_class": anchor["vehicle_class"] = _selected_option_metadata(_view._site_anchor_vehicle_class_btn, "car")
		"width_m": anchor["width_m"] = float(_view._site_anchor_width_spin.value)
		"length_m": anchor["length_m"] = float(_view._site_anchor_length_spin.value)
		"yaw": _set_site_anchor_yaw(_selected_site_anchor_index, _view._site_anchor_yaw_spin.value)
		"x", "y", "z":
			var pos := _anchor_position(anchor)
			pos[field] = float(_view.fields["_anchor_" + field].value)
			pos = _clamp_site_anchor_position_to_lot(anchor, pos)
			anchor["position"] = [pos.x, pos.y, pos.z]
	_refresh_site_anchor_list()

func _update_site_anchor_preview() -> void:
	if _preview and _preview.has_method("set_site_anchors"):
		_preview.set_site_anchors(
			_site_anchors_data,
			_selected_site_anchor_indices,
			_selected_site_anchor_index
		)

func _anchor_position(anchor: Dictionary) -> Vector3:
	return PreviewGeometry.vector3(anchor.get("position"))

func _anchor_forward(anchor: Dictionary) -> Vector3:
	return PreviewGeometry.forward(anchor)

func _anchor_number(anchor: Dictionary, key: String, fallback: float) -> float:
	return PreviewGeometry.number(anchor, key, fallback)

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
	var anchor_type := str(_site_anchors_data[index].get("anchor_type", "")).strip_edges()
	var resolved_forward := _forward_from_yaw(_snap_rotation_y_to_cardinal_if_close(yaw_degrees))
	if anchor_type == "driveway":
		resolved_forward = _driveway_anchor_forward()
	_site_anchors_data[index]["forward"] = _vector3_to_array(
		resolved_forward,
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

func _refresh_selected_part_controls() -> void:
	_suppress_part_transform_changed = true
	var has_part := _selected_part_index >= 0 and _selected_part_index < _parts.size()
	var pos := _parts[_selected_part_index].position if has_part else Vector3.ZERO
	_view._part_x_spin.value = pos.x
	_view._part_y_spin.value = pos.y
	_view._part_z_spin.value = pos.z
	_view._part_rotation_y_spin.value = _parts[_selected_part_index].rotation_y if has_part else 0.0
	_view._preview_scale_spin.value = _parts[_selected_part_index].scale if has_part else 1.0
	_suppress_part_transform_changed = false

func _on_part_transform_changed(_value: float, field: String) -> void:
	if _suppress_part_transform_changed:
		return
	_apply_selected_part_transform(field)

func _apply_selected_part_transform(field: String = "scale") -> void:
	if _selected_part_index < 0 or _selected_part_index >= _parts.size():
		return
	var pos: Vector3 = _parts[_selected_part_index].position
	var rot_y: float = _parts[_selected_part_index].rotation_y
	var scale: float = _parts[_selected_part_index].scale
	match field:
		"x", "y", "z": pos[field] = float(_view.fields["_part_" + field].value)
		"yaw": rot_y = float(_view._part_rotation_y_spin.value)
		"scale": scale = maxf(0.001, float(_view._preview_scale_spin.value))
	_parts[_selected_part_index].position = pos
	_parts[_selected_part_index].rotation_y = rot_y
	_parts[_selected_part_index].scale = scale
	_parts[_selected_part_index].position = _clamp_mesh_part_position_to_lot(
		_selected_part_index,
		_parts[_selected_part_index].position
	)
	_sync_selected_mesh_part_controls()
	_preview.set_mesh_part_transform(
		_selected_part_index,
		_parts[_selected_part_index].position,
		rot_y,
		scale,
		_parts[_selected_part_index].pivot_offset
	)
	_update_dim_label()

func _clear_mesh_parts() -> void:
	_parts.clear()
	_view._preview_panel.set_part(null)
	_selected_part_index = -1
	_selected_part_indices.clear()
	if _view._mesh_part_list:
		_view._mesh_part_list.clear()
	if _preview and _preview.has_method("clear_mesh_parts"):
		_preview.clear_mesh_parts()
	_sync_preview_lods()
	_refresh_selected_part_controls()

func _selected_part_aabb() -> AABB:
	if _selected_part_index < 0 or _selected_part_index >= _parts.size():
		return AABB()
	return _parts[_selected_part_index].aabb

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
	_log("Frontage set: front face points toward camera; entrance unchanged.")

# ──────────────────────────────────────────────────────────────────────────────
# Export
# ──────────────────────────────────────────────────────────────────────────────

func _on_export_pressed() -> void:
	if _export_needs_pack_retarget_choice():
		_open_export_retarget_dialog()
		return
	_export_asset(false)

func _export_needs_pack_retarget_choice() -> bool:
	var origin: Dictionary = _session.document.snapshot().get("origin", {})
	return (
		not str(origin.get("pack_id", "")).is_empty()
		and not str(origin.get("asset_id", "")).is_empty()
		and str(_session.params.get("pack_id", "")) != origin["pack_id"]
	)

func _open_export_retarget_dialog() -> void:
	_ensure_export_retarget_window()
	var origin: Dictionary = _session.document.snapshot()["origin"]
	var target_pack := str(_session.params.get("pack_id", "")).strip_edges()
	var target_asset := _view._asset_id_edit.text.strip_edges()
	if _view._retarget_export_message_lbl:
		_view._retarget_export_message_lbl.text = (
			"This asset was loaded from:\n%s:%s\n\nExporting now targets:\n%s:%s\n\n"
			+ "Copy creates/updates the target asset and leaves the original untouched.\n"
			+ "Move creates/updates the target asset, then deletes the original asset folder after export succeeds."
		) % [origin["pack_id"], origin["asset_id"], target_pack, target_asset]
	_view._retarget_export_window.popup_centered(Vector2i(560, 310))

func _ensure_export_retarget_window() -> void:
	if _view._retarget_export_window and is_instance_valid(_view._retarget_export_window):
		return
	_view._retarget_export_window = Window.new()
	_view._retarget_export_window.title = "Export To Different Pack"
	_view._retarget_export_window.min_size = Vector2i(520, 280)
	_view._retarget_export_window.size = Vector2i(560, 310)
	_view._retarget_export_window.close_requested.connect(func(): _view._retarget_export_window.hide())
	add_child(_view._retarget_export_window)

	var panel := PanelContainer.new()
	panel.set_anchors_preset(Control.PRESET_FULL_RECT)
	_view._retarget_export_window.add_child(panel)

	var margin := _view._add_panel_margin(panel)
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
	message.add_theme_font_size_override("font_size", _view._font_size_label)
	root.add_child(message)
	_view._retarget_export_message_lbl = message

	var footer := HBoxContainer.new()
	footer.add_theme_constant_override("separation", PANEL_GAP)
	root.add_child(footer)
	var spacer := Control.new()
	spacer.size_flags_horizontal = Control.SIZE_EXPAND_FILL
	footer.add_child(spacer)
	var cancel_btn := Button.new()
	cancel_btn.text = "Cancel"
	cancel_btn.pressed.connect(func(): _view._retarget_export_window.hide())
	footer.add_child(cancel_btn)
	var copy_btn := Button.new()
	copy_btn.text = "Copy"
	copy_btn.pressed.connect(func():
		_view._retarget_export_window.hide()
		_export_asset(false)
	)
	footer.add_child(copy_btn)
	var move_btn := Button.new()
	move_btn.text = "Move"
	move_btn.pressed.connect(func():
		_view._retarget_export_window.hide()
		_export_asset(true)
	)
	footer.add_child(move_btn)
	_view._apply_editor_theme(_view._retarget_export_window)

func _export_asset(move_original_after_export: bool) -> void:
	_session.publish(move_original_after_export)

func _part_name_for_index(index: int) -> String:
	return _parts[index].name if index >= 0 and index < _parts.size() else "part_%d" % [index + 1]

func _move_original_asset_after_export(
	source_pack_id: String,
	source_asset_id: String,
	target_pack_id: String,
	target_asset_id: String
) -> void:
	if source_pack_id.is_empty() or source_asset_id.is_empty():
		_log("[color=yellow]Move skipped: this editor state has no original asset path.[/color]")
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
	var err := AssetAuthoringFiles.remove_asset("user://mods", source_pack_id, source_asset_id)
	if not err.is_empty():
		_log("[color=red]Move exported the target, but failed to delete original asset folder '%s': %s.[/color]" % [source_dir, err])
		return
	_log("[color=green]Moved asset from '%s:%s' to '%s:%s'.[/color]" % [
		source_pack_id,
		source_asset_id,
		target_pack_id,
		target_asset_id,
	])

# Persist local editor preferences, never asset preview lighting/material overrides.
func _save_config() -> void:
	# Preserve layout preferences written by the mesh picker since editor startup.
	_config.load(CONFIG_PATH)
	_config.set_value("import", "last_glb_dir",    _last_glb_dir)
	_config.set_value("ui",     "theme_mode",      _view._theme_mode)
	_config.set_value("ui",     "font_size_header",  _view._font_size_header)
	_config.set_value("ui",     "font_size_section", _view._font_size_section)
	_config.set_value("ui",     "font_size_label",   _view._font_size_label)
	var err := _config.save(CONFIG_PATH)
	if err != OK:
		push_warning("AssetEditor: could not save config to %s (error %d)" % [CONFIG_PATH, err])

# ──────────────────────────────────────────────────────────────────────────────
# UI helpers
# ──────────────────────────────────────────────────────────────────────────────

func _set_frontage_forward(fwd: Vector3) -> void:
	var resolved := fwd
	if resolved.length_squared() < 0.001:
		resolved = Vector3.FORWARD
	_frontage_fwd = resolved.normalized()
	_view._frontage_lbl.text = "Forward: (%.2f, 0, %.2f)" % [_frontage_fwd.x, _frontage_fwd.z]
	_preview.set_frontage_forward(_frontage_fwd)
	_clamp_site_anchors_to_lot()

func _snap_xz_to_cardinal(dir: Vector3) -> Vector3:
	if absf(dir.x) >= absf(dir.z):
		return Vector3(1.0 if dir.x >= 0.0 else -1.0, 0.0, 0.0)
	return Vector3(0.0, 0.0, 1.0 if dir.z >= 0.0 else -1.0)

func _on_reset_main_entrance_pressed() -> void:
	_set_main_entrance_forward(_frontage_fwd)
	_set_site_anchor_position(_ensure_main_entrance_anchor(), _default_main_entrance_position())
	_log("Main entrance reset to the current frontage edge.")

func _default_main_entrance_position() -> Vector3:
	var lot_half_w := _view._width_spin.value * 10.0 * 0.5
	var lot_half_d := _view._depth_spin.value * 10.0 * 0.5
	var fwd := _frontage_edge_outward()
	if absf(fwd.x) >= absf(fwd.z):
		return Vector3((1.0 if fwd.x >= 0.0 else -1.0) * lot_half_w, 0.0, 0.0)
	return Vector3(0.0, 0.0, (1.0 if fwd.z >= 0.0 else -1.0) * lot_half_d)

func _frontage_edge_outward() -> Vector3:
	var fwd := _frontage_fwd
	var flat := Vector3(fwd.x, 0.0, fwd.z)
	if flat.length_squared() < 0.001:
		flat = Vector3.FORWARD
	return _snap_xz_to_cardinal(flat.normalized())

func _driveway_anchor_forward() -> Vector3:
	return -_frontage_edge_outward()

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

func _lot_half_extents() -> Vector2:
	return Vector2(PreviewGeometry.number(_session.params, "lot_width_cells", 2), PreviewGeometry.number(_session.params, "lot_depth_cells", 2)) * 5.0

func _clamp_anchor_position_to_lot(pos: Vector3) -> Vector3:
	var point := Vector2(pos.x, pos.z).clamp(-_lot_half_extents(), _lot_half_extents())
	return Vector3(point.x, pos.y, point.y)

func _clamp_site_anchor_position_to_lot(anchor: Dictionary, pos: Vector3) -> Vector3:
	if anchor.get("anchor_type") == "driveway":
		return _clamp_driveway_anchor_position_to_frontage(anchor, pos)
	var offsets := _site_anchor_footprint_offsets(anchor)
	if offsets.is_empty():
		return _clamp_anchor_position_to_lot(pos)
	var bounds := Rect2(Vector2(offsets[0].x, offsets[0].z), Vector2.ZERO)
	for offset: Vector3 in offsets:
		bounds = bounds.expand(Vector2(offset.x, offset.z))
	var point: Vector2 = _session.policy.clamp_lot_translation(bounds, _lot_half_extents(), Vector2(pos.x, pos.z))
	return Vector3(point.x, pos.y, point.y)

func _clamp_driveway_anchor_position_to_frontage(anchor: Dictionary, pos: Vector3) -> Vector3:
	var lot_half_w := float(_view._width_spin.value) * 10.0 * 0.5
	var lot_half_d := float(_view._depth_spin.value) * 10.0 * 0.5
	var outward := _frontage_edge_outward()
	var side := Vector3(-outward.z, 0.0, outward.x)
	var edge_distance := lot_half_w if absf(outward.x) > 0.5 else lot_half_d
	var side_extent := lot_half_d if absf(outward.x) > 0.5 else lot_half_w
	var half_width := maxf(0.1, _anchor_number(anchor, "width_m", SITE_ANCHOR_DRAG_RADIUS_M)) * 0.5
	var side_coord: float = _session.policy.clamp_interval(
		pos.dot(side),
		-side_extent + half_width,
		side_extent - half_width
	)
	var edge_coord := edge_distance if outward.dot(outward) > 0.0 else 0.0
	var clamped := outward * edge_coord + side * side_coord
	return Vector3(clamped.x, pos.y, clamped.z)

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
	return vertex.clamp(-_lot_half_extents(), _lot_half_extents())

func _clamp_site_surface_delta_to_lot(vertices: Array, delta: Vector2) -> Vector2:
	if vertices.is_empty():
		return delta
	var bounds := Rect2(vertices[0], Vector2.ZERO)
	for vertex: Vector2 in vertices:
		bounds = bounds.expand(vertex)
	return _session.policy.clamp_lot_translation(bounds, _lot_half_extents(), delta)

func _clamp_site_anchors_to_lot() -> void:
	var changed := false
	for i in _site_anchors_data.size():
		var anchor := _site_anchors_data[i]
		if str(anchor.get("anchor_type", "")).strip_edges() == "driveway":
			var inward := _driveway_anchor_forward()
			if _anchor_forward(anchor).distance_squared_to(inward) > 0.0001:
				anchor["forward"] = _vector3_to_array(inward, 0.001)
				changed = true
		var pos := _anchor_position(anchor)
		var clamped := _clamp_site_anchor_position_to_lot(anchor, pos)
		if clamped.distance_squared_to(pos) > 0.0001:
			anchor["position"] = _vector3_to_array(clamped, 0.01)
			changed = true
	if changed:
		_refresh_site_anchor_list()
	else:
		_update_site_anchor_preview()

func _clamp_mesh_part_position_to_lot(part_index: int, pos: Vector3) -> Vector3:
	var bounds := _mesh_part_footprint_bounds(part_index, Vector3.ZERO)
	var point: Vector2 = _session.policy.clamp_lot_translation(bounds, _lot_half_extents(), Vector2(pos.x, pos.z))
	return Vector3(point.x, pos.y, point.y)

func _mesh_parts_lot_fit_error() -> String:
	for i in _parts.size():
		if not _mesh_part_fits_in_lot(i):
			return (
				"Mesh part '%s' footprint crosses the lot/plot bounds. "
				+ "Move, rotate, scale it down, or enlarge the lot before exporting."
			) % _part_name_for_index(i)
	return ""

func _mesh_part_fits_in_lot(part_index: int) -> bool:
	var bounds := _mesh_part_footprint_bounds(part_index, _parts[part_index].position)
	var half := _lot_half_extents()
	return not bounds.has_area() or Rect2(-half, half * 2.0).grow(0.01).encloses(bounds)

func _mesh_part_footprint_bounds(part_index: int, pos: Vector3) -> Rect2:
	if part_index < 0 or part_index >= _parts.size():
		return Rect2()
	var part := _parts[part_index]
	if part.aabb.size.x < 0.001 or part.aabb.size.z < 0.001:
		return Rect2()
	# Native AABB transformation replaces eight script-side corner allocations.
	var basis := Basis(Vector3.UP, deg_to_rad(part.rotation_y)).scaled(Vector3.ONE * maxf(0.001, part.scale))
	var bounds: AABB = Transform3D(basis, pos + basis * part.pivot_offset) * part.aabb
	return Rect2(Vector2(bounds.position.x, bounds.position.z), Vector2(bounds.size.x, bounds.size.z))

func _update_dim_label() -> void:
	var aabb := _selected_part_aabb()
	if aabb.size.length() < 0.001 or not _view._dim_label:
		return
	var s := _view._preview_scale_spin.value
	var w := snappedf(aabb.size.x * s, 0.1)
	var d := snappedf(aabb.size.z * s, 0.1)
	var h := snappedf(aabb.size.y * s, 0.1)
	_view._dim_label.text = "→ %.1fm × %.1fm × %.1fm" % [w, d, h]

func _on_clear_ghost_pressed() -> void:
	_preview.clear_ghost()
	_view.update_context_actions()

func _input(event: InputEvent) -> void:
	if _menus.actions != null and (_menus.actions.handle_input(event) or _menus.handle_right(event)):
		get_viewport().set_input_as_handled()
		return
	if _selection.handle_input(event):
		get_viewport().set_input_as_handled()

func _ui_captures_editor_text_input() -> bool:
	var focus_owner := get_viewport().gui_get_focus_owner()
	return (
		focus_owner is LineEdit
		or focus_owner is TextEdit
		or focus_owner is CodeEdit
		or focus_owner is SpinBox
	)

func _drag_site_surface_from_mouse(mouse_pos: Vector2) -> bool:
	if _site_surface_drag_index < 0 or _site_surface_drag_index >= _site_surfaces_data.size():
		return false
	if _site_surface_drag_start_vertices.is_empty():
		return false
	var hit = _project_mouse_to_horizontal_plane(mouse_pos, _drag_plane_y)
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
	var hit = _project_mouse_to_horizontal_plane(mouse_pos, _drag_plane_y)
	if hit == null:
		return false
	var vertices: Array[Vector2] = []
	for raw_vertex in _site_surface_drag_start_vertices:
		if raw_vertex is Vector2:
			vertices.append(raw_vertex as Vector2)
	if _site_surface_vertex_drag_index >= vertices.size():
		return false
	var delta := Vector2(hit.x - _site_surface_drag_start_hit.x, hit.z - _site_surface_drag_start_hit.z)
	vertices[_site_surface_vertex_drag_index] = _clamp_site_surface_vertex_to_lot(vertices[_site_surface_vertex_drag_index] + delta)
	if not _site_surface_polygon_is_valid(vertices):
		return false
	_site_surfaces_data[_site_surface_drag_index]["vertices"] = _site_surface_vertices_to_arrays(vertices)
	_update_site_surface_controls()
	_update_site_surface_preview()
	return true

func _site_surface_contains_world_xz(index: int, world_pos: Vector3) -> bool:
	if index < 0 or index >= _site_surfaces_data.size():
		return false
	return Geometry2D.is_point_in_polygon(Vector2(world_pos.x, world_pos.z), PackedVector2Array(_site_surface_vertices(_site_surfaces_data[index])))

func _drag_mesh_part_from_mouse(mouse_pos: Vector2) -> bool:
	var selected_indices := _mesh_part_drag_indices()
	var anchor_indices := _site_anchor_drag_indices()
	if selected_indices.is_empty() and anchor_indices.is_empty():
		return false
	var hit = _project_mouse_to_horizontal_plane(mouse_pos, _drag_plane_y)
	if hit == null:
		return false
	var delta := Vector3(hit.x, 0.0, hit.z) - _mesh_part_drag_start_hit
	for i in selected_indices.size():
		var index := selected_indices[i]
		if index < 0 or index >= _parts.size() or i >= _mesh_part_drag_start_positions.size():
			continue
		_parts[index].position = _clamp_mesh_part_position_to_lot(
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
	_selection_additive = additive
	_selection_start_screen = mouse_pos
	_selection_end_screen = mouse_pos
	if _view._selection_rect_overlay and _view._selection_rect_overlay.has_method("clear"):
		_view._selection_rect_overlay.clear()

func _update_mesh_part_box_selection(mouse_pos: Vector2) -> void:
	_selection_end_screen = mouse_pos
	if _selection_start_screen.distance_to(_selection_end_screen) < SELECTION_DRAG_THRESHOLD_PX:
		if _view._selection_rect_overlay and _view._selection_rect_overlay.has_method("clear"):
			_view._selection_rect_overlay.clear()
		return
	if _view._selection_rect_overlay and _view._selection_rect_overlay.has_method("set_rect_global"):
		_view._selection_rect_overlay.set_rect_global(_selection_start_screen, _selection_end_screen, true)

func _finish_mesh_part_box_selection(mouse_pos: Vector2) -> void:
	_selection_end_screen = mouse_pos
	if _view._selection_rect_overlay and _view._selection_rect_overlay.has_method("clear"):
		_view._selection_rect_overlay.clear()
	if _selection_start_screen.distance_to(_selection_end_screen) < SELECTION_DRAG_THRESHOLD_PX:
		if not _selection_additive:
			_set_selected_site_anchors([], -1)
			_set_selected_mesh_parts([], -1)
			_set_selected_site_surface(-1)
		_selection_additive = false
		return
	var selection_rect := _selection_screen_rect()
	var filter: String = _selection.effective_filter()
	var selected_meshes: Array[int] = []
	var selected_anchors: Array[int] = []
	var selected_surfaces: Array[int] = []
	if filter in ["all", "mesh"]:
		selected_meshes = _mesh_parts_in_screen_rect(selection_rect)
	if filter in ["all", "anchor"]:
		selected_anchors = _site_anchors_in_screen_rect(selection_rect)
	if filter in ["all", "surface"]:
		selected_surfaces = _site_surfaces_in_screen_rect(selection_rect)
	if _selection_additive:
		selected_meshes = _merged_indices(_selected_part_indices, selected_meshes)
		selected_anchors = _merged_indices(_selected_site_anchor_indices, selected_anchors)
	_set_selected_mesh_parts(selected_meshes, int(selected_meshes[0]) if not selected_meshes.is_empty() else -1)
	_set_selected_site_anchors(selected_anchors, int(selected_anchors[0]) if not selected_anchors.is_empty() else -1)
	if not selected_surfaces.is_empty():
		_set_selected_site_surface(selected_surfaces[0], false)
	elif not _selection_additive:
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
	for index in _parts.size():
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
	var pos: Vector3 = _selection.picker.anchor_position(index)
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
		var radius: float = _selection.picker.ANCHOR_RADIUS
		return Rect2(screen_pos - Vector2(radius, radius), Vector2(radius * 2.0, radius * 2.0))
	return _screen_rect_for_world_points(points, cam)

func _mesh_part_screen_rect(part_index: int, cam: Camera3D):
	var corners: Array = _preview.mesh_part_world_corners(part_index)
	return _screen_rect_for_world_points(corners, cam)

func _site_surface_screen_rect(index: int, cam: Camera3D):
	if index < 0 or index >= _site_surfaces_data.size():
		return null
	var points: Array = []
	var y: float = _selection.picker.surface_height(index)
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

func _toggle_mesh_part_selection(index: int) -> void:
	if index < 0 or index >= _parts.size():
		return
	var selected := _selected_part_indices.duplicate()
	if selected.has(index):
		selected.erase(index)
		_set_selected_mesh_parts(selected, -1 if selected.is_empty() else int(selected[0]))
	else:
		selected.append(index)
		_set_selected_mesh_parts(selected, index)

func _has_selected_mesh_part() -> bool:
	return _selected_part_index >= 0 and _selected_part_index < _parts.size()

func _sync_selected_mesh_part_controls() -> void:
	if not _has_selected_mesh_part():
		return
	_suppress_part_transform_changed = true
	var pos := _parts[_selected_part_index].position
	_view._part_x_spin.value = pos.x
	_view._part_y_spin.value = pos.y
	_view._part_z_spin.value = pos.z
	_view._part_rotation_y_spin.value = _parts[_selected_part_index].rotation_y
	_suppress_part_transform_changed = false

func _apply_selected_part_transform_from_state() -> void:
	if not _has_selected_mesh_part():
		return
	_apply_mesh_part_transform_from_state(_selected_part_index)

func _apply_mesh_part_transform_from_state(index: int, refresh_selection: bool = true) -> void:
	if index < 0 or index >= _parts.size():
		return
	_preview.set_mesh_part_transform(
		index,
		_parts[index].position,
		_parts[index].rotation_y,
		_parts[index].scale,
		_parts[index].pivot_offset,
		refresh_selection
	)

func _normalize_degrees(value: float) -> float:
	var result := value
	while result > 180.0:
		result -= 360.0
	while result < -180.0:
		result += 360.0
	return snappedf(result, 0.1)

func _snap_rotation_y_to_cardinal_if_close(value: float) -> float:
	return AssetAuthoringPolicy.rotation_degrees(value)

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
	var lot_w: float = _view._width_spin.value * 10.0   # CELL_M = 10
	var lot_d: float = _view._depth_spin.value * 10.0
	var mesh_w: float = aabb.size.x
	var mesh_d: float = aabb.size.z
	if mesh_w < 0.001 or mesh_d < 0.001:
		_log("[color=yellow]Mesh has zero XZ extent — cannot auto-fit.[/color]")
		return
	# Scale so the larger mesh dimension fills the corresponding lot dimension.
	var scale_x := lot_w / mesh_w
	var scale_z := lot_d / mesh_d
	var fit_scale := minf(scale_x, scale_z)
	_view._preview_scale_spin.value = snappedf(fit_scale, 0.01)
	_apply_selected_part_transform()
	_update_dim_label()
	var scaled_w := snappedf(mesh_w * fit_scale, 0.1)
	var scaled_d := snappedf(mesh_d * fit_scale, 0.1)
	var scaled_h := snappedf(aabb.size.y * fit_scale, 0.1)
	_log("Building footprint: %.1fm × %.1fm × %.1fm scaled to fit %.0fm × %.0fm lot (scale %.2fx)" % [
		scaled_w, scaled_d, scaled_h, lot_w, lot_d, fit_scale])

func _log(msg: String) -> void:
	if _view._log_label:
		_view._log_label.append_text(msg + "\n")
	_log_plain_lines.append(_strip_bbcode(msg))

func _strip_bbcode(text: String) -> String:
	if _bbcode_strip_regex:
		return _bbcode_strip_regex.sub(text, "", true)
	return text
