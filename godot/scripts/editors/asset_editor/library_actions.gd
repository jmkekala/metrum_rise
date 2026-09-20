# SPDX-License-Identifier: GPL-2.0-only

## Library dialogs and native platform actions; Rust validates all asset file operations.
## Trash is recoverable and separate from document history; publication is always explicit.
extends RefCounted

var editor: Node
var trash_operation: Callable = OS.move_to_trash

func _init(owner: Node) -> void:
	editor = owner

func location(id: String) -> Dictionary:
	return AssetAuthoringFiles.asset_location("user://mods", editor._asset_pack_id(id), editor._asset_local_id(id))

func execute(action: String, target: Dictionary) -> void:
	var id := str(target.get("id", ""))
	match action:
		"library_open": editor._load_asset_manifest(id)
		"library_compare": editor._use_asset_as_ghost(id)
		"library_id": DisplayServer.clipboard_set(id)
		"library_refresh": editor._refresh_asset_browser()
		"library_pack_create": editor._open_new_pack_dialog()
		"library_new": editor._session.new_asset_dialog(target.get("pack", ""), target.get("type", ""))
		"library_folder":
			var path := location(id)
			if path.has("error"): editor._session.message(path.error)
			else: OS.shell_show_in_file_manager(path.path)
		"library_pack_folder":
			# Pack identities originate in the loaded catalog, never arbitrary popup text.
			for pack: Dictionary in editor._known_packs:
				if pack.get("pack_id") == target.get("pack"):
					OS.shell_show_in_file_manager(ProjectSettings.globalize_path("user://mods/" + str(pack.pack_id)))
		"library_copy": editor._session.guard(func(): _copy_dialog(id))
		"library_trash": _trash_dialog(id)

func _copy_dialog(id: String) -> void:
	var revision: int = editor._menus.generation
	var dialog := ConfirmationDialog.new()
	dialog.title = "Create editable copy"
	dialog.get_ok_button().text = "Create working copy"
	dialog.dialog_hide_on_ok = false
	var body := VBoxContainer.new()
	body.custom_minimum_size.x = 460
	dialog.add_child(body)
	var name_field := _field(body, "Name", editor._asset_browser_label(id) + " copy")
	var id_field := _field(body, "New asset ID", editor._asset_local_id(id) + ".copy")
	var label := Label.new()
	label.text = "Destination pack"
	body.add_child(label)
	var packs := OptionButton.new()
	body.add_child(packs)
	for pack: Dictionary in editor._known_packs:
		packs.add_item(str(pack.get("display_name", pack.pack_id)))
		packs.set_item_metadata(packs.item_count - 1, pack)
	var note := Label.new()
	note.text = "Copies models and original pack credits into an independent workspace.\nNothing is published until you export the new asset."
	body.add_child(note)
	var error := Label.new()
	error.autowrap_mode = TextServer.AUTOWRAP_WORD_SMART
	body.add_child(error)
	dialog.confirmed.connect(func():
		if revision != editor._menus.generation:
			error.text = "The open document changed. Close this dialog and start the copy again."
			return
		if packs.selected < 0:
			error.text = "Create a destination pack first."
			return
		var result := copy_asset(id, packs.get_item_metadata(packs.selected), id_field.text.strip_edges(), name_field.text.strip_edges())
		if result.has("error"):
			error.text = result.error
			return
		editor._session._adapter.forget_sources()
		editor._session.document.reset(result.document, false)
		editor._view.show_task("model")
		dialog.queue_free()
	)
	_show_dialog(dialog)

func copy_asset(id: String, destination: Dictionary, new_id: String, name_text: String) -> Dictionary:
	var manifest: Dictionary = editor._asset_manifest(id)
	if manifest.is_empty(): return {"error": "The source asset is no longer available."}
	return AssetAuthoringFiles.copy_for_editing("user://mods", manifest, destination, new_id, name_text, "user://asset_editor/copies")

func _field(body: VBoxContainer, title: String, value: String) -> LineEdit:
	var label := Label.new()
	label.text = title
	body.add_child(label)
	var field := LineEdit.new()
	field.text = value
	body.add_child(field)
	return field

func trash_check(id: String) -> Dictionary:
	if editor._session.document.has_publication_origin(editor._asset_pack_id(id), editor._asset_local_id(id)):
		return {"error": "This asset is open for editing. Open or create another asset before moving it to Trash."}
	return AssetAuthoringFiles.inspect_trash("user://mods", editor._asset_pack_id(id), editor._asset_local_id(id), editor._session.document.protected_sources())

func _trash_dialog(id: String) -> void:
	var check := trash_check(id)
	if check.has("error"):
		editor._session.message(check.error)
		return
	var dialog := ConfirmationDialog.new()
	dialog.title = "Move asset to Trash?"
	dialog.dialog_text = "%s\n%s\n%s\n\nExisting saves may reference this asset.\nRecovery is through your system Trash, not editor Undo." % [editor._asset_browser_label(id), id, check.path]
	dialog.get_ok_button().text = "Move to Trash"
	dialog.confirmed.connect(func():
		var error := move_to_trash(id)
		if not error.is_empty(): editor._session.message(error)
		dialog.queue_free()
	)
	_show_dialog(dialog)

func move_to_trash(id: String) -> String:
	# Revalidate after confirmation, including current working sources and undo references.
	var check := trash_check(id)
	if check.has("error"): return check.error
	var result: int = trash_operation.call(check.path)
	if result != OK: return "Could not move the asset to Trash (%s). No permanent deletion was attempted." % error_string(result)
	editor._refresh_asset_browser()
	return ""

func _show_dialog(dialog: ConfirmationDialog) -> void:
	editor.add_child(dialog)
	editor._view._apply_editor_theme(dialog)
	dialog.canceled.connect(dialog.queue_free)
	dialog.popup_centered()
