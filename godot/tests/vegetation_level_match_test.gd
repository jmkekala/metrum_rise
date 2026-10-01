# SPDX-License-Identifier: GPL-2.0-only

## Holds the near and the distant canopy level to one rendered luminance, on a real GPU.
##
## `tools/vegetation_lod_measure.py` rasterises the crowns on the CPU under a fixed test light.
## It reported the two levels agreeing on colour within 10% while the shipped renderer drew the
## distant level 28-36% brighter, because a CPU reference cannot see the engine's ambient, its
## backlight or its tone map. The invariant that matters is what the GPU puts on the screen, so
## this regression puts both levels on the GPU over one population and compares the pixels.
##
## One pose is not enough. The two levels once agreed with the sun behind the camera and the
## distant level rendered at half the near level's luminance with the sun in front of it, so the
## comparison runs over camera elevation, sun elevation and sun azimuth relative to the view.
##
## Nor is one distance. The poses once all stood at 900 m and 20 degrees or more above the
## stand, and the impostor passed there while every tree was lit as if it faced one way: over a
## stand of random yaws that averages out. From near the ground at the handover distance, where
## a player watches one tree change, the same impostor rendered 26% dark with the sun behind the
## camera and 24% bright with it ahead. So a second set of poses stands there.
extends SceneTree

const Species = preload("res://scripts/renderers/tree_species.gd")

# Stems per hectare of a brush-painted stand, which is the case the level switch is visible in.
const STEMS_PER_HA := 625.0
const PATCH_EXTENT_M := 120.0
# Past TREE_NEAR_M, so the distant level is the one the renderer would choose here.
const CAMERA_DISTANCE_M := 900.0
const CAMERA_ELEVATIONS_DEG := [20.0, 35.0, 55.0, 80.0]
const SUN_ELEVATIONS_DEG := [15.0, 35.0, 60.0]
# Relative to the view: 0 puts the sun behind the camera, 180 puts it in front of the camera.
const SUN_AZIMUTHS_DEG := [0.0, 90.0, 180.0]
# The middle of the handover, from about eye height to the height of a low aerial view.
const HANDOVER_DISTANCE_M := Species.TREE_CROSSFADE_END_M - Species.TREE_CROSSFADE_M * 0.5
const HANDOVER_CAMERA_ELEVATIONS_DEG := [3.0, 10.0, 20.0]
const HANDOVER_SUN_ELEVATIONS_DEG := [15.0, 35.0]
# The two levels are different surfaces standing in for each other, so they are not expected to
# agree exactly. They are expected not to differ the way a viewer reads as two colours of forest.
const LUMINANCE_TOLERANCE := 0.12

var _failures := 0
var _viewport: SubViewport
var _ground: StandardMaterial3D

func _initialize() -> void:
	call_deferred("_run")

func _expect(condition: bool, message: String) -> void:
	if not condition:
		_failures += 1
		push_error(message)

## Deterministic lattice with hash jitter. Every level under test draws this same population.
func _scatter(count: int, extent: float) -> Array[Transform3D]:
	var out: Array[Transform3D] = []
	var side := int(ceil(sqrt(float(count))))
	var step := extent / float(side)
	for i in range(count):
		var gx := i % side
		var gz := i / side
		var h := float((gx * 374761393 + gz * 668265263) & 0xFFFF) / 65536.0
		var h2 := float((gx * 1274126177 + gz * 2246822519) & 0xFFFF) / 65536.0
		var origin := Vector3(
			-extent * 0.5 + (float(gx) + h) * step, 0.0, -extent * 0.5 + (float(gz) + h2) * step
		)
		var placed := Transform3D(Basis().rotated(Vector3.UP, h * TAU), origin)
		out.append(placed.scaled_local(Vector3.ONE * (0.85 + h2 * 0.3)))
	return out

func _add(holder: Node3D, mesh: Mesh, transforms: Array[Transform3D], tinted: bool, species: int = -1) -> void:
	var mm := MultiMesh.new()
	mm.transform_format = MultiMesh.TRANSFORM_3D
	mm.use_colors = tinted
	mm.use_custom_data = not tinted
	mm.mesh = mesh
	mm.instance_count = transforms.size()
	for i in range(transforms.size()):
		mm.set_instance_transform(i, transforms[i])
		if tinted:
			mm.set_instance_color(i, Color.WHITE)
		else:
			var variant: int = i % Species.VARIANT_COUNTS[species]
			mm.set_instance_custom_data(i, Color(float(variant), 1.0, 1.0, 1.0))
	var node := MultiMeshInstance3D.new()
	node.multimesh = mm
	if not tinted:
		node.material_override = Species.impostor_material(species)
		node.cast_shadow = GeometryInstance3D.SHADOW_CASTING_SETTING_OFF
		# Covers this fixed 120 m stand and the reconstructed quad under every pose.
		node.custom_aabb = AABB(Vector3(-100, -40, -100), Vector3(200, 120, 200))
	holder.add_child(node)

func _shot() -> Image:
	await process_frame
	await process_frame
	await RenderingServer.frame_post_draw
	return _viewport.get_texture().get_image()

## Mean lit colour of the crown pixels alone. The mask pass paints every surface that is not a
## crown one flat colour, so the average is not diluted by the ground the crowns stand on.
func _crown_color(holder: Node3D) -> Vector3:
	_ground.shading_mode = BaseMaterial3D.SHADING_MODE_UNSHADED
	_ground.albedo_color = Color(1.0, 0.0, 1.0)
	var mask_image := await _shot()
	var mask: Array[Vector2i] = []
	for y in range(mask_image.get_height()):
		for x in range(mask_image.get_width()):
			var c := mask_image.get_pixel(x, y)
			if not (c.r > 0.30 and c.b > 0.30 and c.g < 0.30):
				mask.append(Vector2i(x, y))
	_ground.shading_mode = BaseMaterial3D.SHADING_MODE_PER_PIXEL
	_ground.albedo_color = Color(0.30, 0.42, 0.18)
	var lit_image := await _shot()
	var sum := Vector3.ZERO
	for pixel in mask:
		var c := lit_image.get_pixel(pixel.x, pixel.y)
		sum += Vector3(c.r, c.g, c.b)
	return sum / maxf(float(mask.size()), 1.0)

func _check_card_backfaces(camera: Camera3D) -> void:
	# Reversing a card's winding must not reverse its authored volume lighting.
	# An opaque mask and zero sway isolate that contract from atlas filtering and TIME.
	var material: ShaderMaterial = Species._foliage_material().duplicate()
	var mask := Image.create(1, 1, false, Image.FORMAT_RGBA8)
	mask.fill(Color.WHITE)
	material.set_shader_parameter("foliage_mask", ImageTexture.create_from_image(mask))
	var arrays := []
	arrays.resize(Mesh.ARRAY_MAX)
	arrays[Mesh.ARRAY_VERTEX] = PackedVector3Array([
		Vector3(-40, 30, -40), Vector3(40, 30, -40),
		Vector3(40, 30, 40), Vector3(-40, 30, 40),
	])
	arrays[Mesh.ARRAY_NORMAL] = PackedVector3Array([Vector3.UP, Vector3.UP, Vector3.UP, Vector3.UP])
	var color := Color(0.2, 0.3, 0.07, 0.0)
	arrays[Mesh.ARRAY_COLOR] = PackedColorArray([color, color, color, color])
	arrays[Mesh.ARRAY_TEX_UV] = PackedVector2Array([Vector2.ZERO, Vector2.RIGHT, Vector2.ONE, Vector2.DOWN])
	var node := MeshInstance3D.new()
	node.cast_shadow = GeometryInstance3D.SHADOW_CASTING_SETTING_OFF
	_viewport.add_child(node)
	var point := Vector2i(camera.unproject_position(Vector3(0, 30, 0)))
	var measured: Array[Color] = []
	for indices in [PackedInt32Array([0, 1, 2, 0, 2, 3]), PackedInt32Array([0, 2, 1, 0, 3, 2])]:
		arrays[Mesh.ARRAY_INDEX] = indices
		var mesh := ArrayMesh.new()
		mesh.add_surface_from_arrays(Mesh.PRIMITIVE_TRIANGLES, arrays)
		mesh.surface_set_material(0, material)
		node.mesh = mesh
		var shot := await _shot()
		measured.append(shot.get_pixelv(point))
	var front := Vector3(measured[0].r, measured[0].g, measured[0].b)
	var back := Vector3(measured[1].r, measured[1].g, measured[1].b)
	_expect(front.y > front.x and front.y > front.z, "the backface probe must sample foliage")
	_expect(front.distance_to(back) < 0.01, "card winding must not change crown lighting")
	node.queue_free()
	await process_frame

## The camera looks at the patch centre from +Z, so a sun azimuth of 0 lights the view from behind.
func _place_camera(camera: Camera3D, elevation_deg: float, distance_m: float) -> void:
	var elevation := deg_to_rad(elevation_deg)
	camera.position = Vector3(0.0, sin(elevation) * distance_m, cos(elevation) * distance_m)
	camera.look_at(Vector3.ZERO, Vector3.UP)

## Crown luminance of one level over the shared population. The near level draws every variant.
func _level_luminance(catalogue: Array, species: int, transforms: Array[Transform3D], near: bool) -> float:
	var holder := Node3D.new()
	_viewport.add_child(holder)
	if near:
		var variants: int = Species.VARIANT_COUNTS[species]
		for variant in range(variants):
			var subset: Array[Transform3D] = []
			for i in range(transforms.size()):
				if i % variants == variant:
					subset.append(transforms[i])
			if not subset.is_empty():
				# The reduced level: it is what the impostor replaces at the switch.
				_add(holder, catalogue[species][variant][1], subset, true)
	else:
		_add(holder, Species.impostor_mesh(), transforms, false, species)
	var color := await _crown_color(holder)
	holder.queue_free()
	await process_frame
	return color.dot(Vector3(0.2126, 0.7152, 0.0722))

func _run() -> void:
	if DisplayServer.get_name() == "headless":
		push_error("This regression needs a rendering display; the dummy renderer draws no pixels.")
		quit(2)
		return
	var catalogue := Species.build_meshes()
	_viewport = SubViewport.new()
	# The pose grid multiplies the pixel walk, so the viewport is smaller than one pose would need.
	_viewport.size = Vector2i(384, 384)
	_viewport.own_world_3d = true
	_viewport.render_target_update_mode = SubViewport.UPDATE_ALWAYS
	root.add_child(_viewport)

	# A flat backdrop and a fixed ambient, so the comparison is between the two levels and not
	# between two samples of the day cycle.
	var environment := Environment.new()
	environment.background_mode = Environment.BG_COLOR
	environment.background_color = Color(1.0, 0.0, 1.0)
	environment.ambient_light_source = Environment.AMBIENT_SOURCE_COLOR
	environment.ambient_light_color = Color(0.55, 0.66, 0.90)
	environment.ambient_light_energy = 0.45
	var world_environment := WorldEnvironment.new()
	world_environment.environment = environment
	_viewport.add_child(world_environment)

	var ground := MeshInstance3D.new()
	var plane := PlaneMesh.new()
	plane.size = Vector2(PATCH_EXTENT_M, PATCH_EXTENT_M)
	ground.mesh = plane
	_ground = StandardMaterial3D.new()
	ground.material_override = _ground
	_viewport.add_child(ground)

	var light := DirectionalLight3D.new()
	light.rotation_degrees = Vector3(-42.0, 35.0, 0.0)
	light.shadow_enabled = true
	_viewport.add_child(light)

	var camera := Camera3D.new()
	camera.fov = 70.0
	camera.far = 20000.0
	_viewport.add_child(camera)
	camera.current = true
	_place_camera(camera, CAMERA_ELEVATIONS_DEG[2], CAMERA_DISTANCE_M)
	await _check_card_backfaces(camera)
	# Every impostor drawn whole: the stand reaches inside the handover from the nearer poses,
	# and the comparison is between two whole levels. The near level here takes no handover.
	# The materials must exist first: set_crossfade moves only the ones already built.
	# Both levels keep their baked size: a far crown grows past the shadow range by design, and
	# this compares their light, not their size.
	for species in [Species.CONIFER, Species.BROADLEAF]:
		Species.impostor_material(species).set_shader_parameter("crown_growth", 1.0)
	Species.set_crossfade(1.0)

	var stems := int(PATCH_EXTENT_M * PATCH_EXTENT_M / 10000.0 * STEMS_PER_HA)
	var transforms := _scatter(stems, PATCH_EXTENT_M)
	for poses in [[CAMERA_DISTANCE_M, CAMERA_ELEVATIONS_DEG, SUN_ELEVATIONS_DEG],
			[HANDOVER_DISTANCE_M, HANDOVER_CAMERA_ELEVATIONS_DEG, HANDOVER_SUN_ELEVATIONS_DEG]]:
		for camera_elevation in poses[1]:
			_place_camera(camera, camera_elevation, poses[0])
			for sun_elevation in poses[2]:
				for sun_azimuth in SUN_AZIMUTHS_DEG:
					light.rotation_degrees = Vector3(-sun_elevation, sun_azimuth, 0.0)
					for species in [Species.CONIFER, Species.BROADLEAF]:
						var near_luminance := await _level_luminance(catalogue, species, transforms, true)
						var distant_luminance := await _level_luminance(catalogue, species, transforms, false)
						var ratio := distant_luminance / maxf(near_luminance, 0.0001)
						print(
							"vegetation_level_match distance=%d camera=%d sun=%d/%d species=%d near=%.4f distant=%.4f ratio=%.3f"
							% [poses[0], camera_elevation, sun_elevation, sun_azimuth, species, near_luminance,
								distant_luminance, ratio]
						)
						_expect(
							absf(ratio - 1.0) <= LUMINANCE_TOLERANCE,
							"the distant level must render to the near level's luminance, got ratio %.3f" % ratio
						)
	_viewport.queue_free()
	await process_frame
	print("Vegetation level match tests: %d failures" % _failures)
	quit(0 if _failures == 0 else 1)
