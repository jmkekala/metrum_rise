// SPDX-License-Identifier: GPL-3.0-or-later

//! The construction sites' rows, published with the per-frame values (`RENDER-30`, `CODE-32`):
//! the site pad, foundation and scaffold transforms of every building under construction, which
//! `buildings.gd` sets on its nine site multimeshes without taking the core.
//!
//! Which buildings are sites comes from a roster of building handles, not from a scan. Every
//! building under construction has its completion scheduled, or waits in the allocator's
//! `pending_construction` to be scheduled at the next slot; the roster notes each handle the
//! scheduling drains ([`SimCore::schedule_construction_completions`]) and a refresh reads the
//! pending ones beside it. A load or a new world (a moved world epoch) and a clock set directly
//! ([`SimCore::align_events_to_clock`], which reschedules every completion from a scan) refill
//! it with one scan. A refresh resolves the handles and keeps the buildings that are under
//! construction, so a stale handle never adds a wrong site; only a missing one could drop a
//! site, which test builds check against a scan at every refresh.
//!
//! The roster is derived state: never saved and never read by the simulation.
//!
//! Cost: `O(1)` while the building revisions are unmoved. A moved revision costs
//! `O(S log S + P + T)` for `S` sites, `P` pending placements and `T` buildings without a handle
//! yet (normally none), plus 22 rows a site; nothing is allocated unless a buffer's rows changed.
//! A new site costs `O(1)` amortized in the scheduling hook; the roster holds at most `2 S + 64`
//! handles between refreshes. Only a load, a new world or a clock set directly scans the
//! buildings, `O(B)`, beside the scans those paths already make.

use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

use super::buildings::{
    construction_foundation_rows, construction_scaffold_rows, construction_site_pad_rows,
};
use crate::nodes::sim::core::SimCore;
use crate::simulation::buildings::allocator::{Building, BuildingAllocator};
use crate::simulation::core::handles::Handle;
use crate::simulation::zoning::ZoneType;

/// The zones whose sites draw construction visuals, in buffer order. Office and mixed-use sites
/// draw none (`RENDER-30`, kept as before).
const SITE_ZONES: [ZoneType; 3] = [
    ZoneType::Residential,
    ZoneType::Commercial,
    ZoneType::Industrial,
];

/// Row kinds per zone, in buffer order: site pad, foundation, scaffold.
const SITE_KINDS: usize = 3;

/// The number of published row buffers: three zones times three kinds, zone-major.
pub(crate) const SITE_ROW_BUFFERS: usize = SITE_ZONES.len() * SITE_KINDS;

// Generations across every publisher of the process, so two products never share one and a
// renderer's known generation never matches another core's product.
static NEXT_GENERATION: AtomicU64 = AtomicU64::new(1);

// The perf channel's publisher counters (`RENDER-30`): refreshes after a moved revision, their
// time in nanoseconds, and the roster's full passes. Only counted while the channel is on.
static REFRESHES: AtomicU64 = AtomicU64::new(0);
static REFRESH_NS: AtomicU64 = AtomicU64::new(0);
static FULL_PASSES: AtomicU64 = AtomicU64::new(0);

/// One published row buffer: 12 floats a row (a `MultiMesh` 3D transform), in building index
/// order, and the generation at which its rows last changed.
#[derive(Clone)]
pub(crate) struct SiteRowBuffer {
    pub(crate) generation: u64,
    pub(crate) rows: Arc<[f32]>,
}

/// The construction sites' rows: buffer `3 z + k` holds kind `k` (site pad, foundation,
/// scaffold) of zone `z` (residential, commercial, industrial), the bytes the site multimeshes
/// draw. Cloning shares every buffer.
#[derive(Clone)]
pub(crate) struct SiteRows {
    /// The product's generation; moves whenever any buffer's rows change.
    pub(crate) generation: u64,
    pub(crate) buffers: [SiteRowBuffer; SITE_ROW_BUFFERS],
}

impl Default for SiteRows {
    fn default() -> Self {
        Self {
            generation: 0,
            buffers: std::array::from_fn(|_| SiteRowBuffer {
                generation: 0,
                rows: Arc::from(Vec::new()),
            }),
        }
    }
}

// The handles of the buildings that were sites at the last refresh, plus those noted since.
#[derive(Default)]
struct SiteRoster {
    handles: Vec<Handle<Building>>,
    // Handles kept by the last refresh; the noting hook prunes past `2 kept + 64`.
    kept: usize,
    // Whether the next refresh refills the roster from a scan.
    stale: bool,
    // Full passes made, for tests.
    full_passes: u64,
}

/// The construction sites' publication and the roster it is built from; derived, never saved.
#[derive(Default)]
pub(crate) struct SiteRowsPublisher {
    roster: SiteRoster,
    // (world epoch, building reference revision, building visual revision) at the last refresh.
    key: Option<(u64, u64, u64)>,
    published: Arc<SiteRows>,
    // Reused per refresh: the member indices and the nine buffers' rows.
    members: Vec<usize>,
    scratch: [Vec<f32>; SITE_ROW_BUFFERS],
}

impl SiteRowsPublisher {
    /// The latest product.
    pub(crate) fn published(&self) -> &Arc<SiteRows> {
        &self.published
    }

    /// Notes a building handle the completion scheduling drained: a new site. `O(1)`.
    pub(crate) fn note_site(&mut self, building: Handle<Building>) {
        self.roster.handles.push(building);
    }

    /// Keeps the roster within `2 S + 64` handles when no refresh runs (a headless game):
    /// `O(1)` amortized.
    pub(crate) fn bound_roster(&mut self, allocator: &BuildingAllocator) {
        let roster = &mut self.roster;
        if roster.handles.len() <= 2 * roster.kept + 64 {
            return;
        }
        roster.handles.retain(|&handle| {
            allocator
                .building_handles
                .resolve(handle)
                .is_some_and(|idx| allocator.buildings[idx].is_under_construction())
        });
        roster
            .handles
            .sort_unstable_by_key(|handle| handle.to_bits());
        roster.handles.dedup();
        roster.kept = roster.handles.len();
    }

    /// Marks the roster for a refill from a scan at the next refresh: the completions were
    /// rescheduled from the buildings themselves.
    pub(crate) fn mark_roster_stale(&mut self) {
        self.roster.stale = true;
    }

    /// The roster's full passes so far.
    #[cfg(test)]
    pub(crate) fn full_passes(&self) -> u64 {
        self.roster.full_passes
    }

    /// The roster's handle count.
    #[cfg(test)]
    pub(crate) fn roster_len(&self) -> usize {
        self.roster.handles.len()
    }
}

/// The publisher's perf-channel counters: refreshes after a moved revision, their time in
/// microseconds, and the roster's full passes, all since the process started.
pub(crate) fn site_row_work() -> (u64, f64, u64) {
    (
        REFRESHES.load(Ordering::Relaxed),
        REFRESH_NS.load(Ordering::Relaxed) as f64 / 1000.0,
        FULL_PASSES.load(Ordering::Relaxed),
    )
}

impl SimCore {
    /// Brings the published site rows up to the buildings; whether a buffer changed. `O(1)`
    /// while the building revisions are unmoved (see the module docs for a change).
    pub(crate) fn refresh_site_rows(&mut self) -> bool {
        let key = (
            self.world_epoch,
            self.allocator.building_ref_revision(),
            self.allocator.building_visual_revision,
        );
        let previous = self.site_rows.key;
        if previous == Some(key) {
            return false;
        }
        let perf = crate::debug::Channel::Perf.enabled();
        let start = perf.then(std::time::Instant::now);
        self.site_rows.key = Some(key);
        let world = previous.is_none_or(|p| p.0 != key.0);
        let full = world || self.site_rows.roster.stale;
        let publisher = &mut self.site_rows;
        if full {
            scan_members(&self.allocator, &mut publisher.members);
            publisher.roster.full_passes += 1;
            if perf {
                FULL_PASSES.fetch_add(1, Ordering::Relaxed);
            }
        } else {
            roster_members(&self.allocator, &publisher.roster, &mut publisher.members);
        }
        #[cfg(test)]
        {
            let mut scanned = Vec::new();
            scan_members(&self.allocator, &mut scanned);
            assert_eq!(
                publisher.members, scanned,
                "the site roster lists every building under construction"
            );
        }
        // The roster keeps the members with a handle; the tail without one is read again.
        let roster = &mut publisher.roster;
        roster.handles.clear();
        roster.handles.extend(
            publisher
                .members
                .iter()
                .map(|&idx| self.allocator.building_handles.handle(idx))
                .filter(|handle| *handle != Handle::NONE),
        );
        roster.kept = roster.handles.len();
        roster.stale = false;

        let cell_m = self.config.zone_cell_m;
        for buffer in &mut publisher.scratch {
            buffer.clear();
        }
        for &idx in &publisher.members {
            let building = &self.allocator.buildings[idx];
            let Some(zone) = SITE_ZONES.iter().position(|&z| z == building.zone_type) else {
                continue;
            };
            let [pads, foundations, scaffolds] =
                publisher.scratch[zone * SITE_KINDS..(zone + 1) * SITE_KINDS].as_mut()
            else {
                unreachable!("three kinds a zone")
            };
            construction_site_pad_rows(pads, cell_m, building);
            construction_foundation_rows(foundations, cell_m, building);
            construction_scaffold_rows(scaffolds, cell_m, building);
        }

        let published = &publisher.published;
        let changed: [bool; SITE_ROW_BUFFERS] = std::array::from_fn(|i| {
            world || !same_bits(&publisher.scratch[i], &published.buffers[i].rows)
        });
        let any = changed.contains(&true);
        if any {
            let generation = NEXT_GENERATION.fetch_add(1, Ordering::Relaxed);
            let buffers = std::array::from_fn(|i| {
                if changed[i] {
                    SiteRowBuffer {
                        generation,
                        rows: Arc::from(publisher.scratch[i].as_slice()),
                    }
                } else {
                    published.buffers[i].clone()
                }
            });
            publisher.published = Arc::new(SiteRows {
                generation,
                buffers,
            });
        }
        if let Some(start) = start {
            REFRESHES.fetch_add(1, Ordering::Relaxed);
            REFRESH_NS.fetch_add(start.elapsed().as_nanos() as u64, Ordering::Relaxed);
        }
        any
    }
}

// Every building under construction, in index order: `O(B)`.
fn scan_members(allocator: &BuildingAllocator, members: &mut Vec<usize>) {
    members.clear();
    members.extend(
        allocator
            .buildings
            .iter()
            .enumerate()
            .filter(|(_, building)| building.is_under_construction())
            .map(|(idx, _)| idx),
    );
}

// The roster's, the pending placements' and the handle-less tail's buildings that are under
// construction, in index order: `O(S log S + P + T)`.
fn roster_members(allocator: &BuildingAllocator, roster: &SiteRoster, members: &mut Vec<usize>) {
    members.clear();
    let handles = &allocator.building_handles;
    let pending = allocator
        .pending_construction
        .iter()
        .map(|&(handle, _)| handle);
    members.extend(
        roster
            .handles
            .iter()
            .copied()
            .chain(pending)
            .filter_map(|handle| handles.resolve(handle)),
    );
    members.extend(handles.registered().min(allocator.buildings.len())..allocator.buildings.len());
    members.retain(|&idx| allocator.buildings[idx].is_under_construction());
    members.sort_unstable();
    members.dedup();
}

fn same_bits(a: &[f32], b: &[f32]) -> bool {
    a.len() == b.len() && a.iter().zip(b).all(|(x, y)| x.to_bits() == y.to_bits())
}

/// Asserts that the published site rows equal the retired scans' rows bit for bit, buffer by
/// buffer, and that the office and mixed-use zones (ids 4 and 5) still draw none.
#[cfg(test)]
pub(crate) fn assert_site_rows_match_scans(core: &SimCore, stage: &str) {
    let rows = &core.frame_queries().site_rows;
    for (zone, zone_id) in (1..=3_u8).enumerate() {
        let oracle = [
            core.get_construction_site_transforms_internal(zone_id),
            core.get_construction_foundation_transforms_internal(zone_id),
            core.get_construction_scaffold_transforms_internal(zone_id),
        ];
        for (kind, expected) in oracle.iter().enumerate() {
            let published = &rows.buffers[zone * SITE_KINDS + kind].rows;
            assert!(
                same_bits(published, expected),
                "{stage}: zone {zone_id} kind {kind}: {} published floats, {} scanned",
                published.len(),
                expected.len()
            );
        }
    }
    for zone_id in [4, 5] {
        assert!(
            core.get_construction_site_transforms_internal(zone_id)
                .is_empty()
        );
        assert!(
            core.get_construction_scaffold_transforms_internal(zone_id)
                .is_empty()
        );
    }
}
