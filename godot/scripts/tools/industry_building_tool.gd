## Explicit industry area placement tool with a player-drawn production polygon.
##
## Rust methods called: get_industry_building_placement_preview(),
##   place_industry_building(), commit_extractor_polygon(), commit_field_polygon(),
##   get_world_surface_height(), intersect_world_surface()
extends Node3D

const WorldMaterials := preload("res://scripts/renderers/world_materials.gd")

@onready var simulation_node = $"../SimulationNode"
@onready var terrain_node = $"../Terrain"
@onready var buildings_node = $"../Buildings"

enum Mode { PLACE_BUILDING, DRAW_POLYGON }

var active: bool = false
var selected_asset_id: String = ""

var preview_mesh: MeshInstance3D
var preview_part_root: Node3D
var polygon_mesh: MeshInstance3D
var first_polygon_marker: MeshInstance3D
var polygon_cursor_marker: MeshInstance3D
var _preview_part_instances: Array[MeshInstance3D] = []
var _preview_part_mesh_cache: Dictionary = {}
var _preview_valid_material: StandardMaterial3D
var _preview_invalid_material: StandardMaterial3D
var _polygon_material: StandardMaterial3D
var _field_polygon_material: ShaderMaterial
var _first_polygon_marker_material: StandardMaterial3D
var _polygon_cursor_marker_material: StandardMaterial3D
var _preview_cache_valid: bool = false
var _preview_cache_asset_id: String = ""
var _preview_cache_pos: Vector2 = Vector2.ZERO
var _preview_cache_mesh: Mesh = null
var _preview_cache_part_transforms: PackedFloat32Array = PackedFloat32Array()
var _preview_cache_placeable: bool = false
var _preview_cache_build_cost: float = 0.0
var _preview_cache_error: String = ""
var _preview_cache_label_world_pos: Vector3 = Vector3.ZERO
var _mode: Mode = Mode.PLACE_BUILDING
var _pending_building_id: int = -1
var _pending_building_footprint: PackedVector2Array = PackedVector2Array()
var _pending_area_kind: String = "extractor"
var _pending_polygon_link_distance_m: float = 10.0
var _polygon_points: Array[Vector2] = []
var _notification_layer: CanvasLayer
var _notification_panel: PanelContainer
var _notification_label: Label
var _notification_hide_at_msec: int = 0
var _hud_canvas: CanvasLayer = null
var _price_label: Label = null
var _label_world_pos: Vector3 = Vector3.ZERO

const PREVIEW_REFRESH_DISTANCE_M := 1.0
const PREVIEW_LABEL_Y_OFFSET_M := 2.0
const POLYGON_LINK_DISTANCE_M := 10.0
const POLYGON_POINT_MARKER_SIZE_M := 0.8
const POLYGON_FIRST_POINT_MARKER_RADIUS_M := 1.65
const POLYGON_CURSOR_MARKER_RADIUS_M := 0.95
const POLYGON_CLOSE_RADIUS_M := 3.0
const POLYGON_SEGMENT_EPS := 0.001
const TOOL_NOTIFICATION_DURATION_SEC := 2.5
const FIELD_POLYGON_ALBEDO_PATH := "res://assets/textures/general/grain/withered_grass_diff_2k.jpg"
const FIELD_POLYGON_TEXTURE_TILE_M := 12.0
const FIELD_POLYGON_TINT := Color(0.98, 1.02, 0.90, 0.72)
const FIELD_POLYGON_FALLBACK_COLOR := Color(0.86, 0.70, 0.34, 1.0)

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

	polygon_mesh = MeshInstance3D.new()
	polygon_mesh.cast_shadow = GeometryInstance3D.SHADOW_CASTING_SETTING_OFF
	polygon_mesh.top_level = true
	polygon_mesh.visible = false
	_polygon_material = StandardMaterial3D.new()
	_polygon_material.transparency = BaseMaterial3D.TRANSPARENCY_ALPHA
	_polygon_material.shading_mode = BaseMaterial3D.SHADING_MODE_UNSHADED
	_polygon_material.cull_mode = BaseMaterial3D.CULL_DISABLED
	_polygon_material.vertex_color_use_as_albedo = true
	_polygon_material.render_priority = 9
	_field_polygon_material = WorldMaterials.field_overlay_material(
		FIELD_POLYGON_ALBEDO_PATH,
		FIELD_POLYGON_FALLBACK_COLOR,
		FIELD_POLYGON_TEXTURE_TILE_M,
		9
	)
	add_child(polygon_mesh)

	first_polygon_marker = MeshInstance3D.new()
	first_polygon_marker.cast_shadow = GeometryInstance3D.SHADOW_CASTING_SETTING_OFF
	first_polygon_marker.top_level = true
	first_polygon_marker.visible = false
	var first_marker_mesh := SphereMesh.new()
	first_marker_mesh.radius = POLYGON_FIRST_POINT_MARKER_RADIUS_M
	first_marker_mesh.height = POLYGON_FIRST_POINT_MARKER_RADIUS_M * 2.0
	first_marker_mesh.radial_segments = 24
	first_marker_mesh.rings = 12
	first_polygon_marker.mesh = first_marker_mesh
	_first_polygon_marker_material = StandardMaterial3D.new()
	_first_polygon_marker_material.transparency = BaseMaterial3D.TRANSPARENCY_ALPHA
	_first_polygon_marker_material.shading_mode = BaseMaterial3D.SHADING_MODE_UNSHADED
	_first_polygon_marker_material.albedo_color = Color(0.20, 0.74, 1.0, 1.0)
	_first_polygon_marker_material.emission_enabled = true
	_first_polygon_marker_material.emission = Color(0.55, 0.92, 1.0, 1.0)
	_first_polygon_marker_material.emission_energy_multiplier = 1.45
	_first_polygon_marker_material.render_priority = 10
	first_polygon_marker.material_override = _first_polygon_marker_material
	add_child(first_polygon_marker)

	polygon_cursor_marker = MeshInstance3D.new()
	polygon_cursor_marker.cast_shadow = GeometryInstance3D.SHADOW_CASTING_SETTING_OFF
	polygon_cursor_marker.top_level = true
	polygon_cursor_marker.visible = false
	var cursor_marker_mesh := SphereMesh.new()
	cursor_marker_mesh.radius = POLYGON_CURSOR_MARKER_RADIUS_M
	cursor_marker_mesh.height = POLYGON_CURSOR_MARKER_RADIUS_M * 2.0
	cursor_marker_mesh.radial_segments = 18
	cursor_marker_mesh.rings = 8
	polygon_cursor_marker.mesh = cursor_marker_mesh
	_polygon_cursor_marker_material = StandardMaterial3D.new()
	_polygon_cursor_marker_material.transparency = BaseMaterial3D.TRANSPARENCY_ALPHA
	_polygon_cursor_marker_material.shading_mode = BaseMaterial3D.SHADING_MODE_UNSHADED
	_polygon_cursor_marker_material.albedo_color = Color(0.76, 0.92, 1.0, 0.62)
	_polygon_cursor_marker_material.emission_enabled = true
	_polygon_cursor_marker_material.emission = Color(0.70, 0.90, 1.0, 1.0)
	_polygon_cursor_marker_material.emission_energy_multiplier = 0.45
	_polygon_cursor_marker_material.render_priority = 9
	polygon_cursor_marker.material_override = _polygon_cursor_marker_material
	add_child(polygon_cursor_marker)

	preview_part_root = Node3D.new()
	add_child(preview_part_root)
	_preview_valid_material = _make_ghost_material(Color(0.70, 0.76, 0.58, 0.44))
	_preview_invalid_material = _make_ghost_material(Color(1.0, 0.23, 0.12, 0.48))
	_create_preview_price_label()
	_create_tool_notification()

func _process(_delta: float) -> void:
	if not active:
		_reset_tool_state()
		return
	_update_tool_notification()
	if _mode == Mode.PLACE_BUILDING:
		_update_preview()
		_project_preview_price_label()
	else:
		_hide_preview_visuals()
		_update_polygon_mesh()
		_update_polygon_cursor_marker()

func _unhandled_input(event) -> void:
	if not active:
		return
	if _mode == Mode.PLACE_BUILDING:
		if event is InputEventMouseButton and event.button_index == MOUSE_BUTTON_LEFT and event.pressed:
			_commit_building_at_mouse()
			get_viewport().set_input_as_handled()
		return

	if event is InputEventMouseButton and event.pressed:
		if event.button_index == MOUSE_BUTTON_LEFT:
			_add_polygon_point_at_mouse()
			get_viewport().set_input_as_handled()
	elif event is InputEventKey and event.pressed and not event.echo:
		if event.keycode == KEY_BACKSPACE:
			if not _polygon_points.is_empty():
				_polygon_points.pop_back()
				_update_polygon_mesh()
			get_viewport().set_input_as_handled()

func select_asset(asset_id: String) -> void:
	if selected_asset_id == asset_id:
		return
	selected_asset_id = asset_id
	_reset_tool_state()

func _commit_building_at_mouse() -> void:
	if selected_asset_id.is_empty():
		return
	var wp = _mouse_world_pos()
	if wp == null:
		return
	var result: Dictionary = simulation_node.place_industry_building(selected_asset_id, wp.x, wp.y)
	if not bool(result.get("ok", false)):
		print("Industry placement rejected: " + str(result.get("error", "")))
		_clear_preview_cache()
		return
	_pending_building_id = int(result.get("building_id", -1))
	_pending_building_footprint = _footprint_from_result(
		result.get("footprint_corners", PackedVector3Array())
	)
	_pending_area_kind = str(result.get("area_kind", "extractor"))
	_pending_polygon_link_distance_m = float(
		result.get("polygon_link_distance_m", POLYGON_LINK_DISTANCE_M)
	)
	if _pending_building_id < 0:
		print("Industry placement failed: missing building id")
		_clear_preview_cache()
		return
	if buildings_node:
		buildings_node.update_all_buildings()
	if terrain_node:
		terrain_node.update_terrain_visuals()
	_mode = Mode.DRAW_POLYGON
	_polygon_points.clear()
	_clear_preview_cache()
	print("Draw production area: left-click points, then click the first point again to commit.")

func _add_polygon_point_at_mouse() -> void:
	var wp = _mouse_world_pos()
	if wp == null:
		return
	if _polygon_points.is_empty() and not _first_polygon_point_is_near_building(wp):
		_show_tool_notification(
			"Start the production area within %.0f m of the building."
			% _pending_polygon_link_distance_m
		)
		return
	if not _polygon_points.is_empty() and wp.distance_to(_polygon_points[0]) <= POLYGON_CLOSE_RADIUS_M:
		if _polygon_points.size() >= 3:
			if _would_closing_polygon_edge_cross():
				print("Production area edges cannot cross.")
				return
			_commit_polygon()
		else:
			print("Production area needs at least three points before it can be closed.")
		return
	if _would_new_polygon_edge_cross(wp):
		print("Production area edges cannot cross.")
		return
	_polygon_points.append(wp)
	_update_polygon_mesh()

func _commit_polygon() -> void:
	if _pending_building_id < 0:
		return
	if _polygon_points.size() < 3:
		print("Production area needs at least three points.")
		return
	if _would_closing_polygon_edge_cross():
		print("Production area edges cannot cross.")
		return
	var packed := PackedVector2Array()
	for point in _polygon_points:
		packed.append(point)
	var result: Dictionary
	if _pending_area_kind == "field":
		result = simulation_node.commit_field_polygon(_pending_building_id, packed)
	else:
		result = simulation_node.commit_extractor_polygon(_pending_building_id, packed)
	if not bool(result.get("ok", false)):
		print("Production area rejected: " + str(result.get("error", "")))
		return
	if _pending_area_kind == "field":
		var area_m2 := float(result.get("area_m2", 0.0))
		print("Field committed. Area: %.0f m2" % area_m2)
		if terrain_node and terrain_node.has_method("mark_field_overlay_dirty"):
			terrain_node.mark_field_overlay_dirty()
	else:
		var reserve := float(result.get("total_reserve_units", 0.0))
		if reserve <= 0.0:
			print("Extractor polygon committed with 0 reserve.")
		else:
			print("Extractor polygon committed. Reserve units: %.0f" % reserve)
		if terrain_node and terrain_node.has_method("mark_coal_pit_overlay_dirty"):
			terrain_node.mark_coal_pit_overlay_dirty()
	_pending_building_id = -1
	_pending_building_footprint = PackedVector2Array()
	_pending_area_kind = "extractor"
	_pending_polygon_link_distance_m = POLYGON_LINK_DISTANCE_M
	_polygon_points.clear()
	_mode = Mode.PLACE_BUILDING
	_update_polygon_mesh()
	_update_polygon_cursor_marker()

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

	var payload: Dictionary = simulation_node.get_industry_building_placement_preview(
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

func _reset_tool_state() -> void:
	_discard_pending_building()
	_pending_building_id = -1
	_pending_building_footprint = PackedVector2Array()
	_pending_area_kind = "extractor"
	_pending_polygon_link_distance_m = POLYGON_LINK_DISTANCE_M
	_polygon_points.clear()
	_mode = Mode.PLACE_BUILDING
	_clear_preview_cache()
	_update_polygon_mesh()
	_update_polygon_cursor_marker()
	_hide_tool_notification()

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

func _discard_pending_building() -> void:
	if _pending_building_id < 0:
		return
	if not simulation_node or not simulation_node.has_method("cancel_pending_industry_building"):
		return
	var removed := bool(simulation_node.cancel_pending_industry_building(_pending_building_id))
	if not removed:
		return
	if buildings_node:
		buildings_node.update_all_buildings()
	if terrain_node:
		terrain_node.update_terrain_visuals()

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

func _build_preview_mesh(corners: PackedVector3Array, is_valid: bool) -> Mesh:
	if corners.size() != 4:
		return null
	var im := ImmediateMesh.new()
	var fill_color := Color(0.42, 0.54, 0.30, 0.16) if is_valid else Color(1.0, 0.18, 0.12, 0.18)
	var line_color := Color(0.72, 0.84, 0.50, 0.78) if is_valid else Color(1.0, 0.28, 0.18, 0.82)
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

func _update_polygon_mesh() -> void:
	if not polygon_mesh:
		return
	var point_count := _polygon_points.size()
	_update_first_polygon_marker(point_count)
	if point_count == 0:
		polygon_mesh.visible = false
		polygon_mesh.mesh = null
		return
	var preview_points := _current_polygon_preview_points()
	if preview_points.size() < 2:
		polygon_mesh.visible = false
		polygon_mesh.mesh = null
		return
	var preview_invalid := _preview_current_click_invalid()
	var im := ImmediateMesh.new()
	var fill_color := Color(0.42, 0.82, 1.0, 0.18)
	var line_color := Color(0.62, 0.90, 1.0, 0.88)
	var vertex_marker_color := Color(0.72, 0.92, 1.0, 0.74)
	var fill_material: Material = _polygon_material
	var use_field_texture := _pending_area_kind == "field" and not preview_invalid
	if use_field_texture:
		fill_color = FIELD_POLYGON_TINT
		fill_material = _field_polygon_material
	if preview_invalid:
		fill_color = Color(1.0, 0.18, 0.12, 0.16)
		line_color = Color(1.0, 0.26, 0.18, 0.92)
	_append_polygon_fill(im, preview_points, fill_color, fill_material, use_field_texture)
	im.surface_begin(Mesh.PRIMITIVE_LINES, _polygon_material)
	im.surface_set_color(line_color)
	for i in range(preview_points.size() - 1):
		im.surface_add_vertex(_polygon_vertex(preview_points[i]))
		im.surface_add_vertex(_polygon_vertex(preview_points[i + 1]))
	if preview_points.size() >= 3:
		im.surface_add_vertex(_polygon_vertex(preview_points[preview_points.size() - 1]))
		im.surface_add_vertex(_polygon_vertex(preview_points[0]))
	im.surface_set_color(vertex_marker_color)
	for i in range(1, point_count):
		_append_polygon_point_marker(
			im,
			_polygon_vertex(_polygon_points[i]),
			POLYGON_POINT_MARKER_SIZE_M
		)
	im.surface_end()
	polygon_mesh.mesh = im
	polygon_mesh.visible = true

func _current_polygon_preview_points() -> Array[Vector2]:
	var preview_points: Array[Vector2] = []
	for point in _polygon_points:
		preview_points.append(point)
	var wp = _mouse_world_pos()
	if wp == null:
		return preview_points
	if (
		_polygon_points.size() >= 3
		and wp.distance_to(_polygon_points[0]) <= POLYGON_CLOSE_RADIUS_M
	):
		return preview_points
	if preview_points.is_empty() or wp.distance_to(preview_points[preview_points.size() - 1]) > POLYGON_SEGMENT_EPS:
		preview_points.append(wp)
	return preview_points

func _preview_current_click_invalid() -> bool:
	var wp = _mouse_world_pos()
	if wp == null:
		return false
	if _polygon_points.is_empty():
		return not _first_polygon_point_is_near_building(wp)
	if _polygon_points.size() >= 3 and wp.distance_to(_polygon_points[0]) <= POLYGON_CLOSE_RADIUS_M:
		return _would_closing_polygon_edge_cross()
	return _would_new_polygon_edge_cross(wp)

func _append_polygon_fill(
	im: ImmediateMesh,
	points: Array[Vector2],
	color: Color,
	material: Material,
	use_texture_uv: bool
) -> void:
	if points.size() < 3:
		return
	var polygon := PackedVector2Array()
	for point in points:
		polygon.append(point)
	var triangles := Geometry2D.triangulate_polygon(polygon)
	if triangles.size() < 3:
		return
	im.surface_begin(Mesh.PRIMITIVE_TRIANGLES, material)
	im.surface_set_color(color)
	for index in triangles:
		var point := points[int(index)]
		if use_texture_uv:
			im.surface_set_uv(_field_polygon_uv(point))
		im.surface_add_vertex(_polygon_vertex(point))
	im.surface_end()

func _field_polygon_uv(point: Vector2) -> Vector2:
	return Vector2(point.x, point.y) / FIELD_POLYGON_TEXTURE_TILE_M

func _update_first_polygon_marker(point_count: int) -> void:
	if not first_polygon_marker:
		return
	if point_count == 0:
		first_polygon_marker.visible = false
		return
	var vertex := _polygon_vertex(_polygon_points[0])
	first_polygon_marker.global_position = (
		vertex + Vector3(0.0, POLYGON_FIRST_POINT_MARKER_RADIUS_M, 0.0)
	)
	first_polygon_marker.visible = true

func _update_polygon_cursor_marker() -> void:
	if not polygon_cursor_marker:
		return
	if not active or _mode != Mode.DRAW_POLYGON:
		polygon_cursor_marker.visible = false
		return
	var wp = _mouse_world_pos()
	if wp == null:
		polygon_cursor_marker.visible = false
		return
	var vertex := _polygon_vertex(wp)
	polygon_cursor_marker.global_position = (
		vertex + Vector3(0.0, POLYGON_CURSOR_MARKER_RADIUS_M, 0.0)
	)
	polygon_cursor_marker.visible = true

func _footprint_from_result(corners: Variant) -> PackedVector2Array:
	var footprint := PackedVector2Array()
	if not (corners is PackedVector3Array):
		return footprint
	for corner in corners:
		footprint.append(Vector2(corner.x, corner.z))
	return footprint

func _first_polygon_point_is_near_building(point: Vector2) -> bool:
	if _pending_building_footprint.size() < 3:
		return true
	return (
		_point_distance_to_polygon(point, _pending_building_footprint)
		<= _pending_polygon_link_distance_m + POLYGON_SEGMENT_EPS
	)

func _point_distance_to_polygon(point: Vector2, polygon: PackedVector2Array) -> float:
	if _point_in_polygon(point, polygon):
		return 0.0
	var best := INF
	for i in range(polygon.size()):
		best = min(best, _point_segment_distance(point, polygon[i], polygon[(i + 1) % polygon.size()]))
	return best

func _point_segment_distance(point: Vector2, start: Vector2, end: Vector2) -> float:
	var segment := end - start
	var len_sq := segment.length_squared()
	if len_sq <= POLYGON_SEGMENT_EPS * POLYGON_SEGMENT_EPS:
		return point.distance_to(start)
	var t := clampf((point - start).dot(segment) / len_sq, 0.0, 1.0)
	return point.distance_to(start + segment * t)

func _point_in_polygon(point: Vector2, polygon: PackedVector2Array) -> bool:
	var inside := false
	var prev := polygon[polygon.size() - 1]
	for curr in polygon:
		if (
			(curr.y > point.y) != (prev.y > point.y)
			and point.x
			< (prev.x - curr.x) * (point.y - curr.y) / (prev.y - curr.y) + curr.x
		):
			inside = not inside
		prev = curr
	return inside

func _append_polygon_point_marker(im: ImmediateMesh, center: Vector3, size_m: float) -> void:
	var half_size := size_m * 0.5
	im.surface_add_vertex(center + Vector3(-half_size, 0.0, 0.0))
	im.surface_add_vertex(center + Vector3(half_size, 0.0, 0.0))
	im.surface_add_vertex(center + Vector3(0.0, 0.0, -half_size))
	im.surface_add_vertex(center + Vector3(0.0, 0.0, half_size))
	im.surface_add_vertex(center + Vector3(-half_size, 0.0, 0.0))
	im.surface_add_vertex(center + Vector3(0.0, 0.0, -half_size))
	im.surface_add_vertex(center + Vector3(0.0, 0.0, -half_size))
	im.surface_add_vertex(center + Vector3(half_size, 0.0, 0.0))
	im.surface_add_vertex(center + Vector3(half_size, 0.0, 0.0))
	im.surface_add_vertex(center + Vector3(0.0, 0.0, half_size))
	im.surface_add_vertex(center + Vector3(0.0, 0.0, half_size))
	im.surface_add_vertex(center + Vector3(-half_size, 0.0, 0.0))

func _polygon_vertex(point: Vector2) -> Vector3:
	var y := 0.12
	if simulation_node and simulation_node.has_method("get_world_surface_height"):
		y = float(simulation_node.get_world_surface_height(point)) + 0.18
	return Vector3(point.x, y, point.y)

func _would_new_polygon_edge_cross(candidate: Vector2) -> bool:
	var points: Array[Vector2] = []
	for point in _polygon_points:
		points.append(point)
	points.append(candidate)
	return _polygon_edges_cross(points, false)

func _would_closing_polygon_edge_cross() -> bool:
	return _polygon_edges_cross(_polygon_points, true)

func _polygon_edges_cross(points: Array[Vector2], closed: bool) -> bool:
	var point_count := points.size()
	var edge_count := point_count if closed else point_count - 1
	if edge_count < 2:
		return false
	for left in range(edge_count):
		var a0 := points[left]
		var a1 := points[(left + 1) % point_count]
		for right in range(left + 1, edge_count):
			if _polygon_edges_are_adjacent(left, right, edge_count, closed):
				continue
			var b0 := points[right]
			var b1 := points[(right + 1) % point_count]
			if _segments_intersect_2d(a0, a1, b0, b1):
				return true
	return false

func _polygon_edges_are_adjacent(left: int, right: int, edge_count: int, closed: bool) -> bool:
	if abs(left - right) == 1:
		return true
	return closed and left == 0 and right == edge_count - 1

func _segments_intersect_2d(a0: Vector2, a1: Vector2, b0: Vector2, b1: Vector2) -> bool:
	if max(a0.x, a1.x) < min(b0.x, b1.x) - POLYGON_SEGMENT_EPS:
		return false
	if max(b0.x, b1.x) < min(a0.x, a1.x) - POLYGON_SEGMENT_EPS:
		return false
	if max(a0.y, a1.y) < min(b0.y, b1.y) - POLYGON_SEGMENT_EPS:
		return false
	if max(b0.y, b1.y) < min(a0.y, a1.y) - POLYGON_SEGMENT_EPS:
		return false
	var d1 := _orientation_2d(a0, a1, b0)
	var d2 := _orientation_2d(a0, a1, b1)
	var d3 := _orientation_2d(b0, b1, a0)
	var d4 := _orientation_2d(b0, b1, a1)
	if abs(d1) <= POLYGON_SEGMENT_EPS and _point_on_segment_2d(b0, a0, a1):
		return true
	if abs(d2) <= POLYGON_SEGMENT_EPS and _point_on_segment_2d(b1, a0, a1):
		return true
	if abs(d3) <= POLYGON_SEGMENT_EPS and _point_on_segment_2d(a0, b0, b1):
		return true
	if abs(d4) <= POLYGON_SEGMENT_EPS and _point_on_segment_2d(a1, b0, b1):
		return true
	return ((d1 > 0.0 and d2 < 0.0) or (d1 < 0.0 and d2 > 0.0)) and (
		(d3 > 0.0 and d4 < 0.0) or (d3 < 0.0 and d4 > 0.0)
	)

func _orientation_2d(a: Vector2, b: Vector2, c: Vector2) -> float:
	return (b.x - a.x) * (c.y - a.y) - (b.y - a.y) * (c.x - a.x)

func _point_on_segment_2d(point: Vector2, start: Vector2, end: Vector2) -> bool:
	return (
		point.x >= min(start.x, end.x) - POLYGON_SEGMENT_EPS
		and point.x <= max(start.x, end.x) + POLYGON_SEGMENT_EPS
		and point.y >= min(start.y, end.y) - POLYGON_SEGMENT_EPS
		and point.y <= max(start.y, end.y) + POLYGON_SEGMENT_EPS
	)

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

func _create_tool_notification() -> void:
	_notification_layer = CanvasLayer.new()
	_notification_layer.layer = 90
	add_child(_notification_layer)

	var root := Control.new()
	root.mouse_filter = Control.MOUSE_FILTER_IGNORE
	root.set_anchors_and_offsets_preset(Control.PRESET_FULL_RECT)
	_notification_layer.add_child(root)

	_notification_panel = PanelContainer.new()
	_notification_panel.visible = false
	_notification_panel.mouse_filter = Control.MOUSE_FILTER_IGNORE
	_notification_panel.custom_minimum_size = Vector2(520.0, 48.0)
	_notification_panel.anchor_left = 0.5
	_notification_panel.anchor_right = 0.5
	_notification_panel.anchor_top = 0.0
	_notification_panel.anchor_bottom = 0.0
	_notification_panel.offset_left = -260.0
	_notification_panel.offset_right = 260.0
	_notification_panel.offset_top = 58.0
	_notification_panel.offset_bottom = 106.0
	var panel_style := StyleBoxFlat.new()
	panel_style.bg_color = Color(0.10, 0.11, 0.10, 0.84)
	panel_style.border_color = Color(0.74, 0.66, 0.36, 0.78)
	panel_style.set_border_width_all(1)
	panel_style.corner_radius_top_left = 6
	panel_style.corner_radius_top_right = 6
	panel_style.corner_radius_bottom_left = 6
	panel_style.corner_radius_bottom_right = 6
	_notification_panel.add_theme_stylebox_override("panel", panel_style)
	root.add_child(_notification_panel)

	var margin := MarginContainer.new()
	margin.add_theme_constant_override("margin_left", 14)
	margin.add_theme_constant_override("margin_right", 14)
	margin.add_theme_constant_override("margin_top", 8)
	margin.add_theme_constant_override("margin_bottom", 8)
	_notification_panel.add_child(margin)

	_notification_label = Label.new()
	_notification_label.horizontal_alignment = HORIZONTAL_ALIGNMENT_CENTER
	_notification_label.vertical_alignment = VERTICAL_ALIGNMENT_CENTER
	_notification_label.autowrap_mode = TextServer.AUTOWRAP_WORD_SMART
	_notification_label.add_theme_color_override("font_color", Color(0.98, 0.92, 0.72, 1.0))
	_notification_label.add_theme_font_size_override("font_size", 16)
	margin.add_child(_notification_label)

func _show_tool_notification(message: String) -> void:
	print(message)
	if not _notification_panel or not _notification_label:
		return
	_notification_label.text = message
	_notification_panel.visible = true
	_notification_hide_at_msec = Time.get_ticks_msec() + int(TOOL_NOTIFICATION_DURATION_SEC * 1000.0)

func _hide_tool_notification() -> void:
	_notification_hide_at_msec = 0
	if _notification_panel:
		_notification_panel.visible = false

func _update_tool_notification() -> void:
	if (
		_notification_panel
		and _notification_panel.visible
		and _notification_hide_at_msec > 0
		and Time.get_ticks_msec() >= _notification_hide_at_msec
	):
		_hide_tool_notification()
