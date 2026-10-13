# SPDX-License-Identifier: GPL-2.0-only

## The `junction_connectors` overlay draws the connector arcs Rust builds, redraws when the
## network generation changes, focuses one junction, runs the stall detector and clears when off.
extends SceneTree

const DebugConsoleScript := preload("res://scripts/ui/debug_console/debug_console.gd")
const OverlayScript := preload("res://scripts/renderers/junction_debug_overlay.gd")
const WORLD_M := 2048.0
const TIMEOUT_FRAMES := 600
const T_CENTER := Vector3.ZERO

var _failures := 0

func _initialize() -> void:
	call_deferred("_run")

func _expect(condition: bool, message: String) -> void:
	if not condition:
		_failures += 1
		push_error(message)

func _run() -> void:
	# `--script` runs load autoloads; fall back to a local console if this one did not.
	var console: Node = root.get_node_or_null("DebugConsole")
	if console == null:
		console = DebugConsoleScript.new()
		console.name = "DebugConsole"
		root.add_child(console)
	var host := Node3D.new()
	root.add_child(host)
	var simulation := SimulationNode.new()
	simulation.name = "SimulationNode"
	host.add_child(simulation)
	_expect(simulation.create_blank_world(WORLD_M, WORLD_M, 8.0, 512.0, 0.0), "fixture world must be created")
	var camera := Camera3D.new()
	host.add_child(camera)
	camera.global_position = Vector3(0.0, 300.0, 1.0)
	camera.look_at(Vector3.ZERO)
	camera.current = true
	var overlay := OverlayScript.new()
	overlay.name = "JunctionDebugOverlay"
	host.add_child(overlay)
	var first_seq: int = MetrumDebug.read_since(0, 0)["last_seq"]

	_expect(not overlay.is_processing() and overlay.vertex_count == 0, "the overlay must do nothing while off")
	_expect(console.set_visual_mode("junction_connectors", "arcs"), "junction_connectors must switch on")
	_expect(overlay.is_processing(), "switching on must start the overlay")
	_expect(await _settle(overlay, simulation), "the empty world must build")
	_expect(overlay.last_counts.get("junctions", -1) == 0 and overlay.vertex_count == 0, "an empty world draws no junctions")

	# One straight road: each dead end carries a U-turn connector.
	await _add_road(simulation, [Vector3(-150, 0, 0), Vector3(150, 0, 0)])
	_expect(await _settle(overlay, simulation), "the straight road must redraw")
	var straight: int = overlay.vertex_count
	_expect(straight > 0 and overlay.last_counts.get("junctions", 0) == 2, "a road's two dead ends must draw, got %s" % overlay.last_counts)
	# A second road from the middle splits the first into a T junction.
	await _add_road(simulation, [T_CENTER, Vector3(0, 0, 150)])
	_expect(await _settle(overlay, simulation), "the T junction must redraw")
	var arcs: int = overlay.vertex_count
	_expect(arcs > straight, "a new network generation must redraw with the T, %d -> %d vertices" % [straight, arcs])
	_expect(overlay.last_counts.get("junctions", 0) == 4, "the T and three dead ends must draw, got %s" % overlay.last_counts)

	var before: int = MetrumDebug.read_since(0, 0)["last_seq"]
	_expect(overlay.try_focus_at(T_CENTER + Vector3(2, 0, 1)), "a click near the T must focus it")
	_expect(await _settle(overlay, simulation), "the focus must redraw")
	_expect(overlay.last_counts.get("junctions", 0) == 1 and overlay.last_counts.get("connectors", 0) == 6, "focus must show only the T's six turns, got %s" % overlay.last_counts)
	_expect(overlay.vertex_count < arcs, "focus must draw fewer vertices than the area")
	var lines: Array = Array(MetrumDebug.read_since(before, 64)["message"])
	_expect(lines.any(func(line: String) -> bool: return line.contains("6 vehicle connectors, turns open")), "focus must log the junction on events")
	_expect(lines.filter(func(line: String) -> bool: return line.contains("connector ") and line.contains("cap ")).size() == 6, "focus must log every connector")
	_expect(not overlay.try_focus_at(Vector3(75, 0, 75)), "a click away from junctions must miss")
	_expect(await _settle(overlay, simulation), "a miss must redraw the area")
	_expect(overlay.vertex_count == arcs, "a miss must clear the focus")
	_expect(overlay.try_focus_at(T_CENTER) and overlay.unfocus(), "Esc must clear a focus")
	_expect(not overlay.unfocus(), "Esc without a focus must do nothing")

	for view in ["speed", "occupancy"]:
		_expect(console.set_visual_mode("junction_connectors", view), "%s must switch" % view)
		_expect(await _settle(overlay, simulation), "%s must build" % view)
		_expect(overlay.vertex_count == arcs, "%s colors the same arcs" % view)
	_expect(console.set_visual_mode("junction_connectors", "lanes"), "lanes must switch")
	_expect(await _settle(overlay, simulation), "lanes must build")
	_expect(overlay.vertex_count > arcs, "lanes must add the road lane center lines")

	# No cars: the detector scans and reports nothing.
	_expect(simulation.scan_junction_stalls() == 0, "an idle network has no stalled cars")
	for frame in 30:
		await process_frame
	var all_lines: Array = Array(MetrumDebug.read_since(first_seq, 4096)["message"])
	_expect(not all_lines.any(func(line: String) -> bool: return line.begins_with("junction stall")), "no stall may be reported without cars")

	_expect(console.set_visual_mode("junction_connectors", "off"), "junction_connectors must switch off")
	_expect(overlay.vertex_count == 0 and overlay._mesh.get_surface_count() == 0, "off must clear the mesh")
	_expect(not overlay.is_processing(), "off must stop the overlay")
	host.queue_free()
	await process_frame
	print("Junction debug overlay tests: %d failures" % _failures)
	quit(0 if _failures == 0 else 1)

func _add_road(simulation: Node, points: Array) -> void:
	var request: int = simulation.add_road_with_snap(PackedVector3Array(points), 1, 1, true)
	for frame in TIMEOUT_FRAMES:
		var result: Variant = simulation.get_road_commit_result(request)
		if result != null:
			_expect(result.get("committed", false), "fixture road must commit: %s" % result)
			return
		await process_frame
	_expect(false, "fixture road commit timed out")

# Built means the overlay has drawn the current network generation for a few frames.
func _settle(overlay: Node, simulation: Node) -> bool:
	var stable := 0
	for frame in TIMEOUT_FRAMES:
		await process_frame
		var built: bool = not overlay._dirty and overlay._built_generation == simulation.get_network_render_generation()
		stable = stable + 1 if built else 0
		if stable >= 3:
			return true
	return false
