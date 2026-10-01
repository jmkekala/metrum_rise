# SPDX-License-Identifier: GPL-2.0-only

## Headless contract test for per-patch vegetation invalidation after a world edit.
## Trees are a derived product of the terrain surface, so a patch must regenerate when
## the terrain renderer commits a newer surface generation for that patch only.
##
## The vegetation grid is finer than the terrain grid, so "that patch only" now means every
## sub-patch the edited terrain patch owns and no sub-patch of any other. Both generations
## are still published per terrain patch, which is what makes the invalidation conservative.
extends SceneTree

const VegetationScript := preload("res://scripts/renderers/vegetation.gd")
const SPAN_M := 510.0
const SUBDIVISION := VegetationScript.PATCH_SUBDIVISION
const VEGETATION_SPAN_M := SPAN_M / SUBDIVISION
# Sub-patches one terrain patch owns, and so uploads one terrain patch costs.
const SUB_PATCHES := SUBDIVISION * SUBDIVISION
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

	# Moves whenever any generation does, which is the contract of the real counter.
	func get_surface_commit_revision() -> int:
		return generations.hash()

class MockSimulation:
	extends Node

	# One tree per patch is enough: the test asserts which patches regenerate, not placement.
	var patch_fetches: Array[Vector2] = []
	var vegetation_generations: Dictionary = {}
	var advance_during_fetch := false

	func get_vegetation_patch_generation(key: Vector2i) -> int:
		return int(vegetation_generations.get(key, 0))

	func get_land_cover_epoch() -> int:
		return vegetation_generations.hash()

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

class ProcessProbe extends VegetationScript:
	var band_checks := 0
	var stale_checks := 0
	var delay_upload := false

	func _bands_changed(patch: Node3D, distance: float, span: float) -> bool:
		band_checks += 1
		return super._bands_changed(patch, distance, span)

	func _is_patch_stale(key: Vector3i, generations = null) -> bool:
		stale_checks += 1
		return super._is_patch_stale(key, generations)

	func _upload_patch(key: Vector3i, span: float) -> void:
		super._upload_patch(key, span)
		if delay_upload:
			OS.delay_usec(UPLOAD_BUDGET_USEC)

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

	var vegetation := ProcessProbe.new()
	vegetation.name = "Vegetation"
	vegetation.set_process(false)
	host.add_child(vegetation)
	await process_frame

	var near := Vector2i(20, 20)
	var far := Vector2i(21, 20)
	var near_origins := _sub_origins(near)
	terrain.resident = [near, far]
	terrain.generations = {near: 4, far: 4}
	# Place the camera inside the near patch so both patches are within tree range.
	camera.global_position = Vector3(
		(near.x + 0.5) * SPAN_M - WORLD_SIZE.x * 0.5, 200.0,
		(near.y + 0.5) * SPAN_M - WORLD_SIZE.y * 0.5
	)
	vegetation.rebuild_from_simulation_state()

	# At least one upload per frame; allow the worst case for both terrain patches.
	for i in range(SUB_PATCHES * 2 + 1):
		vegetation._process(0.016)
	_expect(
		vegetation.patches.size() == SUB_PATCHES * 2,
		"both resident patches must build every sub-patch, got %d" % vegetation.patches.size()
	)
	_expect(
		simulation.patch_fetches.size() == SUB_PATCHES * 2,
		"each sub-patch must fetch placements once, got %d" % simulation.patch_fetches.size()
	)

	# A road edit dirties one patch. The terrain renderer commits it at a newer generation.
	simulation.patch_fetches.clear()
	terrain.generations[near] = 9
	for i in range(SUB_PATCHES * 2):
		vegetation._process(0.016)
	_expect(
		_sorted(simulation.patch_fetches) == _sorted(near_origins),
		"only the edited patch may regenerate, got %s" % [simulation.patch_fetches]
	)
	_expect(
		vegetation.patches.size() == SUB_PATCHES * 2,
		"the replacement must not orphan a patch key"
	)
	for key in _sub_keys(near):
		_expect(
			int(vegetation.patches[key].get_meta("surface_generation")) == 9,
			"the rebuilt patch must record the generation it was built against"
		)

	# A settled patch must not regenerate again, and an uncommitted patch must not churn.
	vegetation._process(0.016)
	vegetation._process(0.016)
	_expect(vegetation.queue.is_empty(), "a patch at the current generation must stay settled")
	terrain.generations[far] = -1
	vegetation._process(0.016)
	_expect(vegetation.queue.is_empty(), "a patch with no committed payload must not churn")

	# A vegetation edit changes no terrain generation, and only its own patch rebuilds.
	simulation.patch_fetches.clear()
	simulation.vegetation_generations[near] = 1
	for i in range(SUB_PATCHES * 2):
		vegetation._process(0.016)
	_expect(
		_sorted(simulation.patch_fetches) == _sorted(near_origins),
		"vegetation edits must rebuild only their touched patch, got %s" % [simulation.patch_fetches]
	)
	for key in _sub_keys(near):
		_expect(int(vegetation.patches[key].get_meta("vegetation_generation")) == 1, "upload must stamp the independent vegetation revision")
	_expect(terrain.generations[near] == 9, "vegetation must not advance terrain generations")
	for key in _sub_keys(far):
		_expect(not vegetation._is_patch_stale(key), "vegetation edits must leave the neighboring patch settled")

	# A concurrent edit during fetch must remain detectable: revisions are read before fetch.
	var probe: Vector3i = _sub_keys(near)[0]
	simulation.advance_during_fetch = true
	vegetation._upload_patch(probe, VEGETATION_SPAN_M)
	_expect(vegetation._is_patch_stale(probe), "an edit during placement fetch must not be stamped as already rendered")
	simulation.advance_during_fetch = false
	vegetation._upload_patch(probe, VEGETATION_SPAN_M)
	_expect(not vegetation._is_patch_stale(probe), "a subsequent upload must settle the vegetation revision")

	_check_hysteresis(vegetation)
	# Finish the other sub-patches dirtied by the concurrent-fetch check above.
	for _i in range(SUB_PATCHES * 2):
		vegetation._process(0.016)
	for _i in range(8):
		vegetation.band_checks = 0
		vegetation.stale_checks = 0
		vegetation._process(0.016)
		_expect(vegetation.band_checks == 0, "a stationary camera must not walk patch bands")
		_expect(vegetation.stale_checks == 0,
			"an idle frame with no revision change must check no patch generation")

	# This patch already carries near meshes. Restore it outside the entry distance but
	# inside the retention interval: its content remains valid and its node must survive.
	var patch_id: int = vegetation.patches[probe].get_instance_id()
	var center := Vector2(probe.x, probe.y) * VEGETATION_SPAN_M - WORLD_SIZE * 0.5 + Vector2.ONE * VEGETATION_SPAN_M * 0.5
	var retention_distance := vegetation.canopy_near_m() + VEGETATION_SPAN_M * VegetationScript.PATCH_HALF_DIAGONAL + VegetationScript.BAND_HYSTERESIS_M * 0.5
	vegetation._retire_patch(probe, WORLD_SIZE, vegetation._camera_xz())
	camera.position = Vector3(center.x + retention_distance, 200.0, center.y)
	terrain.revision += 1
	for _i in range(SUB_PATCHES * 2):
		vegetation._process(0.016)
	_expect(vegetation.patches[probe].get_instance_id() == patch_id,
		"a restored patch in hysteresis slack must reuse its valid content")
	_expect(vegetation.patches[probe].get_meta("near_band"), "restoration must retain near meshes in slack")

	# A queued band request can become obsolete before its upload turn.
	vegetation._queue_patch(probe)
	vegetation._process(0.016)
	_expect(vegetation.patches[probe].get_instance_id() == patch_id,
		"an obsolete band request must not rebuild a valid patch")

	vegetation._retire_patch(probe, WORLD_SIZE, vegetation._camera_xz())
	simulation.vegetation_generations[near] += 1
	terrain.revision += 1
	for _i in range(SUB_PATCHES * 2):
		vegetation._process(0.016)
	_expect(vegetation.patches[probe].get_instance_id() != patch_id,
		"a restored patch edited while hidden must rebuild")
	_expect(not vegetation._is_patch_stale(probe), "restoration must publish the current generation")
	_expect(vegetation.patches[probe].get_meta("near_band"), "an edit must preserve band history")

	# Make each upload exceed the time budget. Two pending uploads must span two frames,
	# even when residency changes between them, and each frame must make progress.
	simulation.vegetation_generations[near] += 1
	for key in _sub_keys(near):
		vegetation._queue_patch(key)
		vegetation._queue_patch(key)
	_expect(vegetation.queue.size() == SUB_PATCHES, "overlapping invalidations must not duplicate queue entries")
	vegetation.delay_upload = true
	for _i in range(2):
		var generated_before: int = vegetation.generated_patches
		terrain.revision += 1
		vegetation._process(0.016)
		_expect(vegetation.generated_patches == generated_before + 1,
			"the time budget must permit one upload and stop before a second")
		_expect(not vegetation.queue.is_empty(), "residency changes must preserve pending edits")
	vegetation.delay_upload = false
	for _i in range(SUB_PATCHES * 2):
		vegetation._process(0.016)
	for key in _sub_keys(near):
		_expect(not vegetation._is_patch_stale(key), "all pending edits must eventually settle")
	_expect(vegetation.queue.is_empty() and vegetation.queued.is_empty(), "settling must drain both queue stores")
	host.free()
	if _failures == 0:
		print("vegetation_invalidation_test: PASS")
	quit(1 if _failures > 0 else 0)

func _check_hysteresis(vegetation: Node3D) -> void:
	var half := VEGETATION_SPAN_M * VegetationScript.PATCH_HALF_DIAGONAL
	var slack := VegetationScript.BAND_HYSTERESIS_M
	for method in ["_near_band_wanted", "_understory_wanted"]:
		var boundary: float = (vegetation.canopy_near_m() if method == "_near_band_wanted" else maxf(VegetationScript.BUSH_RANGE_M, VegetationScript.ROCK_RANGE_M)) + half
		_expect(vegetation.call(method, boundary, VEGETATION_SPAN_M, false), "approach must enter at the original boundary")
		_expect(not vegetation.call(method, boundary + 1.0, VEGETATION_SPAN_M, false), "new content must retain its original entry distance")
		_expect(vegetation.call(method, boundary + slack, VEGETATION_SPAN_M, true), "retreat must retain content through the slack interval")
		_expect(not vegetation.call(method, boundary + slack + 1.0, VEGETATION_SPAN_M, true), "retreat must drop content beyond the slack interval")
	var casts := VegetationScript.ShadowCaster.IMPOSTOR
	var none := VegetationScript.ShadowCaster.NONE
	var boundary: float = VegetationScript.SceneLightingConfig.shadow_max_distance_m() + half
	_expect(vegetation._shadow_caster_wanted(boundary, VEGETATION_SPAN_M, none) == casts, "shadow approach must start casting at the original boundary")
	_expect(vegetation._shadow_caster_wanted(boundary + 1.0, VEGETATION_SPAN_M, none) == none, "a patch that casts nothing must keep its original entry distance")
	_expect(vegetation._shadow_caster_wanted(boundary + slack, VEGETATION_SPAN_M, casts) == casts, "shadow retreat must retain the caster through slack")
	_expect(vegetation._shadow_caster_wanted(boundary + slack + 1.0, VEGETATION_SPAN_M, casts) == none, "shadow retreat must drop the caster beyond slack")

## Every vegetation sub-patch key one terrain render patch owns.
func _sub_keys(terrain_key: Vector2i) -> Array[Vector3i]:
	var keys: Array[Vector3i] = []
	for column in range(SUBDIVISION):
		for row in range(SUBDIVISION):
			keys.append(Vector3i(terrain_key.x * SUBDIVISION + column,
				terrain_key.y * SUBDIVISION + row, SUBDIVISION))
	return keys

## World-space minimum corner of every sub-patch one terrain render patch owns.
func _sub_origins(terrain_key: Vector2i) -> Array[Vector2]:
	var origins: Array[Vector2] = []
	for key in _sub_keys(terrain_key):
		origins.append(Vector2(key.x, key.y) * VEGETATION_SPAN_M - WORLD_SIZE * 0.5)
	return origins

## Upload order follows the queue, which sorts by distance, so a set comparison is what the
## contract actually claims: these origins and no others.
func _sorted(origins: Array) -> Array:
	var copy := origins.duplicate()
	copy.sort_custom(func(a: Vector2, b: Vector2) -> bool:
		return a.x < b.x if a.x != b.x else a.y < b.y)
	return copy

func _expect(condition: bool, message: String) -> void:
	if condition:
		return
	_failures += 1
	push_error(message)
