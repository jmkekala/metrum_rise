# SPDX-License-Identifier: GPL-2.0-only

## Versioned parked MultiMesh chunks. Rust owns supply, custody, pose and persisted palette.
## Payloads arrive with the moving cars of the same snapshot (agents.gd); one acknowledgement
## answers each. Unchanged chunks perform no upload. Parked resources omit all lamps and lights.
extends Node3D

const SceneLighting := preload("res://scripts/core/scene_lighting.gd")
const VehicleResources := preload("res://scripts/renderers/vehicle_resources.gd")
@onready var simulation_node: Node = $"../SimulationNode"
var _resources := VehicleResources.new()
var _meshes := {}
var _chunks := {}
var _versions := {}
var _epoch := -1
var _full := true

func _ready() -> void:
	_resources.lamps_enabled = false

func needs_full_publication() -> bool:
	return _full

func apply_publication(payload: Dictionary) -> void:
	var epoch: int = payload["epoch"]
	if epoch != _epoch:
		for chunk: Node in _chunks.values():
			chunk.queue_free()
		_chunks.clear()
		_versions.clear()
		_epoch = epoch
	if payload["full"]:
		_full = false
	for update: Dictionary in payload["chunks"]:
		var key: Vector2i = update["chunk"]
		var version: int = update["version"]
		if _versions.get(key, -1) != version:
			_install(key, update["buckets"])
			_versions[key] = version
	simulation_node.acknowledge_parked_cars(epoch, payload["serial"])

func _install(key: Vector2i, buckets: Dictionary) -> void:
	if _chunks.has(key):
		_chunks[key].queue_free()
		_chunks.erase(key)
	if buckets.is_empty():
		return
	var chunk := Node3D.new()
	for bucket: int in buckets:
		if not _meshes.has(bucket):
			_meshes[bucket] = _resources.mesh_for_bucket(bucket)
		var buffer: PackedFloat32Array = buckets[bucket]
		var instances := MultiMesh.new()
		instances.transform_format = MultiMesh.TRANSFORM_3D
		instances.mesh = _meshes[bucket]
		instances.instance_count = buffer.size() / 12
		instances.buffer = buffer
		var view := MultiMeshInstance3D.new()
		view.multimesh = instances
		SceneLighting.apply_shadow_policy(view, SceneLighting.SHADOW_DYNAMIC_CASTER, "cars")
		chunk.add_child(view)
	add_child(chunk)
	_chunks[key] = chunk
