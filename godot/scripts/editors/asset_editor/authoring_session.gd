# SPDX-License-Identifier: GPL-2.0-only

## Asset-authoring workflow: documents, commands, dialogs, validation and publication.
## Rust owns applicability and gameplay contracts. The view and preview are document projections.
extends RefCounted

const Adapter = preload("res://scripts/editors/asset_editor/document_adapter.gd")
const CreationDialog = preload("res://scripts/editors/asset_editor/creation_dialog.gd")
const MeshImportDialog = preload("res://scripts/editors/mesh_import_dialog.gd")
const PreviewGeometry = preload("res://scripts/editors/asset_editor/preview_geometry.gd")
const ColourSchemes = preload("res://scripts/editors/asset_editor/colour_schemes.gd")
const ThumbnailCapture = preload("res://scripts/editors/asset_editor/thumbnail_capture.gd")

var document := AssetAuthoringDocument.new()
var policy := AssetAuthoringPolicy.new()
var params: Dictionary = {}
var descriptor: Dictionary = {}
var rendering := false
var ready := false
var has_document := false
var colours: RefCounted
var thumbnails: RefCounted
var _editor: Node
var _adapter: RefCounted
var _capturing := false
var _creation: ConfirmationDialog
var _guard: ConfirmationDialog
var _after_save: Callable
var _catalog_error := ""
var _issues: Array = []

func _init(editor: Node) -> void:
	_editor = editor
	_adapter = Adapter.new(editor)
	colours = ColourSchemes.new(editor)
	thumbnails = ThumbnailCapture.new(editor)

func initialize() -> void:
	_catalog_error = policy.reload_catalog()
	var view = _editor._view
	for key: String in view.fields:
		var control: Control = view.fields[key]
		if not key.begins_with("_"):
			if control is LineEdit:
				control.text_changed.connect(func(value): set_field(key, value))
			elif control is SpinBox:
				control.value_changed.connect(func(value): set_field(key, value))
			elif control is OptionButton:
				control.item_selected.connect(func(index): set_field(key, control.get_item_metadata(index)))
		var focus_control: Control = control.get_line_edit() if control is SpinBox else control
		if focus_control is LineEdit:
			focus_control.focus_entered.connect(func(): document.begin_transaction("Edit " + key.trim_prefix("_")))
			focus_control.focus_exited.connect(document.commit_transaction)
	document.changed.connect(_document_changed)
	ready = true
	_editor.get_tree().auto_accept_quit = false
	document.reset({})

func refresh_catalog() -> void:
	_catalog_error = policy.reload_catalog()
	if ready:
		_document_changed()

func set_field(key: String, value: Variant) -> void:
	if rendering or not ready or not has_document:
		return
	if key == "tags":
		var tags: Array = []
		for tag in str(value).split(",", false):
			if not tag.strip_edges().is_empty():
				tags.append(tag.strip_edges())
		value = tags
	elif key in ["asset_set", "economy_profile"] and str(value).is_empty():
		value = null
	document.set_parameter(key, value, "Edit " + key)

func _document_changed() -> void:
	if not ready:
		return
	rendering = true
	var state := document.snapshot()
	params = state.get("params", {})
	has_document = not state.is_empty()
	_editor._view.set_document_open(has_document)
	if not has_document:
		colours.refresh(state)
		descriptor = {}
		_issues.clear()
		_editor.get_window().title = "Asset editor — Metrum Rise"
		rendering = false
		return
	descriptor = JSON.parse_string(policy.inspect_json(JSON.stringify(params)))["descriptor"]
	var view = _editor._view
	for key: String in view.fields:
		if key.begins_with("_"):
			continue
		var control: Control = view.fields[key]
		var value = params.get(key)
		control.set_block_signals(true)
		if control is LineEdit:
			var display := ", ".join(value.map(func(tag): return str(tag))) if key == "tags" and value is Array else str(value if value != null else "")
			if control.text != display:
				control.text = display
		elif control is SpinBox:
			control.value = PreviewGeometry.number(params, key, 0.0)
		elif control is OptionButton:
			_update_choices(key, control, value)
		elif control is Label and key == "service_class":
			control.text = str(value if value != null else "")
		control.set_block_signals(false)
		if key not in ["asset_id", "display_name", "tags"]:
			view.rows[key].visible = descriptor.get("fields", []).has(key)
	for key in ["footprint", "gameplay"]:
		var group: Dictionary = view.advanced_groups[key]
		var applicable := false
		for child in group["body"].get_children():
			applicable = applicable or child.visible
		group["toggle"].visible = applicable
		group["body"].visible = applicable and group["toggle"].button_pressed
	view.type_label.text = "%s\n%s" % [str(descriptor.get("kind", "unsupported")).capitalize(), str(descriptor.get("placement", ""))]
	if str(params.get("asset_id", "")).is_empty():
		view.type_label.text = "Create or open an asset to begin."
	_editor._update_pack_summary()
	view.derived_workers.text = ""
	var selected = descriptor.get("selected_profile")
	if selected is Dictionary and selected.get("compatible", false):
		var profile: Dictionary = selected["profile"]
		view.derived_workers.text = "Employment: %s · profile-derived, read-only" % str(profile["worker_capacity"])
		if descriptor.get("kind") in ["farm", "extractor"]:
			view.derived_workers.text += "\n%.2f workers / hectare; runtime scales to the committed area." % float(profile["workers_per_hectare"])
	view.derived_workers.visible = not view.derived_workers.text.is_empty()
	view.rows["flat_size_m2"].get_child(0).text = "Farmhouse area (m²)" if descriptor.get("kind") == "farm" else "Apartment area (m²)"
	view._economy_profile_status_lbl.text = _catalog_error if not _catalog_error.is_empty() and descriptor.get("fields", []).has("economy_profile") else str(descriptor.get("service_note", ""))
	view._economy_profile_status_lbl.visible = not view._economy_profile_status_lbl.text.is_empty()
	view.catalog_refresh.visible = descriptor.get("fields", []).has("economy_profile")
	var thumbnail_source := str(state.get("thumbnail_source", ""))
	if thumbnail_source != thumbnails.path():
		thumbnails.load_image(thumbnail_source)
	if not _capturing:
		_adapter.render(state)
	colours.refresh(state)
	selection_changed()
	view.undo_button.disabled = not document.can_undo()
	view.redo_button.disabled = not document.can_redo()
	view.undo_button.tooltip_text = document.undo_label()
	view.redo_button.tooltip_text = document.redo_label()
	_editor.get_window().title = "%s%s — Asset editor" % ["* " if document.is_dirty() else "", str(params.get("display_name", ""))]
	rendering = false
	validate()

func _update_choices(key: String, control: OptionButton, value: Variant) -> void:
	control.clear()
	var selected := str(value if value != null else "")
	var choices: Array = []
	if key == "density":
		for density in _editor._density_types_by_zone.get(str(params.get("zone_type", "")), []):
			choices.append({"id": density, "label": str(density).capitalize()})
	elif key == "economy_profile":
		choices.append({"id": "", "label": "Unassigned"})
		for profile: Dictionary in descriptor.get("profiles", []):
			choices.append({"id": profile["id"], "label": profile["id"]})
	var found := false
	for choice: Dictionary in choices:
		control.add_item(str(choice["label"]))
		var index := control.item_count - 1
		control.set_item_metadata(index, choice["id"])
		if str(choice["id"]) == selected:
			control.select(index)
			found = true
	if not found and not selected.is_empty():
		control.add_item("Preserved — " + selected)
		control.set_item_metadata(control.item_count - 1, selected)
		control.set_item_disabled(control.item_count - 1, true)
		control.select(control.item_count - 1)

func capture_geometry(label: String = "Edit geometry") -> void:
	if not ready or not has_document or rendering or _editor._updating_site_anchor_controls or _editor._updating_site_surface_controls or _editor._suppress_part_transform_changed:
		return
	_capturing = true
	document.apply(_adapter.capture(document.snapshot()), label)
	_capturing = false

func relink_selected_part(lod_index: int = 0) -> void:
	var index: int = _editor._selected_part_index
	if index < 0:
		return
	var part = _editor._parts[index]
	var revision: int = _editor._menus.generation
	var dialog := MeshImportDialog.new()
	dialog.theme_mode = _editor._view._theme_mode
	dialog.mesh_selected.connect(func(path):
		var current_index: int = _editor._parts.find(part)
		if revision != _editor._menus.generation or current_index < 0:
			return
		_editor._last_glb_dir = path.get_base_dir()
		_editor._save_config()
		replace_part_source(current_index, path, lod_index)
		_editor._select_mesh_part(current_index)
		_editor._on_preview_lod_selected(lod_index)
	)
	_editor.add_child(dialog)
	dialog.title = "Replace LOD%d" % lod_index
	dialog.open(_editor._last_glb_dir)

func replace_part_source(index: int, path: String, lod_index: int = 0) -> void:
	var next := document.snapshot()
	var entry: Dictionary = next["params"]["mesh_parts"][index]
	if entry.get("lods", []).is_empty():
		entry["lods"] = [{"file": path.get_file(), "distance_min_m": 0.0}]
	else:
		entry["lods"][lod_index]["file"] = path.get_file()
	if not next.has("sources"):
		next["sources"] = []
	while next["sources"].size() <= index:
		next["sources"].append([])
	while next["sources"][index].size() <= lod_index:
		next["sources"][index].append("")
	next["sources"][index][lod_index] = path
	_adapter.forget_sources()
	document.apply(next, "Replace LOD%d source" % lod_index)

func selection_changed() -> void:
	var view = _editor._view
	view.update_context_actions()
	if view.part_properties == null:
		return
	view.part_properties.visible = _editor._has_selected_mesh_part()
	view.anchor_properties.visible = _editor._selected_site_anchor_index >= 0
	view.surface_properties.visible = _editor._selected_site_surface_index >= 0
	if view.part_properties.visible or view.anchor_properties.visible or view.surface_properties.visible:
		_editor._preview.set_scale_reference_selected(false)
	var anchor: Dictionary = _editor._site_anchors_data[_editor._selected_site_anchor_index] if view.anchor_properties.visible else {}
	var kind := str(anchor.get("anchor_type", ""))
	for key in ["_anchor_vehicle", "_anchor_width"]:
		view.rows[key].visible = kind in ["driveway", "parking", "loading_bay"]
	view.rows["_anchor_length"].visible = kind in ["parking", "loading_bay"]
	if not rendering and not _editor._selection.applying_selection and not _editor._selection.pressed:
		if view.anchor_properties.visible:
			view.show_task("site")
			view.show_site(1)
		elif view.surface_properties.visible:
			view.show_task("site")
			view.show_site(2)
		elif view.part_properties.visible:
			view.show_task("model")

func new_asset_dialog(pack_id: String = "", type_id: String = "") -> void:
	guard(func():
		_creation = CreationDialog.new()
		_creation.theme_mode = _editor._view._theme_mode
		_editor.add_child(_creation)
		_release_on_hide(_creation)
		_creation.configure(JSON.parse_string(policy.types_json()), _editor._known_packs, pack_id if not pack_id.is_empty() else str(params.get("pack_id", "")))
		_creation.select_type(type_id)
		_creation.create_requested.connect(create_asset)
		_creation.create_pack_requested.connect(_editor._open_new_pack_dialog)
		_editor._view._apply_editor_theme(_creation)
		_creation.popup_centered()
	)

func create_asset(selection: Dictionary) -> void:
	var result: Dictionary = JSON.parse_string(policy.conversion_json("{}", str(selection["type"]), str(selection.get("subtype", ""))))
	if result.has("error"):
		message(str(result["error"]))
		return
	var data: Dictionary = result["document"]
	var pack: Dictionary = selection["pack"]
	data.merge({"display_name": selection["name"], "asset_id": "building.%s.%s" % [selection["type"], _editor._asset_id_slug_from_display_name(selection["name"])],
		"pack_id": pack["pack_id"], "pack_name": pack.get("display_name", pack["pack_id"]), "pack_author": pack.get("author", ""),
		"lot_width_cells": 2, "lot_depth_cells": 2, "mesh_parts": [], "anchors": [], "site_surfaces": [], "tags": []}, true)
	document.reset({"params": data, "sources": [], "origin": {}}, false)
	_editor._view.show_task("model")
	if not str(selection.get("model", "")).is_empty():
		_editor._on_glb_file_selected(selection["model"])

func load_manifest(data: Dictionary) -> void:
	var pack_id := str(data.get("pack_id", ""))
	var directory := ProjectSettings.globalize_path("user://mods/%s/assets/%s" % [pack_id, data.get("asset_id", "")])
	var pack = JSON.parse_string(_editor.sim.get_pack_manifest_json(directory.get_base_dir().get_base_dir()))
	var metadata := data.duplicate(true)
	metadata["pack_name"] = pack.get("display_name", pack_id) if pack != null else pack_id
	metadata["pack_author"] = pack.get("author", "") if pack != null else ""
	var sources: Array = []
	for part: Dictionary in metadata.get("mesh_parts", []):
		var paths: Array = []
		for lod: Dictionary in part.get("lods", []):
			paths.append(directory.path_join(str(lod.get("file", ""))))
		sources.append(paths)
	var thumbnail_source := directory.path_join(str(metadata["thumbnail"])) if metadata.get("thumbnail") != null else ""
	_adapter.forget_sources()
	var colour_sources: Dictionary = JSON.parse_string(AssetAuthoringPolicy.colour_sources_json(JSON.stringify(metadata), directory))
	document.reset({"params": metadata, "sources": sources, "colour_sources": colour_sources.get("sources", {}), "origin": {"pack_id": pack_id, "asset_id": data.get("asset_id", "")}, "thumbnail_source": thumbnail_source})
	thumbnails.load_image(thumbnail_source)
	_editor._view.show_task("model")

func choose_pack(pack: Dictionary) -> void:
	if rendering or not ready:
		return
	if is_instance_valid(_creation) and _creation.visible:
		_creation.set_packs(_editor._known_packs, str(pack["pack_id"]))
		return
	var next := document.snapshot()
	next["params"].merge({"pack_id": pack["pack_id"], "pack_name": pack.get("display_name", pack["pack_id"]), "pack_author": pack.get("author", "")}, true)
	document.apply(next, "Change destination pack")

func undo() -> void:
	document.commit_transaction()
	document.undo()

func redo() -> void:
	document.commit_transaction()
	document.redo()

func open_conversion() -> void:
	var dialog := ConfirmationDialog.new()
	dialog.title = "Change asset type"
	dialog.get_ok_button().text = "Review changes…"
	var body := VBoxContainer.new()
	dialog.add_child(body)
	var type := OptionButton.new()
	var subtype := OptionButton.new()
	body.add_child(type)
	body.add_child(subtype)
	var catalog: Dictionary = JSON.parse_string(policy.types_json())
	for item: Dictionary in catalog["types"]:
		type.add_item(item["label"])
		type.set_item_metadata(type.item_count - 1, item["id"])
	for item: Dictionary in catalog["services"]:
		subtype.add_item(item["label"])
		subtype.set_item_metadata(subtype.item_count - 1, item["id"])
	subtype.visible = false
	type.item_selected.connect(func(index): subtype.visible = type.get_item_metadata(index) == "service")
	dialog.confirmed.connect(func():
		dialog.hide()
		_review_conversion(str(type.get_item_metadata(type.selected)), str(subtype.get_item_metadata(subtype.selected)))
	)
	_editor.add_child(dialog)
	_release_on_hide(dialog)
	_editor._view._apply_editor_theme(dialog)
	dialog.popup_centered(Vector2i(520, 180))

func _review_conversion(target: String, subtype: String) -> void:
	var result: Dictionary = JSON.parse_string(policy.conversion_json(JSON.stringify(params), target, subtype))
	if result.has("error"):
		message(result["error"])
		return
	var lines: Array[String] = ["Only the following authored fields will change:"]
	for change: Dictionary in result["changes"]:
		lines.append("%s: %s → %s" % [change["field"], JSON.stringify(change["before"]), JSON.stringify(change["after"])])
	if result["changes"].is_empty():
		message("This asset already has the selected type.")
		return
	var confirm := ConfirmationDialog.new()
	confirm.title = "Confirm type conversion"
	confirm.dialog_text = "\n".join(lines)
	confirm.get_ok_button().text = "Apply conversion"
	confirm.confirmed.connect(func():
		var next := document.snapshot()
		next["params"] = result["document"]
		document.apply(next, "Convert asset type")
	)
	_editor.add_child(confirm)
	_release_on_hide(confirm)
	_editor._view._apply_editor_theme(confirm)
	confirm.popup_centered()

func guard(action: Callable) -> void:
	if _editor._menus.actions != null:
		_editor._menus.actions.cancel()
	document.commit_transaction()
	if is_instance_valid(_guard) and _guard.visible:
		return
	_after_save = Callable()
	if not document.is_dirty():
		action.call()
		return
	_guard = ConfirmationDialog.new()
	_guard.title = "Unsaved asset draft"
	_guard.dialog_text = "Save your changes as a draft before continuing?"
	_guard.get_ok_button().text = "Discard changes"
	_guard.add_button("Save draft", false, "save")
	_guard.confirmed.connect(func():
		_guard.hide()
		action.call()
	)
	_guard.confirmed.connect(func(): _after_save = Callable())
	_guard.canceled.connect(func(): _after_save = Callable())
	_guard.custom_action.connect(func(_id):
		_after_save = action
		_guard.hide()
		save_draft()
	)
	_editor.add_child(_guard)
	_release_on_hide(_guard)
	_editor._view._apply_editor_theme(_guard)
	_guard.popup_centered(Vector2i(540, 200))

func save_draft() -> void:
	if not has_document:
		return
	document.commit_transaction()
	if document.draft_path().is_empty():
		save_draft_as()
	else:
		_save_to(document.draft_path())

func save_draft_as() -> void:
	if not has_document:
		return
	_file_dialog(FileDialog.FILE_MODE_SAVE_FILE, _save_to)

func open_draft_dialog() -> void:
	guard(func(): _file_dialog(FileDialog.FILE_MODE_OPEN_FILE, load_draft))

func _file_dialog(mode: int, selected: Callable) -> void:
	var dialog := FileDialog.new()
	dialog.access = FileDialog.ACCESS_FILESYSTEM
	dialog.file_mode = mode
	dialog.filters = PackedStringArray(["*.metrum-draft ; Metrum asset draft"])
	dialog.current_file = str(params.get("asset_id", "asset")) + ".metrum-draft" if mode == FileDialog.FILE_MODE_SAVE_FILE else ""
	dialog.file_selected.connect(func(path):
		dialog.hide()
		selected.call(path)
	)
	dialog.canceled.connect(func(): _after_save = Callable())
	_editor.add_child(dialog)
	_release_on_hide(dialog)
	_editor._view._apply_editor_theme(dialog)
	dialog.popup_centered(Vector2i(760, 520))

func _save_to(path: String) -> void:
	var error := AssetAuthoringFiles.save_draft(path, document.snapshot())
	if not error.is_empty():
		_after_save = Callable()
		message(error)
		return
	document.mark_saved(path)
	if _after_save.is_valid():
		var action := _after_save
		_after_save = Callable()
		if is_instance_valid(_guard):
			_guard.hide()
		action.call()

func load_draft(path: String) -> void:
	var result := AssetAuthoringFiles.load_draft(path)
	if result.has("error"):
		message(result["error"])
		return
	_adapter.forget_sources()
	document.reset(result["document"], true, path)
	thumbnails.load_image(str(result["document"].get("thumbnail_source", "")))
	_editor._view.show_task("model")

func validate() -> void:
	if not ready or not has_document:
		return
	var result: Dictionary = JSON.parse_string(policy.inspect_json(JSON.stringify(params)))
	_issues = result.get("issues", [])
	if not colours.error().is_empty():
		_issues.append({"section": "model", "field": "appearance", "message": colours.error()})
	for part in _editor._parts:
		var error: String = part.validation_error()
		if not error.is_empty():
			_issues.append({"section": "model", "field": "mesh_parts", "message": error})
	var fit_error: String = _editor._mesh_parts_lot_fit_error()
	if not fit_error.is_empty():
		_issues.append({"section": "site", "field": "lot_width_cells", "message": fit_error})
	var view = _editor._view
	var tiers := 0
	for part in _editor._parts:
		tiers += part.lods.size()
	view.export_summary.text = "Destination: user://mods/%s/assets/%s\n%d mesh part(s), %d authored LOD meshes.\nThumbnail: %s" % [str(params.get("pack_id", "")), str(params.get("asset_id", "")), _editor._parts.size(), tiers, str(params.get("thumbnail", "Not captured"))]
	for child in view.issues_box.get_children():
		view.issues_box.remove_child(child)
		child.queue_free()
	for issue: Dictionary in _issues:
		var button: Button = view.button(view.issues_box, str(issue["message"]), func(): resolve_issue(issue))
		button.autowrap_mode = TextServer.AUTOWRAP_WORD_SMART
	view.status.text = "%s\n%s" % ["Unsaved changes" if document.is_dirty() else "No unsaved changes", "Ready to export" if _issues.is_empty() else "%d issue(s) — open Validate & export" % _issues.size()]
	view._apply_editor_theme(view.issues_box)

func resolve_issue(issue: Dictionary) -> void:
	var field := str(issue["field"])
	var view = _editor._view
	if field == "pack_id":
		view.show_task("overview")
		_editor._open_pack_select_menu(view._pack_set_btn)
	elif field == "type":
		open_conversion()
	elif issue.has("replacement") or (field == "economy_profile" and not descriptor.get("fields", []).has(field)):
		# Legacy metadata stays intact until the author explicitly resolves it.
		var confirm := ConfirmationDialog.new()
		confirm.title = "Resolve preserved metadata"
		var replacement = issue.get("replacement")
		confirm.dialog_text = "%s: %s → %s\n\nApply this correction? All other metadata will be preserved. You can undo this change." % [field, JSON.stringify(params.get(field)), JSON.stringify(replacement)]
		confirm.get_ok_button().text = "Apply correction"
		confirm.confirmed.connect(func(): document.set_parameter(field, replacement, "Resolve " + field))
		_editor.add_child(confirm)
		_release_on_hide(confirm)
		view._apply_editor_theme(confirm)
		confirm.popup_centered(Vector2i(560, 220))
	else:
		view.reveal_field(str(issue["section"]), field)

func publish(move_original: bool = false) -> void:
	if not has_document:
		return
	document.commit_transaction()
	capture_geometry("Finish geometry edit")
	validate()
	if not _issues.is_empty():
		_editor._view.show_task("validate")
		return
	var state := document.snapshot()
	var output := ProjectSettings.globalize_path("user://mods/" + str(params["pack_id"]))
	var error := AssetAuthoringFiles.publish_document(JSON.stringify(state), output)
	if not error.is_empty():
		message("Export failed: " + error)
		return
	var origin: Dictionary = state.get("origin", {})
	var asset_directory := output.path_join("assets").path_join(str(params["asset_id"]))
	state["colour_sources"] = JSON.parse_string(AssetAuthoringPolicy.colour_sources_json(JSON.stringify(params), asset_directory)).get("sources", {})
	for index in _editor._parts.size():
		for tier in _editor._parts[index].lods.size():
			state["sources"][index][tier] = asset_directory.path_join(_editor._parts[index].lods[tier].file)
	if params.get("thumbnail") != null:
		state["thumbnail_source"] = asset_directory.path_join(str(params["thumbnail"]))
	state["origin"] = {"pack_id": params["pack_id"], "asset_id": params["asset_id"]}
	document.apply(state, "Publish runtime asset")
	# Publication persists this revision too, while leaving any draft file/path untouched.
	document.mark_saved(document.draft_path())
	if move_original:
		_editor._move_original_asset_after_export(str(origin.get("pack_id", "")), str(origin.get("asset_id", "")), str(params["pack_id"]), str(params["asset_id"]))
	_editor._refresh_asset_browser()
	_editor._view.status.text = "Runtime asset exported. No unsaved changes."
	_editor._log("Runtime asset exported: " + str(params["asset_id"]))

## Show the capture frame so the shot can be composed before it is taken.
func begin_thumbnail_framing() -> void:
	thumbnails.begin()

## Capture immediately at the fixed output size, skipping the framing step.
func capture_thumbnail() -> void:
	await thumbnails.capture()

func message(text: String) -> void:
	var dialog := AcceptDialog.new()
	dialog.dialog_text = text
	# Error dialogs must belong to the active modal, never compete with it.
	var parent: Node = _guard if is_instance_valid(_guard) and _guard.visible else _editor
	parent.add_child(dialog)
	_release_on_hide(dialog)
	_editor._view._apply_editor_theme(dialog)
	dialog.popup_centered(Vector2i(540, 180))

func _release_on_hide(window: Window) -> void:
	window.visibility_changed.connect(func():
		if not window.visible:
			window.queue_free()
	)
