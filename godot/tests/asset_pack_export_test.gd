# SPDX-License-Identifier: GPL-2.0-only

## Pack settings and share-archive export through the real editor dialogs and Rust bridge.
## Writes packs and archives, so it refuses to run outside an isolated test profile.
extends SceneTree

const EditorScene = preload("res://scenes/AssetEditor.tscn")
const PACK := "export-test"
const ASSET := "user://mods/export-test/assets/prop.bench/"

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
		push_error("asset_pack_export_test must run in the isolated profile from ./run.sh --test")
		quit(1)
		return
	var editor = EditorScene.instantiate()
	root.add_child(editor)
	await process_frame
	_expect(AssetAuthoringFiles.create_pack("user://mods", PACK, "Export test", "Tester").is_empty(), "fixture pack created")
	DirAccess.make_dir_recursive_absolute(ASSET)
	_write(ASSET + "asset.toml", "asset_id = \"prop.bench\"\ndisplay_name = \"Bench\"\n[prop]\ncategory = \"test\"\nbounding_size_m = [1.0, 1.0, 1.0]\nsnap_mode = \"free\"\nterrain_behavior = \"flat_ground\"\n[[lods]]\nfile = \"model.gltf\"\ndistance_min_m = 0.0\n")
	_write(ASSET + "model.gltf", "{\"asset\":{\"version\":\"2.0\"},\"images\":[{\"uri\":\"tex.png\"}]}")
	_write(ASSET + "tex.png", "png")
	_write(ASSET + "notes.txt", "not referenced")
	var packs = editor._menus.library.packs
	await _test_settings(packs)
	await _test_export(packs)
	await process_frame
	_write("user://mods/export-test/loose/asset.toml", "")
	var refused: ConfirmationDialog = packs.export_dialog(PACK)
	var summary: Label = _labels(refused)[0]
	await _until(func(): return summary.text.begins_with("This pack cannot be exported"))
	_expect(summary.text.contains("assets/<asset_id>"), "misplaced assets refuse export with the reason")
	_expect(refused.get_ok_button().disabled, "a refused pack cannot be exported")
	refused.queue_free()
	_remove_tree("user://mods/" + PACK)
	_remove_tree("user://exports")
	editor.free()
	await process_frame
	if _failures == 0:
		print("PASS asset_pack_export_test")
	quit(0 if _failures == 0 else 1)

func _test_settings(packs) -> void:
	var manifest := "user://mods/%s/pack.toml" % PACK
	var before := FileAccess.get_file_as_string(manifest)
	var dialog: ConfirmationDialog = packs.settings_dialog(PACK)
	var fields := _edits(dialog)
	_expect(fields[1].text == "0.1.0", "settings show the current version")
	fields[1].text = "1.2"
	dialog.confirmed.emit()
	_expect(not _labels(dialog)[-1].text.is_empty(), "non-semver version is reported")
	_expect(FileAccess.get_file_as_string(manifest) == before, "rejected settings leave pack.toml untouched")
	fields[1].text = "1.2.0"
	fields[4].text = "Shared \"pack\""
	dialog.confirmed.emit()
	await process_frame
	var after := FileAccess.get_file_as_string(manifest)
	_expect(after.contains("version = \"1.2.0\"") and after.contains("Shared \\\"pack\\\""), "settings rewrite validated metadata")
	_expect(after.contains("pack_id = \"%s\"" % PACK), "pack identity is unchanged")

func _test_export(packs) -> void:
	var exports := ProjectSettings.globalize_path("user://exports")
	DirAccess.make_dir_recursive_absolute(exports)
	packs.destination = exports
	var dialog: ConfirmationDialog = packs.export_dialog(PACK)
	await _until(func(): return not dialog.get_ok_button().disabled)
	var labels := _labels(dialog)
	_expect(labels[0].text.contains("1 assets, 5 files"), "summary counts referenced files: " + labels[0].text)
	_expect(labels[1].text.contains("notes.txt"), "unreferenced files are listed as excluded")
	var bump: OptionButton = _find(dialog, "OptionButton")[0]
	# Never rely on the default destination: it is the real Documents folder.
	var folder: LineEdit = dialog.find_child("Destination", true, false)
	var digest: LineEdit = dialog.find_child("Sha256", true, false)
	folder.text = exports
	bump.select(1)
	dialog.confirmed.emit()
	await _until(func(): return not digest.text.is_empty())
	var archive := exports.path_join("%s-1.2.1.metrum.zip" % PACK)
	_expect(FileAccess.get_sha256(archive) == digest.text, "dialog shows the archive SHA-256")
	_expect(FileAccess.get_file_as_string(archive + ".sha256") == "%s  %s\n" % [digest.text, archive.get_file()], "sidecar records the archive hash")
	_expect(FileAccess.get_file_as_string("user://mods/%s/pack.toml" % PACK).contains("version = \"1.2.1\""), "patch bump rewrote pack.toml")
	_expect(bump.get_item_text(0).contains("1.2.1"), "version choices follow the bumped manifest")
	var zip := ZIPReader.new()
	_expect(zip.open(archive) == OK, "archive opens as a zip")
	var expected := PackedStringArray(["assets/prop.bench/asset.toml", "assets/prop.bench/model.gltf", "assets/prop.bench/tex.png", "checksums.sha256", "pack.toml"])
	var names := PackedStringArray()
	for path in expected: names.append(PACK + "/" + path)
	_expect(zip.get_files() == names, "archive holds exactly the referenced files: %s" % [zip.get_files()])
	zip.close()
	# Same version again: the first press asks, the second replaces with identical bytes.
	var first := digest.text
	digest.text = ""
	bump.select(0)
	dialog.confirmed.emit()
	_expect(_labels(dialog)[-1].text.contains("already exists") and digest.text.is_empty(), "existing archive needs a second press")
	dialog.confirmed.emit()
	await _until(func(): return not digest.text.is_empty())
	_expect(digest.text == first, "re-export of an unchanged pack is byte-identical")
	dialog.queue_free()

func _until(condition: Callable) -> void:
	for frame in 600:
		if condition.call(): return
		await process_frame
	_expect(false, "timed out waiting for a background pack task")

# Searches the dialog body only; the folder picker carries LineEdits of its own.
func _find(dialog: Node, type: String) -> Array:
	return dialog.get_child(0).find_children("*", type, true, false)

func _edits(dialog: Node) -> Array:
	return _find(dialog, "LineEdit")

func _labels(dialog: Node) -> Array:
	# Body labels only: the first is the summary, the last the status/error line.
	var body: VBoxContainer = dialog.get_child(0)
	var titles := ["Version", "Destination folder", "Name", "Version (semantic, e.g. 1.2.0)", "Author", "License", "Description"]
	return body.get_children().filter(func(child): return child is Label and not child.text in titles)

func _write(path: String, text: String) -> void:
	DirAccess.make_dir_recursive_absolute(path.get_base_dir())
	var file := FileAccess.open(path, FileAccess.WRITE)
	file.store_string(text)

func _remove_tree(path: String) -> void:
	for directory in DirAccess.get_directories_at(path): _remove_tree(path.path_join(directory))
	for file in DirAccess.get_files_at(path): DirAccess.remove_absolute(path.path_join(file))
	DirAccess.remove_absolute(path)
