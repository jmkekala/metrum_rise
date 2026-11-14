# SPDX-License-Identifier: GPL-2.0-only

## A committed stroke's products as digests (`ROAD-112` round C, the straight parity check): the
## request the road tool sent (its points), the graph (each edge slot's geometry, width and class,
## and the network's counters), the road chunk meshes as Rust publishes them, and the graded tiles
## installed (their level, step, walls and every page copy read back, as
## `stream_frame_benchmark.gd`'s graded digest). The chunks drawn at that moment are no product:
## their server surfaces differ between two runs of one build. Two runs of the same strokes
## compare these bitwise. Untimed: it reads pages back and locks the core per edge, so a timed run
## leaves it off.
extends RefCounted

## The products after a commit's edit installed: `request`, `graph`, `state`, `mesh` (the network's
## road chunk meshes, `get_road_mesh_data`'s full snapshot), `graded` (sha256 hex digests) and the
## counts behind them. Also, so a change of the curve warning signs alone can be told apart
## (`ROAD-112` round E): `mesh_without_signs` (the same snapshot with every chunk's `road_signs`
## cleared) and `curve_signs`, every curve warning sign as `kind x z` (sign kinds 7..10).
static func record(simulation: Node, terrain: Node, request: PackedVector3Array) -> Dictionary:
	var mesh: Dictionary = simulation.get_road_mesh_data(true)
	var curve_signs := PackedStringArray()
	var without_signs := mesh.duplicate(true)
	for chunk: Dictionary in without_signs.get("chunks", []):
		var signs: PackedFloat32Array = chunk.get("road_signs", PackedFloat32Array())
		# Five floats a sign: x, y, z, yaw, kind.
		for index in range(0, signs.size(), 5):
			var kind := int(signs[index + 4])
			if kind >= 7 and kind <= 10:
				curve_signs.append("%d %.3f %.3f" % [kind, signs[index], signs[index + 2]])
		chunk["road_signs"] = PackedFloat32Array()
	curve_signs.sort()
	var state: Dictionary = simulation.get_road_benchmark_state()
	var edges := PackedStringArray()
	for edge in int(state.get("edge_slots", 0)):
		edges.append("%d|%s|%s|%d" % [edge, _sha(var_to_bytes(simulation.get_edge_geometry_3d(edge))),
			_sha(var_to_bytes(simulation.get_edge_width(edge))), int(simulation.get_edge_class(edge))])
	var counters := PackedStringArray()
	for key in ["live_edges", "edge_slots", "nodes", "lanes"]:
		counters.append("%s=%s" % [key, str(state.get(key, ""))])
	var graded := _graded_lines(terrain)
	return {
		"request": _sha(var_to_bytes(request)),
		"request_points": request.size(),
		"graph": _sha("\n".join(edges).to_utf8_buffer()),
		"state": ",".join(counters),
		"mesh": _sha(var_to_bytes(mesh)),
		"mesh_without_signs": _sha(var_to_bytes(without_signs)),
		"curve_signs": curve_signs,
		"graded": _sha("\n".join(graded).to_utf8_buffer()),
		"graded_tiles": graded.size(),
	}

# Per installed graded tile: its key, level, step, walls and its page copies read back (a headless
# run has no pages to read: the rest only).
static func _graded_lines(terrain: Node) -> PackedStringArray:
	var lines := PackedStringArray()
	var tiles: Variant = terrain.get("graded_tiles") if is_instance_valid(terrain) else null
	if tiles == null:
		return lines
	var pages := DisplayServer.get_name() != "headless"
	var keys: Array = tiles.installed.keys()
	keys.sort()
	for tile: Vector2i in keys:
		var entry: Dictionary = tiles.installed[tile]
		var copies := PackedStringArray()
		for page_id: int in entry["copies"] if pages else {}:
			var page: Dictionary = tiles.page_store.by_id[page_id]
			var copy: Vector3i = entry["copies"][page_id]
			var side := int(tiles.page_store.levels[int(page["code"])]["side"])
			var texels: PackedByteArray = tiles.page_store.layer_data(page, true, copy.x).duplicate()
			# The header's first channel is the copy's first atlas layer: a place, not a product.
			texels.encode_float(side * side * 16, 0.0)
			var rows := PackedByteArray()
			for index in copy.z:
				rows.append_array(tiles.page_store.layer_data(page, false, copy.y + index))
			copies.append("%s/%s" % [_sha(texels), _sha(rows)])
		copies.sort()
		lines.append("%d,%d,%d,%d,%s,%s" % [tile.x, tile.y, int(entry["level"]), int(entry["step"]),
			_sha((entry["walls"] as PackedFloat32Array).to_byte_array()), "+".join(copies)])
	return lines

static func _sha(bytes: PackedByteArray) -> String:
	var context := HashingContext.new()
	context.start(HashingContext.HASH_SHA256)
	context.update(bytes)
	return context.finish().hex_encode()
