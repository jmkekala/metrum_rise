# SPDX-License-Identifier: GPL-3.0-or-later

## Headless entry of the model draft pipeline (`TOOLS-33`, docs/asset_editor.md "Draft command").
## Only `tools/model_draft.py` launches it, in a throwaway profile, with the user arguments
## `<request.json> <report.json>`. It hands the request to `AssetAuthoringFiles.draft_from_model`,
## where every rule lives, writes the JSON report and exits with the draft's status.
extends SceneTree

func _initialize() -> void:
	var args := OS.get_cmdline_user_args()
	if args.size() != 2:
		push_error("model_draft.gd takes <request.json> <report.json>")
		quit(1)
		return
	var result: Dictionary = AssetAuthoringFiles.draft_from_model(FileAccess.get_file_as_string(args[0]))
	var file := FileAccess.open(args[1], FileAccess.WRITE)
	if file == null:
		push_error("model_draft.gd cannot write %s" % args[1])
		quit(1)
		return
	file.store_string(str(result["report"]))
	file.close()
	quit(int(result["exit"]))
