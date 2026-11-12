# SPDX-License-Identifier: GPL-2.0-only

## A load's road chunks, installed one at a time (`RENDER-32-P`, `RENDER-36`): instead of the
## whole network's meshes in the load's first frame, the road tool's chunks are replaced by none
## at the network's generation and the streaming core (`StreamNode`) runs one unit a chunk,
## nearest first with the view and the near ring first, under a budget a frame. Each unit packs
## its chunk from the hydration's source in Rust, builds it with the road tool's own builder and
## installs it through the road tool's own commit, so a chunk draws whole, with its lights and
## signs, as the full snapshot drew it. NetworkRenderer starts a hydration wherever the road tool
## needed its full snapshot.
extends Node

const PerfDebug := preload("res://scripts/core/perf_debug.gd")
const INSTALLED := 0
const GONE := 1
const BUSY := 2
const CANCELLED := 3

var simulation_node: Node
var road_tool: Node
var stream_node: Node
var _plan: Dictionary = {}
# The hydration's chunk grid (its chunks' local frames).
var _span := 0.0
var _origin := Vector2.ZERO

## Makes the road chunks' one-time resources (the sign models and the lamp meshes and kinds) in the
## load call, which waits anyway, instead of in the first chunk's unit (0.3 s at 100k).
func prewarm() -> void:
	var signs: Node = road_tool.get("_road_signs")
	if signs != null:
		signs._ensure_meshes()
	var lights: Node = road_tool.get("_street_lights")
	if lights != null:
		lights._set_kinds()

## Takes the network at `generation` from Rust for a hydration. False when the network moved
## past it (try again later). Nothing is drawn or replaced yet.
func prepare(generation: int) -> bool:
	_plan = simulation_node.begin_road_mesh_hydration(generation)
	return not _plan.is_empty()

## Replaces the road tool's chunks by none at the prepared generation and hands the chunks to the
## streaming core. Returns the generation, or -1 without a prepared hydration.
func commit(invalidate_nodes: bool) -> int:
	if _plan.is_empty():
		return -1
	var plan := _plan
	_plan = {}
	var generation: int = plan["surface_generation"]
	_span = float(plan["chunk_span_m"])
	_origin = Vector2(float(plan["chunk_origin_x_m"]), float(plan["chunk_origin_z_m"]))
	road_tool._commit_road_mesh_update(
		generation, true, _span, _origin.x, _origin.y, [], {}, invalidate_nodes
	)
	if stream_node == null:
		# No streaming core (a scene without one): every chunk now, as the full snapshot did.
		var keys: PackedInt32Array = plan["keys"]
		for at in range(0, keys.size(), 2):
			_install(Vector2i(keys[at], keys[at + 1]), generation)
		stream_road_hydration_done()
		return generation
	stream_node.begin_road_hydration(self, _span, _origin, plan["keys"], plan["vertices"])
	return generation

## Whether the road tool still waits for its network, or a hydration still installs chunks.
func is_pending() -> bool:
	return road_tool.needs_main_mesh_hydration() or (
		stream_node != null and stream_node.has_road_hydration_pending()
	)

## Road chunks not drawn yet any point of which lies within `radius_m` of the camera on the
## ground, the whole network while the road tool still waits for it (the load's hand-over waits
## for none in its near ring, so the hand-over shows the roads it showed before the hydration).
func holes_near(radius_m: float) -> int:
	if road_tool.needs_main_mesh_hydration():
		return 1
	return stream_node.get_road_holes_near(radius_m) if stream_node != null else 0

## One unit (the streaming core calls it): chunk `key` packed, built and installed.
func stream_road_chunk(key: Vector2i) -> int:
	# The road tool's chunks were reset (a world replaced): this hydration is over.
	if road_tool.needs_main_mesh_hydration():
		return CANCELLED
	# A newer revision waits for NetworkRenderer: its terrain is not drawn yet, nor its chunks.
	if simulation_node.is_network_dirty():
		return BUSY
	# The installed generation: a newer acknowledged revision is the road tool's now, and this
	# chunk is current at it (that revision's own chunks left the hydration's source).
	return _install(key, int(simulation_node.get_network_render_generation()))

func _install(key: Vector2i, generation: int) -> int:
	var start_us := Time.get_ticks_usec()
	var data: Dictionary = simulation_node.get_road_mesh_hydration_chunk(key.x, key.y)
	if data.is_empty():
		return GONE
	var packed_us := Time.get_ticks_usec()
	var instance: MeshInstance3D = road_tool._build_road_chunk_instance(
		data, key, _span, _origin.x, _origin.y
	)
	if instance == null:
		push_error("Road chunk %s failed validation during hydration" % key)
		return GONE
	var built_us := Time.get_ticks_usec()
	road_tool._commit_road_mesh_update(
		generation, false, _span, _origin.x, _origin.y, [data], {key: instance}, false
	)
	# The unit's split (the stream node records the unit's whole time as `road_hydration`).
	PerfDebug.record("road_units", 0.0, {
		"pack_ms": float(packed_us - start_us) / 1000.0,
		"build_ms": float(built_us - packed_us) / 1000.0,
		"commit_ms": float(Time.get_ticks_usec() - built_us) / 1000.0,
	})
	return INSTALLED

## The streaming core has no chunk left: the source is let go.
func stream_road_hydration_done() -> void:
	simulation_node.end_road_mesh_hydration()
