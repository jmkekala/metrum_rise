# SPDX-License-Identifier: GPL-3.0-or-later

## Asset editor regression for car routes (`PARK-24`): a grocery layout (a shop across the back,
## one 6 m gate at the left, a row of eight spots nose to the shop front and six nose to the
## street) has no rejected spot, and the preview holds a route per spot, drawn for every spot with
## Car routes on, its car's line rounded at the bends (`PARK-29`). Pinned to the straight path (`car_routes = "direct"`) the far spots are rejected
## with today's reasons; a spot painted across the aisle rejects the spots behind it with the new
## wording and the route last tried. The asset exports, and exporting the reopened asset writes the
## same file. The route panel (`PARK-26`): Edit this route makes the far spot's shown route its
## own, a waypoint drag, Add arc and Back to automatic are one undo step each, a failing own route
## blocks export naming Back to automatic, and an own route exports, reopens and exports the same;
## dropped again, the export is the unpinned file byte for byte.
## Run with --asset-editor in an isolated profile; the fixture model is generated.
extends SceneTree

const EditorScene = preload("res://scenes/AssetEditor.tscn")
const ID := "car-route-test:building.residential.car_route_fixture"
const FILE := "user://mods/car-route-test/assets/building.residential.car_route_fixture/asset.toml"
var failures := 0
var editor: Node

func _initialize() -> void:
	call_deferred("_run")

func expect(value: bool, message: String) -> void:
	if not value:
		failures += 1
		push_error(message)

# A 34 m by 20 m block, 6 m high: the shop, its front at z 2 once placed at z -8.
func _model(path: String) -> void:
	var scene := Node3D.new()
	var mesh := MeshInstance3D.new()
	mesh.mesh = BoxMesh.new()
	mesh.mesh.size = Vector3(34, 6, 20)
	mesh.position = Vector3(0, 3, 0)
	scene.add_child(mesh)
	var document := GLTFDocument.new()
	var state := GLTFState.new()
	expect(document.append_from_scene(scene, state) == OK, "generate the shop")
	expect(document.write_to_filesystem(state, path) == OK, "write the shop")
	scene.free()

func _rejected_issues() -> Array:
	editor._session.validate()
	return editor._session._issues.filter(func(issue): return str(issue.get("message", "")).contains("rejected"))

func _run() -> void:
	var path := ProjectSettings.globalize_path("user://car-route-shop.glb")
	_model(path)
	editor = EditorScene.instantiate()
	root.add_child(editor)
	editor._session.create_asset({"type": "residential", "name": "Car route fixture", "pack": {"pack_id": "car-route-test"}})
	editor._on_glb_file_selected(path)
	var document = editor._session.document
	var fixture: Dictionary = document.snapshot()
	var params: Dictionary = fixture["params"]
	params["lot_width_cells"] = 4
	params["lot_depth_cells"] = 4
	params["frontage_forward"] = [0.0, 0.0, 1.0]
	params["household_capacity"] = 1
	params["mesh_parts"][0]["position"] = [1.0, 0.0, -8.0]
	params["mesh_parts"][0]["pivot_offset"] = [0.0, 0.0, 0.0]
	var anchors := [
		{"anchor_type": "entrance", "name": "main", "position": [10.0, 0.0, 2.0], "forward": [0.0, 0.0, 1.0]},
		{"anchor_type": "driveway", "name": "gate", "position": [-10.75, 0.0, 20.0], "forward": [0.0, 0.0, -1.0], "width_m": 6.0, "vehicle_class": "car"},
	]
	for k in 8:
		anchors.append({"anchor_type": "parking", "name": "front %d" % (k + 1), "position": [-7.5 + 2.5 * k, 0.0, 8.05], "forward": [0.0, 0.0, -1.0], "width_m": 2.5, "length_m": 5.0, "vehicle_class": "car"})
	for k in 6:
		anchors.append({"anchor_type": "parking", "name": "street %d" % (k + 1), "position": [-4.0 + 2.5 * k, 0.0, 15.0], "forward": [0.0, 0.0, 1.0], "width_m": 2.5, "length_m": 5.0, "vehicle_class": "car"})
	params["anchors"] = anchors
	params["site_surfaces"] = [{"name": "Yard", "material": "asphalt", "y_m": 0.01, "vertices": [[-19.5, -18.35], [19.5, -18.35], [19.5, 20.0], [-19.5, 20.0]]}]
	document.reset(fixture)
	for frame in 6:
		await process_frame

	# Every spot is reached: the far front spot down the gate line and along its row's line.
	var diagnostics: Dictionary = editor._parking_diagnostics()
	var spots: Array = diagnostics.get("spots", [])
	expect(diagnostics.get("rejected", []).is_empty() and spots.size() == 14, "every spot has a route: %s" % diagnostics)
	var far: Dictionary = editor._preview.car_spot(9)
	expect(str(far.get("kind", "")) == "auto" and far["route"].size() == 4, "the far front spot is routed along the drive lines: %s" % far)
	expect(absf(float(far["route"][1][0]) + 10.75) < 0.001 and absf(float(far["route"][1][1]) - 10.55) < 0.001, "it turns off the gate line onto the row's line: %s" % far)
	expect(far.get("envelopes", []).size() == 2, "the selected spot's squares are the two bends: %s" % far)
	var drawn: Array = far.get("drawn", [])
	expect(drawn.size() > far["route"].size() and drawn[0] == far["route"][0] and drawn[-1] == far["route"][-1], "its car is drawn on a line rounded at its bends, from the same start to the same stop (PARK-29): %s" % [drawn])
	expect(editor._preview._car_spots.size() == 14, "the preview holds a route per spot")
	expect(_rejected_issues().is_empty(), "no spot blocks export")
	editor._view.car_routes_button.button_pressed = true
	expect(editor._preview._car_routes_visible, "Car routes draws every spot's route")
	editor._set_selected_site_anchors([9], 9)

	# Pinned to the straight path, the far spots fail as before the drive lines.
	var pinned: Dictionary = document.snapshot()
	pinned["params"]["car_routes"] = "direct"
	document.apply(pinned, "Pin")
	diagnostics = editor._parking_diagnostics()
	var rejected: Array = diagnostics.get("rejected", [])
	expect(not rejected.is_empty() and rejected.all(func(issue): return str(issue["reason"]).begins_with("no room to drive in: ")), "pinned, the far spots keep today's reasons: %s" % [rejected])
	expect(not _rejected_issues().is_empty(), "a rejected spot blocks export")
	document.undo()
	expect(editor._parking_diagnostics().get("rejected", []).is_empty(), "undo routes them again")

	# A spot painted across the aisle in front of the far end of the front row.
	var across: Dictionary = document.snapshot()
	across["params"]["anchors"].append({"anchor_type": "parking", "name": "across", "position": [3.5, 0.0, 9.0], "forward": [0.0, 0.0, 1.0], "width_m": 2.5, "length_m": 5.0, "vehicle_class": "car"})
	document.apply(across, "Across")
	diagnostics = editor._parking_diagnostics()
	var blocked: Array = diagnostics.get("rejected", []).filter(func(issue): return int(issue["anchor"]) == 9)
	expect(blocked.size() == 1 and str(blocked[0]["reason"]).begins_with("no way to drive to this spot: straight from the Driveway, ") and blocked[0].get("route") is Array, "the far front spot is rejected with the route it was tried along: %s" % diagnostics)
	expect(editor._preview._parking_rejection_routes.has(9), "the preview draws the tried route")
	document.undo()

	# Edit this route on the far front spot: its shown route becomes its own, drawn solid.
	var routes = editor._routes
	editor._set_selected_site_anchors([9], 9)
	routes.refresh()
	var before: Dictionary = document.snapshot()
	var shown: Array = editor._preview.car_spot(9)["route"]
	expect(routes.car() and routes._own.visible and not routes._automatic.visible and not routes._tools.visible, "an automatic spot offers Edit this route only")
	routes.pin()
	var own: Array = routes.waypoints()
	expect(document.undo_label() == "Edit this route" and own.size() == 2, "Edit this route is one step: %s" % [own])
	for k in 2:
		expect(Vector2(float(own[k][0]), float(own[k][1])).distance_to(Vector2(float(shown[k + 1][0]), float(shown[k + 1][1]))) < 0.001, "waypoint %d is the shown route's: %s %s" % [k, own, shown])
	var spot: Dictionary = editor._preview.car_spot(9)
	expect(str(spot.get("kind", "")) == "yours" and spot.get("route", []).size() == 4, "the spot follows its own route: %s" % spot)
	expect(routes._automatic.visible and routes._tools.visible and not routes._own.visible, "an own route offers its waypoints and Back to automatic")
	# Drag the corner off the gate line 0.45 m toward the street: live, one Move waypoint at release.
	routes.select_waypoint("in", 0)
	routes.begin_drag()
	routes.drag(Vector2(-10.75, 11.0))
	expect(document.snapshot()["params"]["anchors"][9]["route_in"][0][1] != 11.0, "the document waits for the release")
	routes.end_drag()
	spot = editor._preview.car_spot(9)
	expect(document.undo_label() == "Move waypoint" and str(spot.get("kind", "")) == "yours" and absf(float(spot["route"][1][1]) - 11.0) < 0.001, "the dragged route proves: %s" % [editor._parking_diagnostics()])
	# Add arc after the corner: a quarter turn in four chords, one step.
	routes._angle.value = 90
	routes._radius.value = 3.0
	routes.add_arc()
	expect(document.undo_label() == "Add arc" and routes.waypoints().size() == 6, "Add arc lays four chords: %s" % [routes.waypoints()])
	document.undo()
	# The last waypoint 5 m aside: the way into the spot runs 45° off, the spot is rejected with the
	# hint and blocks export.
	routes.select_waypoint("in", 1)
	routes._x.value = 5.0
	var stale: Array = editor._parking_diagnostics().get("rejected", []).filter(func(issue): return int(issue["anchor"]) == 9)
	expect(stale.size() == 1 and str(stale[0]["reason"]).ends_with("or choose Back to automatic") and stale[0].get("route", []).size() == 4, "a failing own route rejects its spot: %s" % [stale])
	expect(not _rejected_issues().is_empty(), "it blocks export")
	document.undo()
	# Back to automatic: the automatic route exactly, one step; three undos return to before.
	routes.unpin()
	expect(document.undo_label() == "Back to automatic" and not editor._site_anchors_data[9].has("route_in"), "Back to automatic drops the own route")
	expect(editor._preview.car_spot(9)["route"] == shown and str(editor._preview.car_spot(9)["kind"]) == "auto", "the automatic route is back as it was")
	for step in 3:
		document.undo()
	expect(document.snapshot() == before, "Edit this route, the move and Back to automatic undo to the start")

	# Export, reopen and export again: the same file.
	editor._session.publish()
	var first := FileAccess.get_file_as_string(FILE)
	expect(first.contains("[[anchors]]") and not first.contains("car_routes"), "the asset exports: %s" % editor._view.export_result.text)
	var loaded = JSON.parse_string(editor.sim.get_asset_manifest_json(ID))
	expect(loaded is Dictionary, "the exported asset loads")
	editor._session.load_manifest(loaded)
	for frame in 4:
		await process_frame
	expect(editor._parking_diagnostics().get("rejected", []).is_empty(), "the reopened asset has no rejected spot")
	editor._session.publish()
	expect(FileAccess.get_file_as_string(FILE) == first, "exporting the reopened asset writes the same file")
	# An own route exports, reopens and exports the same; dropped, the file is the unpinned one.
	editor._set_selected_site_anchors([9], 9)
	routes.refresh()
	routes.pin()
	editor._session.publish()
	var pinned_file := FileAccess.get_file_as_string(FILE)
	expect(pinned_file.contains("route_in = [[-10.75, 10.55], [10, 10.55]]"), "the own route is written: %s" % editor._view.export_result.text)
	loaded = JSON.parse_string(editor.sim.get_asset_manifest_json(ID))
	editor._session.load_manifest(loaded)
	for frame in 4:
		await process_frame
	expect(str(editor._preview.car_spot(9).get("kind", "")) == "yours", "the reopened spot follows its own route")
	editor._session.publish()
	expect(FileAccess.get_file_as_string(FILE) == pinned_file, "exporting the reopened own route writes the same file")
	editor._set_selected_site_anchors([9], 9)
	routes.refresh()
	routes.unpin()
	editor._session.publish()
	expect(FileAccess.get_file_as_string(FILE) == first, "back to automatic exports the unpinned file")
	editor.free()
	await process_frame
	if failures == 0:
		print("PASS asset_car_route_test")
	quit(0 if failures == 0 else 1)
