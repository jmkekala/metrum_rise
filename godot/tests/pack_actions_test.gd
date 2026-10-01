# SPDX-License-Identifier: GPL-2.0-only

## Per-pack Verify / Show folder / Remove in Options → Mods (`TOOLS-11`), through the real
## panel and Rust bridge. Writes packs and a stand-in Trash, so it refuses to run outside an
## isolated test profile.
extends SceneTree

const PackManager = preload("res://scripts/ui/pack_manager.gd")
const ModPackConfig = preload("res://scripts/core/mod_pack_config.gd")
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
	_expect(actions.remove_dialog("kenney", message) == null and str(messages[-1]).contains("bundled"), "bundled packs cannot be removed")

	manager.free()
	_remove_tree(MODS + LOCAL)
	_remove_tree("user://trash-test")
	_remove_tree("user://exports")
	await process_frame
	if _failures == 0:
		print("PASS pack_actions_test")
	quit(0 if _failures == 0 else 1)

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
