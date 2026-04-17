## Selection tool — click to select one road edge; drag to grow the selection
## along connected edges. All selected edges share the same property edits.
## Also supports selecting Junctions to edit Lane Connections and Crosswalks.
extends Node3D

@onready var simulation_node = $"../SimulationNode"
@onready var main_ui = $"../MainUI"
@onready var zoning_overlay = $"../ZoningOverlay"

var _last_click_screen_pos := Vector2.ZERO

var active: bool = false:
	set(value):
		active = value
		if not active:
			_hide_properties_panel()
			var building_inspector = _get_building_inspector()
			if building_inspector:
				building_inspector.close_window()
			_clear_highlight()
			selected_edges.clear()
			_connected_nodes.clear()
			_clear_node_visuals()
			selected_node = -1
			hovered_node = -1

# Edge Selection
var selected_edges: Array[int] = []
var _connected_nodes: Dictionary = {}
var _dragging_edges: bool = false
var _mouse_pressed: bool = false
var _last_hovered: int = -1

var _highlight_mi: MeshInstance3D = null
var _highlight_tween: Tween = null

# Node Selection
var hovered_node: int = -1
var selected_node: int = -1

var lane_spheres = []
var connection_lines = []
var crosswalk_toggles = []

var dragging_lane = false
var drag_start_lane = null
var drag_from_edge = -1
var drag_from_lane = 0
var drag_is_incoming = false
var drag_line_mesh: MeshInstance3D

func _ready():
	_highlight_mi = MeshInstance3D.new()
	_highlight_mi.cast_shadow = GeometryInstance3D.SHADOW_CASTING_SETTING_OFF
	var mat := StandardMaterial3D.new()
	mat.shading_mode = BaseMaterial3D.SHADING_MODE_UNSHADED
	mat.albedo_color = Color(1.0, 1.0, 1.0, 0.45)
	mat.transparency = BaseMaterial3D.TRANSPARENCY_ALPHA
	mat.emission_enabled = true
	mat.emission = Color(1.0, 1.0, 1.0)
	mat.emission_energy_multiplier = 0.6
	mat.cull_mode = BaseMaterial3D.CULL_DISABLED
	mat.no_depth_test = true
	mat.render_priority = 100
	_highlight_mi.material_override = mat
	add_child(_highlight_mi)

	drag_line_mesh = MeshInstance3D.new()
	add_child(drag_line_mesh)

func _unhandled_input(event: InputEvent) -> void:
	if not active: return
	
	if event is InputEventMouseButton and event.button_index == MOUSE_BUTTON_LEFT:
		if event.pressed:
			_mouse_pressed = true
			_last_click_screen_pos = event.position
			
			# If we have a node selected, check if we clicked a UI control (lane sphere or crosswalk)
			if selected_node != -1:
				var clicked_cw = _get_hovered_cw_toggle()
				if clicked_cw != null:
					simulation_node.set_crosswalk_override(selected_node, clicked_cw.edge_id, not clicked_cw.has_cw)
					_build_node_visuals(selected_node)
					if get_node_or_null("../RoadTool"):
						get_node("../RoadTool").update_main_mesh()
					return
						
				var clicked_lane = _get_hovered_lane_sphere()
				if clicked_lane != null and clicked_lane.is_incoming:
					dragging_lane = true
					drag_start_lane = clicked_lane.pos
					drag_from_edge = clicked_lane.edge_id
					drag_from_lane = clicked_lane.lane_id
					drag_is_incoming = clicked_lane.is_incoming
					return
			
			# Check if we clicked near a node to select it
			var mouse_pos = get_viewport().get_mouse_position()
			var camera = get_viewport().get_camera_3d()
			var pos = null
			var edge_idx := -1
			if camera:
				pos = simulation_node.intersect_terrain(camera.project_ray_origin(mouse_pos), camera.project_ray_normal(mouse_pos))
				if pos != null:
					edge_idx = simulation_node.get_hovered_edge(pos.x, pos.z)
					var building_inspector = _get_building_inspector()
					if edge_idx == -1 and building_inspector and building_inspector.try_inspect(pos, _last_click_screen_pos):
						selected_node = -1
						_set_selection([])
						_clear_node_visuals()
						return
					var nearest_node = simulation_node.get_closest_node(pos, 8.0)
					if nearest_node != -1:
						if building_inspector:
							building_inspector.close_window()
						selected_node = nearest_node
						_set_selection([]) # Clear edge selection
						_build_node_visuals(selected_node)
						return
			
			# Otherwise we clicked an edge
			_clear_node_visuals()
			selected_node = -1
			_dragging_edges = false
			_last_hovered = -1
			if edge_idx != -1:
				var building_inspector = _get_building_inspector()
				if building_inspector:
					building_inspector.close_window()
				_set_selection([edge_idx])
			else:
				_set_selection([])
				
		else:
			_mouse_pressed = false
			_dragging_edges = false
			
			if dragging_lane:
				dragging_lane = false
				drag_line_mesh.mesh = null
				var target_lane = _get_hovered_lane_sphere()
				if target_lane != null:
					if target_lane.edge_id == drag_from_edge and target_lane.lane_id == drag_from_lane:
						simulation_node.clear_lane_source(selected_node, drag_from_edge, drag_from_lane)
					elif not target_lane.is_incoming:
						simulation_node.set_lane_connection(selected_node, drag_from_edge, drag_from_lane, target_lane.edge_id, target_lane.lane_id)
					_build_node_visuals(selected_node)

	if event is InputEventMouseButton and event.button_index == MOUSE_BUTTON_RIGHT and event.pressed:
		if selected_node != -1:
			simulation_node.clear_lane_connections(selected_node)
			_build_node_visuals(selected_node)

func _process(_delta):
	if not active: return
	
	if dragging_lane:
		_update_drag_line()
	elif selected_node == -1 and _mouse_pressed:
		var edge_idx := _hovered_edge()
		if edge_idx != -1 and edge_idx != _last_hovered and not selected_edges.has(edge_idx):
			if _is_connected(edge_idx):
				_add_to_selection(edge_idx)
		_last_hovered = edge_idx

# ── Node Visuals ─────────────────────────────────────────────────────────────

func _clear_node_visuals():
	for s in lane_spheres:
		s.mesh_inst.queue_free()
		if s.has("label_inst") and s.label_inst:
			s.label_inst.queue_free()
	lane_spheres.clear()
	for l in connection_lines:
		l.queue_free()
	connection_lines.clear()
	for c in crosswalk_toggles:
		c.mesh_inst.queue_free()
	crosswalk_toggles.clear()
	if drag_line_mesh: drag_line_mesh.mesh = null

func _build_node_visuals(node_id: int):
	_clear_node_visuals()
	
	var unique_edges = {}
	var lanes = simulation_node.get_node_lanes(node_id)
	for lane in lanes:
		var edge_id = lane["edge_id"]
		var lane_id = lane["lane_id"]
		var is_inc = lane["is_incoming"]
		var pos = lane["pos"]
		
		# Find rough center point of edge boundary for crosswalk toggles
		if not unique_edges.has(edge_id):
			unique_edges[edge_id] = pos
		else:
			unique_edges[edge_id] = unique_edges[edge_id].lerp(pos, 0.5)
		
		var mesh_inst = MeshInstance3D.new()
		add_child(mesh_inst)
		var sphere = SphereMesh.new()
		sphere.radius = 0.4
		sphere.height = 0.8
		mesh_inst.mesh = sphere
		mesh_inst.global_position = pos
		
		var mat = StandardMaterial3D.new()
		mat.albedo_color = Color(0.2, 1.0, 0.2) if is_inc else Color(1.0, 0.2, 0.2)
		mat.emission_enabled = true
		mat.emission = mat.albedo_color
		mat.shading_mode = BaseMaterial3D.SHADING_MODE_UNSHADED
		mesh_inst.material_override = mat

		var label = Label3D.new()
		label.text = "e%d" % edge_id
		label.font_size = 64
		label.pixel_size = 0.015
		label.no_depth_test = true
		label.billboard = BaseMaterial3D.BILLBOARD_ENABLED
		label.modulate = Color(1.0, 1.0, 0.0)
		add_child(label)
		label.global_position = pos + Vector3(0, 1.0, 0)

		lane_spheres.push_back({
			"edge_id": edge_id,
			"lane_id": lane_id,
			"is_incoming": is_inc,
			"pos": pos,
			"mesh_inst": mesh_inst,
			"label_inst": label
		})
		
		var node_pos = simulation_node.get_node_pos(node_id)
		var rel_inst = MeshInstance3D.new()
		add_child(rel_inst)
		_draw_line(rel_inst, node_pos, pos, Color(0.5, 0.5, 1.0, 0.4))
		connection_lines.push_back(rel_inst)
		
	for edge_id in unique_edges:
		var pos = unique_edges[edge_id]
		var node_pos = simulation_node.get_node_pos(node_id)
		var dir = (pos - node_pos).normalized()
		var has_cw = simulation_node.has_crosswalk(node_id, edge_id)
		var mesh_inst = MeshInstance3D.new()
		add_child(mesh_inst)
		var box = BoxMesh.new()
		box.size = Vector3(1.5, 0.2, 1.5)
		mesh_inst.mesh = box
		mesh_inst.global_position = pos + dir * 3.5 + Vector3(0, 0.2, 0)
		var mat = StandardMaterial3D.new()
		mat.albedo_color = Color(0, 1, 0, 0.8) if has_cw else Color(1, 0, 0, 0.8)
		mat.shading_mode = BaseMaterial3D.SHADING_MODE_UNSHADED
		mat.transparency = BaseMaterial3D.TRANSPARENCY_ALPHA
		mesh_inst.material_override = mat
		
		crosswalk_toggles.push_back({
			"edge_id": edge_id,
			"pos": mesh_inst.global_position,
			"mesh_inst": mesh_inst,
			"has_cw": has_cw
		})
		
	var connections = simulation_node.get_lane_connections_array(node_id)
	for conn in connections:
		var f_e = int(conn["from_edge"])
		var f_l = int(conn["from_lane"])
		var t_e = int(conn["to_edge"])
		var t_l = int(conn["to_lane"])
		
		var p1 = null
		var p2 = null
		for s in lane_spheres:
			if int(s.edge_id) == f_e and int(s.lane_id) == f_l and s.is_incoming: p1 = s.pos
			if int(s.edge_id) == t_e and int(s.lane_id) == t_l and not s.is_incoming: p2 = s.pos
			
		if p1 != null and p2 != null:
			var line_inst = MeshInstance3D.new()
			add_child(line_inst)
			_draw_arch(line_inst, p1, p2, Color(1.0, 1.0, 0.0))
			connection_lines.push_back(line_inst)

func _get_hovered_lane_sphere():
	var mouse_pos = get_viewport().get_mouse_position()
	var camera = get_viewport().get_camera_3d()
	if not camera: return null
	var pos_variant = simulation_node.intersect_terrain(camera.project_ray_origin(mouse_pos), camera.project_ray_normal(mouse_pos))
	if pos_variant == null: return null
	var pos: Vector3 = pos_variant
	var best_dist = 1.2
	var best_lane = null
	for s in lane_spheres:
		var d = pos.distance_to(s.pos)
		if d < best_dist:
			best_dist = d
			best_lane = s
	return best_lane

func _get_hovered_cw_toggle():
	var mouse_pos = get_viewport().get_mouse_position()
	var camera = get_viewport().get_camera_3d()
	if not camera: return null
	var pos_variant = simulation_node.intersect_terrain(camera.project_ray_origin(mouse_pos), camera.project_ray_normal(mouse_pos))
	if pos_variant == null: return null
	var pos: Vector3 = pos_variant
	var best_dist = 2.0
	var best_cw = null
	for c in crosswalk_toggles:
		var p = c.pos
		p.y = pos.y
		var d = pos.distance_to(p)
		if d < best_dist:
			best_dist = d
			best_cw = c
	return best_cw

func _update_drag_line():
	var mouse_pos = get_viewport().get_mouse_position()
	var camera = get_viewport().get_camera_3d()
	if not camera: return
	var pos_variant = simulation_node.intersect_terrain(camera.project_ray_origin(mouse_pos), camera.project_ray_normal(mouse_pos))
	if pos_variant == null: return
	var drag_end: Vector3 = pos_variant
	drag_end.y += 0.5
	var target = _get_hovered_lane_sphere()
	if target != null and not target.is_incoming:
		drag_end = target.pos
	_draw_arch(drag_line_mesh, drag_start_lane, drag_end, Color(0.0, 1.0, 1.0))

func _draw_arch(mesh_inst: MeshInstance3D, p1: Vector3, p2: Vector3, color: Color):
	var arr_mesh = ArrayMesh.new()
	var arrays = []
	arrays.resize(Mesh.ARRAY_MAX)
	var ribbon_verts = PackedVector3Array()
	var segments = 10
	var dist = p1.distance_to(p2)
	for i in range(segments + 1):
		var t = float(i) / segments
		var p = p1.lerp(p2, t)
		p.y += sin(t * PI) * min(dist * 0.2, 2.0)
		var camera = get_viewport().get_camera_3d()
		var view_dir = (camera.global_position - p).normalized()
		var fwd = (p2 - p1).normalized()
		var right = fwd.cross(view_dir).normalized()
		if right.length_squared() < 0.001: right = Vector3(1, 0, 0)
		ribbon_verts.push_back(p - right * 0.15)
		ribbon_verts.push_back(p + right * 0.15)
	arrays[Mesh.ARRAY_VERTEX] = ribbon_verts
	arr_mesh.add_surface_from_arrays(Mesh.PRIMITIVE_TRIANGLE_STRIP, arrays)
	mesh_inst.mesh = arr_mesh
	var mat = StandardMaterial3D.new()
	mat.albedo_color = color
	mat.emission_enabled = true
	mat.emission = color
	mat.transparency = BaseMaterial3D.TRANSPARENCY_ALPHA
	mat.cull_mode = BaseMaterial3D.CULL_DISABLED
	mat.shading_mode = BaseMaterial3D.SHADING_MODE_UNSHADED
	mesh_inst.material_override = mat

func _draw_line(mesh_inst: MeshInstance3D, p1: Vector3, p2: Vector3, color: Color):
	var im = ImmediateMesh.new()
	mesh_inst.mesh = im
	var mat = StandardMaterial3D.new()
	mat.albedo_color = color
	mat.shading_mode = BaseMaterial3D.SHADING_MODE_UNSHADED
	mat.transparency = BaseMaterial3D.TRANSPARENCY_ALPHA
	mesh_inst.material_override = mat
	im.surface_begin(Mesh.PRIMITIVE_LINES)
	im.surface_add_vertex(p1)
	im.surface_add_vertex(p2)
	im.surface_end()

# ── Edge Selection ──────────────────────────────────────────────────────────

func _set_selection(indices: Array[int]) -> void:
	selected_edges = indices
	_connected_nodes.clear()
	for idx in selected_edges:
		_register_nodes(idx)
	_rebuild_highlight()
	if selected_edges.is_empty():
		if main_ui: main_ui.hide_road_properties()
	else:
		var building_inspector = _get_building_inspector()
		if building_inspector:
			building_inspector.close_window()
		if main_ui: main_ui.show_road_properties_multi(selected_edges, _last_click_screen_pos)

func _add_to_selection(edge_idx: int) -> void:
	selected_edges.append(edge_idx)
	_register_nodes(edge_idx)
	_rebuild_highlight()
	var building_inspector = _get_building_inspector()
	if building_inspector:
		building_inspector.close_window()
	if main_ui: main_ui.show_road_properties_multi(selected_edges, _last_click_screen_pos)

func _register_nodes(edge_idx: int) -> void:
	var nodes: Vector2i = simulation_node.get_edge_nodes(edge_idx)
	if nodes.x >= 0: _connected_nodes[nodes.x] = true
	if nodes.y >= 0: _connected_nodes[nodes.y] = true

func _is_connected(edge_idx: int) -> bool:
	if selected_edges.is_empty(): return true
	var nodes: Vector2i = simulation_node.get_edge_nodes(edge_idx)
	return _connected_nodes.has(nodes.x) or _connected_nodes.has(nodes.y)

func _hovered_edge() -> int:
	var mouse_pos = get_viewport().get_mouse_position()
	var camera = get_viewport().get_camera_3d()
	if not camera: return -1
	var pos = simulation_node.intersect_terrain(camera.project_ray_origin(mouse_pos), camera.project_ray_normal(mouse_pos))
	if pos == null: return -1
	return simulation_node.get_hovered_edge(pos.x, pos.z)

func _hide_properties_panel() -> void:
	if main_ui: main_ui.hide_road_properties()

func _get_building_inspector():
	return get_node_or_null("../BuildingInspector")

func set_selected_edge_class(class_int: int) -> void:
	for idx in selected_edges:
		simulation_node.set_edge_class(idx, class_int)
	if get_node_or_null("../RoadTool"):
		get_node("../RoadTool").update_main_mesh()

func set_selected_edge_no_building_spawn(enabled: bool) -> void:
	for idx in selected_edges:
		simulation_node.set_no_building_spawn(idx, enabled)
	if zoning_overlay:
		zoning_overlay.mark_no_build_dirty()
		zoning_overlay._rebuild_no_build_overlay()

func _rebuild_highlight() -> void:
	if selected_edges.is_empty():
		_clear_highlight()
		return
	var im := ImmediateMesh.new()
	im.surface_begin(Mesh.PRIMITIVE_TRIANGLES)
	for edge_idx in selected_edges:
		var pts: PackedVector3Array = simulation_node.get_edge_geometry_3d(edge_idx)
		if pts.size() < 2: continue
		var half_w: float = simulation_node.get_edge_width(edge_idx) * 0.5 + 1.5
		for i in range(pts.size() - 1):
			var a: Vector3 = pts[i];     a.y += 0.03
			var b: Vector3 = pts[i + 1]; b.y += 0.03
			var tang: Vector3 = (b - a).normalized()
			var lat := Vector3(-tang.z, 0.0, tang.x) * half_w
			var al := a - lat; var ar := a + lat
			var bl := b - lat; var br := b + lat
			im.surface_add_vertex(al); im.surface_add_vertex(ar); im.surface_add_vertex(br)
			im.surface_add_vertex(al); im.surface_add_vertex(br); im.surface_add_vertex(bl)
	im.surface_end()
	_highlight_mi.mesh = im
	if not _highlight_tween or not _highlight_tween.is_running():
		if _highlight_tween: _highlight_tween.kill()
		_highlight_tween = create_tween().set_loops().set_parallel(false)
		var mat := _highlight_mi.material_override as StandardMaterial3D
		_highlight_tween.tween_property(mat, "emission", Color(1.0, 1.0, 0.0), 0.5)
		_highlight_tween.tween_property(mat, "emission", Color(1.0, 1.0, 1.0), 0.5)

func _clear_highlight() -> void:
	if _highlight_tween:
		_highlight_tween.kill()
		_highlight_tween = null
	if _highlight_mi:
		_highlight_mi.mesh = null
		var mat := _highlight_mi.material_override as StandardMaterial3D
		if mat: mat.emission = Color(1, 1, 1)
