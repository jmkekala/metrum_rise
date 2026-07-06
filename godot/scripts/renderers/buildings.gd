## Building renderer — maintains one MultiMeshInstance3D per registered building asset part.
##
## Rust methods called:
##   load_asset_packs(dir_path: String) -> String
##   get_registered_asset_ids() -> PackedStringArray
##   get_building_mesh_part_count(asset_id: String) -> int
##   get_building_mesh_part_lod0_native_path(asset_id: String, part_index: int) -> String
##   get_building_transforms_for_asset_part(asset_id: String, part_index: int) -> PackedFloat32Array
##   get_deserted_building_transforms_for_asset_part(asset_id: String, part_index: int) -> PackedFloat32Array
##   get_building_plot_transforms(zone_id: int) -> PackedFloat32Array
##   get_construction_site_transforms(zone_id: int) -> PackedFloat32Array
##   get_construction_foundation_transforms(zone_id: int) -> PackedFloat32Array
##   get_construction_scaffold_transforms(zone_id: int) -> PackedFloat32Array
##   get_building_site_revision() -> int
##   get_building_site_mesh_data() -> Dictionary
##
## At startup, reads user://active_packs.cfg for the list of enabled pack IDs, then
## passes each enabled pack's native path to Rust for manifest scanning. Packs not
## listed in the config are ignored. Rust parses the manifests; GDScript loads the
## corresponding mesh files and maintains one MultiMeshInstance3D per asset_id/part.
## Building transforms are polled every 30 frames.
## A parallel deserted_multimeshes dict renders economically dead buildings in gray.
extends Node3D

const CFG_PATH := "user://active_packs.cfg"
const PART_KEY_SEP := "|part:"
const WorldMaterials = preload("res://scripts/renderers/world_materials.gd")
const SceneLightingConfig := preload("res://scripts/core/scene_lighting.gd")
const PerfDebug := preload("res://scripts/core/perf_debug.gd")

@onready var simulation_node = $"../SimulationNode"
@onready var zoning_overlay = $"../ZoningOverlay"

## multimeshes[asset_part_key] = MultiMeshInstance3D
var multimeshes: Dictionary = {}
## deserted_multimeshes[asset_part_key] = MultiMeshInstance3D — gray material override for deserted state
var deserted_multimeshes: Dictionary = {}
## part_assets[asset_part_key] = qualified asset id
var part_assets: Dictionary = {}
## part_indices[asset_part_key] = mesh part index
var part_indices: Dictionary = {}
## foundation_multimeshes[zone_id] = MultiMeshInstance3D
var foundation_multimeshes: Dictionary = {}
## construction_site_multimeshes[zone_id] = MultiMeshInstance3D
var construction_site_multimeshes: Dictionary = {}
## construction_foundation_multimeshes[zone_id] = MultiMeshInstance3D
var construction_foundation_multimeshes: Dictionary = {}
## construction_scaffold_multimeshes[zone_id] = MultiMeshInstance3D
var construction_scaffold_multimeshes: Dictionary = {}
var building_site_ground_instance: MeshInstance3D
var building_site_surface_instance: MeshInstance3D
var building_site_revision: int = -1
var building_debug_enabled: bool = false
var building_site_dump_enabled: bool = false
var building_site_visual_mode: String = ""
var building_site_debug_materials: Dictionary = {}

var show_foundations := false

const ZONE_IDS = [1, 2, 3, 4, 5]

func reload_asset_packs() -> void:
	_load_enabled_packs()
	_rebuild_multimeshes()
	building_site_revision = -1
	_update_building_sites()

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
	building_debug_enabled = _building_debug_is_enabled()
	building_site_dump_enabled = _building_site_dump_is_enabled()
	building_site_visual_mode = _building_site_visual_mode_from_env()
	if building_debug_enabled:
		print(
			"[DEBUG:buildings] enabled site_visual_mode=%s site_dump=%s"
			% [
				building_site_visual_mode if not building_site_visual_mode.is_empty() else "off",
				str(building_site_dump_enabled),
			]
		)
	elif _building_site_visual_debug_enabled():
		print("[DEBUG:buildings] site visual overlay enabled mode=%s" % [building_site_visual_mode])

	_load_enabled_packs()

	# Build foundation multimeshes for each zone type.
	for zone_id in ZONE_IDS:
		_setup_foundation(zone_id)
		_setup_construction_site(zone_id)
		_setup_construction_foundation(zone_id)
		_setup_construction_scaffold(zone_id)
	_setup_building_site_surfaces()

	# Build one MultiMeshInstance3D for each registered building asset.
	_rebuild_multimeshes()
	_update_building_sites(true)

func update_all_buildings() -> void:
	_rebuild_multimeshes()
	for key in multimeshes.keys():
		_update_buildings_for_asset_part(key)
	for key in deserted_multimeshes.keys():
		_update_deserted_multimesh(key)
	for zone_id in ZONE_IDS:
		_update_foundation(zone_id)
		_update_construction_site(zone_id)
		_update_construction_foundation(zone_id)
		_update_construction_scaffold(zone_id)
	_update_building_sites(true)

func _rebuild_multimeshes() -> void:
	var asset_ids: PackedStringArray = simulation_node.get_registered_asset_ids()
	for aid in asset_ids:
		if aid == "broken:error":
			var broken_key := _part_key(aid, 0)
			if not multimeshes.has(broken_key):
				_setup_multimesh_for_asset_part(aid, 0)
			continue
		var part_count: int = simulation_node.get_building_mesh_part_count(aid)
		for part_index in part_count:
			var key := _part_key(aid, part_index)
			if not multimeshes.has(key):
				_setup_multimesh_for_asset_part(aid, part_index)

func _part_key(asset_id: String, part_index: int) -> String:
	return "%s%s%d" % [asset_id, PART_KEY_SEP, part_index]

func get_building_mesh_for_asset_part(asset_id: String, part_index: int) -> Mesh:
	var key := _part_key(asset_id, part_index)
	if multimeshes.has(key):
		var mmi: MultiMeshInstance3D = multimeshes[key]
		if mmi.multimesh and mmi.multimesh.mesh:
			return mmi.multimesh.mesh
	return _load_mesh_for_asset_part(asset_id, part_index)

func _setup_multimesh_for_asset_part(asset_id: String, part_index: int) -> void:
	var key := _part_key(asset_id, part_index)
	var mesh := _load_mesh_for_asset_part(asset_id, part_index)
	var is_broken := asset_id == "broken:error"
	var mmi := MultiMeshInstance3D.new()
	var mm := MultiMesh.new()
	mm.transform_format = MultiMesh.TRANSFORM_3D
	mm.instance_count = 0
	if mesh:
		mm.mesh = mesh
	else:
		mm.mesh = _create_fallback_mesh()
		if is_broken:
			var mat := StandardMaterial3D.new()
			mat.albedo_color = Color.MAGENTA
			mat.shading_mode = BaseMaterial3D.SHADING_MODE_UNSHADED # Glow!
			mm.mesh.surface_set_material(0, mat)

	mmi.multimesh = mm
	mmi.gi_mode = GeometryInstance3D.GI_MODE_DYNAMIC
	SceneLightingConfig.apply_shadow_policy(
		mmi,
		SceneLightingConfig.SHADOW_STATIC_CASTER,
		"buildings"
	)
	add_child(mmi)
	multimeshes[key] = mmi
	part_assets[key] = asset_id
	part_indices[key] = part_index
	# Deserted variant: same mesh geometry, warm gray material override.
	if not is_broken:
		_setup_deserted_multimesh_for_asset_part(asset_id, part_index, mesh)

func _setup_deserted_multimesh_for_asset_part(asset_id: String, part_index: int, mesh: Mesh) -> void:
	var key := _part_key(asset_id, part_index)
	var mmi := MultiMeshInstance3D.new()
	var mm := MultiMesh.new()
	mm.transform_format = MultiMesh.TRANSFORM_3D
	mm.instance_count = 0
	# Use the same mesh; if none loaded yet use the fallback (no special broken tint for deserted).
	mm.mesh = mesh if mesh else _create_fallback_mesh()
	var mat := StandardMaterial3D.new()
	# Warm gray, slightly desaturated — visually distinct from the live color palette.
	mat.albedo_color = Color(0.45, 0.42, 0.38, 1.0)
	mat.shading_mode = BaseMaterial3D.SHADING_MODE_PER_PIXEL
	# material_override on the node, not surface_set_material on the mesh — avoids mutating
	# the shared Mesh resource that the normal multimesh also references.
	mmi.material_override = mat
	mmi.multimesh = mm
	mmi.gi_mode = GeometryInstance3D.GI_MODE_DYNAMIC
	SceneLightingConfig.apply_shadow_policy(
		mmi,
		SceneLightingConfig.SHADOW_STATIC_CASTER,
		"buildings"
	)
	add_child(mmi)
	deserted_multimeshes[key] = mmi

func _load_mesh_for_asset_part(asset_id: String, part_index: int) -> Mesh:
	if asset_id == "broken:error":
		return null
	# Ask Rust for the native path to the LOD0 file for this asset part.
	var native_path: String = simulation_node.get_building_mesh_part_lod0_native_path(asset_id, part_index)
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
		push_warning("Buildings: LOD0 file not found for '%s' part %d: %s" % [asset_id, part_index, godot_path])
		return null
	var ext := native_path.get_extension().to_lower()
	var doc: Resource
	var state: Resource
	if ext == "fbx":
		doc = FBXDocument.new()
		state = FBXState.new()
	else:
		doc = GLTFDocument.new()
		state = GLTFState.new()
	if doc.append_from_file(native_path, state) != OK:
		push_warning("Buildings: failed to load mesh for '%s' part %d: %s" % [asset_id, part_index, native_path])
		return null
	var scene: Node = doc.generate_scene(state)
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
	SceneLightingConfig.apply_shadow_policy(
		mmi,
		SceneLightingConfig.SHADOW_DEBUG_OVERLAY,
		"buildings"
	)
	mmi.visible = show_foundations
	add_child(mmi)
	foundation_multimeshes[zone_id] = mmi

func _setup_construction_site(zone_id: int) -> void:
	var mmi := MultiMeshInstance3D.new()
	var mm := MultiMesh.new()
	mm.transform_format = MultiMesh.TRANSFORM_3D
	mm.instance_count = 0
	var mesh := BoxMesh.new()
	mesh.size = Vector3.ONE
	var mat := StandardMaterial3D.new()
	mat.transparency = BaseMaterial3D.TRANSPARENCY_ALPHA
	mat.roughness = 0.9
	mat.albedo_color = Color(0.24, 0.23, 0.20, 0.72)
	mesh.material = mat
	mm.mesh = mesh
	mmi.multimesh = mm
	SceneLightingConfig.apply_shadow_policy(
		mmi,
		SceneLightingConfig.SHADOW_DEBUG_OVERLAY,
		"buildings"
	)
	add_child(mmi)
	construction_site_multimeshes[zone_id] = mmi

func _setup_construction_foundation(zone_id: int) -> void:
	var mmi := MultiMeshInstance3D.new()
	var mm := MultiMesh.new()
	mm.transform_format = MultiMesh.TRANSFORM_3D
	mm.instance_count = 0
	var mesh := BoxMesh.new()
	mesh.size = Vector3.ONE
	var mat := StandardMaterial3D.new()
	mat.roughness = 0.82
	mat.albedo_color = Color(0.48, 0.49, 0.46, 1.0)
	mesh.material = mat
	mm.mesh = mesh
	mmi.multimesh = mm
	SceneLightingConfig.apply_shadow_policy(
		mmi,
		SceneLightingConfig.SHADOW_DEBUG_OVERLAY,
		"buildings"
	)
	add_child(mmi)
	construction_foundation_multimeshes[zone_id] = mmi

func _setup_construction_scaffold(zone_id: int) -> void:
	var mmi := MultiMeshInstance3D.new()
	var mm := MultiMesh.new()
	mm.transform_format = MultiMesh.TRANSFORM_3D
	mm.instance_count = 0
	var mesh := BoxMesh.new()
	mesh.size = Vector3.ONE
	var mat := StandardMaterial3D.new()
	mat.roughness = 0.72
	mat.albedo_color = Color(0.34, 0.33, 0.30, 1.0)
	mesh.material = mat
	mm.mesh = mesh
	mmi.multimesh = mm
	SceneLightingConfig.apply_shadow_policy(
		mmi,
		SceneLightingConfig.SHADOW_DEBUG_OVERLAY,
		"buildings"
	)
	add_child(mmi)
	construction_scaffold_multimeshes[zone_id] = mmi

func _setup_building_site_surfaces() -> void:
	building_site_ground_instance = MeshInstance3D.new()
	SceneLightingConfig.apply_shadow_policy(
		building_site_ground_instance,
		SceneLightingConfig.SHADOW_RECEIVER_ONLY,
		"yards"
	)
	building_site_ground_instance.material_override = _building_site_ground_material()
	add_child(building_site_ground_instance)

	building_site_surface_instance = MeshInstance3D.new()
	SceneLightingConfig.apply_shadow_policy(
		building_site_surface_instance,
		SceneLightingConfig.SHADOW_RECEIVER_ONLY,
		"yards"
	)
	add_child(building_site_surface_instance)

func _process(_delta: float) -> void:
	var rebuild_due := Engine.get_frames_drawn() % 30 == 0
	if rebuild_due and simulation_node.is_sim_core_busy():
		return
	if not PerfDebug.is_enabled():
		if rebuild_due:
			_rebuild_multimeshes()
			for key in multimeshes.keys():
				_update_buildings_for_asset_part(key)
			for key in deserted_multimeshes.keys():
				_update_deserted_multimesh(key)
			for zone_id in ZONE_IDS:
				_update_foundation(zone_id)
				_update_construction_site(zone_id)
				_update_construction_foundation(zone_id)
				_update_construction_scaffold(zone_id)
			_update_building_sites()
		return

	var frame_start_us := Time.get_ticks_usec()
	var rebuild_elapsed_ms := 0.0
	var update_elapsed_ms := 0.0
	if rebuild_due:
		var rebuild_start_us := Time.get_ticks_usec()
		_rebuild_multimeshes()
		rebuild_elapsed_ms = float(Time.get_ticks_usec() - rebuild_start_us) / 1000.0
		var update_start_us := Time.get_ticks_usec()
		for key in multimeshes.keys():
			_update_buildings_for_asset_part(key)
		for key in deserted_multimeshes.keys():
			_update_deserted_multimesh(key)
		for zone_id in ZONE_IDS:
			_update_foundation(zone_id)
			_update_construction_site(zone_id)
			_update_construction_foundation(zone_id)
			_update_construction_scaffold(zone_id)
		_update_building_sites()
		update_elapsed_ms = float(Time.get_ticks_usec() - update_start_us) / 1000.0
	PerfDebug.record(
		"buildings",
		float(Time.get_ticks_usec() - frame_start_us) / 1000.0,
		{
			"rebuild": rebuild_elapsed_ms,
			"update": update_elapsed_ms,
		}
	)

func _input(event: InputEvent) -> void:
	if event is InputEventKey and event.pressed and not event.echo:
		if event.keycode == KEY_F:
			show_foundations = !show_foundations
			for mmi in foundation_multimeshes.values():
				mmi.visible = show_foundations

func _update_buildings_for_asset_part(key: String) -> void:
	if not multimeshes.has(key):
		return
	var asset_id: String = part_assets.get(key, "")
	var part_index: int = part_indices.get(key, 0)
	var buffer: PackedFloat32Array = simulation_node.get_building_transforms_for_asset_part(asset_id, part_index)
	var mmi: MultiMeshInstance3D = multimeshes[key]
	var count := buffer.size() / 12
	mmi.multimesh.instance_count = count
	if count > 0:
		mmi.multimesh.buffer = buffer

func _update_deserted_multimesh(key: String) -> void:
	if not deserted_multimeshes.has(key):
		return
	var asset_id: String = part_assets.get(key, "")
	var part_index: int = part_indices.get(key, 0)
	var buffer: PackedFloat32Array = simulation_node.get_deserted_building_transforms_for_asset_part(asset_id, part_index)
	var mmi: MultiMeshInstance3D = deserted_multimeshes[key]
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

func _update_construction_site(zone_id: int) -> void:
	if not construction_site_multimeshes.has(zone_id):
		return
	var buffer: PackedFloat32Array = simulation_node.get_construction_site_transforms(zone_id)
	var mmi: MultiMeshInstance3D = construction_site_multimeshes[zone_id]
	var count := buffer.size() / 12
	mmi.multimesh.instance_count = count
	if count > 0:
		mmi.multimesh.buffer = buffer

func _update_construction_foundation(zone_id: int) -> void:
	if not construction_foundation_multimeshes.has(zone_id):
		return
	var buffer: PackedFloat32Array = simulation_node.get_construction_foundation_transforms(zone_id)
	var mmi: MultiMeshInstance3D = construction_foundation_multimeshes[zone_id]
	var count := buffer.size() / 12
	mmi.multimesh.instance_count = count
	if count > 0:
		mmi.multimesh.buffer = buffer

func _update_construction_scaffold(zone_id: int) -> void:
	if not construction_scaffold_multimeshes.has(zone_id):
		return
	var buffer: PackedFloat32Array = simulation_node.get_construction_scaffold_transforms(zone_id)
	var mmi: MultiMeshInstance3D = construction_scaffold_multimeshes[zone_id]
	var count := buffer.size() / 12
	mmi.multimesh.instance_count = count
	if count > 0:
		mmi.multimesh.buffer = buffer

func _update_building_sites(force: bool = false) -> void:
	if not building_site_ground_instance or not building_site_surface_instance:
		return
	var revision := int(simulation_node.get_building_site_revision())
	if not force and revision == building_site_revision:
		return
	var data: Dictionary = simulation_node.get_building_site_mesh_data()
	building_site_revision = int(data.get("revision", revision))
	var ground_vertices := data.get("ground_vertices", PackedVector3Array()) as PackedVector3Array
	var asphalt_vertices := data.get("asphalt_vertices", PackedVector3Array()) as PackedVector3Array
	var concrete_vertices := data.get("concrete_vertices", PackedVector3Array()) as PackedVector3Array
	var ground_material := _building_site_ground_material()
	building_site_ground_instance.mesh = _building_site_mesh_from_vertices(
		ground_vertices,
		ground_material
	)
	building_site_surface_instance.mesh = _building_site_surface_mesh(data)
	_print_building_site_debug(
		data,
		ground_vertices,
		asphalt_vertices,
		concrete_vertices,
		ground_material
	)

func _building_site_surface_mesh(data: Dictionary):
	var mesh := ArrayMesh.new()
	_add_building_site_surface(
		mesh,
		data.get("asphalt_vertices", PackedVector3Array()) as PackedVector3Array,
		_building_site_surface_material(WorldMaterials.MATERIAL_ASPHALT)
	)
	_add_building_site_surface(
		mesh,
		data.get("concrete_vertices", PackedVector3Array()) as PackedVector3Array,
		_building_site_surface_material(WorldMaterials.MATERIAL_CONCRETE)
	)
	return mesh if mesh.get_surface_count() > 0 else null

func _building_site_mesh_from_vertices(vertices: PackedVector3Array, material: Material):
	var mesh := ArrayMesh.new()
	_add_building_site_surface(mesh, vertices, material)
	return mesh if mesh.get_surface_count() > 0 else null

func _add_building_site_surface(mesh: ArrayMesh, vertices: PackedVector3Array, material: Material) -> void:
	if vertices.size() < 3:
		return
	var normals := PackedVector3Array()
	normals.resize(vertices.size())
	for i in normals.size():
		normals[i] = Vector3.UP
	var arrays := []
	arrays.resize(Mesh.ARRAY_MAX)
	arrays[Mesh.ARRAY_VERTEX] = vertices
	arrays[Mesh.ARRAY_NORMAL] = normals
	mesh.add_surface_from_arrays(Mesh.PRIMITIVE_TRIANGLES, arrays)
	mesh.surface_set_material(mesh.get_surface_count() - 1, material)

func _building_site_ground_material() -> Material:
	if not _building_site_visual_debug_enabled():
		return WorldMaterials.site_ground_material()
	return _building_site_debug_material("ground", Color(0.0, 0.95, 0.25, 1.0))

func _building_site_surface_material(material_name: String) -> Material:
	if not _building_site_visual_debug_enabled():
		return WorldMaterials.site_surface_material(material_name)
	match material_name:
		WorldMaterials.MATERIAL_CONCRETE:
			return _building_site_debug_material("concrete", Color(0.25, 0.55, 1.0, 0.65))
		_:
			return _building_site_debug_material("asphalt", Color(1.0, 0.20, 0.10, 0.65))

func _building_site_debug_material(role: String, color: Color) -> StandardMaterial3D:
	if building_site_debug_materials.has(role):
		return building_site_debug_materials[role] as StandardMaterial3D
	var mat := StandardMaterial3D.new()
	mat.resource_name = "debug_building_site_" + role
	mat.albedo_color = color
	mat.transparency = BaseMaterial3D.TRANSPARENCY_DISABLED if color.a >= 0.99 else BaseMaterial3D.TRANSPARENCY_ALPHA
	mat.shading_mode = BaseMaterial3D.SHADING_MODE_UNSHADED
	mat.cull_mode = BaseMaterial3D.CULL_DISABLED
	mat.roughness = 1.0
	building_site_debug_materials[role] = mat
	return mat

func _building_site_visual_debug_enabled() -> bool:
	return not building_site_visual_mode.is_empty() and building_site_visual_mode != "off"

func _print_building_site_debug(
	data: Dictionary,
	ground_vertices: PackedVector3Array,
	asphalt_vertices: PackedVector3Array,
	concrete_vertices: PackedVector3Array,
	ground_material: Material
) -> void:
	if not building_debug_enabled and not building_site_dump_enabled:
		return
	if not building_site_dump_enabled:
		print(
			"[DEBUG:buildings] site_mesh revision=%d ground_vertices=%d asphalt_vertices=%d concrete_vertices=%d"
			% [
				int(data.get("revision", building_site_revision)),
				ground_vertices.size(),
				asphalt_vertices.size(),
				concrete_vertices.size(),
			]
		)
		return
	var debug_sites: Array = data.get("debug_sites", [])
	var ground_area := _triangle_area_xz(ground_vertices)
	var asphalt_area := _triangle_area_xz(asphalt_vertices)
	var concrete_area := _triangle_area_xz(concrete_vertices)
	print(
		"[DEBUG:buildings] BUILDING_SITE_DUMP_BEGIN revision=%d sites=%d visual_mode=%s"
		% [
			int(data.get("revision", building_site_revision)),
			debug_sites.size(),
			building_site_visual_mode if not building_site_visual_mode.is_empty() else "off",
		]
	)
	print(
		"[DEBUG:buildings] buffers ground_vertices=%d ground_triangles=%d ground_area_m2=%.3f ground_bounds=%s asphalt_vertices=%d asphalt_triangles=%d asphalt_area_m2=%.3f asphalt_bounds=%s concrete_vertices=%d concrete_triangles=%d concrete_area_m2=%.3f concrete_bounds=%s"
		% [
			ground_vertices.size(),
			ground_vertices.size() / 3,
			ground_area,
			_vector3_bounds_label(ground_vertices),
			asphalt_vertices.size(),
			asphalt_vertices.size() / 3,
			asphalt_area,
			_vector3_bounds_label(asphalt_vertices),
			concrete_vertices.size(),
			concrete_vertices.size() / 3,
			concrete_area,
			_vector3_bounds_label(concrete_vertices),
		]
	)
	print(
		"[DEBUG:buildings] materials ground=%s asphalt=%s concrete=%s"
		% [
			_material_debug_label(ground_material),
			_material_debug_label(_building_site_surface_material(WorldMaterials.MATERIAL_ASPHALT)),
			_material_debug_label(_building_site_surface_material(WorldMaterials.MATERIAL_CONCRETE)),
		]
	)
	print(
		"[DEBUG:buildings] uv_sources ground=world_xz(site_ground grass shader) asphalt=world_xz(site_surface shader) concrete=world_xz(site_surface shader)"
	)
	for site_variant in debug_sites:
		var site: Dictionary = site_variant as Dictionary
		_print_building_site_record(site)
	print("[DEBUG:buildings] BUILDING_SITE_DUMP_END")

func _print_building_site_record(site: Dictionary) -> void:
	var site_index := int(site.get("site_index", -1))
	print(
		"[DEBUG:buildings] site index=%d asset_id=%s zone=%s edge=%d side=%d frontage_t=%.3f center=%s facing=%s side_offset_m=%.3f cells=%dx%d lot_m=(%.3f,%.3f) support_y=%.3f area_m2=%.3f bounds=[%s..%s] footprint=%s surfaces=%d"
		% [
			site_index,
			str(site.get("asset_id", "")),
			str(site.get("zone_type", "")),
			int(site.get("edge_idx", -1)),
			int(site.get("side", 0)),
			float(site.get("frontage_t", 0.0)),
			_vector2_label(site.get("center", Vector2.ZERO)),
			_vector2_label(site.get("facing_dir", Vector2.ZERO)),
			float(site.get("side_offset_m", 0.0)),
			int(site.get("width_cells", 0)),
			int(site.get("depth_cells", 0)),
			float(site.get("lot_width_m", 0.0)),
			float(site.get("lot_depth_m", 0.0)),
			float(site.get("support_height_m", 0.0)),
			float(site.get("footprint_area_m2", 0.0)),
			_vector2_label(site.get("bounds_min", Vector2.ZERO)),
			_vector2_label(site.get("bounds_max", Vector2.ZERO)),
			_packed_vector2_label(site.get("footprint", PackedVector2Array())),
			(site.get("surfaces", []) as Array).size(),
		]
	)
	var road_sample_count := int(site.get("road_sample_count", 0))
	print(
		"[DEBUG:buildings] site_grading site=%d road_samples=%d road_y=[%.3f..%.3f] road_range=%.3f max_abs_support_delta_road=%.3f terrain_visual_y=[%.3f..%.3f] terrain_visual_range=%.3f max_abs_support_delta_visual=%.3f claimed_probe_valid=%s claimed_probe_point=%s claimed_probe_has_height=%s claimed_probe_y=%.3f claimed_probe_delta=%.3f"
		% [
			site_index,
			road_sample_count,
			float(site.get("road_height_min_m", 0.0)),
			float(site.get("road_height_max_m", 0.0)),
			float(site.get("road_height_range_m", 0.0)),
			float(site.get("max_abs_support_delta_road_m", 0.0)),
			float(site.get("terrain_visual_height_min_m", 0.0)),
			float(site.get("terrain_visual_height_max_m", 0.0)),
			float(site.get("terrain_visual_height_range_m", 0.0)),
			float(site.get("max_abs_support_delta_visual_m", 0.0)),
			str(site.get("claimed_road_probe_valid", false)),
			_vector2_label(site.get("claimed_road_probe_point", Vector2.ZERO)),
			str(site.get("claimed_road_probe_has_height", false)),
			float(site.get("claimed_road_probe_height_m", 0.0)),
			float(site.get("claimed_road_probe_support_delta_m", 0.0)),
		]
	)
	var samples: Array = site.get("samples", [])
	for sample_variant in samples:
		var sample: Dictionary = sample_variant as Dictionary
		print(
			"[DEBUG:buildings] site_sample site=%d label=%s point=%s terrain_source_y=%.3f terrain_visual_y=%.3f support_delta_source=%.3f support_delta_visual=%.3f"
			% [
				site_index,
				str(sample.get("label", "")),
				_vector2_label(sample.get("point", Vector2.ZERO)),
				float(sample.get("terrain_source_height_m", 0.0)),
				float(sample.get("terrain_visual_height_m", 0.0)),
				float(sample.get("support_delta_source_m", 0.0)),
				float(sample.get("support_delta_visual_m", 0.0)),
			]
		)
	var edge_samples: Array = site.get("edge_samples", [])
	for edge_sample_variant in edge_samples:
		var edge_sample: Dictionary = edge_sample_variant as Dictionary
		print(
			"[DEBUG:buildings] site_edge_sample site=%d edge=%d role=%s sample=%d/%d t=%.3f point=%s road_probe=%s road_visible=%s road_y=%.3f support_delta_road=%.3f terrain_source_y=%.3f terrain_visual_y=%.3f support_delta_source=%.3f support_delta_visual=%.3f"
			% [
				site_index,
				int(edge_sample.get("edge_index", -1)),
				str(edge_sample.get("edge_role", "")),
				int(edge_sample.get("sample_index", 0)) + 1,
				int(edge_sample.get("sample_count", 0)),
				float(edge_sample.get("t", 0.0)),
				_vector2_label(edge_sample.get("point", Vector2.ZERO)),
				_vector2_label(edge_sample.get("road_probe_point", Vector2.ZERO)),
				str(edge_sample.get("road_visible", false)),
				float(edge_sample.get("road_visible_height_m", 0.0)),
				float(edge_sample.get("support_delta_road_m", 0.0)),
				float(edge_sample.get("terrain_source_height_m", 0.0)),
				float(edge_sample.get("terrain_visual_height_m", 0.0)),
				float(edge_sample.get("support_delta_source_m", 0.0)),
				float(edge_sample.get("support_delta_visual_m", 0.0)),
			]
		)
	var surfaces: Array = site.get("surfaces", [])
	for surface_variant in surfaces:
		var surface: Dictionary = surface_variant as Dictionary
		print(
			"[DEBUG:buildings] site_surface site=%d index=%d name=%s material=%s height_y=%.3f area_m2=%.3f bounds=[%s..%s] vertices=%s"
			% [
				site_index,
				int(surface.get("surface_index", -1)),
				str(surface.get("name", "")),
				str(surface.get("material", "")),
				float(surface.get("height_m", 0.0)),
				float(surface.get("area_m2", 0.0)),
				_vector2_label(surface.get("bounds_min", Vector2.ZERO)),
				_vector2_label(surface.get("bounds_max", Vector2.ZERO)),
				_packed_vector2_label(surface.get("vertices", PackedVector2Array())),
			]
		)

func _triangle_area_xz(vertices: PackedVector3Array) -> float:
	var area := 0.0
	var triangle_count := vertices.size() / 3
	for triangle_index in triangle_count:
		var a := vertices[triangle_index * 3]
		var b := vertices[triangle_index * 3 + 1]
		var c := vertices[triangle_index * 3 + 2]
		area += absf(
			(a.x * (b.z - c.z) + b.x * (c.z - a.z) + c.x * (a.z - b.z)) * 0.5
		)
	return area

func _vector3_bounds_label(vertices: PackedVector3Array) -> String:
	if vertices.is_empty():
		return "none"
	var min_v := vertices[0]
	var max_v := vertices[0]
	for vertex in vertices:
		min_v.x = minf(min_v.x, vertex.x)
		min_v.y = minf(min_v.y, vertex.y)
		min_v.z = minf(min_v.z, vertex.z)
		max_v.x = maxf(max_v.x, vertex.x)
		max_v.y = maxf(max_v.y, vertex.y)
		max_v.z = maxf(max_v.z, vertex.z)
	return "(%.3f,%.3f,%.3f)..(%.3f,%.3f,%.3f)" % [
		min_v.x,
		min_v.y,
		min_v.z,
		max_v.x,
		max_v.y,
		max_v.z,
	]

func _material_debug_label(material: Material) -> String:
	if material == null:
		return "null"
	if material is StandardMaterial3D:
		var standard := material as StandardMaterial3D
		return "StandardMaterial3D name=%s albedo=%s roughness=%.3f cull=%d transparency=%d" % [
			standard.resource_name,
			_color_label(standard.albedo_color),
			standard.roughness,
			standard.cull_mode,
			standard.transparency,
		]
	if material is ShaderMaterial:
		var shader_material := material as ShaderMaterial
		var shader_path := ""
		if shader_material.shader:
			shader_path = shader_material.shader.resource_path
		if shader_path.ends_with("site_ground.gdshader"):
			return "ShaderMaterial shader=%s grass_macro_scale=%s grass_detail_scale=%s grass_albedo_strength=%s hillshade_strength=%s" % [
				shader_path,
				str(shader_material.get_shader_parameter("terrain_grass_macro_scale")),
				str(shader_material.get_shader_parameter("terrain_grass_detail_scale")),
				str(shader_material.get_shader_parameter("terrain_grass_albedo_strength")),
				str(shader_material.get_shader_parameter("hillshade_strength")),
			]
		return "ShaderMaterial shader=%s uv_scale=%s macro_uv_scale=%s macro_influence=%s brightness=%s" % [
			shader_path,
			str(shader_material.get_shader_parameter("uv_scale")),
			str(shader_material.get_shader_parameter("macro_uv_scale")),
			str(shader_material.get_shader_parameter("macro_influence")),
			str(shader_material.get_shader_parameter("brightness")),
		]
	return material.get_class()

func _color_label(color: Color) -> String:
	return "(%.3f,%.3f,%.3f,%.3f)" % [color.r, color.g, color.b, color.a]

func _vector2_label(value) -> String:
	var vector := value as Vector2
	return "(%.3f,%.3f)" % [vector.x, vector.y]

func _packed_vector2_label(value) -> String:
	var points := value as PackedVector2Array
	var parts: Array[String] = []
	for point in points:
		parts.append(_vector2_label(point))
	return "[" + ", ".join(parts) + "]"

func _building_debug_is_enabled() -> bool:
	var explicit_value := OS.get_environment("METRUM_DEBUG_BUILDINGS").strip_edges()
	if explicit_value == "1":
		return true
	var debug_value := OS.get_environment("METRUM_DEBUG").strip_edges()
	if debug_value.is_empty() or debug_value == "0":
		return false
	var filter := OS.get_environment("METRUM_DEBUG_FILTER").strip_edges().to_lower()
	for entry_variant in filter.split(","):
		var entry := String(entry_variant).strip_edges()
		if entry == "buildings" or entry == "building-sites":
			return true
	return false

func _building_site_dump_is_enabled() -> bool:
	var value := OS.get_environment("METRUM_DEBUG_BUILDING_SITES_DUMP").strip_edges().to_lower()
	return value == "1" or value == "true" or value == "yes" or value == "full"

func _building_site_visual_mode_from_env() -> String:
	var value := OS.get_environment("METRUM_DEBUG_BUILDING_SITES_VISUAL").strip_edges().to_lower()
	if value.is_empty() or value == "0" or value == "false":
		return ""
	match value:
		"material", "materials", "source", "sources":
			return "material"
		"off":
			return "off"
		_:
			return "material"
