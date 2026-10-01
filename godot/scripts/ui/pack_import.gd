# SPDX-License-Identifier: GPL-2.0-only

## Share-archive import for Options → Mods (`TOOLS-10`).
## Rust verifies, stages and installs the archive; this script only drives the file picker,
## the SHA-256 prompt and the confirmation, and keeps the work off the main thread.
extends RefCounted

const BackgroundTask = preload("res://scripts/core/background_task.gd")

const MODS := "user://mods"
const BUNDLED := "res://bootstrap/mods"

## `replaced` is false for a pack that was not installed before.
signal installed(pack_id: String, replaced: bool)

var host: Control
## Replaceable for tests: the isolated profile must not reach the desktop Trash.
var trash_operation: Callable = OS.move_to_trash
## Returns true while `pack_id` is loaded by a running city, which must not be replaced.
var pack_in_use: Callable = func(_pack_id: String) -> bool: return false

func _init(owner: Control) -> void:
	host = owner

func pick() -> FileDialog:
	var picker := FileDialog.new()
	picker.title = "Import pack"
	picker.file_mode = FileDialog.FILE_MODE_OPEN_FILE
	picker.access = FileDialog.ACCESS_FILESYSTEM
	picker.filters = PackedStringArray(["*.metrum.zip ; Metrum Rise packs", "*.zip ; Zip archives"])
	picker.current_dir = OS.get_system_dir(OS.SYSTEM_DIR_DOWNLOADS)
	picker.file_selected.connect(func(path: String):
		picker.queue_free()
		open_archive(path)
	)
	picker.canceled.connect(picker.queue_free)
	host.add_child(picker)
	picker.popup_centered_ratio(0.6)
	return picker

## Starts hashing at once, so the comparison is ready by the time the SHA-256 is typed.
func open_archive(path: String) -> ConfirmationDialog:
	var dialog := ConfirmationDialog.new()
	dialog.title = "Import pack"
	dialog.dialog_hide_on_ok = false
	dialog.get_ok_button().text = "Verify"
	var body := VBoxContainer.new()
	body.custom_minimum_size.x = 520
	dialog.add_child(body)
	var state := {"path": path, "actual": {}, "staging": "", "staged": {}}
	_label(body, "Archive: " + path.get_file())
	_label(body, "Expected SHA-256 — copy it from where the author published the pack, not from a file downloaded with it:")
	var field := LineEdit.new()
	field.name = "Sha256"
	field.placeholder_text = "64 hexadecimal characters"
	field.size_flags_horizontal = Control.SIZE_EXPAND_FILL
	var row := HBoxContainer.new()
	body.add_child(row)
	row.add_child(field)
	var paste := Button.new()
	paste.name = "PasteSha256"
	paste.text = "Paste"
	# Whole clipboard: a copied sidecar line is accepted too, and Rust normalises it.
	paste.pressed.connect(func():
		if field.editable: field.text = DisplayServer.clipboard_get().strip_edges()
	)
	row.add_child(paste)
	state.field = field
	state.summary = _label(body, "")
	state.status = _label(body, "")
	field.text_submitted.connect(func(_text: String): dialog.confirmed.emit())
	dialog.confirmed.connect(_confirm.bind(dialog, state))
	dialog.canceled.connect(dialog.queue_free)
	# Covers Cancel, Close and the options window going away while a pack is staged.
	dialog.tree_exiting.connect(func(): _discard(state))
	host.add_child(dialog)
	dialog.popup_centered()
	_fit.call_deferred(dialog)
	field.grab_focus.call_deferred()
	_hash(dialog, state)
	return dialog

func _hash(dialog: ConfirmationDialog, state: Dictionary) -> void:
	var path: String = state.path
	state.actual = await BackgroundTask.run(host.get_tree(), func(): return AssetAuthoringFiles.file_sha256(path))
	if is_instance_valid(dialog) and state.actual.has("error"):
		_fail(dialog, state, "The archive cannot be read: " + str(state.actual.error))

func _confirm(dialog: ConfirmationDialog, state: Dictionary) -> void:
	if not state.staged.is_empty():
		_install(dialog, state)
		return
	var expected := AssetAuthoringFiles.expected_sha256(state.field.text)
	if expected.is_empty():
		state.status.text = "Enter the 64-character SHA-256 the author published."
		_fit(dialog)
		return
	dialog.get_ok_button().disabled = true
	state.field.editable = false
	state.status.text = "Verifying…"
	while state.actual.is_empty():
		await host.get_tree().process_frame
	if not is_instance_valid(dialog) or state.actual.has("error"): return
	if state.actual.sha256 != expected:
		# A typo can be corrected; there is no way to install a mismatching archive.
		state.status.text = "SHA-256 mismatch: the archive is %s. It was not imported." % state.actual.sha256
		state.field.editable = true
		dialog.get_ok_button().disabled = false
		_fit(dialog)
		return
	state.status.text = "Checking the pack…"
	var mods := ProjectSettings.globalize_path(MODS)
	var archive: String = state.path
	var bundled := DirAccess.get_directories_at(BUNDLED)
	var staged := await BackgroundTask.run(host.get_tree(), func(): return AssetAuthoringFiles.stage_import(mods, archive, expected, bundled))
	state.staging = str(staged.get("staging", ""))
	if not is_instance_valid(dialog):
		_discard(state)
		return
	if staged.has("error"):
		_fail(dialog, state, "This archive cannot be imported:\n" + str(staged.error))
		return
	_review(dialog, state, staged)

func _review(dialog: ConfirmationDialog, state: Dictionary, staged: Dictionary) -> void:
	var pack_id := str(staged.pack_id)
	state.summary.text = "%s (%s) — version %s by %s\n%d assets, %d files, %s" % [staged.display_name, pack_id, staged.version, staged.author, staged.assets, staged.files, String.humanize_size(int(staged.bytes))]
	match str(staged.installed):
		"identical":
			_finish(dialog, state, "Already installed: this exact pack is in your mods folder. Nothing was changed.")
			return
		"absent":
			state.status.text = "The pack installs disabled; enable it in the pack list afterwards."
			dialog.get_ok_button().text = "Install"
		_:
			if pack_in_use.call(pack_id):
				_fail(dialog, state, "%s is enabled in the running city. Return to the main menu to replace it." % pack_id)
				return
			var installed_version := str(staged.installed_version)
			var lines := {
				"update": "Update from version %s." % installed_version,
				"same": "Version %s is already installed with different contents. Replacing it discards those changes." % installed_version,
				"downgrade": "Warning: this is older than the installed version %s." % installed_version,
			}
			state.status.text = "%s\nThe installed copy moves to the system Trash." % lines.get(str(staged.change), "The installed copy (unreadable version) will be replaced.")
			dialog.get_ok_button().text = "Replace"
	state.staged = staged
	dialog.get_ok_button().disabled = false
	_fit(dialog)

func _install(dialog: ConfirmationDialog, state: Dictionary) -> void:
	var pack_id := str(state.staged.pack_id)
	var replacing := str(state.staged.installed) == "different"
	if replacing:
		var target := ProjectSettings.globalize_path(MODS).path_join(pack_id)
		var error: int = trash_operation.call(target)
		if error != OK:
			_fail(dialog, state, "Could not move the installed %s to Trash (%s). Nothing was changed." % [pack_id, error_string(error)])
			return
	var result := AssetAuthoringFiles.commit_import(ProjectSettings.globalize_path(MODS), state.staging)
	# Commit consumes the staging folder whether or not it succeeds.
	state.staging = ""
	if result.has("error"):
		_fail(dialog, state, "Install failed: %s%s" % [result.error, "\nThe previous copy is in the system Trash." if replacing else ""])
		return
	_finish(dialog, state, "Installed %s. Enable it in the pack list and apply; packs load when a city starts." % pack_id)
	installed.emit(pack_id, replacing)

func _fail(dialog: ConfirmationDialog, state: Dictionary, message: String) -> void:
	_discard(state)
	_finish(dialog, state, message)

func _finish(dialog: ConfirmationDialog, state: Dictionary, message: String) -> void:
	state.staged = {}
	state.status.text = message
	state.field.editable = false
	dialog.get_ok_button().visible = false
	dialog.get_cancel_button().text = "Close"
	_fit(dialog)

func _discard(state: Dictionary) -> void:
	if state.staging.is_empty(): return
	var error := AssetAuthoringFiles.discard_import(ProjectSettings.globalize_path(MODS), state.staging)
	if not error.is_empty(): push_warning("Could not remove import staging folder: " + error)
	state.staging = ""

func _label(body: VBoxContainer, text: String) -> Label:
	var label := Label.new()
	label.text = text
	label.autowrap_mode = TextServer.AUTOWRAP_WORD_SMART
	body.add_child(label)
	return label

# Wrapping labels report one character per line until laid out; shrink once they have width.
func _fit(dialog: ConfirmationDialog) -> void:
	if not is_instance_valid(dialog): return
	dialog.reset_size()
	dialog.move_to_center()
