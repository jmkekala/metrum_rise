# SPDX-License-Identifier: GPL-2.0-only

## Short type-first creation dialog. Cancellation never touches the active document.
## Type/subtype choices come from Rust; this view owns only input and presentation.
extends ConfirmationDialog

const Spacing = preload("res://scripts/editors/asset_editor/editor_spacing.gd")
const MeshImportDialog = preload("res://scripts/editors/mesh_import_dialog.gd")

signal create_requested(selection: Dictionary)
signal create_pack_requested

var _type: OptionButton
var _subtype: OptionButton
var _subtype_row: VBoxContainer
var _name: LineEdit
var _pack: OptionButton
var _model: LineEdit
var _error: Label
var _body: VBoxContainer
var theme_mode := "dark"

func _init() -> void:
	title = "What are you creating?"
	get_ok_button().text = "Create asset"
	dialog_hide_on_ok = false
	min_size = Vector2i(520, 440)
	var scroll := ScrollContainer.new()
	scroll.custom_minimum_size = Vector2(560, 420)
	scroll.horizontal_scroll_mode = ScrollContainer.SCROLL_MODE_DISABLED
	add_child(scroll)
	var margin := MarginContainer.new()
	margin.size_flags_horizontal = Control.SIZE_EXPAND_FILL
	Spacing.pad_container(margin)
	scroll.add_child(margin)
	_body = VBoxContainer.new()
	margin.add_child(_body)
	_label(_body, "Asset type")
	_type = OptionButton.new()
	_type.name = "AssetType"
	_body.add_child(_type)
	_type.item_selected.connect(func(_index): _update_type())
	_subtype_row = VBoxContainer.new()
	_body.add_child(_subtype_row)
	_label(_subtype_row, "Service subtype")
	_subtype = OptionButton.new()
	_subtype.name = "ServiceSubtype"
	_subtype_row.add_child(_subtype)
	_label(_body, "Name")
	_name = LineEdit.new()
	_name.name = "AssetName"
	_name.placeholder_text = "Name your asset"
	_body.add_child(_name)
	_label(_body, "Destination pack")
	var pack_row := HBoxContainer.new()
	_body.add_child(pack_row)
	_pack = OptionButton.new()
	_pack.name = "DestinationPack"
	_pack.fit_to_longest_item = false
	_pack.size_flags_horizontal = Control.SIZE_EXPAND_FILL
	pack_row.add_child(_pack)
	var new_pack := Button.new()
	new_pack.text = "New pack…"
	new_pack.pressed.connect(func(): create_pack_requested.emit())
	pack_row.add_child(new_pack)
	_label(_body, "Model (optional — you can import it later)")
	var model_row := HBoxContainer.new()
	_body.add_child(model_row)
	_model = LineEdit.new()
	_model.name = "OptionalModel"
	_model.placeholder_text = "Choose a GLB, GLTF or FBX model"
	_model.size_flags_horizontal = Control.SIZE_EXPAND_FILL
	model_row.add_child(_model)
	var browse := Button.new()
	browse.text = "Browse…"
	model_row.add_child(browse)
	browse.pressed.connect(_browse_model)
	_error = _label(_body, "")
	_error.name = "CreationError"
	confirmed.connect(_confirm)
	Spacing.apply_to_tree(self)

func configure(catalog: Dictionary, packs: Array, selected_pack: String = "") -> void:
	_type.clear()
	for item: Dictionary in catalog.get("types", []):
		_type.add_item(str(item["label"]))
		_type.set_item_metadata(_type.item_count - 1, item["id"])
	_subtype.clear()
	for item: Dictionary in catalog.get("services", []):
		_subtype.add_item(str(item["label"]))
		_subtype.set_item_metadata(_subtype.item_count - 1, item["id"])
	set_packs(packs, selected_pack)
	_name.text = ""
	_model.text = ""
	_error.text = ""
	_update_type()

func set_packs(packs: Array, selected_pack: String) -> void:
	_pack.clear()
	for pack: Dictionary in packs:
		var id := str(pack.get("pack_id", ""))
		_pack.add_item("%s (%s)" % [str(pack.get("display_name", id)), id])
		_pack.set_item_metadata(_pack.item_count - 1, pack.duplicate(true))
		if id == selected_pack:
			_pack.select(_pack.item_count - 1)

func select_type(id: String) -> void:
	for index in _type.item_count:
		if _type.get_item_metadata(index) == id:
			_type.select(index)
			_update_type()
			return

func selection() -> Dictionary:
	return {
		"type": _selected_id(_type), "subtype": _selected_id(_subtype) if _subtype_row.visible else "",
		"name": _name.text.strip_edges(), "model": _model.text.strip_edges(),
		"pack": _pack.get_item_metadata(_pack.selected).duplicate(true) if _pack.selected >= 0 else {},
	}

func _confirm() -> void:
	var chosen := selection()
	if str(chosen["type"]).is_empty():
		_error.text = "Choose an asset type."
	elif str(chosen["name"]).is_empty():
		_error.text = "Give the asset a name."
		_name.grab_focus()
	elif chosen["pack"].is_empty():
		_error.text = "Choose or create a destination pack."
	elif not str(chosen["model"]).is_empty() and not FileAccess.file_exists(chosen["model"]):
		_error.text = "The selected model file is missing."
	elif not str(chosen["model"]).is_empty() and not str(chosen["model"]).get_extension().to_lower() in MeshImportDialog.SUPPORTED_EXTENSIONS:
		_error.text = "Choose a GLB, GLTF or FBX model."
	else:
		_error.text = ""
		create_requested.emit(chosen)
		hide()

func _browse_model() -> void:
	var picker := MeshImportDialog.new()
	picker.theme_mode = theme_mode
	picker.mesh_selected.connect(func(path): _model.text = path)
	add_child(picker)
	picker.open(_model.text.get_base_dir())

func _update_type() -> void:
	_subtype_row.visible = _selected_id(_type) == "service"
	reset_size()

func _selected_id(button: OptionButton) -> String:
	return str(button.get_item_metadata(button.selected)) if button.selected >= 0 else ""

func _label(parent: Node, value: String) -> Label:
	var label := Label.new()
	label.text = value
	label.autowrap_mode = TextServer.AUTOWRAP_WORD_SMART
	parent.add_child(label)
	return label
