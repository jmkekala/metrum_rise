# SPDX-License-Identifier: GPL-2.0-only

## A load's road chunks, installed in parts (`RENDER-32-P`, `RENDER-36`, `RENDER-52` part 2):
## instead of the whole network's meshes in the load's first frame, the chunk renderer's chunks
## are replaced by none at the network's generation and the streaming core (`StreamNode`) runs
## them nearest first with the view and the near ring first, under a budget a frame. Rust encodes
## a chunk's surfaces in Godot's layout on the background pool and adds one a part to the chunk's
## hidden mesh; the chunk's last part swaps the finished mesh in through the renderer's own
## install, so a chunk draws whole, with its lights and signs, as the full snapshot drew it.
## NetworkRenderer starts a hydration wherever the chunk renderer needed its full snapshot.
extends Node

const PerfDebug := preload("res://scripts/core/perf_debug.gd")
const INSTALLED := 0
const GONE := 1
const BUSY := 2
const CANCELLED := 3
const PART := 4

var simulation_node: Node
# The road tool's chunk renderer (`road_chunk_renderer.gd`, `RENDER-52`).
var chunks: Node
var stream_node: Node
var _plan: Dictionary = {}
# The hydration's chunk grid (its chunks' local frames).
var _span := 0.0
var _origin := Vector2.ZERO

## Makes the road chunks' one-time resources (the sign models and the lamp meshes and kinds) in the
## load call, which waits anyway, instead of in the first chunk's unit (0.3 s at 100k). A world
## without roads makes them too: else its first commit with a lamp or a sign paid them in one
## frame (Kuopio's first commits' 207-222 ms frames, `RENDER-52` part 2).
func prewarm() -> void:
	if chunks.road_signs != null:
		chunks.road_signs._ensure_meshes()
	if chunks.street_lights != null:
		chunks.street_lights._set_kinds()

## Takes the network at `generation` from Rust for a hydration. False when the network moved
## past it (try again later). Nothing is drawn or replaced yet.
func prepare(generation: int) -> bool:
	_plan = simulation_node.begin_road_mesh_hydration(generation)
	return not _plan.is_empty()

## Replaces the renderer's chunks by none at the prepared generation and hands the chunks to the
## streaming core. Returns the generation, or -1 without a prepared hydration.
func commit(invalidate_nodes: bool) -> int:
	if _plan.is_empty():
		return -1
	var plan := _plan
	_plan = {}
	var generation: int = plan["surface_generation"]
	_span = float(plan["chunk_span_m"])
	_origin = Vector2(float(plan["chunk_origin_x_m"]), float(plan["chunk_origin_z_m"]))
	chunks.install(generation, true, _span, _origin.x, _origin.y, [], {}, invalidate_nodes)
	var keys: PackedInt32Array = plan["keys"]
	if not keys.is_empty():
		simulation_node.set_road_hydration_materials(chunks.hydration_materials())
	if stream_node == null:
		# No streaming core (a scene without one): every chunk now, as the full snapshot did.
		for at in range(0, keys.size(), 2):
			var key := Vector2i(keys[at], keys[at + 1])
			simulation_node.prepare_road_mesh_hydration(PackedInt32Array([key.x, key.y]))
			while simulation_node.get_road_mesh_hydration_part_us(key.x, key.y) == -1.0:
				OS.delay_usec(100)
				simulation_node.prepare_road_mesh_hydration(PackedInt32Array([key.x, key.y]))
			while _part(key, generation) == PART:
				pass
		stream_road_hydration_done()
		return generation
	stream_node.begin_road_hydration(self, _span, _origin, keys)
	return generation

## Whether the renderer still waits for its network, or a hydration still installs chunks.
func is_pending() -> bool:
	return chunks.needs_hydration() or (
		stream_node != null and stream_node.has_road_hydration_pending()
	)

## Road chunks not drawn yet any point of which lies within `radius_m` of the camera on the
## ground, the whole network while the renderer still waits for it (the load's hand-over waits
## for none in its near ring, so the hand-over shows the roads it showed before the hydration).
func holes_near(radius_m: float) -> int:
	if chunks.needs_hydration():
		return 1
	return stream_node.get_road_holes_near(radius_m) if stream_node != null else 0

## The head of the streaming core's order (x, z pairs, nearest first): their surfaces are encoded
## on the background pool.
func stream_road_prepare(keys: PackedInt32Array) -> void:
	simulation_node.prepare_road_mesh_hydration(keys)

## Chunk `key`'s next part's predicted microseconds; -1 while its surfaces are not encoded, -2
## gone.
func stream_road_next_part(key: Vector2i) -> float:
	return simulation_node.get_road_mesh_hydration_part_us(key.x, key.y)

## One part (the streaming core calls it): a surface onto chunk `key`'s hidden mesh, or its swap.
func stream_road_part(key: Vector2i) -> int:
	# The renderer's chunks were reset (a world replaced): this hydration is over.
	if chunks.needs_hydration():
		return CANCELLED
	# The installed generation: a newer acknowledged revision is the renderer's now, and this
	# chunk is current at it (that revision's own chunks left the hydration's source).
	return _part(key, int(simulation_node.get_network_render_generation()))

func _part(key: Vector2i, generation: int) -> int:
	# Surfaces go onto a mesh nothing draws yet: no revision waits for them.
	if simulation_node.add_road_mesh_hydration_surface(key.x, key.y):
		return PART
	# A newer revision waits for NetworkRenderer: its terrain is not drawn yet, nor its chunks.
	if simulation_node.is_network_dirty():
		return BUSY
	var start_us := Time.get_ticks_usec()
	var data: Dictionary = simulation_node.take_road_mesh_hydration_chunk(key.x, key.y)
	if data.is_empty():
		return GONE
	var taken_us := Time.get_ticks_usec()
	chunks.install_built(generation, key, data)
	# The swap's split (the stream node records the parts' whole time as `road_hydration`).
	PerfDebug.record("road_units", 0.0, {
		"take_ms": float(taken_us - start_us) / 1000.0,
		"commit_ms": float(Time.get_ticks_usec() - taken_us) / 1000.0,
	})
	return INSTALLED

## The streaming core has no chunk left: the source is let go.
func stream_road_hydration_done() -> void:
	simulation_node.end_road_mesh_hydration()
