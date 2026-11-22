# SPDX-License-Identifier: GPL-3.0-or-later

## Agent renderer — streams agent positions from Rust into a MultiMeshInstance3D each frame.
##
## Rust methods called: advance_agent_frame(), get_car_frame_parts(), get_agent_frame_parts(),
##   get_vehicle_catalogue(), get_agent_paths_debug(), update_agent_view(),
##   get_agent_draw_distances(), get_agent_snapshot_parts(); on the StreamNode
##   attach_agent_casters(), register_agent_caster(), unregister_agent_casters()
## The agents' view (the ground rectangle the snapshot culls to, and the eye their draw distances
## measure from) comes from the camera in Rust. Walkers and moving cars fade out by a dither over
## a band before their draw distance (`RENDER-28`); the snapshot does not pose them beyond it.
## Only the buckets whose drawn buffer changed are uploaded: a car at rest is drawn as it was, and
## walkers upload only when the snapshot moved them. The walkers, the cars' smoothing and ground
## solve are prepared by a background job a frame ahead (`RENDER-47`): each frame installs what the
## job prepared in the frame before and starts the next. The bucket nodes cast no shadow: the
## StreamNode's shadow-only twins cast for the agents within reach of the sun's shadow range
## (`RENDER-15`); without a StreamNode the bucket nodes cast as before.
## Agent rows arrive as a flat PackedFloat32Array of 16 floats per agent: the MultiMesh
## row-major 3×4 transform [xx, yx, zx, ox, xy, yy, zy, oy, xz, yz, zz, oz], then the instance
## custom data (a walker's walk phase and look, `RENDER-71`; a car's steer, wheel roll, packed
## lamp/paint/blink word and a spare, `RENDER-65`). A car bucket is `look * 4 + tier` of Rust's
## vehicle catalogue: its node is made when the bucket is first drawn, and every car node is
## remade when the frame's catalogue revision changes (packs loaded).
## Path debug lines (toggled with P key) arrive as a PackedVector3Array of point pairs.
extends Node3D

const SceneLightingConfig := preload("res://scripts/core/scene_lighting.gd")
const PerfDebug := preload("res://scripts/core/perf_debug.gd")
const VehicleHeadlights := preload("res://scripts/renderers/vehicle_headlights.gd")
const PipelineTemplate := preload("res://scripts/renderers/pipeline_template.gd")
const WALK_SHADER := preload("res://scripts/shaders/pedestrian_walk.gdshader")

@onready var simulation_node = $"../SimulationNode"

# Key: pedestrian_type (int), Value: MultiMeshInstance3D
var walker_mmis: Dictionary = {}
# Key: car bucket (look * 4 + tier), Value: MultiMeshInstance3D
var car_mmis: Dictionary = {}
## The drawn buffers per bucket as Rust installed them (walker type, car type key).
var walker_buffers: Dictionary = {}
var car_buffers: Dictionary = {}
var texture_cache: Dictionary = {}
const WALKER_VAT_FRAMES := 30.0
# Quaternius walk clips are baked in the opposite temporal direction from our distance phase.
const WALKER_VAT_REVERSE_PHASE := true
# A car's transform, then its custom data.
const CAR_STRIDE := 16
# A walker's transform, then its custom data (the walk phase and look, then two spare).
const WALKER_STRIDE := 16
const DEBUG_LABEL_LIMIT := 96
const SWARM_PARTS: Array[String] = ["agent_frame", "walker_assign", "car_assign", "headlights"]
# The installed frame's job on the background pool, the main thread's install, whether it
# installed one, late jobs so far (`RENDER-47`).
const FRAME_PARTS: Array[String] = ["agent_job", "agent_install", "agent_installed", "agent_late"]
const CAR_PARTS: Array[String] = ["car_interpolate", "car_ground", "car_record", "cars_smoothed", "cars_off_lane", "off_lane_solved", "cars_at_rest", "off_lane_at_rest", "car_buckets_uploaded", "car_buckets_kept"]
var _vehicles := preload("res://scripts/renderers/vehicle_resources.gd").new()
var _headlights := VehicleHeadlights.new()

var debug_mesh_instance: MeshInstance3D
var debug_mesh: ImmediateMesh
var debug_labels: Array = []
var show_paths = false
var _debug_overlay_visible := false
# The last swarm update's parts in microseconds, for the perf channel (`RENDER-28`): the agents'
# frame (install and the next job's start), walker buffers, car buffers, headlights. Written in
# place every update.
var _swarm_us := PackedInt64Array([0, 0, 0, 0])
var _walker_buckets_uploaded := 0
var _casters_attached := false


## The menu's pipeline prewarm template of the walkers (`RENDER-43`): their VAT shader on the
## walker meshes' vertex format (positions, normals, tangents, UV and UV2, compressed).
static func pipeline_prewarm_surfaces() -> Array:
	var format := (Mesh.ARRAY_FORMAT_NORMAL | Mesh.ARRAY_FORMAT_TANGENT | Mesh.ARRAY_FORMAT_TEX_UV
		| Mesh.ARRAY_FORMAT_TEX_UV2)
	return [[PipelineTemplate.format_mesh(format, true), PipelineTemplate.bare(WALK_SHADER)]]

func _ready():
	# --- Walker MultiMeshes — VAT (Vertex Animation Texture) pipeline ---
	# Assets baked from .blend source by tools/bake_vat_blend.py.
	# The GLTF rest mesh has clean Y-up orientation (no FBX rotation offset).
	var vat_base = "res://assets/models/characters/Quaternius/VAT/"
	var person_meshes = {
		0: vat_base + "male_walk_rest.gltf",
		1: vat_base + "male_walk_rest.gltf",
		2: vat_base + "female_walk_rest.gltf",
		3: vat_base + "female_walk_rest.gltf",
	}
	var person_vat_tex = {
		0: vat_base + "male_vat_walk.exr",
		1: vat_base + "male_vat_walk.exr",
		2: vat_base + "female_vat_walk.exr",
		3: vat_base + "female_vat_walk.exr",
	}
	var person_palettes = {
		0: vat_base + "male_palette.png",
		1: vat_base + "male_palette.png",
		2: vat_base + "female_palette.png",
		3: vat_base + "female_palette.png",
	}
	# Palette texels the shader recolours per walker: top, trousers (-1: none) and hair. The
	# female model's top is a dress, so it has no separate trousers.
	var person_garment_slots = {
		0: Vector3i(3, 4, 2),
		1: Vector3i(3, 4, 2),
		2: Vector3i(3, -1, 2),
		3: Vector3i(3, -1, 2),
	}
	var person_tints = {
		0: Vector3(1.0, 1.0, 1.0),
		1: Vector3(0.86, 0.92, 1.0),
		2: Vector3(1.0, 1.0, 1.0),
		3: Vector3(1.0, 0.9, 0.92),
	}

	# Walker fade start and draw distance, then the cars', from the camera in metres.
	var draw: PackedFloat32Array = SimulationNode.get_agent_draw_distances()
	_vehicles.draw_fade_m = Vector2(draw[2], draw[3])
	var console := get_node_or_null("/root/DebugConsole")
	if console != null:
		console.visual_mode_changed.connect(_on_visual_mode_changed)
	# The VAT debug view index is the shader's `debug_mode`.
	var pedestrian_vat_debug_mode := MetrumDebug.visual_mode_index("pedestrian_vat")

	for p_type in person_meshes:
		var gltf_doc   := GLTFDocument.new()
		var gltf_state := GLTFState.new()
		var err := gltf_doc.append_from_file(person_meshes[p_type], gltf_state)
		if err != OK:
			push_error("Failed to load VAT mesh: " + person_meshes[p_type])
			continue

		var node := gltf_doc.generate_scene(gltf_state)
		if not node:
			push_error("Could not generate scene from: " + person_meshes[p_type])
			continue

		# Source .blend is scale=1 rotation=0 → GLTF has no node rotation.
		# _extract_first_mesh returns the raw mesh resource (no baking needed).
		var mesh = _extract_first_mesh(node)
		node.free()
		if not mesh:
			push_error("No mesh found in: " + person_meshes[p_type])
			continue

		var vat_path  := ProjectSettings.globalize_path(person_vat_tex[p_type])
		var vat_image := Image.load_from_file(vat_path)
		if not vat_image:
			push_error("Could not load VAT texture: " + person_vat_tex[p_type])
			continue
		var vat_tex := ImageTexture.create_from_image(vat_image)
		# Note: In Godot 4, the 'filter_nearest' hint in the shader
		# takes care of this perfectly.

		var mmi := MultiMeshInstance3D.new()
		var mm  := MultiMesh.new()
		mm.transform_format = MultiMesh.TRANSFORM_3D
		mm.use_colors       = false
		mm.use_custom_data  = true   # INSTANCE_CUSTOM.x = walk_phase, .y = look, both [0..1)
		mm.instance_count   = 0
		mm.mesh             = mesh
		mmi.multimesh       = mm
		SceneLightingConfig.apply_shadow_policy(
			mmi,
			SceneLightingConfig.SHADOW_DYNAMIC_CASTER,
			"walkers"
		)

		var mat := ShaderMaterial.new()
		mat.shader = WALK_SHADER
		mat.set_shader_parameter("vat_texture",    vat_tex)
		mat.set_shader_parameter("albedo_texture", _load_source_texture(person_palettes[p_type]))
		mat.set_shader_parameter("num_frames",     WALKER_VAT_FRAMES)
		mat.set_shader_parameter("albedo_tint",    person_tints[p_type])
		mat.set_shader_parameter("garment_slots",  person_garment_slots[p_type])
		mat.set_shader_parameter("debug_mode",     pedestrian_vat_debug_mode)
		mat.set_shader_parameter("reverse_phase",  WALKER_VAT_REVERSE_PHASE)
		mat.set_shader_parameter("draw_fade_m",    Vector2(draw[0], draw[1]))
		mmi.material_override = mat

		add_child(mmi)
		walker_mmis[p_type] = mmi


	# --- Car MultiMeshes (Civilians) ---
	add_child(_headlights)
	_attach_shadow_casters()

	debug_mesh_instance = MeshInstance3D.new()
	debug_mesh = ImmediateMesh.new()
	var debug_mat = StandardMaterial3D.new()
	debug_mat.vertex_color_use_as_albedo = true
	debug_mat.albedo_color = Color.WHITE # Use vertex colors directly
	debug_mat.shading_mode = BaseMaterial3D.SHADING_MODE_UNSHADED
	debug_mat.transparency = BaseMaterial3D.TRANSPARENCY_ALPHA
	debug_mat.albedo_color.a = 0.7
	debug_mesh_instance.material_override = debug_mat
	debug_mesh_instance.mesh = debug_mesh
	SceneLightingConfig.apply_shadow_policy(
		debug_mesh_instance,
		SceneLightingConfig.SHADOW_DEBUG_OVERLAY,
		"walkers"
	)
	add_child(debug_mesh_instance)

	# A traffic session launched with `--debug traffic` starts with the path overlay on.
	if MetrumDebug.is_enabled("traffic"):
		show_paths = true
		print("Traffic visual debug overlay enabled (P toggles)")


func _notification(what: int) -> void:
	# A renderer that never entered the tree (tests) never parented its headlight pool.
	if what == NOTIFICATION_PREDELETE and _headlights.get_parent() == null:
		_headlights.free()

func _process(delta):
	if not PerfDebug.is_enabled():
		_update_view()
		# Update every rendered frame so cars do not visually quantize to 12 Hz at fast speeds;
		# only the buckets that changed upload.
		update_swarm(delta)
		return
	var frame_start_us := Time.get_ticks_usec()
	var aabb_start_us := frame_start_us
	_update_view()
	var aabb_elapsed_ms := float(Time.get_ticks_usec() - aabb_start_us) / 1000.0
	# Update every rendered frame so cars do not visually quantize to 12 Hz at fast speeds.
	var swarm_start_us := Time.get_ticks_usec()
	update_swarm(delta)
	var swarm_elapsed_ms := float(Time.get_ticks_usec() - swarm_start_us) / 1000.0
	var elapsed_ms := float(Time.get_ticks_usec() - frame_start_us) / 1000.0
	var details := {"camera_aabb": aabb_elapsed_ms, "swarm": swarm_elapsed_ms}
	for index in SWARM_PARTS.size():
		details[SWARM_PARTS[index]] = float(_swarm_us[index]) / 1000.0
	var frame_parts: PackedInt64Array = simulation_node.get_agent_frame_parts()
	for index in FRAME_PARTS.size():
		details[FRAME_PARTS[index]] = float(frame_parts[index]) / 1000.0 if index < 2 else frame_parts[index]
	# The installed frame's car parts, from Rust on the job's thread: smoothing, the off-lane
	# ground solve, recording (ms), then counts (`RENDER-28`).
	var car_parts: PackedInt64Array = simulation_node.get_car_frame_parts()
	for index in CAR_PARTS.size():
		details[CAR_PARTS[index]] = float(car_parts[index]) / 1000.0 if index < 3 else car_parts[index]
	var walkers := 0
	var cars := 0
	for mmi in walker_mmis.values():
		walkers += mmi.multimesh.instance_count
	for mmi in car_mmis.values():
		cars += mmi.multimesh.instance_count
	details["walkers"] = walkers
	details["cars"] = cars
	# From the latest snapshot: agents inside the camera's rectangle that their draw distance
	# cut, and its agent instances' time (ms, on the simulation thread).
	var snapshot_parts: PackedInt64Array = simulation_node.get_agent_snapshot_parts()
	details["walkers_cut"] = snapshot_parts[0]
	details["cars_cut"] = snapshot_parts[1]
	details["snapshot_agents_ms"] = float(snapshot_parts[2]) / 1000.0
	details["walker_buckets_uploaded"] = _walker_buckets_uploaded
	PerfDebug.record("agents", elapsed_ms, details)

func _update_view() -> void:
	var camera := get_viewport().get_camera_3d()
	if camera:
		simulation_node.update_agent_view(camera)

func update_swarm(delta: float = 0.0):
	var start_us := Time.get_ticks_usec()
	var parked_renderer: Node = get_node_or_null("../ParkedCars")
	var full: bool = parked_renderer.needs_full_publication() if parked_renderer != null else false
	# The frame a background job prepared from one snapshot: walkers, smoothed and grounded cars,
	# and that snapshot's parked chunks, so a car crossing a bay's edge is drawn exactly once.
	var frame: Dictionary = simulation_node.advance_agent_frame(delta, full)
	if parked_renderer != null:
		parked_renderer.follow_vehicle_catalogue(int(frame.get("vehicle_revision", 0)))
	if parked_renderer != null and frame.has("parked"):
		parked_renderer.apply_publication(frame["parked"])
	var frame_us := Time.get_ticks_usec()
	_follow_vehicle_catalogue(int(frame.get("vehicle_revision", 0)))
	walker_buffers = frame["walkers"]
	# A bucket absent from the buffers left the view: it draws none.
	var changed_walkers: PackedInt64Array = frame["changed_walkers"]
	_walker_buckets_uploaded = changed_walkers.size()
	for p_type in changed_walkers:
		_upload(walker_mmis.get(p_type), walker_buffers.get(p_type, PackedFloat32Array()), WALKER_STRIDE)
	var walkers_us := Time.get_ticks_usec()
	car_buffers = frame["cars"]
	for type_key in frame["changed_cars"]:
		var buffer: PackedFloat32Array = car_buffers.get(type_key, PackedFloat32Array())
		if not buffer.is_empty() and not car_mmis.has(type_key):
			_add_car_bucket(type_key)
		_upload(car_mmis.get(type_key), buffer, CAR_STRIDE)
	var assigned_us := Time.get_ticks_usec()
	_headlights.update(simulation_node, car_buffers)
	var lit_us := Time.get_ticks_usec()
	_swarm_us[0] = frame_us - start_us
	_swarm_us[1] = walkers_us - frame_us
	_swarm_us[2] = assigned_us - walkers_us
	_swarm_us[3] = lit_us - assigned_us

	if show_paths:
		var data = simulation_node.get_agent_paths_debug()
		debug_mesh.clear_surfaces()
		var points = data.get("points", PackedVector3Array())
		var colors = data.get("colors", PackedColorArray())
		var label_positions = data.get("label_positions", PackedVector3Array())
		var labels = data.get("labels", PackedStringArray())
		
		if points.size() > 0:
			debug_mesh.surface_begin(Mesh.PRIMITIVE_LINES)
			for i in range(points.size()):
				if i < colors.size():
					debug_mesh.surface_set_color(colors[i])
				debug_mesh.surface_add_vertex(points[i])
			debug_mesh.surface_end()
		_sync_debug_labels(label_positions, labels)
		_debug_overlay_visible = true
	else:
		if _debug_overlay_visible:
			clear_debug_overlay()

# Writes `buffer` (`stride` floats an instance) into a bucket node's MultiMesh.
func _upload(mmi: MultiMeshInstance3D, buffer: PackedFloat32Array, stride: int) -> void:
	if mmi == null:
		return
	var count := int(buffer.size() / stride)
	if count != mmi.multimesh.instance_count:
		mmi.multimesh.instance_count = count
	if count > 0:
		mmi.multimesh.buffer = buffer

# Hands every walker bucket's mesh, material and margin to the StreamNode's shadow-only twins,
# which cast the agents within reach of the sun's shadow range; the bucket nodes then cast nothing
# (`RENDER-15`). Car buckets register as they are made. Without a StreamNode (tests) they cast as
# before.
func _attach_shadow_casters() -> void:
	var stream := _caster_stream()
	if stream == null:
		return
	stream.attach_agent_casters(simulation_node, get_node("../DirectionalLight3D"), get_world_3d().scenario)
	_casters_attached = true
	for key in walker_mmis:
		_register_caster(0, key, walker_mmis[key])

func _caster_stream() -> Node:
	var stream := get_node_or_null("../StreamNode")
	var sun := get_node_or_null("../DirectionalLight3D") as DirectionalLight3D
	if stream == null or sun == null or not stream.has_method("attach_agent_casters"):
		return null
	return stream

func _register_caster(kind: int, key: int, mmi: MultiMeshInstance3D) -> void:
	if not _casters_attached:
		return
	var material: Material = mmi.material_override
	if _caster_stream().register_agent_caster(kind, key, mmi.multimesh.mesh.get_rid(),
			material.get_rid() if material != null else RID(), mmi.extra_cull_margin):
		mmi.cast_shadow = GeometryInstance3D.SHADOW_CASTING_SETTING_OFF

# Remakes the car nodes when the frame's buckets are keyed by another catalogue revision than the
# meshes (packs loaded, `RENDER-65`). One integer compare a frame otherwise.
func _follow_vehicle_catalogue(wanted: int) -> void:
	if not _vehicles.refresh(simulation_node, wanted):
		return
	for mmi: MultiMeshInstance3D in car_mmis.values():
		mmi.queue_free()
	car_mmis.clear()
	_headlights.forget_models()
	if _casters_attached:
		_caster_stream().unregister_agent_casters(1)

# A car bucket's node: its look's tier mesh, rows with custom data (`RENDER-65`).
func _add_car_bucket(key: int) -> void:
	var mesh := _vehicles.mesh_for_bucket(key)
	var mmi := MultiMeshInstance3D.new()
	var mm := MultiMesh.new()
	mm.transform_format = MultiMesh.TRANSFORM_3D
	# No instance colours: the body shader reads the mesh's ids from CUSTOM0 and the car's data
	# from INSTANCE_CUSTOM (`RENDER-70`).
	mm.use_colors = false
	mm.use_custom_data = true
	mm.instance_count = 0
	mm.mesh = mesh
	mmi.multimesh = mm
	SceneLightingConfig.apply_shadow_policy(mmi, SceneLightingConfig.SHADOW_DYNAMIC_CASTER, "cars")
	add_child(mmi)
	car_mmis[key] = mmi
	_headlights.register_model(key, mesh)
	_register_caster(1, key, mmi)

func clear_debug_overlay() -> void:
	if debug_mesh:
		debug_mesh.clear_surfaces()
	_hide_debug_labels()
	_debug_overlay_visible = false

func _sync_debug_labels(label_positions: PackedVector3Array, labels: PackedStringArray) -> void:
	var count := mini(mini(label_positions.size(), labels.size()), DEBUG_LABEL_LIMIT)
	while debug_labels.size() < count:
		var label := Label3D.new()
		label.name = "TrafficDebugLabel%d" % debug_labels.size()
		label.font_size = 64
		label.pixel_size = 0.015
		label.no_depth_test = true
		label.billboard = BaseMaterial3D.BILLBOARD_ENABLED
		label.modulate = Color(1.0, 1.0, 1.0, 0.9)
		add_child(label)
		debug_labels.append(label)

	for i in range(debug_labels.size()):
		var label: Label3D = debug_labels[i]
		if i < count:
			label.text = labels[i]
			label.global_position = label_positions[i]
			label.visible = true
		else:
			label.visible = false

func _hide_debug_labels() -> void:
	for label in debug_labels:
		label.visible = false

func _on_visual_mode_changed(mode: String) -> void:
	if mode != "pedestrian_vat":
		return
	var debug_mode := MetrumDebug.visual_mode_index("pedestrian_vat")
	for mmi in walker_mmis.values():
		(mmi.material_override as ShaderMaterial).set_shader_parameter("debug_mode", debug_mode)

func _load_source_texture(path: String) -> Texture2D:
	if texture_cache.has(path):
		return texture_cache[path]

	var tex: Texture2D = null
	var image := Image.load_from_file(ProjectSettings.globalize_path(path))
	if image:
		tex = ImageTexture.create_from_image(image)
	else:
		push_error("Could not load texture source: " + path)

	texture_cache[path] = tex
	return tex

func _extract_first_mesh(root: Node) -> Mesh:
	if root is MeshInstance3D and root.mesh:
		return root.mesh
	for child in root.get_children():
		var m = _extract_first_mesh(child)
		if m:
			return m
	return null
