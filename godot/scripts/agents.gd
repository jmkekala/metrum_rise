extends Node3D

@onready var simulation_node = $"../SimulationNode"

var multimesh_instance: MultiMeshInstance3D
var physical_mesh: SphereMesh

var debug_mesh_instance: MeshInstance3D
var debug_mesh: ImmediateMesh
var show_paths = false

var ui_layer: CanvasLayer
var pop_label: Label
var emp_label: Label
var hap_label: Label
var wealth_label: Label

func _ready():
	multimesh_instance = MultiMeshInstance3D.new()
	var mm = MultiMesh.new()
	mm.transform_format = MultiMesh.TRANSFORM_3D
	mm.use_colors = false
	mm.use_custom_data = false
	mm.instance_count = 0
	
	physical_mesh = SphereMesh.new()
	physical_mesh.radius = 1.0
	physical_mesh.height = 2.0 # 2-meter tall citizen representation!
	
	var mat = StandardMaterial3D.new()
	mat.albedo_color = Color(1.0, 0.4, 0.4) # Bright reddish-pink spheres
	mat.roughness = 0.5
	physical_mesh.material = mat
	mm.mesh = physical_mesh
	
	multimesh_instance.multimesh = mm
	add_child(multimesh_instance)

	debug_mesh_instance = MeshInstance3D.new()
	debug_mesh = ImmediateMesh.new()
	var debug_mat = StandardMaterial3D.new()
	debug_mat.albedo_color = Color(0.2, 0.8, 1.0) # Bright neon cyan lines for networking
	debug_mat.shading_mode = BaseMaterial3D.SHADING_MODE_UNSHADED
	debug_mat.transparency = BaseMaterial3D.TRANSPARENCY_ALPHA
	debug_mat.albedo_color.a = 0.5
	debug_mesh_instance.material_override = debug_mat
	debug_mesh_instance.mesh = debug_mesh
	add_child(debug_mesh_instance)

	# --- Demographics Floating HUD ---
	ui_layer = CanvasLayer.new()
	add_child(ui_layer)
	
	var margin = MarginContainer.new()
	margin.set_anchors_preset(Control.PRESET_TOP_LEFT)
	margin.add_theme_constant_override("margin_top", 20)
	margin.add_theme_constant_override("margin_left", 20)
	ui_layer.add_child(margin)
	
	var panel = PanelContainer.new()
	var style = StyleBoxFlat.new()
	style.bg_color = Color(0.1, 0.1, 0.1, 0.9) # Darker
	style.set_corner_radius_all(8)
	style.content_margin_left = 20
	style.content_margin_right = 20
	style.content_margin_top = 20
	style.content_margin_bottom = 20
	panel.add_theme_stylebox_override("panel", style)
	margin.add_child(panel)
	
	var vbox = VBoxContainer.new()
	vbox.add_theme_constant_override("separation", 10)
	panel.add_child(vbox)
	
	var title = Label.new()
	title.text = "City Demographics"
	title.add_theme_font_size_override("font_size", 22)
	vbox.add_child(title)
	
	var sep = HSeparator.new()
	vbox.add_child(sep)
	
	pop_label = Label.new()
	pop_label.text = "Population: 0 citizens"
	vbox.add_child(pop_label)
	
	emp_label = Label.new()
	emp_label.text = "Employment: 0.0 %"
	vbox.add_child(emp_label)
	
	hap_label = Label.new()
	hap_label.text = "Avg Happiness: 0.0"
	vbox.add_child(hap_label)

	wealth_label = Label.new()
	wealth_label.text = "Avg Wealth: $ 0.0"
	vbox.add_child(wealth_label)

func _input(event):
	if event is InputEventKey and event.pressed and not event.echo:
		if event.keycode == KEY_P:
			show_paths = !show_paths
			if not show_paths:
				debug_mesh.clear_surfaces()

func _process(delta):
	# Refresh visual swarms very often for smooth movement
	if Engine.get_frames_drawn() % 5 == 0:
		update_swarm()

func update_swarm():
	var buffer = simulation_node.get_agent_transforms()
	var mm = multimesh_instance.multimesh
	
	var count = buffer.size() / 12
	# We ALWAYS update count for agents since they can be killed via Swap-And-Pop
	if count != mm.instance_count:
		mm.instance_count = count
		
	if count > 0:
		mm.buffer = buffer

	if show_paths:
		var paths = simulation_node.get_agent_paths_debug()
		debug_mesh.clear_surfaces()
		if paths.size() > 0:
			debug_mesh.surface_begin(Mesh.PRIMITIVE_LINES)
			for i in range(paths.size()):
				debug_mesh.surface_add_vertex(paths[i])
			debug_mesh.surface_end()
            
	# Update UI Demographics once per second
	if Engine.get_frames_drawn() % 60 == 0:
		var stats = simulation_node.get_city_demographics()
		pop_label.text = "Population: %d citizens" % stats.get("population", 0)
		emp_label.text = "Employment: %.1f %%" % stats.get("employment_rate", 0.0)
		hap_label.text = "Avg Happiness: %.1f" % stats.get("average_happiness", 100.0)
		wealth_label.text = "Avg Wealth: $ %.1f" % stats.get("average_wealth", 0.0)
