# SPDX-License-Identifier: GPL-2.0-only

## Asks one question: does a MultiMeshInstance3D culled by its visibility range still cast?
##
## The vegetation patch already carries a lathe MultiMesh over the same trees as its near
## level, hidden behind visibility_range_begin. Making that instance the shadow caster costs
## no new geometry only if Godot keeps casting from it while the range hides it. This builds
## a ground plane, one sun and one caster, and counts darkened ground pixels against a control
## that does not cast. The caster is SHADOWS_ONLY throughout, so the only thing that can
## change between cases is the shadow itself, never the box's own pixels.
extends SceneTree

var caster: MultiMeshInstance3D


func _init() -> void:
	_build_world()
	await process_frame
	caster.cast_shadow = GeometryInstance3D.SHADOW_CASTING_SETTING_SHADOWS_ONLY
	var control := await _capture()
	caster.cast_shadow = GeometryInstance3D.SHADOW_CASTING_SETTING_OFF
	# Rendered first so a failure to darken anything at all is visible before the real case.
	var in_range := await _capture()
	caster.cast_shadow = GeometryInstance3D.SHADOW_CASTING_SETTING_SHADOWS_ONLY
	var control_again := await _capture()
	# The camera sits about 20 m away, so this band starts far behind it and the instance is
	# out of range: exactly the state a near-band patch keeps its lathe level in.
	caster.visibility_range_begin = 1000.0
	caster.visibility_range_end = 2000.0
	var out_of_range := await _capture()
	var reference := _darkened(in_range, control)
	var culled := _darkened(in_range, out_of_range)
	var drift := _darkened(in_range, control_again)
	print("PROBE shadow_pixels_in_range=%d out_of_range=%d repeat=%d" % [reference, culled, drift])
	print("PROBE casts_while_culled=%s" % str(culled > reference / 2))
	quit()


## Pixels of `lit` that `shadowed` darkens by more than a twentieth. Both frames come from the
## same camera, so a pixel only changes where a shadow landed on it.
func _darkened(lit: Image, shadowed: Image) -> int:
	var count := 0
	var size := lit.get_size()
	for y in range(0, size.y, 2):
		for x in range(0, size.x, 2):
			if shadowed.get_pixel(x, y).get_luminance() < lit.get_pixel(x, y).get_luminance() - 0.05:
				count += 1
	return count


func _build_world() -> void:
	var world := Node3D.new()
	root.add_child(world)

	var ground := MeshInstance3D.new()
	var plane := PlaneMesh.new()
	plane.size = Vector2(60.0, 60.0)
	ground.mesh = plane
	var white := StandardMaterial3D.new()
	white.albedo_color = Color(0.9, 0.9, 0.9)
	ground.material_override = white
	world.add_child(ground)

	var box := BoxMesh.new()
	box.size = Vector3(4.0, 4.0, 4.0)
	var mm := MultiMesh.new()
	mm.transform_format = MultiMesh.TRANSFORM_3D
	mm.mesh = box
	mm.instance_count = 1
	mm.set_instance_transform(0, Transform3D(Basis(), Vector3(0.0, 4.0, 0.0)))
	caster = MultiMeshInstance3D.new()
	caster.multimesh = mm
	world.add_child(caster)

	var sun := DirectionalLight3D.new()
	sun.shadow_enabled = true
	sun.directional_shadow_max_distance = 120.0
	sun.rotation = Vector3(deg_to_rad(-55.0), deg_to_rad(20.0), 0.0)
	world.add_child(sun)

	var environment := Environment.new()
	environment.background_mode = Environment.BG_COLOR
	environment.background_color = Color(0.2, 0.3, 0.5)
	environment.ambient_light_source = Environment.AMBIENT_SOURCE_COLOR
	environment.ambient_light_color = Color(0.4, 0.4, 0.4)
	environment.ambient_light_energy = 0.6
	var camera := Camera3D.new()
	camera.environment = environment
	camera.position = Vector3(0.0, 14.0, 20.0)
	camera.rotation = Vector3(deg_to_rad(-32.0), 0.0, 0.0)
	world.add_child(camera)
	camera.make_current()


## One settled frame, read back out of the framebuffer.
func _capture() -> Image:
	for i in range(4):
		await process_frame
	await RenderingServer.frame_post_draw
	return root.get_texture().get_image()
