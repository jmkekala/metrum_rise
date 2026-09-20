# SPDX-License-Identifier: GPL-2.0-only

## Building refresh cadence and pack replacement through the real registry, importer and renderer.
extends SceneTree

const BuildingRenderer := preload("res://scripts/renderers/buildings.gd")
const ModPackConfig := preload("res://scripts/core/mod_pack_config.gd")
var _failures := 0

class ObservedRenderer extends BuildingRenderer:
	var refreshes := 0
	func _update_building_render_frame(force_site_mesh: bool = false) -> bool:
		refreshes += 1
		return super._update_building_render_frame(force_site_mesh)

func _initialize() -> void:
	call_deferred("_run")

func _expect(condition: bool, message: String) -> void:
	if not condition:
		_failures += 1
		push_error(message)

func _write(path: String, content: String) -> void:
	var file := FileAccess.open(path, FileAccess.WRITE)
	_expect(file != null, "fixture file must open: " + path)
	if file:
		file.store_string(content)

func _write_model(path: String, size: Vector3) -> void:
	var scene := Node3D.new()
	var instance := MeshInstance3D.new()
	var mesh := BoxMesh.new()
	mesh.size = size
	instance.mesh = mesh
	scene.add_child(instance)
	var document := GLTFDocument.new()
	var state := GLTFState.new()
	_expect(document.append_from_scene(scene, state) == OK, "fixture model must export")
	_expect(document.write_to_filesystem(state, path) == OK, "fixture GLB must write")
	scene.free()

func _write_manifest(path: String, parts: int) -> void:
	var manifest := """asset_id = "building.residential.reload_test"
display_name = "Reload test"
[building]
placement_mode = "zoned_private"
zone_type = "residential"
density = "low"
lot_width_cells = 2
lot_depth_cells = 2
household_capacity = 1
[[anchors]]
type = "entrance"
name = "main"
position = [0.0, 0.0, 1.0]
forward = [0.0, 0.0, 1.0]
"""
	for index in parts:
		manifest += """[[mesh_parts]]
name = "part_%d"
[[mesh_parts.lods]]
file = "model.glb"
distance_min_m = 0.0
distance_max_m = 150.0
""" % index
	_write(path, manifest)

func _run() -> void:
	# Save and restore the user's selection; fixtures use a unique pack directory.
	var had_config := FileAccess.file_exists(ModPackConfig.CFG_PATH)
	var original_config := FileAccess.get_file_as_bytes(ModPackConfig.CFG_PATH) if had_config else PackedByteArray()
	var pack_id := "reload-test-%d" % OS.get_process_id()
	var pack_dir := ProjectSettings.globalize_path("user://mods/" + pack_id)
	_expect(DirAccess.make_dir_recursive_absolute(pack_dir) == OK, "fixture pack directory must exist")
	_write(pack_dir.path_join("pack.toml"), """pack_id = "%s"
schema_version = 1
display_name = "Reload test"
version = "1.0.0"
author = "Test"
license = "CC0"
""" % pack_id)
	var model_path := pack_dir.path_join("model.glb")
	var manifest_path := pack_dir.path_join("asset.toml")
	_write_model(model_path, Vector3(1, 2, 3))
	_write_manifest(manifest_path, 2)
	_expect(ModPackConfig.save_enabled_pack_ids([pack_id]) == OK, "fixture selection must save")
	var simulation := SimulationNode.new()
	root.add_child(simulation)
	simulation.set_simulation_speed(0.0)
	var renderer := ObservedRenderer.new()
	renderer.simulation_node = simulation
	renderer.reload_asset_packs()
	var asset_id := pack_id + ":building.residential.reload_test"
	_expect(simulation.get_registered_asset_ids() == PackedStringArray([asset_id]), "only the enabled fixture pack must load")
	_expect(renderer.lod_renderer.catalog.size() == 3, "two asset parts and the broken placeholder must render")
	_expect(renderer.lod_renderer.resources.imports == 1, "shared paths must import once")
	_expect(renderer.lod_renderer.batches.is_empty(), "unplaced assets need no batches")
	var original_mesh: Mesh = renderer.get_building_mesh_for_asset_part(asset_id, 0)
	_expect(original_mesh.get_aabb().size.is_equal_approx(Vector3(1, 2, 3)), "initial mesh must come from the fixture GLB")
	_write_model(model_path, Vector3(2, 4, 6))
	_write_manifest(manifest_path, 1)
	renderer.reload_asset_packs()
	var changed_mesh: Mesh = renderer.get_building_mesh_for_asset_part(asset_id, 0)
	_expect(changed_mesh.get_aabb().size.is_equal_approx(Vector3(2, 4, 6)), "reload must replace geometry at the same asset ID and path")
	_expect(renderer.get_building_mesh_for_asset_part(asset_id, 1) == null, "reload must remove deleted part resources")
	_expect(renderer.lod_renderer.catalog.size() == 2, "reloaded catalog has only surviving part and placeholder")
	var child_count := renderer.get_child_count()
	_expect(child_count == 1, "unplaced assets need only the LOD coordinator")
	renderer.reload_asset_packs()
	_expect(renderer.get_child_count() == child_count, "repeated reload must not accumulate renderer nodes")
	_expect(ModPackConfig.save_enabled_pack_ids([]) == OK, "empty selection must save")
	renderer.reload_asset_packs()
	_expect(simulation.get_registered_asset_ids().is_empty(), "disabling every pack must clear the registry")
	_expect(renderer.lod_renderer.catalog.size() == 1 and renderer.lod_renderer.batches.is_empty(), "disabled packs must leave only the broken placeholder")
	var mods_dir := ProjectSettings.globalize_path("user://mods/")
	simulation.load_all_asset_packs(mods_dir)
	_expect(simulation.get_registered_asset_ids().has(asset_id), "authoring must load installed assets even when gameplay packs are disabled")
	simulation.load_asset_packs(mods_dir, PackedStringArray(["not-installed"]))
	_expect(simulation.get_registered_asset_ids().is_empty(), "a selection must never fall back to loading all packs")
	_expect(ModPackConfig.save_enabled_pack_ids([pack_id]) == OK, "fixture pack must be re-enabled")
	renderer.reload_asset_packs()
	if "--benchmark-building-poll" in OS.get_cmdline_user_args():
		await _benchmark(renderer, manifest_path)
	else:
		var previous_refreshes := renderer.refreshes
		for frame in 60:
			await process_frame
			renderer._update_building_render_frame()
			var idle: Dictionary = simulation.try_get_building_site_frame(renderer._zone_ids, renderer.building_site_revision, renderer.building_visual_revision)
			if not idle.get("busy", true):
				_expect(not idle.has("plot_transforms") and not idle.has("site_mesh_data"), "idle requests must not rebuild site buffers")
		_expect(renderer.refreshes - previous_refreshes == 60, "requests are state-driven rather than a 30-frame timer")
	DirAccess.remove_absolute(manifest_path)
	renderer.reload_asset_packs()
	_expect(simulation.get_registered_asset_ids().is_empty() and renderer.lod_renderer.catalog.size() == 1, "deleting a manifest must remove its registry and renderer entries")
	renderer.free()
	simulation.free()
	for filename in ["model.glb", "pack.toml"]:
		DirAccess.remove_absolute(pack_dir.path_join(filename))
	DirAccess.remove_absolute(pack_dir)
	if had_config:
		var file := FileAccess.open(ModPackConfig.CFG_PATH, FileAccess.WRITE)
		file.store_buffer(original_config)
	else:
		DirAccess.remove_absolute(ModPackConfig.CFG_PATH)
	print("building_asset_reload_test: %s" % ("PASS" if _failures == 0 else "FAIL"))
	quit(_failures)

func _benchmark(renderer: ObservedRenderer, manifest_path: String) -> void:
	# Empty-world frame requests isolate asset-part polling and the bridge. Model import,
	# manifest validation, node setup and correctness checks stay outside the timed loop.
	for parts in [2, 128, 1024]:
		_write_manifest(manifest_path, parts)
		renderer.reload_asset_packs()
		_expect(renderer.lod_renderer.catalog.size() == parts + 1, "benchmark must request every authored part plus the broken placeholder")
		for warmup in 3:
			renderer._update_building_render_frame()
		var samples: Array[float] = []
		var busy_calls := 0
		for sample in 11:
			var elapsed_us := 0
			var completed := 0
			var deadline := Time.get_ticks_msec() + 10000
			while completed < 20 and Time.get_ticks_msec() < deadline:
				var start := Time.get_ticks_usec()
				var ready := renderer._update_building_render_frame()
				var duration_us := Time.get_ticks_usec() - start
				if ready:
					completed += 1
					elapsed_us += duration_us
				else:
					busy_calls += 1
					await process_frame
			_expect(completed == 20, "benchmark must measure completed frames, excluding busy replies")
			samples.append(float(elapsed_us) / maxf(1.0, completed))
		samples.sort()
		print("building_poll: parts=%d median_us=%.3f busy_calls=%d" % [parts, samples[samples.size() / 2], busy_calls])
