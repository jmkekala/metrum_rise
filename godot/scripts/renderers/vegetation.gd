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
# Coverage over 40 m at which a tree's stand is still open, and at which it is closed. The
# same values as CANOPY_FLOOR_OPEN_COVER and CANOPY_FLOOR_CLOSED_COVER in terrain.gdshader.
const STAND_OPEN_COVER := 0.35
const STAND_CLOSED_COVER := 0.7

# How far inside its range one understory variant may stop, as a share of that range. Bush
# and rock carry a single level, so a variant that stops early drops nothing into a gap: the
# plant ends and the rest of the patch carries on. Twelve variants then end at twelve
# distances and the carpet dissolves over the last quarter of its range. Sharing one distance
# instead ends every plant of one patch on one line, and a camera above the canopy reads
# that line as a straight edge between forest floor and bare ground.
const UNDERSTORY_STAGGER_MIN := 0.75

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

## Whether a patch casts. Every canopy tree casts its impostor's silhouette, faced to the light,
## at every distance; the branched level never casts. NONE is for patches the sun's shadow range
## does not reach at all.
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
# residency follows the camera frustum, so a rotation evicts patches that are about to be
# wanted again. These are hidden rather than freed, because rebuilding one costs a frame.
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

## Draw range for one species level as (begin_m, end_m). `near_band` says whether the patch
## also carries the near per-variant instances, and `reach_m` is how far a tree of the patch can
## stand from the centre of its shared bounds, which is where Godot measures a range from.
## `variant` staggers the understory and is ignored by the canopy.
##
## The canopy ranges are outer bounds only. Each tree hands over from its branched level to its
## impostor by its own distance, in the shaders (see vegetation_wind.gdshaderinc), so a patch
## draws a level while any of its trees may still show some of it. A patch built far away
## carries no near band, and then the impostor must begin at zero: it is the only thing in the
## patch, and the rebuild that adds the near band can lag a fast approach.
func lod_range(species: int, lod: int, variant: int, near_band: bool, reach_m: float) -> Vector2:
	if TreeSpecies.is_hedge(species, variant):
		return Vector2(0.0, canopy_far_m())
	if species == TreeSpecies.BUSH:
		return Vector2(0.0, BUSH_RANGE_M * _understory_stagger(variant))
	if species == TreeSpecies.ROCK:
		return Vector2(0.0, ROCK_RANGE_M * _understory_stagger(variant))
	# Both branched levels ride the same instance, which is built as level zero whichever
	# mesh it holds, so there is no second range here for the reduced one.
	if lod == 0:
		return Vector2(0.0, canopy_near_m() + reach_m)
	# Each tree dissolves by its own distance before canopy_far_m, so the patch reaches past it.
	return Vector2(maxf(canopy_crossfade_begin_m() - reach_m, 0.0) if near_band else 0.0,
		canopy_far_m() + reach_m)

## Draw range of a patch's shadow caster: the whole patch life, because every tree casts from it.
func proxy_range() -> Vector2:
	return Vector2(0.0, canopy_far_m())

## Share of its range one understory variant keeps. A low-discrepancy sequence rather than a
## hash: twelve variants then spread evenly across the band, where twelve samples of a hash
## clump and leave the carpet ending on two or three lines instead of twelve.
func _understory_stagger(variant: int) -> float:
	return lerpf(UNDERSTORY_STAGGER_MIN, 1.0, fmod(float(variant) * 0.6180339887498949, 1.0))

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
	# O(resident patches), O(1) each, on camera/residency changes only.
	var camera_position := _camera_position()
	if residency_changed or camera_position != last_detail_camera:
		last_detail_camera = camera_position
		for key in patches:
			_refresh_near_detail(patches[key], camera_position)
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
		_upload_patch(next, _key_span(next))
		budget -= 1
		uploads += 1
		var limit := UPLOAD_BUDGET_USEC
		if not queue.is_empty() and not patches.has(queue.back()):
			limit = MISSING_UPLOAD_BUDGET_USEC
		if Time.get_ticks_usec() - upload_start >= limit:
			break
	if not standins.is_empty():
		_release_standins(camera_xz)

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

## Re-applies the shadow setting to resident patches. Avoids a full placement rebuild.
func set_cast_shadows(value: bool) -> void:
	cast_shadows = value
	for store in [patches, cache]:
		for patch in store.values():
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
	var data: PackedFloat32Array = simulation.get_decorative_tree_patch(origin, span, understory)
	# Crown coverage around the patch, averaged over 40 m, which each canopy tree samples once
	# for its stand closure.
	var cover: Dictionary = simulation.get_vegetation_stand_cover(origin, span)
	var cover_bytes: PackedByteArray = cover.get("bytes", PackedByteArray())
	var cover_size := Vector2i(int(cover.get("width", 0)), int(cover.get("height", 0)))
	var cover_bounds: Vector4 = cover.get("world_bounds", Vector4.ZERO)
	# Texel coordinates of the patch origin, and texels per metre, so a tree's texel is one
	# multiply-add from its patch-local position.
	var cover_scale := Vector2(cover_size) / Vector2(maxf(cover_bounds.z, 1.0), maxf(cover_bounds.w, 1.0))
	var cover_origin := (origin - Vector2(cover_bounds.x, cover_bounds.y)) * cover_scale - Vector2(0.5, 0.5)
	var patch := Node3D.new()
	patch.position = Vector3(origin.x, 0.0, origin.y)
	add_child(patch)
	# One transform and tint array per variant, rebuilt per species. The near level draws
	# each array as its own MultiMesh and the distant level walks them in order, so the
	# placement loop needs no slot bookkeeping, index chains or scratch arrays. Appending
	# into a local typed array is what keeps this loop cheap: routing each placement through
	# a nested untyped Array instead cost 3.8 ms per patch on a 4096-placement fixture.
	# Out of the near band there is one transform bucket, with a variant and tint per tree.
	# Work and storage remain O(placements) at upload; impostors add no per-frame CPU work.
	var count := 0
	# Tree origins of each canopy species, which is what the handover measures distance from.
	var origins: Dictionary = {}
	for species in range(TreeSpecies.SPECIES_COUNT):
		var levels: Array = meshes[species][0]
		# Bush and rock have only a near level, so out of the near band they draw nothing.
		if not near_band and levels.size() == 1:
			continue
		# Two placement loops rather than one loop with two branches in it. The branches
		# would run once per placement, and per-placement interpreted work is what this
		# function is made of: the same reason the variant index reads seed bits instead of
		# hashing. The distant loop keeps brush pins and the tint, which the impostor draws too.
		var variant_transforms: Array = []
		var variant_tints: Array = []
		var species_count := 0
		var flat_layers := PackedFloat32Array()
		var flat_tints: Array[Color] = []
		if near_band:
			var variant_count: int = TreeSpecies.VARIANT_COUNTS[species]
			variant_transforms.resize(variant_count)
			variant_tints.resize(variant_count)
			for variant in range(variant_count):
				var empty_transforms: Array[Transform3D] = []
				variant_transforms[variant] = empty_transforms
				var empty_tints: Array[Color] = []
				variant_tints[variant] = empty_tints
			for i in range(0, data.size(), 6):
				var packed := int(data[i + 5])
				if packed & SPECIES_MASK != species:
					continue
				# Stable subset for paired density experiments, independent of residency order.
				if fmod(absf(data[i] * 0.754877 + data[i + 2] * 0.56984), 1.0) >= density_fraction:
					continue
				var seed := _appearance_seed(data, i)
				var variant := _variant_index(species, seed, packed >> SPECIES_BITS)
				if TreeSpecies.is_hedge(species, variant):
					# A hedge row closes only if every module keeps its authored size, stands
					# upright and shares one colour; any per-module spread shows each metre.
					# Lane four is the ground's rise along the module, which shears its length
					# to follow the slope while its sides stay vertical.
					var yaw := data[i + 3]
					variant_transforms[variant].append(Transform3D(
						Basis(Vector3(cos(yaw), data[i + 4], -sin(yaw)), Vector3.UP,
							Vector3(sin(yaw), 0.0, cos(yaw))),
						Vector3(data[i] - origin.x, data[i + 1], data[i + 2] - origin.y)))
					variant_tints[variant].append(Color.WHITE)
				else:
					variant_transforms[variant].append(_instance_transform(data, i, seed, origin))
					variant_tints[variant].append(_instance_tint(seed))
				species_count += 1
		else:
			var flat: Array[Transform3D] = []
			for i in range(0, data.size(), 6):
				var packed := int(data[i + 5])
				if packed & SPECIES_MASK != species:
					continue
				if fmod(absf(data[i] * 0.754877 + data[i + 2] * 0.56984), 1.0) >= density_fraction:
					continue
				var seed := _appearance_seed(data, i)
				var variant := _variant_index(species, seed, packed >> SPECIES_BITS)
				flat.append(_instance_transform(data, i, seed, origin))
				flat_layers.append(variant)
				flat_tints.append(_instance_tint(seed))
			species_count = flat.size()
			variant_transforms.append(flat)
		if species_count == 0:
			continue
		count += species_count
		if near_band:
			for variant in range(variant_transforms.size()):
				var near_transforms: Array[Transform3D] = variant_transforms[variant]
				if near_transforms.is_empty():
					continue
				var near_tints: Array[Color] = variant_tints[variant]
				var near_mm := MultiMesh.new()
				near_mm.transform_format = MultiMesh.TRANSFORM_3D
				near_mm.use_colors = true
				# The canopy's level is chosen below, once the patch knows where its trees stand.
				near_mm.mesh = meshes[species][variant][DETAIL_FULL]
				var canopy := levels.size() > 1
				near_mm.use_custom_data = canopy
				near_mm.instance_count = near_transforms.size()
				for i in range(near_transforms.size()):
					near_mm.set_instance_transform(i, near_transforms[i])
					near_mm.set_instance_color(i, near_tints[i])
					if canopy:
						near_mm.set_instance_custom_data(i, Color(
							_stand_closure(cover_bytes, cover_size,
							cover_origin + Vector2(near_transforms[i].origin.x, near_transforms[i].origin.z) * cover_scale), 0.0, 0.0, 0.0))
				_add_instance(patch, near_mm, species, 0, variant, caster)
		# One buffer assignment per species, with transform rows followed by custom RGBA.
		# Near buckets already encode the variant; far-only placements supply their own.
		if levels.size() > 1:
			var far_mm := MultiMesh.new()
			far_mm.transform_format = MultiMesh.TRANSFORM_3D
			far_mm.use_custom_data = true
			far_mm.mesh = TreeSpecies.impostor_mesh()
			far_mm.instance_count = species_count
			var buffer := PackedFloat32Array()
			buffer.resize(species_count * 16)
			# The shader moves vertices outside the QuadMesh bounds. Include every baked
			# form before transforming, independent of the rendering backend.
			var object_bounds := AABB()
			var material := TreeSpecies.impostor_material(species)
			var centres: PackedVector3Array = material.get_shader_parameter("bounds_centre")
			var sizes: PackedFloat32Array = material.get_shader_parameter("bounds_size")
			for layer in range(sizes.size()):
				# A rotated square's corner reaches size / sqrt(2) from its centre.
				var radius := sizes[layer] * PATCH_HALF_DIAGONAL
				object_bounds = object_bounds.merge(AABB(centres[layer] - Vector3.ONE * radius,
					Vector3.ONE * radius * 2.0))
			# Per tree, only the origin span and the largest axis scale: transforming the box
			# for every tree was the dearest line of this loop. The box then grows by its
			# farthest corner at that scale, which contains every tree's box.
			var reach := 0.0
			for corner in range(8):
				reach = maxf(reach, object_bounds.get_endpoint(corner).length())
			var low := Vector3.INF
			var high := -Vector3.INF
			var scale_squared := 0.0
			var slot := 0
			for variant in range(variant_transforms.size()):
				var far_transforms: Array[Transform3D] = variant_transforms[variant]
				# The near buckets are the variants themselves; out of the band one bucket holds
				# every tree with its variant and tint alongside.
				var far_tints: Array[Color] = variant_tints[variant] if near_band else flat_tints
				for i in range(far_transforms.size()):
					var placed := far_transforms[i]
					# The layer lane carries the stand closure below one half; see impostor shaders.
					_write_impostor(buffer, slot * 16, placed,
						(float(variant) if near_band else flat_layers[i])
						+ 0.5 * _stand_closure(cover_bytes, cover_size,
						cover_origin + Vector2(placed.origin.x, placed.origin.z) * cover_scale), far_tints[i])
					low = low.min(placed.origin)
					high = high.max(placed.origin)
					scale_squared = maxf(scale_squared,
						maxf(placed.basis.x.length_squared(), placed.basis.y.length_squared()))
					slot += 1
			far_mm.buffer = buffer
			origins[species] = AABB(low, high - low)
			var bounds := AABB(low, high - low).grow(reach * sqrt(scale_squared))
			far_mm.custom_aabb = bounds
			_add_instance(patch, far_mm, species, 2, 0, caster)
			# The shadow caster: every tree's impostor, faced to the light.
			if caster != ShadowCaster.NONE:
				# The same buffer: the caster reads each tree's form from the custom lane.
				# Packed arrays are copy-on-write, so this shares the data instead of copying it.
				var proxy := MultiMesh.new()
				proxy.transform_format = MultiMesh.TRANSFORM_3D
				proxy.use_custom_data = true
				proxy.mesh = TreeSpecies.impostor_mesh()
				proxy.instance_count = species_count
				proxy.buffer = buffer
				proxy.custom_aabb = bounds
				_add_instance(patch, proxy, species, 2, 0, caster, true)
	if not near_band:
		count += _add_hedge_runs(patch, origin, span, caster)
	_share_patch_bounds(patch, near_band, caster, origins)
	patch.set_meta("tree_count", count)
	patch.set_meta("surface_generation", generation)
	patch.set_meta("vegetation_generation", vegetation_generation)
	patch.set_meta("understory", understory)
	patch.set_meta("near_band", near_band)
	patch.set_meta("shadow_caster", caster)
	# Every canopy tree origin of the patch in world space, which the detail choice measures.
	var spreads := origins.values()
	var canopy_box: AABB = spreads[0] if not spreads.is_empty() else AABB()
	for spread in spreads:
		canopy_box = canopy_box.merge(spread)
	canopy_box.position += patch.position
	patch.set_meta("canopy_box", canopy_box)
	patch.set_meta("near_detail_lod", DETAIL_FULL)
	_refresh_near_detail(patch, _camera_position())
	# Replace in place. The previous node stays visible until this one is in the tree,
	# so an edit does not blank the surrounding forest for a frame.
	if patches.has(key):
		tree_count -= int(patches[key].get_meta("tree_count"))
		patches[key].queue_free()
	patches[key] = patch
	tree_count += count
	generated_patches += 1
	generation_ms_max = maxf(generation_ms_max, float(Time.get_ticks_usec() - start) / 1000.0)

# Godot's 3D MultiMesh buffer stores three ROWS, each ending with one origin component.
# Packed arrays are passed by reference; this writes the preallocated buffer without a copy.
# The custom lane is the texture layer, which is the near variant, then the near tint.
## Share of a closed stand around a tree at texel coordinate `at`, from the crown coverage
## averaged over 40 m. Bilinear, and gated as the forest floor is: a lone crown or a small clump
## stays open, the inside of a stand reaches one. O(1).
func _stand_closure(bytes: PackedByteArray, size: Vector2i, at: Vector2) -> float:
	if bytes.is_empty():
		return 0.0
	var x0 := clampi(floori(at.x), 0, size.x - 1)
	var z0 := clampi(floori(at.y), 0, size.y - 1)
	var x1 := mini(x0 + 1, size.x - 1)
	var z1 := mini(z0 + 1, size.y - 1)
	var fx := clampf(at.x - floorf(at.x), 0.0, 1.0)
	var fz := clampf(at.y - floorf(at.y), 0.0, 1.0)
	var c := lerpf(lerpf(bytes[z0 * size.x + x0], bytes[z0 * size.x + x1], fx),
		lerpf(bytes[z1 * size.x + x0], bytes[z1 * size.x + x1], fx), fz) / 255.0
	return c * smoothstep(STAND_OPEN_COVER, STAND_CLOSED_COVER, c)

func _write_impostor(buffer: PackedFloat32Array, offset: int, placed: Transform3D, layer: float,
	tint: Color) -> void:
	buffer[offset] = placed.basis.x.x
	buffer[offset + 1] = placed.basis.y.x
	buffer[offset + 2] = placed.basis.z.x
	buffer[offset + 3] = placed.origin.x
	buffer[offset + 4] = placed.basis.x.y
	buffer[offset + 5] = placed.basis.y.y
	buffer[offset + 6] = placed.basis.z.y
	buffer[offset + 7] = placed.origin.y
	buffer[offset + 8] = placed.basis.x.z
	buffer[offset + 9] = placed.basis.y.z
	buffer[offset + 10] = placed.basis.z.z
	buffer[offset + 11] = placed.origin.z
	buffer[offset + 12] = layer
	buffer[offset + 13] = tint.r
	buffer[offset + 14] = tint.g
	buffer[offset + 15] = tint.b

## Gives every level in a patch one set of bounds, so they all change level on one distance.
## Godot measures a visibility range from the instance bounds, not from the node origin: a
## probe put two MultiMeshInstance3D at the camera's own position and culled the one whose
## single instance sat past the range. Each near level holds one variant and the distant level
## holds the whole species, so their bounds centres stand about 12 m apart on a random scatter
## and up to 67 m apart. The two ranges are complementary by construction and were not
## complementary in practice: over the metres between the two centres the distant level had
## already stopped and the near level had not yet started, and the trees of that variant were
## drawn by neither. Roughly half the variants sit on the losing side of that, which is why the
## canopy thins as the camera closes on the switch and fills back in a moment later.
##
## The pair is one species' near levels and that species' own distant level, so the merge is
## per species. A bush must not pull a 25 m canopy box into the box its own 420 m range is
## measured from, and a species with one level has no partner to fall between: it keeps its
## own bounds, which is also what leaves the understory stagger in `lod_range` an effect.
##
## One pass over the patch's own children, at upload, and nothing per frame. The dummy renderer
## reports empty automatic bounds; explicit impostor bounds also work headlessly.
func _share_patch_bounds(patch: Node3D, near_band: bool, caster: int, origins: Dictionary) -> void:
	var bounds: Dictionary = {}
	var levels: Dictionary = {}
	for instance in patch.get_children():
		var species: int = instance.get_meta("species")
		var box: AABB = instance.multimesh.custom_aabb
		if box.size == Vector3.ZERO:
			box = instance.get_aabb()
		bounds[species] = box if not bounds.has(species) else (bounds[species] as AABB).merge(box)
		var seen: Dictionary = levels.get(species, {})
		seen[int(instance.get_meta("lod"))] = true
		levels[species] = seen
	for instance in patch.get_children():
		var species: int = instance.get_meta("species")
		var box: AABB = bounds[species]
		if box.size != Vector3.ZERO and ((levels[species] as Dictionary).size() > 1
			or instance.multimesh.use_custom_data):
			instance.custom_aabb = box
		# How far a tree origin stands from the box centre the range is measured from: its
		# farthest corner. The box is padded for crowns and quads, which the handover ignores.
		var reach := 0.0
		if origins.has(species):
			var spread: AABB = origins[species]
			for corner in range(8):
				reach = maxf(reach, spread.get_endpoint(corner).distance_to(box.get_center()))
		var range_m: Vector2 = (proxy_range() if instance.get_meta("shadow_proxy")
			else lod_range(species, int(instance.get_meta("lod")), int(instance.get_meta("variant")),
				near_band, reach))
		# No fade mode. VISIBILITY_RANGE_FADE_SELF alpha-blends the whole instance, which is a
		# whole patch, and moves it out of the depth pre-pass. The canopy hands over per tree in
		# its shaders instead, and these ranges only bound the patches that take part.
		instance.visibility_range_begin = range_m.x
		instance.visibility_range_end = range_m.y

## Draws the patch's hedges as straight boxes, one per run Rust merged from the modules, so a
## yard keeps its hedge out to the trees' far range for a dozen triangles a run. Returns the
## number of runs. O(runs) here; the merge is Rust's.
func _add_hedge_runs(patch: Node3D, origin: Vector2, span: float, caster: int) -> int:
	var runs: PackedFloat32Array = simulation.get_vegetation_hedge_runs(origin, span)
	var per_hedge: Array = [[], [], []]
	for i in range(0, runs.size(), 7):
		var yaw := runs[i + 3]
		var length := runs[i + 4]
		var size: Vector2 = TreeSpecies.HEDGE_RUN_SIZES[int(runs[i + 6])]
		# Stretched along the run, sheared up its slope with the sides kept vertical, and sunk
		# into the lawn like a module.
		(per_hedge[int(runs[i + 6])] as Array).append(Transform3D(
			Basis(Vector3(cos(yaw) * length, runs[i + 5], -sin(yaw) * length), Vector3.UP,
				Vector3(sin(yaw), 0.0, cos(yaw))),
			Vector3(runs[i] - origin.x, runs[i + 1] - 0.05 + size.x * 0.5, runs[i + 2] - origin.y)))
	for hedge in per_hedge.size():
		var placed: Array = per_hedge[hedge]
		if placed.is_empty():
			continue
		var mm := MultiMesh.new()
		mm.transform_format = MultiMesh.TRANSFORM_3D
		mm.mesh = TreeSpecies.hedge_run_mesh(hedge)
		mm.instance_count = placed.size()
		for i in placed.size():
			mm.set_instance_transform(i, placed[i])
		_add_instance(patch, mm, TreeSpecies.BUSH, HEDGE_RUN_LOD,
			TreeSpecies.HEDGE_FIRST_VARIANT + hedge, caster)
	return runs.size() / 7

func _add_instance(
	patch: Node3D, mm: MultiMesh, species: int, lod: int, variant: int,
	caster: int, is_proxy: bool = false
) -> void:
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

## Near mesh choice. A brush that plants one named tree passes a `pin` biased by one and that
## mesh is used; zero falls back to the seed's own bits, so the choice stays independent of
## proportions and lean. Bit extraction rather than another hash: the seed is already mixed,
## and one more hash call in the per-placement path is worth about a millisecond per patch on
## its own.
func _variant_index(species: int, appearance_seed: int, pin: int) -> int:
	if pin > 0:
		return pin - 1
	return ((appearance_seed >> 11) & 0xFFFF) % int(TreeSpecies.SEED_VARIANT_COUNTS[species])

## A small value spread and opposing red/blue shifts keep the foliage on its green axis.
## StandardMaterial3D multiplies instance and vertex colours, preserving per-face shade.
## Disjoint bit ranges of the already-mixed seed, for the reason given on _variant_index:
## the two hash calls this replaces cost 1.2 ms per patch on a 4096-placement fixture.
func _instance_tint(appearance_seed: int) -> Color:
	var value := lerpf(0.82, 1.18, float((appearance_seed >> 3) & 0xFFF) / 4096.0)
	var warmth := lerpf(-0.10, 0.10, float((appearance_seed >> 27) & 0xFFF) / 4096.0)
	return Color(value * (1.0 + warmth), value, value * (1.0 - warmth), 1.0)

## Default cosmetic seed from packed placement; never feeds simulation state. Keyed on the world
## position Rust packs, not on one relative to the patch: one plant is drawn by a fine patch near
## the camera and by a coarse one farther out, and a patch-relative key gave it a different form,
## size and tint in each, so a whole terrain patch of trees changed as it crossed the fine radius.
func _appearance_seed(data: PackedFloat32Array, i: int) -> int:
	return hash(Vector2(data[i], data[i + 2]))

## Per-instance appearance. Yaw alone is nearly invisible on a near-symmetric lathe, so the
## variation that reads is the height/width ratio and a small lean off vertical.
## A caller can reroll those proportions with a different seed at the same placement.
## `origin` is the patch origin the transform is made relative to.
func _instance_transform(
	data: PackedFloat32Array, i: int, appearance_seed: int, origin: Vector2
) -> Transform3D:
	var x := data[i] - origin.x
	var z := data[i + 2] - origin.y
	var scale := data[i + 4]
	# The four jitters below are the old _jitter() helper written out. Each of them ran once
	# per placement, and the call alone cost 2.6 ms per patch on a 4096-placement fixture --
	# more than the hashing it wrapped. The values are unchanged.
	var height := scale * (0.82 + float(hash(appearance_seed ^ 17) & 0xFFFFFF) / 16777216.0 * 0.42)
	var width := scale * (0.80 + float(hash(appearance_seed ^ 53) & 0xFFFFFF) / 16777216.0 * 0.48)
	# A boulder's lean is its tilt in the ground, so rocks take it as well as plants.
	var lean := 0.02 + float(hash(appearance_seed ^ 91) & 0xFFFFFF) / 16777216.0 * 0.07
	var lean_direction := float(hash(appearance_seed ^ 137) & 0xFFFFFF) / 16777216.0 * TAU
	var basis := (
		Basis(Vector3.UP, data[i + 3])
		* Basis(Vector3(cos(lean_direction), 0.0, sin(lean_direction)), lean)
		* Basis.from_scale(Vector3(width, height, width))
	)
	return Transform3D(basis, Vector3(x, data[i + 1], z))

## What one instance does in the shadow pass, from the patch's caster choice and whether this
## instance is the caster. The branched and impostor levels never cast: the caster stands in
## for both. Ground cover never casts: it is the densest species and the key light runs four
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
