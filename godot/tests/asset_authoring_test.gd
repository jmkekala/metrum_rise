# SPDX-License-Identifier: GPL-2.0-only

## Building authoring bridge and creation UI regression using the real compiled Rust policy.
## Fixtures are generated metadata; no external asset project or installed pack is needed.
extends SceneTree

const CreationDialog = preload("res://scripts/editors/asset_editor/creation_dialog.gd")

var _failures := 0
var _created: Array[Dictionary] = []

func _initialize() -> void:
	call_deferred("_run")

func _expect(value: bool, message: String) -> void:
	if not value:
		_failures += 1
		push_error(message)

func _run() -> void:
	var policy := AssetAuthoringPolicy.new()
	_expect(policy.reload_catalog().is_empty(), "real runtime catalog loads for authoring")
	var directory := "user://pack-creation-%d" % OS.get_process_id()
	_expect(not AssetAuthoringFiles.create_pack(directory, "../outside", "Invalid", "").is_empty(), "pack creation uses Rust ID validation")
	_expect(AssetAuthoringFiles.create_pack(directory, "test-pack", "Quoted \"pack\"\nname", "A\tB").is_empty(), "Rust creates validated packs")
	var manifest := directory.path_join("test-pack/pack.toml")
	var text := FileAccess.get_file_as_string(manifest)
	_expect(text.contains("\\n") and text.contains("\\t"), "pack serialization escapes authored control characters")
	_expect(not AssetAuthoringFiles.create_pack(directory, "test-pack", "Overwrite", "").is_empty() and FileAccess.get_file_as_string(manifest) == text, "existing pack is never overwritten")
	DirAccess.remove_absolute(manifest)
	DirAccess.remove_absolute(directory.path_join("test-pack"))
	DirAccess.remove_absolute(directory)
	var types: Dictionary = JSON.parse_string(policy.types_json())
	_expect(types["types"].size() == 7, "all seven building presets exist")
	for item: Dictionary in types["types"]:
		var result: Dictionary = JSON.parse_string(policy.conversion_json("{}", item["id"], "power"))
		_expect(not result.has("error"), "preset converts: " + str(item["id"]))
		var inspection: Dictionary = JSON.parse_string(policy.inspect_json(JSON.stringify(result["document"])))
		var descriptor: Dictionary = inspection["descriptor"]
		_expect(descriptor["kind"] == item["id"], "kind follows runtime metadata")
		if item["id"] == "residential":
			_expect(not descriptor["fields"].has("extractor_resource"), "residential has no mining fields")
			_expect(descriptor["profiles"].is_empty(), "residential has no business profiles")
		_expect(not inspection["issues"].is_empty(), "unfinished preset is saveable but not export-ready")
		_expect(inspection["issues"].any(func(issue): return issue["section"] == "site" and issue["field"] == "anchors"), "entrance issue has an exact destination")
	var dialog := CreationDialog.new()
	root.add_child(dialog)
	dialog.create_requested.connect(func(selection): _created.append(selection))
	dialog.configure(types, [{"pack_id": "test-pack", "display_name": "Test pack", "author": "Tester"}])
	dialog.popup_centered()
	await process_frame
	_expect(not dialog._subtype_row.visible, "residential creation hides service subtypes")
	dialog._confirm()
	_expect(_created.is_empty() and not dialog._error.text.is_empty(), "empty name keeps creation open")
	dialog._name.text = "Town house"
	dialog._confirm()
	_expect(_created.size() == 1, "one completed creation request")
	_expect(_created[0]["type"] == "residential" and _created[0]["pack"]["pack_id"] == "test-pack", "request preserves type and chosen pack")
	_expect(_created[0]["model"] == "", "model import is optional")
	for index in dialog._type.item_count:
		if dialog._type.get_item_metadata(index) == "service":
			dialog._type.select(index)
	dialog._update_type()
	_expect(dialog._subtype_row.visible, "services require a subtype")
	_expect(dialog.selection()["subtype"] == "power", "service choice is explicit")
	dialog.set_packs([], "")
	dialog._confirm()
	_expect(_created.size() == 1 and not dialog._error.text.is_empty(), "creation cannot silently invent a pack")
	dialog.free()
	if _failures == 0:
		print("PASS asset_authoring_test")
	quit(0 if _failures == 0 else 1)
