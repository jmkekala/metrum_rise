# SPDX-License-Identifier: GPL-2.0-only

## Replays the refused road strokes recorded in a debug dump against a loaded save.
##
## Every refusal logs a `road_attempt {json}` line on `events` with the exact stroke. A replay
## sends each stroke along the player's path: the road tool's click check, then, if that passes,
## the simulation commit. The attempt reproduces when it is refused for the same reason. The
## dump's `dump save=<path> road_generation=<n>` line says which attempts were made on the saved
## road network; for older attempts the network has changed since, so their result only hints.
extends RefCounted

# Frames to wait for a busy validation or a queued commit before giving up on one attempt.
const POLL_FRAMES := 1800

## Reads a dump or a captured stdout log: `attempts` (one dictionary per `road_attempt` line, in
## order), `save_path` and `save_generation` from the last `dump save=` line ("" and -1 if none).
static func parse_dump(text: String) -> Dictionary:
	var attempts: Array[Dictionary] = []
	var save_path := ""
	var save_generation := -1
	for line in text.split("\n"):
		var at := line.find("road_attempt {")
		if at >= 0:
			var record: Variant = JSON.parse_string(line.substr(at + "road_attempt ".length()))
			if record is Dictionary:
				attempts.append(record)
			continue
		at = line.find("dump save=")
		if at >= 0:
			# The path may contain spaces; the generation is the last field.
			var tail := line.substr(at + "dump save=".length()).strip_edges()
			var marker := tail.rfind(" road_generation=")
			if marker >= 0:
				save_path = tail.substr(0, marker)
				save_generation = int(tail.substr(marker + " road_generation=".length()))
	return {"attempts": attempts, "save_path": save_path, "save_generation": save_generation}

## Replays one attempt. Returns `by` (`tool` or `commit`) and `reason`, empty when the road was
## built. A built road stays in the loaded world, so later attempts see it.
static func replay_attempt(simulation: Node, tree: SceneTree, attempt: Dictionary) -> Dictionary:
	var points := PackedVector3Array()
	for point in attempt.get("points", []):
		points.append(Vector3(point[0], point[1], point[2]))
	var fwd := int(attempt.get("fwd_lanes", 1))
	var bkw := int(attempt.get("bkw_lanes", 1))
	var snap := bool(attempt.get("snap", true))
	if points.size() < 2:
		return {"by": "tool", "reason": "too_short"}
	var validation: Dictionary = {}
	for frame in POLL_FRAMES:
		var result: Variant = simulation.validate_road_candidate_for_commit_with_snap(points, fwd, bkw, snap)
		validation = result if result is Dictionary else {}
		# Pending means the core was busy for the parcel check; ask again next frame.
		if not validation.get("is_pending", false):
			break
		await tree.process_frame
	if not validation.get("is_valid", false):
		return {"by": "tool", "reason": String(validation.get("invalid_reason", "validation_unavailable"))}
	var request: int = simulation.add_road_with_snap(points, fwd, bkw, snap)
	for frame in POLL_FRAMES:
		var result: Variant = simulation.get_road_commit_result(request)
		if result is Dictionary:
			return {"by": "commit", "reason": "" if result.get("committed", false) else String(result.get("reason", ""))}
		await tree.process_frame
	return {"by": "commit", "reason": "replay_timeout"}

## Compares a replay with its record: `reproduced`, `same_network` (made on the saved road
## network) and one `line` for the report.
static func verdict(attempt: Dictionary, outcome: Dictionary, save_generation: int) -> Dictionary:
	var expected := String(attempt.get("reason", ""))
	var reason := String(outcome.get("reason", ""))
	var generation := int(attempt.get("generation", -1))
	var same_network := save_generation >= 0 and generation == save_generation
	var reproduced := not reason.is_empty() and reason == expected
	var line := "attempt %d (%s, %s): " % [int(attempt.get("attempt", -1)), attempt.get("by", "?"), expected]
	if reproduced:
		line += "reproduced, refused by %s" % outcome.get("by", "?")
	elif reason.is_empty():
		line += "NOT reproduced: the road was built"
	else:
		line += "NOT reproduced: refused by %s for %s" % [outcome.get("by", "?"), reason]
	if not same_network:
		line += " [made on road generation %d, save holds %d: the network changed since]" % [generation, save_generation]
	return {"reproduced": reproduced, "same_network": same_network, "line": line}
