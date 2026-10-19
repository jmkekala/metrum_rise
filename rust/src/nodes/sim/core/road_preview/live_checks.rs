// SPDX-License-Identifier: GPL-2.0-only

//! Live-state checks of a completed road preview and their per-frame publication.
//!
//! A preview poll reads a few answers from the live city: the stroke corridor's parcel and field
//! clearance and the edit plan's readiness. The hover path never waits for the simulation lock.
//! A poll that takes the lock evaluates the checks itself. A poll that finds it held uses the
//! checks the simulation thread published at the end of its last frame, which are as current as
//! the render snapshot. Evaluation only reads the core, so it cannot change simulation outcomes.

use std::sync::{Mutex, RwLock, TryLockError};

use godot::prelude::Vector3;

use super::{RoadPreviewSnapshot, SimCore};

/// Corridor clearance of a stroke against live parcels and field reservations.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct RoadCorridorClearance {
    /// Parcels the corridor overlaps; any overlap refuses the stroke (`parcel_overlap`).
    pub(crate) overlapping_parcels: usize,
    /// Lowest overlapping parcel id, present when any parcel overlaps.
    pub(crate) first_overlapping_parcel: Option<u64>,
    /// Whether the corridor overlaps a committed field reservation (`field_overlap`).
    pub(crate) field_overlap: bool,
    /// Zoning overlay revision the parcel query read.
    pub(crate) zoning_revision: u64,
    /// Agriculture field revision the field query read.
    pub(crate) field_revision: u64,
}

impl RoadCorridorClearance {
    /// Queries the parcel chunk index and the field reservations along the corridor that the
    /// lanes and sidewalks cover. O(corridor segments × parcels and fields in the touched chunks).
    pub(crate) fn evaluate(
        core: &SimCore,
        points: &[Vector3],
        fwd_lanes: i32,
        bkw_lanes: i32,
    ) -> Self {
        let lanes = fwd_lanes.clamp(0, i32::from(u8::MAX)) + bkw_lanes.clamp(0, i32::from(u8::MAX));
        let half_width = (lanes as f32 * crate::config::LANE_WIDTH).max(2.0) * 0.5
            + crate::config::SIDEWALK_WIDTH;
        let parcels = core
            .zoning
            .parcel_ids_overlapping_road_corridor(points, half_width);
        Self {
            overlapping_parcels: parcels.len(),
            first_overlapping_parcel: parcels.first().copied(),
            field_overlap: core
                .allocator
                .field_clearance
                .overlaps_road_corridor(points, half_width),
            zoning_revision: core.zoning.overlay_revision(),
            field_revision: core.agriculture.visual_revision(),
        }
    }

    /// Whether neither a parcel nor a field refuses the corridor.
    pub(crate) fn is_clear(&self) -> bool {
        self.overlapping_parcels == 0 && !self.field_overlap
    }
}

/// Everything a poll of one completed preview reads from the live city.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct RoadPreviewLiveChecks {
    // Identity of the result these checks belong to; each request id is published once.
    request_id: u64,
    retained_revision: u64,
    /// Present when the compiled validation passed.
    pub(crate) clearance: Option<RoadCorridorClearance>,
    /// Edit-plan readiness (`ready`, `provisional`, `stale`, `invalid`, ...), present when the
    /// result has an edit plan and its corridor is clear.
    pub(crate) plan_state: Option<&'static str>,
    /// Why an `invalid` plan refuses a clear corridor: `field_overlap`, `cell_overlap` or
    /// `road_plan_invalid`.
    pub(crate) plan_invalid_reason: Option<&'static str>,
}

impl RoadPreviewLiveChecks {
    /// Evaluates the checks of `preview` against `core`. Read-only. Costs one corridor query
    /// plus the plan's local checks (water, fields, cells, parcels and terrain sites), none of
    /// them city-wide.
    pub(crate) fn evaluate(preview: &RoadPreviewSnapshot, core: &SimCore) -> Self {
        let clearance = preview.validation.is_valid.then(|| {
            RoadCorridorClearance::evaluate(
                core,
                &preview.prepared_points,
                i32::from(preview.fwd_lanes),
                i32::from(preview.bkw_lanes),
            )
        });
        let plan = preview
            .live_edit_plan()
            .filter(|_| clearance.is_some_and(|c| c.is_clear()));
        let plan_state = plan.map(|plan| {
            if preview.include_terrain {
                plan.status(core)
            } else {
                plan.road_status(core)
            }
        });
        let plan_invalid_reason = plan.filter(|_| plan_state == Some("invalid")).map(|plan| {
            if plan.overlaps_fields(core) {
                "field_overlap"
            } else if plan.overlaps_cell_zoning(core) {
                "cell_overlap"
            } else {
                "road_plan_invalid"
            }
        });
        Self {
            request_id: preview.request_id,
            retained_revision: retained_revision(preview),
            clearance,
            plan_state,
            plan_invalid_reason,
        }
    }

    fn belongs_to(&self, preview: &RoadPreviewSnapshot) -> bool {
        self.request_id == preview.request_id
            && self.retained_revision == retained_revision(preview)
    }
}

fn retained_revision(preview: &RoadPreviewSnapshot) -> u64 {
    preview
        .junction_preview
        .as_ref()
        .map_or(0, |scene| scene.retained_revision)
}

// A result compiled for an older road-surface generation is stale for good: generations only
// advance. Its checks are not published, so the simulation frame stops evaluating them.
fn is_current(preview: &RoadPreviewSnapshot, core: &SimCore) -> bool {
    preview.surface_generation == core.road_tool_surface_generation
}

/// The live checks of the current preview result, written only under the simulation lock.
///
/// The slot holds the latest evaluation: from the end of the simulation frame, or from a poll
/// that took the lock after it. Reading or writing it copies a few words; nobody holding it waits
/// on anything else.
#[derive(Default)]
pub(crate) struct PublishedRoadPreviewChecks {
    latest: Mutex<Option<RoadPreviewLiveChecks>>,
}

impl PublishedRoadPreviewChecks {
    /// Re-evaluates the current result at the end of a simulation frame; the caller holds the core
    /// lock. Without a current result (none yet, compiled for a superseded road surface, or being
    /// replaced by the worker right now) it publishes nothing: O(1), no allocation.
    pub(crate) fn publish_frame(
        &self,
        core: &SimCore,
        result: &RwLock<Option<RoadPreviewSnapshot>>,
    ) {
        // Never wait for the result lock while holding the core: a writer means a newer result
        // is replacing this one, so its checks would be stale anyway.
        let checks = match result.try_read() {
            Ok(result) => result
                .as_ref()
                .filter(|preview| is_current(preview, core))
                .map(|preview| RoadPreviewLiveChecks::evaluate(preview, core)),
            Err(TryLockError::WouldBlock) => None,
            Err(TryLockError::Poisoned(_)) => panic!("road preview result lock poisoned"),
        };
        *self.lock() = checks;
    }

    /// The checks for a poll of `preview`, the result currently published. Never waits on the
    /// core: a free core is locked, evaluated and the evaluation published; a held one returns the
    /// published checks if they belong to `preview`. `None` means this result was not evaluated
    /// since it was published (pending) or it is stale.
    pub(crate) fn poll(
        &self,
        core: &Mutex<SimCore>,
        preview: &RoadPreviewSnapshot,
    ) -> Option<RoadPreviewLiveChecks> {
        match core.try_lock() {
            Ok(core) => {
                let checks = RoadPreviewLiveChecks::evaluate(preview, &core);
                *self.lock() = is_current(preview, &core).then_some(checks);
                Some(checks)
            }
            Err(TryLockError::WouldBlock) => (*self.lock()).filter(|c| c.belongs_to(preview)),
            Err(TryLockError::Poisoned(_)) => {
                panic!("simulation core lock poisoned by a failed authoritative phase")
            }
        }
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, Option<RoadPreviewLiveChecks>> {
        self.latest
            .lock()
            .expect("road preview checks lock poisoned")
    }
}
