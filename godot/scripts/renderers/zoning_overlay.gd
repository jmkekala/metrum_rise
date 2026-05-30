## Zone overlay — uploads profile-aware zoning textures to a full-map quad and manages active/passive state.
##
## Rust methods called: get_zone_profile_texture_data_rg8(), get_zone_profile_style_lut_rgba8(),
##   get_distance_texture_data(), get_occupied_texture_data(), get_no_build_mask_texture_data(),
##   get_zone_grid_size(), get_heightmap_size(), get_terrain_world_size(),
##   get_no_building_spawn_edge_indices(), get_edge_geometry_3d()
##
## Attach to a MeshInstance3D node named ZoningOverlay positioned at (0, 0.005, 0).
## The mesh is a PlaneMesh sized to the full terrain extent. The shader reads three
## R8 textures and blends them according to the zoning colour LUT.
extends MeshInstance3D

@onready var simulation_node = $"../SimulationNode"

# Grid textures and profile-style LUT
var zone_image: Image
var style_lut_image: Image
var distance_image: Image
var occupied_image: Image
var no_build_image: Image

var zone_tex: ImageTexture
var style_lut_tex: ImageTexture
var distance_tex: ImageTexture
var occupied_tex: ImageTexture
var no_build_tex: ImageTexture

var zone_grid_w: int = 0
var zone_grid_h: int = 0
var profile_style_lut_size: int = 1

# Smooth alpha transition when entering/leaving the zoning tool
var _tool_active: float = 0.0         # current lerped value
var _tool_active_target: float = 0.0  # 0.0 passive / 1.0 active
const FADE_SPEED: float = 6.0         # units per second

# Track whether textures need uploading
var _zone_dirty: bool = true
var _distance_dirty: bool = true
var _occupied_dirty: bool = true
var _no_build_dirty: bool = true
var _zone_debug_nonzero_cells: int = 0
var _zone_debug_cell_count: int = 0

func _ready():
	var size: Vector2i = simulation_node.get_zone_grid_size()
	zone_grid_w = size.x
	zone_grid_h = size.y
	_zone_debug_cell_count = zone_grid_w * zone_grid_h

	var style_bytes: PackedByteArray = simulation_node.get_zone_profile_style_lut_rgba8()
	profile_style_lut_size = maxi(1, style_bytes.size() / 4)

	zone_image     = Image.create(zone_grid_w, zone_grid_h, false, Image.FORMAT_RG8)
	style_lut_image = Image.create(profile_style_lut_size, 1, false, Image.FORMAT_RGBA8)
	distance_image = Image.create(zone_grid_w, zone_grid_h, false, Image.FORMAT_R8)
	occupied_image = Image.create(zone_grid_w, zone_grid_h, false, Image.FORMAT_R8)
	no_build_image = Image.create(zone_grid_w, zone_grid_h, false, Image.FORMAT_R8)
	zone_image.fill(Color.BLACK)
	style_lut_image.fill(Color.TRANSPARENT)
	distance_image.fill(Color.BLACK)
	occupied_image.fill(Color.BLACK)
	no_build_image.fill(Color.BLACK)

	zone_tex     = ImageTexture.create_from_image(zone_image)
	style_lut_image.set_data(profile_style_lut_size, 1, false, Image.FORMAT_RGBA8, style_bytes)
	style_lut_tex = ImageTexture.create_from_image(style_lut_image)
	distance_tex = ImageTexture.create_from_image(distance_image)
	occupied_tex = ImageTexture.create_from_image(occupied_image)
	no_build_tex = ImageTexture.create_from_image(no_build_image)

	# Build a flat quad covering the full terrain extent
	var terrain_world_size = simulation_node.get_terrain_world_size()
	var world_w = terrain_world_size.x
	var world_h = terrain_world_size.y
	var quad = PlaneMesh.new()
	quad.size = Vector2(world_w, world_h)
	self.mesh = quad

	var mat = ShaderMaterial.new()
	mat.shader = load("res://scripts/shaders/zoning_overlay.gdshader")
	mat.set_shader_parameter("zone_texture",     zone_tex)
	mat.set_shader_parameter("profile_style_lut", style_lut_tex)
	mat.set_shader_parameter("profile_style_lut_size", float(profile_style_lut_size))
	mat.set_shader_parameter("distance_texture", distance_tex)
	mat.set_shader_parameter("occupied_texture", occupied_tex)
	mat.set_shader_parameter("no_build_texture", no_build_tex)
	mat.set_shader_parameter("grid_cells",       float(zone_grid_w))
	mat.set_shader_parameter("tool_active",      0.0)
	self.material_override = mat

	# y=0.005: above terrain (y=0) so zones are visible, below road decals (ROAD_DECAL_RENDER_Z_BIAS_M=0.04)
	# so opaque roads occlude the overlay via depth test.
	position = Vector3(0.0, 0.005, 0.0)
	visible = false

func _process(delta):
	# Fade tool_active value
	if abs(_tool_active - _tool_active_target) > 0.001:
		_tool_active = move_toward(_tool_active, _tool_active_target, FADE_SPEED * delta)
		var mat = self.material_override as ShaderMaterial
		if mat:
			mat.set_shader_parameter("tool_active", _tool_active)
	visible = _tool_active > 0.001 or _tool_active_target > 0.0

	# Upload dirty textures
	if _zone_dirty:
		_upload_zone()
		_zone_dirty = false
	if _distance_dirty:
		_upload_distance()
		_distance_dirty = false
	if _occupied_dirty:
		_upload_occupied()
		_occupied_dirty = false
	if _no_build_dirty:
		_upload_no_build()
		_no_build_dirty = false

func _upload_zone():
	var bytes = simulation_node.get_zone_profile_texture_data_rg8()
	if bytes.size() == zone_grid_w * zone_grid_h * 2:
		_update_zone_debug_stats(bytes)
		zone_image.set_data(zone_grid_w, zone_grid_h, false, Image.FORMAT_RG8, bytes)
		zone_tex.update(zone_image)

func _update_zone_debug_stats(bytes: PackedByteArray):
	var nonzero_cells := 0
	var cell_count := int(bytes.size() / 2)
	for cell_index in range(cell_count):
		var byte_index := cell_index * 2
		var runtime_id := int(bytes[byte_index]) | (int(bytes[byte_index + 1]) << 8)
		if runtime_id != 0:
			nonzero_cells += 1
	_zone_debug_nonzero_cells = nonzero_cells
	_zone_debug_cell_count = cell_count

func _upload_distance():
	var bytes = simulation_node.get_distance_texture_data()
	if bytes.size() == zone_grid_w * zone_grid_h:
		distance_image.set_data(zone_grid_w, zone_grid_h, false, Image.FORMAT_R8, bytes)
		distance_tex.update(distance_image)

func _upload_occupied():
	var bytes = simulation_node.get_occupied_texture_data()
	if bytes.size() == zone_grid_w * zone_grid_h:
		occupied_image.set_data(zone_grid_w, zone_grid_h, false, Image.FORMAT_R8, bytes)
		occupied_tex.update(occupied_image)

func _upload_no_build():
	var bytes = simulation_node.get_no_build_mask_texture_data()
	if bytes.size() == zone_grid_w * zone_grid_h:
		no_build_image.set_data(zone_grid_w, zone_grid_h, false, Image.FORMAT_R8, bytes)
		no_build_tex.update(no_build_image)

## Call when zone paint changes (from zoning_tool.gd after commit).
func mark_zone_dirty():
	_zone_dirty = true

## Call when a building is placed or removed.
func mark_occupied_dirty():
	_occupied_dirty = true

## Call when roads change (distance_to_road recomputed by Rust).
func mark_distance_dirty():
	_distance_dirty = true
	_no_build_dirty = true  # update_distance_to_road also calls update_no_build_mask

## Call when a no-build flag is toggled without a road change.
func mark_no_build_dirty():
	_no_build_dirty = true

## Call from InputManager when the zoning tool activates/deactivates.
func set_tool_active(active: bool):
	_tool_active_target = 1.0 if active else 0.0
	if active:
		visible = true
	_rebuild_no_build_overlay()

## Force a full refresh of all textures (e.g. after save/load).
func full_refresh():
	_zone_dirty = true
	_distance_dirty = true
	_occupied_dirty = true
	_no_build_dirty = true
	_rebuild_no_build_overlay()

func road_geometry_debug_patch_lines(_flat_pairs: PackedInt32Array) -> Array[String]:
	var lines: Array[String] = [
		"zoning_overlay visible=%s tool_active=%.3f target=%.3f zone_nonzero=%d/%d"
		% [
			str(visible),
			_tool_active,
			_tool_active_target,
			_zone_debug_nonzero_cells,
			_zone_debug_cell_count,
		]
	]
	return lines

# ── No-build edge hatched overlay ──────────────────────────────────────────

var _no_build_mesh_instance: MeshInstance3D = null

# Draws orange lines along each no-build edge; visible only when tool is active.
func _rebuild_no_build_overlay():
	if not _no_build_mesh_instance:
		_no_build_mesh_instance = MeshInstance3D.new()
		_no_build_mesh_instance.cast_shadow = GeometryInstance3D.SHADOW_CASTING_SETTING_OFF
		var mat := StandardMaterial3D.new()
		mat.shading_mode = BaseMaterial3D.SHADING_MODE_UNSHADED
		mat.albedo_color = Color(1.0, 0.5, 0.0, 0.9)
		mat.transparency = BaseMaterial3D.TRANSPARENCY_ALPHA
		_no_build_mesh_instance.material_override = mat
		add_sibling(_no_build_mesh_instance)

	_no_build_mesh_instance.visible = _tool_active_target > 0.5

	var indices: PackedInt32Array = simulation_node.get_no_building_spawn_edge_indices()
	if indices.is_empty():
		_no_build_mesh_instance.mesh = null
		return

	var im := ImmediateMesh.new()
	im.surface_begin(Mesh.PRIMITIVE_LINES)

	for edge_idx in indices:
		var pts: PackedVector3Array = simulation_node.get_edge_geometry_3d(edge_idx)
		for i in range(pts.size() - 1):
			var a := pts[i];   a.y += 0.08
			var b := pts[i+1]; b.y += 0.08
			im.surface_add_vertex(a)
			im.surface_add_vertex(b)

	im.surface_end()
	_no_build_mesh_instance.mesh = im
