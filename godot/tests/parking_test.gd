# SPDX-License-Identifier: GPL-2.0-only

## PARK-01 bridge regression: shared mesh equivalence, lot supply, immutable chunks and towing;
## `ROAD-109`: a door keeps its sidewalk and a side without one takes no building, with the reason.
## `RENDER-32-P`: the parked renderer draws every car once through the stream node's pooled slots,
## makes no node per chunk, and frees every slot when it leaves. `RENDER-46`: a car body is one surface.
## METRUM_PARKING_AGENT_CONTROL optionally points to the pre-extraction agents.gd source.
extends SceneTree

const AgentRenderer := preload("res://scripts/renderers/agents.gd")
const VehicleResources := preload("res://scripts/renderers/vehicle_resources.gd")
const ParkedRenderer := preload("res://scripts/renderers/parked_cars.gd")
const VehicleLamps := preload("res://scripts/renderers/vehicle_lamps.gd")
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

# `RENDER-46`: a car's body primitives share one material, so they are one surface; its triangles,
# vertices, normals and UVs are the glTF primitives' own in order, and the lamps keep theirs.
func _merged_bodies() -> void:
	var lit := VehicleResources.new()
	var unlit := VehicleResources.new()
	unlit.lamps_enabled = false
	for model: int in VehicleResources.MODELS:
		var path: String = VehicleResources.MODELS[model]
		var source := lit._load_vehicle_glb_source(path)
		var doc: Dictionary = source["json"]
		# The builder's frame for base rotation (0, PI, 0) and no target height.
		var frame := Transform3D().rotated(Vector3.RIGHT, 0.0).rotated(Vector3.UP, PI).rotated(Vector3.FORWARD, 0.0)
		var primitives: Array = []
		var bounds := {"has": false, "aabb": AABB()}
		for node in doc["scenes"][int(doc.get("scene", 0))]["nodes"]:
			lit._collect_glb_node_surfaces(doc, source["bin"], int(node), frame, 0.0, primitives, bounds)
		var y_offset: float = -(bounds["aabb"] as AABB).position.y
		# The source: a surface a primitive, as the builder made them before the merge. Compared
		# as Godot stores them (normals are kept octahedral-encoded, so read back, not as given).
		var source_mesh := ArrayMesh.new()
		for primitive: Dictionary in primitives:
			var surface_verts := PackedVector3Array()
			for v: Vector3 in primitive["vertices"]:
				v.y += y_offset
				surface_verts.append(v)
			var arrays := []
			arrays.resize(Mesh.ARRAY_MAX)
			arrays[Mesh.ARRAY_VERTEX] = surface_verts
			arrays[Mesh.ARRAY_NORMAL] = primitive["normals"]
			arrays[Mesh.ARRAY_TEX_UV] = primitive["uvs"]
			arrays[Mesh.ARRAY_INDEX] = primitive["indices"]
			source_mesh.add_surface_from_arrays(Mesh.PRIMITIVE_TRIANGLES, arrays)
		var verts := PackedVector3Array()
		var normals := PackedVector3Array()
		var uvs := PackedVector2Array()
		var indices := PackedInt32Array()
		for surface in source_mesh.get_surface_count():
			var stored := source_mesh.surface_get_arrays(surface)
			for index: int in stored[Mesh.ARRAY_INDEX]:
				indices.append(verts.size() + index)
			verts.append_array(stored[Mesh.ARRAY_VERTEX])
			normals.append_array(stored[Mesh.ARRAY_NORMAL])
			uvs.append_array(stored[Mesh.ARRAY_TEX_UV])
		_expect(primitives.size() > 1, "%s has several body primitives" % path)
		var meshes: Array[ArrayMesh] = [lit.mesh_for_bucket(model * 10), unlit.mesh_for_bucket(model * 10)]
		for mesh in meshes:
			var arrays := mesh.surface_get_arrays(0)
			_expect(mesh.get_meta(VehicleLamps.BODY_SURFACES_META, 1) == 1, "one body surface")
			_expect(arrays[Mesh.ARRAY_INDEX] == indices, "%s keeps its triangles and their winding" % path)
			_expect(arrays[Mesh.ARRAY_VERTEX] == verts and arrays[Mesh.ARRAY_NORMAL] == normals
				and arrays[Mesh.ARRAY_TEX_UV] == uvs, "%s keeps its vertex data" % path)
			_expect(mesh.get_meta(VehicleLamps.BODY_AABB_META, mesh.get_aabb()) == source_mesh.get_aabb(), "%s keeps its bounds" % path)
		# By material: the body's one, then the lamps' glow and beam on the lit car only.
		_expect(meshes[1].get_surface_count() == 1, "the parked car is its body")
		_expect(meshes[0].get_surface_count() == 3, "the moving car adds its two lamp surfaces")
		var materials := {}
		for surface in meshes[0].get_surface_count():
			materials[meshes[0].surface_get_material(surface)] = true
		_expect(materials.size() == 3, "one surface per material")

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
		for bucket: int in update["buckets"]:
			expected["%s|%d" % [update["chunk"], bucket]] = update["buckets"][bucket]
	# The headless renderer keeps no buffer to read back: the counts here, the rows bit for bit in
	# the layer's recorder tests and the stream benchmark's digests.
	var drawn := 0
	var rows: Array[Dictionary] = parked.drawn_rows()
	for item: Dictionary in rows:
		var key := "%s|%d" % [item.chunk, item.bucket]
		_expect((expected.get(key, PackedFloat32Array()) as PackedFloat32Array).size() == int(item.count) * 12,
			"parked bucket %s draws its every car" % key)
		drawn += int(item.count)
	_expect(rows.size() == expected.size(), "every parked bucket drawn once")
	_expect(drawn == filled, "every parked car drawn once: %d of %d" % [drawn, filled])
	_expect(parked.get_child_count() == 0, "no node per parked chunk")
	parked.free()
	var slots: Dictionary = stream.get_parked_slots()
	_expect(int(slots.slots_live) == 0 and int(slots.slots_spare) == 0 and int(slots.slots_created) == int(slots.slots_freed),
		"the renderer leaving frees every parked slot: %s" % slots)
	stream.free()

func _run() -> void:
	_resources()
	_merged_bodies()
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
