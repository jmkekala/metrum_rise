# SPDX-License-Identifier: GPL-3.0-or-later

## Prints the Godot engine's licence, the copyright notices of its bundled third-party
## components and the licence texts those notices name, for the release packages'
## `THIRD-PARTY-LICENSES.txt` (docs/release.md). The engine carries this text itself, so the
## output always matches the Godot version that exported the game. Run with the pinned editor:
## `godot --headless --script tools/ci/godot_licenses.gd -- <output-file>`.
extends SceneTree

const RULE := "=============================================================================="


func _init() -> void:
	var args := OS.get_cmdline_user_args()
	if args.size() != 1:
		printerr("usage: godot --headless --script godot_licenses.gd -- <output-file>")
		quit(2)
		return
	var lines := PackedStringArray()
	lines.append("Godot Engine %s" % Engine.get_version_info()["string"])
	lines.append("")
	lines.append(Engine.get_license_text())
	lines.append(RULE)
	lines.append("")
	lines.append("Third-party components in the Godot Engine")
	lines.append("")
	for component: Dictionary in Engine.get_copyright_info():
		lines.append(component["name"])
		for part: Dictionary in component["parts"]:
			for holder: String in part["copyright"]:
				lines.append("  (c) %s" % holder)
			lines.append("  License: %s" % part["license"])
		lines.append("")
	var licenses := Engine.get_license_info()
	var names := licenses.keys()
	names.sort()
	for license_name: String in names:
		lines.append(RULE)
		lines.append("")
		lines.append(license_name)
		lines.append("")
		lines.append(licenses[license_name])
		lines.append("")
	var file := FileAccess.open(args[0], FileAccess.WRITE)
	if file == null:
		printerr("cannot write %s: %s" % [args[0], error_string(FileAccess.get_open_error())])
		quit(1)
		return
	file.store_string("\n".join(lines))
	file.close()
	print("%s: %d components, %d licence texts" % [args[0], Engine.get_copyright_info().size(), names.size()])
	quit(0)
