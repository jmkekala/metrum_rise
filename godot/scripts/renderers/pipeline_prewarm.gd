# SPDX-License-Identifier: GPL-2.0-only

## The first load's pipeline prewarm (`RENDER-43`, docs/terrain.md "First-use pipelines at the
## load"). Godot compiles a surface's pipelines in the background when the surface is created, and
## a draw waits for the one it needs; with a cold driver cache one terrain pipeline takes most of a
## second. The layers that first draw frames after a load (the terrain patches, the graded tiles,
## the water, the vegetation, the grass masks) each made the frame they first drew in wait for
## theirs, one after another. `begin` puts one hidden instance of every (mesh, material) template
## those layers draw with into a scenario: Godot creates their surfaces when it next renders a 3D
## scene and compiles all their pipelines at once in the background (the menu does this from the
## game's launch, `menu_pipeline_prewarm.gd`). A hidden instance is never drawn, culled or paired;
## `end` frees them, and the compiled pipelines stay with their shaders and the caches. A pipeline
## depends on the shader, the mesh's vertex format and primitive and the pass, never on a
## material's parameters or a mesh's size, so the pairs are templates (`templates`).
extends RefCounted

const TerrainRenderer := preload("res://scripts/renderers/terrain.gd")
const WaterRenderer := preload("res://scripts/renderers/water.gd")
const VegetationRenderer := preload("res://scripts/renderers/vegetation.gd")
const GrassRenderer := preload("res://scripts/renderers/grass.gd")
const TreeSpecies := preload("res://scripts/renderers/tree_species.gd")

var _instances: Array[RID] = []
# The pairs' resources, alive as long as their instances.
var _pairs: Array = []

## Puts a hidden instance of every `[Mesh, Material or null]` pair into `scenario` (a null
## material draws the mesh's own surface materials). O(pairs).
func begin(scenario: RID, pairs: Array) -> void:
	end()
	_pairs = pairs
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

## Frees the hidden instances; the compiled pipelines stay with their shaders. O(pairs).
func end() -> void:
	for instance in _instances:
		RenderingServer.free_rid(instance)
	_instances.clear()
	_pairs = []

## How many hidden instances are in the world (tests).
func instance_count() -> int:
	return _instances.size()

## Every `[Mesh, Material or null]` template of the layers that first draw after a load: the
## terrain and its graded tiles, the water, the vegetation catalogue (its authored meshes load once
## and stay shared with the vegetation renderer) and the grass masks. Whatever the city to come.
static func templates() -> Array:
	var pairs := TerrainRenderer.pipeline_prewarm_surfaces()
	pairs.append_array(WaterRenderer.pipeline_prewarm_surfaces())
	pairs.append_array(VegetationRenderer.pipeline_prewarm_surfaces(TreeSpecies.build_meshes()))
	pairs.append_array(GrassRenderer.pipeline_prewarm_surfaces())
	return pairs
