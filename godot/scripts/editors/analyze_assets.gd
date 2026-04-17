extends SceneTree

func _init():
	var metadata = {}
	var base_dir = "res://assets/models/buildings/"
	var dir = DirAccess.open(base_dir)
	
	if dir:
		dir.list_dir_begin()
		var zone_dir = dir.get_next()
		while zone_dir != "":
			if dir.current_is_dir() and zone_dir != "." and zone_dir != ".." and zone_dir != "Textures":
				print("Analyzing zone: ", zone_dir)
				metadata[zone_dir] = {}
				analyze_zone(base_dir + zone_dir, metadata[zone_dir])
			zone_dir = dir.get_next()
	
	var file = FileAccess.open("res://assets/models/buildings/model_metadata.json", FileAccess.WRITE)
	file.store_string(JSON.stringify(metadata, "\t"))
	print("Asset analysis complete. Saved to model_metadata.json")
	quit()

func analyze_zone(path: String, zone_meta: Dictionary):
	var dir = DirAccess.open(path)
	if not dir: return
	
	dir.list_dir_begin()
	var file_name = dir.get_next()
	while file_name != "":
		if file_name.ends_with(".glb"):
			var full_path = path + "/" + file_name
			var scene = load(full_path)
			if scene:
				var instance = scene.instantiate()
				var mesh_node = find_mesh_recursive(instance)
				if mesh_node and mesh_node is MeshInstance3D:
					var aabb = mesh_node.mesh.get_aabb()
					var material_names = []
					for s in range(mesh_node.mesh.get_surface_count()):
						var mat = mesh_node.mesh.surface_get_material(s)
						if mat: material_names.append(mat.resource_name)
					
					zone_meta[file_name] = {
						"size_x": aabb.size.x,
						"size_y": aabb.size.y,
						"size_z": aabb.size.z,
						"center_offset_y": aabb.position.y,
						"materials": material_names
					}
				instance.free()
		file_name = dir.get_next()

func find_mesh_recursive(node: Node) -> MeshInstance3D:
	if node is MeshInstance3D:
		return node
	for child in node.get_children():
		var found = find_mesh_recursive(child)
		if found:
			return found
	return null
