// SPDX-License-Identifier: GPL-2.0-only

//! The road tool's tick (`ROAD-112` round R2-2): one typed call a frame. `road_tool.gd` hands
//! over the camera and the mouse (or a scripted world point); the tick answers the sent stroke's
//! commit, resolves the pointer, brings the draft up to date, previews the stroke when it changed
//! (or steps the batch, or stages the next share of the pose's graded tiles), compares the
//! gizmos, the heights, the hue and the chips with what the script last drew, places the chips
//! for the camera and returns flags: what the script still has to do this frame. A frame that
//! changed nothing returns 0, crosses no array and allocates nothing; the tick builds no
//! dictionary outside the preview's own payload (`road_tool_snapshot` builds the old frame
//! record on demand, for tests and probes). The tick records its stage times into a fixed ring
//! (`road_tool_timings`), so the frame path has no timers in GDScript.

use super::labels::ChipView;
use super::*;
use crate::nodes::sim::road_tool::frame::DraftRun;
use godot::classes::{Camera3D, Time};

/// Tick flag: a new preview stands (`road_tool_preview`): a new pose's, or a batch's exact
/// overlay or refusal once its tier 2 settled.
pub(super) const TICK_PREVIEW: i64 = 1;
/// Tick flag: the stroke went away; its preview is dropped.
pub(super) const TICK_PREVIEW_CLEARED: i64 = 1 << 1;
/// Tick flag: a new batch pose's tier 1 (`road_tool_batch_ghost`), the ghost.
pub(super) const TICK_BATCH: i64 = 1 << 2;
/// Tick flag: the batch ghost goes.
pub(super) const TICK_BATCH_CLEARED: i64 = 1 << 3;
/// Tick flag: the gizmos changed (`road_tool_gizmo_layout`, `road_tool_gizmo_bytes`).
pub(super) const TICK_GIZMOS: i64 = 1 << 4;
/// Tick flag: the footprint hue's revision moved (`road_tool_hue_join`, after the preview shows:
/// round HF2's pairing).
pub(super) const TICK_HUE: i64 = 1 << 5;
/// Tick flag: the chips changed (`road_tool_chip_*`, corners included).
pub(super) const TICK_CHIPS: i64 = 1 << 6;
/// Tick flag: only the chips' corners changed (`road_tool_chip_corners`).
pub(super) const TICK_CORNERS: i64 = 1 << 7;
/// Tick flag: the heights (grade, handles, the HUD's heights and slopes, the draft's runs)
/// changed (`road_tool_heights`).
pub(super) const TICK_HEIGHTS: i64 = 1 << 8;
/// Tick flag: the pose's graded tiles staged a share (`road_tool_graded_changes`).
pub(super) const TICK_GRADED: i64 = 1 << 9;
/// Tick flag: the sent stroke's answer arrived (`road_tool_commit_answer`); the tick returned
/// before the frame's work, which the next tick of the same frame does.
pub(super) const TICK_ANSWER: i64 = 1 << 10;
/// Tick flag: the phase or the mode changed (`road_tool_phase`, `road_tool_mode`).
pub(super) const TICK_PHASE: i64 = 1 << 11;
/// Tick flag: the cursor moved, or met or left a surface (`road_tool_cursor`).
pub(super) const TICK_CURSOR: i64 = 1 << 12;

// Ticks the ring holds; older ones are overwritten.
const RING_TICKS: usize = 4096;
// The bytes of a position in the vertex buffer.
const VERTEX_BYTES: usize = 12;

/// What the tick keeps between frames: what the script last drew, the buffers it uploads, what
/// the tick left for the script to fetch, and the stage ring.
#[derive(Default)]
pub(super) struct TickState {
    // The gizmos as the script last drew them (`None` until the first draw, and after the script
    // dropped its drawing), and their upload buffers (`road_tool_gizmo_bytes`).
    drawn: Option<Gizmos>,
    gizmo_bytes: GizmoBytes,
    // The heights last reported.
    heights: Option<HeightsRecord>,
    // The hue revision last reported.
    hue: Option<u64>,
    // The cursor, the phase and the mode last reported.
    cursor: Option<Option<Vector3>>,
    phase: Option<(Phase, DrawMode)>,
    // The previews handed to the script (a pose, a batch's settled overlay, a click's fresh
    // preview, a refused commit's display): each dictionary carries its number as `sequence`.
    pub(super) sequence: i64,
    // What the last tick left for the script: the batch ghost, the graded share, the answer.
    batch_ghost: Option<VarDictionary>,
    graded: Option<VarDictionary>,
    answer: Option<VarDictionary>,
    ring: TickRing,
}

impl TickState {
    // The frame's gizmos (their bytes rebuilt on a change), heights, the hue's revision and the
    // cursor against what was last reported; the flags of those that changed. `O(gizmo points)`;
    // allocates nothing once the buffers have grown.
    fn compare_drawn(&mut self, frame: &RoadToolFrame, hue_revision: u64) -> i64 {
        let mut flags = 0;
        if self.drawn.as_ref() != Some(&frame.gizmos) {
            self.drawn
                .get_or_insert_with(Gizmos::default)
                .clone_from(&frame.gizmos);
            self.gizmo_bytes.fill(&frame.gizmos);
            flags |= TICK_GIZMOS;
        }
        if !self.heights.as_ref().is_some_and(|heights| heights.same(frame)) {
            self.heights.get_or_insert_with(HeightsRecord::default).copy(frame);
            flags |= TICK_HEIGHTS;
        }
        if self.hue != Some(hue_revision) {
            self.hue = Some(hue_revision);
            flags |= TICK_HUE;
        }
        let cursor = frame.cursor.valid.then_some(frame.cursor.position);
        if self.cursor != Some(cursor) {
            self.cursor = Some(cursor);
            flags |= TICK_CURSOR;
        }
        flags
    }
}

// The gizmos' upload buffers in the server's layout: the line quads' positions and attributes
// (`strip_quads`), the handles' positions and their kinds as UVs, in reused buffers.
#[derive(Default)]
struct GizmoBytes {
    quads: LineQuadBuffers,
    line_vertices: Vec<u8>,
    line_attributes: Vec<u8>,
    handles: Vec<u8>,
    handle_attributes: Vec<u8>,
    // The line surface's vertex count and the handles' count.
    layout: Vector2i,
}

impl GizmoBytes {
    // The gizmos' bytes into the kept buffers. `O(gizmo points)`; allocates nothing once the
    // buffers have grown.
    fn fill(&mut self, gizmos: &Gizmos) {
        self.quads.fill(&gizmos.line_points, &gizmos.line_starts, |strip| {
            line_style(gizmos.line_classes[strip])
        });
        write_vectors(&mut self.line_vertices, &self.quads.vertices);
        write_floats(&mut self.line_attributes, &self.quads.attributes);
        write_vectors(&mut self.handles, &gizmos.handles);
        // One point surface for every handle (round V2): each point's kind as its UV.
        let bytes = &mut self.handle_attributes;
        bytes.clear();
        bytes.resize(gizmos.handle_kinds.len() * 8, 0);
        for (chunk, kind) in bytes.as_chunks_mut::<8>().0.iter_mut().zip(&gizmos.handle_kinds) {
            chunk[..4].copy_from_slice(&(handle_kind_code(*kind) as f32).to_le_bytes());
            chunk[4..].copy_from_slice(&0.0f32.to_le_bytes());
        }
        self.layout = Vector2i::new(
            self.quads.vertices.len() as i32,
            gizmos.handles.len() as i32,
        );
    }
}

// Positions as the vertex buffer holds them: three little-endian `f32` each.
fn write_vectors(bytes: &mut Vec<u8>, vectors: &[Vector3]) {
    bytes.clear();
    bytes.resize(vectors.len() * VERTEX_BYTES, 0);
    for (chunk, vector) in bytes.as_chunks_mut::<VERTEX_BYTES>().0.iter_mut().zip(vectors) {
        chunk[..4].copy_from_slice(&vector.x.to_le_bytes());
        chunk[4..8].copy_from_slice(&vector.y.to_le_bytes());
        chunk[8..].copy_from_slice(&vector.z.to_le_bytes());
    }
}

fn write_floats(bytes: &mut Vec<u8>, floats: &[f32]) {
    bytes.clear();
    bytes.resize(floats.len() * 4, 0);
    for (chunk, float) in bytes.as_chunks_mut::<4>().0.iter_mut().zip(floats) {
        *chunk = float.to_le_bytes();
    }
}

// The frame's heights as last reported (`road_tool_heights`' keys).
#[derive(Default, PartialEq)]
struct HeightsRecord {
    grade: Option<[f32; 2]>,
    handles: [Option<crate::nodes::sim::road_tool::height::handles::HeightHandle>; 2],
    heights: [Option<f64>; 3],
    piece_slopes: Vec<f64>,
    runs: Vec<DraftRun>,
    refusal: Option<&'static str>,
}

impl HeightsRecord {
    fn same(&self, frame: &RoadToolFrame) -> bool {
        self.grade == frame.stroke.grade
            && self.handles == frame.height_handles
            && self.heights == Self::heights_of(frame)
            && self.piece_slopes == frame.piece_slopes_percent
            && self.runs == frame.draft.runs
            && self.refusal == frame.draft.refusal
    }

    fn heights_of(frame: &RoadToolFrame) -> [Option<f64>; 3] {
        let hud = &frame.hud;
        [hud.start_height_m, hud.end_height_m, hud.slope_percent]
    }

    // Takes `frame`'s heights, keeping the buffers.
    fn copy(&mut self, frame: &RoadToolFrame) {
        self.grade = frame.stroke.grade;
        self.handles = frame.height_handles;
        self.heights = Self::heights_of(frame);
        self.piece_slopes.clone_from(&frame.piece_slopes_percent);
        self.runs.clone_from(&frame.draft.runs);
        self.refusal = frame.draft.refusal;
    }
}

// One tick's stage times: its start on Godot's ticks clock, its whole, the preview call (a
// pose's `preview_stroke` or the batch's tiers), the chips (their words and placement, and the
// HUD's fetch when it follows), the gizmos' comparison and bytes, the hue's join, and its flags.
#[derive(Clone, Copy, Default)]
struct TickSample {
    start_us: i64,
    total_us: f32,
    preview_us: f32,
    chips_us: f32,
    gizmos_us: f32,
    hue_us: f32,
    flags: i32,
}

// The last `RING_TICKS` ticks' samples, allocated once.
#[derive(Default)]
struct TickRing {
    samples: Vec<TickSample>,
    next: usize,
}

impl TickRing {
    fn push(&mut self, sample: TickSample) {
        if self.samples.len() < RING_TICKS {
            if self.samples.capacity() == 0 {
                self.samples.reserve_exact(RING_TICKS);
            }
            self.samples.push(sample);
        } else {
            self.samples[self.next] = sample;
        }
        self.next = (self.next + 1) % RING_TICKS;
    }

    // The latest tick's sample (a fetch that follows it adds its time to it).
    fn last_mut(&mut self) -> Option<&mut TickSample> {
        let index = (self.next + RING_TICKS - 1) % RING_TICKS;
        self.samples.get_mut(index)
    }

    // The samples oldest first.
    fn ordered(&self) -> impl Iterator<Item = &TickSample> {
        let split = if self.samples.len() < RING_TICKS {
            0
        } else {
            self.next
        };
        self.samples[split..].iter().chain(&self.samples[..split])
    }
}

fn elapsed_us(since: Instant) -> f32 {
    since.elapsed().as_secs_f32() * 1.0e6
}

#[godot_api(secondary)]
impl SimulationNode {
    /// The road tool's frame (round R2-2), once a frame while the tool is active: the pointer is
    /// `camera`'s ray through `mouse` (no camera: the pointer stays), Shift and Ctrl as for
    /// `road_tool_pointer`. Returns the flags of what the script has to do this frame (1 a new
    /// preview stands, 2 the preview went, 4 a batch ghost, 8 the ghost goes, 16 the gizmos
    /// changed, 32 the hue's revision moved, 64 the chips changed, 128 only their corners did,
    /// 256 the heights changed, 512 the pose's graded tiles staged a share, 1024 the sent
    /// stroke's answer arrived (the tick stopped there: draw it, then tick again), 2048 the phase
    /// or mode changed, 4096 the cursor changed); 0 when nothing changed, and then it allocates
    /// nothing. No preview while a sent stroke waits for its answer. `O(gizmo points + chips
    /// (chips + handles))` beside the preview, which runs only when the stroke, its type, its
    /// grade or the context changed.
    #[func]
    pub fn road_tool_tick(
        &mut self,
        camera: Option<Gd<Camera3D>>,
        mouse: Vector2,
        shift: bool,
        ctrl: bool,
    ) -> i64 {
        let pointer = camera.as_ref().map(|camera| Pointer::Ray {
            origin: camera.project_ray_origin(mouse),
            dir: camera.project_ray_normal(mouse),
        });
        self.road_tool_tick_with(pointer, shift, ctrl, camera.as_ref())
    }

    /// `road_tool_tick` with a scripted pointer at world `position` (tests, benchmarks and
    /// probes, as `set_scripted_pointer`): the cursor exactly there, neither picked nor snapped;
    /// `camera` places the chips (none: no corners).
    #[func]
    pub fn road_tool_tick_at(
        &mut self,
        camera: Option<Gd<Camera3D>>,
        position: Vector3,
        ctrl: bool,
    ) -> i64 {
        self.road_tool_tick_with(Some(Pointer::Scripted(position)), false, ctrl, camera.as_ref())
    }

    /// The standing preview's dictionary (`preview_road`'s keys, with `pose` and `sequence`),
    /// `{}` without one: fetched on the tick's preview flag.
    #[func]
    pub fn road_tool_preview(&self) -> VarDictionary {
        (self.road_tool_session.preview.as_ref())
            .map(|standing| standing.dictionary.clone())
            .unwrap_or_default()
    }

    /// Whether the standing preview is for the session's stroke as it stands, on the current
    /// context (benchmarks and probes wait on it).
    #[func]
    pub fn road_tool_preview_current(&self) -> bool {
        self.road_tool_session.session.frame().stroke.points.len() >= 2
            && self.road_tool_preview_is_current()
    }

    /// The previews handed out so far (`sequence`: a pose's, a batch's settled overlay, a
    /// click's fresh preview, a refused commit's display).
    #[func]
    pub fn road_tool_preview_sequence(&self) -> i64 {
        self.road_tool_session.tick.sequence
    }

    /// The last batch pose's tier 1 (`preview_road_draft`'s keys: the ghost), fetched on the
    /// tick's batch flag; `{}` once fetched.
    #[func]
    pub fn road_tool_batch_ghost(&mut self) -> VarDictionary {
        self.road_tool_session.tick.batch_ghost.take().unwrap_or_default()
    }

    /// The share of the pose's graded tiles the last tick staged (`advance_road_preview_terrain`'s
    /// keys), fetched on the tick's graded flag; `{}` once fetched.
    #[func]
    pub fn road_tool_graded_changes(&mut self) -> VarDictionary {
        self.road_tool_session.tick.graded.take().unwrap_or_default()
    }

    /// The latest pose's graded tiles still to stage (`TERRAIN-07-E`; each tick without a new
    /// pose stages the next share while the stroke is editable; probes wait on 0).
    #[func]
    pub fn road_tool_graded_pending(&self) -> i64 {
        let context = self.road_tool.context.lock().expect("road tool context lock poisoned");
        context.graded.pending_tiles() as i64
    }

    /// The sent stroke's answer the last tick heard (`road_tool_commit_poll`'s dictionary),
    /// fetched on the tick's answer flag; `{}` once fetched.
    #[func]
    pub fn road_tool_commit_answer(&mut self) -> VarDictionary {
        self.road_tool_session.tick.answer.take().unwrap_or_default()
    }

    /// The tool's phase (0 idle, 1 placing, 2 waiting for the commit's answer, 3 a freeform
    /// draft's edit phase).
    #[func]
    pub fn road_tool_phase(&self) -> i64 {
        self.road_tool_phase_placed().0
    }

    /// The drawing mode (0 straight, 1 simple curve, 2 complex curve, 3 freeform).
    #[func]
    pub fn road_tool_mode(&self) -> i64 {
        mode_code(self.road_tool_session.session.frame().state.mode)
    }

    /// The resolved cursor, `null` while no pointer has met a surface.
    #[func]
    pub fn road_tool_cursor(&self) -> Variant {
        self.road_tool_cursor_variant()
    }

    /// The session's stroke of this pose, empty without one (probes and the preview metrics).
    #[func]
    pub fn road_tool_points(&self) -> PackedVector3Array {
        PackedVector3Array::from(self.road_tool_session.session.frame().stroke.points.as_slice())
    }

    /// The gizmos' surfaces as of the last tick: the line surface's vertex count (`x`) and the
    /// handles' count (`y`); the script rebuilds its surfaces when they differ from its own.
    #[func]
    pub fn road_tool_gizmo_layout(&self) -> Vector2i {
        self.road_tool_session.tick.gizmo_bytes.layout
    }

    /// Where the gizmos' dashes keep their on-screen length (`road_tool_line.gdshader`'s
    /// `dash_focus`): the cursor's position as of the last tick, met or not.
    #[func]
    pub fn road_tool_gizmo_focus(&self) -> Vector3 {
        self.road_tool_session.session.frame().cursor.position
    }

    /// One of the gizmos' upload buffers as of the last tick, in the server's layout: `part` 0
    /// the line quads' positions (12 bytes a vertex), 1 their attributes for
    /// `road_tool_line.gdshader` (20 bytes a vertex: the other end, the side and dash, the look;
    /// `strip_quads`), 2 the handles' positions, 3 the handles' kinds as UVs (8 bytes a handle:
    /// 0 placed end, 1 control point, 2 chain start, 3 the live end or grabbed grip, 4 a joint, 5
    /// a handle).
    #[func]
    pub fn road_tool_gizmo_bytes(&self, part: i64) -> PackedByteArray {
        let bytes = &self.road_tool_session.tick.gizmo_bytes;
        PackedByteArray::from(match part {
            0 => bytes.line_vertices.as_slice(),
            1 => bytes.line_attributes.as_slice(),
            2 => bytes.handles.as_slice(),
            _ => bytes.handle_attributes.as_slice(),
        })
    }

    /// The script dropped what it drew (`parts`: the tick's flags of the gizmos, the hue and the
    /// cursor; another tool, a cancel, a held commit's hue): the next tick reports them changed.
    /// The HUD forces its own fetch after it dropped its chips.
    #[func]
    pub fn road_tool_forget_drawn(&mut self, parts: i64) {
        let tick = &mut self.road_tool_session.tick;
        if parts & TICK_GIZMOS != 0 {
            tick.drawn = None;
        }
        if parts & TICK_HUE != 0 {
            tick.hue = None;
        }
        if parts & TICK_CURSOR != 0 {
            tick.cursor = None;
        }
    }

    /// The tick ring's samples, oldest first (benchmarks, once at the end of a run): `start_us`
    /// (Godot's ticks clock), `total_ms` (the whole tick), `preview_ms` (the preview call: a
    /// pose's preview, a batch's tier), `chips_ms` (the chips' words and placement, with the HUD's
    /// fetch that followed the tick), `gizmos_ms` (the gizmos' comparison and bytes), `hue_ms`
    /// (the hue's join after the tick) and `flags`. `clear` empties the ring after reading it.
    #[func]
    pub fn road_tool_timings(&mut self, clear: bool) -> VarDictionary {
        let ring = &mut self.road_tool_session.tick.ring;
        let column = |get: fn(&TickSample) -> f32| {
            ring.ordered()
                .map(|sample| f64::from(get(sample)) / 1000.0)
                .collect::<PackedFloat64Array>()
        };
        let dict = vdict! {
            "start_us": ring.ordered().map(|sample| sample.start_us).collect::<PackedInt64Array>(),
            "total_ms": column(|sample| sample.total_us),
            "preview_ms": column(|sample| sample.preview_us),
            "chips_ms": column(|sample| sample.chips_us),
            "gizmos_ms": column(|sample| sample.gizmos_us),
            "hue_ms": column(|sample| sample.hue_us),
            "flags": ring.ordered().map(|sample| sample.flags).collect::<PackedInt32Array>(),
        };
        if clear {
            ring.samples.clear();
            ring.next = 0;
        }
        dict
    }
}

impl SimulationNode {
    // The tick: the answer (alone), then the pointer, the frame's preview step, and what changed
    // for the script. See `road_tool_tick`.
    fn road_tool_tick_with(
        &mut self,
        pointer: Option<Pointer>,
        shift: bool,
        ctrl: bool,
        camera: Option<&Gd<Camera3D>>,
    ) -> i64 {
        let start = Instant::now();
        let mut sample = TickSample {
            start_us: Time::singleton().get_ticks_usec() as i64,
            ..TickSample::default()
        };
        let mut flags = 0;
        if self.road_tool_session.commit.request_id() != 0
            && let Some(answer) = self.road_tool_take_commit_answer()
        {
            self.road_tool_session.tick.answer = Some(answer);
            flags = TICK_ANSWER | self.road_tool_tick_phase();
        } else {
            if let Some(pointer) = pointer {
                self.road_tool_handle(RoadToolEvent::Pointer {
                    pointer,
                    shift,
                    ctrl,
                });
            }
            let preview_start = Instant::now();
            flags |= self.road_tool_tick_preview();
            if flags & (TICK_PREVIEW | TICK_BATCH) != 0 {
                sample.preview_us = elapsed_us(preview_start);
            }
            let gizmos_start = Instant::now();
            flags |= self.road_tool_tick_drawn();
            sample.gizmos_us = elapsed_us(gizmos_start);
            let chips_start = Instant::now();
            let frame = self.road_tool_session.session.frame();
            let pending = frame.state.phase == Phase::AwaitingAck && frame.stroke.points.len() >= 2;
            self.road_tool_set_labels(pending);
            let view = camera.and_then(ChipView::of);
            flags |= match self.road_tool_session.chips.update(view) {
                labels::CHIPS_CHANGED => TICK_CHIPS,
                labels::CORNERS_CHANGED => TICK_CORNERS,
                _ => 0,
            };
            sample.chips_us = elapsed_us(chips_start);
            flags |= self.road_tool_tick_phase();
        }
        sample.total_us = elapsed_us(start);
        sample.flags = flags as i32;
        self.road_tool_session.tick.ring.push(sample);
        flags
    }

    // The frame's preview step (round D's `road_tool_frame`): drop the preview of a stroke that
    // went, preview a changed stroke, step a running batch, else stage the next share of the
    // pose's graded tiles while the stroke is editable. Nothing while the sent stroke waits.
    fn road_tool_tick_preview(&mut self) -> i64 {
        self.road_tool_update_draft();
        let frame = self.road_tool_session.session.frame();
        let phase = frame.state.phase;
        let has_stroke = frame.stroke.points.len() >= 2;
        let mut flags = 0;
        if phase != Phase::AwaitingAck {
            if !has_stroke {
                if self.road_tool_session.previewed.take().is_some() {
                    self.road_tool_session.preview = None;
                    flags |= TICK_PREVIEW_CLEARED;
                }
                self.road_tool_session.batch_running = false;
                flags |= self.road_tool_clear_batch_ghost();
            } else if !self.road_tool_preview_is_current() {
                let tier1 = self.road_tool_run_preview();
                self.road_tool_hand_preview();
                flags |= TICK_PREVIEW;
                match tier1 {
                    Some(ghost) => {
                        let bridge = &mut self.road_tool_session;
                        bridge.batch_ghost = true;
                        bridge.tick.batch_ghost = Some(ghost);
                        flags |= TICK_BATCH;
                    }
                    None => flags |= self.road_tool_clear_batch_ghost(),
                }
            } else if self.road_tool_session.batch_running
                && let Some(cleared) = self.road_tool_step_batch()
            {
                self.road_tool_hand_preview();
                flags |= TICK_PREVIEW | cleared;
            }
            if flags & (TICK_PREVIEW | TICK_PREVIEW_CLEARED) == 0
                && matches!(phase, Phase::Placing { .. } | Phase::Editing)
                && let Some(graded) = self.road_preview_terrain_step()
            {
                self.road_tool_session.tick.graded = Some(graded);
                flags |= TICK_GRADED;
            }
        }
        // A hue in flight stays in flight: its stroke stands, so `follow` would keep it.
        let bridge = &mut self.road_tool_session;
        if !(has_stroke || phase == Phase::AwaitingAck) {
            bridge.hue.hue().follow(bridge.session.frame());
        }
        flags
    }

    // Numbers the standing preview as the next one handed to the script (`sequence`).
    pub(super) fn road_tool_hand_preview(&mut self) {
        let bridge = &mut self.road_tool_session;
        bridge.tick.sequence += 1;
        if let Some(standing) = &mut bridge.preview {
            standing.dictionary.set("sequence", bridge.tick.sequence);
        }
    }

    // What the script draws, against what it last drew (`TickState::compare_drawn`).
    fn road_tool_tick_drawn(&mut self) -> i64 {
        let bridge = &mut self.road_tool_session;
        let revision = bridge.hue.revision();
        bridge.tick.compare_drawn(bridge.session.frame(), revision)
    }

    // The phase flag when the phase or the mode moved since the last report.
    fn road_tool_tick_phase(&mut self) -> i64 {
        let state = self.road_tool_session.session.frame().state;
        let now = Some((state.phase, state.mode));
        let tick = &mut self.road_tool_session.tick;
        if tick.phase == now {
            0
        } else {
            tick.phase = now;
            TICK_PHASE
        }
    }

    // A fetch that follows the tick (the chips', the hue's join) adds its time to the tick's
    // sample.
    pub(super) fn road_tool_tick_add(&mut self, chips_us: f32, hue_us: f32) {
        if let Some(sample) = self.road_tool_session.tick.ring.last_mut() {
            sample.chips_us += chips_us;
            sample.hue_us += hue_us;
        }
    }

    pub(super) fn road_tool_tick_elapsed(since: Instant) -> f32 {
        elapsed_us(since)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::nodes::sim::road_tool::bench::neighbourhood_view;

    // Round R2-2: two ticks with an unchanged pointer bump no revision. A real session on the
    // bench neighbourhood: hover, then one point placed with the pointer moving (straight mode);
    // each new pointer reports the gizmos, the cursor and (once a point is down) the heights, the
    // same pointer again reports nothing.
    #[test]
    fn unchanged_pointer_bumps_no_revision() {
        let view = neighbourhood_view(false, 0, 2_048.0);
        let mut session = RoadToolSession::default();
        let mut tick = TickState::default();
        let point = |session: &mut RoadToolSession, at: Vector3| {
            session.handle_event(
                &view,
                RoadToolEvent::Pointer {
                    pointer: Pointer::Scripted(at),
                    shift: false,
                    ctrl: false,
                },
            );
            session.update_draft(&view);
        };
        let hover = Vector3::new(-300.0, 0.0, -300.0);
        point(&mut session, hover);
        let first = tick.compare_drawn(session.frame(), 0);
        assert_ne!(first & TICK_CURSOR, 0, "the first pointer reports the cursor");
        point(&mut session, hover);
        assert_eq!(tick.compare_drawn(session.frame(), 0), 0, "a still hover reports nothing");
        session.handle_event(&view, RoadToolEvent::Press(PointerButton::Left));
        for step in 1..4 {
            let at = hover + Vector3::new(20.0 * step as f32, 0.0, 5.0);
            point(&mut session, at);
            let moved = tick.compare_drawn(session.frame(), 0);
            assert_ne!(moved & TICK_GIZMOS, 0, "a moved live end reports the gizmos");
            assert_ne!(moved & TICK_CURSOR, 0);
            let drawn = tick.drawn.clone();
            let bytes = tick.gizmo_bytes.line_vertices.clone();
            point(&mut session, at);
            assert_eq!(
                tick.compare_drawn(session.frame(), 0),
                0,
                "the same pointer again reports nothing"
            );
            assert_eq!(tick.drawn, drawn);
            assert_eq!(tick.gizmo_bytes.line_vertices, bytes);
        }
        assert_eq!(tick.compare_drawn(session.frame(), 1), TICK_HUE, "a new hue revision alone");
        let layout = tick.gizmo_bytes.layout;
        assert_eq!(tick.gizmo_bytes.line_vertices.len(), layout.x as usize * 12);
        assert_eq!(tick.gizmo_bytes.line_attributes.len(), layout.x as usize * 20);
        assert_eq!(tick.gizmo_bytes.handles.len(), layout.y as usize * 12);
        assert_eq!(tick.gizmo_bytes.handle_attributes.len(), layout.y as usize * 8);
    }

    // The ring keeps the last `RING_TICKS` samples in order.
    #[test]
    fn ring_keeps_the_latest() {
        let mut ring = TickRing::default();
        for index in 0..RING_TICKS + 5 {
            ring.push(TickSample {
                start_us: index as i64,
                ..TickSample::default()
            });
        }
        let starts: Vec<i64> = ring.ordered().map(|sample| sample.start_us).collect();
        assert_eq!(starts.len(), RING_TICKS);
        assert_eq!(starts[0], 5);
        assert_eq!(*starts.last().unwrap(), (RING_TICKS + 4) as i64);
        assert_eq!(ring.last_mut().unwrap().start_us, (RING_TICKS + 4) as i64);
    }
}
