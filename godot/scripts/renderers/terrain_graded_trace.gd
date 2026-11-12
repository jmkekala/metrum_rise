# SPDX-License-Identifier: GPL-2.0-only

## Record and replay of the graded tile renderer (`RENDER-32-I`, docs/terrain.md "Record and
## replay"). Recording writes, in the order they happen, every input the renderer reads (its
## entry calls with their arguments, the resident patch dictionary's mutations as terrain.gd makes
## them, Rust's replies, the install budget's answers) as `<` lines and every output it makes (its
## Rust calls, page creations and moves, slot uploads, cell buffer publications, draw states, wall faces) as
## `>` lines. Replaying reads a recorded trace back: each input returns the recorded value and each
## output must equal the recorded line, so a port of the renderer's state to Rust (G2a-c) is
## proven by reproducing every output bit for bit (godot/tests/graded_replay.gd). The tile bakes'
## bytes are recorded by size only (replay uploads zeros of that size): their content is Rust's
## bake, which the port does not touch. Off (no trace object) unless METRUM_GRADED_TRACE names the
## file to record.
extends RefCounted

var replaying := false
# The first mismatch (replay) or write error (record), with the line number; empty while none.
var failure := ""
var outputs := 0
var inputs := 0
# A replay's stand-in objects: their instance id -> the recorded object's.
var recorded_ids := {}
var _file: FileAccess = null
var _lines := PackedStringArray()
var _at := 0

## A recorder writing to `path` (null if it cannot be opened).
static func record(path: String) -> RefCounted:
	var trace = load("res://scripts/renderers/terrain_graded_trace.gd").new()
	trace._file = FileAccess.open(path, FileAccess.WRITE)
	return trace if trace._file != null else null

## A replayer reading the trace at `path` (null if it cannot be read).
static func replay(path: String) -> RefCounted:
	var text := FileAccess.get_file_as_string(path)
	if text.is_empty():
		return null
	var trace = load("res://scripts/renderers/terrain_graded_trace.gd").new()
	trace.replaying = true
	# Notes are for Rust's replays of a ported part (its own operations), not the renderer's.
	for line in text.split("\n", false):
		if not line.begins_with("= "):
			trace._lines.append(line)
	return trace

func close() -> void:
	if _file != null:
		_file.close()
		_file = null

## Whether a replay has lines left.
func has_next() -> bool:
	return _at < _lines.size() and failure.is_empty()

## The next line of a replay without taking it.
func peek() -> String:
	return _lines[_at] if _at < _lines.size() else ""

## Takes the next line of a replay.
func take() -> String:
	_at += 1
	return _lines[_at - 1] if _at <= _lines.size() else ""

func _fail(message: String) -> void:
	if failure.is_empty():
		failure = "line %d: %s" % [_at, message]

## A note (recording only): one of the renderer's own operations that a ported part replays in
## Rust (the page allocator's calls), `= ` lines a replay of the renderer skips.
func note(text: String) -> void:
	if not replaying:
		_file.store_line("= " + text)

## An output: written, or checked against the next recorded line.
func out(text: String) -> void:
	outputs += 1
	if not replaying:
		_file.store_line("> " + text)
		return
	if not failure.is_empty():
		return
	var recorded := take()
	if recorded != "> " + text:
		_fail("output differs\n  recorded: %s\n  replayed: > %s" % [recorded, text])

## An input under `tag`: `value` written and returned, or the recorded value returned.
func value(tag: String, recorded_value: String) -> String:
	inputs += 1
	if not replaying:
		_file.store_line("< %s %s" % [tag, recorded_value])
		return recorded_value
	var line := take()
	var head := "< %s " % tag
	if not line.begins_with(head):
		_fail("expected input %s, found: %s" % [tag, line])
		return recorded_value
	return line.substr(head.length())

## An entry call or a resident patch mutation (inputs the replay itself performs): written, or in
## replay taken and checked (the replay peeked it to make the call).
func entry(text: String) -> void:
	inputs += 1
	if not replaying:
		_file.store_line("< " + text)
		# A killed run keeps what it recorded up to its last frames.
		if text.begins_with("process") and inputs % 64 == 0:
			_file.flush()
		return
	var line := take()
	if line != "< " + text:
		_fail("entry differs\n  recorded: %s\n  replayed: < %s" % [line, text])

## An object's recorded instance id (a replay's stand-in maps back to the object it replaces).
func object_id(object: Object) -> int:
	if object == null:
		return 0
	return int(recorded_ids.get(object.get_instance_id(), object.get_instance_id()))

# --- Encodings: exact, so a replayed value equals the recorded one. ---

## Floats as their 32-bit bytes in hex (Godot's real_t), exact.
static func floats(values: PackedFloat32Array) -> String:
	return values.to_byte_array().hex_encode()

static func floats_of(text: String) -> PackedFloat32Array:
	return text.hex_decode().to_float32_array()

static func ints(values: Variant) -> String:
	var parts := PackedStringArray()
	for value in values:
		parts.append(str(int(value)))
	return ",".join(parts) if not parts.is_empty() else "-"

static func ints_of(text: String) -> PackedInt64Array:
	var out := PackedInt64Array()
	if text == "-":
		return out
	for part in text.split(","):
		out.append(int(part))
	return out

static func keys(dict: Dictionary) -> String:
	var flat := []
	for key: Vector2i in dict.keys():
		flat.append(key.x)
		flat.append(key.y)
	return ints(flat)

static func keys_of(text: String) -> Dictionary:
	var out := {}
	var flat := ints_of(text)
	for index in range(0, flat.size(), 2):
		out[Vector2i(flat[index], flat[index + 1])] = true
	return out

## A baked tile (poll, edit, preview): its keys, flags and wall bytes exactly, its texel and row
## bytes by size.
static func tile(tile: Dictionary) -> String:
	var walls: PackedByteArray = tile.get("walls", PackedByteArray())
	return "%d %d %d %d %d %d %d %d %d %d %d %s" % [
		int(tile["tile_x"]), int(tile["tile_z"]), int(tile["level"]), int(tile["step"]), int(tile["token"]),
		int(bool(tile.get("graded", true))), int(bool(tile["behind"])), int(bool(tile.get("adopted", false))),
		int(tile.has("adopted")), (tile["texels"] as PackedByteArray).size(), (tile["rows"] as PackedByteArray).size(),
		walls.hex_encode() if not walls.is_empty() else "-"]

static func tile_of(text: String) -> Dictionary:
	var parts := text.split(" ")
	var texels := PackedByteArray()
	texels.resize(int(parts[9]))
	var rows := PackedByteArray()
	rows.resize(int(parts[10]))
	var out := {
		"tile_x": int(parts[0]), "tile_z": int(parts[1]), "level": int(parts[2]), "step": int(parts[3]),
		"token": int(parts[4]), "graded": parts[5] == "1", "behind": parts[6] == "1",
		"texels": texels, "rows": rows,
		"walls": parts[11].hex_decode() if parts[11] != "-" else PackedByteArray(),
	}
	if parts[8] == "1":
		out["adopted"] = parts[7] == "1"
	return out

## A poll's result, as lines after its own `poll` input line.
func poll(result: Dictionary) -> Dictionary:
	if replaying:
		return _read_poll()
	var lines := PackedStringArray()
	for edit: Dictionary in result.get("edits", []):
		lines.append("edit %d %d %d %d %d" % [int(edit["serial"]), int(bool(edit["all"])), int(edit.get("preview_epoch", -1)),
			(edit["levels"] as Array).size(), (edit.get("staged", []) as Array).size()])
		for level: Dictionary in edit["levels"]:
			lines.append("level %d %d %s" % [int(level["level"]), int(level["step"]), ints(level["tiles"])])
		for staged: Dictionary in edit.get("staged", []):
			lines.append("tile " + tile(staged))
	for list: Dictionary in result.get("patches", []):
		lines.append("list %d %d %d %s" % [int(list["patch_x"]), int(list["patch_z"]), int(list["token"]), ints(list["tiles"])])
	for failed: Dictionary in result.get("failed", []):
		lines.append("failed %s %s %d %d %s" % [failed["kind"], ints(failed["keys"]), int(failed["token"]), int(bool(failed["retry"])), failed["reason"]])
	for baked: Dictionary in result.get("tiles", []):
		lines.append("tile " + tile(baked))
	value("poll", "%d %d" % [int(result.get("tiles_pending", 0)), lines.size()])
	for line in lines:
		_file.store_line("< " + line)
	return result

func _read_poll() -> Dictionary:
	var head := value("poll", "").split(" ")
	var result := {"edits": [], "patches": [], "failed": [], "tiles": [], "tiles_pending": int(head[0])}
	var count := int(head[1])
	var taken := 0
	while taken < count:
		var line := take().substr(2)
		taken += 1
		var kind := line.get_slice(" ", 0)
		var rest := line.substr(kind.length() + 1)
		match kind:
			"edit":
				var parts := rest.split(" ")
				var edit := {"serial": int(parts[0]), "all": parts[1] == "1", "preview_epoch": int(parts[2]), "levels": [], "staged": []}
				for index in int(parts[3]):
					var level := take().substr(2).split(" ")
					taken += 1
					edit["levels"].append({"level": int(level[1]), "step": int(level[2]), "tiles": PackedInt32Array(Array(ints_of(level[3])))})
				for index in int(parts[4]):
					edit["staged"].append(tile_of(take().substr(7)))
					taken += 1
				result["edits"].append(edit)
			"list":
				var parts := rest.split(" ")
				result["patches"].append({"patch_x": int(parts[0]), "patch_z": int(parts[1]), "token": int(parts[2]), "tiles": PackedInt32Array(Array(ints_of(parts[3])))})
			"failed":
				var parts := rest.split(" ")
				result["failed"].append({"kind": parts[0], "keys": ints_of(parts[1]), "token": int(parts[2]), "retry": parts[3] == "1", "reason": parts[4]})
			"tile":
				result["tiles"].append(tile_of(rest))
			_:
				_fail("unknown poll line: " + line)
	return result

## A resident patch entry, as terrain.gd sets it: its node and material by instance id.
static func patch(key: Vector2i, entry: Dictionary) -> String:
	var rect: Rect2 = entry["rect"]
	var aabb: AABB = entry["aabb"]
	return "patch %d %d %d %d %s %d %d %s" % [key.x, key.y, (entry["node"] as Object).get_instance_id(),
		(entry["material"] as Object).get_instance_id(),
		floats(PackedFloat32Array([rect.position.x, rect.position.y, rect.size.x, rect.size.y])),
		int(entry["step"]), int(entry["generation"]),
		floats(PackedFloat32Array([aabb.position.x, aabb.position.y, aabb.position.z, aabb.size.x, aabb.size.y, aabb.size.z]))]

## A preview frame's changes (show_preview's argument).
static func preview(changes: Dictionary) -> String:
	var tiles := PackedStringArray()
	for shown: Dictionary in changes.get("graded_tiles", []):
		tiles.append(tile(shown))
	return "preview %s %s" % [ints(changes.get("graded_removed", PackedInt32Array())), "|".join(tiles) if not tiles.is_empty() else "-"]

static func preview_of(text: String) -> Dictionary:
	var parts := text.split(" ", true, 2)
	var tiles := []
	if parts[2] != "-":
		for shown in parts[2].split("|"):
			tiles.append(tile_of(shown))
	return {"graded_removed": PackedInt32Array(Array(ints_of(parts[1]))), "graded_tiles": tiles}
