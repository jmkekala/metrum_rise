# SPDX-License-Identifier: GPL-2.0-only

## Junction connector overlay for the `junction_connectors` visual mode.
##
## Draws the connector lanes cars drive through junctions near the camera as one line mesh,
## built by Rust (`get_junction_debug_lines`). The mesh is rebuilt only when the network
## render generation, the view, the focused junction or the camera's 512 m chunk changes; the
## `occupancy` view also refreshes four times a second. While the view is on, the overlay also
## runs the Rust stuck-at-junction detector, which logs on `events`. Off, it does no work.
extends Node3D

const CHUNK_SIZE_M := 512.0
# The camera's chunk plus one ring of neighbours.
const HALF_EXTENT_M := 768.0
const OCCUPANCY_REFRESH_S := 0.25
const STALL_SCAN_S := 0.25
const PICK_RADIUS_M := 12.0
const VIEW_OCCUPANCY := 3

var simulation_node: Node
## Counts from the last build: `junctions`, `connectors`, `pass_throughs`, `occupied`, `blocked`.
var last_counts := {}
## Vertices in the drawn mesh.
var vertex_count := 0

var _mesh_instance: MeshInstance3D
var _mesh := ArrayMesh.new()
var _view := 0
var _focus_node := -1
var _dirty := true
var _built_generation := -1
var _built_chunk := Vector2i(2147483647, 2147483647)
var _occupancy_elapsed_s := 0.0
var _stall_elapsed_s := 0.0

func _ready() -> void:
	simulation_node = get_node_or_null("../SimulationNode")
	var material := StandardMaterial3D.new()
	material.shading_mode = BaseMaterial3D.SHADING_MODE_UNSHADED
	material.vertex_color_use_as_albedo = true
	material.transparency = BaseMaterial3D.TRANSPARENCY_ALPHA
	_mesh_instance = MeshInstance3D.new()
	_mesh_instance.mesh = _mesh
	_mesh_instance.material_override = material
	_mesh_instance.cast_shadow = GeometryInstance3D.SHADOW_CASTING_SETTING_OFF
	add_child(_mesh_instance)
	var console := get_node_or_null("/root/DebugConsole")
	if console != null:
		console.visual_mode_changed.connect(_on_visual_mode_changed)
	_apply_view()

func _process(delta: float) -> void:
	if simulation_node == null:
		return
	_stall_elapsed_s += delta
	if _stall_elapsed_s >= STALL_SCAN_S:
		_stall_elapsed_s = 0.0
		simulation_node.scan_junction_stalls()
	if _view == VIEW_OCCUPANCY:
		_occupancy_elapsed_s += delta
		if _occupancy_elapsed_s >= OCCUPANCY_REFRESH_S:
			_occupancy_elapsed_s = 0.0
			_dirty = true
	var generation: int = simulation_node.get_network_render_generation()
	var chunk := _camera_chunk()
	if _dirty or generation != _built_generation or (_focus_node < 0 and chunk != _built_chunk):
		_rebuild(generation, chunk)

## Shows only `node_id`'s connectors and logs them on `events`; `-1` returns to the camera area.
func focus_junction(node_id: int) -> void:
	if node_id == _focus_node:
		return
	_focus_node = node_id
	_dirty = true
	if node_id >= 0 and simulation_node != null:
		simulation_node.log_junction_connectors(node_id)

## Focuses the junction nearest `world_pos`. Returns `true` when one was hit, so the click is
## consumed; a miss clears any focus and leaves the click to the caller.
func try_focus_at(world_pos: Vector3) -> bool:
	if _view == 0 or simulation_node == null:
		return false
	var node_id: int = simulation_node.pick_debug_junction(world_pos, PICK_RADIUS_M)
	focus_junction(node_id)
	return node_id >= 0

## Clears the focus. Returns `true` if a junction was focused.
func unfocus() -> bool:
	if _focus_node < 0:
		return false
	focus_junction(-1)
	return true

## Returns `true` while the overlay draws.
func is_active() -> bool:
	return _view != 0

func _on_visual_mode_changed(mode: String) -> void:
	if mode == "junction_connectors":
		_apply_view()

func _apply_view() -> void:
	_view = MetrumDebug.visual_mode_index("junction_connectors")
	_dirty = true
	set_process(_view != 0)
	if _view != 0:
		return
	_focus_node = -1
	_mesh.clear_surfaces()
	vertex_count = 0
	last_counts = {}
	if simulation_node != null:
		simulation_node.clear_junction_stalls()

func _rebuild(generation: int, chunk: Vector2i) -> void:
	var center := Vector3((chunk.x + 0.5) * CHUNK_SIZE_M, 0.0, (chunk.y + 0.5) * CHUNK_SIZE_M)
	var data: Dictionary = simulation_node.get_junction_debug_lines(_view, center, HALF_EXTENT_M, _focus_node)
	# Empty while the simulation core is busy: keep the last mesh and retry next frame.
	if data.is_empty():
		return
	_dirty = false
	_built_generation = generation
	_built_chunk = chunk
	var points: PackedVector3Array = data["points"]
	_mesh.clear_surfaces()
	vertex_count = points.size()
	if vertex_count > 0:
		var arrays := []
		arrays.resize(Mesh.ARRAY_MAX)
		arrays[Mesh.ARRAY_VERTEX] = points
		arrays[Mesh.ARRAY_COLOR] = data["colors"]
		_mesh.add_surface_from_arrays(Mesh.PRIMITIVE_LINES, arrays)
	data.erase("points")
	data.erase("colors")
	last_counts = data

func _camera_chunk() -> Vector2i:
	var camera := get_viewport().get_camera_3d()
	if camera == null:
		return _built_chunk
	var pos: Vector3 = camera.get_focus_position() if camera.has_method("get_focus_position") else camera.global_position
	return Vector2i(floori(pos.x / CHUNK_SIZE_M), floori(pos.z / CHUNK_SIZE_M))
