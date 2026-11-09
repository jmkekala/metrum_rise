# SPDX-License-Identifier: GPL-2.0-only

## Platform names for keys in UI text. macOS calls Godot's Alt modifier Option; a key bound by
## position is named as the player's layout prints it.
extends RefCounted

## "Option" on macOS, "Alt" elsewhere.
static func alt() -> String:
	return "Option" if OS.get_name() == "macOS" else "Alt"

## The character the player's layout prints on the key at US position `physical`: KEY_BRACKETLEFT
## is "[" on a US layout and "Å" on a Nordic one. The US character where the display server has
## no layout (headless) or the key prints none.
static func physical(key: Key) -> String:
	if DisplayServer.get_name() == "headless":
		return String.chr(key)
	var label := DisplayServer.keyboard_get_label_from_physical(key)
	return String.chr(label).to_upper() if label > 0 and label < KEY_SPECIAL else String.chr(key)
