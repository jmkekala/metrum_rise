# SPDX-License-Identifier: GPL-2.0-only

## Holds the near and the distant canopy level to one rendered luminance, on a real GPU.
##
## `tools/vegetation_lod_measure.py` rasterises the crowns on the CPU under a fixed test light.
## It reported the two levels agreeing on colour within 10% while the shipped renderer drew the
## distant level 28-36% brighter, because a CPU reference cannot see the engine's ambient, its
## backlight or its tone map. The invariant that matters is what the GPU puts on the screen, so
## this regression puts both levels on the GPU over one population and compares the pixels.
extends SceneTree

const Species = preload("res://scripts/renderers/tree_species.gd")

# Stems per hectare of a brush-painted stand, which is the case the level switch is visible in.
const STEMS_PER_HA := 625.0
const PATCH_EXTENT_M := 120.0
# Past TREE_NEAR_M, so the distant level is the one the renderer would choose here.
const CAMERA_DISTANCE_M := 900.0
const CAMERA_ELEVATION_DEG := 55.0
# The two levels are different surfaces standing in for each other, so they are not expected to
# agree exactly. They are expected not to differ the way a viewer reads as two colours of forest.
const LUMINANCE_TOLERANCE := 0.10

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

func _add(holder: Node3D, mesh: Mesh, transforms: Array[Transform3D], tinted: bool) -> void:
	var mm := MultiMesh.new()
	mm.transform_format = MultiMesh.TRANSFORM_3D
	mm.use_colors = tinted
	mm.mesh = mesh
	mm.instance_count = transforms.size()
	for i in range(transforms.size()):
		mm.set_instance_transform(i, transforms[i])
		if tinted:
			mm.set_instance_color(i, Color.WHITE)
	var node := MultiMeshInstance3D.new()
	node.multimesh = mm
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

func _run() -> void:
	if DisplayServer.get_name() == "headless":
		push_error("This regression needs a rendering display; the dummy renderer draws no pixels.")
		quit(2)
		return
	var catalogue := Species.build_meshes()
	_viewport = SubViewport.new()
	_viewport.size = Vector2i(768, 768)
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
	var elevation := deg_to_rad(CAMERA_ELEVATION_DEG)
	camera.position = Vector3(
		0.0, sin(elevation) * CAMERA_DISTANCE_M, cos(elevation) * CAMERA_DISTANCE_M
	)
	camera.look_at(Vector3.ZERO, Vector3.UP)
	await _check_card_backfaces(camera)

	var stems := int(PATCH_EXTENT_M * PATCH_EXTENT_M / 10000.0 * STEMS_PER_HA)
	for species in [Species.CONIFER, Species.BROADLEAF]:
		var transforms := _scatter(stems, PATCH_EXTENT_M)
		var measured := {}
		for level in [0, 1]:
			var holder := Node3D.new()
			_viewport.add_child(holder)
			if level == 0:
				var variants: int = Species.VARIANT_COUNTS[species]
				for variant in range(variants):
					var subset: Array[Transform3D] = []
					for i in range(transforms.size()):
						if i % variants == variant:
							subset.append(transforms[i])
					if not subset.is_empty():
						_add(holder, catalogue[species][variant][0], subset, true)
			else:
				_add(holder, catalogue[species][0][1], transforms, false)
			measured[level] = await _crown_color(holder)
			holder.queue_free()
			await process_frame
		var weights := Vector3(0.2126, 0.7152, 0.0722)
		var near_luminance: float = (measured[0] as Vector3).dot(weights)
		var distant_luminance: float = (measured[1] as Vector3).dot(weights)
		var ratio := distant_luminance / maxf(near_luminance, 0.0001)
		print(
			"vegetation_level_match species=%d near=%.4f distant=%.4f ratio=%.3f"
			% [species, near_luminance, distant_luminance, ratio]
		)
		_expect(
			absf(ratio - 1.0) <= LUMINANCE_TOLERANCE,
			"the distant level must render to the near level's luminance, got ratio %.3f" % ratio
		)
	_viewport.queue_free()
	await process_frame
	print("Vegetation level match tests: %d failures" % _failures)
	quit(0 if _failures == 0 else 1)
