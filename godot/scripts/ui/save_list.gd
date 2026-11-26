# SPDX-License-Identifier: GPL-3.0-or-later

## The files a save browser lists (`UI-16`): the `.sqlite` files of one folder with their time
## saved and size and, for Load and New Game, whether the game loads each one. Rust decides that
## from the file's header with the loaders' own checks (`SaveFiles.verdict`, a few small reads per
## file); the browser reads the folder once per open, never per frame.
extends RefCounted

## The only file type listed.
const EXTENSION := "sqlite"

## One dictionary per file in the native folder `dir`: `file`, `time` (modified, Unix seconds),
## `size` (bytes) and `refusal`: empty when the game loads the file or `check` is off, else
## `reason` (the row's short text) and `message` (the load's own words). `world` checks world
## definitions (New Game) instead of saves (Load).
static func read(dir: String, check: bool, world: bool) -> Array[Dictionary]:
	var entries: Array[Dictionary] = []
	for file in DirAccess.get_files_at(dir):
		if file.get_extension().to_lower() != EXTENSION or file.begins_with("."):
			continue
		var path := dir.path_join(file)
		entries.append({
			"file": file,
			"time": FileAccess.get_modified_time(path),
			"size": _file_size(path),
			"refusal": SaveFiles.verdict(path, world) if check else {},
		})
	return entries

static func _file_size(path: String) -> int:
	var file := FileAccess.open(path, FileAccess.READ)
	return file.get_length() if file else 0
