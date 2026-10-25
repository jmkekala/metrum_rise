// SPDX-License-Identifier: GPL-2.0-only

//! The road tool's terrain inputs (`ROAD-68-E`): per render patch, what the refined source of a
//! road-touched patch reads from the core besides the heightmap (the patch's refined cache
//! entry, its assembly ledger, its site ownership and the building sites within the widest
//! road query margin), in a persistent map the road tool's view shares in `O(1)`.
//!
//! The core notes every patch whose inputs may have changed (generation bumps, ledger stamps
//! and acknowledgements, cache inserts and removals, ownership refreshes, site changes) and
//! rebuilds only those patches when it publishes; a world reset rebuilds every patch. Only a
//! patch noted for a site change queries the sites again: the other notes change the entry or
//! the ledger, and the patch keeps its sites and site ownership. A missed
//! note cannot change a commit: the commit compares its own sources with the preview's and
//! builds its terrain itself on any difference. Docs: `docs/roads.md` "Preview, commit, undo
//! and load".

use std::collections::BTreeSet;
use std::sync::Arc;

use super::state::SimCore;
use super::terrain_payloads::{
    CachedRefinedTerrainPatch, ROAD_LOCKED_TERRAIN_RENDER_STEP_M, RefinedTerrainAssemblyLedger,
};
use crate::nodes::simulation_node::SimulationNode;
use crate::simulation::buildings::allocator::BuildingSiteTerrainSnapshot;

/// One render patch's inputs to a road-touched refined source, as the core held them.
#[derive(Clone, Debug, Default)]
pub(crate) struct RoadTerrainPatchInputs {
    /// The patch's refined cache entry of the current CDT contract, if any.
    pub(crate) entry: Option<Arc<CachedRefinedTerrainPatch>>,
    /// The patch's pending assembly stamps.
    pub(crate) ledger: Option<RefinedTerrainAssemblyLedger>,
    /// Whether a building site lies within the local sample margin, as the ownership refresh
    /// decides it.
    pub(crate) site_owned: bool,
    /// The building sites within the widest road query margin of the patch, in building order;
    /// a source filters them to its own query bounds. Shared until a site near the patch
    /// changes.
    pub(crate) sites: Arc<BuildingSiteTerrainSnapshot>,
}

/// The terrain inputs of every render patch, shared by the road tool's view.
#[derive(Clone, Debug, Default)]
pub(crate) struct RoadTerrainInputs {
    pub(crate) patches: imbl::OrdMap<(usize, usize), Arc<RoadTerrainPatchInputs>>,
    /// The core's global terrain payload generation.
    pub(crate) global_generation: u64,
    /// True while a batched terrain stroke is open: refined sources wait for its end.
    pub(crate) stroke_active: bool,
    /// Increases whenever any of the above changes; a newer revision replaces an older one.
    pub(crate) revision: u64,
}

impl RoadTerrainInputs {
    /// The inputs of one patch; a patch the map does not hold has none.
    pub(crate) fn patch(&self, key: (usize, usize)) -> Option<&RoadTerrainPatchInputs> {
        self.patches.get(&key).map(Arc::as_ref)
    }
}

/// The published terrain inputs and the patches noted since they were last rebuilt.
#[derive(Debug)]
pub(crate) struct RoadTerrainInputsLog {
    inputs: RoadTerrainInputs,
    changed: BTreeSet<(usize, usize)>,
    // The noted patches whose sites may have changed too; a subset of `changed`.
    sites_changed: BTreeSet<(usize, usize)>,
    all: bool,
}

impl Default for RoadTerrainInputsLog {
    fn default() -> Self {
        Self {
            inputs: RoadTerrainInputs::default(),
            changed: BTreeSet::new(),
            sites_changed: BTreeSet::new(),
            all: true,
        }
    }
}

impl RoadTerrainInputsLog {
    /// Notes patches whose inputs may have changed.
    pub(crate) fn note(&mut self, keys: impl IntoIterator<Item = (usize, usize)>) {
        if !self.all {
            self.changed.extend(keys);
        }
    }

    /// Notes patches whose building sites, and so every input, may have changed.
    pub(crate) fn note_sites(&mut self, keys: impl IntoIterator<Item = (usize, usize)>) {
        if !self.all {
            for key in keys {
                self.changed.insert(key);
                self.sites_changed.insert(key);
            }
        }
    }

    /// Notes that every patch may have changed (a world reset, a dense terrain restore).
    pub(crate) fn note_all(&mut self) {
        self.all = true;
        self.changed.clear();
        self.sites_changed.clear();
    }

    /// The inputs as last rebuilt, in `O(1)`.
    pub(crate) fn current(&self) -> RoadTerrainInputs {
        self.inputs.clone()
    }

    /// True when a patch was noted since the last rebuild.
    pub(crate) fn has_changes(&self) -> bool {
        self.all || !self.changed.is_empty()
    }
}

impl SimCore {
    /// Rebuilds the noted patches' terrain inputs and returns the current inputs. `O(noted
    /// patches * log patches + site-noted patches * sites near each)`; every patch after a
    /// reset.
    pub(crate) fn refresh_road_terrain_inputs(&mut self) -> RoadTerrainInputs {
        let log = &mut self.road_terrain_inputs;
        let keys: Vec<(usize, usize)> = if log.all {
            let (cols, rows) = (
                self.heightmap.render_patch_cols(),
                self.heightmap.render_patch_rows(),
            );
            log.inputs.patches = imbl::OrdMap::new();
            (0..rows)
                .flat_map(|z| (0..cols).map(move |x| (x, z)))
                .collect()
        } else {
            std::mem::take(&mut log.changed).into_iter().collect()
        };
        let sites_changed = std::mem::take(&mut log.sites_changed);
        let all = std::mem::replace(&mut log.all, false);
        let global_generation = self.terrain_payload_global_generation;
        let stroke_active = self.terrain_stroke_active;
        let changed = all
            || !keys.is_empty()
            || self.road_terrain_inputs.inputs.global_generation != global_generation
            || self.road_terrain_inputs.inputs.stroke_active != stroke_active;
        if !keys.is_empty() {
            self.allocator
                .prepare_building_site_query_index(self.config.zone_cell_m);
        }
        for key in keys {
            let kept = self
                .road_terrain_inputs
                .inputs
                .patches
                .get(&key)
                .filter(|_| !all && !sites_changed.contains(&key))
                .map(Arc::clone);
            let inputs = self.road_terrain_patch_inputs(key, kept.as_deref());
            self.road_terrain_inputs
                .inputs
                .patches
                .insert(key, Arc::new(inputs));
        }
        let inputs = &mut self.road_terrain_inputs.inputs;
        inputs.global_generation = global_generation;
        inputs.stroke_active = stroke_active;
        if changed {
            inputs.revision += 1;
        }
        inputs.clone()
    }

    // One patch's inputs: the same queries the commit's ownership refresh and source phase run,
    // the sites widened to the largest margin a road edit can give the patch. With `kept`, the
    // patch's sites did not change, so its sites and site ownership are kept.
    fn road_terrain_patch_inputs(
        &self,
        key: (usize, usize),
        kept: Option<&RoadTerrainPatchInputs>,
    ) -> RoadTerrainPatchInputs {
        let step_m = ROAD_LOCKED_TERRAIN_RENDER_STEP_M;
        let cache_key = SimulationNode::refined_patch_cache_key(key.0, key.1, step_m);
        let entry = self
            .refined_terrain_patch_cache
            .get(&cache_key)
            .filter(|entry| SimulationNode::refined_terrain_entry_contract_is_current(entry))
            .cloned();
        let Some((min_x, min_z, max_x, max_z)) =
            self.heightmap.render_patch_world_bounds(key.0, key.1)
        else {
            return RoadTerrainPatchInputs::default();
        };
        let ledger = self.refined_terrain_assembly_ledgers.get(&key).cloned();
        if let Some(kept) = kept {
            return RoadTerrainPatchInputs {
                entry,
                ledger,
                site_owned: kept.site_owned,
                sites: Arc::clone(&kept.sites),
            };
        }
        let widest_m = road_terrain_sites_margin_m(&self.heightmap);
        let sites = self.allocator.terrain_site_snapshot_for_world_bounds(
            min_x - widest_m,
            min_z - widest_m,
            max_x + widest_m,
            max_z + widest_m,
        );
        // The widest margin holds the local one, so the snapshot answers the ownership refresh's
        // site test with the same overlap rule.
        let site_margin_m =
            crate::simulation::terrain::terrain_cdt_local_sample_margin_m(&self.heightmap, step_m);
        let site_owned = sites.has_site_for_world_bounds(
            min_x - site_margin_m,
            min_z - site_margin_m,
            max_x + site_margin_m,
            max_z + site_margin_m,
        );
        RoadTerrainPatchInputs {
            entry,
            ledger,
            site_owned,
            sites: Arc::new(sites),
        }
    }
}

/// The widest site query margin a road-touched source can use around its patch: the road query
/// margin of the larger of the grading envelope cap (which bounds every road margin) and the
/// site margin, plus one metre for the rounding of patch bounds. A site change notes every
/// patch within it.
pub(crate) fn road_terrain_sites_margin_m(
    heightmap: &crate::simulation::terrain::TerrainSystem,
) -> f32 {
    let step_m = ROAD_LOCKED_TERRAIN_RENDER_STEP_M;
    let site_margin_m =
        crate::simulation::terrain::terrain_cdt_local_sample_margin_m(heightmap, step_m);
    crate::simulation::terrain::terrain_cdt_road_query_margin_m(
        heightmap,
        step_m,
        crate::simulation::terrain::grading::GRADING_ENVELOPE_MAX_MARGIN_M.max(site_margin_m),
    ) + 1.0
}
