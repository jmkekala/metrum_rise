// SPDX-License-Identifier: GPL-2.0-only

//! The preview's road chunks on the bridge (`ROAD-112` round R2-4a): `road_tool.gd` attaches the
//! chunk renderer's layer materials once; each preview the tick hands out keeps its overlays,
//! encoded in the payload stage (`PreviewOverlay`), as the source `road_preview_display.gd`
//! shows by number (`road_tool_show_overlay`) after it checked the resident runs it collapses.
//! The pool (`nodes::sim::road_tool::draw::chunks`) stages and swaps in one call; an accepted
//! commit's chunks are held with its hue and released with it. The display's collapsed runs stay
//! GDScript's until round R2-4b.

use super::*;
use crate::nodes::sim::render::stream::layers::roads::surfaces::LAYER_COUNT;
use crate::nodes::sim::road_tool::draw::chunks::{ChunkOverlays, ChunkState, PreviewOverlay};
use godot::classes::Material;
use std::sync::Arc;

// The pool once attached, and the overlays of the last preview handed out with its number.
#[derive(Default)]
pub(super) struct PreviewChunks {
    pool: Option<ChunkOverlays<GodotBackend>>,
    source: Option<(i64, Arc<PreviewOverlay>)>,
}

#[godot_api(secondary)]
impl SimulationNode {
    /// Attaches the preview's road chunks (once, from `road_tool.gd` when it makes its chunk
    /// renderer): their meshes and instances in `scenario` (an invalid RID: drawn nowhere until
    /// a caller copies the meshes, as tests outside the tree do), each payload layer drawn with
    /// `materials`' entry in payload order (`hydration_materials`, its first nine). Replaces an
    /// earlier attachment (whose objects it frees). False (nothing attached) when fewer than
    /// nine materials are given.
    #[func]
    pub fn road_tool_attach_overlay(
        &mut self,
        scenario: Rid,
        materials: Array<Gd<Material>>,
    ) -> bool {
        if materials.len() < LAYER_COUNT {
            return false;
        }
        let kept: Vec<Gd<Material>> = materials.iter_shared().take(LAYER_COUNT).collect();
        let handles: [u64; LAYER_COUNT] =
            std::array::from_fn(|layer| kept[layer].get_rid().to_u64());
        let chunks = &mut self.road_tool_session.chunks;
        // The earlier attachment's objects go first.
        chunks.pool = None;
        chunks.pool = Some(ChunkOverlays::new(
            GodotBackend::plain(scenario, kept),
            handles,
        ));
        true
    }

    /// Frees the preview's road chunks (the road tool leaving the tree or freed).
    #[func]
    pub fn road_tool_detach_overlay(&mut self) {
        self.road_tool_session.chunks.pool = None;
    }

    /// Shows preview `sequence`'s overlay chunks (`road_preview_display.gd`, once the resident
    /// runs it collapses checked out): staged into the pool's spare meshes and swapped in for
    /// the shown ones in this call. True when shown (or already shown); false, with the shown
    /// chunks unchanged, when `sequence` is not the last preview handed out with chunks or no
    /// pool is attached. Its time goes into the tick's sample (`road_tool_timings`'
    /// `overlay_ms`). `O(the overlay's bytes + chunks)`.
    #[func]
    pub fn road_tool_show_overlay(&mut self, sequence: i64) -> bool {
        let start = Instant::now();
        let chunks = &mut self.road_tool_session.chunks;
        let (Some(pool), Some((shown, overlay))) = (&mut chunks.pool, &chunks.source) else {
            return false;
        };
        if *shown != sequence {
            return false;
        }
        pool.show(sequence, overlay);
        let elapsed = Self::road_tool_tick_elapsed(start);
        self.road_tool_tick_add_overlay(elapsed);
        true
    }

    /// Hides the current preview's overlay chunks.
    #[func]
    pub fn road_tool_clear_overlay(&mut self) {
        if let Some(pool) = &mut self.road_tool_session.chunks.pool {
            pool.clear();
        }
    }

    /// The overlay chunks drawn (tests and probes): one dictionary a chunk, the current
    /// preview's first, with `chunk_x`, `chunk_z`, `mesh` (its RID), `origin` (the chunk's
    /// centre), `surfaces` and `held` (an accepted commit's, held until its chunks install);
    /// empty while none is drawn or no pool is attached.
    #[func]
    pub fn road_tool_overlay_chunks(&self) -> Array<VarDictionary> {
        let mut out = Array::new();
        let Some(pool) = &self.road_tool_session.chunks.pool else {
            return out;
        };
        for ChunkState {
            key,
            mesh,
            origin,
            surfaces,
            held,
        } in pool.state()
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

    /// The overlay chunks staged since the pool was attached (tests: an unchanged stroke
    /// uploads nothing) and the shown preview's number (0: none).
    #[func]
    pub fn road_tool_overlay_counts(&self) -> PackedInt64Array {
        let pool = self.road_tool_session.chunks.pool.as_ref();
        PackedInt64Array::from(&[
            pool.map_or(0, |pool| pool.staged_total() as i64),
            pool.map_or(0, ChunkOverlays::sequence),
        ])
    }
}

impl SimulationNode {
    // The overlays of preview `sequence` as just handed out, the source `road_tool_show_overlay`
    // reads (none: the preview has no chunks).
    pub(super) fn road_tool_set_overlay_source(
        &mut self,
        sequence: i64,
        overlay: Option<Arc<PreviewOverlay>>,
    ) {
        self.road_tool_session.chunks.source = overlay.map(|overlay| (sequence, overlay));
    }

    // An accepted commit's chunks stay drawn with its hue until the hold releases.
    pub(super) fn road_tool_hold_overlay(&mut self) {
        if let Some(pool) = &mut self.road_tool_session.chunks.pool {
            pool.hold();
        }
    }

    // The held commit's chunks go with its hue.
    pub(super) fn road_tool_release_held_overlay(&mut self) {
        if let Some(pool) = &mut self.road_tool_session.chunks.pool {
            pool.release_held();
        }
    }
}
