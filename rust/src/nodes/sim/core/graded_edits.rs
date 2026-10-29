// SPDX-License-Identifier: GPL-2.0-only

//! The graded terrain's edit journal (`TERRAIN-07-D` item 5, `docs/terrain.md` "Preview,
//! commit, undo and load"): what each published edit changed for the graded tiles, recorded
//! where the core publishes it, with the `O(1)` snapshots and the copied sites its dirty tile
//! selection reads, so the payload workers select the dirty tiles off the simulation lock with
//! exactly the edit's inputs and Godot rebakes only the dirty resident tiles. A road commit's
//! or undo's install carries the tiles it staged (`TERRAIN-07-E`), which Godot installs in
//! place of rebaking them. Recording starts with Godot's first residency poll: a core no graded
//! renderer reads keeps nothing.

use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

use super::state::SimCore;
use crate::simulation::buildings::allocator::{SiteChange, SiteSnapshot};
use crate::simulation::network::roadbed::{RoadbedSystem, TerrainChanges};
use crate::simulation::terrain::TerrainSystem;
use crate::simulation::terrain::graded::{
    ALL_LEVELS, Level, SelectScratch, SiteBuffer, SiteSource, TileKey, TileStage,
    dirty_tiles_for_base_area, dirty_tiles_for_road_changes, dirty_tiles_for_site_changes,
    selection_site_reach_m,
};

/// Edits kept between two polls; one more collapses them into [`GradedEditKind::All`], so a
/// renderer that stops polling holds a bounded journal.
const MAX_PENDING_EDITS: usize = 64;

/// Every level a resident tile can take, in Godot's code order: an edit's selection runs at
/// each.
pub(crate) const EDIT_LEVELS: [Level; 6] = ALL_LEVELS;

/// The tiles a road commit or undo staged for its install (`TERRAIN-07-E`), each at its own
/// level: bitwise the bakes of the install's inputs, which Godot installs with the edit in place
/// of rebaking them.
#[derive(Clone)]
pub(crate) struct GradedInstallTiles {
    pub(crate) stage: Arc<TileStage>,
    /// The preview epoch whose shown tiles the commit adopted: Godot releases that epoch's
    /// preview tiles as it installs these. `None` for an undo or a commit without a preview.
    pub(crate) epoch: Option<u64>,
    /// Per tile of `stage`, whether it is the very payload the preview showed (adopted from
    /// it): Godot then takes over the preview's slot instead of uploading it again. Empty for
    /// an undo or a commit without a preview.
    pub(crate) adopted: Vec<bool>,
}

/// What one published edit changed for the graded tiles.
// At most `MAX_PENDING_EDITS` records live between two polls; boxing the roadbed snapshots
// would only add allocations.
#[allow(clippy::large_enum_variant)]
pub(crate) enum GradedEditKind {
    /// A roadbed install (a commit or an undo): its terrain changes, the roadbed when the
    /// previous install was recorded and after this one, and the sites within the selection's
    /// reach (`None` while the site index is rebuilt: the selection then reads no site).
    Roads {
        before: RoadbedSystem,
        after: RoadbedSystem,
        changes: TerrainChanges,
        sites: Option<SiteSnapshot>,
    },
    /// Published site changes, the roads, and the sites after the changes within reach.
    Sites {
        roadbed: RoadbedSystem,
        changes: Vec<SiteChange>,
        sites: Option<SiteSnapshot>,
    },
    /// Base heightmap samples written inside this world rectangle.
    Base([f64; 4]),
    /// Every tile: a world-scale reset, or a journal past [`MAX_PENDING_EDITS`].
    All,
}

/// One recorded edit: its serial (increasing; Godot's tile tokens), the heightmap after it, and
/// what it changed.
pub(crate) struct GradedEdit {
    pub(crate) serial: u64,
    pub(crate) terrain: TerrainSystem,
    pub(crate) kind: GradedEditKind,
    pub(crate) staged: Option<GradedInstallTiles>,
}

/// The edits recorded since Godot last took them.
#[derive(Default)]
pub(crate) struct GradedEditJournal {
    enabled: bool,
    serial: u64,
    edits: Vec<GradedEdit>,
    // The roadbed when the last install was recorded: the terrain changes a road edit takes
    // accumulate from there, an undo's restore included.
    roadbed: Option<RoadbedSystem>,
    // The serial of the road install recorded last, until its staged tiles attach.
    awaiting: Option<u64>,
    // `serial`, readable without the core lock: the road tool holds an install's replaced road
    // chunks until the renderer has installed the edits recorded up to it (T61 (2)).
    latest: Arc<AtomicU64>,
}

// The world rectangles a selection's site snapshot covers: each change's bounds grown by the
// widest site reach of the levels whose selections read the snapshot (`GL0`-`GL2`; `GL3`'s band
// reaches farther and reads no site, see `graded_tiles_api`).
fn site_windows(bounds: impl Iterator<Item = [f64; 4]>, cell_m: f64) -> Vec<[f64; 4]> {
    let reach = selection_site_reach_m(Level::Gl2, cell_m);
    bounds
        .map(|[a, b, c, d]| [a - reach, b - reach, c + reach, d + reach])
        .collect()
}

impl SimCore {
    /// Returns the edits recorded since the last call and keeps recording (Godot's residency
    /// poll; the first call starts the journal). `O(edits)`.
    pub(crate) fn take_graded_edits(&mut self) -> Vec<GradedEdit> {
        let journal = &mut self.graded_edits;
        if !journal.enabled {
            journal.enabled = true;
            journal.roadbed = Some(self.transit_network.roadbed.clone());
        }
        std::mem::take(&mut journal.edits)
    }

    /// The journal's latest serial, shared: read without the core lock (`T61 (2)`, the road
    /// tool's undo hold). Release-stored after the edit is recorded, so a reader that saw a
    /// publication made after the edit sees its serial.
    pub(crate) fn graded_edit_serial(&self) -> Arc<AtomicU64> {
        Arc::clone(&self.graded_edits.latest)
    }

    fn push_graded_edit(&mut self, kind: GradedEditKind) {
        let journal = &mut self.graded_edits;
        journal.serial += 1;
        let kind = if journal.edits.len() >= MAX_PENDING_EDITS {
            journal.edits.clear();
            GradedEditKind::All
        } else {
            kind
        };
        journal.edits.push(GradedEdit {
            serial: journal.serial,
            terrain: self.heightmap.clone(),
            kind,
            staged: None,
        });
        journal.latest.store(journal.serial, Ordering::Release);
    }

    /// Records a roadbed install with the terrain changes it took (`O(1)` roadbed snapshots,
    /// the sites within reach of the changed pieces copied). Nothing while the journal is off.
    pub(crate) fn record_graded_road_edit(&mut self, changes: &TerrainChanges) {
        self.graded_edits.awaiting = None;
        let started = std::time::Instant::now();
        self.terrain_query_index.update_roads(
            &self.transit_network.roadbed,
            &self.allocator,
            changes,
        );
        self.terrain_query_index_update_us = started.elapsed().as_micros() as u64;
        if !self.graded_edits.enabled {
            return;
        }
        let after = self.transit_network.roadbed.clone();
        let before = self
            .graded_edits
            .roadbed
            .replace(after.clone())
            .unwrap_or_else(|| after.clone());
        if changes.pieces.is_empty() && changes.segments.is_empty() {
            return;
        }
        let cell_m = f64::from(self.heightmap.cell_size_m());
        // A changed segment lies in its old or new piece, whose bounds `pieces` holds.
        let sites = self
            .allocator
            .graded_site_snapshot(site_windows(changes.pieces.iter().copied(), cell_m));
        self.push_graded_edit(GradedEditKind::Roads {
            before,
            after,
            changes: changes.clone(),
            sites,
        });
        let journal = &mut self.graded_edits;
        journal.awaiting = Some(journal.serial);
    }

    /// Attaches the tiles a commit or undo staged to the journal's record of its install, the
    /// last one recorded. Nothing when the journal is off, the install changed nothing or the
    /// record was collapsed into [`GradedEditKind::All`].
    pub(crate) fn attach_graded_install_tiles(
        &mut self,
        stage: Arc<TileStage>,
        epoch: Option<u64>,
        adopted: Vec<bool>,
    ) {
        let journal = &mut self.graded_edits;
        let Some(serial) = journal.awaiting.take() else {
            return;
        };
        let edit = journal
            .edits
            .last_mut()
            .filter(|edit| edit.serial == serial);
        if let Some(edit) = edit.filter(|edit| matches!(edit.kind, GradedEditKind::Roads { .. })) {
            edit.staged = Some(GradedInstallTiles {
                stage,
                epoch,
                adopted,
            });
        }
    }

    /// Records published site changes. Nothing while the journal is off or without changes.
    pub(crate) fn record_graded_site_edit(&mut self, changes: &[SiteChange]) {
        let started = std::time::Instant::now();
        self.terrain_query_index.update_sites(
            &self.transit_network.roadbed,
            &self.allocator,
            changes,
        );
        self.terrain_query_index_update_us = started.elapsed().as_micros() as u64;
        if !self.graded_edits.enabled || changes.is_empty() {
            return;
        }
        let cell_m = f64::from(self.heightmap.cell_size_m());
        let bounds = changes.iter().map(|change| change.bounds.map(f64::from));
        let sites = self
            .allocator
            .graded_site_snapshot(site_windows(bounds, cell_m));
        self.push_graded_edit(GradedEditKind::Sites {
            roadbed: self.transit_network.roadbed.clone(),
            changes: changes.to_vec(),
            sites,
        });
    }

    /// Records base samples written inside `rect` (`[min_x, min_z, max_x, max_z]`).
    pub(crate) fn record_graded_base_edit(&mut self, rect: [f64; 4]) {
        let started = std::time::Instant::now();
        let mut tiles = Vec::new();
        dirty_tiles_for_base_area(&self.heightmap, rect, Level::Gl0, &mut tiles);
        self.terrain_query_index.update_base_tiles(
            tiles,
            &self.transit_network.roadbed,
            &self.allocator,
        );
        self.terrain_query_index_update_us = started.elapsed().as_micros() as u64;
        if self.graded_edits.enabled {
            self.push_graded_edit(GradedEditKind::Base(rect));
        }
    }

    /// Records a world-scale reset: every tile is dirty.
    pub(crate) fn record_graded_reset(&mut self) {
        let started = std::time::Instant::now();
        self.terrain_query_index
            .rebuild_all(&self.transit_network.roadbed, &self.allocator);
        self.terrain_query_index_update_us = started.elapsed().as_micros() as u64;
        if self.graded_edits.enabled {
            self.graded_edits.roadbed = Some(self.transit_network.roadbed.clone());
            self.push_graded_edit(GradedEditKind::All);
        }
    }
}

/// One edit's dirty tiles per level of [`EDIT_LEVELS`]; `all` marks every tile (a reset).
pub(crate) struct GradedEditSelection {
    pub(crate) serial: u64,
    pub(crate) all: bool,
    pub(crate) levels: Vec<(Level, Vec<TileKey>)>,
    /// The tiles the install staged, which Godot installs instead of rebaking.
    pub(crate) staged: Option<GradedInstallTiles>,
}

// The sites of a selection that may not read any: `GL3`, whose band reaches past the edit's
// site snapshot, or an edit recorded while the site index was rebuilt. An unready source drops
// the selection's floor to `-inf`, so it selects a superset of the exact selection's tiles.
struct UnreadSites;

impl SiteSource for UnreadSites {
    fn ready(&self) -> bool {
        false
    }

    // Never called: the selections read only a ready source.
    fn sites_in_bounds(&self, _: [f64; 4], _: &mut SiteBuffer) {}
}

// The site snapshot a selection at `level` reads: none at `GL3`.
fn readable(sites: &Option<SiteSnapshot>, level: Level) -> Option<&SiteSnapshot> {
    sites
        .as_ref()
        .filter(|_| !matches!(level, Level::Gl3 { .. }))
}

impl GradedEdit {
    /// The edit's dirty tiles at every level of [`EDIT_LEVELS`], from its own snapshots: the
    /// exact selections, a superset at `GL3` and while the site index was rebuilt. `O(the
    /// edit's selections)`, run off the simulation lock.
    pub(crate) fn select(&self, scratch: &mut SelectScratch) -> GradedEditSelection {
        let mut selection = GradedEditSelection {
            serial: self.serial,
            all: matches!(self.kind, GradedEditKind::All),
            levels: Vec::with_capacity(EDIT_LEVELS.len()),
            staged: self.staged.clone(),
        };
        if selection.all {
            return selection;
        }
        let terrain = &self.terrain;
        for level in EDIT_LEVELS {
            let mut tiles = Vec::new();
            match &self.kind {
                GradedEditKind::Roads {
                    before,
                    after,
                    changes,
                    sites,
                } => match readable(sites, level) {
                    Some(sites) => dirty_tiles_for_road_changes(
                        terrain, before, after, sites, changes, level, scratch, &mut tiles,
                    ),
                    None => dirty_tiles_for_road_changes(
                        terrain,
                        before,
                        after,
                        &UnreadSites,
                        changes,
                        level,
                        scratch,
                        &mut tiles,
                    ),
                },
                GradedEditKind::Sites {
                    roadbed,
                    changes,
                    sites,
                } => match readable(sites, level) {
                    Some(sites) => dirty_tiles_for_site_changes(
                        terrain, roadbed, sites, changes, level, scratch, &mut tiles,
                    ),
                    None => dirty_tiles_for_site_changes(
                        terrain,
                        roadbed,
                        &UnreadSites,
                        changes,
                        level,
                        scratch,
                        &mut tiles,
                    ),
                },
                GradedEditKind::Base(rect) => {
                    dirty_tiles_for_base_area(terrain, *rect, level, &mut tiles)
                }
                // Returned above.
                GradedEditKind::All => {}
            }
            selection.levels.push((level, tiles));
        }
        selection
    }
}
