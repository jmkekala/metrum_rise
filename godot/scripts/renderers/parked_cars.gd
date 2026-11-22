# SPDX-License-Identifier: GPL-3.0-or-later

## Versioned parked chunks. Rust owns supply, custody, pose and persisted palette, and draws the
## chunks through the streaming core's pooled slots (`StreamNode`, `RENDER-32-P`): this script
## builds each vehicle look's four tier meshes once, each registered with its tier's band of
## camera distance (`RENDER-65`) and drawn through a material that keeps only the cars whose own
## distance is in that band (`RENDER-69`), and hands every changed chunk's rows (pose, then custom
## data carrying the paint) over; it creates no node per chunk. Payloads arrive with the moving cars of the same snapshot (agents.gd); one
## acknowledgement answers each, once every chunk of it is drawn (a chunk the pool refused is
## offered again). Unchanged chunks perform no upload. Parked resources omit all lamps and lights.
## The cars draw to the moving cars' distance and fade out over the same band (`RENDER-48`):
## their materials take the band from Rust, and each bucket's batch ends its visibility range in
## Rust from the mesh bounds registered here.
## The lots' bay paint (`PARK-30`) is Rust's too: one line quad with the road marking material is
## registered here, and each frame one call asks whether the paint changed (nothing to do when
## not); changed chunks go to the stream node, which draws one batch a parking chunk.
extends Node3D

const VehicleResources := preload("res://scripts/renderers/vehicle_resources.gd")
@onready var simulation_node: Node = $"../SimulationNode"
@onready var _stream: Node = get_node_or_null("../StreamNode")
var _resources := VehicleResources.new()
# Look → its tier meshes (kept, so their RIDs stay valid).
var _meshes := {}
var _versions := {}
var _epoch := -1
var _full := true
var _fresh := false
var _paint_mesh: ArrayMesh
var _paint_epoch := -1
var _paint_revision := 0

func _ready() -> void:
	_resources.lamps_enabled = false
	_resources.tier_bands = true
	var draw: PackedFloat32Array = SimulationNode.get_agent_draw_distances()
	_resources.draw_fade_m = Vector2(draw[2], draw[3])
	if _stream != null:
		_stream.set_parked_scenario(get_world_3d().scenario)
		_paint_mesh = WorldMaterials.lot_paint_mesh()
		_stream.register_lot_paint_mesh(_paint_mesh.get_rid(), _paint_mesh.get_aabb())

func _process(_delta: float) -> void:
	if _stream == null:
		return
	var payload: Dictionary = simulation_node.get_lot_paint(_paint_epoch, _paint_revision)
	# A refused chunk leaves the held revision as it was, so the same changes come again.
	if not payload.is_empty() and _stream.apply_lot_paint(payload):
		_paint_epoch = payload["epoch"]
		_paint_revision = payload["revision"]

func _exit_tree() -> void:
	if _stream != null:
		_stream.clear_parked_chunks()

## Rebuilds the meshes when the parked buckets are keyed by another catalogue revision (packs
## loaded), and asks for every chunk again. One integer compare a frame otherwise.
func follow_vehicle_catalogue(wanted: int) -> void:
	if not _resources.refresh(simulation_node, wanted):
		return
	# The slots go before the meshes they draw.
	if _stream != null:
		_stream.clear_parked_meshes()
	_meshes.clear()
	_versions.clear()
	_full = true

func needs_full_publication() -> bool:
	return _full

func apply_publication(payload: Dictionary) -> void:
	if _stream == null:
		return
	if _resources.revision < 0:
		_resources.refresh(simulation_node)
	var epoch: int = payload["epoch"]
	if epoch != _epoch:
		# Clears the paint too: it is asked for whole again.
		_stream.clear_parked_chunks()
		_versions.clear()
		_epoch = epoch
		_paint_epoch = -1
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
		for look: int in buckets:
			if not _meshes.has(look):
				_register_look(look)
		if _stream.set_parked_chunk(key, buckets):
			_versions[key] = version
		else:
			complete = false
	if complete:
		simulation_node.acknowledge_parked_cars(epoch, payload["serial"])

# A look's tiers, each car drawn in the tier whose band holds its own camera distance (Rust widens
# each batch's range to cover its cars); with no catalogue look (the procedural fallback) one mesh
# at every distance.
func _register_look(look: int) -> void:
	var meshes: Array[Mesh] = []
	var starts := PackedFloat32Array([0.0])
	if look < _resources.looks.size():
		starts = _resources.looks[look]["tier_starts_m"]
	for tier in starts.size():
		var key := look * VehicleResources.TIERS + tier
		var mesh: Mesh = _resources.mesh_for_bucket(key)
		meshes.append(mesh)
		var end := starts[tier + 1] if tier + 1 < starts.size() else 0.0
		_stream.register_parked_mesh(key, mesh.get_rid(), mesh.get_aabb(), starts[tier], end)
	_meshes[look] = meshes

## The drawn batches (the stream benchmark's digests): `chunk`, `bucket`, `cell` (a near tier's
## cell of the chunk, -1 for a whole chunk), `count` and `rows`, the poses read back from the
## engine (a headless renderer keeps no buffer: then empty).
func drawn_rows() -> Array[Dictionary]:
	var out: Array[Dictionary] = []
	if _stream == null:
		return out
	for item: Dictionary in _stream.get_parked_drawn():
		var count: int = item.count
		var rows := RenderingServer.multimesh_get_buffer(item.multimesh).slice(0, count * 16)
		out.append({"chunk": item.chunk, "bucket": item.bucket, "cell": item.cell, "count": count, "rows": rows})
	return out
