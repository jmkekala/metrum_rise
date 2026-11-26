# SPDX-License-Identifier: GPL-3.0-or-later

## Start-up reconciliation of bundled packs and worlds (`TOOLS-17`) end to end through the bridge,
## in a scratch folder of the test profile with a fake bundle and its manifest: a fresh install,
## an untouched pack updated with files added, changed and removed, an edited pack kept as the
## player's (logged), a world no longer bundled removed, a deleted entry seeded again, an
## interrupted copy cleaned and redone, a profile from before the receipt whose stale pack goes to
## the Trash (a scratch folder here, never the system Trash) or, without a Trash, stays, and a
## start with nothing to do planning nothing. The legacy world table lives in Rust and is tested
## there (`assets::bundled::tests`).
extends SceneTree

const UserDataBootstrap = preload("res://scripts/core/user_data_bootstrap.gd")
const ROOT := "user://bootstrap-test"
const BUNDLE := ROOT + "/bundle"
const PROFILE := ROOT + "/profile"
const TRASH := ROOT + "/trash"

var _failures := 0

func _initialize() -> void:
	_run()

func _expect(condition: bool, message: String) -> void:
	if not condition:
		_failures += 1
		push_error(message)

func _native(path: String) -> String:
	return ProjectSettings.globalize_path(path)

func _write(path: String, text: String) -> void:
	DirAccess.make_dir_recursive_absolute(_native(path.get_base_dir()))
	var file := FileAccess.open(path, FileAccess.WRITE)
	file.store_string(text)
	file.close()

func _read(path: String) -> String:
	return FileAccess.get_file_as_string(path)

func _exists(path: String) -> bool:
	return FileAccess.file_exists(path) or DirAccess.dir_exists_absolute(_native(path))

# Deletes a scratch file or folder tree (no links are made here).
func _remove(path: String) -> void:
	if DirAccess.dir_exists_absolute(_native(path)):
		for name in DirAccess.get_directories_at(path):
			_remove(path.path_join(name))
		for name in DirAccess.get_files_at(path):
			DirAccess.remove_absolute(_native(path.path_join(name)))
	DirAccess.remove_absolute(_native(path))

# Every file under `folder`, relative, sorted; `seeded_only` skips what the bootstrap never copies.
func _files(folder: String, seeded_only: bool, prefix := "") -> Array:
	var found := []
	var here := folder.path_join(prefix)
	for name in DirAccess.get_directories_at(here):
		if not seeded_only or UserDataBootstrap._should_seed_entry(name):
			found.append_array(_files(folder, seeded_only, prefix.path_join(name)))
	for name in DirAccess.get_files_at(here):
		if not seeded_only or UserDataBootstrap._should_seed_entry(name):
			found.append(prefix.path_join(name))
	found.sort()
	return found

# The bundle's manifest, computed here independently of Rust: a world's SHA-256, and for a pack
# the SHA-256 of its sorted `<hex>  <relative path>` lines.
func _manifest() -> String:
	var lines := []
	for name in DirAccess.get_directories_at(BUNDLE.path_join("mods")):
		if not UserDataBootstrap._should_seed_entry(name):
			continue
		var pack := BUNDLE.path_join("mods").path_join(name)
		var listing := ""
		for relative: String in _files(pack, true):
			listing += "%s  %s\n" % [FileAccess.get_sha256(pack.path_join(relative)), relative]
		lines.append("%s  mods/%s\n" % [listing.sha256_text(), name])
	for name in DirAccess.get_files_at(BUNDLE.path_join("worlds")):
		if UserDataBootstrap._should_seed_entry(name):
			var world := BUNDLE.path_join("worlds").path_join(name)
			lines.append("%s  worlds/%s\n" % [FileAccess.get_sha256(world), name])
	lines.sort_custom(func(a: String, b: String) -> bool: return a.substr(66) < b.substr(66))
	return "".join(lines)

# Moves the entry into the scratch Trash, as `OS.move_to_trash` would into the system's.
func _scratch_trash(native: String) -> Error:
	DirAccess.make_dir_recursive_absolute(_native(TRASH))
	return DirAccess.rename_absolute(native, _native(TRASH).path_join(native.get_file()))

func _no_trash(_native_path: String) -> Error:
	return ERR_UNAVAILABLE

# One start-up against the fake bundle; returns its steps as "<operation> <path>".
func _start(trash: Callable = Callable()) -> Array:
	if not trash.is_valid():
		trash = _scratch_trash
	var plan := AssetAuthoringFiles.bundled_plan_for_test(_manifest(), PROFILE)
	_expect(not plan.has("error"), "the manifest is refused: %s" % plan.get("error", ""))
	_expect(plan.get("problems", PackedStringArray()).is_empty(), "problems: %s" % [plan.get("problems")])
	UserDataBootstrap.reconcile_bundled(BUNDLE, PROFILE, plan, trash)
	var steps := []
	for action: Dictionary in plan.get("actions", []):
		steps.append("%s %s" % [action.operation, action.path])
	return steps

func _expect_steps(steps: Array, expected: Array, case: String) -> void:
	_expect(steps == expected, "%s: steps %s, expected %s" % [case, steps, expected])

# The installed pack holds exactly the bundled pack's seeded files, with their content.
func _expect_pack_is_bundled(case: String) -> void:
	var bundled := BUNDLE.path_join("mods/pack")
	var installed := PROFILE.path_join("mods/pack")
	var files := _files(bundled, true)
	_expect(_files(installed, false) == files, "%s: installed files %s, bundled %s" % [case, _files(installed, false), files])
	for relative: String in files:
		_expect(_read(installed.path_join(relative)) == _read(bundled.path_join(relative)),
			"%s: %s differs from the bundle" % [case, relative])

func _expect_no_staging(case: String) -> void:
	for folder in ["mods", "worlds"]:
		for name in DirAccess.get_directories_at(PROFILE.path_join(folder)) + DirAccess.get_files_at(PROFILE.path_join(folder)):
			_expect(not name.begins_with("."), "%s: %s/%s left behind" % [case, folder, name])

func _log_has(after_seq: int, text: String) -> bool:
	var lines := MetrumDebug.read_since(after_seq, 4096)
	for message: String in lines.message:
		if message.contains(text):
			return true
	return false

func _bundle_v1() -> void:
	_write(BUNDLE.path_join("mods/pack/pack.toml"), "pack v1")
	_write(BUNDLE.path_join("mods/pack/assets/a/model.glb"), "model v1")
	_write(BUNDLE.path_join("mods/pack/assets/a/model.glb.import"), "sidecar")
	_write(BUNDLE.path_join("mods/pack/.hidden/x"), "hidden")
	_write(BUNDLE.path_join("mods/.keep"), "keep")
	_write(BUNDLE.path_join("worlds/city.sqlite"), "city v1")
	_write(BUNDLE.path_join("worlds/old.sqlite"), "old v1")

func _run() -> void:
	_remove(ROOT)
	_bundle_v1()
	DirAccess.make_dir_recursive_absolute(_native(PROFILE))

	# Fresh install: everything seeded, sidecars and hidden entries left out, a receipt written.
	_expect_steps(_start(), ["seed mods/pack", "seed worlds/city.sqlite", "seed worlds/old.sqlite"], "fresh")
	_expect_pack_is_bundled("fresh")
	_expect(_read(PROFILE.path_join("worlds/city.sqlite")) == "city v1", "fresh: city not seeded")
	_expect(not _exists(PROFILE.path_join("mods/.keep")), "fresh: .keep copied")
	_expect(_exists(PROFILE.path_join("bundled.toml")), "fresh: no receipt")
	_expect_no_staging("fresh")
	_expect_steps(_start(), [], "second start")

	# An untouched pack updated: a file changed, one added, one removed.
	_write(BUNDLE.path_join("mods/pack/pack.toml"), "pack v2")
	_write(BUNDLE.path_join("mods/pack/assets/b/new.glb"), "new")
	_remove(BUNDLE.path_join("mods/pack/assets/a"))
	_expect_steps(_start(), ["replace mods/pack"], "update")
	_expect_pack_is_bundled("update")
	_expect(not _exists(PROFILE.path_join("mods/pack/assets/a")), "update: the removed file stays")
	_expect_no_staging("update")
	_expect_steps(_start(), [], "after update")

	# An edited pack: kept as the player's, logged, and never updated again.
	_write(PROFILE.path_join("mods/pack/pack.toml"), "the player's pack")
	_write(BUNDLE.path_join("mods/pack/pack.toml"), "pack v3")
	var seq: int = MetrumDebug.read_since(0, 0).last_seq
	_expect_steps(_start(), ["keep_players mods/pack"], "edited")
	_expect(_read(PROFILE.path_join("mods/pack/pack.toml")) == "the player's pack", "edited: the player's copy changed")
	_expect(_log_has(seq, "kept the player's copy of mods/pack"), "edited: no debug line")
	_expect(_read(PROFILE.path_join("bundled.toml")).contains("owner = \"player\""), "edited: not recorded as the player's")
	_write(BUNDLE.path_join("mods/pack/pack.toml"), "pack v4")
	_expect_steps(_start(), ["keep_players mods/pack"], "edited, next revision")
	_expect_steps(_start(), [], "after edit")

	# A world no longer bundled, untouched: removed.
	_remove(BUNDLE.path_join("worlds/old.sqlite"))
	_expect_steps(_start(), ["remove worlds/old.sqlite"], "dropped world")
	_expect(not _exists(PROFILE.path_join("worlds/old.sqlite")), "dropped world: still there")

	# A deleted entry, the player's pack included, is seeded again as the game's.
	_remove(PROFILE.path_join("worlds/city.sqlite"))
	_remove(PROFILE.path_join("mods/pack"))
	_expect_steps(_start(), ["seed mods/pack", "seed worlds/city.sqlite"], "deleted")
	_expect_pack_is_bundled("deleted")
	_expect(_read(PROFILE.path_join("bundled.toml")).contains("owner = \"game\""), "deleted: not the game's again")

	# An interrupted copy: partial staging entries are removed and the copy is done again.
	_write(PROFILE.path_join("mods/.bundled-pack/pack.toml"), "partial")
	_remove(PROFILE.path_join("worlds/city.sqlite"))
	_write(PROFILE.path_join("worlds/.bundled-city.sqlite"), "partial")
	_expect_steps(_start(), ["remove_staging mods/pack", "remove_staging worlds/city.sqlite", "seed worlds/city.sqlite"], "interrupted")
	_expect(_read(PROFILE.path_join("worlds/city.sqlite")) == "city v1", "interrupted: city not seeded")
	_expect_no_staging("interrupted")
	_expect_steps(_start(), [], "after interrupted")

	# A profile from before the receipt with a stale pack (D2 (a)): to the Trash, bundled installed.
	_remove(PROFILE)
	_write(PROFILE.path_join("mods/pack/pack.toml"), "pack v0")
	_write(PROFILE.path_join("worlds/city.sqlite"), "city v1")
	_expect_steps(_start(), ["trash_and_replace mods/pack", "adopt worlds/city.sqlite"], "pre-receipt")
	_expect(_read(TRASH.path_join("pack/pack.toml")) == "pack v0", "pre-receipt: the stale pack is not in the Trash")
	_expect_pack_is_bundled("pre-receipt")
	_expect_no_staging("pre-receipt")
	_expect_steps(_start(), [], "after pre-receipt")

	# The same without a Trash: nothing deleted, the receipt still written, so the next start
	# keeps the copy as the player's instead of trying the Trash again.
	_remove(PROFILE)
	_remove(TRASH)
	_write(PROFILE.path_join("mods/pack/pack.toml"), "pack v0")
	_expect_steps(_start(_no_trash), ["trash_and_replace mods/pack", "seed worlds/city.sqlite"], "no Trash")
	_expect(_read(PROFILE.path_join("mods/pack/pack.toml")) == "pack v0", "no Trash: the stale pack was touched")
	_expect(not _exists(TRASH), "no Trash: something was moved")
	_expect(_exists(PROFILE.path_join("bundled.toml")), "no Trash: no receipt")
	_expect_no_staging("no Trash")
	_expect_steps(_start(), ["keep_players mods/pack"], "after no Trash")
	_expect(not _exists(TRASH), "after no Trash: the pack went to the Trash later")

	_remove(ROOT)
	if _failures == 0:
		print("user_data_bootstrap_test: passed")
	quit(1 if _failures > 0 else 0)
