# SPDX-License-Identifier: GPL-2.0-only

## Cosmetic GPU grass window. Reuses resident terrain textures and mesh coverage; never places
## simulation vegetation. CPU work is bounded by the four patches touching the 140 m window, each
## drawn by a far layer and a denser near layer.
extends Node3D

const GameSettings := preload("res://scripts/core/game_settings.gd")
const GRASS_SHADER := preload("res://scripts/shaders/grass.gdshader")
const MASK_SHADER := preload("res://scripts/shaders/grass_surface_mask.gdshader")
const GRADED_MASK_SHADER := preload("res://scripts/shaders/grass_graded_mask.gdshader")
const CELL_M := 0.8
# Farthest a clump draws (grain; meadow fades out by 60 m, see grass.gdshader). The vertex shader
# runs for every cell of the window whether its clump draws or not, so the window is no larger
# than the fade, and it shrinks as the camera rises: a camera RADIUS_M above the ground draws
# none, and the terrain's wind waves (grass_wind.gdshaderinc) carry the wind from there.
const RADIUS_M := 70.0
const WINDOW_CELLS := 177
# Near layer: a second grid of clumps at half the spacing, between the far layer's, so the
# ground reads as turf rather than tufts where the camera can see single blades. It fades out
# over NEAR_FADE_M, inside its window, and costs only its own few thousand instances.
const NEAR_CELL_M := 0.5
const NEAR_RADIUS_M := 24.0
const NEAR_FADE_M := Vector2(14.0, 22.0)
const NEAR_WINDOW_CELLS := 97
const NEAR_LAYER_SALT := 1
const MASK_SIZE := 1024
const PATCH_SLOTS := 4
# Copy bindings, never images. Changes include water uploads, crown edits and pooled materials.
const TERRAIN_PARAMETERS := [
	&"heightmap", &"height_scale", &"heightmap_texture_size", &"inner_sample_offset_texels",
	&"inner_sample_size_texels", &"patch_world_size_m", &"terrain_cell_m",
	&"watermap", &"watermap_texture_size", &"watermap_inner_sample_offset_texels",
	&"watermap_inner_sample_size_texels", &"land_cover_texture", &"land_cover_world_bounds",
	&"field_overlay_texture", &"field_overlay_world_bounds", &"coal_pit_texture",
	&"coal_pit_overlay_world_bounds", &"terrain_grain_albedo", &"terrain_grain_detail_scale",
	&"terrain_grain_tint", &"terrain_grain_brightness", &"terrain_grain_mix_strength",
	&"terrain_grain_macro_strength", &"terrain_grass_albedo", &"terrain_grass_height",
	&"ground_shadow_ambient", &"canopy_floor_sky_transmission",
]

@onready var _terrain = $"../Terrain"
@onready var _simulation = get_node_or_null("../SimulationNode")
var _draws: Array[MultiMeshInstance3D] = []
# Per slot, the near layer's draw over the same patch; index-aligned with _draws.
var _near_draws: Array[MultiMeshInstance3D] = []
var _masks: Array[SubViewport] = []
var _mask_meshes: Array[MeshInstance3D] = []
# Per mask slot, mirrors of the graded renderer's five cell MultiMeshes. They share the cell
# buffers and bind only the grading textures needed to classify grass roots.
var _graded_mask_cells: Array[Array] = []
var _graded_mask_revisions: Array[int] = []
var _bindings: Array[Dictionary] = []
var _keys: Array[Vector2i] = []

func _ready() -> void:
	if not GameSettings.GRASS_ENABLED:
		set_process(false)
		return
	var mesh := _clump_mesh()
	for slot in PATCH_SLOTS:
		_create_mask()
		var draw := _create_draw(mesh, RADIUS_M, WINDOW_CELLS, slot)
		_draws.append(draw)
		var near := _create_draw(mesh, NEAR_RADIUS_M, NEAR_WINDOW_CELLS, slot)
		var near_material := near.material_override as ShaderMaterial
		near_material.set_shader_parameter("cell_m", NEAR_CELL_M)
		near_material.set_shader_parameter("layer_salt", NEAR_LAYER_SALT)
		near_material.set_shader_parameter("near_fade_m", NEAR_FADE_M)
		_near_draws.append(near)
		_keys.append(Vector2i(-1, -1))
		_graded_mask_revisions.append(-1)
	_bindings.resize(PATCH_SLOTS * 2)
	for index in _bindings.size():
		_bindings[index] = {}

func _create_draw(
	mesh: Mesh, radius_m: float, window_cells: int, slot: int
) -> MultiMeshInstance3D:
	var draw := MultiMeshInstance3D.new()
	draw.cast_shadow = GeometryInstance3D.SHADOW_CASTING_SETTING_OFF
	# Shader height is not represented in the small source mesh.
	draw.custom_aabb = AABB(Vector3(-radius_m - 2.0, -1200, -radius_m - 2.0),
		Vector3(2.0 * radius_m + 4.0, 4400, 2.0 * radius_m + 4.0))
	draw.multimesh = MultiMesh.new()
	draw.multimesh.transform_format = MultiMesh.TRANSFORM_3D
	draw.multimesh.mesh = mesh
	draw.multimesh.instance_count = window_cells * window_cells
	draw.multimesh.visible_instance_count = 0
	# Static identity buffers only; the vertex shader owns every instance position.
	var buffer := PackedFloat32Array()
	buffer.resize(window_cells * window_cells * 12)
	for instance in window_cells * window_cells:
		buffer[instance * 12] = 1.0
		buffer[instance * 12 + 5] = 1.0
		buffer[instance * 12 + 10] = 1.0
	draw.multimesh.buffer = buffer
	var material := ShaderMaterial.new()
	material.shader = GRASS_SHADER
	material.set_shader_parameter("surface_mask", _masks[slot].get_texture())
	draw.material_override = material
	add_child(draw)
	return draw

func _process(_delta: float) -> void:
	var camera := get_viewport().get_camera_3d()
	if camera == null:
		return
	var world_size: Vector2 = _terrain.terrain_world_size
	var span: float = _terrain.patch_span_m
	# Current terrain patches span >= 255 m. Smaller layouts need more binding slots.
	if span < RADIUS_M * 2.0 + CELL_M * 2.0:
		visible = false
		return
	visible = _terrain.overlay_mode == 0
	var center := Vector2(camera.global_position.x, camera.global_position.z)
	var ground_y := 0.0
	if _simulation != null:
		ground_y = float(_simulation.get_world_surface_height(center))
	var height := camera.global_position.y - ground_y
	var reach := sqrt(maxf(RADIUS_M * RADIUS_M - height * height, 0.0))
	if reach <= CELL_M:
		for draw in _draws + _near_draws:
			draw.visible = false
		return
	var half_cells := mini(int(ceil(reach / CELL_M)), (WINDOW_CELLS - 1) / 2)
	var first_cell := Vector2i((center / CELL_M).floor()) - Vector2i(half_cells, half_cells)
	var first_key := Vector2i(((Vector2(first_cell) * CELL_M + world_size * 0.5) / span).floor())
	var near_reach := sqrt(maxf(NEAR_RADIUS_M * NEAR_RADIUS_M - height * height, 0.0))
	var near_half_cells := mini(int(ceil(near_reach / NEAR_CELL_M)), (NEAR_WINDOW_CELLS - 1) / 2)
	for slot in PATCH_SLOTS:
		var key := first_key + Vector2i(slot % 2, slot / 2)
		var patch: Dictionary = _terrain.patches.get(key, {})
		var source := patch.get("node") as MeshInstance3D
		var shown := source != null and source.visible and source.mesh != null
		_draws[slot].visible = shown
		_near_draws[slot].visible = shown and near_reach > NEAR_CELL_M
		if not shown:
			continue
		var terrain_material := patch["material"] as ShaderMaterial
		var patch_size: Vector2 = terrain_material.get_shader_parameter("patch_world_size_m")
		var patch_min := Vector2(source.position.x, source.position.z) - patch_size * 0.5
		var mask_mesh := _mask_meshes[slot]
		var graded_cells: Dictionary = {}
		if _terrain.graded_tiles != null:
			graded_cells = _terrain.graded_tiles.patch_cells.get(key, {})
		_sync_graded_mask(slot, source, graded_cells)
		mask_mesh.visible = graded_cells.is_empty()
		if _keys[slot] != key or mask_mesh.mesh != source.mesh:
			_keys[slot] = key
			mask_mesh.mesh = source.mesh
			mask_mesh.position = source.position
			var mask_material := mask_mesh.material_override as ShaderMaterial
			mask_material.set_shader_parameter("height_is_baked", patch["height_is_baked"])
			mask_material.set_shader_parameter("patch_origin", patch_min)
			for parameter in [&"heightmap", &"height_scale", &"heightmap_texture_size",
					&"inner_sample_offset_texels", &"inner_sample_size_texels", &"patch_world_size_m"]:
				mask_material.set_shader_parameter(parameter, terrain_material.get_shader_parameter(parameter))
			var mask_camera := _masks[slot].get_node("Camera") as Camera3D
			mask_camera.position = Vector3(source.position.x, 10.0, source.position.z)
			mask_camera.size = maxf(patch_size.x, patch_size.y)
			_masks[slot].render_target_update_mode = SubViewport.UPDATE_ONCE
		var mask_size := (_masks[slot].get_node("Camera") as Camera3D).size
		var mask_bounds := Vector4(source.position.x - mask_size * 0.5,
			source.position.z - mask_size * 0.5, mask_size, mask_size)
		_place_layer(slot, source, terrain_material, patch_min, patch_size, mask_bounds,
			center, CELL_M, half_cells)
		if _near_draws[slot].visible:
			_place_layer(PATCH_SLOTS + slot, source, terrain_material, patch_min, patch_size,
				mask_bounds, center, NEAR_CELL_M, near_half_cells)

# Binds one layer's draw (index into _draws then _near_draws) to the part of its window over a
# patch. Both patches submit a cell straddling their seam; the shader's half-open root test
# chooses exactly one owner after jitter, so crossing a seam cannot move a clump.
func _place_layer(
	index: int, source: MeshInstance3D, terrain_material: ShaderMaterial, patch_min: Vector2,
	patch_size: Vector2, mask_bounds: Vector4, center: Vector2, cell_m: float, half_cells: int
) -> void:
	var draw := _draw_at(index)
	var first_cell := Vector2i((center / cell_m).floor()) - Vector2i(half_cells, half_cells)
	var end_cell := first_cell + Vector2i(2 * half_cells + 1, 2 * half_cells + 1)
	var begin := first_cell.max(Vector2i((patch_min / cell_m).floor()))
	var end := end_cell.min(Vector2i(((patch_min + patch_size) / cell_m).ceil()))
	var counts := (end - begin).max(Vector2i.ZERO)
	draw.multimesh.visible_instance_count = counts.x * counts.y
	if counts.x == 0 or counts.y == 0:
		return
	var snapped_center := Vector2(first_cell + Vector2i(half_cells, half_cells)) * cell_m
	draw.position = Vector3(snapped_center.x, 0.0, snapped_center.y)
	_set_binding(index, &"window_cell", Vector2(begin))
	_set_binding(index, &"window_columns", counts.x)
	_set_binding(index, &"patch_origin", patch_min)
	_set_binding(index, &"mask_world_bounds", mask_bounds)
	var plane := source.mesh as PlaneMesh
	_set_binding(index, &"terrain_mesh_cells", Vector2(
		plane.subdivide_width + 1, plane.subdivide_depth + 1) if plane != null else Vector2.ZERO)
	for parameter in TERRAIN_PARAMETERS:
		_set_binding(index, parameter, terrain_material.get_shader_parameter(parameter))

func _draw_at(index: int) -> MultiMeshInstance3D:
	return _draws[index] if index < PATCH_SLOTS else _near_draws[index - PATCH_SLOTS]

func _set_binding(index: int, parameter: StringName, value: Variant) -> void:
	if _bindings[index].get(parameter) != value:
		(_draw_at(index).material_override as ShaderMaterial).set_shader_parameter(parameter, value)
		_bindings[index][parameter] = value

func _create_mask() -> void:
	var viewport := SubViewport.new()
	viewport.size = Vector2i(MASK_SIZE, MASK_SIZE)
	viewport.world_3d = World3D.new()
	viewport.render_target_update_mode = SubViewport.UPDATE_DISABLED
	var environment := Environment.new()
	environment.background_mode = Environment.BG_COLOR
	environment.background_color = Color.BLACK
	environment.ambient_light_source = Environment.AMBIENT_SOURCE_DISABLED
	viewport.world_3d.environment = environment
	var camera := Camera3D.new()
	camera.name = "Camera"
	camera.projection = Camera3D.PROJECTION_ORTHOGONAL
	camera.rotation.x = -PI * 0.5
	camera.near = 0.1
	camera.far = 20.0
	viewport.add_child(camera)
	var mesh := MeshInstance3D.new()
	mesh.cast_shadow = GeometryInstance3D.SHADOW_CASTING_SETTING_OFF
	# The mask shader flattens the mesh, so baked mountain bounds cannot cull its draw.
	mesh.extra_cull_margin = 4096.0
	var material := ShaderMaterial.new()
	material.shader = MASK_SHADER
	mesh.material_override = material
	viewport.add_child(mesh)
	add_child(viewport)
	_masks.append(viewport)
	_mask_meshes.append(mesh)
	var cells: Array = []
	for index in 5:
		var cell := MultiMeshInstance3D.new()
		cell.cast_shadow = GeometryInstance3D.SHADOW_CASTING_SETTING_OFF
		cell.visible = false
		var cell_material := ShaderMaterial.new()
		cell_material.shader = GRADED_MASK_SHADER
		cell.material_override = cell_material
		viewport.add_child(cell)
		cells.append(cell)
	_graded_mask_cells.append(cells)

# Mirrors one patch's current cell buffers into the private mask world. The source MultiMeshes
# own all tile movement and counts, so steady frames only compare references and bindings.
func _sync_graded_mask(
	slot: int, source: MeshInstance3D, cells: Dictionary
) -> void:
	var mirrors: Array = _graded_mask_cells[slot]
	if cells.is_empty():
		for mirror: MultiMeshInstance3D in mirrors:
			mirror.visible = false
		if _graded_mask_revisions[slot] >= 0:
			_graded_mask_revisions[slot] = -1
			_masks[slot].render_target_update_mode = SubViewport.UPDATE_ONCE
		return
	var revision := int(cells.get("mask_revision", 0))
	if _graded_mask_revisions[slot] != revision:
		_graded_mask_revisions[slot] = revision
		_masks[slot].render_target_update_mode = SubViewport.UPDATE_ONCE
	var rect: Rect2 = cells["rect"]
	var rect_hm := Vector4i(
		int(round(rect.position.x * 2.0)), int(round(rect.position.y * 2.0)),
		int(round(rect.end.x * 2.0)), int(round(rect.end.y * 2.0))
	)
	var sources: Array = cells["nodes"]
	var materials: Array = cells["materials"]
	for index in mirrors.size():
		var mirror := mirrors[index] as MultiMeshInstance3D
		var cell_source := sources[index] as MultiMeshInstance3D
		mirror.multimesh = cell_source.multimesh
		mirror.position = source.position
		mirror.visible = cell_source.visible and cell_source.multimesh.visible_instance_count > 0
		mirror.set_instance_shader_parameter("cell_patch_rect_hm", rect_hm)
		mirror.set_instance_shader_parameter(
			"cell_lattice_anchor_hm", _terrain.graded_tiles.lattice_anchor_hm
		)
		var material := mirror.material_override as ShaderMaterial
		material.set_shader_parameter("terrain_cell_m", _terrain.terrain_cell_m)
		if index < materials.size():
			var source_material := materials[index] as ShaderMaterial
			for parameter in [
				&"graded_texels", &"graded_rows", &"graded_seam_capacity", &"graded_edge_capacity"
			]:
				material.set_shader_parameter(parameter, source_material.get_shader_parameter(parameter))

func _clump_mesh() -> ArrayMesh:
	var vertices := PackedVector3Array()
	var normals := PackedVector3Array()
	var uvs := PackedVector2Array()
	var indices := PackedInt32Array()
	# Five tapered, two-segment blades fanned around the clump: 25 vertices / 15 triangles. A
	# blade is a few pixels wide at gameplay distances; three 9 cm blades read as specks.
	for blade in 5:
		var angle := float(blade) * TAU / 5.0 + 0.4 * sin(float(blade) * 2.3)
		var across := Vector3(cos(angle), 0.0, sin(angle))
		var root := across * (0.10 + 0.08 * float(blade % 2))
		var base := vertices.size()
		for point in [Vector2(-0.07, 0), Vector2(0.07, 0),
				Vector2(-0.045, 0.55), Vector2(0.045, 0.55), Vector2(0.02, 1)]:
			vertices.append(root + across * point.x + Vector3.UP * point.y)
			normals.append(Vector3.UP)
			uvs.append(Vector2(float(blade) / 5.0, point.y))
		for index in [0, 2, 1, 1, 2, 3, 2, 4, 3]:
			indices.append(base + index)
	var arrays := []
	arrays.resize(Mesh.ARRAY_MAX)
	arrays[Mesh.ARRAY_VERTEX] = vertices
	arrays[Mesh.ARRAY_NORMAL] = normals
	arrays[Mesh.ARRAY_TEX_UV] = uvs
	arrays[Mesh.ARRAY_INDEX] = indices
	var mesh := ArrayMesh.new()
	mesh.add_surface_from_arrays(Mesh.PRIMITIVE_TRIANGLES, arrays)
	return mesh
