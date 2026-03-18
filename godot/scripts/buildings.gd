extends Node3D

@onready var simulation_node = $"../SimulationNode"

var multimeshes = {}

func _ready():
	# 1=Res, 2=Com, 3=Ind, 4=Mix
	setup_multimesh(1, Color(0.2, 0.8, 0.2)) # Green
	setup_multimesh(2, Color(0.2, 0.4, 0.9)) # Blue
	setup_multimesh(3, Color(0.9, 0.8, 0.2)) # Yellow
	setup_multimesh(4, Color(0.7, 0.3, 0.8)) # Purple

func setup_multimesh(zone_id: int, color: Color):
	var mmi = MultiMeshInstance3D.new()
	var mm = MultiMesh.new()
	mm.transform_format = MultiMesh.TRANSFORM_3D
	mm.use_colors = false
	mm.use_custom_data = false
	mm.instance_count = 0
	
	var mesh = BoxMesh.new()
	mesh.size = Vector3(8.0, 10.0, 8.0) # Placeholder building constraints
	
	var mat = StandardMaterial3D.new()
	mat.albedo_color = color
	mat.roughness = 0.8
	mesh.material = mat
	mm.mesh = mesh
	
	mmi.multimesh = mm
	add_child(mmi)
	multimeshes[zone_id] = mmi

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
