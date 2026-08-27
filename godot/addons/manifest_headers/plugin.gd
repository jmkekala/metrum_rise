# =========================================================================
#  MANIFEST
# =========================================================================
#  script_name: plugin.gd
#  script_path: addons/manifest_headers/plugin.gd
#  module_name: plugin
#  version: 1.0.0
#  description: The editor plugin. Adds a Manifest dock with three buttons:
#           check reports what would change and writes nothing, clean up
#           fixes it, and the field editor declares the custom fields a
#           project wants tracked. The dock exists because a tool nobody
#           can find is a tool nobody runs, and a header pass that has to
#           be remembered is a header pass that rots.
#  kind: module
#  spec: none
#  internal_dependencies: [addons/manifest_headers/manifest.gd]
#  external_dependencies: [Godot 4.x]
#  features: [editor-dock, check-mode, cleanup-pass, custom-field-editor]
#  api_version: metrum-v1.0.0
#  last_updated: 2026-08-24
# =========================================================================

@tool
extends EditorPlugin

const ManifestHeaders := preload("res://addons/manifest_headers/manifest.gd")
const CONFIG_PATH := "res://addons/manifest_headers/manifest_fields.cfg"

var _dock: Control
var _log: RichTextLabel
var _scope: LineEdit
var _fields: LineEdit


func _enter_tree() -> void:
	_dock = _build_dock()
	add_control_to_dock(DOCK_SLOT_RIGHT_BL, _dock)


func _exit_tree() -> void:
	if _dock:
		remove_control_from_docks(_dock)
		_dock.queue_free()
		_dock = null


# =========================================================================
# THE DOCK
# =========================================================================

func _build_dock() -> Control:
	var root := VBoxContainer.new()
	root.name = "Manifest"
	root.add_theme_constant_override("separation", 6)

	var title := Label.new()
	title.text = "Manifest Headers"
	title.add_theme_font_size_override("font_size", 15)
	root.add_child(title)

	var scope_label := Label.new()
	scope_label.text = "Scope (res:// path)"
	root.add_child(scope_label)

	_scope = LineEdit.new()
	_scope.text = "res://"
	_scope.tooltip_text = "Folder to walk. Defaults to the whole project."
	root.add_child(_scope)

	var fields_label := Label.new()
	fields_label.text = "Custom fields (comma separated)"
	root.add_child(fields_label)

	_fields = LineEdit.new()
	_fields.placeholder_text = "author, reviewed_by, ticket"
	_fields.tooltip_text = (
		"Extra fields tracked in every header this tool writes.\n"
		+ "Saved to manifest_fields.cfg and applied on the next pass."
	)
	_fields.text = _current_custom_fields()
	root.add_child(_fields)

	var save_fields := Button.new()
	save_fields.text = "Save fields"
	save_fields.pressed.connect(_on_save_fields)
	root.add_child(save_fields)

	root.add_child(HSeparator.new())

	var check := Button.new()
	check.text = "Check (writes nothing)"
	check.pressed.connect(_on_check)
	root.add_child(check)

	var clean := Button.new()
	clean.text = "Clean up"
	clean.tooltip_text = (
		"Injects missing headers, standardises existing ones, and expands\n"
		+ "any bare HEADER marker into a full divider."
	)
	clean.pressed.connect(_on_clean)
	root.add_child(clean)

	root.add_child(HSeparator.new())

	_log = RichTextLabel.new()
	_log.custom_minimum_size = Vector2(0, 260)
	_log.size_flags_vertical = Control.SIZE_EXPAND_FILL
	_log.scroll_following = true
	_log.bbcode_enabled = true
	root.add_child(_log)

	return root


# =========================================================================
# ACTIONS
# =========================================================================

func _on_check() -> void:
	_run(true)


func _on_clean() -> void:
	_run(false)


func _run(check_only: bool) -> void:
	_log.clear()
	var root := _scope.text.strip_edges()
	if root == "":
		root = "res://"
	if not DirAccess.dir_exists_absolute(root):
		_log.append_text("[color=#e06c75]No such folder: %s[/color]\n" % root)
		return

	var cfg := ManifestHeaders.load_config(CONFIG_PATH)
	var files := ManifestHeaders.collect(root, [] as Array[String])
	var changed := 0
	var injected := 0
	var markers := 0
	var errors := 0

	for path in files:
		var r: Dictionary = ManifestHeaders.process_file(path, cfg, check_only)
		if String(r.error) != "":
			_log.append_text("[color=#e06c75]error  %s: %s[/color]\n" % [r.path, r.error])
			errors += 1
			continue
		if bool(r.changed):
			changed += 1
			if bool(r.injected):
				injected += 1
			markers += int(r.markers)
			var verb := "would fix" if check_only else "fixed"
			_log.append_text("%s  %s\n" % [verb, r.path])

	_log.append_text("\n[b]%d scanned, %d %s[/b]\n" % [
		files.size(),
		changed,
		"need work" if check_only else "rewritten",
	])
	_log.append_text("%d header%s injected, %d marker%s expanded" % [
		injected,
		"" if injected == 1 else "s",
		markers,
		"" if markers == 1 else "s",
	])
	if errors > 0:
		_log.append_text("\n[color=#e06c75]%d error%s[/color]" % [
			errors, "" if errors == 1 else "s",
		])
	if not check_only and changed > 0:
		EditorInterface.get_resource_filesystem().scan()


func _on_save_fields() -> void:
	var file := ConfigFile.new()
	file.load(CONFIG_PATH)

	var declared: Array[String] = []
	for part in _fields.text.split(",", false):
		var t := String(part).strip_edges()
		if t != "":
			declared.append(t)

	file.set_value("manifest", "custom_fields", declared)
	if not file.has_section_key("manifest", "api_version"):
		file.set_value("manifest", "api_version", "")
	if not file.has_section_key("manifest", "list_fields"):
		file.set_value("manifest", "list_fields", [] as Array[String])

	var err := file.save(CONFIG_PATH)
	_log.clear()
	if err == OK:
		_log.append_text("Saved %d custom field%s to manifest_fields.cfg\n" % [
			declared.size(), "" if declared.size() == 1 else "s",
		])
		for d in declared:
			_log.append_text("  %s\n" % d)
		_log.append_text("\nThey are written into every header the next pass touches.")
	else:
		_log.append_text("[color=#e06c75]Could not save: %s[/color]" % error_string(err))


func _current_custom_fields() -> String:
	var cfg := ManifestHeaders.load_config(CONFIG_PATH)
	return ", ".join(PackedStringArray(cfg.custom_fields))
