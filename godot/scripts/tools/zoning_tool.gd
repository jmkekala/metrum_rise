## Road-aligned parcel zoning tool -- sends placement points to Rust and previews returned geometry.
##
## Rust methods called: get_zone_profiles(), get_zoning_parcel_preview(),
##   apply_zoning_parcel_at(), intersect_world_surface()
extends Node3D

@onready var simulation_node = $"../SimulationNode"
@onready var zoning_overlay = $"../ZoningOverlay"

var active: bool = false
var current_profile_runtime_id: int = 0
var profiles: Array[Dictionary] = []
var profiles_by_runtime_id: Dictionary = {}

var preview_mesh: MeshInstance3D

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
		preview_mesh.visible = false
		return
	_update_preview()

func _unhandled_input(event):
	if not active:
		return

	if event is InputEventMouseButton and event.button_index == MOUSE_BUTTON_LEFT and event.pressed:
		_commit_at_mouse()

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
		current_profile_runtime_id = runtime_id

func select_profile_by_zone_type(zone_type: String) -> void:
	for profile in profiles:
		if str(profile.get("zone_type", "")).strip_edges() == zone_type:
			select_profile(int(profile.get("runtime_id", 0)))
			return

func set_paint_mode(_mode: String) -> void:
	pass

func undo() -> void:
	pass

func _commit_at_mouse() -> void:
	var wp = _mouse_world_pos()
	if wp == null:
		return
	if simulation_node.apply_zoning_parcel_at(wp.x, wp.y, current_profile_runtime_id):
		if zoning_overlay:
			zoning_overlay.mark_zone_dirty()

func _update_preview() -> void:
	var wp = _mouse_world_pos()
	if wp == null:
		preview_mesh.visible = false
		return
	var payload: Dictionary = simulation_node.get_zoning_parcel_preview(
		wp.x,
		wp.y,
		current_profile_runtime_id
	)
	if payload.is_empty():
		preview_mesh.visible = false
		return
	preview_mesh.mesh = _build_parcel_mesh(payload, true)
	preview_mesh.visible = preview_mesh.mesh != null

func _build_parcel_mesh(payload: Dictionary, include_fill: bool) -> Mesh:
	var corners: PackedVector3Array = payload.get("corners", PackedVector3Array())
	if corners.size() != 4:
		return null
	var color: Color = payload.get("color", Color(0.7, 0.9, 0.7, 0.34))
	var edge_color := Color(color.r, color.g, color.b, 0.88)

	var im := ImmediateMesh.new()
	if include_fill:
		im.surface_begin(Mesh.PRIMITIVE_TRIANGLES)
		im.surface_set_color(color)
		im.surface_add_vertex(corners[0])
		im.surface_add_vertex(corners[1])
		im.surface_add_vertex(corners[2])
		im.surface_add_vertex(corners[0])
		im.surface_add_vertex(corners[2])
		im.surface_add_vertex(corners[3])
		im.surface_end()

	im.surface_begin(Mesh.PRIMITIVE_LINES)
	im.surface_set_color(edge_color)
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
