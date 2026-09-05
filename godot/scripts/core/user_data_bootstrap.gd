# SPDX-License-Identifier: GPL-2.0-only

## First-run user data bootstrap for release builds.
## Creates the canonical writable player-data folders and seeds bundled starter
## content from res://bootstrap/ without overwriting user-owned files.
extends RefCounted

const GameSettings = preload("res://scripts/core/game_settings.gd")
const ModPackConfig = preload("res://scripts/core/mod_pack_config.gd")

const USER_WORLDS_DIR := "user://worlds"
const USER_MODS_DIR := "user://mods"
const USER_SAVES_DIR := "user://saves"
const BOOTSTRAP_WORLDS_DIR := "res://bootstrap/worlds"
const BOOTSTRAP_MODS_DIR := "res://bootstrap/mods"
const COPY_CHUNK_BYTES := 1024 * 1024

static func run() -> void:
	_ensure_user_dir(USER_WORLDS_DIR)
	_ensure_user_dir(USER_MODS_DIR)
	_ensure_user_dir(USER_SAVES_DIR)
	_seed_top_level_entries(BOOTSTRAP_WORLDS_DIR, USER_WORLDS_DIR)
	_seed_top_level_entries(BOOTSTRAP_MODS_DIR, USER_MODS_DIR)
	GameSettings.seed_default_config_if_missing()
	GameSettings.apply_display_settings()
	ModPackConfig.seed_default_config_if_missing()

static func _ensure_user_dir(path: String) -> bool:
	var err := DirAccess.make_dir_recursive_absolute(ProjectSettings.globalize_path(path))
	if err != OK:
		push_warning("Could not create user data directory '%s' (error %d)." % [path, err])
		return false
	return true

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

static func _should_seed_entry(entry: String) -> bool:
	return entry != "." and entry != ".." and not entry.begins_with(".")
