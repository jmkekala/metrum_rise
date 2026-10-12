# SPDX-License-Identifier: GPL-2.0-only

## Copies warnings and errors the engine already reports into the always-on `events` channel.
##
## Covers `push_warning`/`push_error`, `godot_warn!`/`godot_error!`, script and shader errors,
## and `printerr`. Godot may call a logger from any thread at the same time, so this keeps no
## state and forwards to `MetrumDebug.log`, which is thread-safe. `events` is buffer-only: the
## engine has already printed the original message.
extends Logger

const _TYPE_LABELS := ["error", "warning", "script_error", "shader_error"]

func _log_error(
	function: String,
	file: String,
	line: int,
	code: String,
	rationale: String,
	_editor_notify: bool,
	error_type: int,
	script_backtraces: Array[ScriptBacktrace]
) -> void:
	var label: String = _TYPE_LABELS[error_type] if error_type >= 0 and error_type < _TYPE_LABELS.size() else "error"
	var text := rationale if not rationale.is_empty() else code
	# A script call site says more than the engine's push_warning/push_error location.
	for backtrace in script_backtraces:
		if backtrace != null and backtrace.get_frame_count() > 0:
			file = backtrace.get_frame_file(0)
			line = backtrace.get_frame_line(0)
			function = backtrace.get_frame_function(0)
			break
	MetrumDebug.log("events", "%s %s (%s:%d %s)" % [label, text, file, line, function])

func _log_message(message: String, error: bool) -> void:
	if error:
		MetrumDebug.log("events", "stderr " + message.strip_edges())
