# SPDX-License-Identifier: GPL-3.0-or-later

## Asset editor regression for a loading bay's drive-through waypoints in the view (`PARK-21`):
## the selected bay's authored waypoints are picked, dragged and deleted with real pointer and key
## events, a leg takes an inserted waypoint from the context menu, and every change is one undo
## step through the document's "route" command, with the route panel in step. The derived points
## of a proven way and the waypoints of an unselected bay are no handles.
## Run with --asset-editor in an isolated profile; the fixture model is generated.
extends SceneTree

const EditorScene = preload("res://scenes/AssetEditor.tscn")
const BAY := 2
var failures := 0
var editor: Node
var camera: Camera3D
var selection: RefCounted
var routes: RefCounted
var document: Object

func _initialize() -> void:
	call_deferred("_run")

func expect(value: bool, message: String) -> void:
	if not value:
		failures += 1
		push_error(message)

# A 4 m block standing clear of the drive.
func _model(path: String) -> void:
	var scene := Node3D.new()
	var mesh := MeshInstance3D.new()
	mesh.mesh = BoxMesh.new()
	mesh.mesh.size = Vector3(4, 4, 4)
	mesh.position = Vector3(0, 2, 0)
	scene.add_child(mesh)
	var gltf := GLTFDocument.new()
	var state := GLTFState.new()
	expect(gltf.append_from_scene(scene, state) == OK, "generate the block")
	expect(gltf.write_to_filesystem(state, path) == OK, "write the block")
	scene.free()

func _screen(point: Vector3) -> Vector2:
	return camera.unproject_position(point)

func _marker(index: int) -> Vector3:
	return selection.picker.waypoints()[index]["position"]

func _point(index: int) -> Vector2:
	var point: Array = routes.waypoints()[index]
	return Vector2(float(point[0]), float(point[1]))

func _waypoint_hits(mouse: Vector2, filter: String = "all") -> Array:
	return selection.picker.collect(mouse, camera, filter).filter(func(hit): return hit["kind"] == "waypoint")

func _mouse_button(mouse: Vector2, down: bool) -> void:
	var event := InputEventMouseButton.new()
	event.position = mouse
	event.global_position = mouse
	event.button_index = MOUSE_BUTTON_LEFT
	event.pressed = down
	root.push_input(event, true)

func _mouse_motion(mouse: Vector2) -> void:
	var event := InputEventMouseMotion.new()
	event.position = mouse
	event.global_position = mouse
	event.button_mask = MOUSE_BUTTON_MASK_LEFT
	root.push_input(event, true)

func _click(mouse: Vector2) -> void:
	_mouse_button(mouse, true)
	_mouse_button(mouse, false)

func _key(keycode: Key) -> void:
	var event := InputEventKey.new()
	event.keycode = keycode
	event.pressed = true
	root.push_input(event, true)

func _command(action: String) -> int:
	for index in editor._menus._commands.size():
		if editor._menus._commands[index].action == action:
			return index
	return -1

func _run() -> void:
	var path := ProjectSettings.globalize_path("user://waypoint-block.glb")
	_model(path)
	editor = EditorScene.instantiate()
	root.add_child(editor)
	selection = editor._selection
	routes = editor._routes
	camera = editor.get_node("CameraNode")
	editor._session.create_asset({"type": "residential", "name": "Waypoint fixture", "pack": {"pack_id": "waypoint-test"}})
	editor._on_glb_file_selected(path)
	document = editor._session.document
	var fixture: Dictionary = document.snapshot()
	var params: Dictionary = fixture["params"]
	params["lot_width_cells"] = 3
	params["lot_depth_cells"] = 4
	params["frontage_forward"] = [0.0, 0.0, 1.0]
	params["mesh_parts"][0]["position"] = [8.0, 0.0, -12.0]
	params["mesh_parts"][0]["pivot_offset"] = [0.0, 0.0, 0.0]
	# A freight gate on the street edge and a bay facing away from it, reached by a straight way
	# in of two waypoints; a parking spot is the unselected anchor.
	params["anchors"] = [
		{"anchor_type": "entrance", "name": "main", "position": [10.0, 0.0, 10.0], "forward": [0.0, 0.0, 1.0]},
		{"anchor_type": "driveway", "name": "truck_in", "position": [-7.5, 0.0, 20.0], "forward": [0.0, 0.0, -1.0], "width_m": 6.0, "vehicle_class": "freight"},
		{"anchor_type": "loading_bay", "name": "bay", "position": [-7.5, 0.0, 0.0], "forward": [0.0, 0.0, -1.0], "width_m": 3.5, "length_m": 8.0, "vehicle_class": "freight",
			"route_in": [[-7.5, 14.0], [-7.5, 9.0]]},
	]
	params["site_surfaces"] = [{"name": "Yard", "material": "asphalt", "y_m": 0.01, "vertices": [[-15.0, -20.0], [15.0, -20.0], [15.0, 20.0], [-15.0, 20.0]]}]
	document.reset(fixture)
	for frame in 8:
		await process_frame
	editor._cam_input.set_process(false)
	# Fixture cameras are explicit; physical wheel/orbit events must not move them.
	editor._cam_input.set_process_input(false)
	camera.look_at_from_position(Vector3(-7.5, 36, 6), Vector3(-7.5, 0, 6), Vector3.FORWARD)
	editor._cam_input._update_preview_offset()
	await process_frame
	var pane: Rect2 = editor._view._preview_view_rect.get_global_rect()

	# Unselected, the bay has no handles: its waypoints are not drawn as editable.
	var bay: Dictionary = editor._site_anchors_data[BAY]
	var first := _screen(Vector3(-7.5, editor._preview.WAYPOINT_Y_M, 9.0))
	expect(pane.has_point(first), "the fixture's waypoints lie in the interactive pane")
	expect(selection.picker.waypoints().is_empty() and _waypoint_hits(first).is_empty(), "an unselected bay's waypoints are not pickable")
	editor._select_site_anchor(0)
	expect(_waypoint_hits(first).is_empty(), "another selected anchor gives the bay no handles")
	_click(_screen(editor._anchor_position(bay) + Vector3(0, 0.1, -4)))
	expect(editor._selected_site_anchor_index == BAY and routes.selected == -1, "a click on the bay selects the bay, not a waypoint")
	var diagnostics: Dictionary = editor._parking_diagnostics()
	expect(diagnostics.get("dock_rejected", []).is_empty() and editor._preview.dock_way(BAY).get("route_in", []).size() == 5, "the way in proves: %s" % diagnostics)

	# Picking: a click on a marker selects that waypoint, the panel follows.
	routes.select_waypoint("out", -1)
	var mouse := _screen(_marker(1))
	_click(mouse)
	expect(editor._selected_site_anchor_index == BAY and routes.way == "in" and routes.selected == 1 and routes.list.is_selected(1), "a click on a marker selects its waypoint in the panel: way %s selected %d" % [routes.way, routes.selected])
	expect(editor._view.advanced_groups["drive_through"]["body"].visible and editor._view.site_picker.selected == 1, "the Drive-through group shows the picked waypoint")
	selection.refresh(mouse, true)
	expect(selection.hovered.get("kind") == "waypoint" and editor._view.picking_overlay.selected_waypoint.distance_to(mouse) < 0.5, "the marker shows as selected and hovered")
	expect(editor._view.hover_label.text.begins_with("Way in · waypoint 2"), "the status names the waypoint: %s" % editor._view.hover_label.text)
	expect(_waypoint_hits(mouse, "mesh").is_empty() and not _waypoint_hits(mouse, "anchor").is_empty(), "the selection filter keeps waypoints with access points")

	# The derived points of the proven way (connection, approach, stop) are no handles.
	var drawn: Array = editor._preview.dock_way(BAY)["route_in"]
	for index in [0, 1, drawn.size() - 1]:
		var derived := Vector3(float(drawn[index][0]), editor._preview.WAYPOINT_Y_M, float(drawn[index][1]))
		expect(_waypoint_hits(_screen(derived)).is_empty(), "derived point %d is not a waypoint: %s" % [index, drawn[index]])

	# Dragging: live in the projection and the proof, one "move" command at release.
	var before: Dictionary = document.snapshot()
	var target := _screen(_marker(1) + Vector3(0, 0, 2.0))
	_mouse_button(mouse, true)
	_mouse_motion(mouse + (target - mouse) * 0.5)
	_mouse_motion(target)
	expect(document.snapshot() == before and not document.is_editing(), "the document waits for the release")
	expect(_point(1).distance_to(Vector2(-7.5, 11.0)) < 0.05, "the waypoint follows the pointer: %s" % _point(1))
	var live: Array = editor._preview.dock_way(BAY).get("route_in", [])
	expect(live.size() == 5 and Vector2(float(live[3][0]), float(live[3][1])).distance_to(_point(1)) < 0.001, "the proof redraws live from the dragged point: %s" % [live])
	expect(is_equal_approx(routes._x.value, _point(1).x) and is_equal_approx(routes._z.value, _point(1).y), "the panel fields follow the drag")
	_mouse_button(target, false)
	var moved := _point(1)
	expect(document.undo_label() == "Move waypoint" and Vector2(float(document.snapshot()["params"]["anchors"][BAY]["route_in"][1][0]), float(document.snapshot()["params"]["anchors"][BAY]["route_in"][1][1])) == moved, "the drag is one Move waypoint command")
	expect(moved == Vector2(snappedf(moved.x, 0.01), snappedf(moved.y, 0.01)), "a dragged waypoint lands on the centimetre: %s" % moved)
	expect(is_equal_approx(routes._x.value, moved.x) and is_equal_approx(routes._z.value, moved.y) and routes.selected == 1, "the panel shows the dragged position")
	document.undo()
	expect(document.snapshot() == before and _point(1) == Vector2(-7.5, 9.0), "one undo restores the waypoint")

	# The panel's fields move the marker the view picks.
	routes._x.value = -7.25
	expect(document.undo_label() == "Move waypoint" and not _waypoint_hits(_screen(Vector3(-7.25, editor._preview.WAYPOINT_Y_M, 9.0))).is_empty(), "a field change moves the pickable marker")
	document.undo()

	# Escape cancels a drag: no command, the projection and the fields restored.
	mouse = _screen(_marker(1))
	_mouse_button(mouse, true)
	_mouse_motion(target)
	_key(KEY_ESCAPE)
	_mouse_button(target, false)
	expect(document.snapshot() == before and _point(1) == Vector2(-7.5, 9.0) and is_equal_approx(routes._z.value, 9.0), "Escape cancels the drag without an undo step")
	# Dropped where it started: no undo step either.
	var label: String = document.undo_label()
	_mouse_button(mouse, true)
	_mouse_motion(target)
	_mouse_motion(mouse)
	_mouse_button(mouse, false)
	expect(document.snapshot() == before and document.undo_label() == label, "a waypoint dropped where it started adds no undo step")

	# Delete removes the selected waypoint, one undo step; the bay stays.
	_click(mouse)
	expect(routes.selected == 1, "the waypoint is selected again")
	_key(KEY_DELETE)
	expect(routes.waypoints().size() == 1 and editor._site_anchors_data.size() == 3 and document.undo_label() == "Delete waypoint", "Delete removes the waypoint, not its bay")
	document.undo()
	expect(document.snapshot() == before, "one undo restores the deleted waypoint")

	# The context menu: Delete waypoint on a marker, Insert waypoint here on a leg.
	var menus = editor._menus
	menus.open_preview(_screen(_marker(0)))
	var command := _command("delete_waypoint")
	expect(menus.context.hit.get("kind") == "waypoint" and command >= 0 and _command("delete") < 0, "a marker's menu deletes the waypoint, never the bay")
	menus.popup.hide()
	menus.dispatch("delete_waypoint", menus.context.duplicate(true), menus._commands[command].args)
	expect(routes.waypoints().size() == 1 and _point(0) == Vector2(-7.5, 9.0) and document.undo_label() == "Delete waypoint", "Delete waypoint from the menu removes that waypoint")
	document.undo()
	var leg := Vector3(-7.5, editor._preview.DOCK_ROUTE_Y_M, 11.5)
	menus.open_preview(_screen(leg))
	command = _command("insert_waypoint")
	expect(menus.context.hit.get("kind") == "leg" and command >= 0, "a leg's menu inserts a waypoint: %s" % menus.context.hit)
	menus.popup.hide()
	menus.dispatch("insert_waypoint", menus.context.duplicate(true), menus._commands[command].args)
	expect(routes.waypoints().size() == 3 and _point(1).distance_to(Vector2(-7.5, 11.5)) < 0.1 and routes.selected == 1 and document.undo_label() == "Insert waypoint", "the waypoint is inserted where the leg was clicked: %s" % [routes.waypoints()])
	document.undo()
	expect(document.snapshot() == before, "one undo removes the inserted waypoint")
	# The gate's own stretch, from the road connection to the approach, takes no waypoint.
	var gate := Vector3((float(drawn[0][0]) + float(drawn[1][0])) * 0.5, editor._preview.DOCK_ROUTE_Y_M, (float(drawn[0][1]) + float(drawn[1][1])) * 0.5)
	expect(selection.picker.route_leg(_screen(gate), camera).is_empty(), "the gate's stretch is no insertion leg")

	editor.free()
	await process_frame
	if failures == 0:
		print("PASS asset_waypoint_test")
	quit(0 if failures == 0 else 1)
