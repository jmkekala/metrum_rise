## Agent renderer — streams agent positions from Rust into a MultiMeshInstance3D each frame.
##
## Rust methods called: get_agent_transforms(), get_agent_paths_debug(), get_city_demographics()
## Agent transforms arrive as a flat PackedFloat32Array of 12 floats per agent:
##   [basis.x(3), basis.y(3), basis.z(3), origin(3)] — matches Godot's Transform3D memory layout.
## Path debug lines (toggled with P key) arrive as a PackedVector3Array of point pairs.
extends Node3D

@onready var simulation_node = $"../SimulationNode"

var walker_mmi: MultiMeshInstance3D
var car_mmi: MultiMeshInstance3D

var debug_mesh_instance: MeshInstance3D
var debug_mesh: ImmediateMesh
var show_paths = false

var ui_layer: CanvasLayer
var pop_label: Label
var emp_label: Label
var hap_label: Label
var wealth_label: Label

func _ready():
	# --- Walker MultiMesh (pedestrians, future cyclists, etc.) ---
	walker_mmi = MultiMeshInstance3D.new()
	var wmm = MultiMesh.new()
	wmm.transform_format = MultiMesh.TRANSFORM_3D
	wmm.use_colors = false
	wmm.use_custom_data = false
	wmm.instance_count = 0
	var walker_mesh = CapsuleMesh.new()
	walker_mesh.radius = 0.2   # Slim human silhouette
	walker_mesh.height = 1.7
	var walker_mat = StandardMaterial3D.new()
	walker_mat.albedo_color = Color(0.85, 0.72, 0.60)
	walker_mat.roughness = 0.95
	walker_mesh.material = walker_mat
	wmm.mesh = walker_mesh
	walker_mmi.multimesh = wmm
	add_child(walker_mmi)

	# --- Car MultiMesh ---
	car_mmi = MultiMeshInstance3D.new()
	var cmm = MultiMesh.new()
	cmm.transform_format = MultiMesh.TRANSFORM_3D
	cmm.use_colors = false
	cmm.use_custom_data = false
	cmm.instance_count = 0
	cmm.mesh = _build_car_mesh()
	car_mmi.multimesh = cmm
	add_child(car_mmi)

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



func _process(delta):
	# Refresh visual swarms very often for smooth movement
	if Engine.get_frames_drawn() % 5 == 0:
		update_swarm()

func update_swarm():
	# Walkers
	var wbuf = simulation_node.get_agent_transforms()
	var wmm = walker_mmi.multimesh
	var wcount = wbuf.size() / 12
	if wcount != wmm.instance_count:
		wmm.instance_count = wcount
	if wcount > 0:
		wmm.buffer = wbuf

	# Cars
	var cbuf = simulation_node.get_car_transforms()
	var cmm = car_mmi.multimesh
	var ccount = cbuf.size() / 12
	if ccount != cmm.instance_count:
		cmm.instance_count = ccount
	if ccount > 0:
		cmm.buffer = cbuf

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

# Builds a car-shaped ArrayMesh from two boxes (body + cabin). No mesh files required.
# Car faces along local -Z (Godot forward). Origin at bottom-centre of body.
# Body: 1.8 m wide × 0.55 m tall × 4.2 m long
# Cabin: 1.4 m wide × 0.65 m tall × 2.2 m long, centred and sitting on the body
func _build_car_mesh() -> ArrayMesh:
	# Use plain Array (reference type) so helpers can append to the caller's arrays.
	# Converted to PackedVector3Array only when passing to add_surface_from_arrays.
	var verts: Array = []
	var norms: Array = []

	_add_box(verts, norms, Vector3(-0.9, 0.0, -2.1), Vector3(0.9, 0.55, 2.1))
	# Cabin starts at 0.56 (not 0.55) to avoid z-fighting with the body top face
	_add_box(verts, norms, Vector3(-0.9, 0.56, -1.1), Vector3(0.9, 1.2, 1.1))

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
