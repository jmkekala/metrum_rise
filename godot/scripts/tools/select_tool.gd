# SPDX-License-Identifier: GPL-3.0-or-later

## Selection tool — click to select one road edge; drag to grow the selection
## along connected edges. All selected edges share the same property edits.
## Also supports selecting Junctions to edit Lane Connections and Crosswalks: each arm that can
## cross gets a toggle that turns its zebra, and with it its pedestrian crossing, on or off
## (`ROAD-74`). Rust decides: hovering a toggle asks for the commit's verdict, and a click
## commits only a toggle that verdict accepted. The Road Properties window's lane steppers
## (`ROAD-80`) ask the same kind of verdict for the selected roads' lanes and commit on Apply.
## A junction click also opens the Junction window (`junction_window.gd`) beside the click: its
## Turn pockets rows (`ROAD-95`, round R5c2) edit each incoming road's pocket, and this tool draws
## the highlighted row's road from the junction over the requested length and redraws the
## junction's lanes after an edit or an undo. The lots a widening or a pocket would take
## (`ZONE-10`, Rust's `demolish_lots` in each verdict) are outlined with the bulldoze highlight
## while that verdict is shown.
## Lane handles (Rust's `get_node_lanes`) sit across each arm in its lanes' order; an approach
## lane the junction's lane rules leave without a turn (Rust's `no_movement`, `ROAD-119`) is drawn
## orange and labelled "no turn", and the Junction window says why. A lane rule edit publishes
## its arms' paint and a new render generation at once (`ROAD-120`), on which the Junction window
## reads its lane warning again; the tool redraws the handles after its own edit, as after a
## pocket edit.
## Picking is exact (Rust's `pick_road_piece`): a click or hover takes the road or junction whose
## paved piece the cursor is on, never the nearest one. Every junction in view (three or more
## roads, Rust's `get_junction_markers`) carries a small dot, so it is clear which ones open the
## Junction window; the hovered and the selected junction are outlined (`footprint_halo.gd`
## around Rust's `get_junction_outline`, its paved area: exactly where a click opens it). The
## dots are read for a square around the view centre, again only when the view moves a quarter
## of it, zooms or the network changes, so their cost follows the junctions in view, never the
## city.
## The selection is drawn by `road_highlight.gdshader`: a faint fill with a glowing outline and,
## on each vehicle lane, chevrons drifting the way its traffic goes. While the tool is on and nothing is being
## dragged, the road under the cursor gets a fainter outline and the pointer becomes a hand, so
## it is clear a click picks roads. Only the one hovered road is drawn, rebuilt when it changes:
## the cost is one surface probe and one road-index query per frame, whatever the map's size.
## Its ribbon comes from one Rust `get_edge_ribbon` call (polyline, width and lanes) that never
## waits for a busy core: the hover then keeps the last road and asks again the next frame.
extends Node3D

const HighlightMesh := preload("res://scripts/renderers/highlight_mesh.gd")
const RoadHighlightShader := preload("res://assets/materials/road_highlight.gdshader")
# The highlight runs this far past each kerb, in metres, so the outline frames the road.
const HIGHLIGHT_PAD_M := 0.8

@onready var simulation_node = $"../SimulationNode"
@onready var main_ui = $"../MainUI"
@onready var zoning_overlay = $"../ZoningOverlay"

var _last_click_screen_pos := Vector2.ZERO

var active: bool = false:
	set(value):
		active = value
		if not active:
			clear_selection()
			_set_hover(-1)
			_clear_junction_markers()

# Edge Selection
var selected_edges: Array[int] = []
var _connected_nodes: Dictionary = {}
var _mouse_pressed: bool = false
var _last_hovered: int = -1

var _highlight_mi: MeshInstance3D = null
# The road under the cursor while nothing is pressed, and its fainter outline.
var _hover_edge: int = -1
var _hover_mi: MeshInstance3D = null
# The Junction window's highlighted road: a strip from the junction over the pocket's length,
# drawn with the selection highlight's material in its own colour.
var _pocket_end_mi: MeshInstance3D = null
const POCKET_END_COLOR := Color(0.2, 0.9, 1.0, 0.3)
# An approach lane handle the lane rules leave without a turn (`ROAD-119`).
const NO_MOVEMENT_COLOR := Color(1.0, 0.55, 0.0)
# The lots the shown verdict takes (`ZONE-10`), in the bulldoze highlight.
var _demolish_mi: MeshInstance3D = null

# Junction markers: the dots of the junctions in view, the outline of the hovered or selected
# one, what Rust last listed (ids, positions) and the square and network generation it was read
# for.
const FootprintHalo := preload("res://scripts/renderers/footprint_halo.gd")
const JUNCTION_OUTLINE_TINT := Color(0.38, 0.80, 0.88)
# A dot's radius per metre of camera distance (about 4 px across at 1080p), within these bounds.
const JUNCTION_DOT_PER_M := 0.006
const JUNCTION_DOT_MIN_M := 0.6
const JUNCTION_DOT_MAX_M := 5.0
# The marked square's half size follows the camera's distance to the view centre, within these
# bounds; zoomed out further the dots are hidden (too many to read).
const JUNCTION_VIEW_MIN_M := 150.0
const JUNCTION_VIEW_MAX_M := 900.0
var _junction_mi: MeshInstance3D = null
var _junction_halo: MeshInstance3D = null
var _junction_ids := PackedInt32Array()
var _junction_positions := PackedVector3Array()
var _junction_centre := Vector3.INF
var _junction_half := 0.0
var _junction_generation := -1
var _hover_junction := -1
var _lit_junction := -1

# Node Selection
var selected_node: int = -1

var lane_spheres = []
var connection_lines = []
var crosswalk_toggles = []
# The hovered toggle and the commit's verdict on it: [node, edge, has_cw] and preview_crosswalk's
# dictionary, asked once per hovered toggle.
var _crosswalk_preview_key: Array = []
var _crosswalk_preview: Dictionary = {}

var _drag_source: Dictionary = {}
var drag_line_mesh: MeshInstance3D

func _ready():
	_highlight_mi = MeshInstance3D.new()
	_highlight_mi.cast_shadow = GeometryInstance3D.SHADOW_CASTING_SETTING_OFF
	_highlight_mi.material_override = _highlight_material(0.10, 0.30, 0.95, 0.22, 100)
	add_child(_highlight_mi)
	_hover_mi = MeshInstance3D.new()
	_hover_mi.cast_shadow = GeometryInstance3D.SHADOW_CASTING_SETTING_OFF
	_hover_mi.material_override = _highlight_material(0.04, 0.14, 0.55, 0.0, 99)
	add_child(_hover_mi)
	# The pocket stretch keeps a plain see-through fill in its own colour.
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
	_pocket_end_mi = MeshInstance3D.new()
	_pocket_end_mi.cast_shadow = GeometryInstance3D.SHADOW_CASTING_SETTING_OFF
	var end_mat := mat.duplicate() as StandardMaterial3D
	end_mat.albedo_color = POCKET_END_COLOR
	end_mat.emission = Color(POCKET_END_COLOR, 1.0)
	end_mat.render_priority = 101
	_pocket_end_mi.material_override = end_mat
	add_child(_pocket_end_mi)

	drag_line_mesh = MeshInstance3D.new()
	add_child(drag_line_mesh)

	_junction_mi = MeshInstance3D.new()
	_junction_mi.cast_shadow = GeometryInstance3D.SHADOW_CASTING_SETTING_OFF
	_junction_mi.material_override = _highlight_material(0.55, 0.0, 0.0, 0.0, 98)
	add_child(_junction_mi)
	_junction_halo = FootprintHalo.new()
	add_child(_junction_halo)

## Clears roads, junction handles and the building inspector; returns whether anything was selected.
func clear_selection() -> bool:
	var had_selection := not selected_edges.is_empty() or selected_node != -1
	_cancel_gesture()
	_hide_properties_panel()
	var building_inspector = _get_building_inspector()
	if building_inspector:
		had_selection = building_inspector.is_inspecting() or had_selection
		building_inspector.close_window()
	_clear_highlight()
	selected_edges.clear()
	_connected_nodes.clear()
	_clear_node_visuals()
	_select_node(-1)
	return had_selection

func _input(event: InputEvent) -> void:
	if active and _mouse_pressed and event is InputEventMouseButton and event.button_index == MOUSE_BUTTON_LEFT and not event.pressed:
		_mouse_pressed = false
		# Let an unhandled release commit first; a GUI-consumed release only cancels.
		_cancel_released_gesture.call_deferred()

func _cancel_released_gesture() -> void:
	# A new press can arrive before the deferred callback from the previous release.
	if not _mouse_pressed:
		_cancel_gesture()

func _cancel_gesture() -> void:
	_mouse_pressed = false
	_drag_source = {}
	_last_hovered = -1
	if drag_line_mesh:
		drag_line_mesh.mesh = null

func _unhandled_input(event: InputEvent) -> void:
	if not active: return
	
	if event is InputEventMouseButton and event.button_index == MOUSE_BUTTON_LEFT:
		if event.pressed:
			_cancel_gesture()
			_mouse_pressed = true
			_last_click_screen_pos = event.position
			var pos = _world_position(event.position, true)
			
			# If we have a node selected, check if we clicked a UI control (lane sphere or crosswalk)
			if selected_node != -1:
				var clicked_cw = _get_hovered_cw_toggle(pos)
				if clicked_cw != null:
					if _preview_crosswalk(clicked_cw).get("is_valid", false) and simulation_node.set_crosswalk_override(selected_node, clicked_cw.edge_id, not clicked_cw.has_cw):
						_build_node_visuals(selected_node)
					return
						
				var clicked_lane = _get_hovered_lane_sphere(pos)
				if clicked_lane != null and clicked_lane.is_incoming:
					_drag_source = clicked_lane
					return
			
			# The junction or road the click is on, exactly; else a building.
			var picked: Vector2i = _pick_at(event.position, true)
			var edge_idx := picked.x
			if pos != null:
				var building_inspector = _get_building_inspector()
				if picked == Vector2i(-1, -1) and building_inspector and building_inspector.try_inspect(pos, _last_click_screen_pos):
					_select_node(-1)
					_set_selection([])
					_clear_node_visuals()
					return
				if picked.y != -1:
					if building_inspector:
						building_inspector.close_window()
					_select_node(picked.y)
					_set_selection([]) # Clear edge selection
					_build_node_visuals(selected_node)
					return
			
			# Otherwise we clicked an edge
			_clear_node_visuals()
			_select_node(-1)
			if edge_idx != -1:
				var building_inspector = _get_building_inspector()
				if building_inspector:
					building_inspector.close_window()
				_set_selection([edge_idx])
			else:
				clear_selection()
				
		else:
			if not _drag_source.is_empty():
				var target_lane = _get_hovered_lane_sphere(_world_position(event.position, true))
				if target_lane != null:
					if target_lane.is_incoming and target_lane.edge_id == _drag_source.edge_id and target_lane.lane_id == _drag_source.lane_id:
						simulation_node.clear_lane_source(selected_node, _drag_source.edge_id, _drag_source.lane_id)
					elif not target_lane.is_incoming:
						simulation_node.set_lane_connection(selected_node, _drag_source.edge_id, _drag_source.lane_id, target_lane.edge_id, target_lane.lane_id)
					refresh_junction_visuals()
			_cancel_gesture()

	if event is InputEventMouseButton and event.button_index == MOUSE_BUTTON_RIGHT and event.pressed:
		_cancel_gesture()
		if selected_node != -1:
			clear_lane_rules()

func _process(_delta):
	if not active: return
	_update_junction_markers()
	_update_hover()
	
	if not _drag_source.is_empty():
		_update_drag_line()
	elif selected_node != -1:
		_update_crosswalk_hover()
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
		c.label_inst.queue_free()
	crosswalk_toggles.clear()
	_crosswalk_preview_key = []
	_crosswalk_preview = {}
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
		var no_movement: bool = lane.get("no_movement", false)
		
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
		if no_movement:
			mat.albedo_color = NO_MOVEMENT_COLOR
		else:
			mat.albedo_color = Color(0.2, 1.0, 0.2) if is_inc else Color(1.0, 0.2, 0.2)
		mat.emission_enabled = true
		mat.emission = mat.albedo_color
		mat.shading_mode = BaseMaterial3D.SHADING_MODE_UNSHADED
		mesh_inst.material_override = mat

		var label := _world_label(("e%d no turn" if no_movement else "e%d") % edge_id, 32, Color(1.0, 0.92, 0.35))
		add_child(label)
		label.global_position = pos + Vector3(0, 1.0, 0)

		lane_spheres.push_back({
			"edge_id": edge_id,
			"lane_id": lane_id,
			"is_incoming": is_inc,
			"no_movement": no_movement,
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
		# Only arms that can cross get a toggle; the rule is Rust's.
		if not simulation_node.can_toggle_crosswalk(node_id, edge_id):
			continue
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
		
		var label := _world_label(_crosswalk_label(has_cw, {}), 32, Color.WHITE)
		add_child(label)
		label.global_position = mesh_inst.global_position + Vector3(0, 1.5, 0)
		crosswalk_toggles.push_back({
			"edge_id": edge_id,
			"pos": mesh_inst.global_position,
			"mesh_inst": mesh_inst,
			"label_inst": label,
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

# The toggle's label: its state, and while hovered what a click does (or why it cannot) on a
# second line.
func _crosswalk_label(has_cw: bool, preview: Dictionary) -> String:
	var text := "Zebra on" if has_cw else "Zebra off"
	if preview.is_empty():
		return text
	var turn := "off" if has_cw else "on"
	if preview.get("is_valid", false):
		return "%s\nClick to turn %s" % [text, turn]
	return "%s\nCan't turn %s: %s" % [text, turn, str(preview.get("invalid_reason", ""))]

# A billboard label drawn over everything, white-ish text in a dark outline so it reads on asphalt,
# grass and paint alike, the same size on screen at any zoom.
func _world_label(text: String, font_size: int, colour: Color) -> Label3D:
	var label := Label3D.new()
	label.text = text
	label.font_size = font_size
	# The same size on screen at any zoom (`fixed_size`): about `font_size` / 2 px at 1080p.
	label.fixed_size = true
	label.pixel_size = 0.0007
	label.outline_size = maxi(8, font_size / 4)
	label.outline_modulate = Color(0.05, 0.06, 0.08, 0.92)
	label.modulate = colour
	label.no_depth_test = true
	label.billboard = BaseMaterial3D.BILLBOARD_ENABLED
	label.render_priority = 2
	label.outline_render_priority = 1
	return label

# The commit's verdict on toggling `toggle`, asked once while it stays hovered.
func _preview_crosswalk(toggle: Dictionary) -> Dictionary:
	var key := [selected_node, toggle.edge_id, toggle.has_cw]
	if key != _crosswalk_preview_key:
		_crosswalk_preview_key = key
		_crosswalk_preview = simulation_node.preview_crosswalk(selected_node, toggle.edge_id, not toggle.has_cw)
	return _crosswalk_preview

func _update_crosswalk_hover() -> void:
	if crosswalk_toggles.is_empty():
		return
	var pos = _world_position(get_viewport().get_mouse_position())
	# The simulation thread holds the core: the labels keep the last hover.
	if pos is bool:
		return
	var hovered = _get_hovered_cw_toggle(pos)
	for c in crosswalk_toggles:
		var preview: Dictionary = _preview_crosswalk(c) if c == hovered else {}
		c.label_inst.text = _crosswalk_label(c.has_cw, preview)

# The world point under a screen position, null for none. A click's query waits while the
# simulation thread holds the core, else a click would deselect as if it hit nothing
# (`CODE-20`); the per-frame one answers false then, and its hover keeps its last answer
# (`CODE-25`).
func _world_position(screen_position: Vector2, click := false):
	var camera = get_viewport().get_camera_3d()
	if not camera: return null
	var origin = camera.project_ray_origin(screen_position)
	var direction = camera.project_ray_normal(screen_position)
	if click:
		return simulation_node.intersect_world_surface(origin, direction, true)
	return simulation_node.hover_world_surface(origin, direction)

func _get_hovered_lane_sphere(pos):
	if pos == null: return null
	var best_dist = 1.2
	var best_lane = null
	for s in lane_spheres:
		var d = pos.distance_to(s.pos)
		if d < best_dist:
			best_dist = d
			best_lane = s
	return best_lane

func _get_hovered_cw_toggle(pos):
	if pos == null: return null
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
	var pos = _world_position(get_viewport().get_mouse_position())
	# The simulation thread holds the core: the line keeps its last end.
	if pos is bool:
		return
	if pos == null:
		drag_line_mesh.mesh = null
		return
	var drag_end: Vector3 = pos
	drag_end.y += 0.5
	var target = _get_hovered_lane_sphere(pos)
	if target != null and not target.is_incoming:
		drag_end = target.pos
	_draw_arch(drag_line_mesh, _drag_source.pos, drag_end, Color(0.0, 1.0, 1.0))

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
	var picked = _pick_at(get_viewport().get_mouse_position())
	# The simulation thread holds the core: the last hovered edge stays.
	if picked == null: return _last_hovered
	return picked.x

# Rust's exact pick under a screen position: Vector2i(road, junction), -1 for none. A click waits
# while the simulation thread holds the core; a hover answers null then, keeping its last answer.
func _pick_at(screen_position: Vector2, click := false) -> Variant:
	var camera = get_viewport().get_camera_3d()
	if not camera:
		return Vector2i(-1, -1)
	var hit = simulation_node.pick_road_piece(camera.project_ray_origin(screen_position), camera.project_ray_normal(screen_position), click)
	return null if hit is bool else hit

func _hide_properties_panel() -> void:
	if main_ui: main_ui.hide_road_properties()

func _get_building_inspector():
	return get_node_or_null("../BuildingInspector")

## Gives every selected road class `class_int` (0 ground, 1 bridge, 2 tunnel), each with its undo
## record; Rust refuses a road it cannot change. Each change publishes its chunks and a new render
## generation before the call returns (`ROAD-121`), which the network renderer draws.
func set_selected_edge_class(class_int: int) -> void:
	for idx in selected_edges:
		simulation_node.set_edge_class(idx, class_int)

## Rust's verdict on giving every selected road `fwd` + `bkw` lanes, the median `median_style`
## of `median_width` metres (`ROAD-80`), kerbside `parking` (`PARK-05`: bit 0 left, bit 1 right)
## and a road type's `speed_kmh`, `sidewalks` (Rust's side code in each road's own forward
## direction, `ROAD-107`), `zoning` and `street_lights` (`ROAD-101`) in place: the first refusal
## (`invalid_reason`, `detail`), else `is_valid` with the selection's totals of Rust's
## `buildings_removed` and `cost_delta`, and `demolition_permanent` when any road's is, and the
## lots the widenings take (`demolish_lots`), which this tool outlines. Each road is checked as it
## stands, as Apply commits them one by one.
func preview_selected_edge_cross_section(fwd: int, bkw: int, median_style: int, median_width: float, parking: int,
		speed_kmh: int, sidewalks: int, zoning: bool, street_lights: bool) -> Dictionary:
	var total := {"is_valid": true, "buildings_removed": 0, "cost_delta": 0.0, "demolition_permanent": false,
		"demolish_lots": PackedVector3Array()}
	for idx in selected_edges:
		var verdict: Dictionary = simulation_node.preview_edge_cross_section(idx, fwd, bkw, median_style, median_width,
			parking, speed_kmh, sidewalks, zoning, street_lights)
		if not bool(verdict.get("is_valid", false)):
			show_demolition(PackedVector3Array())
			return verdict
		total.buildings_removed += int(verdict.get("buildings_removed", 0))
		total.cost_delta += float(verdict.get("cost_delta", 0.0))
		total.demolition_permanent = total.demolition_permanent or bool(verdict.get("demolition_permanent", false))
		total.demolish_lots.append_array(verdict.get("demolish_lots", PackedVector3Array()))
	show_demolition(total.demolish_lots)
	return total

## Outlines `lots` (four corners each, as Rust packs them) with the bulldoze highlight; an empty
## array hides it.
func show_demolition(lots: PackedVector3Array) -> void:
	var mesh: Mesh = HighlightMesh.polygons(lots, 4)
	if mesh != null and _demolish_mi == null:
		_demolish_mi = HighlightMesh.instance()
		add_child(_demolish_mi)
	if _demolish_mi != null:
		_demolish_mi.mesh = mesh
		_demolish_mi.visible = mesh != null

## Gives every selected road Rust accepts `fwd` + `bkw` lanes, the median, `parking` and the
## type's attributes, each with its undo record, and returns how many were refused. The edits
## publish their chunks at once; zoning off refreshes the no-build overlay, as the checkbox does.
func set_selected_edge_cross_section(fwd: int, bkw: int, median_style: int, median_width: float, parking: int,
		speed_kmh: int, sidewalks: int, zoning: bool, street_lights: bool) -> int:
	var refused := 0
	for idx in selected_edges:
		if not simulation_node.set_edge_cross_section(idx, fwd, bkw, median_style, median_width, parking, speed_kmh,
				sidewalks, zoning, street_lights):
			refused += 1
	show_demolition(PackedVector3Array())
	_rebuild_highlight()
	if not zoning:
		_refresh_no_build_overlay()
	return refused

## Turns the kerbside parking lane `side_bit` (1 left, 2 right, looking along each road's drawn
## direction) on or off on every selected road, keeping its lanes, median and attributes (`PARK-05`), each
## with its undo record. Returns `refused` (how many roads Rust refused) and `verdict`, the first
## refusal, for the window to show.
func set_selected_edge_parking_side(side_bit: int, enabled: bool) -> Dictionary:
	var refused := 0
	var first_refusal := {}
	for idx in selected_edges:
		var section: Dictionary = simulation_node.get_edge_cross_section(idx)
		var code := int(section.get("parking", 0))
		var wanted := (code | side_bit) if enabled else (code & ~side_bit)
		if wanted == code:
			continue
		var args := [idx, int(section.get("fwd_lanes", 0)), int(section.get("bkw_lanes", 0)),
			int(section.get("median_style", 0)), float(section.get("median_width", 0.0)), wanted,
			int(section.get("speed_kmh", 50)), int(section.get("sidewalks", 3)), bool(section.get("zoning", true)),
			bool(section.get("street_lights", true))]
		var verdict: Dictionary = simulation_node.callv("preview_edge_cross_section", args)
		if not bool(verdict.get("is_valid", false)) or not simulation_node.callv("set_edge_cross_section", args):
			refused += 1
			if first_refusal.is_empty():
				first_refusal = verdict
	_rebuild_highlight()
	return {"refused": refused, "verdict": first_refusal}

## Gives every selected car road the speed limit `speed_kmh` (one of Rust's posted limits),
## keeping its lanes, median, parking, sidewalks, zoning and lights, each with its undo record; a
## walkway has no speed and is left alone. Returns `refused` and `verdict`, the first refusal.
func set_selected_edge_speed(speed_kmh: int) -> Dictionary:
	var refused := 0
	var first_refusal := {}
	for idx in selected_edges:
		var section: Dictionary = simulation_node.get_edge_cross_section(idx)
		var current := int(section.get("speed_kmh", 0))
		if current == 0 or current == speed_kmh:
			continue
		var args := [idx, int(section.get("fwd_lanes", 0)), int(section.get("bkw_lanes", 0)),
			int(section.get("median_style", 0)), float(section.get("median_width", 0.0)),
			int(section.get("parking", 0)), speed_kmh, int(section.get("sidewalks", 3)),
			bool(section.get("zoning", true)), bool(section.get("street_lights", true))]
		var verdict: Dictionary = simulation_node.callv("preview_edge_cross_section", args)
		if not bool(verdict.get("is_valid", false)) or not simulation_node.callv("set_edge_cross_section", args):
			refused += 1
			if first_refusal.is_empty():
				first_refusal = verdict
	_rebuild_highlight()
	return {"refused": refused, "verdict": first_refusal}

func set_selected_edge_no_building_spawn(enabled: bool) -> void:
	for idx in selected_edges:
		simulation_node.set_no_building_spawn(idx, enabled)
	_refresh_no_build_overlay()

func _refresh_no_build_overlay() -> void:
	if zoning_overlay:
		zoning_overlay.mark_no_build_dirty()
		zoning_overlay._rebuild_no_build_overlay()

func _rebuild_highlight() -> void:
	if selected_edges.is_empty():
		_clear_highlight()
		return
	_highlight_mi.mesh = _ribbon_mesh(selected_edges, _highlight_mi, true)
	if selected_edges.has(_hover_edge):
		_set_hover(-1)

## Lights the left (`side_bit` 1) or right (2) half of the selected roads, looking along each
## one's drawn direction, as the parking toggles name them; 0 lights neither.
func light_side(side_bit: int) -> void:
	(_highlight_mi.material_override as ShaderMaterial).set_shader_parameter("side_glow", clampi(side_bit, 0, 2))

func _clear_highlight() -> void:
	show_demolition(PackedVector3Array())
	if _highlight_mi:
		_highlight_mi.mesh = null

func _highlight_material(fill: float, glow: float, edge: float, chevrons: float, priority: int) -> ShaderMaterial:
	var mat := ShaderMaterial.new()
	mat.shader = RoadHighlightShader
	mat.set_shader_parameter("fill_alpha", fill)
	mat.set_shader_parameter("glow_alpha", glow)
	mat.set_shader_parameter("edge_alpha", edge)
	mat.set_shader_parameter("chevron_alpha", chevrons)
	mat.render_priority = priority
	return mat

# The mesh for `mi` (whose material gets the width): per road, a ribbon along its polyline,
# mitred at the bends so the outline stays one line, with UV.x across (0 left, 1 right in the
# drawn direction) and UV.y in metres along it. With `lanes`, each vehicle lane also gets a strip
# (vertex colour red 1) whose UV.y runs the way its traffic goes, for the shader's chevrons. The
# shader's width uniform takes the widest road; on a mixed selection narrower roads get a
# slightly thicker outline, which reads fine. Each road is one `get_edge_ribbon` call; without
# `wait` (the hover) a busy core answers null, and the caller keeps what it shows.
func _ribbon_mesh(edges: Array[int], mi: MeshInstance3D, lanes := false, wait := true) -> ImmediateMesh:
	var im := ImmediateMesh.new()
	var widest := 0.0
	var lane_w := 0.0
	if lanes:
		lane_w = simulation_node.get_lane_width()
	im.surface_begin(Mesh.PRIMITIVE_TRIANGLES)
	for edge_idx in edges:
		var ribbon = simulation_node.get_edge_ribbon(edge_idx, wait)
		if ribbon is bool:
			return null
		var pts: PackedVector3Array = ribbon.get("points", PackedVector3Array())
		if pts.size() < 2:
			continue
		var half_w: float = float(ribbon["width"]) * 0.5 + HIGHLIGHT_PAD_M
		widest = maxf(widest, half_w * 2.0)
		var centre := PackedVector3Array()
		var lateral := PackedVector3Array()
		var along := PackedFloat32Array()
		var run := 0.0
		for i in pts.size():
			var prev: Vector3 = pts[maxi(i - 1, 0)]
			var next: Vector3 = pts[mini(i + 1, pts.size() - 1)]
			var tangent := Vector3(next.x - prev.x, 0.0, next.z - prev.z).normalized()
			# The road's right, looking along its drawn direction.
			var right := Vector3(-tangent.z, 0.0, tangent.x)
			# Mitre: widen at a bend so the edges stay parallel to both segments (capped).
			if i > 0 and i < pts.size() - 1:
				var seg := Vector3(pts[i].x - prev.x, 0.0, pts[i].z - prev.z).normalized()
				right /= maxf(right.dot(Vector3(-seg.z, 0.0, seg.x)), 0.5)
			if i > 0:
				run += Vector2(pts[i].x - pts[i - 1].x, pts[i].z - pts[i - 1].z).length()
			centre.append(pts[i] + Vector3.UP * 0.05)
			lateral.append(right)
			along.append(run)
		_add_strip(im, centre, lateral, along, -half_w, half_w, 1.0, Color.BLACK)
		if not lanes:
			continue
		# Rust's lanes: (offset to the right of the centreline, 1 forward or -1 backward).
		for lane: Vector2 in ribbon["lanes"]:
			_add_strip(im, centre, lateral, along, lane.x - lane_w * 0.5, lane.x + lane_w * 0.5, lane.y, Color.RED)
	im.surface_end()
	var mat := mi.material_override as ShaderMaterial
	mat.set_shader_parameter("width_m", maxf(widest, 1.0))
	if lanes:
		mat.set_shader_parameter("lane_m", lane_w)
	return im

# One strip from `from_m` to `to_m` right of the centre line (UV.x 0 to 1), UV.y the distance
# along times `direction`, in vertex colour `kind`.
func _add_strip(im: ImmediateMesh, centre: PackedVector3Array, lateral: PackedVector3Array,
		along: PackedFloat32Array, from_m: float, to_m: float, direction: float, kind: Color) -> void:
	im.surface_set_color(kind)
	for i in centre.size() - 1:
		var a0 := centre[i] + lateral[i] * from_m
		var a1 := centre[i] + lateral[i] * to_m
		var b0 := centre[i + 1] + lateral[i + 1] * from_m
		var b1 := centre[i + 1] + lateral[i + 1] * to_m
		var ya := along[i] * direction
		var yb := along[i + 1] * direction
		for v in [[a0, 0.0, ya], [a1, 1.0, ya], [b1, 1.0, yb], [a0, 0.0, ya], [b1, 1.0, yb], [b0, 0.0, yb]]:
			im.surface_set_uv(Vector2(v[1], v[2]))
			im.surface_add_vertex(v[0])

# The junction or road under the cursor, exactly: a junction is outlined, a road faintly
# outlined; either gets a hand pointer. Not while a button is down, a junction is open (its
# handles own the clicks) or the mouse is over the UI.
func _update_hover() -> void:
	var edge_idx := -1
	var junction := -1
	if not _mouse_pressed and selected_node == -1 and get_viewport().gui_get_hovered_control() == null:
		var picked = _pick_at(get_viewport().get_mouse_position())
		# The simulation thread holds the core: keep the hover as it is, or it blinks off for
		# every busy frame.
		if picked == null:
			return
		edge_idx = picked.x
		junction = picked.y
	if selected_edges.has(edge_idx):
		edge_idx = -1
	var junction_changed := junction != _hover_junction
	_hover_junction = junction
	_set_hover(edge_idx)
	_light_junction(selected_node if selected_node != -1 else junction)
	if junction_changed:
		Input.set_default_cursor_shape(Input.CURSOR_POINTING_HAND if junction != -1 or edge_idx != -1 else Input.CURSOR_ARROW)

func _set_hover(edge_idx: int) -> void:
	if _hover_edge == edge_idx:
		return
	if _hover_mi:
		if edge_idx == -1:
			_hover_mi.mesh = null
		else:
			var one: Array[int] = [edge_idx]
			var mesh := _ribbon_mesh(one, _hover_mi, false, false)
			# The simulation thread holds the core: keep the last hover and ask again next frame.
			if mesh == null:
				return
			_hover_mi.mesh = mesh
	_hover_edge = edge_idx
	Input.set_default_cursor_shape(Input.CURSOR_POINTING_HAND if edge_idx != -1 else Input.CURSOR_ARROW)

# ── Junction window (`ROAD-95` turn pockets) ────────────────────────────────

# Selects junction `node` (-1: none) and opens the Junction window beside the click, or closes it.
func _select_node(node: int) -> void:
	selected_node = node
	if not main_ui:
		return
	if node == -1:
		main_ui.hide_junction_window()
	else:
		main_ui.show_junction_window(node, _last_click_screen_pos)

## Commits the Junction window's pocket `left` / `right` / `length_m` at end `end` of road `edge`
## (Rust's `set_road_pocket`, with its undo record: Ctrl+Z) and redraws the junction's lanes,
## which the pocket changes. Returns whether Rust applied it.
func set_junction_pocket(edge: int, end: int, left: bool, right: bool, length_m: float) -> bool:
	if not simulation_node.set_road_pocket(edge, end, left, right, length_m):
		return false
	refresh_junction_visuals()
	return true

## Adds the lane rule from approach lane `from_lane` of `from_edge` to `to_lane` of `to_edge` at
## the selected junction (Rust's `set_lane_connection`), as a drag between two handles does.
func set_lane_rule(from_edge: int, from_lane: int, to_edge: int, to_lane: int) -> void:
	simulation_node.set_lane_connection(selected_node, from_edge, from_lane, to_edge, to_lane)
	refresh_junction_visuals()

## Clears the selected junction's lane rules (Rust's `clear_lane_connections`), as a right click
## does.
func clear_lane_rules() -> void:
	simulation_node.clear_lane_connections(selected_node)
	refresh_junction_visuals()

## Redraws the selected junction's lane spheres and crosswalk toggles (after an edit or an undo).
func refresh_junction_visuals() -> void:
	if selected_node != -1:
		_build_node_visuals(selected_node)

## Highlights the stretch of `info`'s road (a Junction window row's arm: its polyline from the
## junction outwards) a pocket of `length_m` takes from the junction, as wide as its widest edge.
func show_pocket_end(info: Dictionary, length_m: float) -> void:
	var road: PackedVector3Array = info.get("road", PackedVector3Array())
	if road.size() < 2:
		clear_pocket_end()
		return
	var half_w := float(info.get("width_m", 6.0)) * 0.5 + 1.5
	var im := ImmediateMesh.new()
	im.surface_begin(Mesh.PRIMITIVE_TRIANGLES)
	var run := 0.0
	for i in range(road.size() - 1):
		var a: Vector3 = road[i]
		var b: Vector3 = road[i + 1]
		var step := Vector2(b.x - a.x, b.z - a.z).length()
		if step <= 0.0:
			continue
		# The last segment is cut where the length runs out.
		if run + step > length_m:
			b = a.lerp(b, (length_m - run) / step)
		a.y += 0.05
		b.y += 0.05
		var tang: Vector3 = (b - a).normalized()
		var lat := Vector3(-tang.z, 0.0, tang.x) * half_w
		im.surface_add_vertex(a - lat); im.surface_add_vertex(a + lat); im.surface_add_vertex(b + lat)
		im.surface_add_vertex(a - lat); im.surface_add_vertex(b + lat); im.surface_add_vertex(b - lat)
		run += step
		if run >= length_m:
			break
	im.surface_end()
	_pocket_end_mi.mesh = im

## Removes that highlight, and the outline of the lots its verdict takes.
func clear_pocket_end() -> void:
	show_demolition(PackedVector3Array())
	if _pocket_end_mi:
		_pocket_end_mi.mesh = null

# ── Junction markers ─────────────────────────────────────────────────────────

# Reads the junctions again when the view has moved a quarter of the marked square, zoomed by a
# quarter or the network changed; a busy core keeps the last rings and asks next frame.
func _update_junction_markers() -> void:
	var camera := get_viewport().get_camera_3d()
	if camera == null:
		return
	var centre_screen := get_viewport().get_visible_rect().size * 0.5
	var origin := camera.project_ray_origin(centre_screen)
	var direction := camera.project_ray_normal(centre_screen)
	if direction.y > -0.05:
		_clear_junction_markers()
		return
	var plane_y := _junction_positions[0].y if not _junction_positions.is_empty() else 0.0
	var centre := origin + direction * ((plane_y - origin.y) / direction.y)
	var half := camera.global_position.distance_to(centre) * 1.3
	if half > JUNCTION_VIEW_MAX_M:
		_clear_junction_markers()
		return
	half = maxf(half, JUNCTION_VIEW_MIN_M)
	var generation := int(simulation_node.get_network_render_generation())
	var moved := Vector2(centre.x - _junction_centre.x, centre.z - _junction_centre.z).length() > _junction_half * 0.25
	var zoomed := absf(half - _junction_half) > _junction_half * 0.25
	if not moved and not zoomed and generation == _junction_generation:
		return
	var markers: Dictionary = simulation_node.get_junction_markers(centre, half)
	if markers.is_empty():
		return
	_junction_centre = centre
	_junction_half = half
	_junction_generation = generation
	_junction_ids = markers.ids
	_junction_positions = markers.positions
	var dot_m := clampf(half / 1.3 * JUNCTION_DOT_PER_M, JUNCTION_DOT_MIN_M, JUNCTION_DOT_MAX_M)
	(_junction_mi.material_override as ShaderMaterial).set_shader_parameter("width_m", dot_m)
	_junction_mi.mesh = _dots_mesh(dot_m)

func _clear_junction_markers() -> void:
	_junction_ids = PackedInt32Array()
	_junction_positions = PackedVector3Array()
	_junction_centre = Vector3.INF
	_junction_half = 0.0
	_hover_junction = -1
	if _junction_mi:
		_junction_mi.mesh = null
	_light_junction(-1)

# Outlines junction `node`'s paved area (-1: none), read from Rust when it changes; a busy core
# leaves it unlit and the next frame asks again.
func _light_junction(node: int) -> void:
	if node == _lit_junction or _junction_halo == null:
		return
	if node == -1:
		_lit_junction = -1
		_junction_halo.hide_halo()
		return
	var outline: PackedVector3Array = simulation_node.get_junction_outline(node)
	if outline.size() < 3:
		_junction_halo.hide_halo()
		return
	_lit_junction = node
	# Lifted off the asphalt the band lies on, which would hide it.
	for i in outline.size():
		outline[i].y += 0.05
	_junction_halo.show_footprint(outline, JUNCTION_OUTLINE_TINT)

# A flat dot of radius `radius_m` at each marked junction: the highlight band from the rim
# (UV.x 0) to the centre (UV.x 1).
func _dots_mesh(radius_m: float) -> ImmediateMesh:
	if _junction_ids.is_empty():
		return null
	var im := ImmediateMesh.new()
	im.surface_begin(Mesh.PRIMITIVE_TRIANGLES)
	im.surface_set_color(Color.BLACK)
	const SEGMENTS := 12
	for centre_point in _junction_positions:
		var centre := centre_point + Vector3.UP * 0.1
		for k in SEGMENTS:
			var a := TAU * float(k) / SEGMENTS
			var b := TAU * float(k + 1) / SEGMENTS
			for v in [[Vector3(cos(a), 0.0, sin(a)) * radius_m, 0.0], [Vector3.ZERO, 1.0], [Vector3(cos(b), 0.0, sin(b)) * radius_m, 0.0]]:
				im.surface_set_uv(Vector2(v[1], 0.0))
				im.surface_add_vertex(centre + v[0])
	im.surface_end()
	return im
