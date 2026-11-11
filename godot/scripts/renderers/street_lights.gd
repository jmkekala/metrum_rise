# SPDX-License-Identifier: GPL-2.0-only

## Street lights along roads and footpaths (`ROAD-59`). Rust places them with each road chunk
## (`street_lights` in the chunk data: five floats per light, chunk-local pole base x, y, z, yaw,
## model kind); this draws a chunk's lights as one MultiMesh per 64 m cell, model and level of
## detail. Each model is merged into a single surface, so a batch is one draw call. At night the
## lamp grid, far light map and pool of real lights follow the camera; Rust builds them and the
## chunks' instance buffers (`StreetLightBook`, `RENDER-32-H`), this uploads the two textures and
## places the pool's lights.
extends Node3D

const SceneLightingConfig := preload("res://scripts/core/scene_lighting.gd")
const SceneLighting := preload("res://scripts/core/scene_lighting.gd")
const SHADER := preload("res://scripts/shaders/street_light.gdshader")
const GLOW_SHADER := preload("res://scripts/shaders/street_light_glow.gdshader")
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
# Distance bands from each cell's centre, one per level of detail (the GLBs' _LOD0.._LOD2).
const LOD_RANGES: Array[Vector2] = [Vector2(0.0, 140.0), Vector2(140.0, 420.0), Vector2(420.0, 1400.0)]
const LOD_HYSTERESIS_M := 10.0
const PART_ALPHA := {"pole": 0.0, "housing": 0.5, "lamp": 1.0}
# Lamps light up below this sun elevation (matches street_light.gdshader).
const LAMPS_ON_ELEVATION_DEG := 2.0
const LAMP_ENERGY := 10.0
const LAMP_ANGLE_DEG := 80.0
# Past FAR_FADE_M (camera distance) the exact lamps hand over to the far map of mean ground light.
const FAR_FADE_M := Vector2(500.0, 700.0)

# [kind][lod] -> merged ArrayMesh, built once from the GLBs.
static var _meshes: Array = []
static var _material: ShaderMaterial
static var _glow_mesh: QuadMesh

var _chunks: Dictionary = {}
var _book := StreetLightBook.new()
var _kinds_set := false
var _pool: Array[SpotLight3D] = []
var _kind_texture: ImageTexture
var _cell_texture: ImageTexture
var _far_texture: ImageTexture

func configure_chunks(span_m: float, origin: Vector2) -> void:
	_book.configure_chunks(span_m, origin)

func _exit_tree() -> void:
	RenderingServer.global_shader_parameter_set("street_lamp_window", Vector4.ZERO)

# One RGBAF texel per lamp kind: linear colour * energy, as Godot feeds a real light's colour to
# the shader, and the range shared with the real light.
func _upload_kinds() -> void:
	var data := PackedFloat32Array()
	var linear := PackedColorArray()
	var ranges := StreetLightBook.kind_ranges()
	for kind in LAMP_COLORS.size():
		linear.append(LAMP_COLORS[kind].srgb_to_linear())
		var color := linear[kind] * LAMP_ENERGY
		data.append_array([color.r, color.g, color.b, ranges[kind]])
	_kind_texture = ImageTexture.create_from_image(Image.create_from_data(LAMP_COLORS.size(), 1,
		false, Image.FORMAT_RGBAF, data.to_byte_array()))
	RenderingServer.global_shader_parameter_set("street_lamp_kinds", _kind_texture)
	RenderingServer.global_shader_parameter_set("street_lamp_far_fade", FAR_FADE_M)

# The book needs the lamp colours and which models loaded before its first chunk.
func _set_kinds() -> void:
	if _kinds_set:
		return
	_kinds_set = true
	_ensure_meshes()
	var linear := PackedColorArray()
	var loaded := PackedByteArray()
	for kind in LAMP_COLORS.size():
		linear.append(LAMP_COLORS[kind].srgb_to_linear())
		loaded.append(0 if (_meshes[kind] as Array).is_empty() else 1)
	_book.set_kinds(PackedColorArray(LAMP_COLORS), linear, loaded)

func _upload(texture: ImageTexture, width: int, height: int, data: PackedByteArray, parameter: String) -> ImageTexture:
	var image := Image.create_from_data(width, height, false, Image.FORMAT_RGBAF, data)
	if texture == null:
		texture = ImageTexture.create_from_image(image)
		RenderingServer.global_shader_parameter_set(parameter, texture)
	else:
		texture.update(image)
	return texture

func _ready() -> void:
	RenderingServer.global_shader_parameter_set("street_lamp_settings",
		Vector2(cos(deg_to_rad(LAMP_ANGLE_DEG)), LAMPS_ON_ELEVATION_DEG))
	_upload_kinds()
	# Real lights go to the lamps nearest the camera; the rest are evaluated in lit materials.
	# Clustered Forward+ handles this many unshadowed spot lights at a nearly fixed cost.
	for i in StreetLightBook.POOL_SIZE:
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
	var night := SceneLighting.current_sun_elevation_deg < LAMPS_ON_ELEVATION_DEG
	var camera := get_viewport().get_camera_3d()
	if not night or camera == null:
		if _book.rest(delta):
			for light in _pool:
				light.visible = false
		return
	var start_us := Time.get_ticks_usec()
	var flags := _book.update(camera.global_position, -camera.global_transform.basis.z, delta)
	var book_us := Time.get_ticks_usec()
	if flags & 1:
		_cell_texture = _upload(_cell_texture, StreetLightBook.SIDE * StreetLightBook.CAPACITY,
			StreetLightBook.SIDE, _book.cell_data(), "street_lamp_cells")
		RenderingServer.global_shader_parameter_set("street_lamp_window", _book.cell_window())
	if flags & 2:
		_far_texture = _upload(_far_texture, StreetLightBook.FAR_SIDE, StreetLightBook.FAR_SIDE,
			_book.far_data(), "street_lamp_far")
		RenderingServer.global_shader_parameter_set("street_lamp_far_window", _book.far_window())
	var upload_us := Time.get_ticks_usec()
	if flags & 4:
		_place_pool()
	if PerfDebug.is_enabled():
		var end_us := Time.get_ticks_usec()
		PerfDebug.record("street_lights", float(end_us - start_us) / 1000.0, {
			"book_ms": float(book_us - start_us) / 1000.0, "upload_ms": float(upload_us - book_us) / 1000.0,
			"pool_ms": float(end_us - upload_us) / 1000.0, "grid_rebuilds": flags & 1,
			"far_uploads": (flags >> 1) & 1, "pool_assigns": (flags >> 2) & 1, "lamps": _book.last_work()})

# The pool's lights at the lamps the book chose, nearest first. The material lamps take each lamp
# over by its camera distance over the band in which Godot fades its real light
# (street_lamp_light.gdshaderinc).
func _place_pool() -> void:
	var pool := _book.pool()
	var fade_begin := pool[0]
	var fade_length := pool[1]
	var count := int(pool[3])
	var ranges := StreetLightBook.kind_ranges()
	RenderingServer.global_shader_parameter_set("street_lamp_fade", Vector2(fade_begin, pool[2]))
	for i in _pool.size():
		var light := _pool[i]
		if i >= count:
			light.visible = false
			continue
		var at := 4 + i * 4
		var kind := int(pool[at + 3])
		light.position = Vector3(pool[at], pool[at + 1], pool[at + 2])
		light.distance_fade_begin = fade_begin
		light.distance_fade_length = fade_length
		light.light_color = LAMP_COLORS[kind]
		# Use the same range sphere as the material lamp.
		light.spot_range = ranges[kind]
		light.visible = true

func set_chunk(key: Vector2i, origin: Vector3, lights: PackedFloat32Array) -> void:
	_set_kinds()
	_free_chunk(key)
	var batches := _book.set_chunk(key, origin, lights)
	if lights.size() < 5:
		return
	var root := Node3D.new()
	root.name = "StreetLights_%d_%d" % [key.x, key.y]
	root.position = origin
	var kinds: PackedByteArray = batches["kinds"]
	var transforms: Array = batches["transforms"]
	for batch in kinds.size():
		var lods: Array = _meshes[kinds[batch]]
		for lod in lods.size():
			root.add_child(_batch(lods[lod], transforms[batch], lod))
	root.add_child(_glow_batch(batches["glow"]))
	add_child(root)
	_chunks[key] = root

func remove_chunk(key: Vector2i) -> void:
	_book.remove_chunk(key)
	_free_chunk(key)

func _free_chunk(key: Vector2i) -> void:
	if _chunks.has(key):
		(_chunks[key] as Node3D).queue_free()
		_chunks.erase(key)

func clear() -> void:
	_book.clear()
	for key in _chunks.keys():
		_free_chunk(key)

# Every lamp head of the chunk as one glow quad, at every distance: what keeps a town visible
# from afar at night once the poles' last LOD has ended. 16 floats an instance (transform, colour).
func _glow_batch(buffer: PackedFloat32Array) -> MultiMeshInstance3D:
	var multimesh := MultiMesh.new()
	multimesh.transform_format = MultiMesh.TRANSFORM_3D
	multimesh.use_colors = true
	multimesh.mesh = _glow_mesh
	multimesh.instance_count = buffer.size() / 16
	multimesh.buffer = buffer
	var instance := MultiMeshInstance3D.new()
	instance.name = "Glow"
	instance.multimesh = multimesh
	SceneLightingConfig.apply_shadow_policy(
		instance, SceneLightingConfig.SHADOW_NON_RECEIVER, "street_lights"
	)
	# The quads are expanded in the vertex shader; keep culling from dropping them at the edges.
	instance.extra_cull_margin = 2.0
	return instance

# 12 floats an instance (Transform3D rows).
func _batch(mesh: ArrayMesh, buffer: PackedFloat32Array, lod: int) -> MultiMeshInstance3D:
	var multimesh := MultiMesh.new()
	multimesh.transform_format = MultiMesh.TRANSFORM_3D
	multimesh.mesh = mesh
	multimesh.instance_count = buffer.size() / 12
	multimesh.buffer = buffer
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
