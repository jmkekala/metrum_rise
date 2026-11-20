# SPDX-License-Identifier: GPL-3.0-or-later

## X11's input under Godot's Wayland driver (`RENDER-62`, round W1b): the same physical input gives
## the game the same events under both Linux drivers, so the camera, the tools, the shortcuts and
## Godot's own controls act alike ("the X11 and Wayland controls should be the same", the user,
## 2026-11-19). Inert under every other display server: X11, macOS, Windows and headless keep the
## engine's own input path, with no callback and no per-frame work.
##
## Under Wayland the main window's input callback comes here instead of to the root Window, and each
## event is forwarded as `Window::_window_input` forwards it (`window_input`, then `push_input`) after
## it is given the shape Godot's X11 driver delivers under Xwayland for the same input (Godot 4.7.2's
## `platform/linuxbsd/x11` and `wayland`, Xwayland 24.1's `xwayland-input.c`; the inventory is in
## docs/terrain.md, round W1b):
## - A scroll arrives as whole wheel clicks, factor 1: one per 120 v120 units of a wheel (a
##   high-resolution wheel sends fractions of a notch) and one per 10 units of a touchpad's
##   two-finger scroll, which Wayland sends as a pan gesture. The remainder carries over, as the X
##   server carries its scroll valuator's.
## - Mouse motion's `relative` is the change of the integer pointer position, as X11 computes it.
##   Wayland's relative-pointer deltas go on where the pointer stops at the screen's edge. Motion that
##   does not move the integer position is not delivered: the X server sends none.
## - Physical keycodes take X11's names for the four keys Godot's two key maps name differently.
## - A key whose press completed a dead-key composition gets its release, which Wayland never sends.
## - A pinch, which X11 cannot deliver, still zooms (the magnify gesture, as on macOS); the pan that
##   Wayland sends with each pinch step is not delivered, as macOS sends none.
extends Node

## X11's physical keycode for each key Godot's Wayland key map names differently (by XKB keycode:
## 0x22 and 0x23 are US [ and ], Nordic Å and ¨; 0x31 is US ` and Nordic §; 0x5E is the ISO key
## left of Z).
const X11_PHYSICAL := {
	KEY_BRACELEFT: KEY_BRACKETLEFT,
	KEY_BRACERIGHT: KEY_BRACKETRIGHT,
	KEY_SECTION: KEY_QUOTELEFT,
	KEY_QUOTELEFT: KEY_SECTION,
}
## The Wayland driver's physical keycode for X11's name of the same key, for the display server's
## own lookups.
const WAYLAND_PHYSICAL := {
	KEY_BRACKETLEFT: KEY_BRACELEFT,
	KEY_BRACKETRIGHT: KEY_BRACERIGHT,
	KEY_QUOTELEFT: KEY_SECTION,
	KEY_SECTION: KEY_QUOTELEFT,
}
# InputEvent::DEVICE_ID_INTERNAL, which Window::_window_input keeps out of `window_input`.
const DEVICE_ID_INTERNAL := -2

var _normalizer: Normalizer

func _ready() -> void:
	if DisplayServer.get_name() == "Wayland":
		install()

## Takes the main window's input callback from the root Window. Called under Wayland only, and by
## the parity test (tests/wayland_input_test.gd) under the headless driver.
func install() -> void:
	_normalizer = Normalizer.new(_forward, Input.parse_input_event)
	DisplayServer.window_set_input_event_callback(_on_window_input, DisplayServer.MAIN_WINDOW_ID)

## The display server's physical keycode for `key`, named as X11 names it: what
## `DisplayServer.keyboard_get_label_from_physical` expects under the driver in use.
static func display_physical(key: Key) -> Key:
	if DisplayServer.get_name() == "Wayland":
		return WAYLAND_PHYSICAL.get(key, key)
	return key

func _on_window_input(event: InputEvent) -> void:
	_normalizer.feed(event, Input.get_mouse_button_mask(), Input.mouse_mode == Input.MOUSE_MODE_CAPTURED)

# What Window::_window_input does with an event of the main window, which embeds its subwindows
# (no exclusive native child to defer to).
func _forward(event: InputEvent) -> void:
	var root := get_tree().root
	if event.device != DEVICE_ID_INTERNAL:
		root.window_input.emit(event)
	root.push_input(event)

## Turns Godot's Wayland events into the events its X11 driver delivers for the same input. Each
## result goes to `deliver` (the window) or, for a key release, to `reparse` (Input, which keeps the
## pressed-key state the release must clear).
class Normalizer extends RefCounted:
	# One wheel click in a common integer unit: 120 v120 units (a notch), or 10 surface units of a
	# continuous scroll, which is 2,560 in wl_fixed's 1/256 steps.
	const CLICK := 120 * 2560
	const V120_UNIT := 2560
	const FIXED_UNIT := 120

	var _deliver: Callable
	var _reparse: Callable
	# The scroll not yet delivered as a click, per axis (x horizontal, y vertical), in CLICK units.
	var _scroll := Vector2i.ZERO
	var _pointer := Vector2.ZERO
	var _pointer_known := false
	var _last_gesture_id := 0
	var _after_magnify := false

	func _init(deliver: Callable, reparse: Callable) -> void:
		_deliver = deliver
		_reparse = reparse

	## One event from the Wayland driver. `held` is Input's mouse button mask, `captured` whether
	## the mouse is captured (the game never captures it; captured motion passes as Wayland gives it).
	func feed(event: InputEvent, held: int, captured: bool) -> void:
		# Input has already parsed this event, so a wheel press is in its mask; X11's wheel events
		# carry the buttons held besides (MOUSE_BUTTON_MASK of wheel up to wheel right).
		held &= ~0x78
		if event is InputEventGesture:
			# Input dispatches every gesture twice, the same object both times (input.cpp, the
			# gesture branch and the generic dispatch after it).
			var id := event.get_instance_id()
			if id == _last_gesture_id:
				return
			_last_gesture_id = id
		var pinch_pan := _after_magnify and event is InputEventPanGesture
		_after_magnify = event is InputEventMagnifyGesture
		if event is InputEventMouseMotion:
			_motion(event, captured)
		elif event is InputEventMouseButton:
			_button(event, held)
		elif event is InputEventPanGesture:
			# The pinch's own pan follows its magnify step; a touchpad scroll is a pan alone.
			if not pinch_pan:
				_scroll_by(Vector2i(roundi(event.delta.x * 256.0), roundi(event.delta.y * 256.0)) * FIXED_UNIT, event, held)
		elif event is InputEventKey:
			_key(event)
		else:
			_deliver.call(event)

	func _motion(event: InputEventMouseMotion, captured: bool) -> void:
		if captured:
			_deliver.call(event)
			return
		var position := event.position.floor()
		if _pointer_known and position == _pointer:
			return
		# X11's first motion has nothing to compare with and moves by nothing.
		var relative := position - _pointer if _pointer_known else Vector2.ZERO
		_pointer = position
		_pointer_known = true
		event.position = position
		event.global_position = event.global_position.floor()
		event.relative = relative
		event.screen_relative = relative
		_deliver.call(event)

	func _button(event: InputEventMouseButton, held: int) -> void:
		var index := event.button_index
		if index < MOUSE_BUTTON_WHEEL_UP or index > MOUSE_BUTTON_WHEEL_RIGHT:
			event.position = event.position.floor()
			event.global_position = event.global_position.floor()
			_deliver.call(event)
			return
		# Wayland follows each wheel press with its own release; each click below brings its own.
		if not event.pressed:
			return
		# The factor is the frame's v120 over 120 (wayland_thread.cpp, _wl_pointer_on_frame).
		var amount := roundi(event.factor * 120.0) * V120_UNIT
		if index == MOUSE_BUTTON_WHEEL_UP or index == MOUSE_BUTTON_WHEEL_LEFT:
			amount = -amount
		var vertical := index == MOUSE_BUTTON_WHEEL_UP or index == MOUSE_BUTTON_WHEEL_DOWN
		_scroll_by(Vector2i(0, amount) if vertical else Vector2i(amount, 0), event, held)

	# Positive is down and right, as Wayland's axes and X11's buttons 5 and 7.
	func _scroll_by(amount: Vector2i, source: InputEventWithModifiers, held: int) -> void:
		_scroll += amount
		while _scroll.y >= CLICK:
			_scroll.y -= CLICK
			_click(MOUSE_BUTTON_WHEEL_DOWN, source, held)
		while _scroll.y <= -CLICK:
			_scroll.y += CLICK
			_click(MOUSE_BUTTON_WHEEL_UP, source, held)
		while _scroll.x >= CLICK:
			_scroll.x -= CLICK
			_click(MOUSE_BUTTON_WHEEL_RIGHT, source, held)
		while _scroll.x <= -CLICK:
			_scroll.x += CLICK
			_click(MOUSE_BUTTON_WHEEL_LEFT, source, held)

	# X11's wheel click: a press whose mask adds the wheel's bit to the held buttons, then a release.
	func _click(index: MouseButton, source: InputEventWithModifiers, held: int) -> void:
		var press := InputEventMouseButton.new()
		press.window_id = source.window_id
		press.shift_pressed = source.shift_pressed
		press.ctrl_pressed = source.ctrl_pressed
		press.alt_pressed = source.alt_pressed
		press.meta_pressed = source.meta_pressed
		press.position = source.position.floor()
		press.global_position = press.position
		press.button_index = index
		press.button_mask = held | (1 << (index - 1))
		press.pressed = true
		var release: InputEventMouseButton = press.duplicate()
		release.button_mask = held
		release.pressed = false
		_deliver.call(press)
		_deliver.call(release)

	func _key(event: InputEventKey) -> void:
		# A composed press carries the composition's character, not the key's own: its label is
		# still the key's (wayland_thread.cpp, _seat_state_handle_xkb_keycode), and its release is
		# dropped while xkb's compose state stays COMPOSED. X11 delivers that release.
		var release: InputEventKey = null
		if event.pressed and not event.echo and event.unicode >= 0x20 \
				and event.key_label != String.chr(event.unicode).to_upper().unicode_at(0):
			release = event.duplicate()
			release.pressed = false
			release.unicode = 0
		event.physical_keycode = X11_PHYSICAL.get(event.physical_keycode, event.physical_keycode)
		_deliver.call(event)
		if release != null:
			_reparse.call(release)
