# SPDX-License-Identifier: GPL-2.0-only

## Colour scheme controls and preview projection. Rust discovers and validates source bindings;
## authored changes go through the existing document history, never into imported resources.
extends RefCounted

var _editor: Node
var _box: VBoxContainer
var _list: ItemList
var _default: Label
var _hint: Button
var _details: VBoxContainer
var _spawn: OptionButton
var _error: Label
var _state: Dictionary = {}
var _input: Dictionary = {}
var _inventories: Array = []
var _textures: Dictionary = {}
var _plans: Dictionary = {}
var _selected := ""
var _identity := ""
var _preview_revision := -1
var _validation_error := ""

func _init(editor: Node) -> void:
	_editor = editor

func build(parent: Control) -> void:
	_box = VBoxContainer.new()
	parent.add_child(_box)
	_label(_box, "Colour schemes")
	_default = _label(_box, "Source materials · no schemes")
	_hint = _button(_box, "Found colour scheme candidates — Review…", func(): _override_dialog(-1, true))
	_list = ItemList.new()
	_list.custom_minimum_size.y = 110
	_list.item_selected.connect(func(index):
		_selected = str(_list.get_item_metadata(index))
		_refresh_details()
		_apply_preview())
	_box.add_child(_list)
	var actions := HFlowContainer.new()
	_box.add_child(actions)
	_button(actions, "Discover…", func(): _override_dialog(-1, true))
	_button(actions, "Add…", _add_scheme)
	_button(actions, "Remove", _remove_scheme)
	_spawn = OptionButton.new()
	_spawn.add_item("Spawn appearance: Default only")
	_spawn.add_item("Spawn appearance: Random scheme")
	_spawn.tooltip_text = "Gameplay picks a scheme per parcel, so a street varies but one building never changes."
	_spawn.item_selected.connect(func(index):
		var next := _state.duplicate(true)
		next["params"]["appearance"]["spawn"] = "random_scheme" if index == 1 else "default_only"
		_commit(next, "Change spawn appearance"))
	_box.add_child(_spawn)
	_details = VBoxContainer.new()
	_box.add_child(_details)
	_error = _label(_box, "")

func refresh(state: Dictionary) -> void:
	_state = state
	var params: Dictionary = state.get("params", {})
	var identity := str(params.get("asset_id", ""))
	if identity != _identity:
		_identity = identity
		_selected = ""
	_box.visible = not params.get("mesh_parts", []).is_empty()
	var parts: Array = []
	for part: Dictionary in params.get("mesh_parts", []):
		parts.append({"name": part.get("name"), "lods": part.get("lods", []).map(func(lod): return lod.get("file"))})
	# Placement drags and camera changes do not invalidate material inventories or textures.
	var input := {"appearance": params.get("appearance"), "parts": parts,
		"sources": state.get("sources", []), "colours": state.get("colour_sources", {})}
	var changed := input != _input
	if changed:
		if input["sources"] != _input.get("sources"):
			_read_inventories()
		_input = input.duplicate(true)
		_textures.clear()
		_plans.clear()
		_validation_error = AssetAuthoringFiles.colour_document_error(JSON.stringify(state))
		_list.clear()
		_list.add_item("Original source materials · preview only")
		_list.set_item_metadata(0, "")
		var appearance := _appearance()
		var schemes: Array = appearance.get("schemes", [])
		_list.visible = not schemes.is_empty()
		_spawn.visible = not schemes.is_empty()
		if _selected.is_empty() or not schemes.any(func(scheme): return scheme["id"] == _selected):
			_selected = str(appearance.get("default_scheme", ""))
		_default.text = "Source materials · no schemes" if schemes.is_empty() else "Default: " + str(appearance.get("default_scheme", ""))
		for scheme: Dictionary in schemes:
			var is_default: bool = scheme["id"] == appearance.get("default_scheme")
			_list.add_item(str(scheme["name"]) + ("   · Default" if is_default else ""))
			_list.set_item_metadata(_list.item_count - 1, scheme["id"])
			if is_default:
				_default.text = "Default: " + str(scheme["name"])
		for index in _list.item_count:
			if _list.get_item_metadata(index) == _selected:
				_list.select(index)
		_spawn.disabled = schemes.is_empty()
		# Absent metadata follows the schema default, which is a varied street.
		_spawn.select(0 if appearance.get("spawn") == "default_only" else 1)
		_refresh_details()
	if changed or _preview_revision != _editor._preview.lod_import_count:
		_apply_preview(changed)

func error() -> String:
	return _validation_error

func _appearance() -> Dictionary:
	var value = _state.get("params", {}).get("appearance")
	return value if value is Dictionary else {}

func _scheme_index() -> int:
	var schemes: Array = _appearance().get("schemes", [])
	for index in schemes.size():
		if schemes[index]["id"] == _selected:
			return index
	return -1

func _read_inventories() -> void:
	_inventories.clear()
	var found := false
	for paths: Array in _state.get("sources", []):
		var tiers: Array = []
		for path in paths:
			var result: Dictionary = JSON.parse_string(AssetAuthoringPolicy.colour_materials_json(str(path)))
			tiers.append(result)
		if not tiers.is_empty():
			for material: Dictionary in tiers[0].get("materials", []):
				if material.get("albedo") != null:
					var discovery: Dictionary = JSON.parse_string(AssetAuthoringPolicy.discover_colours_json(str(material["albedo"])))
					found = found or discovery.get("candidates", []).size() > 1
		_inventories.append(tiers)
	_hint.visible = found

func _apply_preview(invalidate: bool = false) -> void:
	_preview_revision = _editor._preview.lod_import_count
	if not _plans.has(_selected):
		_plans[_selected] = JSON.parse_string(AssetAuthoringPolicy.colour_preview_json(JSON.stringify(_state), _selected))
	var result: Dictionary = _plans[_selected]
	var bindings: Array = result.get("plan", [])
	var message := str(result.get("error", _validation_error))
	for binding: Dictionary in bindings:
		for path in binding["textures"].values():
			if _textures.has(path):
				continue
			var image := Image.load_from_file(str(path))
			if image == null or image.is_empty():
				message = "Unreadable colour scheme texture: " + str(path)
				break
			image.generate_mipmaps()
			_textures[path] = ImageTexture.create_from_image(image)
	if not message.is_empty():
		bindings = []
	_editor._preview.set_colour_scheme(_selected if message.is_empty() else "", bindings, _textures, invalidate)
	_error.text = message
	_error.visible = not message.is_empty()

func _refresh_details() -> void:
	for child in _details.get_children():
		_details.remove_child(child)
		child.queue_free()
	var index := _scheme_index()
	if index < 0:
		return
	var scheme: Dictionary = _appearance()["schemes"][index]
	_label(_details, "Selected scheme: " + str(scheme["name"]))
	var actions := HFlowContainer.new()
	_details.add_child(actions)
	_button(actions, "Set as default", func():
		var next := _state.duplicate(true)
		next["params"]["appearance"]["default_scheme"] = _selected
		_commit(next, "Set default colour scheme"))
	_button(actions, "Rename…", func(): _name_dialog(index))
	for override_index in scheme.get("overrides", []).size():
		var entry: Dictionary = scheme["overrides"][override_index]
		_label(_details, "%s · %s" % [entry["part"], ", ".join(entry["materials"])])
		_label(_details, "Albedo: " + str(entry.get("albedo", "Source texture")))
		var row := HFlowContainer.new()
		_details.add_child(row)
		_button(row, "Edit textures / mapping…", _override_dialog.bind(override_index, false))
		_button(row, "Remove override", func():
			var next := _state.duplicate(true)
			next["params"]["appearance"]["schemes"][index]["overrides"].remove_at(override_index)
			_commit(next, "Remove material override"))
	_button(_details, "Add material override…", _override_dialog.bind(-1, false))
	_editor._view._apply_editor_theme(_details)

func _add_scheme() -> void:
	_name_dialog(-1)

func _name_dialog(index: int) -> void:
	var dialog := _dialog("Add colour scheme" if index < 0 else "Rename colour scheme")
	var body: VBoxContainer = dialog.get_meta("body")
	var name_edit := LineEdit.new()
	body.add_child(name_edit)
	name_edit.placeholder_text = "Display name"
	var id_edit := LineEdit.new()
	body.add_child(id_edit)
	id_edit.placeholder_text = "Stable ID: lowercase letters, digits, underscores"
	id_edit.editable = index < 0
	if index >= 0:
		name_edit.text = _appearance()["schemes"][index]["name"]
		id_edit.text = _selected
	var feedback := _label(body, "")
	dialog.confirmed.connect(func():
		var next := _state.duplicate(true)
		var appearance := _appearance().duplicate(true)
		if index < 0:
			var id := id_edit.text.strip_edges()
			var regex := RegEx.new()
			regex.compile("^[a-z0-9_]+$")
			if regex.search(id) == null or name_edit.text.strip_edges().is_empty() or appearance.get("schemes", []).any(func(s): return s["id"] == id):
				feedback.text = "Enter a unique lowercase ID and a display name."
				return
			if appearance.is_empty():
				appearance = {"default_scheme": id, "spawn": "random_scheme", "schemes": []}
			appearance["schemes"].append({"id": id, "name": name_edit.text.strip_edges(), "overrides": []})
			_selected = id
		else:
			if name_edit.text.strip_edges().is_empty():
				feedback.text = "Enter a display name."
				return
			appearance["schemes"][index]["name"] = name_edit.text.strip_edges()
		next["params"]["appearance"] = appearance
		_commit(next, "Edit colour scheme")
		dialog.hide())
	_show(dialog)

func _remove_scheme() -> void:
	var index := _scheme_index()
	if index < 0:
		return
	var next := _state.duplicate(true)
	var appearance: Dictionary = next["params"]["appearance"]
	appearance["schemes"].remove_at(index)
	if appearance["schemes"].is_empty():
		next["params"]["appearance"] = null
	elif appearance["default_scheme"] == _selected:
		appearance["default_scheme"] = appearance["schemes"][0]["id"]
	_selected = ""
	_commit(next, "Remove colour scheme")

func _override_dialog(index: int, discovery: bool) -> void:
	if _inventories.is_empty() or (not discovery and _scheme_index() < 0):
		return
	var dialog := _dialog("Discover colour schemes" if discovery else "Material texture override")
	var body: VBoxContainer = dialog.get_meta("body")
	_label(body, "Confirm which source material this scheme recolours in each LOD. Unambiguous tiers are filled in already. Shared texture channels remain unchanged.")
	var part_picker := OptionButton.new()
	body.add_child(part_picker)
	var parts: Array = _state["params"].get("mesh_parts", [])
	for part: Dictionary in parts:
		part_picker.add_item(str(part["name"]))
	var previous: Dictionary = {}
	if index >= 0:
		previous = _appearance()["schemes"][_scheme_index()]["overrides"][index]
		for p in parts.size():
			if parts[p]["name"] == previous["part"]:
				part_picker.select(p)
	var mappings := VBoxContainer.new()
	body.add_child(mappings)
	var choices: Array[OptionButton] = []
	var candidates: Array = []
	var checks: Array[CheckBox] = []
	var candidates_box := VBoxContainer.new()
	var selected_paths: Dictionary = {}
	# Lambdas capture locals by value, so the chosen scan folder lives in a shared dictionary.
	var scan: Dictionary = {"folder": ""}
	var feedback := _label(body, "")
	var populate_candidates := func():
		for child in candidates_box.get_children():
			candidates_box.remove_child(child)
			child.queue_free()
		candidates.clear()
		checks.clear()
		if choices.is_empty() or choices[0].selected <= 0:
			return
		var materials: Array = _inventories[part_picker.selected][0].get("materials", [])
		var material: Dictionary = materials[choices[0].selected - 1]
		if material.get("albedo") == null:
			feedback.text = "No external albedo to discover. Use Add and Browse for manual textures."
			return
		# Publication copies only the albedo the model referenced, so an asset reopened from
		# the library cannot see its own alternatives; the modelling folder still holds them.
		var folder := str(scan["folder"])
		var result: Dictionary = JSON.parse_string(
			AssetAuthoringPolicy.discover_colours_in_json(str(material["albedo"]), folder) if not folder.is_empty()
			else AssetAuthoringPolicy.discover_colours_json(str(material["albedo"])))
		candidates.assign(result.get("candidates", []))
		var scanned := folder if not folder.is_empty() else str(material["albedo"]).get_base_dir()
		if result.has("error"):
			feedback.text = str(result["error"])
		elif candidates.size() < 2:
			feedback.text = "No colour alternatives beside %s. Use Scan another folder… to reach the modelling export." % scanned.get_file()
		else:
			feedback.text = "%d candidates in %s. Confirmation adds schemes or replaces matching material overrides." % [candidates.size(), scanned.get_file()]
		for candidate: Dictionary in candidates:
			var check := CheckBox.new()
			check.text = "%s — %s" % [str(candidate["id"]).capitalize(), str(candidate["path"]).get_file()]
			check.button_pressed = true
			candidates_box.add_child(check)
			checks.append(check)
		# These arrive after the dialog was themed, so they need the palette applied again.
		_editor._view._apply_editor_theme(candidates_box)
	# An unambiguous tier is not a decision to put to the author: take a sole material, then
	# match LOD0's name in the remaining tiers. Only genuinely ambiguous tiers stay unset, so
	# the explicit per-LOD mapping the document stores is unchanged.
	var auto_fill := func():
		for lod in choices.size():
			var picker: OptionButton = choices[lod]
			if picker.selected > 0:
				continue
			if picker.item_count == 2:
				picker.select(1)
			elif lod > 0 and choices[0].selected > 0:
				var target = choices[0].get_item_metadata(choices[0].selected)
				for item in range(1, picker.item_count):
					if picker.get_item_metadata(item) == target:
						picker.select(item)
	var populate := func(_part_index: int):
		for child in mappings.get_children():
			mappings.remove_child(child)
			child.queue_free()
		choices.clear()
		var tiers: Array = _inventories[part_picker.selected]
		for lod in tiers.size():
			_label(mappings, "LOD%d material" % lod)
			var picker := OptionButton.new()
			picker.add_item("Choose material…")
			picker.set_item_metadata(0, "")
			var names: Array = previous.get("materials", []) if previous.get("part") == parts[part_picker.selected]["name"] else []
			for material: Dictionary in tiers[lod].get("materials", []):
				picker.add_item(str(material["name"]))
				picker.set_item_metadata(picker.item_count - 1, material["name"])
				if lod < names.size() and names[lod] == material["name"]:
					picker.select(picker.item_count - 1)
			mappings.add_child(picker)
			choices.append(picker)
			if tiers[lod].has("error"):
				_label(mappings, str(tiers[lod]["error"]))
		if not choices.is_empty():
			choices[0].item_selected.connect(func(_selected_index):
				auto_fill.call()
				if discovery:
					populate_candidates.call())
		auto_fill.call()
		if discovery and not choices.is_empty():
			populate_candidates.call()
		# Pickers rebuilt on a part change also postdate the dialog's own theming pass.
		_editor._view._apply_editor_theme(mappings)
	part_picker.item_selected.connect(populate)
	populate.call(part_picker.selected)
	if discovery:
		body.add_child(candidates_box)
		_button(body, "Scan another folder…", func(): _browse_folder(func(folder):
			scan["folder"] = folder
			populate_candidates.call(), dialog))
	else:
		candidates_box.free()
		for channel in ["albedo", "orm", "normal", "emission"]:
			var row := HFlowContainer.new()
			body.add_child(row)
			var current := str(previous.get(channel, ""))
			var path_label := _label(row, channel.capitalize() + ": " + (current.get_file() if not current.is_empty() else "Source"), false)
			selected_paths[channel] = str(_state.get("colour_sources", {}).get(previous.get(channel, ""), ""))
			_button(row, "Browse…", _choose_texture.bind(selected_paths, channel, path_label, dialog))
			_button(row, "Use source", func():
				selected_paths[channel] = ""
				path_label.text = channel.capitalize() + ": Source")
	dialog.confirmed.connect(func():
		var names: Array = []
		for lod in choices.size():
			var picker := choices[lod]
			var material := str(picker.get_item_metadata(picker.selected))
			var error := AssetAuthoringPolicy.colour_material_error(material, JSON.stringify(_inventories[part_picker.selected][lod].get("materials", [])))
			if not error.is_empty():
				feedback.text = "LOD%d: %s" % [lod, error]
				return
			names.append(material)
		var next := _state.duplicate(true)
		var appearance := _appearance().duplicate(true)
		var entry := {"part": parts[part_picker.selected]["name"], "materials": names}
		if discovery:
			var added := false
			var first_import := appearance.is_empty()
			var source_material: Dictionary = _inventories[part_picker.selected][0]["materials"][choices[0].selected - 1]
			for c in candidates.size():
				if not checks[c].button_pressed:
					continue
				var candidate: Dictionary = candidates[c]
				var id := str(candidate["id"])
				if appearance.is_empty():
					appearance = {"default_scheme": id, "spawn": "random_scheme", "schemes": []}
				# Compare names, not paths: a scanned folder holds the same colour elsewhere.
				if first_import and str(candidate["path"]).get_file() == str(source_material.get("albedo", "")).get_file():
					appearance["default_scheme"] = id
				var override := entry.duplicate(true)
				override["albedo"] = _store_texture(next, str(candidate["path"]))
				var existing := -1
				for s in appearance["schemes"].size():
					if appearance["schemes"][s]["id"] == id:
						existing = s
				if existing < 0:
					appearance["schemes"].append({"id": id, "name": id.capitalize(), "overrides": [override]})
				else:
					var overrides: Array = appearance["schemes"][existing]["overrides"]
					var replaced := false
					for target in overrides.size():
						if overrides[target]["part"] == entry["part"] and overrides[target]["materials"] == names:
							overrides[target] = override
							replaced = true
					if not replaced:
						overrides.append(override)
				added = true
			if not added:
				feedback.text = "Select at least one candidate."
				return
		else:
			for channel: String in selected_paths:
				if not str(selected_paths[channel]).is_empty():
					entry[channel] = _store_texture(next, selected_paths[channel])
			var overrides: Array = appearance["schemes"][_scheme_index()]["overrides"]
			if index < 0:
				overrides.append(entry)
			else:
				overrides[index] = entry
		next["params"]["appearance"] = appearance
		var error := AssetAuthoringFiles.colour_document_error(JSON.stringify(next))
		if not error.is_empty():
			feedback.text = error
			return
		_commit(next, "Discover colour schemes" if discovery else "Edit material override")
		dialog.hide())
	_show(dialog)

func _store_texture(state: Dictionary, path: String) -> String:
	var sources: Dictionary = state.get("colour_sources", {})
	for relative: String in sources:
		if sources[relative] == path:
			return relative
	var relative := "colours/" + path.get_file()
	var suffix := 1
	while sources.has(relative):
		relative = "colours/%d_%s" % [suffix, path.get_file()]
		suffix += 1
	sources[relative] = path
	state["colour_sources"] = sources
	return relative

func _commit(state: Dictionary, label: String) -> void:
	_editor._session.document.apply(state, label)

func _choose_texture(paths: Dictionary, channel: String, label: Label, owner: Window) -> void:
	_browse(func(path):
		paths[channel] = path
		label.text = channel.capitalize() + ": " + str(path).get_file(), owner)

func _browse_folder(selected: Callable, owner: Window) -> void:
	var dialog := FileDialog.new()
	dialog.access = FileDialog.ACCESS_FILESYSTEM
	dialog.file_mode = FileDialog.FILE_MODE_OPEN_DIR
	dialog.dir_selected.connect(selected)
	owner.add_child(dialog)
	dialog.visibility_changed.connect(func():
		if not dialog.visible:
			dialog.queue_free())
	_editor._view._apply_editor_theme(dialog)
	dialog.popup_centered(Vector2i(800, 560))

func _browse(selected: Callable, owner: Window) -> void:
	var dialog := FileDialog.new()
	dialog.access = FileDialog.ACCESS_FILESYSTEM
	dialog.file_mode = FileDialog.FILE_MODE_OPEN_FILE
	dialog.filters = PackedStringArray(["*.png, *.jpg, *.jpeg, *.webp, *.tga; Textures"])
	dialog.file_selected.connect(selected)
	owner.add_child(dialog)
	dialog.visibility_changed.connect(func():
		if not dialog.visible:
			dialog.queue_free())
	_editor._view._apply_editor_theme(dialog)
	dialog.popup_centered(Vector2i(800, 560))

func _dialog(title: String) -> ConfirmationDialog:
	var dialog := ConfirmationDialog.new()
	dialog.title = title
	dialog.dialog_hide_on_ok = false
	var scroll := ScrollContainer.new()
	scroll.custom_minimum_size = Vector2(520, 320)
	dialog.add_child(scroll)
	var body := VBoxContainer.new()
	body.size_flags_horizontal = Control.SIZE_EXPAND_FILL
	scroll.add_child(body)
	dialog.set_meta("body", body)
	_editor.add_child(dialog)
	dialog.visibility_changed.connect(func():
		if not dialog.visible:
			dialog.queue_free())
	return dialog

func _show(dialog: ConfirmationDialog) -> void:
	_editor._view._apply_editor_theme(dialog)
	dialog.popup_centered(Vector2i(600, 540))

# Wrapping suits a full-width column. In a flow row a wrapping label has no width floor and
# collapses to one character per line, which then stretches its neighbouring buttons.
func _label(parent: Node, text: String, wrap: bool = true) -> Label:
	var label := Label.new()
	label.text = text
	if wrap:
		label.autowrap_mode = TextServer.AUTOWRAP_WORD_SMART
	parent.add_child(label)
	return label

func _button(parent: Node, text: String, action: Callable) -> Button:
	return _editor._view.button(parent, text, action)
