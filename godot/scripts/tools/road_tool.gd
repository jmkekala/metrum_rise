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
## freeform too (round CT). The script forwards the pointer (`road_tool_pointer` with Shift and
## Ctrl, or `road_tool_pointer_at` for a scripted pointer), the clicks and releases, Enter
## (`confirm`), the mode actions and the acknowledgement (`road_tool_event`) and the options
## (`road_tool_set_option`),
## and shows what `road_tool_frame` returns once a frame: the preview, the gizmos
## (`road_tool_gizmos.gd`: centreline, tangent legs, guide and angle lines, handles), the draft's
## footprint hue (`road_tool_hue.gd`, round HU, a stopgap for the lagging preview terrain) and the HUD
## (`road_tool_hud.gd`, from Rust's numbers only). Rust resolves the cursor and its snapping,
## keeps the stroke and the chain, previews the stroke when it changed and sends the confirmed
## stroke.
##
## The synchronous road preview (`ROAD-68-E`): every frame whose stroke changed calls Rust's
## `preview_road` (inside `road_tool_frame`), which plans the stroke on the road tool's context
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
## Rust methods called: road_tool_pointer(), road_tool_pointer_at(), road_tool_event(),
## road_tool_set_option(), road_tool_frame(), road_tool_set_road_type(),
## road_tool_reload_road_type(), road_tool_intent(), advance_road_preview_terrain(),
## release_road_preview_terrain(), get_road_commit_result(),
## check_border_candidate(), set_border_connection(), get_road_tool_surface_generation().
## Placement stays pending until Rust acknowledges acceptance; rejection retains the editable
## stroke. Drawing is continuous: an accepted road starts the next one at its end until
## right-click/Esc, a connection to existing roads, or the map edge; a committed end on the map
## edge asks about an external connection.
extends "res://scripts/tools/network_tool.gd"

const PreviewDisplay := preload("res://scripts/renderers/road_preview_display.gd")
const RoadChunkRenderer := preload("res://scripts/renderers/road_chunk_renderer.gd")
const RoadToolGizmos := preload("res://scripts/renderers/road_tool_gizmos.gd")
const RoadToolHue := preload("res://scripts/renderers/road_tool_hue.gd")
const RoadToolHud := preload("res://scripts/ui/road_tool_hud.gd")
const RoadReasonText := preload("res://scripts/ui/road_reason_text.gd")
const HighlightMesh := preload("res://scripts/renderers/highlight_mesh.gd")
# Early, so the preview of this frame's pointer is staged before the frame renders.
const PREVIEW_SAMPLE_PROCESS_PRIORITY := -100

## Drawing modes, numbered as the session's frame reports them (`mode`).
enum DrawMode { STRAIGHT, SIMPLE_CURVE, COMPLEX_CURVE, FREEFORM }
const _MODE_EVENTS := ["mode_straight", "mode_simple_curve", "mode_complex_curve", "mode_freeform"]
# The session's phases (`phase`): no stroke, placing points, a sent stroke awaiting its
# acknowledgement, a freeform draft's edit phase.
const PHASE_IDLE := 0
const PHASE_PLACING := 1
const PHASE_EDITING := 3

## The drawing mode (`DrawMode`), the session's: setting it sends the mode action, which drops
## the stroke and ends the chain when the mode changes (the session's own mode is what counts:
## another tool on the same node may have left it elsewhere).
var draw_mode: int:
	get:
		return _draw_mode
	set(mode):
		mode = clampi(mode, DrawMode.STRAIGHT, DrawMode.FREEFORM)
		if _has_session():
			_read_press(simulation_node.road_tool_event(_MODE_EVENTS[mode]))
			if mode != _draw_mode:
				_clear_preview_visual()
				_clear_preview_cache()
		_draw_mode = mode
var _draw_mode: int = DrawMode.STRAIGHT
## Straight grade (U3, T or the options checkbox) was turned on or off.
signal straight_grade_toggled(enabled: bool)
## The built-in Walkway type, X's pick.
const WALKWAY_ROAD_TYPE := "builtin:walkway"
## The next stroke's road type changed (the Roads menu marks it).
signal road_type_changed(id: String)


# The HUD (`road_tool_hud.gd`): the label chips of the frame, projected and placed each frame
# (tests assemble a tool with a HUD of their own).
var hud = null
# The tool's gizmos (`road_tool_gizmos.gd`), made on the first frame.
var gizmos = null
# The draft's footprint hue (`road_tool_hue.gd`, round HU), made on the first frame, and the
# `hue_revision` it shows (-1: fetch Rust's on the next frame). A commit's hue moves to
# `_held_hue` with its preview and goes in the frame the commit's hold releases.
var _hue = null
var _hue_revision := -1
var _held_hue = null
## Each frame's gizmo upload and chip update milliseconds while the preview is measured
## (`begin_preview_measurement`).
var gizmo_upload_ms := PackedFloat32Array()
## The same frames' parts: the gizmo upload and the chips' fetch and placement (round R2-5: the
## chips' wording is part of the Rust frame).
var gizmo_part_ms := {"gizmos": PackedFloat32Array(), "label_place": PackedFloat32Array()}

var preview_metrics: RefCounted
var _preview_input_us := 0
var _preview_metric_row: Dictionary = {}

# Read live so the debug console takes effect on the next road action.
var _road_debug_enabled: bool:
	get:
		return MetrumDebug.is_enabled("road")
var _preview_update_pending: bool = false
# The latest synchronous preview (`preview_road`'s dictionary) and the exact stroke it is for.
var _preview: Dictionary = {}
var _preview_points := PackedVector3Array()
# Counts the previews taken; it names displayed results for the metrics trace only.
var _preview_sequence: int = 0
var _preview_drawn_sequence: int = 0
var _preview_display = PreviewDisplay.new()
# An accepted commit's preview, held until the chunks of its generation are installed (the hold
# is `chunks.holds`', Rust's rule).
var _swap_display = null
var _commit_request_id: int = 0
var _commit_points := PackedVector3Array()
var _commit_validation: Dictionary = {}
# The outlines of the lots the previewed stroke removes (`ZONE-09`), made on first use.
var _demolish_mesh: MeshInstance3D = null
# The latest pose's graded tiles still to stage (`TERRAIN-07-E`, T27 (d)): each frame without a
# new pose stages the next share until none is left.
var _graded_pending := 0
# The preview epoch a queued commit adopted: its graded tiles show until the commit's edit
# installs the committed ones, or until the commit is refused.
var _graded_commit_epoch := -1

# The session's last frame: its phase (0 idle, 1 placing, 2 waiting for the acknowledgement),
# the HUD numbers and the stroke.
var _session_phase := 0
var _session_hud: Dictionary = {}
var _session_points := PackedVector3Array()

func _notification(what: int) -> void:
	if what == NOTIFICATION_PREDELETE:
		_release_graded_preview()

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
	_poll_road_commit_result()
	_release_swapped_preview()
	var previous_mouse_pos := _last_world_mouse_pos
	var had_mouse_pos := _has_last_world_mouse_pos
	super._process(delta)
	if not active:
		if hud != null:
			hud.clear()
		if gizmos != null:
			gizmos.clear()
		_clear_hue()
	# The base tool has now resolved this frame's pointer, including camera-only movement.
	var moved := _has_last_world_mouse_pos and (not had_mouse_pos or previous_mouse_pos != _last_world_mouse_pos)
	if active and _has_session():
		_place_labels(_session_frame(moved))

# Hands the HUD this frame's camera (none outside the tree): it fetches and draws the chips only
# when Rust says they or their corners changed, so they stay the same pixel size at any zoom;
# records the frame's gizmo upload plus the chips' update while measured.
func _place_labels(gizmo_ms: float) -> void:
	if hud == null:
		return
	hud.simulation_node = simulation_node
	var label_ms: float = hud.place(get_viewport().get_camera_3d() if is_inside_tree() else null)
	if preview_metrics != null and gizmo_ms + label_ms > 0.0:
		gizmo_upload_ms.append(gizmo_ms + label_ms)
		gizmo_part_ms["gizmos"].append(gizmo_ms)
		gizmo_part_ms["label_place"].append(label_ms)

func _has_session() -> bool:
	return is_instance_valid(simulation_node) and simulation_node.has_method("road_tool_frame")

## The tool's intent as Rust's session holds it (`road_tool_intent`, round R2-6): the road type
## ids, lanes, median, parking, attributes, the snapping options, continuous and straight grade.
## Read when a menu opens or a key acts, never per frame.
func intent() -> Dictionary:
	return simulation_node.road_tool_intent() if _has_session() else {}

## Sets one session option under Rust's name (`road_tool_set_option`: `continuous`,
## `network_snap`, `guide_lines`, `zoning_grid`, `angle_step_deg`, `length_step_m`; the lanes and
## straight grade through `set_lanes` and `set_straight_grade`). Returns Rust's refusal key, ""
## when set.
func set_option(option: String, value: Variant) -> String:
	return simulation_node.road_tool_set_option(option, value) if _has_session() else ""

## Whether a stroke is being drawn: a point is placed, or a sent stroke awaits its answer.
func is_drawing() -> bool:
	return _session_phase != PHASE_IDLE

# Whether the stroke is the player's to change: points are being placed or a freeform draft is
# in its edit phase.
func _is_editable() -> bool:
	return _session_phase == PHASE_PLACING or _session_phase == PHASE_EDITING

# One frame of the session: the scripted pointer, then the frame record, its preview when the
# stroke, its type or the context changed, or the next share of the pose's graded tiles; then
# the gizmos (the HUD's chips follow in `_place_labels`). Returns the gizmo upload's milliseconds.
func _session_frame(moved: bool) -> float:
	if _scripted_pointer_enabled:
		simulation_node.road_tool_pointer_at(_scripted_pointer_position)
	if moved:
		_queue_preview_update()
	_preview_update_pending = false
	var start_us := Time.get_ticks_usec()
	var frame: Dictionary = simulation_node.road_tool_frame()
	var end_us := Time.get_ticks_usec()
	_read_session_frame(frame)
	var points: PackedVector3Array = frame.get("points", PackedVector3Array())
	if frame.has("preview"):
		if preview_metrics != null:
			preview_metrics.sample(points, _preview_input_us if _preview_input_us > 0 else start_us)
		_adopt_session_preview(frame["preview"], points, start_us, end_us)
		_show_preview(points)
	elif bool(frame.get("preview_cleared", false)):
		_clear_preview_visual()
		_clear_preview_cache()
	elif _graded_pending > 0 and _commit_request_id == 0 and _is_editable():
		# A frame without a new pose stages the next share of the pose's graded tiles.
		_show_graded(simulation_node.advance_road_preview_terrain())
	var gizmo_ms := _show_gizmos(frame) + _show_hue(frame)
	_show_batch_ghost(frame)
	return gizmo_ms

# A batch pose's ghost (round I): drawn while the batch's exact pieces are not ready, gone once
# they show or the draft is no batch.
func _show_batch_ghost(frame: Dictionary) -> void:
	if frame.has("batch"):
		gizmos.show_batch_ghost(frame["batch"])
	elif bool(frame.get("batch_cleared", false)):
		gizmos.clear_batch_ghost()

func _read_session_frame(frame: Dictionary) -> void:
	_session_phase = int(frame.get("phase", 0))
	_session_hud = frame.get("hud", {})
	_session_points = frame.get("points", PackedVector3Array())
	_draw_mode = int(frame.get("mode", _draw_mode))

# Draws the frame's gizmos (one reused mesh); returns the upload's milliseconds.
func _show_gizmos(frame: Dictionary) -> float:
	if gizmos == null:
		gizmos = RoadToolGizmos.new()
		add_child(gizmos)
	return gizmos.show_frame(frame)

# Draws the draft's footprint hue when Rust's changed (`hue_revision`); returns the upload's
# milliseconds.
func _show_hue(frame: Dictionary) -> float:
	var revision := int(frame.get("hue_revision", -1))
	if revision == _hue_revision:
		return 0.0
	_hue_revision = revision
	if _hue == null:
		_hue = RoadToolHue.new(gizmos)
		add_child(_hue)
	# Rust joins the hue's build here (`ROAD-123` round HF2): the frame's one wait for it.
	var start_us := Time.get_ticks_usec()
	var hue: Dictionary = simulation_node.road_tool_hue()
	var upload_ms: float = _hue.show_hue(hue)
	if preview_metrics != null:
		preview_metrics.hue(_preview_sequence, hue, float(Time.get_ticks_usec() - start_us) / 1000.0)
	return upload_ms

func _clear_hue() -> void:
	if _hue != null:
		_hue.clear()
	_hue_revision = -1

# Takes a preview the session's bridge ran (in its frame or at a click) as the latest preview.
func _adopt_session_preview(preview: Dictionary, points: PackedVector3Array, start_us: int, end_us: int) -> void:
	_preview_sequence += 1
	if preview_metrics != null:
		preview_metrics.dispatch(_preview_sequence, start_us, end_us, true)
		preview_metrics.poll(_preview_sequence, 0, preview)
	preview["sequence"] = _preview_sequence
	_preview = preview
	_preview_points = points

## Sets the next stroke's lanes (Rust clamps them to 0..4 each; 0 + 0 is a walkway).
func set_lanes(fwd: int, bkw: int) -> void:
	set_option("lanes", Vector2i(fwd, bkw))
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
		_read_press(simulation_node.road_tool_event("raise_handle" if steps > 0 else "lower_handle"))
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
			simulation_node.road_tool_event("release")

## Enter: ends a freeform draft's placing, or builds the draft in its edit phase (the session's
## `confirm`; in the click modes it places the next point at the cursor, as a click would).
func confirm_road() -> void:
	if _scripted_pointer_enabled or _commit_request_id > 0 or not _has_session() or not is_drawing():
		return
	_session_press("confirm")

# A click: the session's cursor under the pointer now (a click can arrive before _process), then
# the press, which places the next point or sends the stroke.
func _handle_click():
	if _scripted_pointer_enabled or _commit_request_id > 0:
		return
	var total_start_us := Time.get_ticks_usec()
	var state_before := _session_phase
	var mouse_start_us := Time.get_ticks_usec()
	var pos = get_world_mouse_pos()
	_last_world_mouse_pos = pos
	_has_last_world_mouse_pos = is_valid
	var mouse_ms := float(Time.get_ticks_usec() - mouse_start_us) / 1000.0
	var commit_ms := 0.0
	if is_valid:
		var press_start_us := Time.get_ticks_usec()
		_session_press()
		commit_ms = float(Time.get_ticks_usec() - press_start_us) / 1000.0
	_log_click_detail(state_before, is_valid, mouse_ms, commit_ms, total_start_us)

# Shows the latest preview's chunks in this frame; an invalid stroke shows only its reason.
func _show_preview(_points: PackedVector3Array) -> void:
	is_valid = bool(_preview.get("is_valid", false))
	var sequence := int(_preview.get("sequence", 0))
	if sequence != _preview_drawn_sequence:
		if preview_metrics != null:
			_preview_metric_row = preview_metrics.request(sequence)
		var stage_us := Time.get_ticks_usec()
		var shown := false
		# The pose's graded tiles show in its frame, whatever the road chunks do: Rust handed
		# them over once.
		var graded_ms := _show_graded(_preview)
		if is_valid and bool(_preview.get("has_road_chunks", false)):
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
	_graded_pending = int(frame.get("graded_pending", 0))
	var graded = _graded_renderer()
	return graded.show_preview(frame) if graded != null else 0.0

# Ends the preview's graded tiles: Rust starts a new epoch and the renderer drops the tiles of the
# one that ends (a queued commit's epoch stays until its edit installs).
func _release_graded_preview() -> void:
	_graded_pending = 0
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
	# The commit's hue stays with its preview until the hold releases.
	_drop_held_hue()
	_held_hue = _hue
	_hue = null
	_hue_revision = -1
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

func _drop_held_hue() -> void:
	if _held_hue != null:
		_held_hue.queue_free()
		_held_hue = null

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
	simulation_node.road_tool_event("cancel")
	simulation_node.road_tool_pointer_at(pos)
	_read_press(simulation_node.road_tool_event("press"))
	_queue_preview_update()

func _queue_preview_update() -> void:
	if preview_metrics != null:
		_preview_input_us = Time.get_ticks_usec()
	# Coalesce input bursts; the frame previews once in _process.
	_preview_update_pending = true

# The road tool draws no blueprint mesh (its preview is road chunks): no per-frame writes to the
# base's blueprint material.
func _update_blueprint_visuals() -> void:
	pass

# A press (or Enter's `confirm`) on the session: places a point, grabs a freeform grip, or sends
# the stroke. True when a commit was queued.
func _session_press(event_name: String = "press") -> bool:
	var start_us := Time.get_ticks_usec()
	var result: Dictionary = simulation_node.road_tool_event(event_name)
	_read_press(result)
	if not result.has("points"):
		return false
	var points: PackedVector3Array = result["points"]
	var validation: Dictionary = result["preview"]
	if bool(result.get("fresh", false)):
		if preview_metrics != null:
			preview_metrics.sample(points, start_us)
		_adopt_session_preview(validation, points, start_us, Time.get_ticks_usec())
	var request_id := int(result.get("request_id", 0))
	if request_id == 0:
		# Rust logged the refusal on events; the stroke stays editable and shows why.
		_show_preview(points)
		return false
	# A click that previewed a pose of its own shows that pose's graded tiles first.
	if int(validation.get("sequence", 0)) != _preview_drawn_sequence:
		_show_graded(validation)
	_commit_request_id = request_id
	# The commit adopts this epoch's graded tiles; they show until its edit installs.
	_graded_commit_epoch = simulation_node.release_road_preview_terrain()
	_graded_pending = 0
	_commit_points = points
	_commit_validation = validation
	return true

func _read_press(result: Dictionary) -> void:
	_session_phase = int(result.get("phase", _session_phase))

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

func _poll_road_commit_result() -> void:
	if _commit_request_id == 0:
		return
	var result: Variant = simulation_node.get_road_commit_result(_commit_request_id)
	if not result is Dictionary:
		return
	var committed := bool(result.get("committed", false))
	if _road_debug_enabled:
		MetrumDebug.log("road", "road_commit_result request_id=%d committed=%s detail=%s" % [_commit_request_id, committed, result.get("detail", "")])
	_commit_request_id = 0
	# A committed end on the map edge asks about an external connection (the road is in the graph
	# once acknowledged); the chain ends there so the dialog never sits over a stroke still
	# following the pointer off the map.
	var border_node := _border_candidate(_commit_points) if committed else -1
	if not committed:
		var graded = _graded_renderer()
		if graded != null and _graded_commit_epoch >= 0:
			graded.release_preview(_graded_commit_epoch, _graded_commit_epoch)
	else:
		# The preview shows until the commit's own chunks are installed (`ROAD-68-E`).
		_hold_preview_until(int(result.get("generation", 0)))
	_read_press(simulation_node.road_tool_event("committed" if committed else "refused"))
	if committed:
		if border_node >= 0:
			_read_press(simulation_node.road_tool_event("cancel"))
		_clear_preview_visual()
		_clear_preview_cache()
	elif _is_editable():
		_show_rejected_commit()
	if border_node >= 0:
		_prompt_border_connection(border_node)
	_commit_points = PackedVector3Array()
	_commit_validation = {}
	_graded_commit_epoch = -1

# The rejection stays on the stroke until the pointer or the context moves.
func _show_rejected_commit() -> void:
	var rejected := _commit_validation.duplicate(false)
	rejected["is_valid"] = false
	rejected["is_pending"] = false
	rejected["invalid_reason"] = "road_commit_rejected"
	# Its graded tiles were handed over at the click.
	for key in ["graded_tiles", "graded_removed", "graded_pending"]:
		rejected.erase(key)
	rejected["surface_generation"] = simulation_node.get_road_tool_surface_generation()
	rejected["sequence"] = _preview_sequence + 1
	_preview_sequence += 1
	_preview = rejected
	_preview_points = _commit_points
	_show_preview(_commit_points)

# The border connection candidate at either end of a committed stroke, -1 for none.
func _border_candidate(points: PackedVector3Array) -> int:
	if points.size() < 2:
		return -1
	var total_start_us := Time.get_ticks_usec()
	var candidate: int = simulation_node.check_border_candidate(points[0])
	if candidate < 0:
		candidate = simulation_node.check_border_candidate(points[points.size() - 1])
	if _road_debug_enabled:
		MetrumDebug.log("road", "border_check_detail candidate=%d total_ms=%.3f" % [candidate, float(Time.get_ticks_usec() - total_start_us) / 1000.0])
	return candidate

## Drops the stroke and ends the chain (right-click, Esc, another tool or world).
func cancel_road():
	if _has_session():
		_read_press(simulation_node.road_tool_event("cancel"))
	_clear_preview_visual()
	_preview_update_pending = false
	_clear_preview_cache()
	if gizmos != null:
		gizmos.clear()
	_clear_hue()
	if hud != null:
		hud.clear()

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


# The session's cursor for the pointer's camera ray (Shift draws free, Ctrl breaks a freeform
# joint while dragging its handle), sent once per frame by
# the base tool and once per click.
func get_world_mouse_pos() -> Vector3:
	var camera := get_viewport().get_camera_3d()
	if camera == null or not _has_session():
		is_valid = false
		return Vector3.ZERO
	var mouse_screen := get_viewport().get_mouse_position()
	var cursor: Variant = simulation_node.road_tool_pointer(camera.project_ray_origin(mouse_screen), camera.project_ray_normal(mouse_screen), Input.is_key_pressed(KEY_SHIFT), Input.is_key_pressed(KEY_CTRL))
	is_valid = cursor != null
	return cursor if is_valid else Vector3.ZERO

## Whether the latest preview is for the stroke as it stands (benchmarks and probes wait on it).
func stroke_preview_current() -> bool:
	if _preview.is_empty() or _preview_update_pending:
		return false
	return _session_points.size() >= 2 and _preview_points == _session_points

func _clear_preview_cache() -> void:
	# The session's bridge forgets its standing preview too: the next frame previews afresh.
	if _has_session():
		simulation_node.road_tool_event("forget_preview")
	_preview = {}
	_preview_points = PackedVector3Array()
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
