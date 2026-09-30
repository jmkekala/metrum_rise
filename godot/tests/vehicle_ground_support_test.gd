# SPDX-License-Identifier: GPL-2.0-only

## Real vehicle mesh contacts through native grounding, native smoothing and MultiMesh upload.
extends SceneTree

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
	func get_agent_transforms() -> Dictionary:
		return {}
	func get_car_render_data() -> Dictionary:
		return cars
	func smooth_car_render_data(data: Dictionary, delta: float) -> Dictionary:
		drawn = simulation.smooth_car_render_data(data, delta)
		return drawn
	func set_vehicle_ground_support(kind: int, bounds: AABB, contacts: PackedVector3Array) -> bool:
		return simulation.set_vehicle_ground_support(kind, bounds, contacts)

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
	var models := ["civilian/sedan", "civilian/sedan-sports", "civilian/suv", "civilian/suv-luxury", "freight/delivery"]
	var contacts_by_model: Array = []
	for kind in range(models.size()):
		var path: String = "res://assets/models/vehicles/%s.glb" % models[kind]
		var model: Dictionary = renderer._load_vehicle_glb_source(path)
		var mesh: Mesh = renderer._build_vehicle_mesh_from_glb(model, path, 0.0, 0.0, Vector3(0, PI, 0))
		_expect(mesh != null, "vehicle source mesh must load: " + path)
		if mesh == null:
			quit(1)
			return
		_expect(renderer._add_car_multimesh(kind, 0, mesh), "native support registration must accept " + path)
		var contacts: Array[Vector3] = []
		for surface in range(mesh.get_surface_count()):
			var vertices: PackedVector3Array = mesh.surface_get_arrays(surface)[Mesh.ARRAY_VERTEX]
			for point in vertices:
				if point.y <= 0.0001:
					contacts.append(point)
		_expect(not contacts.is_empty(), "vehicle must have bottom contacts")
		contacts_by_model.append(contacts)

	for side in [-1.0, 1.0]:
		for rise in [-1.0, 1.0]:
			_expect(simulation.create_blank_world(128.0, 128.0, 1.0, 64.0, 0.0), "support test world must load")
			simulation.set_simulation_speed(0.0)
			# A clamped, oblique two-metre grade has both pitch and roll, and two abrupt
			# grade breaks. The fine source grid keeps this independent of road placement.
			simulation.slope_terrain(Vector2.ZERO, 48.0, Vector2.ZERO, 0.0, Vector2(0.4, side * 2.0), rise, 1000.0)
			for kind in range(models.size()):
				for direction in [-1.0, 1.0]:
					# A frame without cars forgets every drawn pose.
					simulation.smooth_car_render_data({}, 0.0)
					var target := PackedFloat32Array()
					for frame in range(121):
						if frame % 5 == 0:
							var distance := -5.0 + float(frame) / 10.0
							if direction < 0.0:
								distance = 2.0 - distance
							var heading := Vector3(0.0, 0.0, -side * direction)
							var pose := Transform3D(Basis(Vector3.UP.cross(heading), Vector3.UP, heading), Vector3(0.0, -100.0, side * distance))
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
						source.cars = {kind * 10: {"transforms": target, "ids": PackedInt64Array([1]), "ground_flags": PackedByteArray([1])}}
						renderer.update_swarm(1.0 / 60.0)
						var visual := _unpack(source.drawn[kind * 10])
						if DisplayServer.get_name() != "headless":
							var uploaded: Transform3D = renderer.car_mmis[kind * 10].multimesh.get_instance_transform(0)
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
							_expect(clearance >= 0.0198, "buried contact model=%s side=%s direction=%s frame=%d clearance=%f" % [models[kind], side, direction, frame, clearance])
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
		var result: Dictionary = simulation.smooth_car_render_data({0: {"transforms": target, "ids": PackedInt64Array([1]), "ground_flags": PackedByteArray([0 if previous else 1])}}, 0.4 / 24.0)
		var drawn: PackedFloat32Array = result[0]
		_expect(drawn[3] == target[3] and drawn[11] == target[11], "height owner handoff must use target pose")
		if previous:
			_expect(drawn == target, "lane target after handoff must be shown unchanged")
	_expect(_tilted_poses > 100, "fixture must exercise pitch/roll, not flat poses")
	_expect(_largest_center_lift > 0.1, "fixture must require footprint support beyond a centre-height solve")
	if "--benchmark-vehicle-support" in OS.get_cmdline_user_args():
		_benchmark(renderer, source)
	renderer.free()
	simulation.free()
	print("vehicle_ground_support_test: poses=%d tilted=%d minimum_contact_clearance_m=%.6f largest_center_lift_m=%.4f %s" % [_poses, _tilted_poses, _minimum_clearance, _largest_center_lift, "PASS" if _failures == 0 else "FAIL"])
	quit(_failures)

func _benchmark(renderer: Node3D, source: RenderSource) -> void:
	# Time the production update/bridge/upload path separately from correctness oracles and setup.
	# Lane cars are smoothing only; off-lane cars add the native support solve. This synthetic
	# surface has no CDT/site owners; the Rust Kuopio batch bench covers those.
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
