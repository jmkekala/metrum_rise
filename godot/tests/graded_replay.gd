# SPDX-License-Identifier: GPL-2.0-only

## Replays a recorded graded tile renderer trace (terrain_graded_trace.gd, `RENDER-32-I`) through
## the renderer as it is now, headless and without Rust's residency jobs: the streaming core's
## resident entries and terrain.gd's patch nodes (since `RENDER-32-K` part 6), entry calls, polled
## results, Rust's replies and the install budget's answers are fed back in order, and every output
## (Rust calls, page creations and moves, slot uploads, cell buffer publications, draw states, wall
## faces) must equal the recorded one. A trace recorded before the core's hand-off (the resident
## patch dictionary's mutations and `process` frames with their changed keys) is fed through the
## same entries: each changed key's entry set in the book and its node lent, then the frame run
## with the recorded keys as its news (`RENDER-32-K` part 7, which removed the dictionary-fed
## frame). The
## trace is METRUM_GRADED_REPLAY or the first user argument; a list of traces separated by `;`
## replays each; METRUM_GRADED_REPLAY_POOL=unbounded replays traces recorded before `TERRAIN-09`'s
## bound on the pooled pages. Exit 0 when every trace replays to its end.
extends SceneTree

const Tiles := preload("res://scripts/renderers/terrain_graded_tiles.gd")
const Trace := preload("res://scripts/renderers/terrain_graded_trace.gd")

var _nodes := {}
var _materials := {}
var _shader: Shader = null

func _initialize() -> void:
	var paths := OS.get_environment("METRUM_GRADED_REPLAY")
	if paths.is_empty() and not OS.get_cmdline_user_args().is_empty():
		paths = OS.get_cmdline_user_args()[0]
	if paths.is_empty():
		push_error("graded_replay: no trace (METRUM_GRADED_REPLAY)")
		quit(2)
		return
	# The patches' own shader: a cell set's material sync walks its uniforms under the budget, so
	# the replay's patches must list the same ones (their values stay unset).
	_shader = load("res://assets/materials/terrain.gdshader")
	var failed := 0
	for path in paths.split(";", false):
		if not _replay(path):
			failed += 1
	for node: Node in _nodes.values():
		node.free()
	quit(1 if failed > 0 else 0)

var _trace: RefCounted = null

func _node(id: int) -> GeometryInstance3D:
	if id == 0:
		return null
	if not _nodes.has(id):
		_nodes[id] = MeshInstance3D.new()
	# The renderer's trace lines name the recorded node.
	_trace.recorded_ids[(_nodes[id] as Object).get_instance_id()] = id
	return _nodes[id]

func _material(id: int) -> ShaderMaterial:
	if not _materials.has(id):
		var material := ShaderMaterial.new()
		material.shader = _shader
		_materials[id] = material
	return _materials[id]

# A pre-hand-off trace's resident entry as the book takes it: its rectangle, step and generation
# (empty: not resident) without news, and its node lent (or taken back).
func _legacy_patch(renderer: RefCounted, key: Vector2i, entry: Dictionary) -> void:
	if entry.is_empty():
		renderer._book.set_patch(key, false, Rect2(), 0, 0)
		renderer.set_patch_node(key, null, null, AABB())
		return
	renderer._book.set_patch(key, true, entry["rect"], int(entry["step"]), int(entry["generation"]))
	renderer.set_patch_node(key, entry["node"], entry["material"], entry["aabb"])

func _replay(path: String) -> bool:
	var started := Time.get_ticks_msec()
	var trace: RefCounted = Trace.replay(path)
	if trace == null:
		print("graded_replay: FAIL %s: unreadable" % path)
		return false
	var renderer: RefCounted = Tiles.new()
	renderer.trace = trace
	_trace = trace
	# A pre-hand-off trace's resident dictionary, and whether its first frame synced the book.
	var resident := {}
	var synced := false
	var calls := {}
	while trace.has_next():
		var line: String = trace.peek()
		var words := line.split(" ")
		if words.size() < 2 or words[0] != "<":
			trace.failure = "line %d: expected an entry, found: %s" % [trace._at + 1, line]
			break
		var kind := words[1]
		calls[kind] = int(calls.get(kind, 0)) + 1
		match kind:
			"setup":
				var values: PackedFloat32Array = Trace.floats_of(words[2])
				var slots := Array(Trace.ints_of(words[3]))
				renderer.setup(null, values[0], Vector2(values[1], values[2]), values[3], slots)
				# Traces recorded before `TERRAIN-09`'s pool bound replay with the pool unbounded.
				if OS.get_environment("METRUM_GRADED_REPLAY_POOL") == "unbounded":
					renderer.page_store.book.set_pool_kept_bytes(-1)
			"patch":
				trace.take()
				var rect: PackedFloat32Array = Trace.floats_of(words[6])
				var box: PackedFloat32Array = Trace.floats_of(words[9])
				var key := Vector2i(int(words[2]), int(words[3]))
				resident[key] = {
					"node": _node(int(words[4])), "material": _material(int(words[5])),
					"rect": Rect2(rect[0], rect[1], rect[2], rect[3]), "step": int(words[7]), "generation": int(words[8]),
					"aabb": AABB(Vector3(box[0], box[1], box[2]), Vector3(box[3], box[4], box[5])),
				}
				if synced:
					_legacy_patch(renderer, key, resident[key])
			"patch_erase":
				trace.take()
				var key := Vector2i(int(words[2]), int(words[3]))
				resident.erase(key)
				if synced:
					_legacy_patch(renderer, key, {})
			"patches_clear":
				trace.take()
				resident.clear()
				renderer._book.forget_patches()
			"process":
				# A pre-hand-off frame: its first syncs the book with the dictionary, each takes its
				# changed keys' entries again and follows them as its news.
				trace.take()
				var camera: PackedFloat32Array = Trace.floats_of(words[2])
				renderer.unbudgeted = words[3] == "1"
				renderer.residency_settling = words[4] == "1"
				if not synced:
					renderer._book.clear_resident()
					for key: Vector2i in resident:
						_legacy_patch(renderer, key, resident[key])
					synced = true
				var news := PackedInt32Array()
				for key: Vector2i in Trace.keys_of(words[5]):
					_legacy_patch(renderer, key, resident.get(key, {}))
					news.append_array(PackedInt32Array([key.x, key.y]))
				renderer._run_frame(Vector3(camera[0], camera[1], camera[2]), news, Trace.keys_of(words[6]))
			# Traces recorded since the streaming core hands the resident patches over
			# (`RENDER-32-K` part 6): the core's entries, terrain.gd's nodes, the frames.
			"patch_core":
				trace.take()
				var rect: PackedFloat32Array = Trace.floats_of(words[4])
				renderer.page_store.book.core_patch(Vector2i(int(words[2]), int(words[3])), true,
					Rect2(rect[0], rect[1], rect[2], rect[3]), int(words[5]), int(words[6]))
			"patch_core_erase":
				trace.take()
				renderer.page_store.book.core_patch(Vector2i(int(words[2]), int(words[3])), false, Rect2(), 0, 0)
			"patch_node":
				trace.take()
				var box: PackedFloat32Array = Trace.floats_of(words[6])
				renderer.set_patch_node(Vector2i(int(words[2]), int(words[3])), _node(int(words[4])), _material(int(words[5])),
					AABB(Vector3(box[0], box[1], box[2]), Vector3(box[3], box[4], box[5])))
			"patch_node_erase":
				trace.take()
				renderer.set_patch_node(Vector2i(int(words[2]), int(words[3])), null, null, AABB())
			"frame":
				var camera: PackedFloat32Array = Trace.floats_of(words[2])
				renderer.unbudgeted = words[3] == "1"
				renderer.residency_settling = words[4] == "1"
				renderer.process(Vector3(camera[0], camera[1], camera[2]), Trace.keys_of(words[6]))
			"preview":
				renderer.show_preview(Trace.preview_of(line.substr(2)))
			"release_preview":
				renderer.release_preview(int(words[2]), int(words[3]))
			"release_patch":
				renderer.release_patch(Vector2i(int(words[2]), int(words[3])), _node(int(words[4])))
			"clear":
				renderer.clear()
			_:
				trace.failure = "line %d: unknown entry: %s" % [trace._at + 1, line]
	# Its own disposal is not part of the trace.
	renderer.trace = null
	renderer.page_store.trace = null
	renderer._book.set_trace(null)
	renderer.dispose()
	var seconds := float(Time.get_ticks_msec() - started) / 1000.0
	if not trace.failure.is_empty():
		print("graded_replay: FAIL %s: %s" % [path, trace.failure])
		return false
	print("graded_replay: PASS %s: %d lines, %d outputs, %d inputs, %s, %.1f s" % [path, trace._lines.size(), trace.outputs, trace.inputs, str(calls), seconds])
	return true
