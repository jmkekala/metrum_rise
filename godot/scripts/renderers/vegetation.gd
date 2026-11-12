# SPDX-License-Identifier: GPL-2.0-only

## Vegetation presentation reuses terrain residency; Rust owns the generated and edited state.
## Rust supplies deterministic patch-local placements for four species: conifer, broadleaf,
## bush and rock. A patch is requested by key and Rust builds its product on the background pool
## from a snapshot (`RENDER-32-D`); the stream node installs finished products through pooled
## `RenderingServer` objects under the frame's streaming budget and reports each install back
## (`RENDER-32-E`). This script keeps the mesh and material catalogue, the wanted set, the bands
## and the staleness; it creates no node per patch.
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
# The near levels a canopy variant registers, in order: the mesh each near variant draws (the
# stream node chooses per patch).
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
# Lanes of one install event from the stream node: key x, y, z, serial, plants drawn (-1: the
# core was busy, ask again), install microseconds.
const INSTALL_EVENT_LANES := 6

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
# Installed patches drawn, by key: the record of what each was built with (tree_count, both
# generations, understory, near_band, shadow_caster). The stream node holds what they draw.
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
# Rust's vegetation patch revisions that are not zero, by owner patch, as of the land cover
# epoch `_generations_epoch`; read without waiting for the core (`_refresh_generations`).
var _vegetation_generations: Dictionary = {}
var _generations_epoch := -1
var stale_surface_revision := -1
var last_band_camera_xz := Vector2.INF
# meshes[species][variant] is an Array[ArrayMesh], ordered near to far. A canopy variant holds
# the full level, the reduced level, and both in one mesh; see TREE_NEAR_DETAIL_M. The stream
# node draws them by RID (_register_catalogue).
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
# Patches requested from Rust and not yet installed, by key: the request's serial and what the
# install stamps on the patch (both generations as read before the request, and the bands).
var requested: Dictionary = {}
# The longest request this frame, microseconds, and the products that came back busy (their
# worker never got the core).
var _request_max_us := 0
var _frame_busy := 0
# The mesh catalogue packed for the product; see _product_catalogue.
var _catalogue := PackedFloat32Array()
# Whether Rust holds the catalogue the background products are built against.
var _catalogue_sent := false
# The world's size, see _world_size.
var _world := Vector2.ZERO
@onready var terrain = $"../Terrain"
# What the stream node was last told: shown, perf channel on, fresh buffers (motion vectors).
var _stream_enabled := true
var _stream_perf := false
var _stream_fresh := false
# The water reflection's world, handed to the stream node once the catalogue is registered.
var _reflection_scenario := RID()
var _catalogue_registered := false
@onready var simulation = $"../SimulationNode"
@onready var stream = $"../StreamNode"

func _ready() -> void:
	meshes = TreeSpecies.build_meshes()
	# Texture IO and array creation belong to catalogue setup, outside patch uploads.
	TreeSpecies.impostor_mesh()
	for species in [TreeSpecies.CONIFER, TreeSpecies.BROADLEAF]:
		TreeSpecies.impostor_material(species)
	TreeSpecies.set_crossfade(canopy_near_m())
	TreeSpecies.set_detail(near_detail_m())
	TreeSpecies.set_far(canopy_far_m())
	if simulation != null and simulation.is_class("SimulationNode"):
		stream.attach_simulation(simulation)
	_register_catalogue()

## Hands the stream node what it draws patches with, by RID (the scripts keep the resources
## alive): every near mesh, three levels a variant (a one-level species repeats its own), then the
## impostor quad and the distant hedge boxes; the impostor materials and their shadow casters;
## and the draw rules it applies. Again after `meshes` changes; installed patches keep theirs.
func _register_catalogue() -> void:
	var mesh_rids: Array[RID] = []
	for species in range(TreeSpecies.SPECIES_COUNT):
		for variant in range(TreeSpecies.VARIANT_COUNTS[species]):
			var levels: Array = meshes[species][variant]
			for level in range(3):
				mesh_rids.append((levels[mini(level, levels.size() - 1)] as Mesh).get_rid())
	mesh_rids.append(TreeSpecies.impostor_mesh().get_rid())
	for hedge in range(TreeSpecies.HEDGE_VARIANTS):
		mesh_rids.append(TreeSpecies.hedge_run_mesh(hedge).get_rid())
	var material_rids: Array[RID] = [
		TreeSpecies.impostor_material(TreeSpecies.CONIFER).get_rid(),
		TreeSpecies.impostor_material(TreeSpecies.BROADLEAF).get_rid(),
		TreeSpecies.impostor_shadow_material(TreeSpecies.CONIFER).get_rid(),
		TreeSpecies.impostor_shadow_material(TreeSpecies.BROADLEAF).get_rid(),
	]
	_catalogue_registered = stream.set_vegetation_catalogue(get_world_3d().scenario, mesh_rids,
		material_rids, PackedFloat64Array([near_detail_m(), TreeSpecies.TREE_DETAIL_FADE_M,
			TreeSpecies.TREE_SHADOW_HANDOVER_M, TreeSpecies.LANDSCAPE_FIRST_VARIANT,
			TreeSpecies.HEDGE_FIRST_VARIANT]))
	if _catalogue_registered:
		stream.set_vegetation_reflection_scenario(_reflection_scenario)
		stream.set_vegetation_cast_shadows(cast_shadows)

## The water reflection's world, where the stream node also draws the impostors
## (`water_reflection.gd`); an invalid RID when it goes away.
func set_reflection_scenario(scenario: RID) -> void:
	_reflection_scenario = scenario
	if _catalogue_registered:
		stream.set_vegetation_reflection_scenario(scenario)

## Tells the stream node what changed of the layer's switches: shown (`enabled`), the perf channel,
## and whether the viewport draws motion vectors (reused slots then take fresh storage).
func _sync_stream() -> void:
	if enabled != _stream_enabled:
		_stream_enabled = enabled
		stream.set_vegetation_enabled(enabled)
	var perf := PerfDebug.is_enabled()
	if perf != _stream_perf:
		_stream_perf = perf
		stream.set_perf_enabled(perf)
	var viewport := get_viewport()
	var fresh := viewport != null and (viewport.use_taa
		or viewport.scaling_3d_mode == Viewport.SCALING_3D_MODE_FSR2)
	if fresh != _stream_fresh:
		_stream_fresh = fresh
		stream.set_vegetation_fresh_buffers(fresh)

## The world's size, read from Rust once per loaded world: it changes only with a load, which
## rebuilds the renderer, and the read waits on the render snapshot's lock while the simulation
## publishes a frame (`RENDER-32-D`: no wait on the vegetation's frame path).
func _world_size() -> Vector2:
	if _world == Vector2.ZERO:
		_world = simulation.get_terrain_world_size()
	return _world

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
	# The detail distance and the catalogue may have changed (probes swap both).
	_register_catalogue()
	stream.clear_vegetation()
	patches.clear()
	cache.clear()
	standins.clear()
	wanted_keys.clear()
	queue.clear()
	queued.clear()
	requested.clear()
	_world = Vector2.ZERO
	simulation.clear_vegetation_requests()
	stale_land_epoch = -1
	_vegetation_generations.clear()
	_generations_epoch = -1
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
	_sync_stream()
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
	# The vegetation revisions once a frame, before anything reads them through the frame's
	# memo, without waiting for the core (`RENDER-32-D`).
	var generations_fresh := _refresh_generations()
	var camera_xz := Vector2(camera.global_position.x, camera.global_position.z)
	var camera_cell := Vector2i((camera_xz / span).floor())
	var start_us := Time.get_ticks_usec()
	_request_max_us = 0
	_frame_busy = 0
	# What the stream node installed since the last frame, before anything reads `patches`.
	var installs := _take_installs()
	var revision: int = terrain.get_resident_patch_revision()
	# O(resident patches) only on residency/camera-cell changes, not O(total scatter).
	var residency_changed := revision != last_revision or camera_cell != last_camera_cell
	if residency_changed:
		last_revision = revision
		last_camera_cell = camera_cell
		var wanted: Dictionary = {}
		var world: Vector2 = _world_size()
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
				var outgoing: Dictionary = patches[key]
				tree_count -= int(outgoing["tree_count"])
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
				stream.free_vegetation_patch(key)
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
		# A request for a patch no longer wanted is dropped: its product will not be delivered.
		for key in requested.keys():
			if not wanted.has(key):
				simulation.cancel_vegetation_patch(key)
				stream.forget_vegetation_patch(key)
				requested.erase(key)
		for key in wanted:
			if patches.has(key):
				continue
			if standins.has(key):
				patches[key] = standins[key]
				standins.erase(key)
				tree_count += int(patches[key]["tree_count"])
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
		var world: Vector2 = _world_size()
		for key in patches:
			var patch: Dictionary = patches[key]
			var distance := _patch_distance(key, world, camera_xz)
			if _bands_changed(patch, distance, _key_span(key)):
				_queue_patch(key)
	# The near canopy's detail level and branched shadow follow the camera in the stream node.
	var bands_us := Time.get_ticks_usec()
	# O(1) while no revision moved. A patch is stale only when its terrain owner committed a
	# newer surface or Rust advanced its vegetation revision, and each of those moves one of
	# these two counters, so the O(resident patches) sweep runs on the frame an edit lands.
	# It runs even while uploads are pending, so a long upload queue cannot hide an edit.
	# The revisions are read without waiting for the core: while the simulation holds it, the
	# sweep waits for a frame that can read them (`RENDER-32-D`).
	var surface_revision: int = terrain.get_surface_commit_revision()
	if generations_fresh and (_generations_epoch != stale_land_epoch
			or surface_revision != stale_surface_revision):
		stale_land_epoch = _generations_epoch
		stale_surface_revision = surface_revision
		for key in patches:
			if _is_patch_stale(key, owner_generations):
				_queue_patch(key)
	# Requests up to the area/count limit a frame, most needed first; a patch already in flight
	# with the same bands and generations is not asked again. Rust refuses a request while its
	# jobs are full, and the patch then stays queued for the next frame.
	var budget := upload_budget_override if upload_budget_override >= 1 else divisor * divisor
	var upload_start := Time.get_ticks_usec()
	var requests := 0
	var world: Vector2 = _world_size()
	while budget > 0 and not queue.is_empty():
		var next: Vector3i = queue.pop_back()
		queued.erase(next)
		var distance := _patch_distance(next, world, camera_xz)
		if patches.has(next):
			if not _is_patch_stale(next) and not _bands_changed(patches[next], distance, _key_span(next)):
				continue
		if requested.has(next) and not _request_changed(next, distance):
			continue
		var one_start := Time.get_ticks_usec()
		if not _upload_patch(next, _key_span(next)):
			_queue_patch(next)
			break
		_request_max_us = maxi(_request_max_us, Time.get_ticks_usec() - one_start)
		budget -= 1
		requests += 1
	var uploaded_us := Time.get_ticks_usec()
	if not standins.is_empty():
		_release_standins(camera_xz)
	if PerfDebug.is_enabled():
		# The stream node records its own share of the frame under the same name before the draw
		# (installs, `upload_max_ms`, `poll_ms`); the benchmark sums the two.
		var ms := func(from_us: int, to_us: int) -> float: return float(to_us - from_us) / 1000.0
		var end_us := Time.get_ticks_usec()
		PerfDebug.record("vegetation", ms.call(start_us, end_us), {
			"residency_ms": ms.call(start_us, residency_us), "bands_ms": ms.call(residency_us, bands_us),
			"stale_ms": ms.call(bands_us, upload_start), "requests_ms": ms.call(upload_start, uploaded_us),
			"request_max_ms": float(_request_max_us) / 1000.0, "standins_ms": ms.call(uploaded_us, end_us),
			"uploads": installs, "busy": _frame_busy, "requests": requests, "requested": requested.size(),
			"residency_changed": 1 if residency_changed else 0,
			"queue": queue.size(), "patches": patches.size(), "wanted": wanted_keys.size()})

## Takes the stream node's install events: an installed patch becomes drawn with the bands and
## generations of its request (and is queued again when the camera or an edit moved since); a
## busy one is queued again. Returns the installs. O(installs).
func _take_installs() -> int:
	var events: PackedInt64Array = stream.take_vegetation_installs()
	var installs := 0
	for at in range(0, events.size(), INSTALL_EVENT_LANES):
		var key := Vector3i(events[at], events[at + 1], events[at + 2])
		var request: Dictionary = requested.get(key, {})
		if request.is_empty() or int(request["serial"]) != events[at + 3]:
			# The stream node installs live requests only; a mismatch is a bookkeeping fault.
			push_error("vegetation: install of %s serial %d without its request" % [key, events[at + 3]])
			continue
		requested.erase(key)
		var count: int = events[at + 4]
		if count < 0:
			_frame_busy += 1
			_queue_patch(key)
			continue
		if patches.has(key):
			tree_count -= int(patches[key]["tree_count"])
		patches[key] = {"tree_count": count, "surface_generation": request["generation"],
			"vegetation_generation": request["vegetation_generation"],
			"understory": request["understory"], "near_band": request["near_band"],
			"shadow_caster": request["shadow_caster"]}
		tree_count += count
		generated_patches += 1
		generation_ms_max = maxf(generation_ms_max, float(events[at + 5]) / 1000.0)
		installs += 1
		# The camera or an edit may have moved since the request: a patch whose bands or generations
		# no longer match is asked for again.
		var camera_xz := _camera_xz()
		if camera_xz != Vector2.INF:
			var distance := _patch_distance(key, _world_size(), camera_xz)
			if _is_patch_stale(key) or _bands_changed(patches[key], distance, _key_span(key)):
				_queue_patch(key)
	return installs

## Puts away every stand-in whose area the wanted patches now cover. O(stand-ins x the
## sub-patches of one terrain patch), and stand-ins exist only while a tier change uploads.
func _release_standins(camera_xz: Vector2) -> void:
	var world: Vector2 = _world_size()
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

func _bands_changed(patch: Dictionary, distance: float, span: float) -> bool:
	var understory := bool(patch["understory"])
	var near_band := bool(patch["near_band"])
	var caster := int(patch["shadow_caster"])
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
		(generations.x >= 0 and generations.x != int(patches[key]["surface_generation"]))
		or generations.y != int(patches[key]["vegetation_generation"])
	)

## The surface and vegetation generations of an owner patch, as (surface, vegetation). With a
## cache this asks the terrain once per owner instead of once per caller; without one it first
## brings the vegetation revisions up to date when the core is free, because the answer is only
## about the moment it is asked.
func _owner_generations(owner: Vector2i, cache) -> Vector2i:
	if cache != null and cache.has(owner):
		return cache[owner]
	if cache == null:
		_refresh_generations()
	var generations := Vector2i(
		terrain.get_patch_surface_generation(owner),
		int(_vegetation_generations.get(owner, 0))
	)
	if cache != null:
		cache[owner] = generations
	return generations

## Brings `_vegetation_generations` to Rust's current revisions without waiting for the core:
## one try-lock read, which copies the table only when the land cover epoch moved. Returns false
## while the simulation holds the core; the table then keeps its older revisions, which can only
## make a patch look stale, never current.
func _refresh_generations() -> bool:
	var table: PackedInt64Array = simulation.try_get_vegetation_generations(_generations_epoch)
	if table.is_empty():
		return false
	if table[0] != _generations_epoch:
		_vegetation_generations.clear()
		for at in range(1, table.size(), 3):
			_vegetation_generations[Vector2i(table[at], table[at + 1])] = table[at + 2]
		_generations_epoch = table[0]
	return true

## Camera position on the ground plane, or `INF` when there is no camera to measure from.
func _camera_xz() -> Vector2:
	var camera := get_viewport().get_camera_3d()
	if camera == null:
		return Vector2.INF
	return Vector2(camera.global_position.x, camera.global_position.z)

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

## Re-applies the shadow setting to installed patches (the stream node). Avoids a full placement
## rebuild.
func set_cast_shadows(value: bool) -> void:
	cast_shadows = value
	stream.set_vegetation_cast_shadows(value)

## Takes a patch out of the drawn set. A patch still inside the scatter radius is hidden and
## kept, because a camera moving back will very likely ask for it again. Only a patch that is
## genuinely out of range is freed.
func _retire_patch(key: Vector3i, world: Vector2, camera_xz: Vector2) -> void:
	var patch: Dictionary = patches[key]
	tree_count -= int(patch["tree_count"])
	patches.erase(key)
	_put_away(key, patch, world, camera_xz)

## Hides a patch that left the drawn set into the cache, or frees it when out of range.
func _put_away(key: Vector3i, patch: Dictionary, world: Vector2, camera_xz: Vector2) -> void:
	if _patch_distance(key, world, camera_xz) <= canopy_far_m() + _key_span(key):
		stream.set_vegetation_patch_visible(key, false)
		cache[key] = patch
		return
	stream.free_vegetation_patch(key)

## Returns a hidden patch to the drawn set. The caller checks generations and bands in the
## same frame. Retained band content remains valid inside its hysteresis interval.
func _restore_patch(key: Vector3i) -> void:
	var patch: Dictionary = cache[key]
	cache.erase(key)
	stream.set_vegetation_patch_visible(key, true)
	patches[key] = patch
	tree_count += int(patch["tree_count"])

func has_pending_work() -> bool:
	return enabled and (last_revision == -1 or not queue.is_empty() or not requested.is_empty())

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

## Requests a patch's product from Rust with the bands its distance asks for. Returns false when
## Rust refuses for now (its jobs are full), and the caller keeps the patch queued.
func _upload_patch(key: Vector3i, span: float) -> bool:
	# Read before the request. Stamping an older generation onto newer data only causes one
	# redundant rebuild; the reverse would leave a stale patch undetected. The order matters
	# here: this value is stamped onto the patch, so it must not be newer than the inputs Rust's
	# worker takes after the request. The revisions are refreshed without waiting for the core;
	# when it is busy they are older, which costs at most that one rebuild.
	var owner := _owner_key(key)
	var generations := _owner_generations(owner, null)
	var generation: int = generations.x
	var vegetation_generation: int = generations.y
	var world: Vector2 = _world_size()
	patch_span_m = span
	# A direct caller can name a span the key's own grid does not imply, so the key is made
	# to agree with it before any distance is measured from it.
	terrain_span_m = span * float(maxi(key.z, 1))
	var bands := _bands_for(key, _patch_distance(key, world, _camera_xz()))
	var origin := Vector2(key.x, key.y) * span - world * 0.5
	if not _catalogue_sent:
		_catalogue_sent = simulation.set_vegetation_product_catalogue(_product_catalogue())
	# Every buffer, bounds and range of the patch, built in Rust on the background pool from a
	# snapshot (`RENDER-32-D`): the placements, the crown coverage around the patch (each canopy
	# tree's stand closure), the distant hedge runs outside the near band, then the product
	# (`RENDER-32-C`). The stream node installs it (`RENDER-32-E`); a product of an older request
	# still waiting there is dropped first.
	stream.forget_vegetation_patch(key)
	var serial: int = simulation.request_vegetation_patch(key, origin, span, bands[0], bands[1],
		bands[2] != ShadowCaster.NONE, _product_settings())
	if serial < 0:
		return false
	requested[key] = {"serial": serial, "generation": generation,
		"vegetation_generation": vegetation_generation, "understory": bands[0],
		"near_band": bands[1], "shadow_caster": bands[2], "origin": origin}
	return true

## The bands a patch at `distance` wants: understory, near band and shadow caster, each keeping
## the patch's current band through its hysteresis.
func _bands_for(key: Vector3i, distance: float) -> Array:
	var span := _key_span(key)
	var previous: Dictionary = patches.get(key, {})
	var has := not previous.is_empty()
	return [
		_understory_wanted(distance, span, has and bool(previous["understory"])),
		_near_band_wanted(distance, span, has and bool(previous["near_band"])),
		_shadow_caster_wanted(distance, span,
			int(previous["shadow_caster"]) if has else ShadowCaster.NONE),
	]

## Whether a patch in flight must be asked again: its owner's generations or its bands moved
## since the request.
func _request_changed(key: Vector3i, distance: float) -> bool:
	var request: Dictionary = requested[key]
	var generations := _owner_generations(_owner_key(key), null)
	var bands := _bands_for(key, distance)
	return (generations.x != int(request["generation"])
		or generations.y != int(request["vegetation_generation"])
		or bands[0] != bool(request["understory"]) or bands[1] != bool(request["near_band"])
		or bands[2] != int(request["shadow_caster"]))

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
