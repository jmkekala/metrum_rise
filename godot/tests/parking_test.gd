# SPDX-License-Identifier: GPL-2.0-only

## PARK-01 bridge regression: shared mesh equivalence, lot supply, immutable chunks and towing;
## `ROAD-109`: a door keeps its sidewalk and a side without one takes no building, with the reason.
## METRUM_PARKING_AGENT_CONTROL optionally points to the pre-extraction agents.gd source.
extends SceneTree

const AgentRenderer := preload("res://scripts/renderers/agents.gd")
const VehicleResources := preload("res://scripts/renderers/vehicle_resources.gd")
var _failures := 0

func _initialize() -> void:
	call_deferred("_run")

func _expect(condition: bool, message: String) -> void:
	if not condition:
		_failures += 1
		push_error(message)

func _resources() -> void:
	var parked := VehicleResources.new()
	parked.lamps_enabled = false
	var control_path := OS.get_environment("METRUM_PARKING_AGENT_CONTROL")
	if control_path.is_empty():
		return
	var script := GDScript.new()
	script.source_code = FileAccess.get_file_as_string(control_path)
	_expect(script.reload() == OK, "pre-extraction control script compiles")
	var control: Node = script.new()
	var current: Node = AgentRenderer.new()
	var hashes := {}
	for model: int in VehicleResources.MODELS:
		var path: String = VehicleResources.MODELS[model]
		for colour in range(5):
			var shift := 0.0 if model == 4 else colour * 0.1
			var old: Mesh = control._build_vehicle_mesh_from_glb(control._load_vehicle_glb_source(path), path, shift, 0.0, Vector3(0, PI, 0))
			var new: Mesh = current._build_vehicle_mesh_from_glb(current._load_vehicle_glb_source(path), path, shift, 0.0, Vector3(0, PI, 0))
			_expect(old != null and new != null, "vehicle source loads")
			if old == null or new == null:
				continue
			_expect(old.get_surface_count() == new.get_surface_count(), "surface count preserved")
			var bytes := PackedByteArray()
			for surface in range(old.get_surface_count()):
				var before := var_to_bytes(old.surface_get_arrays(surface))
				var after := var_to_bytes(new.surface_get_arrays(surface))
				_expect(before == after, "vehicle arrays bit-identical after extraction")
				bytes.append_array(before)
			hashes[model * 10 + colour] = bytes.hex_encode().sha256_text()
			var unlit: Mesh = parked.mesh_for_bucket(model * 10 + colour)
			_expect(unlit.get_surface_count() + 2 == new.get_surface_count(), "parked mesh omits lamp and beam surfaces")
	var output := OS.get_environment("METRUM_PARKING_RESOURCE_HASHES")
	if not output.is_empty():
		var file := FileAccess.open(output, FileAccess.WRITE)
		file.store_string(JSON.stringify(hashes, "  "))
	control.free()
	current.free()

# The lot's placement preview at (0, z). A hover answers `pending` while the simulation thread holds
# the core (`CODE-25`, since `ZONE-11` round RZ11b), as it may just after an edit: ask again.
func _placement_preview(sim: SimulationNode, z: float) -> Dictionary:
	var preview: Dictionary = {"pending": true}
	var deadline := Time.get_ticks_msec() + 20000
	while bool(preview.get("pending", false)) and Time.get_ticks_msec() < deadline:
		preview = sim.get_service_building_placement_preview("parking:building.parking.small_lot", 0.0, z)
		if bool(preview.get("pending", false)):
			await process_frame
	return preview

func _run() -> void:
	_resources()
	var sim := SimulationNode.new()
	root.add_child(sim)
	_expect(sim.create_blank_world(256.0, 256.0, 2.0, 128.0, 0.0), "isolated world")
	sim.set_simulation_speed(0.0)
	var generation := sim.get_network_render_generation()
	sim.add_road_with_snap(PackedVector3Array([Vector3(-90, 0, 0), Vector3(90, 0, 0)]), 1, 1, true)
	var deadline := Time.get_ticks_msec() + 20000
	while sim.get_network_render_generation() <= generation and Time.get_ticks_msec() < deadline:
		await process_frame
	_expect(sim.get_parking_info()["places"] == 0, "roads alone create no parking")
	var warnings := sim.load_asset_packs(ProjectSettings.globalize_path("res://bootstrap/mods"), PackedStringArray(["parking"]))
	_expect(warnings.is_empty(), "lot pack loads: " + warnings)
	var preview := await _placement_preview(sim, -30.0)
	_expect(preview.get("valid", false), "lot preview is valid: %s" % preview)
	_expect(sim.place_service_building("parking:building.parking.small_lot", 0.0, -30.0).is_empty(), "standalone lot places")
	var filled := sim.debug_fill_parking()
	_expect(filled > 0, "explicit debug fleet fills supply")
	# Chunks reach Godot only through a snapshot; a full request is answered by a later one.
	var full: Dictionary = {}
	deadline = Time.get_ticks_msec() + 20000
	while not full.get("full", false) and Time.get_ticks_msec() < deadline:
		full = sim.get_parked_car_chunks(true)
		await process_frame
	_expect(full.get("full", false), "a full request is served without the core lock")
	var published := 0
	for update: Dictionary in full.get("chunks", []):
		for ids: PackedInt64Array in update["identities"].values():
			published += ids.size()
	_expect(published == filled, "the snapshot holds every parked car")
	var frame: Dictionary = sim.get_vehicle_frame(false)
	_expect(not frame.has("busy"), "the vehicle frame has no busy state")
	_expect(frame.has("cars") and frame.has("parked"), "cars and unseen chunks come together")
	sim.acknowledge_parked_cars(full["epoch"], full["serial"])
	sim.acknowledge_parked_cars(full["epoch"] + 1, 1 << 40)
	frame = sim.get_vehicle_frame(false)
	var resent: Dictionary = frame.get("parked", {})
	_expect(frame.has("cars") and not frame.has("parked"), "nothing parked when nothing changed: acknowledged serial %d, sent serial %s full %s with %d chunks" % [full["serial"], resent.get("serial"), resent.get("full"), resent.get("chunks", []).size()])
	var unchanged: Dictionary = sim.get_parked_car_chunks(false)
	_expect(unchanged.get("chunks", []).is_empty(), "unchanged chunks send nothing")
	# Placement snaps to its frontage; target the published centre, and retry a busy bridge.
	var target: Dictionary = {}
	deadline = Time.get_ticks_msec() + 20000
	while not target.get("queued", false) and Time.get_ticks_msec() < deadline:
		target = sim.bulldoze_at(preview["center_x"], preview["center_z"])
		await process_frame
	_expect(target.get("queued", false), "lot demolition queued: " + str(target))
	deadline = Time.get_ticks_msec() + 20000
	while sim.get_parking_info()["places"] > 0 and Time.get_ticks_msec() < deadline:
		await process_frame
	var info: Dictionary = sim.get_parking_info()
	_expect(info["vehicles"] == filled and info["stored"] == filled, "supply removal stores every car exactly once")
	_expect(sim.undo_action(), "lot undo queued")
	deadline = Time.get_ticks_msec() + 20000
	while sim.get_parking_info()["places"] == 0 and Time.get_ticks_msec() < deadline:
		await process_frame
	info = sim.get_parking_info()
	_expect(info["vehicles"] == filled and info["stored"] == filled, "restored supply waits for driven arrivals; debug fleet has no external home")
	# `ROAD-109`: the lot's door keeps its sidewalk, and nothing is placed where there is none.
	var road: int = sim.get_hovered_edge(0.0, 0.0)
	var under_door: Dictionary = sim.preview_edge_cross_section(road, 1, 1, 0, 0.0, 0, 50, 2, true, true)
	_expect(under_door.get("invalid_reason", "") == "sidewalk_under_door", "the door keeps its sidewalk: %s" % under_door)
	_expect(sim.set_edge_cross_section(road, 1, 1, 0, 0.0, 0, 50, 1, true, true), "the far sidewalk comes off")
	var shoulder := await _placement_preview(sim, 30.0)
	_expect(not shoulder.get("valid", true) and String(shoulder.get("error", "")).contains("sidewalk"), "the preview says why: %s" % shoulder)
	_expect(sim.place_service_building("parking:building.parking.small_lot", 0.0, 30.0).contains("sidewalk"), "the tool refuses the site")
	sim.queue_free()
	await process_frame
	quit(0 if _failures == 0 else 1)
