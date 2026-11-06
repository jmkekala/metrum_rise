# SPDX-License-Identifier: GPL-2.0-only

## Headless-safe reflection proxy and mip-filter regressions; no frame capture or rendering.
extends SceneTree

const Reflection := preload("res://scripts/renderers/water_reflection.gd")

class TerrainSource extends Node3D:
	var patches := {}

class VoteSource extends Node:
	var sources := PackedInt64Array([0, 0, 1])
	var marches := 0
	var busy := false
	var march_busy := false

	func get_water_reflection_sources(_previous: PackedInt64Array) -> PackedInt64Array:
		return PackedInt64Array() if busy else sources.duplicate()

	func get_first_water_surfaces(_origin: Vector3, rays: PackedVector3Array, _reach: float) -> PackedFloat32Array:
		marches += 1
		var levels := PackedFloat32Array()
		if not march_busy:
			levels.resize(rays.size())
			levels.fill(2.0)
		return levels

	func select_water_reflection_planes(_levels: PackedFloat32Array, _weights: PackedFloat32Array, _previous: PackedFloat32Array) -> PackedFloat32Array:
		return PackedFloat32Array([2.0, NAN])

var _failures := 0

func _initialize() -> void:
	call_deferred("_run")

func _expect(condition: bool, message: String) -> void:
	if not condition:
		_failures += 1
		push_error(message)

# Linear test texture: an exact integer-mip sample reads the inset-clamped coordinate.
func _sample_ramp(uv: float, lod: float) -> float:
	var lo := floorf(lod)
	var hi := minf(lo + 1.0, 6.0)
	return lerpf(clampf(uv, 0.5 * pow(2.0, lo) / 64.0, 1.0 - 0.5 * pow(2.0, lo) / 64.0),
		clampf(uv, 0.5 * pow(2.0, hi) / 64.0, 1.0 - 0.5 * pow(2.0, hi) / 64.0), lod - lo)

# Godot's edge AA with FRAGCOORD has no mip boost. Unclamped alpha = 0.5 + distance
# therefore gives coverage = clamp(0.5 + distance / fwidth(distance), 0, 1).
func _clip_coverage(distance_m: float, pixel_span_m: float) -> float:
	return clampf(0.5 + distance_m / maxf(pixel_span_m, 0.0001), 0.0, 1.0)

func _run() -> void:
	_test_vote_cache()
	var terrain_code := FileAccess.get_file_as_string("res://assets/materials/terrain.gdshader")
	_expect(terrain_code.contains("alpha_to_coverage_and_one, depth_draw_always"),
		"mirror coverage must write depth on surviving MSAA samples")
	_expect(not terrain_code.contains("ALPHA_SCISSOR_THRESHOLD")
		and terrain_code.contains("ALPHA = 0.5 + clip_distance")
		and terrain_code.contains("clip_distance <= -0.5 * clip_span")
		and terrain_code.contains("ALPHA_TEXTURE_COORDINATE = FRAGCOORD.xy"),
		"mirror clip must retain unclamped derivatives without a scissor cutoff or mip boost")
	for span in [0.01, 0.25, 1.0, 8.0]:
		_expect(_clip_coverage(-0.5 * span, span) == 0.0,
			"cheap lake-bed discard must reject only zero analytic coverage")
		_expect(_clip_coverage(0.0, span) == 0.5, "coverage must centre on the exact water plane")
		_expect(is_equal_approx(_clip_coverage(-0.25 * span, span), 0.25)
			and is_equal_approx(_clip_coverage(0.25 * span, span), 0.75),
			"clip feather must follow pixel scale, including steep subpixel shores")
	var renderer := Reflection.new()
	var terrain := TerrainSource.new()
	root.add_child(terrain)
	var source := MeshInstance3D.new()
	source.mesh = PlaneMesh.new()
	var material := ShaderMaterial.new()
	material.shader = load("res://assets/materials/terrain.gdshader")
	material.set_shader_parameter("height_scale", 20.0)
	source.material_override = material
	terrain.add_child(source)
	terrain.patches[Vector2i.ZERO] = {"node": source}
	renderer._terrain = terrain
	renderer._world = World3D.new()
	renderer._proxy_frame = 1
	renderer._sync_proxies()
	var id := source.get_instance_id()
	_expect(renderer._proxies[id][1] == source.mesh.get_rid(), "mirror must share the displayed terrain geometry")
	var copy: ShaderMaterial = renderer._grid_materials[material]
	# A texture/material upload must bypass the round robin and invalidate the old copy.
	renderer._proxy_frame = 2
	source.mesh = PlaneMesh.new()
	material.set_shader_parameter("height_scale", 12.0)
	renderer._on_terrain_patch_will_change(Vector2i.ZERO)
	renderer._sync_proxies()
	copy = renderer._grid_materials[material]
	_expect(renderer._proxies[id][1] == source.mesh.get_rid(), "changed shore mesh must sync in the same frame")
	_expect(copy.get_shader_parameter("height_scale") == 12.0, "changed source parameters must refresh the mirror")
	var code := FileAccess.get_file_as_string("res://scripts/shaders/vegetation_impostor.gdshader")
	_expect(code.contains("atlas_sample(sampler2DArray") and not code.contains("exp2(ceil(lod))"),
		"atlas filtering must blend separately clamped integer mips")
	for boundary in range(1, 6):
		_expect(absf(_sample_ramp(0.0, boundary - 0.00001) - _sample_ramp(0.0, boundary + 0.00001)) < 0.0001,
			"mip filtering must remain continuous at LOD %d" % boundary)
	# Remove a patch outside its cleanup slice: the signal must drop it in this frame.
	renderer._proxy_frame = (id + 1) % Reflection.PROXY_SYNC_FRAMES
	terrain.patches.erase(Vector2i.ZERO)
	source.visible = false
	renderer._on_terrain_patch_will_change(Vector2i.ZERO)
	renderer._sync_proxies()
	_expect(not renderer._proxies.has(id), "removed shore proxy must not persist for ten frames")
	renderer._on_terrain_reset()
	renderer.free()
	terrain.free()
	print("water_reflection_math_test: %s" % ("PASS" if _failures == 0 else "FAIL"))
	quit(_failures)


func _test_vote_cache() -> void:
	var renderer := Reflection.new()
	var simulation := VoteSource.new()
	var viewport := SubViewport.new()
	viewport.size = Vector2i(800, 600)
	root.add_child(viewport)
	var camera := Camera3D.new()
	viewport.add_child(camera)
	camera.position = Vector3(0.0, 100.0, 0.0)
	camera.rotation.x = -0.6
	renderer._simulation = simulation
	renderer._vote_planes(camera)
	renderer._vote_planes(camera)
	_expect(simulation.marches == 1, "unchanged view must keep the last vote")
	camera.position.x += 0.00001
	renderer._vote_planes(camera)
	_expect(simulation.marches == 1, "sub-epsilon motion must reuse the vote")
	camera.position.x += 0.001
	renderer._vote_planes(camera)
	_expect(simulation.marches == 2, "accumulated camera movement must invalidate")
	camera.rotation.y += 0.01
	renderer._vote_planes(camera)
	camera.fov += 1.0
	renderer._vote_planes(camera)
	viewport.size.x += 1
	renderer._vote_planes(camera)
	camera.h_offset += 1.0
	renderer._vote_planes(camera)
	_expect(simulation.marches == 6, "rotation, projection, viewport and offsets must invalidate")
	for index in [0, 1]:
		simulation.sources[index] += 1
		renderer._vote_planes(camera)
	_expect(simulation.marches == 8, "terrain and water revisions must invalidate")
	simulation.busy = true
	camera.position.x += 1.0
	_expect(renderer._vote_planes(camera)[0] == 2.0, "busy core must preserve the last levels")
	simulation.busy = false
	simulation.march_busy = true
	renderer._vote_planes(camera)
	simulation.march_busy = false
	renderer._vote_planes(camera)
	_expect(simulation.marches == 10, "failed march must retry instead of caching the new pose")
	simulation.sources[1] += 1
	simulation.sources[2] = 0
	_expect(is_nan(renderer._vote_planes(camera)[0]), "removing water must clear the old planes")
	camera.position.x += 1.0
	renderer._vote_planes(camera)
	_expect(simulation.marches == 10, "dry worlds must skip marching even while moving")
	simulation.sources[1] += 1
	simulation.sources[2] = 1
	renderer._vote_planes(camera)
	renderer._on_terrain_reset()
	renderer._vote_planes(camera)
	_expect(simulation.marches == 12, "water additions and world replacement must invalidate")
	renderer.free()
	simulation.free()
	viewport.free()
