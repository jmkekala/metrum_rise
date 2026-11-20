# SPDX-License-Identifier: GPL-3.0-or-later

## Asset library rows from Rust (`TOOLS-34`): manifest groups, one row per upgrade family with
## clickable level chips, a refused Delete on a family row, search on level and group fields,
## and the groups no pack fills.
## Fixture packs live in the isolated test profile; optional `--capture-browser=<dir>` stills.
extends SceneTree

const EditorScene = preload("res://scenes/AssetEditor.tscn")

var _failures := 0
var _capture_dir := ""

func _initialize() -> void:
	for argument in OS.get_cmdline_user_args():
		if argument.begins_with("--capture-browser="):
			_capture_dir = argument.trim_prefix("--capture-browser=")
	call_deferred("_run")

func _expect(condition: bool, message: String) -> void:
	if not condition:
		_failures += 1
		push_error(message)

func _write(path: String, content: String) -> void:
	DirAccess.make_dir_recursive_absolute(path.get_base_dir())
	var file := FileAccess.open(path, FileAccess.WRITE)
	_expect(file != null, "fixture file opens: " + path)
	if file:
		file.store_string(content)

func _write_model(path: String) -> void:
	var scene := Node3D.new()
	var instance := MeshInstance3D.new()
	instance.mesh = BoxMesh.new()
	scene.add_child(instance)
	var document := GLTFDocument.new()
	var state := GLTFState.new()
	_expect(document.append_from_scene(scene, state) == OK, "fixture model exports")
	_expect(document.write_to_filesystem(state, path) == OK, "fixture model writes")
	scene.free()

# One asset folder with its own model; `head` and `building` add manifest lines.
func _asset(pack_dir: String, id: String, name: String, head: String, building: String) -> void:
	var dir := pack_dir.path_join("assets").path_join(id)
	DirAccess.make_dir_recursive_absolute(dir)
	_write_model(dir.path_join("model.glb"))
	_write(dir.path_join("asset.toml"), """asset_id = "%s"
display_name = "%s"
%s
[building]
lot_width_cells = 2
lot_depth_cells = 2
%s
[[mesh_parts]]
name = "main"
[[mesh_parts.lods]]
file = "model.glb"
distance_min_m = 0.0
[[anchors]]
type = "entrance"
name = "main"
position = [0.0, 0.0, 1.0]
forward = [0.0, 0.0, 1.0]
""" % [id, name, head, building])

func _house(pack_dir: String, id: String, name: String, family: String, level: int) -> void:
	var head := "asset_set = \"%s\"" % family if not family.is_empty() else ""
	_asset(pack_dir, id, name, head, "zone_type = \"residential\"\ndensity = \"low\"\nlevel = %d\nhousehold_capacity = 1" % level)

func _children(item: TreeItem) -> Array[TreeItem]:
	var result: Array[TreeItem] = []
	var child := item.get_first_child() if item != null else null
	while child != null:
		result.append(child)
		child = child.get_next()
	return result

func _texts(items: Array[TreeItem]) -> Array[String]:
	var result: Array[String] = []
	for item in items:
		result.append(item.get_text(0))
	return result

func _pack_item(tree: Tree, pack_id: String) -> TreeItem:
	for item in _children(tree.get_root()):
		if item.get_text(0).begins_with(pack_id + " ("):
			return item
	return null

func _settle() -> void:
	for frame in 4:
		await process_frame

# Through Input in window coordinates, as a window delivers it: Tree reports a button only on
# that path, not for an event pushed into the viewport.
func _click(tree: Tree, local: Vector2) -> void:
	var point := root.get_final_transform() * (tree.get_global_transform_with_canvas() * local)
	var motion := InputEventMouseMotion.new()
	motion.position = point
	motion.global_position = point
	Input.parse_input_event(motion)
	await process_frame
	for pressed in [true, false]:
		var event := InputEventMouseButton.new()
		event.button_index = MOUSE_BUTTON_LEFT
		event.pressed = pressed
		event.position = point
		event.global_position = point
		Input.parse_input_event(event)
		await process_frame

func _trash_dialog(editor: Node) -> ConfirmationDialog:
	for dialog: ConfirmationDialog in editor.find_children("*", "ConfirmationDialog", false, false):
		if dialog.visible and dialog.title == "Move asset to Trash?":
			return dialog
	return null

func _capture(name: String) -> void:
	if _capture_dir.is_empty():
		return
	DirAccess.make_dir_recursive_absolute(_capture_dir)
	await process_frame
	await RenderingServer.frame_post_draw
	_expect(root.get_texture().get_image().save_png(_capture_dir.path_join(name + ".png")) == OK, "rendered capture: " + name)

func _run() -> void:
	var pack_id := "browser-test-%d" % OS.get_process_id()
	var pack_dir := ProjectSettings.globalize_path("user://mods/" + pack_id)
	_write(pack_dir.path_join("pack.toml"), """pack_id = "%s"
schema_version = 1
display_name = "Browser test"
version = "1.0.0"
author = "Test"
license = "CC0"
""" % pack_id)
	# Two levels of one family share a display name, as do two unfamilied cottages.
	_house(pack_dir, "building.residential.house_1", "House 1", "browser_house_1", 1)
	_house(pack_dir, "building.residential.house_1_level_2", "House 1", "browser_house_1", 2)
	_house(pack_dir, "building.residential.cottage", "Cottage", "", 1)
	_house(pack_dir, "building.residential.cottage_2", "Cottage", "", 2)
	# The id says commercial; the manifest's explicit fire service decides the group.
	_asset(pack_dir, "building.commercial.station", "Station", "", "placement_mode = \"explicit\"\nservice_class = \"fire\"\nworker_capacity = 0")
	var family_1 := pack_id + ":building.residential.house_1"
	var family_2 := pack_id + ":building.residential.house_1_level_2"

	var editor = EditorScene.instantiate()
	root.add_child(editor)
	await _settle()
	editor._view.toggle_library()
	editor._layout.request_layout()
	await _settle()
	var tree: Tree = editor._view._asset_tree
	var pack := _pack_item(tree, pack_id)
	_expect(pack != null and pack.get_text(0) == pack_id + " (5)", "the pack counts its assets: " + (pack.get_text(0) if pack else "missing"))
	if pack == null:
		quit(1)
		return
	_expect(_texts(_children(pack)) == ["Residential · low density (4)", "Service · Fire (1)"], "groups come from manifest fields: %s" % [_texts(_children(pack))])
	var houses := _children(_children(pack)[0])
	_expect(_texts(houses) == ["Cottage", "Cottage", "House 1"], "one row per family, unfamilied namesakes apart: %s" % [_texts(houses)])
	_expect(_children(pack)[0].get_metadata(0) == {"pack": pack_id, "type": "residential"}, "group rows keep the pack and creation type")
	var chips: Array[int] = []
	for row in houses:
		chips.append(row.get_button_count(0))
	_expect(chips == [1, 1, 2], "every building row has its level chips: %s" % [chips])
	_expect(houses[0].get_button_tooltip_text(0, 0).begins_with("Level 1 ") and houses[1].get_button_tooltip_text(0, 0).begins_with("Level 2 "), "namesakes differ by their level chip")
	var family := houses[2]
	_expect(family.get_metadata(0) == family_1 and family.get_tooltip_text(0).begins_with(family_1), "the family row opens and names its lowest level")
	_expect(editor._asset_browser_label(family_2) == "House 1 · L2", "menus name the level: " + editor._asset_browser_label(family_2))

	tree.scroll_to_item(family, true)
	await _settle()
	var row_rect := tree.get_item_area_rect(family, 0)
	var chip_rect := tree.get_item_area_rect(family, 0, 1)
	_expect(row_rect.encloses(chip_rect) and chip_rect.size.x > 0.0, "the L2 chip sits inside its row: %s in %s" % [chip_rect, row_rect])
	_expect(tree.get_button_id_at_position(chip_rect.get_center()) == 1, "Tree hit-tests the L2 chip")
	editor._menus.open_library(chip_rect.get_center())
	_expect(editor._menus.context.library.get("id") == family_2, "a chip's context menu targets its level")
	editor._menus.popup.hide()

	# Delete never guesses a level: a family row is refused with a hint, a single-level row
	# still asks to move its asset to Trash.
	var delete := InputEventKey.new()
	delete.keycode = KEY_DELETE
	delete.pressed = true
	family.select(0)
	tree.gui_input.emit(delete)
	var status: Label = editor._view._asset_status_lbl
	_expect(status.visible and status.text.contains("level chip") and _trash_dialog(editor) == null, "Delete on a family row is refused: " + status.text)
	_expect(editor._asset_ids.has(family_1) and editor._asset_ids.has(family_2), "the refused Delete keeps both levels")
	houses[0].select(0)
	tree.gui_input.emit(delete)
	var dialog := _trash_dialog(editor)
	_expect(dialog != null and dialog.dialog_text.contains(houses[0].get_metadata(0)), "Delete on a single-level row asks to move it to Trash")
	if dialog != null:
		dialog.canceled.emit()
		dialog.hide()
	await _settle()
	await _capture("library")
	# Stills of the last pack and of the missing section, for packs copied in with `--mods`.
	if not _capture_dir.is_empty():
		var headings := _children(tree.get_root())
		for index in [headings.size() - 2, headings.size() - 1]:
			tree.scroll_to_item(headings[index], true)
			await _capture("library-top-%d" % index)
		tree.scroll_to_item(family, true)
		await _settle()
	await _click(tree, chip_rect.get_center())
	await _settle()
	_expect(editor._session.has_document and editor._session.params.get("asset_id") == "building.residential.house_1_level_2", "clicking the L2 chip opens level 2: %s" % editor._session.params.get("asset_id"))
	family.select(0)
	tree.item_activated.emit()
	await _settle()
	_expect(editor._session.params.get("asset_id") == "building.residential.house_1", "activating the family row opens level 1")

	# Groups no pack fills follow the packs while no search is given.
	var top := _children(tree.get_root())
	var missing := top[top.size() - 1]
	_expect(missing.get_text(0).begins_with("No assets yet (") and not missing.is_selectable(0), "the missing groups are one labelled section after the packs")
	var stocked := {}
	for pack_item in top.slice(0, top.size() - 1):
		for group in _children(pack_item):
			stocked[group.get_text(0).get_slice(" (", 0)] = true
	for group in _children(missing):
		_expect(group.get_text(0).ends_with(" (0)") and not group.is_selectable(0), "a missing group is a greyed zero row: " + group.get_text(0))
		_expect(not stocked.has(group.get_text(0).trim_suffix(" (0)")), "a group some pack fills is never missing: " + group.get_text(0))
	_expect((not stocked.has("Parking lots")) == _texts(_children(missing)).has("Parking lots (0)"), "parking lots are missing exactly when no pack has one")

	for query in ["L2", "level 2"]:
		editor._view._asset_search_edit.text = query
		editor._on_asset_search_changed(query)
		pack = _pack_item(tree, pack_id)
		var rows := _children(_children(pack)[0]) if pack != null else []
		_expect(_texts(rows) == ["Cottage", "House 1"] and rows[1].get_button_count(0) == 1, "%s keeps level-2 chips only: %s" % [query, _texts(rows)])
		top = _children(tree.get_root())
		_expect(not top[top.size() - 1].get_text(0).begins_with("No assets yet"), "a search shows matches only")
	editor._view._asset_search_edit.text = "fire"
	editor._on_asset_search_changed("fire")
	pack = _pack_item(tree, pack_id)
	_expect(pack != null and _texts(_children(pack)) == ["Service · Fire (1)"], "search matches the service class")
	var matching := 0
	for pack_item in _children(tree.get_root()):
		matching += int(pack_item.get_text(0).get_slice(" (", 1).trim_suffix(")"))
	_expect(editor._view._asset_count_lbl.text.begins_with("%d / " % matching), "the count shows matching assets: " + editor._view._asset_count_lbl.text)
	editor._view._asset_search_edit.text = ""
	editor._on_asset_search_changed("")
	await _capture("library-search-cleared")

	editor.free()
	await process_frame
	quit(0 if _failures == 0 else 1)
