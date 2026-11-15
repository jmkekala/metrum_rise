# SPDX-License-Identifier: GPL-2.0-only

## Bakes the Terrain menu's planting pictures (`terrain_panel.gd`) into
## `assets/ui/vegetation/<name>.png`, one per brush option of `vegetation_tool.gd`. Rust says
## what each preset plants (`get_vegetation_preset_sample`: one plant, a stand of seven for a mix,
## a four-module hedge row); `vegetation_placements.gd` draws it with the game's meshes in a
## private world with its own light and a transparent background. Nothing of the game's weather
## or time of day is running here, so the pictures are the same on every bake. Re-run it when a
## vegetation mesh, a preset or the option list changes:
##   godot --path godot --script res://tests/bake_vegetation_pictures.gd
## It needs a renderer (not `--headless`). Not part of `./run.sh --test`.
extends SceneTree

const VegetationTool = preload("res://scripts/tools/vegetation_tool.gd")
const TerrainPanel = preload("res://scripts/ui/terrain_panel.gd")
const VegetationPlacements = preload("res://scripts/renderers/vegetation_placements.gd")
const TreeSpecies = preload("res://scripts/renderers/tree_species.gd")

## Pixel side of a picture (a 72 px tile at UI scale 2).
const SIDE_PX := 144
const FOV_DEG := 28.0
# The camera looks down at a garden-visit angle, from the front-left.
const PITCH_DEG := 24.0
const YAW_DEG := 35.0
# The share of the frame the plants may fill, on their longer side.
const FRAME_MARGIN := 1.0

func _initialize() -> void:
	_run.call_deferred()

func _run() -> void:
	var simulation := SimulationNode.new()
	root.add_child(simulation)
	var meshes := TreeSpecies.build_meshes()
	var viewport := _stage()
	root.add_child(viewport)
	var camera: Camera3D = viewport.get_node("Camera")
	DirAccess.make_dir_recursive_absolute(ProjectSettings.globalize_path(TerrainPanel.PICTURE_DIR))
	var failures := 0
	for index in VegetationTool.BRUSH_OPTIONS.size():
		var option: Dictionary = VegetationTool.BRUSH_OPTIONS[index]
		var placements: PackedFloat32Array = simulation.get_vegetation_preset_sample(option.preset)
		if placements.is_empty():
			push_error("%s: Rust has no sample for preset %d" % [option.label, option.preset])
			failures += 1
			continue
		var plants := VegetationPlacements.instances(placements, meshes)
		viewport.add_child(plants)
		_frame(camera, plants)
		viewport.render_target_update_mode = SubViewport.UPDATE_ONCE
		await RenderingServer.frame_post_draw
		var image := viewport.get_texture().get_image()
		plants.free()
		var path := ProjectSettings.globalize_path(TerrainPanel.picture_path(index))
		if image == null or image.is_empty() or image.save_png(path) != OK:
			push_error("%s: the picture did not save to %s" % [option.label, path])
			failures += 1
			continue
		print("baked ", path)
	quit(failures)

# A transparent private world with a camera, a warm sun and a soft sky fill.
func _stage() -> SubViewport:
	var viewport := SubViewport.new()
	viewport.size = Vector2i(SIDE_PX, SIDE_PX)
	viewport.own_world_3d = true
	viewport.transparent_bg = true
	viewport.msaa_3d = Viewport.MSAA_4X
	viewport.render_target_update_mode = SubViewport.UPDATE_DISABLED
	var environment := Environment.new()
	environment.background_mode = Environment.BG_CLEAR_COLOR
	environment.ambient_light_source = Environment.AMBIENT_SOURCE_COLOR
	environment.ambient_light_color = Color(0.78, 0.84, 0.92)
	environment.ambient_light_energy = 1.0
	environment.tonemap_mode = Environment.TONE_MAPPER_FILMIC
	var world := WorldEnvironment.new()
	world.environment = environment
	viewport.add_child(world)
	var sun := DirectionalLight3D.new()
	sun.light_color = Color(1.0, 0.96, 0.88)
	sun.light_energy = 2.2
	sun.rotation_degrees = Vector3(-48.0, -30.0, 0.0)
	viewport.add_child(sun)
	var camera := Camera3D.new()
	camera.name = "Camera"
	camera.fov = FOV_DEG
	camera.current = true
	viewport.add_child(camera)
	return viewport

# Fits the plants' bounds in the view: the camera backs off along its fixed direction until
# every corner of the bounds is inside the field of view, with FRAME_MARGIN to spare, so a tall
# lone tree fills the height and a hedge row the width.
func _frame(camera: Camera3D, plants: Node3D) -> void:
	var bounds := AABB()
	var first := true
	for child in plants.get_children():
		var box: AABB = (child as MultiMeshInstance3D).multimesh.get_aabb()
		bounds = box if first else bounds.merge(box)
		first = false
	var basis := Basis.from_euler(Vector3(deg_to_rad(-PITCH_DEG), deg_to_rad(-YAW_DEG), 0.0))
	var back := basis * Vector3.BACK
	var centre := bounds.get_center()
	var slope := tan(deg_to_rad(FOV_DEG * 0.5)) / FRAME_MARGIN
	var distance := 0.5
	for corner in 8:
		var offset := bounds.get_endpoint(corner) - centre
		var across := maxf(absf(offset.dot(basis.x)), absf(offset.dot(basis.y)))
		distance = maxf(distance, offset.dot(back) + across / slope)
	camera.basis = basis
	camera.position = centre + back * distance
	var depth := bounds.size.length()
	camera.near = maxf(distance - depth, 0.05)
	camera.far = distance + depth
