# SPDX-License-Identifier: GPL-2.0-only

## Shared vehicle mesh and palette/material loading for moving and parked renderers.
## Lamps are optional so parked meshes never include headlight/glow surfaces.
extends RefCounted

const WindowMaterials := preload("res://scripts/renderers/window_materials.gd")
const VehicleLamps := preload("res://scripts/renderers/vehicle_lamps.gd")
const MODELS := {
	0: "res://assets/models/vehicles/civilian/sedan.glb",
	1: "res://assets/models/vehicles/civilian/sedan-sports.glb",
	2: "res://assets/models/vehicles/civilian/suv.glb",
	3: "res://assets/models/vehicles/civilian/suv-luxury.glb",
	4: "res://assets/models/vehicles/freight/delivery.glb",
}
const COLOURS := [Color(0.22, 0.42, 0.78), Color(0.72, 0.16, 0.14), Color(0.17, 0.55, 0.38), Color(0.88, 0.78, 0.30), Color(0.82, 0.84, 0.86)]

const GLB_MAGIC := 0x46546c67
const GLB_CHUNK_JSON := 0x4e4f534a
const GLB_CHUNK_BIN := 0x004e4942
const GLTF_COMPONENT_UNSIGNED_BYTE := 5121
const GLTF_COMPONENT_UNSIGNED_SHORT := 5123
const GLTF_COMPONENT_UNSIGNED_INT := 5125
const GLTF_COMPONENT_FLOAT := 5126
const GLTF_MODE_TRIANGLES := 4
var _vehicle_glb_cache: Dictionary = {}
var texture_cache: Dictionary = {}
var _lamp_materials: Dictionary = {}
var lamps_enabled := true

func _load_source_texture(path: String) -> Texture2D:
	if texture_cache.has(path):
		return texture_cache[path]

	var tex: Texture2D = null
	var image := Image.load_from_file(ProjectSettings.globalize_path(path))
	if image:
		tex = ImageTexture.create_from_image(image)
	else:
		push_error("Could not load texture source: " + path)

	texture_cache[path] = tex
	return tex

func _load_vehicle_glb_source(model_path: String) -> Dictionary:
	if _vehicle_glb_cache.has(model_path):
		return _vehicle_glb_cache[model_path]

	var result: Dictionary = {}
	var file := FileAccess.open(ProjectSettings.globalize_path(model_path), FileAccess.READ)
	if not file:
		_vehicle_glb_cache[model_path] = result
		return result

	if file.get_32() != GLB_MAGIC:
		_vehicle_glb_cache[model_path] = result
		return result

	var version := file.get_32()
	file.get_32() # total length
	if version != 2:
		_vehicle_glb_cache[model_path] = result
		return result

	var json_doc = null
	var bin_chunk := PackedByteArray()
	while file.get_position() + 8 <= file.get_length():
		var chunk_length := file.get_32()
		var chunk_type := file.get_32()
		var chunk_data := file.get_buffer(chunk_length)
		if chunk_type == GLB_CHUNK_JSON:
			json_doc = JSON.parse_string(chunk_data.get_string_from_utf8().strip_edges())
		elif chunk_type == GLB_CHUNK_BIN:
			bin_chunk = chunk_data

	if typeof(json_doc) == TYPE_DICTIONARY and not bin_chunk.is_empty():
		result = {
			"json": json_doc,
			"bin": bin_chunk,
		}
	_vehicle_glb_cache[model_path] = result
	return result

func _build_vehicle_mesh_from_glb(
	model: Dictionary,
	model_path: String,
	uv_shift: float,
	target_height: float,
	base_rotation: Vector3
) -> ArrayMesh:
	var doc: Dictionary = model.get("json", {})
	var bin: PackedByteArray = model.get("bin", PackedByteArray())
	var scenes: Array = doc.get("scenes", [])
	if scenes.is_empty() or bin.is_empty():
		return null

	var scene_index := int(doc.get("scene", 0))
	if scene_index < 0 or scene_index >= scenes.size():
		return null

	var base_tf = Transform3D().rotated(Vector3.RIGHT, base_rotation.x)
	base_tf = base_tf.rotated(Vector3.UP, base_rotation.y)
	base_tf = base_tf.rotated(Vector3.FORWARD, base_rotation.z)

	var surfaces: Array = []
	var bounds := {
		"has": false,
		"aabb": AABB(),
	}
	var scene: Dictionary = scenes[scene_index]
	for node_index in scene.get("nodes", []):
		_collect_glb_node_surfaces(doc, bin, int(node_index), base_tf, uv_shift, surfaces, bounds)

	if surfaces.is_empty() or not bounds["has"]:
		return null

	var aabb: AABB = bounds["aabb"]
	var scale := 1.0
	if target_height > 0.0 and aabb.size.y > 0.01:
		scale = target_height / aabb.size.y
	var y_offset := -aabb.position.y * scale

	var final_mesh := ArrayMesh.new()
	var material := _vehicle_glb_material(model_path)
	for surface in surfaces:
		var verts: PackedVector3Array = surface["vertices"]
		for i in range(verts.size()):
			var v := verts[i] * scale
			v.y += y_offset
			verts[i] = v

		var arrays := []
		arrays.resize(Mesh.ARRAY_MAX)
		arrays[Mesh.ARRAY_VERTEX] = verts
		arrays[Mesh.ARRAY_NORMAL] = surface["normals"]
		arrays[Mesh.ARRAY_TEX_UV] = surface["uvs"]
		arrays[Mesh.ARRAY_INDEX] = surface["indices"]
		final_mesh.add_surface_from_arrays(Mesh.PRIMITIVE_TRIANGLES, arrays)
		final_mesh.surface_set_material(final_mesh.get_surface_count() - 1, material)

	if lamps_enabled:
		VehicleLamps.add_surfaces(final_mesh, model_path, _lamp_materials)
	return final_mesh

func _vehicle_glb_material(model_path: String) -> Material:
	var mat := StandardMaterial3D.new()
	mat.albedo_texture = _load_source_texture(model_path.get_base_dir() + "/Textures/colormap.png")
	mat.albedo_color = Color.WHITE
	mat.roughness = 0.8
	mat.metallic = 0.0
	mat.cull_mode = BaseMaterial3D.CULL_BACK
	mat.disable_receive_shadows = true
	# Through the building shader, so the clouds dim the sun on cars as on the street.
	return WindowMaterials.create(mat)

func _collect_glb_node_surfaces(
	doc: Dictionary,
	bin: PackedByteArray,
	node_index: int,
	parent_transform: Transform3D,
	uv_shift: float,
	surfaces: Array,
	bounds: Dictionary
) -> void:
	var nodes: Array = doc.get("nodes", [])
	if node_index < 0 or node_index >= nodes.size():
		return

	var node: Dictionary = nodes[node_index]
	var current_transform: Transform3D = parent_transform * _glb_node_transform(node)
	if node.has("mesh"):
		var meshes: Array = doc.get("meshes", [])
		var mesh_index := int(node["mesh"])
		if mesh_index >= 0 and mesh_index < meshes.size():
			var mesh_def: Dictionary = meshes[mesh_index]
			for primitive in mesh_def.get("primitives", []):
				var surface := _read_glb_primitive_surface(
					doc,
					bin,
					primitive,
					current_transform,
					uv_shift,
					bounds
				)
				if not surface.is_empty():
					surfaces.append(surface)

	for child_index in node.get("children", []):
		_collect_glb_node_surfaces(
			doc,
			bin,
			int(child_index),
			current_transform,
			uv_shift,
			surfaces,
			bounds
		)

func _glb_node_transform(node: Dictionary) -> Transform3D:
	if node.has("matrix"):
		var matrix: Array = node["matrix"]
		if matrix.size() >= 16:
			var basis := Basis(
				Vector3(float(matrix[0]), float(matrix[1]), float(matrix[2])),
				Vector3(float(matrix[4]), float(matrix[5]), float(matrix[6])),
				Vector3(float(matrix[8]), float(matrix[9]), float(matrix[10]))
			)
			var origin := Vector3(float(matrix[12]), float(matrix[13]), float(matrix[14]))
			return Transform3D(basis, origin)

	var basis := Basis()
	if node.has("rotation"):
		var rotation: Array = node["rotation"]
		if rotation.size() >= 4:
			basis = Basis(Quaternion(
				float(rotation[0]),
				float(rotation[1]),
				float(rotation[2]),
				float(rotation[3])
			))
	if node.has("scale"):
		basis = basis.scaled(_json_vec3(node["scale"], Vector3(1.0, 1.0, 1.0)))

	return Transform3D(basis, _json_vec3(node.get("translation", []), Vector3.ZERO))

func _json_vec3(value, fallback: Vector3) -> Vector3:
	if not (value is Array) or value.size() < 3:
		return fallback
	return Vector3(float(value[0]), float(value[1]), float(value[2]))

func _read_glb_primitive_surface(
	doc: Dictionary,
	bin: PackedByteArray,
	primitive: Dictionary,
	transform: Transform3D,
	uv_shift: float,
	bounds: Dictionary
) -> Dictionary:
	var attrs: Dictionary = primitive.get("attributes", {})
	if not attrs.has("POSITION"):
		return {}
	if int(primitive.get("mode", GLTF_MODE_TRIANGLES)) != GLTF_MODE_TRIANGLES:
		return {}

	var positions := _read_glb_accessor_vec3(doc, bin, int(attrs["POSITION"]))
	if positions.is_empty():
		return {}

	var normals := PackedVector3Array()
	if attrs.has("NORMAL"):
		normals = _read_glb_accessor_vec3(doc, bin, int(attrs["NORMAL"]))
	var uvs := PackedVector2Array()
	if attrs.has("TEXCOORD_0"):
		uvs = _read_glb_accessor_vec2(doc, bin, int(attrs["TEXCOORD_0"]), uv_shift)

	var indices := PackedInt32Array()
	if primitive.has("indices"):
		indices = _read_glb_accessor_indices(doc, bin, int(primitive["indices"]))
	if indices.is_empty():
		indices.resize(positions.size())
		for i in range(positions.size()):
			indices[i] = i
	indices = _reversed_triangle_winding(indices)

	var verts_out := PackedVector3Array()
	var normals_out := PackedVector3Array()
	var uvs_out := PackedVector2Array()
	verts_out.resize(positions.size())
	normals_out.resize(positions.size())
	uvs_out.resize(positions.size())

	for i in range(positions.size()):
		var pos: Vector3 = transform * positions[i]
		verts_out[i] = pos
		_expand_glb_bounds(bounds, pos)
		if normals.size() == positions.size():
			normals_out[i] = (transform.basis * normals[i]).normalized()
		else:
			normals_out[i] = Vector3.UP
		if uvs.size() == positions.size():
			uvs_out[i] = uvs[i]
		else:
			uvs_out[i] = Vector2.ZERO

	return {
		"vertices": verts_out,
		"normals": normals_out,
		"uvs": uvs_out,
		"indices": indices,
	}

func _reversed_triangle_winding(indices: PackedInt32Array) -> PackedInt32Array:
	var out := indices
	for i in range(0, indices.size() - 2, 3):
		var tmp := out[i]
		out[i] = out[i + 2]
		out[i + 2] = tmp
	return out

func _expand_glb_bounds(bounds: Dictionary, point: Vector3) -> void:
	if not bounds["has"]:
		bounds["aabb"] = AABB(point, Vector3.ZERO)
		bounds["has"] = true
		return
	var aabb: AABB = bounds["aabb"]
	bounds["aabb"] = aabb.expand(point)

func _read_glb_accessor_vec3(doc: Dictionary, bin: PackedByteArray, accessor_index: int) -> PackedVector3Array:
	var layout := _glb_accessor_layout(doc, accessor_index, "VEC3")
	var out := PackedVector3Array()
	if layout.is_empty() or int(layout["component_type"]) != GLTF_COMPONENT_FLOAT:
		return out

	var offset := int(layout["offset"])
	var stride := int(layout["stride"])
	var count := int(layout["count"])
	out.resize(count)
	for i in range(count):
		var p := offset + i * stride
		if p + 12 > bin.size():
			return PackedVector3Array()
		out[i] = Vector3(bin.decode_float(p), bin.decode_float(p + 4), bin.decode_float(p + 8))
	return out

func _read_glb_accessor_vec2(
	doc: Dictionary,
	bin: PackedByteArray,
	accessor_index: int,
	uv_shift: float
) -> PackedVector2Array:
	var layout := _glb_accessor_layout(doc, accessor_index, "VEC2")
	var out := PackedVector2Array()
	if layout.is_empty() or int(layout["component_type"]) != GLTF_COMPONENT_FLOAT:
		return out

	var offset := int(layout["offset"])
	var stride := int(layout["stride"])
	var count := int(layout["count"])
	out.resize(count)
	for i in range(count):
		var p := offset + i * stride
		if p + 8 > bin.size():
			return PackedVector2Array()
		var u := bin.decode_float(p)
		if uv_shift != 0.0:
			u = fmod(u + uv_shift, 1.0)
		out[i] = Vector2(u, bin.decode_float(p + 4))
	return out

func _read_glb_accessor_indices(doc: Dictionary, bin: PackedByteArray, accessor_index: int) -> PackedInt32Array:
	var layout := _glb_accessor_layout(doc, accessor_index, "SCALAR")
	var out := PackedInt32Array()
	if layout.is_empty():
		return out

	var offset := int(layout["offset"])
	var stride := int(layout["stride"])
	var count := int(layout["count"])
	var component_type := int(layout["component_type"])
	out.resize(count)
	for i in range(count):
		var p := offset + i * stride
		if component_type == GLTF_COMPONENT_UNSIGNED_BYTE:
			if p + 1 > bin.size():
				return PackedInt32Array()
			out[i] = int(bin[p])
		elif component_type == GLTF_COMPONENT_UNSIGNED_SHORT:
			if p + 2 > bin.size():
				return PackedInt32Array()
			out[i] = _decode_glb_u16(bin, p)
		elif component_type == GLTF_COMPONENT_UNSIGNED_INT:
			if p + 4 > bin.size():
				return PackedInt32Array()
			out[i] = _decode_glb_u32(bin, p)
		else:
			return PackedInt32Array()
	return out

func _glb_accessor_layout(doc: Dictionary, accessor_index: int, expected_type: String) -> Dictionary:
	var accessors: Array = doc.get("accessors", [])
	if accessor_index < 0 or accessor_index >= accessors.size():
		return {}
	var accessor: Dictionary = accessors[accessor_index]
	if str(accessor.get("type", "")) != expected_type:
		return {}

	var views: Array = doc.get("bufferViews", [])
	var view_index := int(accessor.get("bufferView", -1))
	if view_index < 0 or view_index >= views.size():
		return {}

	var component_type := int(accessor.get("componentType", 0))
	var component_size := _glb_component_size(component_type)
	var component_count := _glb_type_component_count(expected_type)
	if component_size <= 0 or component_count <= 0:
		return {}

	var view: Dictionary = views[view_index]
	var stride := int(view.get("byteStride", component_size * component_count))
	return {
		"offset": int(view.get("byteOffset", 0)) + int(accessor.get("byteOffset", 0)),
		"stride": stride,
		"count": int(accessor.get("count", 0)),
		"component_type": component_type,
	}

func _glb_component_size(component_type: int) -> int:
	match component_type:
		GLTF_COMPONENT_UNSIGNED_BYTE:
			return 1
		GLTF_COMPONENT_UNSIGNED_SHORT:
			return 2
		GLTF_COMPONENT_UNSIGNED_INT, GLTF_COMPONENT_FLOAT:
			return 4
	return 0

func _glb_type_component_count(accessor_type: String) -> int:
	match accessor_type:
		"SCALAR":
			return 1
		"VEC2":
			return 2
		"VEC3":
			return 3
		"VEC4":
			return 4
	return 0

func _decode_glb_u16(bin: PackedByteArray, offset: int) -> int:
	return int(bin[offset]) | (int(bin[offset + 1]) << 8)

func _decode_glb_u32(bin: PackedByteArray, offset: int) -> int:
	return (
		int(bin[offset])
		| (int(bin[offset + 1]) << 8)
		| (int(bin[offset + 2]) << 16)
		| (int(bin[offset + 3]) << 24)
	)

func _build_car_mesh(body_color: Color) -> ArrayMesh:
	# Use plain Array (reference type) so helpers can append to the caller's arrays.
	# Converted to PackedVector3Array only when passing to add_surface_from_arrays.
	var verts: Array = []
	var norms: Array = []

	_add_box(verts, norms, Vector3(-0.9, 0.0, -2.1), Vector3(0.9, 0.55, 2.1))
	# Cabin starts at y=0 so its front/back faces seal the gap at z=±1.1 all the way to ground,
	# preventing the hollow body interior from being visible through the junction edges.
	_add_box(verts, norms, Vector3(-0.9, 0.0, -1.1), Vector3(0.9, 1.2, 1.1))

	var arrays := []
	arrays.resize(Mesh.ARRAY_MAX)
	arrays[Mesh.ARRAY_VERTEX] = PackedVector3Array(verts)
	arrays[Mesh.ARRAY_NORMAL] = PackedVector3Array(norms)

	var mesh := ArrayMesh.new()
	mesh.add_surface_from_arrays(Mesh.PRIMITIVE_TRIANGLES, arrays)

	var mat := StandardMaterial3D.new()
	mat.albedo_color = body_color
	mat.roughness = 0.8
	mat.metallic = 0.0   # metallic without an HDR environment looks glassy/see-through
	mat.disable_receive_shadows = true
	mesh.surface_set_material(0, WindowMaterials.create(mat))

	if lamps_enabled:
		VehicleLamps.add_surfaces(mesh, "procedural", _lamp_materials)
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

func mesh_for_bucket(bucket: int) -> Mesh:
	var model := bucket / 10
	var colour := bucket % 10
	var path: String = MODELS[model]
	var data := _load_vehicle_glb_source(path)
	if data.is_empty():
		return _build_car_mesh(COLOURS[colour])
	return _build_vehicle_mesh_from_glb(data, path, 0.0 if model == 4 else colour * 0.1, 0.0, Vector3(0, PI, 0))
