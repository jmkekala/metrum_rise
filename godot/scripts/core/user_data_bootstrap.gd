# SPDX-License-Identifier: GPL-3.0-or-later

## Start-up user data bootstrap.
## Creates the canonical writable player-data folders and brings the game's bundled packs and
## worlds from res://bootstrap/ into them (`TOOLS-17`): Rust plans every step from the bundle
## manifest, the profile's receipt `user://bundled.toml` and the installed copies; this script
## only copies out of the bundle, moves a stale pre-receipt pack to the system Trash, hands each
## step back to Rust and logs. An installed copy the player changed stays theirs until they
## restore the game's version (Options > Mods > Restore, `restore_bundled`), which takes the same
## steps.
extends RefCounted

const GameSettings = preload("res://scripts/core/game_settings.gd")
const ModPackConfig = preload("res://scripts/core/mod_pack_config.gd")

const USER_DIR := "user://"
const USER_WORLDS_DIR := "user://worlds"
const USER_MODS_DIR := "user://mods"
const USER_SAVES_DIR := "user://saves"
const BOOTSTRAP_DIR := "res://bootstrap"
const BOOTSTRAP_MODS_DIR := "res://bootstrap/mods"
# The repository's test building pack, for development runs. Exports exclude res://tests. Seeded
# only when missing and never updated (decision D7 of `TOOLS-17`).
const DEV_MODS_DIR := "res://tests/mods"
const COPY_CHUNK_BYTES := 1024 * 1024

static func run() -> void:
	_ensure_user_dir(USER_WORLDS_DIR)
	_ensure_user_dir(USER_MODS_DIR)
	_ensure_user_dir(USER_SAVES_DIR)
	# A normal start reads the receipt and stats three paths per bundled entry: no hashing, no
	# copy, no write. A start after an update hashes and copies only the changed entries.
	reconcile_bundled(BOOTSTRAP_DIR, USER_DIR, AssetAuthoringFiles.bundled_plan(USER_DIR), OS.move_to_trash)
	if not OS.has_feature("template"):
		_seed_top_level_entries(DEV_MODS_DIR, USER_MODS_DIR)
	# Pack imports that never finished (a crash mid-import) leave hidden staging folders.
	AssetAuthoringFiles.sweep_imports(USER_MODS_DIR)
	GameSettings.seed_default_config_if_missing()
	GameSettings.apply_display_settings()
	GameSettings.fit_launch_window()
	ModPackConfig.seed_default_config_if_missing()

## Carries out `plan` (from `AssetAuthoringFiles.bundled_plan` for `profile`): copies each
## seeded or replacing entry from `bundle` into its staging sibling, moves a stale pre-receipt
## pack away with `trash` (`OS.move_to_trash` in the game, a scratch move in the test; it gets
## the native path and returns an `Error`), lets Rust commit the step, then records every step
## that succeeded in one write. A failed step is skipped and planned again at the next start.
static func reconcile_bundled(bundle: String, profile: String, plan: Dictionary, trash: Callable) -> void:
	for problem: String in plan.get("problems", PackedStringArray()):
		_log("could not check: " + problem)
	if plan.has("error"):
		push_warning("Bundled packs and worlds were not checked: %s" % plan.error)
		return
	var done: Array[Dictionary] = []
	for action: Dictionary in plan.actions:
		if _take_step(bundle, profile, action, trash):
			done.append(action)
	var err := AssetAuthoringFiles.bundled_record(profile, done)
	if not err.is_empty():
		push_warning("The bundled install receipt was not updated: %s" % err)

## Gives the game's version of the bundled pack `pack` back over the player's changed copy in
## `profile` (Options > Mods > Restore): Rust's restore step, carried out as a start carries out
## its own trash-and-replace (`trash` moves the player's copy away, the bundled one is staged from
## `bundle` and committed), then recorded as the game's. Returns "" on success, else what
## happened, in the player's words; when the Trash fails nothing has changed.
static func restore_bundled(bundle: String, profile: String, pack: String, trash: Callable) -> String:
	var action := AssetAuthoringFiles.bundled_restore(profile, pack)
	if action.has("error"):
		return str(action.error)
	var err := _trash_installed(profile, action, trash)
	if err != OK:
		return "Could not move your copy of %s to the Trash (%s). Nothing was changed." % [pack, error_string(err)]
	# The player's copy is in the Trash: a step failing now leaves the pack absent (or a staging
	# sibling), which the next start cleans and seeds as the game's.
	if not _stage(bundle, profile, action):
		return "Your copy of %s is in the Trash, but the game's version could not be copied. It is installed at the next start." % pack
	var error := _commit(profile, action)
	if not error.is_empty():
		return "Your copy of %s is in the Trash, but the game's version was not installed (%s). It is installed at the next start." % [pack, error]
	var done: Array[Dictionary] = [action]
	error = AssetAuthoringFiles.bundled_record(profile, done)
	if not error.is_empty():
		return "The game's version of %s is installed, but the install receipt was not updated (%s)." % [pack, error]
	return ""

# One planned step; true when it succeeded.
static func _take_step(bundle: String, profile: String, action: Dictionary, trash: Callable) -> bool:
	var path: String = action.path
	match action.operation:
		"trash_and_replace":
			# Decision D2 (a): a pack from before the receipt that differs from the bundle goes to
			# the Trash once. Without a Trash it is kept, never deleted; the step fails and the next
			# start keeps the copy as the player's.
			var err := _trash_installed(profile, action, trash)
			if err != OK:
				push_warning("Kept %s: it differs from the game's copy and could not be moved to the Trash (error %d)." % [path, err])
				return false
			if not _stage(bundle, profile, action):
				return false
		"seed", "replace":
			if not _stage(bundle, profile, action):
				return false
		"keep_players":
			# Decision D3 (a): no message on screen, one line here and the documented rule.
			_log("kept the player's copy of %s; delete it to get the game's version" % path)
	return _commit(profile, action).is_empty()

# Moves the installed entry to the Trash with `trash` (it gets the native path, returns an Error).
static func _trash_installed(profile: String, action: Dictionary, trash: Callable) -> Error:
	return trash.call(ProjectSettings.globalize_path(profile.path_join(action.path)))

# Rust's profile side of a step, logged; returns its error, empty on success.
static func _commit(profile: String, action: Dictionary) -> String:
	var path: String = action.path
	var error := AssetAuthoringFiles.bundled_commit(profile, action)
	if not error.is_empty():
		push_warning("Bundled %s step for %s failed: %s" % [action.operation, path, error])
		return error
	if action.operation in ["seed", "replace", "trash_and_replace", "remove"]:
		_log("%s %s" % [action.operation, path])
	return ""

# Copies the bundled entry into its staging sibling; a staging entry already there (a cleanup that
# failed) is never merged into.
static func _stage(bundle: String, profile: String, action: Dictionary) -> bool:
	var source := bundle.path_join(action.path)
	var target := profile.path_join(action.staging)
	if _target_exists(target):
		push_warning("Could not stage %s: '%s' is in the way." % [action.path, target])
		return false
	if action.pack:
		return _copy_dir(source, target)
	return _copy_file(source, target)

static func _log(message: String) -> void:
	MetrumDebug.log("events", "bundled: " + message)

static func _ensure_user_dir(path: String) -> bool:
	var err := DirAccess.make_dir_recursive_absolute(ProjectSettings.globalize_path(path))
	if err != OK:
		push_warning("Could not create user data directory '%s' (error %d)." % [path, err])
		return false
	return true

# Copies each top-level entry of `source_dir` that `target_dir` lacks (the development packs).
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
					_copy_dir(source_path, target_path)
				else:
					_copy_file(source_path, target_path)
		entry = dir.get_next()
	dir.list_dir_end()

# Copies every seeded entry of `source_dir` into `target_dir`; false on the first failure.
static func _copy_dir(source_dir: String, target_dir: String) -> bool:
	if not _ensure_user_dir(target_dir):
		return false
	var dir := DirAccess.open(source_dir)
	if not dir:
		push_warning("Could not read bootstrap directory '%s'." % source_dir)
		return false
	dir.list_dir_begin()
	var entry := dir.get_next()
	var ok := true
	while ok and entry != "":
		if _should_seed_entry(entry):
			var source_path := source_dir.path_join(entry)
			var target_path := target_dir.path_join(entry)
			ok = _copy_dir(source_path, target_path) if dir.current_is_dir() else _copy_file(source_path, target_path)
		entry = dir.get_next()
	dir.list_dir_end()
	return ok

static func _copy_file(source_path: String, target_path: String) -> bool:
	var source := FileAccess.open(source_path, FileAccess.READ)
	if not source:
		push_warning("Could not read bootstrap file '%s' (error %d)." % [
			source_path,
			FileAccess.get_open_error()
		])
		return false

	var target_dir := target_path.get_base_dir()
	if not _ensure_user_dir(target_dir):
		source.close()
		return false

	var target := FileAccess.open(target_path, FileAccess.WRITE)
	if not target:
		push_warning("Could not write user data file '%s' (error %d)." % [
			target_path,
			FileAccess.get_open_error()
		])
		source.close()
		return false

	var remaining := source.get_length()
	var ok := true
	while ok and remaining > 0:
		var byte_count := mini(COPY_CHUNK_BYTES, remaining)
		ok = target.store_buffer(source.get_buffer(byte_count))
		remaining -= byte_count
	target.close()
	source.close()
	if not ok:
		push_warning("Could not write user data file '%s'." % target_path)
	return ok

static func _target_exists(path: String) -> bool:
	return FileAccess.file_exists(path) or DirAccess.dir_exists_absolute(ProjectSettings.globalize_path(path))

# `.import` sidecars are project import state, not pack content: exports never ship them, and a
# copied sidecar makes glTF loading resolve the pack's textures through the import system
# instead of reading the files (TOOLS-15).
static func _should_seed_entry(entry: String) -> bool:
	return entry != "." and entry != ".." and not entry.begins_with(".") and not entry.ends_with(".import")
