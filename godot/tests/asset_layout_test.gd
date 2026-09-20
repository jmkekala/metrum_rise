# SPDX-License-Identifier: GPL-2.0-only

## Real-editor startup, contextual chrome and persisted layout regression.
## Run with the isolated user profile supplied by run.sh; optional rendered captures use generated geometry.
extends SceneTree

const EditorScene = preload("res://scenes/AssetEditor.tscn")
const CONFIG := "user://asset_editor.cfg"
var _failures := 0
var _capture_dir := ""

func _initialize() -> void:
	for argument in OS.get_cmdline_user_args():
		if argument.begins_with("--capture-layout="):
			_capture_dir = argument.trim_prefix("--capture-layout=")
	call_deferred("_run")

func _expect(condition: bool, message: String) -> void:
	if not condition:
		_failures += 1
		push_error(message)

func _settle(editor: Node) -> void:
	for frame in 40:
		await process_frame
		if frame >= 6 and not editor._layout.pending:
			return
	_expect(false, "workspace layout must converge without a per-frame resize loop")

func _config(width: int = 420, collapsed: bool = false) -> void:
	var config := ConfigFile.new()
	config.set_value("layout", "window_width", 1920)
	config.set_value("layout", "window_height", 1080)
	config.set_value("layout", "right_panel_width", width)
	config.set_value("layout", "bottom_log_height", 1263)
	config.set_value("layout", "inspector_collapsed", collapsed)
	_expect(config.save(CONFIG) == OK, "test layout config saves")

func _create(editor: Node) -> void:
	editor._session.create_asset({"type": "residential", "name": "Layout house", "pack": {"pack_id": "layout-test"}})

func _welcome(editor: Node) -> void:
	var view = editor._view
	_expect(not editor._session.has_document and editor._session.document.snapshot().is_empty(), "startup has no dummy building document")
	_expect(view.welcome.is_visible_in_tree(), "welcome is visible")
	_expect(not view.inspector.is_visible_in_tree() and not view.toolbar.is_visible_in_tree() and not view.preview_bar.is_visible_in_tree(), "welcome hides all authoring chrome")
	_expect(not editor._preview.visible and not view._preview_view_rect.is_visible_in_tree(), "welcome has no placeholder lot or interactive preview")
	_expect(editor._session._issues.is_empty() and view.issues_box.get_child_count() == 0, "welcome has no validation issues")
	_expect(not view.library.visible and not view.diagnostics.visible, "startup never opens auxiliary panes")
	var buttons: Array = view.welcome.find_children("*", "Button", true, false)
	_expect(buttons.size() == 2, "welcome prioritizes create and library; drafts remain in File")
	for window in editor.find_children("*", "Window", true, false):
		_expect(not window.visible, "startup never opens a modal")
	editor.menu_save()
	editor.menu_import_mesh()
	editor.menu_export_asset()
	_expect(not editor._session.document.is_dirty(), "disabled authoring actions cannot create a phantom document")
	for popup in editor._top_menu._menu_bar.get_children():
		if popup is PopupMenu:
			for id in [editor._top_menu.ActionId.FILE_SAVE, editor._top_menu.ActionId.FILE_EXPORT_ASSET, editor._top_menu.ActionId.ASSET_IMPORT_MESH]:
				var index: int = popup.get_item_index(id)
				if index >= 0:
					_expect(popup.is_item_disabled(index), "document-only menu actions are disabled")

func _bounds(editor: Node, label: String) -> void:
	var view = editor._view
	var logical := root.get_visible_rect().size
	var pane: Rect2 = view.inspector.get_global_rect()
	var preview: Rect2 = view._preview_view_rect.get_global_rect()
	_expect(pane.size.x >= 340 and pane.size.x <= 600, label + ": inspector stays within logical bounds")
	_expect(preview.size.x >= 318, label + ": preview retains usable space")
	_expect(pane.end.x <= logical.x + 1 and pane.end.y <= logical.y + 1, label + ": inspector fits window")
	_expect(preview.end.x <= pane.position.x + 1, label + ": preview does not extend underneath inspector")
	_expect(view._theme_root.position.y >= editor._top_menu._shell.size.y, label + ": chrome does not overlap menus")
	_expect(editor._top_menu._shell.size.x >= logical.x - 1, label + ": menu shell spans the window (%s / %s)" % [editor._top_menu._shell.size.x, logical.x])
	_expect(editor._top_menu._menu_bar.size.x >= 180, label + ": menu labels have room (%s)" % editor._top_menu._menu_bar.size)
	_expect(not view._preview_view_rect.get_global_rect().intersects(view.inspector.get_global_rect()), label + ": inspector and preview do not overlap")
	if view.task_picker.selected == view.TASKS.find("model") and view.part_properties.visible:
		_expect(view._preview_panel._add.is_visible_in_tree() and pane.encloses(view._preview_panel._add.get_global_rect()), label + ": Add LOD is immediately reachable without scrolling or changing a subsection")

func _capture(name: String) -> void:
	if _capture_dir.is_empty():
		return
	DirAccess.make_dir_recursive_absolute(_capture_dir)
	await process_frame
	await process_frame
	await RenderingServer.frame_post_draw
	_expect(root.get_texture().get_image().save_png(_capture_dir.path_join(name + ".png")) == OK, "rendered capture: " + name)

func _run() -> void:
	# A genuinely fresh profile and a wide startup reproduce the user's original entry path.
	var fresh := ConfigFile.new()
	_expect(fresh.save(CONFIG) == OK, "fresh preferences")
	root.size = Vector2i(1920, 1080)
	var editor = EditorScene.instantiate()
	root.add_child(editor)
	var before: String = editor._config.encode_to_text()
	editor._save_layout_state()
	_expect(editor._config.encode_to_text() == before, "startup dimensions are never saved before layout settles")
	await _settle(editor)
	_welcome(editor)
	await _capture("welcome-dark")
	editor.set_ui_theme_mode("light")
	await process_frame
	await _capture("welcome-light")
	editor._view.toggle_library()
	await _settle(editor)
	_expect(editor._view.library.visible and editor._view.welcome.visible and not editor._view.inspector.visible, "browse library works without inventing a document")
	await _capture("welcome-library")
	editor._view.toggle_library()
	editor.menu_new_asset()
	_expect(editor._session._creation.visible, "New asset opens type-first creation only on request")
	editor._session._creation.hide()
	_welcome(editor)
	var file_menu: PopupMenu = editor._top_menu._menu_bar.get_node("File")
	file_menu.id_pressed.emit(editor._top_menu.ActionId.FILE_LOAD)
	var picker: FileDialog = editor.get_child(editor.get_child_count() - 1)
	_expect(picker.visible, "File > Open Draft remains reachable from welcome")
	picker.hide()
	_create(editor)
	await _settle(editor)
	_expect(editor._view.task_picker.selected == 1, "new asset starts in Model")
	_expect(absf(editor._view.inspector.size.x - 420) <= 1, "fresh inspector should be 420, got %s (preferred %s, offset %s)" % [editor._view.inspector.size.x, editor._layout.inspector_width, editor._view._right_split.split_offset])
	_expect(not editor._view.comparison_button.visible and not editor._view.frame_button.visible, "unavailable preview actions are hidden")
	_expect(not editor._view.remove_part_button.visible and not editor._view.remove_anchor_button.visible and not editor._view.remove_surface_button.visible, "remove actions need a selection")
	_expect(editor._view.model_hint.visible and not editor._view._mesh_part_list.visible, "empty Model task invites import without an empty list")
	_bounds(editor, "fresh editing")
	await _capture("editing-empty-model")
	await _test_export_navigation(editor)
	await _test_model(editor)
	for scale in [1.0, 1.5, 2.0]:
		root.content_scale_factor = scale
		for logical_size in [Vector2i(960, 640), Vector2i(1440, 900), Vector2i(3063, 1751)]:
			root.size = Vector2i(Vector2(logical_size) * scale)
			await _settle(editor)
			_bounds(editor, "%s at %.1fx" % [logical_size, scale])
			editor._view.toggle_library()
			await _settle(editor)
			_bounds(editor, "library %s at %.1fx" % [logical_size, scale])
			editor._view.toggle_library()
			await _settle(editor)
			if logical_size == Vector2i(960, 640):
				await _capture("editing-small-%.1fx" % scale)
	root.content_scale_factor = 1.0
	root.size = Vector2i(1920, 1080)
	await _settle(editor)
	var idle_revision: int = editor._layout._revision
	var idle_config: String = editor._config.encode_to_text()
	for frame in 12:
		await process_frame
	_expect(editor._layout._revision == idle_revision and editor._config.encode_to_text() == idle_config, "idle frames neither relayout nor rewrite preferences")
	# Exercise the same split signal as a divider drag, after the container sorts its new offset.
	editor._view._right_split.split_offset += roundi(editor._view.inspector.size.x) - 500
	editor._view._right_split.dragged.emit(editor._view._right_split.split_offset)
	await _settle(editor)
	_expect(absf(editor._view.inspector.size.x - 500) <= 1 and editor._layout.inspector_width == 500, "intentional divider resizing is preserved")
	for desired in [1900, 100, 500]:
		editor._view._right_split.split_offset += roundi(editor._view.inspector.size.x) - desired
		editor._view._right_split.dragged.emit(editor._view._right_split.split_offset)
		await _settle(editor)
		_bounds(editor, "drag limits %d" % desired)
	editor._view.toggle_inspector()
	await _settle(editor)
	_expect(not editor._view.inspector.visible and editor._view._preview_view_rect.size.x > 1800, "collapse gives the viewport the freed space")
	await _capture("inspector-collapsed")
	editor._session._save_to("user://layout-test.metrum-draft")
	editor._menus.actions.create_from_controls("entrance")
	editor._menus.actions.confirm()
	editor._session.capture_geometry("Add entrance")
	await _test_export_navigation(editor)
	editor._view.toggle_inspector()
	await _settle(editor)
	editor._session.publish()
	_expect(editor._session._issues.is_empty(), "library fixture publishes")
	editor.free()
	await process_frame
	editor = EditorScene.instantiate()
	root.add_child(editor)
	await _settle(editor)
	_welcome(editor)
	editor._session.load_draft("user://layout-test.metrum-draft")
	await _settle(editor)
	_expect(editor._session.has_document and not editor._view.inspector.visible, "reopen restores collapsed preference without changing the draft")
	editor._view.toggle_inspector()
	await _settle(editor)
	_expect(absf(editor._view.inspector.size.x - 500) <= 1, "reopen should retain 500, got %s (preferred %s, offset %s)" % [editor._view.inspector.size.x, editor._layout.inspector_width, editor._view._right_split.split_offset])
	editor.free()
	await process_frame
	_config(1699)
	editor = EditorScene.instantiate()
	root.add_child(editor)
	await _settle(editor)
	_welcome(editor)
	_create(editor)
	await _settle(editor)
	_bounds(editor, "stale oversized layout")
	_expect(editor._view.inspector.size.x <= 600, "1699px saved inspector cannot consume half the screen")
	editor._view.toggle_diagnostics()
	await _settle(editor)
	_expect(editor._view.diagnostics.size.y <= 320, "stale diagnostics height bounded: got %s (preferred %s, offset %s)" % [editor._view.diagnostics.size.y, editor._layout.log_height, editor._view._main_vsplit.split_offset])
	editor.menu_reset_layout()
	await _settle(editor)
	_expect(absf(editor._view.inspector.size.x - 420) <= 1, "Reset layout restores default inspector")
	_expect(not editor._view.library.visible and not editor._view.diagnostics.visible, "Reset layout closes optional panes")
	await _capture("reset-layout")
	editor.free()
	await process_frame
	editor = EditorScene.instantiate()
	root.add_child(editor)
	await _settle(editor)
	_welcome(editor)
	editor._view.toggle_library()
	editor._view._asset_search_edit.text = "Layout house"
	editor._on_asset_search_changed("Layout house")
	var item: TreeItem = editor._view._asset_tree.get_root().get_first_child()
	while item != null and not item.get_metadata(0) is String:
		item = item.get_first_child()
	_expect(item != null, "published fixture appears in welcome library")
	if item != null:
		item.select(0)
		editor._view._asset_tree.item_activated.emit()
		await _settle(editor)
		_expect(editor._session.has_document and editor._parts.size() == 1 and not editor._view.welcome.visible, "library activation opens the actual asset workspace")
		_bounds(editor, "welcome library activation")
	# Scene replacement detaches the editor before PREDELETE; its window is then unavailable.
	root.remove_child(editor)
	var saved_on_exit: String = editor._config.encode_to_text()
	editor._save_layout_state()
	editor._layout.request_layout()
	_expect(editor._config.encode_to_text() == saved_on_exit, "detached layout callbacks cannot access a missing window or rewrite preferences")
	editor.free()
	await process_frame
	editor = EditorScene.instantiate()
	root.add_child(editor)
	await _settle(editor)
	editor._layout.request_layout()
	root.remove_child(editor)
	editor.free()
	for frame in 6:
		await process_frame
	if _failures == 0:
		print("PASS asset_layout_test")
	quit(0 if _failures == 0 else 1)

func _test_export_navigation(editor: Node) -> void:
	var view = editor._view
	var export_button: Button
	for control in view.toolbar.get_children():
		if control is Button:
			_expect(not control.text.to_lower().contains("draft"), "draft actions do not occupy the main toolbar")
			if control.text == "Export asset…":
				export_button = control
	_expect(export_button != null, "main toolbar exposes Export asset")
	if export_button == null:
		return
	var before: Dictionary = editor._session.document.snapshot()
	var dirty: bool = editor._session.document.is_dirty()
	var output: String = ProjectSettings.globalize_path("user://mods/layout-test/assets/" + str(before["params"]["asset_id"]))
	_expect(not DirAccess.dir_exists_absolute(output), "navigation fixture has not been published")
	var file_menu: PopupMenu = editor._top_menu._menu_bar.get_node("File")
	var export_id: int = editor._top_menu.ActionId.FILE_EXPORT_ASSET
	var export_index := file_menu.get_item_index(export_id)
	_expect(export_index >= 0 and not file_menu.is_item_disabled(export_index), "File > Export Asset is enabled for an open document")
	for action: Callable in [func(): export_button.pressed.emit(), func(): file_menu.id_pressed.emit(export_id)]:
		view.show_task("model")
		if view.inspector.visible:
			view.toggle_inspector()
		await _settle(editor)
		action.call()
		await _settle(editor)
		_expect(view.inspector.is_visible_in_tree() and view.task_picker.selected == view.TASKS.find("validate"), "export entry points reveal validation even with the inspector collapsed")
		_expect(view.tasks["validate"].is_visible_in_tree() and view.export_summary.text.contains("user://mods/layout-test/assets/"), "export review displays its destination")
		_expect(not editor._layout.inspector_collapsed, "export navigation synchronizes the inspector collapse preference")
		_expect(editor._session.document.snapshot() == before and editor._session.document.is_dirty() == dirty, "export review does not change the document or draft save state")
		_expect(not DirAccess.dir_exists_absolute(output), "opening export review never publishes, even when valid")
		_bounds(editor, "export review")
	var save_index := file_menu.get_item_index(editor._top_menu.ActionId.FILE_SAVE)
	_expect(save_index >= 0 and not file_menu.is_item_disabled(save_index), "File > Save Draft remains enabled")
	if editor._session.document.draft_path().is_empty():
		file_menu.id_pressed.emit(editor._top_menu.ActionId.FILE_SAVE)
		var picker: FileDialog = editor.get_child(editor.get_child_count() - 1)
		_expect(picker.visible and picker.file_mode == FileDialog.FILE_MODE_SAVE_FILE, "File > Save Draft opens the draft save dialog")
		picker.hide()
	await _capture("export-review")
	view.show_task("model")

func _test_model(editor: Node) -> void:
	var fixture := Node3D.new()
	var mesh := MeshInstance3D.new()
	mesh.mesh = BoxMesh.new()
	mesh.mesh.size = Vector3(6, 8, 6)
	fixture.add_child(mesh)
	var exporter := GLTFDocument.new()
	var state := GLTFState.new()
	var path := ProjectSettings.globalize_path("user://layout-box.glb")
	_expect(exporter.append_from_scene(fixture, state) == OK and exporter.write_to_filesystem(state, path) == OK, "generated preview fixture")
	fixture.free()
	editor._on_glb_file_selected(path)
	_expect(editor._view.frame_button.visible, "Frame selection appears for a selected part")
	_expect(editor._view.remove_part_button.visible and not editor._view.model_hint.visible, "selected model exposes applicable actions")
	editor._preview.load_ghost(path, 1.0, 2, 2)
	editor._view.update_context_actions()
	_expect(editor._view.comparison_button.visible, "Clear comparison appears with a comparison")
	editor._on_clear_ghost_pressed()
	_expect(not editor._view.comparison_button.visible, "Clear comparison disappears after clearing")
	for mode in ["dark", "light"]:
		editor.set_ui_theme_mode(mode)
		for task in ["model", "overview", "site", "gameplay", "validate"]:
			editor._view.show_task(task)
			await _settle(editor)
			_bounds(editor, mode + " " + task)
			await _capture(mode + "-" + task)
	editor._view.show_task("model")
