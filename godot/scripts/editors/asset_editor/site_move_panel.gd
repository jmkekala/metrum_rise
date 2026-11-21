# SPDX-License-Identifier: GPL-3.0-or-later

## "Move whole site" (`TOOLS-40`): every mesh part with its passages, every access point, site
## surface and planting vertex and every loading bay waypoint moves by one offset through the
## document's "move_site" command, one undo step, so a layout is re-centred on a resized lot. A
## Driveway stays on the street edge and moves only along it. Rust moves them and refuses a move
## that would take anything off the lot; this panel only takes the offset.
extends RefCounted

const TaskPanels = preload("res://scripts/editors/asset_editor/task_panels.gd")

var _editor: Node
var _dx: SpinBox
var _dz: SpinBox

func build(view: RefCounted, parent: Control) -> void:
	_editor = view._editor
	var box: VBoxContainer = view.advanced(parent, "move_site", "Move whole site")
	var hint := Label.new()
	hint.text = "Moves everything on the lot by one offset: every mesh part with its passages, every access point, surface and waypoint. A Driveway stays on the street edge and moves along it. A selected part dragged or typed alone moves alone."
	hint.autowrap_mode = TextServer.AUTOWRAP_WORD_SMART
	box.add_child(hint)
	_dx = _offset(view, box, "_site_move_dx", "Move X (m)")
	_dz = _offset(view, box, "_site_move_dz", "Move Z (m)")
	view.button(box, "Move whole site", apply)

func _offset(view: RefCounted, box: Control, key: String, title: String) -> SpinBox:
	var spin: SpinBox = view.number_field(box, key, title, 500, TaskPanels.POSITION_STEP_M)
	spin.min_value = -500
	spin.allow_lesser = true
	spin.custom_arrow_step = TaskPanels.POSITION_ARROW_STEP_M
	return spin

## Move the whole site by the fields' offset.
func apply() -> void:
	move(_dx.value, _dz.value)

## Move the whole site by `dx`, `dz` metres as one undo step; false, with the reason shown, when
## Rust refuses it.
func move(dx: float, dz: float) -> bool:
	var document = _editor._session.document
	var targets: Array[Dictionary] = []
	var result: Dictionary = document.prepare_edit("move_site", targets, {"dx": dx, "dz": dz, "part_bounds": _editor._session.part_bounds()})
	if result.has("error"):
		_editor._session.message(str(result.error))
		return false
	document.apply(result.document, "Move whole site")
	return true
