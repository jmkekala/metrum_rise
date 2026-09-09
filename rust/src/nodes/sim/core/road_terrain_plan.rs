// SPDX-License-Identifier: GPL-2.0-only

//! Complete plan-owned terrain products and bounded post-topology adoption checks.

use super::SimCore;
use super::terrain_payloads::*;
use crate::nodes::simulation_node::SimulationNode;
use crate::simulation::network::surface::RoadEarthworkPlan;
use crate::simulation::terrain::TerrainSystem;
#[cfg(test)]
use crate::simulation::terrain::cdt::TerrainCdtInput;
use rayon::prelude::*;
use std::collections::HashMap;
use std::sync::Arc;

mod sites;
pub(super) use sites::RoadTerrainSiteInputs;

#[cfg(test)]
#[derive(Debug)]
struct PlannedTerrainWindow {
    input: TerrainCdtInput,
    product: Arc<CachedRefinedTerrainCdtWindow>,
}

#[derive(Debug)]
struct PlannedTerrainPatch {
    #[cfg(test)]
    windows: HashMap<RefinedTerrainCdtWindowKey, PlannedTerrainWindow>,
    product: Arc<CachedRefinedTerrainPatch>,
}

#[cfg(test)]
impl PlannedTerrainPatch {
    fn contains_window(&self, window: &Arc<CachedRefinedTerrainCdtWindow>) -> bool {
        self.windows
            .get(&window.key)
            .is_some_and(|candidate| Arc::ptr_eq(&candidate.product, window))
    }

    fn matches_complete_window_set(&self, patch: &RefinedTerrainPatchBuildInput) -> bool {
        self.product.windows.len() == patch.windows.len() + patch.reused_windows.len()
            && patch.windows.iter().all(|window| {
                window
                    .previous
                    .as_ref()
                    .is_some_and(|previous| self.contains_window(previous))
            })
            && patch
                .reused_windows
                .iter()
                .all(|window| self.contains_window(window))
    }
}

/// Local road/site CDT tiles, joined buffers and their exact adoption dependencies.
/// Only the enclosing ready road plan may authorize publication under the simulation lock.
#[derive(Debug)]
pub(crate) struct RoadTerrainPlan {
    source_generation: u64,
    visual_generation: u64,
    patches: HashMap<RefinedTerrainPatchCacheKey, PlannedTerrainPatch>,
    sites: Option<RoadTerrainSiteInputs>,
    failure_reason: Option<&'static str>,
    earthworks: Arc<RoadEarthworkPlan>,
}

impl RoadTerrainPlan {
    /// Uses production CDT and patch/seam composition over the old/new road footprint.
    pub(super) fn compile(
        terrain: &TerrainSystem,
        earthworks: &Arc<RoadEarthworkPlan>,
        graph: &crate::simulation::network::graph::RegionGraph,
        surface: &crate::simulation::network::surface::RoadSurfaceSystem,
        sites: Option<RoadTerrainSiteInputs>,
    ) -> Self {
        let roads = earthworks.roads.view(graph, surface);
        let visual = earthworks.visual.view(terrain);
        let inputs: Vec<_> = earthworks
            .clip_patches
            .par_iter()
            .filter_map(|clip| {
                let (loops, source_count) = clip.loops.as_ref().ok()?;
                let patch = earthworks.visual.patch_snapshot(terrain, &clip.patch);
                let input = SimulationNode::planned_road_terrain_patch_input(
                    &visual,
                    &patch,
                    loops,
                    *source_count,
                    clip.road_owned,
                    clip.query_margin_m,
                    ROAD_LOCKED_TERRAIN_RENDER_STEP_M,
                    sites
                        .as_ref()
                        .and_then(|sites| sites.get(clip.patch.patch_x, clip.patch.patch_z)),
                    roads,
                );
                Some(input)
            })
            .collect();
        // Cold-compiler parity tests retain full inputs; production adopts products directly.
        #[cfg(test)]
        let mut dependencies: HashMap<_, _> = inputs
            .iter()
            .map(|input| {
                (
                    input.key,
                    input
                        .windows
                        .iter()
                        .map(|window| (window.key, window.cdt_input.clone()))
                        .collect::<HashMap<_, _>>(),
                )
            })
            .collect();
        let products = SimCore::build_refined_terrain_patch_cache_entries(inputs);
        // Keep setup failures explicitly; a skipped clip export must never look like readiness.
        let failure_reason = earthworks
            .clip_patches
            .iter()
            .filter_map(|clip| {
                clip.loops
                    .as_ref()
                    .err()
                    .map(|err| ((clip.patch.patch_x, clip.patch.patch_z), err.debug_label()))
            })
            .min_by_key(|(key, _)| *key)
            .map(|(_, reason)| reason)
            .or_else(|| {
                products
                    .iter()
                    .find_map(|patch| SimulationNode::cached_refined_cdt_failure_label(patch))
            });
        let patches = products
            .into_iter()
            .map(|product| {
                #[cfg(test)]
                let mut inputs = dependencies.remove(&product.key).unwrap_or_default();
                #[cfg(test)]
                let windows = product
                    .windows
                    .iter()
                    .filter_map(|window| {
                        let input = inputs.remove(&window.key)?;
                        Some((
                            window.key,
                            PlannedTerrainWindow {
                                input,
                                product: Arc::clone(window),
                            },
                        ))
                    })
                    .collect();
                (
                    product.key,
                    PlannedTerrainPatch {
                        #[cfg(test)]
                        windows,
                        product,
                    },
                )
            })
            .collect();
        Self {
            source_generation: terrain.source_generation(),
            visual_generation: terrain.visual_generation(),
            patches,
            sites,
            failure_reason,
            earthworks: Arc::clone(earthworks),
        }
    }

    /// Checks post-topology site geometry and exact road products before accepting any terrain.
    /// Attachment repair changes references, not authored site pose or support height.
    pub(crate) fn adopted_dependencies_match(&self, core: &SimCore) -> bool {
        !core.terrain_stroke_active
            && self.source_generation == core.heightmap.source_generation()
            && self.earthworks.visual.matches_current(&core.heightmap)
            && self.sites_match(core)
            && self
                .earthworks
                .roads
                .matches_published(&core.region_graph, &core.transit_network.road_surface)
    }

    /// Local visual checkpoint for both failed adoption and user undo.
    pub(crate) fn visual_checkpoint(
        &self,
        terrain: &TerrainSystem,
    ) -> crate::simulation::terrain::TerrainVisualOverlay {
        self.earthworks.visual_checkpoint(terrain)
    }

    /// Complete planned buffers must be adopted: a mismatch is a transaction failure, not
    /// permission to solve new geometry after a ready preview. Metadata remains live.
    #[cfg(test)]
    pub(crate) fn entries_match(&self, entries: &[Arc<CachedRefinedTerrainPatch>]) -> bool {
        entries.iter().all(|entry| {
            self.patches.get(&entry.key).is_some_and(|planned| {
                entry.patch == planned.product.patch
                    && entry.requires_engineered_refinement
                        == planned.product.requires_engineered_refinement
                    && entry.requires_road_clipping == planned.product.requires_road_clipping
                    && entry.clip_query_margin_m == planned.product.clip_query_margin_m
                    && entry.mesh_buffers == planned.product.mesh_buffers
            })
        })
    }

    /// Rejects newly discovered affected patches and any planned owned patch missing at commit.
    pub(crate) fn coverage_matches(&self, keys: &[(usize, usize)]) -> bool {
        let actual: std::collections::HashSet<_> = keys.iter().copied().collect();
        let planned: std::collections::HashSet<_> = self
            .patches
            .keys()
            .map(|key| (key.patch_x, key.patch_z))
            .collect();
        let matches = keys.iter().all(|key| planned.contains(key))
            && self.patches.values().all(|patch| {
                !patch.product.requires_engineered_refinement
                    || actual.contains(&(patch.product.key.patch_x, patch.product.key.patch_z))
            });
        if !matches {
            crate::debug_log!(
                "road",
                "road_plan_coverage actual={actual:?} planned={planned:?}"
            );
        }
        matches
    }

    /// Adopts the complete planned batch after post-topology dependency/coverage checks. Only
    /// generation metadata is rebound; CDT, grading and seam composition do not run at commit.
    pub(crate) fn adopt_products(
        &self,
        core: &SimCore,
    ) -> Result<Vec<Arc<CachedRefinedTerrainPatch>>, String> {
        let patches = self.preview_patches().ok_or("road_plan_incomplete")?;
        let site_margin = crate::simulation::terrain::terrain_cdt_local_sample_margin_m(
            &core.heightmap,
            ROAD_LOCKED_TERRAIN_RENDER_STEP_M,
        );
        patches
            .into_par_iter()
            .map(|planned| {
                let key = (planned.key.patch_x, planned.key.patch_z);
                let current = core
                    .heightmap
                    .visual_patch_snapshot(key.0, key.1)
                    .ok_or("road_plan_patch_missing")?;
                let road_owned = core.road_locked_terrain_patch_margins.contains_key(&key);
                let margin = core
                    .engineered_terrain_patch_margins
                    .get(&key)
                    .copied()
                    .unwrap_or(site_margin)
                    .max(site_margin);
                let margin = if road_owned {
                    crate::simulation::terrain::terrain_cdt_road_query_margin_m(
                        &core.heightmap,
                        ROAD_LOCKED_TERRAIN_RENDER_STEP_M,
                        margin,
                    )
                } else {
                    margin
                };
                // Margins select dependencies, not mesh geometry. Compare the exact road/site
                // sets at both reaches rather than treating a query hint as a geometric input.
                // Extra work is two indexed local queries, only when the margins differ.
                let query_matches = margin == planned.clip_query_margin_m || {
                    let bounds = |margin| {
                        (
                            current.world_origin_x - margin,
                            current.world_origin_z - margin,
                            current.world_origin_x + current.world_size_x + margin,
                            current.world_origin_z + current.world_size_z + margin,
                        )
                    };
                    core.transit_network
                        .road_surface
                        .terrain_clip_contributors_match_world_bounds(
                            &core.region_graph,
                            bounds(planned.clip_query_margin_m),
                            bounds(margin),
                        )
                        && self.sites.as_ref().is_some_and(|sites| {
                            sites.matches_bounds(core, key, bounds(margin))
                        })
                };
                if current != planned.patch
                    || road_owned != planned.requires_road_clipping
                    || core.terrain_patch_requires_engineered_refinement(key.0, key.1)
                        != planned.requires_engineered_refinement
                    || !query_matches
                {
                    return Err(format!(
                        "road_plan_patch_dependency_mismatch patch=({},{}) samples={} road={}/{} engineered={}/{} margin={}/{}",
                        key.0, key.1, current == planned.patch, road_owned, planned.requires_road_clipping,
                        core.terrain_patch_requires_engineered_refinement(key.0,key.1), planned.requires_engineered_refinement,
                        margin, planned.clip_query_margin_m
                    ));
                }
                let mut adopted = planned.as_ref().clone();
                adopted.surface_generation =
                    core.terrain_payload_generation_for_patch(key.0, key.1);
                adopted.cdt_ms = 0.0;
                adopted.reused_windows = adopted.windows.len();
                Ok(Arc::new(adopted))
            })
            .collect()
    }

    /// Whether the plan includes an exact, still-current local building-site dependency set.
    pub(crate) fn sites_match(&self, core: &SimCore) -> bool {
        self.sites.as_ref().is_some_and(|sites| sites.matches(core))
    }

    /// Terrain candidate state, not a promise that its terrain is displayed or atomically ready.
    pub(crate) fn status(&self, core: &SimCore) -> &'static str {
        if core.terrain_stroke_active || !core.allocator.building_site_query_index_ready() {
            return "pending";
        }
        if core.heightmap.source_generation() != self.source_generation
            || core.heightmap.visual_generation() != self.visual_generation
        {
            return "stale";
        }
        if self.sites.is_none() {
            return "provisional";
        }
        if !self.sites_match(core) {
            return "stale";
        }
        if self.failure_reason.is_some() {
            return "invalid";
        }
        "compiled"
    }

    /// First deterministic setup/geometry failure in this candidate, if any.
    pub(crate) fn failure_reason(&self) -> Option<&'static str> {
        self.failure_reason
    }

    /// Whether the full post-stamp batch has site inputs and valid buffers for every cutout.
    /// Caller must separately check current dependencies; exporting is not commit authorization.
    pub(crate) fn has_complete_products(&self) -> bool {
        !(self.sites.is_none()
            || self.failure_reason.is_some()
            || self.patches.len() != self.earthworks.clip_patches.len()
            || self.patches.values().any(|patch| {
                patch.product.input_road_loops > 0
                    && !patch
                        .product
                        .mesh_buffers
                        .as_ref()
                        .is_some_and(|buffers| buffers.variant_payload_valid)
            }))
    }

    /// Exports a complete paired batch in stable patch order without cloning mesh arrays.
    pub(crate) fn preview_patches(&self) -> Option<Vec<Arc<CachedRefinedTerrainPatch>>> {
        if !self.has_complete_products() {
            return None;
        }
        let mut patches: Vec<_> = self
            .patches
            .values()
            .map(|patch| Arc::clone(&patch.product))
            .collect();
        patches.sort_by_key(|patch| {
            (
                patch.key.patch_x,
                patch.key.patch_z,
                patch.key.render_step_mm,
            )
        });
        Some(patches)
    }

    /// Offers exact tiles and composed buffers to the existing cache builder. Fresh inputs retain
    /// their revision, ownership, coverage and errors. O(patch samples + local CDT input size).
    /// Final buffers are adopted only when the builder proves every tile's Arc identity matches.
    #[cfg(test)]
    pub(crate) fn reuse_matching_windows(
        &self,
        source_generation: u64,
        inputs: &mut [RefinedTerrainPatchBuildInput],
    ) -> usize {
        if self.source_generation != source_generation {
            return 0;
        }
        let mut reused = 0;
        for patch in inputs {
            let Some(planned) = self.patches.get(&patch.key) else {
                continue;
            };
            // Patch-local buffers also depend on the patch frame and boundary samples.
            if planned.product.patch != patch.patch {
                continue;
            }
            // Unchanged live windows may be carried by the incremental source planner. Adopt
            // the planned identity only after comparing the complete existing geometry.
            for window in &mut patch.reused_windows {
                if let Some(candidate) = planned.windows.get(&window.key)
                    && candidate.product.mesh_result == window.mesh_result
                    && candidate.product.mesh_buffers == window.mesh_buffers
                    && candidate.product.road_input == window.road_input
                    && candidate.product.road_clip_fingerprints == window.road_clip_fingerprints
                    && candidate.product.site_clip_fingerprints == window.site_clip_fingerprints
                    && candidate.product.has_engineered_contributor
                        == window.has_engineered_contributor
                {
                    *window = Arc::clone(&candidate.product);
                }
            }
            for window in &mut patch.windows {
                let Some(candidate) = planned.windows.get(&window.key) else {
                    continue;
                };
                let product = &candidate.product;
                if product.mesh_result.is_ok()
                    && product.mesh_buffers.is_some()
                    && candidate.input == window.cdt_input
                    && product.road_input == window.road_input
                    && product.has_engineered_contributor == window.has_engineered_contributor
                    && product.road_clip_fingerprints == window.road_clip_fingerprints
                    && product.site_clip_fingerprints == window.site_clip_fingerprints
                {
                    window.previous = Some(Arc::clone(product));
                    reused += 1;
                }
            }
            // Never replace the authoritative build input or its contributor manifests. The
            // production builder rechecks complete tile identity and coverage before sharing the
            // composed buffers, and commit runs the same final quality checks as a cold build.
            if planned.matches_complete_window_set(patch)
                && planned.product.mesh_buffers.is_some()
                && SimulationNode::cached_refined_cdt_failure_label(&planned.product).is_none()
            {
                patch.previous_patch = Some(Arc::clone(&planned.product));
            }
        }
        reused
    }

    /// Counts final buffer identity reuse, not merely a successfully offered candidate.
    #[cfg(test)]
    pub(crate) fn reused_patch_buffer_count(
        &self,
        entries: &[Arc<CachedRefinedTerrainPatch>],
    ) -> usize {
        entries
            .iter()
            .filter(|entry| {
                self.patches.get(&entry.key).is_some_and(|planned| {
                    planned
                        .product
                        .mesh_buffers
                        .as_ref()
                        .zip(entry.mesh_buffers.as_ref())
                        .is_some_and(|(planned, actual)| Arc::ptr_eq(planned, actual))
                })
            })
            .count()
    }
}
