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
## the pose rests) and commit as one undo step. The next stroke's lanes, median, kerbside parking and
## attributes come from a road type (`ROAD-101`, `PARK-05`): `set_road_type` reads the type's
## cross-section from Rust's `get_road_types` and sets the lanes, the median through
## `set_road_tool_median`, the parking through `set_road_tool_parking`, and the type's speed,
## sidewalks, zoning and street lights through `set_road_tool_attributes` (a walkway, which has
## no speed, sends the built-ins' defaults); Rust holds them for every later preview and commit.
## R keeps the last road type picked (2-Lane Road at first), X picks the Walkway. `set_lanes`
## and `set_median` stay the two low-level setters (benchmarks and probes write them). The tool
## sends its whole intent to Rust again whenever it is switched on (`reassert_intent`), so
## whatever set Rust's intent directly meanwhile (the debug replay) never outlasts the tool's
## readout. Rust stays the guard: the stroke's check refuses a median its lanes cannot carry.
## A stroke over zoned land takes it (`ZONE-09`): the preview outlines the lots Rust says the
## stroke removes (`demolish_lots`) with the bulldoze tool's highlight and the HUD names the
## buildings it demolishes (`demolish_count`); plotted land refuses with its reason.
## Rust methods called: road_tool_pointer(), road_tool_pointer_at(), road_tool_event(),
## road_tool_set_option(), road_tool_frame(), set_road_tool_median(),
## set_road_tool_parking(), set_road_tool_attributes(), get_road_types(),
## advance_road_preview_terrain(), release_road_preview_terrain(), get_road_commit_result(),
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
const RoadTypeMenu := preload("res://scripts/ui/road_type_menu.gd")
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

# The next stroke's lanes; Rust's session holds them too (`lanes`).
var fwd_lanes: int = 1:
	set(count):
		fwd_lanes = count
		_send_lanes()
var bkw_lanes: int = 1:
	set(count):
		bkw_lanes = count
		_send_lanes()

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
## Continuous: a stroke leaves along the road or the chain it starts on (the session's tangent
## constraint: straight strokes and simple curves; a chain's S-curve and freeform, round CT).
var continuous: bool = false:
	set(enabled):
		continuous = enabled
		_send_option("continuous", enabled)
var zoning_grid_snap_enabled: bool = false:
	set(enabled):
		zoning_grid_snap_enabled = enabled
		_send_option("zoning_grid", enabled)
# The other snapping options of the options row's Snapping row (`ROAD-112` round E), sent under
# Rust's names; their defaults are the session's (U2: network snap and guide lines on, the angle
# lock and the length step off). 0 turns a step off.
var network_snap_enabled: bool = true:
	set(enabled):
		network_snap_enabled = enabled
		_send_option("network_snap", enabled)
var guide_lines_enabled: bool = true:
	set(enabled):
		guide_lines_enabled = enabled
		_send_option("guide_lines", enabled)
## Angle lock step in degrees: 0 (free), 15, 30, 45 or 90.
var angle_step_deg: float = 0.0:
	set(step):
		angle_step_deg = step
		_send_option("angle_step_deg", step)
## Length step in metres, 0 for off.
var length_step_m: float = 0.0:
	set(step):
		length_step_m = step
		_send_option("length_step_m", step)
## Straight grade (U3): the grade line between the ends' heights even with both handles on the
## ground; the T toggle's and the checkbox's state, held by Rust's session.
var straight_grade: bool = false
signal straight_grade_toggled(enabled: bool)
# The next stroke's median (`ROAD-80`) as last set in Rust: style 0 none, 1 painted, 2 kerbed
# grass, 3 kerbed paving, and its width with kerbs, in metres.
var median_style: int = 0
var median_width: float = 0.0
# The next stroke's kerbside parking (`PARK-05`) as last set in Rust: bit 0 the left kerb, bit 1
# the right, looking along the stroke.
var parking: int = 0
# The next stroke's road-type attributes (`ROAD-101`) as last set in Rust: the speed limit in km/h, the sides with a sidewalk in the stroke's direction (Rust's code, 0 none,
# 1 left, 2 right, 3 both; a shoulder on a side without), zoning and street lights.
var speed_kmh: int = 50
var sidewalks: int = RoadTypeMenu.SIDEWALKS_BOTH
var zoning: bool = true
var street_lights: bool = true
## The built-in type R starts with and falls back to when the picked type is gone.
const DEFAULT_ROAD_TYPE := "builtin:road_2l"
## The built-in Walkway type, X's pick.
const WALKWAY_ROAD_TYPE := "builtin:walkway"
## The road type the next stroke draws (`ROAD-101`), "" before the first pick.
var road_type_id: String = ""
## The last car road type picked: R draws it again.
var last_road_type_id: String = DEFAULT_ROAD_TYPE
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
## Each frame's gizmo upload and label placement milliseconds while the preview is measured
## (`begin_preview_measurement`).
var gizmo_upload_ms := PackedFloat32Array()
## The same frames' parts: the gizmo upload, the chips' wording and their placement.
var gizmo_part_ms := {"gizmos": PackedFloat32Array(), "label_words": PackedFloat32Array(), "label_place": PackedFloat32Array()}
# This frame's chip wording milliseconds (`_session_frame`).
var _label_word_ms := 0.0

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
var _road_preview_material: ShaderMaterial
# The outlines of the lots the previewed stroke removes (`ZONE-09`), made on first use.
var _demolish_mesh: MeshInstance3D = null
# The latest pose's graded tiles still to stage (`TERRAIN-07-E`, T27 (d)): each frame without a
# new pose stages the next share until none is left.
var _graded_pending := 0
# The preview epoch a queued commit adopted: its graded tiles show until the commit's edit
# installs the committed ones, or until the commit is refused.
var _graded_commit_epoch := -1
# Whether the tool was active last frame (switching it on re-asserts the intent).
var _was_active := false

# The session's last frame: its phase (0 idle, 1 placing, 2 waiting for the acknowledgement),
# what took the cursor, the HUD numbers and the stroke.
var _session_phase := 0
var _session_snap := "free"
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
	if blueprint_mesh:
		_road_preview_material = WorldMaterials.road_preview_material()
		blueprint_mesh.material_override = _road_preview_material
	if MetrumDebug.is_enabled("perf"):
		begin_preview_measurement()
	attach_hud(RoadToolHud.new())
	reassert_intent()

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

## Sends the tool's whole intent to Rust's session: lanes, every snapping option, continuous,
## straight grade, median, parking and attributes, as this script reads them out.
## Called when the tool is switched on, so whatever set Rust's intent directly meanwhile (the
## debug replay) is undone before the next stroke.
func reassert_intent() -> void:
	if not _has_session():
		return
	_send_lanes()
	_send_option("network_snap", network_snap_enabled)
	_send_option("guide_lines", guide_lines_enabled)
	_send_option("zoning_grid", zoning_grid_snap_enabled)
	_send_option("angle_step_deg", angle_step_deg)
	_send_option("length_step_m", length_step_m)
	_send_option("continuous", continuous)
	_send_option("straight_grade", straight_grade)
	simulation_node.set_road_tool_median(median_style, median_width)
	simulation_node.set_road_tool_parking(parking)
	simulation_node.set_road_tool_attributes(speed_kmh, sidewalks, zoning, street_lights)

func _process(delta):
	if preview_metrics != null and DisplayServer.get_name() == "headless":
		preview_metrics.frame(_preview_drawn_sequence, false)
	_poll_road_commit_result()
	_release_swapped_preview()
	if active and not _was_active:
		reassert_intent()
	_was_active = active
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
	var gizmo_ms := 0.0
	_label_word_ms = 0.0
	if active and _has_session():
		gizmo_ms = _session_frame(moved)
	_place_labels(gizmo_ms)

# Projects and places the HUD's chips each frame, so they stay the same pixel size at any zoom;
# records the frame's gizmo upload plus placement while measured.
func _place_labels(gizmo_ms: float) -> void:
	if hud == null or not is_inside_tree():
		return
	hud.simulation_node = simulation_node
	var label_ms: float = hud.place(get_viewport().get_camera_3d())
	if preview_metrics != null and gizmo_ms + _label_word_ms + label_ms > 0.0:
		gizmo_upload_ms.append(gizmo_ms + _label_word_ms + label_ms)
		gizmo_part_ms["gizmos"].append(gizmo_ms)
		gizmo_part_ms["label_words"].append(_label_word_ms)
		gizmo_part_ms["label_place"].append(label_ms)

func _snap_to_roads_enabled() -> bool:
	return true

func _has_session() -> bool:
	return is_instance_valid(simulation_node) and simulation_node.has_method("road_tool_frame")

func _send_option(option: String, value: Variant) -> void:
	if _has_session():
		simulation_node.road_tool_set_option(option, value)

func _send_lanes() -> void:
	_send_option("lanes", Vector2i(fwd_lanes, bkw_lanes))

## Whether a stroke is being drawn: a point is placed, or a sent stroke awaits its answer.
func is_drawing() -> bool:
	return _session_phase != PHASE_IDLE

# Whether the stroke is the player's to change: points are being placed or a freeform draft is
# in its edit phase.
func _is_editable() -> bool:
	return _session_phase == PHASE_PLACING or _session_phase == PHASE_EDITING

# One frame of the session: the scripted pointer, then the frame record, its preview when the
# stroke, its type or the context changed, or the next share of the pose's graded tiles; then
# the gizmos and the HUD's chips. Returns the gizmo upload's milliseconds.
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
	if hud != null:
		var word_us := Time.get_ticks_usec()
		if hud.show_labels(frame):
			_label_word_ms = float(Time.get_ticks_usec() - word_us) / 1000.0
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
	_session_snap = String(frame.get("snap", "free"))
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
	return _hue.show_hue(simulation_node.road_tool_hue())

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

func _zoning_snap_enabled() -> bool:
	return zoning_grid_snap_enabled

## Sets the next stroke's lanes (0..4 each; 0 + 0 is a walkway).
func set_lanes(fwd: int, bkw: int) -> void:
	fwd_lanes = clamp(fwd, 0, 4)
	bkw_lanes = clamp(bkw, 0, 4)
	_queue_preview_update()

## Turns straight grade on or off (T, the options checkbox).
func set_straight_grade(enabled: bool) -> void:
	if enabled == straight_grade:
		return
	straight_grade = enabled
	_send_option("straight_grade", enabled)
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

## Sets the next stroke's median through Rust and returns its refusal key ("" when set). A
## refused median leaves the tool's median as it was.
func set_median(style: int, width: float) -> String:
	if style == 0:
		width = 0.0
	var reason: String = simulation_node.set_road_tool_median(style, width)
	if not reason.is_empty():
		return reason
	median_style = style
	median_width = width
	_intent_changed()
	return ""

## Sets the next stroke's kerbside parking through Rust (`PARK-05`) and returns its refusal key
## ("" when set). A refused code leaves the tool's parking as it was.
func set_parking(code: int) -> String:
	var reason: String = simulation_node.set_road_tool_parking(code)
	if not reason.is_empty():
		return reason
	parking = code
	_intent_changed()
	return ""

## Sets the next stroke's speed limit, sidewalk sides, zoning and street lights through Rust and
## returns its refusal key ("" when set; `speed_out_of_range` off Rust's grid or
## `sidewalks_out_of_range`, keeping the tool's attributes).
func set_attributes(speed: int, with_sidewalks: int, with_zoning: bool, with_lights: bool) -> String:
	var reason: String = simulation_node.set_road_tool_attributes(speed, with_sidewalks, with_zoning, with_lights)
	if not reason.is_empty():
		return reason
	speed_kmh = speed
	sidewalks = with_sidewalks
	zoning = with_zoning
	street_lights = with_lights
	_intent_changed()
	return ""

## Draws the next strokes with road type `id` (`ROAD-101`): its lanes, median, parking, speed,
## sidewalks, zoning and street lights as Rust's `get_road_types` lists them. Returns "" when
## picked, else the refusal key: the type's `invalid_reason` when it is unusable,
## `road_type_unknown` when no type has `id`; a refused pick leaves the tool as it was (a usable
## type's median, parking and speed always pass). `O(types x parts)` on a UI action.
func set_road_type(id: String) -> String:
	var library: Dictionary = simulation_node.get_road_types()
	var type := _road_type(id, library)
	if type.is_empty():
		return "road_type_unknown"
	if not bool(type.get("usable", false)):
		return String(type.get("invalid_reason", "road_type_unknown"))
	var attributes := RoadTypeMenu.tool_attributes(type, library.get("speeds", {}))
	var reason := set_attributes(attributes[0], attributes[1], attributes[2], attributes[3])
	if not reason.is_empty():
		return reason
	reason = set_median(int(type.get("median_style", 0)), float(type.get("median_width", 0.0)))
	if reason.is_empty():
		reason = set_parking(int(type.get("parking", 0)))
	if not reason.is_empty():
		return reason
	set_lanes(int(type.get("fwd_lanes", 0)), int(type.get("bkw_lanes", 0)))
	road_type_id = id
	if id != WALKWAY_ROAD_TYPE:
		last_road_type_id = id
	road_type_changed.emit(id)
	return ""

## Picks the current type again after the library changed (`ROAD-101`): an edited type's new
## layout draws from the next stroke on; built roads keep theirs. A type that is gone or no longer
## usable falls back to the 2-Lane Road (the Walkway stays the Walkway).
func reload_road_type() -> void:
	if not bool(_road_type(last_road_type_id, simulation_node.get_road_types()).get("usable", false)):
		last_road_type_id = DEFAULT_ROAD_TYPE
	if not road_type_id.is_empty() and not set_road_type(road_type_id).is_empty():
		set_road_type(last_road_type_id)

func _road_type(id: String, library: Dictionary) -> Dictionary:
	for type in library.get("types", []):
		if String(type.get("id", "")) == id:
			return type
	return {}

# Rust's straight grade, median, parking or attributes changed: the stroke previews again (the
# session compares them itself).
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
	_log_click_detail(state_before, is_valid, mouse_ms, 0.0, 0.0, commit_ms, total_start_us)

# Shows the latest preview's chunks in this frame; an invalid stroke shows only its reason.
func _show_preview(_points: PackedVector3Array) -> void:
	is_valid = bool(_preview.get("is_valid", false))
	_update_road_preview_material(_preview)
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

func _update_blueprint_visuals() -> void:
	# Road status is applied with its matching geometry, after this frame's validation.
	pass

func _update_road_preview_material(preview: Dictionary) -> void:
	if _road_preview_material == null:
		return
	_road_preview_material.set_shader_parameter("forward_lanes", fwd_lanes)
	_road_preview_material.set_shader_parameter("backward_lanes", bkw_lanes)
	var state := 1 if bool(preview.get("is_pending", false)) else (0 if bool(preview.get("is_valid", false)) else 2)
	_road_preview_material.set_shader_parameter("placement_state", state)

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
	mark_network_nodes_dirty()
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
	tangent_ms: float,
	path_ms: float,
	commit_ms: float,
	total_start_us: int
) -> void:
	if not _road_debug_enabled:
		return
	MetrumDebug.log(
		"road",
		"road_click_detail state=%d valid=%s mouse_ms=%.3f tangent_ms=%.3f path_ms=%.3f commit_ms=%.3f total_ms=%.3f"
		% [
			state_before,
			str(valid_after),
			mouse_ms,
			tangent_ms,
			path_ms,
			commit_ms,
			float(Time.get_ticks_usec() - total_start_us) / 1000.0,
		]
	)
