# SPDX-License-Identifier: GPL-2.0-only

## Native cell/parcel overlay locality and real-device frame measurements.
## Run explicitly with METRUM_ZONE_OVERLAY_DIR; setup and surface inspection are not timed.
extends "res://tests/zoning_reference_test.gd"

const BACKGROUNDS := [0, 8, 32, 128]
const WARMUP := 20
const SAMPLES := 101
const IDLE_P95_US := 500.0
const EDIT_P95_US := 5000.0
const SETTLE_P95_US := 50000.0
const GPU_P95_MS := 2.0
const UPLOAD_BYTES_LIMIT := 128 * 1024

var _benchmark_rows: Array[Dictionary] = []
var _local_geometry := {}
var _remote_cells := 0

func _run() -> void:
	_output = OS.get_environment("METRUM_ZONE_OVERLAY_DIR")
	_expect(not _output.is_empty(), "Set METRUM_ZONE_OVERLAY_DIR for benchmark artifacts")
	_expect(DisplayServer.get_name() != "headless", "Overlay timing requires a real rendering device")
	if _failures != 0:
		quit(1)
		return
	_expect(DirAccess.make_dir_recursive_absolute(_output) == OK, "Benchmark directory must exist")
	DisplayServer.window_set_vsync_mode(DisplayServer.VSYNC_DISABLED)
	Engine.max_fps = 0
	var viewport := SubViewport.new()
	viewport.size = Vector2i(1440, 960)
	viewport.own_world_3d = true
	viewport.render_target_update_mode = SubViewport.UPDATE_ALWAYS
	root.add_child(viewport)
	RenderingServer.viewport_set_measure_render_time(viewport.get_viewport_rid(), true)
	var scene := Node3D.new()
	viewport.add_child(scene)
	simulation = SimulationNode.new()
	simulation.name = "SimulationNode"
	scene.add_child(simulation)
	var setup_start := Time.get_ticks_usec()
	_expect(simulation.create_blank_world(8192.0, 8192.0, 8.0, 512.0, 0.0), "Overlay world must load")
	simulation.set_simulation_speed(0.0)
	_expect(simulation.get_registered_asset_ids().is_empty(), "Fixture excludes asset-driven building growth")
	if not await _commit(_path([Vector2(-280, 0), Vector2(280, 0)])):
		quit(1)
		return
	_expect(simulation.apply_zoning_parcel_at(-160.0, -16.0, 2, 2, 2), "Mixed fixture includes a manual parcel")
	var world_setup_us := Time.get_ticks_usec() - setup_start
	var camera := Camera3D.new()
	scene.add_child(camera)
	camera.position = Vector3(0, 750, 170)
	camera.projection = Camera3D.PROJECTION_ORTHOGONAL
	camera.size = 600.0
	camera.far = 2500.0
	camera.look_at(Vector3.ZERO)
	camera.current = true
	var overlay := OverlayScript.new()
	scene.add_child(overlay)
	overlay.set_tool_active(true)
	await _settle_cell_overlay(overlay)
	# Drive precisely one production cell update per measured frame. The parent handles
	# unrelated road eligibility lines, which are outside this cell/parcel mesh fixture.
	overlay.set_process(false)
	var cells: Node = overlay._cell_chunks
	cells.set_process(false)
	_local_geometry = _geometry(cells)
	var count := 0
	for background: int in BACKGROUNDS:
		setup_start = Time.get_ticks_usec()
		while count < background:
			if not await _add_remote_road(count):
				quit(1)
				return
			count += 1
		var remote_setup_us := Time.get_ticks_usec() - setup_start
		cells.set_process(true)
		await _settle_cell_overlay(overlay)
		cells.set_process(false)
		_expect(_geometry(cells) == _local_geometry, "Distant roads and paint preserve exact local mesh arrays")
		var state := simulation.get_road_benchmark_state()
		_expect(state.live_edges == background + 1, "Native background cardinality must match")
		var idle := await _measure(cells, viewport, false)
		var edits := await _measure(cells, viewport, true)
		var row := {"remote_roads": background, "remote_painted_cells": _remote_cells,
			"world_setup_us": world_setup_us, "incremental_background_setup_us": remote_setup_us,
			"native_state": state, "visible_chunks": cells._chunks.size(),
			"resident_geometry": _resident_bytes(cells), "idle": idle, "edits": edits}
		_benchmark_rows.append(row)
		print("ZONE_OVERLAY_BENCHMARK ", JSON.stringify(row))
		# Every measured sequence returns the local cell to its original unpainted state.
		_expect(_geometry(cells) == _local_geometry, "Paint/erase trials restore exact local geometry and colors")
	var manifest := {"engine": Engine.get_version_info(), "adapter": RenderingServer.get_video_adapter_name(),
		"vendor": RenderingServer.get_video_adapter_vendor(), "warmup": WARMUP, "samples": SAMPLES,
		"viewport": [1440, 960], "budgets": {"idle_p95_us": IDLE_P95_US, "edit_p95_us": EDIT_P95_US,
			"settle_p95_us": SETTLE_P95_US, "gpu_p95_ms": GPU_P95_MS, "edit_packed_bytes": UPLOAD_BYTES_LIMIT},
		"rows": _benchmark_rows, "failures": _failures}
	var file := FileAccess.open(_output.path_join("results.json"), FileAccess.WRITE)
	_expect(file != null, "Benchmark results must open")
	if file != null:
		file.store_string(JSON.stringify(manifest, "\t"))
	viewport.free()
	# Finish the draw callback before requesting engine shutdown, as in the reference fixture.
	await process_frame
	print("zoning_overlay_benchmark: %s" % ("PASS" if _failures == 0 else "FAIL"))
	quit(0 if _failures == 0 else 1)

func _add_remote_road(index: int) -> bool:
	var centre := Vector2(1500.0 + (index % 8) * 300.0, -2400.0 + (index / 8) * 300.0)
	if not await _commit(_path([centre - Vector2(80, 0), centre + Vector2(80, 0)])):
		return false
	for side: float in [-20.0, 20.0]:
		var preview := simulation.get_zoning_cells_preview_packed(2, PackedVector2Array([centre + Vector2(0, side)]), 20.0, 1)
		_expect(preview.get("valid", false), "Each remote road side must produce a connected fill")
		if not preview.get("valid", false):
			return false
		_expect(simulation.apply_zoning_cells_preview(preview.cells, preview.dependencies, 1), "Remote fill must commit")
		_remote_cells += int(preview.cell_count)
	return true

func _measure(cells: Node, viewport: SubViewport, editing: bool) -> Dictionary:
	var updates: Array = []
	var operation_updates: Array = []
	var settle_times: Array = []
	var presented_times: Array = []
	var render_cpu: Array = []
	var render_gpu: Array = []
	var frames: Array = []
	var packed_bytes: Array = []
	var mesh_bytes: Array = []
	var replacements: Array = []
	var draw_calls: Array = []
	var primitives: Array = []
	for sample in range(WARMUP + SAMPLES):
		var before := _identities(cells)
		if editing:
			# Erase on even samples; paint on odd samples. Seed paint makes every sample
			# an actual edit and the last (even) sample leaves the fixture unpainted.
			if sample == 0:
				_paint_one(1)
				await _flush(cells)
				before = _identities(cells)
			_paint_one(sample % 2)
		# Include queued work and frame waits from the completed commit until all visible
		# versions match. Setup, gesture selection and geometry inspection remain separate.
		var settle_start := Time.get_ticks_usec()
		var elapsed := 0
		var worst_frame := 0
		var attempts := 0
		var changed: Array = []
		var settled := false
		while attempts < 100:
			var frame_before := _identities(cells)
			var start := Time.get_ticks_usec()
			cells._process(0.0)
			var duration := Time.get_ticks_usec() - start
			elapsed += duration
			worst_frame = maxi(worst_frame, duration)
			attempts += 1
			_expect(_changed(frame_before, cells).size() <= cells.UPLOADS_PER_FRAME, "A frame respects the mesh upload cap")
			changed = _changed(before, cells)
			settled = _versions_match(cells)
			if settled:
				break
			await process_frame
		var settle_us := Time.get_ticks_usec() - settle_start
		_expect(settled, "Each operation must settle all visible generations and versions")
		_expect(changed.size() == (1 if editing else 0), "Only the edited chunk may replace its mesh; idle replaces none")
		var bytes := 0
		var buffers := 0
		for key: Vector2i in changed:
			_expect(key == Vector2i(0, 0), "A one-cell edit only replaces its owning chunk")
			var instance: MeshInstance3D = cells._chunks[key]
			var sizes := _mesh_bytes(instance.mesh)
			bytes += sizes.packed_bytes
			buffers += sizes.surface_buffer_bytes
		await process_frame
		await RenderingServer.frame_post_draw
		var presented_us := Time.get_ticks_usec() - settle_start
		if sample >= WARMUP:
			updates.append(worst_frame)
			operation_updates.append(elapsed)
			settle_times.append(settle_us)
			presented_times.append(presented_us)
			render_cpu.append(RenderingServer.viewport_get_measured_render_time_cpu(viewport.get_viewport_rid()))
			render_gpu.append(RenderingServer.viewport_get_measured_render_time_gpu(viewport.get_viewport_rid()))
			frames.append(attempts)
			packed_bytes.append(bytes)
			mesh_bytes.append(buffers)
			replacements.append(changed.size())
			draw_calls.append(viewport.get_render_info(Viewport.RENDER_INFO_TYPE_VISIBLE, Viewport.RENDER_INFO_DRAW_CALLS_IN_FRAME))
			primitives.append(viewport.get_render_info(Viewport.RENDER_INFO_TYPE_VISIBLE, Viewport.RENDER_INFO_PRIMITIVES_IN_FRAME))
	var result := {"update_us": _summary(updates), "render_cpu_ms": _summary(render_cpu),
		"operation_update_us": _summary(operation_updates), "frames_to_settle": _summary(frames),
		"settle_us": _summary(settle_times),
		"presented_us": _summary(presented_times),
		"gpu_ms": _summary(render_gpu), "draw_calls": _summary(draw_calls), "primitives": _summary(primitives),
		"packed_bytes": _summary(packed_bytes), "surface_buffer_bytes": _summary(mesh_bytes),
		"replaced_chunks": _summary(replacements)}
	_expect(result.update_us.p95 <= (EDIT_P95_US if editing else IDLE_P95_US), "Cell update p95 must meet its fixed budget")
	_expect(result.settle_us.p95 <= SETTLE_P95_US, "Cell completion p95 must meet its fixed budget")
	_expect(result.presented_us.p95 <= SETTLE_P95_US, "Cell presentation p95 must meet its fixed budget")
	_expect(result.gpu_ms.p95 > 0.0 and result.gpu_ms.p95 <= GPU_P95_MS, "Measured viewport GPU p95 must meet its fixed budget")
	_expect(result.primitives.min > 0 and result.draw_calls.min > 0, "Viewport timing must include rendered geometry")
	_expect(result.packed_bytes.max <= (UPLOAD_BYTES_LIMIT if editing else 0), "Geometry payload stays within the fixed local upload budget")
	return result

func _paint_one(profile: int) -> void:
	var preview := simulation.get_zoning_cells_preview_packed(0, PackedVector2Array([Vector2(25, 25)]), 20.0, profile)
	_expect(preview.get("cell_count", 0) == 1, "Local edit selects exactly one cell")
	_expect(simulation.apply_zoning_cells_preview(preview.cells, preview.dependencies, profile), "Local gesture commits")

func _flush(cells: Node) -> void:
	for attempt in range(100):
		cells._process(0.0)
		if _versions_match(cells):
			return
		await process_frame
	_expect(false, "Seed paint must reach the renderer")

func _versions_match(cells: Node) -> bool:
	var keys: Array = cells._visible_chunks()
	var packed := PackedInt32Array()
	for key: Vector2i in keys:
		packed.append(key.x)
		packed.append(key.y)
	var metadata := simulation.try_get_zoning_cell_chunk_states(packed)
	if metadata.get("busy", true):
		return false
	var states: PackedInt64Array = metadata.states
	if states.size() != keys.size() * 5:
		return false
	for i in range(keys.size()):
		var expected := PackedInt64Array([states[i * 5], states[i * 5 + 2], states[i * 5 + 3], states[i * 5 + 4]])
		if states[i * 5 + 1] == 0 or cells._versions.get(keys[i], PackedInt64Array()) != expected:
			return false
	return not keys.is_empty()

func _identities(cells: Node) -> Dictionary:
	var result := {}
	for key: Vector2i in cells._chunks:
		var instance: MeshInstance3D = cells._chunks[key]
		result[key] = [instance.get_instance_id(), instance.mesh.get_instance_id() if instance.mesh != null else 0]
	return result

func _changed(before: Dictionary, cells: Node) -> Array:
	var result := []
	var after := _identities(cells)
	_expect(before.keys() == after.keys(), "Fixed camera retains the same chunk instances")
	for key: Vector2i in after:
		if before.get(key) != after[key]:
			result.append(key)
	return result

func _geometry(cells: Node) -> Dictionary:
	var result := {}
	for key: Vector2i in cells._chunks:
		var instance: MeshInstance3D = cells._chunks[key]
		var surfaces := []
		if instance.mesh != null:
			for surface in range(instance.mesh.get_surface_count()):
				surfaces.append(instance.mesh.surface_get_arrays(surface))
		result[key] = surfaces
	return result

func _mesh_bytes(mesh: Mesh) -> Dictionary:
	var packed := 0
	var buffers := 0
	for surface in range(mesh.get_surface_count()):
		var arrays := mesh.surface_get_arrays(surface)
		# The native payload contains only Vector3 positions and float Color values.
		packed += arrays[Mesh.ARRAY_VERTEX].size() * 12 + arrays[Mesh.ARRAY_COLOR].size() * 16
		var raw := RenderingServer.mesh_get_surface(mesh.get_rid(), surface)
		for field in ["vertex_data", "attribute_data", "skin_data", "index_data", "blend_shape_data"]:
			buffers += raw.get(field, PackedByteArray()).size()
	return {"packed_bytes": packed, "surface_buffer_bytes": buffers}

func _resident_bytes(cells: Node) -> Dictionary:
	var packed := 0
	var buffers := 0
	var meshes := 0
	for instance: MeshInstance3D in cells._chunks.values():
		if instance.mesh != null:
			var sizes := _mesh_bytes(instance.mesh)
			packed += sizes.packed_bytes
			buffers += sizes.surface_buffer_bytes
			meshes += 1
	return {"meshes": meshes, "packed_bytes": packed, "surface_buffer_bytes": buffers}

func _summary(values: Array) -> Dictionary:
	values.sort()
	return {"min": values[0], "p50": values[values.size() / 2], "p95": values[int(values.size() * 0.95)], "max": values[-1]}
