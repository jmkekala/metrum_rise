# SPDX-License-Identifier: GPL-3.0-or-later

## Headless-safe reflection proxy and mip-filter regressions; no frame capture or rendering.
extends SceneTree

const Reflection := preload("res://scripts/renderers/water_reflection.gd")

class TerrainSource extends Node3D:
	var patches := {}

class CountedTerrain extends Node3D:
	var patches: CountedPatches

# Counts the visits a walk over the built patches makes (`RENDER-75`): iterating it (`for key in
# patches`) or reading it by key. The reflection's sync must touch only the patches marked since it
# last ran.
class CountedPatches extends RefCounted:
	var book := {}
	var reads := 0

	func _iter_init(iter: Array) -> bool:
		iter[0] = 0
		return book.size() > 0

	func _iter_next(iter: Array) -> bool:
		iter[0] += 1
		return iter[0] < book.size()

	func _iter_get(iter: Variant) -> Variant:
		reads += 1
		return book.keys()[iter]

	# Called as the sync calls a Dictionary's get(key, default): a script method takes precedence.
	@warning_ignore("native_method_override")
	func get(key: Variant, default: Variant = null) -> Variant:
		reads += 1
		return book.get(key, default)

	func has(key: Variant) -> bool:
		reads += 1
		return book.has(key)

# The water renderer's resident patches as the reflection's screen rectangle reads them (`RENDER-76`).
class WaterSource extends Node3D:
	var patches := CountedPatches.new()
	var resident_patch_lookup := CountedPatches.new()
	var terrain_patch_span_m := 256.0
	var terrain_world_size := Vector2.ZERO

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
	_test_fixed_target()
	_test_proxy_events()
	_test_rect_kept()
	_test_kept_hidden_meshes()
	# The mirror clip lives in the surface include the terrain shader pulls in.
	var terrain_code := FileAccess.get_file_as_string("res://assets/materials/terrain.gdshader") \
		+ FileAccess.get_file_as_string("res://scripts/shaders/terrain_surface.gdshaderinc")
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
	material.set_shader_parameter("site_concrete_brightness", 20.0)
	source.material_override = material
	terrain.add_child(source)
	terrain.patches[Vector2i.ZERO] = {"node": source}
	renderer._terrain = terrain
	renderer._world = World3D.new()
	renderer._on_terrain_patch_visibility_changed(Vector2i.ZERO)
	renderer._sync_proxies()
	var id := Vector2i.ZERO
	_expect(renderer._proxies[id][1] == source.mesh.get_rid(), "mirror must share the displayed terrain geometry")
	var copy: ShaderMaterial = renderer._grid_materials[material]
	var instance: RID = renderer._proxies[id][0]
	# A texture/material upload must refresh the copy in the same frame.
	source.mesh = PlaneMesh.new()
	material.set_shader_parameter("site_concrete_brightness", 12.0)
	material.set_shader_parameter("heightmap", ImageTexture.create_from_image(
		Image.create(4, 4, false, Image.FORMAT_RF)))
	renderer._on_terrain_patch_will_change(Vector2i.ZERO)
	renderer._sync_proxies()
	_expect(renderer._proxies[id][1] == source.mesh.get_rid(), "changed shore mesh must sync in the same frame")
	_expect(copy.get_shader_parameter("site_concrete_brightness") == 12.0, "changed source parameters must refresh the mirror")
	# In place (`RENDER-27` WR1b): the same copy and instance, holding what a new duplicate would.
	_expect(renderer._grid_materials[material] == copy and renderer._proxies[id][0] == instance,
		"a patch change must refresh the mirror's material and instance in place")
	# A changed parameter set (`TERRAIN-41` L1b): before the first vote it waits for the sync's
	# refresh; on a wet world the proxy takes the new mesh and values at once, nothing left marked.
	source.mesh = PlaneMesh.new()
	material.set_shader_parameter("terrain_debug_lod_step", 4.0)
	renderer._on_terrain_patch_parameters_changed(Vector2i.ZERO, [&"terrain_debug_lod_step"] as Array[StringName])
	_expect(renderer._dirty_terrain_patches.get(Vector2i.ZERO, false),
		"a parameter change before the proxies follow must wait for the refresh")
	renderer._sync_proxies()
	_expect(renderer._proxies[id][1] == source.mesh.get_rid()
		and copy.get_shader_parameter("terrain_debug_lod_step") == 4.0,
		"the refresh must give the proxy its new mesh and step")
	renderer._vote_sources = PackedInt64Array([0, 0, 1])
	source.mesh = PlaneMesh.new()
	material.set_shader_parameter("terrain_debug_lod_step", 8.0)
	material.set_shader_parameter("watermap_texture_size", Vector2(3.0, 5.0))
	renderer._on_terrain_patch_parameters_changed(Vector2i.ZERO, [&"terrain_debug_lod_step"] as Array[StringName])
	renderer._on_terrain_patch_parameters_changed(Vector2i.ZERO, [&"watermap_texture_size"] as Array[StringName])
	_expect(renderer._proxies[id][1] == source.mesh.get_rid()
		and copy.get_shader_parameter("terrain_debug_lod_step") == 8.0
		and copy.get_shader_parameter("watermap_texture_size") == Vector2(3.0, 5.0)
		and renderer._grid_materials[material] == copy and renderer._proxies[id][0] == instance
		and not renderer._dirty_terrain_patches.has(Vector2i.ZERO),
		"on a wet world a parameter change must reach the proxy in place, unmarked")
	# A refresh already pending for the patch stays the refresh.
	material.set_shader_parameter("site_concrete_brightness", 7.0)
	renderer._on_terrain_patch_will_change(Vector2i.ZERO)
	renderer._on_terrain_patch_parameters_changed(Vector2i.ZERO, [&"terrain_debug_lod_step"] as Array[StringName])
	renderer._sync_proxies()
	_expect(copy.get_shader_parameter("site_concrete_brightness") == 7.0,
		"a parameter change must not weaken a pending refresh")
	renderer._vote_sources = PackedInt64Array()
	material.set_shader_parameter("site_concrete_brightness", null)
	renderer._on_terrain_patch_will_change(Vector2i.ZERO)
	renderer._sync_proxies()
	var duplicate := material.duplicate() as ShaderMaterial
	for entry: Dictionary in material.shader.get_shader_uniform_list():
		var held: Variant = copy.get_shader_parameter(entry["name"])
		var fresh: Variant = duplicate.get_shader_parameter(entry["name"])
		_expect(typeof(held) == typeof(fresh) and held == fresh,
			"refreshed mirror parameter %s must equal a new duplicate's" % entry["name"])
	var code := FileAccess.get_file_as_string("res://scripts/shaders/vegetation_impostor.gdshader")
	_expect(code.contains("atlas_sample(sampler2DArray") and not code.contains("exp2(ceil(lod))"),
		"atlas filtering must blend separately clamped integer mips")
	for boundary in range(1, 6):
		_expect(absf(_sample_ramp(0.0, boundary - 0.00001) - _sample_ramp(0.0, boundary + 0.00001)) < 0.0001,
			"mip filtering must remain continuous at LOD %d" % boundary)
	# A removed patch: the signal must drop its proxy in this frame.
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


# The proxies follow the terrain's events (`RENDER-75`): a sync visits only the patches marked since
# the last one, however many are built, and leaves one proxy per drawn patch in the same frame.
func _test_proxy_events() -> void:
	const BUILT := 2000
	const SIDE := 50
	var renderer := Reflection.new()
	var terrain := CountedTerrain.new()
	root.add_child(terrain)
	var book := CountedPatches.new()
	terrain.patches = book
	var mesh := PlaneMesh.new()
	var drawn: Array[Vector2i] = []
	for index in BUILT:
		var key := Vector2i(index % SIDE, index / SIDE)
		var node := MeshInstance3D.new()
		node.mesh = mesh
		node.position = Vector3(key.x * 256.0, 0.0, key.y * 256.0)
		# 64 drawn; every other built patch is hidden, as after a long flight.
		node.visible = key.x < 8 and key.y < 8
		terrain.add_child(node)
		book.book[key] = {"node": node}
		if node.visible:
			drawn.append(key)
	renderer._terrain = terrain
	renderer._world = World3D.new()
	for key in drawn:
		renderer._on_terrain_patch_visibility_changed(key)
	book.reads = 0
	renderer._sync_proxies()
	_expect(renderer._proxies.size() == drawn.size() and renderer._proxy_visits == drawn.size(),
		"the first sync must mirror the %d drawn patches, visiting only them (visited %d, %d proxies)"
			% [drawn.size(), renderer._proxy_visits, renderer._proxies.size()])
	_expect(book.reads <= drawn.size(), "the first sync read %d built patches" % book.reads)
	# A frame with no event touches no patch.
	book.reads = 0
	for frame in 20:
		renderer._sync_proxies()
	_expect(book.reads == 0 and renderer._proxy_visits == 0,
		"syncs without events must visit no built patch (read %d)" % book.reads)
	# Shown and hidden in one frame: both proxies follow in that frame.
	var shown := Vector2i(20, 20)
	var hidden := drawn[5]
	(book.book[shown]["node"] as MeshInstance3D).visible = true
	renderer._on_terrain_patch_visibility_changed(shown)
	(book.book[hidden]["node"] as MeshInstance3D).visible = false
	renderer._on_terrain_patch_visibility_changed(hidden)
	book.reads = 0
	renderer._sync_proxies()
	_expect(renderer._proxies.has(shown) and not renderer._proxies.has(hidden),
		"a shown patch must gain and a hidden one lose its proxy in the same frame")
	_expect(book.reads == 2 and renderer._proxy_visits == 2,
		"two events must cost two visits (read %d)" % book.reads)
	var node: MeshInstance3D = book.book[shown]["node"]
	var proxy: Array = renderer._proxies[shown]
	_expect(proxy[1] == mesh.get_rid() and proxy[2] == node.global_transform
		and proxy[4] == node.custom_aabb and proxy[5] == node.extra_cull_margin,
		"a shown patch's proxy must hold its mesh, transform, box and margin")
	# Hiding the terrain node hides every patch's proxy; showing it brings them back.
	terrain.visible = false
	renderer._on_terrain_visibility_changed()
	renderer._sync_proxies()
	_expect(renderer._proxies.is_empty(), "a hidden terrain must leave no proxy")
	terrain.visible = true
	renderer._on_terrain_visibility_changed()
	renderer._sync_proxies()
	_expect(renderer._proxies.size() == drawn.size(), "a shown terrain must mirror its drawn patches again")
	renderer._on_terrain_reset()
	_expect(renderer._proxies.is_empty(), "a reset must free every proxy")
	renderer.free()
	terrain.free()

# The screen rectangle is kept by the water patches' events (`RENDER-76`): with nothing changed it
# reads no patch, a camera move projects the hull's outline (and a few patches by the near plane's
# cut), not every resident patch, and every rectangle equals the walk over every resident patch.
func _test_rect_kept() -> void:
	const SIDE := 60
	const SPAN := 256.0
	var renderer := Reflection.new()
	var water := WaterSource.new()
	water.terrain_world_size = Vector2(SIDE * SPAN, SIDE * SPAN)
	root.add_child(water)
	var dry := ArrayMesh.new()
	var wet := PlaneMesh.new()
	wet.size = Vector2(SPAN, SPAN)
	# Every terrain patch has a water patch: a dry one's bounds are its centre, a wet one's its
	# footprint (a lake and a river here). A disc of them is resident, as about a camera.
	for z in SIDE:
		for x in SIDE:
			var key := Vector2i(x, z)
			if Vector2(key - Vector2i(30, 30)).length() > 27.5:
				continue
			var node := MeshInstance3D.new()
			var lake := Vector2(key - Vector2i(38, 24)).length() < 6.0 or x == 17
			node.mesh = wet if lake else dry
			node.position = Vector3((x + 0.5) * SPAN - water.terrain_world_size.x * 0.5, 0.0,
				(z + 0.5) * SPAN - water.terrain_world_size.y * 0.5)
			water.add_child(node)
			water.patches.book[key] = {"node": node}
			water.resident_patch_lookup.book[key] = true
	var resident := water.resident_patch_lookup.book.size()
	renderer._water = water
	renderer._rect_hull = WaterScreenHull.new()
	for key: Vector2i in water.resident_patch_lookup.book:
		renderer._on_water_patch_bounds_changed(key)
	renderer._sync_rect_hull()
	var viewport := SubViewport.new()
	viewport.size = Vector2i(1920, 1080)
	root.add_child(viewport)
	var camera := Camera3D.new()
	viewport.add_child(camera)
	camera.near = 0.5
	camera.far = 20000.0
	# Poses: high and low obliques along and across the grid (hull edges then lie along the level
	# lines), a turn, straight down, at the horizon, outside the resident disc looking in.
	var poses := [
		[Vector3(120.0, 900.0, 40.0), Vector3(-0.75, -PI * 0.5, 0.0)],
		[Vector3(120.0, 900.0, 40.0), Vector3(-0.75, 0.0, 0.0)],
		[Vector3(-1300.0, 60.0, 700.0), Vector3(-0.35, 0.9, 0.0)],
		[Vector3(400.0, 25.0, -200.0), Vector3(-0.12, 2.4, 0.0)],
		[Vector3(0.0, 3000.0, 0.0), Vector3(-PI * 0.5, 0.0, 0.0)],
		[Vector3(2000.0, 200.0, 2000.0), Vector3(0.0, -2.2, 0.0)],
		[Vector3(9000.0, 1500.0, 0.0), Vector3(-0.3, PI * 0.5, 0.0)],
	]
	for pose: Array in poses:
		camera.position = pose[0]
		camera.rotation = pose[1]
		for plane: float in [0.0, 12.5]:
			var walked: Rect2 = renderer._water_screen_rect_walk(camera, plane)
			water.resident_patch_lookup.reads = 0
			water.patches.reads = 0
			renderer._rect_points = 0
			renderer._rect_walks = 0
			var kept: Rect2 = renderer._water_screen_rect(camera, plane, 0)
			_expect(kept == walked, "pose %s plane %.1f: kept %s, walk %s" % [pose, plane, kept, walked])
			_expect(renderer._rect_walks == 0,
				"pose %s plane %.1f: the hull must settle the rectangle without the walk" % [pose, plane])
			var reads: int = water.resident_patch_lookup.reads + water.patches.reads
			_expect(reads <= 60 and renderer._rect_points < resident / 4,
				"pose %s: a camera move read %d of %d resident patches and projected %d points"
					% [pose, reads, resident, renderer._rect_points])
			# Nothing changed: no patch read, no point projected.
			water.resident_patch_lookup.reads = 0
			water.patches.reads = 0
			renderer._rect_points = 0
			for frame in 10:
				_expect(renderer._water_screen_rect(camera, plane, 0) == walked, "a kept frame must repeat the rectangle")
			_expect(water.resident_patch_lookup.reads + water.patches.reads == 0 and renderer._rect_points == 0,
				"frames without a change must read no patch (read %d, projected %d)"
					% [water.resident_patch_lookup.reads + water.patches.reads, renderer._rect_points])
	# Events: the lake drains from its edge and patches leave; the rectangle follows exactly.
	camera.position = Vector3(120.0, 900.0, 40.0)
	camera.rotation = Vector3(-0.75, -PI * 0.5, 0.0)
	for key: Vector2i in [Vector2i(44, 24), Vector2i(57, 30), Vector2i(30, 3), Vector2i(31, 3)]:
		water.resident_patch_lookup.book.erase(key)
		renderer._on_water_patch_bounds_changed(key)
	(water.patches.book[Vector2i(38, 24)]["node"] as MeshInstance3D).mesh = dry
	renderer._on_water_patch_bounds_changed(Vector2i(38, 24))
	renderer._sync_rect_hull()
	_expect(renderer._water_screen_rect(camera, 0.0, 0) == renderer._water_screen_rect_walk(camera, 0.0),
		"the rectangle must follow residency and bounds events")
	renderer._on_water_patches_cleared()
	_expect(renderer._water_screen_rect(camera, 0.0, 0) == Rect2(), "no patch held, no rectangle")
	renderer.free()
	water.free()
	viewport.free()

# Kept water meshes (`TERRAIN-63`): hidden patches holding a wet mesh (applied by the kept cause,
# which emits their bounds' change) never enter the rectangle's book; one shown enters with its mesh
# box at once, and hidden again it leaves. The hull holds exactly the shown patches, and the kept
# rectangle equals the walk's.
func _test_kept_hidden_meshes() -> void:
	const SIDE := 40
	const SPAN := 256.0
	var renderer := Reflection.new()
	var water := WaterSource.new()
	water.terrain_world_size = Vector2(SIDE * SPAN, SIDE * SPAN)
	root.add_child(water)
	var dry := ArrayMesh.new()
	var wet := PlaneMesh.new()
	wet.size = Vector2(SPAN, SPAN)
	var hidden_wet: Array[Vector2i] = []
	for z in SIDE:
		for x in SIDE:
			var key := Vector2i(x, z)
			var shown := Vector2(key - Vector2i(20, 20)).length() <= 12.0
			var node := MeshInstance3D.new()
			# A ring of lakes beyond the shown disc: hidden, wet, holding their kept meshes.
			node.mesh = wet if not shown or x == 20 else dry
			node.visible = shown
			node.position = Vector3((x + 0.5) * SPAN - water.terrain_world_size.x * 0.5, 0.0,
				(z + 0.5) * SPAN - water.terrain_world_size.y * 0.5)
			water.add_child(node)
			water.patches.book[key] = {"node": node}
			if shown:
				water.resident_patch_lookup.book[key] = true
			else:
				hidden_wet.append(key)
	renderer._water = water
	renderer._rect_hull = WaterScreenHull.new()
	for key: Vector2i in water.patches.book:
		renderer._on_water_patch_bounds_changed(key)
	renderer._sync_rect_hull()
	var shown_count := water.resident_patch_lookup.book.size()
	_expect(renderer._rect_hull.patch_count() == shown_count,
		"the hull holds the shown patches only (%d, shown %d, hidden wet %d)"
			% [renderer._rect_hull.patch_count(), shown_count, hidden_wet.size()])
	var viewport := SubViewport.new()
	viewport.size = Vector2i(1920, 1080)
	root.add_child(viewport)
	var camera := Camera3D.new()
	viewport.add_child(camera)
	camera.near = 0.5
	camera.far = 20000.0
	camera.position = Vector3(0.0, 1200.0, 2500.0)
	camera.rotation = Vector3(-0.6, 0.0, 0.0)
	_expect(renderer._water_screen_rect(camera, 0.0, 0) == renderer._water_screen_rect_walk(camera, 0.0),
		"kept hidden meshes: the kept rectangle equals the walk's")
	# A kept mesh applied again to a hidden patch (its bounds' event): still not in the book.
	renderer._on_water_patch_bounds_changed(hidden_wet[0])
	renderer._sync_rect_hull()
	_expect(renderer._rect_hull.patch_count() == shown_count, "a hidden patch's mesh event adds nothing")
	# Shown with its kept mesh: in the book at once with its mesh box; hidden again: out.
	var key: Vector2i = hidden_wet[hidden_wet.size() / 2]
	(water.patches.book[key]["node"] as MeshInstance3D).visible = true
	water.resident_patch_lookup.book[key] = true
	renderer._on_water_patch_bounds_changed(key)
	renderer._sync_rect_hull()
	_expect(renderer._rect_hull.patch_count() == shown_count + 1, "a shown kept patch enters the book")
	_expect(renderer._water_screen_rect(camera, 0.0, 0) == renderer._water_screen_rect_walk(camera, 0.0),
		"a shown kept patch: the kept rectangle equals the walk's")
	(water.patches.book[key]["node"] as MeshInstance3D).visible = false
	water.resident_patch_lookup.book.erase(key)
	renderer._on_water_patch_bounds_changed(key)
	renderer._sync_rect_hull()
	_expect(renderer._rect_hull.patch_count() == shown_count, "hidden again, it leaves the book")
	renderer.free()
	water.free()
	viewport.free()

# The mirror's target keeps one size across crops (`RENDER-27` WR1a), and every texel of the
# crop's block, its top left, sees the ray it saw in a target of the crop's own size.
func _test_fixed_target() -> void:
	var renderer := Reflection.new()
	var main := SubViewport.new()
	main.size = Vector2i(1920, 1080)
	root.add_child(main)
	var main_camera := Camera3D.new()
	main.add_child(main_camera)
	main_camera.position = Vector3(10.0, 120.0, -30.0)
	main_camera.rotation = Vector3(-0.4, 0.7, 0.0)
	var mirror := SubViewport.new()
	var camera := Camera3D.new()
	mirror.add_child(camera)
	root.add_child(mirror)
	var alone_viewport := SubViewport.new()
	var alone := Camera3D.new()
	alone_viewport.add_child(alone)
	root.add_child(alone_viewport)
	var screen := Vector2(main.size)
	var target := Vector2i((screen * (1.0 + 2.0 * Reflection.CROP_MARGIN) * Reflection.RESOLUTION_SCALE).round())
	var height := 2.0 * main_camera.near * tan(deg_to_rad(main_camera.fov) * 0.5)
	var width := height * screen.x / screen.y
	for crop: Rect2 in [Rect2(0.1, 0.4, 0.5, 0.35), Rect2(0.62, 0.55, 0.31, 0.21),
			Rect2(-0.03, -0.03, 1.06, 1.06), Rect2(0.3, 0.97, 0.001, 0.002)]:
		renderer._place_camera([mirror, camera, 1.0, true], 0, main_camera, main, crop)
		_expect(mirror.size == target, "the mirror's target must keep one size across crops")
		# The crop alone, as a target of its own size placed it before.
		var size := Vector2i((screen * crop.size * Reflection.RESOLUTION_SCALE).round()).max(Vector2i.ONE)
		alone_viewport.size = size
		alone.global_transform = camera.global_transform
		alone.projection = Camera3D.PROJECTION_FRUSTUM
		alone.keep_aspect = Camera3D.KEEP_HEIGHT
		alone.size = height * crop.size.y
		alone.frustum_offset = Vector2((crop.get_center().x - 0.5) * width, (0.5 - crop.get_center().y) * height)
		alone.near = camera.near
		alone.far = camera.far
		for local: Vector3 in [Vector3(0.0, 0.0, -50.0), Vector3(-20.0, 7.0, -80.0), Vector3(35.0, -12.0, -400.0)]:
			var point := camera.global_transform * local
			var texel := _texel(camera, point, Vector2(target))
			var texel_alone := _texel(alone, point, Vector2(size))
			_expect(texel.distance_to(texel_alone) < 0.01,
				"crop %s: texel %s must be the crop-sized target's %s" % [crop, texel, texel_alone])
	renderer.free()
	main.free()
	mirror.free()
	alone_viewport.free()

# `point`'s position in texels of a target of `size` drawn by `camera`, as the water samples it.
func _texel(camera: Camera3D, point: Vector3, size: Vector2) -> Vector2:
	var clip: Vector4 = (camera.get_camera_projection() * Projection(camera.global_transform.affine_inverse())
		* Vector4(point.x, point.y, point.z, 1.0))
	return (Vector2(0.5, 0.5) + Vector2(0.5, -0.5) * Vector2(clip.x, clip.y) / clip.w) * size

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
