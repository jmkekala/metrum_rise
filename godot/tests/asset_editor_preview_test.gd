# SPDX-License-Identifier: GPL-2.0-only

## Asset editor regression: real UI, four-tier round-trip, dependency-safe saves and lighting.
## Run with --asset-editor; fixtures use a process-unique pack and never edit source assets.
extends SceneTree

const Editor = preload("res://scripts/editors/asset_editor.gd")
const MeshPart = preload("res://scripts/editors/asset_editor/mesh_part.gd")
const PreviewMaterials = preload("res://scripts/editors/asset_editor/preview_materials.gd")
const ThumbnailCapture = preload("res://scripts/editors/asset_editor/thumbnail_capture.gd")

class TestEditor extends Editor:
	var config_save_count := 0
	func _save_config() -> void:
		config_save_count += 1
	func _save_layout_state() -> void:
		pass
	func _restore_window_geometry() -> void:
		pass

var _failures := 0
var _source: String
var _pack: String
var _output: String

func _initialize() -> void:
	call_deferred("_run")

func _expect(condition: bool, message: String) -> void:
	if not condition:
		_failures += 1
		push_error(message)

func _write(path: String, text: String) -> void:
	var file := FileAccess.open(path, FileAccess.WRITE)
	_expect(file != null, "fixture must open: " + path)
	if file:
		file.store_string(text)

func _write_model(path: String, level: int) -> void:
	var scene := Node3D.new()
	var instance := MeshInstance3D.new()
	instance.mesh = BoxMesh.new()
	instance.mesh.size = Vector3(2.0 - level * 0.1, 3.0, 2.0)
	instance.mesh.subdivide_height = 3 - level
	var material := StandardMaterial3D.new()
	material.emission_enabled = true
	material.emission = Color.WHITE
	var image := Image.create(4, 4, false, Image.FORMAT_RGB8)
	image.fill(Color.WHITE)
	material.emission_texture = ImageTexture.create_from_image(image)
	instance.mesh.surface_set_material(0, material)
	scene.add_child(instance)
	var document := GLTFDocument.new()
	var state := GLTFState.new()
	_expect(document.append_from_scene(scene, state) == OK, "fixture scene export")
	_expect(document.write_to_filesystem(state, path) == OK, "fixture GLB export")
	scene.free()
	# Export a default-off texture binding exactly as the authored model contract requires.
	var data := FileAccess.get_file_as_bytes(path)
	var gltf: Dictionary = JSON.parse_string(AssetAuthoringFiles.read_gltf_json(path))
	var texture_name := "window_%d.png" % level
	_expect(image.save_png(path.get_base_dir().path_join(texture_name)) == OK, "external texture export")
	for entry: Dictionary in gltf["images"]:
		entry.erase("bufferView")
		entry["uri"] = texture_name
	gltf["materials"][0]["emissiveFactor"] = [0.0, 0.0, 0.0]
	var json := JSON.stringify(gltf).to_utf8_buffer()
	while json.size() % 4 != 0:
		json.append(32)
	var tail := data.slice(20 + data.decode_u32(12))
	var file := FileAccess.open(path, FileAccess.WRITE)
	file.store_32(0x46546C67)
	file.store_32(2)
	file.store_32(20 + json.size() + tail.size())
	file.store_32(json.size())
	file.store_32(0x4E4F534A)
	file.store_buffer(json)
	file.store_buffer(tail)

func _run() -> void:
	_pack = "editor-preview-test-%d" % OS.get_process_id()
	_source = ProjectSettings.globalize_path("user://" + _pack + "-source")
	_output = ProjectSettings.globalize_path("user://mods/" + _pack)
	_expect(DirAccess.make_dir_recursive_absolute(_source) == OK, "fixture directory")
	for level in 4:
		_write_model(_source.path_join("model_lod%d.glb" % level), level)
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
	for margin: MarginContainer in [editor._view._preview_panel.get_node("PreviewTabs/Lighting"), editor._view._preview_panel._lod_controls]:
		for side in ["left", "top", "right", "bottom"]:
			_expect(margin.get_theme_constant("margin_" + side) >= 12, "preview tabs need content padding")
		_expect(margin.get_node("Content").get_theme_constant("separation") >= 8, "preview controls need vertical breathing room")
	_test_editor_spacing(editor)
	editor._session.create_asset({"type": "residential", "name": "Preview test", "pack": {"pack_id": _pack, "display_name": "Editor test", "author": "Test"}})
	editor._menus.actions.create_from_controls("entrance")
	editor._menus.actions.confirm()
	editor._view._asset_id_edit.text = "building.residential.preview_test"
	editor._view._display_name_edit.text = "Preview test"
	editor._view._residents_spin.value = 1
	var panel = editor._view._preview_panel
	_expect(panel._emission.selected == 0, "emission follows the preview clock by default")
	panel.find_child("Night", true, false).pressed.emit()
	editor._on_glb_file_selected(_source.path_join("model_lod0.glb"))
	var index: int = editor._selected_part_index
	_expect(index == 0, "LOD0 must create one part")
	_expect_emission(editor, true, "meshes imported during Night are lit automatically")
	_test_automatic_emission(editor)
	var part: MeshPart = editor._parts[0]
	part.position = Vector3(1, 0, 1)
	part.rotation_y = 90.0
	editor._apply_mesh_part_transform_from_state(0)
	editor._select_mesh_part(0)
	editor._session.capture_geometry("Import fixture")
	await _test_lod_authoring(editor)
	part = editor._parts[0]
	_expect(part.validation_error().is_empty(), "four contiguous tiers must validate")
	_expect(part.lods[3].distance_min_m == 120.0, "authored starting distances")
	var camera_transform := camera.global_transform
	var bounds := part.aabb
	var transform: Transform3D = editor._preview._mesh_parts[0].transform
	for level in 4:
		panel.set_lighting(0.0)
		editor._on_preview_lod_selected(level)
		_expect_emission(editor, true, "automatic night emission follows LOD switches")
		_expect(editor._preview._mesh_parts[0].transform == transform, "LOD changes preserve authored placement")
		_expect(part.aabb == bounds, "LOD changes preserve LOD0 placement bounds")
		_expect(camera.global_transform == camera_transform, "LOD changes preserve camera")
		var materials: PreviewMaterials = editor._preview.mesh_part_materials(0)
		_expect(materials.surface_count() == 1, "default-off glTF must retain imported emission texture")
		if materials.surface_count() > 0:
			var source: BaseMaterial3D = materials._surfaces[0]["source"]
			var authored_color := source.emission
			editor._preview.set_preview_emission(PreviewMaterials.Mode.ON, 2.0)
			var entry: Dictionary = materials._surfaces[0]
			var lit: BaseMaterial3D = entry["instance"].get_active_material(entry["surface"])
			_expect(lit != source and lit.emission_enabled, "force-on uses a preview-owned material")
			_expect(source.emission == authored_color, "emission override must not mutate source")
			editor._preview.set_preview_emission(PreviewMaterials.Mode.AUTHORED, 1.0)
			_expect(entry["instance"].get_active_material(entry["surface"]) == source, "authored reset restores material")
	_test_automatic_lod(editor, camera, part)
	panel.set_lighting(10.5)
	var day: Color = world.environment.ambient_light_color
	panel.find_child("Night", true, false).pressed.emit()
	_expect(world.environment.ambient_light_color != day, "night control must relight environment, not just rotate sun")
	editor.set_ui_theme_mode("light")
	_expect(world.environment.ambient_light_color != day, "UI theme must not reset night lighting")
	editor._export_asset(false)
	_expect(editor._session._issues.is_empty(), "export validation: " + JSON.stringify(editor._session._issues))
	var asset_dir := _output.path_join("assets/building.residential.preview_test")
	var manifest_path := asset_dir.path_join("asset.toml")
	_expect(FileAccess.file_exists(manifest_path), "real Rust export must publish manifest")
	await _test_export_savepoint(editor, manifest_path)
	for level in 4:
		_expect(FileAccess.file_exists(asset_dir.path_join("model_lod%d.glb" % level)), "every LOD must be copied")
		_expect(FileAccess.file_exists(asset_dir.path_join("window_%d.png" % level)), "each tier's dependencies must be copied")
		var saved: Dictionary = JSON.parse_string(AssetAuthoringFiles.read_gltf_json(asset_dir.path_join("model_lod%d.glb" % level)))
		_expect(saved["materials"][0]["emissiveFactor"] == [0.0, 0.0, 0.0], "preview emission must never be baked into export")
	editor._session.load_manifest(JSON.parse_string(editor.sim.get_asset_manifest_json(_pack + ":building.residential.preview_test")))
	_expect(editor._parts.size() == 1 and editor._parts[0].lods.size() == 4, "editor reload preserves the whole chain")
	editor._export_asset(false)
	var before := FileAccess.get_file_as_bytes(manifest_path)
	_write(asset_dir.path_join("unmanaged.txt"), "keep this creator-owned file")
	editor._export_asset(false)
	_expect(FileAccess.get_file_as_string(asset_dir.path_join("unmanaged.txt")) == "keep this creator-owned file", "saving preserves unmanaged asset files")
	var invalid: Dictionary = editor._session.document.snapshot()
	invalid["params"]["household_capacity"] = 0
	var failure := AssetAuthoringFiles.publish_document(JSON.stringify(invalid), _output)
	_expect(not failure.is_empty() and before == FileAccess.get_file_as_bytes(manifest_path), "failed export must leave installed manifest untouched")
	invalid = editor._session.document.snapshot()
	invalid["sources"][0][3] = _source.path_join("missing.glb")
	_expect(not AssetAuthoringFiles.publish_document(JSON.stringify(invalid), _output).is_empty(), "missing tiers block before publication")
	# Dependency collisions, traversal and rollback injection are Rust file-service unit tests.
	var invalid_part := MeshPart.new(_source.path_join("model_lod0.glb"), "invalid")
	invalid_part.append_lod(_source.path_join("model_lod1.glb"))
	invalid_part.lods[1].distance_min_m = 12.0
	_expect(not invalid_part.validation_error().is_empty(), "overlapping bands must fail")
	invalid_part.lods[1].distance_min_m = 35.0
	invalid_part.lods[1].distance_max_m = INF
	_expect(not invalid_part.validation_error().is_empty(), "typed Rust LOD validation rejects infinity instead of treating it as an unbounded tier")
	# Exercise the existing editing actions after the state/view split.
	editor._menus.actions.create_from_controls("parking")
	editor._menus.actions.confirm()
	_expect(editor._site_anchors_data.size() == 2, "parking tools remain functional")
	editor._menus.actions.create_from_controls("asphalt")
	editor._menus.actions.confirm()
	_expect(editor._site_surfaces_data.size() == 1, "yard tools remain functional")
	editor._session.capture_geometry("Fixture before deletion")
	editor._menus.actions.edit("delete", editor._menus.actions.selection())
	_expect(editor._site_surfaces_data.is_empty(), "yard removal remains functional")
	# Container layout is deferred; frame only after the newly opened workspace is sized.
	editor._select_mesh_part(0)
	await process_frame
	await process_frame
	editor._frame_selected_part()
	editor._lod_preview.update(camera)
	_expect(editor._lod_preview.states[editor._parts[0]].pixels >= 100.0, "frame selection makes the fixture clearly visible")
	await _optional_capture_and_measure(editor)
	await _test_comparison_preview(editor)
	editor._session.create_asset({"type": "residential", "name": "New fixture", "pack": {"pack_id": _pack}})
	_expect(editor._parts.is_empty(), "new asset clears document and preview selection")
	_expect(editor._lod_preview.states.is_empty(), "new asset releases all LOD preview state")
	_expect(panel._projected_size.text.is_empty(), "clearing the asset clears projected-size details")
	editor.free()
	await process_frame
	_remove_fixture_directory(_output)
	_remove_fixture_directory(_source)
	print("asset_editor_preview_test: %s" % ("PASS" if _failures == 0 else "FAIL"))
	quit(_failures)

func _test_export_savepoint(editor: Node3D, manifest_path: String) -> void:
	var session = editor._session
	var document = session.document
	_expect(not document.is_dirty() and document.draft_path().is_empty(), "export is clean without requiring a draft")
	_expect(not editor.get_window().title.begins_with("* "), "export clears the title's unsaved marker")
	session.new_asset_dialog()
	_expect(is_instance_valid(session._creation) and session._creation.visible, "New asset opens immediately after export")
	_expect(not is_instance_valid(session._guard), "unchanged export does not ask to save a draft")
	session._creation.hide()
	await process_frame
	session.set_field("display_name", "Edited after export")
	session.new_asset_dialog()
	_expect(is_instance_valid(session._guard) and session._guard.visible, "edits after export still require the unsaved-change guard")
	session._guard.canceled.emit()
	session._guard.hide()
	await process_frame
	session.undo()
	_expect(not document.is_dirty(), "undo to the exported revision is clean")
	session.redo()
	_expect(document.is_dirty(), "redo of a later edit is dirty")
	var draft_path := _source.path_join("export-savepoint.metrum-draft")
	session._save_to(draft_path)
	var draft_bytes := FileAccess.get_file_as_bytes(draft_path)
	session.set_field("display_name", "Exported after draft")
	editor._export_asset(false)
	_expect(not document.is_dirty() and document.draft_path() == draft_path, "export retains the draft path while marking the current revision clean")
	_expect(FileAccess.get_file_as_bytes(draft_path) == draft_bytes, "export never overwrites a saved draft")
	var manifest_bytes := FileAccess.get_file_as_bytes(manifest_path)
	session.set_field("household_capacity", 0)
	editor._export_asset(false)
	_expect(document.is_dirty() and not session._issues.is_empty(), "rejected export must leave edits dirty")
	_expect(FileAccess.get_file_as_bytes(manifest_path) == manifest_bytes, "rejected export keeps the installed asset")
	session.undo()
	_expect(not document.is_dirty(), "failed export does not replace the successful export savepoint")

func _expect_emission(editor: Node3D, enabled: bool, message: String) -> void:
	var materials: PreviewMaterials = editor._preview.mesh_part_materials(0)
	_expect(materials.surface_count() == 1, "fixture has one emission-textured surface")
	for entry in materials._surfaces:
		var active: Material = entry["instance"].get_active_material(entry["surface"])
		if active is ShaderMaterial:
			var clock: Vector2 = active.get_shader_parameter("window_preview_clock")
			var sample = editor._lighting.current_sample()
			_expect(is_equal_approx(clock.x, fposmod(sample.day_fraction * 24.0, 24.0)) and is_equal_approx(clock.y, sample.sun_elevation_deg), message + " (shared shader clock)")
			_expect(active.get_shader_parameter("texture_emission") == entry.source.emission_texture, "automatic keeps the emission mask")
		else:
			_expect(active.emission_enabled == enabled, message)
			if enabled:
				_expect(active.emission.get_luminance() > 0.0 and active.emission_energy_multiplier > 0.0,
					"forced emission must have nonzero tint and intensity")

func _test_comparison_preview(editor: Node3D) -> void:
	# A textured wall and contrasting roof expose the old flat, translucent override.
	var scene := Node3D.new()
	var albedo := Image.create(96, 96, false, Image.FORMAT_RGB8)
	albedo.fill(Color(0.55, 0.25, 0.12))
	var emission := Image.create(96, 96, false, Image.FORMAT_RGB8)
	emission.fill(Color.BLACK)
	for y in 96:
		for x in 96:
			if y % 8 == 0:
				albedo.set_pixel(x, y, Color(0.75, 0.65, 0.55))
			elif x % 16 >= 4 and x % 16 <= 11 and y % 16 >= 4 and y % 16 <= 11:
				albedo.set_pixel(x, y, Color(0.2, 0.25, 0.3))
				emission.set_pixel(x, y, Color.WHITE)
	for roof in [false, true]:
		var instance := MeshInstance3D.new()
		instance.name = "Roof" if roof else "Walls"
		instance.mesh = PrismMesh.new() if roof else BoxMesh.new()
		instance.mesh.size = Vector3(3.4, 1.4, 3.0) if roof else Vector3(3, 3, 2.6)
		instance.position.y = 3.7 if roof else 1.5
		var material := StandardMaterial3D.new()
		material.albedo_color = Color(0.15, 0.18, 0.22) if roof else Color.WHITE
		material.roughness = 0.8
		if not roof:
			material.albedo_texture = ImageTexture.create_from_image(albedo)
			material.emission_enabled = true
			material.emission = Color.WHITE
			material.emission_texture = ImageTexture.create_from_image(emission)
		instance.mesh.surface_set_material(0, material)
		scene.add_child(instance)
	var path := _source.path_join("comparison.glb")
	var gltf := GLTFDocument.new()
	var state := GLTFState.new()
	_expect(gltf.append_from_scene(scene, state) == OK, "comparison scene export")
	_expect(gltf.write_to_filesystem(state, path) == OK, "comparison GLB export")
	scene.free()
	var source_bytes := FileAccess.get_file_as_bytes(path)
	var snapshot: Dictionary = editor._session.document.snapshot()
	var panel = editor._view._preview_panel
	var preview = editor._preview
	panel._emission.select(0)
	panel._emission.item_selected.emit(0)
	panel._strength.value = 1.8
	panel.find_child("Night", true, false).pressed.emit()
	_expect(preview.load_ghost(path, 1.0, 1, 1), "load textured comparison at night")
	_expect(preview._ghost_materials.surface_count() == 1, "comparison retains the window emission binding")
	for entry in preview._ghost_materials._surfaces:
		if entry.source.emission_texture == null:
			continue
		var active := entry.instance.get_active_material(entry.surface) as ShaderMaterial
		_expect(active != null and is_equal_approx(active.get_shader_parameter("window_strength"), 1.8),
			"new comparisons immediately inherit the current automatic emission strength")
	for preset in ["Night", "Day", "Night"]:
		panel.find_child(preset, true, false).pressed.emit()
		for instance: MeshInstance3D in preview._ghost_root.find_children("*", "MeshInstance3D", true, false):
			var source: BaseMaterial3D = instance.mesh.surface_get_material(0)
			var active: Material = instance.get_active_material(0)
			_expect(active != source, "comparison tint and lighting use preview-owned material copies")
			if active is ShaderMaterial:
				_expect(active.get_shader_parameter("texture_albedo") == source.albedo_texture
					and active.get_shader_parameter("texture_emission") == source.emission_texture
					and active.get_shader_parameter("roughness") == source.roughness,
					"automatic comparison retains source textures and roughness")
				_expect(active.get_shader_parameter("albedo").is_equal_approx(source.albedo_color * preview.GHOST_TINT), "comparison has a subtle non-cumulative tint")
				_expect(active.get_shader_parameter("window_preview_clock").x == (0.0 if preset == "Night" else 10.5), "comparison follows the preview clock")
			else:
				_expect(active.transparency == source.transparency and active.shading_mode == source.shading_mode, "non-window comparison preserves source shading")
	# Intentional source glass/cutouts must not be flattened into opaque walls either.
	var glass := MeshInstance3D.new()
	glass.mesh = BoxMesh.new()
	var glass_source := StandardMaterial3D.new()
	glass_source.albedo_color = Color(0.8, 0.9, 1.0, 0.4)
	glass.mesh.surface_set_material(0, glass_source)
	for transparency in [BaseMaterial3D.TRANSPARENCY_ALPHA, BaseMaterial3D.TRANSPARENCY_ALPHA_SCISSOR]:
		glass_source.transparency = transparency
		glass.set_surface_override_material(0, null)
		preview._apply_ghost_material(glass)
		var active: BaseMaterial3D = glass.get_active_material(0)
		_expect(active.transparency == transparency and active.albedo_color.a == glass_source.albedo_color.a,
			"comparison preserves authored glass/cutout transparency")
	glass.free()
	await _capture_comparison(editor)
	preview.clear_ghost()
	_expect(not preview.has_ghost() and preview._ghost_materials == null, "clearing comparison releases material state")
	preview.set_preview_emission(PreviewMaterials.Mode.ON, 1.0)
	_expect(preview.load_ghost(path, 1.0, 1, 1), "comparison can be replaced after clearing")
	preview.set_preview_emission(PreviewMaterials.Mode.AUTHORED, 1.0)
	for entry in preview._ghost_materials._surfaces:
		_expect(entry.instance.get_active_material(entry.surface) == entry.source, "Authored restores the tinted comparison's source emission")
	if "--benchmark-asset-preview" in OS.get_cmdline_user_args():
		var load_usec := 0
		for iteration in 10:
			var started := Time.get_ticks_usec()
			_expect(preview.load_ghost(path, 1.0, 1, 1), "measured comparison load")
			load_usec += Time.get_ticks_usec() - started
			# Retire replaced scenes outside the import/material/pick-cache measurement.
			await process_frame
		print("asset_preview_measure comparison_load_mean_ms=%.3f iterations=10 surfaces=2" % (load_usec / 10000.0))
		var cached: Material = preview._ghost_materials._surfaces[0].preview
		var started := Time.get_ticks_usec()
		for iteration in 2000:
			preview._ghost_materials.apply(PreviewMaterials.Mode.ON if iteration % 2 == 0 else PreviewMaterials.Mode.OFF, 1.0)
		print("asset_preview_measure comparison_emission_mean_us=%.3f iterations=2000 surfaces=1" % ((Time.get_ticks_usec() - started) / 2000.0))
		_expect(preview._ghost_materials._surfaces[0].preview == cached, "comparison emission reuses its material instead of allocating on changes")
	preview.clear_ghost()
	_expect(editor._session.document.snapshot() == snapshot, "comparison rendering does not change the asset document")
	_expect(FileAccess.get_file_as_bytes(path) == source_bytes, "comparison rendering never rewrites source files")

func _capture_comparison(editor: Node3D) -> void:
	for argument in OS.get_cmdline_user_args():
		if not argument.begins_with("--capture-preview="):
			continue
		var directory := argument.trim_prefix("--capture-preview=")
		editor._preview.set_ghost_world_position(Vector3(-5, 0, 0))
		if not editor._layout.inspector_collapsed:
			editor._view.toggle_inspector()
		editor._cam_input.set_process(false)
		editor._cam_input.set_process_input(false)
		var camera: Camera3D = editor.get_node("CameraNode")
		camera.look_at_from_position(Vector3(-14, 9, 15), Vector3(-2, 1.5, 0))
		for theme in ["light", "dark"]:
			editor.set_ui_theme_mode(theme)
			for preset in ["Day", "Night"]:
				editor._view._preview_panel.find_child(preset, true, false).pressed.emit()
				await process_frame
				await process_frame
				await RenderingServer.frame_post_draw
				_expect(root.get_texture().get_image().save_png(directory.path_join("comparison-%s-%s.png" % [theme, preset.to_lower()])) == OK, "rendered textured comparison capture")
		await _test_framing_click(editor)
		await _test_thumbnail_capture(editor, directory)

func _test_thumbnail_capture(editor: Node3D, directory: String) -> void:
	var preview = editor._preview
	var view = editor._view
	view._preview_panel.set_lighting(10.5)
	preview.set_scale_reference_visible(true)
	var helpers := [preview._site_anchor_overlay, preview._site_anchor_label_root,
		preview._site_surface_overlay, preview._site_surface_label_root,
		preview._frontage_label, preview._frontage_arrow, preview._lot_overlay,
		preview._ground_grid, preview._selection_overlay, preview._hover_outline,
		preview._scale_reference, preview._ghost_root, view.picking_overlay,
		view.hover_label, view._selection_rect_overlay,
		editor._session.thumbnails._overlay, editor._session.thumbnails._actions]
	var visibility := {}
	for helper in helpers:
		visibility[helper] = helper.visible
	var grid: Material = preview._ground.material_overlay
	var mode := editor.process_mode
	var before: Dictionary = editor._session.document.snapshot()
	var captured_frames: Array = []
	RenderingServer.frame_pre_draw.connect(func():
		captured_frames.append(true)
		for helper in helpers:
			_expect(not helper.is_visible_in_tree(), "thumbnail render excludes " + str(helper))
		_expect(preview._ground.material_overlay == null, "thumbnail render excludes the terrain grid")
		_expect(preview._mesh_instance.is_visible_in_tree() and preview._site_surface_fill.is_visible_in_tree()
			and preview._ground.is_visible_in_tree() and preview._lot_plane.is_visible_in_tree(), "thumbnail retains authored geometry and ground")
	, CONNECT_ONE_SHOT)
	await editor._session.capture_thumbnail()
	_expect(captured_frames.size() == 1, "capture waits for a rendered frame with hidden helpers")
	for helper in helpers:
		_expect(helper.visible == visibility[helper], "capture restores helper visibility")
	_expect(preview._ground.material_overlay == grid and editor.process_mode == mode, "capture restores grid and interaction")
	var captured: Dictionary = editor._session.document.snapshot()
	var thumbnail := Image.load_from_file(captured.thumbnail_source)
	# Output must not follow the window: one size backs both the editor list and in-game details.
	_expect(thumbnail != null and thumbnail.get_size() == ThumbnailCapture.OUTPUT_SIZE, "saved thumbnail normalises to the fixed output size")
	_expect(str(captured.params.thumbnail) == ThumbnailCapture.FILE_NAME, "capture publishes the compressed thumbnail name")
	_expect(thumbnail.save_png(directory.path_join("thumbnail-clean.png")) == OK, "save clean thumbnail verification artifact")
	editor._session.undo()
	_expect(editor._session.document.snapshot() == before, "thumbnail capture is one undoable command and does not edit geometry")
	preview.set_scale_reference_visible(false)

func _test_automatic_emission(editor: Node3D) -> void:
	var panel = editor._view._preview_panel
	_expect(panel._asset_window_profile == (1.0 if editor._session.params.get("zone_type") == "residential" else 2.0),
		"loaded asset metadata selects its automatic window profile")
	var document: String = JSON.stringify(editor._session.document.snapshot())
	var imports: int = editor._preview.lod_import_count
	for preset in [["Day", false], ["Dusk", true], ["Night", true], ["Day", false]]:
		panel.find_child(preset[0], true, false).pressed.emit()
		_expect_emission(editor, preset[1], "%s preset updates window emission" % preset[0])
	for hour in [24.0, 12.0, 0.0]:
		panel._hour.value = hour
		_expect_emission(editor, hour != 12.0, "manual hour changes update emission, including midnight wrap")
	# Explicit inspection overrides survive preset changes; Authored restores the source.
	for mode in [PreviewMaterials.Mode.AUTHORED, PreviewMaterials.Mode.OFF, PreviewMaterials.Mode.ON]:
		panel._emission.select(mode + 1)
		panel._emission.item_selected.emit(mode + 1)
		for preset in ["Day", "Night"]:
			panel.find_child(preset, true, false).pressed.emit()
			if mode == PreviewMaterials.Mode.AUTHORED:
				var materials: PreviewMaterials = editor._preview.mesh_part_materials(0)
				for entry in materials._surfaces:
					_expect(entry["instance"].get_active_material(entry["surface"]) == entry["source"], "Authored restores the original material")
			else:
				_expect_emission(editor, mode == PreviewMaterials.Mode.ON, "manual emission overrides survive presets")
	panel._emission.select(0)
	panel._emission.item_selected.emit(0)
	_expect_emission(editor, true, "selecting Automatic immediately follows the current night hour")
	_expect(panel._strength.editable, "automatic emission intensity remains adjustable")
	panel._strength.value = 2.5
	var materials: PreviewMaterials = editor._preview.mesh_part_materials(0)
	for entry in materials._surfaces:
		var active: ShaderMaterial = entry["instance"].get_active_material(entry["surface"])
		_expect(is_equal_approx(active.get_shader_parameter("window_strength"), 2.5), "automatic emission uses reference intensity")
		_expect(entry["source"].emission == Color.BLACK, "automatic emission preserves the default-off source factor")
	panel.set_lighting(0.0)
	for profile in 3:
		panel._window_profile.select(profile + 1)
		panel._window_profile.item_selected.emit(profile + 1)
		for entry in materials._surfaces:
			var automatic: ShaderMaterial = entry.instance.get_active_material(entry.surface)
			_expect(automatic.get_shader_parameter("window_preview_schedule").w == [1.0, 2.0, 0.0][profile],
				"preview profile controls select residential, overnight and dark schedules")
	panel._window_profile.select(0)
	panel._window_profile.item_selected.emit(0)
	for residential in [false, true]:
		panel.set_asset_window_profile(residential)
		for entry in materials._surfaces:
			var automatic: ShaderMaterial = entry.instance.get_active_material(entry.surface)
			_expect(automatic.get_shader_parameter("window_preview_schedule").w == (1.0 if residential else 2.0),
				"automatic profile follows the current asset type")
	_expect(JSON.stringify(editor._session.document.snapshot()) == document, "lighting controls never edit the authored document")
	_expect(editor._preview.lod_import_count == imports, "lighting controls never reimport meshes")

func _remove_fixture_directory(path: String) -> void:
	var dir := DirAccess.open(path)
	_expect(dir != null, "fixture directory exists for cleanup")
	if dir == null:
		return
	for file in dir.get_files():
		_expect(DirAccess.remove_absolute(path.path_join(file)) == OK, "remove generated fixture file")
	for child in dir.get_directories():
		_remove_fixture_directory(path.path_join(child))
	_expect(DirAccess.remove_absolute(path) == OK, "remove generated fixture directory")

func _test_editor_spacing(editor: Node3D) -> void:
	var view = editor._view
	for mode in ["dark", "light", "dark"]:
		editor.set_ui_theme_mode(mode)
		var reference: CheckButton = view.scale_reference_button
		_expect(not reference.flat, "scale-reference toggle draws its background over the sky")
		for state in ["normal", "hover", "pressed", "hover_pressed", "focus", "disabled"]:
			var reference_style := reference.get_theme_stylebox(state) as StyleBoxFlat
			_expect(reference_style != null and is_equal_approx(reference_style.bg_color.a, 1.0), "scale reference has an opaque background in " + state)
			if reference_style != null:
				_expect(reference_style.content_margin_left >= 12 and reference_style.content_margin_top >= 6, "reference toggle keeps toolbar padding")
		_expect(reference.get_theme_color("font_hover_pressed_color") == Editor.EditorTheme.color(mode, "text"), "checked-hovered reference text follows the UI palette")
		for control in [view._pack_set_btn, view._asset_search_edit, view._residents_spin.get_line_edit(), view._log_label]:
			var style: StyleBox = control.get_theme_stylebox("normal")
			_expect(style.content_margin_left >= 12 and style.content_margin_top >= 6, "asset editor text padding survives palette changes")
		var inspector_margin: MarginContainer = view._pack_set_btn.get_parent().get_parent()
		_expect(inspector_margin.get_theme_constant("margin_left") >= 12, "inspector tabs share preview content padding")
		_expect(view._asset_tree.get_theme_stylebox("panel").content_margin_left >= 12, "asset browser text is padded")
		_expect(view._pack_summary_lbl.get_theme_constant("line_spacing") >= 4, "multiline editor text has breathing room")
	# The shared palette remains compact for unrelated editors and gameplay consumers.
	var shared_style: StyleBox = Editor.EditorTheme.style_box("dark", "panel")
	_expect(shared_style.content_margin_left == 8 and shared_style.content_margin_top == 4, "asset editor spacing must not change the shared theme")

func _choose_lod_file(editor: Node3D, button: Button, path: String, title: String) -> void:
	button.pressed.emit()
	var dialog: Window = editor.get_child(editor.get_child_count() - 1)
	_expect(dialog.visible and dialog.title == title, "LOD action opens the matching file picker")
	_expect(dialog._current_dir == editor._last_glb_dir, "LOD picker opens in the last selected source directory")
	await process_frame
	dialog._selected_path = path
	dialog._confirm_selection()
	await process_frame

func _test_lod_authoring(editor: Node3D) -> void:
	editor._last_glb_dir = _source
	var panel = editor._view._preview_panel
	var camera: Camera3D = editor.get_node("CameraNode")
	var saved_projection := camera.projection
	var saved_size := camera.size
	camera.set_orthogonal(500.0, camera.near, camera.far)
	editor._lod_preview.update(camera)
	_expect(panel._add.is_visible_in_tree() and panel._lod_list.is_visible_in_tree(), "selecting a mesh immediately exposes LOD authoring")
	_expect(panel._replace.text == "Replace LOD0…" and panel._remove.disabled and not panel._remove.visible, "single-tier part offers replacement without an inapplicable Remove action")
	var single_tier_height: float = panel._lod_list.custom_minimum_size.y
	for level in range(1, 4):
		_expect(panel._add.text == "Add LOD%d…" % level, "Add identifies the next level")
		await _choose_lod_file(editor, panel._add, _source.path_join("model_lod%d.glb" % level), "Add LOD%d" % level)
		_expect(editor._parts.size() == 1 and editor._parts[0].lods.size() == level + 1, "Add LOD extends the selected part, not the parts list")
		_expect(panel._lod_list.item_count == level + 2 and panel._lod_list.is_selected(level + 1), "visible chain selects the newly added tier")
		_expect(editor._lod_preview.states[editor._parts[0]].active == level, "new LOD is shown without saving or exporting")
	_expect(panel._lod_list.custom_minimum_size.y > single_tier_height and panel._remove.visible, "chain grows to show additional tiers and their removal action")
	_expect(panel._metrics.text.contains("Move camera to resume Automatic"), "temporary LOD inspection explains how to resume zoom switching")
	_set_projected_pixels(editor, camera, 900.0)
	var state = editor._lod_preview.states[editor._parts[0]]
	_expect(state.active == 0 and state.forced == -1 and panel._lod_list.is_selected(1), "adding LOD3 then zooming close resumes Automatic at LOD0 without using the mode row")
	_expect(editor._preview.mesh_part_lod_path(0) == _source.path_join("model_lod0.glb") and panel._replace.text == "Replace LOD0…", "post-import refinement updates both the actual mesh and replacement target")
	var session = editor._session
	var before: Dictionary = session.document.snapshot()
	session.undo()
	_expect(editor._parts[0].lods.size() == 3 and panel._add.text == "Add LOD3…", "undo Add refreshes the visible chain")
	session.redo()
	_expect(session.document.snapshot() == before, "redo restores the added LOD exactly")
	panel._lod_list.item_selected.emit(3)
	_expect(panel._replace.text == "Replace LOD2…", "clicking a LOD targets that level for replacement")
	var replacement_dir := _source.path_join("replacement")
	_expect(DirAccess.make_dir_recursive_absolute(replacement_dir) == OK, "replacement fixture directory")
	var replacement := replacement_dir.path_join("replacement_lod2.glb")
	_write_model(replacement, 2)
	var bounds: AABB = editor._parts[0].aabb
	var saves_before: int = editor.config_save_count
	await _choose_lod_file(editor, panel._replace, replacement, "Replace LOD2")
	_expect(editor._last_glb_dir == replacement_dir, "replacing a LOD remembers its source directory")
	_expect(editor.config_save_count > saves_before, "replacement persists the remembered directory")
	panel._replace.pressed.emit()
	var next_picker: Window = editor.get_child(editor.get_child_count() - 1)
	_expect(next_picker._current_dir == replacement_dir, "next replacement starts in the newly selected directory")
	next_picker._on_close_requested()
	await process_frame
	var expected := before.duplicate(true)
	expected["params"]["mesh_parts"][0]["lods"][2]["file"] = replacement.get_file()
	expected["sources"][0][2] = replacement
	_expect(session.document.snapshot() == expected, "replacement changes only the selected source, preserving other tiers, placement and authored bands")
	_expect(editor._parts[0].aabb == bounds and editor._preview.mesh_part_lod_path(0) == replacement, "replaced tier previews live without changing LOD0 bounds")
	_set_projected_pixels(editor, camera, 1000.0)
	_expect(state.active == 0 and state.forced == -1 and panel._lod_list.is_selected(1), "replacing a tier cannot leave zoom locked in inspection mode")
	session.undo()
	_expect(session.document.snapshot() == before, "replacement is undoable")
	_expect(editor._last_glb_dir == replacement_dir, "undoing a mesh edit preserves the browse preference")
	session.redo()
	_expect(session.document.snapshot() == expected, "replacement is redoable")
	session.undo()
	panel._remove.pressed.emit()
	_expect(editor._parts[0].lods.size() == 3 and panel._add.text == "Add LOD3…", "Remove last refreshes the visible chain")
	session.undo()
	_expect(session.document.snapshot() == before, "removing the last tier is undoable")
	panel._lod_list.item_selected.emit(0)
	_expect(panel._selected_lod == -1 and panel._lod_list.is_selected(panel._active_lod + 1), "Automatic remains active while highlighting the visible tier")
	camera.projection = saved_projection
	camera.size = saved_size
	editor._lod_preview.update(camera)

func _test_automatic_lod(editor: Node3D, camera: Camera3D, part: MeshPart) -> void:
	var preview = editor._lod_preview
	var panel = editor._view._preview_panel
	var saved_transform := camera.transform
	var saved_fov := camera.fov
	var saved_projection := camera.projection
	var authored := JSON.stringify(part.to_manifest())
	var original_bounds := part.aabb
	var center: Vector3 = editor._preview.mesh_part_transform(0) * part.aabb.get_center()
	camera.transform = Transform3D(Basis.IDENTITY, center + Vector3(0, 0, 50))
	camera.set_orthogonal(10.0, 0.1, 5000.0)
	panel._select_lod(0)
	var state = preview.states[part]
	_expect(panel._bands.text.contains("512.0 px"), "read-only boundaries come from the shared policy")
	var imports: int = editor._preview.lod_import_count
	_set_projected_pixels(editor, camera, 900.0)
	_expect(state.active == 0 and state.forced == -1, "Automatic starts at LOD0 for a large projected asset")
	var fine_triangles: int = state.triangles
	_set_projected_pixels(editor, camera, 32.0)
	_expect(state.active == 3 and state.triangles < fine_triangles, "zooming out selects the coarse mesh and updates triangle count")
	_expect(panel._lod_list.is_selected(4) and panel._replace.text == "Replace LOD3…" and state.forced == -1, "automatic inspection and replacement follow the rendered LOD without forcing it")
	panel._replace.pressed.emit()
	var replace_dialog: Window = editor.get_child(editor.get_child_count() - 1)
	_expect(replace_dialog.title == "Replace LOD3", "replacement targets the visible automatic tier, not LOD0")
	replace_dialog.hide()
	replace_dialog.queue_free()
	_expect(panel._metrics.is_visible_in_tree() and panel._metrics.text == "Automatic → LOD3 · %d triangles" % state.triangles, "default readout shows only active LOD and triangle count")
	var details_toggle: CheckButton = panel._lod_controls.find_child("LODDetails", true, false)
	_expect(not details_toggle.button_pressed and not panel._projected_size.is_visible_in_tree(), "projected size is hidden in collapsed LOD details by default")
	details_toggle.button_pressed = true
	_expect(panel._projected_size.is_visible_in_tree() and panel._projected_size.text == "Projected asset size: 32.0 render px", "expanding LOD details reveals the diagnostic size")
	_set_projected_pixels(editor, camera, 300.0)
	_expect(panel._projected_size.text == "Projected asset size: 300.0 render px", "expanded projected size follows camera changes live")
	details_toggle.button_pressed = false
	_expect(state.active == 1, "moving closer refines directly")
	_expect(panel._lod_list.is_selected(2) and panel._replace.text == "Replace LOD1…", "highlight follows subsequent automatic transitions")
	_set_projected_pixels(editor, camera, 250.0)
	_expect(not panel._projected_size.is_visible_in_tree() and panel._projected_size.text == "Projected asset size: 250.0 render px", "collapsed details stay current without exposing the diagnostic")
	_expect(state.active == 1, "coarsening dead band holds previous tier")
	_set_projected_pixels(editor, camera, 220.0)
	_expect(state.active == 2, "crossing dead band coarsens")
	_set_projected_pixels(editor, camera, 260.0)
	_expect(state.active == 2, "refining dead band holds previous tier")
	_set_projected_pixels(editor, camera, 300.0)
	_expect(state.active == 1, "crossing refining dead band restores detail")
	panel.set_quality(0)
	_expect(state.active == 2, "Performance uses shared engine quality policy")
	panel.set_quality(2)
	_expect(state.active == 0, "Quality retains detail longer")
	panel.set_quality(1)
	panel._select_lod(2)
	preview.update(camera)
	_expect(state.active == 1 and state.forced == 1, "explicit inspection stays on the chosen tier while the camera is idle")
	_set_projected_pixels(editor, camera, 16.0)
	_expect(state.active == 3 and state.forced == -1, "zooming ends temporary inspection and resumes Automatic without forced-tier history")
	panel._select_lod(2)
	panel._select_lod(0)
	_expect(state.active == 3, "Automatic row also ends temporary inspection immediately")
	var evaluations: int = preview.evaluations
	for iteration in 20:
		preview.update(camera)
	_expect(preview.evaluations == evaluations, "idle frames skip projection and selection")
	_expect(editor._preview.lod_import_count == imports, "automatic and forced switches reuse imported tiers")
	# Real camera projection, render scale and placement scale must drive the same policy.
	camera.set_perspective(60.0, 0.1, 5000.0)
	preview.update(camera)
	var pixels: float = state.pixels
	camera.position.z += 50.0
	preview.update(camera)
	_expect(state.pixels < pixels, "perspective distance affects projected size")
	pixels = state.pixels
	camera.fov = 90.0
	preview.update(camera)
	_expect(state.pixels < pixels, "FOV changes invalidate the preview")
	pixels = state.pixels
	var render_scale := camera.get_viewport().scaling_3d_scale
	camera.get_viewport().scaling_3d_scale = render_scale * 0.5
	preview.update(camera)
	_expect(is_equal_approx(state.pixels, pixels * 0.5), "render resolution participates in projection")
	camera.get_viewport().scaling_3d_scale = render_scale
	var scale := part.scale
	part.scale *= 2.0
	editor._apply_mesh_part_transform_from_state(0)
	preview.update(camera)
	_expect(state.pixels > pixels, "placement scale invalidates projected size")
	part.scale = scale
	editor._apply_mesh_part_transform_from_state(0)
	_expect(part.aabb == original_bounds, "automatic selection never changes LOD0 bounds")
	_expect(JSON.stringify(part.to_manifest()) == authored, "preview modes/quality do not change export metadata")
	# A second part owns independent automatic/forced history; index removal must not transfer it.
	editor._on_glb_file_selected(_source.path_join("model_lod0.glb"))
	var extra: int = editor._selected_part_index
	editor._select_mesh_part(extra)
	_expect(preview.states.size() == 2 and preview.states[editor._parts[extra]].active == 0, "single-tier part stays visible beside a multi-tier part")
	editor._on_preview_lod_selected(0)
	editor._session.capture_geometry("Fixture before deletion")
	editor._menus.actions.edit("delete", editor._menus.actions.selection())
	_expect(preview.states.size() == 1 and preview.states.has(part), "removing a part releases only its own LOD history")
	editor._select_mesh_part(0)
	# Chain edits prune cached tiers and clamp a removed forced level to the new last tier.
	editor._on_preview_lod_selected(3)
	panel._remove_last()
	_expect(part.lods.size() == 3 and state.active == 2, "removing the active final tier selects the remaining final tier")
	part.append_lod(_source.path_join("model_lod3.glb"))
	editor._sync_preview_lods()
	camera.set_orthogonal(10.0, 0.1, 5000.0)
	panel._select_lod(0)
	_set_projected_pixels(editor, camera, 900.0)
	part.append_lod(_source.path_join("missing_preview.glb"))
	editor._sync_preview_lods()
	imports = editor._preview.lod_import_count
	_set_projected_pixels(editor, camera, 1.0)
	_expect(state.active == 0 and panel._status.text.contains("failed to load"), "unavailable automatic tier reports failure and retains the visible mesh")
	_expect(panel._lod_list.is_selected(1) and panel._replace.text == "Replace LOD0…", "failed automatic targets do not select a tier that is not rendered")
	panel._select_lod(5)
	_expect(panel._lod_list.is_selected(5) and panel._replace.text == "Replace LOD4…", "explicitly selecting a missing tier still allows its source to be repaired")
	panel._select_lod(0)
	_set_projected_pixels(editor, camera, 2.0)
	_expect(editor._preview.lod_import_count == imports, "failed tiers are not retried every camera update")
	panel._remove_last()
	camera.transform = saved_transform
	camera.projection = saved_projection
	camera.fov = saved_fov
	preview.update(camera)

func _set_projected_pixels(editor: Node3D, camera: Camera3D, pixels: float) -> void:
	var preview = editor._lod_preview
	preview.update(camera)
	var state = preview.states[editor._parts[0]]
	camera.size *= state.pixels / pixels
	preview.update(camera)

func _optional_capture_and_measure(editor: Node3D) -> void:
	if "--benchmark-asset-preview" in OS.get_cmdline_user_args():
		editor._sync_preview_lods()
		var imports: int = editor._preview.lod_import_count
		var started := Time.get_ticks_usec()
		for iteration in 40:
			editor._preview.set_mesh_part_lod(0, editor._parts[0].lods[iteration % 4].source_path)
		print("asset_preview_measure cached_switch_mean_ms=%.3f iterations=40 synthetic_box=true imports=%d" % [(Time.get_ticks_usec() - started) / 40000.0, editor._preview.lod_import_count - imports])
		var camera: Camera3D = editor.get_node("CameraNode")
		var saved_projection := camera.projection
		var saved_size := camera.size
		camera.set_orthogonal(10.0, camera.near, camera.far)
		editor._on_preview_lod_selected(-1)
		for iteration in 100:
			_set_projected_pixels(editor, camera, 32.0 if iteration % 2 == 0 else 900.0)
		started = Time.get_ticks_usec()
		for iteration in 2000:
			_set_projected_pixels(editor, camera, 32.0 if iteration % 2 == 0 else 900.0)
		print("asset_preview_measure automatic_transition_mean_us=%.3f iterations=2000 parts=1 lods=4 imports=%d" % [(Time.get_ticks_usec() - started) / 2000.0, editor._preview.lod_import_count - imports])
		_expect(editor._preview.lod_import_count == imports, "measured LOD transitions never reimport meshes")
		camera.projection = saved_projection
		camera.size = saved_size
		editor._lod_preview.update(camera)
	for argument in OS.get_cmdline_user_args():
		if not argument.begins_with("--capture-preview="):
			continue
		var directory := argument.trim_prefix("--capture-preview=")
		_expect(DirAccess.make_dir_recursive_absolute(directory) == OK, "capture output directory")
		editor._select_mesh_part(0)
		editor._frame_selected_part()
		editor._view._preview_panel.set_lighting(10.5)
		for preset in ["Day", "Night"]:
			editor._view._preview_panel.find_child(preset, true, false).pressed.emit()
			await process_frame
			await process_frame
			await RenderingServer.frame_post_draw
			_expect(root.get_texture().get_image().save_png(directory.path_join(preset.to_lower() + ".png")) == OK, "rendered preview capture")
		editor._view.show_task("model")
		for theme_mode in ["light", "dark"]:
			editor.set_ui_theme_mode(theme_mode)
			await process_frame
			await RenderingServer.frame_post_draw
			_expect(root.get_texture().get_image().save_png(directory.path_join("lod-controls-" + theme_mode + ".png")) == OK, "rendered automatic LOD controls")
			for enabled in [true, false]:
				editor._view.scale_reference_button.button_pressed = enabled
				await process_frame
				await RenderingServer.frame_post_draw
				_expect(root.get_texture().get_image().save_png(directory.path_join("reference-%s-%s.png" % [theme_mode, "on" if enabled else "off"])) == OK, "night reference toggle remains visible in both themes and states")
		for tab in ["gameplay", "site"]:
			editor._view.show_task(tab)
			await process_frame
			await RenderingServer.frame_post_draw
			_expect(root.get_texture().get_image().save_png(directory.path_join("inspector-%s.png" % tab)) == OK, "rendered inspector spacing")

# Drives the framing button through the real input pipeline. The editor's _input runs ahead of the
# GUI layer and claims left-clicks inside the preview pane, so a regression there swallows the
# press while still showing hover feedback — which looks like a dead button, not a broken handler.
func _test_framing_click(editor: Node3D) -> void:
	var thumbs = editor._session.thumbnails
	var before := str(thumbs.path())
	editor._session.begin_thumbnail_framing()
	await process_frame
	_expect(thumbs._actions.visible, "framing mode shows its actions")
	var take: Button = thumbs._actions.get_child(0)
	_expect(take.text == "Take snapshot", "first action is the snapshot button")
	var at: Vector2 = take.get_global_rect().get_center()
	var motion := InputEventMouseMotion.new()
	motion.position = at
	motion.global_position = at
	Input.parse_input_event(motion)
	await process_frame
	_expect(editor.get_viewport().gui_get_hovered_control() == take, "pointer hovers the snapshot button")
	for is_down in [true, false]:
		var click := InputEventMouseButton.new()
		click.button_index = MOUSE_BUTTON_LEFT
		click.pressed = is_down
		click.position = at
		click.global_position = at
		Input.parse_input_event(click)
		await process_frame
	# The capture awaits a drawn frame before it writes, so settle before asserting on the result.
	for frame in 4:
		await RenderingServer.frame_post_draw
		await process_frame
	_expect(str(thumbs.path()) != before and not str(thumbs.path()).is_empty(), "clicking Take snapshot captures a thumbnail")
	_expect(not thumbs._actions.visible, "capturing leaves framing mode")
	var state: Dictionary = editor._session.document.snapshot()
	_expect(str(state.params.thumbnail) == thumbs.FILE_NAME, "click capture writes the compressed thumbnail")
	editor._session.undo()
	await process_frame
