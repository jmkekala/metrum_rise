# SPDX-License-Identifier: GPL-2.0-only

## Fixed-resolution thumbnail capture for the asset editor.
## The published image backs both the editor list and in-game asset detail views, so its size is a
## content contract and must not follow the window: framing crops, then one resize normalises.
extends RefCounted

const Frame = preload("res://scripts/editors/asset_editor/thumbnail_frame.gd")
const Spacing = preload("res://scripts/editors/asset_editor/editor_spacing.gd")

## Published thumbnail size. Identical for every asset regardless of window or monitor.
const OUTPUT_SIZE := Vector2i(1024, 768)
## Lossy WebP holds render gradients cleanly at roughly a thirtieth of the equivalent PNG.
const WEBP_QUALITY := 0.9
const FILE_NAME := "thumbnail.webp"

var _editor: Node
var _overlay: Control
var _actions: Control
var _path := ""
var _capturing := false

func _init(editor: Node) -> void:
	_editor = editor

## Build the framing overlay and its actions inside the preview pane.
func build(pane: Control) -> void:
	_overlay = Frame.new()
	_overlay.set_anchors_and_offsets_preset(Control.PRESET_FULL_RECT)
	_overlay.mouse_filter = Control.MOUSE_FILTER_IGNORE
	_overlay.hide()
	pane.add_child(_overlay)
	# Buttons sit inside the pane so framing and confirming stay in one place, and they keep
	# MOUSE_FILTER_STOP so a confirming click never reaches the orbit camera underneath.
	_actions = HBoxContainer.new()
	_actions.set_anchors_and_offsets_preset(Control.PRESET_CENTER_BOTTOM)
	_actions.grow_horizontal = Control.GROW_DIRECTION_BOTH
	_actions.grow_vertical = Control.GROW_DIRECTION_BEGIN
	_actions.offset_bottom = -Spacing.CONTENT_PADDING
	_actions.hide()
	pane.add_child(_actions)
	_button("Take snapshot", capture)
	_button("Cancel", cancel)

## Enter framing mode. The camera stays live so the shot can be composed before capturing.
func begin() -> void:
	if not _editor._session.has_document or _capturing:
		return
	if DisplayServer.get_name() == "headless":
		_editor._session.message("Thumbnail capture needs a rendered preview.")
		return
	_overlay.activate(OUTPUT_SIZE)
	_overlay.show()
	_actions.show()
	_status("Frame the asset, then take the snapshot. Output is %d × %d." % [OUTPUT_SIZE.x, OUTPUT_SIZE.y])

## Leave framing mode without capturing.
func cancel() -> void:
	if _overlay == null:
		return
	_overlay.clear()
	_overlay.hide()
	_actions.hide()

## Capture the framed region and normalise it to [constant OUTPUT_SIZE].
## Awaitable and usable without framing mode, which is how regression tests drive it.
func capture() -> void:
	var session = _editor._session
	if not session.has_document or _capturing:
		return
	if DisplayServer.get_name() == "headless":
		session.message("Thumbnail capture needs a rendered preview.")
		return
	_capturing = true
	var region := _capture_region()
	var process_mode: Node.ProcessMode = _editor.process_mode
	# Freeze interaction/preview updates for the capture frame so guides cannot reappear.
	_editor.process_mode = Node.PROCESS_MODE_DISABLED
	var preview_state: Dictionary = _editor._preview.begin_thumbnail_capture()
	var visibility := {}
	for overlay in [_editor._view.picking_overlay, _editor._view.hover_label,
		_editor._view._selection_rect_overlay, _overlay, _actions]:
		visibility[overlay] = overlay.visible
		overlay.hide()
	await RenderingServer.frame_post_draw
	var image: Image = _editor.get_viewport().get_texture().get_image()
	# Restore before file I/O, including its failure paths, or document-change callbacks.
	_editor._preview.end_thumbnail_capture(preview_state)
	for overlay: Control in visibility:
		overlay.visible = visibility[overlay]
	_editor.process_mode = process_mode
	_capturing = false
	image = image.get_region(region.intersection(Rect2i(Vector2i.ZERO, image.get_size())))
	if image.is_empty():
		session.message("Thumbnail capture needs a visible preview pane.")
		return
	# A pane narrower than the output would only be upscaled, so say so rather than ship mush.
	var stretched := image.get_width() < OUTPUT_SIZE.x
	image.convert(Image.FORMAT_RGB8)
	image.resize(OUTPUT_SIZE.x, OUTPUT_SIZE.y, Image.INTERPOLATE_LANCZOS)
	var directory := ProjectSettings.globalize_path("user://asset_drafts/thumbnails")
	DirAccess.make_dir_recursive_absolute(directory)
	var path := directory.path_join("%d-%d.webp" % [OS.get_process_id(), Time.get_ticks_usec()])
	var error := image.save_webp(path, true, WEBP_QUALITY)
	if error != OK:
		session.message("Could not save thumbnail: " + error_string(error))
		return
	var next: Dictionary = session.document.snapshot()
	next["params"]["thumbnail"] = FILE_NAME
	next["thumbnail_source"] = path
	session.document.apply(next, "Capture thumbnail")
	load_image(path)
	cancel()
	if stretched:
		_status("Preview pane is narrower than %d px; the thumbnail was upscaled." % OUTPUT_SIZE.x)

## Mirror the document's thumbnail into the inspector preview.
func load_image(path: String) -> void:
	_path = path
	_editor._view.thumbnail.texture = null
	if path.is_empty() or not FileAccess.file_exists(path):
		return
	var image := Image.load_from_file(path)
	if image != null:
		_editor._view.thumbnail.texture = ImageTexture.create_from_image(image)

## Source path of the image currently mirrored into the inspector.
func path() -> String:
	return _path

# The pane rect, not the overlay's, so the region is correct even when framing was never entered.
func _capture_region() -> Rect2i:
	var pane: Control = _editor._view._preview_view_rect
	var local := Frame.frame_in(pane.size, float(OUTPUT_SIZE.x) / float(OUTPUT_SIZE.y))
	return Rect2i(Rect2(pane.get_global_rect().position + local.position, local.size))

func _button(title: String, action: Callable) -> void:
	var control := Button.new()
	control.text = title
	control.pressed.connect(action)
	_actions.add_child(control)

func _status(text: String) -> void:
	if _editor._view.status != null:
		_editor._view.status.text = text
