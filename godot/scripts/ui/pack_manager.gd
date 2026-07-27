## Mod pack manager popup.
##
## Scans user://mods/ for installed packs, reads/writes user://active_packs.cfg
## to persist which packs are enabled. Missing config defaults to the bundled
## starter pack; an explicitly saved empty list disables all packs.
##
## No Rust methods called directly — pack loading happens in buildings.gd via
## load_asset_packs(). This script only manages the config file and the UI.
extends Window

const WindowResizeHandles = preload("res://scripts/ui/window_resize_handles.gd")
const ModPackConfig = preload("res://scripts/core/mod_pack_config.gd")

const MODS_DIR := "user://mods/"

signal packs_changed

var _checks: Dictionary = {}   # pack_id -> CheckBox

func _ready() -> void:
	title = "Mod Packs"
	size = Vector2(480, 400)
	min_size = Vector2i(360, 280)
	unresizable = false
	exclusive = true
	close_requested.connect(hide)

	var margin := MarginContainer.new()
	margin.set_anchors_and_offsets_preset(Control.PRESET_FULL_RECT)
	margin.add_theme_constant_override("margin_left", 16)
	margin.add_theme_constant_override("margin_right", 16)
	margin.add_theme_constant_override("margin_top", 16)
	margin.add_theme_constant_override("margin_bottom", 16)
	add_child(margin)

	var vbox := VBoxContainer.new()
	vbox.add_theme_constant_override("separation", 10)
	margin.add_child(vbox)

	var title_lbl := Label.new()
	title_lbl.text = "Installed Packs"
	title_lbl.add_theme_font_size_override("font_size", 16)
	vbox.add_child(title_lbl)

	var scroll := ScrollContainer.new()
	scroll.size_flags_vertical = Control.SIZE_EXPAND_FILL
	vbox.add_child(scroll)

	var list := VBoxContainer.new()
	list.name = "PackList"
	list.size_flags_horizontal = Control.SIZE_EXPAND_FILL
	list.add_theme_constant_override("separation", 6)
	scroll.add_child(list)

	var btn_row := HBoxContainer.new()
	btn_row.alignment = BoxContainer.ALIGNMENT_END
	btn_row.add_theme_constant_override("separation", 8)
	vbox.add_child(btn_row)

	var apply_btn := Button.new()
	apply_btn.text = "Apply & Restart Required"
	apply_btn.pressed.connect(_on_apply_pressed)
	btn_row.add_child(apply_btn)

	var close_btn := Button.new()
	close_btn.text = "Close"
	close_btn.pressed.connect(hide)
	btn_row.add_child(close_btn)

	_refresh_list(list)
	WindowResizeHandles.install(self)

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
	hbox.add_child(chk)
	_checks[pack_id] = chk

	var info := VBoxContainer.new()
	info.size_flags_horizontal = Control.SIZE_EXPAND_FILL
	hbox.add_child(info)

	var name_lbl := Label.new()
	name_lbl.text = meta.get("display_name", pack_id)
	name_lbl.add_theme_font_size_override("font_size", 14)
	info.add_child(name_lbl)

	var detail := Label.new()
	var author: String = meta.get("author", "")
	var version: String = meta.get("version", "")
	detail.text = "ID: %s  |  Author: %s  |  v%s" % [pack_id, author, version]
	detail.add_theme_color_override("font_color", Color(0.7, 0.7, 0.7))
	detail.add_theme_font_size_override("font_size", 11)
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

func _on_apply_pressed() -> void:
	var enabled: Array = []
	for pack_id in _checks:
		if (_checks[pack_id] as CheckBox).button_pressed:
			enabled.append(pack_id)
	enabled.sort()
	var err := ModPackConfig.save_enabled_pack_ids(enabled)
	if err != OK:
		push_warning("Could not save active pack selection (error %d)." % err)
	emit_signal("packs_changed")
	# Inform the user a restart is needed for changes to take effect.
	var dialog := AcceptDialog.new()
	dialog.dialog_text = "Pack selection saved.\nRestart the game to apply changes."
	add_child(dialog)
	dialog.popup_centered()
	dialog.confirmed.connect(dialog.queue_free)
