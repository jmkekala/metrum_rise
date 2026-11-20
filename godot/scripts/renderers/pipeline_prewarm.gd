# SPDX-License-Identifier: GPL-3.0-or-later

## The first load's pipeline prewarm (`RENDER-43`, docs/terrain.md "First-use pipelines at the
## load"). Godot compiles a surface's pipelines in the background when the surface is created, and
## a draw waits for the one it needs; with a cold driver cache one terrain pipeline takes most of a
## second. The layers that first draw frames after a load (the terrain patches, the graded tiles,
## the water, the vegetation, the grass masks) each made the frame they first drew in wait for
## theirs, one after another. `add` puts one hidden instance of every (mesh, material) template
## of a group into a scenario: Godot creates their surfaces when it next renders a 3D scene and
## compiles their pipelines in the background (the menu does this a group at a time from the
## game's launch, `menu_pipeline_prewarm.gd`). A hidden instance is never drawn, culled or paired;
## `release` and `end` free them, and the compiled pipelines stay with their shaders and the
## caches. A pipeline depends on the shader, the mesh's vertex format and primitive and the pass,
## never on a material's parameters or a mesh's size, so the pairs are templates (`groups`,
## `pipeline_template.gd`).
extends RefCounted

const TerrainRenderer := preload("res://scripts/renderers/terrain.gd")
const WaterRenderer := preload("res://scripts/renderers/water.gd")
const VegetationRenderer := preload("res://scripts/renderers/vegetation.gd")
const GrassRenderer := preload("res://scripts/renderers/grass.gd")
const TreeSpecies := preload("res://scripts/renderers/tree_species.gd")
const TerrainGradedTiles := preload("res://scripts/renderers/terrain_graded_tiles.gd")
const WaterReflection := preload("res://scripts/renderers/water_reflection.gd")
const AgentsRenderer := preload("res://scripts/renderers/agents.gd")
const VehicleResources := preload("res://scripts/renderers/vehicle_resources.gd")
const VehicleLamps := preload("res://scripts/renderers/vehicle_lamps.gd")
const BuildingsRenderer := preload("res://scripts/renderers/buildings.gd")
const BuildingLods := preload("res://scripts/renderers/building_lods.gd")
const WorldMaterials := preload("res://scripts/renderers/world_materials.gd")
const StreetLights := preload("res://scripts/renderers/street_lights.gd")
const RoadSigns := preload("res://scripts/renderers/road_signs.gd")

## A group's name, its pairs' builder (a Callable answering `[Mesh, Material or null]` pairs) and
## whether the water mirror draws it (its pipelines are also wanted at the mirror's MSAA).
const GROUP_NAME := "name"
const GROUP_PAIRS := "pairs"
const GROUP_MIRROR := "mirror"

# The hidden instances of the group in flight (its slices added so far).
var _instances: Array[RID] = []
# Every added group's resources, alive until `end`: the shaders keep their pipelines.
var _pairs: Array = []

## Puts a hidden instance of every `[Mesh, Material or null]` pair into `scenario` (a null
## material draws the mesh's own surface materials), beside those already added. O(pairs).
func add(scenario: RID, pairs: Array) -> void:
	_pairs.append_array(pairs)
	for pair: Array in pairs:
		var mesh := pair[0] as Mesh
		if mesh == null or mesh.get_surface_count() == 0:
			continue
		var instance := RenderingServer.instance_create()
		RenderingServer.instance_set_visible(instance, false)
		RenderingServer.instance_set_base(instance, mesh.get_rid())
		if pair[1] != null:
			RenderingServer.instance_geometry_set_material_override(instance, (pair[1] as Material).get_rid())
		RenderingServer.instance_set_scenario(instance, scenario)
		_instances.append(instance)

## Frees the hidden instances and keeps their resources: the next 3D render compiles nothing more
## for them (a finished group leaves the precompiled set the project's MSAA setting selects).
## O(instances).
func release() -> void:
	for instance in _instances:
		RenderingServer.free_rid(instance)
	_instances.clear()

## Frees the hidden instances and the resources; the compiled pipelines stay with their shaders,
## which the renderers hold. O(pairs).
func end() -> void:
	release()
	_pairs = []

## How many hidden instances are in the world (tests).
func instance_count() -> int:
	return _instances.size()

## The templates of every layer that first draws after a load, in groups in the order the menu
## adds them (`menu_pipeline_prewarm.gd`): what every world draws first. Whatever the city to
## come. Each group is `{GROUP_NAME, GROUP_PAIRS, GROUP_MIRROR}`; its pairs are built when called.
static func groups() -> Array[Dictionary]:
	var out: Array[Dictionary] = []
	# What every world draws first, in one build (the menu's second frame, as before R43c): the
	# terrain and its graded tiles, the water, the grass masks and the vegetation catalogue (its
	# authored meshes load once and stay shared with the vegetation renderer) with the impostors
	# and hedges. Built later, a share of it would fall on the world scene's start after an early
	# click instead (round R43c: built a species a frame, a first run left at the earliest
	# moment waited 1.5 s more after its click, though 2 s earlier).
	out.append(_group("ground", func() -> Array:
		var pairs := TerrainRenderer.pipeline_prewarm_surfaces()
		pairs.append_array(WaterRenderer.pipeline_prewarm_surfaces())
		pairs.append_array(GrassRenderer.pipeline_prewarm_surfaces())
		pairs.append_array(VegetationRenderer.pipeline_prewarm_surfaces(TreeSpecies.build_meshes()))
		return pairs))
	# The sets the world scene makes at its start, before any content: the cars, their lamps and
	# beams, the walkers, the site batches, the grass clumps and the world's borders.
	out.append(_group("first_frame", func() -> Array:
		var pairs := VehicleResources.pipeline_prewarm_surfaces()
		pairs.append_array(VehicleLamps.pipeline_prewarm_surfaces())
		pairs.append_array(AgentsRenderer.pipeline_prewarm_surfaces())
		pairs.append_array(BuildingsRenderer.pipeline_prewarm_surfaces())
		pairs.append_array(GrassRenderer.pipeline_prewarm_clumps())
		pairs.append_array(TerrainRenderer.pipeline_prewarm_border())
		pairs.append_array(WaterRenderer.pipeline_prewarm_border())
		return pairs))
	# The graded tiles' base cells: the terrain shader on the lattice's vertex format.
	out.append(_group("base_cells", func() -> Array:
		return TerrainGradedTiles.pipeline_prewarm_base_cells()))
	# Worlds with roads: the chunk layers, then the street lights and the signs. Before the mirror
	# (round R43d): a player who leaves while a group is in flight waits for it at the world
	# scene's start, and the mirror's slices (the terrain proxies at 2x) are among the slowest;
	# only worlds with water draw it, and late (a pose toward water).
	out.append(_group("roads", func() -> Array: return WorldMaterials.pipeline_prewarm_roads()))
	out.append(_group("lights_signs", func() -> Array:
		var pairs := StreetLights.pipeline_prewarm_surfaces()
		pairs.append_array(RoadSigns.pipeline_prewarm_surfaces())
		return pairs))
	# What the water mirror draws, at its MSAA too (and, with the setting restored, at the game's):
	# the terrain proxies, the tree impostors, the building tiers and the site batches. Last.
	var mirror := func() -> Array:
		var pairs := WaterReflection.pipeline_prewarm_surfaces()
		for species in [TreeSpecies.CONIFER, TreeSpecies.BROADLEAF]:
			pairs.append([TreeSpecies.impostor_mesh(), TreeSpecies.impostor_material(species)])
		pairs.append_array(BuildingLods.pipeline_prewarm_surfaces())
		pairs.append_array(BuildingsRenderer.pipeline_prewarm_surfaces())
		return pairs
	out.append(_group("mirror", mirror, true))
	return out

## Every group's pairs, in order (the benchmark's inventory). Builds them all at once.
static func templates() -> Array:
	var pairs := []
	for group in groups():
		pairs.append_array((group[GROUP_PAIRS] as Callable).call())
	return pairs

static func _group(name: String, pairs: Callable, mirror := false) -> Dictionary:
	return {GROUP_NAME: name, GROUP_PAIRS: pairs, GROUP_MIRROR: mirror}

## Queues a marker: a low-priority task that does nothing, in the queue Godot's background
## pipeline compilations wait in (first in, first out, a few running at once:
## `threading/worker_pool/low_priority_thread_ratio` of the pool's threads). When it has run, the
## queue has taken up every compilation queued before it (a few may still be running). Answers
## its task id, for `WorkerThreadPool.is_task_completed` and `wait_for_task_completion`. O(1).
static func queue_marker() -> int:
	return WorkerThreadPool.add_task(func() -> void: pass, false, "pipeline prewarm marker")
