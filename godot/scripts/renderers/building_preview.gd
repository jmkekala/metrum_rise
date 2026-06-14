## Manages the 3D preview for the building importer.
## Handles the imported GLB mesh, lot rectangle wireframe, frontage arrow,
## authored site surfaces, site anchors, ground grid, human-scale reference figure, and ghost comparison mesh.
extends Node3D

## Emitted after a GLB mesh is successfully loaded.
## `aabb` is the model-space AABB of the imported scene root.
signal mesh_loaded(aabb: AABB)

const WorldMaterials = preload("res://scripts/renderers/world_materials.gd")

# Zone cell size in metres — must match `WorldConfig::editor_sandbox()` (zone_cell_m = 10.0).
const CELL_M := 10.0
const GHOST_TINT := Color(0.16, 0.38, 0.95, 0.58)
const GHOST_ORIGINAL_COLOR_BLEND := 0.55
const SELECTED_PART_COLOR := Color(0.05, 0.85, 1.0, 0.88)
const ACTIVE_PART_COLOR := Color(1.0, 0.85, 0.12, 0.92)
const THEME_DARK := "dark"
const THEME_LIGHT := "light"
const LABEL_PIXEL_SIZE := 0.025
const LABEL_HEIGHT_M := 1.15
const ANCHOR_FILL_ALPHA := 0.30
const SELECTED_ANCHOR_FILL_ALPHA := 0.42
const LOT_BORDER_WIDTH_M := 0.14
const ANCHOR_BORDER_WIDTH_M := 0.14
const GUIDE_ARROW_WIDTH_M := 0.16
const SELECTION_BORDER_WIDTH_M := 0.14
const SELECTION_CORNER_FRACTION := 0.22
const SELECTION_CORNER_MIN_M := 0.55
const SELECTION_CORNER_MAX_M := 2.20
const GUIDE_DASH_M := 1.20
const GUIDE_GAP_M := 0.65
const ANCHOR_DASH_M := 0.55
const ANCHOR_GAP_M := 0.32
const SITE_SURFACE_FILL_Y := 0.09
const SITE_SURFACE_GUIDE_Y := 0.105
const GUIDE_HALO_ALPHA := 0.34
const GUIDE_HALO_WIDTH_MULT := 2.35

var _mesh_instance: Node3D
var _selection_overlay: MeshInstance3D
var _site_surface_fill: MeshInstance3D
var _site_surface_overlay: MeshInstance3D
var _site_anchor_overlay: MeshInstance3D
var _lot_overlay: MeshInstance3D
var _frontage_arrow: MeshInstance3D
var _ground_grid: MeshInstance3D
var _human_figure: MeshInstance3D  # 1.8 m reference capsule
var _frontage_label: Label3D
var _site_surface_label_root: Node3D
var _site_anchor_label_root: Node3D
var _mesh_parts: Array[Node3D] = []
var _mesh_part_aabbs: Array[AABB] = []
var _selected_mesh_part_indices: Array[int] = []
var _active_mesh_part_index: int = -1
var _site_anchors: Array[Dictionary] = []
var _site_surfaces: Array[Dictionary] = []
var _selected_site_anchor_indices: Array[int] = []
var _selected_site_anchor_index: int = -1
var _selected_site_surface_index: int = -1

# Ghost: explicitly selected comparison mesh shown semi-transparent.
var _ghost_root: Node3D
var _ghost_lot_width: float = 0.0
var _ghost_lot_depth: float = 0.0
var _ghost_aabb: AABB = AABB()
var _ghost_has_mesh: bool = false

var _width_cells: int = 1
var _depth_cells: int = 1

var preview_scale: float = 1.0
var frontage_forward: Vector3 = Vector3.FORWARD
var theme_mode: String = THEME_DARK
var _show_human: bool = false

# ──────────────────────────────────────────────────────────────────────────────

func _ready() -> void:
	_ground_grid = MeshInstance3D.new()
	add_child(_ground_grid)
	_build_ground_grid()

	_ghost_root = Node3D.new()
	add_child(_ghost_root)

	_mesh_instance = Node3D.new()
	add_child(_mesh_instance)

	_selection_overlay = MeshInstance3D.new()
	add_child(_selection_overlay)

	_site_surface_fill = MeshInstance3D.new()
	_site_surface_fill.cast_shadow = GeometryInstance3D.SHADOW_CASTING_SETTING_OFF
	add_child(_site_surface_fill)

	_site_surface_overlay = MeshInstance3D.new()
	add_child(_site_surface_overlay)

	_site_anchor_overlay = MeshInstance3D.new()
	add_child(_site_anchor_overlay)

	_lot_overlay = MeshInstance3D.new()
	add_child(_lot_overlay)

	_frontage_arrow = MeshInstance3D.new()
	add_child(_frontage_arrow)

	_human_figure = MeshInstance3D.new()
	add_child(_human_figure)

	_frontage_label = _new_overlay_label("Frontage")
	add_child(_frontage_label)

	_site_surface_label_root = Node3D.new()
	add_child(_site_surface_label_root)

	_site_anchor_label_root = Node3D.new()
	add_child(_site_anchor_label_root)

# ──────────────────────────────────────────────────────────────────────────────
# Public API
# ──────────────────────────────────────────────────────────────────────────────

## Load a GLB, GLTF, or FBX from an absolute native path and place it at the origin.
func load_glb(native_path: String) -> void:
	clear_mesh_parts()
	var aabb := add_mesh_part(native_path)
	emit_signal("mesh_loaded", aabb)

## Add a GLB, GLTF, or FBX as another building mesh part and return its local AABB.
func add_mesh_part(native_path: String) -> AABB:
	var ext := native_path.get_extension().to_lower()
	var doc: GLTFDocument
	var state: GLTFState
	if ext == "fbx":
		doc = FBXDocument.new()
		state = FBXState.new()
	else:
		doc = GLTFDocument.new()
		state = GLTFState.new()
	var err := doc.append_from_file(native_path, state)
	if err != OK:
		push_warning("BuildingPreview: failed to load '%s' (error %d)" % [native_path, err])
		return AABB()

	var scene: Node = doc.generate_scene(state)
	var part_root := Node3D.new()
	_mesh_instance.add_child(part_root)
	if scene:
		part_root.add_child(scene)
	_mesh_parts.append(part_root)

	_rebuild_overlays()

	var aabb := AABB()
	if scene is Node3D:
		aabb = _compute_aabb(scene as Node3D)
	_mesh_part_aabbs.append(aabb)
	return aabb

## Update one mesh part's local transform.
func set_mesh_part_transform(
	part_index: int,
	position: Vector3,
	yaw_degrees: float,
	scale_value: float,
	pivot_offset: Vector3 = Vector3.ZERO
) -> void:
	if part_index < 0 or part_index >= _mesh_parts.size():
		return
	var root := _mesh_parts[part_index]
	root.rotation_degrees = Vector3(0.0, yaw_degrees, 0.0)
	var scale := maxf(0.001, scale_value)
	root.scale = Vector3.ONE * scale
	var pivot := Basis(Vector3.UP, deg_to_rad(yaw_degrees)) * (pivot_offset * scale)
	root.position = position + pivot
	_build_selection_overlay()

## Mark the selected mesh parts with corner handles in the preview.
func set_selected_mesh_parts(indices: Array, active_index: int = -1) -> void:
	_selected_mesh_part_indices.clear()
	for raw_index in indices:
		var index := int(raw_index)
		if index >= 0 and index < _mesh_parts.size() and not _selected_mesh_part_indices.has(index):
			_selected_mesh_part_indices.append(index)
	_selected_mesh_part_indices.sort()
	_active_mesh_part_index = active_index if _selected_mesh_part_indices.has(active_index) else -1
	_build_selection_overlay()

## Replace the editor-only site anchor preview list.
func set_site_anchors(anchors: Array, selected_indices: Array = [], active_index: int = -1) -> void:
	_site_anchors.clear()
	for anchor in anchors:
		if anchor is Dictionary:
			_site_anchors.append((anchor as Dictionary).duplicate(true))
	_selected_site_anchor_indices.clear()
	for raw_index in selected_indices:
		var index := int(raw_index)
		if index >= 0 and index < _site_anchors.size() and not _selected_site_anchor_indices.has(index):
			_selected_site_anchor_indices.append(index)
	_selected_site_anchor_indices.sort()
	_selected_site_anchor_index = (
		active_index
		if _selected_site_anchor_indices.has(active_index)
		else (-1 if _selected_site_anchor_indices.is_empty() else int(_selected_site_anchor_indices[0]))
	)
	_build_site_anchor_overlay()

## Replace the editor-only authored site surface preview list.
func set_site_surfaces(surfaces: Array, active_index: int = -1) -> void:
	_site_surfaces.clear()
	for surface in surfaces:
		if surface is Dictionary:
			_site_surfaces.append((surface as Dictionary).duplicate(true))
	_selected_site_surface_index = active_index if active_index >= 0 and active_index < _site_surfaces.size() else -1
	_build_site_surface_overlay()

## Switch overlay colours for the editor preview theme.
func set_theme_mode(mode: String) -> void:
	var resolved := mode.strip_edges().to_lower()
	theme_mode = THEME_LIGHT if resolved == THEME_LIGHT else THEME_DARK
	_rebuild_overlays()
	_build_selection_overlay()
	_build_site_surface_overlay()
	_build_site_anchor_overlay()

## Return the eight world-space corners of a mesh part's transformed local AABB.
func mesh_part_world_corners(part_index: int) -> Array[Vector3]:
	var corners: Array[Vector3] = []
	if part_index < 0 or part_index >= _mesh_parts.size() or part_index >= _mesh_part_aabbs.size():
		return corners
	var aabb := _mesh_part_aabbs[part_index]
	if aabb.size.length() < 0.001:
		return corners
	var root := _mesh_parts[part_index]
	for local_corner in _aabb_corners(aabb):
		corners.append(root.to_global(local_corner))
	return corners

## Remove mesh parts by index. Indices may be unsorted; invalid entries are ignored.
func remove_mesh_parts(indices: Array) -> void:
	var resolved: Array[int] = []
	for raw_index in indices:
		var index := int(raw_index)
		if index >= 0 and index < _mesh_parts.size() and not resolved.has(index):
			resolved.append(index)
	resolved.sort()
	for i in range(resolved.size() - 1, -1, -1):
		var index := resolved[i]
		var part := _mesh_parts[index]
		_mesh_parts.remove_at(index)
		_mesh_part_aabbs.remove_at(index)
		if is_instance_valid(part):
			part.queue_free()
	set_selected_mesh_parts([], -1)

## Update lot dimensions and rebuild overlays.
func set_lot_size(width_cells: int, depth_cells: int) -> void:
	_width_cells = maxi(1, width_cells)
	_depth_cells = maxi(1, depth_cells)
	_rebuild_overlays()

## Apply a uniform scale to the mesh. Does not affect lot overlays.
func set_preview_scale(scale_value: float) -> void:
	preview_scale = maxf(0.001, scale_value)
	for part in _mesh_parts:
		part.scale = Vector3.ONE * preview_scale

## Update frontage forward vector and rebuild the arrow.
func set_frontage_forward(fwd: Vector3) -> void:
	frontage_forward = fwd.normalized()
	_rebuild_overlays()

## Show or hide the 1.8 m human reference figure.
func set_show_human(visible: bool) -> void:
	_show_human = visible
	_human_figure.visible = visible

## Place the human figure at a world XZ position, snapped to the nearest grid line.
func place_human_at(world_x: float, world_z: float) -> void:
	_human_figure.position = Vector3(world_x, 0.9, world_z)

## Clear the active mesh and overlays. The explicit comparison ghost remains loaded.
func clear() -> void:
	clear_mesh_parts()
	clear_site_surfaces()
	clear_site_anchors()
	_lot_overlay.mesh = null
	_frontage_arrow.mesh = null
	_human_figure.mesh = null
	_frontage_label.visible = false

## Clear only active mesh parts. The explicit comparison ghost remains loaded.
func clear_mesh_parts() -> void:
	for child in _mesh_instance.get_children():
		child.queue_free()
	_mesh_parts.clear()
	_mesh_part_aabbs.clear()
	_selected_mesh_part_indices.clear()
	_active_mesh_part_index = -1
	if _selection_overlay:
		_selection_overlay.mesh = null

## Clear only editor-only authored site surfaces and their overlay.
func clear_site_surfaces() -> void:
	_site_surfaces.clear()
	_selected_site_surface_index = -1
	if _site_surface_overlay:
		_site_surface_overlay.mesh = null
		_clear_site_surface_labels()

## Clear only editor-only site anchors and their overlay.
func clear_site_anchors() -> void:
	_site_anchors.clear()
	_selected_site_anchor_indices.clear()
	_selected_site_anchor_index = -1
	if _site_anchor_overlay:
		_site_anchor_overlay.mesh = null
	_clear_site_anchor_labels()

## Load an explicit comparison ghost without changing the active preview mesh.
func load_ghost(native_path: String, scale_value: float, width_cells: int, depth_cells: int) -> bool:
	var ext := native_path.get_extension().to_lower()
	var doc: GLTFDocument
	var state: GLTFState
	if ext == "fbx":
		doc = FBXDocument.new()
		state = FBXState.new()
	else:
		doc = GLTFDocument.new()
		state = GLTFState.new()
	var err := doc.append_from_file(native_path, state)
	if err != OK:
		push_warning("BuildingPreview: failed to load ghost '%s' (error %d)" % [native_path, err])
		return false

	var scene: Node = doc.generate_scene(state)
	if not scene:
		return false
	clear_ghost()
	_ghost_root.add_child(scene)
	_ghost_root.scale = Vector3.ONE * maxf(0.001, scale_value)
	_ghost_lot_width = maxi(1, width_cells) * CELL_M
	_ghost_lot_depth = maxi(1, depth_cells) * CELL_M
	if scene is Node3D:
		_ghost_aabb = _compute_aabb(scene as Node3D)
	else:
		_ghost_aabb = AABB(
			Vector3(-_ghost_lot_width * 0.5, 0.0, -_ghost_lot_depth * 0.5),
			Vector3(_ghost_lot_width, CELL_M, _ghost_lot_depth)
		)
	if _ghost_aabb.size.length() < 0.001:
		_ghost_aabb = AABB(
			Vector3(-_ghost_lot_width * 0.5, 0.0, -_ghost_lot_depth * 0.5),
			Vector3(_ghost_lot_width, CELL_M, _ghost_lot_depth)
		)
	_ghost_has_mesh = true
	_apply_ghost_material(scene)
	_position_ghost()
	return true

## Returns true when a world-space point on the ground plane is within the ghost footprint.
func ghost_contains_world_xz(world_pos: Vector3) -> bool:
	if not _ghost_has_mesh:
		return false
	var local := _ghost_root.to_local(world_pos)
	var margin := 1.0
	return (
		local.x >= _ghost_aabb.position.x - margin
		and local.x <= _ghost_aabb.position.x + _ghost_aabb.size.x + margin
		and local.z >= _ghost_aabb.position.z - margin
		and local.z <= _ghost_aabb.position.z + _ghost_aabb.size.z + margin
	)

## Current ghost root position in world space.
func get_ghost_world_position() -> Vector3:
	return _ghost_root.global_position

## Move the ghost root on the XZ plane while keeping it grounded.
func set_ghost_world_position(world_pos: Vector3) -> void:
	var local := to_local(world_pos)
	_ghost_root.position = Vector3(local.x, 0.0, local.z)

## Returns whether an explicit comparison ghost is loaded.
func has_ghost() -> bool:
	return _ghost_has_mesh

## Clear the explicit comparison ghost.
func clear_ghost() -> void:
	for child in _ghost_root.get_children():
		child.queue_free()
	_ghost_lot_width = 0.0
	_ghost_lot_depth = 0.0
	_ghost_aabb = AABB()
	_ghost_has_mesh = false

func _position_ghost() -> void:
	if not _ghost_has_mesh:
		return
	var gap := CELL_M
	var offset_x := -(_ghost_lot_width * 0.5 + gap + _width_cells * CELL_M * 0.5)
	_ghost_root.position = Vector3(offset_x, 0.0, 0.0)

# Walk the ghost subtree and replace every surface material with a stronger
# blueprint tint that remains legible in both light and dark editor themes.
func _apply_ghost_material(node: Node) -> void:
	if node is MeshInstance3D:
		var mi := node as MeshInstance3D
		for surf in mi.get_surface_override_material_count():
			var orig: Material = mi.get_active_material(surf)
			var mat := StandardMaterial3D.new()
			mat.transparency = BaseMaterial3D.TRANSPARENCY_ALPHA
			mat.shading_mode = BaseMaterial3D.SHADING_MODE_UNSHADED
			mat.albedo_color = GHOST_TINT
			if orig is StandardMaterial3D:
				# Tint the original albedo rather than replacing it entirely.
				var orig_color: Color = (orig as StandardMaterial3D).albedo_color
				mat.albedo_color = Color(
					lerpf(orig_color.r, GHOST_TINT.r, GHOST_ORIGINAL_COLOR_BLEND),
					lerpf(orig_color.g, GHOST_TINT.g, GHOST_ORIGINAL_COLOR_BLEND),
					lerpf(orig_color.b, GHOST_TINT.b, GHOST_ORIGINAL_COLOR_BLEND),
					GHOST_TINT.a)
			mi.set_surface_override_material(surf, mat)
	for child in node.get_children():
		_apply_ghost_material(child)

# ──────────────────────────────────────────────────────────────────────────────
# Human figure
# ──────────────────────────────────────────────────────────────────────────────

func _build_human_figure() -> void:
	# 1.8 m tall capsule: CapsuleMesh height = cylinder part, total = height + 2*radius.
	# radius=0.2, height=1.4 → total=1.8 m.
	var capsule := CapsuleMesh.new()
	capsule.radius = 0.2
	capsule.height = 1.4

	var mat := StandardMaterial3D.new()
	mat.albedo_color = Color(1.0, 0.85, 0.1, 0.85)
	mat.transparency = BaseMaterial3D.TRANSPARENCY_ALPHA
	mat.shading_mode = BaseMaterial3D.SHADING_MODE_UNSHADED
	capsule.surface_set_material(0, mat)

	_human_figure.mesh = capsule
	_human_figure.visible = _show_human
	_place_human_figure()

func _place_human_figure() -> void:
	# Place at the right corner of the frontage edge, just outside the lot.
	var w := _width_cells * CELL_M
	var d := _depth_cells * CELL_M
	var fwd := frontage_forward

	var edge_center: Vector3
	var along_edge: Vector3
	var edge_half: float

	if abs(fwd.x) >= abs(fwd.z):
		edge_center = Vector3(sign(fwd.x) * w * 0.5, 0.0, 0.0)
		along_edge  = Vector3(0.0, 0.0, 1.0)
		edge_half   = d * 0.5
	else:
		edge_center = Vector3(0.0, 0.0, sign(fwd.z) * d * 0.5)
		along_edge  = Vector3(1.0, 0.0, 0.0)
		edge_half   = w * 0.5

	# Right corner of the frontage edge, one step outside the lot.
	var corner := edge_center + along_edge * (edge_half + 1.5)
	_human_figure.position = Vector3(corner.x, 0.9, corner.z)  # y=0.9 centres the capsule

# ──────────────────────────────────────────────────────────────────────────────
# Overlay builders
# ──────────────────────────────────────────────────────────────────────────────

func _rebuild_overlays() -> void:
	_build_ground_grid()
	_build_lot_wireframe()
	_build_frontage_arrow()
	_build_human_figure()
	_build_site_surface_overlay()

func _is_light_theme() -> bool:
	return theme_mode == THEME_LIGHT

func _grid_color() -> Color:
	return Color(0.46, 0.49, 0.53, 0.88) if _is_light_theme() else Color(0.38, 0.40, 0.44, 0.9)

func _lot_color() -> Color:
	return Color(0.00, 0.48, 0.20, 1.0) if _is_light_theme() else Color(0.45, 1.0, 0.55, 1.0)

func _frontage_color() -> Color:
	return Color(0.62, 0.13, 0.72, 1.0) if _is_light_theme() else Color(1.0, 0.28, 0.92, 1.0)

func _entrance_color() -> Color:
	return Color(0.00, 0.44, 0.28, 1.0) if _is_light_theme() else Color(0.20, 1.0, 0.58, 1.0)

func _driveway_color() -> Color:
	return Color(0.00, 0.45, 0.52, 1.0) if _is_light_theme() else Color(0.25, 0.95, 1.0, 1.0)

func _parking_color() -> Color:
	return Color(0.10, 0.27, 0.73, 1.0) if _is_light_theme() else Color(0.38, 0.66, 1.0, 1.0)

func _loading_color() -> Color:
	return Color(0.74, 0.22, 0.03, 1.0) if _is_light_theme() else Color(1.0, 0.55, 0.18, 1.0)

func _selected_anchor_color() -> Color:
	return Color(0.08, 0.08, 0.09, 1.0) if _is_light_theme() else Color(1.0, 0.92, 0.15, 1.0)

func _site_surface_color(material: String) -> Color:
	match material:
		"asphalt":
			return Color(0.25, 0.26, 0.24, 1.0) if _is_light_theme() else Color(0.34, 0.35, 0.32, 1.0)
		"concrete":
			return Color(0.58, 0.59, 0.56, 1.0) if _is_light_theme() else Color(0.62, 0.63, 0.60, 1.0)
		_:
			return Color(0.45, 0.45, 0.42, 1.0)

func _label_outline_color() -> Color:
	return Color(1.0, 1.0, 1.0, 1.0) if _is_light_theme() else Color(0.0, 0.0, 0.0, 1.0)

func _guide_halo_color() -> Color:
	return Color(0.03, 0.04, 0.05, GUIDE_HALO_ALPHA) if _is_light_theme() else Color(1.0, 1.0, 1.0, GUIDE_HALO_ALPHA)

func _new_overlay_label(text: String) -> Label3D:
	var label := Label3D.new()
	label.text = text
	label.font_size = 28
	label.pixel_size = LABEL_PIXEL_SIZE
	label.outline_size = 8
	label.billboard = BaseMaterial3D.BILLBOARD_ENABLED
	label.no_depth_test = true
	label.horizontal_alignment = HORIZONTAL_ALIGNMENT_CENTER
	label.vertical_alignment = VERTICAL_ALIGNMENT_CENTER
	label.visible = false
	return label

func _new_overlay_material(alpha: bool = true) -> StandardMaterial3D:
	var mat := StandardMaterial3D.new()
	if alpha:
		mat.transparency = BaseMaterial3D.TRANSPARENCY_ALPHA
	mat.shading_mode = BaseMaterial3D.SHADING_MODE_UNSHADED
	mat.vertex_color_use_as_albedo = true
	mat.no_depth_test = true
	mat.cull_mode = BaseMaterial3D.CULL_DISABLED
	return mat

func _style_overlay_label(label: Label3D, text: String, color: Color, position: Vector3) -> void:
	label.text = text
	label.modulate = color
	label.outline_modulate = _label_outline_color()
	label.position = position
	label.visible = true

func _build_ground_grid() -> void:
	var lot_half_w := _width_cells * CELL_M * 0.5
	var lot_half_d := _depth_cells * CELL_M * 0.5
	var offset_x   := (_width_cells  % 2) * CELL_M * 0.5
	var offset_z   := (_depth_cells  % 2) * CELL_M * 0.5
	const MIN_CELLS := 5
	var cells_x := maxi(ceili(lot_half_w / CELL_M) + 1, MIN_CELLS)
	var cells_z := maxi(ceili(lot_half_d / CELL_M) + 1, MIN_CELLS)
	var start_x := offset_x - cells_x * CELL_M
	var start_z := offset_z - cells_z * CELL_M
	var end_x   := offset_x + cells_x * CELL_M
	var end_z   := offset_z + cells_z * CELL_M
	var n_x     := cells_x * 2 + 1
	var n_z     := cells_z * 2 + 1
	const Y := -0.01

	var im := ImmediateMesh.new()
	im.surface_begin(Mesh.PRIMITIVE_LINES)
	var color := _grid_color()
	for ix in n_x:
		var x := start_x + ix * CELL_M
		im.surface_set_color(color)
		im.surface_add_vertex(Vector3(x, Y, start_z))
		im.surface_set_color(color)
		im.surface_add_vertex(Vector3(x, Y, end_z))
	for iz in n_z:
		var z := start_z + iz * CELL_M
		im.surface_set_color(color)
		im.surface_add_vertex(Vector3(start_x, Y, z))
		im.surface_set_color(color)
		im.surface_add_vertex(Vector3(end_x,   Y, z))
	im.surface_end()
	var mat := StandardMaterial3D.new()
	mat.shading_mode = BaseMaterial3D.SHADING_MODE_UNSHADED
	mat.vertex_color_use_as_albedo = true
	im.surface_set_material(0, mat)
	_ground_grid.mesh = im

func _build_lot_wireframe() -> void:
	var w := _width_cells * CELL_M
	var d := _depth_cells * CELL_M
	var corners := [
		Vector3(-w * 0.5, 0.0,  d * 0.5),
		Vector3( w * 0.5, 0.0,  d * 0.5),
		Vector3( w * 0.5, 0.0, -d * 0.5),
		Vector3(-w * 0.5, 0.0, -d * 0.5),
	]
	var im := ImmediateMesh.new()
	im.surface_begin(Mesh.PRIMITIVE_TRIANGLES)
	var color := _lot_color()
	for i in 4:
		_draw_dashed_line(
			im,
			corners[i],
			corners[(i + 1) % 4],
			color,
			LOT_BORDER_WIDTH_M,
			GUIDE_DASH_M,
			GUIDE_GAP_M
		)
	im.surface_end()
	im.surface_set_material(0, _new_overlay_material())
	_lot_overlay.mesh = im

func _build_selection_overlay() -> void:
	if not _selection_overlay:
		return
	if _selected_mesh_part_indices.is_empty():
		_selection_overlay.mesh = null
		return

	var im := ImmediateMesh.new()
	im.surface_begin(Mesh.PRIMITIVE_TRIANGLES)
	for part_index in _selected_mesh_part_indices:
		var corners := mesh_part_world_corners(part_index)
		if corners.size() != 8:
			continue
		var color := ACTIVE_PART_COLOR if part_index == _active_mesh_part_index else SELECTED_PART_COLOR
		var local_corners: Array[Vector3] = []
		for corner in corners:
			local_corners.append(to_local(corner))
		_draw_selection_corners(im, local_corners, color)
	im.surface_end()

	im.surface_set_material(0, _new_overlay_material())
	_selection_overlay.mesh = im

func _build_site_surface_overlay() -> void:
	if not _site_surface_overlay:
		return
	_clear_site_surface_labels()
	if _site_surfaces.is_empty():
		if _site_surface_fill:
			_site_surface_fill.mesh = null
		_site_surface_overlay.mesh = null
		return

	_build_site_surface_fill()

	var im := ImmediateMesh.new()
	im.surface_begin(Mesh.PRIMITIVE_TRIANGLES)
	var material_counts := {}
	for index in _site_surfaces.size():
		var surface := _site_surfaces[index]
		var material := str(surface.get("material", "asphalt"))
		material_counts[material] = int(material_counts.get(material, 0)) + 1
		var vertices := _site_surface_vertices(surface, SITE_SURFACE_GUIDE_Y)
		if vertices.size() < 3:
			continue
		var selected := index == _selected_site_surface_index
		var color := _selected_anchor_color() if selected else _site_surface_color(material)
		for edge in vertices.size():
			if selected:
				_draw_editor_line(im, vertices[edge], vertices[(edge + 1) % vertices.size()], color, ANCHOR_BORDER_WIDTH_M)
			else:
				_draw_dashed_line(
					im,
					vertices[edge],
					vertices[(edge + 1) % vertices.size()],
					color,
					ANCHOR_BORDER_WIDTH_M,
					ANCHOR_DASH_M,
					ANCHOR_GAP_M
				)
		if selected:
			for vertex in vertices:
				_draw_site_surface_vertex_handle(im, vertex, color)
		var label := _new_overlay_label(_site_surface_label(surface, material_counts[material]))
		_site_surface_label_root.add_child(label)
		var label_pos := _site_surface_label_position(surface)
		label_pos.y = 0.08 + LABEL_HEIGHT_M
		_style_overlay_label(label, label.text, color, label_pos)
	im.surface_end()

	im.surface_set_material(0, _new_overlay_material())
	_site_surface_overlay.mesh = im

func _build_site_surface_fill() -> void:
	if not _site_surface_fill:
		return

	var triangles_by_material := {
		WorldMaterials.MATERIAL_ASPHALT: PackedVector3Array(),
		WorldMaterials.MATERIAL_CONCRETE: PackedVector3Array(),
	}

	for surface in _site_surfaces:
		var material := str(surface.get("material", WorldMaterials.MATERIAL_ASPHALT))
		if not triangles_by_material.has(material):
			material = WorldMaterials.MATERIAL_ASPHALT
		var vertices := _site_surface_vertices(surface, SITE_SURFACE_FILL_Y)
		if vertices.size() < 3:
			continue
		var material_triangles: PackedVector3Array = triangles_by_material[material]
		_append_polygon_triangles(material_triangles, vertices)
		triangles_by_material[material] = material_triangles

	var mesh := ArrayMesh.new()
	for material in [WorldMaterials.MATERIAL_ASPHALT, WorldMaterials.MATERIAL_CONCRETE]:
		var vertices: PackedVector3Array = triangles_by_material[material]
		if vertices.is_empty():
			continue
		var normals := PackedVector3Array()
		normals.resize(vertices.size())
		for i in normals.size():
			normals[i] = Vector3.UP
		var arrays := []
		arrays.resize(Mesh.ARRAY_MAX)
		arrays[Mesh.ARRAY_VERTEX] = vertices
		arrays[Mesh.ARRAY_NORMAL] = normals
		mesh.add_surface_from_arrays(Mesh.PRIMITIVE_TRIANGLES, arrays)
		mesh.surface_set_material(mesh.get_surface_count() - 1, WorldMaterials.site_surface_material(material))

	_site_surface_fill.mesh = mesh if mesh.get_surface_count() > 0 else null

func _append_polygon_triangles(out: PackedVector3Array, vertices: Array) -> void:
	if vertices.size() < 3:
		return

	var indices := []
	for i in vertices.size():
		indices.append(i)
	if _site_surface_signed_area(vertices) < 0.0:
		indices.reverse()

	var guard := 0
	while indices.size() > 3 and guard < vertices.size() * vertices.size():
		guard += 1
		var clipped := false
		for i in indices.size():
			var prev_idx := int(indices[(i + indices.size() - 1) % indices.size()])
			var current_idx := int(indices[i])
			var next_idx := int(indices[(i + 1) % indices.size()])
			var prev: Vector3 = vertices[prev_idx]
			var current: Vector3 = vertices[current_idx]
			var next: Vector3 = vertices[next_idx]
			if _site_surface_orientation(prev, current, next) <= 0.0001:
				continue

			var contains_vertex := false
			for candidate_raw in indices:
				var candidate_idx := int(candidate_raw)
				if candidate_idx == prev_idx or candidate_idx == current_idx or candidate_idx == next_idx:
					continue
				var candidate: Vector3 = vertices[candidate_idx]
				if _site_surface_point_in_triangle(candidate, prev, current, next):
					contains_vertex = true
					break
			if contains_vertex:
				continue

			_append_site_surface_triangle(out, prev, current, next)
			indices.remove_at(i)
			clipped = true
			break
		if not clipped:
			_append_polygon_fan_triangles(out, vertices)
			return

	if indices.size() == 3:
		_append_site_surface_triangle(
			out,
			vertices[int(indices[0])],
			vertices[int(indices[1])],
			vertices[int(indices[2])]
		)

func _append_polygon_fan_triangles(out: PackedVector3Array, vertices: Array) -> void:
	for i in range(1, vertices.size() - 1):
		_append_site_surface_triangle(out, vertices[0], vertices[i], vertices[i + 1])

func _append_site_surface_triangle(out: PackedVector3Array, a: Vector3, b: Vector3, c: Vector3) -> void:
	out.append(a)
	out.append(c)
	out.append(b)

func _site_surface_signed_area(vertices: Array) -> float:
	var area := 0.0
	for i in vertices.size():
		var a: Vector3 = vertices[i]
		var b: Vector3 = vertices[(i + 1) % vertices.size()]
		area += a.x * b.z - b.x * a.z
	return area * 0.5

func _site_surface_orientation(a: Vector3, b: Vector3, c: Vector3) -> float:
	return (b.x - a.x) * (c.z - a.z) - (b.z - a.z) * (c.x - a.x)

func _site_surface_point_in_triangle(p: Vector3, a: Vector3, b: Vector3, c: Vector3) -> bool:
	const EPS := 0.0001
	var ab := _site_surface_orientation(a, b, p)
	var bc := _site_surface_orientation(b, c, p)
	var ca := _site_surface_orientation(c, a, p)
	return ab >= -EPS and bc >= -EPS and ca >= -EPS

func _site_surface_vertices(surface: Dictionary, y: float) -> Array:
	var result := []
	var raw_vertices = surface.get("vertices", [])
	if raw_vertices is Array:
		for raw_vertex in raw_vertices:
			if raw_vertex is Array and raw_vertex.size() >= 2:
				result.append(Vector3(float(raw_vertex[0]), y, float(raw_vertex[1])))
	return result

func _site_surface_label_position(surface: Dictionary) -> Vector3:
	var vertices := _site_surface_vertices(surface, 0.08)
	if vertices.is_empty():
		return Vector3.ZERO
	var center := Vector3.ZERO
	for vertex in vertices:
		center += vertex
	return center / float(vertices.size())

func _site_surface_label(surface: Dictionary, material_index: int) -> String:
	var name := str(surface.get("name", "")).strip_edges()
	var material := _site_surface_label_prefix(str(surface.get("material", "")))
	if not name.is_empty():
		return "%s: %s" % [material, name]
	return "%s %d" % [material, material_index]

func _site_surface_label_prefix(material: String) -> String:
	match material:
		"asphalt":
			return "Asphalt"
		"concrete":
			return "Concrete"
		_:
			return "Surface"

func _build_site_anchor_overlay() -> void:
	if not _site_anchor_overlay:
		return
	_clear_site_anchor_labels()
	if _site_anchors.is_empty():
		_site_anchor_overlay.mesh = null
		return

	var im := ImmediateMesh.new()
	im.surface_begin(Mesh.PRIMITIVE_TRIANGLES)
	var type_counts := {}
	for index in _site_anchors.size():
		var anchor := _site_anchors[index]
		var anchor_type := str(anchor.get("anchor_type", ""))
		var pos := _preview_anchor_position(anchor)
		var forward := _preview_anchor_forward(anchor)
		var side := Vector3(-forward.z, 0.0, forward.x)
		var width := maxf(0.1, _preview_anchor_number(anchor, "width_m", 2.0))
		var length := maxf(0.0, _preview_anchor_number(anchor, "length_m", 0.0))
		var y := maxf(0.12, pos.y + 0.12)
		var base := Vector3(pos.x, y, pos.z)
		var color := _site_anchor_color(anchor_type)
		var selected := _selected_site_anchor_indices.has(index)
		var fill_alpha := SELECTED_ANCHOR_FILL_ALPHA if selected else ANCHOR_FILL_ALPHA
		if selected:
			color = _selected_anchor_color()
		var fill_color := Color(color.r, color.g, color.b, fill_alpha)
		if anchor_type == "parking" or anchor_type == "loading_bay":
			var half_w := width * 0.5
			var half_l := maxf(0.5, length) * 0.5
			var center := base + forward * half_l
			var corners := [
				center - side * half_w - forward * half_l,
				center + side * half_w - forward * half_l,
				center + side * half_w + forward * half_l,
				center - side * half_w + forward * half_l,
			]
			_draw_quad_fill(im, corners, fill_color)
		elif anchor_type == "driveway":
			var half_w := width * 0.5
			var length_m := maxf(1.5, width * 1.4)
			_draw_quad_fill(im, [
				base - side * half_w,
				base + side * half_w,
				base + side * half_w + forward * length_m,
				base - side * half_w + forward * length_m,
			], fill_color)

	for index in _site_anchors.size():
		var anchor := _site_anchors[index]
		var anchor_type := str(anchor.get("anchor_type", ""))
		type_counts[anchor_type] = int(type_counts.get(anchor_type, 0)) + 1
		var pos := _preview_anchor_position(anchor)
		var forward := _preview_anchor_forward(anchor)
		var side := Vector3(-forward.z, 0.0, forward.x)
		var width := maxf(0.1, _preview_anchor_number(anchor, "width_m", 2.0))
		var length := maxf(0.0, _preview_anchor_number(anchor, "length_m", 0.0))
		var y := maxf(0.12, pos.y + 0.12)
		var base := Vector3(pos.x, y, pos.z)
		var color := _site_anchor_color(anchor_type)
		var selected := _selected_site_anchor_indices.has(index)
		if selected:
			color = _selected_anchor_color()

		var cross := maxf(0.5, width * 0.35)
		_draw_editor_line(im, base - side * cross, base + side * cross, color, ANCHOR_BORDER_WIDTH_M)
		_draw_editor_line(im, base - forward * cross, base + forward * cross, color, ANCHOR_BORDER_WIDTH_M)
		_draw_editor_line(im, base, base + forward * maxf(1.2, width), color, GUIDE_ARROW_WIDTH_M)
		var tip := base + forward * maxf(1.2, width)
		_draw_editor_line(im, tip, tip - forward * 0.5 + side * 0.35, color, GUIDE_ARROW_WIDTH_M)
		_draw_editor_line(im, tip, tip - forward * 0.5 - side * 0.35, color, GUIDE_ARROW_WIDTH_M)

		if anchor_type == "parking" or anchor_type == "loading_bay":
			var half_w := width * 0.5
			var half_l := maxf(0.5, length) * 0.5
			var center := base + forward * half_l
			var corners := [
				center - side * half_w - forward * half_l,
				center + side * half_w - forward * half_l,
				center + side * half_w + forward * half_l,
				center - side * half_w + forward * half_l,
			]
			for edge in 4:
				_draw_site_anchor_edge(
					im,
					corners[edge],
					corners[(edge + 1) % 4],
					color,
					selected
				)
		elif anchor_type == "driveway":
			var half_w := width * 0.5
			var length_m := maxf(1.5, width * 1.4)
			_draw_site_anchor_edge(
				im,
				base - side * half_w,
				base - side * half_w + forward * length_m,
				color,
				selected
			)
			_draw_site_anchor_edge(
				im,
				base + side * half_w,
				base + side * half_w + forward * length_m,
				color,
				selected
			)
			_draw_site_anchor_edge(
				im,
				base - side * half_w + forward * length_m,
				base + side * half_w + forward * length_m,
				color,
				selected
			)
		var label := _new_overlay_label(_site_anchor_label(anchor, type_counts[anchor_type]))
		_site_anchor_label_root.add_child(label)
		var label_pos := base + forward * maxf(1.25, width * 0.6) + Vector3(0.0, LABEL_HEIGHT_M, 0.0)
		_style_overlay_label(label, label.text, color, label_pos)
	im.surface_end()

	im.surface_set_material(0, _new_overlay_material())
	_site_anchor_overlay.mesh = im

func _draw_quad_fill(im: ImmediateMesh, corners: Array, color: Color) -> void:
	if corners.size() != 4:
		return
	im.surface_set_color(color)
	im.surface_add_vertex(corners[0])
	im.surface_set_color(color)
	im.surface_add_vertex(corners[1])
	im.surface_set_color(color)
	im.surface_add_vertex(corners[2])
	im.surface_set_color(color)
	im.surface_add_vertex(corners[0])
	im.surface_set_color(color)
	im.surface_add_vertex(corners[2])
	im.surface_set_color(color)
	im.surface_add_vertex(corners[3])

func _draw_polygon_fill(im: ImmediateMesh, vertices: Array, color: Color) -> void:
	var local_points: Array[Vector2] = []
	for vertex in vertices:
		if vertex is Vector3:
			local_points.append(Vector2((vertex as Vector3).x, (vertex as Vector3).z))
	var triangles := _triangulate_polygon_indices(local_points)
	for triangle in triangles:
		for raw_index in triangle:
			var index := int(raw_index)
			if index < 0 or index >= vertices.size():
				continue
			im.surface_set_color(color)
			im.surface_add_vertex(vertices[index])

func _triangulate_polygon_indices(points: Array[Vector2]) -> Array:
	if points.size() < 3:
		return []
	var indices := []
	for i in points.size():
		indices.append(i)
	if _polygon_signed_area(points) < 0.0:
		indices.reverse()
	var triangles := []
	var guard := 0
	while indices.size() > 3 and guard < points.size() * points.size():
		guard += 1
		var clipped := false
		for i in indices.size():
			var prev := int(indices[(i + indices.size() - 1) % indices.size()])
			var current := int(indices[i])
			var next := int(indices[(i + 1) % indices.size()])
			if _orientation_2d(points[prev], points[current], points[next]) <= 0.0001:
				continue
			var contains_point := false
			for raw_candidate in indices:
				var candidate := int(raw_candidate)
				if candidate == prev or candidate == current or candidate == next:
					continue
				if _point_in_triangle_2d(points[candidate], points[prev], points[current], points[next]):
					contains_point = true
					break
			if contains_point:
				continue
			triangles.append([prev, current, next])
			indices.remove_at(i)
			clipped = true
			break
		if not clipped:
			return []
	if indices.size() == 3:
		triangles.append([int(indices[0]), int(indices[1]), int(indices[2])])
	return triangles

func _polygon_signed_area(points: Array[Vector2]) -> float:
	var twice_area := 0.0
	for i in points.size():
		var a := points[i]
		var b := points[(i + 1) % points.size()]
		twice_area += a.x * b.y - b.x * a.y
	return twice_area * 0.5

func _orientation_2d(a: Vector2, b: Vector2, c: Vector2) -> float:
	return (b.x - a.x) * (c.y - a.y) - (b.y - a.y) * (c.x - a.x)

func _point_in_triangle_2d(p: Vector2, a: Vector2, b: Vector2, c: Vector2) -> bool:
	var eps := 0.0001
	return (
		_orientation_2d(a, b, p) >= -eps
		and _orientation_2d(b, c, p) >= -eps
		and _orientation_2d(c, a, p) >= -eps
	)

func _draw_site_surface_vertex_handle(im: ImmediateMesh, vertex: Vector3, color: Color) -> void:
	var size := 0.35
	_draw_editor_line(
		im,
		vertex + Vector3(-size, 0.02, 0.0),
		vertex + Vector3(size, 0.02, 0.0),
		color,
		ANCHOR_BORDER_WIDTH_M * 0.85
	)
	_draw_editor_line(
		im,
		vertex + Vector3(0.0, 0.02, -size),
		vertex + Vector3(0.0, 0.02, size),
		color,
		ANCHOR_BORDER_WIDTH_M * 0.85
	)

func _draw_line(im: ImmediateMesh, a: Vector3, b: Vector3, color: Color) -> void:
	im.surface_set_color(color)
	im.surface_add_vertex(a)
	im.surface_set_color(color)
	im.surface_add_vertex(b)

func _draw_thick_line(im: ImmediateMesh, a: Vector3, b: Vector3, color: Color, width_m: float) -> void:
	var dir := b - a
	var side := Vector3(-dir.z, 0.0, dir.x)
	if side.length_squared() < 0.0001:
		side = Vector3.RIGHT
	else:
		side = side.normalized()
	var offset := side * maxf(0.01, width_m) * 0.5
	_draw_quad_fill(im, [
		a - offset,
		a + offset,
		b + offset,
		b - offset,
	], color)

func _draw_selection_corners(im: ImmediateMesh, corners: Array[Vector3], color: Color) -> void:
	if corners.size() != 8:
		return
	_draw_selection_corner_segment(im, corners[0], corners[1], color)
	_draw_selection_corner_segment(im, corners[0], corners[2], color)
	_draw_selection_corner_segment(im, corners[0], corners[4], color)
	_draw_selection_corner_segment(im, corners[1], corners[0], color)
	_draw_selection_corner_segment(im, corners[1], corners[3], color)
	_draw_selection_corner_segment(im, corners[1], corners[5], color)
	_draw_selection_corner_segment(im, corners[2], corners[0], color)
	_draw_selection_corner_segment(im, corners[2], corners[3], color)
	_draw_selection_corner_segment(im, corners[2], corners[6], color)
	_draw_selection_corner_segment(im, corners[3], corners[1], color)
	_draw_selection_corner_segment(im, corners[3], corners[2], color)
	_draw_selection_corner_segment(im, corners[3], corners[7], color)
	_draw_selection_corner_segment(im, corners[4], corners[0], color)
	_draw_selection_corner_segment(im, corners[4], corners[5], color)
	_draw_selection_corner_segment(im, corners[4], corners[6], color)
	_draw_selection_corner_segment(im, corners[5], corners[1], color)
	_draw_selection_corner_segment(im, corners[5], corners[4], color)
	_draw_selection_corner_segment(im, corners[5], corners[7], color)
	_draw_selection_corner_segment(im, corners[6], corners[2], color)
	_draw_selection_corner_segment(im, corners[6], corners[4], color)
	_draw_selection_corner_segment(im, corners[6], corners[7], color)
	_draw_selection_corner_segment(im, corners[7], corners[3], color)
	_draw_selection_corner_segment(im, corners[7], corners[5], color)
	_draw_selection_corner_segment(im, corners[7], corners[6], color)

func _draw_selection_corner_segment(
	im: ImmediateMesh,
	corner: Vector3,
	neighbor: Vector3,
	color: Color
) -> void:
	var edge := neighbor - corner
	var length := edge.length()
	if length <= 0.001:
		return
	var target_len := clampf(
		length * SELECTION_CORNER_FRACTION,
		SELECTION_CORNER_MIN_M,
		SELECTION_CORNER_MAX_M
	)
	var segment_len := minf(target_len, length * 0.45)
	_draw_editor_line(
		im,
		corner,
		corner + edge.normalized() * segment_len,
		color,
		SELECTION_BORDER_WIDTH_M,
		false
	)

func _draw_editor_line(
	im: ImmediateMesh,
	a: Vector3,
	b: Vector3,
	color: Color,
	width_m: float,
	with_halo: bool = true
) -> void:
	if with_halo:
		_draw_thick_line(im, a, b, _guide_halo_color(), width_m * GUIDE_HALO_WIDTH_MULT)
	_draw_thick_line(im, a, b, color, width_m)

func _draw_dashed_line(
	im: ImmediateMesh,
	a: Vector3,
	b: Vector3,
	color: Color,
	width_m: float,
	dash_m: float,
	gap_m: float
) -> void:
	var length := a.distance_to(b)
	if length <= 0.001:
		return
	var dir := (b - a) / length
	var cursor := 0.0
	var dash := maxf(0.05, dash_m)
	var gap := maxf(0.02, gap_m)
	while cursor < length:
		var end_distance := minf(cursor + dash, length)
		if end_distance > cursor:
			_draw_editor_line(
				im,
				a + dir * cursor,
				a + dir * end_distance,
				color,
				width_m
			)
		cursor = end_distance + gap

func _draw_site_anchor_edge(
	im: ImmediateMesh,
	a: Vector3,
	b: Vector3,
	color: Color,
	selected: bool
) -> void:
	if selected:
		_draw_editor_line(im, a, b, color, ANCHOR_BORDER_WIDTH_M)
	else:
		_draw_dashed_line(
			im,
			a,
			b,
			color,
			ANCHOR_BORDER_WIDTH_M,
			ANCHOR_DASH_M,
			ANCHOR_GAP_M
		)

func _clear_site_anchor_labels() -> void:
	if not _site_anchor_label_root:
		return
	for child in _site_anchor_label_root.get_children():
		child.queue_free()

func _clear_site_surface_labels() -> void:
	if not _site_surface_label_root:
		return
	for child in _site_surface_label_root.get_children():
		child.queue_free()

func _preview_anchor_position(anchor: Dictionary) -> Vector3:
	var pos = anchor.get("position", [])
	if pos is Array and pos.size() == 3:
		return Vector3(float(pos[0]), float(pos[1]), float(pos[2]))
	return Vector3.ZERO

func _preview_anchor_forward(anchor: Dictionary) -> Vector3:
	var fwd = anchor.get("forward", [])
	if fwd is Array and fwd.size() == 3:
		var resolved := Vector3(float(fwd[0]), 0.0, float(fwd[2]))
		if resolved.length_squared() > 0.001:
			return resolved.normalized()
	return Vector3.FORWARD

func _preview_anchor_number(anchor: Dictionary, key: String, fallback: float) -> float:
	var value = anchor.get(key, null)
	if value == null:
		return fallback
	var value_type := typeof(value)
	if value_type == TYPE_FLOAT or value_type == TYPE_INT:
		return float(value)
	if value is String:
		var text := (value as String).strip_edges()
		if text.is_valid_float():
			return text.to_float()
	return fallback

func _site_anchor_color(anchor_type: String) -> Color:
	match anchor_type:
		"entrance":
			return _entrance_color()
		"parking":
			return _parking_color()
		"loading_bay":
			return _loading_color()
		"driveway":
			return _driveway_color()
		_:
			return _driveway_color()

func _site_anchor_label(anchor: Dictionary, type_index: int) -> String:
	var name := str(anchor.get("name", "")).strip_edges()
	if not name.is_empty():
		return "%s: %s" % [_site_anchor_label_prefix(str(anchor.get("anchor_type", ""))), name]
	return "%s %d" % [_site_anchor_label_prefix(str(anchor.get("anchor_type", ""))), type_index]

func _site_anchor_label_prefix(anchor_type: String) -> String:
	match anchor_type:
		"entrance":
			return "Entrance"
		"parking":
			return "Parking"
		"loading_bay":
			return "Loading"
		"driveway":
			return "Driveway"
		_:
			return "Anchor"

func _aabb_corners(aabb: AABB) -> Array[Vector3]:
	var p := aabb.position
	var s := aabb.size
	return [
		Vector3(p.x, p.y, p.z),
		Vector3(p.x + s.x, p.y, p.z),
		Vector3(p.x, p.y + s.y, p.z),
		Vector3(p.x + s.x, p.y + s.y, p.z),
		Vector3(p.x, p.y, p.z + s.z),
		Vector3(p.x + s.x, p.y, p.z + s.z),
		Vector3(p.x, p.y + s.y, p.z + s.z),
		Vector3(p.x + s.x, p.y + s.y, p.z + s.z),
	]

func _compute_aabb(node: Node3D) -> AABB:
	var result := AABB()
	var first := true
	for child in node.find_children("*", "MeshInstance3D", true, false):
		var mi := child as MeshInstance3D
		if not mi or not mi.mesh:
			continue
		var rel := Transform3D.IDENTITY
		var cur: Node = mi
		while cur != node and cur != null:
			if cur is Node3D:
				rel = (cur as Node3D).transform * rel
			cur = cur.get_parent()
		var node_aabb := rel * mi.get_aabb()
		if first:
			result = node_aabb
			first = false
		else:
			result = result.merge(node_aabb)
	return result

func _build_frontage_arrow() -> void:
	var w := _width_cells  * CELL_M
	var d := _depth_cells * CELL_M
	var fwd := frontage_forward

	var edge_center: Vector3
	var along_edge: Vector3
	var edge_half: float
	var num_arrows: int

	if abs(fwd.x) >= abs(fwd.z):
		edge_center = Vector3(sign(fwd.x) * w * 0.5, 0.0, 0.0)
		along_edge  = Vector3(0.0, 0.0, 1.0)
		edge_half   = d * 0.5
		num_arrows  = _depth_cells
	else:
		edge_center = Vector3(0.0, 0.0, sign(fwd.z) * d * 0.5)
		along_edge  = Vector3(1.0, 0.0, 0.0)
		edge_half   = w * 0.5
		num_arrows  = _width_cells

	var arrow_len := CELL_M * 0.45
	var head_size := CELL_M * 0.18
	var y         := 0.06
	var color := _frontage_color()

	var im := ImmediateMesh.new()
	im.surface_begin(Mesh.PRIMITIVE_TRIANGLES)
	_draw_dashed_line(
		im,
		edge_center + along_edge * edge_half + Vector3(0, y, 0),
		edge_center - along_edge * edge_half + Vector3(0, y, 0),
		color,
		LOT_BORDER_WIDTH_M,
		GUIDE_DASH_M,
		GUIDE_GAP_M
	)
	for i in num_arrows:
		var t    := -edge_half + CELL_M * (i + 0.5)
		var base :=  edge_center + along_edge * t + Vector3(0, y, 0)
		var tip  :=  base + fwd * arrow_len
		var wing_l := tip - fwd * head_size + along_edge *  head_size * 0.6
		var wing_r := tip - fwd * head_size - along_edge *  head_size * 0.6
		_draw_editor_line(im, base, tip, color, GUIDE_ARROW_WIDTH_M)
		_draw_editor_line(im, tip, wing_l, color, GUIDE_ARROW_WIDTH_M)
		_draw_editor_line(im, tip, wing_r, color, GUIDE_ARROW_WIDTH_M)
	im.surface_end()

	im.surface_set_material(0, _new_overlay_material())
	_frontage_arrow.mesh = im
	_style_overlay_label(
		_frontage_label,
		"Frontage",
		color,
		edge_center + fwd * (CELL_M * 0.75) + Vector3(0.0, LABEL_HEIGHT_M, 0.0)
	)
