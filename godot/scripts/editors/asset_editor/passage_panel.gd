# SPDX-License-Identifier: GPL-3.0-or-later

## The selected mesh part's passages (`PARK-19`): openings a vehicle drives into or through,
## listed, added, moved, sized and deleted through the document's "passage" command, so each
## change is one undo step. Rust validates them and cuts them from the part's walls for the
## lot check; this panel only edits and lists them.
extends RefCounted

# Part-local directions a passage may open toward; turned by the part's rotation, each runs
# along a lot axis when the part is turned by a quarter turn.
const DIRECTIONS := {"-Z": [0.0, 0.0, -1.0], "+Z": [0.0, 0.0, 1.0], "-X": [-1.0, 0.0, 0.0], "+X": [1.0, 0.0, 0.0]}

var selected := -1
var list: ItemList
var _editor: Node
var _body: VBoxContainer
var _name: LineEdit
var _x: SpinBox
var _z: SpinBox
var _direction: OptionButton
var _width: SpinBox
var _length: SpinBox
var _height: SpinBox
var _through: CheckBox
var _updating := false
var _part := -1

func build(view: RefCounted, parent: Control) -> void:
	_editor = view._editor
	var box: VBoxContainer = view.advanced(parent, "passages", "Passages")
	var hint := Label.new()
	hint.text = "A lane a vehicle drives into or through this part. Its rectangle opens the part's walls for trucks when its clear height is 4 m or more, for cars from 2 m."
	hint.autowrap_mode = TextServer.AUTOWRAP_WORD_SMART
	box.add_child(hint)
	var actions := HFlowContainer.new()
	box.add_child(actions)
	view.button(actions, "Add passage", add)
	view.button(actions, "Delete passage", delete)
	list = ItemList.new()
	list.custom_minimum_size.y = 56
	list.item_selected.connect(_on_selected)
	box.add_child(list)
	_body = VBoxContainer.new()
	box.add_child(_body)
	_name = view.text_field(_body, "_passage_name", "Name")
	_name.text_submitted.connect(func(text): _set_field("name", text))
	_name.focus_exited.connect(func(): _set_field("name", _name.text))
	_x = _number(view, "_passage_x", "Mouth X (m, part-local)", -500, 500, 0.05, "position")
	_z = _number(view, "_passage_z", "Mouth Z (m, part-local)", -500, 500, 0.05, "position")
	_direction = view.choice_field(_body, "_passage_forward", "Opens toward (part-local)", DIRECTIONS.keys())
	_direction.item_selected.connect(func(index): _set_field("forward", DIRECTIONS[_direction.get_item_metadata(index)]))
	_width = _number(view, "_passage_width", "Clear width (m)", 0.1, 100, 0.1, "width_m")
	_length = _number(view, "_passage_length", "Depth from the mouth (m)", 0.1, 200, 0.1, "length_m")
	_height = _number(view, "_passage_height", "Clear height (m)", 0.1, 50, 0.1, "clear_height_m")
	_through = CheckBox.new()
	_through.text = "Open at the far end too"
	_through.toggled.connect(func(on): _set_field("through", on))
	_body.add_child(_through)
	_body.visible = false

func _number(view: RefCounted, key: String, title: String, minimum: float, maximum: float, step: float, field: String) -> SpinBox:
	var spin: SpinBox = view.number_field(_body, key, title, maximum, step)
	spin.min_value = minimum
	spin.allow_lesser = minimum < 0.0
	spin.value_changed.connect(func(_value):
		_set_field(field, [_x.value, 0.0, _z.value] if field == "position" else spin.value))
	return spin

func _on_selected(index: int) -> void:
	selected = index
	refresh()

## The passages of the selected part, as the document holds them.
func passages() -> Array:
	var part: int = _editor._selected_part_index
	var parts: Array = _editor._session.document.snapshot().get("params", {}).get("mesh_parts", [])
	if part < 0 or part >= parts.size():
		return []
	return parts[part].get("passages", [])

## Fill the list and the selected passage's fields from the document.
func refresh() -> void:
	if list == null:
		return
	var entries := passages()
	# Another part's list starts with no passage selected.
	if _part != _editor._selected_part_index:
		_part = _editor._selected_part_index
		selected = -1
	_updating = true
	list.clear()
	for entry: Dictionary in entries:
		list.add_item(str(entry.get("name", "")))
	selected = clampi(selected, -1, entries.size() - 1)
	_body.visible = selected >= 0
	if selected >= 0:
		list.select(selected)
		var entry: Dictionary = entries[selected]
		var position: Array = entry.get("position", [0.0, 0.0, 0.0])
		var forward: Array = entry.get("forward", [0.0, 0.0, -1.0])
		if not _name.has_focus():
			_name.text = str(entry.get("name", ""))
		_x.value = float(position[0])
		_z.value = float(position[2])
		for index in _direction.item_count:
			var axis: Array = DIRECTIONS[_direction.get_item_metadata(index)]
			if Vector3(axis[0], axis[1], axis[2]).is_equal_approx(Vector3(float(forward[0]), float(forward[1]), float(forward[2]))):
				_direction.select(index)
		_width.value = float(entry.get("width_m", 5.0))
		_length.value = float(entry.get("length_m", 8.0))
		_height.value = float(entry.get("clear_height_m", 4.5))
		_through.button_pressed = bool(entry.get("through", false))
	_updating = false

## Add a passage at the selected part's front face, opening into it along -Z.
func add() -> void:
	var part: int = _editor._selected_part_index
	if part < 0:
		_editor._session.message("Select a mesh part first")
		return
	var box: AABB = _editor._parts[part].aabb
	selected = passages().size()
	if not _edit({"op": "add", "position": [box.get_center().x, 0.0, box.end.z]}, "Add passage"):
		selected = -1

func delete() -> void:
	if selected >= 0 and _edit({"op": "delete", "passage": selected}, "Delete passage"):
		selected -= 1
		refresh()

func _set_field(field: String, value: Variant) -> void:
	if _updating or selected < 0:
		return
	_edit({"op": "set", "passage": selected, "field": field, "value": value}, "Edit passage")

func _edit(args: Dictionary, label: String) -> bool:
	var targets: Array[Dictionary] = [{"kind": "mesh", "index": _editor._selected_part_index}]
	var document = _editor._session.document
	var result: Dictionary = document.prepare_edit("passage", targets, args)
	if result.has("error"):
		_editor._session.message(str(result.error))
		refresh()
		return false
	document.apply(result.document, label)
	return true
