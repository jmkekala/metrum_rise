## Mesh import picker with a live 3D preview for the asset editor.
##
## Shows supported model files from the native filesystem and previews the
## selected GLB/GLTF/FBX before emitting the chosen absolute path.
extends Window

signal mesh_selected(path: String)

const EditorTheme = preload("res://scripts/ui/editor_theme.gd")

const SUPPORTED_EXTENSIONS := ["glb", "gltf", "fbx"]
const INITIAL_SIZE := Vector2i(1120, 720)
const LIST_MIN_W := 430.0
const WINDOW_PAD := 12
const WINDOW_SCREEN_MARGIN := 16
const PANEL_GAP := 12
const CONTROL_GAP := 6
const CONFIG_PATH := "user://asset_editor.cfg"

var theme_mode: String = EditorTheme.MODE_DARK

var _current_dir: String = ""
var _selected_path: String = ""
var _current_dirs: Array[String] = []
var _current_files: Array[String] = []

var _path_edit: LineEdit
var _filter_edit: LineEdit
var _file_list: ItemList
var _status_lbl: Label
var _stats_lbl: Label
var _import_btn: Button
var _body_split: HSplitContainer

var _subviewport: SubViewport
var _preview_root: Node3D
var _model_root: Node3D
var _camera: Camera3D
var _config := ConfigFile.new()
var _layout_restoring: bool = false

func _ready() -> void:
	title = "Import Mesh"
	min_size = Vector2i(860, 520)
	close_requested.connect(_on_close_requested)
	_config.load(CONFIG_PATH)
	_build_ui()
	_connect_layout_signals()

func open(start_dir: String) -> void:
	if not is_inside_tree():
		call_deferred("open", start_dir)
		return
	var resolved_dir := start_dir.strip_edges()
	if resolved_dir.is_empty() or not DirAccess.dir_exists_absolute(resolved_dir):
		resolved_dir = ProjectSettings.globalize_path("res://../")
	_current_dir = resolved_dir
	if _path_edit:
		_path_edit.text = _current_dir
	_refresh_file_list()
	var dialog_size := _clamped_dialog_size(_saved_dialog_size())
	if _has_saved_position():
		var dialog_position := _clamped_dialog_position(_saved_dialog_position(), dialog_size)
		popup(Rect2i(dialog_position, dialog_size))
	else:
		popup_centered(dialog_size)
	call_deferred("_keep_inside_parent_viewport")
	call_deferred("_restore_split_layout")

func _build_ui() -> void:
	theme_mode = EditorTheme.normalize_mode(theme_mode)
	var shell := PanelContainer.new()
	shell.set_anchors_preset(Control.PRESET_FULL_RECT)
	add_child(shell)

	var margin := MarginContainer.new()
	margin.set_anchors_preset(Control.PRESET_FULL_RECT)
	margin.add_theme_constant_override("margin_left", WINDOW_PAD)
	margin.add_theme_constant_override("margin_right", WINDOW_PAD)
	margin.add_theme_constant_override("margin_top", WINDOW_PAD)
	margin.add_theme_constant_override("margin_bottom", WINDOW_PAD)
	shell.add_child(margin)

	var root := VBoxContainer.new()
	root.add_theme_constant_override("separation", PANEL_GAP)
	margin.add_child(root)

	_body_split = HSplitContainer.new()
	_body_split.add_theme_constant_override("separation", PANEL_GAP)
	_body_split.size_flags_horizontal = Control.SIZE_EXPAND_FILL
	_body_split.size_flags_vertical = Control.SIZE_EXPAND_FILL
	root.add_child(_body_split)

	var browser := VBoxContainer.new()
	browser.custom_minimum_size.x = LIST_MIN_W
	browser.size_flags_vertical = Control.SIZE_EXPAND_FILL
	browser.add_theme_constant_override("separation", CONTROL_GAP)
	_body_split.add_child(browser)

	var location_row := HBoxContainer.new()
	location_row.add_theme_constant_override("separation", CONTROL_GAP)
	browser.add_child(location_row)

	var home_btn := Button.new()
	home_btn.text = "Home"
	home_btn.pressed.connect(_go_home)
	location_row.add_child(home_btn)

	var project_btn := Button.new()
	project_btn.text = "Project"
	project_btn.pressed.connect(_go_project)
	location_row.add_child(project_btn)

	var parent_btn := Button.new()
	parent_btn.text = "Parent"
	parent_btn.pressed.connect(_go_up)
	location_row.add_child(parent_btn)

	var refresh_btn := Button.new()
	refresh_btn.text = "Refresh"
	refresh_btn.pressed.connect(_refresh_file_list)
	location_row.add_child(refresh_btn)

	_path_edit = LineEdit.new()
	_path_edit.size_flags_horizontal = Control.SIZE_EXPAND_FILL
	_path_edit.placeholder_text = "Folder path"
	_path_edit.text_submitted.connect(_on_path_submitted)
	browser.add_child(_path_edit)

	_filter_edit = LineEdit.new()
	_filter_edit.size_flags_horizontal = Control.SIZE_EXPAND_FILL
	_filter_edit.placeholder_text = "Filter files and folders"
	_filter_edit.text_changed.connect(_on_filter_changed)
	browser.add_child(_filter_edit)

	_file_list = ItemList.new()
	_file_list.size_flags_horizontal = Control.SIZE_EXPAND_FILL
	_file_list.size_flags_vertical = Control.SIZE_EXPAND_FILL
	_file_list.item_selected.connect(_on_file_item_selected)
	_file_list.item_activated.connect(_on_file_item_activated)
	browser.add_child(_file_list)

	_status_lbl = Label.new()
	_status_lbl.autowrap_mode = TextServer.AUTOWRAP_WORD_SMART
	_status_lbl.text = "Select a mesh file."
	browser.add_child(_status_lbl)

	var preview_box := VBoxContainer.new()
	preview_box.size_flags_horizontal = Control.SIZE_EXPAND_FILL
	preview_box.size_flags_vertical = Control.SIZE_EXPAND_FILL
	preview_box.add_theme_constant_override("separation", CONTROL_GAP)
	_body_split.add_child(preview_box)

	var viewport_container := SubViewportContainer.new()
	viewport_container.size_flags_horizontal = Control.SIZE_EXPAND_FILL
	viewport_container.size_flags_vertical = Control.SIZE_EXPAND_FILL
	viewport_container.stretch = true
	preview_box.add_child(viewport_container)

	_subviewport = SubViewport.new()
	_subviewport.size = Vector2i(700, 520)
	_subviewport.own_world_3d = true
	_subviewport.render_target_update_mode = SubViewport.UPDATE_ALWAYS
	_subviewport.transparent_bg = false
	viewport_container.add_child(_subviewport)

	_preview_root = Node3D.new()
	_subviewport.add_child(_preview_root)

	var world_environment := WorldEnvironment.new()
	world_environment.environment = EditorTheme.preview_environment(theme_mode)
	_preview_root.add_child(world_environment)

	var light := DirectionalLight3D.new()
	light.rotation_degrees = Vector3(-55.0, -35.0, 0.0)
	light.light_energy = 3.0
	_preview_root.add_child(light)

	_camera = Camera3D.new()
	_camera.current = true
	_camera.near = 0.05
	_camera.far = 5000.0
	_camera.fov = 42.0
	_preview_root.add_child(_camera)

	_model_root = Node3D.new()
	_preview_root.add_child(_model_root)

	_stats_lbl = Label.new()
	_stats_lbl.autowrap_mode = TextServer.AUTOWRAP_WORD_SMART
	_stats_lbl.text = "No mesh selected."
	preview_box.add_child(_stats_lbl)

	var footer := HBoxContainer.new()
	footer.add_theme_constant_override("separation", CONTROL_GAP)
	root.add_child(footer)

	var spacer := Control.new()
	spacer.size_flags_horizontal = Control.SIZE_EXPAND_FILL
	footer.add_child(spacer)

	var cancel_btn := Button.new()
	cancel_btn.text = "Cancel"
	cancel_btn.pressed.connect(_on_close_requested)
	footer.add_child(cancel_btn)

	_import_btn = Button.new()
	_import_btn.text = "Import"
	_import_btn.disabled = true
	_import_btn.pressed.connect(_confirm_selection)
	footer.add_child(_import_btn)
	_apply_theme(shell)

func _refresh_file_list() -> void:
	if not _file_list:
		return
	_file_list.clear()
	_selected_path = ""
	if _import_btn:
		_import_btn.disabled = true

	if not DirAccess.dir_exists_absolute(_current_dir):
		_set_status("Directory does not exist: %s" % _current_dir, true)
		return

	if _path_edit:
		_path_edit.text = _current_dir

	var dir := DirAccess.open(_current_dir)
	if not dir:
		_set_status("Could not open directory: %s" % _current_dir, true)
		return

	var dirs: Array[String] = []
	var files: Array[String] = []
	dir.list_dir_begin()
	var entry := dir.get_next()
	while not entry.is_empty():
		if entry.begins_with("."):
			entry = dir.get_next()
			continue
		if dir.current_is_dir():
			dirs.append(entry)
		elif _is_supported_mesh(entry):
			files.append(entry)
		entry = dir.get_next()
	dir.list_dir_end()
	dirs.sort()
	files.sort()

	_current_dirs = dirs
	_current_files = files
	_populate_file_list()

func _on_filter_changed(_text: String) -> void:
	_populate_file_list()

func _populate_file_list() -> void:
	if not _file_list:
		return
	_file_list.clear()

	var filter_text := ""
	if _filter_edit:
		filter_text = _filter_edit.text.strip_edges().to_lower()

	var visible_dirs: Array[String] = []
	var visible_files: Array[String] = []
	for name in _current_dirs:
		if filter_text.is_empty() or name.to_lower().contains(filter_text):
			visible_dirs.append(name)
	for name in _current_files:
		if filter_text.is_empty() or name.to_lower().contains(filter_text):
			visible_files.append(name)

	var parent_dir := _current_dir.get_base_dir()
	if parent_dir != _current_dir and not parent_dir.is_empty():
		_add_file_list_item("..", parent_dir, true)

	if not visible_dirs.is_empty():
		_add_file_list_header("Folders")
		for name in visible_dirs:
			_add_file_list_item("%s/" % name, _current_dir.path_join(name), true)

	if not visible_files.is_empty():
		_add_file_list_header("Meshes")
		for name in visible_files:
			_add_file_list_item(name, _current_dir.path_join(name), false)

	if visible_dirs.is_empty() and visible_files.is_empty():
		_add_file_list_header("No matches")

	if filter_text.is_empty():
		_set_status(
			"%d folder(s), %d mesh file(s)" % [_current_dirs.size(), _current_files.size()],
			false
		)
	else:
		_set_status(
			"%d folder(s), %d mesh file(s) match" % [visible_dirs.size(), visible_files.size()],
			false
		)

func _add_file_list_header(label: String) -> void:
	_file_list.add_item(label)
	var idx := _file_list.item_count - 1
	_file_list.set_item_metadata(idx, {"header": true})
	_file_list.set_item_disabled(idx, true)

func _add_file_list_item(label: String, path: String, is_dir: bool) -> void:
	_file_list.add_item(label)
	var idx := _file_list.item_count - 1
	_file_list.set_item_metadata(idx, {"path": path, "dir": is_dir})

func _on_path_submitted(path: String) -> void:
	var resolved := path.strip_edges()
	if DirAccess.dir_exists_absolute(resolved):
		_current_dir = resolved
		_refresh_file_list()
	else:
		_set_status("Directory does not exist: %s" % resolved, true)

func _go_up() -> void:
	var parent_dir := _current_dir.get_base_dir()
	if parent_dir == _current_dir or parent_dir.is_empty():
		return
	_current_dir = parent_dir
	_refresh_file_list()

func _go_home() -> void:
	var home_dir := OS.get_environment("HOME").strip_edges()
	if home_dir.is_empty() or not DirAccess.dir_exists_absolute(home_dir):
		return
	_current_dir = home_dir
	_refresh_file_list()

func _go_project() -> void:
	var project_dir := ProjectSettings.globalize_path("res://../")
	if not DirAccess.dir_exists_absolute(project_dir):
		return
	_current_dir = project_dir
	_refresh_file_list()

func _on_file_item_selected(index: int) -> void:
	var meta = _file_list.get_item_metadata(index)
	if not (meta is Dictionary):
		return
	if bool(meta.get("header", false)):
		return
	var path := str(meta.get("path", "")).strip_edges()
	if bool(meta.get("dir", false)):
		_selected_path = ""
		_import_btn.disabled = true
		_set_status("Folder: %s" % path, false)
		return
	_preview_mesh(path)

func _on_file_item_activated(index: int) -> void:
	var meta = _file_list.get_item_metadata(index)
	if not (meta is Dictionary):
		return
	if bool(meta.get("header", false)):
		return
	var path := str(meta.get("path", "")).strip_edges()
	if bool(meta.get("dir", false)):
		_current_dir = path
		_refresh_file_list()
		return
	_preview_mesh(path)
	_confirm_selection()

func _preview_mesh(path: String) -> void:
	if path.is_empty() or not FileAccess.file_exists(path):
		return
	_clear_model()
	_selected_path = ""
	_import_btn.disabled = true
	_set_status("Loading preview: %s" % path.get_file(), false)
	_stats_lbl.text = "Loading..."

	var ext := path.get_extension().to_lower()
	var doc: GLTFDocument
	var state: GLTFState
	if ext == "fbx":
		doc = FBXDocument.new()
		state = FBXState.new()
	else:
		doc = GLTFDocument.new()
		state = GLTFState.new()
	var err := doc.append_from_file(path, state)
	if err != OK:
		_set_status("Could not preview '%s' (error %d)" % [path.get_file(), err], true)
		_stats_lbl.text = "Preview failed."
		return

	var scene := doc.generate_scene(state)
	if not scene:
		_set_status("Preview generated no scene: %s" % path.get_file(), true)
		_stats_lbl.text = "Preview failed."
		return

	_model_root.add_child(scene)
	var aabb := AABB()
	if scene is Node3D:
		aabb = _compute_aabb(scene as Node3D)
	_frame_camera(aabb)
	_selected_path = path
	_import_btn.disabled = false
	_set_status("Selected: %s" % path.get_file(), false)
	_stats_lbl.text = _format_stats(path, scene, aabb)

func _confirm_selection() -> void:
	if _selected_path.is_empty():
		return
	_save_layout_state()
	emit_signal("mesh_selected", _selected_path)
	queue_free()

func _clear_model() -> void:
	for child in _model_root.get_children():
		child.queue_free()

func _is_supported_mesh(path: String) -> bool:
	return SUPPORTED_EXTENSIONS.has(path.get_extension().to_lower())

func _set_status(message: String, is_error: bool) -> void:
	if not _status_lbl:
		return
	_status_lbl.text = message
	_status_lbl.add_theme_color_override(
		"font_color",
		EditorTheme.color(theme_mode, "error") if is_error else EditorTheme.color(theme_mode, "status")
	)

func _saved_dialog_size() -> Vector2i:
	var width := int(_config.get_value("mesh_import_dialog", "window_width", INITIAL_SIZE.x))
	var height := int(_config.get_value("mesh_import_dialog", "window_height", INITIAL_SIZE.y))
	return Vector2i(maxi(width, min_size.x), maxi(height, min_size.y))

func _clamped_dialog_size(desired_size: Vector2i) -> Vector2i:
	var viewport_size := _parent_viewport_size()
	var max_size := Vector2i(
		maxi(320, viewport_size.x - WINDOW_SCREEN_MARGIN * 2),
		maxi(260, viewport_size.y - WINDOW_SCREEN_MARGIN * 2)
	)
	var effective_min := Vector2i(
		mini(min_size.x, max_size.x),
		mini(min_size.y, max_size.y)
	)
	min_size = effective_min
	return Vector2i(
		clampi(desired_size.x, effective_min.x, max_size.x),
		clampi(desired_size.y, effective_min.y, max_size.y)
	)

func _clamped_dialog_position(desired_position: Vector2i, dialog_size: Vector2i) -> Vector2i:
	var viewport_size := _parent_viewport_size()
	var max_x := maxi(WINDOW_SCREEN_MARGIN, viewport_size.x - dialog_size.x - WINDOW_SCREEN_MARGIN)
	var max_y := maxi(WINDOW_SCREEN_MARGIN, viewport_size.y - dialog_size.y - WINDOW_SCREEN_MARGIN)
	return Vector2i(
		clampi(desired_position.x, WINDOW_SCREEN_MARGIN, max_x),
		clampi(desired_position.y, WINDOW_SCREEN_MARGIN, max_y)
	)

func _keep_inside_parent_viewport() -> void:
	var dialog_size := _clamped_dialog_size(size)
	if dialog_size != size:
		size = dialog_size
	position = _clamped_dialog_position(position, dialog_size)

func _parent_viewport_size() -> Vector2i:
	var parent_node := get_parent()
	if parent_node:
		var parent_viewport := parent_node.get_viewport()
		if parent_viewport:
			var parent_size := parent_viewport.get_visible_rect().size
			if parent_size.x > 0.0 and parent_size.y > 0.0:
				return Vector2i(int(round(parent_size.x)), int(round(parent_size.y)))
	var root := get_tree().root if get_tree() else null
	if root:
		return root.size
	return INITIAL_SIZE

func _has_saved_position() -> bool:
	return (
		_config.has_section_key("mesh_import_dialog", "window_x")
		and _config.has_section_key("mesh_import_dialog", "window_y")
	)

func _saved_dialog_position() -> Vector2i:
	return Vector2i(
		int(_config.get_value("mesh_import_dialog", "window_x", position.x)),
		int(_config.get_value("mesh_import_dialog", "window_y", position.y))
	)

func _restore_split_layout() -> void:
	if not _body_split:
		return
	_layout_restoring = true
	var browser_w := int(_config.get_value("mesh_import_dialog", "browser_width", LIST_MIN_W))
	var max_browser_w := int(maxf(LIST_MIN_W, _body_split.size.x - 360.0))
	_body_split.split_offset = clampi(browser_w, int(LIST_MIN_W), max_browser_w)
	_layout_restoring = false

func _connect_layout_signals() -> void:
	if has_signal("size_changed"):
		connect("size_changed", Callable(self, "_on_layout_changed"))
	if _body_split and _body_split.has_signal("dragged"):
		_body_split.connect("dragged", Callable(self, "_on_split_dragged"))

func _on_split_dragged(_offset: int) -> void:
	_save_layout_state()

func _on_layout_changed() -> void:
	if _layout_restoring:
		return
	_save_layout_state()

func _on_close_requested() -> void:
	_save_layout_state()
	queue_free()

func _notification(what: int) -> void:
	if what == NOTIFICATION_PREDELETE or what == NOTIFICATION_WM_CLOSE_REQUEST:
		_save_layout_state()

func _save_layout_state() -> void:
	if _layout_restoring:
		return
	_config.set_value("mesh_import_dialog", "window_width", size.x)
	_config.set_value("mesh_import_dialog", "window_height", size.y)
	_config.set_value("mesh_import_dialog", "window_x", position.x)
	_config.set_value("mesh_import_dialog", "window_y", position.y)
	if _body_split and _body_split.size.x > 0.0:
		_config.set_value("mesh_import_dialog", "browser_width", _body_split.split_offset)
	var err := _config.save(CONFIG_PATH)
	if err != OK:
		push_warning("MeshImportDialog: could not save layout to %s (error %d)" % [CONFIG_PATH, err])

func _apply_theme(root: Node) -> void:
	EditorTheme.apply_to_tree(root, theme_mode)

func _format_stats(path: String, scene: Node, aabb: AABB) -> String:
	var mesh_count := scene.find_children("*", "MeshInstance3D", true, false).size()
	return "%s\n%.2f m x %.2f m x %.2f m\n%d mesh node(s)" % [
		path,
		aabb.size.x,
		aabb.size.y,
		aabb.size.z,
		mesh_count,
	]

func _frame_camera(aabb: AABB) -> void:
	var center := aabb.position + aabb.size * 0.5
	var max_dim := maxf(aabb.size.x, maxf(aabb.size.y, aabb.size.z))
	var distance := maxf(max_dim * 1.55, 2.5)
	_camera.global_position = center + Vector3(distance, distance * 0.58, distance)
	_camera.look_at(center)
	_camera.far = maxf(100.0, distance * 6.0)

func _compute_aabb(node: Node3D) -> AABB:
	var result := AABB()
	var first := true
	for child in node.find_children("*", "MeshInstance3D", true, false):
		var mi := child as MeshInstance3D
		if not mi or not mi.mesh:
			continue
		var rel := Transform3D.IDENTITY
		var cur: Node = mi
		while cur != node and cur != null:
			if cur is Node3D:
				rel = (cur as Node3D).transform * rel
			cur = cur.get_parent()
		var node_aabb := rel * mi.get_aabb()
		if first:
			result = node_aabb
			first = false
		else:
			result = result.merge(node_aabb)
	return result
