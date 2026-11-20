// SPDX-License-Identifier: GPL-3.0-or-later

//! The preview's road chunks on the bridge (`ROAD-112` rounds R2-4a, R2-4b): `road_tool.gd`
//! attaches the chunk renderer's layer materials and its `RoadDrawHolds` once; each preview the
//! tick hands out keeps its overlays and resident runs, encoded in the payload stage
//! (`PreviewOverlay`), as the source the display shows by number (`road_tool_show_chunks`).
//! The chunk renderer reports its resident chunks (`road_tool_resident_*`), so the display
//! (`nodes::sim::road_tool::draw::chunks::display`) checks, collapses and restores runs without
//! walking the scene. An accepted commit's display is held with its hue until `RoadDrawHolds`
//! releases it, on the install of its chunks (`road_tool_chunks_changed`).

use super::super::super::road_holds::RoadDrawHolds;
use super::*;
use crate::nodes::sim::render::stream::layers::roads::surfaces::LAYER_COUNT;
use crate::nodes::sim::road_tool::draw::chunks::display::{
    ChunkDisplay, CollapsedState, POSITION_BYTES, Resident, ResidentChunk, ResidentLayer,
};
use crate::nodes::sim::road_tool::draw::chunks::{ChunkOverlays, ChunkState, PreviewOverlay};
use godot::classes::rendering_server::ArrayFormat;
use godot::classes::{Material, RenderingServer};
use godot::obj::EngineBitfield;
use std::sync::Arc;

// The display once attached with the chunk renderer's holds, the resident chunks as the
// renderer reported them, and the overlays of the last valid preview handed out with its number.
#[derive(Default)]
pub(super) struct PreviewChunks {
    display: Option<ChunkDisplay<GodotBackend>>,
    holds: Option<Gd<RoadDrawHolds>>,
    resident: Resident,
    source: Option<(i64, Arc<PreviewOverlay>)>,
}

#[godot_api(secondary)]
impl SimulationNode {
    /// Attaches the preview's road chunks (once, from `road_tool.gd` when it makes its chunk
    /// renderer): their meshes and instances in `scenario` (an invalid RID: drawn nowhere until
    /// a caller copies the meshes, as tests outside the tree do), each payload layer drawn with
    /// `materials`' entry in payload order (`hydration_materials`, its first nine), and the chunk
    /// renderer's `holds`, whose rule releases an accepted commit's display. Replaces an earlier
    /// attachment (whose objects it frees). False (nothing attached) when fewer than nine
    /// materials are given.
    #[func]
    pub fn road_tool_attach_overlay(
        &mut self,
        scenario: Rid,
        materials: Array<Gd<Material>>,
        holds: Gd<RoadDrawHolds>,
    ) -> bool {
        if materials.len() < LAYER_COUNT {
            return false;
        }
        let kept: Vec<Gd<Material>> = materials.iter_shared().take(LAYER_COUNT).collect();
        let handles: [u64; LAYER_COUNT] =
            std::array::from_fn(|layer| kept[layer].get_rid().to_u64());
        let chunks = &mut self.road_tool_session.chunks;
        // The earlier attachment's objects go first.
        chunks.display = None;
        let pool = ChunkOverlays::new(GodotBackend::plain(scenario, kept), handles);
        chunks.display = Some(ChunkDisplay::new(pool));
        chunks.holds = Some(holds);
        true
    }

    /// Frees the preview's road chunks (the road tool leaving the tree or freed).
    #[func]
    pub fn road_tool_detach_overlay(&mut self) {
        let chunks = &mut self.road_tool_session.chunks;
        chunks.display = None;
        chunks.holds = None;
    }

    /// What the frame shows of the standing preview (`ROAD-112` round R2-10: the decision is
    /// Rust's, `road_tool.gd` executes it; after a tick's preview flag, a click's press and a
    /// refused commit's answer): its `sequence` when the display shows it whole, in this order:
    /// its graded tiles (the graded renderer reads `road_tool_preview`'s graded keys), its road
    /// chunks (`road_tool_show_chunks(sequence)`) and its lots' outlines
    /// (`road_tool_preview_lots`); minus its `sequence` when only its graded tiles show (a click
    /// that sent the stroke: its commit adopted them, and the preview's road chunks give way to
    /// the commit's at their install); 0 when the display already shows it or none stands. A
    /// whole show becomes the shown one (`road_tool_shown_sequence`). `O(1)`, no allocation.
    #[func]
    pub fn road_tool_show_preview(&mut self) -> i64 {
        let bridge = &mut self.road_tool_session;
        let Some(standing) = &bridge.preview else {
            return 0;
        };
        if standing.sequence == bridge.tick.shown {
            return 0;
        }
        if bridge.commit.request_id() != 0 {
            return -standing.sequence;
        }
        bridge.tick.shown = standing.sequence;
        standing.sequence
    }

    /// The preview number the display shows whole (`road_tool_show_preview`), 0 for none: the
    /// preview metrics, probes and benchmarks.
    #[func]
    pub fn road_tool_shown_sequence(&self) -> i64 {
        self.road_tool_session.tick.shown
    }

    /// The lots a valid standing preview removes (`ZONE-09`: four corners each, as
    /// `demolish_lots`), empty for an invalid one or none; the frame outlines them when
    /// `road_tool_show_preview` shows the preview whole. Shared, not copied.
    #[func]
    pub fn road_tool_preview_lots(&self) -> PackedVector3Array {
        (self.road_tool_session.preview.as_ref())
            .filter(|standing| standing.outcome.valid)
            .map(|standing| standing.lots.clone())
            .unwrap_or_default()
    }

    /// Shows preview `sequence`'s road chunks (`road_tool.gd`, the frame that fetched it): its
    /// overlays staged into the pool's spare meshes and swapped in, and the resident runs they
    /// replace collapsed (the ones no longer replaced restored), in this call; a preview of a
    /// road mesh generation not installed yet, or while an install is held, waits for the
    /// install. True when shown or waiting; false, with the display cleared, when `sequence`
    /// is not the last valid preview handed out with chunks, its runs do not fit the resident
    /// chunks, or no display is attached. The preview's join marks (`ROAD-118` round JS5b: its
    /// junction outline, or a refusal's overlapped stretch) are drawn with it, only when they
    /// changed. Its time goes into the tick's sample (`road_tool_timings`' `overlay_ms`, the
    /// marks' part also as `marks_ms`). `O(the overlay's bytes + chunks * log resident + runs *
    /// log runs + hidden vertices + mark points)`.
    #[func]
    pub fn road_tool_show_chunks(&mut self, sequence: i64) -> bool {
        let start = Instant::now();
        self.road_tool_draw_marks(sequence);
        let marks = Self::road_tool_tick_elapsed(start);
        self.road_tool_note_holding();
        let chunks = &mut self.road_tool_session.chunks;
        let Some(display) = &mut chunks.display else {
            return false;
        };
        let shown = match &chunks.source {
            Some((source, overlay)) if *source == sequence => {
                display.show(sequence, overlay, &chunks.resident)
            }
            _ => false,
        };
        if !shown {
            display.clear();
        }
        let elapsed = Self::road_tool_tick_elapsed(start);
        self.road_tool_tick_add_overlay(elapsed, marks);
        shown
    }

    /// Restores the current preview's collapsed runs and hides its overlays and join marks
    /// (the preview went: the display shows none, `road_tool_shown_sequence` 0); a held
    /// commit's display stays.
    #[func]
    pub fn road_tool_clear_chunks(&mut self) {
        let bridge = &mut self.road_tool_session;
        bridge.tick.shown = 0;
        if let Some(display) = &mut bridge.chunks.display {
            display.clear();
        }
        if let Some(draw) = &mut bridge.draw {
            draw.clear_marks();
        }
    }

    /// The chunk renderer installed resident chunks, or swapped in the ones it held
    /// (`road_tool.gd`'s `road_chunks_installed`, `road_chunks_released`, after the commit's
    /// answer was polled): a held commit's display goes when `RoadDrawHolds` releases it, then
    /// the preview that waited for the install shows, or the shown one again when the install
    /// replaced chunks it indexes.
    #[func]
    pub fn road_tool_chunks_changed(&mut self) {
        self.road_tool_note_holding();
        self.road_tool_release_due();
        let chunks = &mut self.road_tool_session.chunks;
        if let Some(display) = &mut chunks.display {
            display.resident_changed(&chunks.resident);
        }
    }

    /// Drops the whole display (another world): the current preview cleared, a held commit's
    /// display and hue released and the holds' preview hold dropped.
    #[func]
    pub fn road_tool_drop_chunks(&mut self) {
        if let Some(draw) = &mut self.road_tool_session.draw {
            draw.clear_marks();
        }
        let chunks = &mut self.road_tool_session.chunks;
        if let Some(display) = &mut chunks.display {
            display.clear();
        }
        if let Some(holds) = &mut chunks.holds {
            holds.bind_mut().drop_preview();
        }
        self.road_tool_release_held();
    }

    /// The chunk renderer installed resident chunk `key` (`road_chunk_renderer.gd`, round
    /// R2-4b) with mesh `mesh` and its layer table: each payload layer's surface (-1 when
    /// empty), vertices and format, as the mesh's `road_layer_*` metas hold them. A layer whose
    /// vertex stream is not positions only, or a short table, takes no run. `O(layers +
    /// log resident)`.
    #[func]
    pub fn road_tool_resident_chunk(
        &mut self,
        key: Vector2i,
        mesh: Rid,
        surfaces: PackedInt32Array,
        counts: PackedInt32Array,
        formats: PackedInt64Array,
    ) {
        let server = RenderingServer::singleton();
        let layers = std::array::from_fn(|layer| {
            let (Some(surface), Some(count), Some(format)) =
                (surfaces.get(layer), counts.get(layer), formats.get(layer))
            else {
                return ResidentLayer::default();
            };
            if surface < 0 || count <= 0 {
                return ResidentLayer::default();
            }
            let stride = server
                .mesh_surface_get_format_vertex_stride(ArrayFormat::from_ord(format as u64), count);
            ResidentLayer {
                surface,
                vertices: count as u32,
                positions_only: stride == POSITION_BYTES,
            }
        });
        let chunk = ResidentChunk {
            mesh: mesh.to_u64(),
            layers,
        };
        (self.road_tool_session.chunks.resident).install((key.x, key.y), chunk);
    }

    /// Resident chunk `key` left the chunk renderer's index (its mesh may still draw while a
    /// held install keeps it, until `road_tool_resident_freed`). `O(log resident)`.
    #[func]
    pub fn road_tool_resident_retired(&mut self, key: Vector2i) {
        (self.road_tool_session.chunks.resident).retire((key.x, key.y));
    }

    /// The chunk renderer freed a chunk whose mesh is `mesh`: runs the display collapsed on it
    /// are forgotten, never written again. `O(collapsed chunks)`.
    #[func]
    pub fn road_tool_resident_freed(&mut self, mesh: Rid) {
        if let Some(display) = &mut self.road_tool_session.chunks.display {
            display.forget_mesh(mesh.to_u64());
        }
    }

    /// The chunk renderer's installed road mesh generation (-1 while it needs a full snapshot)
    /// and grid: the chunks' span and the corner of chunk (0, 0), metres.
    #[func]
    pub fn road_tool_resident_grid(
        &mut self,
        generation: i64,
        span_m: f64,
        origin_x_m: f64,
        origin_z_m: f64,
    ) {
        (self.road_tool_session.chunks.resident).set_grid(
            generation,
            span_m as f32,
            (origin_x_m as f32, origin_z_m as f32),
        );
    }

    /// The overlay chunks drawn (tests and probes): one dictionary a chunk, the current
    /// preview's first, with `chunk_x`, `chunk_z`, `mesh` (its RID), `origin` (the chunk's
    /// centre), `surfaces` and `held` (an accepted commit's, held until its chunks install);
    /// empty while none is drawn or no display is attached.
    #[func]
    pub fn road_tool_overlay_chunks(&self) -> Array<VarDictionary> {
        let mut out = Array::new();
        let Some(display) = &self.road_tool_session.chunks.display else {
            return out;
        };
        for ChunkState {
            key,
            mesh,
            origin,
            surfaces,
            held,
        } in display.pool().state()
        {
            out.push(&vdict! {
                "chunk_x": key.0,
                "chunk_z": key.1,
                "mesh": Rid::new(mesh),
                "origin": Vector3::from_array(origin),
                "surfaces": i64::from(surfaces),
                "held": held,
            });
        }
        out
    }

    /// The overlay chunks staged since the display was attached (tests: an unchanged stroke
    /// uploads nothing) and the shown preview's number (0: none).
    #[func]
    pub fn road_tool_overlay_counts(&self) -> PackedInt64Array {
        let display = self.road_tool_session.chunks.display.as_ref();
        PackedInt64Array::from(&[
            display.map_or(0, |display| display.pool().staged_total() as i64),
            display.map_or(0, ChunkDisplay::sequence),
        ])
    }

    /// The display's state (tests and probes): `sequence` (the shown preview, 0: none),
    /// `pending` (the preview waiting for an install, 0: none), `held` (an accepted commit's
    /// display is held) and `collapsed`, one dictionary a chunk with collapsed runs, the current
    /// preview's first: `chunk_x`, `chunk_z`, `mesh` (the resident mesh's RID), `runs` and
    /// `held`. `{}` while no display is attached.
    #[func]
    pub fn road_tool_chunk_display(&self) -> VarDictionary {
        let Some(display) = &self.road_tool_session.chunks.display else {
            return VarDictionary::new();
        };
        let mut collapsed = Array::<VarDictionary>::new();
        for CollapsedState {
            key,
            mesh,
            runs,
            held,
        } in display.collapsed()
        {
            collapsed.push(&vdict! {
                "chunk_x": key.0,
                "chunk_z": key.1,
                "mesh": Rid::new(mesh),
                "runs": runs as i64,
                "held": held,
            });
        }
        vdict! {
            "sequence": display.sequence(),
            "pending": display.pending(),
            "held": display.held(),
            "collapsed": collapsed,
        }
    }
}

impl SimulationNode {
    // The join marks of preview `sequence` (`ROAD-118` round JS5b): the standing preview's when
    // it is the last one handed out, else none; drawn only when they changed. `O(mark points)`.
    fn road_tool_draw_marks(&mut self, sequence: i64) {
        let bridge = &mut self.road_tool_session;
        let Some(draw) = &mut bridge.draw else {
            return;
        };
        match &bridge.preview {
            Some(standing) if sequence == bridge.tick.sequence => {
                draw.draw_marks(&standing.outcome.marks);
            }
            _ => draw.clear_marks(),
        }
    }

    // The overlays of preview `sequence` as just handed out, the source `road_tool_show_chunks`
    // reads (none: the preview is not valid or has no chunks).
    pub(super) fn road_tool_set_overlay_source(
        &mut self,
        sequence: i64,
        overlay: Option<Arc<PreviewOverlay>>,
    ) {
        self.road_tool_session.chunks.source = overlay.map(|overlay| (sequence, overlay));
    }

    // An accepted commit's display stays drawn with its hue until the chunks of `generation`
    // install (`RoadDrawHolds::hold_preview`); released at once when they already did.
    pub(super) fn road_tool_hold_chunks(&mut self, generation: i64) {
        let chunks = &mut self.road_tool_session.chunks;
        if let Some(display) = &mut chunks.display {
            display.hold();
        }
        if let Some(holds) = &mut chunks.holds {
            holds.bind_mut().hold_preview(generation);
        }
        self.road_tool_note_holding();
        self.road_tool_release_due();
    }

    // The held commit's display and hue go when the holds release its preview: the installed
    // chunks are of its generation or later and no install is held.
    fn road_tool_release_due(&mut self) {
        let chunks = &mut self.road_tool_session.chunks;
        let (Some(display), Some(holds)) = (&chunks.display, &mut chunks.holds) else {
            return;
        };
        if display.held()
            && holds
                .bind_mut()
                .release_preview(chunks.resident.generation())
        {
            self.road_tool_release_held();
        }
    }

    // The held commit's hue and display go (released, or another world).
    fn road_tool_release_held(&mut self) {
        let bridge = &mut self.road_tool_session;
        if let Some(draw) = &mut bridge.draw {
            draw.release_held_hue();
        }
        if let Some(display) = &mut bridge.chunks.display {
            display.release_held();
        }
    }

    // Whether the chunk renderer holds an install, as the display reads it.
    fn road_tool_note_holding(&mut self) {
        let chunks = &mut self.road_tool_session.chunks;
        chunks.resident.holding =
            (chunks.holds.as_ref()).is_some_and(|holds| holds.bind().is_holding());
    }
}
