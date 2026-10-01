# SPDX-License-Identifier: GPL-2.0-only

## Pack settings and share-archive export dialogs (`TOOLS-09`).
## Rust validates metadata and builds the archive; these dialogs only present and dispatch.
extends RefCounted

const MODS := "user://mods"
const FIELDS := [["display_name", "Name"], ["version", "Version (semantic, e.g. 1.2.0)"], ["author", "Author"], ["license", "License"], ["description", "Description"]]
const BUMPS := [["", "Keep version"], ["patch", "Patch"], ["minor", "Minor"], ["major", "Major"]]

var editor: Node
var destination := OS.get_system_dir(OS.SYSTEM_DIR_DOCUMENTS)

func _init(owner: Node) -> void:
	editor = owner

func settings_dialog(pack_id: String) -> ConfirmationDialog:
	var current := AssetAuthoringFiles.pack_settings(MODS, pack_id)
	if current.has("error"):
		editor._session.message(current.error)
		return null
	var dialog := _dialog("Pack settings — " + pack_id, "Save")
	var body: VBoxContainer = dialog.get_child(0)
	var fields := {}
	for entry in FIELDS:
		fields[entry[0]] = _field(body, entry[1], str(current.get(entry[0], "")))
	var error := _label(body, "")
	dialog.confirmed.connect(func():
		var settings := {}
		for key in fields: settings[key] = fields[key].text.strip_edges()
		var message := AssetAuthoringFiles.update_pack(MODS, pack_id, settings)
		if not message.is_empty():
			error.text = message
			return
		editor._refresh_asset_browser()
		dialog.queue_free()
	)
	_show(dialog)
	return dialog

## Opens immediately; inspection and export run on the WorkerThreadPool.
func export_dialog(pack_id: String) -> ConfirmationDialog:
	var dialog := _dialog("Export pack as zip", "Export")
	var body: VBoxContainer = dialog.get_child(0)
	dialog.get_ok_button().disabled = true
	var state := {"pack_id": pack_id, "mods": ProjectSettings.globalize_path(MODS), "path": "", "replace": ""}
	state.summary = _label(body, "Checking %s…" % pack_id)
	var session = editor._session
	if session.has_document and session.document.is_dirty() and str(session.params.get("pack_id", "")) == pack_id:
		_label(body, "The open asset has unsaved changes for this pack. The archive contains only what is already published.")
	state.excluded = _label(body, "")
	_label(body, "Version")
	state.bump = OptionButton.new()
	body.add_child(state.bump)
	_label(body, "Destination folder")
	var row := HBoxContainer.new()
	body.add_child(row)
	var folder := LineEdit.new()
	folder.name = "Destination"
	folder.text = destination
	folder.size_flags_horizontal = Control.SIZE_EXPAND_FILL
	row.add_child(folder)
	state.folder = folder
	var browse := Button.new()
	browse.text = "Browse…"
	row.add_child(browse)
	var picker := FileDialog.new()
	picker.file_mode = FileDialog.FILE_MODE_OPEN_DIR
	picker.access = FileDialog.ACCESS_FILESYSTEM
	picker.dir_selected.connect(func(path: String): folder.text = path)
	dialog.add_child(picker)
	browse.pressed.connect(func():
		picker.current_dir = folder.text
		picker.popup_centered_ratio(0.6)
	)
	state.error = _label(body, "")
	var result := HBoxContainer.new()
	result.visible = false
	body.add_child(result)
	state.result = result
	var digest := LineEdit.new()
	digest.name = "Sha256"
	digest.editable = false
	digest.size_flags_horizontal = Control.SIZE_EXPAND_FILL
	result.add_child(digest)
	state.digest = digest
	var copy := Button.new()
	copy.text = "Copy SHA-256"
	copy.pressed.connect(func(): DisplayServer.clipboard_set(digest.text))
	result.add_child(copy)
	var reveal := Button.new()
	reveal.text = "Show file"
	reveal.pressed.connect(func(): OS.shell_show_in_file_manager(state.path))
	result.add_child(reveal)
	dialog.confirmed.connect(_confirm_export.bind(dialog, state))
	_show(dialog)
	_inspect(dialog, state)
	return dialog

func _inspect(dialog: ConfirmationDialog, state: Dictionary) -> void:
	var mods: String = state.mods
	var pack_id: String = state.pack_id
	var inspection := await _in_background(func(): return AssetAuthoringFiles.inspect_pack(mods, pack_id))
	if not is_instance_valid(dialog): return
	if inspection.has("error"):
		state.summary.text = "This pack cannot be exported:\n" + str(inspection.error)
		_fit(dialog)
		return
	var version := str(inspection.version)
	state.summary.text = "%s — version %s\n%d assets, %d files, %s" % [inspection.display_name, version, inspection.assets, inspection.files, String.humanize_size(int(inspection.bytes))]
	var left_out: PackedStringArray = inspection.excluded
	if not left_out.is_empty():
		var shown := left_out.slice(0, 12)
		state.excluded.text = "Not included (unreferenced or derived, %d):\n%s%s" % [left_out.size(), "\n".join(shown), "\n…" if left_out.size() > shown.size() else ""]
	_versions(state.bump, version)
	dialog.get_ok_button().disabled = false
	_fit(dialog)

# Choices carry the archive version, so the overwrite check names the file export will write.
func _versions(bump: OptionButton, version: String) -> void:
	bump.clear()
	for entry in BUMPS:
		var next: String = version if entry[0].is_empty() else AssetAuthoringFiles.bumped_version(version, entry[0])
		bump.add_item("%s (%s)" % [entry[1], version] if entry[0].is_empty() else "%s → %s" % [entry[1], next])
		bump.set_item_metadata(bump.item_count - 1, [entry[0], next])
		bump.set_item_disabled(bump.item_count - 1, next.is_empty())
	bump.select(0)

func _confirm_export(dialog: ConfirmationDialog, state: Dictionary) -> void:
	var path: String = state.folder.text.strip_edges()
	if not DirAccess.dir_exists_absolute(path):
		state.error.text = "Choose an existing destination folder."
		return
	var choice: Array = state.bump.get_item_metadata(state.bump.selected)
	var target := path.path_join("%s-%s.metrum.zip" % [state.pack_id, choice[1]])
	# A second press on the same existing target is the overwrite confirmation.
	if FileAccess.file_exists(target) and state.replace != target:
		state.replace = target
		state.error.text = "%s already exists. Press Export again to replace it." % target.get_file()
		return
	destination = path
	var mods: String = state.mods
	var pack_id: String = state.pack_id
	var part: String = choice[0]
	dialog.get_ok_button().disabled = true
	dialog.get_cancel_button().disabled = true
	state.error.text = "Exporting…"
	var exported := await _in_background(func(): return AssetAuthoringFiles.export_pack(mods, pack_id, path, part))
	if not is_instance_valid(dialog): return
	dialog.get_cancel_button().disabled = false
	dialog.get_cancel_button().text = "Close"
	if exported.has("error"):
		state.error.text = "Export failed: " + str(exported.error)
		dialog.get_ok_button().disabled = false
		_fit(dialog)
		return
	state.path = str(exported.path)
	state.error.text = "Exported %s\nSHA-256 (publish it separately so players can verify the download):" % state.path
	state.digest.text = str(exported.sha256)
	state.result.visible = true
	_fit(dialog)
	if not part.is_empty():
		state.summary.text += "\nVersion is now %s." % str(exported.version)
		_versions(state.bump, str(exported.version))
		editor._refresh_asset_browser()

# Runs `work` on the WorkerThreadPool and resumes on the main thread with its Dictionary.
func _in_background(work: Callable) -> Dictionary:
	var box := {}
	var task := WorkerThreadPool.add_task(func(): box.result = work.call())
	while not WorkerThreadPool.is_task_completed(task):
		await editor.get_tree().process_frame
	WorkerThreadPool.wait_for_task_completion(task)
	return box.get("result", {"error": "The background task returned no result."})

func _dialog(title: String, confirm: String) -> ConfirmationDialog:
	var dialog := ConfirmationDialog.new()
	dialog.title = title
	dialog.get_ok_button().text = confirm
	dialog.dialog_hide_on_ok = false
	var body := VBoxContainer.new()
	body.custom_minimum_size.x = 520
	dialog.add_child(body)
	return dialog

func _field(body: VBoxContainer, title: String, value: String) -> LineEdit:
	_label(body, title)
	var field := LineEdit.new()
	field.text = value
	body.add_child(field)
	return field

func _label(body: VBoxContainer, text: String) -> Label:
	var label := Label.new()
	label.text = text
	label.autowrap_mode = TextServer.AUTOWRAP_WORD_SMART
	body.add_child(label)
	return label

func _show(dialog: ConfirmationDialog) -> void:
	editor.add_child(dialog)
	editor._view._apply_editor_theme(dialog)
	dialog.canceled.connect(dialog.queue_free)
	dialog.popup_centered()
	_fit.call_deferred(dialog)

# Wrapping labels report one character per line until laid out at the dialog width, which
# leaves the window far taller than its content; shrink once the text has its width.
func _fit(dialog: ConfirmationDialog) -> void:
	if not is_instance_valid(dialog): return
	dialog.reset_size()
	dialog.move_to_center()
