# SPDX-License-Identifier: GPL-3.0-or-later

## Per-pack Verify / Show folder / Remove in Options → Mods (`TOOLS-11`), through the real
## panel and Rust bridge, and Restore of a bundled pack the player changed (`TOOLS-17`): the note
## and button only on that pack, the running-city refusal, cancel, a failing Trash and the restore
## itself. Writes packs, the install receipt and a stand-in Trash, so it refuses to run outside
## an isolated test profile.
extends SceneTree

const PackManager = preload("res://scripts/ui/pack_manager.gd")
const ModPackConfig = preload("res://scripts/core/mod_pack_config.gd")
const UserDataBootstrap = preload("res://scripts/core/user_data_bootstrap.gd")
const RECEIPT := "user://bundled.toml"
const PARKING := "user://mods/parking/"
const PACK := "actions-test"
const LOCAL := "actions-local"
const MODS := "user://mods/"
const ASSET := "user://mods/actions-test/assets/prop.bench/"

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
		push_error("pack_actions_test must run in the isolated profile from ./run.sh --test")
		quit(1)
		return
	_install_fixture()
	_expect(AssetAuthoringFiles.create_pack("user://mods", LOCAL, "Local", "Tester").is_empty(), "local pack created")
	ModPackConfig.save_enabled_pack_ids([PACK, LOCAL])
	var manager = PackManager.new()
	root.add_child(manager)
	await process_frame
	var actions = manager.actions
	var trashed := []
	DirAccess.make_dir_recursive_absolute(ProjectSettings.globalize_path("user://trash-test"))
	actions.trash_operation = func(path: String) -> int:
		trashed.append(path)
		return DirAccess.rename_absolute(path, ProjectSettings.globalize_path("user://trash-test/%d" % trashed.size()))
	var shown := []
	actions.show_operation = func(path: String) -> int:
		shown.append(path)
		return OK

	_expect(_button(manager, PACK, "Verify").visible, "imported pack offers Verify")
	_expect(not _button(manager, LOCAL, "Verify").visible, "a local pack without checksums hides Verify")
	_button(manager, PACK, "ShowFolder").pressed.emit()
	_expect(shown == [ProjectSettings.globalize_path(MODS + PACK)], "Show folder opens the pack folder: %s" % [shown])

	var dialog: AcceptDialog = actions.verify_dialog(PACK)
	var report: Label = dialog.get_child(0)
	await _until(func(): return not report.text.begins_with("Checking"))
	_expect(report.text.begins_with("All 4 files match"), "intact pack verifies: " + report.text)
	dialog.queue_free()
	await process_frame
	_write(ASSET + "tex.png", "edited")
	dialog = actions.verify_dialog(PACK)
	report = dialog.get_child(0)
	await _until(func(): return not report.text.begins_with("Checking"))
	_expect(report.text.contains("Changed since import (1):\nassets/prop.bench/tex.png"), "changed file reported: " + report.text)
	dialog.queue_free()
	await process_frame

	# Removal is refused while a running city uses the pack, then moves it to Trash.
	var messages := []
	var message := func(text: String): messages.append(text)
	actions.pack_in_use = func(_id: String) -> bool: return true
	_expect(actions.remove_dialog(PACK, message) == null and str(messages[-1]).contains("running city"), "in-use pack cannot be removed")
	actions.pack_in_use = func(_id: String) -> bool: return false
	_button(manager, LOCAL, "Enabled").button_pressed = false
	var confirm: ConfirmationDialog = actions.remove_dialog(PACK, message)
	confirm.confirmed.emit()
	await process_frame
	_expect(trashed.size() == 1 and DirAccess.dir_exists_absolute(ProjectSettings.globalize_path("user://trash-test/1/assets")), "pack moved to Trash")
	_expect(not DirAccess.dir_exists_absolute(MODS + PACK) and not manager._checks.has(PACK), "removed pack leaves the list")
	_expect(not PACK in ModPackConfig.load_enabled_pack_ids(), "removal drops the enabled entry")
	_expect(manager._status_label.text == "Moved %s to Trash." % PACK, "status reports the removal: " + manager._status_label.text)
	_expect(manager._checks.has(LOCAL) and not manager._checks[LOCAL].button_pressed and manager.has_pending_changes(), "unapplied changes survive the reload")
	messages.clear()
	_expect(actions.remove_dialog("parking", message) == null and str(messages[-1]).contains("bundled"), "bundled packs cannot be removed")
	await _restore(manager, actions)

	manager.free()
	_remove_tree(MODS + LOCAL)
	_remove_tree("user://trash-test")
	_remove_tree("user://exports")
	await process_frame
	if _failures == 0:
		print("PASS pack_actions_test")
	quit(0 if _failures == 0 else 1)

# Restore: the bundled `parking` made the player's (an edited copy, a receipt saying so) offers
# the note and the button; cancel and a failing Trash change nothing; confirming trashes the
# copy, installs the bundled one and records it as the game's.
func _restore(manager: Node, actions) -> void:
	var receipt_before := FileAccess.get_file_as_string(RECEIPT) if FileAccess.file_exists(RECEIPT) else ""
	var had_parking := DirAccess.dir_exists_absolute(PARKING)
	if had_parking:
		_remove_tree(PARKING.trim_suffix("/"))
	UserDataBootstrap._copy_dir("res://bootstrap/mods/parking", PARKING.trim_suffix("/"))
	var edited := FileAccess.get_file_as_string(PARKING + "pack.toml").replace("version = \"0.1.0\"", "version = \"0.0.9\"")
	_write(PARKING + "pack.toml", edited)
	_write(RECEIPT, "[entries.\"mods/parking\"]\nrevision = \"%s\"\nowner = \"player\"\n" % "0".repeat(64))
	manager.refresh()
	var note: Label = manager.find_child("parking", true, false).find_child("PlayersCopy", true, false)
	_expect(note != null and note.text == "Your changed copy: it no longer follows the game's updates. The game's version is v0.1.0.", "the player's bundled copy says so: %s" % (note.text if note else "no note"))
	_expect(_button(manager, "parking", "Restore").visible, "the player's bundled copy offers Restore")
	_expect(not _button(manager, LOCAL, "Restore").visible and manager.find_child(LOCAL, true, false).find_child("PlayersCopy", true, false) == null, "a player's own pack has no Restore")

	var messages := []
	var message := func(text: String): messages.append(text)
	actions.pack_in_use = func(_id: String) -> bool: return true
	_expect(actions.restore_dialog("parking", message) == null and str(messages[-1]).contains("running city"), "an in-use pack is not restored")
	actions.pack_in_use = func(_id: String) -> bool: return false
	var restore_trash := []
	actions.trash_operation = func(path: String) -> int:
		restore_trash.append(path)
		return ERR_UNAVAILABLE
	var dialog: ConfirmationDialog = actions.restore_dialog("parking", message)
	_expect(dialog != null and dialog.get_ok_button().text == "Restore", "Restore asks first")
	dialog.canceled.emit()
	await process_frame
	_expect(restore_trash.is_empty() and FileAccess.get_file_as_string(PARKING + "pack.toml") == edited, "cancel changes nothing")
	_button(manager, "parking", "Restore").pressed.emit()
	var shown: Array = manager.get_children().filter(func(child: Node) -> bool: return child is ConfirmationDialog and child.visible)
	_expect(shown.size() == 1, "the Restore button opens the confirmation")
	if shown.size() == 1:
		shown[0].confirmed.emit()
	await process_frame
	_expect(restore_trash == [ProjectSettings.globalize_path(PARKING.trim_suffix("/"))], "the player's copy was offered to the Trash: %s" % [restore_trash])
	_expect(manager._status_label.text.begins_with("Could not move your copy of parking to the Trash") and manager._status_label.text.ends_with("Nothing was changed."), "a failing Trash is reported: " + manager._status_label.text)
	_expect(FileAccess.get_file_as_string(PARKING + "pack.toml") == edited and FileAccess.get_file_as_string(RECEIPT).contains("owner = \"player\""), "a failing Trash leaves the copy and the receipt")

	DirAccess.make_dir_recursive_absolute(ProjectSettings.globalize_path("user://trash-test"))
	actions.trash_operation = func(path: String) -> int:
		restore_trash.append(path)
		return DirAccess.rename_absolute(path, ProjectSettings.globalize_path("user://trash-test/restored"))
	dialog = actions.restore_dialog("parking", message)
	dialog.confirmed.emit()
	await process_frame
	_expect(FileAccess.get_file_as_string("user://trash-test/restored/pack.toml") == edited, "the player's copy is in the Trash")
	_expect(FileAccess.get_file_as_string(PARKING + "pack.toml") == FileAccess.get_file_as_string("res://bootstrap/mods/parking/pack.toml"), "the game's version is installed")
	_expect(FileAccess.get_file_as_string(RECEIPT).contains("owner = \"game\"") and AssetAuthoringFiles.bundled_restorable("user://").is_empty(), "the receipt records it as the game's")
	_expect(manager._status_label.text == "Restored the game's version of parking. Your copy is in the system Trash.", "status reports the restore: " + manager._status_label.text)
	_expect(not _button(manager, "parking", "Restore").visible and manager.find_child("parking", true, false).find_child("PlayersCopy", true, false) == null, "the restored pack no longer offers Restore")

	# The profile as it was for the tests after this one.
	if receipt_before.is_empty():
		DirAccess.remove_absolute(ProjectSettings.globalize_path(RECEIPT))
	else:
		_write(RECEIPT, receipt_before)
	if not had_parking:
		_remove_tree(PARKING.trim_suffix("/"))

# Exports a pack, then imports it so the installed copy keeps checksums.sha256.
func _install_fixture() -> void:
	_expect(AssetAuthoringFiles.create_pack("user://mods", PACK, "Actions test", "Tester").is_empty(), "fixture pack created")
	_write(ASSET + "asset.toml", "asset_id = \"prop.bench\"\ndisplay_name = \"Bench\"\n[prop]\ncategory = \"test\"\nbounding_size_m = [1.0, 1.0, 1.0]\nsnap_mode = \"free\"\nterrain_behavior = \"flat_ground\"\n[[lods]]\nfile = \"model.gltf\"\ndistance_min_m = 0.0\n")
	_write(ASSET + "model.gltf", "{\"asset\":{\"version\":\"2.0\"},\"images\":[{\"uri\":\"tex.png\"}]}")
	_write(ASSET + "tex.png", "png")
	var mods := ProjectSettings.globalize_path("user://mods")
	var exports := ProjectSettings.globalize_path("user://exports")
	DirAccess.make_dir_recursive_absolute(exports)
	var exported := AssetAuthoringFiles.export_pack(mods, PACK, exports, "")
	_remove_tree(MODS + PACK)
	var staged := AssetAuthoringFiles.stage_import(mods, str(exported.get("path", "")), str(exported.get("sha256", "")), PackedStringArray())
	_expect(AssetAuthoringFiles.commit_import(mods, str(staged.get("staging", ""))).has("pack_id"), "fixture imported: %s" % [staged])

func _button(manager: Node, pack_id: String, button: String) -> BaseButton:
	var row: Node = manager.find_child(pack_id, true, false)
	if button == "Enabled": return manager._checks[pack_id]
	return row.find_child(button, true, false)

func _until(condition: Callable) -> void:
	for frame in 600:
		if condition.call(): return
		await process_frame
	_expect(false, "timed out waiting for a background pack task")

func _write(path: String, text: String) -> void:
	DirAccess.make_dir_recursive_absolute(path.get_base_dir())
	var file := FileAccess.open(path, FileAccess.WRITE)
	file.store_string(text)

func _remove_tree(path: String) -> void:
	for directory in DirAccess.get_directories_at(path): _remove_tree(path.path_join(directory))
	for file in DirAccess.get_files_at(path): DirAccess.remove_absolute(path.path_join(file))
	DirAccess.remove_absolute(path)
