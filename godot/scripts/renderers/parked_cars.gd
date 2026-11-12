# SPDX-License-Identifier: GPL-2.0-only

## Versioned parked chunks. Rust owns supply, custody, pose and persisted palette, and draws the
## chunks through the streaming core's pooled slots (`StreamNode`, `RENDER-32-P`): this script
## builds each vehicle bucket's mesh once and hands every changed chunk's poses over; it creates
## no node per chunk. Payloads arrive with the moving cars of the same snapshot (agents.gd); one
## acknowledgement answers each, once every chunk of it is drawn (a chunk the pool refused is
## offered again). Unchanged chunks perform no upload. Parked resources omit all lamps and lights.
extends Node3D

const VehicleResources := preload("res://scripts/renderers/vehicle_resources.gd")
@onready var simulation_node: Node = $"../SimulationNode"
@onready var _stream: Node = get_node_or_null("../StreamNode")
var _resources := VehicleResources.new()
var _meshes := {}
var _versions := {}
var _epoch := -1
var _full := true
var _fresh := false

func _ready() -> void:
	_resources.lamps_enabled = false
	if _stream != null:
		_stream.set_parked_scenario(get_world_3d().scenario)

func _exit_tree() -> void:
	if _stream != null:
		_stream.clear_parked_chunks()

func needs_full_publication() -> bool:
	return _full

func apply_publication(payload: Dictionary) -> void:
	if _stream == null:
		return
	var epoch: int = payload["epoch"]
	if epoch != _epoch:
		_stream.clear_parked_chunks()
		_versions.clear()
		_epoch = epoch
	if payload["full"]:
		_full = false
	# Motion vectors (TAA, FSR 2) keep a buffer's previous upload: reused slots take fresh storage.
	var viewport := get_viewport()
	var fresh := viewport.use_taa or viewport.scaling_3d_mode == Viewport.SCALING_3D_MODE_FSR2
	if fresh != _fresh:
		_fresh = fresh
		_stream.set_parked_fresh_buffers(fresh)
	var complete := true
	for update: Dictionary in payload["chunks"]:
		var key: Vector2i = update["chunk"]
		var version: int = update["version"]
		if _versions.get(key, -1) == version:
			continue
		var buckets: Dictionary = update["buckets"]
		for bucket: int in buckets:
			if not _meshes.has(bucket):
				_meshes[bucket] = _resources.mesh_for_bucket(bucket)
				_stream.register_parked_mesh(bucket, _meshes[bucket].get_rid())
		if _stream.set_parked_chunk(key, buckets):
			_versions[key] = version
		else:
			complete = false
	if complete:
		simulation_node.acknowledge_parked_cars(epoch, payload["serial"])

## The drawn buckets (the stream benchmark's digests): `chunk`, `bucket`, `count` and `rows`, the
## poses read back from the engine (a headless renderer keeps no buffer: then empty).
func drawn_rows() -> Array[Dictionary]:
	var out: Array[Dictionary] = []
	if _stream == null:
		return out
	for item: Dictionary in _stream.get_parked_drawn():
		var count: int = item.count
		var rows := RenderingServer.multimesh_get_buffer(item.multimesh).slice(0, count * 12)
		out.append({"chunk": item.chunk, "bucket": item.bucket, "count": count, "rows": rows})
	return out
