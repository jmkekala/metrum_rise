# SPDX-License-Identifier: GPL-2.0-only

## Street lights along roads and footpaths (`ROAD-59`). Rust places them with each road chunk
## (`street_lights` in the chunk data: five floats per light, chunk-local pole base x, y, z, yaw,
## model kind); this draws a chunk's lights as one MultiMesh per 64 m cell, model and level of
## detail. Each model is merged into a single surface, so a batch is one draw call.
extends Node3D

const SceneLightingConfig := preload("res://scripts/core/scene_lighting.gd")
const SceneLighting := preload("res://scripts/core/scene_lighting.gd")
const SHADER := preload("res://scripts/shaders/street_light.gdshader")
const GLOW_SHADER := preload("res://scripts/shaders/street_light_glow.gdshader")
const LampGrid := preload("res://scripts/renderers/street_lamp_grid.gd")
const MODEL_DIR := "res://assets/models/street_lights/"
# Indexed by the Rust `StreetLightKind`.
const MODELS: Array[String] = ["SL-RES-STEEL", "SL-RES-WOOD", "SL-MAIN", "SL-MEDIAN", "SL-PATH"]
# Lamp colour by kind (sRGB): 3000 K LED, 2000 K sodium on the older wooden poles, 4000 K LED on
# main roads and medians, 3000 K on paths. From the models' metadata.
const LAMP_COLORS: Array[Color] = [
	Color(1.0, 0.82, 0.62),
	Color(1.0, 0.62, 0.28),
	Color(0.96, 0.92, 0.86),
	Color(0.96, 0.92, 0.86),
	Color(1.0, 0.82, 0.62),
]
const CELL_M := 64.0
# Distance bands from each cell's centre, one per level of detail (the GLBs' _LOD0.._LOD2).
const LOD_RANGES: Array[Vector2] = [Vector2(0.0, 140.0), Vector2(140.0, 420.0), Vector2(420.0, 1400.0)]
const LOD_HYSTERESIS_M := 10.0
const PART_ALPHA := {"pole": 0.0, "housing": 0.5, "lamp": 1.0}
# Light points relative to the pole base in model space (arm reach along +X, lens height), from
# the models' metadata; the median pole has a lamp at each end.
const LIGHT_POINTS: Array = [
	[Vector3(1.45, 7.95, 0.0)],
	[Vector3(1.4, 8.05, 0.0)],
	[Vector3(2.55, 9.95, 0.0)],
	[Vector3(3.4, 11.15, 0.0), Vector3(-3.4, 11.15, 0.0)],
	[Vector3(0.0, 4.4, 0.0)],
]
# Real lights go to the lamps nearest the camera; the rest are evaluated in lit materials. Clustered
# Forward+ handles this many unshadowed spot lights at a nearly fixed cost once any is in view.
const POOL_SIZE := 32
# Real lights reach this far when fewer than POOL_SIZE lamps are inside it; otherwise the handoff
# moves in to the farthest lamp that got a real light, so no lamp is left with neither.
const POOL_RADIUS_M := 80.0
const POOL_FADE_M := 30.0
# Both real and material lamps reach this far past their head height.
const LAMP_RANGE_MARGIN_M := 20.0
const POOL_REFRESH_S := 0.2
const GRID_RECENTRE_CELLS := 16
const POOL_REFRESH_MOVE_M := 3.0
# Lamps light up below this sun elevation (matches street_light.gdshader).
const LAMPS_ON_ELEVATION_DEG := 2.0
const LAMP_ENERGY := 10.0
const LAMP_ANGLE_DEG := 80.0
# Past FAR_FADE_M (camera distance) the exact lamps hand over to a coarse map of mean ground light,
# FAR_SIDE cells of FAR_CELL_M (8 km) around the ground point the camera looks at, re-centred after
# FAR_RECENTRE_M. The exact grid reaches at least 768 m from the camera, past the band's end.
const FAR_CELL_M := 16.0
const FAR_SIDE := 512
const FAR_RECENTRE_M := 1024.0
const FAR_FADE_M := Vector2(500.0, 700.0)
# The look ray's ground point is capped this far ahead, so a view along the horizon stays centred.
const FAR_FOCUS_MAX_M := 3000.0

# [kind][lod] -> merged ArrayMesh, built once from the GLBs.
static var _meshes: Array = []
static var _material: ShaderMaterial
static var _glow_mesh: QuadMesh

var _chunks: Dictionary = {}
# Per chunk: world-space light points and their kinds, for the light pool.
var _chunk_points: Dictionary = {}
var _chunk_kinds: Dictionary = {}
var _pool: Array[SpotLight3D] = []
var _pool_age_s := INF
var _pool_camera := Vector3(INF, INF, INF)
var _grid_dirty := true
var _grid_cell := Vector2i(2147483647, 2147483647)
var _chunk_span_m := 512.0
var _chunk_origin := Vector2.ZERO
var _cell_texture: ImageTexture
var _kind_texture: ImageTexture
var _far_texture: ImageTexture
var _far_texels := PackedFloat32Array()
var _far_origin := Vector2.ZERO
var _far_dirty := true
var _far_upload := false
var _far_centre := Vector2(INF, INF)
# Per kind: linear colour * energy * ground flux / far cell area, the mean light one lamp adds.
var _far_kind_light := PackedColorArray()

func configure_chunks(span_m: float, origin: Vector2) -> void:
	if span_m != _chunk_span_m or origin != _chunk_origin:
		_chunk_span_m = span_m
		_chunk_origin = origin
		_grid_dirty = true
		_far_dirty = true

func _exit_tree() -> void:
	RenderingServer.global_shader_parameter_set("street_lamp_window", Vector4.ZERO)

# Reuse road chunk keys; pole ownership is by position, with at most 3.4 m arm overhang.
func _chunk_bounds(low: Vector2, high: Vector2) -> Rect2i:
	var first := Vector2i(((low - _chunk_origin) / _chunk_span_m).floor())
	var last := Vector2i(((high - _chunk_origin) / _chunk_span_m).floor())
	return Rect2i(first, last - first + Vector2i.ONE)

func _rebuild_grid(cell: Vector2i) -> void:
	_grid_cell = cell
	_grid_dirty = false
	var origin := Vector2(cell - Vector2i(LampGrid.SIDE / 2, LampGrid.SIDE / 2)) * LampGrid.CELL_M
	var end := origin + Vector2.ONE * LampGrid.SIDE * LampGrid.CELL_M
	# Largest range plus arm overhang; chunk ownership is at the pole, not the head.
	var bounds := _chunk_bounds(origin - Vector2.ONE * 35.0, end + Vector2.ONE * 35.0)
	var rows: Array = []
	for z in range(bounds.position.y, bounds.end.y):
		for x in range(bounds.position.x, bounds.end.x):
			var key := Vector2i(x, z)
			if not _chunk_points.has(key):
				continue
			var points: PackedVector3Array = _chunk_points[key]
			var kinds: PackedByteArray = _chunk_kinds[key]
			for i in points.size():
				var point := points[i]
				if (point.x >= origin.x - 32.0 and point.x <= end.x + 32.0
						and point.z >= origin.y - 32.0 and point.z <= end.y + 32.0):
					rows.append([point, int(kinds[i])])
	# Stable world order makes cell overflow independent of chunk upload order.
	rows.sort_custom(func(a, b):
		if a[0].x != b[0].x: return a[0].x < b[0].x
		if a[0].z != b[0].z: return a[0].z < b[0].z
		if a[0].y != b[0].y: return a[0].y < b[0].y
		return a[1] < b[1])
	var heads := PackedVector3Array()
	var kinds := PackedByteArray()
	var ranges := PackedFloat32Array()
	for row in rows:
		heads.append(row[0])
		kinds.append(row[1])
		ranges.append(LIGHT_POINTS[row[1]][0].y + LAMP_RANGE_MARGIN_M)
	var packed := LampGrid.pack_cells(heads, kinds, ranges, origin)
	var image := Image.create_from_data(LampGrid.SIDE * LampGrid.CAPACITY, LampGrid.SIDE,
		false, Image.FORMAT_RGBAF, packed.to_byte_array())
	if _cell_texture == null:
		_cell_texture = ImageTexture.create_from_image(image)
		RenderingServer.global_shader_parameter_set("street_lamp_cells", _cell_texture)
	else:
		_cell_texture.update(image)
	RenderingServer.global_shader_parameter_set("street_lamp_window", Vector4(origin.x, origin.y, LampGrid.CELL_M, LampGrid.SIDE))

# One RGBAF texel per lamp kind: linear colour * energy, as Godot feeds a real light's colour to
# the shader, and the range shared with the real light.
func _upload_kinds() -> void:
	var data := PackedFloat32Array()
	for kind in LAMP_COLORS.size():
		var color := LAMP_COLORS[kind].srgb_to_linear() * LAMP_ENERGY
		data.append_array([color.r, color.g, color.b,
			(LIGHT_POINTS[kind][0] as Vector3).y + LAMP_RANGE_MARGIN_M])
	_kind_texture = ImageTexture.create_from_image(Image.create_from_data(LAMP_COLORS.size(), 1,
		false, Image.FORMAT_RGBAF, data.to_byte_array()))
	RenderingServer.global_shader_parameter_set("street_lamp_kinds", _kind_texture)
	var cone_cos := cos(deg_to_rad(LAMP_ANGLE_DEG))
	_far_kind_light.clear()
	for kind in LAMP_COLORS.size():
		var height := (LIGHT_POINTS[kind][0] as Vector3).y
		var flux := LampGrid.ground_flux(height, height + LAMP_RANGE_MARGIN_M, cone_cos)
		_far_kind_light.append(LAMP_COLORS[kind].srgb_to_linear() * (LAMP_ENERGY * flux / (FAR_CELL_M * FAR_CELL_M)))
	RenderingServer.global_shader_parameter_set("street_lamp_far_fade", FAR_FADE_M)

# Every loaded lamp splatted into the far map (texels outside the window are skipped): O(lamps),
# when the chunk grid changes and after the camera's focus moves FAR_RECENTRE_M (33 ms for 30,000
# lamps on the M2 Pro).
# Road edits patch it per chunk instead (_splat_chunk).
func _rebuild_far(focus: Vector2) -> void:
	_far_dirty = false
	_far_origin = (focus / FAR_CELL_M).floor() * FAR_CELL_M - Vector2.ONE * (FAR_SIDE / 2) * FAR_CELL_M
	_far_centre = _far_origin + Vector2.ONE * (FAR_SIDE / 2) * FAR_CELL_M
	_far_texels.resize(FAR_SIDE * FAR_SIDE * 4)
	_far_texels.fill(0.0)
	for key in _chunk_points:
		_far_texels = LampGrid.splat_far(_far_texels, _chunk_points[key], _chunk_kinds[key], _far_kind_light,
			_far_origin, FAR_CELL_M, FAR_SIDE)
	_far_upload = true
	RenderingServer.global_shader_parameter_set("street_lamp_far_window",
		Vector4(_far_origin.x, _far_origin.y, FAR_CELL_M, FAR_SIDE))

# Adds (1) or takes back out (-1) one chunk's lamps; O(lamps in the chunk) plus one upload a frame.
func _splat_chunk(key: Vector2i, scale: float) -> void:
	if _far_dirty or not _chunk_points.has(key):
		return
	_far_texels = LampGrid.splat_far(_far_texels, _chunk_points[key], _chunk_kinds[key], _far_kind_light,
		_far_origin, FAR_CELL_M, FAR_SIDE, scale)
	_far_upload = true

func _upload_far() -> void:
	_far_upload = false
	var image := Image.create_from_data(FAR_SIDE, FAR_SIDE, false, Image.FORMAT_RGBAF, _far_texels.to_byte_array())
	if _far_texture == null:
		_far_texture = ImageTexture.create_from_image(image)
		RenderingServer.global_shader_parameter_set("street_lamp_far", _far_texture)
	else:
		_far_texture.update(image)

# Where the camera's look ray meets the ground, at most FAR_FOCUS_MAX_M ahead.
func _ground_focus(camera: Camera3D) -> Vector2:
	var eye := camera.global_position
	var forward := -camera.global_transform.basis.z
	var ahead := FAR_FOCUS_MAX_M
	if forward.y < -0.001:
		ahead = minf(eye.y / -forward.y, FAR_FOCUS_MAX_M)
	return Vector2(eye.x + forward.x * ahead, eye.z + forward.z * ahead)

func _ready() -> void:
	RenderingServer.global_shader_parameter_set("street_lamp_settings",
		Vector2(cos(deg_to_rad(LAMP_ANGLE_DEG)), LAMPS_ON_ELEVATION_DEG))
	_upload_kinds()
	for i in POOL_SIZE:
		var light := SpotLight3D.new()
		light.name = "StreetLamp%d" % i
		light.visible = false
		light.shadow_enabled = false
		# Godot feeds every light into volumetric fog by default, a fixed cost per frame.
		light.light_volumetric_fog_energy = 0.0
		light.spot_angle = LAMP_ANGLE_DEG
		light.spot_angle_attenuation = 0.5
		light.spot_attenuation = 1.0
		light.light_energy = LAMP_ENERGY
		light.light_specular = 0.35
		light.distance_fade_enabled = true
		# Spot lights shine along -Z: point it straight down.
		light.basis = Basis(Vector3.RIGHT, -PI * 0.5)
		add_child(light)
		_pool.append(light)

func _process(delta: float) -> void:
	_pool_age_s += delta
	var night := SceneLighting.current_sun_elevation_deg < LAMPS_ON_ELEVATION_DEG
	var camera := get_viewport().get_camera_3d()
	if not night or camera == null:
		if _pool_camera.x != INF:
			for light in _pool:
				light.visible = false
			_pool_camera = Vector3(INF, INF, INF)
		return
	var eye := camera.global_position
	var cell := Vector2i(floori(eye.x / LampGrid.CELL_M), floori(eye.z / LampGrid.CELL_M))
	# Re-centre only after a long move: the window reaches 1 km each way, and a rebuild walks every
	# lamp in it, so following each 16 m cell would cost a GDScript pass per few metres of panning.
	var moved := (cell - _grid_cell).abs()
	if _grid_dirty or maxi(moved.x, moved.y) > GRID_RECENTRE_CELLS:
		_rebuild_grid(cell)
		_pool_age_s = INF
	var focus := _ground_focus(camera)
	if _far_dirty or focus.distance_to(_far_centre) > FAR_RECENTRE_M:
		_rebuild_far(focus)
	if _far_upload:
		_upload_far()
	if _pool_age_s < POOL_REFRESH_S and eye.distance_to(_pool_camera) < POOL_REFRESH_MOVE_M:
		return
	_pool_age_s = 0.0
	_pool_camera = eye
	_assign_pool(eye)

# Nearest lamps in local road chunks: O(N + C log C), C candidates within 80 m.
func _assign_pool(eye: Vector3) -> void:
	var candidates: Array = []
	var radius_sq := POOL_RADIUS_M * POOL_RADIUS_M
	var bounds := _chunk_bounds(Vector2(eye.x, eye.z) - Vector2.ONE * (POOL_RADIUS_M + 3.4),
		Vector2(eye.x, eye.z) + Vector2.ONE * (POOL_RADIUS_M + 3.4))
	for z in range(bounds.position.y, bounds.end.y):
		for x in range(bounds.position.x, bounds.end.x):
			var key := Vector2i(x, z)
			if not _chunk_points.has(key):
				continue
			var points: PackedVector3Array = _chunk_points[key]
			var kinds: PackedByteArray = _chunk_kinds[key]
			for i in points.size():
				var d := points[i].distance_squared_to(eye)
				if d < radius_sq:
					candidates.append([d, points[i], kinds[i]])
	candidates.sort_custom(func(a, b): return a[0] < b[0])
	# Hand off where the real lights run out. The material lamps take each lamp over by its camera
	# distance over the band in which Godot fades its real light (street_lamp_light.gdshaderinc).
	var handoff_m := POOL_RADIUS_M
	if candidates.size() > _pool.size():
		handoff_m = sqrt(float(candidates[_pool.size() - 1][0]))
	var fade_m := minf(POOL_FADE_M, handoff_m * 0.5)
	RenderingServer.global_shader_parameter_set("street_lamp_fade", Vector2(handoff_m - fade_m, handoff_m))
	for i in _pool.size():
		var light := _pool[i]
		if i >= candidates.size():
			light.visible = false
			continue
		var point: Vector3 = candidates[i][1]
		var kind: int = candidates[i][2]
		light.position = point
		light.distance_fade_begin = handoff_m - fade_m
		light.distance_fade_length = fade_m
		light.light_color = LAMP_COLORS[kind]
		# Use the same range sphere as the material lamp.
		light.spot_range = (LIGHT_POINTS[kind][0] as Vector3).y + LAMP_RANGE_MARGIN_M
		light.visible = true

func set_chunk(key: Vector2i, origin: Vector3, lights: PackedFloat32Array) -> void:
	remove_chunk(key)
	if lights.size() < 5:
		return
	_ensure_meshes()
	# Transforms bucketed by (cell x, cell z, kind).
	var buckets: Dictionary = {}
	for i in range(0, lights.size() - 4, 5):
		var kind := int(lights[i + 4])
		if kind < 0 or kind >= MODELS.size() or (_meshes[kind] as Array).is_empty():
			continue
		var base := Vector3(lights[i], lights[i + 1], lights[i + 2])
		var bucket := Vector3i(floori(base.x / CELL_M), floori(base.z / CELL_M), kind)
		if not buckets.has(bucket):
			buckets[bucket] = []
		# Basis about +Y by yaw turns the model's +X arm to (cos yaw, 0, -sin yaw), as in Rust.
		(buckets[bucket] as Array).append(Transform3D(Basis(Vector3.UP, lights[i + 3]), base))
	var points := PackedVector3Array()
	var point_kinds := PackedByteArray()
	for bucket in buckets:
		for transform in buckets[bucket]:
			for local in LIGHT_POINTS[bucket.z]:
				points.append(origin + (transform as Transform3D) * (local as Vector3))
				point_kinds.append(bucket.z)
	_chunk_points[key] = points
	_chunk_kinds[key] = point_kinds
	_pool_age_s = INF
	_grid_dirty = true
	_splat_chunk(key, 1.0)
	var root := Node3D.new()
	root.name = "StreetLights_%d_%d" % [key.x, key.y]
	root.position = origin
	for bucket in buckets:
		var transforms: Array = buckets[bucket]
		var lods: Array = _meshes[bucket.z]
		for lod in lods.size():
			root.add_child(_batch(lods[lod], transforms, lod))
	root.add_child(_glow_batch(points, point_kinds, origin))
	add_child(root)
	_chunks[key] = root

func remove_chunk(key: Vector2i) -> void:
	if not _chunks.has(key):
		return
	var root: Node3D = _chunks[key]
	_chunks.erase(key)
	_splat_chunk(key, -1.0)
	_chunk_points.erase(key)
	_chunk_kinds.erase(key)
	_pool_age_s = INF
	_grid_dirty = true
	root.queue_free()

func clear() -> void:
	for key in _chunks.keys():
		remove_chunk(key)

# Every lamp head of the chunk as one glow quad, at every distance: what keeps a town visible
# from afar at night once the poles' last LOD has ended.
func _glow_batch(points: PackedVector3Array, kinds: PackedByteArray, origin: Vector3) -> MultiMeshInstance3D:
	var multimesh := MultiMesh.new()
	multimesh.transform_format = MultiMesh.TRANSFORM_3D
	multimesh.use_colors = true
	multimesh.mesh = _glow_mesh
	multimesh.instance_count = points.size()
	for i in points.size():
		multimesh.set_instance_transform(i, Transform3D(Basis.IDENTITY, points[i] - origin))
		multimesh.set_instance_color(i, LAMP_COLORS[kinds[i]])
	var instance := MultiMeshInstance3D.new()
	instance.name = "Glow"
	instance.multimesh = multimesh
	SceneLightingConfig.apply_shadow_policy(
		instance, SceneLightingConfig.SHADOW_NON_RECEIVER, "street_lights"
	)
	# The quads are expanded in the vertex shader; keep culling from dropping them at the edges.
	instance.extra_cull_margin = 2.0
	return instance

func _batch(mesh: ArrayMesh, transforms: Array, lod: int) -> MultiMeshInstance3D:
	var multimesh := MultiMesh.new()
	multimesh.transform_format = MultiMesh.TRANSFORM_3D
	multimesh.mesh = mesh
	multimesh.instance_count = transforms.size()
	for i in transforms.size():
		multimesh.set_instance_transform(i, transforms[i])
	var instance := MultiMeshInstance3D.new()
	instance.multimesh = multimesh
	instance.material_override = _material
	var band: Vector2 = LOD_RANGES[lod]
	instance.visibility_range_begin = band.x
	instance.visibility_range_end = band.y
	instance.visibility_range_begin_margin = LOD_HYSTERESIS_M if lod > 0 else 0.0
	instance.visibility_range_end_margin = LOD_HYSTERESIS_M
	SceneLightingConfig.apply_shadow_policy(
		instance,
		SceneLightingConfig.SHADOW_STATIC_CASTER if lod < 2 else SceneLightingConfig.SHADOW_RECEIVER_ONLY,
		"street_lights"
	)
	return instance

static func _ensure_meshes() -> void:
	if not _meshes.is_empty():
		return
	_material = ShaderMaterial.new()
	_material.shader = SHADER
	var glow_material := ShaderMaterial.new()
	glow_material.shader = GLOW_SHADER
	_glow_mesh = QuadMesh.new()
	_glow_mesh.size = Vector2(2.0, 2.0)
	_glow_mesh.material = glow_material
	for kind in MODELS.size():
		var lods: Array = []
		var scene := load(MODEL_DIR + MODELS[kind] + ".glb") as PackedScene
		if scene != null:
			var model := scene.instantiate()
			for lod in LOD_RANGES.size():
				var node := model.find_child("%s_LOD%d" % [MODELS[kind], lod], true, false) as MeshInstance3D
				if node == null:
					lods.clear()
					break
				lods.append(_merged(node, LAMP_COLORS[kind]))
			model.free()
		if lods.is_empty():
			push_error("street_lights: %s.glb lacks its three LOD meshes" % MODELS[kind])
		_meshes.append(lods)

# One surface with the part in vertex colour: the lens carries its lamp colour, other parts their
# base colour; UV is (roughness, metallic).
static func _merged(node: MeshInstance3D, lamp_color: Color) -> ArrayMesh:
	var source := node.mesh
	var vertices := PackedVector3Array()
	var normals := PackedVector3Array()
	var colors := PackedColorArray()
	var uvs := PackedVector2Array()
	var indices := PackedInt32Array()
	for surface in source.get_surface_count():
		var arrays := source.surface_get_arrays(surface)
		var material := node.get_active_material(surface) as BaseMaterial3D
		var part := material.resource_name if material else ""
		var alpha: float = PART_ALPHA.get(part, 0.0)
		var color := lamp_color if part == "lamp" else (material.albedo_color if material else Color.GRAY)
		color.a = alpha
		var surface_uv := Vector2(material.roughness, material.metallic) if material else Vector2(0.8, 0.0)
		var offset := vertices.size()
		var surface_vertices: PackedVector3Array = arrays[Mesh.ARRAY_VERTEX]
		var surface_normals: PackedVector3Array = arrays[Mesh.ARRAY_NORMAL]
		for v in surface_vertices.size():
			vertices.append(node.transform * surface_vertices[v])
			normals.append((node.transform.basis * surface_normals[v]).normalized())
			colors.append(color)
			uvs.append(surface_uv)
		var surface_indices = arrays[Mesh.ARRAY_INDEX]
		if surface_indices == null:
			for v in surface_vertices.size():
				indices.append(offset + v)
		else:
			for index in surface_indices as PackedInt32Array:
				indices.append(offset + index)
	var arrays := []
	arrays.resize(Mesh.ARRAY_MAX)
	arrays[Mesh.ARRAY_VERTEX] = vertices
	arrays[Mesh.ARRAY_NORMAL] = normals
	arrays[Mesh.ARRAY_COLOR] = colors
	arrays[Mesh.ARRAY_TEX_UV] = uvs
	arrays[Mesh.ARRAY_INDEX] = indices
	var merged := ArrayMesh.new()
	merged.add_surface_from_arrays(Mesh.PRIMITIVE_TRIANGLES, arrays)
	return merged
