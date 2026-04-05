## Building renderer — maintains one MultiMeshInstance3D per registered asset ID.
##
## Rust methods called:
##   load_asset_packs(dir_path: String) -> String
##   get_registered_asset_ids() -> PackedStringArray
##   get_building_transforms_for_asset(asset_id: String) -> PackedFloat32Array
##   get_building_plot_transforms(zone_id: int) -> PackedFloat32Array
##
## At startup, reads user://active_packs.cfg for the list of enabled pack IDs, then
## passes each enabled pack's native path to Rust for manifest scanning. Packs not
## listed in the config are ignored. Rust parses the manifests; GDScript loads the
## corresponding mesh files and maintains one MultiMeshInstance3D per asset_id.
## Building transforms are polled every 30 frames.
extends Node3D

const CFG_PATH := "user://active_packs.cfg"

@onready var simulation_node = $"../SimulationNode"

## multimeshes[asset_id] = MultiMeshInstance3D
var multimeshes: Dictionary = {}
## foundation_multimeshes[zone_id] = MultiMeshInstance3D
var foundation_multimeshes: Dictionary = {}

var show_foundations := false

const ZONE_IDS = [1, 2, 3, 4, 5]

func reload_asset_packs() -> void:
	_load_enabled_packs()
	_rebuild_multimeshes()

func _load_enabled_packs() -> void:
	var cfg := ConfigFile.new()
	if cfg.load(CFG_PATH) != OK:
		push_warning("Buildings: no active_packs.cfg found — no packs loaded. Use Mods menu to enable packs.")
		return
	var enabled: Array = cfg.get_value("packs", "enabled", [])
	if enabled.is_empty():
		push_warning("Buildings: no packs enabled in active_packs.cfg.")
		return
	var mods_native := ProjectSettings.globalize_path("user://mods/")
	var filter := ",".join(enabled)
	var warnings: String = simulation_node.load_asset_packs(mods_native, filter)
	if warnings != "":
		for w in warnings.split("\n"):
			if w != "":
				push_warning("Asset pack warning: " + w)

func _ready() -> void:
	_load_enabled_packs()

	# Build foundation multimeshes for each zone type.
	for zone_id in ZONE_IDS:
		_setup_foundation(zone_id)

	# Build one MultiMeshInstance3D for each registered building asset.
	_rebuild_multimeshes()

func update_all_buildings() -> void:
	_rebuild_multimeshes()
	for aid in multimeshes.keys():
		_update_buildings_for_asset(aid)
	for zone_id in ZONE_IDS:
		_update_foundation(zone_id)

func _rebuild_multimeshes() -> void:
	var asset_ids: PackedStringArray = simulation_node.get_registered_asset_ids()
	for aid in asset_ids:
		if not multimeshes.has(aid):
			_setup_multimesh_for_asset(aid)

func _setup_multimesh_for_asset(asset_id: String) -> void:
	var mesh := _load_mesh_for_asset(asset_id)
	var mmi := MultiMeshInstance3D.new()
	var mm := MultiMesh.new()
	mm.transform_format = MultiMesh.TRANSFORM_3D
	mm.instance_count = 0
	mm.mesh = mesh if mesh else _create_fallback_mesh()
	mmi.multimesh = mm
	mmi.gi_mode = GeometryInstance3D.GI_MODE_DYNAMIC
	add_child(mmi)
	multimeshes[asset_id] = mmi

func _load_mesh_for_asset(asset_id: String) -> Mesh:
	# Ask Rust for the native path to the LOD0 file for this asset.
	var native_path: String = simulation_node.get_lod0_native_path(asset_id)
	if native_path.is_empty():
		return null
	# Convert native path to a Godot res:// or user:// path via globalize/localize.
	# Since the file is under user://, we derive the user:// path from the native path.
	var user_native := ProjectSettings.globalize_path("user://")
	var godot_path: String
	if native_path.begins_with(user_native):
		godot_path = "user://" + native_path.substr(user_native.length())
	else:
		godot_path = native_path  # fallback: use native path directly
	if not FileAccess.file_exists(godot_path):
		push_warning("Buildings: LOD0 file not found for '%s': %s" % [asset_id, godot_path])
		return null
	var doc := GLTFDocument.new()
	var state := GLTFState.new()
	if doc.append_from_file(native_path, state) != OK:
		push_warning("Buildings: failed to load GLB for '%s': %s" % [asset_id, native_path])
		return null
	var scene := doc.generate_scene(state)
	if not scene:
		return null
	var mesh := _bake_scene_to_mesh(scene)
	scene.queue_free()
	return mesh

# Bakes all MeshInstance3D nodes in the scene into a single ArrayMesh,
# applying each node's transform relative to the scene root so the result
# is correctly positioned and scaled at the scene origin.
func _bake_scene_to_mesh(root: Node) -> ArrayMesh:
	var result := ArrayMesh.new()
	_bake_node(root, root, Transform3D.IDENTITY, result)
	return result if result.get_surface_count() > 0 else null

func _bake_node(node: Node, root: Node, parent_xform: Transform3D, result: ArrayMesh) -> void:
	var xform := parent_xform
	if node is Node3D and node != root:
		xform = parent_xform * (node as Node3D).transform
	if node is MeshInstance3D:
		var mi := node as MeshInstance3D
		if mi.mesh:
			var normal_xform := xform.basis.inverse().transposed()
			for surf in mi.mesh.get_surface_count():
				var arrays := mi.mesh.surface_get_arrays(surf)
				# Transform vertex positions and normals into root space.
				var verts: PackedVector3Array = arrays[Mesh.ARRAY_VERTEX]
				for i in verts.size():
					verts[i] = xform * verts[i]
				arrays[Mesh.ARRAY_VERTEX] = verts
				if arrays[Mesh.ARRAY_NORMAL] is PackedVector3Array:
					var normals: PackedVector3Array = arrays[Mesh.ARRAY_NORMAL]
					for i in normals.size():
						normals[i] = (normal_xform * normals[i]).normalized()
					arrays[Mesh.ARRAY_NORMAL] = normals
				var prim: Mesh.PrimitiveType = mi.mesh.surface_get_primitive_type(surf)
				var mat := mi.mesh.surface_get_material(surf)
				var surf_idx := result.get_surface_count()
				result.add_surface_from_arrays(prim, arrays)
				if mat:
					result.surface_set_material(surf_idx, mat)
	for child in node.get_children():
		_bake_node(child, root, xform, result)

func _create_fallback_mesh() -> ArrayMesh:
	var st := SurfaceTool.new()
	st.begin(Mesh.PRIMITIVE_TRIANGLES)
	var w := 0.45; var d := 0.45; var h := 4.0
	var wall_c := Color(0.9, 0.85, 0.7)
	var roof_c := Color(0.2, 0.4, 0.2)
	var add_face := func(pts: Array, normal: Vector3, color: Color) -> void:
		st.set_color(color); st.set_normal(normal)
		st.add_vertex(pts[0]); st.add_vertex(pts[1]); st.add_vertex(pts[2])
		st.add_vertex(pts[0]); st.add_vertex(pts[2]); st.add_vertex(pts[3])
	var bfl := Vector3(-w, 0, -d); var bfr := Vector3(w, 0, -d)
	var bbr := Vector3(w, 0,  d); var bbl := Vector3(-w, 0,  d)
	var tfl := Vector3(-w, h, -d); var tfr := Vector3(w, h, -d)
	var tbr := Vector3(w, h,  d); var tbl := Vector3(-w, h,  d)
	add_face.call([bfl, bfr, tfr, tfl], Vector3(0, 0, -1), wall_c)
	add_face.call([bfr, bbr, tbr, tfr], Vector3(1, 0,  0), wall_c)
	add_face.call([bbr, bbl, tbl, tbr], Vector3(0, 0,  1), wall_c)
	add_face.call([bbl, bfl, tfl, tbl], Vector3(-1, 0, 0), wall_c)
	add_face.call([tfl, tfr, tbr, tbl], Vector3(0, 1,  0), roof_c)
	var mat := StandardMaterial3D.new()
	mat.vertex_color_use_as_albedo = true
	var mesh := st.commit()
	mesh.surface_set_material(0, mat)
	return mesh

func _setup_foundation(zone_id: int) -> void:
	var mmi := MultiMeshInstance3D.new()
	var mm := MultiMesh.new()
	mm.transform_format = MultiMesh.TRANSFORM_3D
	mm.instance_count = 0
	var mesh := BoxMesh.new()
	mesh.size = Vector3(1.0, 0.1, 1.0)
	var mat := StandardMaterial3D.new()
	mat.transparency = BaseMaterial3D.TRANSPARENCY_ALPHA
	match zone_id:
		1: mat.albedo_color = Color(0.2, 0.4, 0.2, 0.5)
		2: mat.albedo_color = Color(0.2, 0.2, 0.5, 0.5)
		3: mat.albedo_color = Color(0.4, 0.4, 0.1, 0.5)
		4: mat.albedo_color = Color(0.1, 0.4, 0.4, 0.5)
		_: mat.albedo_color = Color(0.3, 0.3, 0.3, 0.5)
	mesh.material = mat
	mm.mesh = mesh
	mmi.multimesh = mm
	mmi.cast_shadow = GeometryInstance3D.SHADOW_CASTING_SETTING_OFF
	mmi.visible = show_foundations
	add_child(mmi)
	foundation_multimeshes[zone_id] = mmi

func _process(_delta: float) -> void:
	if Engine.get_frames_drawn() % 30 == 0:
		_rebuild_multimeshes()
		for aid in multimeshes.keys():
			_update_buildings_for_asset(aid)
		for zone_id in ZONE_IDS:
			_update_foundation(zone_id)

func _input(event: InputEvent) -> void:
	if event is InputEventKey and event.pressed and not event.echo:
		if event.keycode == KEY_F:
			show_foundations = !show_foundations
			for mmi in foundation_multimeshes.values():
				mmi.visible = show_foundations

func _update_buildings_for_asset(asset_id: String) -> void:
	if not multimeshes.has(asset_id):
		return
	var buffer: PackedFloat32Array = simulation_node.get_building_transforms_for_asset(asset_id)
	var mmi: MultiMeshInstance3D = multimeshes[asset_id]
	var count := buffer.size() / 12
	mmi.multimesh.instance_count = count
	if count > 0:
		mmi.multimesh.buffer = buffer

func _update_foundation(zone_id: int) -> void:
	if not foundation_multimeshes.has(zone_id):
		return
	var buffer: PackedFloat32Array = simulation_node.get_building_plot_transforms(zone_id)
	var mmi: MultiMeshInstance3D = foundation_multimeshes[zone_id]
	var count := buffer.size() / 12
	mmi.multimesh.instance_count = count
	if count > 0:
		mmi.multimesh.buffer = buffer
