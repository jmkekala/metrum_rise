## Agent renderer — streams agent positions from Rust into a MultiMeshInstance3D each frame.
##
## Rust methods called: get_agent_transforms(), get_agent_paths_debug(),
##   get_agent_cull_far_m(), get_agent_cull_padding_m(), set_camera_aabb()
## Agent transforms arrive as a flat PackedFloat32Array of 12 floats per agent:
##   [basis.x(3), basis.y(3), basis.z(3), origin(3)] — matches Godot's Transform3D memory layout.
## Path debug lines (toggled with P key) arrive as a PackedVector3Array of point pairs.
extends Node3D

@onready var simulation_node = $"../SimulationNode"

# Key: pedestrian_type (int), Value: MultiMeshInstance3D
var walker_mmis: Dictionary = {}
# Key: vehicle_type (int), Value: MultiMeshInstance3D
var car_mmis: Dictionary = {}

var debug_mesh_instance: MeshInstance3D
var debug_mesh: ImmediateMesh
var show_paths = false

func _ready():
	# --- Walker MultiMeshes — VAT (Vertex Animation Texture) pipeline ---
	# Assets baked from .blend source by tools/bake_vat_blend.py.
	# The GLTF rest mesh has clean Y-up orientation (no FBX rotation offset).
	var vat_base = "res://assets/models/characters/civilians/VAT/"
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
	var person_skins = {
		0: "res://assets/models/characters/civilians/Skins/casualMaleA.png",
		1: "res://assets/models/characters/civilians/Skins/casualMaleB.png",
		2: "res://assets/models/characters/civilians/Skins/casualFemaleA.png",
		3: "res://assets/models/characters/civilians/Skins/casualFemaleB.png",
	}

	var walk_shader = preload("res://scripts/shaders/pedestrian_walk.gdshader")

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

		var mat := ShaderMaterial.new()
		mat.shader = walk_shader
		mat.set_shader_parameter("albedo_texture", load(person_skins[p_type]))
		mat.set_shader_parameter("vat_texture",    vat_tex)
		mat.set_shader_parameter("num_frames",     31.0)
		mmi.material_override = mat

		add_child(mmi)
		walker_mmis[p_type] = mmi


	# --- Car MultiMeshes (Civilians) ---
	var car_models = {
		0: "res://assets/models/vehicles/civilian/sedan.glb",
		1: "res://assets/models/vehicles/civilian/sedan-sports.glb",
		2: "res://assets/models/vehicles/civilian/suv.glb",
		3: "res://assets/models/vehicles/civilian/suv-luxury.glb"
	}

	# We'll create 5 color variations for each model by shifting UVs
	# Each variation will have its own MMI node
	# Keys in car_mmis will be: (vehicle_type * 10) + color_variant
	var color_offsets = [0.0, 0.1, 0.2, 0.3, 0.4] # Horizontal UV shifts

	for v_type in car_models:
		var model_path = car_models[v_type]
		var gltf_doc := GLTFDocument.new()
		var gltf_state := GLTFState.new()
		var err := gltf_doc.append_from_file(model_path, gltf_state)
		
		if err == OK:
			var node := gltf_doc.generate_scene(gltf_state)
			if node:
				for variant_id in range(color_offsets.size()):
					var uv_shift = color_offsets[variant_id]
					var mesh = _extract_mesh(node, uv_shift, 0.0, Vector3(0, PI, 0))
					if not mesh: continue
					
					var mmi = MultiMeshInstance3D.new()
					var mm = MultiMesh.new()
					mm.transform_format = MultiMesh.TRANSFORM_3D
					mm.use_colors = false
					mm.use_custom_data = false
					mm.instance_count = 0
					mm.mesh = mesh
					mmi.multimesh = mm
					add_child(mmi)
					
					var key = (v_type * 10) + variant_id
					car_mmis[key] = mmi
					
				node.free()
		else:
			push_error("Failed to load car model: " + model_path)

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
	add_child(debug_mesh_instance)


func _process(delta):
	_update_camera_aabb()
	# Refresh visual swarms very often for smooth movement
	if Engine.get_frames_drawn() % 5 == 0:
		update_swarm()

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
	simulation_node.set_camera_aabb(x_min - pad, x_max + pad, z_min - pad, z_max + pad)

func update_swarm():
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

	# Cars (Now grouped by vehicle type and color variant)
	var car_data = simulation_node.get_car_transforms()
	
	# Clear types that are no longer present in the simulation (optional, but clean)
	for type_key in car_mmis:
		var mmi = car_mmis[type_key]
		var buffer = car_data.get(type_key, PackedFloat32Array())
		var count = buffer.size() / 12
		
		if count != mmi.multimesh.instance_count:
			mmi.multimesh.instance_count = count
		if count > 0:
			mmi.multimesh.buffer = buffer

	if show_paths:
		var data = simulation_node.get_agent_paths_debug()
		debug_mesh.clear_surfaces()
		var points = data.get("points", PackedVector3Array())
		var colors = data.get("colors", PackedColorArray())
		
		if points.size() > 0:
			debug_mesh.surface_begin(Mesh.PRIMITIVE_LINES)
			for i in range(points.size()):
				if i < colors.size():
					debug_mesh.surface_set_color(colors[i])
				debug_mesh.surface_add_vertex(points[i])
			debug_mesh.surface_end()

# Builds a car-shaped ArrayMesh from two boxes (body + cabin). No mesh files required.
# Car faces along local -Z (Godot forward). Origin at bottom-centre of body.
# Body: 1.8 m wide × 0.55 m tall × 4.2 m long
# Cabin: 1.8 m wide × 1.2 m tall × 2.2 m long, full-height to seal the body opening at z=±1.1
func _build_car_mesh() -> ArrayMesh:
	# Use plain Array (reference type) so helpers can append to the caller's arrays.
	# Converted to PackedVector3Array only when passing to add_surface_from_arrays.
	var verts: Array = []
	var norms: Array = []

	_add_box(verts, norms, Vector3(-0.9, 0.0, -2.1), Vector3(0.9, 0.55, 2.1))
	# Cabin starts at y=0 so its front/back faces seal the gap at z=±1.1 all the way to ground,
	# preventing the hollow body interior from being visible through the junction edges.
	_add_box(verts, norms, Vector3(-0.9, 0.0, -1.1), Vector3(0.9, 1.2, 1.1))

	var arrays := []
	arrays.resize(Mesh.ARRAY_MAX)
	arrays[Mesh.ARRAY_VERTEX] = PackedVector3Array(verts)
	arrays[Mesh.ARRAY_NORMAL] = PackedVector3Array(norms)

	var mesh := ArrayMesh.new()
	mesh.add_surface_from_arrays(Mesh.PRIMITIVE_TRIANGLES, arrays)

	var mat := StandardMaterial3D.new()
	mat.albedo_color = Color(0.22, 0.42, 0.78)  # body colour; will vary per model once item 47 ships
	mat.roughness = 0.8
	mat.metallic = 0.0   # metallic without an HDR environment looks glassy/see-through
	mesh.surface_set_material(0, mat)

	return mesh

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
func _add_box(verts: Array, norms: Array, mn: Vector3, mx: Vector3) -> void:
	var x0 := mn.x; var y0 := mn.y; var z0 := mn.z
	var x1 := mx.x; var y1 := mx.y; var z1 := mx.z
	_add_quad(verts, norms, Vector3(x1,y0,z1), Vector3(x1,y0,z0), Vector3(x1,y1,z0), Vector3(x1,y1,z1), Vector3.RIGHT)
	_add_quad(verts, norms, Vector3(x0,y0,z0), Vector3(x0,y0,z1), Vector3(x0,y1,z1), Vector3(x0,y1,z0), Vector3.LEFT)
	_add_quad(verts, norms, Vector3(x0,y1,z1), Vector3(x1,y1,z1), Vector3(x1,y1,z0), Vector3(x0,y1,z0), Vector3.UP)
	_add_quad(verts, norms, Vector3(x0,y0,z0), Vector3(x1,y0,z0), Vector3(x1,y0,z1), Vector3(x0,y0,z1), Vector3.DOWN)
	_add_quad(verts, norms, Vector3(x0,y0,z1), Vector3(x1,y0,z1), Vector3(x1,y1,z1), Vector3(x0,y1,z1), Vector3.BACK)
	_add_quad(verts, norms, Vector3(x1,y0,z0), Vector3(x0,y0,z0), Vector3(x0,y1,z0), Vector3(x1,y1,z0), Vector3.FORWARD)

func _add_quad(verts: Array, norms: Array,
               a: Vector3, b: Vector3, c: Vector3, d: Vector3, n: Vector3) -> void:
	verts.append(a); verts.append(b); verts.append(c)
	verts.append(a); verts.append(c); verts.append(d)
	for _i in 6:
		norms.append(n)
