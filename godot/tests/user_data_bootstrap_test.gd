# SPDX-License-Identifier: GPL-3.0-or-later

## First-run bootstrap of bundled worlds a release replaced (`SUPERSEDED_WORLDS`): in a scratch
## folder of the test profile, an untouched stale copy is removed and the current world seeded in
## its place, a copy the player changed stays, and no bundled world is itself listed as stale
## (it would be removed and copied again at every start).
extends SceneTree

const UserDataBootstrap = preload("res://scripts/core/user_data_bootstrap.gd")
const ROOT := "user://bootstrap-test"

var _failures := 0

func _initialize() -> void:
	_run()

func _expect(condition: bool, message: String) -> void:
	if not condition:
		_failures += 1
		push_error(message)

func _write(path: String, text: String) -> void:
	DirAccess.make_dir_recursive_absolute(ProjectSettings.globalize_path(path.get_base_dir()))
	var file := FileAccess.open(path, FileAccess.WRITE)
	file.store_string(text)
	file.close()

# Empties the scratch folder (two levels: ROOT/<dir>/<file>).
func _clear() -> void:
	for dir_name in DirAccess.get_directories_at(ROOT):
		var dir := ROOT.path_join(dir_name)
		for file_name in DirAccess.get_files_at(dir):
			DirAccess.remove_absolute(ProjectSettings.globalize_path(dir.path_join(file_name)))
		DirAccess.remove_absolute(ProjectSettings.globalize_path(dir))

func _run() -> void:
	_clear()
	var bundle := ROOT.path_join("bundle")
	var profile := ROOT.path_join("worlds")
	_write(bundle.path_join("current.sqlite"), "current world")
	_write(profile.path_join("stale.sqlite"), "stale world as shipped")
	_write(profile.path_join("edited.sqlite"), "stale world the player edited")
	var shipped := FileAccess.get_sha256(profile.path_join("stale.sqlite"))
	var superseded := {"stale.sqlite": [shipped], "edited.sqlite": [shipped], "absent.sqlite": [shipped]}

	UserDataBootstrap._remove_superseded_copies(profile, superseded)
	UserDataBootstrap._seed_top_level_entries(bundle, profile)

	_expect(not FileAccess.file_exists(profile.path_join("stale.sqlite")),
		"the untouched stale copy must be removed")
	_expect(FileAccess.get_file_as_string(profile.path_join("edited.sqlite")) == "stale world the player edited",
		"a copy the player changed must stay as it is")
	_expect(FileAccess.get_file_as_string(profile.path_join("current.sqlite")) == "current world",
		"the current world must be seeded")
	_expect(not FileAccess.file_exists(profile.path_join("absent.sqlite")),
		"a stale name the profile never had must not appear")

	_expect(not UserDataBootstrap.SUPERSEDED_WORLDS.is_empty(), "the replaced Kuopio must be listed")
	for entry: String in UserDataBootstrap.SUPERSEDED_WORLDS:
		var bundled := UserDataBootstrap.BOOTSTRAP_WORLDS_DIR.path_join(entry)
		_expect(not FileAccess.file_exists(bundled)
			or not UserDataBootstrap.SUPERSEDED_WORLDS[entry].has(FileAccess.get_sha256(bundled)),
			"the bundled %s is listed as stale" % entry)

	_clear()
	if _failures == 0:
		print("user_data_bootstrap_test: passed")
	quit(1 if _failures > 0 else 0)
