# SPDX-License-Identifier: GPL-3.0-or-later

## Real vehicle mesh contacts through native grounding, native smoothing and MultiMesh upload:
## the bundled pack's models (`RENDER-65`), grounded by the support profiles Rust builds from their
## wheel facts, checked against the tyre vertices of their nearest tier.
extends SceneTree

const VehicleResources := preload("res://scripts/renderers/vehicle_resources.gd")
const AgentRenderer := preload("res://scripts/renderers/agents.gd")
var _failures := 0
var _poses := 0
var _minimum_clearance := INF
var _tilted_poses := 0
var _largest_center_lift := 0.0

# Only the snapshot source is synthetic. Model loading, native smoothing, native surface queries,
# native support correction and the uploaded vehicle mesh all use their production paths.
# The busy-support fallback is covered by the native `car_visual` tests.
class RenderSource extends RefCounted:
	var simulation: SimulationNode
	var cars: Dictionary = {}
	var drawn: Dictionary = {}
	# The agents' frame of `cars`, smoothed at once by the synchronous path (the game's frame
	# job runs the same frame a frame later, `RENDER-47`).
	func advance_agent_frame(delta: float, _full: bool) -> Dictionary:
		drawn = simulation.smooth_car_render_data(cars, delta)
		return {"installed": true, "cars": drawn, "walkers": {}, "changed_cars": simulation.get_changed_car_buckets(),
			"changed_walkers": PackedInt64Array(), "vehicle_revision": revision}
	var revision := 0
	func get_vehicle_catalogue() -> Dictionary:
		var catalogue: Dictionary = simulation.get_vehicle_catalogue()
		revision = catalogue["revision"]
		return catalogue

func _initialize() -> void:
	call_deferred("_run")

func _expect(condition: bool, message: String) -> void:
	if not condition:
		_failures += 1
		push_error(message)

func _unpack(buffer: PackedFloat32Array) -> Transform3D:
	var b := buffer
	return Transform3D(Basis(Vector3(b[0], b[4], b[8]), Vector3(b[1], b[5], b[9]), Vector3(b[2], b[6], b[10])), Vector3(b[3], b[7], b[11]))

func _pack(pose: Transform3D) -> PackedFloat32Array:
	var b := pose.basis
	var p := pose.origin
	return PackedFloat32Array([b.x.x, b.y.x, b.z.x, p.x, b.x.y, b.y.y, b.z.y, p.y, b.x.z, b.y.z, b.z.z, p.z])

func _run() -> void:
	var simulation := SimulationNode.new()
	root.add_child(simulation)
	var source := RenderSource.new()
	source.simulation = simulation
	var renderer := AgentRenderer.new()
	renderer.simulation_node = source
	var warnings := simulation.load_asset_packs(ProjectSettings.globalize_path("res://bootstrap/mods"), PackedStringArray(["vehicles"]))
	_expect(warnings.is_empty(), "the vehicle pack loads: " + warnings)
	var meshes := VehicleResources.new()
	meshes.lamps_enabled = false
	meshes.refresh(source)
	# One look a model: its index is the model's ground profile, its tier 0 the drawn bucket.
	var models: Array[String] = []
	var buckets: Array[int] = []
	var contacts_by_model: Array = []
	for look in meshes.looks.size():
		var model: int = meshes.looks[look]["model"]
		if model < models.size():
			continue
		models.append(meshes.looks[look]["id"])
		buckets.append(look * VehicleResources.TIERS)
		var mesh: Mesh = meshes.mesh_for_bucket(look * VehicleResources.TIERS)
		var contacts: Array[Vector3] = []
		for point: Vector3 in mesh.surface_get_arrays(0)[Mesh.ARRAY_VERTEX]:
			# The tyres' lowest vertices: the contract puts them on y = 0 exactly.
			if point.y <= 0.001 and not contacts.has(point):
				contacts.append(point)
		_expect(not contacts.is_empty(), "%s must have tyre contacts" % models[model])
		contacts_by_model.append(contacts)
	_expect(models.size() == 2, "the estate and the truck: %s" % [models])

	for side in [-1.0, 1.0]:
		for rise in [-1.0, 1.0]:
			_expect(simulation.create_blank_world(128.0, 128.0, 2.0, 0.0), "support test world must load")
			simulation.set_simulation_speed(0.0)
			# A clamped, oblique two-metre grade has both pitch and roll, and two abrupt
			# grade breaks. The fine source grid keeps this independent of road placement.
			simulation.slope_terrain(Vector2.ZERO, 48.0, Vector2.ZERO, 0.0, Vector2(0.4, side * 2.0), rise, 20000.0)
			for kind in range(models.size()):
				for direction in [-1.0, 1.0]:
					# A frame without cars forgets every drawn pose.
					simulation.smooth_car_render_data({}, 0.0)
					var target := PackedFloat32Array()
					var previous_visual_z := NAN
					for frame in range(121):
						if frame % 5 == 0:
							var distance := -5.0 + float(frame) / 10.0
							if direction < 0.0:
								distance = 2.0 - distance
							var heading := Vector3(0.0, 0.0, -side * direction)
							var pose := Transform3D(Basis(Vector3.UP.cross(heading), Vector3.UP, heading), Vector3(0.0, -100.0, side * distance))
							# The direct target solve is exactly on the full-detail boundary.
							simulation.set_camera_aabb(
								-1000.0, 1000.0, -1000.0, 1000.0,
								Vector3(SimulationNode.get_vehicle_ground_full_detail_m(), 0.0, pose.origin.z)
							)
							var request := _pack(pose)
							target = simulation.ground_car_transforms(kind, request, PackedByteArray([1]))
							var deadline := Time.get_ticks_msec() + 5000
							while target.is_empty() and Time.get_ticks_msec() < deadline:
								await process_frame
								target = simulation.ground_car_transforms(kind, request, PackedByteArray([1]))
							_expect(not target.is_empty(), "native support batch must become ready")
							if target.is_empty():
								quit(1)
								return
						# Keep the lagging interpolated pose, rather than only its snapshot target,
						# exactly on the boundary tested by the per-frame grounding path.
						var predicted_visual_z: float = target[11] if is_nan(previous_visual_z) else lerpf(previous_visual_z, target[11], 0.4)
						simulation.set_camera_aabb(
							-1000.0, 1000.0, -1000.0, 1000.0,
							Vector3(SimulationNode.get_vehicle_ground_full_detail_m(), 0.0, predicted_visual_z)
						)
						source.cars = {buckets[kind]: {"transforms": target, "ids": PackedInt64Array([1]), "ground_flags": PackedByteArray([1])}}
						renderer.update_swarm(1.0 / 60.0)
						var visual := _unpack(source.drawn[buckets[kind]])
						previous_visual_z = visual.origin.z
						if DisplayServer.get_name() != "headless":
							var uploaded: Transform3D = renderer.car_mmis[buckets[kind]].multimesh.get_instance_transform(0)
							_expect(uploaded.is_equal_approx(visual), "GPU buffer layout must preserve corrected pose")
							visual = uploaded
						if visual.basis.y.distance_to(Vector3.UP) > 0.01:
							_tilted_poses += 1
						var center_height := simulation.get_world_surface_height(Vector2(visual.origin.x, visual.origin.z))
						_largest_center_lift = maxf(_largest_center_lift, visual.origin.y - center_height)
						for contact in contacts_by_model[kind]:
							var world: Vector3 = visual * contact
							var height := simulation.get_world_surface_height(Vector2(world.x, world.z))
							var clearance: float = world.y - height
							_minimum_clearance = minf(_minimum_clearance, clearance)
							# The solve lifts its support points 2 cm; a tyre vertex between them, over a
							# grade break, may sit up to a millimetre lower (`RENDER-65`), never buried.
							_expect(clearance >= 0.019, "buried contact model=%s side=%s direction=%s frame=%d clearance=%f" % [models[kind], side, direction, frame, clearance])
						_poses += 1
						if _failures > 0:
							renderer.free()
							simulation.free()
							quit(1)
							return

	# Leaving/entering an access phase must not blend across different height owners, even
	# within the snap distance. Support correction may change height and tilt, never XZ.
	for previous in [false, true]:
		simulation.smooth_car_render_data({}, 0.0)
		var start := _pack(Transform3D(Basis.IDENTITY, Vector3(0, -50, 0)))
		simulation.smooth_car_render_data({0: {"transforms": start, "ids": PackedInt64Array([1]), "ground_flags": PackedByteArray([1 if previous else 0])}}, 0.0)
		var target := _pack(Transform3D(Basis.IDENTITY, Vector3(0, 2, 2)))
		simulation.set_camera_aabb(
			-1000.0, 1000.0, -1000.0, 1000.0,
			Vector3(SimulationNode.get_vehicle_ground_full_detail_m(), 0.0, 2.0)
		)
		var result: Dictionary = simulation.smooth_car_render_data({0: {"transforms": target, "ids": PackedInt64Array([1]), "ground_flags": PackedByteArray([0 if previous else 1])}}, 0.4 / 24.0)
		var drawn: PackedFloat32Array = result[0]
		_expect(drawn[3] == target[3] and drawn[11] == target[11], "height owner handoff must use target pose")
		if previous:
			_expect(drawn.slice(0, 12) == target and drawn.size() == 16, "lane target after handoff must be shown unchanged")
	_expect(_tilted_poses > 100, "fixture must exercise pitch/roll, not flat poses")
	_expect(_largest_center_lift > 0.1, "fixture must require footprint support beyond a centre-height solve")
	if "--benchmark-vehicle-support" in OS.get_cmdline_user_args():
		_benchmark(renderer, source)
	renderer.free()
	simulation.free()
	print("vehicle_ground_support_test: full_detail_m=%.1f poses=%d tilted=%d minimum_contact_clearance_m=%.6f largest_center_lift_m=%.4f %s" % [SimulationNode.get_vehicle_ground_full_detail_m(), _poses, _tilted_poses, _minimum_clearance, _largest_center_lift, "PASS" if _failures == 0 else "FAIL"])
	quit(_failures)

func _benchmark(renderer: Node3D, source: RenderSource) -> void:
	# Time the production update/bridge/upload path separately from correctness oracles and setup.
	# Lane cars are smoothing only; off-lane cars add the native support solve. This synthetic
	# surface has no road or site owners; the Rust Kuopio batch bench covers those.
	for grounded in [0, 1]:
		for count in [100, 1000, 5000, 20000]:
			var packed := PackedFloat32Array()
			packed.resize(count * 12)
			var ids := PackedInt64Array()
			ids.resize(count)
			var flags := PackedByteArray()
			flags.resize(count)
			flags.fill(grounded)
			for i in range(count):
				ids[i] = i + 1
				var pose := _pack(Transform3D(Basis.IDENTITY, Vector3(0, 1, -4.0 + float(i % 24) * 0.5)))
				for j in range(12):
					packed[i * 12 + j] = pose[j]
			source.cars = {0: {"transforms": packed, "ids": ids, "ground_flags": flags}}
			for i in range(10):
				renderer.update_swarm(1.0 / 60.0)
			var samples: Array[float] = []
			for i in range(100):
				# Cars advance so every frame interpolates instead of settling on the target.
				for c in range(0, count * 12, 12):
					packed[c + 11] += 0.01
				source.cars[0]["transforms"] = packed
				var start := Time.get_ticks_usec()
				renderer.update_swarm(1.0 / 60.0)
				samples.append(float(Time.get_ticks_usec() - start) / 1000.0)
			samples.sort()
			print("vehicle_render_update: cars=%d off_lane=%d p50_ms=%.4f p90_ms=%.4f" % [count, grounded, samples[samples.size() / 2], samples[int(samples.size() * 0.9)]])
