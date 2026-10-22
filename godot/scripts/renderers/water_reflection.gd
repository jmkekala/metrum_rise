# SPDX-License-Identifier: GPL-2.0-only

## Planar reflection of the scene in the lakes the camera looks at.
##
## Screen-space reflection can only reflect what is already on screen. Where a tree stands
## between the camera and a lake, the far shore its reflection needs is hidden behind that tree,
## so no screen march can find it. This node renders the scene again from a camera mirrored below
## the water plane, at reduced resolution, into `water_reflection_map`, and for a second lake in
## view at another level into `water_reflection_map_2`. Water on either plane samples its map
## instead of marching the screen; water on neither keeps the screen march.
##
## The pass draws without shadows: a second key light on REFLECTION_LAYER, unshadowed, stands
## in for the sun, which is moved to KEY_LIGHT_LAYER that only the main camera sees. A shadowed
## sun would render its cascades a second time for the mirrored camera. The pass also skips the
## cloud and mist compositor effects and draws no sky: where it is transparent the water
## reflects the sky as before. Terrain below the plane is clipped by its own shader
## (water_reflection.gdshaderinc), since Godot's camera has no oblique near plane.
extends Node

const ReflectionSources := preload("res://scripts/renderers/water_reflection_sources.gd")

## Layer 18 holds the water, which the reflection camera does not draw.
const WATER_LAYER := 1 << 17
# Layer 19 holds the shadowed key light, which the reflection camera does not see; layer 20
# holds the reflection's unshadowed key light, which the main camera does not see.
const KEY_LIGHT_LAYER := 1 << 18
# Must match WATER_REFLECTION_LAYER in water_reflection.gdshaderinc.
const REFLECTION_LAYER := 1 << 19
# Layer 17 holds nothing; it is in the second mirror camera's mask only, so a shader can tell the
# mirrors apart. Must match WATER_REFLECTION_SECOND_LAYER in water_reflection.gdshaderinc.
const SECOND_MIRROR_LAYER := 1 << 16
## Lakes at distinct levels reflected at once, each by its own pass. Two cover a lake beside a
## lake, and a river's steps, which a single mirror left to the thin screen march by turns.
const MIRROR_COUNT := 2
const ALL_LAYERS := (1 << 20) - 1
# Must match WATER_SURFACE_LIFT_M in water.gdshader: the mirror is the surface as drawn.
const WATER_SURFACE_LIFT_M := 0.05
## Share of the main viewport's width and height the reflection is rendered at.
const RESOLUTION_SCALE := 0.5
# Screen points whose rays vote for the plane: columns x rows between the horizon and the
# bottom of the screen, where any water in view lies.
const VOTE_COLUMNS := 14
const VOTE_ROWS := 12
const VOTE_MAX_DISTANCE_M := 3000.0
# Screen share added around the water's rectangle for rays the ripples bend out of it.
const CROP_MARGIN := 0.03
# Frames between syncs of the reflection world with the scene's resident patches. Residency
# changes slowly; a patch a few frames late only shows late in the reflection.
const PROXY_SYNC_FRAMES := 10

var _simulation: Node
# The water renderer (water.gd), whose resident patches bound the screen region to reflect.
var _water: Node
var _sun: DirectionalLight3D
var _main_environment: Environment
# One per mirror: [SubViewport, Camera3D, plane height, active].
var _mirrors: Array = []
# The reflection's own world: only the proxies below, never the scene's agents, grass, props or
# water. Every mirror draws it.
var _world: World3D
var _environment: Environment
# One per mirror: the main view's distance fade, applied as coverage (water_reflection_fog.glsl).
var _fog_effects: Array[WaterReflectionFogEffect] = []
var _reflection_sun: DirectionalLight3D
# Levels the last vote chose, which the next vote favours.
var _voted_levels := PackedFloat32Array()
# Per vote ray, the water level it last came down onto, or NAN.
var _ray_levels := PackedFloat32Array()
var _ray_weights := PackedFloat32Array()
var _ray_directions := PackedVector3Array()
# The terrain patches the reflection world mirrors, by instance id, to [proxy instance RID,
# base RID, transform, material]. Meshes are shared; terrain uses a cached clip-material variant.
var _proxies := {}
# The same for tree impostor MultiMeshes, which follow their patches' events, not the round robin.
var _tree_proxies := {}
var _proxy_frame := 0
var _terrain: Node
# Reflection variants of terrain materials, by source.
var _grid_materials := {}
var _proxy_shaders := {}
var _dirty_terrain_patches := {}
var _vegetation: Node
# Tree patches added, shown or hidden since the last frame, by instance id: their proxies are
# synced in this frame's pre-draw, so the reflection's forest changes when the scene's does.
var _dirty_tree_patches := {}
# Building and scaffold batches mirrored as shared MultiMesh proxies (water_reflection_sources.gd).
var _building_proxies: RefCounted

## Wires the pass to the scene's key light and environment. Call once before the first frame.
func setup(
	simulation: Node, sun: DirectionalLight3D, environment: Environment, water: Node
) -> void:
	_simulation = simulation
	_water = water
	if water != null:
		_terrain = water.get_node_or_null("../Terrain")
		_vegetation = water.get_node_or_null("../Vegetation")
	if _terrain != null:
		_terrain.patch_render_will_change.connect(_on_terrain_patch_will_change)
		_terrain.patches_will_reset.connect(_on_terrain_reset)
	_sun = sun
	_main_environment = environment
	if _vegetation != null:
		_vegetation.child_entered_tree.connect(_on_tree_patch_entered)
		_vegetation.child_exiting_tree.connect(_on_tree_patch_exiting)
		_vegetation.visibility_changed.connect(_on_vegetation_visibility_changed)
		for patch in _vegetation.get_children():
			_on_tree_patch_entered(patch)

func _ready() -> void:
	_world = World3D.new()
	_building_proxies = ReflectionSources.new()
	_building_proxies.setup(_world.scenario, PROXY_SYNC_FRAMES,
		_water.get_node_or_null("../Buildings") if _water != null else null)
	for index in MIRROR_COUNT:
		var viewport := SubViewport.new()
		viewport.name = "WaterReflectionViewport%d" % index
		viewport.transparent_bg = true
		# Linear HDR, like the screen texture the screen march reads, rather than tonemapped.
		viewport.use_hdr_2d = true
		# Resolve analytic terrain clip coverage as well as raster edges into the map's alpha.
		viewport.msaa_3d = Viewport.MSAA_2X
		viewport.screen_space_aa = Viewport.SCREEN_SPACE_AA_DISABLED
		viewport.render_target_update_mode = SubViewport.UPDATE_DISABLED
		viewport.world_3d = _world
		var camera := Camera3D.new()
		camera.cull_mask = ALL_LAYERS if index > 0 else ALL_LAYERS & ~SECOND_MIRROR_LAYER
		# Its own compositor replaces the environment's: no cloud march or mist for the mirror,
		# and the distance fade as coverage in place of the environment's fog.
		var fog := WaterReflectionFogEffect.new()
		camera.compositor = Compositor.new()
		camera.compositor.compositor_effects = [fog]
		_fog_effects.append(fog)
		viewport.add_child(camera)
		add_child(viewport)
		_mirrors.append([viewport, camera, 0.0, false])
	_reflection_sun = DirectionalLight3D.new()
	_reflection_sun.name = "WaterReflectionKeyLight"
	_reflection_sun.shadow_enabled = false
	_reflection_sun.sky_mode = DirectionalLight3D.SKY_MODE_LIGHT_ONLY
	_mirrors[0][0].add_child(_reflection_sun)
	RenderingServer.global_shader_parameter_set("water_reflection_map", _mirrors[0][0].get_texture())
	RenderingServer.global_shader_parameter_set(
		"water_reflection_map_2", _mirrors[1][0].get_texture())
	RenderingServer.frame_pre_draw.connect(_update_before_draw)

func _exit_tree() -> void:
	if _building_proxies != null:
		_building_proxies.clear()
	for proxies in [_proxies, _tree_proxies]:
		for proxy in proxies.values():
			RenderingServer.free_rid(proxy[0])
		proxies.clear()
	if RenderingServer.frame_pre_draw.is_connected(_update_before_draw):
		RenderingServer.frame_pre_draw.disconnect(_update_before_draw)
	RenderingServer.global_shader_parameter_set("water_reflection_map", null)
	RenderingServer.global_shader_parameter_set("water_reflection_map_2", null)
	RenderingServer.global_shader_parameter_set("water_reflection_plane", Vector4.ZERO)

# After every node has moved the camera and the key light for this frame.
func _update_before_draw() -> void:
	_sync_tree_patches()
	_building_proxies.sync()
	var main_viewport := get_viewport()
	var main_camera := main_viewport.get_camera_3d() if main_viewport != null else null
	if main_camera == null or _sun == null:
		for mirror in _mirrors:
			_set_active(mirror, false)
		_publish_planes()
		return
	main_camera.cull_mask &= ~REFLECTION_LAYER
	var levels := _vote_planes(main_camera)
	var synced := false
	for index in MIRROR_COUNT:
		var mirror: Array = _mirrors[index]
		var level := levels[index]
		if is_nan(level) or main_camera.global_position.y <= level:
			_set_active(mirror, false)
			continue
		mirror[2] = level + WATER_SURFACE_LIFT_M
		var crop := _water_screen_rect(main_camera, mirror[2])
		if not crop.has_area():
			_set_active(mirror, false)
			continue
		if not synced:
			synced = true
			_sync_environment()
			_sync_key_light()
			_proxy_frame += 1
			_sync_proxies()
		# Mirrored left to right: see _place_camera.
		_place_camera(mirror, index, main_camera, main_viewport,
			Rect2(1.0 - crop.end.x, crop.position.y, crop.size.x, crop.size.y))
		_set_active(mirror, true)
	_publish_planes()

func _set_active(mirror: Array, active: bool) -> void:
	if active == mirror[3]:
		return
	mirror[3] = active
	mirror[0].render_target_update_mode = (
		SubViewport.UPDATE_ALWAYS if active else SubViewport.UPDATE_DISABLED
	)

func _publish_planes() -> void:
	RenderingServer.global_shader_parameter_set("water_reflection_plane", Vector4(
		_mirrors[0][2], 1.0 if _mirrors[0][3] else 0.0,
		_mirrors[1][2], 1.0 if _mirrors[1][3] else 0.0))

# Mirrors the main camera through the plane of `slot`, the first or `index`th mirror. The
# mirrored basis is left-handed; negating its x axis makes it a rotation again, which only flips
# the picture left to right, and the water samples it by projecting through this camera's own
# matrix, so the flip never shows.
func _place_camera(
	slot: Array, index: int, main_camera: Camera3D, main_viewport: Viewport, crop: Rect2
) -> void:
	var viewport: SubViewport = slot[0]
	var camera: Camera3D = slot[1]
	var screen := Vector2(main_viewport.get_visible_rect().size)
	var size := Vector2i((screen * crop.size * RESOLUTION_SCALE).round()).max(Vector2i.ONE)
	if viewport.size != size:
		viewport.size = size
	var source := main_camera.global_transform
	var mirror := Vector3(1.0, -1.0, 1.0)
	var basis := Basis(-source.basis.x * mirror, source.basis.y * mirror, source.basis.z * mirror)
	var origin := source.origin * mirror + Vector3(0.0, 2.0 * float(slot[2]), 0.0)
	camera.global_transform = Transform3D(basis, origin)
	# A Node3D's new transform reaches the server only at the scene tree's next transform flush,
	# which is next frame's process: set from frame_pre_draw, the mirror rendered a frame behind
	# the matrix the water samples it with, so the reflection trailed every camera turn. The
	# frustum setters below reach the server at once; the transform is sent the same way.
	RenderingServer.camera_set_transform(camera.get_camera_rid(), camera.global_transform)
	# A point on the plane has the same camera-space position for both cameras, bar the negated
	# x axis, so the water's screen rectangle mirrored left to right is where the reflection
	# camera sees it. An off-axis frustum over that rectangle draws only what the water shows.
	var height := 2.0 * main_camera.near * tan(deg_to_rad(main_camera.fov) * 0.5)
	var width := height * screen.x / maxf(screen.y, 1.0)
	var centre := crop.get_center()
	camera.projection = Camera3D.PROJECTION_FRUSTUM
	camera.keep_aspect = Camera3D.KEEP_HEIGHT
	camera.size = height * crop.size.y
	camera.frustum_offset = Vector2((centre.x - 0.5) * width, (0.5 - centre.y) * height)
	camera.near = main_camera.near
	camera.far = main_camera.far
	var view_projection: Projection = (
		camera.get_camera_projection() * Projection(camera.global_transform.affine_inverse())
	)
	RenderingServer.global_shader_parameter_set(
		"water_reflection_view_projection" if index == 0 else "water_reflection_view_projection_2",
		view_projection
	)

# Screen rectangle, in [0, 1], covering the water: the union of the resident water patches laid
# on the plane at `plane_y`, each clipped to the space in front of the camera.
# Patch bounds are coarse, so the rectangle can include land, never exclude water. Empty when
# no such patch is in front of the camera. O(resident water patches) per frame.
func _water_screen_rect(camera: Camera3D, plane_y: float) -> Rect2:
	if _water == null:
		return Rect2(0.0, 0.0, 1.0, 1.0)
	var screen := camera.get_viewport().get_visible_rect().size
	var forward := -camera.global_transform.basis.z
	var eye := camera.global_position
	var near_m := camera.near * 1.5
	var lo := Vector2(INF, INF)
	var hi := Vector2(-INF, -INF)
	for patch in _water.patches.values():
		var node: MeshInstance3D = patch.get("node")
		if node == null or not node.visible:
			continue
		# The mesh is flat; its height comes from the shader, so its bounds say nothing of level.
		var bounds := node.global_transform * node.get_aabb()
		var corners: Array[Vector3] = [
			Vector3(bounds.position.x, plane_y, bounds.position.z),
			Vector3(bounds.end.x, plane_y, bounds.position.z),
			Vector3(bounds.end.x, plane_y, bounds.end.z),
			Vector3(bounds.position.x, plane_y, bounds.end.z),
		]
		# Sutherland-Hodgman against the plane just in front of the near plane.
		for i in corners.size():
			var a := corners[i]
			var b := corners[(i + 1) % corners.size()]
			var da := (a - eye).dot(forward) - near_m
			var db := (b - eye).dot(forward) - near_m
			if da >= 0.0:
				var point := camera.unproject_position(a) / screen
				lo = lo.min(point)
				hi = hi.max(point)
			if (da >= 0.0) != (db >= 0.0):
				var point := camera.unproject_position(a.lerp(b, da / (da - db))) / screen
				lo = lo.min(point)
				hi = hi.max(point)
	if lo.x > hi.x:
		return Rect2()
	# Room for the ripples' bend, kept past the screen's own edges too: the water fades the
	# reflection out over the outer 2% of the map (water_planar_reflection), and with the
	# rectangle clipped to the screen that fade showed as an unreflected sliver along each side.
	var margin := Vector2(CROP_MARGIN, CROP_MARGIN)
	lo = (lo - margin).max(-margin)
	hi = (hi + margin).min(Vector2.ONE + margin)
	if lo.x >= hi.x or lo.y >= hi.y:
		return Rect2()
	return Rect2(lo, hi - lo)

func _sync_key_light() -> void:
	_reflection_sun.visible = _sun.visible
	_reflection_sun.global_transform = _sun.global_transform
	_reflection_sun.light_color = _sun.light_color
	_reflection_sun.light_energy = _sun.light_energy
	_reflection_sun.light_indirect_energy = _sun.light_indirect_energy

# The palette and the distance fade change every frame; a copy of the scene's environment
# follows them, with no sky drawn behind the scene so the water can tell sky from scene.
func _sync_environment() -> void:
	if _main_environment == null:
		return
	if _environment == null:
		_environment = _main_environment.duplicate()
		_environment.background_mode = Environment.BG_CLEAR_COLOR
		_environment.ssao_enabled = false
		_environment.ssil_enabled = false
		_environment.ssr_enabled = false
		_environment.glow_enabled = false
		_environment.tonemap_mode = Environment.TONE_MAPPER_LINEAR
		_environment.tonemap_exposure = 1.0
		_environment.tonemap_white = 1.0
		for slot in _mirrors:
			slot[1].environment = _environment
	_environment.ambient_light_color = _main_environment.ambient_light_color
	_environment.ambient_light_energy = _main_environment.ambient_light_energy
	# The fog fades far ground into a colour matching the sky behind it, and the water reflects
	# the sky brighter than that colour at the scene's reflection strength, so fogged hills
	# reflected dark. The mirror leaves the fog out and the fade removes the scene instead, so
	# the water reflects the sky there (water_reflection_fog.glsl).
	_environment.fog_enabled = false
	for fog in _fog_effects:
		fog.fade_begin_m = _main_environment.fog_depth_begin
		fog.fade_end_m = _main_environment.fog_depth_end
		fog.fade_curve = _main_environment.fog_depth_curve
		fog.fade_density = _main_environment.fog_density if _main_environment.fog_enabled else 0.0

# Levels of the lakes a grid of view rays first comes down onto, in stable mirror slots; NAN
# where a mirror has no lake. Rust marches the rays against the terrain and the water
# as drawn (get_first_water_surfaces), so water a hill hides gets no vote and a river stepping
# down its valley polls at the levels it is drawn at. O(VOTE_COLUMNS * VOTE_ROWS *
# VOTE_MAX_DISTANCE_M / 4 m) samples in Rust per frame.
func _vote_planes(camera: Camera3D) -> PackedFloat32Array:
	if _simulation == null:
		return PackedFloat32Array([NAN, NAN])
	var rect := camera.get_viewport().get_visible_rect()
	# The rows start at the horizon, or the top of the screen when it is above, and close up
	# towards it with the square of their spacing: far water is a thin band just under the
	# horizon in a low view. Rows fixed over the lower two thirds of the screen all passed
	# below that band once the camera tilted up to level, and the vote went empty.
	var top := 0.0
	var forward := -camera.global_basis.z
	var flat := Vector3(forward.x, 0.0, forward.z)
	if flat.length_squared() > 1.0e-6:
		var horizon := camera.global_position + flat.normalized() * camera.far * 0.5
		if not camera.is_position_behind(horizon):
			top = clampf((camera.unproject_position(horizon).y - rect.position.y) / rect.size.y,
				0.0, 1.0)
	if _ray_directions.is_empty():
		_ray_directions.resize(VOTE_COLUMNS * VOTE_ROWS)
	for row in VOTE_ROWS:
		var down := (float(row) + 0.5) / float(VOTE_ROWS)
		for column in VOTE_COLUMNS:
			_ray_directions[row * VOTE_COLUMNS + column] = camera.project_ray_normal(
				rect.position + rect.size * Vector2(
					(float(column) + 0.5) / float(VOTE_COLUMNS),
					lerpf(top, 1.0, down * down)))
	var levels: PackedFloat32Array = _simulation.get_first_water_surfaces(
		camera.global_position, _ray_directions, VOTE_MAX_DISTANCE_M)
	# Empty while the simulation holds its core. Last frame's levels stand in: abstaining rays
	# emptied the vote then, and the reflection flickered off.
	if levels.size() == _ray_directions.size():
		_ray_levels = levels
	# Rust owns selection and its hysteresis. Row weights are immutable screen shares; a second
	# lake is ranked against other lakes, without a separate absolute entry/exit threshold.
	if _ray_weights.is_empty():
		_ray_weights.resize(VOTE_COLUMNS * VOTE_ROWS)
		for index in _ray_weights.size():
			var row := index / VOTE_COLUMNS
			_ray_weights[index] = float(2 * row + 1) / float(VOTE_ROWS * VOTE_ROWS * VOTE_COLUMNS)
	_voted_levels = _simulation.select_water_reflection_planes(_ray_levels, _ray_weights, _voted_levels)
	return _voted_levels

# Mirrors the resident terrain patches into the reflection world; tree impostors follow their
# patches' events (_sync_tree_patches), buildings their batches' (water_reflection_sources.gd).
# Agents, grass, props, understory, near tree meshes, hedges, lots and roads are left out; a
# bridge would need its own geometry first. Each frame handles
# one PROXY_SYNC_FRAMES-th of the patches and of the proxies, round robin, so no frame pays for
# the whole scan: done every tenth frame at once it was the frame-time p95.
# O(resident patches) checks and O(changed patches + resident patches / PROXY_SYNC_FRAMES)
# uploads per frame. Terrain changes bypass the round robin so shore geometry never trails.
func _on_terrain_patch_will_change(key: Vector2i) -> void:
	_dirty_terrain_patches[key] = true

func _on_terrain_reset() -> void:
	for proxy in _proxies.values():
		RenderingServer.free_rid(proxy[0])
	_proxies.clear()
	_grid_materials.clear()
	_dirty_terrain_patches.clear()
	_voted_levels.clear()
	_ray_levels.clear()

func _sync_proxies() -> void:
	var scenario := _world.scenario
	var slice := _proxy_frame % PROXY_SYNC_FRAMES
	var index := 0
	if _terrain != null:
		for key in _terrain.patches:
			var dirty := _dirty_terrain_patches.has(key)
			index += 1
			if not dirty and index % PROXY_SYNC_FRAMES != slice:
				continue
			var patch: Dictionary = _terrain.patches[key]
			var node := patch.get("node") as MeshInstance3D
			if node == null or not node.visible or node.mesh == null:
				continue
			var material := node.material_override as ShaderMaterial
			# Share the exact displayed mesh: a coarser grid (or a heightmap substitute for
			# baked ground) crosses the lake plane at a different shore and masks its reflection.
			if material != null:
				material = _proxy_material(material, dirty)
			_sync_proxy(_proxies, node, node.mesh.get_rid(), material, scenario)
	# A signalled patch removal/hide must drop its old occluder before this draw too.
	# Check all existing proxies on change frames; ordinary frames retain the round robin.
	for id in _proxies.keys():
		if _dirty_terrain_patches.is_empty() and id % PROXY_SYNC_FRAMES != slice:
			continue
		var source := instance_from_id(id) as GeometryInstance3D
		if source == null or not source.is_visible_in_tree():
			RenderingServer.free_rid(_proxies[id][0])
			_proxies.erase(id)
	_dirty_terrain_patches.clear()

# Tree patches are built, replaced, cached and restored as the camera moves. Synced round robin
# like the terrain, a replaced patch's old proxies went in one slice and its new ones came in
# another, up to ten frames apart, and its trees vanished from the reflection meanwhile. Synced
# from the patch's own events, the reflection's forest changes in the frame the scene's does.
# O(changed patches) per frame.
func _on_tree_patch_entered(node: Node) -> void:
	if not (node is Node3D):
		return
	if not node.visibility_changed.is_connected(_on_tree_patch_visibility_changed):
		node.visibility_changed.connect(_on_tree_patch_visibility_changed.bind(node))
	_dirty_tree_patches[node.get_instance_id()] = node

func _on_tree_patch_exiting(node: Node) -> void:
	_dirty_tree_patches.erase(node.get_instance_id())
	_free_tree_proxies(node)

func _on_tree_patch_visibility_changed(patch: Node3D) -> void:
	_dirty_tree_patches[patch.get_instance_id()] = patch

func _on_vegetation_visibility_changed() -> void:
	for patch in _vegetation.get_children():
		_on_tree_patch_entered(patch)

func _sync_tree_patches() -> void:
	if _dirty_tree_patches.is_empty():
		return
	var scenario := _world.scenario
	for patch in _dirty_tree_patches.values():
		if not is_instance_valid(patch) or not patch.is_inside_tree():
			continue
		if not patch.is_visible_in_tree():
			_free_tree_proxies(patch)
			continue
		for child in patch.get_children():
			var trees := child as MultiMeshInstance3D
			if trees == null or not trees.visible or int(trees.get_meta("lod", 0)) != 2 \
					or bool(trees.get_meta("shadow_proxy", false)):
				continue
			_sync_proxy(_tree_proxies, trees, trees.multimesh.get_rid(), trees.material_override,
				scenario)
	_dirty_tree_patches.clear()

func _free_tree_proxies(patch: Node) -> void:
	for child in patch.get_children():
		var id := child.get_instance_id()
		if _tree_proxies.has(id):
			RenderingServer.free_rid(_tree_proxies[id][0])
			_tree_proxies.erase(id)

func _sync_proxy(
	proxies: Dictionary, source: GeometryInstance3D, base: RID, material: Material, scenario: RID
) -> void:
	var id := source.get_instance_id()
	var proxy: Array = proxies.get(id, [])
	if not proxy.is_empty() and proxy[1] != base:
		RenderingServer.free_rid(proxy[0])
		proxy = []
	if proxy.is_empty():
		var instance := RenderingServer.instance_create2(base, scenario)
		RenderingServer.instance_geometry_set_cast_shadows_setting(
			instance, RenderingServer.SHADOW_CASTING_SETTING_OFF)
		proxy = [instance, base]
		proxies[id] = proxy
	# Only what changed is sent: pooled patch nodes move and change material, but most syncs
	# find every proxy as it was, and re-sending hundreds of instances stalled a frame.
	var instance: RID = proxy[0]
	var transform := source.global_transform
	if proxy.size() < 3 or proxy[2] != transform or proxy[3] != material:
		RenderingServer.instance_set_transform(instance, transform)
		RenderingServer.instance_geometry_set_material_override(
			instance, material.get_rid() if material != null else RID())
		if source.custom_aabb.has_volume():
			RenderingServer.instance_set_custom_aabb(instance, source.custom_aabb)
		RenderingServer.instance_set_extra_visibility_margin(instance, source.extra_cull_margin)
		proxy.resize(4)
		proxy[2] = transform
		proxy[3] = material

# Use the source geometry and its displacement mode; only the mirror clip and underside
# shading differ. Cached by source.
func _proxy_material(source: ShaderMaterial, refresh: bool = false) -> ShaderMaterial:
	var copy: ShaderMaterial = _grid_materials.get(source)
	if copy == null or refresh:
		# Terrain swaps its height texture in-place on the same material. An old duplicate
		# otherwise keeps the previous patch's shore after streaming, edits or LOD changes.
		copy = source.duplicate() as ShaderMaterial
		copy.shader = _proxy_shader(source.shader)
		_grid_materials[source] = copy
	return copy

func _proxy_shader(source: Shader) -> Shader:
	var shader: Shader = _proxy_shaders.get(source)
	if shader == null:
		shader = Shader.new()
		shader.code = source.code.replace("shader_type spatial;",
			"shader_type spatial;\n#define WATER_REFLECTION_PROXY")
		_proxy_shaders[source] = shader
	return shader
