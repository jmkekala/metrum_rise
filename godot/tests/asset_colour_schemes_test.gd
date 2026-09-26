# SPDX-License-Identifier: GPL-2.0-only

## Generated colour authoring regression. Reuses the preview fixture harness, never real packs.
extends "res://tests/asset_editor_preview_test.gd"

func _colour_model(level: int) -> String:
	var original := _source.path_join("generated%d.glb" % level)
	_write_model(original, level)
	var bytes := FileAccess.get_file_as_bytes(original)
	var gltf: Dictionary = JSON.parse_string(AssetAuthoringFiles.read_gltf_json(original))
	var buffer_name := "geometry%d.bin" % level
	var file := FileAccess.open(_source.path_join(buffer_name), FileAccess.WRITE)
	var buffer_start := 20 + bytes.decode_u32(12) + 8
	file.store_buffer(bytes.slice(buffer_start, buffer_start + int(gltf["buffers"][0]["byteLength"])))
	file.close()
	gltf["buffers"][0]["uri"] = buffer_name
	gltf["materials"][0]["name"] = "walls" if level == 0 else "walls_low"
	var texture_index: int = gltf["textures"].size()
	gltf["textures"].append({"source": gltf["images"].size()})
	gltf["images"].append({"uri": "house_albedo_red.png"})
	gltf["materials"][0]["pbrMetallicRoughness"]["baseColorTexture"] = {"index": texture_index}
	var path := _source.path_join("house_lod%d.gltf" % level)
	_write(path, JSON.stringify(gltf))
	return path

func _run() -> void:
	_pack = "editor-colour-test-%d" % OS.get_process_id()
	_source = ProjectSettings.globalize_path("user://" + _pack + "-source")
	_output = ProjectSettings.globalize_path("user://mods/" + _pack)
	DirAccess.make_dir_recursive_absolute(_source)
	for variant in ["red", "blue"]:
		var image := Image.create(8, 8, false, Image.FORMAT_RGB8)
		image.fill(Color.RED if variant == "red" else Color.BLUE)
		_expect(image.save_png(_source.path_join("house_albedo_%s.png" % variant)) == OK, "colour texture fixture")
	var paths: Array[String] = [_colour_model(0), _colour_model(1)]
	var editor := TestEditor.new()
	var simulation := SimulationNode.new()
	simulation.name = "SimulationNode"
	editor.add_child(simulation)
	var camera := CameraNode.new()
	camera.name = "CameraNode"
	editor.add_child(camera)
	var world := WorldEnvironment.new()
	world.name = "WorldEnvironment"
	world.environment = Environment.new()
	editor.add_child(world)
	var sun := DirectionalLight3D.new()
	sun.name = "DirectionalLight3D"
	editor.add_child(sun)
	root.add_child(editor)
	await process_frame
	editor._session.create_asset({"type": "residential", "name": "Colours", "pack": {"pack_id": _pack, "display_name": "Colour test", "author": "Test"}})
	editor._menus.actions.create_from_controls("entrance")
	editor._menus.actions.confirm()
	editor._on_glb_file_selected(paths[0])
	var state: Dictionary = editor._session.document.snapshot()
	state["params"]["asset_id"] = "building.residential.colours"
	state["params"]["mesh_parts"][0]["lods"][0]["distance_max_m"] = 35.0
	state["params"]["mesh_parts"][0]["lods"].append({"file": paths[1].get_file(), "distance_min_m": 35.0})
	state["sources"][0].append(paths[1])
	editor._session.document.apply(state, "Add fixture LOD")
	var colours = editor._session.colours
	_expect(colours._hint.visible, "import exposes non-blocking discovery prompt")
	_expect(editor._session.params.get("appearance") == null, "discovery prompt does not silently author schemes")
	colours._override_dialog(-1, true)
	var dialog: ConfirmationDialog
	for child in editor.get_children():
		if child is ConfirmationDialog and child.title == "Discover colour schemes":
			dialog = child
	_expect(dialog != null, "discovery opens review dialog")
	if dialog == null:
		quit(1)
		return
	var body: VBoxContainer = dialog.get_meta("body")
	var mappings: VBoxContainer = body.get_child(2)
	var unset := 0
	for child in mappings.get_children():
		if child is OptionButton and child.selected <= 0:
			unset += 1
	_expect(unset == 0, "a sole source material fills every LOD without asking")
	for child in mappings.get_children():
		if child is OptionButton:
			child.select(1)
			child.item_selected.emit(1)
	_expect(editor._session.params.get("appearance") == null, "reviewing candidates is read-only")
	dialog.confirmed.emit()
	await process_frame
	_expect(editor._session.params.get("appearance") != null, "confirmation adds schemes")
	if editor._session.params.get("appearance") == null:
		quit(1)
		return
	_expect(editor._session.params["appearance"]["schemes"].size() == 2, "both confirmed variants are authored")
	_expect(editor._session.params["appearance"]["default_scheme"] == "red", "discovery preserves the source colour as default")
	# Publication copies only the referenced albedo, so a reopened asset must be able to scan
	# the modelling folder that still holds the alternatives.
	var elsewhere := _source.path_join("elsewhere")
	DirAccess.make_dir_recursive_absolute(elsewhere)
	for variant in ["red", "green"]:
		var moved := Image.create(8, 8, false, Image.FORMAT_RGB8)
		moved.fill(Color.GREEN)
		_expect(moved.save_png(elsewhere.path_join("house_albedo_%s.png" % variant)) == OK, "scan fixture")
	var scanned: Dictionary = JSON.parse_string(AssetAuthoringPolicy.discover_colours_in_json(_source.path_join("house_albedo_red.png"), elsewhere))
	var scanned_ids: Array = []
	for candidate: Dictionary in scanned.get("candidates", []):
		scanned_ids.append(str(candidate["id"]))
		_expect(str(candidate["path"]).begins_with(elsewhere), "scanned candidates resolve inside the chosen folder")
	_expect(scanned_ids == ["green", "red"], "scanning another folder discovers its alternatives: " + JSON.stringify(scanned_ids))
	var default_id: String = editor._session.params["appearance"]["default_scheme"]
	var imports: int = editor._preview.lod_import_count
	var picking: int = editor._preview.pick_build_count
	var revision: int = editor._preview.lod_revision
	for id in ["red", "blue", "red"]:
		colours._selected = id
		colours._apply_preview()
		for level in 2:
			_expect(editor._preview.set_mesh_part_lod(0, paths[level]), "select cached tier")
			var materials = editor._preview.mesh_part_materials(0)
			var surface: Dictionary = materials._surfaces[0]
			var source: BaseMaterial3D = surface["source"]
			var active: Material = surface["instance"].get_active_material(surface["surface"])
			var albedo: Texture2D = active.get_shader_parameter("texture_albedo") if active is ShaderMaterial else active.albedo_texture
			_expect(albedo != source.albedo_texture, "scheme uses preview-owned texture")
			var pixel: Color = albedo.get_image().get_pixel(0, 0)
			_expect(pixel.is_equal_approx(Color.RED if id == "red" else Color.BLUE), "scheme follows all LODs")
			editor._preview.set_preview_emission(PreviewMaterials.Mode.ON, 2.0)
			active = surface["instance"].get_active_material(surface["surface"])
			_expect(active.emission_texture == source.emission_texture and active.emission_enabled, "albedo switch preserves illuminated windows")
			editor._preview.set_preview_emission(PreviewMaterials.Mode.OFF, 1.0)
			active = surface["instance"].get_active_material(surface["surface"])
			_expect(not active.emission_enabled, "day preview can disable emission without losing colour")
	_expect(editor._session.params["appearance"]["default_scheme"] == default_id, "preview does not change authored default")
	_expect(editor._preview.lod_import_count == imports and editor._preview.pick_build_count == picking and editor._preview.lod_revision == revision, "colour switches preserve geometry and picking caches")
	var texture: Texture2D = colours._textures.values()[0]
	state = editor._session.document.snapshot()
	state["params"]["mesh_parts"][0]["position"][0] = 0.1
	editor._session.document.apply(state, "Move fixture")
	_expect(colours._textures.values()[0] == texture, "placement changes retain texture cache")
	editor._session.document.undo()
	editor._session.document.undo()
	_expect(editor._session.params.get("appearance") == null, "discovery undo removes schemes in one action")
	editor._session.document.redo()
	_expect(editor._session.params["appearance"]["schemes"].size() == 2, "redo restores schemes")
	await _test_manual(editor)
	var authored: Dictionary = editor._session.params["appearance"].duplicate(true)
	editor._session.publish()
	_expect(editor._session._issues.is_empty(), "colour export validation: " + JSON.stringify(editor._session._issues))
	var manifest: Dictionary = JSON.parse_string(editor.sim.get_asset_manifest_json(_pack + ":building.residential.colours"))
	_expect(manifest.get("appearance") == authored, "export and registry preserve all schemes")
	editor._session.load_manifest(manifest)
	_expect(editor._session.colours.error().is_empty(), "reopened schemes have valid texture sources")
	_expect(editor._session.params["appearance"] == authored, "reopen restores editable schemes")
	var draft := _source.path_join("colours.draft")
	var saved := AssetAuthoringFiles.save_draft(draft, editor._session.document.snapshot())
	_expect(saved.is_empty(), "draft save supports scheme metadata")
	var loaded := AssetAuthoringFiles.load_draft(draft)
	_expect(loaded.get("document", {}).get("params", {}).get("appearance") == authored, "draft round-trip preserves schemes")
	await _capture_colours(editor)
	editor.queue_free()
	await process_frame
	print("asset_colour_schemes_test: %s" % ("PASS" if _failures == 0 else "FAIL"))
	quit(_failures)

func _find_dialog(editor: Node, title: String) -> ConfirmationDialog:
	for child in editor.get_children():
		if child is ConfirmationDialog and child.title == title and child.visible:
			return child
	return null

func _test_manual(editor: Node) -> void:
	var colours = editor._session.colours
	colours._add_scheme()
	var dialog := _find_dialog(editor, "Add colour scheme")
	var body: VBoxContainer = dialog.get_meta("body")
	body.get_child(0).text = "Garden green"
	body.get_child(1).text = "garden"
	dialog.confirmed.emit()
	await process_frame
	_expect(colours._selected == "garden", "manual Add selects the new scheme without changing default")
	var image := Image.create(8, 8, false, Image.FORMAT_RGB8)
	image.fill(Color.GREEN)
	var arbitrary := _source.path_join("arbitrary-name.png")
	image.save_png(arbitrary)
	colours._override_dialog(-1, false)
	dialog = _find_dialog(editor, "Material texture override")
	body = dialog.get_meta("body")
	for child in body.get_child(2).get_children():
		if child is OptionButton:
			child.select(1)
	for button in body.find_children("*", "Button", true, false):
		if button.text == "Browse…":
			button.pressed.emit()
			break
	for child in dialog.get_children():
		if child is FileDialog and child.visible:
			child.file_selected.emit(arbitrary)
			child.hide()
	dialog.confirmed.emit()
	await process_frame
	_expect(not is_instance_valid(dialog), "manual mapping and arbitrary texture are accepted")
	var scheme: Dictionary = editor._session.params["appearance"]["schemes"][2]
	_expect(scheme["overrides"].size() == 1 and scheme["overrides"][0]["albedo"].ends_with("arbitrary-name.png"), "manual override retains arbitrary filename")
	for button in colours._details.find_children("*", "Button", true, false):
		if button.text == "Set as default":
			button.pressed.emit()
			break
	_expect(editor._session.params["appearance"]["default_scheme"] == "garden", "explicit default action authors the selected scheme")
	# Several schemes usually mean a varied street, so discovery authors random spawning and a
	# branded asset opts out. Both directions are authored, not just the one leaving the default.
	_expect(editor._session.params["appearance"]["spawn"] == "random_scheme", "discovery authors random spawning by default")
	_expect(colours._spawn.selected == 1, "the spawn control shows the authored default")
	colours._spawn.item_selected.emit(0)
	_expect(editor._session.params["appearance"]["spawn"] == "default_only", "a branded asset can pin one colour")
	colours._spawn.item_selected.emit(1)
	_expect(editor._session.params["appearance"]["spawn"] == "random_scheme", "spawn policy metadata is authored")
	colours._remove_scheme()
	_expect(editor._session.params["appearance"]["schemes"].size() == 2, "remove scheme")
	editor._session.document.undo()
	_expect(editor._session.params["appearance"]["default_scheme"] == "garden", "undo restores removed default and its textures")

func _capture_colours(editor: Node) -> void:
	for argument in OS.get_cmdline_user_args():
		if not argument.begins_with("--capture-colours="):
			continue
		var directory := argument.trim_prefix("--capture-colours=")
		editor._view.show_task("model")
		editor._frame_selected_part()
		await process_frame
		var ancestor: Node = editor._session.colours._box.get_parent()
		while ancestor != null and not ancestor is ScrollContainer:
			ancestor = ancestor.get_parent()
		if ancestor is ScrollContainer:
			ancestor.ensure_control_visible(editor._session.colours._details)
		for theme in ["light", "dark"]:
			editor.set_ui_theme_mode(theme)
			for preset in ["Day", "Night"]:
				editor._view._preview_panel.find_child(preset, true, false).pressed.emit()
				await process_frame
				await process_frame
				await RenderingServer.frame_post_draw
				_expect(root.get_texture().get_image().save_png(directory.path_join("colours-%s-%s.png" % [theme, preset.to_lower()])) == OK, "rendered colour UI capture")
