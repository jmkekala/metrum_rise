# SPDX-License-Identifier: GPL-3.0-or-later

## PARK-01 bridge regression: the vehicle meshes, lot supply, immutable chunks and towing;
## `ROAD-109`: a door keeps its sidewalk and a side without one takes no building, with the reason.
## `RENDER-32-P`: the parked renderer draws every car once through the stream node's pooled slots,
## makes no node per chunk, and frees every slot when it leaves. `RENDER-65`: every tier of every
## bundled vehicle look is one body surface whose ids arrive as exact bytes, turned to drive along
## -Z; parked meshes have no lamp surfaces; a parked look draws its four tiers in contiguous bands.
## `RENDER-48`: parked cars fade with the moving cars' material and band, and the farthest tier's
## visibility range reaches past the draw distance.
extends SceneTree

const VehicleResources := preload("res://scripts/renderers/vehicle_resources.gd")
const ParkedRenderer := preload("res://scripts/renderers/parked_cars.gd")
var _failures := 0

func _initialize() -> void:
	call_deferred("_run")

func _expect(condition: bool, message: String) -> void:
	if not condition:
		_failures += 1
		push_error(message)

# `RENDER-65`: each look's tiers, lit (moving) and unlit (parked), from the bundled pack.
func _vehicle_meshes(sim: SimulationNode) -> void:
	var lit := VehicleResources.new()
	var unlit := VehicleResources.new()
	unlit.lamps_enabled = false
	lit.refresh(sim)
	unlit.refresh(sim)
	_expect(lit.looks.size() == 3, "the bundled pack gives three looks: %d" % lit.looks.size())
	for look in lit.looks.size():
		for tier in VehicleResources.TIERS:
			var key := look * VehicleResources.TIERS + tier
			var moving := lit.mesh_for_bucket(key)
			var parked := unlit.mesh_for_bucket(key)
			_expect(parked.get_surface_count() == 1, "look %d tier %d: the parked car is its body" % [look, tier])
			_expect(moving.get_surface_count() == 3, "look %d tier %d: the moving car adds its two lamp surfaces" % [look, tier])
			_expect((parked.surface_get_material(0) as ShaderMaterial).shader == VehicleResources.BODY_SHADER,
				"look %d tier %d draws through the body shader" % [look, tier])
			var arrays := parked.surface_get_arrays(0)
			var colours: PackedColorArray = arrays[Mesh.ARRAY_COLOR]
			var vertices: PackedVector3Array = arrays[Mesh.ARRAY_VERTEX]
			var parts := {}
			var exact := true
			var nose := 0.0
			for i in colours.size():
				var c := colours[i] * 255.0
				exact = exact and absf(c.r - roundf(c.r)) < 0.01 and absf(c.g - roundf(c.g)) < 0.01 and absf(c.b - roundf(c.b)) < 0.01
				parts[int(roundf(c.r))] = true
				var surface := int(roundf(c.g))
				exact = exact and (surface <= 15 or (surface >= 32 and surface <= 47)) and int(roundf(c.b)) in [0, 255]
				if surface == 7:
					nose += vertices[i].z
			_expect(exact, "look %d tier %d: ids arrive as exact bytes" % [look, tier])
			var expected_parts := [0, 1, 2, 3, 4] if tier < 2 else [0]
			_expect(parts.keys().all(func(part): return part in expected_parts) and parts.size() == expected_parts.size(),
				"look %d tier %d: wheel parts %s" % [look, tier, parts.keys()])
			if tier == 0:
				_expect(nose < 0.0, "look %d: the low beams sit at the nose, -Z" % look)

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

# The full payload through the renderer as the game installs it (`RENDER-32-P`).
func _parked_renderer(sim: SimulationNode, full: Dictionary, filled: int) -> void:
	sim.name = "SimulationNode"
	var stream := StreamNode.new()
	stream.name = "StreamNode"
	root.add_child(stream)
	var parked: Node3D = ParkedRenderer.new()
	parked.name = "ParkedCars"
	root.add_child(parked)
	parked.apply_publication(full)
	var expected := {}
	for update: Dictionary in full["chunks"]:
		for look: int in update["buckets"]:
			expected["%s|%d" % [update["chunk"], look]] = update["buckets"][look]
	# The headless renderer keeps no buffer to read back: the counts here, the rows bit for bit in
	# the layer's recorder tests and the stream benchmark's digests. Each look draws in its four
	# tier buckets (`RENDER-65`).
	var drawn := 0
	var rows: Array[Dictionary] = parked.drawn_rows()
	for item: Dictionary in rows:
		var key := "%s|%d" % [item.chunk, int(item.bucket) / VehicleResources.TIERS]
		_expect((expected.get(key, PackedFloat32Array()) as PackedFloat32Array).size() == int(item.count) * 16,
			"parked bucket %s draws its every car" % key)
		drawn += int(item.count)
	_expect(rows.size() == expected.size() * VehicleResources.TIERS, "every parked look drawn once a tier")
	_expect(drawn == filled * VehicleResources.TIERS, "every parked car drawn once a tier: %d of %d" % [drawn, filled])
	_expect(parked.get_child_count() == 0, "no node per parked chunk")
	_parked_draw_distance(parked, stream)
	parked.free()
	var slots: Dictionary = stream.get_parked_slots()
	_expect(int(slots.slots_live) == 0 and int(slots.slots_spare) == 0 and int(slots.slots_created) == int(slots.slots_freed),
		"the renderer leaving frees every parked slot: %s" % slots)
	stream.free()

# `RENDER-48`: a parked car is drawn with the moving cars' fade at the same distances (the same
# shader, the band from Rust), and a bucket's batch is cut only beyond them (its range in Rust's
# layer tests; here it reaches past the draw distance for every drawn bucket).
func _parked_draw_distance(parked: Node3D, stream: Node) -> void:
	var draw: PackedFloat32Array = SimulationNode.get_agent_draw_distances()
	var band := Vector2(draw[2], draw[3])
	var moving := VehicleResources.new()
	moving.draw_fade_m = band
	moving.refresh(parked.simulation_node)
	for look: int in parked._meshes:
		var bucket := look * VehicleResources.TIERS
		var mesh: Mesh = parked._meshes[look][0]
		var body := mesh.surface_get_material(0) as ShaderMaterial
		var reference := moving.mesh_for_bucket(bucket).surface_get_material(0) as ShaderMaterial
		_expect(body != null and reference != null, "car bodies draw through the body shader")
		if body == null or reference == null:
			continue
		_expect(body.shader == reference.shader, "parked bucket %d shares the moving cars' shader" % bucket)
		_expect(body.get_shader_parameter("draw_fade_m") == band,
			"parked bucket %d fades over the cars' band %s" % [bucket, band])
	var census: Dictionary = stream.get_draw_census("parked")
	var ranges: PackedVector2Array = census.get("range", PackedVector2Array())
	_expect(ranges.size() == (census.get("count", PackedInt32Array()) as PackedInt32Array).size(), "a range per slot")
	var nearest := INF
	var farthest := 0.0
	for i in range(ranges.size()):
		if (census["visible"] as PackedByteArray)[i] == 1:
			_expect(ranges[i].y > ranges[i].x, "a drawn tier's band is not empty: %s" % ranges[i])
			nearest = minf(nearest, ranges[i].x)
			farthest = maxf(farthest, ranges[i].y)
	_expect(nearest == 0.0 and farthest >= band.y,
		"the parked tiers cover every distance to past the draw distance: %f to %f" % [nearest, farthest])

func _run() -> void:
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
	var warnings := sim.load_asset_packs(ProjectSettings.globalize_path("res://bootstrap/mods"), PackedStringArray(["parking", "vehicles"]))
	_expect(warnings.is_empty(), "lot and vehicle packs load: " + warnings)
	_vehicle_meshes(sim)
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
	_parked_renderer(sim, full, filled)
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
