# SPDX-License-Identifier: GPL-2.0-only

## Isolated authoring state regression: lossless drafts, dirty tracking and grouped history.
## No live scene, source assets or installed packs are needed.
extends SceneTree

const MeshPart = preload("res://scripts/editors/asset_editor/mesh_part.gd")

var _failures := 0

func _initialize() -> void:
	call_deferred("_run")

func _expect(value: bool, message: String) -> void:
	if not value:
		_failures += 1
		push_error(message)

func _run() -> void:
	var part := MeshPart.new()
	var authored := {"name": "Incomplete part", "lods": [{"file": "lod0.glb", "future_lod": {"keep": [1, 2]}}]}
	part.lods.append(MeshPart.Lod.new("/missing/lod0.glb", authored["lods"][0]))
	part.retain_manifest(authored)
	part.append_lod("/missing/lod1.glb")
	var edited: Dictionary = part.to_manifest()["lods"][0]
	_expect(edited["future_lod"] == authored["lods"][0]["future_lod"], "adding LODs preserves unknown metadata on existing tiers")
	_expect(not edited.has("distance_min_m") and edited["distance_max_m"] == 35.0, "LOD edits change only the authored boundary, not missing fields")
	part.remove_last_lod()
	_expect(part.to_manifest() == authored, "removing the added tier restores the original incomplete chain")
	var doc := AssetAuthoringDocument.new()
	var incomplete := {
		"params": {"display_name": "Unfinished", "mesh_parts": [], "household_capacity": null,
			"economy_profile": "missing-profile", "future_metadata": {"keep": [1, "two", null]}},
		"sources": [["/missing/lod0.glb"]],
	}
	doc.reset(incomplete)
	var returned := doc.snapshot()
	returned["params"]["future_metadata"]["keep"].clear()
	_expect(doc.snapshot() == incomplete, "snapshots must not alias document state")
	_expect(not doc.is_dirty(), "loaded document starts clean")
	var incoming := {"nested": [1, 2]}
	doc.set_parameter("external", incoming)
	incoming["nested"].clear()
	_expect(doc.snapshot()["params"]["external"]["nested"] == [1, 2], "Rust commands detach incoming mutable values")
	doc.undo()
	var callback_reads: Array = []
	var read_callback := func(): callback_reads.append(doc.snapshot())
	doc.changed.connect(read_callback)
	doc.begin_transaction("Move entrance")
	doc.set_parameter("anchors", [{"position": [1, 0, 2]}])
	doc.set_parameter("anchors", [{"position": [3, 0, 4]}])
	doc.commit_transaction()
	_expect(doc.is_dirty() and doc.undo_label() == "Move entrance", "drag is one named action")
	doc.undo()
	_expect(not doc.is_dirty() and doc.snapshot() == incomplete, "undo restores missing and unknown metadata exactly")
	doc.redo()
	_expect(doc.snapshot()["params"]["anchors"][0]["position"] == [3, 0, 4], "redo restores final drag")
	doc.mark_saved("saved.metrum-draft")
	_expect(not doc.is_dirty(), "savepoint is clean")
	doc.undo()
	_expect(doc.is_dirty(), "undo away from a savepoint is dirty")
	doc.redo()
	_expect(not doc.is_dirty(), "redo to a savepoint is clean")
	doc.begin_transaction("Cancelled conversion")
	doc.set_parameter("placement_mode", "explicit")
	doc.cancel_transaction()
	_expect(not doc.is_dirty(), "cancelled conversion changes nothing")
	doc.undo()
	doc.set_parameter("display_name", "New branch")
	_expect(not doc.can_redo(), "a new edit drops abandoned redo commands")
	for index in 110:
		doc.set_parameter("display_name", str(index))
	var undo_count := 0
	while doc.can_undo():
		doc.undo()
		undo_count += 1
	_expect(undo_count == AssetAuthoringDocument.HISTORY_LIMIT, "history memory is bounded")
	var path := "user://document-test-%d.metrum-draft" % OS.get_process_id()
	_expect(AssetAuthoringFiles.save_draft(path, incomplete).is_empty(), "incomplete draft saves without runtime validation")
	var loaded := AssetAuthoringFiles.load_draft(path)
	_expect(loaded.get("document", {}) == incomplete, "draft preserves unresolved profiles, missing meshes and hidden fields")
	var revised := incomplete.duplicate(true)
	revised["params"]["display_name"] = "Saved again"
	_expect(AssetAuthoringFiles.save_draft(path, revised).is_empty(), "draft safely replaces a previous save")
	_expect(AssetAuthoringFiles.load_draft(path).get("document", {}) == revised, "latest complete snapshot loads")
	_expect(not callback_reads.is_empty(), "changed callbacks can synchronously read Rust state without borrow conflicts")
	doc.changed.disconnect(read_callback)
	var file := FileAccess.open(path, FileAccess.WRITE)
	file.store_string('{"format":"metrum-asset-draft","version":999,"document":{}}')
	file.close()
	_expect(AssetAuthoringFiles.load_draft(path).has("error"), "unknown versions are rejected without normalization")
	DirAccess.remove_absolute(path)
	_benchmark()
	if _failures == 0:
		print("PASS asset_document_test")
	quit(0 if _failures == 0 else 1)

func _benchmark() -> void:
	if not "--benchmark-asset-document" in OS.get_cmdline_user_args():
		return
	var document := AssetAuthoringDocument.new()
	for entries in [100, 1000]:
		var metadata: Array = []
		for index in entries:
			metadata.append({"name": str(index), "values": [1, 2, 3, 4]})
		document.reset({"params": {"display_name": "", "metadata": metadata}})
		var start := Time.get_ticks_usec()
		for index in 100:
			document.set_parameter("display_name", str(index))
		var edit_us := (Time.get_ticks_usec() - start) / 100.0
		start = Time.get_ticks_usec()
		for index in 100:
			document.undo()
		var undo_us := (Time.get_ticks_usec() - start) / 100.0
		_expect(not document.is_dirty(), "benchmark undo restores the savepoint")
		print("asset_document_measure entries=%d commands=100 edit_mean_us=%.3f undo_mean_us=%.3f" % [entries, edit_us, undo_us])
