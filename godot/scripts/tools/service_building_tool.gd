# SPDX-License-Identifier: GPL-2.0-only

## Explicit service-building placement tool with Rust-authored frontage snapping and validation.
##
## Rust methods called: get_service_building_placement_preview(),
##   place_service_building(), get_world_surface_height(), intersect_world_surface()
extends Node3D

@onready var simulation_node = $"../SimulationNode"
@onready var terrain_node = $"../Terrain"
@onready var buildings_node = $"../Buildings"

var active: bool = false
var selected_asset_id: String = ""

var preview_mesh: MeshInstance3D
var preview_part_root: Node3D
var _preview_part_instances: Array[MeshInstance3D] = []
var _preview_part_mesh_cache: Dictionary = {}
var _preview_valid_material: StandardMaterial3D
var _preview_invalid_material: StandardMaterial3D
var _preview_cache_valid: bool = false
var _preview_cache_asset_id: String = ""
var _preview_cache_pos: Vector2 = Vector2.ZERO
var _preview_cache_mesh: Mesh = null
var _preview_cache_part_transforms: PackedFloat32Array = PackedFloat32Array()
var _preview_cache_placeable: bool = false
var _preview_cache_build_cost: float = 0.0
var _preview_cache_error: String = ""
var _preview_cache_label_world_pos: Vector3 = Vector3.ZERO
var _hud_canvas: CanvasLayer = null
var _price_label: Label = null
var _label_world_pos: Vector3 = Vector3.ZERO

const PREVIEW_REFRESH_DISTANCE_M := 1.0
const PREVIEW_LABEL_Y_OFFSET_M := 2.0

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
	mat.no_depth_test = false
	mat.render_priority = 6
	preview_mesh.material_override = mat
	add_child(preview_mesh)

	preview_part_root = Node3D.new()
	add_child(preview_part_root)
	_preview_valid_material = _make_ghost_material(Color(0.25, 0.95, 0.82, 0.42))
	_preview_invalid_material = _make_ghost_material(Color(1.0, 0.23, 0.12, 0.48))
	_create_preview_price_label()

func _process(_delta: float) -> void:
	if not active:
		_clear_preview_cache()
		return
	_update_preview()
	_project_preview_price_label()

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
		print("Service placement rejected: " + error)
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
		_hide_preview_visuals()
		return
	if (
		_preview_cache_valid
		and _preview_cache_asset_id == selected_asset_id
		and _preview_cache_pos.distance_to(wp) < PREVIEW_REFRESH_DISTANCE_M
	):
		_apply_preview_payload(
			_preview_cache_mesh,
			_preview_cache_part_transforms,
			_preview_cache_placeable,
			_preview_cache_build_cost,
			_preview_cache_error,
			_preview_cache_label_world_pos
		)
		return

	var payload: Dictionary = simulation_node.get_service_building_placement_preview(
		selected_asset_id,
		wp.x,
		wp.y
	)
	var mesh: Mesh = null
	var is_valid := bool(payload.get("valid", false))
	var error := str(payload.get("error", ""))
	var build_cost := float(payload.get("build_cost", 0.0))
	var corners: PackedVector3Array = payload.get("corners", PackedVector3Array())
	var part_transforms: PackedFloat32Array = payload.get("part_transforms", PackedFloat32Array())
	if corners.size() == 4:
		mesh = _build_preview_mesh(corners, is_valid)
	var label_world_pos := _preview_label_world_pos(payload, wp, corners)
	_preview_cache_valid = true
	_preview_cache_asset_id = selected_asset_id
	_preview_cache_pos = wp
	_preview_cache_mesh = mesh
	_preview_cache_part_transforms = part_transforms
	_preview_cache_placeable = is_valid
	_preview_cache_build_cost = build_cost
	_preview_cache_error = error
	_preview_cache_label_world_pos = label_world_pos
	_apply_preview_payload(mesh, part_transforms, is_valid, build_cost, error, label_world_pos)

func _apply_preview_payload(
	support_mesh: Mesh,
	part_transforms: PackedFloat32Array,
	is_valid: bool,
	build_cost: float,
	error: String,
	label_world_pos: Vector3
) -> void:
	_apply_preview_mesh(support_mesh)
	_apply_part_preview(part_transforms, is_valid)
	_apply_preview_price_label(build_cost, is_valid, error, label_world_pos)

func _apply_preview_mesh(mesh: Mesh) -> void:
	preview_mesh.mesh = mesh
	preview_mesh.visible = mesh != null

func _clear_preview_cache() -> void:
	_preview_cache_valid = false
	_preview_cache_mesh = null
	_preview_cache_part_transforms = PackedFloat32Array()
	_preview_cache_placeable = false
	_preview_cache_build_cost = 0.0
	_preview_cache_error = ""
	_preview_cache_label_world_pos = Vector3.ZERO
	_hide_preview_visuals()

func _hide_preview_visuals() -> void:
	if preview_mesh:
		preview_mesh.visible = false
	for instance in _preview_part_instances:
		instance.visible = false
	if _price_label:
		_price_label.visible = false

func _create_preview_price_label() -> void:
	_hud_canvas = CanvasLayer.new()
	_hud_canvas.layer = 10
	add_child(_hud_canvas)

	_price_label = Label.new()
	_price_label.add_theme_font_size_override("font_size", 18)
	_price_label.add_theme_color_override("font_color", Color(1.0, 1.0, 1.0, 0.95))
	_price_label.add_theme_color_override("font_shadow_color", Color(0.0, 0.0, 0.0, 0.8))
	_price_label.add_theme_constant_override("shadow_offset_x", 1)
	_price_label.add_theme_constant_override("shadow_offset_y", 1)
	_price_label.visible = false
	_hud_canvas.add_child(_price_label)

func _apply_preview_price_label(
	build_cost: float,
	is_valid: bool,
	error: String,
	label_world_pos: Vector3
) -> void:
	if not _price_label:
		return
	var price_text := "price %s" % _money(build_cost)
	if not is_valid and not error.is_empty():
		price_text += "  " + error
		_price_label.add_theme_color_override("font_color", Color(1.0, 0.28, 0.18, 0.98))
	else:
		_price_label.add_theme_color_override("font_color", Color(1.0, 1.0, 1.0, 0.95))
	_price_label.text = price_text
	_label_world_pos = label_world_pos
	_price_label.visible = true

func _project_preview_price_label() -> void:
	if not _price_label or not _price_label.visible:
		return
	var camera := get_viewport().get_camera_3d()
	if camera:
		var screen_pos: Vector2 = camera.unproject_position(_label_world_pos)
		_price_label.position = screen_pos + Vector2(8.0, -24.0)

func _preview_label_world_pos(
	payload: Dictionary,
	fallback_world_pos: Vector2,
	corners: PackedVector3Array
) -> Vector3:
	if corners.size() == 4:
		return Vector3(
			float(payload.get("center_x", fallback_world_pos.x)),
			float(payload.get("support_height_m", 0.0)) + PREVIEW_LABEL_Y_OFFSET_M,
			float(payload.get("center_z", fallback_world_pos.y))
		)
	var y := 0.0
	if simulation_node and simulation_node.has_method("get_world_surface_height"):
		y = float(simulation_node.get_world_surface_height(fallback_world_pos))
	return Vector3(fallback_world_pos.x, y + PREVIEW_LABEL_Y_OFFSET_M, fallback_world_pos.y)

func _money(value: float) -> String:
	var sign := "-" if value < 0.0 else ""
	var amount := absf(value)
	if amount >= 1000000.0:
		return "%s$%.1fM" % [sign, amount / 1000000.0]
	if amount >= 1000.0:
		return "%s$%.1fk" % [sign, amount / 1000.0]
	return "%s$%.0f" % [sign, amount]

func _apply_part_preview(part_transforms: PackedFloat32Array, is_valid: bool) -> void:
	var count := int(part_transforms.size() / 12)
	if part_transforms.size() % 12 != 0:
		count = 0
	_ensure_preview_part_instances(count)
	var material := _preview_valid_material if is_valid else _preview_invalid_material
	for part_index in range(count):
		var instance := _preview_part_instances[part_index]
		var mesh := _preview_mesh_for_part(selected_asset_id, part_index)
		if mesh == null:
			instance.visible = false
			continue
		instance.mesh = mesh
		instance.transform = _transform_from_preview_buffer(part_transforms, part_index * 12)
		instance.material_override = material
		instance.visible = true
	for part_index in range(count, _preview_part_instances.size()):
		_preview_part_instances[part_index].visible = false

func _ensure_preview_part_instances(count: int) -> void:
	while _preview_part_instances.size() < count:
		var instance := MeshInstance3D.new()
		instance.cast_shadow = GeometryInstance3D.SHADOW_CASTING_SETTING_OFF
		instance.gi_mode = GeometryInstance3D.GI_MODE_DISABLED
		instance.top_level = true
		instance.visible = false
		preview_part_root.add_child(instance)
		_preview_part_instances.append(instance)

func _preview_mesh_for_part(asset_id: String, part_index: int) -> Mesh:
	var key := "%s|part:%d" % [asset_id, part_index]
	if _preview_part_mesh_cache.has(key):
		return _preview_part_mesh_cache[key]
	var mesh: Mesh = null
	if buildings_node and buildings_node.has_method("get_building_mesh_for_asset_part"):
		mesh = buildings_node.get_building_mesh_for_asset_part(asset_id, part_index)
	_preview_part_mesh_cache[key] = mesh
	return mesh

func _transform_from_preview_buffer(buffer: PackedFloat32Array, offset: int) -> Transform3D:
	var basis := Basis(
		Vector3(buffer[offset], buffer[offset + 4], buffer[offset + 8]),
		Vector3(buffer[offset + 1], buffer[offset + 5], buffer[offset + 9]),
		Vector3(buffer[offset + 2], buffer[offset + 6], buffer[offset + 10])
	)
	var origin := Vector3(buffer[offset + 3], buffer[offset + 7], buffer[offset + 11])
	return Transform3D(basis, origin)

func _make_ghost_material(color: Color) -> StandardMaterial3D:
	var mat := StandardMaterial3D.new()
	mat.transparency = BaseMaterial3D.TRANSPARENCY_ALPHA
	mat.albedo_color = color
	mat.shading_mode = BaseMaterial3D.SHADING_MODE_PER_PIXEL
	mat.cull_mode = BaseMaterial3D.CULL_DISABLED
	mat.render_priority = 8
	return mat

func _build_preview_mesh(corners: PackedVector3Array, is_valid: bool) -> Mesh:
	if corners.size() != 4:
		return null
	var im := ImmediateMesh.new()
	var fill_color := Color(0.16, 0.72, 0.95, 0.14) if is_valid else Color(1.0, 0.18, 0.12, 0.18)
	var line_color := Color(0.25, 0.92, 1.0, 0.72) if is_valid else Color(1.0, 0.28, 0.18, 0.82)
	im.surface_begin(Mesh.PRIMITIVE_TRIANGLES)
	im.surface_set_color(fill_color)
	im.surface_add_vertex(corners[0])
	im.surface_add_vertex(corners[1])
	im.surface_add_vertex(corners[2])
	im.surface_add_vertex(corners[0])
	im.surface_add_vertex(corners[2])
	im.surface_add_vertex(corners[3])
	im.surface_end()

	im.surface_begin(Mesh.PRIMITIVE_LINES)
	im.surface_set_color(line_color)
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
