# SPDX-License-Identifier: GPL-2.0-only

## Event-driven workspace sizing in logical UI pixels, independent of monitor resolution.
## Restore after container sorting; persist user preferences, never transient startup geometry.
extends RefCounted

const INSPECTOR_DEFAULT := 420
const INSPECTOR_MIN := 340
const INSPECTOR_MAX := 600
const LIBRARY_DEFAULT := 360
const LIBRARY_MIN := 240
const PREVIEW_MIN := 320
const LOG_DEFAULT := 180

var inspector_collapsed := false
var inspector_width := INSPECTOR_DEFAULT
var library_width := LIBRARY_DEFAULT
var log_height := LOG_DEFAULT
var pending := false
var initialized := false
var _editor: Node
var _view: RefCounted
var _dragged := ""
var _revision := 0

func configure(editor: Node, view: RefCounted) -> void:
	_editor = editor
	_view = view
	var config: ConfigFile = editor._config
	inspector_width = clampi(int(config.get_value("layout", "right_panel_width", INSPECTOR_DEFAULT)), INSPECTOR_MIN, INSPECTOR_MAX)
	library_width = clampi(int(config.get_value("layout", "left_panel_width", LIBRARY_DEFAULT)), LIBRARY_MIN, 480)
	log_height = clampi(int(config.get_value("layout", "bottom_log_height", LOG_DEFAULT)), 120, 320)
	inspector_collapsed = bool(config.get_value("layout", "inspector_collapsed", false))
	view._theme_root.resized.connect(request_layout)
	editor.get_viewport().size_changed.connect(request_layout)
	view._right_split.dragged.connect(_on_dragged.bind("inspector"))
	view._left_split.dragged.connect(_on_dragged.bind("library"))
	view._main_vsplit.dragged.connect(_on_dragged.bind("diagnostics"))
	request_layout()

func request_layout() -> void:
	if not is_instance_valid(_editor) or not _editor.is_inside_tree():
		return
	var window := _editor.get_window()
	window.min_size = Vector2i(Vector2(960, 640) * window.content_scale_factor)
	_revision += 1
	if pending:
		return
	pending = true
	_settle_layout()

func _on_dragged(_offset: int, pane: String) -> void:
	if not initialized:
		return
	_dragged = pane
	request_layout()

func _settle_layout() -> void:
	var tree := _editor.get_tree()
	# Containers queue their own sorts. One deferred call can still see zero or old sizes.
	await tree.process_frame
	await tree.process_frame
	if not is_instance_valid(_editor) or not _editor.is_inside_tree():
		return
	var revision := _revision
	match _dragged:
		"inspector": inspector_width = clampi(roundi(_view.inspector.size.x), INSPECTOR_MIN, INSPECTOR_MAX)
		"library": library_width = clampi(roundi(_view.library.size.x), LIBRARY_MIN, 480)
		"diagnostics": log_height = clampi(roundi(_view.diagnostics.size.y), 120, 320)
	_dragged = ""
	if _view.library.visible:
		var reserved := INSPECTOR_MIN if _view.inspector.visible else 0
		var maximum := maxi(LIBRARY_MIN, int(_view._left_split.size.x) - reserved - PREVIEW_MIN - 24)
		var width := mini(library_width, maximum)
		_view._left_split.clamp_split_offset()
		_view._left_split.split_offset += width - roundi(_view.library.size.x)
	await tree.process_frame
	if not is_instance_valid(_editor) or not _editor.is_inside_tree():
		return
	if revision != _revision:
		pending = false
		request_layout()
		return
	if _view.inspector.visible:
		var available: float = _view._right_split.size.x
		var maximum := maxi(INSPECTOR_MIN, mini(INSPECTOR_MAX, mini(int(available * 0.45), int(available) - PREVIEW_MIN - 12)))
		var width := mini(inspector_width, maximum)
		# SplitContainer can visually clamp a pane without updating its requested offset.
		# Normalize that offset before applying a delta from the measured pane size.
		_view._right_split.clamp_split_offset()
		_view._right_split.split_offset += roundi(_view.inspector.size.x) - width
	if _view.diagnostics.visible:
		var height := mini(log_height, maxi(120, int(_view._main_vsplit.size.y * 0.3)))
		_view._main_vsplit.clamp_split_offset()
		_view._main_vsplit.split_offset += roundi(_view.diagnostics.size.y) - height
	await tree.process_frame
	if not is_instance_valid(_editor) or not _editor.is_inside_tree():
		return
	pending = false
	initialized = true
	if revision != _revision:
		request_layout()
	else:
		save()

func reset() -> void:
	inspector_width = INSPECTOR_DEFAULT
	library_width = LIBRARY_DEFAULT
	log_height = LOG_DEFAULT
	inspector_collapsed = false
	_dragged = ""
	_view.library.hide()
	_view.diagnostics.hide()
	_view.inspector.visible = _view._document_open
	_view.update_context_actions()
	request_layout()

func save() -> void:
	if not initialized or pending or not is_instance_valid(_editor) or not _editor.is_inside_tree():
		return
	var window := _editor.get_window()
	if not is_instance_valid(window):
		return
	var config: ConfigFile = _editor._config
	config.set_value("layout", "window_width", window.size.x)
	config.set_value("layout", "window_height", window.size.y)
	config.set_value("layout", "window_x", window.position.x)
	config.set_value("layout", "window_y", window.position.y)
	config.set_value("layout", "right_panel_width", inspector_width)
	config.set_value("layout", "left_panel_width", library_width)
	config.set_value("layout", "bottom_log_height", log_height)
	config.set_value("layout", "inspector_collapsed", inspector_collapsed)
	_editor._save_config()
