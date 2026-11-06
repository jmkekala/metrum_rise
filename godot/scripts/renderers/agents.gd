# SPDX-License-Identifier: GPL-2.0-only

## Agent renderer — streams agent positions from Rust into a MultiMeshInstance3D each frame.
##
## Rust methods called: get_agent_transforms(), get_car_render_data(), smooth_car_render_data(),
##   set_vehicle_ground_support(),
##   get_agent_paths_debug(), get_agent_cull_far_m(), get_agent_cull_padding_m(), set_camera_aabb()
## Agent transforms arrive as a flat PackedFloat32Array of 12 floats per agent:
##   [xx, yx, zx, ox, xy, yy, zy, oy, xz, yz, zz, oz] — MultiMesh row-major 3×4 layout.
## Path debug lines (toggled with P key) arrive as a PackedVector3Array of point pairs.
extends Node3D

const SceneLightingConfig := preload("res://scripts/core/scene_lighting.gd")
const PerfDebug := preload("res://scripts/core/perf_debug.gd")
const WindowMaterials := preload("res://scripts/renderers/window_materials.gd")
const VehicleLamps := preload("res://scripts/renderers/vehicle_lamps.gd")
const VehicleHeadlights := preload("res://scripts/renderers/vehicle_headlights.gd")

@onready var simulation_node = $"../SimulationNode"

# Key: pedestrian_type (int), Value: MultiMeshInstance3D
var walker_mmis: Dictionary = {}
# Key: vehicle_type (int), Value: MultiMeshInstance3D
var car_mmis: Dictionary = {}
var texture_cache: Dictionary = {}
const WALKER_VAT_FRAMES := 30.0
# Quaternius walk clips are baked in the opposite temporal direction from our distance phase.
const WALKER_VAT_REVERSE_PHASE := true
const CAR_TRANSFORM_STRIDE := 12
const DEBUG_LABEL_LIMIT := 96
var _vehicles := preload("res://scripts/renderers/vehicle_resources.gd").new()
var _headlights := VehicleHeadlights.new()

var debug_mesh_instance: MeshInstance3D
var debug_mesh: ImmediateMesh
var debug_labels: Array = []
var show_paths = false
var _debug_overlay_visible := false

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
	var person_tints = {
		0: Vector3(1.0, 1.0, 1.0),
		1: Vector3(0.86, 0.92, 1.0),
		2: Vector3(1.0, 1.0, 1.0),
		3: Vector3(1.0, 0.9, 0.92),
	}

	var walk_shader = preload("res://scripts/shaders/pedestrian_walk.gdshader")
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
		mm.use_custom_data  = true   # INSTANCE_CUSTOM.x = walk_phase [0..1]
		mm.instance_count   = 0
		mm.mesh             = mesh
		mmi.multimesh       = mm
		SceneLightingConfig.apply_shadow_policy(
			mmi,
			SceneLightingConfig.SHADOW_DYNAMIC_CASTER,
			"walkers"
		)

		var mat := ShaderMaterial.new()
		mat.shader = walk_shader
		mat.set_shader_parameter("vat_texture",    vat_tex)
		mat.set_shader_parameter("albedo_texture", _load_source_texture(person_palettes[p_type]))
		mat.set_shader_parameter("num_frames",     WALKER_VAT_FRAMES)
		mat.set_shader_parameter("albedo_tint",    person_tints[p_type])
		mat.set_shader_parameter("debug_mode",     pedestrian_vat_debug_mode)
		mat.set_shader_parameter("reverse_phase",  WALKER_VAT_REVERSE_PHASE)
		mmi.material_override = mat

		add_child(mmi)
		walker_mmis[p_type] = mmi


	# --- Car MultiMeshes (Civilians) ---
	add_child(_headlights)
	var car_models = {
		0: "res://assets/models/vehicles/civilian/sedan.glb",
		1: "res://assets/models/vehicles/civilian/sedan-sports.glb",
		2: "res://assets/models/vehicles/civilian/suv.glb",
		3: "res://assets/models/vehicles/civilian/suv-luxury.glb",
		4: "res://assets/models/vehicles/freight/delivery.glb"
	}

	# Original atlas palette variants; bucket keys stay (vehicle_type * 10) + variant.
	var color_offsets = [0.0, 0.1, 0.2, 0.3, 0.4]
	var car_colors = [
		Color(0.22, 0.42, 0.78),
		Color(0.72, 0.16, 0.14),
		Color(0.17, 0.55, 0.38),
		Color(0.88, 0.78, 0.30),
		Color(0.82, 0.84, 0.86),
	]
	for v_type in car_models:
		var offsets = [0.0, 0.0, 0.0, 0.0, 0.0] if v_type == 4 else color_offsets
		var loaded_model = _add_car_model_variants(v_type, car_models[v_type], offsets)
		if not loaded_model:
			_add_procedural_car_variants(v_type, car_colors)

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
		_update_camera_aabb()
		# Upload every rendered frame so cars do not visually quantize to 12 Hz at fast speeds.
		update_swarm(delta)
		return
	var frame_start_us := Time.get_ticks_usec()
	var aabb_start_us := frame_start_us
	_update_camera_aabb()
	var aabb_elapsed_ms := float(Time.get_ticks_usec() - aabb_start_us) / 1000.0
	# Upload every rendered frame so cars do not visually quantize to 12 Hz at fast speeds.
	var swarm_start_us := Time.get_ticks_usec()
	update_swarm(delta)
	var swarm_elapsed_ms := float(Time.get_ticks_usec() - swarm_start_us) / 1000.0
	PerfDebug.record(
		"agents",
		float(Time.get_ticks_usec() - frame_start_us) / 1000.0,
		{
			"camera_aabb": aabb_elapsed_ms,
			"swarm": swarm_elapsed_ms,
		}
	)

func _update_camera_aabb():
	var camera = get_viewport().get_camera_3d()
	if not camera:
		return
	# For each viewport corner ray, find how far along the ray to sample.
	# Prefer the ground-plane (y=0) intersection — this gives a tight AABB when
	# the camera is steep. Fall back to AGENT_CULL_FAR_M when the ray is
	# near-horizontal (dir.y small), which was the original bug: ground intersection
	# produced a point at astronomical distance or didn't exist at all.
	var cull_far = SimulationNode.get_agent_cull_far_m()
	var vp_size = get_viewport().get_visible_rect().size
	var screen_corners = [
		Vector2(0, 0),
		Vector2(vp_size.x, 0),
		Vector2(vp_size.x, vp_size.y),
		Vector2(0, vp_size.y),
	]
	var x_min = INF; var x_max = -INF
	var z_min = INF; var z_max = -INF
	for c in screen_corners:
		var origin = camera.project_ray_origin(c)
		var dir = camera.project_ray_normal(c)
		# Compute the distance to y=0; use it if it's valid and closer than cull_far.
		var dist: float
		if dir.y < -1e-3:
			dist = min(-origin.y / dir.y, cull_far)
		else:
			dist = cull_far
		var pt = origin + dir * dist
		x_min = min(x_min, pt.x); x_max = max(x_max, pt.x)
		z_min = min(z_min, pt.z); z_max = max(z_max, pt.z)
	var pad = SimulationNode.get_agent_cull_padding_m()
	simulation_node.set_camera_aabb(
		x_min - pad, x_max + pad, z_min - pad, z_max + pad, camera.global_position
	)

func update_swarm(delta: float = 0.0):
	# Walkers (Grouped by variant)
	var walker_data = simulation_node.get_agent_transforms()
	for p_type in walker_mmis:
		var mmi = walker_mmis[p_type]
		var buffer = walker_data.get(p_type, PackedFloat32Array())
		var count = buffer.size() / 16 # Transform (12) + Custom (4)
		
		# MMIs are initialized on demand; skip if no agents of this type.
		if count != mmi.multimesh.instance_count:
			mmi.multimesh.instance_count = count
		if count > 0:
			mmi.multimesh.buffer = buffer

	# Cars (grouped by vehicle type and color variant). Rust moves each drawn pose toward its
	# snapshot pose and grounds off-lane cars, so they do not step at the simulation tick rate.
	var parked_renderer: Node = get_node_or_null("../ParkedCars")
	var full: bool = parked_renderer.needs_full_publication() if parked_renderer != null else false
	var vehicle_frame: Dictionary = simulation_node.get_vehicle_frame(full)
	if vehicle_frame.get("busy", true):
		return
	if parked_renderer != null:
		parked_renderer.apply_publication(vehicle_frame["parked"])
	var car_buffers: Dictionary = simulation_node.smooth_car_render_data(
		vehicle_frame["cars"], delta
	)
	for type_key in car_mmis:
		var mmi = car_mmis[type_key]
		var buffer: PackedFloat32Array = car_buffers.get(type_key, PackedFloat32Array())
		var count := int(buffer.size() / CAR_TRANSFORM_STRIDE)
		if count != mmi.multimesh.instance_count:
			mmi.multimesh.instance_count = count
		if count > 0:
			mmi.multimesh.buffer = buffer
	_headlights.update(simulation_node, car_buffers)

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

func _add_car_model_variants(v_type: int, model_path: String, color_offsets: Array) -> bool:
	var model := _load_vehicle_glb_source(model_path)
	if model.is_empty():
		push_warning("Using procedural fallback for car model: " + model_path)
		return false

	var added_count := 0
	for variant_id in range(color_offsets.size()):
		var uv_shift = color_offsets[variant_id]
		var mesh = _build_vehicle_mesh_from_glb(model, model_path, uv_shift, 0.0, Vector3(0, PI, 0))
		if not mesh:
			continue
		if not _add_car_multimesh(v_type, variant_id, mesh):
			return false
		added_count += 1

	return added_count > 0

func _load_vehicle_glb_source(model_path: String) -> Dictionary:
	return _vehicles._load_vehicle_glb_source(model_path)

func _build_vehicle_mesh_from_glb(model: Dictionary, model_path: String, uv_shift: float, target_height: float, base_rotation: Vector3) -> ArrayMesh:
	return _vehicles._build_vehicle_mesh_from_glb(model, model_path, uv_shift, target_height, base_rotation)

func _add_procedural_car_variants(v_type: int, colors: Array) -> void:
	for variant_id in range(colors.size()):
		_add_car_multimesh(v_type, variant_id, _build_car_mesh(colors[variant_id]))

func _add_car_multimesh(v_type: int, variant_id: int, mesh: Mesh) -> bool:
	if variant_id == 0:
		var unique_contacts: Dictionary = {}
		# The body alone: lamp glows and the headlight volume are not ground contact or car extent.
		var body_aabb: AABB = mesh.get_meta(VehicleLamps.BODY_AABB_META, mesh.get_aabb())
		var body_surfaces: int = mesh.get_meta(VehicleLamps.BODY_SURFACES_META, mesh.get_surface_count())
		for surface in range(body_surfaces):
			var vertices: PackedVector3Array = mesh.surface_get_arrays(surface)[Mesh.ARRAY_VERTEX]
			for vertex in vertices:
				# Mesh loading has already moved its lowest point to y=0. This is a geometric
				# tolerance for coincident bottom vertices, not a per-frame terrain probe.
				if absf(vertex.y) <= 0.0001:
					unique_contacts[vertex] = true
		if not simulation_node.set_vehicle_ground_support(v_type, body_aabb, PackedVector3Array(unique_contacts.keys())):
			push_warning("Invalid vehicle ground support geometry for type %d; using fallback" % v_type)
			return false
	var mmi = MultiMeshInstance3D.new()
	var mm = MultiMesh.new()
	mm.transform_format = MultiMesh.TRANSFORM_3D
	mm.use_colors = false
	mm.use_custom_data = false
	mm.instance_count = 0
	mm.mesh = mesh
	mmi.multimesh = mm
	SceneLightingConfig.apply_shadow_policy(
		mmi,
		SceneLightingConfig.SHADOW_DYNAMIC_CASTER,
		"cars"
	)
	add_child(mmi)

	var key = (v_type * 10) + variant_id
	car_mmis[key] = mmi
	_headlights.register_model(key, mesh)
	return true

# Builds a car-shaped ArrayMesh from two boxes (body + cabin). No mesh files required.
# Car faces along local -Z (Godot forward). Origin at bottom-centre of body.
# Body: 1.8 m wide × 0.55 m tall × 4.2 m long
# Cabin: 1.8 m wide × 1.2 m tall × 2.2 m long, full-height to seal the body opening at z=±1.1
func _build_car_mesh(body_color: Color) -> ArrayMesh:
	return _vehicles._build_car_mesh(body_color)

# Extracts and merges all surfaces from a Node hierarchy into an ArrayMesh.
# target_height: if > 0, scales the model to this height in meters.
# base_rotation: additional rotation (Euler) to apply before normalization.
func _extract_mesh(root_node: Node, uv_shift: float = 0.0, target_height: float = 0.0, base_rotation: Vector3 = Vector3.ZERO) -> Mesh:
	var mesh_instances = []
	var base_tf = Transform3D().rotated(Vector3.RIGHT, base_rotation.x)
	base_tf = base_tf.rotated(Vector3.UP, base_rotation.y)
	base_tf = base_tf.rotated(Vector3.FORWARD, base_rotation.z)
	
	_find_mesh_instances(root_node, base_tf, mesh_instances)
	
	if mesh_instances.size() == 0:
		return null
		
	# 1. Calculate the bounding box of the entire merged model to find height and bottom.
	var aabb = AABB()
	var first = true
	for item in mesh_instances:
		var mi = item.node
		var tf = item.transform
		var mi_aabb = tf * mi.mesh.get_aabb()
		if first:
			aabb = mi_aabb
			first = false
		else:
			aabb = aabb.merge(mi_aabb)
			
	# 2. Define our "normalization" transform: 
	var normalization = Transform3D()
	
	# Scaling
	if target_height > 0.0 and aabb.size.y > 0.01:
		var s = target_height / aabb.size.y
		normalization = normalization.scaled(Vector3(s, s, s))
		aabb.position *= s
		aabb.size *= s

	# Shift so bottom is at Y=0
	normalization.origin.y = -aabb.position.y
	
	var final_mesh := ArrayMesh.new()
	for item in mesh_instances:
		var mi = item.node
		var tf = normalization * item.transform
		
		for i in range(mi.mesh.get_surface_count()):
			var st = SurfaceTool.new()
			st.begin(Mesh.PRIMITIVE_TRIANGLES)
			st.append_from(mi.mesh, i, tf)
			
			# Apply UV shift for randomization
			if uv_shift != 0.0:
				var attr = st.commit_to_arrays()
				var uvs = attr[Mesh.ARRAY_TEX_UV]
				for j in range(uvs.size()):
					uvs[j] = Vector2(fmod(uvs[j].x + uv_shift, 1.0), uvs[j].y)
				attr[Mesh.ARRAY_TEX_UV] = uvs
				
				# We cannot easily re-inject into SurfaceTool without manual vertex loop,
				# so we'll just add the array directly to ArrayMesh.
				var mat = mi.get_active_material(i)
				final_mesh.add_surface_from_arrays(Mesh.PRIMITIVE_TRIANGLES, attr)
				if mat:
					final_mesh.surface_set_material(final_mesh.get_surface_count() - 1, mat)
			else:
				var mat = mi.get_active_material(i)
				if mat:
					st.set_material(mat)
				st.commit(final_mesh)
			
	return final_mesh

# Extracts the first ArrayMesh from a GLTF scene tree as-is (no coordinate transform).
# Used for VAT rest-pose meshes which are already in Godot Y-up space.
func _extract_first_mesh(root: Node) -> Mesh:
	if root is MeshInstance3D and root.mesh:
		return root.mesh
	for child in root.get_children():
		var m = _extract_first_mesh(child)
		if m:
			return m
	return null

func _find_mesh_instances(node: Node, parent_transform: Transform3D, out_list: Array) -> void:
	var current_transform = parent_transform
	if node is Node3D:
		current_transform = parent_transform * node.transform
		
	if node is MeshInstance3D and node.mesh:
		out_list.append({"node": node, "transform": current_transform})

	for child in node.get_children():
		_find_mesh_instances(child, current_transform, out_list)

# Adds all 6 faces of an AABB to verts/norms with correct CCW winding.
# Arrays must be plain Array (reference type), not PackedVector3Array.
