# SPDX-License-Identifier: GPL-2.0-only

## Mod pack options panel.
##
## Scans user://mods/ for installed packs, reads/writes user://active_packs.cfg
## to persist which packs are enabled. Missing config defaults to the bundled
## starter pack; an explicitly saved empty list disables all packs.
##
## Pack loading happens in buildings.gd via load_asset_packs(). This script manages the
## config file and the UI; `Import pack…` hands share archives to pack_import.gd, and the
## per-pack Verify / Show folder / Remove buttons go through pack_actions.gd.
extends VBoxContainer

const UIStyle = preload("res://scripts/ui/ui_style.gd")
const ModPackConfig = preload("res://scripts/core/mod_pack_config.gd")
const PackImport = preload("res://scripts/ui/pack_import.gd")
const PackActions = preload("res://scripts/ui/pack_actions.gd")

const MODS_DIR := "user://mods/"

signal packs_changed
signal dirty_changed(has_pending_changes: bool)

var _checks: Dictionary = {}   # pack_id -> CheckBox
var _initial_enabled: Array = []
var _list: VBoxContainer
var _status_label: Label
var importer: PackImport
var actions: PackActions

func _ready() -> void:
	_build_ui()
	refresh()

func _build_ui() -> void:
	size_flags_horizontal = Control.SIZE_EXPAND_FILL
	size_flags_vertical = Control.SIZE_EXPAND_FILL
	add_theme_constant_override("separation", 10)

	var header := HBoxContainer.new()
	add_child(header)
	var title_lbl := Label.new()
	title_lbl.text = "Installed Packs"
	title_lbl.size_flags_horizontal = Control.SIZE_EXPAND_FILL
	UIStyle.set_font_size(title_lbl, 18)
	title_lbl.add_theme_color_override("font_color", UIStyle.TEXT_PRIMARY)
	header.add_child(title_lbl)
	importer = PackImport.new(self)
	importer.pack_in_use = _pack_in_use
	importer.installed.connect(_on_pack_installed)
	actions = PackActions.new(self)
	actions.pack_in_use = _pack_in_use
	actions.removed.connect(_on_pack_removed)
	var import_btn := Button.new()
	import_btn.name = "ImportPack"
	import_btn.text = "Import pack…"
	import_btn.pressed.connect(importer.pick)
	header.add_child(import_btn)

	var restart_lbl := Label.new()
	restart_lbl.text = "Pack selection takes effect after restarting the game."
	restart_lbl.autowrap_mode = TextServer.AUTOWRAP_WORD_SMART
	restart_lbl.add_theme_color_override("font_color", UIStyle.TEXT_DIM)
	UIStyle.set_font_size(restart_lbl, 12)
	add_child(restart_lbl)

	var scroll := ScrollContainer.new()
	scroll.size_flags_horizontal = Control.SIZE_EXPAND_FILL
	scroll.size_flags_vertical = Control.SIZE_EXPAND_FILL
	add_child(scroll)

	_list = VBoxContainer.new()
	_list.name = "PackList"
	_list.size_flags_horizontal = Control.SIZE_EXPAND_FILL
	_list.add_theme_constant_override("separation", 6)
	scroll.add_child(_list)

	_status_label = Label.new()
	_status_label.autowrap_mode = TextServer.AUTOWRAP_WORD_SMART
	_status_label.add_theme_color_override("font_color", UIStyle.TEXT_DIM)
	UIStyle.set_font_size(_status_label, 12)
	add_child(_status_label)

func refresh() -> void:
	if not _list:
		return
	_refresh_list(_list)
	_initial_enabled = _selected_pack_ids()
	_emit_dirty_state()

func has_pending_changes() -> bool:
	return _selected_pack_ids() != _initial_enabled

func apply_changes() -> Error:
	var enabled := _selected_pack_ids()
	var err := ModPackConfig.save_enabled_pack_ids(enabled)
	if err != OK:
		_status_label.text = "Could not save active pack selection."
		push_warning("Could not save active pack selection (error %d)." % err)
		return err
	_initial_enabled = enabled
	_status_label.text = "Pack selection saved. Restart required."
	emit_signal("packs_changed")
	_emit_dirty_state()
	return OK

func reset_defaults() -> void:
	var defaults := ModPackConfig.DEFAULT_ENABLED_PACK_IDS
	for pack_id in _checks:
		(_checks[pack_id] as CheckBox).button_pressed = pack_id in defaults
	_emit_dirty_state()

func _refresh_list(list: VBoxContainer) -> void:
	# Detach at once so rebuilt rows keep their pack_id names.
	for child in list.get_children():
		list.remove_child(child)
		child.queue_free()
	_checks.clear()

	var enabled := _load_enabled_packs()
	var mods_native := ProjectSettings.globalize_path(MODS_DIR)
	var dir := DirAccess.open(mods_native)
	if not dir:
		var lbl := Label.new()
		lbl.text = "No mods directory found.\nExport a pack first."
		lbl.add_theme_color_override("font_color", Color.YELLOW)
		list.add_child(lbl)
		return

	var found := false
	dir.list_dir_begin()
	var entry := dir.get_next()
	while entry != "":
		if dir.current_is_dir() and not entry.begins_with("."):
			var pack_toml_path := mods_native.path_join(entry).path_join("pack.toml")
			if FileAccess.file_exists(pack_toml_path):
				found = true
				var meta := _read_pack_meta(pack_toml_path)
				_add_pack_row(list, entry, meta, entry in enabled)
		entry = dir.get_next()
	dir.list_dir_end()

	if not found:
		var lbl := Label.new()
		lbl.text = "No packs found in mods directory."
		lbl.add_theme_color_override("font_color", Color.YELLOW)
		list.add_child(lbl)
	_status_label.text = ""

# Lists the new pack without discarding unapplied checkbox changes; a new pack starts
# disabled, a replaced one keeps its state.
func _on_pack_installed(pack_id: String, replaced: bool) -> void:
	var message := "Imported %s." % pack_id
	if not replaced and _forget_enabled(pack_id) != OK:
		message += " Could not update the active pack selection; uncheck it and apply."
	_reload(message)

func _on_pack_removed(pack_id: String) -> void:
	var message := "Moved %s to Trash." % pack_id
	if _forget_enabled(pack_id) != OK:
		message += " Could not update the active pack selection."
	_initial_enabled.erase(pack_id)
	_reload(message)

# Rebuilds the list after an install or removal without losing unapplied checkbox changes.
func _reload(message: String) -> void:
	var pending := _selected_pack_ids()
	_refresh_list(_list)
	for id in _checks:
		(_checks[id] as CheckBox).button_pressed = id in pending
	_status_label.text = message
	_emit_dirty_state()

# A deleted pack can still be listed as enabled; without this, reinstalling it would load it
# at the next start although the list shows it disabled.
func _forget_enabled(pack_id: String) -> Error:
	var enabled := ModPackConfig.load_enabled_pack_ids()
	if not pack_id in enabled:
		return OK
	enabled.erase(pack_id)
	return ModPackConfig.save_enabled_pack_ids(enabled)

# A running city has loaded its enabled packs; their files must not change underneath it.
func _pack_in_use(pack_id: String) -> bool:
	return str(get_window().get("context")) == "gameplay" and pack_id in _load_enabled_packs()

func _add_pack_row(list: VBoxContainer, pack_id: String, meta: Dictionary, enabled: bool) -> void:
	var panel := PanelContainer.new()
	panel.name = pack_id
	var style := StyleBoxFlat.new()
	style.bg_color = Color(0.15, 0.15, 0.15, 1.0)
	style.set_corner_radius_all(6)
	panel.add_theme_stylebox_override("panel", style)
	list.add_child(panel)

	var hbox := HBoxContainer.new()
	hbox.add_theme_constant_override("separation", 10)
	var pad := MarginContainer.new()
	pad.add_theme_constant_override("margin_left", 10)
	pad.add_theme_constant_override("margin_right", 10)
	pad.add_theme_constant_override("margin_top", 8)
	pad.add_theme_constant_override("margin_bottom", 8)
	pad.add_child(hbox)
	panel.add_child(pad)

	var chk := CheckBox.new()
	chk.button_pressed = enabled
	chk.toggled.connect(func(_pressed: bool): _emit_dirty_state())
	hbox.add_child(chk)
	_checks[pack_id] = chk

	var info := VBoxContainer.new()
	info.size_flags_horizontal = Control.SIZE_EXPAND_FILL
	hbox.add_child(info)

	var name_lbl := Label.new()
	name_lbl.text = meta.get("display_name", pack_id)
	UIStyle.set_font_size(name_lbl, 14)
	info.add_child(name_lbl)

	var detail := Label.new()
	var author: String = meta.get("author", "")
	var version: String = meta.get("version", "")
	detail.text = "ID: %s  |  Author: %s  |  v%s" % [pack_id, author, version]
	detail.add_theme_color_override("font_color", Color(0.7, 0.7, 0.7))
	UIStyle.set_font_size(detail, 11)
	info.add_child(detail)

	var verify := _row_button(hbox, "Verify", "Verify", "Check the files against the checksums saved when the pack was imported.")
	verify.visible = actions.can_verify(pack_id)
	verify.pressed.connect(actions.verify_dialog.bind(pack_id))
	var folder := _row_button(hbox, "ShowFolder", "Show folder", "Open the pack folder in the file manager.")
	folder.pressed.connect(actions.show_folder.bind(pack_id))
	var remove := _row_button(hbox, "Remove", "Remove…", "Move the pack to the system Trash.")
	if actions.is_bundled(pack_id):
		remove.disabled = true
		remove.tooltip_text = "Bundled with the game and restored at startup; uncheck it to disable it instead."
	remove.pressed.connect(func(): actions.remove_dialog(pack_id, func(text: String): _status_label.text = text))

func _row_button(row: HBoxContainer, node_name: String, text: String, tooltip: String) -> Button:
	var button := Button.new()
	button.name = node_name
	button.text = text
	button.tooltip_text = tooltip
	button.size_flags_vertical = Control.SIZE_SHRINK_CENTER
	row.add_child(button)
	return button

func _read_pack_meta(path: String) -> Dictionary:
	var meta: Dictionary = {}
	var f := FileAccess.open(path, FileAccess.READ)
	if not f:
		return meta
	var content := f.get_as_text()
	f.close()
	# Minimal TOML key=value parser — only reads top-level string fields.
	for line in content.split("\n"):
		var eq := line.find("=")
		if eq < 0:
			continue
		var key := line.left(eq).strip_edges()
		var val := line.right(line.length() - eq - 1).strip_edges()
		if val.begins_with("\"") and val.ends_with("\""):
			meta[key] = val.substr(1, val.length() - 2)
	return meta

func _load_enabled_packs() -> Array:
	return ModPackConfig.load_enabled_pack_ids()

func _selected_pack_ids() -> Array:
	var enabled: Array = []
	for pack_id in _checks:
		if (_checks[pack_id] as CheckBox).button_pressed:
			enabled.append(pack_id)
	enabled.sort()
	return enabled

func _emit_dirty_state() -> void:
	emit_signal("dirty_changed", has_pending_changes())
