# SPDX-License-Identifier: GPL-2.0-only

## Vegetation presentation reuses terrain residency; Rust owns the generated and edited state.
## Rust supplies deterministic patch-local placements for four species: conifer, broadleaf,
## bush and rock. Uploads at most one patch per frame.
## A patch regenerates when the terrain renderer commits a newer surface generation for it,
## or Rust advances its independent vegetation revision. Later surface edits clear generated
## scatter; player placements remain authoritative.
extends Node3D

const TreeSpecies := preload("res://scripts/renderers/tree_species.gd")

# Per-species draw ranges. Canopy trees carry three levels because they are the silhouette
# of the landscape at every distance. Understory reads only close up, so it gets one level
# and a short range: it exists to stop near forest floor looking like mown lawn.
#
# Every range below is evaluated ONCE PER PATCH. Godot tests one distance for a whole
# MultiMeshInstance3D, and a patch is a terrain patch: 510 m across, 361 m from centre to
# corner. A plant therefore gets the level its patch centre asks for, not the level its own
# distance asks for, and that error is up to 361 m. Two rules follow, and both were broken
# before this was measured from a ground-level camera:
#   - A band must be wider than the patch diagonal, or one patch spans several bands and
#     the level it draws is wrong for most of what it contains. Only TREE_NEAR_M is a band
#     now: the two crown levels share one instance and TREE_MID_M picks its mesh per patch.
#   - A range must be longer than the patch half-diagonal, or the patch under the camera
#     can be culled while the plants at the camera's feet are still in view.
# Lane five of a packed placement carries the species ordinal in its low two bits and, above
# them, the renderer mesh variant the brush pinned, biased by one. Zero there leaves the
# variant to the appearance seed, which is every generated plant and every plant authored
# before the brush could name one tree. Rust packs the lane in vegetation_api.rs.
const SPECIES_BITS := 2
const SPECIES_MASK := 3

const TREE_NEAR_M := 800.0
const TREE_MID_M := 2000.0
const TREE_FAR_M := 4500.0
const BUSH_RANGE_M := 420.0
const ROCK_RANGE_M := 420.0
# Patch-level cutoff for generating the understory at all. Must clear the longest understory
# range plus the patch half-diagonal, or a patch is skipped while its near corner wants bushes.
const UNDERSTORY_PATCH_RANGE_M := 800.0
# Half the diagonal of a square patch, per metre of span. A patch is one instance, so every
# conservative range test measures from the patch centre out to its farthest corner.
const PATCH_HALF_DIAGONAL := 0.7071067811865476

# How far inside its range one understory variant may stop, as a share of that range. Bush
# and rock carry a single level, so a variant that stops early drops nothing into a gap: the
# plant ends and the rest of the patch carries on. Twelve variants then end at twelve
# distances and the carpet dissolves over the last quarter of its range. Sharing one distance
# instead ends every plant of a 510 m patch on one line, and a camera above the canopy reads
# that line as a straight edge between forest floor and bare ground.
const UNDERSTORY_STAGGER_MIN := 0.75
# How far before TREE_NEAR_M a patch may swap its near canopy for the distant level. The
# switch is a per-patch decision, so on one distance every patch of the 510 m grid switches
# on one circle and the boundary reads as a staircase of squares. Giving each patch its own
# share of this breaks the staircase up. The offset is negative only: a patch may switch
# early, never late, so the near band stays the conservative superset it already is, no patch
# builds near meshes it did not build before, and the near level draws over less ground.
const CANOPY_SWITCH_JITTER_M := 160.0

var enabled := true
var density_fraction := 1.0
var cast_shadows := true
# Probe override for the canopy far range, in metres. Values at or below zero keep the
# authored TREE_FAR_M. Must stay above TREE_MID_M or the far level gets an empty range.
var far_range_override_m := 0.0
var patches: Dictionary = {}
# Patches that left terrain residency but are still inside the scatter radius. Terrain
# residency follows the camera frustum, so a rotation evicts patches that are about to be
# wanted again. These are hidden rather than freed, because rebuilding one costs a frame.
var cache: Dictionary = {}
var queue: Array[Vector2i] = []
# meshes[species][variant] is an Array[ArrayMesh], ordered near to far.
var meshes: Array = []
var last_revision := -1
var last_camera_cell := Vector2i(2147483647, 2147483647)
var tree_count := 0
var generated_patches := 0
var generation_ms_max := 0.0
var ready_for_world := false
@onready var terrain = $"../Terrain"
@onready var simulation = $"../SimulationNode"

func _ready() -> void:
	meshes = TreeSpecies.build_meshes()

## Draw range for one species level as (begin_m, end_m). `near_band` says whether the patch
## also carries the near per-variant instances, and `switch_m` is the distance this patch
## swaps its canopy on. `variant` staggers the understory and is ignored by the canopy. A patch built far away carries none, and then
## the distant instance must begin at zero: it is the only thing in the patch, so a begin of
## TREE_NEAR_M empties the patch the moment the camera reaches that distance. The rebuild that
## adds the near band is queued at one patch per frame, so a fast approach outruns it.
func lod_range(species: int, lod: int, variant: int, near_band: bool, switch_m: float) -> Vector2:
	if species == TreeSpecies.BUSH:
		return Vector2(0.0, BUSH_RANGE_M * _understory_stagger(variant))
	if species == TreeSpecies.ROCK:
		return Vector2(0.0, ROCK_RANGE_M * _understory_stagger(variant))
	if lod == 0:
		return Vector2(0.0, switch_m)
	# One instance covers both crown levels. TREE_MID_M chooses which mesh it carries, not
	# where it starts and stops, so the mid/far switch is not a visibility band at all.
	return Vector2(switch_m if near_band else 0.0, canopy_far_m())

## Share of its range one understory variant keeps. A low-discrepancy sequence rather than a
## hash: twelve variants then spread evenly across the band, where twelve samples of a hash
## clump and leave the carpet ending on two or three lines instead of twelve.
func _understory_stagger(variant: int) -> float:
	return lerpf(UNDERSTORY_STAGGER_MIN, 1.0, fmod(float(variant) * 0.6180339887498949, 1.0))

## Distance at which one patch swaps its near canopy for the distant level. See
## CANOPY_SWITCH_JITTER_M. Keyed on the patch, so a rebuild reproduces the same distance and
## the switch does not move when a terrain edit regenerates the patch.
func canopy_switch_m(key: Vector2i) -> float:
	return TREE_NEAR_M - CANOPY_SWITCH_JITTER_M * _patch_hash01(key)

## Deterministic value in [0,1) for one patch key. The mix the vegetation shaders use for
## their cell hashes. GDScript integers are 64-bit and signed, so the multiplies wrap into
## negative values; the mask at the end is what brings the result back into range.
func _patch_hash01(key: Vector2i) -> float:
	var h := key.x * 374761393 + key.y * 668265263
	h = (h ^ (h >> 13)) * 1274126177
	return float((h ^ (h >> 16)) & 0xFFFFFF) / 16777216.0

## Canopy far range in effect. One accessor so the draw range and the patch residency test
## can never disagree about how far the scatter reaches.
func canopy_far_m() -> float:
	return far_range_override_m if far_range_override_m > TREE_MID_M else TREE_FAR_M

func rebuild_from_simulation_state() -> void:
	for patch in patches.values():
		patch.queue_free()
	patches.clear()
	for patch in cache.values():
		patch.queue_free()
	cache.clear()
	queue.clear()
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
	var span: float = terrain.get_render_patch_span_m()
	if span <= 0.0:
		return
	var camera_xz := Vector2(camera.global_position.x, camera.global_position.z)
	var camera_cell := Vector2i((camera_xz / span).floor())
	var revision: int = terrain.get_resident_patch_revision()
	# O(resident patches) only on residency/camera-cell changes, not O(total scatter).
	if revision != last_revision or camera_cell != last_camera_cell:
		last_revision = revision
		last_camera_cell = camera_cell
		var wanted: Dictionary = {}
		var world: Vector2 = simulation.get_terrain_world_size()
		for key in terrain.get_resident_patch_keys():
			var center := Vector2(key) * span - world * 0.5 + Vector2.ONE * span * 0.5
			if center.distance_to(camera_xz) <= canopy_far_m() + span:
				wanted[key] = center.distance_squared_to(camera_xz)
		for key in patches.keys():
			if not wanted.has(key):
				_retire_patch(key, span, world, camera_xz)
		# A cached patch outside the scatter radius will not be wanted again from here, so it
		# is freed. This is what bounds the cache: it holds at most the patches of one disk.
		for key in cache.keys():
			if _patch_distance(key, span, world, camera_xz) > canopy_far_m() + span:
				cache[key].queue_free()
				cache.erase(key)
		queue.clear()
		for key in wanted:
			if patches.has(key):
				continue
			# A cached patch is already built. Showing it again costs no upload, so it does
			# not enter the queue and does not compete with a patch that has none.
			if cache.has(key):
				_restore_patch(key)
				continue
			queue.append(key)
		queue.sort_custom(func(a: Vector2i, b: Vector2i) -> bool: return wanted[a] > wanted[b])
	if queue.is_empty():
		# O(resident patches) per frame: one dictionary lookup and one distance. A road,
		# building or terrain edit advances the terrain surface generation of the patches it
		# covered, and only those patches regenerate. The two range checks rebuild a patch
		# when it crosses the distance where the understory or the near band starts or stops.
		var world: Vector2 = simulation.get_terrain_world_size()
		for key in patches:
			var patch: Node3D = patches[key]
			var distance := _patch_distance(key, span, world, camera_xz)
			_refresh_distant_lod(patch, distance)
			if (
				_is_patch_stale(key)
				or _understory_wanted(distance) != bool(patch.get_meta("understory"))
				or _near_band_wanted(distance, span) != bool(patch.get_meta("near_band"))
			):
				queue.append(key)
	if not queue.is_empty():
		_upload_patch(queue.pop_back(), span)

func _is_patch_stale(key: Vector2i) -> bool:
	# -1 means the terrain patch has committed no payload yet. Keep the current scatter
	# rather than churning; the next commit advances the generation and triggers a rebuild.
	var current: int = terrain.get_patch_surface_generation(key)
	var vegetation_generation: int = simulation.get_vegetation_patch_generation(key)
	return (
		(current >= 0 and current != int(patches[key].get_meta("surface_generation")))
		or vegetation_generation != int(patches[key].get_meta("vegetation_generation"))
	)

## Camera position on the ground plane, or `INF` when there is no camera to measure from.
func _camera_xz() -> Vector2:
	var camera := get_viewport().get_camera_3d()
	if camera == null:
		return Vector2.INF
	return Vector2(camera.global_position.x, camera.global_position.z)

## Distance from the camera to this patch centre, or a negative value with no camera. One
## distance feeds every range decision for the patch, so they cannot disagree.
func _patch_distance(key: Vector2i, span: float, world: Vector2, camera_xz: Vector2) -> float:
	if camera_xz == Vector2.INF:
		return -1.0
	return (Vector2(key) * span - world * 0.5 + Vector2.ONE * span * 0.5).distance_to(camera_xz)

## Whether this patch is close enough to be worth generating the dense understory layer.
func _understory_wanted(distance: float) -> bool:
	return distance >= 0.0 and distance <= UNDERSTORY_PATCH_RANGE_M

## Whether any part of this patch can fall inside the near band, and so whether the near
## per-variant meshes are worth building at all. A patch is one instance, so the test is
## conservative by the patch half-diagonal rather than by the centre. Beyond this the patch
## builds one distant level and skips the variant split, the tints and 36 of its 38 nodes.
## With no camera every level is built, which is what a headless caller wants.
func _near_band_wanted(distance: float, span: float) -> bool:
	return distance < 0.0 or distance <= TREE_NEAR_M + span * PATCH_HALF_DIAGONAL

## Which crown mesh the patch's distant instance carries. The patch is one instance, so this
## is a per-patch choice and needs no band wider than the patch.
func _distant_lod(distance: float) -> int:
	return 2 if distance >= TREE_MID_M else 1

## Moves resident distant instances across the mid/far boundary. The transforms are unchanged,
## so this is a property write where a rebuild would be another full patch upload.
func _refresh_distant_lod(patch: Node3D, distance: float) -> void:
	var lod := _distant_lod(distance)
	if int(patch.get_meta("distant_lod")) == lod:
		return
	patch.set_meta("distant_lod", lod)
	for instance in patch.get_children():
		if int(instance.get_meta("lod")) > 0:
			instance.multimesh.mesh = meshes[int(instance.get_meta("species"))][0][lod]

## Re-applies the shadow setting to resident patches. Avoids a full placement rebuild.
func set_cast_shadows(value: bool) -> void:
	cast_shadows = value
	for store in [patches, cache]:
		for patch in store.values():
			for instance in patch.get_children():
				instance.cast_shadow = _shadow_setting(
					int(instance.get_meta("species")), int(instance.get_meta("lod"))
				)

## Takes a patch out of the drawn set. A patch still inside the scatter radius is hidden and
## kept, because terrain residency is frustum-derived and will very likely ask for it again
## within a few frames. Only a patch that is genuinely out of range is freed.
func _retire_patch(key: Vector2i, span: float, world: Vector2, camera_xz: Vector2) -> void:
	var patch: Node3D = patches[key]
	tree_count -= int(patch.get_meta("tree_count"))
	patches.erase(key)
	if _patch_distance(key, span, world, camera_xz) <= canopy_far_m() + span:
		patch.visible = false
		cache[key] = patch
		return
	patch.queue_free()

## Returns a hidden patch to the drawn set. The staleness and range tests in `_process` run
## against it on the following frame, so a patch that was edited while it was hidden still
## rebuilds; showing it first is what keeps the terrain from being bare in the meantime.
func _restore_patch(key: Vector2i) -> void:
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
		"max_patch_generation_upload_ms": generation_ms_max, "density_fraction": density_fraction}

func _upload_patch(key: Vector2i, span: float) -> void:
	var start := Time.get_ticks_usec()
	# Read before the placement fetch. Stamping an older generation onto newer data only
	# causes one redundant rebuild; the reverse would leave a stale patch undetected.
	var generation: int = terrain.get_patch_surface_generation(key)
	var vegetation_generation: int = simulation.get_vegetation_patch_generation(key)
	var world: Vector2 = simulation.get_terrain_world_size()
	var distance := _patch_distance(key, span, world, _camera_xz())
	var understory := _understory_wanted(distance)
	var near_band := _near_band_wanted(distance, span)
	var distant_lod := _distant_lod(distance)
	var switch_m := canopy_switch_m(key)
	var origin := Vector2(key) * span - world * 0.5
	var data: PackedFloat32Array = simulation.get_decorative_tree_patch(origin, span, understory)
	var patch := Node3D.new()
	patch.position = Vector3(origin.x, 0.0, origin.y)
	add_child(patch)
	# One transform and tint array per variant, rebuilt per species. The near level draws
	# each array as its own MultiMesh and the distant level walks them in order, so the
	# placement loop needs no slot bookkeeping, index chains or scratch arrays. Appending
	# into a local typed array is what keeps this loop cheap: routing each placement through
	# a nested untyped Array instead cost 3.8 ms per patch on a 4096-placement fixture.
	# Out of the near band there is one bucket and no tints, because nothing that
	# distinguishes them is drawn there.
	var count := 0
	for species in range(TreeSpecies.SPECIES_COUNT):
		var levels: Array = meshes[species][0]
		# Bush and rock have only a near level, so out of the near band they draw nothing.
		if not near_band and levels.size() == 1:
			continue
		# Two placement loops rather than one loop with two branches in it. The branches
		# would run once per placement, and per-placement interpreted work is what this
		# function is made of: the same reason the variant index reads seed bits instead of
		# hashing. The distant loop has no buckets to index and no tint to compute.
		var variant_transforms: Array = []
		var variant_tints: Array = []
		var species_count := 0
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
				variant_transforms[variant].append(_instance_transform(data, i, species, seed))
				variant_tints[variant].append(_instance_tint(seed))
				species_count += 1
		else:
			var flat: Array[Transform3D] = []
			for i in range(0, data.size(), 6):
				# Out of the near band every plant of a species draws variant zero, so the
				# pin above the species bits is masked away and never read here.
				if int(data[i + 5]) & SPECIES_MASK != species:
					continue
				if fmod(absf(data[i] * 0.754877 + data[i + 2] * 0.56984), 1.0) >= density_fraction:
					continue
				flat.append(_instance_transform(data, i, species, _appearance_seed(data, i)))
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
				near_mm.mesh = meshes[species][variant][0]
				near_mm.instance_count = near_transforms.size()
				for i in range(near_transforms.size()):
					near_mm.set_instance_transform(i, near_transforms[i])
					near_mm.set_instance_color(i, near_tints[i])
				_add_instance(patch, near_mm, species, 0, variant, near_band, switch_m)
		# Only the near level pays for variants. Mid and far share variant zero's mesh and the
		# whole species population, so distance does not multiply draw calls. They also share
		# one instance: the two levels draw the same transforms with a different mesh, so the
		# level is a mesh swap on the buffer already uploaded, not a second copy of it.
		if levels.size() > 1:
			var far_mm := MultiMesh.new()
			far_mm.transform_format = MultiMesh.TRANSFORM_3D
			# No instance colours past the near band. A tree there is a few pixels, so the
			# tint is not readable, and carrying it costs an upload call and four floats per
			# instance. A patch switches level as a unit and the tint averages to one, so
			# the patch keeps its mean colour across the switch.
			far_mm.mesh = levels[distant_lod]
			far_mm.instance_count = species_count
			var slot := 0
			for variant in range(variant_transforms.size()):
				var far_transforms: Array[Transform3D] = variant_transforms[variant]
				for i in range(far_transforms.size()):
					far_mm.set_instance_transform(slot, far_transforms[i])
					slot += 1
			_add_instance(patch, far_mm, species, 1, 0, near_band, switch_m)
	_share_patch_bounds(patch)
	patch.set_meta("tree_count", count)
	patch.set_meta("surface_generation", generation)
	patch.set_meta("vegetation_generation", vegetation_generation)
	patch.set_meta("understory", understory)
	patch.set_meta("near_band", near_band)
	patch.set_meta("distant_lod", distant_lod)
	# Replace in place. The previous node stays visible until this one is in the tree,
	# so an edit does not blank the surrounding forest for a frame.
	if patches.has(key):
		tree_count -= int(patches[key].get_meta("tree_count"))
		patches[key].queue_free()
	patches[key] = patch
	tree_count += count
	generated_patches += 1
	generation_ms_max = maxf(generation_ms_max, float(Time.get_ticks_usec() - start) / 1000.0)

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
## reports empty bounds, so a headless caller shares nothing and keeps the engine default.
func _share_patch_bounds(patch: Node3D) -> void:
	var bounds: Dictionary = {}
	var levels: Dictionary = {}
	for instance in patch.get_children():
		var species: int = instance.get_meta("species")
		var box: AABB = instance.get_aabb()
		bounds[species] = box if not bounds.has(species) else (bounds[species] as AABB).merge(box)
		var seen: Dictionary = levels.get(species, {})
		seen[int(instance.get_meta("lod"))] = true
		levels[species] = seen
	for instance in patch.get_children():
		var species: int = instance.get_meta("species")
		var box: AABB = bounds[species]
		if (levels[species] as Dictionary).size() > 1 and box.size != Vector3.ZERO:
			instance.custom_aabb = box

func _add_instance(
	patch: Node3D, mm: MultiMesh, species: int, lod: int, variant: int,
	near_band: bool, switch_m: float
) -> void:
	var instance := MultiMeshInstance3D.new()
	instance.multimesh = mm
	instance.set_meta("species", species)
	instance.set_meta("lod", lod)
	# The distant level stands for every variant of its species and carries variant zero's
	# mesh, so the variant it records is the mesh it draws, not a subset of the population.
	instance.set_meta("variant", variant)
	instance.cast_shadow = _shadow_setting(species, lod)
	var range_m := lod_range(species, lod, variant, near_band, switch_m)
	# No fade mode. VISIBILITY_RANGE_FADE_SELF alpha-blends the whole instance, and
	# the instance is a whole patch: it made every plant in a patch translucent
	# whenever the patch centre sat in a band, however close the plant itself was,
	# and it moved the geometry out of the depth pre-pass to do it. The bands above
	# are now wider than a patch, so the switch is a step at a distance where one
	# patch changing silhouette is not readable.
	instance.visibility_range_begin = range_m.x
	instance.visibility_range_end = range_m.y
	patch.add_child(instance)

## Near mesh choice. A brush that plants one named tree passes a `pin` biased by one and that
## mesh is used; zero falls back to the seed's own bits, so the choice stays independent of
## proportions and lean. Bit extraction rather than another hash: the seed is already mixed,
## and one more hash call in the per-placement path is worth about a millisecond per patch on
## its own.
func _variant_index(species: int, appearance_seed: int, pin: int) -> int:
	if pin > 0:
		return pin - 1
	return ((appearance_seed >> 11) & 0xFFFF) % int(TreeSpecies.VARIANT_COUNTS[species])

## A small value spread and opposing red/blue shifts keep the foliage on its green axis.
## StandardMaterial3D multiplies instance and vertex colours, preserving per-face shade.
## Disjoint bit ranges of the already-mixed seed, for the reason given on _variant_index:
## the two hash calls this replaces cost 1.2 ms per patch on a 4096-placement fixture.
func _instance_tint(appearance_seed: int) -> Color:
	var value := lerpf(0.94, 1.06, float((appearance_seed >> 3) & 0xFFF) / 4096.0)
	var warmth := lerpf(-0.035, 0.035, float((appearance_seed >> 27) & 0xFFF) / 4096.0)
	return Color(value * (1.0 + warmth), value, value * (1.0 - warmth), 1.0)

## Default cosmetic seed from packed placement; never feeds simulation state.
func _appearance_seed(data: PackedFloat32Array, i: int) -> int:
	return hash(Vector2(data[i], data[i + 2]))

## Per-instance appearance. Yaw alone is nearly invisible on a near-symmetric lathe, so the
## variation that reads is the height/width ratio and a small lean off vertical.
## A caller can reroll those proportions with a different seed at the same placement.
func _instance_transform(
	data: PackedFloat32Array, i: int, species: int, appearance_seed: int
) -> Transform3D:
	var x := data[i]
	var z := data[i + 2]
	var scale := data[i + 4]
	# The four jitters below are the old _jitter() helper written out. Each of them ran once
	# per placement, and the call alone cost 2.6 ms per patch on a 4096-placement fixture --
	# more than the hashing it wrapped. The values are unchanged.
	var height := scale * (0.82 + float(hash(appearance_seed ^ 17) & 0xFFFFFF) / 16777216.0 * 0.42)
	var width := scale * (0.80 + float(hash(appearance_seed ^ 53) & 0xFFFFFF) / 16777216.0 * 0.48)
	# Rocks are squat and irregular; trees keep their proportions closer to upright.
	if species == TreeSpecies.ROCK:
		height *= 0.6
		width *= 1.1
	var lean := 0.0
	if species != TreeSpecies.ROCK:
		lean = 0.02 + float(hash(appearance_seed ^ 91) & 0xFFFFFF) / 16777216.0 * 0.07
	var lean_direction := float(hash(appearance_seed ^ 137) & 0xFFFFFF) / 16777216.0 * TAU
	var basis := (
		Basis(Vector3.UP, data[i + 3])
		* Basis(Vector3(cos(lean_direction), 0.0, sin(lean_direction)), lean)
		* Basis.from_scale(Vector3(width, height, width))
	)
	return Transform3D(basis, Vector3(x, data[i + 1], z))

func _shadow_setting(species: int, lod: int) -> int:
	# Only near canopy trees cast. Distant crowns are a few triangles and their cascade
	# contribution is not readable, so casting from them buys nothing for the draw cost.
	# Understory is excluded for a different reason: it is the densest species and the
	# key light runs four PSSM cascades, so each bush is submitted up to five times, while
	# its own shadow sits under a canopy shadow that already darkens the same ground.
	if cast_shadows and lod == 0 and species != TreeSpecies.ROCK and species != TreeSpecies.BUSH:
		return GeometryInstance3D.SHADOW_CASTING_SETTING_ON
	return GeometryInstance3D.SHADOW_CASTING_SETTING_OFF
