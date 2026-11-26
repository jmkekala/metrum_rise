# SPDX-License-Identifier: GPL-3.0-or-later

## Per-pack actions in Options → Mods (`TOOLS-11`): verify, show folder, remove and, for a
## bundled pack the player changed, restore the game's version (`TOOLS-17`). Rust checks removal
## eligibility, verifies files and decides which copies are the player's and how a restore goes
## (through the start-up's own steps, `UserDataBootstrap.restore_bundled`); this script only
## presents and dispatches.
extends RefCounted

const BackgroundTask = preload("res://scripts/core/background_task.gd")
const UserDataBootstrap = preload("res://scripts/core/user_data_bootstrap.gd")

const MODS := "user://mods"
const BUNDLED := "res://bootstrap/mods"
const BUNDLE := "res://bootstrap"
const PROFILE := "user://"
## Most paths listed per kind of difference; the rest are counted.
const SHOWN := 12

## Emitted after a pack folder has moved to Trash.
signal removed(pack_id: String)
## Emitted after the game's version of a bundled pack replaced the player's copy.
signal restored(pack_id: String)

var host: Control
## Replaceable for tests: the isolated profile must not reach the desktop Trash or file manager.
var trash_operation: Callable = OS.move_to_trash
var show_operation: Callable = OS.shell_show_in_file_manager
## Returns true while `pack_id` is loaded by a running city, which must not lose its files.
var pack_in_use: Callable = func(_pack_id: String) -> bool: return false

func _init(owner: Control) -> void:
	host = owner

func is_bundled(pack_id: String) -> bool:
	return pack_id in DirAccess.get_directories_at(BUNDLED)

## The bundled packs whose installed copy is the player's (the install receipt says so): no
## start replaces them, and Restore is offered. One receipt read; nothing hashed.
func players_copies() -> PackedStringArray:
	return AssetAuthoringFiles.bundled_restorable(PROFILE)

func can_verify(pack_id: String) -> bool:
	return FileAccess.file_exists(MODS.path_join(pack_id).path_join("checksums.sha256"))

func show_folder(pack_id: String) -> void:
	show_operation.call(ProjectSettings.globalize_path(MODS).path_join(pack_id))

## Opens immediately; hashing runs on the WorkerThreadPool.
func verify_dialog(pack_id: String) -> AcceptDialog:
	var dialog := AcceptDialog.new()
	dialog.title = "Verify " + pack_id
	dialog.get_ok_button().text = "Close"
	var report := _label(dialog, "Checking %s…" % pack_id)
	_show(dialog)
	_verify(dialog, report, pack_id)
	return dialog

func _verify(dialog: AcceptDialog, report: Label, pack_id: String) -> void:
	var mods := ProjectSettings.globalize_path(MODS)
	var result := await BackgroundTask.run(host.get_tree(), func(): return AssetAuthoringFiles.verify_installed_pack(mods, pack_id))
	if not is_instance_valid(dialog): return
	if result.has("error"):
		report.text = "Cannot verify: " + str(result.error)
	else:
		var sections := PackedStringArray()
		for entry in [["changed", "Changed since import"], ["missing", "Missing"], ["extra", "Added (not in the imported pack)"]]:
			var paths: PackedStringArray = result[entry[0]]
			if paths.is_empty(): continue
			var shown := paths.slice(0, SHOWN)
			sections.append("%s (%d):\n%s%s" % [entry[1], paths.size(), "\n".join(shown), "\n…" if paths.size() > shown.size() else ""])
		if not str(result.invalid).is_empty():
			sections.append("The pack no longer validates: " + str(result.invalid))
		if sections.is_empty():
			report.text = "All %d files match the pack as imported." % int(result.files)
		else:
			report.text = "%s\n\nRe-import the archive to restore the original files." % "\n\n".join(sections)
	_fit(dialog)

## Returns null when removal is refused. Refusals and failures go to `message`; success is
## reported through `removed`.
func remove_dialog(pack_id: String, message: Callable) -> ConfirmationDialog:
	var reason := _removal_refusal(pack_id)
	if not reason.is_empty():
		message.call(reason)
		return null
	var dialog := ConfirmationDialog.new()
	dialog.title = "Remove pack?"
	dialog.get_ok_button().text = "Move to Trash"
	_label(dialog, "%s moves to the system Trash.\n\nSaves that use its assets show placeholders for them until it is installed again. Recovery is through your system Trash." % pack_id)
	dialog.confirmed.connect(func():
		var error := remove(pack_id)
		if not error.is_empty(): message.call(error)
		dialog.queue_free()
	)
	_show(dialog)
	return dialog

## Revalidates, then moves the pack to Trash. Returns why it did not, or "" on success.
func remove(pack_id: String) -> String:
	var reason := _removal_refusal(pack_id)
	if not reason.is_empty(): return reason
	var path := str(AssetAuthoringFiles.inspect_pack_removal(MODS, pack_id, DirAccess.get_directories_at(BUNDLED)).path)
	var error: int = trash_operation.call(path)
	if error != OK:
		return "Could not move %s to Trash (%s). Nothing was deleted." % [pack_id, error_string(error)]
	removed.emit(pack_id)
	return ""

## Returns null when a running city uses the pack (as Remove and an import over it are refused).
## Refusals and failures go to `message`; success is reported through `restored`.
func restore_dialog(pack_id: String, message: Callable) -> ConfirmationDialog:
	var reason := _restore_refusal(pack_id)
	if not reason.is_empty():
		message.call(reason)
		return null
	var dialog := ConfirmationDialog.new()
	dialog.title = "Restore the game's version?"
	dialog.get_ok_button().text = "Restore"
	_label(dialog, "Your changed copy of %s moves to the system Trash and the game's version is installed in its place. From then on it follows the game's updates again.\n\nThe game's version loads when a city starts. Recovery of your copy is through your system Trash." % pack_id)
	dialog.confirmed.connect(func():
		var error := restore(pack_id)
		if not error.is_empty(): message.call(error)
		dialog.queue_free()
	)
	_show(dialog)
	return dialog

## Revalidates, then moves the player's copy to Trash and installs the game's version. Returns
## why it did not (or what is left to happen), or "" on success.
func restore(pack_id: String) -> String:
	var reason := _restore_refusal(pack_id)
	if not reason.is_empty(): return reason
	var error := UserDataBootstrap.restore_bundled(BUNDLE, PROFILE, pack_id, trash_operation)
	if not error.is_empty(): return error
	restored.emit(pack_id)
	return ""

func _restore_refusal(pack_id: String) -> String:
	if pack_in_use.call(pack_id):
		return "%s is enabled in the running city. Return to the main menu to restore it." % pack_id
	return ""

func _removal_refusal(pack_id: String) -> String:
	if pack_in_use.call(pack_id):
		return "%s is enabled in the running city. Return to the main menu to remove it." % pack_id
	var check := AssetAuthoringFiles.inspect_pack_removal(MODS, pack_id, DirAccess.get_directories_at(BUNDLED))
	return str(check.get("error", ""))

func _label(dialog: AcceptDialog, text: String) -> Label:
	var label := Label.new()
	label.text = text
	label.autowrap_mode = TextServer.AUTOWRAP_WORD_SMART
	label.custom_minimum_size.x = 480
	dialog.add_child(label)
	return label

func _show(dialog: AcceptDialog) -> void:
	dialog.canceled.connect(dialog.queue_free)
	dialog.confirmed.connect(func(): if dialog is not ConfirmationDialog: dialog.queue_free())
	host.add_child(dialog)
	dialog.popup_centered()
	_fit.call_deferred(dialog)

# Wrapping labels report one character per line until laid out; shrink once they have width.
func _fit(dialog: AcceptDialog) -> void:
	if not is_instance_valid(dialog): return
	dialog.reset_size()
	dialog.move_to_center()
