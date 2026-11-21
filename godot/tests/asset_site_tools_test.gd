# SPDX-License-Identifier: GPL-3.0-or-later

## Asset editor regression for the site layout tools (`TOOLS-40`): position fields keep a drafted
## value to 0.1 mm and an edit writes only the coordinate it changes; "Move whole site" moves
## everything on the lot as one undo step and refuses a move off the lot; an asset with a model
## draft ledger offers a loading bay at each vehicle door.
## Run with --asset-editor in an isolated profile; the fixture model is generated.
extends SceneTree

const EditorScene = preload("res://scenes/AssetEditor.tscn")
const BAY := 1
const PACK := "site-tools-test"
const ASSET := "building.industrial.site_tools"
var failures := 0
var editor: Node
var document: Object

func _initialize() -> void:
	call_deferred("_run")

func expect(value: bool, message: String) -> void:
	if not value:
		failures += 1
		push_error(message)

# A 4 m block standing on the ground.
func _model(path: String) -> void:
	var scene := Node3D.new()
	var mesh := MeshInstance3D.new()
	mesh.mesh = BoxMesh.new()
	mesh.mesh.size = Vector3(4, 4, 4)
	mesh.position = Vector3(0, 2, 0)
	scene.add_child(mesh)
	var gltf := GLTFDocument.new()
	var state := GLTFState.new()
	expect(gltf.append_from_scene(scene, state) == OK, "generate the block")
	expect(gltf.write_to_filesystem(state, path) == OK, "write the block")
	scene.free()

# The model draft ledger of the fixture asset: a valid sidecar of the block with a main entrance
# and a vehicle door on its east wall, as `tools/model_draft.py` writes it.
func _write_ledger() -> void:
	var digest := "0".repeat(64)
	var sidecar := {
		"schema": "metrum-model-facts", "version": 1, "name": "block", "path": "buildings/test/block",
		"source": {"commit": "0".repeat(40), "dirty": false, "blender": "5.2.0"}, "license": "CC-BY-4.0",
		"bounds": {"min": [-2.0, 0.0, -2.0], "max": [2.0, 4.0, 2.0]},
		"footprint": [[-2.0, -2.0], [2.0, -2.0], [2.0, 2.0], [-2.0, 2.0]],
		"storeys": [], "dwellings": 0,
		"doors": [
			{"id": "front", "kind": "entrance", "position": [0.0, 0.0, 2.0], "facing": [0.0, 0.0, 1.0], "width_m": 1.0, "height_m": 2.1, "main": true},
			{"id": "dock", "kind": "vehicle", "position": [2.0, 0.0, 0.5], "facing": [1.0, 0.0, 0.0], "width_m": 3.0, "height_m": 3.5, "main": false},
		],
		"lods": [{"file": "site-tools-block.glb", "triangles": 12, "sha256": digest}],
		"textures": {"orm": {"file": "block_orm.png", "sha256": digest}, "normal": {"file": "block_normal.png", "sha256": digest}},
		"variants": [{"id": "red", "albedo": "block_albedo_red.png", "sha256": digest}],
		"default_variant": "red", "preview": {"file": "../preview.png", "sha256": digest}, "hero": false,
	}
	var folder := ProjectSettings.globalize_path("user://mods/%s/model_sources" % PACK)
	DirAccess.make_dir_recursive_absolute(folder)
	var file := FileAccess.open(folder.path_join(ASSET + ".json"), FileAccess.WRITE)
	file.store_string(JSON.stringify({"sidecar": sidecar, "role": {}}))
	file.close()

func _params() -> Dictionary:
	return document.snapshot()["params"]

func _run() -> void:
	var path := ProjectSettings.globalize_path("user://site-tools-block.glb")
	_model(path)
	_write_ledger()
	editor = EditorScene.instantiate()
	root.add_child(editor)
	editor._session.create_asset({"type": "industrial", "name": "Site tools fixture", "pack": {"pack_id": PACK}})
	editor._on_glb_file_selected(path)
	document = editor._session.document
	var fixture: Dictionary = document.snapshot()
	var params: Dictionary = fixture["params"]
	params["asset_id"] = ASSET
	params["lot_width_cells"] = 3
	params["lot_depth_cells"] = 4
	params["frontage_forward"] = [0.0, 0.0, 1.0]
	# Drafted values finer than a centimetre: the part's R2 depth and a bay on a door's centre.
	params["mesh_parts"][0]["position"] = [-8.5, 0.0, -2.2031]
	params["mesh_parts"][0]["pivot_offset"] = [0.0, 0.0, 0.0]
	params["anchors"] = [
		{"anchor_type": "entrance", "name": "main", "position": [-8.5, 0.0, -0.2031], "forward": [0.0, 0.0, 1.0]},
		{"anchor_type": "loading_bay", "name": "bay", "position": [12.0, 0.0, -1.5729], "forward": [-1.0, 0.0, 0.0], "width_m": 3.5, "length_m": 8.0, "vehicle_class": "freight"},
		{"anchor_type": "driveway", "name": "gate", "position": [10.0, 0.0, 20.0], "forward": [0.0, 0.0, -1.0], "width_m": 6.0, "vehicle_class": "freight"},
	]
	params["site_surfaces"] = [{"name": "Yard", "material": "asphalt", "y_m": 0.01, "vertices": [[0.0, -10.0], [14.5, -10.0], [14.5, 19.5], [0.0, 19.5]]}]
	document.reset(fixture)
	for frame in 4:
		await process_frame
	expect(document.snapshot() == fixture, "opening the document changes nothing")

	_precision()
	_move_site()
	_bay_at_door()

	editor.free()
	await process_frame
	if failures == 0:
		print("PASS asset_site_tools_test")
	quit(0 if failures == 0 else 1)

# Positions step in 0.1 mm and an edit writes only its own coordinate.
func _precision() -> void:
	var view = editor._view
	editor._set_selected_site_anchors([BAY], BAY)
	expect(is_equal_approx(view._site_anchor_z_spin.value, -1.5729), "the Z field shows the drafted value: %f" % view._site_anchor_z_spin.value)
	view._site_anchor_x_spin.value = 12.25
	var bay: Array = _params()["anchors"][BAY]["position"]
	expect(is_equal_approx(float(bay[0]), 12.25) and float(bay[2]) == -1.5729 and document.undo_label() == "Position X (m)", "an X edit keeps the stored Z: %s" % [bay])
	view._site_anchor_z_spin.value = -1.5731
	bay = _params()["anchors"][BAY]["position"]
	expect(absf(float(bay[2]) + 1.5731) < 1e-9 and is_equal_approx(float(bay[0]), 12.25), "a typed Z keeps its 0.1 mm: %s" % [bay])
	document.undo()
	view._site_anchor_yaw_spin.value = 0.0
	bay = _params()["anchors"][BAY]["position"]
	expect(is_equal_approx(float(bay[0]), 12.25) and float(bay[2]) == -1.5729, "a turn leaves the position as it was: %s" % [bay])
	document.undo()
	document.undo()
	editor._select_mesh_part(0)
	expect(is_equal_approx(view._part_z_spin.value, -2.2031), "the part's Z field shows the drafted value")
	view._part_x_spin.value = -8.25
	var part: Array = _params()["mesh_parts"][0]["position"]
	expect(is_equal_approx(float(part[0]), -8.25) and float(part[2]) == -2.2031 and float(part[1]) == 0.0, "a part X edit keeps the stored Z: %s" % [part])
	document.undo()
	expect(float(_params()["mesh_parts"][0]["position"][0]) == -8.5, "one undo restores the part")

# One offset for everything, one undo step; a Driveway moves only along the street edge.
func _move_site() -> void:
	var before: Dictionary = document.snapshot()
	expect(editor._site_move.move(-1.0, -0.5), "the whole site moves")
	var params := _params()
	expect(params["mesh_parts"][0]["position"] == [-9.5, 0.0, -2.7031], "the part moves: %s" % [params["mesh_parts"][0]["position"]])
	expect(params["anchors"][BAY]["position"] == [11.0, 0.0, -2.0729], "the bay moves: %s" % [params["anchors"][BAY]["position"]])
	expect(params["anchors"][2]["position"] == [9.0, 0.0, 20.0], "the gate moves along the street edge only: %s" % [params["anchors"][2]["position"]])
	expect(params["site_surfaces"][0]["vertices"][0] == [-1.0, -10.5], "the yard moves: %s" % [params["site_surfaces"][0]["vertices"]])
	expect(document.undo_label() == "Move whole site", "one command: %s" % document.undo_label())
	expect(editor._parts[0].position.is_equal_approx(Vector3(-9.5, 0.0, -2.7031)), "the preview follows")
	document.undo()
	expect(document.snapshot() == before, "one undo restores everything")
	# The block's back would leave the lot: refused, nothing changes.
	expect(not editor._site_move.move(0.0, -16.0), "a move off the lot is refused")
	expect(document.snapshot() == before and document.undo_label() != "Move whole site", "a refused move changes nothing")

# The ledger's vehicle door is offered; its bay stands end-on to it, just clear of the block.
func _bay_at_door() -> void:
	editor._update_site_anchor_controls()
	var doors: RefCounted = editor._door_bays
	expect(doors.door_ids() == ["dock"] and doors._row.visible, "the vehicle door is offered, the entrance is not: %s" % [doors.door_ids()])
	var before: Dictionary = document.snapshot()
	expect(doors.add(doors._doors[0]), "the bay is added")
	var anchors: Array = _params()["anchors"]
	var bay: Dictionary = anchors.back()
	# The door at lot (-6.5, -1.7031) faces +X; the block ends at x -6.5, so the far end stands
	# at -6.49 and the 8 m bay's entry edge at 1.51.
	var position: Array = bay["position"]
	expect(anchors.size() == 4 and bay["anchor_type"] == "loading_bay" and bay["name"] == "dock", "a loading bay named after the door: %s" % [bay])
	expect(is_equal_approx(float(position[0]), 1.51) and is_equal_approx(float(position[2]), -1.7031) and bay["forward"] == [-1.0, 0.0, 0.0], "end-on to the door: %s" % [bay])
	expect(float(bay["width_m"]) == 3.5 and float(bay["length_m"]) == 8.0 and bay["vehicle_class"] == "freight", "the standard size: %s" % [bay])
	expect(document.undo_label() == "Loading bay at door" and editor._selected_site_anchor_index == 3, "one step, the new bay selected")
	document.undo()
	expect(document.snapshot() == before, "one undo removes it")
