# SPDX-License-Identifier: GPL-2.0-only

## Street lights along roads and footpaths (`ROAD-59`). Rust places them with each road chunk
## (`street_lights` in the chunk data: five floats per light, chunk-local pole base x, y, z, yaw,
## model kind); a chunk's lights draw as one MultiMesh per model and level of detail, on the Rust
## book's pooled RenderingServer slots (`RENDER-52` part 2: no node or resource a batch); this
## hands the book the merged meshes, material and world. Each pole keeps the level its 64 m cell's
## own batch drew under Godot's visibility range: the book picks the cells from the main camera
## just before each frame draws (`draw_poles`, `RENDER-46`). Each model is merged into a single
## surface, so a batch is one draw call. At night the
## lamp grid, far light map and pool of real lights follow the camera; Rust builds them and the
## chunks' instance buffers (`StreetLightBook`, `RENDER-32-H`), this uploads the two textures and
## places the pool's lights.
extends Node3D

const SceneLighting := preload("res://scripts/core/scene_lighting.gd")
const SHADER := preload("res://scripts/shaders/street_light.gdshader")
const GLOW_SHADER := preload("res://scripts/shaders/street_light_glow.gdshader")
const PipelineTemplate := preload("res://scripts/renderers/pipeline_template.gd")
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
# Distance bands from each 64 m cell's centre, one per level of detail (the GLBs' _LOD0.._LOD2);
# the Rust draw (`street_lights/draw.rs`) picks each cell's level by these with 10 m of hysteresis.
const LOD_RANGES: Array[Vector2] = [Vector2(0.0, 140.0), Vector2(140.0, 420.0), Vector2(420.0, 1400.0)]
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

var _book := StreetLightBook.new()
var _kinds_set := false
# The book draws the batches once it has this node's world (`_attach_draw`).
var _draw_attached := false
var _pool: Array[SpotLight3D] = []
var _kind_texture: ImageTexture
var _cell_texture: ImageTexture
var _far_texture: ImageTexture

func configure_chunks(span_m: float, origin: Vector2) -> void:
	_book.configure_chunks(span_m, origin)

func _enter_tree() -> void:
	# The pole levels follow the camera Godot draws from: after every script moved it.
	if not RenderingServer.frame_pre_draw.is_connected(_before_draw):
		RenderingServer.frame_pre_draw.connect(_before_draw)

func _exit_tree() -> void:
	if RenderingServer.frame_pre_draw.is_connected(_before_draw):
		RenderingServer.frame_pre_draw.disconnect(_before_draw)
	RenderingServer.global_shader_parameter_set("street_lamp_window", Vector4.ZERO)
	# The batches' slots are no node's: freed here, before the meshes they name.
	_book.release_draw()
	_draw_attached = false

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
	_attach_draw()

## The lamps' one-time resources and the world's batches (the load call's prewarm,
## `road_chunk_hydration.gd`): the meshes, the book's kinds, the draw's spare slot of every mesh,
## and every road chunk's batches (`keys` x, z pairs, their frame `origins`, `lights` one
## PackedFloat32Array a chunk) made and written hidden, so a load frame's lights part only shows
## them (a slot's creation or upload now and then stalls 1-3 ms, `RENDER-52` part 2).
func prewarm(keys := PackedInt32Array(), origins := PackedVector3Array(), lights: Array = []) -> void:
	_set_kinds()
	_attach_draw()
	_book.prebuild_draw(keys, origins, lights)

# Hands the book the meshes (every kind's three levels, then the glow quad; their bounds place
# the cells' level centres), the lamp material and this node's world. The book makes a hidden spare slot of every mesh as it attaches, so each
# mesh's first-use costs fall in the load call's prewarm, not in a load frame's lights part
# (4-9 ms at 30k, `RENDER-52` part 2). Outside the tree (tests) only the book's passes run.
func _attach_draw() -> void:
	if _draw_attached or not is_inside_tree():
		return
	var meshes: Array[Mesh] = []
	for lods: Array in _meshes:
		for lod in LOD_RANGES.size():
			# A kind without its model draws nothing (the book skips its lights): a stand-in.
			meshes.append(lods[lod] as ArrayMesh if not lods.is_empty() else _glow_mesh)
	meshes.append(_glow_mesh)
	_draw_attached = _book.attach_draw(get_world_3d().scenario, meshes, _material.get_rid())

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

func _before_draw() -> void:
	if not _draw_attached:
		return
	var camera := get_viewport().get_camera_3d()
	if camera == null:
		return
	if not PerfDebug.is_enabled():
		_book.draw_poles(camera.get_camera_transform().origin)
		return
	var start_us := Time.get_ticks_usec()
	var changed := _book.draw_poles(camera.get_camera_transform().origin)
	PerfDebug.record("street_light_poles", float(Time.get_ticks_usec() - start_us) / 1000.0,
		{"batches_changed": changed})

func set_chunk(key: Vector2i, origin: Vector3, lights: PackedFloat32Array) -> void:
	begin_chunk(key, origin, lights)
	build_chunk_part(key, INF)

## `set_chunk` in parts (a road chunk's hydration, `RENDER-52` part 2): the book takes the
## chunk's lights now, `build_chunk_part` makes its batches' slots, and the chunk draws once they
## are all made.
func begin_chunk(key: Vector2i, origin: Vector3, lights: PackedFloat32Array) -> void:
	_set_kinds()
	_attach_draw()
	# Motion vectors (TAA, FSR 2) keep a buffer's previous upload: reused slots take fresh storage.
	var viewport := get_viewport() if is_inside_tree() else null
	if viewport != null:
		_book.set_fresh_buffers(viewport.use_taa or viewport.scaling_3d_mode == Viewport.SCALING_3D_MODE_FSR2)
	_book.begin_chunk(key, origin, lights)

## Makes chunk `key`'s next batches until `budget_us` has passed (one batch at least); true
## when none is left to make, the chunk then drawn.
func build_chunk_part(key: Vector2i, budget_us: float) -> bool:
	return _book.build_chunk_part(key, budget_us)

## Whether chunk `key`'s batches are still being made.
func is_building(key: Vector2i) -> bool:
	return _book.is_building(key)

func remove_chunk(key: Vector2i) -> void:
	_book.remove_chunk(key)

func clear() -> void:
	_book.clear()

## Whether the lamps cast the sun's shadow (a shadow benchmark's class toggle,
## `shadow_variants.gd`; the game always casts).
func set_cast_shadows(cast: bool) -> void:
	_book.set_cast_shadows(cast)

## The menu's pipeline prewarm templates (`RENDER-43`): the lamps' shader on the merged lamp
## meshes' vertex format (`_merged`: positions, normals, colours, UVs) and the glow's on its quad.
static func pipeline_prewarm_surfaces() -> Array:
	var format := Mesh.ARRAY_FORMAT_NORMAL | Mesh.ARRAY_FORMAT_COLOR | Mesh.ARRAY_FORMAT_TEX_UV
	return [[PipelineTemplate.format_mesh(format), PipelineTemplate.bare(SHADER)],
		[QuadMesh.new(), PipelineTemplate.bare(GLOW_SHADER)]]

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
