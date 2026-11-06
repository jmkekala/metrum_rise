# SPDX-License-Identifier: GPL-2.0-only

## Platform names for modifier keys in UI text. macOS calls Godot's Alt modifier Option.
extends RefCounted

## "Option" on macOS, "Alt" elsewhere.
static func alt() -> String:
	return "Option" if OS.get_name() == "macOS" else "Alt"
