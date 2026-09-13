# SPDX-License-Identifier: GPL-2.0-only

## Tests native both-side zoning and the tool's hover cache, option changes and immediate clicks.
## Reuses the native road fixture helpers; controlled cursor responses exercise input transitions.
extends "res://tests/road_junction_preview_test.gd"

class SimulationStub extends Node3D:
	var hit := Vector3.ZERO
	var edge := 4
	var revision := 0
	var previews := 0
	var commits: Array = []
	var single_commits := 0
	var drag_commits := 0
	var reject := false
	var payload := {
		"parcel_count": 1,
		"corners": PackedVector3Array([Vector3(-10, 0, -6), Vector3(10, 0, -6), Vector3(10, 0, -26), Vector3(-10, 0, -26)]),
		"color": Color.GREEN,
	}
	func get_zone_profiles() -> Array:
		return [{"runtime_id": 1}, {"runtime_id": 2}]
	func get_zoning_site_dependencies() -> PackedInt64Array:
		return PackedInt64Array([revision])
	func intersect_world_surface(_origin: Vector3, _direction: Vector3) -> Vector3:
		return hit
	func get_zoning_road_at(_x: float, _z: float) -> int:
		return edge
	func get_zoning_road_preview_packed(_edge: int, _profile: int, _width: int, _depth: int, _gap: float) -> Dictionary:
		previews += 1
		return {"reason": "blocked"} if reject else payload
	func apply_zoning_road_at(x: float, z: float, profile: int, width: int, depth: int, gap: float) -> bool:
		commits.append([x, z, profile, width, depth, gap])
		return not reject
	func get_zoning_parcel_profile_runtime_id_at(_x: float, _z: float) -> int:
		return -1
	func get_zoning_parcel_preview(_x: float, _z: float, _profile: int, _width: int, _depth: int) -> Dictionary:
		return {}
	func apply_zoning_parcel_at(_x: float, _z: float, _profile: int, _width: int, _depth: int) -> bool:
		single_commits += 1
		return true
	func apply_zoning_parcel_drag(_sx: float, _sz: float, _ex: float, _ez: float, _profile: int, _width: int, _depth: int, _gap: float) -> bool:
		drag_commits += 1
		return true

class OverlayStub extends Node3D:
	var refreshes := 0
	func mark_zone_dirty() -> void:
		refreshes += 1

func _run() -> void:
	_test_tool()
	simulation = SimulationNode.new()
	root.add_child(simulation)
	await _test_native()
	await _test_terrain_consistency()
	await _test_manual_group_fill()
	simulation.free()
	if _failures == 0:
		print("zoning_road_tool_test: PASS")
	quit(0 if _failures == 0 else 1)

func _test_tool() -> void:
	var scene := Node3D.new()
	root.add_child(scene)
	var sim := SimulationStub.new()
	sim.name = "SimulationNode"
	scene.add_child(sim)
	var overlay := OverlayStub.new()
	overlay.name = "ZoningOverlay"
	scene.add_child(overlay)
	var camera := Camera3D.new()
	scene.add_child(camera)
	camera.position = Vector3(0, 100, 100)
	camera.look_at(Vector3.ZERO)
	camera.current = true
	var tool := ZoningToolScript.new()
	scene.add_child(tool)
	tool.active = true
	tool._process(0.0)
	_expect(tool.preview_mesh.visible and sim.previews == 1, "Road hover renders Rust geometry")
	var mesh: Mesh = tool.preview_mesh.mesh
	sim.hit.x = 40.0
	tool._process(0.0)
	_expect(tool.preview_mesh.mesh == mesh and sim.previews == 1, "Moving along one road reuses its preview")
	tool.select_profile(2)
	tool.set_parcel_options(3, 4, 5.0)
	tool._process(0.0)
	_expect(sim.previews == 2, "Changing selected options rebuilds the road preview")
	sim.revision += 1
	tool._process(0.0)
	_expect(sim.previews == 3, "Stationary previews refresh on dependency changes")
	sim.edge = 5
	tool._process(0.0)
	_expect(sim.previews == 4, "A different road rebuilds even without pointer movement")
	var press := InputEventMouseButton.new()
	press.button_index = MOUSE_BUTTON_LEFT
	press.pressed = true
	tool._unhandled_input(press)
	_expect(sim.commits == [[40.0, 0.0, 2, 3, 4, 5.0]], "Press commits once with the selected zoning options")
	_expect(not tool.dragging and overlay.refreshes == 1, "Road press ends the gesture and refreshes the overlay")
	press.pressed = false
	tool._unhandled_input(press)
	_expect(sim.commits.size() == 1 and sim.single_commits == 0, "Release cannot commit a second parcel")
	sim.reject = true
	tool._process(0.0)
	_expect(not tool.preview_mesh.visible, "Blocked roads clear prior geometry")
	press.pressed = true
	tool._unhandled_input(press)
	_expect(overlay.refreshes == 1 and not tool.dragging, "Rejected road clicks cannot fall through to single placement")
	sim.edge = -1
	tool._process(0.0)
	_expect(not tool.preview_mesh.visible, "Leaving a road cannot retain its lots")
	tool._unhandled_input(press)
	press.pressed = false
	tool._unhandled_input(press)
	_expect(sim.single_commits == 1, "Off-road clicks preserve single parcel placement")
	press.pressed = true
	tool._unhandled_input(press)
	sim.hit.x += 20.0
	press.pressed = false
	tool._unhandled_input(press)
	_expect(sim.drag_commits == 1, "Off-road drags preserve parcel runs")
	tool.active = false
	tool._process(0.0)
	_expect(not tool.preview_mesh.visible, "Deactivating the tool clears the preview")
	scene.free()

func _test_native() -> void:
	_expect(simulation.create_blank_world(512.0, 512.0, 8.0, 128.0, 0.0), "Zoning fixture must load")
	simulation.set_simulation_speed(0.0)
	if not await _commit(PackedVector3Array([Vector3(-96, 0, 0), Vector3(96, 0, 0)])):
		return
	var edge := simulation.get_zoning_road_at(0.0, 0.0)
	_expect(edge >= 0 and simulation.get_zoning_road_at(0.0, 40.0) == -1, "Native picking distinguishes roads from adjacent land")
	var before := simulation.get_zoning_site_dependencies()
	var rejected := simulation.get_zoning_road_preview_packed(edge, 65535, 2, 3, 5.0)
	_expect(rejected.get("parcel_count", 0) > 0 and rejected.get("valid_count", -1) == 0, "Unknown profiles keep red preview lots on both sides")
	_expect(not simulation.apply_zoning_road_at(0.0, 0.0, 65535, 2, 3, 5.0), "Unknown profile cannot commit")
	_expect(simulation.get_zoning_site_dependencies() == before, "Preview and rejected commit do not change terrain or parcels")
	var preview := simulation.get_zoning_road_preview_packed(edge, 0, 2, 3, 5.0)
	_expect(preview.get("parcel_count", 0) > 0 and preview.get("valid_count", -1) == preview.get("parcel_count", 0), "Free lots remain buildable without assets")
	for profile in simulation.get_zone_profiles():
		var zoned := simulation.get_zoning_road_preview_packed(edge, profile.runtime_id, 2, 3, 5.0)
		_expect(zoned.get("valid_count", -1) == preview.parcel_count, "Every zoning density accepts lots without assets")
		_expect(zoned.get("corners") == preview.corners, "Missing assets do not change the road layout")
	_expect(simulation.get_zoning_site_dependencies() == before, "Previews do not change terrain or parcels")
	_expect(simulation.apply_zoning_road_at(0.0, 0.0, 2, 2, 3, 5.0), "Native click commits medium-density lots without assets")
	var parcels := simulation.get_zoning_parcels_overlay()
	_expect(parcels.size() == preview.get("valid_count", -1), "Commit count matches preview")
	var corners: PackedVector3Array = preview.get("corners", PackedVector3Array())
	for index in range(parcels.size()):
		_expect(parcels[index].corners == corners.slice(index * 4, index * 4 + 4), "Committed geometry matches preview exactly")
		_expect(parcels[index].profile_runtime_id == 2, "Committed lots retain their selected density")
	_expect(not simulation.apply_zoning_road_at(0.0, 0.0, 2, 2, 3, 5.0), "Repeated road clicks do not duplicate parcels")
	_expect(simulation.get_zoning_parcels_overlay().size() == parcels.size(), "Repeated click preserves stored parcels")
	simulation.set_no_building_spawn(edge, true)
	_expect(simulation.get_zoning_road_at(0.0, 0.0) == edge, "No-build roads remain explicit hover targets")
	_expect(simulation.get_zoning_road_preview_packed(edge, 0, 2, 3, 5.0).get("parcel_count", 0) == 0, "No-build roads have no placeable preview")
	_expect(not simulation.apply_zoning_road_at(0.0, 0.0, 0, 2, 3, 5.0), "No-build flag blocks road zoning")

func _test_terrain_consistency() -> void:
	_expect(simulation.create_blank_world(512.0, 512.0, 8.0, 128.0, 0.0), "Terrain zoning fixture must load")
	simulation.set_simulation_speed(0.0)
	if not await _commit(PackedVector3Array([Vector3(-96, 0, 0), Vector3(96, 0, 0)])):
		return
	_expect(simulation.get_registered_asset_ids().is_empty(), "Terrain zoning fixture has no assets")
	var edge := simulation.get_zoning_road_at(0.0, 0.0)
	var flat := simulation.get_zoning_road_preview_packed(edge, 1, 2, 2, 0.0)
	_expect(flat.get("valid_count", 0) > 0, "Flat ground accepts zoning without assets")
	simulation.level_terrain(Vector2(0, 40), 32.0, 100.0, 1.0)
	var reference := simulation.get_zoning_road_preview_packed(edge, 1, 2, 2, 0.0)
	_expect(reference.valid_count > 0 and reference.valid_count < reference.parcel_count, "Steep ground rejects only the affected lots without assets")
	var colors: PackedColorArray = reference.colors
	for profile in simulation.get_zone_profiles():
		if profile.zone_type != "residential":
			continue
		var preview := simulation.get_zoning_road_preview_packed(edge, profile.runtime_id, 2, 2, 0.0)
		_expect(preview.valid_count == reference.valid_count and preview.corners == reference.corners, "Every density has identical terrain eligibility and geometry")
		for index in range(colors.size()):
			_expect((preview.colors[index].r > preview.colors[index].g) == (colors[index].r > colors[index].g), "Every density marks the same steep lots red")
		_expect(not simulation.get_zoning_parcel_preview(0.0, 16.0, profile.runtime_id, 2, 2).get("valid", true), "Single lots use the same asset-independent terrain check")
	_expect(simulation.apply_zoning_road_at(0.0, 0.0, 2, 2, 2, 0.0), "Medium-density road fill commits the terrain-valid subset")
	_expect(simulation.get_zoning_parcels_overlay().size() == reference.valid_count, "Terrain preview and commit accept the same number of lots")

func _test_manual_group_fill() -> void:
	_expect(simulation.create_blank_world(512.0, 512.0, 8.0, 128.0, 0.0), "Manual group zoning fixture must load")
	simulation.set_simulation_speed(0.0)
	if not await _commit(PackedVector3Array([Vector3(-96, 0, 0), Vector3(96, 0, 0)])):
		return
	for x in [-17.0, 3.0, 23.0]:
		_expect(simulation.apply_zoning_parcel_at(x, -16.0, 1, 2, 2), "Place manual lot north of the road")
	for x in [-34.0, -14.0, 6.0, 26.0, 46.0]:
		_expect(simulation.apply_zoning_parcel_at(x, 16.0, 1, 2, 2), "Place manual lot south of the road")
	var existing := simulation.get_zoning_parcels_overlay()
	var edge := simulation.get_zoning_road_at(0.0, 0.0)
	var preview := simulation.get_zoning_road_preview_packed(edge, 2, 2, 2, 0.0)
	_expect(preview.get("valid_count", 0) == 10, "Automatic fill packs the remaining ten lots around eight manual lots")
	for north in [true, false]:
		var z := -16.0 if north else 16.0
		var manual := simulation.get_zoning_parcel_drag_preview_packed(-86.0, z, 86.0, z, 2, 2, 2, 0.0)
		var manual_corners: PackedVector3Array = manual.get("corners", PackedVector3Array())
		var automatic_corners: PackedVector3Array = preview.get("corners", PackedVector3Array())
		var row_count := 0
		for index in range(0, automatic_corners.size(), 4):
			if (automatic_corners[index].z < 0.0) != north:
				continue
			row_count += 1
			var matched := false
			for manual_index in range(0, manual_corners.size(), 4):
				if manual_corners.slice(manual_index, manual_index + 4) == automatic_corners.slice(index, index + 4):
					matched = true
			_expect(matched, "Manual drag and automatic fill use identical anchored lots")
		_expect(manual_corners.size() == row_count * 4, "Manual drag contains exactly its automatic road row")
	_expect(simulation.apply_zoning_road_at(0.0, 0.0, 2, 2, 2, 0.0), "Fill around both manual groups")
	var placed := simulation.get_zoning_parcels_overlay()
	_expect(placed.size() == existing.size() + preview.get("valid_count", 0), "Fill commits exactly the previewed additions")
	for index in range(existing.size()):
		_expect(placed[index] == existing[index], "Automatic fill preserves manual parcels and profiles")
	var corners: PackedVector3Array = preview.get("corners", PackedVector3Array())
	for index in range(placed.size() - existing.size()):
		_expect(placed[existing.size() + index].corners == corners.slice(index * 4, index * 4 + 4), "Anchored fill preview and committed geometry match")
	for north in [true, false]:
		var centers: Array[float] = []
		for parcel in placed:
			var center: Vector3 = (parcel.corners[0] + parcel.corners[1]) * 0.5
			if (center.z < 0.0) == north:
				centers.append(center.x)
		centers.sort()
		for index in range(1, centers.size()):
			_expect(absf(centers[index] - centers[index - 1] - 20.0) < 0.002, "Zero gap joins automatic lots to both ends of each manual group")
	_expect(not simulation.apply_zoning_road_at(0.0, 0.0, 2, 2, 2, 0.0), "A second fill adds no overlapping or duplicate lots")
