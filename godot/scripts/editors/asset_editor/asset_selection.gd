# SPDX-License-Identifier: GPL-2.0-only

## Editor interaction controller: direct selection, hover/cycling and thresholded gestures.
## Geometry mutators and document history remain owned by the editor/session.
extends RefCounted

const Picker = preload("res://scripts/editors/asset_editor/asset_picker.gd")
const FILTERS := ["all", "mesh", "anchor", "surface"]
const DRAG_THRESHOLD := 6.0
var editor: Node
var picker: RefCounted
var filter := "all"
var hits: Array[Dictionary] = []
var hovered: Dictionary = {}
var applying_selection := false
var pressed := false
var dragging := false
var rotating := false
var _button := 0
var _start := Vector2.ZERO
var _target: Dictionary = {}
var _preview_start := Vector3.ZERO
var _preview_drag_hit := Vector3.ZERO
var _box := false
var _mouse := Vector2.INF
var _camera_transform := Transform3D.IDENTITY
var _projection := Projection()
var _pane := Rect2()
var _revision := -1
var _selected_surface := -2
var _effective := ""
var _cycle_keys: Array[String] = []
var _cycle_index := -1
var _cycle_mouse := Vector2.INF
var _enabled := false

func configure(owner: Node) -> void:
	editor = owner
	picker = Picker.new(owner)
	editor.get_window().focus_exited.connect(cancel)

func effective_filter() -> String:
	return filter

func set_filter(index: int) -> void:
	cancel()
	filter = FILTERS[index]
	if filter != "all":
		editor._preview.set_scale_reference_selected(false)
	_cycle_keys.clear()
	editor._view.selection_filter.select(index)
	context_changed()

func set_scale_reference_visible(shown: bool) -> void:
	cancel()
	editor._preview.set_scale_reference_visible(shown)
	context_changed()

func context_changed() -> void:
	_revision = -1

func _filter_label(value: String) -> String:
	return {"mesh": "meshes", "anchor": "anchors", "surface": "yards", "all": "all"}.get(value, value)

func _blocked() -> bool:
	return editor._cam_input._ui_has_modal_popup()

func update() -> void:
	if editor == null:
		return
	var view = editor._view
	var mouse: Vector2 = editor.get_viewport().get_mouse_position()
	var active: bool = editor._session.has_document and view._preview_view_rect.is_visible_in_tree() and not _blocked()
	if not active:
		if pressed:
			cancel()
		if _enabled:
			hovered = {}
			view.picking_overlay.clear()
			editor._preview.set_hover_outline(PackedVector3Array())
			view.hover_label.text = ""
		_enabled = false
		return
	_enabled = true
	if not pressed:
		refresh(mouse)

func refresh(mouse: Vector2, force: bool = false) -> void:
	var camera: Camera3D = editor.get_node("CameraNode")
	var rect: Rect2 = editor._view._preview_view_rect.get_global_rect()
	var effective := effective_filter()
	var revision: int = editor._preview.pick_revision
	var selected: int = editor._selected_site_surface_index
	var transform := camera.get_camera_transform()
	var projection := camera.get_camera_projection()
	if not force and mouse == _mouse and transform == _camera_transform and projection == _projection and rect == _pane and revision == _revision and selected == _selected_surface and effective == _effective:
		return
	var context_changed_flag := transform != _camera_transform or projection != _projection or rect != _pane or effective != _effective
	_mouse = mouse
	_camera_transform = transform
	_projection = projection
	_pane = rect
	_revision = revision
	_selected_surface = selected
	_effective = effective
	if rect.has_point(mouse):
		hits = picker.collect(mouse, camera, effective)
	else:
		hits.clear()
	var keys: Array[String] = []
	for hit in hits:
		if hit["kind"] != "vertex":
			keys.append(_key(hit))
	if keys != _cycle_keys or mouse.distance_to(_cycle_mouse) > 3.0 or context_changed_flag:
		_cycle_index = -1
		_cycle_mouse = mouse
	_cycle_keys = keys
	hovered = hits[0] if not hits.is_empty() and not hits[0].get("occluded", false) else {}
	if _cycle_index >= 0 and not keys.is_empty():
		for hit in hits:
			if _key(hit) == keys[_cycle_index]:
				hovered = hit
	_draw_hover(camera)

func _key(hit: Dictionary) -> String:
	return "%s:%d" % [hit["kind"], hit["index"]]

func cycle(mouse: Vector2) -> void:
	cancel()
	editor.get_viewport().gui_release_focus()
	refresh(mouse, true)
	if _cycle_keys.is_empty():
		return
	if _cycle_index < 0:
		_cycle_index = (0 if hovered.is_empty() else 1) % _cycle_keys.size()
	else:
		_cycle_index = (_cycle_index + 1) % _cycle_keys.size()
	for hit in hits:
		if _key(hit) == _cycle_keys[_cycle_index]:
			hovered = hit
			select_hit(hit, false)
			break
	_draw_hover(editor.get_node("CameraNode"))

func select_hit(hit: Dictionary, additive: bool) -> void:
	applying_selection = true
	var index: int = hit["index"]
	match hit["kind"]:
		"reference", "ghost":
			# Preview helpers are exclusive selections, never members of authored transform groups.
			editor._set_selected_mesh_parts([], -1)
			editor._set_selected_site_anchors([], -1)
			editor._set_selected_site_surface(-1)
		"mesh":
			if additive:
				editor._toggle_mesh_part_selection(index)
			elif not editor._selected_part_indices.has(index):
				editor._select_mesh_part(index)
			else:
				editor._set_selected_mesh_parts(editor._selected_part_indices, index)
		"anchor":
			if additive:
				editor._toggle_site_anchor_selection(index)
			elif not editor._selected_site_anchor_indices.has(index):
				editor._select_site_anchor(index)
			else:
				editor._set_selected_site_anchors(editor._selected_site_anchor_indices, index)
		"surface", "vertex":
			if additive:
				editor._set_selected_site_surface(-1 if editor._selected_site_surface_index == index else index, false)
			elif editor._selected_site_surface_index != index:
				editor._set_selected_site_surface(index)
	editor._preview.set_scale_reference_selected(hit["kind"] == "reference")
	applying_selection = false
	if not additive:
		if hit["kind"] not in ["ghost", "reference"] and not editor._view.inspector.visible:
			editor._view.toggle_inspector()
		match hit["kind"]:
			"mesh": editor._view.show_task("model")
			"anchor":
				editor._view.show_task("site")
				editor._view.show_site(1)
			"surface", "vertex":
				editor._view.show_task("site")
				editor._view.show_site(2)

func handle_input(event: InputEvent) -> bool:
	if editor == null or not editor._session.has_document:
		return false
	if _blocked():
		cancel()
		return false
	if event is InputEventKey and event.pressed and not event.echo:
		if event.keycode == KEY_ESCAPE and pressed:
			cancel()
			return true
		if editor._ui_captures_editor_text_input():
			return false
		if event.ctrl_pressed and event.keycode in [KEY_Z, KEY_Y]:
			cancel()
			if event.keycode == KEY_Y or event.shift_pressed:
				editor._session.redo()
			else:
				editor._session.undo()
			return true
		if event.keycode == KEY_DELETE:
			cancel()
			var removed: bool = editor._remove_selected_site_surface() if editor._selected_site_surface_index >= 0 else false
			if not editor._selected_site_anchor_indices.is_empty():
				removed = editor._remove_selected_site_anchor() or removed
			if not editor._selected_part_indices.is_empty():
				removed = editor._remove_selected_mesh_parts() or removed
			if removed:
				editor._session.capture_geometry("Delete selection")
			return removed
	if event is InputEventMouseButton and event.button_index in [MOUSE_BUTTON_LEFT, MOUSE_BUTTON_RIGHT]:
		if event.pressed:
			if not editor._view._preview_view_rect.get_global_rect().has_point(event.position):
				return false
			if event.alt_pressed and event.button_index == MOUSE_BUTTON_LEFT:
				cycle(event.position)
				return true
			return press(event.position, event.button_index, event.ctrl_pressed, event.shift_pressed)
		if pressed and event.button_index == _button:
			release(event.position)
			return true
	if event is InputEventMouseMotion and pressed:
		motion(event.position)
		return true
	return false

func press(mouse: Vector2, button: int, additive: bool = false, box: bool = false) -> bool:
	cancel()
	# Commit an active inspector edit before beginning a separate viewport gesture.
	editor.get_viewport().gui_release_focus()
	refresh(mouse, true)
	if button == MOUSE_BUTTON_RIGHT and effective_filter() in ["surface", "all"] and editor._try_open_site_surface_context_menu(mouse):
		return true
	_target = hovered.duplicate()
	if button == MOUSE_BUTTON_RIGHT and (_target.is_empty() or _target["kind"] not in ["mesh", "anchor"]):
		return false
	_start = mouse
	_button = button
	_box = box or _target.is_empty()
	if _box:
		editor._preview.set_scale_reference_selected(false)
		editor._begin_box_selection(mouse, additive)
	else:
		select_hit(_target, additive)
		if additive:
			return true
	pressed = true
	rotating = button == MOUSE_BUTTON_RIGHT
	return true

func motion(mouse: Vector2) -> void:
	if not pressed:
		return
	if _box:
		editor._update_mesh_part_box_selection(mouse)
		return
	if not dragging:
		if mouse.distance_to(_start) < DRAG_THRESHOLD:
			return
		if not _prepare_drag():
			return
		if not _preview_target():
			editor._session.document.begin_transaction("Rotate selection" if rotating else "Move selection")
		dragging = true
	if rotating:
		if _target["kind"] == "mesh":
			editor._rotate_mesh_part_from_mouse(mouse)
		else:
			editor._rotate_site_anchor_from_mouse(mouse)
	elif _target["kind"] == "vertex":
		editor._drag_site_surface_vertex_from_mouse(mouse)
	elif _preview_target():
		var hit = editor._project_mouse_to_horizontal_plane(mouse, editor._drag_plane_y)
		if hit != null:
			_set_preview_position(_preview_start + Vector3(hit.x - _preview_drag_hit.x, 0, hit.z - _preview_drag_hit.z))
	else:
		editor._drag_mesh_part_from_mouse(mouse)
		if editor._site_surface_drag_index >= 0:
			editor._drag_site_surface_from_mouse(mouse)
	# Keep the gesture target locked while the highlight and edit handles follow its geometry.
	hovered = _target.duplicate()
	var marker = editor._project_mouse_to_horizontal_plane(mouse, editor._drag_plane_y)
	if marker != null:
		hovered["position"] = marker
	_draw_hover(editor.get_node("CameraNode"))

func _prepare_drag() -> bool:
	editor._drag_plane_y = float(_target["position"].y)
	var hit = editor._project_mouse_to_horizontal_plane(_start, editor._drag_plane_y)
	if hit == null:
		return false
	if _preview_target():
		_preview_start = editor._preview.scale_reference_world_position() if _target["kind"] == "reference" else editor._preview.get_ghost_world_position()
		_preview_drag_hit = hit
		return true
	editor._mesh_part_drag_start_hit = Vector3(hit.x, 0, hit.z)
	editor._mesh_part_drag_start_positions.clear()
	for index in editor._mesh_part_drag_indices():
		editor._mesh_part_drag_start_positions.append(editor._parts[index].position)
	editor._site_anchor_drag_start_positions.clear()
	for index in editor._site_anchor_drag_indices():
		editor._site_anchor_drag_start_positions.append(editor._anchor_position(editor._site_anchors_data[index]))
	editor._site_surface_drag_index = editor._selected_site_surface_index
	editor._site_surface_drag_start_hit = Vector3(hit.x, 0, hit.z)
	if editor._site_surface_drag_index >= 0:
		editor._site_surface_drag_start_vertices = editor._site_surface_vertices(editor._site_surfaces_data[editor._site_surface_drag_index])
	editor._site_surface_vertex_drag_index = _target.get("vertex", -1)
	if rotating:
		if _target["kind"] == "mesh":
			editor._mesh_part_rotate_start_x = _start.x
			editor._mesh_part_rotate_start_yaw = editor._parts[_target["index"]].rotation_y
		else:
			editor._site_anchor_rotate_start_x = _start.x
			editor._site_anchor_rotate_start_yaw = editor._yaw_from_forward(editor._anchor_forward(editor._site_anchors_data[_target["index"]]))
	return true

func _preview_target() -> bool:
	return _target.get("kind") in ["reference", "ghost"]

func _set_preview_position(position: Vector3) -> void:
	if _target["kind"] == "reference":
		editor._preview.set_scale_reference_world_position(position)
	else:
		editor._preview.set_ghost_world_position(position)

func release(mouse: Vector2) -> void:
	if _box:
		applying_selection = true
		editor._finish_mesh_part_box_selection(mouse)
		applying_selection = false
	if dragging and not _preview_target():
		editor._session.capture_geometry("Transform selection")
		editor._session.document.commit_transaction()
	pressed = false
	dragging = false
	rotating = false
	_box = false
	_revision = -1

func cancel() -> void:
	if not pressed:
		return
	if dragging:
		if _preview_target():
			_set_preview_position(_preview_start)
		else:
			editor._session.capture_geometry("Transform selection")
			editor._session.document.cancel_transaction()
	editor._view._selection_rect_overlay.clear()
	pressed = false
	dragging = false
	rotating = false
	_box = false
	_revision = -1

func _draw_hover(camera: Camera3D) -> void:
	var view = editor._view
	var overlay = view.picking_overlay
	overlay.clear()
	var outline := PackedVector3Array()
	editor._preview.set_hover_outline(outline)
	if _effective == "all" and editor._preview.has_scale_reference():
		var centre: Vector3 = editor._preview.scale_reference_world_position()
		if Picker.in_depth_range(camera, centre):
			overlay.reference = camera.unproject_position(centre)
			overlay.reference_selected = editor._preview.scale_reference_selected
	if _effective in ["anchor", "all"]:
		for index in editor._site_anchors_data.size():
			var point: Vector3 = picker.anchor_position(index)
			if picker.point_visible(camera, point):
				overlay.anchors.append(camera.unproject_position(point))
	if _effective in ["surface", "all"] and editor._selected_site_surface_index >= 0:
		for point in picker.surface_points(editor._selected_site_surface_index):
			if picker.point_visible(camera, point):
				overlay.vertices.append(camera.unproject_position(point))
	view.hover_label.text = "%s · Click selects · Alt+click cycles overlaps" % _filter_label(_effective).capitalize()
	if editor._preview.scale_reference_selected:
		view.hover_label.text = "Scale reference · 1.8 m · Drag to move · Escape cancels · Preview only"
	if hovered.is_empty():
		return
	var index: int = hovered["index"]
	var title := "Comparison mesh"
	var points: Array = []
	match hovered["kind"]:
		"reference": title = "Scale reference · 1.8 m · Drag to move · Preview only"
		"mesh":
			title = "Mesh: " + editor._parts[index].name
			points = editor._preview.mesh_part_world_corners(index)
			for edge in [[0, 1], [0, 2], [0, 4], [1, 3], [1, 5], [2, 3], [2, 6], [3, 7], [4, 5], [4, 6], [5, 7], [6, 7]]:
				if points.size() == 8:
					outline.append(points[edge[0]])
					outline.append(points[edge[1]])
		"surface", "vertex":
			title = "Yard: " + editor._site_surface_display_label(index)
			points.assign(picker.surface_points(index))
			for edge in points.size():
				outline.append(points[edge] + Vector3(0, 0.01, 0))
				outline.append(points[(edge + 1) % points.size()] + Vector3(0, 0.01, 0))
			if hovered["kind"] == "vertex":
				title += " · vertex %d" % (hovered["vertex"] + 1)
		"anchor": title = "Anchor: " + editor._site_anchor_display_label(index)
	if hovered["kind"] != "reference" and not hovered.get("occluded", false):
		overlay.marker = camera.unproject_position(hovered["position"])
		overlay.marker_radius = Picker.ANCHOR_RADIUS if hovered["kind"] == "anchor" else Picker.VERTEX_RADIUS
	view.hover_label.text = title + (" · %d overlaps · Alt+click: next" % _cycle_keys.size() if _cycle_keys.size() > 1 else "")
	editor._preview.set_hover_outline(outline)
