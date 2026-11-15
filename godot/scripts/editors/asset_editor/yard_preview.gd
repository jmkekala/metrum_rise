# SPDX-License-Identifier: GPL-2.0-only

## Temporary game vegetation view. Rust plans the lot; this controller only renders placements
## and restores the exact editing visibility. No preview state enters the authoring document.
extends RefCounted

const TreeSpecies = preload("res://scripts/renderers/tree_species.gd")
const VegetationPlacements = preload("res://scripts/renderers/vegetation_placements.gd")

var active := false
var placements := PackedFloat32Array()
var _seed := 0
var _editor: Node
var _root: Node3D
var _state: Dictionary = {}
var _visibility: Dictionary = {}
var _meshes: Array = []

func _init(editor: Node) -> void:
	_editor = editor

func reroll() -> void:
	if not _editor._session.has_document:
		return
	if not active:
		_editor._selection.cancel()
		_editor._menus.actions.cancel()
		_state = _editor._preview.begin_yard_preview()
		var view = _editor._view
		for control: Control in [view.picking_overlay, view.hover_label,
			view._selection_rect_overlay, view.scale_reference_button,
			view.selection_filter, view.comparison_button]:
			_visibility[control] = control.visible
			control.hide()
		active = true
	_seed += 1
	placements = _editor._session.policy.yard_preview_plants(
		JSON.stringify(_editor._session.params), _editor._session.yard_structure_bounds(), _seed)
	# Rust plans a lot at height zero; the editor's lot plane sits just below it.
	for i in range(1, placements.size(), 6):
		placements[i] = _editor._preview.LOT_PLANE_Y
	_render()
	_editor._view.yard_preview_button.text = "Reroll"
	_editor._view.exit_yard_preview_button.show()

func exit() -> void:
	if not active:
		return
	active = false
	_clear_instances()
	_editor._preview.end_yard_preview(_state)
	for control: Control in _visibility:
		control.visible = _visibility[control]
	_visibility.clear()
	_state.clear()
	placements = PackedFloat32Array()
	_editor._view.yard_preview_button.text = "Preview"
	_editor._view.exit_yard_preview_button.hide()

func _clear_instances() -> void:
	if is_instance_valid(_root):
		_root.free()
	_root = null

func _render() -> void:
	_clear_instances()
	if _meshes.is_empty():
		_meshes = TreeSpecies.build_meshes()
	# The game's cosmetic seed, variant, proportions and tint, as well as its meshes.
	_root = VegetationPlacements.instances(placements, _meshes)
	_root.name = "YardVegetationPreview"
	_editor._preview.add_child(_root)
