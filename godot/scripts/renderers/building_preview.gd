## Manages the 3D preview for the building importer.
## Handles the imported GLB mesh, lot rectangle wireframe, frontage arrow,
## main entrance anchor sphere/gizmo, ground grid, human-scale reference figure,
## and ghost comparison mesh.
extends Node3D

## Emitted after a GLB mesh is successfully loaded.
## `aabb` is the model-space AABB of the imported scene root.
signal mesh_loaded(aabb: AABB)

# Zone cell size in metres — must match `WorldConfig::editor_sandbox()` (zone_cell_m = 10.0).
const CELL_M := 10.0
const ENTRANCE_SPHERE_RADIUS_M := 0.35
const GHOST_TINT := Color(0.16, 0.38, 0.95, 0.58)
const GHOST_ORIGINAL_COLOR_BLEND := 0.55

var _mesh_instance: Node3D
var _lot_overlay: MeshInstance3D
var _frontage_arrow: MeshInstance3D
var _entrance_gizmo: MeshInstance3D
var _entrance_sphere: MeshInstance3D
var _ground_grid: MeshInstance3D
var _human_figure: MeshInstance3D  # 1.8 m reference capsule
var _mesh_parts: Array[Node3D] = []

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
var entrance_anchor_local: Vector3 = Vector3(0.0, 0.0, CELL_M * 0.5)
var entrance_anchor_forward: Vector3 = Vector3.FORWARD
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

	_lot_overlay = MeshInstance3D.new()
	add_child(_lot_overlay)

	_frontage_arrow = MeshInstance3D.new()
	add_child(_frontage_arrow)

	_entrance_gizmo = MeshInstance3D.new()
	add_child(_entrance_gizmo)

	_entrance_sphere = MeshInstance3D.new()
	add_child(_entrance_sphere)

	_human_figure = MeshInstance3D.new()
	add_child(_human_figure)

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

## Update the previewed main entrance anchor.
func set_entrance_anchor(position: Vector3, forward: Vector3) -> void:
	entrance_anchor_local = position
	entrance_anchor_forward = forward.normalized()
	_rebuild_entrance_visuals()

## Returns the current main entrance anchor in world space for mouse picking.
func get_entrance_anchor_world_position() -> Vector3:
	return to_global(entrance_anchor_local)

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
	_lot_overlay.mesh = null
	_frontage_arrow.mesh = null
	_entrance_gizmo.mesh = null
	_entrance_sphere.mesh = null
	_human_figure.mesh = null

## Clear only active mesh parts. The explicit comparison ghost remains loaded.
func clear_mesh_parts() -> void:
	for child in _mesh_instance.get_children():
		child.queue_free()
	_mesh_parts.clear()

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
	_rebuild_entrance_visuals()
	_build_human_figure()

func _rebuild_entrance_visuals() -> void:
	_build_entrance_gizmo()
	_build_entrance_sphere()

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
	for ix in n_x:
		var x := start_x + ix * CELL_M
		im.surface_set_color(Color(0.25, 0.25, 0.25, 1.0))
		im.surface_add_vertex(Vector3(x, Y, start_z))
		im.surface_set_color(Color(0.25, 0.25, 0.25, 1.0))
		im.surface_add_vertex(Vector3(x, Y, end_z))
	for iz in n_z:
		var z := start_z + iz * CELL_M
		im.surface_set_color(Color(0.25, 0.25, 0.25, 1.0))
		im.surface_add_vertex(Vector3(start_x, Y, z))
		im.surface_set_color(Color(0.25, 0.25, 0.25, 1.0))
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
	im.surface_begin(Mesh.PRIMITIVE_LINES)
	for i in 4:
		im.surface_set_color(Color(0.2, 0.8, 0.2, 1.0))
		im.surface_add_vertex(corners[i])
		im.surface_add_vertex(corners[(i + 1) % 4])
	im.surface_end()
	var mat := StandardMaterial3D.new()
	mat.shading_mode = BaseMaterial3D.SHADING_MODE_UNSHADED
	mat.vertex_color_use_as_albedo = true
	im.surface_set_material(0, mat)
	_lot_overlay.mesh = im

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

	var im := ImmediateMesh.new()
	im.surface_begin(Mesh.PRIMITIVE_LINES)
	im.surface_set_color(Color(1.0, 0.5, 0.0, 1.0))
	im.surface_add_vertex(edge_center + along_edge *  edge_half + Vector3(0, y, 0))
	im.surface_add_vertex(edge_center - along_edge *  edge_half + Vector3(0, y, 0))
	for i in num_arrows:
		var t    := -edge_half + CELL_M * (i + 0.5)
		var base :=  edge_center + along_edge * t + Vector3(0, y, 0)
		var tip  :=  base + fwd * arrow_len
		var wing_l := tip - fwd * head_size + along_edge *  head_size * 0.6
		var wing_r := tip - fwd * head_size - along_edge *  head_size * 0.6
		im.surface_set_color(Color(1.0, 0.5, 0.0, 1.0))
		im.surface_add_vertex(base)
		im.surface_add_vertex(tip)
		im.surface_add_vertex(tip)
		im.surface_add_vertex(wing_l)
		im.surface_add_vertex(tip)
		im.surface_add_vertex(wing_r)
	im.surface_end()

	var mat := StandardMaterial3D.new()
	mat.shading_mode = BaseMaterial3D.SHADING_MODE_UNSHADED
	mat.vertex_color_use_as_albedo = true
	im.surface_set_material(0, mat)
	_frontage_arrow.mesh = im

func _build_entrance_gizmo() -> void:
	var base := entrance_anchor_local + Vector3(0.0, 0.08, 0.0)
	var forward := Vector3(entrance_anchor_forward.x, 0.0, entrance_anchor_forward.z)
	if forward.length_squared() < 0.001:
		forward = Vector3.FORWARD
	forward = forward.normalized()
	var side := Vector3(-forward.z, 0.0, forward.x)

	var arrow_len := CELL_M * 0.35
	var cross_len := CELL_M * 0.18
	var head_len := CELL_M * 0.12
	var head_w := CELL_M * 0.10

	var im := ImmediateMesh.new()
	im.surface_begin(Mesh.PRIMITIVE_LINES)
	im.surface_set_color(Color(1.0, 0.95, 0.2, 1.0))

	im.surface_add_vertex(base - side * cross_len)
	im.surface_add_vertex(base + side * cross_len)
	im.surface_add_vertex(base - Vector3(0.0, cross_len * 0.5, 0.0))
	im.surface_add_vertex(base + Vector3(0.0, cross_len * 0.5, 0.0))

	var tail := base - forward * (CELL_M * 0.08)
	var tip := base + forward * arrow_len
	im.surface_add_vertex(tail)
	im.surface_add_vertex(tip)
	im.surface_add_vertex(tip)
	im.surface_add_vertex(tip - forward * head_len + side * head_w)
	im.surface_add_vertex(tip)
	im.surface_add_vertex(tip - forward * head_len - side * head_w)
	im.surface_end()

	var mat := StandardMaterial3D.new()
	mat.shading_mode = BaseMaterial3D.SHADING_MODE_UNSHADED
	mat.vertex_color_use_as_albedo = true
	im.surface_set_material(0, mat)
	_entrance_gizmo.mesh = im

func _build_entrance_sphere() -> void:
	if _entrance_sphere.mesh == null:
		var sphere := SphereMesh.new()
		sphere.radius = ENTRANCE_SPHERE_RADIUS_M
		var mat := StandardMaterial3D.new()
		mat.transparency = BaseMaterial3D.TRANSPARENCY_ALPHA
		mat.shading_mode = BaseMaterial3D.SHADING_MODE_UNSHADED
		mat.albedo_color = Color(1.0, 0.88, 0.15, 0.85)
		sphere.surface_set_material(0, mat)
		_entrance_sphere.mesh = sphere
	_entrance_sphere.position = entrance_anchor_local
