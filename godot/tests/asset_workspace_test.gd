# SPDX-License-Identifier: GPL-2.0-only

## Real task-oriented editor regression: all building types, document history, guards and drafts.
## Generated fixtures live in the isolated test profile; no creator project is required.
extends SceneTree

const EditorScene = preload("res://scenes/AssetEditor.tscn")

var _failures := 0
var _guard_actions := 0
var _model_path := ""

func _initialize() -> void:
	call_deferred("_run")

func _expect(value: bool, message: String) -> void:
	if not value:
		_failures += 1
		push_error(message)

func _run() -> void:
	var editor = EditorScene.instantiate()
	root.add_child(editor)
	await process_frame
	var session = editor._session
	var view = editor._view
	_expect(not view.library.visible and not view.diagnostics.visible, "library and log leave room for the viewport")
	_expect(view.tasks.keys() == ["overview", "model", "site", "gameplay", "validate"], "five freely navigable tasks")
	var pack := {"pack_id": "workspace-test", "display_name": "Workspace test"}
	for kind in ["residential", "commercial", "industrial", "extractor", "farm", "service", "explicit"]:
		session.create_asset({"type": kind, "subtype": "power", "name": kind, "pack": pack})
		view.show_task("gameplay")
		_expect(session.descriptor["kind"] == kind, "creation chooses " + kind)
		for field in ["density", "household_capacity", "flat_size_m2", "worker_capacity", "economy_profile", "extractor_resource", "field_resource", "service_class"]:
			_expect(view.rows[field].visible == session.descriptor["fields"].has(field), "%s visibility for %s" % [field, kind])
		if kind == "residential":
			_expect(not view.catalog_refresh.visible and not view.derived_workers.visible, "housing has no economy controls")
		if kind in ["commercial", "industrial", "extractor", "farm", "service"]:
			var profile: String = {"commercial": "grocery_basic", "industrial": "machinery_factory_basic", "extractor": "coal_mine_basic", "farm": "grain_farm_basic", "service": "power_plant_basic"}[kind]
			session.set_field("economy_profile", profile)
			_expect(not view.rows["worker_capacity"].visible and view.derived_workers.visible, "profile employment is derived, not editable")
			if kind != "extractor":
				for index in view._economy_profile_btn.item_count:
					_expect(view._economy_profile_btn.get_item_metadata(index) != "coal_mine_basic", "mine profile must not appear for " + kind)
	for subtype: Dictionary in JSON.parse_string(session.policy.types_json())["services"]:
		session.create_asset({"type": "service", "subtype": subtype["id"], "name": "Service", "pack": pack})
		_expect(session.params["service_class"] == subtype["id"], "all supported service subtype contracts")
		_expect(not view.rows["extractor_resource"].visible and not view.rows["field_resource"].visible, "services have no resource-area controls")
	session.create_asset({"type": "residential", "name": "House", "pack": pack})
	var original: Dictionary = session.document.snapshot()
	session._review_conversion("commercial", "")
	var conversion: ConfirmationDialog = editor.get_child(editor.get_child_count() - 1)
	_expect(conversion.title == "Confirm type conversion" and conversion.dialog_text.contains("household_capacity"), "conversion explains actual changed fields")
	conversion.canceled.emit()
	conversion.hide()
	_expect(session.document.snapshot() == original, "cancelled conversion leaves document untouched")
	session._review_conversion("commercial", "")
	conversion = editor.get_child(editor.get_child_count() - 1)
	conversion.confirmed.emit()
	conversion.hide()
	_expect(session.descriptor["kind"] == "commercial", "confirmed conversion applies")
	session.undo()
	_expect(session.document.snapshot() == original, "type conversion is reversible including cleared data")
	session.redo()
	_expect(session.descriptor["kind"] == "commercial", "type conversion can be redone")
	session.undo()
	view.show_task("validate")
	var entrance_issue: Button
	for button: Button in view.issues_box.get_children():
		if button.text.begins_with("Main entrance missing"):
			entrance_issue = button
	_expect(entrance_issue != null, "missing entrance produces an actionable issue")
	if entrance_issue != null:
		entrance_issue.pressed.emit()
		_expect(view.task_picker.selected == 2 and view.site_picker.selected == 1, "entrance issue opens access-point authoring")
	editor._menus.actions.create_from_controls("entrance")
	editor._menus.actions.confirm()
	session.capture_geometry("Add main entrance")
	_expect(session.params["anchors"].size() == 1, "access point edits enter the document")
	session.undo()
	_expect(editor._site_anchors_data.is_empty(), "undo does not auto-repair a missing entrance")
	session.redo()
	editor._menus.actions.create_from_controls("parking")
	editor._menus.actions.confirm()
	session.capture_geometry("Add parking")
	_expect(view.anchor_properties.visible and view.rows["_anchor_length"].is_visible_in_tree(), "parking opens its own size properties")
	session.document.begin_transaction("Move parking")
	editor._set_site_anchor_position(1, Vector3(1, 0, 1))
	editor._set_site_anchor_position(1, Vector3(2, 0, 2))
	session.capture_geometry("Move parking")
	session.document.commit_transaction()
	var moved: Dictionary = session.document.snapshot()
	session.undo()
	_expect(session.document.snapshot() != moved, "one undo reverses the entire drag")
	session.redo()
	_expect(session.document.snapshot() == moved, "redo restores the final drag")
	editor._menus.actions.create_from_controls("asphalt")
	editor._menus.actions.confirm()
	session.capture_geometry("Add surface")
	_expect(session.params["site_surfaces"].size() == 1, "surface command is persisted")
	session.undo()
	_expect(editor._site_surfaces_data.is_empty(), "surface creation is reversible")
	session.redo()
	var surface_before: Dictionary = session.document.snapshot()
	var vertex_targets: Array[Dictionary] = [{"kind": "surface", "index": 0}]
	var vertex_result: Dictionary = session.document.prepare_edit("delete_vertex", vertex_targets, {"vertex": 0})
	_expect(not vertex_result.has("error"), "vertex command accepted: " + str(vertex_result.get("error", "")))
	editor._menus.actions.edit("delete_vertex", [{"kind": "surface", "index": 0}], {"vertex": 0})
	_expect(session.params["site_surfaces"][0]["vertices"].size() == 3, "surface context-menu edits update the document")
	session.undo()
	_expect(session.document.snapshot() == surface_before, "surface vertex edits support undo")
	session.guard(func(): _guard_actions += 1)
	_expect(_guard_actions == 0 and session._guard.visible, "dirty document replacement is guarded")
	session._guard.canceled.emit()
	session._guard.hide()
	_expect(_guard_actions == 0, "cancelled guard does not continue")
	session.guard(func(): _guard_actions += 1)
	session._guard.confirmed.emit()
	session._guard.hide()
	_expect(_guard_actions == 1, "explicit discard permits the requested action")
	var path := "user://workspace-test.metrum-draft"
	var unfinished: Dictionary = session.document.snapshot()
	unfinished["params"]["future_metadata"] = {"keep": "unrecognized"}
	unfinished["params"]["economy_profile"] = "missing-profile"
	unfinished["params"]["worker_capacity"] = 77
	session.document.apply(unfinished, "Retain imported metadata")
	session._save_to(path)
	_expect(not session.document.is_dirty() and FileAccess.file_exists(path), "invalid runtime asset is saveable as a draft")
	session.create_asset({"type": "farm", "name": "Different", "pack": pack})
	session.load_draft(path)
	_expect(session.document.snapshot() == unfinished, "reopening a draft preserves all hidden/unresolved data")
	_expect(not view.rows["worker_capacity"].visible and not view.rows["economy_profile"].visible, "preserved legacy metadata does not leak into housing controls")
	_expect(not view.derived_workers.visible, "incompatible legacy profile does not present derived housing jobs")
	var empty_surface := unfinished.duplicate(true)
	empty_surface["params"]["site_surfaces"] = [{"material": "asphalt", "vertices": []}]
	session.document.reset(empty_surface)
	_expect(editor._preview._site_surface_overlay.mesh == null, "incomplete yard has no invalid empty mesh surface")
	_expect(session.document.snapshot() == empty_surface, "rendering an incomplete yard preserves its metadata")
	var malformed := empty_surface.duplicate(true)
	malformed["params"]["lot_width_cells"] = null
	malformed["params"]["household_capacity"] = {"unresolved": true}
	malformed["params"]["tags"] = [1, null, "valid"]
	malformed["params"]["mesh_parts"] = [{"name": "Incomplete", "scale": null, "position": [null, {}, []], "lods": [{"file": "missing.glb", "distance_min_m": null}]}]
	malformed["params"]["anchors"] = [{"position": [null, {}, []], "forward": [null, {}, []]}]
	malformed["params"]["site_surfaces"][0]["vertices"] = [[null, {}]]
	session.document.reset(malformed)
	_expect(session.document.snapshot() == malformed and not session._issues.is_empty(), "invalid draft values remain intact while preview uses safe display defaults")
	session.document.reset(unfinished)
	session.document.mark_saved(path)
	session.guard(func(): _guard_actions += 1)
	_expect(_guard_actions == 2, "unchanged saved draft needs no prompt")
	DirAccess.remove_absolute(path)
	_test_preserved_profile(editor)
	_test_save_guard(editor)
	_test_model_and_publication(editor)
	await _test_library_actions(editor)
	_measure_document_work(editor)
	await _capture_layouts(editor)
	editor.free()
	await process_frame
	if _failures == 0:
		print("PASS asset_workspace_test")
	quit(0 if _failures == 0 else 1)

func _test_library_actions(editor: Node) -> void:
	var session = editor._session
	var before: Dictionary = session.document.snapshot()
	var library = editor._menus.library
	var pack_id := "workspace-publish-%d" % OS.get_process_id()
	var id := pack_id + ":building.residential.residential"
	var manifest: Dictionary = editor._asset_manifest(id)
	manifest["future_metadata"] = {"attribution": "keep me", "values": [null, 3, "four"]}
	var pack := {"pack_id": pack_id, "display_name": "Publication fixtures", "author": "Fixture"}
	var result: Dictionary = AssetAuthoringFiles.copy_for_editing("user://mods", manifest, pack, "building.residential.copy", "Editable copy", "user://asset_editor/copies")
	_expect(not result.has("error"), "independent editable copy: " + str(result.get("error", "")))
	if result.has("error"): return
	var copy: Dictionary = result.document
	_expect(copy.params.future_metadata == manifest.future_metadata and copy.origin.is_empty(), "copy preserves metadata but not publication origin")
	_expect(copy.params.asset_id == "building.residential.copy" and copy.params.pack_id == pack_id, "copy has a new explicit identity")
	var credit: String = copy.supporting_sources.values()[0]
	var pack_bytes := FileAccess.get_file_as_bytes("user://mods/" + pack_id + "/pack.toml")
	_expect(FileAccess.get_file_as_bytes(credit) == pack_bytes, "copy preserves original pack author and licence verbatim")
	var source: Dictionary = library.location(id)
	editor._view.library.show()
	editor._layout.request_layout()
	for frame in 5: await process_frame
	var tree: Tree = editor._view._asset_tree
	var item := tree.get_root().get_first_child()
	while item != null and not (item.get_metadata(0) is String and item.get_metadata(0) == id): item = item.get_next_in_tree()
	_expect(item != null, "published synthetic asset has a library entry")
	if item != null:
		for target in [item, item.get_parent(), item.get_parent().get_parent()]:
			tree.scroll_to_item(target, true)
			await process_frame
			editor._menus.open_library(tree.get_item_area_rect(target, 0).get_center())
			_expect(not editor._menus.context.library.is_empty(), "asset, category and pack rows retain captured metadata: %s rect=%s tree=%s" % [target.get_metadata(0), tree.get_item_area_rect(target, 0), tree.size])
			editor._menus.popup.hide()
		for argument in OS.get_cmdline_user_args():
			if not argument.begins_with("--capture-workspace="): continue
			var directory := argument.trim_prefix("--capture-workspace=")
			DirAccess.make_dir_recursive_absolute(directory)
			for theme in ["light", "dark"]:
				editor.set_ui_theme_mode(theme)
				tree.scroll_to_item(item, true)
				await process_frame
				editor._menus.open_library(tree.get_item_area_rect(item, 0).get_center())
				await process_frame
				_expect(editor._menus.popup.visible and editor._menus.context.library.get("id") == id, "rendered library menu targets the visible asset row")
				await RenderingServer.frame_post_draw
				_expect(root.get_texture().get_image().save_png(directory.path_join(theme + "-library-context.png")) == OK, "capture library context actions")
				editor._menus.popup.hide()
	editor._view.library.hide()
	editor._layout.request_layout()
	var native: String = copy.sources[0][0]
	_expect(FileAccess.file_exists(native) and not native.begins_with(source.path), "copy source files do not depend on the original asset")
	_expect(not DirAccess.dir_exists_absolute("user://mods/" + pack_id + "/assets/building.residential.copy"), "working copy is not published")
	session.document.reset(before, false)
	library.execute("library_copy", {"id": id})
	_expect(session._guard.visible, "editable copy protects a dirty document before opening its dialog")
	session._guard.confirmed.emit()
	var dialog: ConfirmationDialog = editor.get_child(editor.get_child_count() - 1)
	var fields := dialog.find_children("*", "LineEdit", true, false)
	fields[0].text = "UI copy"
	fields[1].text = "building.residential.ui_copy"
	var destinations: OptionButton = dialog.find_children("*", "OptionButton", true, false)[0]
	for index in destinations.item_count:
		if destinations.get_item_metadata(index).pack_id == pack_id: destinations.select(index)
	dialog.confirmed.emit()
	await process_frame
	_expect(session.params.asset_id == "building.residential.ui_copy" and session.document.is_dirty() and session.document.snapshot().origin.is_empty(), "confirmed copy dialog opens an independent unpublished document")
	var ui_copy: Dictionary = session.document.snapshot()
	var export_error := AssetAuthoringFiles.publish_document(JSON.stringify(ui_copy), "user://mods/" + pack_id)
	_expect(export_error.is_empty(), "copied asset can be exported with retained credits: " + export_error)
	var relative: String = ui_copy.supporting_sources.keys()[0]
	_expect(FileAccess.get_file_as_bytes("user://mods/" + pack_id + "/assets/building.residential.ui_copy/" + relative) == pack_bytes, "export includes original pack credits without changing the gameplay manifest")
	session.load_manifest(manifest)
	_expect(library.trash_check(id).has("error"), "currently open asset cannot be trashed")
	session.document.apply(copy, "Switch sources")
	_expect(library.trash_check(id).has("error"), "undo-retained source references prevent trash")
	session.document.reset(copy, false)
	_expect(not library.trash_check(id).has("error"), "independent working copy does not block removal of original")
	var original_bytes := FileAccess.get_file_as_bytes(source.path.path_join("asset.toml"))
	library.trash_operation = func(_path): return ERR_UNAVAILABLE
	_expect(not library.move_to_trash(id).is_empty(), "native trash failure is surfaced")
	_expect(FileAccess.get_file_as_bytes(source.path.path_join("asset.toml")) == original_bytes, "trash failure never falls back to permanent deletion")
	library.trash_operation = OS.move_to_trash
	var collision: Dictionary = AssetAuthoringFiles.copy_for_editing("user://mods", manifest, pack, manifest.asset_id, "Collision", "user://asset_editor/copies")
	_expect(collision.has("error"), "editable copy cannot reuse an existing asset ID")
	var error: String = library.move_to_trash(id)
	_expect(error.is_empty(), "fixture uses recoverable native Trash: " + error)
	_expect(not DirAccess.dir_exists_absolute(source.path) and FileAccess.file_exists(native), "trashing original leaves independent copy files intact")
	_expect(not editor._asset_ids.has(id), "library refresh follows successful trash")
	_expect(session.document.snapshot() == copy and session.document.is_dirty(), "library operations do not publish or mutate the open copy")
	session.document.reset({})
	editor._menus.open_library(Vector2(-1, -1))
	_expect(editor._menus._commands.any(func(command): return command.action == "library_new"), "library context menu works without an open document")
	editor._menus.popup.hide()
	library.execute("library_new", {"pack": pack_id, "type": "industrial"})
	_expect(session._creation.selection().type == "industrial" and session._creation.selection().pack.pack_id == pack_id, "category creation prefills both building type and destination pack")
	session._creation.hide()
	await process_frame
	session.document.reset(before)

func _test_preserved_profile(editor: Node) -> void:
	var session = editor._session
	var before: Dictionary = session.document.snapshot()
	session.resolve_issue({"field": "economy_profile", "section": "gameplay"})
	var confirm: ConfirmationDialog = editor.get_child(editor.get_child_count() - 1)
	_expect(confirm.title == "Resolve preserved metadata", "hidden incompatible metadata has an explicit resolution action")
	confirm.confirmed.emit()
	confirm.hide()
	_expect(session.params["economy_profile"] == null and session.params["worker_capacity"] == 77, "profile resolution changes only the confirmed binding")
	session.undo()
	_expect(session.document.snapshot() == before, "profile resolution preserves unknown data and supports undo")
	var worker_issue: Dictionary = {}
	for issue: Dictionary in session._issues:
		if issue["field"] == "worker_capacity":
			worker_issue = issue
	_expect(worker_issue.has("replacement"), "incompatible hidden employment has a reviewable correction")
	if worker_issue.has("replacement"):
		session.resolve_issue(worker_issue)
		confirm = editor.get_child(editor.get_child_count() - 1)
		confirm.confirmed.emit()
		confirm.hide()
		_expect(session.params["worker_capacity"] == null, "confirmed hidden metadata correction is explicit")
		session.undo()
		_expect(session.document.snapshot() == before, "hidden metadata correction is losslessly undoable")

func _test_save_guard(editor: Node) -> void:
	var session = editor._session
	session.set_field("display_name", "Save before leaving")
	session.guard(func(): _guard_actions += 1)
	session._guard.custom_action.emit("save")
	_expect(_guard_actions == 3 and not session.document.is_dirty(), "save-and-continue writes the draft before proceeding")
	session.create_asset({"type": "residential", "name": "Unsaved", "pack": {"pack_id": "workspace-test"}})
	session.guard(func(): _guard_actions += 1)
	session._guard.custom_action.emit("save")
	var picker: FileDialog = editor.get_child(editor.get_child_count() - 1)
	picker.canceled.emit()
	picker.hide()
	_expect(_guard_actions == 3 and session.document.is_dirty(), "cancelling Save As does not continue or clear dirty state")
	session._guard.canceled.emit()
	session._guard.hide()
	session.guard(func(): _guard_actions += 1)
	session._after_save = func(): _guard_actions += 1
	var existing_directory := "user://workspace-save-failure"
	DirAccess.make_dir_recursive_absolute(existing_directory)
	session._save_to(existing_directory)
	_expect(_guard_actions == 3 and session.document.is_dirty() and not session._after_save.is_valid(), "failed save never runs a deferred destructive action")
	var failure_dialog: AcceptDialog = session._guard.get_child(session._guard.get_child_count() - 1)
	failure_dialog.hide()
	session._guard.canceled.emit()
	session._guard.hide()
	editor._notification(Node.NOTIFICATION_WM_CLOSE_REQUEST)
	_expect(session._guard.visible, "window close is guarded")
	session._guard.canceled.emit()
	session._guard.hide()

func _test_creation_pack_dialog(editor: Node) -> void:
	var session = editor._session
	var before: Dictionary = session.document.snapshot()
	session.document.reset(before, false)
	session.new_asset_dialog()
	_expect(session._guard.visible, "New asset first protects the active dirty document")
	session._guard.confirmed.emit()
	_expect(session._creation.visible, "New asset opens the type-first dialog")
	editor._open_new_pack_dialog()
	_expect(editor._view._pack_create_window.get_parent() == session._creation, "pack creation is nested under the creation modal")
	var id := "workspace-created-%d-%d" % [OS.get_process_id(), Time.get_ticks_usec()]
	editor._view._new_pack_id_edit.text = id
	editor._view._new_pack_name_edit.text = "New workspace pack"
	editor._on_create_pack_pressed()
	_expect(session._creation.selection()["pack"].get("pack_id") == id, "new pack becomes the creation dialog's destination")
	session._creation.hide()
	_expect(session.document.snapshot() == before, "creating a pack then cancelling asset creation leaves the active document untouched")

func _make_model(path: String, width: float) -> void:
	var scene := Node3D.new()
	var mesh := MeshInstance3D.new()
	mesh.mesh = BoxMesh.new()
	mesh.mesh.size = Vector3(width, 6, 4)
	scene.add_child(mesh)
	var exporter := GLTFDocument.new()
	var state := GLTFState.new()
	_expect(exporter.append_from_scene(scene, state) == OK, "generated mesh fixture")
	_expect(exporter.write_to_filesystem(state, path) == OK, "generated GLB fixture")
	scene.free()

func _test_model_and_publication(editor: Node) -> void:
	_test_creation_pack_dialog(editor)
	var session = editor._session
	var pack := {"pack_id": "workspace-publish-%d" % OS.get_process_id(), "display_name": "Workspace publication"}
	_model_path = ProjectSettings.globalize_path("user://workspace-model.glb")
	var replacement := ProjectSettings.globalize_path("user://workspace-replacement.glb")
	_make_model(_model_path, 4)
	_make_model(replacement, 5)
	for kind in ["residential", "commercial", "industrial", "extractor", "farm", "service", "explicit"]:
		session.create_asset({"type": kind, "subtype": "power", "name": kind, "pack": pack, "model": _model_path})
		editor._menus.actions.create_from_controls("entrance")
		editor._menus.actions.confirm()
		session.capture_geometry("Add entrance")
		var profile: String = {"commercial": "grocery_basic", "industrial": "machinery_factory_basic", "extractor": "coal_mine_basic", "farm": "grain_farm_basic", "service": "power_plant_basic"}.get(kind, "")
		if not profile.is_empty():
			session.set_field("economy_profile", profile)
		if kind in ["farm", "extractor"]:
			var field := "field_resource" if kind == "farm" else "extractor_resource"
			var resource: String = session.params[field]
			session.set_field(field, "")
			_expect(session.descriptor["kind"] == kind and editor._view.rows[field].visible, "resource editing retains type and controls")
			session.set_field(field, resource)
		session.publish()
		_expect(session._issues.is_empty(), "runtime publication for %s: %s" % [kind, JSON.stringify(session._issues)])
		var id: String = session.params["asset_id"]
		var loaded = JSON.parse_string(editor.sim.get_asset_manifest_json(pack["pack_id"] + ":" + id))
		_expect(loaded is Dictionary, "published manifest is loadable for " + kind)
		if loaded is Dictionary:
			session.load_manifest(loaded)
			_expect(session.descriptor["kind"] == kind and editor._parts.size() == 1, "runtime roundtrip retains type and mesh for " + kind)
	var state: Dictionary = session.document.snapshot()
	state["params"]["mesh_parts"][0]["position"] = [0.1234, 0.2345, 0.3456]
	state["params"]["mesh_parts"][0]["rotation_degrees"] = [0, 12.3456, 0]
	state["params"]["anchors"][0]["width_m"] = 8.7654
	session.document.apply(state, "Loaded precise metadata")
	editor._select_site_anchor(0)
	editor._view._site_anchor_y_spin.value = 2
	_expect(session.params["anchors"][0]["width_m"] == 8.7654, "entrance coordinate edits preserve dormant width metadata")
	editor._select_mesh_part(0)
	var before: Dictionary = session.document.snapshot()
	session.document.begin_transaction("Move mesh")
	editor._view._part_x_spin.value = 1
	editor._view._part_x_spin.value = 2
	session.document.commit_transaction()
	_expect(is_equal_approx(editor._parts[0].position.y, 0.2345) and is_equal_approx(editor._parts[0].rotation_y, 12.3456), "editing X does not round Y or rotation")
	session.undo()
	_expect(session.document.snapshot() == before, "one undo reverses grouped transform edits exactly")
	session.redo()
	var transformed: Dictionary = session.document.snapshot()
	session.replace_part_source(0, replacement)
	_expect(session.params["mesh_parts"][0]["position"] == transformed["params"]["mesh_parts"][0]["position"], "relink preserves placement")
	session.undo()
	_expect(session.document.snapshot() == transformed, "relink is undoable")
	var missing := transformed.duplicate(true)
	missing["sources"][0][0] = ProjectSettings.globalize_path("user://missing.glb")
	session.document.reset(missing)
	_expect(editor._parts.size() == 1 and editor._preview.mesh_part_count() == 1, "missing source reserves its part index")
	_expect(session.document.snapshot() == missing, "opening missing sources never repairs or deletes authored metadata")
	session.replace_part_source(0, replacement)
	_expect(editor._parts[0].aabb.size.x > 0 and session.params["mesh_parts"][0]["position"] == missing["params"]["mesh_parts"][0]["position"], "missing source can be repaired without moving it")
	missing.erase("sources")
	session.document.reset(missing)
	session.replace_part_source(0, replacement)
	_expect(session.document.snapshot()["sources"] == [[replacement]], "relink initializes omitted source metadata in incomplete drafts")
	session.undo()
	_expect(session.document.snapshot() == missing, "undo relink restores omitted source metadata exactly")
	session.redo()
	editor._view.show_task("model")
	_expect(editor._view._preview_panel._lod_list.is_visible_in_tree(), "LOD chain is visible directly in Model")
	_expect(not editor._view.advanced_groups.has("materials"), "Model does not expose the removed source-material summary")
	var placement: Dictionary = editor._view.advanced_groups["placement"]
	placement["toggle"].button_pressed = true
	_expect(placement["body"].is_visible_in_tree(), "Model expands Placement")
	_expect(editor._view._preview_panel._add.is_visible_in_tree(), "expanding properties never hides Add LOD")
	placement["toggle"].button_pressed = false

func _capture_layouts(editor: Node) -> void:
	for argument in OS.get_cmdline_user_args():
		if not argument.begins_with("--capture-workspace="):
			continue
		var directory := argument.trim_prefix("--capture-workspace=")
		DirAccess.make_dir_recursive_absolute(directory)
		root.size = Vector2i(1280, 900)
		await process_frame
		editor._session.create_asset({"type": "residential", "name": "Residential house", "pack": {"pack_id": "workspace-test", "display_name": "Workspace test"}, "model": _model_path})
		editor._menus.actions.create_from_controls("entrance")
		editor._menus.actions.confirm()
		editor._session.capture_geometry("Add entrance")
		editor._select_mesh_part(0)
		editor._frame_selected_part()
		for mode in ["dark", "light"]:
			editor.set_ui_theme_mode(mode)
			for task in ["overview", "gameplay", "site", "model", "validate"]:
				editor._view.show_task(task)
				await process_frame
				await RenderingServer.frame_post_draw
				_expect(root.get_texture().get_image().save_png(directory.path_join(mode + "-" + task + ".png")) == OK, "capture task layout")
				_expect(editor._view._theme_root.position.y >= editor._top_menu._shell.size.y, "toolbar never overlaps the menu")
			editor._view.show_task("model")
			editor._view.advanced_groups["placement"]["toggle"].button_pressed = true
			await process_frame
			await RenderingServer.frame_post_draw
			_expect(root.get_texture().get_image().save_png(directory.path_join(mode + "-model-placement.png")) == OK, "capture contextual model properties")
			editor._view.advanced_groups["placement"]["toggle"].button_pressed = false
		editor._view.open_preview_popup()
		await process_frame
		await RenderingServer.frame_post_draw
		_expect(editor._view._preview_panel._quality.is_visible_in_tree(), "quality controls live in the working viewport popup")
		_expect(editor._view.preview_popup.size.y < root.size.y, "preview dialog fits the window without stale wrapped-text height")
		_expect(root.get_texture().get_image().save_png(directory.path_join("preview-popup.png")) == OK, "capture preview popup")
		editor._view.preview_popup.hide()
		editor._session.new_asset_dialog()
		if is_instance_valid(editor._session._guard) and editor._session._guard.visible:
			editor._session._guard.confirmed.emit()
		await process_frame
		await RenderingServer.frame_post_draw
		_expect(editor._session._creation.size.y < root.size.y, "creation dialog keeps its action buttons within the window")
		_expect(root.get_texture().get_image().save_png(directory.path_join("creation.png")) == OK, "capture type-first creation")
		editor._open_new_pack_dialog()
		await process_frame
		await RenderingServer.frame_post_draw
		_expect(root.get_texture().get_image().save_png(directory.path_join("new-pack.png")) == OK, "capture separate pack administration")
		editor._view._pack_create_window.hide()
		editor._session._creation.hide()
		await editor._session.capture_thumbnail()
		_expect(editor._view.thumbnail.texture != null, "thumbnail captures the rendered preview")
		var thumbnail_state: Dictionary = editor._session.document.snapshot()
		editor._session.undo()
		_expect(editor._view.thumbnail.texture == null, "thumbnail undo restores the previous image")
		editor._session.redo()
		editor._session._save_to("user://workspace-thumbnail.metrum-draft")
		editor._session.load_draft("user://workspace-thumbnail.metrum-draft")
		_expect(editor._session.document.snapshot() == thumbnail_state and editor._view.thumbnail.texture != null, "draft reopening restores the thumbnail")
		editor._session.publish()
		var loaded: Dictionary = JSON.parse_string(editor.sim.get_asset_manifest_json("workspace-test:building.residential.residential_house"))
		editor._session.load_manifest(loaded)
		_expect(editor._view.thumbnail.texture != null, "published asset reopening restores the packaged thumbnail")
		root.size = Vector2i(960, 640)
		await process_frame
		editor._frame_selected_part()
		await RenderingServer.frame_post_draw
		_expect(root.get_texture().get_image().save_png(directory.path_join("narrow-overview.png")) == OK, "capture minimum window layout")
		var camera: Camera3D = editor.get_node("CameraNode")
		var projected := camera.unproject_position(camera.get_focus_position())
		_expect(projected.distance_to(editor._view._preview_view_rect.get_global_rect().get_center()) < 2.0, "framing centers the asset in the visible pane, not behind the inspector")

func _measure_document_work(editor: Node) -> void:
	if not "--benchmark-asset-workspace" in OS.get_cmdline_user_args():
		return
	var session = editor._session
	var node_id: int = editor._preview._mesh_parts[0].get_instance_id()
	var imports: int = editor._preview.lod_import_count
	var started := Time.get_ticks_usec()
	for index in 100:
		session.set_field("display_name", "Measured edit %d" % index)
	var edit_us := (Time.get_ticks_usec() - started) / 100.0
	_expect(editor._preview._mesh_parts[0].get_instance_id() == node_id and editor._preview.lod_import_count == imports, "metadata commands never reload model resources")
	var camera: Camera3D = editor.get_node("CameraNode")
	editor._cam_input._update_preview_offset()
	editor._lod_preview.update(camera)
	var evaluations: int = editor._lod_preview.evaluations
	started = Time.get_ticks_usec()
	for index in 10000:
		editor._cam_input._update_preview_offset()
		editor._lod_preview.update(camera)
	var idle_us := (Time.get_ticks_usec() - started) / 10000.0
	_expect(editor._lod_preview.evaluations == evaluations, "idle preview performs no per-part policy evaluations")
	print("asset_workspace_measure metadata_edit_mean_us=%.3f idle_update_mean_us=%.3f imports=%d edit_iterations=100 idle_iterations=10000 synthetic_box=true" % [edit_us, idle_us, editor._preview.lod_import_count - imports])
