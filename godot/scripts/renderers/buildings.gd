# SPDX-License-Identifier: GPL-2.0-only

## Building scene coordinator: pack activation, spatial LOD resources and site visuals.
## Rust selects/batches building tiers; building_lods.gd owns mesh resources and GPU uploads.
extends Node3D

const ModPackConfig = preload("res://scripts/core/mod_pack_config.gd")
const BuildingLods := preload("res://scripts/renderers/building_lods.gd")
const WindowMaterials := preload("res://scripts/renderers/window_materials.gd")
const SceneLightingConfig := preload("res://scripts/core/scene_lighting.gd")
const PerfDebug := preload("res://scripts/core/perf_debug.gd")

@onready var simulation_node = $"../SimulationNode"
@onready var _stream: Node = get_node_or_null("../StreamNode")

var lod_renderer: Node3D
## The nine construction site nodes in the published rows' order: residential, commercial and
## industrial, each a site pad, a foundation and a scaffold (`get_building_site_rows`).
var site_multimeshes: Array[MultiMeshInstance3D] = []
## The site rows' generation last installed; -1 asks for every buffer.
var site_rows_generation: int = -1
# Whether this frame's site frame set transforms (the install ledger's charge).
var _sites_worked := false

func reload_asset_packs() -> void:
	_load_enabled_packs()
	if lod_renderer == null:
		_create_lod_renderer()
	else:
		lod_renderer.reload_resources()
	update_all_buildings()

func _create_lod_renderer() -> void:
	lod_renderer = BuildingLods.new()
	add_child(lod_renderer)
	var placeholder := _create_fallback_mesh()
	var material := StandardMaterial3D.new()
	material.albedo_color = Color.MAGENTA
	material.shading_mode = BaseMaterial3D.SHADING_MODE_UNSHADED
	placeholder.surface_set_material(0, material)
	lod_renderer.configure(simulation_node, placeholder)

func _load_enabled_packs() -> void:
	var enabled: Array = ModPackConfig.load_enabled_pack_ids()
	var mods_native := ProjectSettings.globalize_path("user://mods/")
	var warnings: String = simulation_node.load_asset_packs(mods_native, PackedStringArray(enabled))
	if warnings != "":
		for w in warnings.split("\n"):
			if w != "":
				push_warning("Asset pack warning: " + w)

func _ready() -> void:
	_load_enabled_packs()

	for _zone in 3:
		_setup_construction_site()
		_setup_construction_foundation()
		_setup_construction_scaffold()

	_create_lod_renderer()
	_install_site_rows(true)

func update_all_buildings() -> void:
	_install_site_rows(true)
	if lod_renderer != null and is_inside_tree():
		lod_renderer.update(get_viewport().get_camera_3d())

func get_building_mesh_for_asset_part(asset_id: String, part_index: int) -> Mesh:
	return lod_renderer.get_source_mesh(asset_id, part_index) if lod_renderer != null else null

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

func _setup_construction_site() -> void:
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
	mesh.material = WindowMaterials.create(mat)
	mm.mesh = mesh
	mmi.multimesh = mm
	SceneLightingConfig.apply_shadow_policy(
		mmi,
		SceneLightingConfig.SHADOW_DEBUG_OVERLAY,
		"buildings"
	)
	add_child(mmi)
	site_multimeshes.append(mmi)

func _setup_construction_foundation() -> void:
	var mmi := MultiMeshInstance3D.new()
	var mm := MultiMesh.new()
	mm.transform_format = MultiMesh.TRANSFORM_3D
	mm.instance_count = 0
	var mesh := BoxMesh.new()
	mesh.size = Vector3.ONE
	var mat := StandardMaterial3D.new()
	mat.roughness = 0.82
	mat.albedo_color = Color(0.48, 0.49, 0.46, 1.0)
	mesh.material = WindowMaterials.create(mat)
	mm.mesh = mesh
	mmi.multimesh = mm
	SceneLightingConfig.apply_shadow_policy(
		mmi,
		SceneLightingConfig.SHADOW_DEBUG_OVERLAY,
		"buildings"
	)
	add_child(mmi)
	site_multimeshes.append(mmi)

func _setup_construction_scaffold() -> void:
	var mmi := MultiMeshInstance3D.new()
	mmi.set_meta("reflection_source", true)
	var mm := MultiMesh.new()
	mm.transform_format = MultiMesh.TRANSFORM_3D
	mm.instance_count = 0
	var mesh := BoxMesh.new()
	mesh.size = Vector3.ONE
	var mat := StandardMaterial3D.new()
	mat.roughness = 0.72
	mat.albedo_color = Color(0.34, 0.33, 0.30, 1.0)
	mesh.material = WindowMaterials.create(mat)
	mm.mesh = mesh
	mmi.multimesh = mm
	SceneLightingConfig.apply_shadow_policy(
		mmi,
		SceneLightingConfig.SHADOW_DYNAMIC_CASTER,
		"buildings"
	)
	add_child(mmi)
	site_multimeshes.append(mmi)

func _process(_delta: float) -> void:
	var frame_start_us := Time.get_ticks_usec() if PerfDebug.is_enabled() else 0
	var lod_busy := false
	if lod_renderer != null:
		lod_renderer.update(get_viewport().get_camera_3d())
		lod_busy = lod_renderer.core_busy
	var sites_start_us := Time.get_ticks_usec()
	_sites_worked = false
	# Sites change in the frame their buildings do: the rows wait while the LOD update finds the
	# core busy (`RENDER-30`).
	if not lod_busy:
		_install_site_rows()
	var end_us := Time.get_ticks_usec()
	# A frame that set the sites' transforms charges the install ledger (`RENDER-30`, `RENDER-32-Q`).
	if _sites_worked and is_instance_valid(_stream):
		_stream.charge_install(4, float(end_us - sites_start_us))
	if PerfDebug.is_enabled():
		var elapsed_ms := float(end_us - frame_start_us) / 1000.0
		var details := {
			# The construction sites' frame (`RENDER-30`), apart from the LOD batches.
			"sites_ms": float(end_us - sites_start_us) / 1000.0,
			"lod_parts": lod_renderer.evaluated_parts if lod_renderer else 0,
			"lod_chunks": lod_renderer.queried_chunks if lod_renderer else 0,
			"upload_bytes": lod_renderer.uploaded_bytes if lod_renderer else 0,
		}
		# The batches' churn through the stream backend's pool (`RENDER-32-L`).
		if lod_renderer != null:
			details.merge(simulation_node.get_building_lod_work())
			details.merge(lod_renderer.timing)
		# The site rows' publisher (`RENDER-30`): refreshes, their time and the roster's scans.
		details.merge(simulation_node.get_building_site_row_work())
		PerfDebug.record("buildings", elapsed_ms, details)

## Sets the site nodes whose published rows changed, from the per-frame publication: never the
## core. `force` installs every buffer. Whether anything was installed.
func _install_site_rows(force: bool = false) -> bool:
	var rows: Array = simulation_node.get_building_site_rows(-1 if force else site_rows_generation)
	if rows.is_empty():
		return false
	_sites_worked = true
	for index in site_multimeshes.size():
		if rows[index + 1] != null:
			_set_multimesh_buffer(site_multimeshes[index], rows[index + 1] as PackedFloat32Array)
	site_rows_generation = int(rows[0])
	return true

func _set_multimesh_buffer(mmi: MultiMeshInstance3D, buffer: PackedFloat32Array) -> void:
	var count := buffer.size() / 12
	mmi.multimesh.instance_count = count
	if count > 0:
		mmi.multimesh.buffer = buffer
