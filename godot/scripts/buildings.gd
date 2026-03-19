extends Node3D

@onready var simulation_node = $"../SimulationNode"

var multimeshes = {}

func _ready():
	# 1=Res, 2=Com, 3=Ind, 4=Mix
	# 1=Res, 2=Com, 3=Ind, 4=Mix
	setup_multimesh(1, Color(0.9, 0.85, 0.7), Color(0.2, 0.4, 0.2)) # Beige walls, Green roof
	setup_multimesh(2, Color(0.8, 0.8, 0.9), Color(0.1, 0.2, 0.5)) # Cool Blue/Grey
	setup_multimesh(3, Color(0.5, 0.5, 0.5), Color(0.4, 0.1, 0.1)) # Concrete, Rusty roof
	setup_multimesh(4, Color(0.8, 0.7, 0.6), Color(0.3, 0.2, 0.1)) # Earthy Mix

func setup_multimesh(zone_id: int, wall_color: Color, roof_color: Color):
	var mmi = MultiMeshInstance3D.new()
	var mm = MultiMesh.new()
	mm.transform_format = MultiMesh.TRANSFORM_3D
	mm.use_colors = false # We use vertex colors from the mesh, not per-instance colors
	mm.use_custom_data = false
	mm.instance_count = 0
	
	var mesh = create_house_mesh(wall_color, roof_color)
	
	var mat = StandardMaterial3D.new()
	mat.vertex_color_use_as_albedo = true # Use the procedural colors!
	mat.roughness = 0.7
	mat.cull_mode = BaseMaterial3D.CULL_BACK # Re-enable culling now that winding is fixed
	mesh.surface_set_material(0, mat)
	mm.mesh = mesh
	
	mmi.multimesh = mm
	add_child(mmi)
	multimeshes[zone_id] = mmi

func create_house_mesh(wall_c: Color, roof_c: Color) -> ArrayMesh:
	var st = SurfaceTool.new()
	st.begin(Mesh.PRIMITIVE_TRIANGLES)
	
	# Dimensions (Residential scale: 5m total height)
	var w = 0.45
	var d = 0.45
	var h_wall = 3.5
	var h_roof = 5.0
	var offset = -2.5 # Centered at 0, ranging from -2.5 to +2.5.
	
	# Helper to add a quad with a fixed normal
	var add_face = func(pts: Array, normal: Vector3, color: Color):
		st.set_color(color)
		st.set_normal(normal)
		st.add_vertex(pts[0]); st.add_vertex(pts[1]); st.add_vertex(pts[2])
		st.add_vertex(pts[0]); st.add_vertex(pts[2]); st.add_vertex(pts[3])
	
	# Vertices (Centered)
	var bfl = Vector3(-w, 0 + offset, -d); var bfr = Vector3(w, 0 + offset, -d)
	var bbr = Vector3(w, 0 + offset, d);  var bbl = Vector3(-w, 0 + offset, d)
	var tfl = Vector3(-w, h_wall + offset, -d); var tfr = Vector3(w, h_wall + offset, -d)
	var tbr = Vector3(w, h_wall + offset, d);  var tbl = Vector3(-w, h_wall + offset, d)
	
	# Walls
	add_face.call([bfl, bfr, tfr, tfl], Vector3(0, 0, -1), wall_c) # Front
	add_face.call([bfr, bbr, tbr, tfr], Vector3(1, 0, 0), wall_c)  # Right
	add_face.call([bbr, bbl, tbl, tbr], Vector3(0, 0, 1), wall_c)  # Back
	add_face.call([bbl, bfl, tfl, tbl], Vector3(-1, 0, 0), wall_c) # Left
	
	# Roof Ridge
	var rf = Vector3(0, h_roof + offset, -d)
	var rb = Vector3(0, h_roof + offset, d)
	
	# Roof Gables
	st.set_color(wall_c)
	st.set_normal(Vector3(0, 0, -1))
	st.add_vertex(tfl); st.add_vertex(tfr); st.add_vertex(rf)
	st.set_normal(Vector3(0, 0, 1))
	st.add_vertex(tbr); st.add_vertex(tbl); st.add_vertex(rb)
	
	# Roof Slopes
	st.set_color(roof_c)
	var nl = Vector3(-1, 1, 0).normalized()
	add_face.call([tfl, rf, rb, tbl], nl, roof_c)
	var nr = Vector3(1, 1, 0).normalized()
	add_face.call([tfr, tbr, rb, rf], nr, roof_c)
	
	return st.commit()

func _process(delta):
	# We poll for updates every half-second to avoid hammering the FFI buffer
	if Engine.get_frames_drawn() % 30 == 0:
		update_buildings(1)
		update_buildings(2)
		update_buildings(3)
		update_buildings(4)

func update_buildings(zone_id: int):
	var buffer = simulation_node.get_building_transforms(zone_id)
	var mmi = multimeshes[zone_id]
	var mm = mmi.multimesh
	
	var count = buffer.size() / 12
	mm.instance_count = count
	if count > 0:
		mm.buffer = buffer
