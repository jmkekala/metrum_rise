## Explicit service-building placement tool with Rust-authored frontage snapping and validation.
##
## Rust methods called: get_service_building_placement_preview(),
##   place_service_building(), intersect_world_surface()
extends Node3D

@onready var simulation_node = $"../SimulationNode"
@onready var terrain_node = $"../Terrain"
@onready var buildings_node = $"../Buildings"

var active: bool = false
var selected_asset_id: String = ""

var preview_mesh: MeshInstance3D
var _preview_cache_valid: bool = false
var _preview_cache_asset_id: String = ""
var _preview_cache_pos: Vector2 = Vector2.ZERO
var _preview_cache_mesh: Mesh = null

const PREVIEW_REFRESH_DISTANCE_M := 1.0

func _ready() -> void:
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
	mat.render_priority = 9
	preview_mesh.material_override = mat
	add_child(preview_mesh)

func _process(_delta: float) -> void:
	if not active:
		_clear_preview_cache()
		return
	_update_preview()

func _unhandled_input(event) -> void:
	if not active:
		return
	if event is InputEventMouseButton and event.button_index == MOUSE_BUTTON_LEFT and event.pressed:
		_commit_at_mouse()
		get_viewport().set_input_as_handled()

func select_asset(asset_id: String) -> void:
	if selected_asset_id == asset_id:
		return
	selected_asset_id = asset_id
	_clear_preview_cache()

func _commit_at_mouse() -> void:
	if selected_asset_id.is_empty():
		return
	var wp = _mouse_world_pos()
	if wp == null:
		return
	var error: String = simulation_node.place_service_building(selected_asset_id, wp.x, wp.y)
	if not error.is_empty():
		push_warning("Service placement failed: " + error)
		_clear_preview_cache()
		return
	if buildings_node:
		buildings_node.update_all_buildings()
	if terrain_node:
		terrain_node.update_terrain_visuals()
	_clear_preview_cache()

func _update_preview() -> void:
	if selected_asset_id.is_empty():
		_clear_preview_cache()
		return
	var wp = _mouse_world_pos()
	if wp == null:
		preview_mesh.visible = false
		return
	if (
		_preview_cache_valid
		and _preview_cache_asset_id == selected_asset_id
		and _preview_cache_pos.distance_to(wp) < PREVIEW_REFRESH_DISTANCE_M
	):
		_apply_preview_mesh(_preview_cache_mesh)
		return

	var payload: Dictionary = simulation_node.get_service_building_placement_preview(
		selected_asset_id,
		wp.x,
		wp.y
	)
	var mesh: Mesh = null
	if bool(payload.get("valid", false)):
		mesh = _build_preview_mesh(payload.get("corners", PackedVector3Array()))
	_preview_cache_valid = true
	_preview_cache_asset_id = selected_asset_id
	_preview_cache_pos = wp
	_preview_cache_mesh = mesh
	_apply_preview_mesh(mesh)

func _apply_preview_mesh(mesh: Mesh) -> void:
	preview_mesh.mesh = mesh
	preview_mesh.visible = mesh != null

func _clear_preview_cache() -> void:
	_preview_cache_valid = false
	_preview_cache_mesh = null
	if preview_mesh:
		preview_mesh.visible = false

func _build_preview_mesh(corners: PackedVector3Array) -> Mesh:
	if corners.size() != 4:
		return null
	var im := ImmediateMesh.new()
	im.surface_begin(Mesh.PRIMITIVE_TRIANGLES)
	im.surface_set_color(Color(0.16, 0.72, 0.95, 0.25))
	im.surface_add_vertex(corners[0])
	im.surface_add_vertex(corners[1])
	im.surface_add_vertex(corners[2])
	im.surface_add_vertex(corners[0])
	im.surface_add_vertex(corners[2])
	im.surface_add_vertex(corners[3])
	im.surface_end()

	im.surface_begin(Mesh.PRIMITIVE_LINES)
	im.surface_set_color(Color(0.25, 0.92, 1.0, 0.9))
	for i in range(4):
		im.surface_add_vertex(corners[i])
		im.surface_add_vertex(corners[(i + 1) % 4])
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
