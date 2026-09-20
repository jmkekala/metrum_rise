# SPDX-License-Identifier: GPL-2.0-only

## Shared object commands and provisional transform/placement gestures.
## Rust prepares authored changes; pointer motion changes only the preview projection.
extends RefCounted

const MeshImportDialog = preload("res://scripts/editors/mesh_import_dialog.gd")

var editor: Node
var mode := ""
var targets: Array[Dictionary] = []
var _pending: Dictionary = {}
var _before_selection: Array[Dictionary] = []
var _positions := PackedVector3Array()
var _angles := PackedFloat32Array()
var _vertices: Array[PackedVector2Array] = []
var _bounds := Rect2()
var _frontage_bound := false
var _pointer_origin := Vector2.INF
var _ground_origin := Vector3.ZERO
var _await_pointer := false
var _committing := false
var _has_meshes := false
var _has_anchors := false
var _has_surfaces := false

func _init(owner: Node) -> void:
	editor = owner
	editor.get_window().focus_exited.connect(cancel)

func selection() -> Array[Dictionary]:
	var result: Array[Dictionary] = []
	for index in editor._selected_part_indices:
		result.append({"kind": "mesh", "index": index})
	for index in editor._selected_site_anchor_indices:
		result.append({"kind": "anchor", "index": index})
	if editor._selected_site_surface_index >= 0:
		result.append({"kind": "surface", "index": editor._selected_site_surface_index})
	return result

func select(objects: Array) -> void:
	var meshes: Array[int] = []
	var anchors: Array[int] = []
	var surface := -1
	for object in objects:
		match object.kind:
			"mesh": meshes.append(object.index)
			"anchor": anchors.append(object.index)
			"surface": surface = object.index
	editor._selection.applying_selection = true
	editor._set_selected_mesh_parts(meshes, meshes.back() if not meshes.is_empty() else -1)
	editor._set_selected_site_anchors(anchors, anchors.back() if not anchors.is_empty() else -1)
	editor._set_selected_site_surface(surface, false)
	editor._selection.applying_selection = false
	editor._session.selection_changed()

func edit(action: String, objects: Array, args: Dictionary = {}) -> bool:
	var result: Dictionary = editor._session.document.prepare_edit(action, _typed(objects), args)
	if result.has("error"):
		editor._session.message(result.error)
		return false
	editor._session.document.apply(result.document, action.capitalize().replace("_", " "))
	select(_typed(result.selection))
	return true

func rename(objects: Array[Dictionary], valid: Callable) -> void:
	var dialog := ConfirmationDialog.new()
	dialog.title = "Rename object"
	dialog.dialog_hide_on_ok = false
	var input := LineEdit.new()
	input.custom_minimum_size.x = 340
	var target := objects[0]
	match target.kind:
		"mesh": input.text = editor._parts[target.index].name
		"anchor": input.text = str(editor._site_anchors_data[target.index].get("name", ""))
		"surface": input.text = str(editor._site_surfaces_data[target.index].get("name", ""))
	dialog.add_child(input)
	dialog.confirmed.connect(func():
		if not valid.call():
			dialog.queue_free()
			return
		if edit("rename", objects, {"name": input.text}):
			dialog.queue_free()
	)
	dialog.canceled.connect(dialog.queue_free)
	editor.add_child(dialog)
	editor._view._apply_editor_theme(dialog)
	dialog.popup_centered()
	input.grab_focus()
	input.select_all()

func label(target: Dictionary) -> String:
	match target.kind:
		"mesh": return editor._parts[target.index].name
		"anchor": return editor._site_anchor_display_label(target.index)
		"surface", "vertex": return editor._site_surface_display_label(target.index)
		"reference": return "Scale reference"
		"ghost": return "Comparison model"
	return "Preview"

func properties(target: Dictionary) -> void:
	editor._selection.select_hit(target, false)

func frame(objects: Array[Dictionary], helper: String = "") -> void:
	var points: Array[Vector3] = []
	for object in objects:
		match object.kind:
			"mesh": points.append_array(editor._preview.mesh_part_world_corners(object.index))
			"anchor":
				var anchor: Dictionary = editor._site_anchors_data[object.index]
				var origin: Vector3 = editor._anchor_position(anchor)
				points.append(origin)
				for offset: Vector3 in editor._site_anchor_footprint_offsets(anchor):
					points.append(origin + offset)
			"surface": points.append_array(editor._selection.picker.surface_points(object.index))
	if helper == "reference":
		points.append(editor._preview.scale_reference_world_position())
	elif helper == "ghost":
		for corner in 8:
			points.append(editor._preview._ghost_root.global_transform * editor._preview._ghost_aabb.get_endpoint(corner))
	if points.is_empty():
		var half: Vector2 = editor._lot_half_extents()
		points.assign([Vector3(-half.x, 0, -half.y), Vector3(half.x, 0, half.y)])
	var bounds := AABB(points[0], Vector3.ZERO)
	for point in points:
		bounds = bounds.expand(point)
	var viewport_size: Vector2 = editor.get_viewport().get_visible_rect().size
	var pane: Vector2 = editor._view._preview_view_rect.size
	var scale := maxf(viewport_size.x / maxf(1, pane.x), viewport_size.y / maxf(1, pane.y))
	editor._cam_input.focus_on(bounds.get_center(), maxf(2, bounds.size.length() * 0.5) * scale)

func begin_rotation(objects: Array[Dictionary], angle: float = NAN) -> void:
	cancel()
	if not editor._session.document.selection_actions(objects).get("rotate", false):
		return
	_begin("rotate", objects, editor._session.document.snapshot())
	if not is_nan(angle):
		_rotate(angle)
		confirm()

func begin_placement(action: String, objects: Array, args: Dictionary, ground: Variant) -> void:
	cancel()
	var result: Dictionary = editor._session.document.prepare_edit(action, _typed(objects), args)
	if result.has("error"):
		editor._session.message(result.error)
		return
	var previous := selection()
	editor._session._adapter.render(result.document)
	if action == "create" and args.get("kind") == "mesh":
		var index: int = result.selection[0].index
		var part = editor._parts[index]
		# Imported models use bottom-centre as their placement pivot, as in initial asset creation.
		part.pivot_offset = -Vector3(part.aabb.get_center().x, part.aabb.position.y, part.aabb.get_center().z)
		editor._apply_mesh_part_transform_from_state(index)
		editor._last_glb_dir = str(args.path).get_base_dir()
		editor._save_config()
	_begin("place", _typed(result.selection), result.document)
	_before_selection = previous
	# New objects begin at the clicked ground point. Duplicates keep group-relative offsets.
	var centre := _bounds.get_center()
	if ground == null:
		ground = Vector3(centre.x, 0, centre.y)
	_ground_origin = ground
	_place(Vector3(ground.x - centre.x, 0, ground.z - centre.y))
	_capture_origins()

func create_from_controls(kind: String) -> void:
	if kind == "entrance" and editor._main_entrance_index() >= 0:
		properties({"kind": "anchor", "index": editor._main_entrance_index()})
		return
	begin_placement("create", [], {"kind": kind}, Vector3.ZERO)

func import_at(ground: Vector3, valid: Callable) -> void:
	var dialog := MeshImportDialog.new()
	dialog.theme_mode = editor._view._theme_mode
	dialog.mesh_selected.connect(func(path):
		if valid.call():
			dialog.hide()
			begin_placement("create", [], {"kind": "mesh", "path": path}, ground)
	)
	editor.add_child(dialog)
	dialog.open(editor._last_glb_dir)

func _begin(next_mode: String, objects: Array[Dictionary], document: Dictionary) -> void:
	editor._selection.cancel()
	_before_selection = selection()
	targets = objects
	_pending = document
	mode = next_mode
	_has_meshes = objects.any(func(object): return object.kind == "mesh")
	_has_anchors = objects.any(func(object): return object.kind == "anchor")
	_has_surfaces = objects.any(func(object): return object.kind == "surface")
	_pointer_origin = Vector2.INF
	_await_pointer = true
	select(targets)
	_capture_origins()
	_hint(0)

func _capture_origins() -> void:
	_positions.resize(targets.size())
	_angles.resize(targets.size())
	_vertices.resize(targets.size())
	_frontage_bound = false
	var first := true
	for slot in targets.size():
		var object := targets[slot]
		var points: Array[Vector3] = []
		match object.kind:
			"mesh":
				_positions[slot] = editor._parts[object.index].position
				_angles[slot] = editor._parts[object.index].rotation_y
				points = editor._preview.mesh_part_world_corners(object.index)
			"anchor":
				var anchor: Dictionary = editor._site_anchors_data[object.index]
				if anchor.get("anchor_type") == "driveway":
					_frontage_bound = true
					anchor.forward = editor._vector3_to_array(editor._driveway_anchor_forward(), 0.001)
					anchor.position = editor._vector3_to_array(editor._clamp_site_anchor_position_to_lot(anchor, editor._anchor_position(anchor)), 0.01)
				_positions[slot] = editor._anchor_position(anchor)
				_angles[slot] = editor._yaw_from_forward(editor._anchor_forward(anchor))
				points.append(_positions[slot])
				for offset: Vector3 in editor._site_anchor_footprint_offsets(anchor):
					points.append(_positions[slot] + offset)
			"surface":
				_vertices[slot] = PackedVector2Array(editor._site_surface_vertices(editor._site_surfaces_data[object.index]))
				points.assign(editor._selection.picker.surface_points(object.index))
		for point in points:
			var p := Vector2(point.x, point.z)
			_bounds = Rect2(p, Vector2.ZERO) if first else _bounds.expand(p)
			first = false

func handle_input(event: InputEvent) -> bool:
	if mode.is_empty():
		return false
	if event is InputEventKey and event.pressed and event.keycode == KEY_ESCAPE:
		cancel()
		return true
	if event is InputEventMouseButton and event.pressed and event.button_index == MOUSE_BUTTON_RIGHT:
		cancel()
		return false
	if editor._cam_input._ui_has_modal_popup():
		# A menu launches the operation deferred, after its own popup has closed.
		cancel()
		return false
	var pointer: Vector2 = event.position if event is InputEventMouse else editor.get_viewport().get_mouse_position()
	var inside: bool = editor._view._preview_view_rect.get_global_rect().has_point(pointer)
	if event is InputEventMouseMotion and inside:
		if _await_pointer:
			_pointer_origin = event.position
			_await_pointer = false
		if mode == "rotate":
			_rotate(AssetAuthoringPolicy.rotation_degrees((event.position.x - _pointer_origin.x) * editor.MESH_ROTATION_DRAG_DEG_PER_PX))
		else:
			var point = editor._project_mouse_to_horizontal_plane(event.position, 0.0)
			if point != null:
				_place(point - _ground_origin)
		return true
	if event is InputEventMouseButton and event.button_index == MOUSE_BUTTON_LEFT:
		if event.pressed and inside:
			confirm()
		return true
	return event is InputEventKey

func _rotate(delta: float) -> void:
	_rotate_transforms(delta)
	_refresh_geometry()
	_hint(delta)

func _rotate_transforms(delta: float) -> void:
	for slot in targets.size():
		var object := targets[slot]
		var yaw := wrapf(_angles[slot] + delta, -180, 180)
		if object.kind == "mesh":
			var part = editor._parts[object.index]
			part.rotation_y = yaw
			part.position = editor._clamp_mesh_part_position_to_lot(object.index, _positions[slot])
			editor._apply_mesh_part_transform_from_state(object.index, false)
		else:
			var anchor: Dictionary = editor._site_anchors_data[object.index]
			anchor.forward = editor._vector3_to_array(editor._forward_from_yaw(yaw), 0.001)
			anchor.position = editor._vector3_to_array(editor._clamp_site_anchor_position_to_lot(anchor, _positions[slot]), 0.01)

func _place(requested: Vector3) -> void:
	var delta := Vector2(requested.x, requested.z)
	if _frontage_bound:
		var outward: Vector3 = editor._frontage_edge_outward()
		if absf(outward.x) > 0.5: delta.x = 0
		else: delta.y = 0
	delta = editor._session.policy.clamp_lot_translation(_bounds, editor._lot_half_extents(), delta)
	for slot in targets.size():
		var object := targets[slot]
		match object.kind:
			"mesh":
				editor._parts[object.index].position = _positions[slot] + Vector3(delta.x, 0, delta.y)
				editor._apply_mesh_part_transform_from_state(object.index, false)
			"anchor":
				var anchor: Dictionary = editor._site_anchors_data[object.index]
				anchor.position = editor._vector3_to_array(_positions[slot] + Vector3(delta.x, 0, delta.y), 0.01)
			"surface":
				var vertices: Array = editor._site_surfaces_data[object.index].vertices
				for vertex in vertices.size():
					vertices[vertex][0] = _vertices[slot][vertex].x + delta.x
					vertices[vertex][1] = _vertices[slot][vertex].y + delta.y
	_refresh_geometry()
	_hint(0)

func _refresh_geometry() -> void:
	# Rebuild only affected overlay domains, never yards/anchors for a mesh-only gesture.
	if _has_meshes:
		editor._preview._build_selection_overlay()
		editor._sync_selected_mesh_part_controls()
	if _has_anchors:
		editor._update_site_anchor_preview()
		editor._update_site_anchor_controls()
	if _has_surfaces:
		editor._update_site_surface_preview()
		editor._update_site_surface_controls()

func _hint(angle: float) -> void:
	editor._view.hover_label.text = ("Rotate %.1f°" % angle if mode == "rotate" else "Place objects") + " · Click to confirm · Esc to cancel"

func confirm() -> void:
	if mode.is_empty(): return
	_committing = true
	var chosen := targets
	var label_text := "Rotate selection" if mode == "rotate" else "Place objects"
	mode = ""
	editor._session.document.apply(editor._session._adapter.capture(_pending), label_text)
	select(chosen)
	_committing = false
	_pending = {}
	editor._selection.context_changed()

func cancel() -> void:
	if mode.is_empty() or _committing: return
	mode = ""
	_committing = true
	editor._session._adapter.restore(editor._session.document.snapshot())
	select(_before_selection)
	_pending = {}
	_committing = false
	editor._selection.context_changed()

func document_changed() -> void:
	if not _committing and not mode.is_empty():
		# A replacement document owns the new projection; never restore stale gesture data.
		mode = ""
		_pending = {}
		editor._session._adapter.restore(editor._session.document.snapshot())

func _typed(values: Array) -> Array[Dictionary]:
	var result: Array[Dictionary] = []
	result.assign(values)
	return result
