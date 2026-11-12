# SPDX-License-Identifier: GPL-2.0-only

## The Godot test guard's refusal (`TOOLS-19` part 2, docs/debug.md "Godot Test Guard"): run.sh
## starts this script twice. In the gate's isolated profile it writes its marker into user:// and
## exits 0; in a project keeping the player's name, under a throwaway HOME whose default data
## directory stands in for the player's profile, the extension must exit non-zero before this
## script runs, so the marker is never written.
extends SceneTree

const MARKER := "user://profile_guard_marker"

func _initialize() -> void:
	var file := FileAccess.open(MARKER, FileAccess.WRITE)
	if file == null:
		push_error("profile guard test: cannot write %s" % MARKER)
		quit(1)
		return
	file.store_string("written\n")
	file.close()
	print("profile guard test: marker written in %s" % OS.get_user_data_dir())
	quit(0)
