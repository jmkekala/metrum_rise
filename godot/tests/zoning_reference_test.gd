# SPDX-License-Identifier: GPL-2.0-only

## Native road/cell reference layouts through the production chunk renderer and terrain mesh.
## Set METRUM_ZONE_REFERENCE_DIR for PNGs and a manifest; use a real rendering device.
extends "res://tests/zoning_cells_tool_test.gd"

const OverlayScript := preload("res://scripts/renderers/zoning_overlay.gd")
var _records: Array[Dictionary] = []
var _output := ""

func _run() -> void:
	_output = OS.get_environment("METRUM_ZONE_REFERENCE_DIR")
	if not _output.is_empty():
		_expect(DisplayServer.get_name() != "headless", "Reference PNGs require a real renderer")
		_expect(DirAccess.make_dir_recursive_absolute(_output) == OK, "Capture directory must exist")
		if _failures != 0:
			quit(1)
			return
	simulation = SimulationNode.new()
	simulation.name = "SimulationNode"
	root.add_child(simulation)
	for fixture in _fixtures():
		await _reference(fixture)
	if not _output.is_empty():
		var manifest := {"engine": Engine.get_version_info(), "adapter": RenderingServer.get_video_adapter_name(), "vendor": RenderingServer.get_video_adapter_vendor(), "fixtures": _records, "failures": _failures}
		var file := FileAccess.open(_output.path_join("manifest.json"), FileAccess.WRITE)
		_expect(file != null, "Reference manifest must open")
		if file != null:
			file.store_string(JSON.stringify(manifest, "\t"))
	simulation.free()
	if _failures == 0:
		print("zoning_reference_test: PASS (%d layouts)" % _records.size())
	quit(0 if _failures == 0 else 1)

func _path(points: Array, angle: float = 0.0) -> PackedVector3Array:
	var result := PackedVector3Array()
	for point: Vector2 in points:
		point = point.rotated(angle)
		result.append(Vector3(point.x, 0.0, point.y))
	return result

func _commit(points: PackedVector3Array, forward: int = 1, backward: int = 1) -> bool:
	var preview_request := simulation.request_preview_road_surface_with_snap(points, forward, backward, true)
	var preview_deadline := Time.get_ticks_msec() + 20000
	var preview: Variant = null
	while preview == null and Time.get_ticks_msec() < preview_deadline:
		preview = simulation.get_preview_road_surface_result(preview_request, 0, PackedInt64Array())
		await process_frame
	_expect(preview is Dictionary and preview.get("is_valid", false), "Reference road preview %s: %s" % [points, preview.get("invalid_reason", "missing") if preview is Dictionary else "timeout"])
	if not preview is Dictionary or not preview.get("is_valid", false):
		return false
	var request := simulation.add_road_with_snap(points, forward, backward, true)
	var deadline := Time.get_ticks_msec() + 20000
	while Time.get_ticks_msec() < deadline:
		var result: Variant = simulation.get_road_commit_result(request)
		if result is Dictionary:
			_expect(result.get("committed", false), "Reference road %s: %s" % [points, result])
			return bool(result.get("committed", false))
		await process_frame
	_expect(false, "Reference road commit did not settle: %s" % points)
	return false

func _fixtures() -> Array[Dictionary]:
	var block := [[Vector2(-100, -200), Vector2(-100, 200)], [Vector2(-100, -150), Vector2(200, -150)], [Vector2(30, -150), Vector2(30, 150)], [Vector2(-100, 150), Vector2(30, 150)]]
	var curve: Array = []
	for i in range(33):
		var x := -320.0 + i * 20.0
		curve.append(Vector2(x, sin(x / 110.0) * 105.0))
	return [
		{"name": "01_curved_groups", "size": 400.0, "height": 620, "roads": [curve]},
		{"name": "02_straight_six_rows", "size": 450.0, "roads": [[Vector2(0, -180), Vector2(0, 180)]]},
		{"name": "03_orthogonal_t", "size": 480.0, "roads": [[Vector2(-120, -190), Vector2(-120, 190)], [Vector2(-120, 0), Vector2(220, 0)]]},
		{"name": "04_orthogonal_block", "size": 460.0, "roads": block},
		{"name": "05_angled_junctions", "size": 610.0, "roads": [[Vector2(-250, 180), Vector2(260, -120)], [Vector2(-70, -180), Vector2(-70, 200)], [Vector2(-10, -240), Vector2(90, -20)]]},
		{"name": "06_competing_roads", "size": 550.0, "roads": [[Vector2(-130, -210), Vector2(-55, 210)], [Vector2(65, -210), Vector2(65, 210)]]},
		{"name": "07_mixed_paint", "size": 220.0, "height": 480, "roads": [[Vector2(-280, 0), Vector2(280, 0)]], "paint": true},
		{"name": "08_rotated_block", "size": 590.0, "roads": block, "rotation": 0.35},
	]

func _paint_reference() -> void:
	for gesture in [
		[Vector2(-100, -12), Vector2(90, -65), 1],
		[Vector2(120, -12), Vector2(250, -65), 4],
		[Vector2(-260, 12), Vector2(-190, 65), 7],
		[Vector2(-150, 12), Vector2(170, 65), 1],
	]:
		var preview := simulation.get_zoning_cells_preview_packed(1, PackedVector2Array([gesture[0], gesture[1]]), 20.0, gesture[2])
		_expect(preview.get("valid", false) and preview.get("cell_count", 0) > 0, "Paint reference marquee must select cells")
		if preview.get("valid", false):
			_expect(simulation.apply_zoning_cells_preview(preview.cells, preview.dependencies, gesture[2]), "Paint reference must commit")

func _reference(fixture: Dictionary) -> void:
	var failures_before := _failures
	_expect(simulation.create_blank_world(2048.0, 2048.0, 8.0, 512.0, 0.0), "Reference world must load")
	simulation.set_simulation_speed(0.0)
	for points: Array in fixture.roads:
		if not await _commit(_path(points, fixture.get("rotation", 0.0))):
			return
	if fixture.get("paint", false):
		_paint_reference()
	# Preview exports authoritative, uninset corners. Checks here exercise the native bridge;
	# exact integer non-overlap and local-frame ownership are also covered by Rust regressions.
	var geometry := simulation.get_zoning_cells_preview_packed(3, PackedVector2Array([Vector2.ZERO]), 650.0, 0)
	var cell_count := int(geometry.get("cell_count", 0))
	_expect(cell_count > 100, "%s must contain generated cells" % fixture.name)
	_validate_squares(geometry, fixture.name)
	_validate_nonoverlap(geometry, fixture.name)
	if fixture.name == "02_straight_six_rows":
		_validate_straight_rows(geometry)
	if fixture.name == "03_orthogonal_t":
		# The compiled junction's curved sidewalk enters the nearest square on each corner.
		_validate_lattice(geometry, Vector2(-110, -60), Vector2i(6, 6), fixture.name + " upper corner", [Vector2(-110, -10)])
		_validate_lattice(geometry, Vector2(-110, 10), Vector2i(6, 6), fixture.name + " lower corner", [Vector2(-110, 10)])
		_validate_lattice(geometry, Vector2(-180, -60), Vector2i(6, 13), fixture.name + " uninterrupted backside")
	if fixture.name == "04_orthogonal_block":
		_validate_lattice(geometry, Vector2(-90, -120), Vector2i(12, 25), fixture.name + " interior")
	if fixture.name == "08_rotated_block":
		_validate_lattice(geometry, Vector2(-90, -120), Vector2i(12, 25), fixture.name + " interior", [], fixture.rotation)
	var viewport := SubViewport.new()
	viewport.size = Vector2i(1440, fixture.get("height", 960))
	viewport.own_world_3d = true
	viewport.render_target_update_mode = SubViewport.UPDATE_ALWAYS
	root.add_child(viewport)
	var scene := Node3D.new()
	viewport.add_child(scene)
	simulation.reparent(scene)
	var camera := Camera3D.new()
	scene.add_child(camera)
	camera.position = Vector3(0, 750, 170)
	camera.projection = Camera3D.PROJECTION_ORTHOGONAL
	camera.size = fixture.size
	camera.far = 2500.0
	camera.look_at(Vector3.ZERO)
	camera.current = true
	var environment := WorldEnvironment.new()
	environment.environment = Environment.new()
	environment.environment.background_mode = Environment.BG_COLOR
	environment.environment.background_color = Color(0.15, 0.19, 0.14)
	environment.environment.ambient_light_source = Environment.AMBIENT_SOURCE_COLOR
	environment.environment.ambient_light_color = Color.WHITE
	environment.environment.ambient_light_energy = 0.7
	scene.add_child(environment)
	var light := DirectionalLight3D.new()
	light.rotation_degrees = Vector3(-60, -25, 0)
	scene.add_child(light)
	var road := RoadToolScript.new()
	road.name = "RoadTool"
	road.simulation_node = simulation
	road.road_mesh_root = Node3D.new()
	road.add_child(road.road_mesh_root)
	var generation := simulation.get_network_render_generation()
	_expect(road.update_main_mesh(generation) == generation, "Reference uses the committed road chunks")
	road.remove_child(road.road_mesh_root)
	scene.add_child(road.road_mesh_root)
	var road_chunks := road._road_chunk_instances.size()
	scene.add_child(await _capture_terrain())
	var overlay := OverlayScript.new()
	scene.add_child(overlay)
	overlay.set_tool_active(true)
	await _settle_cell_overlay(overlay)
	var cell_meshes := 0
	var uploaded_cells := 0
	var palette := {}
	for instance: MeshInstance3D in overlay._cell_chunks._chunks.values():
		if instance.mesh == null:
			continue
		cell_meshes += 1
		var arrays := instance.mesh.surface_get_arrays(0)
		var vertices: PackedVector3Array = arrays[Mesh.ARRAY_VERTEX]
		var colors: PackedColorArray = arrays[Mesh.ARRAY_COLOR]
		uploaded_cells += vertices.size() / 6
		for index in range(0, colors.size(), 6):
			var key := colors[index].to_html(true)
			palette[key] = palette.get(key, 0) + 1
	_expect(uploaded_cells == cell_count, "Visible chunks must upload every cell exactly once")
	if fixture.get("paint", false):
		_expect(palette.size() == 4, "Paint reference must show three profiles and empty cells")
	for frame in range(5):
		await process_frame
	var png := ""
	if not _output.is_empty():
		await RenderingServer.frame_post_draw
		png = str(fixture.name) + ".png"
		_expect(viewport.get_texture().get_image().save_png(_output.path_join(png)) == OK, "Reference image must save")
	_records.append({"name": fixture.name, "image": png, "cells": cell_count, "uploaded_cells": uploaded_cells, "cell_meshes": cell_meshes, "road_chunks": road_chunks, "palette": palette, "camera_size_m": fixture.size, "passed": _failures == failures_before})
	print("zone_reference %s cells=%d meshes=%d" % [fixture.name, cell_count, cell_meshes])
	simulation.reparent(root)
	road.free()
	viewport.free()
	await process_frame

func _settle_cell_overlay(overlay: Node) -> void:
	var deadline := Time.get_ticks_msec() + 20000
	while Time.get_ticks_msec() < deadline:
		await process_frame
		var cells: Node = overlay._cell_chunks
		var keys: Array = cells._visible_chunks()
		var packed := PackedInt32Array()
		for key: Vector2i in keys:
			packed.append_array(PackedInt32Array([key.x, key.y]))
		var metadata := simulation.try_get_zoning_cell_chunk_states(packed)
		if metadata.get("busy", true):
			continue
		var states: PackedInt64Array = metadata.states
		var settled := not keys.is_empty()
		for i in range(keys.size()):
			var expected := PackedInt64Array([states[i * 4], states[i * 4 + 2], states[i * 4 + 3]])
			settled = settled and states[i * 4 + 1] != 0 and cells._versions.get(keys[i], PackedInt64Array()) == expected
		if settled:
			return
	_expect(false, "Production cell overlay must settle all visible chunk versions")

func _validate_squares(payload: Dictionary, label: String) -> void:
	var corners: PackedVector3Array = payload.get("corners", PackedVector3Array())
	var cells: PackedInt64Array = payload.get("cells", PackedInt64Array())
	_expect(corners.size() / 4 == cells.size() / 3, label + " cell addresses and corners must agree")
	var addresses := {}
	for i in range(0, corners.size(), 4):
		var a := Vector2(corners[i].x, corners[i].z)
		var b := Vector2(corners[i + 1].x, corners[i + 1].z)
		var c := Vector2(corners[i + 2].x, corners[i + 2].z)
		var d := Vector2(corners[i + 3].x, corners[i + 3].z)
		_expect(absf(a.distance_to(b) - 10.0) < 0.001 and absf(b.distance_to(c) - 10.0) < 0.001, label + " cells remain 10 m squares")
		_expect(absf((b - a).dot(c - b)) < 0.01 and (a + c).distance_to(b + d) < 0.001, label + " cells remain orthogonal")
		var index := i / 4 * 3
		var key := "%d/%d/%d" % [cells[index], cells[index + 1], cells[index + 2]]
		_expect(not addresses.has(key), label + " must not export duplicate cells")
		addresses[key] = true

func _validate_straight_rows(payload: Dictionary) -> void:
	var corners: PackedVector3Array = payload.corners
	var left := {}
	var right := {}
	for i in range(0, corners.size(), 4):
		var centre := (corners[i] + corners[i + 2]) * 0.5
		if absf(centre.z) < 140.0:
			if centre.x < 0:
				left[roundi(centre.x * 1000.0)] = true
			else:
				right[roundi(centre.x * 1000.0)] = true
	_expect(left.size() == 6 and right.size() == 6, "Unobstructed straight roads must have six rows on both sides")

func _validate_lattice(payload: Dictionary, first: Vector2, size: Vector2i, label: String, road_exclusions: Array[Vector2] = [], angle: float = 0.0) -> void:
	var corners: PackedVector3Array = payload.corners
	var cells: PackedInt64Array = payload.cells
	var centres := {}
	for i in range(0, corners.size(), 4):
		var centre := (corners[i] + corners[i + 2]) * 0.5
		var local := Vector2(centre.x, centre.z).rotated(-angle)
		centres[Vector2i(roundi(local.x * 1000), roundi(local.y * 1000))] = cells[i / 4 * 3]
	var frame: Variant = null
	for x in range(size.x):
		for y in range(size.y):
			var centre := first + Vector2(x, y) * 10.0
			var key := Vector2i(roundi(centre.x * 1000), roundi(centre.y * 1000))
			if centre in road_exclusions:
				_expect(not centres.has(key), label + " must exclude the compiled sidewalk footprint")
				continue
			_expect(centres.has(key), "%s must include cell at %s" % [label, centre])
			if centres.has(key):
				if frame == null:
					frame = centres[key]
				_expect(centres[key] == frame, label + " must use one shared grid frame")

func _validate_nonoverlap(payload: Dictionary, label: String) -> void:
	var corners: PackedVector3Array = payload.corners
	var quads: Array[PackedVector2Array] = []
	var bounds: Array[Rect2] = []
	for i in range(0, corners.size(), 4):
		var quad := PackedVector2Array()
		var box := Rect2(Vector2(corners[i].x, corners[i].z), Vector2.ZERO)
		for j in range(4):
			var point := Vector2(corners[i + j].x, corners[i + j].z)
			quad.append(point)
			box = box.expand(point)
		quads.append(quad)
		bounds.append(box)
	# Fixed small regression scenes: independent SAT over exported float geometry. Rust tests
	# separately enforce zero positive-area overlap on the authoritative micrometre lattice.
	for a in range(quads.size()):
		for b in range(a + 1, quads.size()):
			if bounds[a].intersects(bounds[b]):
				_expect(not _quads_overlap(quads[a], quads[b]), "%s exported cells %d and %d must be disjoint" % [label, a, b])

func _quads_overlap(a: PackedVector2Array, b: PackedVector2Array) -> bool:
	for quad in [a, b]:
		for i in range(2):
			var edge: Vector2 = quad[i + 1] - quad[i]
			var axis := Vector2(-edge.y, edge.x).normalized()
			var a_min := INF
			var a_max := -INF
			var b_min := INF
			var b_max := -INF
			for j in range(4):
				a_min = minf(a_min, axis.dot(a[j]))
				a_max = maxf(a_max, axis.dot(a[j]))
				b_min = minf(b_min, axis.dot(b[j]))
				b_max = maxf(b_max, axis.dot(b[j]))
			# PackedVector3 export uses f32, so boundary contact can differ by a few ULPs.
			if minf(a_max, b_max) - maxf(a_min, b_min) <= 0.0001:
				return false
	return true
