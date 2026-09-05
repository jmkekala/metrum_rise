# SPDX-License-Identifier: GPL-2.0-only

## Mod pack options panel.
##
## Scans user://mods/ for installed packs, reads/writes user://active_packs.cfg
## to persist which packs are enabled. Missing config defaults to the bundled
## starter pack; an explicitly saved empty list disables all packs.
##
## No Rust methods called directly — pack loading happens in buildings.gd via
## load_asset_packs(). This script only manages the config file and the UI.
extends VBoxContainer

const UIStyle = preload("res://scripts/ui/ui_style.gd")
const ModPackConfig = preload("res://scripts/core/mod_pack_config.gd")

const MODS_DIR := "user://mods/"

signal packs_changed
signal dirty_changed(has_pending_changes: bool)

var _checks: Dictionary = {}   # pack_id -> CheckBox
var _initial_enabled: Array = []
var _list: VBoxContainer
var _status_label: Label

func _ready() -> void:
	_build_ui()
	refresh()

func _build_ui() -> void:
	size_flags_horizontal = Control.SIZE_EXPAND_FILL
	size_flags_vertical = Control.SIZE_EXPAND_FILL
	add_theme_constant_override("separation", 10)

	var title_lbl := Label.new()
	title_lbl.text = "Installed Packs"
	UIStyle.set_font_size(title_lbl, 18)
	title_lbl.add_theme_color_override("font_color", UIStyle.TEXT_PRIMARY)
	add_child(title_lbl)

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
	for child in list.get_children():
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

func _add_pack_row(list: VBoxContainer, pack_id: String, meta: Dictionary, enabled: bool) -> void:
	var panel := PanelContainer.new()
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
