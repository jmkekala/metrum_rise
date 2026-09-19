# SPDX-License-Identifier: GPL-2.0-only

## Asset editor regression: real UI, four-tier round-trip, dependency-safe saves and lighting.
## Run with --asset-editor; fixtures use a process-unique pack and never edit source assets.
extends SceneTree

const Editor = preload("res://scripts/editors/asset_editor.gd")
const MeshPart = preload("res://scripts/editors/asset_editor/mesh_part.gd")
const PreviewMaterials = preload("res://scripts/editors/asset_editor/preview_materials.gd")

class TestEditor extends Editor:
	func _save_config() -> void:
		pass
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
	editor._add_site_anchor("entrance")
	editor._view._asset_id_edit.text = "building.residential.preview_test"
	editor._view._display_name_edit.text = "Preview test"
	editor._view._residents_spin.value = 1
	var index: int = editor._add_mesh_part_from_path(_source.path_join("model_lod0.glb"), "house")
	_expect(index == 0, "LOD0 must create one part")
	var part: MeshPart = editor._parts[0]
	for level in range(1, 4):
		part.append_lod(_source.path_join("model_lod%d.glb" % level))
	part.position = Vector3(1, 0, 1)
	part.rotation_y = 90.0
	editor._apply_mesh_part_transform_from_state(0)
	editor._select_mesh_part(0)
	_expect(part.validation_error().is_empty(), "four contiguous tiers must validate")
	_expect(part.lods[3].distance_min_m == 120.0, "authored starting distances")
	var camera_transform := camera.global_transform
	var bounds := part.aabb
	var transform: Transform3D = editor._preview._mesh_parts[0].transform
	for level in 4:
		editor._on_preview_lod_selected(level)
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
	editor._view._preview_panel.set_lighting(10.5, PreviewMaterials.Mode.OFF)
	var day: Color = world.environment.ambient_light_color
	editor._view._preview_panel.set_lighting(0.0, PreviewMaterials.Mode.ON)
	_expect(world.environment.ambient_light_color != day, "night control must relight environment, not just rotate sun")
	editor.set_ui_theme_mode("light")
	_expect(world.environment.ambient_light_color != day, "UI theme must not reset night lighting")
	editor._export_asset(false)
	_expect(editor._session._issues.is_empty(), "export validation: " + JSON.stringify(editor._session._issues))
	var asset_dir := _output.path_join("assets/building.residential.preview_test")
	var manifest_path := asset_dir.path_join("asset.toml")
	_expect(FileAccess.file_exists(manifest_path), "real Rust export must publish manifest")
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
	editor._add_site_anchor("parking")
	_expect(editor._site_anchors_data.size() == 2, "parking tools remain functional")
	editor._add_site_surface("asphalt")
	_expect(editor._site_surfaces_data.size() == 1, "yard tools remain functional")
	editor._remove_selected_site_surface()
	_expect(editor._site_surfaces_data.is_empty(), "yard removal remains functional")
	# Container layout is deferred; frame only after the newly opened workspace is sized.
	editor._select_mesh_part(0)
	await process_frame
	await process_frame
	editor._frame_selected_part()
	editor._lod_preview.update(camera)
	_expect(editor._lod_preview.states[editor._parts[0]].pixels >= 100.0, "frame selection makes the fixture clearly visible")
	await _optional_capture_and_measure(editor)
	editor._session.create_asset({"type": "residential", "name": "New fixture", "pack": {"pack_id": _pack}})
	_expect(editor._parts.is_empty(), "new asset clears document and preview selection")
	_expect(editor._lod_preview.states.is_empty(), "new asset releases all LOD preview state")
	editor.free()
	await process_frame
	_remove_fixture_directory(_output)
	_remove_fixture_directory(_source)
	print("asset_editor_preview_test: %s" % ("PASS" if _failures == 0 else "FAIL"))
	quit(_failures)

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
	_expect(panel._metrics.text.contains("LOD3") and panel._metrics.text.contains("32.0"), "readout reports active mesh and projected pixels")
	_set_projected_pixels(editor, camera, 300.0)
	_expect(state.active == 1, "moving closer refines directly")
	_set_projected_pixels(editor, camera, 250.0)
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
	_set_projected_pixels(editor, camera, 16.0)
	_expect(state.active == 1 and state.forced == 1, "forced tier persists through camera changes")
	panel._select_lod(0)
	_expect(state.active == 3, "returning to Automatic recomputes without forced-tier history")
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
	var extra: int = editor._add_mesh_part_from_path(_source.path_join("model_lod0.glb"), "annex")
	editor._select_mesh_part(extra)
	_expect(preview.states.size() == 2 and preview.states[editor._parts[extra]].active == 0, "single-tier part stays visible beside a multi-tier part")
	editor._on_preview_lod_selected(0)
	editor._remove_selected_mesh_parts()
	_expect(preview.states.size() == 1 and preview.states.has(part), "removing a part releases only its own LOD history")
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
	for argument in OS.get_cmdline_user_args():
		if not argument.begins_with("--capture-preview="):
			continue
		var directory := argument.trim_prefix("--capture-preview=")
		_expect(DirAccess.make_dir_recursive_absolute(directory) == OK, "capture output directory")
		editor._select_mesh_part(0)
		editor._frame_selected_part()
		for state in [["day", 10.5, PreviewMaterials.Mode.OFF], ["night", 0.0, PreviewMaterials.Mode.ON]]:
			editor._view._preview_panel.set_lighting(state[1], state[2])
			await process_frame
			await process_frame
			await RenderingServer.frame_post_draw
			_expect(root.get_texture().get_image().save_png(directory.path_join(state[0] + ".png")) == OK, "rendered preview capture")
		editor._view.show_task("model")
		for theme_mode in ["light", "dark"]:
			editor.set_ui_theme_mode(theme_mode)
			await process_frame
			await RenderingServer.frame_post_draw
			_expect(root.get_texture().get_image().save_png(directory.path_join("lod-controls-" + theme_mode + ".png")) == OK, "rendered automatic LOD controls")
		for tab in ["gameplay", "site"]:
			editor._view.show_task(tab)
			await process_frame
			await RenderingServer.frame_post_draw
			_expect(root.get_texture().get_image().save_png(directory.path_join("inspector-%s.png" % tab)) == OK, "rendered inspector spacing")
