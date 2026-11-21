# SPDX-License-Identifier: GPL-3.0-or-later

## "Loading bay at vehicle door" (`TOOLS-40`): for an asset drafted from a model, whose pack keeps
## its model draft ledger (`model_sources/<asset_id>.json`), one button per `vehicle` door of the
## model adds a loading bay end-on to that door through the document's "bay_at_door" command, one
## undo step. Rust reads the ledger and places the bay; this panel only lists the doors. Without
## a ledger, or without a vehicle door, the row is hidden. Docks are never derived on their own.
extends RefCounted

var _editor: Node
var _row: HFlowContainer
var _doors: Array = []
var _part_file := ""
var _source := ""

func build(view: RefCounted, parent: Control) -> void:
	_editor = view._editor
	_row = HFlowContainer.new()
	parent.add_child(_row)
	_row.visible = false

## The ledger's vehicle doors of the asset's pack and id; re-read only when either changes.
func refresh() -> void:
	if _row == null or _editor._session == null:
		return
	var params: Dictionary = _editor._session.params
	var pack_id := str(params.get("pack_id", ""))
	var asset_id := str(params.get("asset_id", ""))
	var source := pack_id + ":" + asset_id
	if source == _source:
		return
	_source = source
	_doors = []
	_part_file = ""
	if not pack_id.is_empty() and not asset_id.is_empty():
		var pack := ProjectSettings.globalize_path("user://mods/" + pack_id)
		var ledger = JSON.parse_string(AssetAuthoringFiles.model_vehicle_doors(pack, asset_id))
		if ledger is Dictionary:
			_doors = ledger.get("doors", [])
			_part_file = str(ledger.get("part", ""))
	for child in _row.get_children():
		child.queue_free()
	_row.visible = not _doors.is_empty()
	if _doors.is_empty():
		return
	var title := Label.new()
	title.text = "Loading bay at vehicle door:"
	_row.add_child(title)
	for door: Dictionary in _doors:
		var button := Button.new()
		button.text = str(door.get("id", ""))
		button.tooltip_text = "A %.1f m wide door: a loading bay end-on to it, centred, its far end just clear of the building" % float(door.get("width_m", 0.0))
		button.pressed.connect(add.bind(door))
		_row.add_child(button)

## The ids of the doors offered, in ledger order.
func door_ids() -> Array:
	return _doors.map(func(door): return str(door.get("id", "")))

## Add a loading bay at `door` (a ledger door) as one undo step and select it.
func add(door: Dictionary) -> bool:
	var part := _part()
	var targets: Array[Dictionary] = [{"kind": "mesh", "index": part}]
	var bounds: PackedFloat32Array = _editor._session.part_bounds().slice(part * 6, part * 6 + 6)
	var document = _editor._session.document
	var result: Dictionary = document.prepare_edit("bay_at_door", targets, {"door": door, "part_bounds": bounds})
	if result.has("error"):
		_editor._session.message(str(result.error))
		return false
	document.apply(result.document, "Loading bay at door")
	_editor._menus.actions.select(result.selection)
	return true

# The part that shows the model: the one whose LOD0 file is the ledger's, else the first.
func _part() -> int:
	var parts: Array = _editor._session.params.get("mesh_parts", [])
	for index in parts.size():
		var lods: Array = parts[index].get("lods", [])
		if not lods.is_empty() and str(lods[0].get("file", "")) == _part_file:
			return index
	return 0
