# SPDX-License-Identifier: GPL-2.0-only

## Generated real-editor fixtures for 3D picking, overlap/filter UX, gestures and cache invalidation.
## Isolated user profile via run.sh; optional captures/benchmarks never depend on creator assets.
extends SceneTree

const EditorScene = preload("res://scenes/AssetEditor.tscn")
var failures := 0
var editor: Node
var camera: Camera3D
var selection: RefCounted
var fixture: Dictionary
var box_path := ""
var narrow_path := ""
var gap_path := ""

func _initialize() -> void:
	call_deferred("_run")

func expect(value: bool, message: String) -> void:
	if not value:
		failures += 1
		push_error(message)

func _model(path: String, gap: bool = false, width: float = 6.0) -> void:
	var scene := Node3D.new()
	var group := Node3D.new()
	scene.add_child(group)
	for index in (2 if gap else 1):
		var mesh := MeshInstance3D.new()
		mesh.mesh = BoxMesh.new()
		mesh.mesh.size = Vector3(2.0 if gap else width, 20, 6)
		mesh.position = Vector3((-3.0 if index == 0 else 3.0) if gap else 0.0, 10, 0)
		group.add_child(mesh)
	var document := GLTFDocument.new()
	var state := GLTFState.new()
	expect(document.append_from_scene(scene, state) == OK, "generate geometry fixture")
	expect(document.write_to_filesystem(state, path) == OK, "write geometry fixture")
	scene.free()

func _reset() -> void:
	selection.cancel()
	editor._session.document.reset(fixture)
	editor._set_selected_mesh_parts([], -1)
	editor._set_selected_site_anchors([], -1)
	editor._set_selected_site_surface(-1)
	editor._view.show_task("model")
	selection.set_filter(0)
	editor._view.selection_filter.select(0)
	_top_camera()

func _top_camera(height: float = 60.0) -> void:
	camera.look_at_from_position(Vector3(0, height, 0), Vector3.ZERO, Vector3.FORWARD)
	camera.h_offset = 0
	camera.v_offset = 0

func _screen(point: Vector3) -> Vector2:
	return camera.unproject_position(point)

func _has(hits: Array, kind: String, index: int = -1) -> bool:
	for hit: Dictionary in hits:
		if hit["kind"] == kind and (index < 0 or hit["index"] == index):
			return true
	return false

func _pick(point: Vector3, filter: String) -> Array[Dictionary]:
	return selection.picker.collect(_screen(point), camera, filter)

func _run() -> void:
	box_path = ProjectSettings.globalize_path("user://pick-box.glb")
	narrow_path = ProjectSettings.globalize_path("user://pick-narrow.glb")
	gap_path = ProjectSettings.globalize_path("user://pick-gap.glb")
	_model(box_path)
	_model(narrow_path, false, 0.5)
	_model(gap_path, true)
	editor = EditorScene.instantiate()
	root.add_child(editor)
	selection = editor._selection
	camera = editor.get_node("CameraNode")
	editor._session.create_asset({"type": "residential", "name": "Picking fixture", "pack": {"pack_id": "selection-test"}})
	editor._session.set_field("lot_width_cells", 10)
	editor._session.set_field("lot_depth_cells", 10)
	editor._on_glb_file_selected(box_path)
	editor._on_glb_file_selected(box_path)
	fixture = editor._session.document.snapshot()
	fixture["params"]["mesh_parts"][0]["position"] = [0.0, 0.0, 0.0]
	fixture["params"]["mesh_parts"][1]["position"] = [0.0, -5.0, 0.0]
	fixture["params"]["anchors"] = [{"anchor_type": "entrance", "name": "Door", "position": [0.0, 0.0, 0.0], "forward": [0.0, 0.0, 1.0]}]
	fixture["params"]["site_surfaces"] = [{"name": "Yard", "material": "asphalt", "y_m": 0.01, "vertices": [[-8.0, -8.0], [8.0, -8.0], [8.0, 8.0], [-8.0, 8.0]]}]
	for frame in 12:
		await process_frame
	editor._cam_input.set_process(false)
	# Fixture cameras are explicit; physical wheel/orbit events must not move a capture mid-frame.
	editor._cam_input.set_process_input(false)
	_reset()
	_test_guide_geometry()
	_test_filters_and_overlap()
	_test_guide_occlusion()
	await _test_list_selection()
	await _test_access_point_input()
	_test_geometry()
	await _test_visible_lod_selection()
	await _test_handles()
	_test_gestures()
	_test_mixed_and_ghost_gestures()
	await _test_scale_reference()
	_test_invalidations()
	await _captures()
	_benchmark()
	editor.free()
	await process_frame
	if failures == 0:
		print("PASS asset_selection_test")
	quit(0 if failures == 0 else 1)

func _test_list_selection() -> void:
	_reset()
	editor._set_selected_site_anchors([0], 0)
	editor._set_selected_site_surface(0, false)
	editor._view.show_task("model")
	for frame in 8:
		await process_frame
	var list: ItemList = editor._view._mesh_part_list
	_click(list.global_position + list.get_item_rect(1).get_center())
	expect(editor._selected_part_indices == [1] and editor._selected_part_index == 1, "mesh list click selects the clicked part and opens its properties")
	expect(editor._view.part_properties.visible and editor._selected_site_anchor_indices.is_empty() and editor._selected_site_surface_index == -1, "plain list click clears unrelated authored selections")
	var modifier := InputEventKey.new()
	modifier.keycode = KEY_CTRL
	modifier.pressed = true
	Input.parse_input_event(modifier)
	_click(list.global_position + list.get_item_rect(0).get_center(), true)
	expect(editor._selected_part_indices == [0, 1], "Ctrl+click in the mesh list preserves multiple selection")
	_click(list.global_position + list.get_item_rect(1).get_center(), true)
	expect(editor._selected_part_indices == [0], "Ctrl+click can deselect a mesh list item")
	modifier = InputEventKey.new()
	modifier.keycode = KEY_CTRL
	Input.parse_input_event(modifier)
	editor._view.show_task("site")
	editor._view.show_site(1)
	for frame in 8:
		await process_frame
	list = editor._view._site_anchor_list
	_click(list.global_position + list.get_item_rect(0).get_center())
	expect(editor._selected_site_anchor_index == 0 and editor._selected_part_indices.is_empty(), "anchor list click opens access properties without retaining mesh selection")
	_reset()

func _test_guide_geometry() -> void:
	var preview = editor._preview
	var state: Dictionary = editor._session.document.snapshot()
	var builds: int = preview.pick_build_count
	var concave := [Vector3(0, 2, 0), Vector3(4, 2, 0), Vector3(4, 2, 1), Vector3(1, 2, 1), Vector3(1, 2, 4), Vector3(0, 2, 4)]
	for winding in 2:
		var triangles := PackedVector3Array()
		preview._append_polygon_triangles(triangles, concave)
		expect(triangles.size() == 12, "native triangulator fills a six-vertex concave yard in either winding")
		var area := 0.0
		for index in range(0, triangles.size(), 3):
			var normal := (triangles[index + 1] - triangles[index]).cross(triangles[index + 2] - triangles[index])
			expect(normal.y > 0.0, "yard fill faces upward")
			area += normal.length() * 0.5
		expect(is_equal_approx(area, 7.0), "concave yard does not fall back to overlapping fan triangles")
		concave.reverse()
	var invalid := [Vector3.ZERO, Vector3(4, 0, 4), Vector3(0, 0, 4), Vector3(4, 0, 0)]
	expect(preview._polygon_triangles(invalid).is_empty(), "self-intersecting yards are rejected before native triangulation")
	for cells in [1, 2, 12]:
		preview.set_lot_size(cells, cells)
		for forward in [Vector3.FORWARD, Vector3.BACK, Vector3.LEFT, Vector3.RIGHT]:
			preview.set_frontage_forward(forward)
			var bounds: AABB = preview._frontage_arrow.mesh.get_aabb()
			var outward := -INF
			var inward := INF
			for index in 8:
				var extent := bounds.get_endpoint(index).dot(forward)
				outward = maxf(outward, extent)
				inward = minf(inward, extent)
			var edge: float = cells * preview.CELL_M * 0.5
			expect(inward >= edge - 0.2 and outward > edge + 2.4 and outward < edge + 3.3, "compact frontage glyphs face outward on every lot edge and size")
			expect(preview._frontage_label.position.dot(forward) > outward, "frontage label stays beyond the glyph tips")
			expect(not preview._frontage_arrow.mesh.surface_get_material(0).no_depth_test, "frontage respects mesh depth")
			expect(not preview._frontage_label.no_depth_test, "frontage text respects mesh depth")
	for forward in [Vector3.FORWARD, Vector3.BACK, Vector3.LEFT, Vector3.RIGHT]:
		var mesh := ImmediateMesh.new()
		mesh.surface_begin(Mesh.PRIMITIVE_TRIANGLES)
		preview._draw_direction_arrow(mesh, Vector3(0, 2, 0), forward, 2.0, 0.8, Color.WHITE)
		mesh.surface_end()
		var bounds := mesh.get_aabb()
		# ImmediateMesh gives flat bounds a 0.00001 m minimum thickness.
		expect(is_equal_approx(bounds.position.y, 2.0) and bounds.size.y <= 0.00002, "arrow styling preserves authored guide height: " + str(bounds))
		var length_m: float = bounds.size.x if absf(forward.x) > 0 else bounds.size.z
		expect(is_equal_approx(length_m, 2.1), "shared arrow silhouette has bounded keyline geometry")
	expect(preview.pick_build_count == builds and editor._session.document.snapshot() == state, "guide styling never rebuilds imported geometry or changes the asset")
	preview.set_lot_size(10, 10)
	preview.set_frontage_forward(editor._frontage_fwd)

func _test_filters_and_overlap() -> void:
	expect(selection.effective_filter() == "all", "all visible object kinds are selectable by default, including in Model")
	var hits := _pick(Vector3.ZERO, "all")
	expect(hits.size() == 4 and hits[0]["kind"] == "mesh" and hits[0]["index"] == 0, "opaque mesh takes priority over the hidden anchor and yard")
	selection.set_filter(1)
	selection.refresh(_screen(Vector3.ZERO), true)
	expect(selection.hovered.get("kind") == "mesh", "Model hover cannot be stolen by the yard")
	expect(editor._view.hover_label.text.contains("Mesh:") and editor._preview._hover_outline.mesh != null, "hover identifies and outlines target")
	editor._view.show_task("site")
	editor._view.show_site(1)
	expect(selection.effective_filter() == "mesh", "changing inspector sections does not change an explicit filter")
	selection.set_filter(2)
	selection.refresh(_screen(Vector3.ZERO), true)
	expect(selection.hovered.is_empty(), "an anchor filter does not make hidden anchors clickable")
	selection.cycle(_screen(Vector3.ZERO))
	expect(selection.hovered.get("kind") == "anchor", "Alt+click deliberately reaches the hidden filtered anchor")
	editor._view.show_site(2)
	selection.set_filter(3)
	selection.refresh(_screen(Vector3.ZERO), true)
	expect(selection.hovered.is_empty(), "a yard filter does not make a hidden yard clickable")
	selection.set_filter(0)
	editor._view.show_task("model")
	expect(selection.effective_filter() == "all", "All persists across inspector changes")
	var mouse := _screen(Vector3.ZERO)
	selection.refresh(mouse, true)
	var visited := {}
	for count in 4:
		selection.cycle(mouse)
		visited[selection._key(selection.hovered)] = true
	expect(visited.size() == 4, "Alt+click reaches every overlapping object, including occluded ones")
	selection.refresh(mouse, true)
	expect(selection.hovered["kind"] == "mesh" and selection.hovered["index"] == 0, "cycle wraps deterministically even when the inspector follows the selection")
	selection.select_hit({"kind": "mesh", "index": 0}, false)
	selection.select_hit({"kind": "anchor", "index": 0}, true)
	selection.select_hit({"kind": "surface", "index": 0}, true)
	expect(editor._selected_part_indices.has(0) and editor._selected_site_anchor_indices.has(0) and editor._selected_site_surface_index == 0, "All supports mixed mesh/anchor/yard selection")
	_reset()
	selection.set_filter(1)
	editor._begin_box_selection(_screen(Vector3(-10, 0, -10)), false)
	editor._finish_mesh_part_box_selection(_screen(Vector3(10, 0, 10)))
	expect(not editor._selected_part_indices.is_empty() and editor._selected_site_anchor_indices.is_empty() and editor._selected_site_surface_index == -1, "box selection respects the explicit filter")

func _test_guide_occlusion() -> void:
	_reset()
	var preview = editor._preview
	var point: Vector3 = selection.picker.anchor_position(0)
	selection.refresh(_screen(point), true)
	expect(editor._view.picking_overlay.anchors.is_empty(), "hidden anchor rings do not draw through the building")
	expect(not selection.picker.point_visible(camera, point), "solid imported geometry hides handle centres")
	var queries: int = selection.picker.visibility_queries
	selection.refresh(_screen(point) + Vector2(2, 0), true)
	expect(selection.picker.visibility_queries == queries, "pointer-only changes reuse handle visibility")
	preview.set_scale_reference_world_position(Vector3(30, 0.9, 30))
	selection.refresh(_screen(point) + Vector2(3, 0), true)
	expect(selection.picker.visibility_queries == queries, "moving preview helpers does not recast fixed handle visibility")
	for node in [preview._lot_overlay, preview._site_anchor_overlay, preview._site_surface_overlay, preview._hover_outline]:
		expect(not node.mesh.surface_get_material(0).no_depth_test, "lot, site and hover geometry use depth testing")
	for label in [preview.site_label("anchor", 0), preview.site_label("surface", 0)]:
		expect(not label.no_depth_test, "site text uses depth testing")
	preview.set_mesh_part_transform(0, Vector3(20, 0, 0), 0.0, 1.0)
	preview.set_mesh_part_transform(1, Vector3(20, -5, 0), 0.0, 1.0)
	selection.refresh(_screen(point), true)
	expect(selection.picker.point_visible(camera, point) and editor._view.picking_overlay.anchors.size() == 1, "moving occluders reveals the handle without stale visibility")
	expect(selection.hovered.get("kind") == "anchor", "visible anchor remains directly selectable above its yard")
	# Occlusion uses triangles, not a model's enclosing box: the gap must remain visible.
	preview.set_mesh_part_transform(0, Vector3.ZERO, 0.0, 1.0)
	expect(preview.set_mesh_part_lod(0, gap_path), "load the split-mesh visibility fixture")
	expect(selection.picker.point_visible(camera, point), "a real opening in a mesh does not hide an anchor")
	expect(preview.set_mesh_part_lod(0, box_path), "restore the solid visibility fixture")
	expect(not selection.picker.point_visible(camera, point), "LOD swaps invalidate occlusion without rebuilding triangles")
	_reset()

func _test_access_point_input() -> void:
	var data: Dictionary = fixture.duplicate(true)
	data["params"]["lot_width_cells"] = 2
	data["params"]["lot_depth_cells"] = 2
	data["params"]["frontage_forward"] = [0.0, 0.0, -1.0]
	data["params"]["mesh_parts"].resize(1)
	data["sources"].resize(1)
	data["params"]["mesh_parts"][0]["position"] = [2.0, 0.0, 3.0]
	data["params"]["anchors"] = [
		{"anchor_type": "entrance", "name": "main", "position": [-2.0, 0.0, -1.0], "forward": [0.0, 0.0, -1.0]},
		{"anchor_type": "parking", "position": [-5.0, 0.0, -7.0], "forward": [-1.0, 0.0, 0.0], "width_m": 2.5, "length_m": 5.0},
		{"anchor_type": "parking", "position": [-5.0, 0.0, -1.0], "forward": [-1.0, 0.0, 0.0], "width_m": 2.5, "length_m": 5.0},
		{"anchor_type": "parking", "position": [-5.0, 0.0, -4.0], "forward": [-1.0, 0.0, 0.0], "width_m": 2.5, "length_m": 5.0},
		{"anchor_type": "driveway", "position": [-2.0, 0.0, -10.0], "forward": [0.0, 0.0, 1.0], "width_m": 3.0},
		{"anchor_type": "loading_bay", "position": [1.0, 0.0, -6.5], "forward": [1.0, 0.0, 0.0], "width_m": 3.5, "length_m": 8.0},
	]
	data["params"]["site_surfaces"][0]["vertices"] = [[-10.0, -10.0], [10.0, -10.0], [10.0, 1.0], [-10.0, 1.0]]
	editor._session.document.reset(data)
	editor._view.show_task("model")
	selection.set_filter(0)
	for frame in 8:
		await process_frame
	camera.look_at_from_position(Vector3(-24, 30, -32), Vector3(0, 0, -2))
	editor._cam_input._update_preview_offset()
	for task in ["model", "overview", "site"]:
		for index in range(1, 6):
			editor._view.show_task(task)
			var anchor: Dictionary = editor._site_anchors_data[index]
			var length: float = anchor.get("length_m", 4.2)
			var point: Vector3 = selection.picker.anchor_position(index) + editor._anchor_forward(anchor) * length * 0.5
			var mouse := _screen(point)
			expect(editor._view._preview_view_rect.get_global_rect().has_point(mouse), "access-point fixture stays inside the interactive pane")
			_click(mouse)
			expect(editor._selected_site_anchor_index == index, "plain viewport click selects parking/loading/driveway from %s, anchor %d" % [task, index])
			expect(editor._view.task_picker.selected == 2 and editor._view.site_picker.selected == 1, "anchor clicks open their Access points settings")
			expect(selection.effective_filter() == "all", "clicking or changing sections never locks out another object kind")
	# The mesh and bare yard remain selectable immediately after clicking an anchor.
	selection.press(_screen(Vector3(2, 20, 3)), MOUSE_BUTTON_LEFT)
	selection.release(_screen(Vector3(2, 20, 3)))
	expect(editor._selected_part_index == 0 and editor._view.task_picker.selected == 1, "mesh click opens Model without a toolbar action")
	selection.press(_screen(Vector3(0, 0.1, -3)), MOUSE_BUTTON_LEFT)
	selection.release(_screen(Vector3(0, 0.1, -3)))
	expect(editor._selected_site_surface_index == 0 and editor._view.task_picker.selected == 2 and editor._view.site_picker.selected == 2, "bare yard click opens Surfaces without a toolbar action")
	for scale in [1.0, 2.0]:
		root.content_scale_factor = scale
		root.size = Vector2i(Vector2(1920, 1080) * scale)
		for index in 6:
			editor._view.show_task("model")
			for frame in 8:
				await process_frame
			camera.look_at_from_position(Vector3(-24, 30, -32), Vector3(0, 0, -2))
			editor._cam_input._update_preview_offset()
			var label: Label3D = editor._preview.site_label("anchor", index)
			var point := label.global_position + camera.global_basis.x * label.get_aabb().size.x * 0.3
			var mouse := _screen(point)
			expect(selection.picker._label_contains("anchor", index, mouse, camera), "anchor text uses its visible billboard bounds at %.1fx UI scale" % scale)
			_click(mouse)
			expect(editor._selected_site_anchor_index == index and editor._view.site_picker.selected == 1, "clicking entrance/parking/loading/driveway text selects it and opens its settings")
			if scale == 1.0 and index == 1:
				await _capture_access_selection()
	root.content_scale_factor = 1.0
	root.size = Vector2i(1920, 1080)
	for frame in 8:
		await process_frame
	editor._view.toggle_inspector()
	selection.select_hit({"kind": "anchor", "index": 2}, false)
	expect(editor._view.inspector.visible and not editor._layout.inspector_collapsed, "click selection reopens its settings when the inspector was collapsed")
	_reset()

func _capture_access_selection() -> void:
	for argument in OS.get_cmdline_user_args():
		if not argument.begins_with("--capture-selection="):
			continue
		var directory := argument.trim_prefix("--capture-selection=")
		DirAccess.make_dir_recursive_absolute(directory)
		var camera_transform := camera.transform
		var hour: float = editor._view._preview_panel._hour.value
		for mode in ["dark", "light"]:
			camera.transform = camera_transform
			editor.set_ui_theme_mode(mode)
			for frame in 8:
				await process_frame
			editor.set_process(false)
			var label: Label3D = editor._preview.site_label("anchor", 1)
			selection.refresh(_screen(label.global_position), true)
			await RenderingServer.frame_post_draw
			expect(root.get_texture().get_image().save_png(directory.path_join(mode + "-access-click.png")) == OK, "direct access-point selection capture")
			camera.look_at_from_position(Vector3(-15, 18, -20), Vector3(0, 0, -4))
			editor._cam_input._update_preview_offset()
			selection.refresh(_screen(label.global_position), true)
			await RenderingServer.frame_post_draw
			expect(root.get_texture().get_image().save_png(directory.path_join(mode + "-guide-detail.png")) == OK, "frontage and access-guide detail capture")
			editor._view._preview_panel._hour.value = 22.0
			for frame in 8:
				await process_frame
			await RenderingServer.frame_post_draw
			expect(root.get_texture().get_image().save_png(directory.path_join(mode + "-guide-night.png")) == OK, "night guide readability capture")
			editor._view._preview_panel._hour.value = hour
			editor.set_process(true)
		camera.transform = camera_transform

func _click(mouse: Vector2, additive: bool = false) -> void:
	for down in [true, false]:
		_mouse_button(mouse, down, additive)

func _mouse_button(mouse: Vector2, down: bool, additive: bool = false) -> void:
	var event := InputEventMouseButton.new()
	event.ctrl_pressed = additive
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

func _test_visible_lod_selection() -> void:
	_reset()
	var saved_projection := camera.projection
	var saved_size := camera.size
	var saved_transform := camera.transform
	var data: Dictionary = fixture.duplicate(true)
	data["params"]["mesh_parts"][0]["lods"][0]["distance_max_m"] = 35.0
	data["params"]["mesh_parts"][0]["lods"].append({"file": narrow_path.get_file(), "distance_min_m": 35.0, "distance_max_m": null})
	data["sources"][0].append(narrow_path)
	editor._session.document.reset(data)
	var part = editor._parts[0]
	camera.set_orthogonal(200.0, camera.near, camera.far)
	editor._lod_preview.update(camera)
	var state = editor._lod_preview.states[part]
	var panel = editor._view._preview_panel
	expect(state.active == 1 and state.forced == -1, "distant fixture renders LOD1 automatically before selection")
	var before: Dictionary = editor._session.document.snapshot()
	var mouse := _screen(Vector3(0, 20, 0))
	selection.press(mouse, MOUSE_BUTTON_LEFT)
	selection.release(mouse)
	expect(editor._selected_part_index == 0 and panel._lod_list.is_selected(2), "viewport click highlights the visible LOD1, not LOD0 or Automatic")
	expect(panel._replace.text == "Replace LOD1…" and state.forced == -1, "clicking the mesh targets the visible source without freezing automatic LODs")
	camera.size = 4.0
	editor._lod_preview.update(camera)
	expect(state.active == 0 and panel._lod_list.is_selected(1) and state.forced == -1, "zooming back in keeps Automatic and highlights LOD0")
	# A real click on the already highlighted row must still allow explicit forcing.
	for frame in 8:
		await process_frame
	_click(panel._lod_list.global_position + panel._lod_list.get_item_rect(1).get_center())
	expect(state.forced == 0, "clicking the automatically highlighted row explicitly forces that tier")
	camera.size = 200.0
	editor._lod_preview.update(camera)
	expect(state.forced == -1 and state.active == 1 and panel._lod_list.is_selected(2), "zooming out ends explicit inspection and highlights the rendered distant tier")
	selection.select_hit({"kind": "mesh", "index": 1}, false)
	expect(panel._lod_list.is_selected(1) and panel._replace.text == "Replace LOD0…", "switching parts cannot reuse the previous part's visible tier")
	selection.select_hit({"kind": "mesh", "index": 0}, false)
	expect(panel._lod_list.is_selected(2) and state.forced == -1, "reselecting the distant part restores its actual visible LOD")
	# Reproduce the reported close-up with a perspective camera and a previously chosen coarse tier.
	camera.projection = Camera3D.PROJECTION_PERSPECTIVE
	_top_camera(500.0)
	panel._lod_list.item_selected.emit(2)
	expect(state.active == 1 and state.forced == 1, "choosing a tier after a camera change starts inspection at that current view")
	selection.select_hit({"kind": "mesh", "index": 1}, false)
	_top_camera(25.0)
	editor._lod_preview.update(camera)
	expect(state.forced == -1, "navigation also releases inspection on unselected parts")
	mouse = _screen(Vector3(0, 20, 0))
	selection.press(mouse, MOUSE_BUTTON_LEFT)
	selection.release(mouse)
	expect(editor._selected_part_index == 0 and state.active == 0 and state.forced == -1 and panel._lod_list.is_selected(1), "close perspective mesh click selects LOD0 after coarse-tier inspection, without clicking Automatic")
	expect(editor._preview.mesh_part_lod_path(0) == box_path and panel._replace.text == "Replace LOD0…", "visible fine mesh, picked part and replacement target agree")
	expect(editor._session.document.snapshot() == before and not editor._session.document.is_dirty(), "LOD inspection never edits or saves the asset")
	camera.projection = saved_projection
	camera.size = saved_size
	camera.transform = saved_transform
	_reset()

func _test_geometry() -> void:
	_reset()
	camera.look_at_from_position(Vector3(30, 40, 30), Vector3(0, 10, 0))
	expect(_has(_pick(Vector3(0, 20, 0), "mesh"), "mesh", 0), "oblique roof clicks hit 3D geometry, not the projected ground footprint")
	_top_camera()
	var data: Dictionary = fixture.duplicate(true)
	data["sources"] = [[gap_path]]
	data["params"]["mesh_parts"].resize(1)
	editor._session.document.reset(data)
	expect(_pick(Vector3.ZERO, "mesh").is_empty(), "empty space inside mesh bounds is not a triangle hit")
	expect(_has(_pick(Vector3(-3, 10, 0), "mesh"), "mesh", 0), "nested imported mesh transforms are honored")
	editor._parts[0].position = Vector3(10, 7, 2)
	editor._parts[0].rotation_y = 90
	editor._parts[0].scale = 1.5
	editor._apply_mesh_part_transform_from_state(0)
	var transform: Transform3D = editor._preview.mesh_part_transform(0)
	expect(_has(_pick(transform * Vector3(-3, 10, 0), "mesh"), "mesh", 0), "translated, elevated, rotated and scaled mesh picks correctly")
	_reset()
	editor._parts[1].position.x = 30
	editor._apply_mesh_part_transform_from_state(1)
	var builds: int = editor._preview.pick_build_count
	var paths: Array[String] = [box_path, narrow_path]
	editor._preview.prepare_mesh_part_lods(0, paths)
	expect(editor._preview.pick_build_count == builds + 1, "new LOD builds its triangle cache once")
	editor._preview.set_mesh_part_lod(0, narrow_path)
	expect(not _has(_pick(Vector3(2, 20, 0), "mesh"), "mesh", 0), "picking follows visible LOD, not hidden LOD0")
	editor._preview.set_mesh_part_lod(0, box_path)
	expect(_has(_pick(Vector3(2, 20, 0), "mesh"), "mesh", 0), "return to cached LOD0 restores exact picking")
	expect(editor._preview.pick_build_count == builds + 1, "LOD swaps and picks never rebuild triangle caches")
	data = fixture.duplicate(true)
	data["params"]["site_surfaces"][0]["y_m"] = 25.0
	data["params"]["anchors"][0]["position"] = [0.0, 26.0, 0.0]
	editor._session.document.reset(data)
	var hits := _pick(Vector3.ZERO, "all")
	expect(hits[0]["kind"] == "anchor" and hits[1]["kind"] == "surface", "elevated site geometry participates in actual depth order")
	var rendered: Array = editor._preview._site_surface_vertices(data["params"]["site_surfaces"][0], editor._preview.SITE_SURFACE_FILL_Y)
	expect(is_equal_approx(rendered[0].y, selection.picker.surface_height(0)), "yard rendering and picking agree on authored elevation")
	camera.look_at_from_position(Vector3(0, 60, 0), Vector3(0, 100, 0), Vector3.FORWARD)
	expect(_pick(Vector3.ZERO, "all").is_empty(), "objects behind the camera are not pickable")

func _test_handles() -> void:
	_reset()
	for height in [35.0, 200.0]:
		_top_camera(height)
		for scale in [1.0, 2.0]:
			root.content_scale_factor = scale
			root.size = Vector2i(Vector2(1920, 1080) * scale)
			for frame in 8:
				await process_frame
			_top_camera(height)
			# Isolate the circular handle from the separately clickable text beside it.
			editor._preview.site_label("anchor", 0).hide()
			var center := _screen(selection.picker.anchor_position(0))
			expect(_has(selection.picker.collect(center + Vector2(11, 0), camera, "anchor"), "anchor"), "anchor handle stays 12 logical pixels at any zoom/UI scale")
			expect(not _has(selection.picker.collect(center + Vector2(13, 0), camera, "anchor"), "anchor"), "anchor handle rejects outside its screen radius")
			var vertex := _screen(selection.picker.surface_points(0)[0])
			expect(not _has(selection.picker.collect(vertex, camera, "surface"), "vertex"), "unselected yard has no editable vertices")
			editor._set_selected_site_surface(0)
			expect(_has(selection.picker.collect(vertex + Vector2(7, 0), camera, "surface"), "vertex"), "selected yard uses an eight-pixel vertex handle")
			expect(not _has(selection.picker.collect(vertex + Vector2(9, 0), camera, "surface"), "vertex"), "vertex handle rejects outside its screen radius")
			expect(not _has(selection.picker.collect(vertex, camera, "mesh"), "vertex"), "mesh filter never edits yard vertices")
			editor._set_selected_site_surface(-1)
	root.content_scale_factor = 1.0
	root.size = Vector2i(1920, 1080)
	for frame in 8:
		await process_frame
	_reset()
	editor._set_selected_site_surface(0)
	var points: PackedVector3Array = selection.picker.surface_points(0)
	var edge_mouse := _screen((points[0] + points[1]) * 0.5)
	expect(not selection.picker.surface_edge(edge_mouse, camera).is_empty(), "selected yard edge supports a pixel-sized add-vertex target")
	editor._set_selected_site_surface(-1)
	expect(selection.picker.surface_edge(edge_mouse, camera).is_empty(), "unselected yards cannot steal edge-edit context clicks")

func _test_gestures() -> void:
	_reset()
	var mouse := _screen(Vector3(2, 20, 0))
	var original: Dictionary = editor._session.document.snapshot()
	selection.press(mouse, MOUSE_BUTTON_LEFT)
	expect(editor._selected_part_index == 0 and not selection.dragging, "mouse-down selects without starting a transform")
	selection.motion(mouse + Vector2(3, 2))
	selection.release(mouse + Vector2(3, 2))
	expect(editor._session.document.snapshot() == original and not editor._session.document.is_dirty() and not editor._session.document.can_undo(), "click jitter never moves geometry, dirties the draft or adds undo")
	selection.press(mouse, MOUSE_BUTTON_LEFT)
	selection.motion(mouse + Vector2(30, 0))
	expect(selection.dragging and editor._parts[0].position != Vector3.ZERO, "crossing the threshold moves the picked mesh")
	selection.release(mouse + Vector2(30, 0))
	var moved: Dictionary = editor._session.document.snapshot()
	editor._session.undo()
	expect(editor._session.document.snapshot() == original and editor._parts[0].position == Vector3.ZERO, "one undo restores the whole drag")
	editor._session.redo()
	expect(editor._session.document.snapshot() == moved, "redo restores movement")
	_reset()
	selection.press(mouse, MOUSE_BUTTON_LEFT)
	selection.motion(mouse + Vector2(30, 0))
	selection.cancel()
	expect(editor._session.document.snapshot() == fixture and editor._parts[0].position == Vector3.ZERO and not editor._session.document.can_undo(), "Escape/focus loss restores live geometry without an undo command")
	selection.press(mouse, MOUSE_BUTTON_RIGHT)
	selection.motion(mouse + Vector2(2, 0))
	selection.release(mouse + Vector2(2, 0))
	expect(not editor._session.document.is_dirty(), "right-click does not rotate without a drag")
	selection.press(mouse, MOUSE_BUTTON_RIGHT)
	selection.motion(mouse + Vector2(40, 0))
	selection.release(mouse + Vector2(40, 0))
	expect(editor._parts[0].rotation_y != 0, "right-drag retains mesh rotation")
	editor._session.undo()
	expect(editor._parts[0].rotation_y == 0, "rotation is undoable")
	_reset()
	selection.set_filter(3)
	editor._set_selected_site_surface(0)
	var vertex := _screen(selection.picker.surface_points(0)[0])
	selection.press(vertex + Vector2(3, 0), MOUSE_BUTTON_LEFT)
	selection.motion(vertex + Vector2(23, 0))
	selection.release(vertex + Vector2(23, 0))
	expect(editor._session.params["site_surfaces"][0]["vertices"] != fixture["params"]["site_surfaces"][0]["vertices"], "only selected yard vertex moves through its handle")
	expect(editor._parts[0].position == Vector3.ZERO, "yard vertex edit cannot move an overlapping mesh")
	editor._session.undo()
	expect(editor._session.document.snapshot() == fixture, "yard vertex drag is one reversible command")
	var popup := AcceptDialog.new()
	editor.add_child(popup)
	popup.popup_centered(Vector2i(400, 200))
	var event := InputEventMouseButton.new()
	event.position = mouse
	event.button_index = MOUSE_BUTTON_LEFT
	event.pressed = true
	expect(not selection.handle_input(event) and not selection.pressed, "modal UI cannot click through into selection")
	popup.hide()
	popup.queue_free()
	_reset()
	editor._view.show_task("overview")
	var name_edit: LineEdit = editor._view.fields["display_name"]
	name_edit.grab_focus()
	name_edit.text = "Renamed before drag"
	name_edit.text_changed.emit(name_edit.text)
	selection.set_filter(1)
	selection.press(mouse, MOUSE_BUTTON_LEFT)
	selection.motion(mouse + Vector2(30, 0))
	selection.cancel()
	expect(editor._session.params["display_name"] == "Renamed before drag" and editor._parts[0].position == Vector3.ZERO, "cancelling a drag never rolls back a preceding inspector edit")
	editor._session.undo()
	expect(editor._session.document.snapshot() == fixture, "preceding inspector edit retains its own undo command")

func _test_mixed_and_ghost_gestures() -> void:
	_reset()
	var mouse := _screen(Vector3(2, 20, 0))
	selection.set_filter(0)
	selection.select_hit({"kind": "mesh", "index": 0}, false)
	selection.select_hit({"kind": "anchor", "index": 0}, true)
	selection.select_hit({"kind": "surface", "index": 0}, true)
	selection.press(mouse, MOUSE_BUTTON_LEFT)
	selection.motion(mouse + Vector2(30, 0))
	selection.release(mouse + Vector2(30, 0))
	var delta: Vector3 = editor._parts[0].position
	expect(delta != Vector3.ZERO and editor._anchor_position(editor._site_anchors_data[0]).distance_to(delta) < 0.02, "mixed mesh/anchor selection translates together")
	var vertex: Vector2 = editor._site_surface_vertices(editor._site_surfaces_data[0])[0]
	expect(vertex.distance_to(Vector2(-8, -8) + Vector2(delta.x, delta.z)) < 0.02, "mixed selection also moves its yard")
	editor._session.undo()
	expect(editor._session.document.snapshot() == fixture, "one undo restores all selected object kinds")
	_reset()
	editor._set_selected_mesh_parts([0, 1], 1)
	selection.press(mouse, MOUSE_BUTTON_RIGHT)
	selection.motion(mouse + Vector2(40, 0))
	selection.release(mouse + Vector2(40, 0))
	expect(editor._selected_part_index == 0 and editor._parts[0].rotation_y != 0 and editor._parts[1].rotation_y == 0, "right-drag rotates the clicked mesh within a multi-selection")
	_reset()
	expect(editor._preview.load_ghost(box_path, 1.0, 1, 1), "load comparison geometry")
	editor._preview.set_ghost_world_position(Vector3(15, 0, 0))
	var ghost_start: Vector3 = editor._preview.get_ghost_world_position()
	var ghost_mouse := _screen(ghost_start + Vector3(0, 20, 0))
	expect(not _has(_pick(ghost_start, "mesh"), "ghost") and _has(_pick(ghost_start, "all"), "ghost"), "comparison mesh is pickable only in All mode")
	selection.set_filter(0)
	selection.press(ghost_mouse, MOUSE_BUTTON_LEFT)
	selection.motion(ghost_mouse + Vector2(30, 0))
	expect(editor._preview.get_ghost_world_position() != ghost_start, "comparison mesh drags after the threshold")
	selection.cancel()
	expect(editor._preview.get_ghost_world_position() == ghost_start and not editor._session.document.is_dirty() and not editor._session.document.can_undo(), "cancelling comparison drag restores preview without touching the draft")
	editor._preview.clear_ghost()
	expect(not _has(_pick(ghost_start, "all"), "ghost"), "clearing comparison discards its pick cache")

func _test_scale_reference() -> void:
	_reset()
	var data: Dictionary = fixture.duplicate(true)
	data["params"]["lot_width_cells"] = 2
	data["params"]["lot_depth_cells"] = 2
	editor._session.document.reset(data)
	var preview = editor._preview
	var document = editor._session.document
	var builds: int = preview.pick_build_count
	editor._view.scale_reference_button.button_pressed = true
	var bounds: AABB = preview._scale_reference.mesh.get_aabb()
	expect(is_equal_approx(bounds.size.y, 1.8) and is_zero_approx(bounds.position.y + preview._scale_reference.position.y), "scale reference is actually 1.8 metres tall and grounded")
	preview.set_scale_reference_world_position(Vector3(-6, 0.9, 5))
	for scale in [1.0, 2.0]:
		root.content_scale_factor = scale
		root.size = Vector2i(Vector2(1920, 1080) * scale)
		for frame in 8:
			await process_frame
		_top_camera()
		var mouse := _screen(preview.scale_reference_world_position())
		expect(_has(selection.picker.collect(mouse + Vector2(9, 0), camera, "all"), "reference"), "reference handle has a ten-logical-pixel radius independent of UI scale")
		expect(not _has(selection.picker.collect(mouse + Vector2(11, 0), camera, "all"), "reference"), "reference handle rejects clicks outside its visible ring")
		# Preserve the grab offset, including when starting on the handle rather than the capsule.
		for target in [Vector3(5, 0.9, 5), Vector3(0, 0.9, 0), Vector3(14, 0.9, 6), Vector3(-6, 0.9, 5)]:
			mouse = _screen(preview.scale_reference_world_position()) + Vector2(7, 0)
			var end := _screen(target) + Vector2(7, 0)
			expect(editor._view._preview_view_rect.get_global_rect().has_point(mouse) and editor._view._preview_view_rect.get_global_rect().has_point(end), "reference drag endpoints are inside the preview")
			_mouse_button(mouse, true)
			expect(preview.scale_reference_selected and editor._selected_part_indices.is_empty() and editor._selected_site_anchor_indices.is_empty() and editor._selected_site_surface_index < 0, "reference click exclusively selects the helper over yards, meshes and anchors")
			var before: Vector3 = preview.scale_reference_world_position()
			_mouse_motion(mouse + Vector2(2, 1))
			expect(preview.scale_reference_world_position() == before and not selection.dragging, "reference click jitter does not move it")
			_mouse_motion(end)
			expect(selection.dragging and not document.is_editing(), "reference drag never begins a document transaction")
			_mouse_button(end, false)
			expect(preview.scale_reference_world_position().distance_to(target) < 0.001, "reference drags across the yard, underlying geometry and outside the lot without snapping or jumping")
			expect(document.snapshot() == data and not document.is_dirty() and not document.can_undo(), "reference placement never changes authored geometry or history")
	root.content_scale_factor = 1.0
	root.size = Vector2i(1920, 1080)
	for frame in 8:
		await process_frame
	_top_camera()
	var original: Vector3 = preview.scale_reference_world_position()
	camera.look_at_from_position(original + Vector3(0, 1, 4), original)
	var body_mouse := _screen(original + Vector3(0, 0.6, 0))
	var body_hits: Array = selection.picker.collect(body_mouse, camera, "all")
	expect(not body_hits.is_empty() and body_hits[0]["kind"] == "reference" and not body_hits[0]["handle"], "the visible capsule itself is triangle-pickable outside its small handle")
	_click(body_mouse)
	expect(preview.scale_reference_selected, "left click on the figure selects it")
	_mouse_button(body_mouse, true)
	var plane_y: float = selection._target["position"].y
	var start_hit: Vector3 = editor._project_mouse_to_horizontal_plane(body_mouse, plane_y)
	var end_mouse := body_mouse + Vector2(50, 30)
	var end_hit: Vector3 = editor._project_mouse_to_horizontal_plane(end_mouse, plane_y)
	_mouse_motion(end_mouse)
	_mouse_button(end_mouse, false)
	var expected := original + Vector3(end_hit.x - start_hit.x, 0, end_hit.z - start_hit.z)
	expect(preview.scale_reference_world_position().distance_to(expected) < 0.001, "oblique figure drag preserves the picked-height grab offset without changing grounding")
	preview.set_scale_reference_world_position(original)
	_top_camera()
	_click(_screen(Vector3(-14, 0, 9)))
	expect(not preview.scale_reference_selected and preview.scale_reference_world_position() == original, "empty clicks deselect instead of teleporting the reference")
	_click(_screen(Vector3(6, 0.1, -6)))
	expect(editor._selected_site_surface_index == 0 and preview.scale_reference_world_position() == original, "clicking a yard selects it without placing the reference")
	_click(_screen(original))
	var delete := InputEventKey.new()
	delete.keycode = KEY_DELETE
	delete.pressed = true
	root.push_input(delete, true)
	expect(document.snapshot() == data and preview.scale_reference_selected, "selecting the reference leaves no authored selection for Delete to remove")
	for reason in ["escape", "focus", "modal", "hide"]:
		var mouse := _screen(original)
		_mouse_button(mouse, true)
		_mouse_motion(mouse + Vector2(40, 0))
		expect(preview.scale_reference_world_position() != original, "reference cancellation fixture begins a real drag")
		match reason:
			"escape":
				var escape := InputEventKey.new()
				escape.keycode = KEY_ESCAPE
				escape.pressed = true
				root.push_input(escape, true)
			"focus": editor.get_window().focus_exited.emit()
			"modal":
				var popup := AcceptDialog.new()
				editor.add_child(popup)
				popup.popup_centered(Vector2i(400, 200))
				selection.update()
				popup.hide()
				popup.queue_free()
			"hide": editor._view.scale_reference_button.button_pressed = false
		expect(not selection.pressed and preview.scale_reference_world_position() == original, "reference drag is restored on " + reason)
	_mouse_button(_screen(original), false)
	expect(not _has(_pick(original, "all"), "reference") and not preview.scale_reference_selected, "hidden reference has no pick target or selection")
	editor._view.scale_reference_button.button_pressed = true
	expect(preview.scale_reference_world_position() == original, "showing the reference preserves its last placement")
	selection.set_filter(1)
	expect(not _has(_pick(original, "mesh"), "reference"), "explicit authored-only filters exclude preview helpers")
	selection.set_filter(0)
	_click(_screen(original))
	selection.refresh(_screen(original), true)
	expect(editor._view.picking_overlay.reference_selected and editor._view.hover_label.text.contains("Scale reference"), "reference selection has persistent visual feedback")
	editor._select_mesh_part(0)
	expect(not preview.scale_reference_selected, "selecting an authored object from its list clears reference selection")
	editor._session.set_field("lot_width_cells", 3)
	editor._session.undo()
	editor.set_ui_theme_mode("light")
	preview.set_frontage_forward(Vector3.LEFT)
	expect(preview.scale_reference_world_position() == original and preview.pick_build_count == builds, "lot/frontage/theme rebuilds and undo preserve reference placement and its one-time pick cache")
	var mouse := _screen(original)
	_mouse_button(mouse, true)
	_mouse_motion(mouse + Vector2(30, 0))
	_mouse_button(mouse + Vector2(30, 0), false)
	expect(document.can_redo() and not document.can_undo() and not document.is_dirty(), "reference drag preserves an existing document redo chain")
	await _capture_scale_reference()
	editor._view.scale_reference_button.button_pressed = false
	_reset()

func _capture_scale_reference() -> void:
	for argument in OS.get_cmdline_user_args():
		if not argument.begins_with("--capture-selection="):
			continue
		var directory := argument.trim_prefix("--capture-selection=")
		for mode in ["dark", "light"]:
			editor.set_ui_theme_mode(mode)
			editor._preview.set_scale_reference_world_position(Vector3(5, 0.9, 5))
			for frame in 8:
				await process_frame
			camera.look_at_from_position(Vector3(22, 24, 28), Vector3.ZERO)
			editor._cam_input._update_preview_offset()
			editor.set_process(false)
			selection.refresh(_screen(editor._preview.scale_reference_world_position()), true)
			await RenderingServer.frame_post_draw
			expect(root.get_texture().get_image().save_png(directory.path_join(mode + "-scale-reference.png")) == OK, "selected scale reference capture")
			editor.set_process(true)

func _test_invalidations() -> void:
	_reset()
	selection.set_filter(1)
	var mouse := _screen(Vector3.ZERO)
	selection.refresh(mouse, true)
	var queries: int = selection.picker.queries
	var builds: int = editor._preview.pick_build_count
	for index in 100:
		selection.refresh(mouse)
	expect(selection.picker.queries == queries, "unchanged pointer/camera does no picking work")
	_top_camera(80)
	selection.refresh(mouse)
	expect(selection.picker.queries == queries + 1, "stationary pointer updates on camera movement")
	queries = selection.picker.queries
	editor._parts[0].position.x = 25
	editor._apply_mesh_part_transform_from_state(0)
	selection.refresh(mouse)
	expect(selection.picker.queries == queries + 1 and selection.hovered["index"] == 1, "transform invalidation updates hover without a mouse event")
	expect(editor._preview.pick_build_count == builds, "transforms never rebuild BVHs")
	editor._select_mesh_part(1)
	editor._remove_selected_mesh_parts()
	editor._session.capture_geometry("Delete fixture part")
	selection.refresh(mouse)
	expect(selection.hovered.is_empty(), "removed parts never leave stale pick proxies")
	var missing: Dictionary = fixture.duplicate(true)
	missing["sources"][0] = [ProjectSettings.globalize_path("user://missing-pick-source.glb")]
	editor._session.document.reset(missing)
	var hits := _pick(Vector3.ZERO, "mesh")
	expect(hits.size() == 1 and hits[0]["index"] == 1, "missing LOD0 does not pick stale geometry or shift later part indices")
	editor._session.replace_part_source(0, narrow_path)
	expect(_has(_pick(Vector3.ZERO, "mesh"), "mesh", 0) and not _has(_pick(Vector3(2, 20, 0), "mesh"), "mesh", 0), "source replacement rebuilds the changed geometry cache")

func _captures() -> void:
	for argument in OS.get_cmdline_user_args():
		if not argument.begins_with("--capture-selection="):
			continue
		var directory := argument.trim_prefix("--capture-selection=")
		DirAccess.make_dir_recursive_absolute(directory)
		_reset()
		await _capture_occlusion(directory)
		_reset()
		for mode in ["dark", "light"]:
			editor.set_ui_theme_mode(mode)
			for kind in ["mesh", "anchor", "surface"]:
				selection.set_filter(selection.FILTERS.find(kind))
				editor._view.selection_filter.select(selection.FILTERS.find(kind))
				if kind == "surface":
					editor._set_selected_site_surface(0)
				for frame in 8:
					await process_frame
				editor.set_process(false)
				selection.refresh(_screen(Vector3.ZERO), true)
				if selection.hovered.is_empty():
					selection.cycle(_screen(Vector3.ZERO))
				expect(not selection.hovered.is_empty() and selection.hovered["kind"] == kind, "rendered hover matches selection filter")
				expect(editor._view.hover_label.get_theme_color("font_color") == Color.WHITE, "hover text remains legible in either theme")
				await RenderingServer.frame_post_draw
				expect(root.get_texture().get_image().save_png(directory.path_join(mode + "-" + kind + ".png")) == OK, "selection screenshot")
				editor.set_process(true)

func _capture_occlusion(directory: String) -> void:
	var preview = editor._preview
	preview.set_lot_size(1, 1)
	preview.set_frontage_forward(Vector3.FORWARD)
	editor._view.show_task("model")
	for frame in 8:
		await process_frame
	camera.look_at_from_position(Vector3(0, 24, 32), Vector3(0, 8, 0))
	editor._cam_input._update_preview_offset()
	editor.set_process(false)
	selection.refresh(Vector2(-100, -100), true)
	var guides := [preview._frontage_arrow, preview._frontage_label, preview._lot_overlay,
		preview._site_anchor_overlay, preview._site_anchor_label_root,
		preview._site_surface_overlay, preview._site_surface_label_root]
	var probes := [selection.picker.anchor_position(0), preview.site_label("anchor", 0).global_position,
		preview._frontage_label.global_position, Vector3(0, 0.06, -5)]
	for state in [["day", 10.5], ["night", 0.0]]:
		editor._view._preview_panel.set_lighting(state[1])
		for frame in 8:
			await process_frame
		await RenderingServer.frame_post_draw
		var visible_guides := root.get_texture().get_image()
		expect(visible_guides.save_png(directory.path_join(state[0] + "-occlusion.png")) == OK, "capture depth-tested guides")
		for guide in guides:
			guide.hide()
		await process_frame
		await RenderingServer.frame_post_draw
		var without_guides := root.get_texture().get_image()
		var pixel_scale := Vector2(visible_guides.get_size()) / root.get_visible_rect().size
		for point: Vector3 in probes:
			var mouse := _screen(point)
			var begin := camera.project_position(mouse, camera.near)
			expect(not preview.pick_mesh_parts(begin, point).is_empty(), "capture probe lies behind the building")
			var region := Rect2i(Vector2i(mouse * pixel_scale) - Vector2i(8, 8), Vector2i(16, 16))
			expect(visible_guides.get_region(region).get_data() == without_guides.get_region(region).get_data(),
				"occluded anchor/frontage/lot pixels match guides hidden entirely in " + state[0])
		for guide in guides:
			guide.show()
	editor.set_process(true)

func _benchmark() -> void:
	if not "--benchmark-asset-selection" in OS.get_cmdline_user_args():
		return
	_reset()
	var mouse := _screen(Vector3.ZERO)
	selection.set_filter(0)
	selection.refresh(mouse, true)
	var builds: int = editor._preview.pick_build_count
	var queries: int = selection.picker.queries
	var start := Time.get_ticks_usec()
	for index in 10000:
		selection.refresh(mouse)
	var idle := (Time.get_ticks_usec() - start) / 10000.0
	expect(selection.picker.queries == queries, "benchmark idle has zero geometry queries")
	start = Time.get_ticks_usec()
	for index in 1000:
		selection.picker.collect(mouse + Vector2(index % 12, 0), camera, "all")
	var query := (Time.get_ticks_usec() - start) / 1000.0
	expect(editor._preview.pick_build_count == builds, "benchmark pointer queries have zero cache builds")
	print("asset_selection_measure idle_mean_us=%.3f query_mean_us=%.3f idle_iterations=10000 query_iterations=1000 parts=2 anchors=1 yards=1 cache_builds=0" % [idle, query])
	editor._view.scale_reference_button.button_pressed = true
	editor._preview.set_scale_reference_world_position(Vector3(0, 0.9, 0))
	mouse = _screen(editor._preview.scale_reference_world_position())
	selection.refresh(mouse, true)
	queries = selection.picker.queries
	start = Time.get_ticks_usec()
	for index in 10000:
		selection.refresh(mouse)
	idle = (Time.get_ticks_usec() - start) / 10000.0
	expect(selection.picker.queries == queries, "visible reference adds no idle picking work")
	start = Time.get_ticks_usec()
	for index in 1000:
		selection.picker.collect(mouse + Vector2(index % 12, 0), camera, "all")
	query = (Time.get_ticks_usec() - start) / 1000.0
	selection.press(mouse, MOUSE_BUTTON_LEFT)
	selection.motion(mouse + Vector2(10, 0))
	queries = selection.picker.queries
	var visibility_queries: int = selection.picker.visibility_queries
	start = Time.get_ticks_usec()
	for index in 1000:
		selection.motion(mouse + Vector2(10 + index % 30, 0))
	var motion := (Time.get_ticks_usec() - start) / 1000.0
	selection.release(mouse + Vector2(19, 0))
	expect(editor._preview.pick_build_count == builds and selection.picker.queries == queries and not editor._session.document.can_undo(), "reference drag locks its target without picking, rebuilding or document history work")
	expect(selection.picker.visibility_queries == visibility_queries, "reference drag reuses site-handle occlusion without new rays")
	print("asset_reference_measure idle_mean_us=%.3f query_mean_us=%.3f drag_motion_mean_us=%.3f idle_iterations=10000 query_iterations=1000 motion_iterations=1000 parts=2 anchors=1 yards=1 references=1 cache_builds=0" % [idle, query, motion])
	editor._view.scale_reference_button.button_pressed = false
	# Measure rebuild submission separately; deferred Label3D cleanup is outside this loop.
	start = Time.get_ticks_usec()
	for index in 500:
		editor._preview._build_frontage_arrow()
	var frontage := (Time.get_ticks_usec() - start) / 500.0
	start = Time.get_ticks_usec()
	for index in 100:
		editor._preview._build_site_anchor_overlay()
	var anchors := (Time.get_ticks_usec() - start) / 100.0
	print("asset_guide_measure frontage_rebuild_mean_us=%.3f frontage_iterations=500 frontage_cells=10 anchor_rebuild_mean_us=%.3f anchor_iterations=100 anchors=1" % [frontage, anchors])
	# Keep setup/BVH construction out of query timing. Shared meshes match repeated imported parts.
	for segments in [64, 256]:
		for instances in [1, 16]:
			var scene := Node3D.new()
			root.add_child(scene)
			var mesh := SphereMesh.new()
			mesh.radius = 3.0
			mesh.height = 6.0
			mesh.radial_segments = segments
			mesh.rings = segments / 2
			for index in instances:
				var instance := MeshInstance3D.new()
				instance.mesh = mesh
				instance.position.x = index * 20
				scene.add_child(instance)
			var geometry := preload("res://scripts/editors/asset_editor/mesh_pick_geometry.gd").new()
			start = Time.get_ticks_usec()
			geometry.build(scene)
			var build_us := Time.get_ticks_usec() - start
			var triangles: int = mesh.get_mesh_arrays()[Mesh.ARRAY_INDEX].size() / 3
			start = Time.get_ticks_usec()
			for index in 1000:
				geometry.intersect(Vector3(index % 12 * 0.1, 60, 0), Vector3(0, -60, 0))
			query = (Time.get_ticks_usec() - start) / 1000.0
			print("asset_selection_bvh triangles_per_mesh=%d instances=%d build_us=%d query_mean_us=%.3f query_iterations=1000" % [triangles, instances, build_us, query])
			scene.free()
