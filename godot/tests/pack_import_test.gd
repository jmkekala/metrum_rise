# SPDX-License-Identifier: GPL-2.0-only

## Share-archive import through the Options → Mods panel and the Rust bridge (`TOOLS-10`).
## Writes packs, archives and a stand-in Trash, so it refuses to run outside an isolated
## test profile.
extends SceneTree

const PackManager = preload("res://scripts/ui/pack_manager.gd")
const ModPackConfig = preload("res://scripts/core/mod_pack_config.gd")
const PACK := "import-test"
const MODS := "user://mods/"
const ASSET := "user://mods/import-test/assets/prop.bench/"

var _failures := 0

func _initialize() -> void:
	call_deferred("_run")

func _expect(value: bool, message: String) -> void:
	if not value:
		_failures += 1
		push_error(message)

func _run() -> void:
	# run.sh points user:// at a throwaway metrum-godot-tests profile; never touch a real one.
	if not OS.get_user_data_dir().to_lower().contains("godot-tests"):
		push_error("pack_import_test must run in the isolated profile from ./run.sh --test")
		quit(1)
		return
	var exported := _export_fixture()
	var archive := str(exported.get("path", ""))
	var sha := str(exported.get("sha256", ""))
	_expect(not archive.is_empty(), "fixture archive exported: %s" % [exported])
	_remove_tree(MODS + PACK)
	# A deleted pack can stay listed as enabled; a fresh import must not inherit that.
	ModPackConfig.save_enabled_pack_ids([PACK])
	var manager = PackManager.new()
	root.add_child(manager)
	await process_frame
	var importer = manager.importer
	var trashed := []
	DirAccess.make_dir_recursive_absolute(ProjectSettings.globalize_path("user://trash-test"))
	importer.trash_operation = func(path: String) -> int:
		trashed.append(path)
		return DirAccess.rename_absolute(path, ProjectSettings.globalize_path("user://trash-test/%d" % trashed.size()))

	# A wrong hash refuses; correcting it (as a pasted sidecar line) installs the pack disabled.
	var dialog: ConfirmationDialog = importer.open_archive(archive)
	_expect(dialog.find_child("PasteSha256", true, false) is Button, "the SHA-256 prompt offers Paste")
	_enter(dialog, "0".repeat(64))
	await _until(func(): return _status(dialog).contains("mismatch"))
	_expect(_status(dialog).contains(sha), "mismatch names the archive's SHA-256")
	_expect(not DirAccess.dir_exists_absolute(MODS + PACK), "a mismatching archive installs nothing")
	_enter(dialog, "%s  %s\n" % [sha.to_upper(), archive.get_file()])
	await _until(func(): return dialog.get_ok_button().text == "Install" and not dialog.get_ok_button().disabled)
	_expect(_labels(dialog)[-2].text.contains("1 assets"), "review summarises the pack: " + _labels(dialog)[-2].text)
	dialog.confirmed.emit()
	await _until(func(): return _status(dialog).begins_with("Installed"))
	_expect(FileAccess.file_exists(ASSET + "model.gltf"), "pack installed into mods")
	_expect(FileAccess.file_exists(MODS + PACK + "/checksums.sha256"), "checksums kept for later verification")
	_expect(manager._checks.has(PACK) and not manager._checks[PACK].button_pressed, "new pack is listed disabled")
	_expect(not manager.has_pending_changes(), "import leaves no pending selection change")
	_expect(not PACK in ModPackConfig.load_enabled_pack_ids(), "fresh import drops a stale enabled entry")
	_expect(_hidden(MODS).is_empty(), "no staging folder remains")
	dialog.queue_free()
	await process_frame

	# Same archive again: nothing changes.
	dialog = importer.open_archive(archive)
	_enter(dialog, sha)
	await _until(func(): return _status(dialog).begins_with("Already installed"))
	_expect(not dialog.get_ok_button().visible and trashed.is_empty(), "identical reinstall is a no-op")
	dialog.queue_free()
	await process_frame

	# Locally changed contents: replacement is refused while a city uses the pack ...
	_write(ASSET + "tex.png", "edited")
	importer.pack_in_use = func(_id: String) -> bool: return true
	dialog = importer.open_archive(archive)
	_enter(dialog, sha)
	await _until(func(): return _status(dialog).contains("running city"))
	_expect(_hidden(MODS).is_empty(), "a refused replacement removes its staging folder")
	dialog.queue_free()
	await process_frame
	# ... closing a staged import discards it ...
	importer.pack_in_use = func(_id: String) -> bool: return false
	dialog = importer.open_archive(archive)
	_enter(dialog, sha)
	await _until(func(): return dialog.get_ok_button().text == "Replace" and not dialog.get_ok_button().disabled)
	_expect(_status(dialog).contains("different contents"), "same-version replacement warns: " + _status(dialog))
	_expect(_hidden(MODS).size() == 1, "the reviewed pack waits in one staging folder: %s" % [_hidden(MODS)])
	dialog.queue_free()
	await process_frame
	_expect(_hidden(MODS).is_empty(), "closing the dialog discards the staged pack")
	# ... and confirming moves the installed copy to Trash first, keeping it enabled.
	ModPackConfig.save_enabled_pack_ids([PACK])
	dialog = importer.open_archive(archive)
	_enter(dialog, sha)
	await _until(func(): return dialog.get_ok_button().text == "Replace" and not dialog.get_ok_button().disabled)
	dialog.confirmed.emit()
	await _until(func(): return _status(dialog).begins_with("Installed"))
	_expect(trashed.size() == 1 and FileAccess.get_file_as_string("user://trash-test/1/assets/prop.bench/tex.png") == "edited", "previous copy went to Trash")
	_expect(FileAccess.get_file_as_string(ASSET + "tex.png") == "png", "archive contents replaced the edited copy")
	_expect(PACK in ModPackConfig.load_enabled_pack_ids(), "a replaced pack keeps its enabled state")
	dialog.queue_free()
	await process_frame

	manager.free()
	_remove_tree(MODS + PACK)
	_remove_tree("user://trash-test")
	_remove_tree("user://exports")
	await process_frame
	if _failures == 0:
		print("PASS pack_import_test")
	quit(0 if _failures == 0 else 1)

func _export_fixture() -> Dictionary:
	_expect(AssetAuthoringFiles.create_pack("user://mods", PACK, "Import test", "Tester").is_empty(), "fixture pack created")
	_write(ASSET + "asset.toml", "asset_id = \"prop.bench\"\ndisplay_name = \"Bench\"\n[prop]\ncategory = \"test\"\nbounding_size_m = [1.0, 1.0, 1.0]\nsnap_mode = \"free\"\nterrain_behavior = \"flat_ground\"\n[[lods]]\nfile = \"model.gltf\"\ndistance_min_m = 0.0\n")
	_write(ASSET + "model.gltf", "{\"asset\":{\"version\":\"2.0\"},\"images\":[{\"uri\":\"tex.png\"}]}")
	_write(ASSET + "tex.png", "png")
	var exports := ProjectSettings.globalize_path("user://exports")
	DirAccess.make_dir_recursive_absolute(exports)
	return AssetAuthoringFiles.export_pack(ProjectSettings.globalize_path("user://mods"), PACK, exports, "")

func _enter(dialog: ConfirmationDialog, text: String) -> void:
	(dialog.find_child("Sha256", true, false) as LineEdit).text = text
	dialog.confirmed.emit()

func _labels(dialog: ConfirmationDialog) -> Array:
	return dialog.get_child(0).get_children().filter(func(child): return child is Label)

func _status(dialog: ConfirmationDialog) -> String:
	return _labels(dialog)[-1].text

# Staging folders are dot-prefixed, which the static DirAccess listings skip.
func _hidden(path: String) -> Array:
	var dir := DirAccess.open(path)
	dir.include_hidden = true
	return Array(dir.get_directories()).filter(func(name): return name.begins_with("."))

func _until(condition: Callable) -> void:
	for frame in 600:
		if condition.call(): return
		await process_frame
	_expect(false, "timed out waiting for a background import task")

func _write(path: String, text: String) -> void:
	DirAccess.make_dir_recursive_absolute(path.get_base_dir())
	var file := FileAccess.open(path, FileAccess.WRITE)
	file.store_string(text)

func _remove_tree(path: String) -> void:
	for directory in DirAccess.get_directories_at(path): _remove_tree(path.path_join(directory))
	for file in DirAccess.get_files_at(path): DirAccess.remove_absolute(path.path_join(file))
	DirAccess.remove_absolute(path)
