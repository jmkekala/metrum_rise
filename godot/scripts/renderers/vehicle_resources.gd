# SPDX-License-Identifier: GPL-3.0-or-later

## Vehicle meshes and materials for the moving and parked car renderers (`RENDER-65`,
## docs/vehicles.md "Renderer"). Rust's vehicle catalogue (`SimulationNode.get_vehicle_catalogue`)
## names the looks, a model in one albedo variant, and each car bucket is `look * 4 + tier`. A
## tier is read through Godot's glTF importer as building meshes are, turned half a turn about Y
## (cars drive along their -Z) and drawn with one body shader (`vehicle_body.gdshader`) that reads
## the ids the model carries in COLOR_0, moved to CUSTOM0 at load (`RENDER-70`). Lamps are optional, so parked meshes have no headlight or
## glow surface. With no vehicle model at all a procedural box car, sized to the car class box,
## draws every bucket. Loading runs when a renderer first meets a bucket, never per frame.
extends RefCounted

const WindowMaterials := preload("res://scripts/renderers/window_materials.gd")
const VehicleLamps := preload("res://scripts/renderers/vehicle_lamps.gd")
const PipelineTemplate := preload("res://scripts/renderers/pipeline_template.gd")
const BODY_SHADER := preload("res://scripts/shaders/vehicle_body.gdshader")
const TIERS := 4
const PALETTE_SIZE := 64
# The ids' vertex format: CUSTOM0 as four unsigned normalized bytes.
const IDS_CUSTOM := Mesh.ARRAY_CUSTOM_RGBA8_UNORM
const WHEEL_SLOTS := 8

var lamps_enabled := true
## Whether each tier draws through a material of its own that keeps only the cars whose camera
## distance is in the tier's band (`RENDER-69`): the parked cars, whose batches hold a chunk's cars
## in every tier.
var tier_bands := false
# The cars' draw distance fade band from the camera, metres (agents.gd and parked_cars.gd set it
# from Rust, `RENDER-28`, `RENDER-48`); zero draws at every distance.
var draw_fade_m := Vector2.ZERO
## The catalogue revision the looks were read at; -1 before the first read.
var revision := -1
## The catalogue's looks, by look index.
var looks: Array = []
var _tier_arrays: Dictionary = {}
var _textures: Dictionary = {}
var _materials: Dictionary = {}
var _tier_materials: Dictionary = {}
var _lamp_materials: Dictionary = {}


## Reads the catalogue when its revision is not `wanted` (-1: always) and drops every mesh source
## and material of the last one. Returns whether the looks changed.
func refresh(simulation_node: Object, wanted := -1) -> bool:
	if wanted >= 0 and wanted == revision:
		return false
	var catalogue: Dictionary = simulation_node.get_vehicle_catalogue()
	revision = catalogue["revision"]
	looks = catalogue["looks"]
	_tier_arrays.clear()
	_materials.clear()
	_tier_materials.clear()
	_lamp_materials.clear()
	return true


## Bucket `bucket`'s mesh: its look's tier with the look's material, and lamps when enabled.
func mesh_for_bucket(bucket: int) -> ArrayMesh:
	var look_index := bucket / TIERS
	if look_index >= looks.size():
		return _build_car_mesh()
	var look: Dictionary = looks[look_index]
	var arrays := _tier(look["tiers"][bucket % TIERS])
	if arrays.is_empty():
		return _build_car_mesh()
	var mesh := ArrayMesh.new()
	mesh.add_surface_from_arrays(Mesh.PRIMITIVE_TRIANGLES, arrays, [], {},
		IDS_CUSTOM << Mesh.ARRAY_FORMAT_CUSTOM0_SHIFT)
	mesh.surface_set_material(0, _tier_material(bucket) if tier_bands else _material(look_index))
	if lamps_enabled:
		VehicleLamps.add_surfaces(mesh, "model%d" % look["model"], look["head_lamp"], look["tail_lamp"],
			_lamp_materials, draw_fade_m)
	return mesh


## A tier's surface arrays in the drawn frame, read once a revision. Empty when it cannot be read.
func _tier(path: String) -> Array:
	if _tier_arrays.has(path):
		return _tier_arrays[path]
	var arrays := _import_tier(path)
	_tier_arrays[path] = arrays
	return arrays


func _import_tier(path: String) -> Array:
	var document := GLTFDocument.new()
	var state := GLTFState.new()
	var glb := _without_textures(FileAccess.get_file_as_bytes(path)) if FileAccess.file_exists(path) else PackedByteArray()
	if glb.is_empty() or document.append_from_buffer(glb, path.get_base_dir(), state) != OK:
		push_error("Vehicle tier could not be read: " + path)
		return []
	var scene := document.generate_scene(state)
	if scene == null:
		push_error("Vehicle tier has no scene: " + path)
		return []
	var arrays := []
	var instance := _first_mesh_instance(scene)
	if instance != null and instance.mesh != null and instance.mesh.get_surface_count() > 0:
		arrays = _drawn_arrays(instance.mesh.surface_get_arrays(0), _scene_transform(instance, scene))
		if instance.mesh.get_surface_count() > 1:
			push_warning("Vehicle tier has more than one surface; drawing the first: " + path)
	scene.free()
	return arrays


# The tier in the frame cars are drawn in: the node transform, then half a turn about Y (x and z
# negated exactly). The ids move from COLOR to CUSTOM0 as their exact bytes (`RENDER-65`,
# `RENDER-70`, docs/vehicles.md "Ids, instance colours and paint"): the Compatibility renderer
# multiplies COLOR by an instance colour a MultiMesh without colours never wrote.
static func _drawn_arrays(arrays: Array, node: Transform3D) -> Array:
	var vertices: PackedVector3Array = arrays[Mesh.ARRAY_VERTEX]
	for i in vertices.size():
		var v := node * vertices[i]
		vertices[i] = Vector3(-v.x, v.y, -v.z)
	arrays[Mesh.ARRAY_VERTEX] = vertices
	if arrays[Mesh.ARRAY_NORMAL] is PackedVector3Array:
		var normals: PackedVector3Array = arrays[Mesh.ARRAY_NORMAL]
		var normal_basis := node.basis.inverse().transposed()
		for i in normals.size():
			var n := (normal_basis * normals[i]).normalized()
			normals[i] = Vector3(-n.x, n.y, -n.z)
		arrays[Mesh.ARRAY_NORMAL] = normals
	if arrays[Mesh.ARRAY_TANGENT] is PackedFloat32Array:
		var tangents: PackedFloat32Array = arrays[Mesh.ARRAY_TANGENT]
		for i in range(0, tangents.size(), 4):
			var t := (node.basis * Vector3(tangents[i], tangents[i + 1], tangents[i + 2])).normalized()
			tangents[i] = -t.x
			tangents[i + 1] = t.y
			tangents[i + 2] = -t.z
		arrays[Mesh.ARRAY_TANGENT] = tangents
	if arrays[Mesh.ARRAY_COLOR] is PackedColorArray:
		var colors: PackedColorArray = arrays[Mesh.ARRAY_COLOR]
		var ids := PackedByteArray()
		ids.resize(colors.size() * 4)
		for i in colors.size():
			var c := colors[i]
			ids[i * 4] = int(roundf(c.r * 255.0))
			ids[i * 4 + 1] = int(roundf(c.g * 255.0))
			ids[i * 4 + 2] = int(roundf(c.b * 255.0))
			ids[i * 4 + 3] = 255
		arrays[Mesh.ARRAY_CUSTOM0] = ids
		arrays[Mesh.ARRAY_COLOR] = null
	return arrays


# The GLB with its JSON chunk's images, textures and samplers and the material's texture slots
# taken out: the body shader binds the maps itself, so the importer neither decodes them nor tries
# to load an external `keep` file as a resource. Mesh data is untouched. Empty when `glb` is not a
# GLB with a JSON chunk first.
static func _without_textures(glb: PackedByteArray) -> PackedByteArray:
	if glb.size() < 20 or glb.decode_u32(0) != 0x46546c67 or glb.decode_u32(16) != 0x4e4f534a:
		return PackedByteArray()
	var json_length := glb.decode_u32(12)
	var doc = JSON.parse_string(glb.slice(20, 20 + json_length).get_string_from_utf8())
	if not doc is Dictionary:
		return PackedByteArray()
	for key in ["images", "textures", "samplers"]:
		doc.erase(key)
	for material: Dictionary in doc.get("materials", []):
		for key in ["normalTexture", "occlusionTexture", "emissiveTexture"]:
			material.erase(key)
		var pbr: Dictionary = material.get("pbrMetallicRoughness", {})
		pbr.erase("baseColorTexture")
		pbr.erase("metallicRoughnessTexture")
	var json := JSON.stringify(doc, "", false, true).to_utf8_buffer()
	while json.size() % 4 != 0:
		json.append(0x20)
	var rest := glb.slice(20 + json_length)
	var out := PackedByteArray()
	out.resize(20)
	out.encode_u32(0, 0x46546c67)
	out.encode_u32(4, 2)
	out.encode_u32(8, 20 + json.size() + rest.size())
	out.encode_u32(12, json.size())
	out.encode_u32(16, 0x4e4f534a)
	out.append_array(json)
	out.append_array(rest)
	return out


static func _first_mesh_instance(node: Node) -> MeshInstance3D:
	if node is MeshInstance3D:
		return node
	for child in node.get_children():
		var found := _first_mesh_instance(child)
		if found != null:
			return found
	return null


static func _scene_transform(node: Node, root: Node) -> Transform3D:
	var transform := Transform3D.IDENTITY
	while node != null and node != root:
		if node is Node3D:
			transform = (node as Node3D).transform * transform
		node = node.get_parent()
	return transform


func _material(look_index: int) -> ShaderMaterial:
	if _materials.has(look_index):
		return _materials[look_index]
	var look: Dictionary = looks[look_index]
	var material := ShaderMaterial.new()
	material.shader = BODY_SHADER
	material.set_shader_parameter("albedo_texture", _texture(look["albedo"]))
	material.set_shader_parameter("orm_texture", _texture(look["orm"]))
	material.set_shader_parameter("emission_texture", _texture(look["emission"]))
	var normal: String = look["normal"]
	material.set_shader_parameter("has_normal", normal != "")
	if normal != "":
		material.set_shader_parameter("normal_texture", _texture(normal))
	material.set_shader_parameter("paint_enabled", look["paint"])
	var palette := PackedVector3Array()
	palette.resize(PALETTE_SIZE)
	var colours: PackedColorArray = look["palette"]
	for i in mini(colours.size(), PALETTE_SIZE):
		palette[i] = Vector3(colours[i].r, colours[i].g, colours[i].b)
	material.set_shader_parameter("palette", palette)
	var wheels := PackedVector4Array()
	wheels.resize(WHEEL_SLOTS)
	var centres: PackedVector4Array = look["wheels"]
	for i in mini(centres.size(), WHEEL_SLOTS):
		wheels[i] = centres[i]
	material.set_shader_parameter("wheels", wheels)
	if draw_fade_m != Vector2.ZERO:
		material.set_shader_parameter("draw_fade_m", draw_fade_m)
	_materials[look_index] = material
	return material


# Bucket `bucket`'s tier material: the look's, keeping the cars in the tier's band of the
# catalogue's tier starts, `[start, next start)`, the last one unbounded.
func _tier_material(bucket: int) -> ShaderMaterial:
	if _tier_materials.has(bucket):
		return _tier_materials[bucket]
	var starts: PackedFloat32Array = looks[bucket / TIERS]["tier_starts_m"]
	var tier := bucket % TIERS
	var material := _material(bucket / TIERS).duplicate() as ShaderMaterial
	material.set_shader_parameter("tier_band_m",
		Vector2(starts[tier], starts[tier + 1] if tier + 1 < starts.size() else 1.0e30))
	_tier_materials[bucket] = material
	return material


func _texture(path: String) -> Texture2D:
	if _textures.has(path):
		return _textures[path]
	var texture: Texture2D = null
	var image := Image.load_from_file(path)
	if image != null and not image.is_empty():
		image.generate_mipmaps()
		texture = ImageTexture.create_from_image(image)
	else:
		push_error("Vehicle texture could not be read: " + path)
	_textures[path] = texture
	return texture


## The last fallback with no vehicle model: a box car the size of the car class box (4.5 × 1.8 m,
## nose at -Z), body and cabin, through the building shader.
func _build_car_mesh() -> ArrayMesh:
	# Plain Arrays (reference types) so the helpers append to the caller's arrays.
	var verts: Array = []
	var norms: Array = []
	_add_box(verts, norms, Vector3(-0.9, 0.0, -2.25), Vector3(0.9, 0.7, 2.25))
	# The cabin starts at y = 0 so its faces seal the body's open ends at z = ±1.1.
	_add_box(verts, norms, Vector3(-0.85, 0.0, -1.1), Vector3(0.85, 1.45, 1.1))
	var arrays := []
	arrays.resize(Mesh.ARRAY_MAX)
	arrays[Mesh.ARRAY_VERTEX] = PackedVector3Array(verts)
	arrays[Mesh.ARRAY_NORMAL] = PackedVector3Array(norms)
	var mesh := ArrayMesh.new()
	mesh.add_surface_from_arrays(Mesh.PRIMITIVE_TRIANGLES, arrays)
	var mat := StandardMaterial3D.new()
	mat.albedo_color = Color(0.62, 0.64, 0.66)
	mat.roughness = 0.8
	mat.metallic = 0.0
	mat.disable_receive_shadows = true
	mesh.surface_set_material(0, WindowMaterials.create(mat, false, WindowMaterials.DEFAULT_BRIGHTNESS, draw_fade_m))
	if lamps_enabled:
		VehicleLamps.add_surfaces(mesh, "procedural", Vector3(0.6, 0.55, -2.25), Vector3(0.65, 0.6, 2.25),
			_lamp_materials, draw_fade_m)
	return mesh


func _add_box(verts: Array, norms: Array, mn: Vector3, mx: Vector3) -> void:
	var x0 := mn.x; var y0 := mn.y; var z0 := mn.z
	var x1 := mx.x; var y1 := mx.y; var z1 := mx.z
	_add_quad(verts, norms, Vector3(x1,y0,z1), Vector3(x1,y0,z0), Vector3(x1,y1,z0), Vector3(x1,y1,z1), Vector3.RIGHT)
	_add_quad(verts, norms, Vector3(x0,y0,z0), Vector3(x0,y0,z1), Vector3(x0,y1,z1), Vector3(x0,y1,z0), Vector3.LEFT)
	_add_quad(verts, norms, Vector3(x0,y1,z1), Vector3(x1,y1,z1), Vector3(x1,y1,z0), Vector3(x0,y1,z0), Vector3.UP)
	_add_quad(verts, norms, Vector3(x0,y0,z0), Vector3(x1,y0,z0), Vector3(x1,y0,z1), Vector3(x0,y0,z1), Vector3.DOWN)
	_add_quad(verts, norms, Vector3(x0,y0,z1), Vector3(x1,y0,z1), Vector3(x1,y1,z1), Vector3(x0,y1,z1), Vector3.BACK)
	_add_quad(verts, norms, Vector3(x1,y0,z0), Vector3(x0,y0,z0), Vector3(x0,y1,z0), Vector3(x1,y1,z0), Vector3.FORWARD)


func _add_quad(verts: Array, norms: Array,
			   a: Vector3, b: Vector3, c: Vector3, d: Vector3, n: Vector3) -> void:
	verts.append(a); verts.append(b); verts.append(c)
	verts.append(a); verts.append(c); verts.append(d)
	for _i in 6:
		norms.append(n)


## The menu's pipeline prewarm template of the cars (`RENDER-43`): the body shader with the fade
## (any band selects it) on the tiers' vertex format (positions, normals, tangents, the ids in
## CUSTOM0 as bytes, UVs).
static func pipeline_prewarm_surfaces() -> Array:
	var format := (Mesh.ARRAY_FORMAT_NORMAL | Mesh.ARRAY_FORMAT_TANGENT | Mesh.ARRAY_FORMAT_CUSTOM0
		| Mesh.ARRAY_FORMAT_TEX_UV)
	var body := PipelineTemplate.bare(BODY_SHADER)
	body.set_shader_parameter("draw_fade_m", Vector2.ONE)
	return [[PipelineTemplate.format_mesh(format, false, IDS_CUSTOM), body]]
