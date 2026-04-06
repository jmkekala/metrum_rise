## Selection tool — click to select one road edge; drag to grow the selection
## along connected edges. All selected edges share the same property edits.
##
## Rust methods called: get_hovered_edge(), get_edge_nodes(), set_edge_class(),
##   set_no_building_spawn(), get_no_building_spawn(), get_edge_geometry_3d(),
##   get_edge_width(), intersect_terrain()
extends Node3D

@onready var simulation_node = $"../SimulationNode"
@onready var main_ui = $"../MainUI"
@onready var zoning_overlay = $"../ZoningOverlay"

var active: bool = false:
	set(value):
		active = value
		if not active:
			_hide_properties_panel()
			_clear_highlight()
			selected_edges.clear()
			_connected_nodes.clear()

# Current selection: array of edge indices (int).
var selected_edges: Array[int] = []
# Set of node indices that border any selected edge — used for connectivity check.
var _connected_nodes: Dictionary = {}   # node_idx -> true

# Drag state
var _dragging: bool = false
var _mouse_pressed: bool = false
var _last_hovered: int = -1   # edge idx hovered on last drag frame

# Highlight MeshInstance3D
var _highlight_mi: MeshInstance3D = null
var _highlight_tween: Tween = null

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

func _unhandled_input(event: InputEvent) -> void:
	if not active: return
	if event is InputEventMouseButton and event.button_index == MOUSE_BUTTON_LEFT:
		if event.pressed:
			# Start of a new press — select the edge under cursor immediately.
			_mouse_pressed = true
			_dragging = false
			_last_hovered = -1
			var edge_idx := _hovered_edge()
			if edge_idx != -1:
				_set_selection([edge_idx])
			else:
				_set_selection([])
		else:
			_mouse_pressed = false
			_dragging = false

func _process(_delta):
	if not active or not _mouse_pressed: return
	# Drag frame — extend selection to connected edges under cursor.
	var edge_idx := _hovered_edge()
	if edge_idx != -1 and edge_idx != _last_hovered \
			and not selected_edges.has(edge_idx):
		if _is_connected(edge_idx):
			_add_to_selection(edge_idx)
	_last_hovered = edge_idx

# ── Selection management ────────────────────────────────────────────────────

func _set_selection(indices: Array[int]) -> void:
	selected_edges = indices
	_connected_nodes.clear()
	for idx in selected_edges:
		_register_nodes(idx)
	_rebuild_highlight()
	if selected_edges.is_empty():
		_hide_properties_panel()
	else:
		_show_properties_panel()

func _add_to_selection(edge_idx: int) -> void:
	selected_edges.append(edge_idx)
	_register_nodes(edge_idx)
	_rebuild_highlight()
	_show_properties_panel()

func _register_nodes(edge_idx: int) -> void:
	var nodes: Vector2i = simulation_node.get_edge_nodes(edge_idx)
	if nodes.x >= 0: _connected_nodes[nodes.x] = true
	if nodes.y >= 0: _connected_nodes[nodes.y] = true

func _is_connected(edge_idx: int) -> bool:
	if selected_edges.is_empty(): return true
	var nodes: Vector2i = simulation_node.get_edge_nodes(edge_idx)
	return _connected_nodes.has(nodes.x) or _connected_nodes.has(nodes.y)

# ── Ray helpers ─────────────────────────────────────────────────────────────

func _hovered_edge() -> int:
	var mouse_pos := get_viewport().get_mouse_position()
	var camera := get_viewport().get_camera_3d()
	if not camera: return -1
	var pos = simulation_node.intersect_terrain(
		camera.project_ray_origin(mouse_pos),
		camera.project_ray_normal(mouse_pos))
	if pos == null: return -1
	return simulation_node.get_hovered_edge(pos.x, pos.z)

# ── Properties panel ────────────────────────────────────────────────────────

func _show_properties_panel() -> void:
	if main_ui:
		main_ui.show_road_properties_multi(selected_edges)

func _hide_properties_panel() -> void:
	if main_ui:
		main_ui.hide_road_properties()

# Called by main_ui buttons — applies to all selected edges.
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

# ── Highlight mesh ───────────────────────────────────────────────────────────

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
