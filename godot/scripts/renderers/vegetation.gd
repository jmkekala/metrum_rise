# SPDX-License-Identifier: GPL-2.0-only

## Vegetation presentation: the mesh and material catalogue and the draw settings. Rust owns the
## rest. Rust supplies deterministic patch-local placements for four species: conifer, broadleaf,
## bush and rock, built per patch on the background pool from a snapshot (`RENDER-32-D`) and
## installed through pooled `RenderingServer` objects (`RENDER-32-E`). The stream node's
## vegetation residency (`RENDER-32-F`) decides which patches are wanted on which grid, their
## bands, their staleness and the stand-ins, from the view `CameraNode` publishes and the terrain
## residency and surface commits `terrain.gd` pushes. This script registers the catalogue, hands
## over the settings at each world load and passes its switches on; it does no per-patch work.
## A patch regenerates when the terrain renderer commits a newer surface generation for it,
## or Rust advances its independent vegetation revision. Later surface edits clear generated
## scatter; player placements remain authoritative.
extends Node3D

const TreeSpecies := preload("res://scripts/renderers/tree_species.gd")
# The shadow caster band is bounded by the sun's own range.
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
# Probe override for PATCH_SUBDIVISION (1 to 8). Values below one keep the authored subdivision.
var patch_subdivision_override := 0
# Probe override for TREE_NEAR_DETAIL_M, in metres. Values below zero keep the authored
# distance. Zero is meaningful: past the fade band it draws the near band from the reduced level.
var near_detail_override_m := -1.0
# Probe override for the per-frame request budget, in sub-patch builds. Values below one keep
# the area budget the subdivision derives. Separates "how fine is the grid" from "how much
# work may one frame do", which the derived budget ties together.
var upload_budget_override := 0
# meshes[species][variant] is an Array[ArrayMesh], ordered near to far. A canopy variant holds
# the full level, the reduced level, and both in one mesh; see TREE_NEAR_DETAIL_M. The stream
# node draws them by RID (_register_catalogue).
var meshes: Array = []
# The mesh catalogue packed for the product; see _product_catalogue.
var _catalogue := PackedFloat32Array()
@onready var terrain = $"../Terrain"
@onready var simulation = $"../SimulationNode"
@onready var stream = $"../StreamNode"
# What the stream node was last told: shown, perf channel on, fresh buffers (motion vectors).
var _stream_enabled := true
var _stream_perf := false
var _stream_fresh := false
# The water reflection's world, handed to the stream node once the catalogue is registered.
var _reflection_scenario := RID()
var _catalogue_registered := false

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

## Canopy far range in effect. One accessor so the draw range and the patch residency test
## can never disagree about how far the scatter reaches.
func canopy_far_m() -> float:
	return far_range_override_m if far_range_override_m > canopy_near_m() else TREE_FAR_M

## Distance every tree has finished its handover to the impostor by. The handover is per tree,
## so this no longer has to clear the patch diagonal; the patch ranges pay for that instead.
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
## Also the square root of the per-frame request budget.
func patch_subdivision() -> int:
	return clampi(patch_subdivision_override, 1, 8) if patch_subdivision_override >= 1 else PATCH_SUBDIVISION

## Starts the vegetation over for the loaded world: the catalogue again (probes swap the detail
## distance and the meshes), the products' catalogue to the background jobs, and the settings to
## the stream node, which frees every patch and drops every request.
func rebuild_from_simulation_state() -> void:
	TreeSpecies.set_crossfade(canopy_near_m())
	TreeSpecies.set_detail(near_detail_m())
	TreeSpecies.set_far(canopy_far_m())
	_register_catalogue()
	simulation.set_vegetation_product_catalogue(_product_catalogue())
	stream.set_vegetation_settings(_settings())

## The stream node's vegetation settings: the products' draw distances, the subdivision, the
## request budget (0: the subdivision squared), the sun's shadow range and the world's size. The
## world's size is read once per load, which waits on the render snapshot's lock.
func _settings() -> PackedFloat64Array:
	var world: Vector2 = simulation.get_terrain_world_size()
	var settings := _product_settings()
	settings.append_array(PackedFloat64Array([patch_subdivision(), maxi(upload_budget_override, 0),
		SceneLightingConfig.shadow_max_distance_m(), world.x, world.y]))
	return settings

func _process(_delta: float) -> void:
	var start_us := Time.get_ticks_usec()
	visible = enabled
	_sync_stream()
	if _stream_perf:
		# The stream node records the residency and installs under the same name; the benchmark
		# sums the two.
		var ms := float(Time.get_ticks_usec() - start_us) / 1000.0
		PerfDebug.record("vegetation", ms, {"script_ms": ms})

## Re-applies the shadow setting to installed patches (the stream node). Avoids a full placement
## rebuild.
func set_cast_shadows(value: bool) -> void:
	cast_shadows = value
	stream.set_vegetation_cast_shadows(value)

func has_pending_work() -> bool:
	return enabled and stream.has_vegetation_pending_work()

func metrics() -> Dictionary:
	var counts: Dictionary = stream.get_vegetation_metrics()
	return {"enabled": enabled, "resident_patches": counts["patches"], "resident_trees": counts["trees"],
		"pending_patches": counts["queue"], "cached_patches": counts["cached"],
		"generated_patches": counts["generated"],
		"max_patch_generation_upload_ms": float(counts["generation_max_us"]) / 1000.0,
		"density_fraction": density_fraction,
		"patch_span_m": float(terrain.get_render_patch_span_m()) / patch_subdivision(),
		"patch_subdivision": patch_subdivision(), "fine_patches": counts["fine"],
		"coarse_patches": counts["coarse"],
		"fine_tier_radius_m": maxf(canopy_near_m(), maxf(BUSH_RANGE_M, ROCK_RANGE_M)),
		"canopy_near_m": canopy_near_m(), "canopy_far_m": canopy_far_m()}

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
