# SPDX-License-Identifier: GPL-2.0-only

## Manages the 3D preview for the building importer.
## Handles the imported GLB mesh, flat lot plane, frontage arrow, authored site surfaces,
## site anchors, lot cell guides, human-scale reference figure, and ghost comparison mesh.
extends Node3D

const PreviewGeometry = preload("res://scripts/editors/asset_editor/preview_geometry.gd")

## Emitted after a GLB mesh is successfully loaded.
## `aabb` is the model-space AABB of the imported scene root.

const WorldMaterials = preload("res://scripts/renderers/world_materials.gd")
const PreviewMaterials = preload("res://scripts/editors/asset_editor/preview_materials.gd")
const PickGeometry = preload("res://scripts/editors/asset_editor/mesh_pick_geometry.gd")

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
const ANCHOR_FILL_ALPHA := 0.12
const SELECTED_ANCHOR_FILL_ALPHA := 0.24
const LOT_BORDER_WIDTH_M := 0.07
const ANCHOR_BORDER_WIDTH_M := 0.10
const FRONTAGE_ARROW_LENGTH_M := 2.4
const FRONTAGE_ARROW_WIDTH_M := 1.1
const FRONTAGE_MAX_ARROWS := 3
const SELECTION_BORDER_WIDTH_M := 0.14
const SELECTION_CORNER_FRACTION := 0.22
const SELECTION_CORNER_MIN_M := 0.55
const SELECTION_CORNER_MAX_M := 2.20
const ANCHOR_DASH_M := 0.80
const ANCHOR_GAP_M := 0.40
const SITE_SURFACE_FILL_Y := 0.09
const SITE_SURFACE_GUIDE_Y := 0.105
const LOT_PLANE_Y := -0.025
const LOT_GRID_Y := 0.012
const GUIDE_HALO_ALPHA := 0.38
const GUIDE_HALO_WIDTH_MULT := 1.8
const SCALE_REFERENCE_HEIGHT := 1.8
const SCALE_REFERENCE_COLOR := Color(1.0, 0.85, 0.1)

var _mesh_instance: Node3D
var _authoring_policy := AssetAuthoringPolicy.new()
var _selection_overlay: MeshInstance3D
var _lot_plane: MeshInstance3D
var _site_surface_fill: MeshInstance3D
var _site_surface_overlay: MeshInstance3D
var _site_anchor_overlay: MeshInstance3D
var _lot_overlay: MeshInstance3D
var _frontage_arrow: MeshInstance3D
var _ground_grid: MeshInstance3D
var _scale_reference: MeshInstance3D
var _scale_reference_pick: RefCounted
var _scale_reference_placed := false
var scale_reference_selected := false
var _frontage_label: Label3D
var _site_surface_label_root: Node3D
var _site_anchor_label_root: Node3D
var _site_anchor_labels: Array[Label3D] = []
var _site_surface_labels: Array[Label3D] = []
var _mesh_parts: Array[Node3D] = []
var _part_lod_cache: Array[Dictionary] = []
var _active_lod_paths: Array[String] = []
## Changes to placement/content invalidate the automatic preview, not the simulation.
var lod_revision := 0
## Invalidates editor hover queries on geometry, active LOD or guide changes.
var pick_revision := 0
## Diagnostic BVH-cache build count, independent of pointer motion and transforms.
var pick_build_count := 0
## Diagnostic import count; warmed LOD switches must not increment this.
var lod_import_count := 0
var _emission_mode := PreviewMaterials.Mode.AUTHORED
var _emission_strength := 1.0
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
var _ghost_pick: RefCounted

var _width_cells: int = 1
var _depth_cells: int = 1

var frontage_forward: Vector3 = Vector3.FORWARD
var theme_mode: String = THEME_DARK

# ──────────────────────────────────────────────────────────────────────────────

func _ready() -> void:
	_lot_plane = MeshInstance3D.new()
	_lot_plane.cast_shadow = GeometryInstance3D.SHADOW_CASTING_SETTING_OFF
	add_child(_lot_plane)

	_ground_grid = MeshInstance3D.new()
	_ground_grid.cast_shadow = GeometryInstance3D.SHADOW_CASTING_SETTING_OFF
	add_child(_ground_grid)

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

	_build_scale_reference()

	_frontage_label = _new_overlay_label("Frontage")
	add_child(_frontage_label)

	_site_surface_label_root = Node3D.new()
	add_child(_site_surface_label_root)

	_site_anchor_label_root = Node3D.new()
	add_child(_site_anchor_label_root)

	_rebuild_overlays()

# ──────────────────────────────────────────────────────────────────────────────
# Public API
# ──────────────────────────────────────────────────────────────────────────────

## Add a GLB, GLTF, or FBX as another building mesh part and return its local AABB.
func add_mesh_part(native_path: String) -> AABB:
	var scene := _load_part_scene(native_path)
	if scene == null:
		return AABB()
	return _append_part(scene, native_path)

## Keep document/preview indices aligned when an authored source cannot be loaded.
func add_missing_mesh_part() -> AABB:
	return _append_part(Node3D.new(), "")

func _append_part(scene: Node3D, native_path: String) -> AABB:
	var part_root := Node3D.new()
	_mesh_instance.add_child(part_root)
	part_root.add_child(scene)
	_mesh_parts.append(part_root)
	var materials := PreviewMaterials.new()
	materials.capture(scene)
	materials.apply(_emission_mode, _emission_strength)
	_part_lod_cache.append({native_path: {"scene": scene, "materials": materials, "triangles": _triangle_count(scene), "picking": _build_pick_geometry(scene)}})
	_active_lod_paths.append(native_path)
	lod_revision += 1
	pick_revision += 1
	var aabb := _compute_aabb(scene as Node3D) if scene is Node3D else AABB()
	_mesh_part_aabbs.append(aabb)
	_rebuild_overlays()
	return aabb

## Swap a selected part's preview tier without recentering, rescaling or moving the camera.
## Placement/selection bounds remain those of LOD0, not the simplified tier.
func set_mesh_part_lod(part_index: int, native_path: String) -> bool:
	if part_index < 0 or part_index >= _mesh_parts.size():
		return false
	if _active_lod_paths[part_index] == native_path:
		return true
	if not _cache_lod(part_index, native_path):
		return false
	var cache := _part_lod_cache[part_index]
	if cache.has(_active_lod_paths[part_index]):
		cache[_active_lod_paths[part_index]]["scene"].visible = false
	var tier: Dictionary = cache[native_path]
	tier["scene"].visible = true
	var materials: PreviewMaterials = tier["materials"]
	materials.apply(_emission_mode, _emission_strength)
	_active_lod_paths[part_index] = native_path
	pick_revision += 1
	return true

## Preload on document/chain changes, never from the automatic camera-update path.
## Return failed filenames for explicit UI reporting; invalid tiers are not substituted.
func prepare_mesh_part_lods(part_index: int, paths: Array[String]) -> Array[String]:
	var failed: Array[String] = []
	var cache := _part_lod_cache[part_index]
	for path in paths:
		if not _cache_lod(part_index, path):
			failed.append(path)
	for path: String in cache.keys():
		if not paths.has(path):
			cache[path]["scene"].queue_free()
			cache.erase(path)
	lod_revision += 1
	pick_revision += 1
	return failed

func _cache_lod(part_index: int, path: String) -> bool:
	if _part_lod_cache[part_index].has(path):
		return true
	var scene := _load_part_scene(path) as Node3D
	if scene == null:
		return false
	scene.visible = false
	scene.process_mode = Node.PROCESS_MODE_DISABLED
	_mesh_parts[part_index].add_child(scene)
	var materials := PreviewMaterials.new()
	materials.capture(scene)
	_part_lod_cache[part_index][path] = {"scene": scene, "materials": materials, "triangles": _triangle_count(scene), "picking": _build_pick_geometry(scene)}
	return true

func _build_pick_geometry(scene: Node) -> RefCounted:
	var picking := PickGeometry.new()
	picking.build(scene)
	pick_build_count += 1
	return picking

## Exact hits against only each part's visible imported LOD, retaining nested node transforms.
func pick_mesh_parts(begin: Vector3, end: Vector3) -> Array[Dictionary]:
	var hits: Array[Dictionary] = []
	for index in _mesh_parts.size():
		var cache := _part_lod_cache[index]
		if not cache.has(_active_lod_paths[index]):
			continue
		var hit: Dictionary = cache[_active_lod_paths[index]]["picking"].intersect(begin, end)
		if not hit.is_empty():
			hit["kind"] = "mesh"
			hit["index"] = index
			hits.append(hit)
	return hits

func pick_ghost(begin: Vector3, end: Vector3) -> Dictionary:
	return _ghost_pick.intersect(begin, end) if _ghost_has_mesh and _ghost_pick != null else {}

## Current visible site label, without stale nodes awaiting queue_free after a guide rebuild.
func site_label(kind: String, index: int) -> Label3D:
	var labels := _site_anchor_labels if kind == "anchor" else _site_surface_labels
	return labels[index] if index >= 0 and index < labels.size() else null

func _site_labels_ready() -> void:
	# Label3D computes glyph bounds deferred; refresh hover once those bounds are available.
	pick_revision += 1

func mesh_part_lod_path(index: int) -> String:
	return _active_lod_paths[index]

func mesh_part_materials(index: int) -> PreviewMaterials:
	return _part_lod_cache[index].get(_active_lod_paths[index], {}).get("materials")

func mesh_part_triangles(index: int) -> int:
	var cache := _part_lod_cache[index]
	return int(cache[_active_lod_paths[index]]["triangles"]) if cache.has(_active_lod_paths[index]) else 0

## Authored source-material metadata, unaffected by preview emission overrides.
func mesh_part_source_materials(index: int, source_path: String) -> Array[Dictionary]:
	if index < 0 or index >= _part_lod_cache.size() or not _part_lod_cache[index].has(source_path):
		return []
	return _part_lod_cache[index][source_path]["materials"].source_catalog()

func mesh_part_transform(index: int) -> Transform3D:
	return _mesh_parts[index].global_transform

func _triangle_count(node: Node) -> int:
	var count := 0
	# Runtime GLTF/FBX imports produce ArrayMesh surfaces; no vertex-array copies needed.
	if node is MeshInstance3D and node.mesh is ArrayMesh:
		var mesh: ArrayMesh = node.mesh
		for surface in mesh.get_surface_count():
			var vertices := mesh.surface_get_array_index_len(surface)
			if vertices == 0:
				vertices = mesh.surface_get_array_len(surface)
			match mesh.surface_get_primitive_type(surface):
				Mesh.PRIMITIVE_TRIANGLES:
					count += vertices / 3
				Mesh.PRIMITIVE_TRIANGLE_STRIP:
					count += maxi(0, vertices - 2)
	for child in node.get_children():
		count += _triangle_count(child)
	return count

## Override emission on preview-owned materials only, including subsequently loaded tiers.
func set_preview_emission(mode: int, strength: float) -> void:
	_emission_mode = mode
	_emission_strength = strength
	for index in _mesh_parts.size():
		var materials := mesh_part_materials(index)
		if materials != null:
			materials.apply(mode, strength)

func mesh_part_count() -> int:
	return _mesh_parts.size()

func _load_part_scene(native_path: String) -> Node:
	lod_import_count += 1
	if not FileAccess.file_exists(native_path):
		return null
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
		return null
	return doc.generate_scene(state)

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
	lod_revision += 1
	pick_revision += 1
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
	pick_revision += 1
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
	pick_revision += 1
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
	pick_revision += 1
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
		_part_lod_cache.remove_at(index)
		_active_lod_paths.remove_at(index)
		_mesh_part_aabbs.remove_at(index)
		if is_instance_valid(part):
			part.queue_free()
	set_selected_mesh_parts([], -1)
	lod_revision += 1

## Update lot dimensions and rebuild overlays.
func set_lot_size(width_cells: int, depth_cells: int) -> void:
	_width_cells = maxi(1, width_cells)
	_depth_cells = maxi(1, depth_cells)
	_rebuild_overlays()

## Update frontage forward vector and rebuild the arrow.
func set_frontage_forward(fwd: Vector3) -> void:
	frontage_forward = fwd.normalized()
	_rebuild_overlays()

## Preview-only reference; visibility changes never discard its placement or pick cache.
func set_scale_reference_visible(shown: bool) -> void:
	if shown and not _scale_reference_placed:
		_place_scale_reference()
		_scale_reference_placed = true
	_scale_reference.visible = shown
	if not shown:
		set_scale_reference_selected(false)
	pick_revision += 1

func has_scale_reference() -> bool:
	return _scale_reference.is_visible_in_tree()

func set_scale_reference_selected(selected: bool) -> void:
	if scale_reference_selected == selected:
		return
	scale_reference_selected = selected
	_scale_reference.material_override.albedo_color = Color(0.2, 0.95, 1.0) if selected else SCALE_REFERENCE_COLOR
	pick_revision += 1

## The capsule centre is also the origin of its screen-sized selection handle.
func scale_reference_world_position() -> Vector3:
	return _scale_reference.global_position

## Free XZ movement, grounded at a fixed height; no lot bounds or authored-geometry snapping.
func set_scale_reference_world_position(world_pos: Vector3) -> void:
	var local := to_local(world_pos)
	_scale_reference.position = Vector3(local.x, SCALE_REFERENCE_HEIGHT * 0.5, local.z)
	pick_revision += 1

func pick_scale_reference(begin: Vector3, end: Vector3) -> Dictionary:
	return _scale_reference_pick.intersect(begin, end)

## Clear authored geometry and guides, retaining preview-only comparison/reference helpers.
func clear() -> void:
	clear_mesh_parts()
	clear_site_surfaces()
	clear_site_anchors()
	_frontage_arrow.mesh = null
	_frontage_label.visible = false

## Clear only active mesh parts. The explicit comparison ghost remains loaded.
func clear_mesh_parts() -> void:
	pick_revision += 1
	for child in _mesh_instance.get_children():
		child.queue_free()
	_mesh_parts.clear()
	_part_lod_cache.clear()
	_active_lod_paths.clear()
	lod_revision += 1
	_mesh_part_aabbs.clear()
	_selected_mesh_part_indices.clear()
	_active_mesh_part_index = -1
	if _selection_overlay:
		_selection_overlay.mesh = null

## Clear only editor-only authored site surfaces and their overlay.
func clear_site_surfaces() -> void:
	pick_revision += 1
	_site_surfaces.clear()
	_selected_site_surface_index = -1
	if _site_surface_fill:
		_site_surface_fill.mesh = null
	if _site_surface_overlay:
		_site_surface_overlay.mesh = null
	_clear_site_surface_labels()

## Clear only editor-only site anchors and their overlay.
func clear_site_anchors() -> void:
	pick_revision += 1
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
	_ghost_pick = _build_pick_geometry(scene)
	pick_revision += 1
	_apply_ghost_material(scene)
	_position_ghost()
	return true

## Current ghost root position in world space.
func get_ghost_world_position() -> Vector3:
	return _ghost_root.global_position

## Move the ghost root on the XZ plane while keeping it grounded.
func set_ghost_world_position(world_pos: Vector3) -> void:
	pick_revision += 1
	var local := to_local(world_pos)
	_ghost_root.position = Vector3(local.x, 0.0, local.z)

## Returns whether an explicit comparison ghost is loaded.
func has_ghost() -> bool:
	return _ghost_has_mesh

## Clear the explicit comparison ghost.
func clear_ghost() -> void:
	_ghost_pick = null
	pick_revision += 1
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
# Scale reference: constructed once, independent of authored overlay rebuilds.
# ──────────────────────────────────────────────────────────────────────────────

func _build_scale_reference() -> void:
	var capsule := CapsuleMesh.new()
	capsule.radius = 0.2
	capsule.height = SCALE_REFERENCE_HEIGHT
	capsule.radial_segments = 16
	capsule.rings = 4

	var mat := StandardMaterial3D.new()
	mat.albedo_color = SCALE_REFERENCE_COLOR
	mat.shading_mode = BaseMaterial3D.SHADING_MODE_UNSHADED
	_scale_reference = MeshInstance3D.new()
	_scale_reference.mesh = capsule
	_scale_reference.material_override = mat
	_scale_reference.position.y = SCALE_REFERENCE_HEIGHT * 0.5
	_scale_reference.visible = false
	add_child(_scale_reference)
	_scale_reference_pick = _build_pick_geometry(_scale_reference)

func _place_scale_reference() -> void:
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
	_scale_reference.position = Vector3(corner.x, SCALE_REFERENCE_HEIGHT * 0.5, corner.z)

# ──────────────────────────────────────────────────────────────────────────────
# Overlay builders
# ──────────────────────────────────────────────────────────────────────────────

func _rebuild_overlays() -> void:
	_build_lot_plane()
	_build_ground_grid()
	_build_lot_wireframe()
	_build_frontage_arrow()
	_build_site_surface_overlay()

func _is_light_theme() -> bool:
	return theme_mode == THEME_LIGHT

func _grid_color() -> Color:
	return Color(0.43, 0.47, 0.51, 0.20) if _is_light_theme() else Color(0.56, 0.60, 0.66, 0.18)

func _lot_plane_color() -> Color:
	return Color(0.82, 0.86, 0.88, 0.96) if _is_light_theme() else Color(0.12, 0.14, 0.15, 0.98)

func _lot_color() -> Color:
	return Color(0.35, 0.43, 0.49, 0.9) if _is_light_theme() else Color(0.64, 0.73, 0.79, 0.85)

func _frontage_color() -> Color:
	return Color(0.47, 0.32, 0.72) if _is_light_theme() else Color(0.77, 0.64, 1.0)

func _entrance_color() -> Color:
	return Color(0.10, 0.46, 0.33) if _is_light_theme() else Color(0.46, 0.88, 0.68)

func _driveway_color() -> Color:
	return Color(0.08, 0.44, 0.52) if _is_light_theme() else Color(0.44, 0.82, 0.89)

func _parking_color() -> Color:
	return Color(0.25, 0.40, 0.73) if _is_light_theme() else Color(0.56, 0.73, 1.0)

func _loading_color() -> Color:
	return Color(0.62, 0.35, 0.10) if _is_light_theme() else Color(0.96, 0.71, 0.40)

func _selected_anchor_color() -> Color:
	return Color(0.68, 0.40, 0.06) if _is_light_theme() else Color(1.0, 0.84, 0.40)

func _site_surface_color(material: String) -> Color:
	match material:
		"asphalt":
			return Color(0.38, 0.45, 0.49) if _is_light_theme() else Color(0.63, 0.70, 0.75)
		"concrete":
			return Color(0.46, 0.50, 0.51) if _is_light_theme() else Color(0.76, 0.79, 0.79)
		_:
			return Color(0.45, 0.45, 0.42, 1.0)

func _label_outline_color() -> Color:
	return Color(1.0, 1.0, 1.0, 1.0) if _is_light_theme() else Color(0.0, 0.0, 0.0, 1.0)

func _guide_halo_color() -> Color:
	return Color(0.03, 0.04, 0.06, GUIDE_HALO_ALPHA)

func _new_overlay_label(text: String) -> Label3D:
	var label := Label3D.new()
	label.text = text
	label.font_size = 28
	label.pixel_size = LABEL_PIXEL_SIZE
	label.outline_size = 3
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

func _build_lot_plane() -> void:
	if not _lot_plane:
		return
	var w := _width_cells * CELL_M
	var d := _depth_cells * CELL_M
	var vertices := PackedVector3Array([
		Vector3(-w * 0.5, LOT_PLANE_Y, -d * 0.5),
		Vector3(w * 0.5, LOT_PLANE_Y, -d * 0.5),
		Vector3(w * 0.5, LOT_PLANE_Y, d * 0.5),
		Vector3(-w * 0.5, LOT_PLANE_Y, -d * 0.5),
		Vector3(w * 0.5, LOT_PLANE_Y, d * 0.5),
		Vector3(-w * 0.5, LOT_PLANE_Y, d * 0.5),
	])
	var normals := PackedVector3Array()
	normals.resize(vertices.size())
	for i in normals.size():
		normals[i] = Vector3.UP
	var colors := PackedColorArray()
	colors.resize(vertices.size())
	var color := _lot_plane_color()
	for i in colors.size():
		colors[i] = color
	var arrays := []
	arrays.resize(Mesh.ARRAY_MAX)
	arrays[Mesh.ARRAY_VERTEX] = vertices
	arrays[Mesh.ARRAY_NORMAL] = normals
	arrays[Mesh.ARRAY_COLOR] = colors
	var mesh := ArrayMesh.new()
	mesh.add_surface_from_arrays(Mesh.PRIMITIVE_TRIANGLES, arrays)
	var mat := StandardMaterial3D.new()
	# The ground participates in lighting; editor guides remain unshaded/readable.
	mat.shading_mode = BaseMaterial3D.SHADING_MODE_PER_PIXEL
	mat.roughness = 1.0
	mat.vertex_color_use_as_albedo = true
	mat.cull_mode = BaseMaterial3D.CULL_DISABLED
	mesh.surface_set_material(0, mat)
	_lot_plane.mesh = mesh

func _build_ground_grid() -> void:
	var lot_half_w := _width_cells * CELL_M * 0.5
	var lot_half_d := _depth_cells * CELL_M * 0.5
	var start_x := -lot_half_w
	var start_z := -lot_half_d
	var end_x := lot_half_w
	var end_z := lot_half_d

	var im := ImmediateMesh.new()
	im.surface_begin(Mesh.PRIMITIVE_LINES)
	var color := _grid_color()
	for ix in range(_width_cells + 1):
		var x := start_x + float(ix) * CELL_M
		im.surface_set_color(color)
		im.surface_add_vertex(Vector3(x, LOT_GRID_Y, start_z))
		im.surface_set_color(color)
		im.surface_add_vertex(Vector3(x, LOT_GRID_Y, end_z))
	for iz in range(_depth_cells + 1):
		var z := start_z + float(iz) * CELL_M
		im.surface_set_color(color)
		im.surface_add_vertex(Vector3(start_x, LOT_GRID_Y, z))
		im.surface_set_color(color)
		im.surface_add_vertex(Vector3(end_x, LOT_GRID_Y, z))
	im.surface_end()
	var mat := StandardMaterial3D.new()
	mat.shading_mode = BaseMaterial3D.SHADING_MODE_UNSHADED
	mat.vertex_color_use_as_albedo = true
	mat.transparency = BaseMaterial3D.TRANSPARENCY_ALPHA
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
		var a: Vector3 = corners[i]
		var b: Vector3 = corners[(i + 1) % 4]
		var direction := (b - a).normalized()
		_draw_editor_line(im, a, b, color, LOT_BORDER_WIDTH_M)
		# Corner brackets identify the lot without competing with its frontage accent.
		_draw_editor_line(im, a, a + direction * 1.0, color, 0.18)
		_draw_editor_line(im, b, b - direction * 1.0, color, 0.18)
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
	var started := false
	for part_index in _selected_mesh_part_indices:
		var corners := mesh_part_world_corners(part_index)
		if corners.size() != 8:
			continue
		if corners[0].is_equal_approx(corners[7]):
			continue
		if not started:
			im.surface_begin(Mesh.PRIMITIVE_TRIANGLES)
			started = true
		var color := ACTIVE_PART_COLOR if part_index == _active_mesh_part_index else SELECTED_PART_COLOR
		var local_corners: Array[Vector3] = []
		for corner in corners:
			local_corners.append(to_local(corner))
		_draw_selection_corners(im, local_corners, color)
	if not started:
		_selection_overlay.mesh = null
		return
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
	_site_surface_labels.resize(_site_surfaces.size())

	var im := ImmediateMesh.new()
	var has_geometry := false
	var material_counts := {}
	for index in _site_surfaces.size():
		var surface := _site_surfaces[index]
		var material := str(surface.get("material", "asphalt"))
		material_counts[material] = int(material_counts.get(material, 0)) + 1
		var vertices := _site_surface_vertices(surface, SITE_SURFACE_GUIDE_Y)
		if vertices.size() < 3:
			continue
		if not has_geometry:
			im.surface_begin(Mesh.PRIMITIVE_TRIANGLES)
			has_geometry = true
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
		var label := _new_overlay_label(_site_surface_label(surface, material_counts[material]))
		_site_surface_label_root.add_child(label)
		_site_surface_labels[index] = label
		var label_pos := _site_surface_label_position(surface)
		label_pos.y += LABEL_HEIGHT_M
		_style_overlay_label(label, label.text, color, label_pos)
	if not has_geometry:
		_site_surface_overlay.mesh = null
		return
	im.surface_end()

	im.surface_set_material(0, _new_overlay_material())
	_site_surface_overlay.mesh = im
	_site_labels_ready.call_deferred()

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
	var indices := _polygon_triangles(vertices)
	for index in range(0, indices.size(), 3):
		# Godot triangulates counterclockwise in XZ; reverse for upward-facing 3D triangles.
		out.append(vertices[indices[index]])
		out.append(vertices[indices[index + 2]])
		out.append(vertices[indices[index + 1]])

func _site_surface_vertices(surface: Dictionary, y: float) -> Array:
	var result := []
	y += PreviewGeometry.number(surface, "y_m", 0.01)
	for vertex in PreviewGeometry.surface_vertices(surface):
		result.append(Vector3(vertex.x, y, vertex.y))
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
		var y := pos.y + 0.12
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
		var y := pos.y + 0.12
		var base := Vector3(pos.x, y, pos.z)
		var color := _site_anchor_color(anchor_type)
		var selected := _selected_site_anchor_indices.has(index)
		if selected:
			color = _selected_anchor_color()

		var extent := maxf(0.5, length) if anchor_type in ["parking", "loading_bay"] else maxf(1.5, width * 1.4)
		var arrow_length := minf(clampf(width * 0.7, 0.9, 2.2), extent * 0.7)
		_draw_direction_arrow(im, base + forward * extent * 0.12, forward, arrow_length, minf(width * 0.4, 0.9), color)

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
		_site_anchor_labels.append(label)
		var label_pos := base + forward * maxf(1.25, width * 0.6) + Vector3(0.0, LABEL_HEIGHT_M, 0.0)
		_style_overlay_label(label, label.text, color, label_pos)
	im.surface_end()

	im.surface_set_material(0, _new_overlay_material())
	_site_anchor_overlay.mesh = im
	_site_labels_ready.call_deferred()

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
	im.surface_set_color(color)
	for index in _polygon_triangles(vertices):
		im.surface_add_vertex(vertices[index])

## Native triangulation is shared by the physical surface and its selection highlight.
## Invalid polygons produce no triangles; never substitute a fan over a concave outline.
func _polygon_triangles(vertices: Array) -> PackedInt32Array:
	var points := PackedVector2Array()
	for vertex: Vector3 in vertices:
		points.append(Vector2(vertex.x, vertex.z))
	return Geometry2D.triangulate_polygon(points) if _authoring_policy.is_valid_site_polygon(points) else PackedInt32Array()

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

func _draw_direction_arrow(im: ImmediateMesh, base: Vector3, forward: Vector3, length_m: float, width_m: float, color: Color) -> void:
	# Two fixed-size silhouettes: soft dark keyline, then a tapered shaft and split-tone head.
	# Eight triangles total; no outline tessellation, textures or per-frame geometry updates.
	var side := Vector3(-forward.z, 0.0, forward.x)
	_draw_arrow_silhouette(im, base - forward * length_m * 0.025, forward, side, length_m * 1.05, width_m * 1.18, Color(0.03, 0.04, 0.06, 0.65), false)
	_draw_arrow_silhouette(im, base, forward, side, length_m, width_m, color, true)

func _draw_arrow_silhouette(im: ImmediateMesh, base: Vector3, forward: Vector3, side: Vector3, length_m: float, width_m: float, color: Color, shaded: bool) -> void:
	var neck := base + forward * length_m * 0.56
	var tip := base + forward * length_m
	_draw_quad_fill(im, [base - side * width_m * 0.12, base + side * width_m * 0.12,
		neck + side * width_m * 0.17, neck - side * width_m * 0.17], color)
	for sign_value in [-1.0, 1.0]:
		im.surface_set_color(color.lightened(0.14) if shaded and sign_value > 0 else color)
		im.surface_add_vertex(neck)
		im.surface_add_vertex(neck + side * width_m * 0.5 * sign_value)
		im.surface_add_vertex(tip)

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
	_site_anchor_labels.clear()
	if not _site_anchor_label_root:
		return
	for child in _site_anchor_label_root.get_children():
		child.queue_free()

func _clear_site_surface_labels() -> void:
	_site_surface_labels.clear()
	if not _site_surface_label_root:
		return
	for child in _site_surface_label_root.get_children():
		child.queue_free()

func _preview_anchor_position(anchor: Dictionary) -> Vector3:
	return PreviewGeometry.vector3(anchor.get("position"))

func _preview_anchor_forward(anchor: Dictionary) -> Vector3:
	return PreviewGeometry.forward(anchor)

func _preview_anchor_number(anchor: Dictionary, key: String, fallback: float) -> float:
	return PreviewGeometry.number(anchor, key, fallback)

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

	var y         := 0.06
	var color := _frontage_color()

	var im := ImmediateMesh.new()
	im.surface_begin(Mesh.PRIMITIVE_TRIANGLES)
	var a := edge_center - along_edge * edge_half + Vector3(0, y, 0)
	var b := edge_center + along_edge * edge_half + Vector3(0, y, 0)
	_draw_quad_fill(im, [a, b, b + fwd * 0.4, a + fwd * 0.4], Color(color, 0.15))
	_draw_editor_line(im, a, b, color, 0.18)
	# Preserve cell divisions on the edge, but keep the directional glyphs bounded on large lots.
	for i in num_arrows + 1:
		var tick := a + along_edge * CELL_M * i
		_draw_editor_line(im, tick, tick + fwd * 0.38, color, 0.10)
	var arrow_count := mini(num_arrows, FRONTAGE_MAX_ARROWS)
	for i in arrow_count:
		var t := -edge_half + edge_half * 2.0 * (i + 0.5) / arrow_count
		var base := edge_center + along_edge * t + fwd * 0.65 + Vector3(0, y, 0)
		_draw_direction_arrow(im, base, fwd, FRONTAGE_ARROW_LENGTH_M, FRONTAGE_ARROW_WIDTH_M, color)
	im.surface_end()

	im.surface_set_material(0, _new_overlay_material())
	_frontage_arrow.mesh = im
	_style_overlay_label(
		_frontage_label,
		"Frontage",
		color,
		edge_center + fwd * 4.2 + Vector3(0.0, LABEL_HEIGHT_M, 0.0)
	)
