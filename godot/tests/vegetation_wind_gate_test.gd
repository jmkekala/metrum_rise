# SPDX-License-Identifier: GPL-2.0-only

## Holds tree sway to the close range the effect is authored for, on a real GPU.
##
## Sway is authored in metres and drawn in pixels. Nothing connected the two, so a stand kept its
## full 0.50 m offset out to `TREE_NEAR_M`, where one pixel covers 1.14 m of world. Motion that
## small cannot move a crown's outline; it only flips alpha-scissor pixels along every cutout
## edge in the stand, which is what a forest seen from altitude reads as a boil rather than as
## wind. The gate ends well before that floor, because sway of one or two pixels still reads as
## shimmer across a closed canopy. This regression measures the two ends of it: the stand must
## still move close up, and must be still at the far edge of the band.
##
## It also pins the gate against the failure that produced it. The first implementation read the
## fade from `PROJECTION_MATRIX`, which is not populated in the vertex stage, so it silently
## returned zero and removed the wind from the whole game. Shader compilation caught nothing,
## and no other test did either.
extends SceneTree

const Species = preload("res://scripts/renderers/tree_species.gd")

# The brush-painted density, which is the case the boil is visible in.
const STEMS_PER_HA := 625.0
const PATCH_EXTENT_M := 120.0
const VIEWPORT_PX := 768
const CAMERA_FOV_DEG := 70.0
const ELEVATION_DEG := 55.0
# Inside the gate's full-sway distance, and past its end. Both are read through this viewport
# and field of view, not the game's: the gate is screen-space, so the metric distance the same
# curve lands on moves with them. At 768 px and 70 degrees the full offset covers 62 m.
const NEAR_DISTANCE_M := 55.0
const FAR_DISTANCE_M := 300.0
# Share of crown pixels the wind is allowed to move, as a luminance change of more than
# LUMINANCE_STEP. Close up the sway is the liveliness the near level exists for; past the band
# anything above the floor is the boil itself.
const NEAR_MOTION_MIN := 0.30
const FAR_MOTION_MAX := 0.02
const LUMINANCE_STEP := 0.02

var _failures := 0
var _viewport: SubViewport
var _camera: Camera3D

func _initialize() -> void:
	call_deferred("_run")

func _expect(condition: bool, message: String) -> void:
	if not condition:
		_failures += 1
		push_error(message)

## Deterministic lattice with hash jitter, matching `vegetation_level_match_test`.
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

func _add(holder: Node3D, mesh: Mesh, transforms: Array[Transform3D]) -> void:
	var mm := MultiMesh.new()
	mm.transform_format = MultiMesh.TRANSFORM_3D
	mm.use_colors = true
	mm.mesh = mesh
	mm.instance_count = transforms.size()
	for i in range(transforms.size()):
		mm.set_instance_transform(i, transforms[i])
		mm.set_instance_color(i, Color.WHITE)
	var node := MultiMeshInstance3D.new()
	node.multimesh = mm
	holder.add_child(node)

## Wind held at one phase. `wind_speed` zero drops TIME out of the term and leaves the offset a
## fixed function of position, so a displaced stand can be compared against a still one without
## either sample depending on when the frame happened to be captured.
func _set_strength(value: float) -> void:
	for material in [Species._wind_branch_material(), Species._foliage_material()]:
		material.set_shader_parameter("wind_strength", value)
		material.set_shader_parameter("wind_speed", 0.0)

## Crown mask and luminance for one frame. The mask pass paints sky and ground one flat colour,
## so a pixel is a crown pixel exactly when it is not that colour.
func _frame() -> Array:
	await process_frame
	await process_frame
	await RenderingServer.frame_post_draw
	var image := _viewport.get_texture().get_image()
	var mask := PackedByteArray()
	var luminance := PackedFloat32Array()
	var count := image.get_width() * image.get_height()
	mask.resize(count)
	luminance.resize(count)
	var i := 0
	for y in range(image.get_height()):
		for x in range(image.get_width()):
			var c := image.get_pixel(x, y)
			mask[i] = 0 if (c.r > 0.30 and c.b > 0.30 and c.g < 0.30) else 1
			luminance[i] = c.r * 0.2126 + c.g * 0.7152 + c.b * 0.0722
			i += 1
	return [mask, luminance]

## Share of the stand's pixels the wind moves at this camera distance. Inside a closed canopy the
## outline barely shifts, so what is counted is interior pixels changing, not the silhouette.
func _motion_at(distance_m: float) -> float:
	var elevation := deg_to_rad(ELEVATION_DEG)
	_camera.position = Vector3(
		0.0, sin(elevation) * distance_m, cos(elevation) * distance_m
	)
	_camera.look_at(Vector3.ZERO, Vector3.UP)
	_set_strength(0.0)
	var still: Array = await _frame()
	_set_strength(1.0)
	var blown: Array = await _frame()
	var still_mask: PackedByteArray = still[0]
	var still_lum: PackedFloat32Array = still[1]
	var blown_mask: PackedByteArray = blown[0]
	var blown_lum: PackedFloat32Array = blown[1]
	var union := 0
	var moved := 0
	for i in range(still_mask.size()):
		if still_mask[i] == 1 or blown_mask[i] == 1:
			union += 1
			if absf(still_lum[i] - blown_lum[i]) > LUMINANCE_STEP:
				moved += 1
	return float(moved) / maxf(float(union), 1.0)

func _run() -> void:
	if DisplayServer.get_name() == "headless":
		push_error("This regression needs a rendering display; the dummy renderer draws no pixels.")
		quit(2)
		return
	var catalogue := Species.build_meshes()
	_viewport = SubViewport.new()
	_viewport.size = Vector2i(VIEWPORT_PX, VIEWPORT_PX)
	_viewport.own_world_3d = true
	_viewport.render_target_update_mode = SubViewport.UPDATE_ALWAYS
	root.add_child(_viewport)

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
	var ground_material := StandardMaterial3D.new()
	ground_material.shading_mode = BaseMaterial3D.SHADING_MODE_UNSHADED
	ground_material.albedo_color = Color(1.0, 0.0, 1.0)
	ground.material_override = ground_material
	_viewport.add_child(ground)

	var light := DirectionalLight3D.new()
	light.rotation_degrees = Vector3(-42.0, 35.0, 0.0)
	_viewport.add_child(light)

	_camera = Camera3D.new()
	_camera.fov = CAMERA_FOV_DEG
	_camera.far = 20000.0
	_viewport.add_child(_camera)
	_camera.current = true

	var stems := int(PATCH_EXTENT_M * PATCH_EXTENT_M / 10000.0 * STEMS_PER_HA)
	var transforms := _scatter(stems, PATCH_EXTENT_M)
	var holder := Node3D.new()
	_viewport.add_child(holder)
	var variants: int = Species.VARIANT_COUNTS[Species.CONIFER]
	for variant in range(variants):
		var subset: Array[Transform3D] = []
		for i in range(transforms.size()):
			if i % variants == variant:
				subset.append(transforms[i])
		if not subset.is_empty():
			_add(holder, catalogue[Species.CONIFER][variant][0], subset)

	var near_motion: float = await _motion_at(NEAR_DISTANCE_M)
	var far_motion: float = await _motion_at(FAR_DISTANCE_M)
	print(
		"vegetation_wind_gate near=%.0fm moved=%.4f far=%.0fm moved=%.4f"
		% [NEAR_DISTANCE_M, near_motion, FAR_DISTANCE_M, far_motion]
	)
	_expect(
		near_motion >= NEAR_MOTION_MIN,
		"the stand must still sway close up, got %.4f moved" % near_motion
	)
	_expect(
		far_motion <= FAR_MOTION_MAX,
		"sway past the gate's band must not reach the screen, got %.4f moved" % far_motion
	)
	_viewport.queue_free()
	await process_frame
	print("Vegetation wind gate tests: %d failures" % _failures)
	quit(0 if _failures == 0 else 1)
