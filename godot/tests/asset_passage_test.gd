# SPDX-License-Identifier: GPL-3.0-or-later

## Asset editor regression for freight yards (`PARK-19`): a Passage authored on a mesh part through
## the passage panel opens the part for a truck, its loading bay becomes a dock whose route the
## preview holds, and a rejected loading bay is an export issue only with a freight gate.
## Run with --asset-editor in an isolated profile; the fixture model is generated.
extends SceneTree

const EditorScene = preload("res://scenes/AssetEditor.tscn")
var failures := 0
var editor: Node

func _initialize() -> void:
	call_deferred("_run")

func expect(value: bool, message: String) -> void:
	if not value:
		failures += 1
		push_error(message)

# A 12 m by 16 m block, 10 m high, standing on the ground: the bunker a lane runs through.
func _model(path: String) -> void:
	var scene := Node3D.new()
	var mesh := MeshInstance3D.new()
	mesh.mesh = BoxMesh.new()
	mesh.mesh.size = Vector3(12, 10, 16)
	mesh.position = Vector3(0, 5, 0)
	scene.add_child(mesh)
	var document := GLTFDocument.new()
	var state := GLTFState.new()
	expect(document.append_from_scene(scene, state) == OK, "generate the bunker")
	expect(document.write_to_filesystem(state, path) == OK, "write the bunker")
	scene.free()

func _loading_bay_issue() -> bool:
	editor._session.validate()
	return editor._session._issues.any(func(issue): return issue.get("anchor", -1) == 2)

func _run() -> void:
	var path := ProjectSettings.globalize_path("user://passage-bunker.glb")
	_model(path)
	editor = EditorScene.instantiate()
	root.add_child(editor)
	editor._session.create_asset({"type": "residential", "name": "Yard fixture", "pack": {"pack_id": "passage-test"}})
	editor._on_glb_file_selected(path)
	var document = editor._session.document
	var fixture: Dictionary = document.snapshot()
	var params: Dictionary = fixture["params"]
	params["lot_width_cells"] = 3
	params["lot_depth_cells"] = 4
	params["frontage_forward"] = [0.0, 0.0, 1.0]
	params["mesh_parts"][0]["position"] = [-7.5, 0.0, 0.0]
	params["mesh_parts"][0]["pivot_offset"] = [0.0, 0.0, 0.0]
	params["anchors"] = [
		{"anchor_type": "entrance", "name": "main", "position": [10.0, 0.0, 10.0], "forward": [0.0, 0.0, 1.0]},
		{"anchor_type": "driveway", "name": "truck_in", "position": [-7.5, 0.0, 20.0], "forward": [0.0, 0.0, -1.0], "width_m": 6.0, "vehicle_class": "freight"},
		{"anchor_type": "loading_bay", "name": "bay", "position": [-7.5, 0.0, 6.0], "forward": [0.0, 0.0, -1.0], "width_m": 3.5, "length_m": 8.0, "vehicle_class": "freight"},
	]
	params["site_surfaces"] = [{"name": "Yard", "material": "asphalt", "y_m": 0.01, "vertices": [[-15.0, -20.0], [15.0, -20.0], [15.0, 20.0], [-15.0, 20.0]]}]
	document.reset(fixture)
	for frame in 6:
		await process_frame

	# The bay lies inside the bunker: rejected, and with a freight gate an export issue.
	var diagnostics: Dictionary = editor._parking_diagnostics()
	expect(diagnostics.get("dock_rejected", []).size() == 1 and str(diagnostics["dock_rejected"][0]["reason"]) == "inside the building: mark the lane as a Passage on its mesh part" and bool(diagnostics["dock_rejected"][0]["blocks"]), "a bay in a wall is rejected: %s" % diagnostics)
	expect(_loading_bay_issue(), "a rejected loading bay of an asset with a gate blocks export")
	expect(editor._preview.parking_rejection(2).begins_with("inside the building"), "the preview marks the rejected bay")

	# The Passage object: listed on its part, added, sized and opened through the panel.
	editor._select_mesh_part(0)
	var panel = editor._passages
	panel.add()
	expect(panel.passages().size() == 1 and panel.list.item_count == 1 and panel.selected == 0, "Add passage lists one passage on the part")
	var added: Dictionary = panel.passages()[0]
	expect(added["position"] == [0.0, 0.0, 8.0] and added["forward"] == [0.0, 0.0, -1.0], "a new passage opens into the part's front face: %s" % added)
	panel._length.value = 16.0
	panel._height.value = 4.6
	panel._through.button_pressed = true
	var lane: Dictionary = panel.passages()[0]
	expect(is_equal_approx(float(lane["length_m"]), 16.0) and is_equal_approx(float(lane["clear_height_m"]), 4.6) and bool(lane["through"]), "the panel sizes the passage: %s" % lane)
	expect(document.undo_label() == "Edit passage", "each passage change is one undo step")
	diagnostics = editor._parking_diagnostics()
	expect(diagnostics.get("dock_rejected", []).is_empty() and diagnostics.get("docks", []).size() == 1, "the passage opens the bunker for the truck: %s" % diagnostics)
	expect(not _loading_bay_issue(), "a proven dock is no export issue")
	expect(diagnostics.get("passages", []).size() == 1 and editor._preview._passages.size() == 1, "the preview draws the passage")
	editor._set_selected_site_anchors([2], 2)
	expect(editor._preview._dock_routes.has(2), "the preview holds the dock's route")
	# The projection keeps the passage, so the export receives it.
	expect(editor._session._adapter.geometry()["mesh_parts"][0].get("passages", []).size() == 1, "the part's manifest carries its passage")
	var inspected: Dictionary = JSON.parse_string(editor._session.policy.inspect_json(JSON.stringify(document.snapshot()["params"])))
	expect(not inspected["issues"].any(func(issue): return str(issue["message"]).contains("passage")), "the passage validates: %s" % str(inspected["issues"]))

	# A passage left outside its part is an issue.
	panel._z.value = 14.0
	editor._session.validate()
	expect(editor._session._issues.any(func(issue): return str(issue["message"]).contains("leaves the part's bounds")), "a passage out of its part is an issue")
	document.undo()
	expect(is_equal_approx(float(panel.passages()[0]["position"][2]), 8.0), "undo puts the passage back")

	# Without a freight gate, a rejected loading bay is a note and the asset still exports.
	editor._select_mesh_part(0)
	panel.delete()
	expect(panel.passages().is_empty(), "Delete passage removes it")
	var car_gate: Dictionary = document.snapshot()
	car_gate["params"]["anchors"][1]["vehicle_class"] = "car"
	document.apply(car_gate, "Car gate")
	diagnostics = editor._parking_diagnostics()
	expect(str(diagnostics["dock_rejected"][0]["reason"]) == "unused: no freight Driveway" and not bool(diagnostics["dock_rejected"][0]["blocks"]), "without a freight Driveway the bay is a note")
	expect(not _loading_bay_issue(), "a note does not block export")
	expect(editor._preview.parking_note(2) == "unused: no freight Driveway", "the preview shows the note")
	editor.free()
	await process_frame
	if failures == 0:
		print("PASS asset_passage_test")
	quit(0 if failures == 0 else 1)
