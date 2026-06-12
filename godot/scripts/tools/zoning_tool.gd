## Road-aligned parcel zoning tool -- sends placement points to Rust and previews returned geometry.
##
## Rust methods called: get_zone_profiles(), get_zoning_parcel_preview(),
##   get_zoning_parcel_drag_preview_packed(), apply_zoning_parcel_at(),
##   apply_zoning_parcel_drag(), get_zoning_parcel_profile_runtime_id_at(),
##   get_zoning_parcel_rezone_drag_preview_packed(),
##   apply_zoning_parcel_rezone_drag(), intersect_world_surface()
extends Node3D

@onready var simulation_node = $"../SimulationNode"
@onready var zoning_overlay = $"../ZoningOverlay"

var active: bool = false
var current_profile_runtime_id: int = 0
var parcel_width_cells: int = 2
var parcel_depth_cells: int = 2
var parcel_gap_m: float = 0.0
var profiles: Array[Dictionary] = []
var profiles_by_runtime_id: Dictionary = {}

var preview_mesh: MeshInstance3D
var dragging: bool = false
var drag_start_world = null
var drag_mode: int = 0
var _preview_cache_valid: bool = false
var _preview_cache_kind: int = -1
var _preview_cache_start: Vector2 = Vector2.ZERO
var _preview_cache_end: Vector2 = Vector2.ZERO
var _preview_cache_profile_runtime_id: int = -1
var _preview_cache_width_cells: int = -1
var _preview_cache_depth_cells: int = -1
var _preview_cache_gap_m: float = -1.0
var _preview_cache_mesh: Mesh = null
var _last_valid_single_preview_mesh: Mesh = null

const DRAG_THRESHOLD_M: float = 4.0
const PREVIEW_REFRESH_DISTANCE_M: float = 1.0
const PREVIEW_KIND_SINGLE: int = 0
const PREVIEW_KIND_CREATE_DRAG: int = 1
const PREVIEW_KIND_REZONE_DRAG: int = 2
const DRAG_MODE_NONE: int = 0
const DRAG_MODE_CREATE: int = 1
const DRAG_MODE_REZONE: int = 2

func _ready():
	_reload_profiles()

	preview_mesh = MeshInstance3D.new()
	preview_mesh.cast_shadow = GeometryInstance3D.SHADOW_CASTING_SETTING_OFF
	preview_mesh.top_level = true
	preview_mesh.visible = false
	var mat := StandardMaterial3D.new()
	mat.transparency = BaseMaterial3D.TRANSPARENCY_ALPHA
	mat.shading_mode = BaseMaterial3D.SHADING_MODE_UNSHADED
	mat.cull_mode = BaseMaterial3D.CULL_DISABLED
	mat.vertex_color_use_as_albedo = true
	mat.no_depth_test = true
	preview_mesh.material_override = mat
	add_child(preview_mesh)

func _process(_delta):
	if not active:
		dragging = false
		drag_start_world = null
		drag_mode = DRAG_MODE_NONE
		_clear_preview_cache()
		preview_mesh.visible = false
		return
	_update_preview()

func _unhandled_input(event):
	if not active:
		return

	if event is InputEventMouseButton and event.button_index == MOUSE_BUTTON_LEFT:
		if event.pressed:
			_begin_drag()
		else:
			_finish_drag()

func _reload_profiles() -> void:
	profiles.clear()
	profiles_by_runtime_id.clear()
	var payload = simulation_node.get_zone_profiles()
	if payload is Array:
		for entry in payload:
			if entry is Dictionary:
				var profile: Dictionary = entry
				profiles.append(profile.duplicate(true))
				profiles_by_runtime_id[int(profile.get("runtime_id", 0))] = profile
	if current_profile_runtime_id == 0 and not profiles.is_empty():
		current_profile_runtime_id = int(profiles[0].get("runtime_id", 0))

func select_profile(runtime_id: int) -> void:
	if runtime_id == 0 or profiles_by_runtime_id.has(runtime_id):
		if current_profile_runtime_id != runtime_id:
			_clear_preview_cache()
		current_profile_runtime_id = runtime_id

func select_profile_by_zone_type(zone_type: String) -> void:
	for profile in profiles:
		if str(profile.get("zone_type", "")).strip_edges() == zone_type:
			select_profile(int(profile.get("runtime_id", 0)))
			return

func set_parcel_options(width_cells: int, depth_cells: int, gap_m: float) -> void:
	var next_width := clampi(width_cells, 1, 8)
	var next_depth := clampi(depth_cells, 1, 12)
	var next_gap := clampf(gap_m, 0.0, 20.0)
	if next_width != parcel_width_cells or next_depth != parcel_depth_cells or abs(next_gap - parcel_gap_m) > 0.001:
		_clear_preview_cache()
	parcel_width_cells = next_width
	parcel_depth_cells = next_depth
	parcel_gap_m = next_gap

func undo() -> void:
	pass

func _begin_drag() -> void:
	drag_start_world = _mouse_world_pos()
	dragging = drag_start_world != null
	drag_mode = DRAG_MODE_NONE
	if dragging:
		var start_profile: int = int(simulation_node.get_zoning_parcel_profile_runtime_id_at(
			drag_start_world.x,
			drag_start_world.y
		))
		if start_profile >= 0 and start_profile != current_profile_runtime_id:
			drag_mode = DRAG_MODE_REZONE
		else:
			drag_mode = DRAG_MODE_CREATE
	_clear_preview_cache()

func _finish_drag() -> void:
	if not dragging:
		return
	var start = drag_start_world
	var end = _mouse_world_pos()
	var mode = drag_mode
	dragging = false
	drag_start_world = null
	drag_mode = DRAG_MODE_NONE
	if start == null or end == null:
		return
	if start.distance_to(end) >= DRAG_THRESHOLD_M and mode == DRAG_MODE_REZONE:
		_commit_rezone_drag(start, end)
	elif start.distance_to(end) >= DRAG_THRESHOLD_M:
		_commit_drag(start, end)
	else:
		_commit_single_at(start)
	_clear_preview_cache()

func _commit_single_at(wp: Vector2) -> void:
	if simulation_node.apply_zoning_parcel_at(
		wp.x,
		wp.y,
		current_profile_runtime_id,
		parcel_width_cells,
		parcel_depth_cells
	):
		if zoning_overlay:
			zoning_overlay.mark_zone_dirty()

func _commit_drag(start: Vector2, end: Vector2) -> void:
	if simulation_node.apply_zoning_parcel_drag(
		start.x,
		start.y,
		end.x,
		end.y,
		current_profile_runtime_id,
		parcel_width_cells,
		parcel_depth_cells,
		parcel_gap_m
	):
		if zoning_overlay:
			zoning_overlay.mark_zone_dirty()

func _commit_rezone_drag(start: Vector2, end: Vector2) -> void:
	if simulation_node.apply_zoning_parcel_rezone_drag(
		start.x,
		start.y,
		end.x,
		end.y,
		current_profile_runtime_id
	):
		if zoning_overlay:
			zoning_overlay.mark_zone_dirty()

func _update_preview() -> void:
	var wp = _mouse_world_pos()
	if wp == null:
		preview_mesh.visible = false
		return
	if dragging and drag_start_world != null and drag_start_world.distance_to(wp) >= DRAG_THRESHOLD_M:
		if drag_mode == DRAG_MODE_REZONE:
			if _preview_cache_matches(PREVIEW_KIND_REZONE_DRAG, drag_start_world, wp):
				_apply_preview_mesh(_preview_cache_mesh)
				return

			var rezone_payload: Dictionary = simulation_node.get_zoning_parcel_rezone_drag_preview_packed(
				drag_start_world.x,
				drag_start_world.y,
				wp.x,
				wp.y,
				current_profile_runtime_id
			)
			var rezone_mesh := _build_packed_parcels_mesh(rezone_payload, true)
			_store_preview_cache(PREVIEW_KIND_REZONE_DRAG, drag_start_world, wp, rezone_mesh)
			_apply_preview_mesh(rezone_mesh)
			return

		if _preview_cache_matches(PREVIEW_KIND_CREATE_DRAG, drag_start_world, wp):
			_apply_preview_mesh(_preview_cache_mesh)
			return

		var drag_payload: Dictionary = simulation_node.get_zoning_parcel_drag_preview_packed(
			drag_start_world.x,
			drag_start_world.y,
			wp.x,
			wp.y,
			current_profile_runtime_id,
			parcel_width_cells,
			parcel_depth_cells,
			parcel_gap_m
		)
		var drag_mesh := _build_packed_parcels_mesh(drag_payload, true)
		_store_preview_cache(PREVIEW_KIND_CREATE_DRAG, drag_start_world, wp, drag_mesh)
		_apply_preview_mesh(drag_mesh)
		return

	if _preview_cache_matches(PREVIEW_KIND_SINGLE, Vector2.ZERO, wp):
		_apply_single_preview_mesh(_preview_cache_mesh)
		return

	var payload: Dictionary = simulation_node.get_zoning_parcel_preview(
		wp.x,
		wp.y,
		current_profile_runtime_id,
		parcel_width_cells,
		parcel_depth_cells
	)
	var mesh: Mesh = null if payload.is_empty() else _build_parcels_mesh([payload], true)
	_store_preview_cache(PREVIEW_KIND_SINGLE, Vector2.ZERO, wp, mesh)
	_apply_single_preview_mesh(mesh)

func _preview_cache_matches(kind: int, start: Vector2, end: Vector2) -> bool:
	if not _preview_cache_valid:
		return false
	if _preview_cache_kind != kind:
		return false
	if _preview_cache_profile_runtime_id != current_profile_runtime_id:
		return false
	if _preview_cache_width_cells != parcel_width_cells or _preview_cache_depth_cells != parcel_depth_cells:
		return false
	if abs(_preview_cache_gap_m - parcel_gap_m) > 0.001:
		return false
	if kind != PREVIEW_KIND_SINGLE and _preview_cache_start.distance_to(start) > 0.001:
		return false
	return _preview_cache_end.distance_to(end) < PREVIEW_REFRESH_DISTANCE_M

func _store_preview_cache(kind: int, start: Vector2, end: Vector2, mesh: Mesh) -> void:
	_preview_cache_valid = true
	_preview_cache_kind = kind
	_preview_cache_start = start
	_preview_cache_end = end
	_preview_cache_profile_runtime_id = current_profile_runtime_id
	_preview_cache_width_cells = parcel_width_cells
	_preview_cache_depth_cells = parcel_depth_cells
	_preview_cache_gap_m = parcel_gap_m
	_preview_cache_mesh = mesh

func _clear_preview_cache() -> void:
	_preview_cache_valid = false
	_preview_cache_mesh = null
	_last_valid_single_preview_mesh = null
	if preview_mesh != null:
		preview_mesh.visible = false

func _apply_preview_mesh(mesh: Mesh) -> void:
	preview_mesh.mesh = mesh
	preview_mesh.visible = mesh != null

func _apply_single_preview_mesh(mesh: Mesh) -> void:
	if mesh != null:
		_last_valid_single_preview_mesh = mesh
		_apply_preview_mesh(mesh)
	else:
		_apply_preview_mesh(_last_valid_single_preview_mesh)

func _build_parcels_mesh(payloads: Array, include_fill: bool) -> Mesh:
	if payloads.is_empty():
		return null

	var im := ImmediateMesh.new()
	if include_fill:
		im.surface_begin(Mesh.PRIMITIVE_TRIANGLES)
		for payload in payloads:
			if not (payload is Dictionary):
				continue
			var fill: Dictionary = payload
			var corners: PackedVector3Array = fill.get("corners", PackedVector3Array())
			if corners.size() != 4:
				continue
			var color: Color = fill.get("color", Color(0.7, 0.9, 0.7, 0.34))
			im.surface_set_color(color)
			im.surface_add_vertex(corners[0])
			im.surface_add_vertex(corners[1])
			im.surface_add_vertex(corners[2])
			im.surface_add_vertex(corners[0])
			im.surface_add_vertex(corners[2])
			im.surface_add_vertex(corners[3])
		im.surface_end()

	im.surface_begin(Mesh.PRIMITIVE_LINES)
	for payload in payloads:
		if not (payload is Dictionary):
			continue
		var border: Dictionary = payload
		var corners: PackedVector3Array = border.get("corners", PackedVector3Array())
		if corners.size() != 4:
			continue
		var color: Color = border.get("color", Color(0.7, 0.9, 0.7, 0.34))
		im.surface_set_color(Color(color.r, color.g, color.b, 0.88))
		for i in range(4):
			im.surface_add_vertex(corners[i])
			im.surface_add_vertex(corners[(i + 1) % 4])
	im.surface_end()
	return im

func _build_packed_parcels_mesh(payload: Dictionary, include_fill: bool) -> Mesh:
	if payload.is_empty():
		return null
	var corners: PackedVector3Array = payload.get("corners", PackedVector3Array())
	var parcel_count := int(payload.get("parcel_count", int(corners.size() / 4)))
	if parcel_count <= 0 or corners.size() < parcel_count * 4:
		return null

	var color: Color = payload.get("color", Color(0.7, 0.9, 0.7, 0.34))
	var im := ImmediateMesh.new()
	if include_fill:
		im.surface_begin(Mesh.PRIMITIVE_TRIANGLES)
		im.surface_set_color(color)
		for parcel_index in range(parcel_count):
			var base := parcel_index * 4
			im.surface_add_vertex(corners[base])
			im.surface_add_vertex(corners[base + 1])
			im.surface_add_vertex(corners[base + 2])
			im.surface_add_vertex(corners[base])
			im.surface_add_vertex(corners[base + 2])
			im.surface_add_vertex(corners[base + 3])
		im.surface_end()

	im.surface_begin(Mesh.PRIMITIVE_LINES)
	im.surface_set_color(Color(color.r, color.g, color.b, 0.88))
	for parcel_index in range(parcel_count):
		var base := parcel_index * 4
		for i in range(4):
			im.surface_add_vertex(corners[base + i])
			im.surface_add_vertex(corners[base + ((i + 1) % 4)])
	im.surface_end()
	return im

func _mouse_world_pos() -> Variant:
	var mouse_pos = get_viewport().get_mouse_position()
	var camera = get_viewport().get_camera_3d()
	if camera == null:
		return null
	var ray_origin = camera.project_ray_origin(mouse_pos)
	var ray_dir = camera.project_ray_normal(mouse_pos)
	var hit = simulation_node.intersect_world_surface(ray_origin, ray_dir)
	if hit == null:
		return null
	return Vector2(hit.x, hit.z)
