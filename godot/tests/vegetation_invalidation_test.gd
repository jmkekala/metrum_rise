# SPDX-License-Identifier: GPL-2.0-only

## Headless contract test for per-patch vegetation invalidation after a world edit.
## Trees are a derived product of the terrain surface, so a patch must regenerate when
## the terrain renderer commits a newer surface generation for that patch only.
extends SceneTree

const VegetationScript := preload("res://scripts/renderers/vegetation.gd")
const SPAN_M := 510.0
const WORLD_SIZE := Vector2(20400.0, 20400.0)

class MockTerrain:
	extends Node

	var resident: Array[Vector2i] = []
	var revision: int = 1
	var generations: Dictionary = {}

	func get_render_patch_span_m() -> float:
		return SPAN_M

	func get_resident_patch_keys() -> Array[Vector2i]:
		return resident

	func get_resident_patch_revision() -> int:
		return revision

	func get_patch_surface_generation(key: Vector2i) -> int:
		return int(generations.get(key, -1))

class MockSimulation:
	extends Node

	# One tree per patch is enough: the test asserts which patches regenerate, not placement.
	var patch_fetches: Array[Vector2] = []
	var vegetation_generations: Dictionary = {}
	var advance_during_fetch := false

	func get_vegetation_patch_generation(key: Vector2i) -> int:
		return int(vegetation_generations.get(key, 0))

	func get_terrain_world_size() -> Vector2:
		return WORLD_SIZE

	func get_decorative_tree_patch(
		origin: Vector2, _span: float, _understory: bool
	) -> PackedFloat32Array:
		patch_fetches.append(origin)
		if advance_during_fetch:
			var key := Vector2i(((origin + WORLD_SIZE * 0.5) / SPAN_M).floor())
			vegetation_generations[key] = get_vegetation_patch_generation(key) + 1
		return PackedFloat32Array([1.0, 0.0, 1.0, 0.0, 1.0, 0.0])

var _failures := 0

func _initialize() -> void:
	_run()

func _run() -> void:
	var host := Node3D.new()
	root.add_child(host)
	# The nodes must be inside the tree before global transforms or the viewport camera
	# are valid, and Vegetation resolves its siblings with @onready.
	await process_frame
	var camera := Camera3D.new()
	host.add_child(camera)
	camera.current = true

	var terrain := MockTerrain.new()
	terrain.name = "Terrain"
	host.add_child(terrain)
	var simulation := MockSimulation.new()
	simulation.name = "SimulationNode"
	host.add_child(simulation)

	var vegetation := VegetationScript.new()
	vegetation.name = "Vegetation"
	vegetation.set_process(false)
	host.add_child(vegetation)
	await process_frame

	var near := Vector2i(20, 20)
	var far := Vector2i(21, 20)
	terrain.resident = [near, far]
	terrain.generations = {near: 4, far: 4}
	# Place the camera inside the near patch so both patches are within tree range.
	camera.global_position = Vector3(
		(near.x + 0.5) * SPAN_M - WORLD_SIZE.x * 0.5, 200.0,
		(near.y + 0.5) * SPAN_M - WORLD_SIZE.y * 0.5
	)
	vegetation.rebuild_from_simulation_state()

	# One patch per frame, so two frames build both.
	for i in range(3):
		vegetation._process(0.016)
	_expect(vegetation.patches.size() == 2, "both resident patches must build")
	_expect(simulation.patch_fetches.size() == 2, "each patch must fetch placements once")

	# A road edit dirties one patch. The terrain renderer commits it at a newer generation.
	simulation.patch_fetches.clear()
	terrain.generations[near] = 9
	vegetation._process(0.016)
	var near_origin := Vector2(near) * SPAN_M - WORLD_SIZE * 0.5
	_expect(
		simulation.patch_fetches == [near_origin],
		"only the edited patch may regenerate, got %s" % [simulation.patch_fetches]
	)
	_expect(vegetation.patches.size() == 2, "the replacement must not orphan a patch key")
	_expect(
		int(vegetation.patches[near].get_meta("surface_generation")) == 9,
		"the rebuilt patch must record the generation it was built against"
	)

	# A settled patch must not regenerate again, and an uncommitted patch must not churn.
	vegetation._process(0.016)
	_expect(vegetation.queue.is_empty(), "a patch at the current generation must stay settled")
	terrain.generations[far] = -1
	vegetation._process(0.016)
	_expect(vegetation.queue.is_empty(), "a patch with no committed payload must not churn")

	# A vegetation edit changes no terrain generation, and only its own patch rebuilds.
	simulation.patch_fetches.clear()
	simulation.vegetation_generations[near] = 1
	vegetation._process(0.016)
	_expect(simulation.patch_fetches == [near_origin], "vegetation edits must rebuild only their touched patch")
	_expect(int(vegetation.patches[near].get_meta("vegetation_generation")) == 1, "upload must stamp the independent vegetation revision")
	_expect(terrain.generations[near] == 9, "vegetation must not advance terrain generations")
	_expect(not vegetation._is_patch_stale(far), "vegetation edits must leave the neighboring patch settled")

	# A concurrent edit during fetch must remain detectable: revisions are read before fetch.
	simulation.advance_during_fetch = true
	vegetation._upload_patch(near, SPAN_M)
	_expect(vegetation._is_patch_stale(near), "an edit during placement fetch must not be stamped as already rendered")
	simulation.advance_during_fetch = false
	vegetation._upload_patch(near, SPAN_M)
	_expect(not vegetation._is_patch_stale(near), "a subsequent upload must settle the vegetation revision")

	host.free()
	quit(1 if _failures > 0 else 0)

func _expect(condition: bool, message: String) -> void:
	if condition:
		return
	_failures += 1
	push_error(message)
