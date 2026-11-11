# SPDX-License-Identifier: GPL-2.0-only

## Vegetation presentation reuses terrain residency; Rust owns the generated and edited state.
## Rust supplies deterministic patch-local placements for four species: conifer, broadleaf,
## bush and rock. Uploads use a time budget, with at least one pending upload per frame.
## A patch regenerates when the terrain renderer commits a newer surface generation for it,
## or Rust advances its independent vegetation revision. Later surface edits clear generated
## scatter; player placements remain authoritative.
extends Node3D

const TreeSpecies := preload("res://scripts/renderers/tree_species.gd")
# The shadow caster choice is bounded by the sun's own range; see _shadow_caster_wanted.
const SceneLightingConfig := preload("res://scripts/core/scene_lighting.gd")

# Per-species draw ranges. Canopy trees use near meshes and impostors for the silhouette
# of the landscape at every distance. Understory reads only close up, so it gets one level
# and a short range: it exists to stop near forest floor looking like mown lawn.
#
# Every range below is evaluated ONCE PER PATCH. Godot tests one distance for a whole
# MultiMeshInstance3D, so a plant gets the level its patch centre asks for and not the level
# its own distance asks for. Two rules follow, and both were broken before this was measured
# from a ground-level camera:
#   - A band must be wider than the patch diagonal, or one patch spans several bands and
#     the level it draws is wrong for most of what it contains. Only the near canopy is a
#     band: the two near levels share one instance and TREE_NEAR_DETAIL_M picks its mesh per
#     patch. The distant band draws one impostor per tree.
#   - A range must be longer than the patch half-diagonal, or the patch under the camera
#     can be culled while the plants at the camera's feet are still in view.
# Both rules are a tax on the patch size, and the patch used to be a 510 m terrain patch:
# 361 m from centre to corner, which put a 721 m floor under the near band on its own. That
# is why the near canopy reached 800 m, where a tree is 13 pixels tall. PATCH_SUBDIVISION
# below cuts the vegetation grid off the terrain grid so the band can follow the trees.
# Lane five of a packed placement carries the species ordinal in its low two bits and, above
# them, the renderer mesh variant the brush pinned, biased by one. Zero there leaves the
# variant to the appearance seed, which is every generated plant and every plant authored
# before the brush could name one tree. Rust packs the lane in vegetation_api.rs.
const SPECIES_BITS := 2
const SPECIES_MASK := 3

# Distance beyond which a tree hands over from its full level to the reduced level that
# tools/model_trees.py exports beside each tree: the trunk and main limbs, and one card in five
# grown to cover the same crown. It is 17% to 25% of the full tree's triangles. At 1080p and the
# default 75-degree vertical FOV the projection is 703.7 px/rad, so a 15 m tree covers
# 15 / d * 703.7 pixels: about 235 px here, where the two levels are plainly different trees.
# So each tree dissolves from one into the other by its own distance over
# TreeSpecies.TREE_DETAIL_FADE_M, like the impostor handover. This used to be a whole-patch mesh
# swap on the distance to the patch centre: trees 15 m from the camera changed with trees 100 m
# away, and a small orbit flipped them back and forth. The patch still picks one mesh per
# variant on the buffer already uploaded: the full or the reduced level where the band misses
# all its trees, and both levels in one mesh where it can reach any of them.
const TREE_NEAR_DETAIL_M := TreeSpecies.TREE_NEAR_DETAIL_M
# Values of the near_detail_lod patch meta: the mesh index each near variant draws.
const DETAIL_FULL := 0
const DETAIL_REDUCED := 1
const DETAIL_BLEND := 2
const TREE_FAR_M := TreeSpecies.TREE_FAR_M
const BUSH_RANGE_M := 420.0
# Level of a patch's distant hedge runs. A hedge is part of a yard, so it stays as long as the
# trees do: modules in the near band, one box per straight run beyond it.
const HEDGE_RUN_LOD := 3
const ROCK_RANGE_M := 420.0
# Half the diagonal of a square patch, per metre of span. A patch is one instance, so every
# conservative range test measures from the patch centre out to its farthest corner.
const PATCH_HALF_DIAGONAL := 0.7071067811865476
# Bush and rock carry a single level, so a variant that stops early drops nothing into a gap:
# the plant ends and the rest of the patch carries on. The product (Rust) staggers the twelve
# variants' ranges over the last quarter of the band, so the carpet dissolves there instead of
# ending every plant of one patch on one line, which a camera above the canopy reads as a
# straight edge between forest floor and bare ground.

# How many vegetation patches one terrain render patch is cut into along each axis. The
# vegetation grid WAS the terrain grid, and that is what set every distance above: a patch is
# one instance, so no band can be narrower than the patch that has to fit inside it, and a
# 510 m terrain patch therefore forced a 721 m floor under the near band. The trees never
# needed that. Rust already takes an arbitrary origin and span in get_decorative_tree_patch,
# so the finer grid costs no simulation change: a sub-patch key divides back to its owning
# terrain key for both staleness questions, which leaves invalidation terrain-coarse and so
# conservative. Subdividing raises the distant instance count by its square, which is the
# cost this buys the near band's area reduction with.
const PATCH_SUBDIVISION := 4
# Retain richer content beyond its entry distance. Never delay an approaching upgrade.
const BAND_HYSTERESIS_M := 64.0
# A single upload is indivisible and may exceed these soft budgets. A patch that draws nothing
# yet gets the larger one: at 2 ms a camera flying over forest left up to 220 wanted patches
# unbuilt, and the ground showed through where their trees belonged.
const UPLOAD_BUDGET_USEC := 2000
const MISSING_UPLOAD_BUDGET_USEC := 6000

## Whether a patch casts. A canopy tree casts from its branched level within
## TreeSpecies.TREE_SHADOW_HANDOVER_M of the camera and from its impostor's silhouette, faced to
## the light, past it. NONE is for patches the sun's shadow range does not reach at all.
enum ShadowCaster { NONE, IMPOSTOR }

var enabled := true
var density_fraction := 1.0
var cast_shadows := true
# Probe override for the canopy far range, in metres. Values at or below zero keep the
# authored TREE_FAR_M. Must exceed the near range to leave a distant band.
var far_range_override_m := 0.0
# Probe override for the canopy near range, in metres: the distance the handover to the
# impostor ends on. Values at or below zero keep TreeSpecies.TREE_CROSSFADE_END_M.
var near_range_override_m := 0.0
# Probe override for PATCH_SUBDIVISION. Values below one keep the authored subdivision.
var patch_subdivision_override := 0
# Probe override for TREE_NEAR_DETAIL_M, in metres. Values below zero keep the authored
# distance. Zero is meaningful: past the fade band it draws the near band from the reduced level.
var near_detail_override_m := -1.0
# Probe override for the per-frame upload budget, in sub-patch builds. Values below one keep
# the area budget the subdivision derives. Separates "how fine is the grid" from "how much
# work may one frame do", which the derived budget ties together.
var upload_budget_override := 0
# Vegetation patch span in metres, cached from the last residency pass so the range
# accessors can answer without a terrain call. Zero until the first patch is built.
var patch_span_m := 0.0
var patches: Dictionary = {}
# Patches that left terrain residency but are still inside the scatter radius. Terrain
# residency is a square about the camera (not its frustum), so a camera moving back wants
# them again. These are hidden rather than freed, because rebuilding one costs a frame.
var cache: Dictionary = {}
var queue: Array[Vector3i] = []
# Membership avoids duplicate work when camera checks and edit polling overlap.
var queued: Dictionary = {}
# Patches no longer wanted that stay drawn until the wanted patches over their area are built.
# A terrain block that moves between the coarse and fine grids swaps one patch for sixteen, and
# hiding the old one first left the whole 510 m block bare until the last of them uploaded.
var standins: Dictionary = {}
# The wanted set of the last residency pass, which decides when a stand-in may go.
var wanted_keys: Dictionary = {}
# The two revision counters at the last staleness sweep; the sweep reruns when either moves.
var stale_land_epoch := -1
var stale_surface_revision := -1
var last_band_camera_xz := Vector2.INF
# The detail handover measures in three dimensions, so a camera that only rises moves it too.
var last_detail_camera := Vector3.INF
# meshes[species][variant] is an Array[ArrayMesh], ordered near to far. A canopy variant holds
# the full level, the reduced level, and both in one mesh; see TREE_NEAR_DETAIL_M.
var meshes: Array = []
var last_revision := -1
# Span of one terrain render patch, in metres. A patch key carries the grid it belongs to,
# and this is what turns that grid back into a distance.
var terrain_span_m := 0.0
var last_camera_cell := Vector2i(2147483647, 2147483647)
var tree_count := 0
var generated_patches := 0
var generation_ms_max := 0.0
# Both staleness generations, keyed by owner patch, for one frame's residency pass and
# staleness sweep. Every sub-patch of one terrain patch shares an owner, so this crosses into
# Rust once per owner instead of once per sub-patch. Cleared, never replaced, each frame.
var owner_generations: Dictionary = {}
var ready_for_world := false
# Microseconds this frame's uploads spent in the Rust placement and stand cover fetches, for
# the perf channel (`RENDER-27`: the larger part of an upload on a city).
var _fetch_us := 0
# Microseconds this frame's uploads spent building their products in Rust (`RENDER-32-C`).
var _product_us := 0
# The mesh catalogue packed for the product; see _product_catalogue.
var _catalogue := PackedFloat32Array()
@onready var terrain = $"../Terrain"
@onready var simulation = $"../SimulationNode"

func _ready() -> void:
	meshes = TreeSpecies.build_meshes()
	# Texture IO and array creation belong to catalogue setup, outside patch uploads.
	TreeSpecies.impostor_mesh()
	for species in [TreeSpecies.CONIFER, TreeSpecies.BROADLEAF]:
		TreeSpecies.impostor_material(species)
	TreeSpecies.set_crossfade(canopy_near_m())
	TreeSpecies.set_detail(near_detail_m())
	TreeSpecies.set_far(canopy_far_m())

## Canopy far range in effect. One accessor so the draw range and the patch residency test
## can never disagree about how far the scatter reaches.
func canopy_far_m() -> float:
	return far_range_override_m if far_range_override_m > canopy_near_m() else TREE_FAR_M

## Distance every tree has finished its handover to the impostor by. The handover is per tree,
## so this no longer has to clear the patch diagonal; the patch ranges above pay for that
## instead. It never reads `patch_span_m`: an upload sets that to its own key's span, and a
## band read from it once flipped with every upload and rebuilt the forest without end.
func canopy_near_m() -> float:
	return near_range_override_m if near_range_override_m > 0.0 else TreeSpecies.TREE_CROSSFADE_END_M

## Distance a tree begins its handover to the impostor on.
func canopy_crossfade_begin_m() -> float:
	return canopy_near_m() - TreeSpecies.TREE_CROSSFADE_M

## Distance the branched tree hands over to the reduced near level. One accessor so the two
## bands can never disagree about where they meet.
func near_detail_m() -> float:
	return near_detail_override_m if near_detail_override_m >= 0.0 else TREE_NEAR_DETAIL_M

## Vegetation patches per terrain render patch along one axis, with the probe override.
## Also the square root of the per-frame upload budget; see the budget in _process.
func patch_subdivision() -> int:
	return patch_subdivision_override if patch_subdivision_override >= 1 else PATCH_SUBDIVISION

## The terrain render patch that owns a vegetation sub-patch. Both staleness questions are
## answered on the terrain grid: the surface generation is published there, and routing the
## vegetation revision through the same key keeps one edit invalidating a whole terrain
## patch. That is coarser than it has to be and therefore never misses an edit.
func _owner_key(key: Vector3i) -> Vector2i:
	var divisor := maxi(key.z, 1)
	return Vector2i(floori(float(key.x) / divisor), floori(float(key.y) / divisor))

## Span of the patch this key names, in metres. The key carries its own grid divisor, so a
## coarse far patch and a fine near patch answer this differently and never collide in the
## patch dictionary even where their corners coincide.
func _key_span(key: Vector3i) -> float:
	return terrain_span_m / float(maxi(key.z, 1))

## Distance within which a terrain block has to be carried on the fine grid. Only a patch
## that can hold near canopy or understory needs the fine grid: those are the levels whose
## band is narrower than a terrain patch. Everything beyond draws one distant crown mesh,
## which is chosen per patch and needs no band at all, so subdividing it buys nothing and
## costs a patch, four instances and a sweep entry each.
func _fine_tier_radius() -> float:
	return maxf(canopy_near_m(), maxf(BUSH_RANGE_M, ROCK_RANGE_M))

func rebuild_from_simulation_state() -> void:
	TreeSpecies.set_crossfade(canopy_near_m())
	TreeSpecies.set_detail(near_detail_m())
	TreeSpecies.set_far(canopy_far_m())
	for patch in patches.values():
		patch.queue_free()
	patches.clear()
	for patch in cache.values():
		patch.queue_free()
	cache.clear()
	for patch in standins.values():
		patch.queue_free()
	standins.clear()
	wanted_keys.clear()
	queue.clear()
	queued.clear()
	stale_land_epoch = -1
	stale_surface_revision = -1
	last_band_camera_xz = Vector2.INF
	tree_count = 0
	generated_patches = 0
	generation_ms_max = 0.0
	last_revision = -1
	last_camera_cell = Vector2i(2147483647, 2147483647)
	ready_for_world = true

func _process(_delta: float) -> void:
	visible = enabled
	if not enabled or not ready_for_world:
		return
	var camera := get_viewport().get_camera_3d()
	if camera == null:
		return
	var terrain_span: float = terrain.get_render_patch_span_m()
	if terrain_span <= 0.0:
		return
	terrain_span_m = terrain_span
	var divisor := patch_subdivision()
	var span := terrain_span / divisor
	patch_span_m = span
	owner_generations.clear()
	var camera_xz := Vector2(camera.global_position.x, camera.global_position.z)
	var camera_cell := Vector2i((camera_xz / span).floor())
	var start_us := Time.get_ticks_usec()
	_fetch_us = 0
	_product_us = 0
	var revision: int = terrain.get_resident_patch_revision()
	# O(resident patches) only on residency/camera-cell changes, not O(total scatter).
	var residency_changed := revision != last_revision or camera_cell != last_camera_cell
	if residency_changed:
		last_revision = revision
		last_camera_cell = camera_cell
		var wanted: Dictionary = {}
		var world: Vector2 = simulation.get_terrain_world_size()
		# Terrain residency is answered on the terrain grid. Each resident terrain patch expands
		# into the sub-patches it owns, so residency stays terrain-coarse while every distance
		# below is answered on the finer grid.
		var fine_radius := _fine_tier_radius()
		for terrain_key in terrain.get_resident_patch_keys():
			var block := Vector2i(terrain_key)
			var block_center := (Vector2(block) * terrain_span - world * 0.5
				+ Vector2.ONE * terrain_span * 0.5)
			var block_distance := block_center.distance_to(camera_xz)
			# Measured from the block's nearest corner, so a block with any part of it inside
			# the fine radius is carried whole on the fine grid.
			if block_distance - terrain_span * PATCH_HALF_DIAGONAL <= fine_radius:
				for column in range(divisor):
					for row in range(divisor):
						var key := Vector3i(block.x * divisor + column, block.y * divisor + row, divisor)
						var center := Vector2(key.x, key.y) * span - world * 0.5 + Vector2.ONE * span * 0.5
						if center.distance_to(camera_xz) <= canopy_far_m() + span:
							wanted[key] = center.distance_squared_to(camera_xz)
			elif block_distance <= canopy_far_m() + terrain_span:
				wanted[Vector3i(block.x, block.y, 1)] = block_center.distance_squared_to(camera_xz)
		wanted_keys = wanted
		for key in patches.keys():
			if not wanted.has(key):
				var outgoing: Node3D = patches[key]
				tree_count -= int(outgoing.get_meta("tree_count"))
				patches.erase(key)
				standins[key] = outgoing
		# A cached patch outside the scatter radius will not be wanted again from here, so it
		# is freed. A cached fine patch is freed once its block is a whole terrain patch past
		# the fine tier: residency covers every direction, so the cache only serves a camera
		# moving back. Keeping fine patches over the whole disk held dozens of nodes each, and
		# every vegetation node takes 16 of the 65536 instance uniform slots, so a long flight
		# overflowed the buffer and new trees failed to draw.
		var fine_keep := fine_radius + terrain_span * (1.0 + PATCH_HALF_DIAGONAL)
		for key in cache.keys():
			var cached_distance := _patch_distance(key, world, camera_xz)
			if (cached_distance > canopy_far_m() + _key_span(key)
					or (key.z > 1 and cached_distance > fine_keep)):
				cache[key].queue_free()
				cache.erase(key)
		# Preserve pending edits through residency changes. Compact in O(queue size), rather
		# than removing array entries one by one or discarding work that polling found.
		var retained := 0
		for key in queue:
			if wanted.has(key):
				queue[retained] = key
				retained += 1
			else:
				queued.erase(key)
		queue.resize(retained)
		for key in wanted:
			if patches.has(key):
				continue
			if standins.has(key):
				patches[key] = standins[key]
				standins.erase(key)
				tree_count += int(patches[key].get_meta("tree_count"))
				if _is_patch_stale(key, owner_generations):
					_queue_patch(key)
				continue
			if cache.has(key):
				_restore_patch(key)
				if _is_patch_stale(key, owner_generations):
					_queue_patch(key)
				continue
			_queue_patch(key)
		# Popped from the back: patches that draw nothing yet come last, nearest last of all.
		queue.sort_custom(func(a: Vector3i, b: Vector3i) -> bool:
			var a_missing := not patches.has(a)
			if a_missing != (not patches.has(b)):
				return not a_missing
			return wanted[a] > wanted[b])
	var residency_us := Time.get_ticks_usec()
	# O(resident patches) on camera/residency changes only. Rotation about a stationary
	# camera changes no distance; restored patches still need their mesh and band checks.
	if residency_changed or camera_xz != last_band_camera_xz:
		last_band_camera_xz = camera_xz
		var world: Vector2 = simulation.get_terrain_world_size()
		for key in patches:
			var patch: Node3D = patches[key]
			var distance := _patch_distance(key, world, camera_xz)
			if _bands_changed(patch, distance, _key_span(key)):
				_queue_patch(key)
	var bands_us := Time.get_ticks_usec()
	# O(resident patches), O(1) each, on camera/residency changes only.
	var camera_position := _camera_position()
	if residency_changed or camera_position != last_detail_camera:
		last_detail_camera = camera_position
		for key in patches:
			_refresh_near_detail(patches[key], camera_position)
			_refresh_branched_shadow(patches[key], camera_position)
	var detail_us := Time.get_ticks_usec()
	# O(1) while no revision moved. A patch is stale only when its terrain owner committed a
	# newer surface or Rust advanced its vegetation revision, and each of those moves one of
	# these two counters, so the O(resident patches) sweep runs on the frame an edit lands.
	# It runs even while uploads are pending, so a long upload queue cannot hide an edit.
	var land_epoch: int = simulation.get_land_cover_epoch()
	var surface_revision: int = terrain.get_surface_commit_revision()
	if land_epoch != stale_land_epoch or surface_revision != stale_surface_revision:
		stale_land_epoch = land_epoch
		stale_surface_revision = surface_revision
		for key in patches:
			if _is_patch_stale(key, owner_generations):
				_queue_patch(key)
	# Retain the area/count limit as a ceiling, but stop after the first upload that exhausts
	# the elapsed-time budget. A valid cached patch or an obsolete band request costs no upload.
	var budget := upload_budget_override if upload_budget_override >= 1 else divisor * divisor
	var upload_start := Time.get_ticks_usec()
	var uploads := 0
	var upload_max_us := 0
	while budget > 0 and not queue.is_empty():
		var next: Vector3i = queue.pop_back()
		queued.erase(next)
		if patches.has(next):
			var world: Vector2 = simulation.get_terrain_world_size()
			var distance := _patch_distance(next, world, camera_xz)
			if not _is_patch_stale(next) and not _bands_changed(patches[next], distance, _key_span(next)):
				if uploads > 0 and Time.get_ticks_usec() - upload_start >= UPLOAD_BUDGET_USEC:
					break
				continue
		var one_start := Time.get_ticks_usec()
		_upload_patch(next, _key_span(next))
		upload_max_us = maxi(upload_max_us, Time.get_ticks_usec() - one_start)
		budget -= 1
		uploads += 1
		var limit := UPLOAD_BUDGET_USEC
		if not queue.is_empty() and not patches.has(queue.back()):
			limit = MISSING_UPLOAD_BUDGET_USEC
		if Time.get_ticks_usec() - upload_start >= limit:
			break
	var uploaded_us := Time.get_ticks_usec()
	if not standins.is_empty():
		_release_standins(camera_xz)
	if PerfDebug.is_enabled():
		var ms := func(from_us: int, to_us: int) -> float: return float(to_us - from_us) / 1000.0
		var end_us := Time.get_ticks_usec()
		PerfDebug.record("vegetation", ms.call(start_us, end_us), {
			"residency_ms": ms.call(start_us, residency_us), "bands_ms": ms.call(residency_us, bands_us),
			"detail_ms": ms.call(bands_us, detail_us), "stale_ms": ms.call(detail_us, upload_start),
			"uploads_ms": ms.call(upload_start, uploaded_us), "fetch_ms": float(_fetch_us) / 1000.0,
			"product_ms": float(_product_us) / 1000.0,
			"upload_max_ms": float(upload_max_us) / 1000.0, "standins_ms": ms.call(uploaded_us, end_us),
			"uploads": uploads, "residency_changed": 1 if residency_changed else 0,
			"queue": queue.size(), "patches": patches.size(), "wanted": wanted_keys.size()})

## Puts away every stand-in whose area the wanted patches now cover. O(stand-ins x the
## sub-patches of one terrain patch), and stand-ins exist only while a tier change uploads.
func _release_standins(camera_xz: Vector2) -> void:
	var world: Vector2 = simulation.get_terrain_world_size()
	for key in standins.keys():
		if _area_built(key):
			_put_away(key, standins[key], world, camera_xz)
			standins.erase(key)

## Whether no wanted patch over this key's area is still waiting for its first upload.
func _area_built(key: Vector3i) -> bool:
	var divisor := patch_subdivision()
	if key.z == 1:
		for column in range(divisor):
			for row in range(divisor):
				var fine := Vector3i(key.x * divisor + column, key.y * divisor + row, divisor)
				if wanted_keys.has(fine) and not patches.has(fine):
					return false
		return true
	var coarse := Vector3i(floori(float(key.x) / divisor), floori(float(key.y) / divisor), 1)
	return not (wanted_keys.has(coarse) and not patches.has(coarse))

func _queue_patch(key: Vector3i) -> void:
	if not queued.has(key):
		queued[key] = true
		# A patch that already draws something waits behind every patch that draws nothing.
		if patches.has(key):
			queue.push_front(key)
		else:
			queue.append(key)

func _bands_changed(patch: Node3D, distance: float, span: float) -> bool:
	var understory := bool(patch.get_meta("understory"))
	var near_band := bool(patch.get_meta("near_band"))
	var caster := int(patch.get_meta("shadow_caster"))
	return (
		_understory_wanted(distance, span, understory) != understory
		or _near_band_wanted(distance, span, near_band) != near_band
		or _shadow_caster_wanted(distance, span, caster) != caster
	)

func _is_patch_stale(key: Vector3i, cache = null) -> bool:
	# -1 means the terrain patch has committed no payload yet. Keep the current scatter
	# rather than churning; the next commit advances the generation and triggers a rebuild.
	var generations := _owner_generations(_owner_key(key), cache)
	return (
		(generations.x >= 0 and generations.x != int(patches[key].get_meta("surface_generation")))
		or generations.y != int(patches[key].get_meta("vegetation_generation"))
	)

## The surface and vegetation generations of an owner patch, as (surface, vegetation). With a
## cache this crosses into Rust once per owner instead of once per caller; without one it
## always crosses, because the answer is only about the moment it is asked.
func _owner_generations(owner: Vector2i, cache) -> Vector2i:
	if cache != null and cache.has(owner):
		return cache[owner]
	var generations := Vector2i(
		terrain.get_patch_surface_generation(owner),
		simulation.get_vegetation_patch_generation(owner)
	)
	if cache != null:
		cache[owner] = generations
	return generations

## Camera position on the ground plane, or `INF` when there is no camera to measure from.
func _camera_xz() -> Vector2:
	var camera := get_viewport().get_camera_3d()
	if camera == null:
		return Vector2.INF
	return Vector2(camera.global_position.x, camera.global_position.z)

## Camera position, or `INF` when there is no camera to measure from. The shaders measure the
## detail handover from this same point.
func _camera_position() -> Vector3:
	var camera := get_viewport().get_camera_3d()
	return Vector3.INF if camera == null else camera.global_position

## Distance from the camera to this patch centre, or a negative value with no camera. One
## distance feeds every range decision for the patch, so they cannot disagree.
func _patch_distance(key: Vector3i, world: Vector2, camera_xz: Vector2) -> float:
	if camera_xz == Vector2.INF:
		return -1.0
	var span := _key_span(key)
	return (Vector2(key.x, key.y) * span - world * 0.5
		+ Vector2.ONE * span * 0.5).distance_to(camera_xz)

## Whether this patch is close enough to be worth generating the dense understory layer.
## Conservative by the patch half-diagonal, like the near band: a patch centre outside the
## longest understory range can still have a near corner inside it. On the terrain grid that
## slack was 361 m on a 420 m range, so nearly half of every understory patch generated was
## never drawn. Existing content is retained for BAND_HYSTERESIS_M beyond the entry range.
func _understory_wanted(distance: float, span: float, previous: bool = false) -> bool:
	var longest := maxf(BUSH_RANGE_M, ROCK_RANGE_M) + span * PATCH_HALF_DIAGONAL
	return distance >= 0.0 and distance <= longest + (BAND_HYSTERESIS_M if previous else 0.0)

## Whether any part of this patch can fall inside the near band, and so whether the near
## per-variant meshes are worth building at all. A patch is one instance, so the test is
## conservative by the patch half-diagonal rather than by the centre. Beyond this the patch
## builds one distant instance per canopy species and skips the variant split and tints.
## Existing near meshes persist through BAND_HYSTERESIS_M of retreat. Entry is unchanged.
## With no camera every level is built, which is what a headless caller wants.
func _near_band_wanted(distance: float, span: float, previous: bool = false) -> bool:
	return distance < 0.0 or distance <= canopy_near_m() + span * PATCH_HALF_DIAGONAL + (
		BAND_HYSTERESIS_M if previous else 0.0)

## Which near mesh the patch's per-variant instances carry, from the box of its canopy tree
## origins in world space. The shaders dissolve each tree by its own distance, so this only has
## to be conservative: a single level where the whole box lies on one side of the band, and
## both where the band can reach any tree. The farthest point is bounded by the box's radius.
func _near_detail_lod(trees: AABB, camera: Vector3) -> int:
	# With no camera every patch takes the detailed level, which is what a headless caller
	# wants: the catalogue test reads the mesh the instance holds.
	if camera == Vector3.INF:
		return DETAIL_FULL
	if camera.clamp(trees.position, trees.end).distance_to(camera) \
			>= near_detail_m() + TreeSpecies.TREE_DETAIL_FADE_M:
		return DETAIL_REDUCED
	if trees.get_center().distance_to(camera) + trees.size.length() * 0.5 <= near_detail_m():
		return DETAIL_FULL
	return DETAIL_BLEND

## Moves resident near instances between the branched levels. Every level carries the same
## transforms, so this is a mesh write on the buffer already uploaded.
func _refresh_near_detail(patch: Node3D, camera: Vector3) -> void:
	if not bool(patch.get_meta("near_band")):
		return
	var lod := _near_detail_lod(patch.get_meta("canopy_box"), camera)
	if int(patch.get_meta("near_detail_lod")) == lod:
		return
	patch.set_meta("near_detail_lod", lod)
	for instance in patch.get_children():
		var species := int(instance.get_meta("species"))
		if species >= TreeSpecies.BUSH or int(instance.get_meta("lod")) != 0:
			continue
		instance.multimesh.mesh = meshes[species][int(instance.get_meta("variant"))][lod]

## Submits a patch's branched canopy to the shadow pass while any of its trees may stand within
## TREE_SHADOW_HANDOVER_M of the camera, and withdraws it once none can. The shaders pick the
## casting trees one by one; this only keeps the rest of the near band, whose every card would
## otherwise run its vertex stage once per cascade only to collapse, out of the shadow pass.
func _refresh_branched_shadow(patch: Node3D, camera: Vector3) -> void:
	var trees: AABB = patch.get_meta("canopy_box")
	var wants := (cast_shadows and bool(patch.get_meta("near_band"))
		and int(patch.get_meta("shadow_caster")) != ShadowCaster.NONE
		and (camera == Vector3.INF
			or camera.clamp(trees.position, trees.end).distance_to(camera)
				< TreeSpecies.TREE_SHADOW_HANDOVER_M))
	if bool(patch.get_meta("branched_casts", false)) == wants:
		return
	patch.set_meta("branched_casts", wants)
	for instance in patch.get_children():
		if int(instance.get_meta("species")) < TreeSpecies.BUSH and int(instance.get_meta("lod")) == 0:
			instance.cast_shadow = (GeometryInstance3D.SHADOW_CASTING_SETTING_ON if wants
				else GeometryInstance3D.SHADOW_CASTING_SETTING_OFF)

## Re-applies the shadow setting to resident patches. Avoids a full placement rebuild.
func set_cast_shadows(value: bool) -> void:
	cast_shadows = value
	for store in [patches, cache]:
		for patch in store.values():
			patch.set_meta("branched_casts", false)
			var caster: int = patch.get_meta("shadow_caster")
			for instance in patch.get_children():
				var is_proxy: bool = instance.get_meta("shadow_proxy")
				instance.cast_shadow = _shadow_setting(
					int(instance.get_meta("species")), int(instance.get_meta("lod")),
					int(instance.get_meta("variant")), is_proxy, caster
				)
				# OFF permits colour drawing, so a silent proxy must also be hidden.
				instance.visible = (not is_proxy
					or instance.cast_shadow != GeometryInstance3D.SHADOW_CASTING_SETTING_OFF)
			_refresh_branched_shadow(patch, _camera_position())

## Takes a patch out of the drawn set. A patch still inside the scatter radius is hidden and
## kept, because a camera moving back will very likely ask for it again. Only a patch that is
## genuinely out of range is freed.
func _retire_patch(key: Vector3i, world: Vector2, camera_xz: Vector2) -> void:
	var patch: Node3D = patches[key]
	tree_count -= int(patch.get_meta("tree_count"))
	patches.erase(key)
	_put_away(key, patch, world, camera_xz)

## Hides a patch that left the drawn set into the cache, or frees it when out of range.
func _put_away(key: Vector3i, patch: Node3D, world: Vector2, camera_xz: Vector2) -> void:
	if _patch_distance(key, world, camera_xz) <= canopy_far_m() + _key_span(key):
		patch.visible = false
		cache[key] = patch
		return
	patch.queue_free()

## Returns a hidden patch to the drawn set. The caller checks generations and bands in the
## same frame. Retained band content remains valid inside its hysteresis interval.
func _restore_patch(key: Vector3i) -> void:
	var patch: Node3D = cache[key]
	cache.erase(key)
	patch.visible = true
	patches[key] = patch
	tree_count += int(patch.get_meta("tree_count"))

func has_pending_work() -> bool:
	return enabled and (last_revision == -1 or not queue.is_empty())

func metrics() -> Dictionary:
	return {"enabled": enabled, "resident_patches": patches.size(), "resident_trees": tree_count,
		"pending_patches": queue.size(), "cached_patches": cache.size(),
		"generated_patches": generated_patches,
		"max_patch_generation_upload_ms": generation_ms_max, "density_fraction": density_fraction,
		"patch_span_m": patch_span_m, "patch_subdivision": patch_subdivision(),
		"fine_patches": _tier_count(patch_subdivision()), "coarse_patches": _tier_count(1),
		"fine_tier_radius_m": _fine_tier_radius(),
		"canopy_near_m": canopy_near_m(), "canopy_far_m": canopy_far_m()}

## How many resident patches sit on one grid. Reported so a probe can show what the two
## tiers actually cost against each other.
func _tier_count(divisor: int) -> int:
	var total := 0
	for key in patches:
		if key.z == divisor:
			total += 1
	return total

func _upload_patch(key: Vector3i, span: float) -> void:
	var start := Time.get_ticks_usec()
	# Read before the placement fetch. Stamping an older generation onto newer data only
	# causes one redundant rebuild; the reverse would leave a stale patch undetected.
	# Deliberately not the memoised read. The order matters here: this value is stamped onto
	# the patch, so it must be fetched after any edit that preceded this upload and before the
	# placement fetch below. A value memoised earlier in the frame can be newer than that and
	# would stamp an edit as already rendered.
	var owner := _owner_key(key)
	var generation: int = terrain.get_patch_surface_generation(owner)
	var vegetation_generation: int = simulation.get_vegetation_patch_generation(owner)
	var world: Vector2 = simulation.get_terrain_world_size()
	patch_span_m = span
	# A direct caller can name a span the key's own grid does not imply, so the key is made
	# to agree with it before any distance is measured from it.
	terrain_span_m = span * float(maxi(key.z, 1))
	var distance := _patch_distance(key, world, _camera_xz())
	# Preserve band history through edits and queued uploads as well as cache restoration.
	var previous: Node3D = patches.get(key)
	var understory := _understory_wanted(distance, span,
		previous != null and bool(previous.get_meta("understory")))
	var near_band := _near_band_wanted(distance, span,
		previous != null and bool(previous.get_meta("near_band")))
	var caster := _shadow_caster_wanted(distance, span,
		int(previous.get_meta("shadow_caster")) if previous != null else ShadowCaster.NONE)
	var origin :=Vector2(key.x, key.y) * span - world * 0.5
	var fetch_start := Time.get_ticks_usec()
	var data: PackedFloat32Array = simulation.get_decorative_tree_patch(origin, span, understory)
	# Crown coverage around the patch, averaged over 40 m, which each canopy tree samples once
	# for its stand closure.
	var cover: Dictionary = simulation.get_vegetation_stand_cover(origin, span)
	# The distant hedge: one box per straight run Rust merged from the modules.
	var runs: PackedFloat32Array = (PackedFloat32Array() if near_band
		else simulation.get_vegetation_hedge_runs(origin, span))
	_fetch_us += Time.get_ticks_usec() - fetch_start
	# Every buffer, bounds and range of the patch, built in Rust (`RENDER-32-C`): the appearance
	# seed, variant, transform, tint and stand closure of each plant, the impostor rows, the hedge
	# boxes and the bounds every level of a species shares. One buffer assignment per instance.
	var product_start := Time.get_ticks_usec()
	var product: Dictionary = SimulationNode.build_vegetation_product(data, cover, runs, origin,
		near_band, caster != ShadowCaster.NONE, _product_settings(), _product_catalogue())
	_product_us += Time.get_ticks_usec() - product_start
	var patch := Node3D.new()
	patch.position = Vector3(origin.x, 0.0, origin.y)
	add_child(patch)
	var batches: PackedInt32Array = product.get("batches", PackedInt32Array())
	var bounds: PackedFloat32Array = product.get("bounds", PackedFloat32Array())
	var buffers: Array = product.get("buffers", [])
	# Eight ints and fourteen floats a batch; see SimulationNode.build_vegetation_product.
	for batch in range(batches.size() / 8):
		var at := batch * 8
		var species := batches[at]
		var lod := batches[at + 1]
		var variant := batches[at + 2]
		var mm := MultiMesh.new()
		mm.transform_format = MultiMesh.TRANSFORM_3D
		mm.use_colors = batches[at + 5] == 1
		mm.use_custom_data = batches[at + 6] == 1
		# A canopy variant's level is chosen below, once the patch knows where its trees stand.
		mm.mesh = (TreeSpecies.impostor_mesh() if lod == 2
			else TreeSpecies.hedge_run_mesh(variant - TreeSpecies.HEDGE_FIRST_VARIANT) if lod == HEDGE_RUN_LOD
			else meshes[species][variant][DETAIL_FULL])
		mm.instance_count = batches[at + 4]
		# A shadow caster names its impostor's buffer: packed arrays are copy-on-write, so the
		# two share the data instead of copying it.
		mm.buffer = buffers[batches[at + 7]]
		var box := batch * 14
		if lod == 2:
			# The shader moves vertices outside the quad's own bounds; Rust bounds every form.
			mm.custom_aabb = _box(bounds, box)
		var instance := _add_instance(patch, mm, species, lod, variant, caster, batches[at + 3] == 1)
		var shared := _box(bounds, box + 6)
		if shared.size != Vector3.ZERO:
			instance.custom_aabb = shared
		# No fade mode. VISIBILITY_RANGE_FADE_SELF alpha-blends the whole instance, which is a
		# whole patch, and moves it out of the depth pre-pass. The canopy hands over per tree in
		# its shaders instead, and these ranges only bound the patches that take part.
		instance.visibility_range_begin = bounds[box + 12]
		instance.visibility_range_end = bounds[box + 13]
	var count: int = product.get("tree_count", 0)
	patch.set_meta("tree_count", count)
	patch.set_meta("surface_generation", generation)
	patch.set_meta("vegetation_generation", vegetation_generation)
	patch.set_meta("understory", understory)
	patch.set_meta("near_band", near_band)
	patch.set_meta("shadow_caster", caster)
	# Every canopy tree origin of the patch in world space, which the detail choice measures.
	patch.set_meta("canopy_box", product.get("canopy_box", AABB()))
	patch.set_meta("near_detail_lod", DETAIL_FULL)
	_refresh_near_detail(patch, _camera_position())
	_refresh_branched_shadow(patch, _camera_position())
	# Replace in place. The previous node stays visible until this one is in the tree,
	# so an edit does not blank the surrounding forest for a frame.
	if patches.has(key):
		tree_count -= int(patches[key].get_meta("tree_count"))
		patches[key].queue_free()
	patches[key] = patch
	tree_count += count
	generated_patches += 1
	generation_ms_max = maxf(generation_ms_max, float(Time.get_ticks_usec() - start) / 1000.0)

func _add_instance(
	patch: Node3D, mm: MultiMesh, species: int, lod: int, variant: int,
	caster: int, is_proxy: bool = false
) -> MultiMeshInstance3D:
	var instance := MultiMeshInstance3D.new()
	instance.multimesh = mm
	if lod == 2:
		instance.material_override = (TreeSpecies.impostor_shadow_material(species) if is_proxy
			else TreeSpecies.impostor_material(species))
	instance.set_meta("species", species)
	instance.set_meta("lod", lod)
	instance.set_meta("shadow_proxy", is_proxy)
	# Near instances record their bucket; distant instances carry their form in custom data.
	instance.set_meta("variant", variant)
	instance.cast_shadow = _shadow_setting(species, lod, variant, is_proxy, caster)
	# A proxy is SHADOWS_ONLY when it casts and OFF when it does not, and OFF would let it
	# draw over the crown it stands inside, so a silent proxy has to be hidden outright.
	instance.visible = not is_proxy or instance.cast_shadow != GeometryInstance3D.SHADOW_CASTING_SETTING_OFF
	if lod == 0 and species < TreeSpecies.BUSH:
		instance.set_instance_shader_parameter("hands_over", 1.0)
	patch.add_child(instance)
	return instance

## Bounds packed as position then size from `at` in a product's bounds lanes.
func _box(lanes: PackedFloat32Array, at: int) -> AABB:
	return AABB(Vector3(lanes[at], lanes[at + 1], lanes[at + 2]),
		Vector3(lanes[at + 3], lanes[at + 4], lanes[at + 5]))

## The draw distances a product's ranges are measured with, overrides included.
func _product_settings() -> PackedFloat64Array:
	return PackedFloat64Array([density_fraction, canopy_near_m(), canopy_crossfade_begin_m(),
		canopy_far_m(), BUSH_RANGE_M, ROCK_RANGE_M])

## The mesh catalogue as the product reads it, packed once: per canopy species the impostor
## material's baked form centres and sizes (its shader moves vertices outside the quad's own
## bounds), then the bounds of each near variant's full mesh, which the engine transforms by
## every instance to bound a near batch. See SimulationNode.build_vegetation_product.
func _product_catalogue() -> PackedFloat32Array:
	if not _catalogue.is_empty():
		return _catalogue
	for species in [TreeSpecies.CONIFER, TreeSpecies.BROADLEAF]:
		var material := TreeSpecies.impostor_material(species)
		var centres: PackedVector3Array = material.get_shader_parameter("bounds_centre")
		var sizes: PackedFloat32Array = material.get_shader_parameter("bounds_size")
		_catalogue.append(sizes.size())
		for centre in centres:
			_catalogue.append_array(PackedFloat32Array([centre.x, centre.y, centre.z]))
		_catalogue.append_array(sizes)
		for variant in range(TreeSpecies.VARIANT_COUNTS[species]):
			var box: AABB = meshes[species][variant][DETAIL_FULL].get_aabb()
			_catalogue.append_array(PackedFloat32Array([box.position.x, box.position.y,
				box.position.z, box.size.x, box.size.y, box.size.z]))
	return _catalogue

## What one instance does in the shadow pass, from the patch's caster choice and whether this
## instance is the caster. The branched canopy level starts out silent; _refresh_branched_shadow
## turns it on for the patches near the camera. The visible impostor never casts. Ground cover
## never casts: it is the densest species and the key light runs four
## cascades, so each plant would be submitted up to five times while its own shadow sits under
## a canopy shadow that already darkens the same ground. Rocks and yard plants do cast: they
## stand in the open, and without a shadow they float on the lawn.
func _shadow_setting(species: int, lod: int, variant: int, is_proxy: bool, caster: int) -> int:
	if not cast_shadows:
		return GeometryInstance3D.SHADOW_CASTING_SETTING_OFF
	if is_proxy:
		return (GeometryInstance3D.SHADOW_CASTING_SETTING_SHADOWS_ONLY
			if caster != ShadowCaster.NONE else GeometryInstance3D.SHADOW_CASTING_SETTING_OFF)
	if species == TreeSpecies.ROCK or (species == TreeSpecies.BUSH
			and variant >= TreeSpecies.LANDSCAPE_FIRST_VARIANT):
		return GeometryInstance3D.SHADOW_CASTING_SETTING_ON
	return GeometryInstance3D.SHADOW_CASTING_SETTING_OFF

## Whether a patch casts. Measured from the patch's nearest corner, so a patch with any part of
## it inside the sun's range casts whole: one patch is one instance and cannot split. A caster is
## kept through BAND_HYSTERESIS_M of retreat; past the sun's range it cannot reach a cascade.
func _shadow_caster_wanted(distance: float, span: float, previous: int = ShadowCaster.NONE) -> int:
	# Deliberately independent of cast_shadows. This chooses which instances a patch holds, and
	# the runtime toggle must be able to turn casting back on without rebuilding a placement.
	var reach := distance - span * PATCH_HALF_DIAGONAL
	# With no camera every patch casts, which is what a headless caller wants.
	if distance < 0.0 or reach <= SceneLightingConfig.shadow_max_distance_m() + (
			BAND_HYSTERESIS_M if previous != ShadowCaster.NONE else 0.0):
		return ShadowCaster.IMPOSTOR
	return ShadowCaster.NONE
