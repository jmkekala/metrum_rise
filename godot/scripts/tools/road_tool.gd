# SPDX-License-Identifier: GPL-2.0-only

## Road drawing tool: a thin bridge to Rust's road tool session (`ROAD-112`), with a synchronous
## WYSIWYG preview, gizmos and the HUD.
##
## Every drawing mode runs on the session (round D): straight (two clicks), the simple curve
## (three clicks: start, a free control point, end; the exact fillet arc plus a straight lead,
## U1), the S-curve (round N: four clicks, start, a start aim point, an end aim point, end; the
## biarc) and freeform (round N: clicks place the nodes of Bezier spans; a click on the last
## node or Enter starts the edit phase, where a press drags a node or a handle until the release,
## Ctrl breaks a joint, and Enter or a click elsewhere builds; right-click or Esc drops it). The
## straight stroke and the simple curve can be continuous (the stroke leaves along the road or
## the chain it starts on), and a continuous chain carries its direction into the S-curve and
## freeform too (round CT). The script ticks Rust once a frame (`road_tool_tick` with the camera,
## the mouse, Shift and Ctrl, or `road_tool_tick_at` for a scripted pointer; round R2-2) and acts
## only on the flags it returns: the preview, the draft's footprint hue (round HU, a stopgap for
## the lagging preview terrain; Rust draws it when asked after the preview shows,
## `road_tool_draw_hue`), the HUD's chips (`road_tool_hud.gd`,
## from Rust's numbers only), the hover cursor and the sent stroke's answer; a frame that changed
## nothing does nothing. The tick draws the gizmos (centreline, tangent legs, guide and angle
## lines, handles) and the batch ghost itself (round R2-3), with the palette and materials
## `road_tool_gizmos.gd` hands it once. Clicks, releases, Enter and the mode actions are typed calls
## (`road_tool_press`, `road_tool_release`, `road_tool_action`), the options too. Rust resolves
## the cursor and its snapping, keeps the stroke, the chain, the phase and the mode, previews the
## stroke when it changed, sends the confirmed stroke and answers its commit (round R2-1, drawn
## here).
##
## The synchronous road preview (`ROAD-68-E`): every frame whose stroke changed calls Rust's
## `preview_road` (inside `road_tool_tick`), which plans the stroke on the road tool's context
## and returns the changed road chunks exactly as the commit publishes them, and shows them in
## that frame in place of the resident chunks. A click submits that preview; the preview stays
## shown until the committed generation's chunks are installed, then both swap in one call. The
## preview's graded terrain tiles (`TERRAIN-07-E`) show on the terrain renderer's graded tiles
## from the frame that staged them; frames without a new pose stage the pose's rest, and the
## commit's edit replaces them with the committed tiles in one frame. Also the snapping options
## (network snap, guide lines, zoning grid, angle lock step, length step; Shift draws free of all
## but the network, B5). Heights (round I): each free end has a height handle, a grip above it;
## a press on a grip drags it (Rust projects the pointer onto its vertical line in 0.5 m steps,
## Shift free) until the release, and PageUp/PageDown (`step_height`) move the active handle by
## the same step. A handle off the ground, or straight grade (T or the options checkbox), gives
## the stroke a grade line that Rust cuts into ground, bridge and tunnel strokes: one previews
## as any stroke, several as a batch (the ghost while the pointer moves, the exact pieces once
## the pose rests) and commit as one undo step. The tool's intent (round R2-6: the road type,
## lanes, median, kerbside parking, attributes, the snapping options, continuous and straight
## grade) lives in Rust's session only; this script holds no copy. `set_road_type` hands Rust a
## road type id (`ROAD-101`, `PARK-05`) and Rust looks it up and sets all its settings at once
## (a walkway, which has no speed, takes the built-ins' defaults); R draws the last road type
## picked (2-Lane Road at first), X picks the Walkway. `set_lanes`, `set_straight_grade` and
## `set_option` forward one setting; the UI reads them back through `intent()` when a menu opens
## or a key acts. Rust stays the guard: the stroke's check refuses a median its lanes cannot
## carry.
## A stroke over zoned land takes it (`ZONE-09`): the preview outlines the lots Rust says the
## stroke removes (`demolish_lots`) with the bulldoze tool's highlight and the HUD names the
## buildings it demolishes (`demolish_count`); plotted land refuses with its reason.
## Rust methods called: road_tool_tick(), road_tool_tick_at(), road_tool_pointer(),
## road_tool_pointer_at(), road_tool_press(), road_tool_release(), road_tool_action(),
## road_tool_set_lanes(), road_tool_set_flag(), road_tool_set_step(), road_tool_preview(),
## road_tool_preview_current(), road_tool_preview_sequence(), road_tool_graded_changes(),
## road_tool_commit_answer(), road_tool_phase(), road_tool_mode(), road_tool_cursor(),
## road_tool_points(), road_tool_draw_hue(), road_tool_hue_times(), road_tool_release_held(),
## road_tool_attach_overlay(), road_tool_detach_overlay(), road_tool_forget_drawn(), road_tool_set_road_type(), road_tool_reload_road_type(),
## road_tool_intent(), release_road_preview_terrain(), road_tool_commit_poll(),
## road_tool_commit_request_id(), road_tool_commit_points(), road_tool_commit_graded_epoch(),
## set_border_connection().
## Placement stays pending until Rust acknowledges acceptance; rejection retains the editable
## stroke. Drawing is continuous: an accepted road starts the next one at its end until
## right-click/Esc, a connection to existing roads, or the map edge; a committed end on the map
## edge asks about an external connection.
extends "res://scripts/tools/network_tool.gd"

const PreviewDisplay := preload("res://scripts/renderers/road_preview_display.gd")
const RoadChunkRenderer := preload("res://scripts/renderers/road_chunk_renderer.gd")
const RoadToolGizmos := preload("res://scripts/renderers/road_tool_gizmos.gd")
const RoadToolHud := preload("res://scripts/ui/road_tool_hud.gd")
const RoadReasonText := preload("res://scripts/ui/road_reason_text.gd")
const HighlightMesh := preload("res://scripts/renderers/highlight_mesh.gd")
# Early, so the preview of this frame's pointer is staged before the frame renders.
const PREVIEW_SAMPLE_PROCESS_PRIORITY := -100

## Drawing modes, numbered as Rust's `road_tool_mode` reports them.
enum DrawMode { STRAIGHT, SIMPLE_CURVE, COMPLEX_CURVE, FREEFORM }
# The session's phase with no stroke (`road_tool_phase`: 0 no stroke, 1 placing points, 2 a sent
# stroke awaiting its acknowledgement, 3 a freeform draft's edit phase).
const PHASE_IDLE := 0
# `road_tool_action`'s codes: cancel, confirm, the four modes from straight on, forget the preview.
const ACTION_CANCEL := 0
const ACTION_CONFIRM := 1
const ACTION_MODE_STRAIGHT := 2
const ACTION_RAISE_HANDLE := 6
const ACTION_LOWER_HANDLE := 7
const ACTION_FORGET_PREVIEW := 8
# `road_tool_set_flag`'s and `road_tool_set_step`'s codes by option name.
const _FLAG_OPTIONS := {"network_snap": 0, "guide_lines": 1, "zoning_grid": 2, "continuous": 3, "straight_grade": 4}
const _STEP_OPTIONS := {"angle_step_deg": 0, "length_step_m": 1}
# The tick's flags (`road_tool_tick`): a new preview, the preview gone, a batch ghost, the ghost
# gone, the gizmos, the hue, the chips, only the chips' corners, the pose's graded share, the sent
# stroke's answer (the tick stopped there), the cursor.
const TICK_PREVIEW := 1
const TICK_PREVIEW_CLEARED := 2
const TICK_BATCH := 4
const TICK_BATCH_CLEARED := 8
const TICK_GIZMOS := 16
const TICK_HUE := 32
const TICK_CHIPS := 64
const TICK_CORNERS := 128
const TICK_GRADED := 512
const TICK_ANSWER := 1024
const TICK_CURSOR := 4096

## The drawing mode (`DrawMode`), the session's: setting it sends the mode action, which drops
## the stroke and ends the chain when the mode changes.
var draw_mode: int:
	get:
		return simulation_node.road_tool_mode() if _has_session() else DrawMode.STRAIGHT
	set(mode):
		if not _has_session():
			return
		mode = clampi(mode, DrawMode.STRAIGHT, DrawMode.FREEFORM)
		var previous: int = simulation_node.road_tool_mode()
		simulation_node.road_tool_action(ACTION_MODE_STRAIGHT + mode)
		if mode != previous:
			_clear_preview_visual()
			_clear_preview_cache()
## Straight grade (U3, T or the options checkbox) was turned on or off.
signal straight_grade_toggled(enabled: bool)
## The built-in Walkway type, X's pick.
const WALKWAY_ROAD_TYPE := "builtin:walkway"
## The next stroke's road type changed (the Roads menu marks it).
signal road_type_changed(id: String)


# The HUD (`road_tool_hud.gd`): the label chips, fetched and drawn when the tick says they
# changed (tests assemble a tool with a HUD of their own).
var hud = null
# The tool's drawing setup (`road_tool_gizmos.gd`: the palette and materials Rust draws the
# gizmos, the hue and the batch ghost with), made before the first tick.
var gizmos = null

var preview_metrics: RefCounted
# The latest input's time, only while the preview is measured.
var _preview_input_us := 0
var _preview_metric_row: Dictionary = {}

# Read live so the debug console takes effect on the next road action.
var _road_debug_enabled: bool:
	get:
		return MetrumDebug.is_enabled("road")
# An input asked for a new preview since the last tick (benchmarks and probes wait past it).
var _preview_update_pending: bool = false
# The latest synchronous preview the tick handed over (`road_tool_preview`'s dictionary).
var _preview: Dictionary = {}
# The previews Rust handed out (`sequence`; tests, benchmarks and the metrics trace) and the one
# drawn.
var _preview_sequence: int:
	get:
		return simulation_node.road_tool_preview_sequence() if _has_session() else 0
var _preview_drawn_sequence: int = 0
var _preview_display = PreviewDisplay.new()
# An accepted commit's preview, held until the chunks of its generation are installed (the hold
# is `chunks.holds`', Rust's rule).
var _swap_display = null
# The sent stroke's commit, its points and the graded preview epoch it adopted, as Rust's commit
# sequence holds them (round R2-1; tests, benchmarks and probes wait on them): 0, empty and -1
# while none waits.
var _commit_request_id: int:
	get:
		return simulation_node.road_tool_commit_request_id() if _has_session() else 0
var _commit_points: PackedVector3Array:
	get:
		return simulation_node.road_tool_commit_points() if _has_session() else PackedVector3Array()
var _graded_commit_epoch: int:
	get:
		return simulation_node.road_tool_commit_graded_epoch() if _has_session() else -1
# The outlines of the lots the previewed stroke removes (`ZONE-09`), made on first use.
var _demolish_mesh: MeshInstance3D = null
# The latest pose's graded tiles still to stage (`TERRAIN-07-E`, T27 (d); the tick stages the
# next share each frame without a new pose; probes and tests wait on 0).
var _graded_pending: int:
	get:
		return simulation_node.road_tool_graded_pending() if _has_session() else 0
# The tool ticked since its drawings were last dropped.
var _ticked := false
# A new hue revision waits for the frame's end to be joined and drawn (`_show_hue`).
var _hue_queued := false

func _notification(what: int) -> void:
	if what == NOTIFICATION_PREDELETE:
		_release_graded_preview()
		if _has_session():
			simulation_node.road_tool_detach_overlay()

func begin_preview_measurement() -> RefCounted:
	preview_metrics = preload("res://scripts/benchmarks/road_preview_metrics.gd").new()
	if DisplayServer.get_name() != "headless" and not RenderingServer.frame_post_draw.is_connected(_preview_frame_completed):
		RenderingServer.frame_post_draw.connect(_preview_frame_completed)
	if DisplayServer.get_name() != "headless":
		preview_metrics.viewport = get_viewport().get_viewport_rid()
		RenderingServer.viewport_set_measure_render_time(preview_metrics.viewport, true)
	return preview_metrics

func _preview_frame_completed() -> void:
	if preview_metrics != null:
		preview_metrics.frame(_preview_drawn_sequence, DisplayServer.get_name() != "headless")

## The committed network's chunks (`RENDER-52`): resident set, staging, holds, lights and signs.
var chunks: Node3D

## Makes the tool's chunk renderer, its first child, reading the tool's simulation and terrain
## nodes. `_ready` calls it; tests that assemble a tool outside a scene call it themselves.
func make_chunks() -> void:
	chunks = RoadChunkRenderer.new()
	chunks.simulation_node = simulation_node
	chunks.terrain_node = terrain_node
	chunks.host = self
	add_child(chunks)
	# Rust draws the preview's overlay chunks with the chunks' layer materials (round R2-4a); a
	# tool outside the tree (tests) gets meshes drawn nowhere, which its captures copy.
	if _has_session():
		var scenario := get_world_3d().scenario if is_inside_tree() else RID()
		simulation_node.road_tool_attach_overlay(scenario, chunks.hydration_materials())

func _ready():
	# Before the base class's visuals: the chunks, lights and signs keep their place in the tree.
	make_chunks()
	super._ready()
	process_priority = PREVIEW_SAMPLE_PROCESS_PRIORITY
	if MetrumDebug.is_enabled("perf"):
		begin_preview_measurement()
	attach_hud(RoadToolHud.new())

## Makes `new_hud` (`road_tool_hud.gd`) the tool's HUD, a child of the tool: its Build chip
## builds an open freeform draft (Enter's `confirm`), its Cancel chip drops it.
func attach_hud(new_hud) -> void:
	hud = new_hud
	add_child(hud)
	hud.build_pressed.connect(confirm_road)
	hud.cancel_pressed.connect(cancel_road)
	# The HUD sends its chip font to Rust once it knows the node (round V2).
	if is_instance_valid(simulation_node):
		hud.simulation_node = simulation_node

func _process(delta):
	if preview_metrics != null and DisplayServer.get_name() == "headless":
		preview_metrics.frame(_preview_drawn_sequence, false)
	_release_swapped_preview()
	super._process(delta)
	if not active:
		if _ticked:
			_ticked = false
			_clear_drawn()
		# A stroke sent before the tool went off still hears its answer.
		if _commit_request_id > 0:
			_poll_road_commit_result()
		return
	if _has_session():
		_tick()

# The road tool resolves its pointer in its tick, not in the base's frame.
func _update_pointer() -> void:
	pass

# One frame: Rust's tick (`road_tool_tick`), then what its flags say changed. The answer to a sent
# stroke stops the tick: it is drawn, then the frame ticks again.
func _tick() -> void:
	_ticked = true
	_gizmos()
	var camera: Camera3D = get_viewport().get_camera_3d() if is_inside_tree() else null
	var start_us := Time.get_ticks_usec() if preview_metrics != null else 0
	var flags := _tick_once(camera)
	if flags & TICK_ANSWER:
		_draw_commit_answer(simulation_node.road_tool_commit_answer())
		flags = _tick_once(camera)
	_draw_tick(flags, start_us)

# Acts on one tick's flags (`start_us`: the frame's start while the preview is measured).
func _draw_tick(flags: int, start_us := 0) -> void:
	_preview_update_pending = false
	if flags & TICK_CURSOR:
		_show_cursor()
		if preview_metrics != null and not _scripted_pointer_enabled:
			_preview_input_us = start_us
	if flags & TICK_PREVIEW:
		_preview = simulation_node.road_tool_preview()
		if preview_metrics != null:
			_measure_preview(start_us)
		_show_preview()
	elif flags & TICK_PREVIEW_CLEARED:
		_clear_preview_visual()
		_clear_preview_cache()
	elif flags & TICK_GRADED:
		# A frame without a new pose staged the next share of the pose's graded tiles.
		_show_graded(simulation_node.road_tool_graded_changes())
	if flags & TICK_HUE and not _hue_queued:
		_hue_queued = true
		_show_hue.call_deferred()
	if hud != null and (flags & (TICK_CHIPS | TICK_CORNERS) or hud.stale):
		hud.simulation_node = simulation_node
		hud.show_chips(flags & TICK_CHIPS != 0)

func _tick_once(camera: Camera3D) -> int:
	if _scripted_pointer_enabled:
		return simulation_node.road_tool_tick_at(camera, _scripted_pointer_position, false)
	if camera == null:
		return simulation_node.road_tool_tick(null, Vector2.ZERO, false, false)
	return simulation_node.road_tool_tick(camera, get_viewport().get_mouse_position(), Input.is_key_pressed(KEY_SHIFT), Input.is_key_pressed(KEY_CTRL))

# The hover cursor on Rust's cursor (hidden while no pointer has met a surface).
func _show_cursor() -> void:
	var cursor: Variant = simulation_node.road_tool_cursor()
	is_valid = cursor != null
	if is_valid:
		_last_world_mouse_pos = cursor
		_has_last_world_mouse_pos = true
	if cursor_mesh:
		cursor_mesh.visible = is_valid
		if is_valid:
			cursor_mesh.global_position = cursor + Vector3(0.0, 0.35, 0.0)

# The new preview into the metrics trace (measured runs only): its sample, its tick and payload.
func _measure_preview(start_us: int) -> void:
	var now := Time.get_ticks_usec()
	var sequence := int(_preview.get("sequence", 0))
	preview_metrics.sample(simulation_node.road_tool_points(), _preview_input_us if _preview_input_us > 0 else start_us)
	preview_metrics.dispatch(sequence, start_us, now, true)
	preview_metrics.poll(sequence, 0, _preview)

# Drops the gizmos, the hue and the chips (another tool, a cancel).
func _clear_drawn() -> void:
	_hue_queued = false
	if hud != null:
		hud.clear()
	if _has_session():
		simulation_node.road_tool_forget_drawn(TICK_GIZMOS | TICK_HUE | TICK_CURSOR)

func _has_session() -> bool:
	return is_instance_valid(simulation_node) and simulation_node.has_method("road_tool_tick")

## The tool's intent as Rust's session holds it (`road_tool_intent`, round R2-6): the road type
## ids, lanes, median, parking, attributes, the snapping options, continuous and straight grade.
## Read when a menu opens or a key acts, never per frame.
func intent() -> Dictionary:
	return simulation_node.road_tool_intent() if _has_session() else {}

## Sets one session option by name (`continuous`, `network_snap`, `guide_lines`, `zoning_grid`,
## `straight_grade` through `road_tool_set_flag`; `angle_step_deg`, `length_step_m` through
## `road_tool_set_step`; `lanes` as a `Vector2i`). Returns "" when set, else
## `road_tool_option_unknown`.
func set_option(option: String, value: Variant) -> String:
	if not _has_session():
		return ""
	if _FLAG_OPTIONS.has(option):
		simulation_node.road_tool_set_flag(_FLAG_OPTIONS[option], bool(value))
	elif _STEP_OPTIONS.has(option):
		simulation_node.road_tool_set_step(_STEP_OPTIONS[option], float(value))
	elif option == "lanes":
		simulation_node.road_tool_set_lanes(value.x, value.y)
	else:
		return "road_tool_option_unknown"
	return ""

## Whether a stroke is being drawn: a point is placed, or a sent stroke awaits its answer.
func is_drawing() -> bool:
	return _has_session() and simulation_node.road_tool_phase() != PHASE_IDLE

# The tool's drawing setup, made (and attached to Rust) on first use.
func _gizmos():
	if gizmos == null:
		gizmos = RoadToolGizmos.new(simulation_node)
		add_child(gizmos)
	return gizmos

# Draws the draft's footprint hue on the tick's hue flag, at the end of the frame's process step
# (a deferred call: still before this frame draws, so the hue and its preview show together).
# Rust joins the hue's build and draws it (`ROAD-123` round HF2: the frame's one wait for it);
# the build runs off the frame, so the frame's other nodes run beside it before the join (round
# R2-4a: the preview's overlay left the tool's own frame work shorter than a cross pose's hue).
# Nothing is drawn when the drawings were dropped since.
func _show_hue() -> void:
	if not _hue_queued or not _has_session():
		return
	_hue_queued = false
	simulation_node.road_tool_draw_hue()
	if preview_metrics != null:
		preview_metrics.hue(_preview_sequence, simulation_node.road_tool_hue_times())

## Sets the next stroke's lanes (Rust clamps them to 0..4 each; 0 + 0 is a walkway).
func set_lanes(fwd: int, bkw: int) -> void:
	if _has_session():
		simulation_node.road_tool_set_lanes(fwd, bkw)
	_queue_preview_update()

## Turns straight grade on or off (T, the options checkbox).
func set_straight_grade(enabled: bool) -> void:
	set_option("straight_grade", enabled)
	straight_grade_toggled.emit(enabled)
	_intent_changed()

## PageUp/PageDown: the active height handle by `steps` 0.5 m steps, as a drag sets it (the live
## end's while drawing, the hovered or last dragged one in a freeform edit phase).
func step_height(steps: int) -> void:
	if not _has_session():
		return
	for _step in absi(steps):
		simulation_node.road_tool_action(ACTION_RAISE_HANDLE if steps > 0 else ACTION_LOWER_HANDLE)
	_queue_preview_update()

## Draws the next strokes with road type `id` (`ROAD-101`): Rust looks the type up and sets its
## lanes, median, parking, speed, sidewalks, zoning and street lights at once
## (`road_tool_set_road_type`). Returns "" when picked, else Rust's refusal key; a refused pick
## leaves the intent as it was.
func set_road_type(id: String) -> String:
	var reason: String = simulation_node.road_tool_set_road_type(id)
	if reason.is_empty():
		road_type_changed.emit(id)
		_intent_changed()
	return reason

## Picks the current type again after the library changed (`ROAD-101`, Rust's
## `road_tool_reload_road_type`): an edited type's new layout draws from the next stroke on; a
## type that is gone or no longer usable falls back as Rust says.
func reload_road_type() -> void:
	var id: String = simulation_node.road_tool_reload_road_type()
	if not id.is_empty():
		road_type_changed.emit(id)
		_intent_changed()

# Rust's straight grade or road type changed: the stroke previews again (the session compares
# them itself).
func _intent_changed() -> void:
	_queue_preview_update()

func _unhandled_input(event):
	if _scripted_pointer_enabled:
		return
	# Pointer motion is sampled by _process, after the base tool resolves the current frame.
	if active and event is InputEventMouseButton and event.button_index == MOUSE_BUTTON_LEFT:
		if event.pressed:
			_handle_click()
		elif _has_session():
			# Ends a freeform drag.
			simulation_node.road_tool_release()

## Enter: ends a freeform draft's placing, or builds the draft in its edit phase (the session's
## `confirm`; in the click modes it places the next point at the cursor, as a click would).
func confirm_road() -> void:
	if _scripted_pointer_enabled or _commit_request_id > 0 or not _has_session() or not is_drawing():
		return
	_session_press(true)

# A click: the session's cursor under the pointer now (a click can arrive before _process), then
# the press, which places the next point or sends the stroke.
func _handle_click():
	if _scripted_pointer_enabled or _commit_request_id > 0:
		return
	var camera := get_viewport().get_camera_3d()
	if camera == null or not _has_session():
		return
	var total_start_us := Time.get_ticks_usec()
	var state_before: int = simulation_node.road_tool_phase()
	var mouse := get_viewport().get_mouse_position()
	var cursor: Variant = simulation_node.road_tool_pointer(camera.project_ray_origin(mouse), camera.project_ray_normal(mouse), Input.is_key_pressed(KEY_SHIFT), Input.is_key_pressed(KEY_CTRL))
	var mouse_ms := float(Time.get_ticks_usec() - total_start_us) / 1000.0
	var commit_ms := 0.0
	if cursor != null:
		_last_world_mouse_pos = cursor
		_has_last_world_mouse_pos = true
		var press_start_us := Time.get_ticks_usec()
		_session_press()
		commit_ms = float(Time.get_ticks_usec() - press_start_us) / 1000.0
	_log_click_detail(state_before, cursor != null, mouse_ms, commit_ms, total_start_us)

# Shows the latest preview's chunks in this frame; an invalid stroke shows only its reason.
func _show_preview() -> void:
	var valid := bool(_preview.get("is_valid", false))
	var sequence := int(_preview.get("sequence", 0))
	if sequence != _preview_drawn_sequence:
		if preview_metrics != null:
			_preview_metric_row = preview_metrics.request(sequence)
		var stage_us := Time.get_ticks_usec() if preview_metrics != null else 0
		var shown := false
		# The pose's graded tiles show in its frame, whatever the road chunks do: Rust handed
		# them over once.
		var graded_ms := _show_graded(_preview)
		if valid and bool(_preview.get("has_road_chunks", false)):
			shown = _preview_display.show(chunks, _preview, sequence)
		if not shown:
			_preview_display.clear()
		_show_demolition(_preview)
		_preview_drawn_sequence = sequence
		if preview_metrics != null:
			_preview_metric_row["stage_install_ms"] = float(Time.get_ticks_usec() - stage_us) / 1000.0
			_preview_metric_row["graded_install_ms"] = graded_ms
			preview_metrics.installed(sequence)

# Outlines the lots a valid stroke removes (`ZONE-09`), four corners each, as the bulldoze tool
# outlines what it removes; anything else clears them.
func _show_demolition(preview: Dictionary) -> void:
	var lots: PackedVector3Array = preview.get("demolish_lots", PackedVector3Array())
	var mesh: Mesh = HighlightMesh.polygons(lots, 4) if bool(preview.get("is_valid", false)) else null
	if mesh == null:
		if _demolish_mesh != null:
			_demolish_mesh.mesh = null
			_demolish_mesh.visible = false
		return
	if _demolish_mesh == null:
		_demolish_mesh = HighlightMesh.instance()
		add_child(_demolish_mesh)
	_demolish_mesh.mesh = mesh
	_demolish_mesh.visible = true

# The terrain renderer's graded tiles, or null without one (tests without terrain).
func _graded_renderer():
	if not is_instance_valid(terrain_node):
		return null
	return terrain_node.get("graded_tiles")

# Shows one frame's graded tile changes (a pose's or a catch-up frame's) and returns the
# milliseconds the renderer took.
func _show_graded(frame: Dictionary) -> float:
	var graded = _graded_renderer()
	return graded.show_preview(frame) if graded != null else 0.0

# Ends the preview's graded tiles: Rust starts a new epoch and the renderer drops the tiles of the
# one that ends (a queued commit's epoch stays until its edit installs).
func _release_graded_preview() -> void:
	if not is_instance_valid(simulation_node):
		return
	var epoch: int = simulation_node.release_road_preview_terrain()
	var graded = _graded_renderer()
	if graded != null:
		graded.release_preview(epoch, epoch)

func _clear_preview_visual() -> void:
	_preview_display.clear()
	_show_demolition({})
	_release_graded_preview()
	_preview_drawn_sequence = 0
	if blueprint_mesh:
		blueprint_mesh.mesh = null
	if hud != null:
		hud.clear()

# After an accepted commit the preview stays shown until the committed generation's chunks are
# installed; `road_chunks_installed` releases it in the same call, so no frame shows neither.
func _hold_preview_until(generation: int) -> void:
	if _swap_display != null:
		_swap_display.reset()
	_swap_display = _preview_display
	# Rust held the commit's hue and overlay chunks with its answer (rounds R2-3, R2-4a); they go
	# with this preview.
	_swap_display.held = true
	chunks.holds.hold_preview(generation)
	_preview_display = PreviewDisplay.new()
	_preview_drawn_sequence = 0
	_release_swapped_preview()

func _release_swapped_preview() -> void:
	# Chunks held for a graded edit (T61 (2)) draw the preview's state with it until they swap.
	if _swap_display != null and chunks.holds.release_preview(chunks.generation):
		_swap_display.reset()
		_swap_display = null
		_drop_held_hue()

# The held commit's hue and overlay chunks go with its preview.
func _drop_held_hue() -> void:
	if _has_session():
		simulation_node.road_tool_release_held()

## The chunk renderer swapped the chunks it held for a graded edit (T61 (2)) for the installed
## ones; a held commit's preview (`_release_swapped_preview`) goes with them.
func road_chunks_released() -> void:
	_release_swapped_preview()
	# A preview that waited for the held chunks shows on the installed ones.
	_preview_display.resident_installed(chunks)

## The chunk renderer installed a generation's chunks.
func road_chunks_installed(_generation: int) -> void:
	# The acknowledgement may arrive in the same frame as the chunks it names.
	_poll_road_commit_result()
	_release_swapped_preview()
	# A preview made meanwhile (the next stroke of a chain) indexes the chunks just installed:
	# it shows in this call.
	_preview_display.resident_installed(chunks)

# Starts a stroke at exactly `pos` (tests, benchmarks and probes): the session's stroke is
# cancelled, then a scripted press there.
func _begin_stroke(pos: Vector3) -> void:
	active = true
	if not _has_session():
		return
	simulation_node.road_tool_action(ACTION_CANCEL)
	simulation_node.road_tool_pointer_at(pos)
	simulation_node.road_tool_press(0)
	_queue_preview_update()

func _queue_preview_update() -> void:
	if preview_metrics != null:
		_preview_input_us = Time.get_ticks_usec()
	# Coalesce input bursts; the tick previews once a frame.
	_preview_update_pending = true

# The road tool draws no blueprint mesh (its preview is road chunks): no per-frame writes to the
# base's blueprint material.
func _update_blueprint_visuals() -> void:
	pass

# A press (or Enter's `confirm`) on the session: places a point, grabs a freeform grip, or sends
# the stroke (Rust decides and keeps the commit, round R2-1). True when a commit was queued.
func _session_press(confirm := false) -> bool:
	var start_us := Time.get_ticks_usec() if preview_metrics != null else 0
	var result: Dictionary = simulation_node.road_tool_action(ACTION_CONFIRM) if confirm else simulation_node.road_tool_press(0)
	if not result.has("points"):
		return false
	var validation: Dictionary = result["preview"]
	if bool(result.get("fresh", false)):
		# The click previewed the stroke itself (Rust numbered it).
		_preview = validation
		if preview_metrics != null:
			preview_metrics.sample(result["points"], start_us)
			var sequence := int(validation.get("sequence", 0))
			preview_metrics.dispatch(sequence, start_us, Time.get_ticks_usec(), true)
			preview_metrics.poll(sequence, 0, validation)
	if int(result.get("request_id", 0)) == 0:
		# Rust logged the refusal on events; the stroke stays editable and shows why.
		_show_preview()
		return false
	# A click that previewed a pose of its own shows that pose's graded tiles first; the commit
	# adopted them (Rust ended their epoch), so none are left to stage.
	if int(validation.get("sequence", 0)) != _preview_drawn_sequence:
		_show_graded(validation)
	return true

## A scripted click at exactly `pos` (tests, benchmarks and probes): the session's cursor there,
## then a press, which places the stroke's next point (a curve's control point) or sends the
## stroke. True when it queued a commit.
func _commit_segment(pos: Vector3) -> bool:
	if _commit_request_id > 0 or not _has_session():
		return false
	_last_world_mouse_pos = pos
	_has_last_world_mouse_pos = true
	simulation_node.road_tool_pointer_at(pos)
	return _session_press()

# Draws Rust's answer to the sent stroke when chunks install (it may arrive in their frame).
func _poll_road_commit_result() -> void:
	if not _has_session():
		return
	var answer: Variant = simulation_node.road_tool_commit_poll()
	if answer != null:
		_draw_commit_answer(answer)

# Draws Rust's answer to the sent stroke (round R2-1: the session heard it, a refusal already
# turned the standing preview into its rejection).
func _draw_commit_answer(answer: Dictionary) -> void:
	if bool(answer["committed"]):
		# The preview shows until the commit's own chunks are installed (`ROAD-68-E`).
		_hold_preview_until(int(answer["generation"]))
		_clear_preview_visual()
		_clear_preview_cache()
		# A committed end on the map edge asks about an external connection (the chain ended
		# there, so the dialog never sits over a stroke still following the pointer).
		if int(answer["border_node"]) >= 0:
			_prompt_border_connection(int(answer["border_node"]))
		return
	var graded = _graded_renderer()
	var epoch := int(answer["graded_epoch"])
	if graded != null and epoch >= 0:
		graded.release_preview(epoch, epoch)
	if answer.has("rejected"):
		# The rejection (Rust's display of it, numbered) stays on the stroke until the pointer or
		# the context moves.
		_preview = answer["rejected"]
		_show_preview()

## Drops the stroke and ends the chain (right-click, Esc, another tool or world).
func cancel_road():
	if _has_session():
		simulation_node.road_tool_action(ACTION_CANCEL)
	_clear_preview_visual()
	_preview_update_pending = false
	_clear_preview_cache()
	_clear_drawn()

## Drops the committed chunks and the preview (another world): the next refresh takes the whole
## network.
func reset_main_mesh_chunks() -> void:
	_clear_preview_visual()
	_preview_display.reset()
	if _swap_display != null:
		_swap_display.reset()
		_swap_display = null
	_drop_held_hue()
	chunks.holds.drop_preview()
	_clear_preview_cache()
	chunks.reset()
	_queue_preview_update()

func mark_network_topology_dirty() -> void:
	_queue_preview_update()

## Shows a dialog asking whether to make this road endpoint an external connection.
## On confirmation, the node is promoted to Border and becomes an immigrant spawn point.
func _prompt_border_connection(node_id: int) -> void:
	# A tool outside the scene (tests) has nowhere to show it.
	if not is_inside_tree():
		return
	var dialog := ConfirmationDialog.new()
	dialog.title = "External Connection"
	dialog.dialog_text = (
		"This road reaches the map boundary.\n" +
		"Create an external connection here?\n\n" +
		"Immigrants will enter and leave the city through this point.\n" +
		"Immigration only flows while the road remains connected."
	)
	dialog.ok_button_text = "Create Connection"
	dialog.cancel_button_text = "No Thanks"
	add_child(dialog)
	dialog.confirmed.connect(func() -> void:
		simulation_node.set_border_connection(node_id)
		dialog.queue_free()
	)
	dialog.canceled.connect(func() -> void: dialog.queue_free())
	dialog.popup_centered()


## Whether the latest preview is for the stroke as it stands (benchmarks and probes wait on it).
func stroke_preview_current() -> bool:
	if _preview.is_empty() or _preview_update_pending or not _has_session():
		return false
	return simulation_node.road_tool_preview_current()

func _clear_preview_cache() -> void:
	# The session's bridge forgets its standing preview too: the next tick previews afresh.
	if _has_session():
		simulation_node.road_tool_action(ACTION_FORGET_PREVIEW)
	_preview = {}
	_preview_drawn_sequence = 0

func _log_click_detail(
	state_before: int,
	valid_after: bool,
	mouse_ms: float,
	commit_ms: float,
	total_start_us: int
) -> void:
	if not _road_debug_enabled:
		return
	MetrumDebug.log(
		"road",
		"road_click_detail state=%d valid=%s mouse_ms=%.3f commit_ms=%.3f total_ms=%.3f"
		% [
			state_before,
			str(valid_after),
			mouse_ms,
			commit_ms,
			float(Time.get_ticks_usec() - total_start_us) / 1000.0,
		]
	)
