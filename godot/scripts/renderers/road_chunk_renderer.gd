# SPDX-License-Identifier: GPL-2.0-only

## The committed road network's chunks (`RENDER-52`, docs/roads.md "Road tools"): the resident
## set, staged updates, the holds of T61 (2), payload validation and the chunk build, with the
## chunks' street lights and road signs. Split out of the road tool's base class; the road tool
## owns one as its child `chunks` (its first, where the chunk root was) and hears it through
## `host` (`mark_network_nodes_dirty`, `road_chunks_installed`, `road_chunks_released`).
##
## Callers: NetworkRenderer (`update`, `commit_staged`, `discard_staged`, `release_held`,
## `needs_hydration`), `road_chunk_hydration.gd` (`build_chunk`, `install`, `street_lights`,
## `road_signs`), the road tool's preview display (`chunks`, `new_chunk_node`, `fill_chunk_mesh`,
## `place_chunk`, `chunk_root`), tools that change the network outside a stroke (`update`).
##
## Rust methods called: get_road_mesh_data(full_snapshot), is_network_dirty(),
##   get_network_render_generation(), get_graded_edit_serial()
extends Node3D

const WorldMaterials = preload("res://scripts/renderers/world_materials.gd")
const SceneLightingConfig := preload("res://scripts/core/scene_lighting.gd")
const StreetLights := preload("res://scripts/renderers/street_lights.gd")
const RoadSigns := preload("res://scripts/renderers/road_signs.gd")

const CHUNK_COORD_MIN := -2147483648
const CHUNK_COORD_MAX := 2147483647

var simulation_node: Node
# The terrain renderer whose graded tiles an install waits for (T61 (2)); none in some tests.
var terrain_node: Node
# The road tool: told when the network's node markers go stale and when chunks install or swap.
var host: Node

## The resident chunks' parent.
var chunk_root: Node3D
## Street lights per road chunk, outside the chunk meshes: junction previews hide those. Lights
## and signs exist only on the scene's road tool (`_ready`); a renderer assembled by a test has
## neither.
var street_lights: StreetLights
var road_signs: RoadSigns
## The resident chunks by key (Vector2i -> MeshInstance3D). Read-only outside this script.
var chunks: Dictionary = {}
## The installed road mesh generation, -1 while the renderer needs a full snapshot.
var generation: int = -1
## The installed chunks' grid: span and the corner of chunk (0, 0).
var span_m: float = 0.0
var origin_x_m: float = 0.0
var origin_z_m: float = 0.0
var _staged: Dictionary = {}
# T61 (2), docs/terrain.md: an install made while a graded terrain edit is still to draw keeps
# the chunks it replaces drawn and its own hidden until the graded renderer has installed the
# edits up to `held_serial` (the journal's serial read at the install), so the road and the
# terrain cut under it change in one frame (the undo's frame had the road gone over the cut).
var _held_chunks: Array[MeshInstance3D] = []
var _hidden_chunks: Array[MeshInstance3D] = []
## The graded edit serial the held install waits for, -1 when nothing is held.
var held_serial: int = -1
## Populated only while benchmark/diagnostic preview meshes are staged.
var upload_metrics: Dictionary = {}

static var _marking_mat: ShaderMaterial = null

func _init() -> void:
	name = "RoadChunks"
	chunk_root = Node3D.new()
	chunk_root.name = "RoadMeshChunks"
	add_child(chunk_root)

func _ready() -> void:
	if host == null or host.name != "RoadTool":
		return
	WorldMaterials.prewarm_road_materials()
	street_lights = StreetLights.new()
	street_lights.name = "StreetLights"
	add_child(street_lights)
	road_signs = RoadSigns.new()
	road_signs.name = "RoadSigns"
	add_child(road_signs)

## Takes the network's road chunks from Rust and installs them (a full snapshot while the renderer
## needs one, else the batch since the last acknowledgement); `stage_only` builds and keeps them
## for `commit_staged`. Returns the generation, or -1 when nothing was taken: a dirty network
## without `expected_generation` (NetworkRenderer pairs dirty revisions with their terrain), a
## different generation, a malformed batch or a chunk that fails validation, with the resident
## set unchanged.
func update(expected_generation: int = -1, stage_only: bool = false) -> int:
	discard_staged()
	if expected_generation < 0 and simulation_node.is_network_dirty():
		return -1
	var data = simulation_node.get_road_mesh_data(generation < 0)
	if typeof(data) != TYPE_DICTIONARY:
		return -1
	if (
		not data.has("surface_generation")
		or typeof(data["surface_generation"]) != TYPE_INT
		or not data.has("full_replace")
		or typeof(data["full_replace"]) != TYPE_BOOL
		or not data.has("chunk_span_m")
		or (
			typeof(data["chunk_span_m"]) != TYPE_FLOAT
			and typeof(data["chunk_span_m"]) != TYPE_INT
		)
		or not data.has("chunk_origin_x_m")
		or (
			typeof(data["chunk_origin_x_m"]) != TYPE_FLOAT
			and typeof(data["chunk_origin_x_m"]) != TYPE_INT
		)
		or not data.has("chunk_origin_z_m")
		or (
			typeof(data["chunk_origin_z_m"]) != TYPE_FLOAT
			and typeof(data["chunk_origin_z_m"]) != TYPE_INT
		)
		or not data.has("chunks")
		or typeof(data["chunks"]) != TYPE_ARRAY
	):
		return -1
	var surface_generation: int = data["surface_generation"]
	if surface_generation < 0:
		return -1
	if expected_generation >= 0 and surface_generation != expected_generation:
		return -1
	if surface_generation < generation:
		return -1
	var full_replace: bool = data["full_replace"]
	if generation < 0 and not full_replace:
		return -1
	var chunk_span_m := float(data["chunk_span_m"])
	if not is_finite(chunk_span_m) or chunk_span_m <= 0.0:
		return -1
	var chunk_origin_x_m := float(data["chunk_origin_x_m"])
	var chunk_origin_z_m := float(data["chunk_origin_z_m"])
	if not is_finite(chunk_origin_x_m) or not is_finite(chunk_origin_z_m):
		return -1
	if not full_replace and chunk_span_m != span_m:
		return -1
	if (
		not full_replace
		and (
			chunk_origin_x_m != origin_x_m
			or chunk_origin_z_m != origin_z_m
		)
	):
		return -1
	var chunk_updates: Array = data["chunks"]
	var validated_updates: Array[Dictionary] = []
	var seen_keys: Dictionary = {}
	for chunk_variant in chunk_updates:
		if typeof(chunk_variant) != TYPE_DICTIONARY:
			return -1
		var chunk_data: Dictionary = chunk_variant
		if (
			not chunk_data.has("chunk_x")
			or typeof(chunk_data["chunk_x"]) != TYPE_INT
			or not chunk_data.has("chunk_z")
			or typeof(chunk_data["chunk_z"]) != TYPE_INT
			or not chunk_data.has("removed")
			or typeof(chunk_data["removed"]) != TYPE_BOOL
		):
			return -1
		if full_replace and chunk_data["removed"]:
			return -1
		var chunk_x: int = chunk_data["chunk_x"]
		var chunk_z: int = chunk_data["chunk_z"]
		if (
			chunk_x < CHUNK_COORD_MIN
			or chunk_x > CHUNK_COORD_MAX
			or chunk_z < CHUNK_COORD_MIN
			or chunk_z > CHUNK_COORD_MAX
		):
			return -1
		var key := Vector2i(chunk_x, chunk_z)
		if seen_keys.has(key):
			return -1
		seen_keys[key] = true
		validated_updates.append(chunk_data)

	var staged_instances: Dictionary = {}
	for chunk_data in validated_updates:
		if chunk_data["removed"]:
			continue
		var key := Vector2i(chunk_data["chunk_x"], chunk_data["chunk_z"])
		var instance := build_chunk(
			chunk_data,
			key,
			chunk_span_m,
			chunk_origin_x_m,
			chunk_origin_z_m
		)
		if instance == null:
			_discard_staged_chunks(staged_instances)
			return -1
		staged_instances[key] = instance

	# Building ArrayMeshes can span frames under a debugger. Never commit a stale batch.
	if int(simulation_node.get_network_render_generation()) != surface_generation:
		_discard_staged_chunks(staged_instances)
		return -1

	if stage_only:
		_staged = {
			"surface_generation": surface_generation,
			"full_replace": full_replace,
			"chunk_span_m": chunk_span_m,
			"chunk_origin_x_m": chunk_origin_x_m,
			"chunk_origin_z_m": chunk_origin_z_m,
			"validated_updates": validated_updates,
			"staged_instances": staged_instances,
		}
		return surface_generation
	return install(
		surface_generation,
		full_replace,
		chunk_span_m,
		chunk_origin_x_m,
		chunk_origin_z_m,
		validated_updates,
		staged_instances
	)

## Installs the batch `update` staged for `expected_generation`. Returns it, or -1 (the staged
## batch is dropped) for none or another generation.
func commit_staged(expected_generation: int) -> int:
	if _staged.is_empty():
		return -1
	var staged := _staged
	_staged = {}
	var surface_generation := int(staged.get("surface_generation", -1))
	if surface_generation != expected_generation or surface_generation < generation:
		_discard_staged_chunks(staged.get("staged_instances", {}) as Dictionary)
		return -1
	# A newer sim revision may arrive after staging. Committing this complete older road/terrain
	# pair is safe; its exact acknowledgement will fail and the newer pair remains dirty.
	return install(
		surface_generation,
		bool(staged["full_replace"]),
		float(staged["chunk_span_m"]),
		float(staged["chunk_origin_x_m"]),
		float(staged["chunk_origin_z_m"]),
		staged["validated_updates"] as Array,
		staged["staged_instances"] as Dictionary,
		false
	)

## Drops a staged batch, if any.
func discard_staged() -> void:
	if _staged.is_empty():
		return
	_discard_staged_chunks(_staged.get("staged_instances", {}) as Dictionary)
	_staged = {}

## Installs a validated batch at `surface_generation`: `validated_updates` (chunk payloads and
## tombstones) give the lights, signs and removals, `staged_instances` (key -> a chunk from
## `build_chunk`) the meshes. A full replace drops every resident chunk first; any other install
## made while a graded edit is still to draw is held (T61 (2)). Returns the generation.
func install(
	surface_generation: int,
	full_replace: bool,
	chunk_span_m: float,
	chunk_origin_x_m: float,
	chunk_origin_z_m: float,
	validated_updates: Array,
	staged_instances: Dictionary,
	invalidate_nodes: bool = true
) -> int:
	if invalidate_nodes and host != null:
		host.mark_network_nodes_dirty()
	var hold := -1 if full_replace else _chunk_hold_serial()
	if hold < 0:
		release_held(true)
	# Lights and signs exist only on the scene's road tool (`_ready`); tests that assemble a
	# renderer by hand have neither.
	var props := street_lights != null and road_signs != null
	if props:
		street_lights.configure_chunks(chunk_span_m, Vector2(chunk_origin_x_m, chunk_origin_z_m))
	if full_replace:
		for key in chunks.keys():
			_remove_chunk(key)
		if props:
			street_lights.clear()
			road_signs.clear()
	for chunk_data in validated_updates:
		var key := Vector2i(chunk_data["chunk_x"], chunk_data["chunk_z"])
		if chunk_data["removed"]:
			_retire_chunk(key, hold >= 0)
			if props:
				street_lights.remove_chunk(key)
				road_signs.remove_chunk(key)
			continue
		if not props:
			continue
		var origin := chunk_frame_centre(key, chunk_span_m, chunk_origin_x_m, chunk_origin_z_m)
		var lights = chunk_data.get("street_lights")
		street_lights.set_chunk(
			key, origin, lights if typeof(lights) == TYPE_PACKED_FLOAT32_ARRAY else PackedFloat32Array()
		)
		var signs = chunk_data.get("road_signs")
		road_signs.set_chunk(
			key, origin, signs if typeof(signs) == TYPE_PACKED_FLOAT32_ARRAY else PackedFloat32Array()
		)
	for key in staged_instances:
		_retire_chunk(key, hold >= 0)
		var instance: MeshInstance3D = staged_instances[key]
		if hold >= 0:
			instance.visible = false
			_hidden_chunks.append(instance)
		chunk_root.add_child(instance)
		chunks[key] = instance
	held_serial = maxi(held_serial, hold)
	generation = surface_generation
	span_m = chunk_span_m
	origin_x_m = chunk_origin_x_m
	origin_z_m = chunk_origin_z_m
	if host != null:
		host.road_chunks_installed(surface_generation)
	return surface_generation

# Any install (an undo, a commit, a deletion) made while the graded journal holds an edit the
# renderer has not installed waits for it (T61 (2)): until then the replaced chunks draw over the
# terrain cut they were baked with. A held commit keeps its preview drawn too (the road tool's
# swapped preview). The acknowledgement is not delayed: only the drawing waits. -1 to draw now.
func _chunk_hold_serial() -> int:
	var graded = _graded_renderer()
	if graded == null or not is_instance_valid(simulation_node):
		return -1
	var serial := int(simulation_node.get_graded_edit_serial())
	return serial if serial > graded.installed_edit_serial() else -1

# Whether the graded renderer has installed the edits up to `serial`.
func _held_edit_installed(serial: int) -> bool:
	var graded = _graded_renderer()
	return graded == null or graded.installed_edit_serial() >= serial

# The terrain renderer's graded tiles, or null without one (tests without terrain).
func _graded_renderer():
	if not is_instance_valid(terrain_node):
		return null
	return terrain_node.get("graded_tiles")

## Whether an install is held for a graded edit (T61 (2)): its chunks are not drawn yet.
func is_holding() -> bool:
	return held_serial >= 0

## Swaps the chunks an install held (T61 (2)) for the installed ones once the graded renderer
## has installed the edits it waited for, or now with `force`. NetworkRenderer calls it each
## frame after the terrain has processed, so the swap draws in the edit batch's frame.
## O(held chunks).
func release_held(force: bool = false) -> void:
	if held_serial < 0 or (not force and not _held_edit_installed(held_serial)):
		return
	for instance in _held_chunks:
		_free_chunk_instance(instance)
	for instance in _hidden_chunks:
		if is_instance_valid(instance):
			instance.visible = true
	_held_chunks.clear()
	_hidden_chunks.clear()
	held_serial = -1
	if host != null:
		host.road_chunks_released()

# Takes `key`'s resident chunk out of the index: kept drawn while `hold` (unless an earlier held
# install put it there, never drawn), else freed.
func _retire_chunk(key: Vector2i, hold: bool) -> void:
	if not hold or not chunks.has(key):
		_remove_chunk(key)
		return
	var instance: MeshInstance3D = chunks[key]
	chunks.erase(key)
	var hidden := _hidden_chunks.find(instance)
	if hidden >= 0:
		_hidden_chunks.remove_at(hidden)
		_free_chunk_instance(instance)
	else:
		_held_chunks.append(instance)

## Drops every chunk, light and sign: the next `update` takes a full snapshot (another world).
func reset() -> void:
	discard_staged()
	release_held(true)
	for key in chunks.keys():
		_remove_chunk(key)
	if street_lights:
		street_lights.clear()
	if road_signs:
		road_signs.clear()
	generation = -1
	span_m = 0.0
	origin_x_m = 0.0
	origin_z_m = 0.0

## Whether the renderer waits for its whole network (a load, a new game): NetworkRenderer
## hydrates it through the streaming core then.
func needs_hydration() -> bool:
	return generation < 0

func _discard_staged_chunks(staged_instances: Dictionary) -> void:
	for instance in staged_instances.values():
		instance.queue_free()

func _remove_chunk(key: Vector2i) -> void:
	if not chunks.has(key):
		return
	var instance: MeshInstance3D = chunks[key]
	chunks.erase(key)
	_free_chunk_instance(instance)

func _free_chunk_instance(instance: MeshInstance3D) -> void:
	if is_instance_valid(instance):
		if instance.get_parent() != null:
			instance.get_parent().remove_child(instance)
		instance.queue_free()

## A detached chunk node for `chunk_data` (a chunk payload of the full snapshot's format), placed
## at its chunk's centre, or null when the payload fails validation. The caller owns it until
## `install` takes it.
func build_chunk(
	chunk_data: Dictionary,
	key: Vector2i,
	chunk_span_m: float,
	chunk_origin_x_m: float,
	chunk_origin_z_m: float
) -> MeshInstance3D:
	var instance := new_chunk_node()
	if not fill_chunk_mesh(instance.mesh, chunk_data):
		instance.free()
		return null
	instance.name = "RoadChunk_%d_%d" % [key.x, key.y]
	_add_lining_shadow(instance, chunk_data)
	place_chunk(instance, key, chunk_span_m, chunk_origin_x_m, chunk_origin_z_m)
	return instance

# A tunnel's lining casts the sun's shadow into its bore (ROAD-61): road chunks only receive
# shadows, so a chunk with a lining layer (only a chunk with a tunnel has one) gets a shadow-only
# child holding that layer again. Bridges' concrete is never copied.
func _add_lining_shadow(instance: MeshInstance3D, chunk_data: Dictionary) -> void:
	var vertices: PackedVector3Array = chunk_data.get("lining_vertices", PackedVector3Array())
	if vertices.is_empty():
		return
	# The layer's positions as fill_chunk_mesh uploaded them, already checked there.
	var arrays := []
	arrays.resize(Mesh.ARRAY_MAX)
	arrays[Mesh.ARRAY_VERTEX] = vertices
	var mesh := ArrayMesh.new()
	mesh.add_surface_from_arrays(Mesh.PRIMITIVE_TRIANGLES, arrays)
	mesh.surface_set_material(0, WorldMaterials.tunnel_lining_shadow_material())
	var caster := MeshInstance3D.new()
	caster.name = "LiningShadow"
	caster.mesh = mesh
	caster.cast_shadow = GeometryInstance3D.SHADOW_CASTING_SETTING_SHADOWS_ONLY
	instance.add_child(caster)

## A detached chunk node with an empty ArrayMesh; callers own it until it enters the tree.
static func new_chunk_node() -> MeshInstance3D:
	var instance := MeshInstance3D.new()
	instance.mesh = ArrayMesh.new()
	SceneLightingConfig.apply_shadow_policy(
		instance,
		SceneLightingConfig.SHADOW_RECEIVER_ONLY,
		"roads"
	)
	return instance

## Places a chunk node at its chunk's centre.
static func place_chunk(
	instance: MeshInstance3D,
	key: Vector2i,
	chunk_span_m: float,
	chunk_origin_x_m: float,
	chunk_origin_z_m: float
) -> void:
	instance.position = chunk_frame_centre(key, chunk_span_m, chunk_origin_x_m, chunk_origin_z_m)

## A road chunk's payload (mesh, lights, signs) is local to the chunk's centre, the frame of the
## terrain patch over it, so shared seam vertices land on the same bits (roads.md, ROAD-68-D).
static func chunk_frame_centre(
	key: Vector2i,
	chunk_span_m: float,
	chunk_origin_x_m: float,
	chunk_origin_z_m: float
) -> Vector3:
	return Vector3(
		chunk_origin_x_m + float(key.x) * chunk_span_m + chunk_span_m * 0.5,
		0.0,
		chunk_origin_z_m + float(key.y) * chunk_span_m + chunk_span_m * 0.5
	)

## A road chunk's layers in payload order, as `NetworkMeshData` lists them (its owner runs index
## these layers). A chunk without a kerbed grass median sends no `median_grass` arrays (ROAD-80),
## which reads as an empty layer.
const ROAD_CHUNK_LAYERS: Array[String] = ["skirt", "curb", "raised_step", "sidewalk", "road", "marking", "concrete", "lining", "median_grass"]
const OPTIONAL_ROAD_CHUNK_LAYER := "median_grass"

## Appends every chunk layer to an empty mesh. Callers discard or clear the mesh on failure. The
## mesh's `road_layer_surfaces` meta maps each layer to its surface (-1 when the layer is empty),
## which the road tool's preview uses to collapse runs of a resident chunk (`ROAD-68-H` round 3).
func fill_chunk_mesh(arr_mesh: ArrayMesh, chunk_data: Dictionary) -> bool:
	_ensure_road_mesh_materials()
	# Rust certifies the exact immutable chunk buffers off the main thread. A missing
	# certificate falls back to the full scan; a malformed one rejects the chunk.
	var certificate = chunk_data.get("road_mesh_payload_validated", false)
	if typeof(certificate) != TYPE_BOOL:
		return false
	var rust_validated: bool = certificate
	# Road skirts (TERRAIN-07-B): the curb's stone, bottoms deepened with distance by the shader.
	var materials: Array[Material] = [
		WorldMaterials.road_skirt_material(),
		WorldMaterials.road_sidewalk_face_material(),
		WorldMaterials.road_sidewalk_face_material(),
		WorldMaterials.road_sidewalk_material(),
		WorldMaterials.road_asphalt_material(),
		_marking_mat,
		WorldMaterials.road_concrete_material(),
		# A tunnel's lining (ROAD-61): concrete, kept from the sun and lit by its lamp strips.
		WorldMaterials.road_concrete_material(),
		# A kerbed grass median's top (ROAD-80): the building sites' lawn, clouds' shade included.
		WorldMaterials.site_ground_material(),
	]
	var surfaces := PackedInt32Array()
	for index in ROAD_CHUNK_LAYERS.size():
		var count := arr_mesh.get_surface_count()
		if not _append_road_surface(arr_mesh, chunk_data, ROAD_CHUNK_LAYERS[index], materials[index], rust_validated):
			return false
		surfaces.append(count if arr_mesh.get_surface_count() > count else -1)
	arr_mesh.set_meta("road_layer_surfaces", surfaces)
	return arr_mesh.get_surface_count() > 0

func _append_road_surface(
	arr_mesh: ArrayMesh,
	chunk_data: Dictionary,
	layer: String,
	material: Material,
	rust_validated: bool
) -> bool:
	var validation_us := Time.get_ticks_usec() if not upload_metrics.is_empty() else 0
	var vertices_key := layer + "_vertices"
	var normals_key := layer + "_normals"
	var colors_key := layer + "_colors"
	var uvs_key := layer + "_uvs"
	if layer == OPTIONAL_ROAD_CHUNK_LAYER and not chunk_data.has(vertices_key):
		return true
	if (
		not chunk_data.has(vertices_key)
		or typeof(chunk_data[vertices_key]) != TYPE_PACKED_VECTOR3_ARRAY
		or not chunk_data.has(normals_key)
		or typeof(chunk_data[normals_key]) != TYPE_PACKED_VECTOR3_ARRAY
		or not chunk_data.has(colors_key)
		or typeof(chunk_data[colors_key]) != TYPE_PACKED_COLOR_ARRAY
		or not chunk_data.has(uvs_key)
		or typeof(chunk_data[uvs_key]) != TYPE_PACKED_VECTOR2_ARRAY
	):
		return false
	var vertices: PackedVector3Array = chunk_data[vertices_key]
	var normals: PackedVector3Array = chunk_data[normals_key]
	var colors: PackedColorArray = chunk_data[colors_key]
	var uvs: PackedVector2Array = chunk_data[uvs_key]
	if (
		vertices.size() % 3 != 0
		or normals.size() != vertices.size()
		or colors.size() != vertices.size()
		or uvs.size() != vertices.size()
	):
		return false
	if vertices.is_empty():
		return true
	if material == null:
		return false
	if not rust_validated and not _road_layer_is_finite(vertices, normals, uvs, colors):
		return false
	var upload_us := Time.get_ticks_usec() if not upload_metrics.is_empty() else 0
	if not upload_metrics.is_empty():
		upload_metrics["road_validation_ms"] = upload_metrics.get("road_validation_ms", 0.0) + float(upload_us - validation_us) / 1000.0
	var arrays := []
	arrays.resize(Mesh.ARRAY_MAX)
	arrays[Mesh.ARRAY_VERTEX] = vertices
	arrays[Mesh.ARRAY_NORMAL] = normals
	arrays[Mesh.ARRAY_COLOR] = colors
	arrays[Mesh.ARRAY_TEX_UV] = uvs
	var flags := 0
	if layer == "road":
		# Full-precision crossing metres/age must not pass through 8-bit vertex COLOR.
		var crossing_data = chunk_data.get("road_crossings", null)
		if typeof(crossing_data) != TYPE_PACKED_FLOAT32_ARRAY:
			return false
		var crossings: PackedFloat32Array = crossing_data
		if crossings.size() != vertices.size() * 4:
			return false
		if not rust_validated:
			for value in crossings:
				if not is_finite(value):
					return false
		arrays[Mesh.ARRAY_CUSTOM0] = crossings
		flags = Mesh.ARRAY_CUSTOM_RGBA_FLOAT << Mesh.ARRAY_FORMAT_CUSTOM0_SHIFT
	var previous_surface_count := arr_mesh.get_surface_count()
	arr_mesh.add_surface_from_arrays(Mesh.PRIMITIVE_TRIANGLES, arrays, [], {}, flags)
	if arr_mesh.get_surface_count() != previous_surface_count + 1:
		return false
	arr_mesh.surface_set_material(previous_surface_count, material)
	if not upload_metrics.is_empty():
		upload_metrics["road_submit_ms"] = upload_metrics.get("road_submit_ms", 0.0) + float(Time.get_ticks_usec() - upload_us) / 1000.0
		upload_metrics["road_layers_created"] = upload_metrics.get("road_layers_created", 0) + 1
	return true

func _road_layer_is_finite(
	vertices: PackedVector3Array,
	normals: PackedVector3Array,
	uvs: PackedVector2Array,
	colors: PackedColorArray
) -> bool:
	for index in vertices.size():
		var vertex := vertices[index]
		var normal := normals[index]
		var uv := uvs[index]
		var color := colors[index]
		if (
			not is_finite(vertex.x)
			or not is_finite(vertex.y)
			or not is_finite(vertex.z)
			or not is_finite(normal.x)
			or not is_finite(normal.y)
			or not is_finite(normal.z)
			or not is_finite(uv.x)
			or not is_finite(uv.y)
			or not is_finite(color.r)
			or not is_finite(color.g)
			or not is_finite(color.b)
			or not is_finite(color.a)
		):
			return false
	return true

static func _ensure_road_mesh_materials() -> void:
	if _marking_mat == null:
		_marking_mat = WorldMaterials.road_marking_material()
