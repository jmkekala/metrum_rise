# SPDX-License-Identifier: GPL-3.0-or-later

## First-run user data bootstrap for release builds.
## Creates the canonical writable player-data folders and seeds bundled starter
## content from res://bootstrap/ without overwriting user-owned files. Untouched copies of bundled
## worlds a release replaced are removed first (`SUPERSEDED_WORLDS`).
extends RefCounted

const GameSettings = preload("res://scripts/core/game_settings.gd")
const ModPackConfig = preload("res://scripts/core/mod_pack_config.gd")

const USER_WORLDS_DIR := "user://worlds"
const USER_MODS_DIR := "user://mods"
const USER_SAVES_DIR := "user://saves"
const BOOTSTRAP_WORLDS_DIR := "res://bootstrap/worlds"
const BOOTSTRAP_MODS_DIR := "res://bootstrap/mods"
# The repository's test building pack, for development runs. Exports exclude res://tests.
const DEV_MODS_DIR := "res://tests/mods"
const COPY_CHUNK_BYTES := 1024 * 1024
# Bundled worlds a release replaced, by file name, with the SHA-256 of every copy a release seeded
# under that name. A profile copy with one of these hashes is the player's untouched copy of a
# stale world and is removed before seeding, so the current world takes its place; a copy the
# player changed has another hash and stays theirs.
const SUPERSEDED_WORLDS := {
	# The 18,000 m, 10 m Kuopio with hand-made lakes, replaced by kuopio_18432m_8m.sqlite.
	"kuopio_324km2_10m.sqlite": ["07ae5ff39f9c905eccfec41b15ac4aa69de70fba50e96b34a19aeee991cabdbd"],
	# The 500 m, 10 m blank world, off the aligned grid's rules; the same name now ships 512 m, 8 m.
	"test.sqlite": ["feb256447173c9aab87b6295c10492f92c008b4ec4d3c250b3fe3f09af070e81"],
}

static func run() -> void:
	_ensure_user_dir(USER_WORLDS_DIR)
	_ensure_user_dir(USER_MODS_DIR)
	_ensure_user_dir(USER_SAVES_DIR)
	_remove_superseded_copies(USER_WORLDS_DIR, SUPERSEDED_WORLDS)
	_seed_top_level_entries(BOOTSTRAP_WORLDS_DIR, USER_WORLDS_DIR)
	_seed_top_level_entries(BOOTSTRAP_MODS_DIR, USER_MODS_DIR)
	if not OS.has_feature("template"):
		_seed_top_level_entries(DEV_MODS_DIR, USER_MODS_DIR)
	# Pack imports that never finished (a crash mid-import) leave hidden staging folders.
	AssetAuthoringFiles.sweep_imports(USER_MODS_DIR)
	GameSettings.seed_default_config_if_missing()
	GameSettings.apply_display_settings()
	GameSettings.fit_launch_window()
	ModPackConfig.seed_default_config_if_missing()

static func _ensure_user_dir(path: String) -> bool:
	var err := DirAccess.make_dir_recursive_absolute(ProjectSettings.globalize_path(path))
	if err != OK:
		push_warning("Could not create user data directory '%s' (error %d)." % [path, err])
		return false
	return true

# Removes each file of `superseded` (name -> shipped hashes) from `target_dir` whose content is one
# of its shipped copies. Hashes a file only while it exists: once per stale copy, or at each start
# for a copy the player changed.
static func _remove_superseded_copies(target_dir: String, superseded: Dictionary) -> void:
	for entry: String in superseded:
		var path := target_dir.path_join(entry)
		if not FileAccess.file_exists(path) or not superseded[entry].has(FileAccess.get_sha256(path)):
			continue
		var err := DirAccess.remove_absolute(ProjectSettings.globalize_path(path))
		if err != OK:
			push_warning("Could not remove superseded world '%s' (error %d)." % [path, err])

static func _seed_top_level_entries(source_dir: String, target_dir: String) -> void:
	var dir := DirAccess.open(source_dir)
	if not dir:
		return
	if not _ensure_user_dir(target_dir):
		return

	dir.list_dir_begin()
	var entry := dir.get_next()
	while entry != "":
		if _should_seed_entry(entry):
			var source_path := source_dir.path_join(entry)
			var target_path := target_dir.path_join(entry)
			if not _target_exists(target_path):
				if dir.current_is_dir():
					_copy_new_dir_recursive(source_path, target_path)
				else:
					_copy_file_if_missing(source_path, target_path)
		entry = dir.get_next()
	dir.list_dir_end()

static func _copy_new_dir_recursive(source_dir: String, target_dir: String) -> void:
	if _target_exists(target_dir):
		return
	if not _ensure_user_dir(target_dir):
		return

	var dir := DirAccess.open(source_dir)
	if not dir:
		push_warning("Could not read bootstrap directory '%s'." % source_dir)
		return

	dir.list_dir_begin()
	var entry := dir.get_next()
	while entry != "":
		if _should_seed_entry(entry):
			var source_path := source_dir.path_join(entry)
			var target_path := target_dir.path_join(entry)
			if dir.current_is_dir():
				_copy_new_dir_recursive(source_path, target_path)
			else:
				_copy_file_if_missing(source_path, target_path)
		entry = dir.get_next()
	dir.list_dir_end()

static func _copy_file_if_missing(source_path: String, target_path: String) -> void:
	if _target_exists(target_path):
		return

	var source := FileAccess.open(source_path, FileAccess.READ)
	if not source:
		push_warning("Could not read bootstrap file '%s' (error %d)." % [
			source_path,
			FileAccess.get_open_error()
		])
		return

	var target_dir := target_path.get_base_dir()
	if not _ensure_user_dir(target_dir):
		source.close()
		return

	var target := FileAccess.open(target_path, FileAccess.WRITE)
	if not target:
		push_warning("Could not write user data file '%s' (error %d)." % [
			target_path,
			FileAccess.get_open_error()
		])
		source.close()
		return

	var remaining := source.get_length()
	while remaining > 0:
		var byte_count := mini(COPY_CHUNK_BYTES, remaining)
		target.store_buffer(source.get_buffer(byte_count))
		remaining -= byte_count

	target.close()
	source.close()

static func _target_exists(path: String) -> bool:
	return FileAccess.file_exists(path) or DirAccess.dir_exists_absolute(ProjectSettings.globalize_path(path))

# `.import` sidecars are project import state, not pack content: exports never ship them, and a
# copied sidecar makes glTF loading resolve the pack's textures through the import system
# instead of reading the files (TOOLS-15).
static func _should_seed_entry(entry: String) -> bool:
	return entry != "." and entry != ".." and not entry.begins_with(".") and not entry.ends_with(".import")
