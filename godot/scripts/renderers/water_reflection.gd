# SPDX-License-Identifier: GPL-3.0-or-later

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
# The terrain patches' shader (terrain.gd's TERRAIN_SHADER; by path, as terrain.gd reaches this
# script through scene_lighting.gd).
const TERRAIN_SHADER := preload("res://assets/materials/terrain.gdshader")
## The mirrors' MSAA. Godot precompiles pipelines for the project setting's sample count only, so
## the menu's prewarm holds that setting here while the mirror's templates get their surfaces
## (`menu_pipeline_prewarm.gd`, `RENDER-43`).
const MIRROR_MSAA := Viewport.MSAA_2X

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
# Screen points (in shares) whose rays' ground the screen rectangle looks about when the hull leaves
# a side open: the middles of the bottom, left, right and top edges (`RENDER-76`).
const RECT_EDGE_RAYS: Array[Vector2] = [Vector2(0.5, 1.0), Vector2(0.0, 0.5), Vector2(1.0, 0.5), Vector2(0.5, 0.0)]
# Frames over which the scaffold batches' proxies are checked round robin
# (water_reflection_sources.gd); a batch a few frames late only shows late in the reflection.
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
var _voted_levels := PackedFloat32Array([NAN, NAN])
# Inputs of the last successful vote. Compare against that pose so sub-epsilon motion accrues.
var _vote_sources := PackedInt64Array()
var _vote_valid := false
var _vote_transform := Transform3D.IDENTITY
var _vote_projection := Projection()
var _vote_rect := Rect2()
var _vote_offsets := Vector2.ZERO
# Microseconds the last frame's vote, water rectangles, terrain proxy sync and scaffold proxy sync
# took, for the perf channel.
var _vote_us := 0
var _rect_us := 0
# The resident water patches' bounds and their hull, kept in Rust by water.gd's events
# (`RENDER-76`): patch keys marked since the last frame, a count of the frames that changed the
# book (a side may rest on a patch inside the hull), the boundary as last read and its revision, and
# per mirror the inputs of its last rectangle with the rectangle ([book epoch, plane, screen,
# transform, camera transform, projection, near, rectangle]).
var _rect_hull: WaterScreenHull
var _rect_dirty := {}
var _rect_epoch := 0
var _rect_boundary := PackedVector2Array()
var _rect_revision := -1
var _rect_cache: Array = [[], []]
# The running extremes of the rectangle being made, in screen shares.
var _rect_lo := Vector2.ZERO
var _rect_hi := Vector2.ZERO
# Points the last frame's rectangles projected, and its rectangles that walked every resident
# patch, for the perf channel; walks since setup.
var _rect_points := 0
var _rect_walks := 0
var _rect_walks_total := 0
# METRUM_WATER_RECT_CHECK=1 (`RENDER-76`, an untimed run): every rectangle is made the old way too,
# by the walk; rectangles checked, and of them unequal.
var _rect_checking := false
var _rect_checked := 0
var _rect_differences := 0
var _proxy_us := 0
var _scaffold_us := 0
# Per vote ray, the water level it last came down onto, or NAN.
var _ray_levels := PackedFloat32Array()
var _ray_weights := PackedFloat32Array()
var _ray_directions := PackedVector3Array()
# The terrain patches the reflection world mirrors, by patch key, to [proxy instance RID,
# base RID, transform, material, custom AABB, extra cull margin]. Meshes are shared; terrain uses a
# cached clip-material variant.
var _proxies := {}
# Patch keys the last sync looked at, for the perf channel (`RENDER-75`).
var _proxy_visits := 0
var _terrain: Node
# Reflection variants of terrain materials, by source.
var _grid_materials := {}
# The proxy variant of each source shader, shared by every mirror and the menu's templates (one
# shader object, so the pipelines the menu compiled are the mirror's).
static var _proxy_shaders := {}
# Per source shader, its material parameters' names, which a refresh compares.
var _shader_parameters := {}
# Patch keys whose proxy may be out of step since the last sync, to whether the copy of their
# material must be refreshed (a render change) or not (only shown or hidden).
var _dirty_terrain_patches := {}
# The vegetation renderer: the stream node draws its tree impostors in this pass's world too
# (`RENDER-32-E`), from the frame they install in the scene.
var _vegetation: Node
# Scaffold batches mirrored as shared MultiMesh proxies (water_reflection_sources.gd); the building
# LOD batches' proxies are drawn by Rust with their batches (`RENDER-32-L`).
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
		_rect_hull = WaterScreenHull.new()
		water.patch_bounds_changed.connect(_on_water_patch_bounds_changed)
		water.patches_cleared.connect(_on_water_patches_cleared)
		# Patches made resident before this connection sent their events.
		for key: Vector2i in water.resident_patch_lookup:
			_rect_dirty[key] = true
	if _terrain != null:
		_terrain.patch_render_will_change.connect(_on_terrain_patch_will_change)
		_terrain.patch_visibility_changed.connect(_on_terrain_patch_visibility_changed)
		_terrain.patch_parameters_changed.connect(_on_terrain_patch_parameters_changed)
		_terrain.patches_will_reset.connect(_on_terrain_reset)
		_terrain.visibility_changed.connect(_on_terrain_visibility_changed)
		# Patches shown before these connections sent their events.
		_on_terrain_visibility_changed()
	_sun = sun
	_main_environment = environment
	_link_vegetation()

## Hands the reflection's world to the vegetation once both exist (setup may come before or after
## the world is made), and to the building batches (`RENDER-32-L`), which draw their proxies in it;
## a building renderer made later asks for it (`reflection_scenario`).
func _link_vegetation() -> void:
	if _vegetation != null and _world != null and _vegetation.has_method("set_reflection_scenario"):
		_vegetation.set_reflection_scenario(_world.scenario)
	if _world != null and is_inside_tree():
		get_tree().call_group("building_lod_renderers", "set_reflection_scenario", _world.scenario)

## The reflection's world, or an invalid RID before it exists.
func reflection_scenario() -> RID:
	return _world.scenario if _world != null else RID()

func _ready() -> void:
	add_to_group("water_reflections")
	_rect_checking = OS.get_environment("METRUM_WATER_RECT_CHECK") == "1"
	_world = World3D.new()
	_link_vegetation()
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
		viewport.msaa_3d = MIRROR_MSAA
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
	if is_instance_valid(_vegetation) and _vegetation.has_method("set_reflection_scenario"):
		_vegetation.set_reflection_scenario(RID())
	get_tree().call_group("building_lod_renderers", "set_reflection_scenario", RID())
	for proxy in _proxies.values():
		RenderingServer.free_rid(proxy[0])
	_proxies.clear()
	if RenderingServer.frame_pre_draw.is_connected(_update_before_draw):
		RenderingServer.frame_pre_draw.disconnect(_update_before_draw)
	RenderingServer.global_shader_parameter_set("water_reflection_map", null)
	RenderingServer.global_shader_parameter_set("water_reflection_map_2", null)
	RenderingServer.global_shader_parameter_set("water_reflection_plane", Vector4.ZERO)

# After every node has moved the camera and the key light for this frame. The perf channel
# records the hook as "water_reflection", with its parts' shares and the patches the sync visited.
func _update_before_draw() -> void:
	var start_us := Time.get_ticks_usec()
	_vote_us = 0
	_rect_us = 0
	_rect_points = 0
	_rect_walks = 0
	_proxy_us = 0
	_proxy_visits = 0
	_update_mirrors()
	if PerfDebug.is_enabled():
		PerfDebug.record("water_reflection", float(Time.get_ticks_usec() - start_us) / 1000.0, {
			"vote_ms": float(_vote_us) / 1000.0,
			"rect_ms": float(_rect_us) / 1000.0,
			"rect_points": _rect_points,
			"rect_walks": _rect_walks,
			"proxy_ms": float(_proxy_us) / 1000.0,
			"scaffold_ms": float(_scaffold_us) / 1000.0,
			"mirrors": int(_mirrors[0][3]) + int(_mirrors[1][3]),
			"proxy_visits": _proxy_visits})

func _update_mirrors() -> void:
	var scaffold_start := Time.get_ticks_usec()
	_building_proxies.sync()
	_scaffold_us = Time.get_ticks_usec() - scaffold_start
	var main_viewport := get_viewport()
	var main_camera := main_viewport.get_camera_3d() if main_viewport != null else null
	if main_camera == null or _sun == null:
		for mirror in _mirrors:
			_set_active(mirror, false)
		_publish_planes()
		return
	main_camera.cull_mask &= ~REFLECTION_LAYER
	var vote_start := Time.get_ticks_usec()
	var levels := _vote_planes(main_camera)
	_vote_us = Time.get_ticks_usec() - vote_start
	# While the world has water the proxies follow every frame's events, a mirror drawing or not,
	# so a mirror that turns on finds them in step: synced only then, a 1 s turn's marks came due
	# at once (1,015 patches, 131 ms in one frame, `RENDER-75`). A dry world keeps its marks, one
	# a patch, and pays nothing.
	if _proxies_follow():
		var proxy_start := Time.get_ticks_usec()
		_sync_proxies()
		_proxy_us = Time.get_ticks_usec() - proxy_start
	var hull_start := Time.get_ticks_usec()
	_sync_rect_hull()
	_rect_us = Time.get_ticks_usec() - hull_start
	var synced := false
	for index in MIRROR_COUNT:
		var mirror: Array = _mirrors[index]
		var level := levels[index]
		if is_nan(level) or main_camera.global_position.y <= level:
			_set_active(mirror, false)
			continue
		mirror[2] = level + WATER_SURFACE_LIFT_M
		var rect_start := Time.get_ticks_usec()
		var crop := _water_screen_rect(main_camera, mirror[2], index)
		_rect_us += Time.get_ticks_usec() - rect_start
		if not crop.has_area():
			_set_active(mirror, false)
			continue
		if not synced:
			synced = true
			_sync_environment()
			_sync_key_light()
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
#
# The target keeps one size, the largest crop's (the screen and its margins at RESOLUTION_SCALE),
# and changes only with the screen: resized to each frame's crop, it was freed and made again on
# every frame of a camera turn, and every material sampling it built its uniform set again
# (`RENDER-27`). The crop's own texels, the size it would have had alone, are its top left block,
# the frustum widened right and down over the rest: each of them sees the ray it saw in a target
# of the crop's size at the same pixel coordinate (the vegetation's dithered crossfades read
# FRAGCOORD), and the water keeps its fade and clamp to that block (`water_reflection_rect`), so
# the picture it samples is that one.
func _place_camera(
	slot: Array, index: int, main_camera: Camera3D, main_viewport: Viewport, crop: Rect2
) -> void:
	var viewport: SubViewport = slot[0]
	var camera: Camera3D = slot[1]
	var screen := Vector2(main_viewport.get_visible_rect().size)
	var target := Vector2i((screen * (1.0 + 2.0 * CROP_MARGIN) * RESOLUTION_SCALE).round()).max(
		Vector2i.ONE)
	var size := Vector2i((screen * crop.size * RESOLUTION_SCALE).round()).clamp(Vector2i.ONE, target)
	if viewport.size != target:
		viewport.size = target
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
	# camera sees it. An off-axis frustum over that rectangle draws what the water shows, and over
	# the target's spare texels what it never samples.
	var height := 2.0 * main_camera.near * tan(deg_to_rad(main_camera.fov) * 0.5)
	var width := height * screen.x / maxf(screen.y, 1.0)
	var centre := crop.get_center()
	# A texel's side on the near plane: the crop's height over its own texel rows (the width
	# follows the target's aspect, KEEP_HEIGHT). The frustum keeps the crop's left and top edges
	# and grows by the spare texels to the right and down (the near plane's y points up).
	var texel := height * crop.size.y / float(size.y)
	var spare := Vector2(target - size) * texel
	camera.projection = Camera3D.PROJECTION_FRUSTUM
	camera.keep_aspect = Camera3D.KEEP_HEIGHT
	camera.size = texel * float(target.y)
	camera.frustum_offset = Vector2((centre.x - 0.5) * width + spare.x * 0.5,
		(0.5 - centre.y) * height - spare.y * 0.5)
	camera.near = main_camera.near
	camera.far = main_camera.far
	var view_projection: Projection = (
		camera.get_camera_projection() * Projection(camera.global_transform.affine_inverse())
	)
	RenderingServer.global_shader_parameter_set(
		"water_reflection_view_projection" if index == 0 else "water_reflection_view_projection_2",
		view_projection
	)
	# The crop's block, which the water's fade and clamp keep to (`water_planar_reflection`).
	RenderingServer.global_shader_parameter_set(
		"water_reflection_rect" if index == 0 else "water_reflection_rect_2",
		Vector4(0.0, 0.0, float(size.x) / float(target.x), float(size.y) / float(target.y)))

# Screen rectangle, in [0, 1], covering the water: the union of the resident water patches laid
# on the plane at `plane_y`, each clipped to the space in front of the camera, with room for the
# ripples. Patch bounds are coarse (a dry patch's is its centre), so the rectangle can include land,
# never exclude water. Empty when no such patch is in front of the camera. It crops the mirror's
# frustum and the block of its target the water samples (_place_camera), and an empty one turns the
# mirror off: a pixel of difference moves every texel of the reflection, so it is kept exactly
# equal to the walk over every resident patch (_water_screen_rect_walk), not approximated.
#
# Cost (`RENDER-76`): O(1) when no water patch event came, and the plane, the screen and the camera
# are as at this mirror's last rectangle. Otherwise O(h) projections, h the hull's boundary corners (the resident
# set's outline: it does not grow with the patches inside it), plus, where the near plane cuts the
# hull, at most 63 patches: 3 x 3 about the cut's two ends, the camera's foot and the ground of the
# screen edges' middles. The walk, O(resident patches), only when those cannot settle a side
# (counted as `rect_walks`).
func _water_screen_rect(camera: Camera3D, plane_y: float, index: int) -> Rect2:
	if _water == null:
		return Rect2(0.0, 0.0, 1.0, 1.0)
	var rect := (_water_screen_rect_kept(camera, plane_y, index) if _rect_hull != null
		else _water_screen_rect_walk(camera, plane_y))
	if _rect_checking:
		_rect_checked += 1
		var walked := _water_screen_rect_walk(camera, plane_y)
		if walked != rect:
			_rect_differences += 1
			MetrumDebug.log("terrain", "water rect differs: kept %s walk %s" % [rect, walked])
	return rect

# The rectangle from the resident patches' hull. A perspective is a projective map on the half-plane
# in front of the camera (its divisor, the depth, is positive there), so each screen coordinate is
# monotone along every segment there and takes its extremes over a convex region at the region's
# vertices. The walk's points (the patches' corners in front, and where their edges cross the
# clipping line) all lie in the hull cut to that half-plane, whose vertices are the hull's corners
# in front (each one a patch's corner the walk projects too) and the two points where the hull's
# boundary crosses the clipping line. A side is settled exactly when the corners give an extreme
# beyond what those two crossings can (the walk's own point attains it), or when a point of the
# walk's own set reaches the screen's edge there (the side is clamped to the margin past it either
# way). The hull keeps collinear boundary corners, so a side whose extreme lies along a hull edge
# takes the float the walk would take from any of them.
func _water_screen_rect_kept(camera: Camera3D, plane_y: float, index: int) -> Rect2:
	var revision := _rect_hull.revision()
	if revision != _rect_revision:
		_rect_boundary = _rect_hull.boundary()
		_rect_revision = revision
	var screen := camera.get_viewport().get_visible_rect().size
	var transform := camera.global_transform
	var view := camera.get_camera_transform()
	var projection := camera.get_camera_projection()
	var cache: Array = _rect_cache[index]
	if (not cache.is_empty() and cache[0] == _rect_epoch and cache[1] == plane_y
			and cache[2] == screen and cache[3] == transform and cache[4] == view
			and cache[5] == projection and cache[6] == camera.near):
		return cache[7]
	var forward := -transform.basis.z
	var eye := camera.global_position
	var near_m := camera.near * 1.5
	_rect_lo = Vector2(INF, INF)
	_rect_hi = Vector2(-INF, -INF)
	var cut_lo := Vector2(INF, INF)
	var cut_hi := Vector2(-INF, -INF)
	var cuts := PackedVector3Array()
	var count := _rect_boundary.size()
	var previous := Vector3.ZERO
	var previous_d := 0.0
	var first := Vector3.ZERO
	var first_d := 0.0
	for i in count + 1:
		var a := first
		var da := first_d
		if i < count:
			var corner := _rect_boundary[i]
			a = Vector3(corner.x, plane_y, corner.y)
			da = (a - eye).dot(forward) - near_m
			if da >= 0.0:
				var point := camera.unproject_position(a) / screen
				_rect_lo = _rect_lo.min(point)
				_rect_hi = _rect_hi.max(point)
				_rect_points += 1
			if i == 0:
				first = a
				first_d = da
		if i > 0 and (da >= 0.0) != (previous_d >= 0.0):
			var cut := previous.lerp(a, previous_d / (previous_d - da))
			var point := camera.unproject_position(cut) / screen
			cut_lo = cut_lo.min(point)
			cut_hi = cut_hi.max(point)
			cuts.append(cut)
			_rect_points += 1
		previous = a
		previous_d = da
	if not cuts.is_empty() and not _rect_settled(cut_lo, cut_hi):
		# The patches about the cut's ends and the camera's foot, then about where the rays through
		# the middles of the screen's edges meet the plane, add the walk's own points there: those
		# reach past a screen edge wherever water lies beyond it.
		cuts.append(eye)
		for cut in cuts:
			_add_patches_near(cut, camera, plane_y, eye, forward, near_m, screen)
		for edge: Vector2 in RECT_EDGE_RAYS:
			if _rect_settled(cut_lo, cut_hi):
				break
			var pixel := edge * screen
			var origin := camera.project_ray_origin(pixel)
			var direction := camera.project_ray_normal(pixel)
			if direction.y < -1.0e-6 and (plane_y - origin.y) / direction.y > 0.0:
				_add_patches_near(origin + direction * ((plane_y - origin.y) / direction.y), camera,
					plane_y, eye, forward, near_m, screen)
		if not _rect_settled(cut_lo, cut_hi):
			_rect_walks += 1
			_rect_walks_total += 1
			var walked := _water_screen_rect_walk(camera, plane_y)
			_rect_cache[index] = [_rect_epoch, plane_y, screen, transform, view, projection,
				camera.near, walked]
			return walked
	var rect := _rect_from_extremes()
	_rect_cache[index] = [_rect_epoch, plane_y, screen, transform, view, projection, camera.near,
		rect]
	return rect

# Whether every side of the running extremes is the walk's (see _water_screen_rect_kept), given the
# extremes of where the hull's boundary crosses the clipping line. Slack covers the crossings' own
# rounding.
func _rect_settled(cut_lo: Vector2, cut_hi: Vector2) -> bool:
	for axis in 2:
		var high := _rect_hi[axis]
		var low := _rect_lo[axis]
		if not (high >= 1.0 or high > cut_hi[axis] + 1.0e-4 * maxf(1.0, absf(cut_hi[axis]))):
			return false
		if not (low <= 0.0 or low < cut_lo[axis] - 1.0e-4 * maxf(1.0, absf(cut_lo[axis]))):
			return false
	return true

# Adds the walk's points of the resident patches in the 3 x 3 patch keys about `point`.
func _add_patches_near(
	point: Vector3, camera: Camera3D, plane_y: float, eye: Vector3, forward: Vector3, near_m: float,
	screen: Vector2
) -> void:
	var span: float = _water.terrain_patch_span_m
	if span <= 0.0:
		return
	var half: Vector2 = _water.terrain_world_size * 0.5
	var centre := Vector2i(int(floor((point.x + half.x) / span)), int(floor((point.z + half.y) / span)))
	for dz in range(-1, 2):
		for dx in range(-1, 2):
			var key := centre + Vector2i(dx, dz)
			if _water.resident_patch_lookup.has(key):
				_add_patch_points(key, camera, plane_y, eye, forward, near_m, screen)

# The rectangle the walk, as before `RENDER-76`: every resident water patch, projected. O(resident
# water patches). A water patch is drawn only while resident, and the built ones that left are only
# hidden, so a walk over every built patch grew with the distance travelled (`RENDER-75`).
func _water_screen_rect_walk(camera: Camera3D, plane_y: float) -> Rect2:
	var screen := camera.get_viewport().get_visible_rect().size
	var forward := -camera.global_transform.basis.z
	var eye := camera.global_position
	var near_m := camera.near * 1.5
	_rect_lo = Vector2(INF, INF)
	_rect_hi = Vector2(-INF, -INF)
	for key: Vector2i in _water.resident_patch_lookup:
		_add_patch_points(key, camera, plane_y, eye, forward, near_m, screen)
	return _rect_from_extremes()

# Projects patch `key`'s bounds laid on the plane, clipped to the space in front of the camera,
# into the running extremes.
func _add_patch_points(
	key: Vector2i, camera: Camera3D, plane_y: float, eye: Vector3, forward: Vector3, near_m: float,
	screen: Vector2
) -> void:
	var node: MeshInstance3D = (_water.patches.get(key, {}) as Dictionary).get("node")
	if node == null or not node.visible:
		return
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
			_rect_lo = _rect_lo.min(point)
			_rect_hi = _rect_hi.max(point)
		if (da >= 0.0) != (db >= 0.0):
			var point := camera.unproject_position(a.lerp(b, da / (da - db))) / screen
			_rect_lo = _rect_lo.min(point)
			_rect_hi = _rect_hi.max(point)
	_rect_points += 4

# The running extremes as the mirror's crop: room for the ripples' bend, kept past the screen's own
# edges too: the water fades the reflection out over the outer 2% of the map
# (water_planar_reflection), and with the rectangle clipped to the screen that fade showed as an
# unreflected sliver along each side.
func _rect_from_extremes() -> Rect2:
	var lo := _rect_lo
	var hi := _rect_hi
	if lo.x > hi.x:
		return Rect2()
	var margin := Vector2(CROP_MARGIN, CROP_MARGIN)
	lo = (lo - margin).max(-margin)
	hi = (hi + margin).min(Vector2.ONE + margin)
	if lo.x >= hi.x or lo.y >= hi.y:
		return Rect2()
	return Rect2(lo, hi - lo)

# A water patch's part in the rectangle may have changed (water.gd's events); the next frame brings
# the hull in step. A dict, so a patch's several events in a frame cost one update.
func _on_water_patch_bounds_changed(key: Vector2i) -> void:
	_rect_dirty[key] = true

func _on_water_patches_cleared() -> void:
	_rect_dirty.clear()
	_rect_epoch += 1
	if _rect_hull != null:
		_rect_hull.clear()

# Brings the hull to the marked patches: a resident, shown patch holds its bounds on the ground
# (the walk's condition, and its arithmetic for the bounds); any other holds none. O(patches marked
# since the last frame), each O(log corners) in Rust. The bounds are read in world space once: the
# water node never moves.
func _sync_rect_hull() -> void:
	if _rect_hull == null or _rect_dirty.is_empty():
		return
	_rect_epoch += 1
	for key: Vector2i in _rect_dirty:
		var node: MeshInstance3D = (_water.patches.get(key, {}) as Dictionary).get("node")
		if node != null and node.visible and _water.resident_patch_lookup.has(key):
			var bounds := node.global_transform * node.get_aabb()
			_rect_hull.set_patch(key, Vector2(bounds.position.x, bounds.position.z),
				Vector2(bounds.end.x, bounds.end.z))
		else:
			_rect_hull.remove_patch(key)
	_rect_dirty.clear()

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
# VOTE_MAX_DISTANCE_M / 4 m) samples only when a vote input changes; O(1) otherwise.
func _vote_planes(camera: Camera3D) -> PackedFloat32Array:
	if _simulation == null:
		return _voted_levels
	var sources: PackedInt64Array = _simulation.get_water_reflection_sources(_vote_sources)
	# No lock means retry next frame, not a successful vote for these inputs.
	if sources.is_empty():
		return _voted_levels
	if sources[2] == 0:
		if sources != _vote_sources:
			_voted_levels = PackedFloat32Array([NAN, NAN])
			_ray_levels.clear()
			_vote_sources = sources
			_vote_valid = false
		return _voted_levels
	var rect := camera.get_viewport().get_visible_rect()
	var transform := camera.global_transform
	var projection := camera.get_camera_projection()
	var offsets := Vector2(camera.h_offset, camera.v_offset)
	# Absolute tolerances: relative approximate equality grows too loose far from world zero.
	if _vote_valid and sources == _vote_sources and rect == _vote_rect \
			and projection == _vote_projection and offsets == _vote_offsets \
			and transform.origin.distance_squared_to(_vote_transform.origin) <= 1.0e-8 \
			and transform.basis.x.distance_squared_to(_vote_transform.basis.x) <= 1.0e-12 \
			and transform.basis.y.distance_squared_to(_vote_transform.basis.y) <= 1.0e-12 \
			and transform.basis.z.distance_squared_to(_vote_transform.basis.z) <= 1.0e-12:
		return _voted_levels
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
	if levels.size() != _ray_directions.size():
		return _voted_levels
	_ray_levels = levels
	_vote_sources = sources
	_vote_transform = transform
	_vote_projection = projection
	_vote_rect = rect
	_vote_offsets = offsets
	_vote_valid = true
	# Rust owns selection and its hysteresis. Row weights are immutable screen shares; a second
	# lake is ranked against other lakes, without a separate absolute entry/exit threshold.
	if _ray_weights.is_empty():
		_ray_weights.resize(VOTE_COLUMNS * VOTE_ROWS)
		for index in _ray_weights.size():
			var row := index / VOTE_COLUMNS
			_ray_weights[index] = float(2 * row + 1) / float(VOTE_ROWS * VOTE_ROWS * VOTE_COLUMNS)
	_voted_levels = _simulation.select_water_reflection_planes(_ray_levels, _ray_weights, _voted_levels)
	return _voted_levels

# Mirrors the resident terrain patches into the reflection world; tree impostors are drawn there
# by the stream node, building batches by Rust with their batches, scaffolds by their events
# (water_reflection_sources.gd).
# Agents, grass, props, understory, near tree meshes, hedges, lots and roads are left out; a
# bridge would need its own geometry first.
# A proxy follows its patch's node: the events terrain.gd sends when it changes what a patch
# draws (patch_render_will_change: a restage, a removal) or shows or hides it
# (patch_visibility_changed), the reset, and the terrain node's own visibility mark the patch, and
# the next sync brings exactly the marked patches' proxies in step. A round robin over every built
# patch (hidden ones included, which only grow with the distance travelled) cost 2 us a built
# patch a frame: 12 ms at 5,459 on a 60 km map (`RENDER-75`).
func _on_terrain_patch_will_change(key: Vector2i) -> void:
	_dirty_terrain_patches[key] = true

func _on_terrain_patch_visibility_changed(key: Vector2i) -> void:
	if not _dirty_terrain_patches.has(key):
		_dirty_terrain_patches[key] = false

# A patch's material changed in exactly `names` (a level of detail step, a water map, a land
# cover; a step also gave the node a new mesh). While the proxies follow (a wet world), a proxy
# whose copy is in step takes the new mesh and those values here, inside the unit that changed
# them (a level change, the water texture sync, a land cover commit), which measures it as its own
# (`TERRAIN-41` L1b). A whole refresh at the next sync compared every parameter, 34 us a patch on
# the shipped Kuopio: a camera jump's frame of 116 steps spent 4.5 ms on it after the 2.0 ms
# install ledger. A patch with a refresh pending or without a proxy is marked for the refresh.
# O(names).
func _on_terrain_patch_parameters_changed(key: Vector2i, names: Array[StringName]) -> void:
	if _dirty_terrain_patches.get(key, false):
		return
	var proxy: Array = _proxies.get(key, [])
	if _proxies_follow() and proxy.size() >= 6 and _terrain != null:
		var node := (_terrain.patches.get(key, {}) as Dictionary).get("node") as MeshInstance3D
		var source: ShaderMaterial = null
		if node != null:
			source = node.material_override as ShaderMaterial
		if node != null and node.mesh != null and source != null and proxy[3] == _grid_materials.get(source):
			var base := node.mesh.get_rid()
			if proxy[1] != base:
				RenderingServer.instance_set_base(proxy[0], base)
				proxy[1] = base
			_copy_parameters(proxy[3] as ShaderMaterial, source, names)
			return
	_dirty_terrain_patches[key] = true

# Hiding or showing the terrain node shows or hides every patch with it. O(built patches), once a
# toggle (a debug view or a benchmark variant), never a frame.
func _on_terrain_visibility_changed() -> void:
	for key: Vector2i in _terrain.patches:
		_on_terrain_patch_visibility_changed(key)

func _on_terrain_reset() -> void:
	for proxy in _proxies.values():
		RenderingServer.free_rid(proxy[0])
	_proxies.clear()
	_grid_materials.clear()
	_dirty_terrain_patches.clear()
	_voted_levels = PackedFloat32Array([NAN, NAN])
	_vote_valid = false
	_vote_sources.clear()
	_ray_levels.clear()

# Brings the marked patches' proxies in step with their nodes: a drawn patch gets its proxy, made
# or updated; a hidden, removed or meshless one loses it. O(patches marked since the last sync)
# a sync, independent of the built and the drawn patches; a frame's sync is that frame's events
# (marks wait only on a dry world or before the first vote, at most one a patch). The proxy book
# after a sync is one proxy per drawn patch, holding its mesh, transform, material copy, culling
# box and margin.
func _sync_proxies() -> void:
	var scenario := _world.scenario
	_proxy_visits = _dirty_terrain_patches.size()
	for key: Vector2i in _dirty_terrain_patches:
		var patch: Dictionary = _terrain.patches.get(key, {}) if _terrain != null else {}
		var node := patch.get("node") as MeshInstance3D
		if node == null or not node.is_visible_in_tree() or node.mesh == null:
			var proxy: Array = _proxies.get(key, [])
			if not proxy.is_empty():
				RenderingServer.free_rid(proxy[0])
				_proxies.erase(key)
			continue
		var material := node.material_override as ShaderMaterial
		# Share the exact displayed mesh: a coarser grid (or a heightmap substitute for
		# baked ground) crosses the lake plane at a different shore and masks its reflection.
		if material != null:
			material = _proxy_material(material, _dirty_terrain_patches[key])
		_sync_proxy(key, node, node.mesh.get_rid(), material, scenario)
	_dirty_terrain_patches.clear()

func _sync_proxy(
	key: Vector2i, source: GeometryInstance3D, base: RID, material: Material, scenario: RID
) -> void:
	var proxy: Array = _proxies.get(key, [])
	if proxy.is_empty():
		var instance := RenderingServer.instance_create2(base, scenario)
		RenderingServer.instance_geometry_set_cast_shadows_setting(
			instance, RenderingServer.SHADOW_CASTING_SETTING_OFF)
		proxy = [instance, base]
		_proxies[key] = proxy
	elif proxy[1] != base:
		# A level change gives the patch a new mesh: the instance takes it as its base and keeps
		# its scenario, transform, material and bounds, where a new instance (freed and created
		# again, every patch the level rings swept) cost the camera turn's frames (`RENDER-27`).
		RenderingServer.instance_set_base(proxy[0], base)
		proxy[1] = base
	# Only what changed is sent: pooled patch nodes move and change material, but most syncs
	# find every proxy as it was, and re-sending hundreds of instances stalled a frame.
	var instance: RID = proxy[0]
	var transform := source.global_transform
	if (proxy.size() < 3 or proxy[2] != transform or proxy[3] != material
			or proxy[4] != source.custom_aabb or proxy[5] != source.extra_cull_margin):
		RenderingServer.instance_set_transform(instance, transform)
		RenderingServer.instance_geometry_set_material_override(
			instance, material.get_rid() if material != null else RID())
		if source.custom_aabb.has_volume():
			RenderingServer.instance_set_custom_aabb(instance, source.custom_aabb)
		RenderingServer.instance_set_extra_visibility_margin(instance, source.extra_cull_margin)
		proxy.resize(6)
		proxy[2] = transform
		proxy[3] = material
		proxy[4] = source.custom_aabb
		proxy[5] = source.extra_cull_margin

# Use the source geometry and its displacement mode; only the mirror clip and underside
# shading differ. Cached by source.
func _proxy_material(source: ShaderMaterial, refresh: bool = false) -> ShaderMaterial:
	var copy: ShaderMaterial = _grid_materials.get(source)
	if copy == null:
		copy = source.duplicate() as ShaderMaterial
		copy.shader = _proxy_shader(source.shader)
		_grid_materials[source] = copy
	elif refresh:
		# Terrain swaps its height texture in-place on the same material. An old copy otherwise
		# keeps the previous patch's shore after streaming, edits or LOD changes.
		_refresh_proxy_material(copy, source)
	return copy

# Brings `copy`'s `names` to the values `source` holds now. O(names).
func _copy_parameters(copy: ShaderMaterial, source: ShaderMaterial, names: Array[StringName]) -> void:
	for parameter: StringName in names:
		var value: Variant = source.get_shader_parameter(parameter)
		var held: Variant = copy.get_shader_parameter(parameter)
		# Unlike values of two types do not compare in GDScript: the type first.
		if typeof(held) != typeof(value) or held != value:
			copy.set_shader_parameter(parameter, value)

# Whether the proxies follow the terrain's events every frame: while the world has water (see
# _update_mirrors). A dry world keeps its marks, one a patch, and pays nothing.
func _proxies_follow() -> bool:
	return _vote_sources.size() > 2 and _vote_sources[2] != 0

# Brings `copy` to the parameters `source` holds now, in place: what a new duplicate would hold,
# without the new material whose new resource and uniform set cost the camera turn's frames
# (`RENDER-27`). Only the parameters that differ are sent, so a level change (one float) does not
# rebind the textures. O(the shader's parameters) comparisons.
func _refresh_proxy_material(copy: ShaderMaterial, source: ShaderMaterial) -> void:
	var shader := _proxy_shader(source.shader)
	if copy.shader != shader:
		copy.shader = shader
	if copy.render_priority != source.render_priority:
		copy.render_priority = source.render_priority
	if copy.next_pass != source.next_pass:
		copy.next_pass = source.next_pass
	var names: Array = _shader_parameters.get(source.shader, [])
	if names.is_empty() and source.shader != null:
		for entry: Dictionary in source.shader.get_shader_uniform_list():
			names.append(StringName(entry["name"]))
		_shader_parameters[source.shader] = names
	for parameter: StringName in names:
		var value: Variant = source.get_shader_parameter(parameter)
		var held: Variant = copy.get_shader_parameter(parameter)
		# Unlike values of two types do not compare in GDScript: the type first.
		if typeof(held) != typeof(value) or held != value:
			copy.set_shader_parameter(parameter, value)

## The menu's pipeline prewarm template of the mirror (`RENDER-43`): the terrain proxies' shader
## on a patch's PlaneMesh.
static func pipeline_prewarm_surfaces() -> Array:
	var material := ShaderMaterial.new()
	material.shader = _proxy_shader(TERRAIN_SHADER)
	return [[PlaneMesh.new(), material]]

static func _proxy_shader(source: Shader) -> Shader:
	var shader: Shader = _proxy_shaders.get(source)
	if shader == null:
		shader = Shader.new()
		shader.code = source.code.replace("shader_type spatial;",
			"shader_type spatial;\n#define WATER_REFLECTION_PROXY")
		_proxy_shaders[source] = shader
	return shader
