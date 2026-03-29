## Building renderer — maintains one MultiMeshInstance3D per (zone, variant) pair.
##
## Rust methods called: get_building_transforms(zone_id: int, variant: int)
## Building transforms arrive as a flat PackedFloat32Array of 12 floats per building.
## Zone IDs: 1=Residential, 2=Commercial, 3=Industrial, 4=Mixed.
extends Node3D

@onready var simulation_node = $"../SimulationNode"

const NUM_VARIANTS = 64
const ZONE_NAMES = {
	1: "residential",
	2: "commercial",
	3: "industrial",
	4: "mixed"
}

# multimeshes[zone_id][variant] = MultiMeshInstance3D
var multimeshes = {}
# model_files[zone_id] = [filename1, filename2, ...]
var model_files = {}

func _ready():
	# Load metadata to inform Rust about footprint sizes
	var meta_data = {}
	var meta_path = "res://assets/models/buildings/model_metadata.json"
	if FileAccess.file_exists(meta_path):
		var file = FileAccess.open(meta_path, FileAccess.READ)
		var json = JSON.new()
		json.parse(file.get_as_text())
		meta_data = json.get_data()
	
	for zone_id in ZONE_NAMES.keys():
		multimeshes[zone_id] = {}
		
		# Discover all models in this zone's directory
		var zone_name = ZONE_NAMES[zone_id]
		var model_dir = "res://assets/models/buildings/" + zone_name + "/"
		model_files[zone_id] = []
		
		var dir = DirAccess.open(model_dir)
		if dir:
			dir.list_dir_begin()
			var fn = dir.get_next()
			while fn != "":
				if fn.ends_with(".glb"):
					model_files[zone_id].append(fn)
				fn = dir.get_next()
			model_files[zone_id].sort()
		
		# Register metadata with Rust for EVERY variant
		for variant in range(NUM_VARIANTS):
			var mesh = null
			if model_files[zone_id].size() > 0:
				var fn = model_files[zone_id][variant % model_files[zone_id].size()]
				if meta_data.has(zone_name) and meta_data[zone_name].has(fn):
					var m = meta_data[zone_name][fn]
					# Register with Rust: width, height, depth
					simulation_node.register_building_metadata(zone_id, variant, m.size_x, m.size_y, m.size_z)
				
				mesh = load_building_mesh(zone_id, variant)
			
			setup_building_variant(zone_id, variant, mesh)

func setup_building_variant(zone_id: int, variant: int, mesh: Mesh):
	var mmi = MultiMeshInstance3D.new()
	var mm = MultiMesh.new()
	mm.transform_format = MultiMesh.TRANSFORM_3D
	mm.instance_count = 0
	
	if not mesh:
		# Fallback to procedural box if model missing
		mesh = create_fallback_mesh(zone_id)
	
	mm.mesh = mesh
	mmi.multimesh = mm
	mmi.gi_mode = GeometryInstance3D.GI_MODE_DYNAMIC
	
	add_child(mmi)
	multimeshes[zone_id][variant] = mmi

func load_building_mesh(zone_id: int, variant: int) -> Mesh:
	if not model_files.has(zone_id) or model_files[zone_id].size() == 0:
		return null
		
	var zone_name = ZONE_NAMES[zone_id]
	var file_name = model_files[zone_id][variant % model_files[zone_id].size()]
	var path = "res://assets/models/buildings/%s/%s" % [zone_name, file_name]
	
	var scene = load(path)
	if not scene:
		return null
		
	var instance = scene.instantiate()
	var mesh_node = find_mesh_recursive(instance)
	var mesh = null
	if mesh_node and mesh_node is MeshInstance3D:
		mesh = mesh_node.mesh
	
	instance.queue_free()
	return mesh

func find_mesh_recursive(node: Node) -> MeshInstance3D:
	if node is MeshInstance3D:
		return node
	for child in node.get_children():
		var found = find_mesh_recursive(child)
		if found:
			return found
	return null

func create_fallback_mesh(zone_id: int) -> ArrayMesh:
	# Simplified procedural box for missing models
	var wall_c = Color(0.9, 0.85, 0.7)
	var roof_c = Color(0.2, 0.4, 0.2)
	
	match zone_id:
		2: # Commercial
			wall_c = Color(0.8, 0.8, 0.9); roof_c = Color(0.1, 0.2, 0.5)
		3: # Industrial
			wall_c = Color(0.5, 0.5, 0.5); roof_c = Color(0.4, 0.1, 0.1)
		4: # Mixed
			wall_c = Color(0.8, 0.7, 0.6); roof_c = Color(0.3, 0.2, 0.1)
			
	var st = SurfaceTool.new()
	st.begin(Mesh.PRIMITIVE_TRIANGLES)
	
	var w = 0.45; var d = 0.45; var h = 4.0
	var offset = 0.0 # Pivot at base for 3D models
	
	var add_face = func(pts: Array, normal: Vector3, color: Color):
		st.set_color(color)
		st.set_normal(normal)
		st.add_vertex(pts[0]); st.add_vertex(pts[1]); st.add_vertex(pts[2])
		st.add_vertex(pts[0]); st.add_vertex(pts[2]); st.add_vertex(pts[3])
	
	var bfl = Vector3(-w, 0, -d); var bfr = Vector3(w, 0, -d)
	var bbr = Vector3(w, 0, d);  var bbl = Vector3(-w, 0, d)
	var tfl = Vector3(-w, h, -d); var tfr = Vector3(w, h, -d)
	var tbr = Vector3(w, h, d);  var tbl = Vector3(-w, h, d)
	
	add_face.call([bfl, bfr, tfr, tfl], Vector3(0, 0, -1), wall_c)
	add_face.call([bfr, bbr, tbr, tfr], Vector3(1, 0, 0), wall_c)
	add_face.call([bbr, bbl, tbl, tbr], Vector3(0, 0, 1), wall_c)
	add_face.call([bbl, bfl, tfl, tbl], Vector3(-1, 0, 0), wall_c)
	add_face.call([tfl, tfr, tbr, tbl], Vector3(0, 1, 0), roof_c) # Flat roof for fallback
	
	var mat = StandardMaterial3D.new()
	mat.vertex_color_use_as_albedo = true
	var mesh = st.commit()
	mesh.surface_set_material(0, mat)
	return mesh

func _process(_delta):
	# Poll for updates every 30 frames
	if Engine.get_frames_drawn() % 30 == 0:
		for zone_id in ZONE_NAMES.keys():
			for variant in range(NUM_VARIANTS):
				update_buildings(zone_id, variant)

func update_buildings(zone_id: int, variant: int):
	var buffer = simulation_node.get_building_transforms(zone_id, variant)
	var mmi = multimeshes[zone_id][variant]
	var mm = mmi.multimesh
	
	var count = buffer.size() / 12
	mm.instance_count = count
	if count > 0:
		mm.buffer = buffer
