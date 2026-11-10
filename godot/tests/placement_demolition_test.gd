# SPDX-License-Identifier: GPL-2.0-only

## Plotted placements over zoned land (`ZONE-11`, round RZ11b, PLAN-RZ 11.6 test 6): on a street
## lined with houses (`METRUM_PLACEMENT_DEMOLITION_FIXTURE`, written by the Rust recorder
## `record_placement_demolition_fixture`, with the houses' front line and the pointers in
## `<fixture>.json`), the service tool's hover over the houses outlines the lots the parking lot
## takes with the bulldoze highlight and names the buildings in its price label, and the click
## demolishes them; with the core held by the simulation thread the hover answers at once and the
## tool keeps its last preview. The field editor's drag over a placed lot shows the refusal in its
## hint; its drag over houses outlines their lots and names them, and the release takes them. The
## industry tool's extraction area closed over houses does the same. Each part loads the fixture
## afresh. A top-down orthographic camera is moved so the pointer's ray meets the point a step
## needs, so the tools resolve the pointer as they do in the game.
extends SceneTree

const ServiceTool := preload("res://scripts/tools/service_building_tool.gd")
const IndustryTool := preload("res://scripts/tools/industry_building_tool.gd")
const FieldTool := preload("res://scripts/tools/field_edit_tool.gd")
const TIMEOUT_MS := 20000
const FARM := "kenney:building.industrial.small_farm"
const MINE := "kenney:building.coal.mine"

class TerrainStub extends Node3D:
	func update_terrain_visuals() -> void:
		pass
	func mark_field_overlay_dirty() -> void:
		pass
	func mark_coal_pit_overlay_dirty() -> void:
		pass

class BuildingsStub extends Node3D:
	func update_all_buildings() -> void:
		pass

var _failures := 0
var simulation: SimulationNode
var _camera: Camera3D
var _fixture := ""
var _spec: Dictionary = {}

func _initialize() -> void:
	call_deferred("_run")

func _expect(condition: bool, message: String) -> void:
	if not condition:
		_failures += 1
		push_error(message)

func _run() -> void:
	_fixture = OS.get_environment("METRUM_PLACEMENT_DEMOLITION_FIXTURE")
	_expect(FileAccess.file_exists(_fixture), "METRUM_PLACEMENT_DEMOLITION_FIXTURE must name the street's save, got '%s'" % _fixture)
	var spec = JSON.parse_string(FileAccess.get_file_as_string(_fixture + ".json"))
	_expect(spec is Dictionary and spec.has("lot_pointer"), "the fixture's spec must load")
	if _failures > 0:
		_finish(null)
		return
	_spec = spec
	var scene := Node3D.new()
	root.add_child(scene)
	await _test_service(scene)
	_test_field(scene)
	_test_extraction_area(scene)
	_finish(scene)

func _finish(scene: Node) -> void:
	if scene != null:
		scene.free()
	if _failures == 0:
		print("placement_demolition_test: PASS")
	quit(0 if _failures == 0 else 1)

func _point(value: Array) -> Vector2:
	return Vector2(value[0], value[1])

# A fresh simulation on the fixture, under `scene` with the stubs and camera the tools look up.
func _load(scene: Node3D) -> void:
	for child in scene.get_children():
		child.free()
	simulation = SimulationNode.new()
	simulation.name = "SimulationNode"
	scene.add_child(simulation)
	var warnings := simulation.load_asset_packs(ProjectSettings.globalize_path("res://bootstrap/mods"), PackedStringArray(["kenney", "parking"]))
	_expect(warnings.is_empty(), "the packs load: " + warnings)
	_expect(simulation.load_game(_fixture), "the street fixture must load")
	simulation.set_simulation_speed(0.0)
	var terrain := TerrainStub.new()
	terrain.name = "Terrain"
	scene.add_child(terrain)
	var buildings := BuildingsStub.new()
	buildings.name = "Buildings"
	scene.add_child(buildings)
	_camera = Camera3D.new()
	_camera.projection = Camera3D.PROJECTION_ORTHOGONAL
	_camera.size = 200.0
	_camera.position = Vector3(0.0, 300.0, 0.0)
	_camera.rotation_degrees = Vector3(-90.0, 0.0, 0.0)
	scene.add_child(_camera)
	_camera.current = true

# Moves the top-down camera so the pointer's ray meets the ground at `target`: the ray is
# vertical, so the ground point moves with the camera.
func _point_at(target: Vector2) -> void:
	var mouse := root.get_mouse_position()
	var hit = simulation.intersect_world_surface(_camera.project_ray_origin(mouse), _camera.project_ray_normal(mouse), true)
	_expect(hit != null, "the pointer is over the ground")
	if hit == null:
		return
	_camera.position += Vector3(target.x - hit.x, 0.0, target.y - hit.z)

func _buildings() -> int:
	return int(simulation.get_road_benchmark_state()["buildings"])

func _left_click() -> InputEventMouseButton:
	var click := InputEventMouseButton.new()
	click.button_index = MOUSE_BUTTON_LEFT
	click.pressed = true
	click.position = root.get_mouse_position()
	return click

func _test_service(scene: Node3D) -> void:
	_load(scene)
	var tool: Node3D = ServiceTool.new()
	tool.name = "ServiceTool"
	scene.add_child(tool)
	tool.set_process(false)
	tool.active = true
	var asset := str(_spec["lot_asset"])
	tool.select_asset(asset)
	var pointer := _point(_spec["lot_pointer"])
	_point_at(pointer)
	# A paused frame may hold the core: hover until the hover answers.
	var deadline := Time.get_ticks_msec() + TIMEOUT_MS
	while not tool._preview_cache_valid and Time.get_ticks_msec() < deadline:
		tool._update_preview()
		if not tool._preview_cache_valid:
			await process_frame
	var count: int = tool._preview_cache_demolish_count
	_expect(tool._preview_cache_placeable, "the lot over the houses is valid: '%s'" % tool._preview_cache_error)
	_expect(count > 0 and count == int(_spec["lot_demolishes"]), "the hover names the houses the Rust hover took: %d" % count)
	_expect(tool._demolish_mesh.visible and tool._demolish_mesh.mesh != null, "the taken lots are outlined")
	_expect(tool._price_label.text.contains("demolishes %d building" % count), "the price label names the demolition: '%s'" % tool._price_label.text)
	var payload: Dictionary = simulation.get_service_building_placement_preview(asset, pointer.x, pointer.y)
	var lots: PackedVector3Array = payload.get("demolish_lots", PackedVector3Array())
	_expect(lots.size() == 4 * count, "four corners per taken lot: %d" % lots.size())
	for corner in lots:
		_expect(Vector2(corner.x - pointer.x, corner.z - pointer.y).length() < 40.0, "the outline lies at the lot: %s" % corner)

	# The simulation thread holds the core: the hover answers at once and the tool keeps its
	# last preview, the core still held after both.
	var label: String = tool._price_label.text
	# The pointer moves first: placing it waits for the core.
	_point_at(pointer + Vector2(5.0, 0.0))
	_expect(simulation.debug_hold_core(20000), "the simulation thread holds the core")
	var held: Dictionary = simulation.get_service_building_placement_preview(asset, pointer.x + 5.0, pointer.y)
	_expect(bool(held.get("pending", false)), "the hover answers pending while the core is held: %s" % held)
	tool._update_preview()
	_expect(tool._demolish_mesh.visible and tool._price_label.text == label, "the tool keeps its last preview while the core is held")
	_expect(bool(simulation.get_bulldoze_target_at(pointer.x, pointer.y).get("busy", false)), "the hovers left the core held")
	# A waiting call ends the hold.
	simulation.get_bulldoze_target_at(pointer.x, pointer.y, true)

	_point_at(pointer)
	var before := _buildings()
	tool._unhandled_input(_left_click())
	_expect(_buildings() == before - count + 1, "the click demolishes what the hover showed: %d -> %d" % [before, _buildings()])
	_expect(not tool._demolish_mesh.visible, "the outline goes with the click")
	tool.active = false

# The bounds of a placement result's lot corners: x0, x1, z0 (back), z1 (front).
func _lot_bounds(corners: PackedVector3Array) -> Array[float]:
	var bounds: Array[float] = [INF, -INF, INF, -INF]
	for corner in corners:
		bounds[0] = minf(bounds[0], corner.x)
		bounds[1] = maxf(bounds[1], corner.x)
		bounds[2] = minf(bounds[2], corner.z)
		bounds[3] = maxf(bounds[3], corner.z)
	return bounds

func _test_field(scene: Node3D) -> void:
	_load(scene)
	var front_z := float(_spec["front_z"])
	var farm_pointer := _point(_spec["farm_pointer"])
	var placed: Dictionary = simulation.place_industry_building(FARM, farm_pointer.x, farm_pointer.y)
	_expect(bool(placed.get("ok", false)), "the farm stands west of the houses: %s" % placed.get("error", ""))
	var lot := _lot_bounds(placed.get("plot_corners", PackedVector3Array()))
	# The first field, behind the farm's lot and the houses' lots, taking nothing.
	var back := lot[2] - 1.0
	var field := PackedVector2Array([Vector2(lot[0], back - 500.0), Vector2(99.0, back - 500.0), Vector2(99.0, back), Vector2(lot[0], back)])
	var committed: Dictionary = simulation.commit_field_polygon(int(placed["building_id"]), field)
	_expect(bool(committed.get("ok", false)), "the first field stands behind the houses: %s" % committed.get("error", ""))
	# A parking lot east of the houses, plotted land the field may not cover.
	_expect(simulation.place_service_building(str(_spec["lot_asset"]), 90.0, -30.0).is_empty(), "the parking lot stands east of the houses")
	var info: Dictionary = simulation.get_building_info_at(farm_pointer.x, farm_pointer.y)
	_expect(int(info.get("building_id", -1)) == int(placed["building_id"]), "the farm's info: %s" % info)
	var tool: Node3D = FieldTool.new()
	tool.name = "FieldTool"
	scene.add_child(tool)
	tool.set_process(false)
	tool.begin_edit(info)
	_expect(tool.active and tool._handles.size() == 4, "Edit Field opens the field")
	tool._dragged = 2

	# Over the parking lot: its refusal, no outline.
	_point_at(Vector2(99.0, front_z - 1.0))
	tool._update_drag(true)
	_expect(not tool._valid and tool._hint.text.contains("field overlaps a placed building's lot"), "the hint names the plotted lot: '%s'" % tool._hint.text)
	_expect(not tool._demolish_mesh.visible, "a refused drag outlines nothing")

	# Over the houses: their lots outlined and named.
	_point_at(Vector2(60.0, front_z - 1.0))
	tool._update_drag(true)
	var validation: Dictionary = simulation.validate_field_polygon(int(placed["building_id"]), tool._preview)
	var count := int(validation.get("demolish_count", 0))
	_expect(tool._valid and count > 0, "the drag over the houses is valid and takes them: %s" % validation)
	_expect(tool._hint.text.contains("demolishes %d building" % count), "the hint names the demolition: '%s'" % tool._hint.text)
	_expect(tool._demolish_mesh.visible and tool._demolish_mesh.mesh != null, "the taken lots are outlined")
	var before := _buildings()
	var release := InputEventMouseButton.new()
	release.button_index = MOUSE_BUTTON_LEFT
	release.pressed = false
	tool._input(release)
	_expect(tool._committed == tool._preview and tool._committed[2].distance_to(Vector2(60.0, front_z - 1.0)) < 0.01, "the release commits the drag: %s" % tool._hint.text)
	_expect(_buildings() == before - count, "the release demolishes what the drag showed: %d -> %d" % [before, _buildings()])
	_expect(not tool._demolish_mesh.visible, "the outline goes with the release")
	tool.cancel_edit()

func _test_extraction_area(scene: Node3D) -> void:
	_load(scene)
	var front_z := float(_spec["front_z"])
	var tool: Node3D = IndustryTool.new()
	tool.name = "IndustryTool"
	scene.add_child(tool)
	tool.active = true
	tool.set_process(false)
	tool.selected_asset_id = MINE
	var mine_pointer := _point(_spec["mine_pointer"])
	var placed: Dictionary = simulation.place_industry_building(MINE, mine_pointer.x, mine_pointer.y)
	_expect(bool(placed.get("ok", false)), "the mine stands east of the houses: %s" % placed.get("error", ""))
	var lot := _lot_bounds(placed.get("plot_corners", PackedVector3Array()))
	# As the tool's click leaves it: drawing the mine's area.
	tool._pending_building_id = int(placed["building_id"])
	tool._pending_area_kind = str(placed.get("area_kind", "extractor"))
	tool._mode = tool.Mode.DRAW_POLYGON
	var area: Array[Vector2] = [Vector2(15.0, front_z - 40.0), Vector2(lot[0] - 1.0, front_z - 40.0), Vector2(lot[0] - 1.0, front_z - 1.0), Vector2(15.0, front_z - 1.0)]
	tool._polygon_points.assign(area)
	# The pointer on the first point: the close click's preview, the polygon as drawn.
	_point_at(area[0])
	tool._update_polygon_mesh()
	var quote: Dictionary = simulation.get_industry_area_quote(int(placed["building_id"]), PackedVector2Array(area))
	var count := int(quote.get("demolish_count", 0))
	_expect(bool(quote.get("ok", false)) and count > 0, "the area over the houses is valid and takes them: %s" % quote)
	_expect(tool._price_label.visible and tool._price_label.text.contains("demolishes %d building" % count), "the quote label names the demolition: '%s'" % tool._price_label.text)
	_expect(tool._area_demolish_mesh.visible and tool._area_demolish_mesh.mesh != null, "the taken lots are outlined")
	var before := _buildings()
	tool._commit_polygon()
	_expect(tool._mode == tool.Mode.PLACE_BUILDING, "the close commits the area")
	_expect(_buildings() == before - count, "the close demolishes what the quote showed: %d -> %d" % [before, _buildings()])
	_expect(not tool._area_demolish_mesh.visible, "the outline goes with the commit")
	tool.active = false
