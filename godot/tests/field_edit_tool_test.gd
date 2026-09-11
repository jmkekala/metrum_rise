# SPDX-License-Identifier: GPL-2.0-only

## Exercises vertex picking, release commits, rejection rollback and cancellation through the tool.
## Also checks that the farm inspector keeps resident household details beside its business data.
## Simulation responses are controlled here; native field geometry/economy have Rust regressions.
extends SceneTree

const FieldTool = preload("res://scripts/tools/field_edit_tool.gd")
const IndustryTool = preload("res://scripts/tools/industry_building_tool.gd")
const Inspector = preload("res://scripts/ui/building_inspector.gd")

class SimulationStub extends Node3D:
	var hit := Vector3.ZERO
	var reject := false
	var commits := 0
	var cancelled_buildings: Array[int] = []
	var previous := PackedVector2Array()
	var boundaries := {
		"plot_corners": PackedVector3Array([Vector3(-10, 0.25, -20), Vector3(10, 0.25, -20), Vector3(10, 0.25, 0), Vector3(-10, 0.25, 0)]),
		"building_site_corners": PackedVector3Array([Vector3(-5, 0.25, -15), Vector3(5, 0.25, -15), Vector3(5, 0.25, -5), Vector3(-5, 0.25, -5)]),
	}
	func place_industry_building(_asset: String, _x: float, _z: float) -> Dictionary:
		var result := boundaries.duplicate()
		result.merge({"ok": true, "building_id": 7, "area_kind": "field"})
		return result
	func commit_field_polygon(_id: int, _polygon: PackedVector2Array) -> Dictionary:
		return {"ok": true, "area_m2": 10000.0}
	func cancel_pending_industry_building(id: int) -> bool:
		cancelled_buildings.append(id)
		return true
	func get_world_surface_height(_point: Vector2) -> float:
		return 0.0
	func intersect_world_surface(_origin: Vector3, _direction: Vector3) -> Vector3:
		return hit
	func validate_field_polygon(_id: int, _polygon: PackedVector2Array) -> Dictionary:
		return {"ok": not reject, "error": "field overlaps a road"}
	func resize_field_polygon(id: int, center: Vector2, expected: PackedVector2Array, polygon: PackedVector2Array) -> Dictionary:
		commits += 1
		previous = expected.duplicate()
		if reject:
			return {"ok": false, "error": "field overlaps a road"}
		return {"ok": true, "building_id": id, "center_x": center.x, "center_z": center.y, "field_polygon": polygon, "field_area_m2": 12000.0, "worker_capacity": 2}

class TerrainStub extends Node3D:
	var refreshes := 0
	func mark_field_overlay_dirty() -> void:
		refreshes += 1
	func update_terrain_visuals() -> void:
		pass

class BuildingsStub extends Node3D:
	func update_all_buildings() -> void:
		pass

var _failures := 0

func _initialize() -> void:
	call_deferred("_run")

func _expect(condition: bool, message: String) -> void:
	if not condition:
		_failures += 1
		push_error(message)

func _run() -> void:
	var scene := Node3D.new()
	root.add_child(scene)
	var sim := SimulationStub.new()
	sim.name = "SimulationNode"
	scene.add_child(sim)
	var terrain := TerrainStub.new()
	terrain.name = "Terrain"
	scene.add_child(terrain)
	var buildings := BuildingsStub.new()
	buildings.name = "Buildings"
	scene.add_child(buildings)
	var camera := Camera3D.new()
	scene.add_child(camera)
	camera.position = Vector3(50, 150, 180)
	camera.look_at(Vector3(50, 0, 50))
	camera.current = true
	var tool := FieldTool.new()
	scene.add_child(tool)
	var updates: Array[Dictionary] = []
	tool.field_changed.connect(func(_id: int, details: Dictionary): updates.append(details))
	var polygon := PackedVector2Array([Vector2.ZERO, Vector2(100, 0), Vector2(100, 100), Vector2(0, 100)])
	var details := sim.boundaries.duplicate()
	details.merge({"building_id": 7, "center_x": 0.0, "center_z": -10.0, "field_polygon": polygon})
	tool.begin_edit(details)
	_expect(tool._plot_overlay.visible and tool._plot_overlay.mesh != null, "Resizing displays both farm boundaries")
	var boundary_mesh: Mesh = tool._plot_overlay.mesh
	_expect(tool._handles.size() == 4, "Edit Field exposes exactly the existing vertices")
	var press := InputEventMouseButton.new()
	press.button_index = MOUSE_BUTTON_LEFT
	press.pressed = true
	press.position = camera.unproject_position(tool._handles[2].global_position)
	tool._unhandled_input(press)
	_expect(tool._dragged == 2, "Clicking a handle selects that vertex")
	sim.hit = Vector3(140, 0, 100)
	tool._input(InputEventMouseMotion.new())
	tool._process(0.016)
	_expect(sim.commits == 0 and tool._committed == polygon, "Motion must not mutate the committed field")
	var release := InputEventMouseButton.new()
	release.button_index = MOUSE_BUTTON_LEFT
	tool._input(release)
	_expect(sim.commits == 1 and sim.previous == polygon, "Release commits once against the original polygon")
	_expect(tool._committed[2] == Vector2(140, 100) and terrain.refreshes == 1, "Accepted release refreshes the field")
	_expect(updates.size() == 1 and updates[0]["worker_capacity"] == 2, "Authoritative recalculation reaches the inspector listener")
	_expect(tool._plot_overlay.mesh == boundary_mesh, "Vertex movement reuses the fixed boundary mesh")
	var accepted := tool._committed.duplicate()
	press.position = camera.unproject_position(tool._handles[2].global_position)
	tool._unhandled_input(press)
	sim.reject = true
	sim.hit = Vector3(160, 0, 100)
	tool._input(release)
	_expect(sim.commits == 2 and sim.previous == accepted, "The second release uses the last accepted field")
	_expect(tool._preview == accepted and tool._committed == accepted, "Invalid release restores the last accepted vertices")
	_expect(terrain.refreshes == 1 and updates.size() == 1, "Rejected moves leave committed overlay and details unchanged")
	_expect(tool._hint.text.contains("overlaps a road"), "A rejected move explains the conflict")
	_expect(tool._hint.text.contains("Yellow:") and tool._hint.text.contains("Red:"), "A rejected move keeps the boundary legend visible")
	tool._unhandled_input(press)
	tool.cancel_edit()
	_expect(not tool.active and tool._dragged == -1 and sim.commits == 2, "Cancellation does not commit or delete a farm")
	_expect(not tool._plot_overlay.visible, "Ending a resize hides its boundary guide")
	var industry := IndustryTool.new()
	scene.add_child(industry)
	industry.active = true
	industry.selected_asset_id = "test:farm"
	industry._commit_building_at_mouse()
	_expect(industry._plot_overlay.visible and industry._field_hint.visible, "Drawing a new field immediately displays plot and forbidden-area guides")
	industry._hide_preview_visuals()
	_expect(industry._plot_overlay.visible, "Switching from the building ghost to polygon drawing retains the guide")
	industry._polygon_points.assign(Array(polygon))
	industry._commit_polygon()
	_expect(not industry._plot_overlay.visible and not industry._field_hint.visible, "Finishing the polygon clears the guide")
	_expect(sim.cancelled_buildings.is_empty(), "Finishing a polygon preserves its farm")
	industry._commit_building_at_mouse()
	industry.active = false
	_expect(sim.cancelled_buildings == [7], "Deactivation cancels the pending farm exactly once")
	_expect(not industry._plot_overlay.visible and not industry.is_processing(), "Inactive placement tools clear their guides and stop processing")
	industry._process(0.016)
	industry.active = false
	_expect(sim.cancelled_buildings == [7], "Repeated idle updates do not repeat cancellation")
	_test_farm_inspector(scene)
	scene.free()
	if _failures == 0:
		print("field_edit_tool_test: PASS")
	quit(0 if _failures == 0 else 1)

func _test_farm_inspector(scene: Node) -> void:
	var inspector := Inspector.new()
	scene.add_child(inspector)
	# Use a small view fixture without persistent window placement or user-data writes.
	var window := Window.new()
	inspector.add_child(window)
	var title := Label.new()
	window.add_child(title)
	var body := VBoxContainer.new()
	window.add_child(body)
	var entry := {"window": window, "title_label": title, "stats_body": body}
	var info := {"zone_type": "utility", "field_resource": "grain", "occupancy": 1,
		"household_capacity": 1, "household_count": 1, "child_count": 2,
		"adult_count": 2, "elder_count": 1, "worker_count": 7, "worker_capacity": 8,
		"household_budget_total": 125.0, "household_budget_avg": 125.0,
		"household_stock_total": 12.0, "household_replenishment_state": "Stable"}
	inspector._populate(entry, info)
	var rows := {}
	for row in body.get_children():
		if row is HBoxContainer and row.get_child_count() == 2:
			rows[row.get_child(0).text] = row.get_child(1).text
	_expect(rows.get("Farm Household") == "1 / 1", "Farm inspector retains household occupancy")
	_expect(rows.get("Children") == "2" and rows.get("Adults") == "2" and rows.get("Elders") == "1", "Farm inspector displays resident age groups independently of workers")
	_expect(rows.get("Workers") == "7 / 8", "Household details preserve farm business staffing")
	_expect(rows.get("Household Money") == "$125.0 total / $125.0 avg", "Farm inspector shows household money")
	_expect(rows.get("Supply Units") == "12.0" and rows.get("Replenishment") == "Stable", "Farm inspector shows household supplies")
