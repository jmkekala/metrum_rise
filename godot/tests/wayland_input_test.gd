# SPDX-License-Identifier: GPL-2.0-only

## X11 and Wayland input parity (`RENDER-62`, round W1b). Each physical input is fed as the event
## shapes Godot 4.7.2's X11 driver (under Xwayland) and its Wayland driver deliver for it, read from
## their sources (the inventory in docs/terrain.md, round W1b). The X11 shapes go through the engine's
## own window callback, as on `dev`; the Wayland shapes through scripts/core/wayland_input.gd
## installed on that callback. The test asserts that the game's handlers see the same events and act
## alike: the camera (input_manager.gd), the vegetation brush's wheel gestures, the debug console's
## toggle key, the roundabout tool's size keys and Input's held keys. Headless, no Rust world beyond
## a blank one for the camera.
extends SceneTree

const WaylandInput := preload("res://scripts/core/wayland_input.gd")
const DebugConsole := preload("res://scripts/ui/debug_console/debug_console.gd")
const VegetationTool := preload("res://scripts/tools/vegetation_tool.gd")

## Sees every event before the game's handlers (the last node gets _input first) and records it
## and what the tools whose input paths read it would do.
class Probe extends Node:
	var seen: Array[String] = []
	var actions: Array[String] = []
	var brush: Node3D
	var sign: Callable
	func _input(event: InputEvent) -> void:
		seen.append(sign.call(event))
		if brush.apply_brush_gesture(event):
			actions.append("brush %.4f option %d" % [brush.radius, brush.option_index])
		var key := event as InputEventKey
		if key != null and key.pressed and not key.echo:
			if DebugConsole._is_toggle_key(key):
				actions.append("console toggle")
			# roundabout_tool.gd's size keys, matched by physical keycode.
			if key.physical_keycode in [KEY_BRACKETLEFT, KEY_BRACKETRIGHT]:
				actions.append("roundabout size %d" % key.physical_keycode)

var failures := 0
var camera: CameraNode
var probe: Probe

func _initialize() -> void:
	call_deferred("_run")

func _expect(condition: bool, message: String) -> void:
	if not condition:
		failures += 1
		push_error(message)

## The fields a handler can read, as one comparable string.
static func signature(e: InputEvent) -> String:
	var parts: Array = [e.get_class(), e.device]
	if e is InputEventWithModifiers:
		parts.append_array([e.shift_pressed, e.ctrl_pressed, e.alt_pressed, e.meta_pressed])
	if e is InputEventMouse:
		parts.append_array([e.position, e.global_position, e.button_mask])
	if e is InputEventMouseButton:
		parts.append_array([e.button_index, e.pressed, e.factor, e.double_click])
	if e is InputEventMouseMotion:
		parts.append_array([e.relative, e.screen_relative])
	if e is InputEventKey:
		parts.append_array([e.keycode, e.physical_keycode, e.key_label, e.unicode, e.pressed, e.echo, e.location])
	if e is InputEventGesture:
		parts.append(e.position)
	if e is InputEventMagnifyGesture:
		parts.append(e.factor)
	if e is InputEventPanGesture:
		parts.append(e.delta)
	return str(parts)

static func _button(index: MouseButton, pressed: bool, position: Vector2, mask: int, factor := 1.0, ctrl := false) -> InputEventMouseButton:
	var e := InputEventMouseButton.new()
	e.button_index = index
	e.pressed = pressed
	e.position = position
	e.global_position = position
	e.button_mask = mask
	e.factor = factor
	e.ctrl_pressed = ctrl
	return e

static func _motion(position: Vector2, relative: Vector2, mask: int) -> InputEventMouseMotion:
	var e := InputEventMouseMotion.new()
	e.position = position
	e.global_position = position
	e.relative = relative
	e.screen_relative = relative
	e.button_mask = mask
	return e

static func _pan(delta: Vector2, position: Vector2, ctrl := false) -> InputEventPanGesture:
	var e := InputEventPanGesture.new()
	e.delta = delta
	e.position = position
	e.ctrl_pressed = ctrl
	return e

static func _key(keycode: int, physical: int, label: int, unicode: int, pressed: bool, echo := false) -> InputEventKey:
	var e := InputEventKey.new()
	e.keycode = keycode
	e.physical_keycode = physical
	e.key_label = label
	e.unicode = unicode if pressed else 0
	e.pressed = pressed
	e.echo = echo
	return e

# X11's wheel click: press with the wheel's bit beside the held buttons, then the release
# (display_server_x11.cpp, ButtonPress and ButtonRelease).
static func _x11_click(index: MouseButton, position: Vector2, held := 0, ctrl := false) -> Array:
	var bit := 1 << (index - 1)
	return [_button(index, true, position, held | bit, 1.0, ctrl), _button(index, false, position, held, 1.0, ctrl)]

# Wayland's wheel event: a press whose factor is the frame's v120 over 120, its mask the pressed
# buttons with the wheel's bit, and an immediate release without modifiers or factor
# (wayland_thread.cpp, _wl_pointer_on_frame).
static func _wayland_wheel(index: MouseButton, v120: int, position: Vector2, held := 0, ctrl := false) -> Array:
	var bit := 1 << (index - 1)
	return [_button(index, true, position, held | bit, v120 / 120.0, ctrl), _button(index, false, position, held)]

func _repeat(events: Array, times: int) -> Array:
	var all := []
	for i in times:
		for e: InputEvent in events:
			all.append(e.duplicate())
	return all

# Each scenario: a name, X11's events, Wayland's events, and whether the game must see the same
# events (false where X11 delivers an extra event no handler acts on, or Wayland delivers an input
# X11 cannot).
func _scenarios() -> Array:
	var at := Vector2(400.6, 300.3)
	var x11_at := at.floor()
	var middle := MOUSE_BUTTON_MASK_MIDDLE
	var list := []
	# A wheel notch (v120 120): one click either way.
	list.append(["wheel notch", _x11_click(MOUSE_BUTTON_WHEEL_UP, x11_at),
		_wayland_wheel(MOUSE_BUTTON_WHEEL_UP, 120, at), true])
	# A high-resolution wheel: eight 15-unit events per notch; 1.5 notches give one click, the
	# half notch carried to the next one.
	list.append(["high-resolution wheel", _x11_click(MOUSE_BUTTON_WHEEL_DOWN, x11_at) + _x11_click(MOUSE_BUTTON_WHEEL_DOWN, x11_at),
		_repeat(_wayland_wheel(MOUSE_BUTTON_WHEEL_DOWN, 15, at), 12) + _repeat(_wayland_wheel(MOUSE_BUTTON_WHEEL_DOWN, 15, at), 4), true])
	# A touchpad's two-finger scroll: one click per 10 surface units, the remainder carried, back
	# and forth, and sideways (Xwayland's buttons 6 and 7).
	var x11_scroll := _x11_click(MOUSE_BUTTON_WHEEL_DOWN, x11_at) + _x11_click(MOUSE_BUTTON_WHEEL_UP, x11_at) \
		+ _x11_click(MOUSE_BUTTON_WHEEL_UP, x11_at) + _x11_click(MOUSE_BUTTON_WHEEL_RIGHT, x11_at)
	var wayland_scroll := [_pan(Vector2(0, 3.5), at), _pan(Vector2(0, 4.0), at), _pan(Vector2(0, 4.0), at),
		_pan(Vector2(0, -1.5), at), _pan(Vector2(0, -10.0), at), _pan(Vector2(0, -12.25), at), _pan(Vector2(12.0, 0), at)]
	list.append(["touchpad scroll", x11_scroll, wayland_scroll, true])
	# Ctrl with a touchpad scroll sizes the brush, as Ctrl with the wheel does.
	list.append(["touchpad scroll with Ctrl", _x11_click(MOUSE_BUTTON_WHEEL_UP, x11_at, 0, true) + _x11_click(MOUSE_BUTTON_WHEEL_UP, x11_at, 0, true),
		[_pan(Vector2(0, -7.0), at, true), _pan(Vector2(0, -13.0), at, true)], true])
	# A middle-button orbit drag: X11's relative is the change of the integer position; Wayland's
	# relative pointer goes on at the screen's edge, where the position stops (scale 1.2).
	list.append(["orbit drag", [_motion(x11_at, Vector2.ZERO, 0), _button(MOUSE_BUTTON_MIDDLE, true, x11_at, middle), _motion(Vector2(405, 302), Vector2(5, 2), middle),
		_motion(Vector2(431, 290), Vector2(26, -12), middle), _button(MOUSE_BUTTON_MIDDLE, false, Vector2(431, 290), 0)],
		[_motion(at, Vector2(1.2, 0.0), 0), _button(MOUSE_BUTTON_MIDDLE, true, at, middle), _motion(Vector2(405.9, 302.2), Vector2(6.0, 2.4), middle),
		_motion(Vector2(405.2, 302.9), Vector2(-0.84, 0.84), middle), _motion(Vector2(431.5, 290.0), Vector2(31.4, -15.6), middle),
		_motion(Vector2(431.5, 290.0), Vector2(48.0, 0.0), middle), _button(MOUSE_BUTTON_MIDDLE, false, Vector2(431.5, 290.0), 0)], true])
	# Keys whose physical names differ between Godot's key maps (Finnish layout): Å, §, <.
	var x11_keys := []
	var wayland_keys := []
	for key: Array in [[0xC5, KEY_BRACKETLEFT, KEY_BRACELEFT, 0xE5], [KEY_SECTION, KEY_QUOTELEFT, KEY_SECTION, 0xA7],
			[KEY_LESS, KEY_SECTION, KEY_QUOTELEFT, 0x3C], [0xA8, KEY_BRACKETRIGHT, KEY_BRACERIGHT, 0xA8]]:
		for pressed in [true, false]:
			x11_keys.append(_key(key[0], key[1], key[0], key[3], pressed))
			wayland_keys.append(_key(key[0], key[2], key[0], key[3], pressed))
	list.append(["physical keycodes", x11_keys, wayland_keys, true])
	# Key repeat: both send echo presses at the compositor's rate, then one release.
	list.append(["key repeat", [_key(KEY_W, KEY_W, KEY_W, 0x77, true), _key(KEY_W, KEY_W, KEY_W, 0x77, true, true), _key(KEY_W, KEY_W, KEY_W, 0, false)],
		[_key(KEY_W, KEY_W, KEY_W, 0x77, true), _key(KEY_W, KEY_W, KEY_W, 0x77, true, true), _key(KEY_W, KEY_W, KEY_W, 0, false)], true])
	# A dead key, then A: "á". X11's input method takes the dead key's press and delivers its release
	# (keycode from the physical key) and A's release; Wayland's xkb compose delivers A's press only.
	list.append(["dead key composition", [_key(KEY_A, KEY_A, KEY_A, 0xE1, true), _key(KEY_EQUAL, KEY_EQUAL, KEY_EQUAL, 0, false), _key(KEY_A, KEY_A, KEY_A, 0, false)],
		[_key(KEY_A, KEY_A, KEY_A, 0xE1, true)], false])
	# A pinch: Wayland only. Each step is a magnify and the fingers' pan; it zooms, as on macOS.
	var pinch := InputEventMagnifyGesture.new()
	pinch.factor = 1.44
	pinch.position = at
	list.append(["pinch", [], [pinch, _pan(Vector2(3.0, 25.0), at)], false])
	return list

func _reset() -> void:
	_expect(camera.set_view_state(PackedFloat32Array([0, 1000.25, 0, 0, -PI / 4, 400, 0])), "restore valid view")
	probe.seen.clear()
	probe.actions.clear()
	probe.brush.radius = VegetationTool.MIN_RADIUS_M

# Every event through Input, one frame's flush each, as the display servers send them.
func _play(events: Array) -> Dictionary:
	_reset()
	for event: InputEvent in events:
		Input.parse_input_event(event.duplicate())
		Input.flush_buffered_events()
	camera.advance_motion(10.0)
	return {
		"view": camera.get_view_state(),
		"seen": probe.seen.duplicate(),
		"actions": probe.actions.duplicate(),
		"a_held": Input.is_key_pressed(KEY_A),
		"w_held": Input.is_key_pressed(KEY_W),
	}

# The normaliser alone: each gesture fed twice, as Input dispatches it (input.cpp).
func _shapes() -> void:
	var delivered: Array[String] = []
	var normalizer := WaylandInput.Normalizer.new(func(e: InputEvent) -> void: delivered.append(signature(e)), func(_e: InputEvent) -> void: pass)
	var x11: Array[String] = []
	for e: InputEvent in _x11_click(MOUSE_BUTTON_WHEEL_DOWN, Vector2(10, 20), MOUSE_BUTTON_MASK_LEFT, true):
		x11.append(signature(e))
	var pan := _pan(Vector2(0, 10.0), Vector2(10.5, 20.9), true)
	normalizer.feed(pan, MOUSE_BUTTON_MASK_LEFT, false)
	normalizer.feed(pan, MOUSE_BUTTON_MASK_LEFT, false)
	_expect(delivered == x11, "a 10-unit scroll, dispatched twice, is one X11 click with the held button: %s" % [delivered])
	delivered.clear()
	normalizer.feed(_button(MOUSE_BUTTON_WHEEL_UP, true, Vector2(1, 1), 1 << 3, 0.5), 1 << 3, false)
	normalizer.feed(_pan(Vector2(0, -5.0), Vector2(1, 1)), 0, false)
	_expect(delivered.size() == 2, "half a notch and half a touchpad click share X11's one scroll valuator")
	delivered.clear()
	var captured := _motion(Vector2(5.5, 5.5), Vector2(40, 0), 0)
	normalizer.feed(captured, 0, true)
	_expect(delivered.size() == 1 and captured.relative == Vector2(40, 0), "captured motion keeps Wayland's relative pointer")

func _run() -> void:
	root.size = Vector2i(960, 540)
	var host := Node3D.new()
	root.add_child(host)
	var simulation := SimulationNode.new()
	simulation.name = "SimulationNode"
	host.add_child(simulation)
	_expect(simulation.create_blank_world(512, 512, 8, 128, 50), "fixture world")
	camera = CameraNode.new()
	camera.name = "CameraNode"
	host.add_child(camera)
	camera.make_current()
	camera.configure_world_camera()
	camera.set_debug_under_terrain_enabled(false)
	camera.set_process(false)
	var input := Node.new()
	host.add_child(input)
	input.set_script(load("res://scripts/core/input_manager.gd"))
	input.set_process(false)
	# Its _ready (tools, overlays) is not needed; the script came after entering the tree, so its
	# input callbacks are switched on here.
	input.set_process_input(true)
	input.set_process_unhandled_input(true)
	probe = Probe.new()
	probe.brush = VegetationTool.new()
	probe.sign = signature
	root.add_child(probe)
	await process_frame
	await process_frame

	_shapes()
	var scenarios := _scenarios()
	# X11: the engine's own window callback, unchanged from `dev`.
	var x11 := []
	for scenario: Array in scenarios:
		x11.append(_play(scenario[1]))
	# The X11 outcomes `dev` gives: a notch zooms one step of 1.2, the orbit turns by the integer
	# motion, the bracket keys step the roundabout and § and < open the console.
	_expect(absf(x11[0]["view"][5] - 400.0 / 1.2) < 0.01, "X11: a wheel notch zooms one step")
	_expect(absf(x11[1]["view"][5] - 400.0 * 1.44) < 0.01, "X11: two clicks zoom out two steps")
	_expect(x11[4]["view"][3] != 0.0, "X11: the middle drag orbits")
	_expect(x11[5]["actions"] == ["roundabout size %d" % KEY_BRACKETLEFT, "console toggle", "console toggle", "roundabout size %d" % KEY_BRACKETRIGHT],
		"X11: Å and ¨ step the roundabout, § and the ISO < key toggle the console: %s" % [x11[5]["actions"]])
	_expect(x11[3]["actions"].size() == 2, "X11: Ctrl with two clicks steps the brush twice")
	_expect(not x11[7]["a_held"], "X11: a composed key is released")
	_expect(x11[8]["view"][5] == 400.0, "X11: no pinch")

	var wayland_input := Node.new()
	wayland_input.set_script(WaylandInput)
	root.add_child(wayland_input)
	wayland_input.install()
	var wayland := []
	for scenario: Array in scenarios:
		wayland.append(_play(scenario[2]))

	for i in scenarios.size():
		var name: String = scenarios[i][0]
		if name == "pinch":
			_expect(absf(wayland[i]["view"][5] - 400.0 / 1.44) < 0.01, "Wayland: a pinch of 1.44 zooms in two steps, once: %s" % wayland[i]["view"][5])
			_expect(wayland[i]["seen"].size() == 1 and wayland[i]["seen"][0].contains("InputEventMagnifyGesture"), "Wayland: the pinch's own pan is not delivered: %s" % [wayland[i]["seen"]])
			continue
		_expect(wayland[i]["view"] == x11[i]["view"], "%s: the camera acts alike: X11 %s, Wayland %s" % [name, x11[i]["view"], wayland[i]["view"]])
		_expect(wayland[i]["actions"] == x11[i]["actions"], "%s: the tools act alike: X11 %s, Wayland %s" % [name, x11[i]["actions"], wayland[i]["actions"]])
		_expect(wayland[i]["a_held"] == x11[i]["a_held"] and wayland[i]["w_held"] == x11[i]["w_held"], "%s: Input holds the same keys" % name)
		if scenarios[i][3]:
			_expect(wayland[i]["seen"] == x11[i]["seen"], "%s: the game sees X11's events:\nX11     %s\nWayland %s" % [name, x11[i]["seen"], wayland[i]["seen"]])
	host.free()
	probe.brush.free()
	if failures == 0:
		print("wayland_input_test: PASS")
	quit(failures)
