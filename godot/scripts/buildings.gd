## Building renderer — maintains one MultiMeshInstance3D per registered asset ID.
##
## Rust methods called:
##   load_asset_packs(dir_path: String) -> String
##   get_registered_asset_ids() -> PackedStringArray
##   get_building_transforms_for_asset(asset_id: String) -> PackedFloat32Array
##   get_building_plot_transforms(zone_id: int) -> PackedFloat32Array
##
## At startup, this script scans the native path of "user://mods/" for content packs.
## Rust parses the manifests; GDScript loads the corresponding mesh files and maintains
## one MultiMeshInstance3D per asset_id. Building transforms are polled every 30 frames.
extends Node3D

@onready var simulation_node = $"../SimulationNode"

## multimeshes[asset_id] = MultiMeshInstance3D
var multimeshes: Dictionary = {}
## foundation_multimeshes[zone_id] = MultiMeshInstance3D
var foundation_multimeshes: Dictionary = {}

var show_foundations := false

const ZONE_IDS = [1, 2, 3, 4, 5]

func _ready() -> void:
	# Resolve the mods directory to a native path and hand it to Rust.
	var mods_path := ProjectSettings.globalize_path("user://mods/")
	var warnings := simulation_node.load_asset_packs(mods_path)
	if warnings != "":
		for w in warnings.split("\n"):
			if w != "":
				push_warning("Asset pack warning: " + w)

	# Build foundation multimeshes for each zone type.
	for zone_id in ZONE_IDS:
		_setup_foundation(zone_id)

	# Build one MultiMeshInstance3D for each registered building asset.
	_rebuild_multimeshes()

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
	# asset_id format: "pack_id:category.subcategory.name"
	# Expected pack layout: user://mods/<pack_id>/buildings/<subcategory>/<name>/lods/lod0.glb
	var parts := asset_id.split(":")
	if parts.size() != 2:
		return null
	var pack_id := parts[0]
	var local_id := parts[1]     # e.g. "building.residential.lowrise_corner"
	var segments := local_id.split(".")
	if segments.size() < 3:
		return null
	# segments[0] = "building", segments[1] = category, segments[2..] = name parts
	var category := segments[1]
	var name_part := ".".join(segments.slice(2))
	var path := "user://mods/%s/buildings/%s/%s/lods/lod0.glb" % [pack_id, category, name_part]
	if not FileAccess.file_exists(path):
		return null
	var scene = load(path)
	if not scene:
		return null
	var instance = scene.instantiate()
	var mesh_node := _find_mesh_recursive(instance)
	var mesh: Mesh = null
	if mesh_node and mesh_node is MeshInstance3D:
		mesh = mesh_node.mesh
	instance.queue_free()
	return mesh

func _find_mesh_recursive(node: Node) -> MeshInstance3D:
	if node is MeshInstance3D:
		return node
	for child in node.get_children():
		var found := _find_mesh_recursive(child)
		if found:
			return found
	return null

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
